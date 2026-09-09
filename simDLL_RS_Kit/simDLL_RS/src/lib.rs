//! simDLL_RS v2 - Rust port of Oxygen Not Included's simDLL.dll
//!
//! 阶段 A1：项目骨架 + 基础设施

#![allow(dead_code)]

pub mod a_framework;
pub mod b_elements;
pub mod c_simulation;
pub mod c2_physics;
pub mod d1_activity;
pub mod d2_thread;
#[cfg(feature = "d3-rayon")]
pub mod d3_rayon;
#[cfg(feature = "d4-slice")]
pub mod d4_slice;                 // 新增

pub mod globals {
    //! 全局静态变量，对应原版 simDLL 的全局状态。

    use crate::a_framework::abi::GameMessageHandler;
    use crate::a_framework::frame_sync::FrameSync;
    use crate::a_framework::game_data_update::GameDataUpdate;
    use crate::a_framework::sim::Sim;
    use crate::a_framework::sim_data::SimData;
    use crate::b_elements::disease::Disease;
    use once_cell::sync::OnceCell;
    use parking_lot::Mutex;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicU32};

    /// 裸指针包装类型，实现 Send + Sync。
    ///
    /// 原版 simDLL 的全局变量（gSimData/gSim/gDisease/SaveBuffer）都是裸指针，
    /// Rust 的裸指针不实现 Send/Sync。这些指针受 Mutex 保护，可安全跨线程访问。
    /// `#[repr(transparent)]` 确保内存布局与 `*mut T` 完全一致。
    #[repr(transparent)]
    pub struct SendSyncPtr<T>(pub *mut T);

    unsafe impl<T> Send for SendSyncPtr<T> {}
    unsafe impl<T> Sync for SendSyncPtr<T> {}

    impl<T> SendSyncPtr<T> {
        /// 返回裸指针。方法调用强制闭包捕获整个包装（Rust 2021 disjoint capture
        /// 下直接读 `.0` 字段只会捕获裸指针本身，绕过 Send 实现——d3 rayon 必需）。
        pub fn get(&self) -> *mut T {
            self.0
        }
    }

    impl<T> Default for SendSyncPtr<T> {
        fn default() -> Self {
            Self(std::ptr::null_mut())
        }
    }

    impl<T> Clone for SendSyncPtr<T> {
        fn clone(&self) -> Self {
            Self(self.0)
        }
    }

    impl<T> Copy for SendSyncPtr<T> {}

    /// C# 注册的消息回调。对应原版 `gGameMessageHandler`。
    /// 用 Mutex<Option<..>> 以便 CleanUp 时置空（原版 CleanUp L645 置 0）。
    pub static G_GAME_MESSAGE_HANDLER: Mutex<Option<GameMessageHandler>> = Mutex::new(None);

    /// 全局 SimData 指针。对应原版 `gSimData`（unique_ptr<SimData>）。
    /// null 表示尚未 AllocateCells。
    pub static G_SIM_DATA: Mutex<SendSyncPtr<SimData>> = Mutex::new(SendSyncPtr(std::ptr::null_mut()));

    /// 全局 FrameSync。对应原版 `gFrameSync`。
    /// A3 用 OnceCell 延迟初始化（需要 Rust 同步原语，不能零初始化）。
    pub static G_FRAME_SYNC: OnceCell<Mutex<FrameSync>> = OnceCell::new();

    /// 全局 Sim 指针。对应原版 `gSim`（unique_ptr<Sim>）。
    pub static G_SIM: Mutex<SendSyncPtr<Sim>> = Mutex::new(SendSyncPtr(std::ptr::null_mut()));

    /// 全局 Disease 指针。对应原版 `gDisease`。
    pub static G_DISEASE: Mutex<SendSyncPtr<Disease>> = Mutex::new(SendSyncPtr(std::ptr::null_mut()));

    /// 全局 GameDataUpdate holder。对应原版 PrepareGameDataUpdate 返回值。
    /// Start handler 返回此指针的地址给 C#。
    /// 用 Mutex 包装以支持 interior mutability（PrepareGameDataUpdate 每帧修改字段）。
    pub static G_GAME_DATA_UPDATE: OnceCell<parking_lot::Mutex<GameDataUpdate>> = OnceCell::new();

    /// 全局 SaveBuffer 指针。对应原版 `SaveBuffer`。
    pub static G_SAVE_BUFFER: Mutex<SendSyncPtr<c_void>> = Mutex::new(SendSyncPtr(std::ptr::null_mut()));

    // ===== C1: Sim 线程控制全局变量 =====

    /// sim 线程退出标志。
    /// SIM_Shutdown 设置为 true，sim 线程检测到后退出主循环。
    /// 对照源码 Sim::Main while 条件 `(char)this->_padding_ == '\0'`。
    pub static G_SIM_EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

    /// sim 线程时间余量（f32 的位模式，用 AtomicU32 存储）。
    /// 对照源码 Sim::Main L2133-2145 的 `*(float *)&this->_padding_`。
    /// 用于累积帧时间不足一个 200ms 子步的余量。
    pub static G_SIM_TIME_REMAINDER: AtomicU32 = AtomicU32::new(0);

    /// D2：全局 ParallelTaskQueue（原版 Sim 对象上的 taskQueue，SimBase::InitializeTasks
    /// 创建，1 worker）。Box 堆分配保证地址稳定（worker 持有队列指针），
    /// SIM_Initialize 重建 / SIM_Shutdown 释放。更新数据里温度任务经它分发；
    /// d3（rayon）可增加 worker 数。
    pub static G_PARALLEL_TASK_QUEUE: Mutex<SendSyncPtr<crate::d2_thread::ParallelTaskQueue>> =
        Mutex::new(SendSyncPtr(std::ptr::null_mut()));

    /// 初始化全局任务队列（原版 SimBase::InitializeTasks：1 worker）。
    pub fn init_parallel_task_queue() {
        let mut g = G_PARALLEL_TASK_QUEUE.lock();
        if !g.0.is_null() {
            unsafe {
                (*g.0).shutdown();
                let _ = Box::from_raw(g.0);
            }
            g.0 = std::ptr::null_mut();
        }
        let q = Box::new(crate::d2_thread::ParallelTaskQueue::new(1));
        q.start_workers();
        g.0 = Box::into_raw(q);
    }

    /// 关闭并释放全局任务队列（原版析构：置 mShuttingDown + join workers）。
    pub fn shutdown_parallel_task_queue() {
        let mut g = G_PARALLEL_TASK_QUEUE.lock();
        if !g.0.is_null() {
            unsafe {
                (*g.0).shutdown();
                let _ = Box::from_raw(g.0);
            }
            g.0 = std::ptr::null_mut();
        }
    }
}

// ─────────────────────────────────────────────────────────────
// DLL 入口点（Windows 加载/卸载 DLL 时调用）
// ─────────────────────────────────────────────────────────────
#[cfg(windows)]
#[no_mangle]
pub extern "system" fn DllMain(
    _hinst: *mut core::ffi::c_void,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        // A1 阶段：无初始化逻辑
    }
    1 // TRUE
}

/// 跨模块共享的测试序列化锁：串行化所有操作全局状态的 lib 单元测试，
/// 避免并行执行导致 setup 互相覆盖。
#[cfg(test)]
pub(crate) static LIB_TESTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
