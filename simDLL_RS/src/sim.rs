//! Sim — 模拟主类（仅字段 + Phase 7 Sim::Main 后台线程）。
//!
//! 字段对照源码 00_types_reference.c L6982-7052（`struct Sim`）。
//! 运行时：
//! - `operator_new(0x228)` 分配 Sim（01_sim_api.c L330）→ sizeof(Sim) = 552B
//! - `Sim::Sim(this, &gFrameSync)` 构造（01_sim_api.c L332）
//! - SimFrameManager 子对象位于 offset 0x50（01_sim_api.c L227）
//! - Thread 基类位于 offset 0..0x50（Thread::Start/Join/Stop 调用 gSim 强转）
//! - frameSync 指针位于 offset 0x220（最后 8B，源码 L7051）
//!
//! Phase 7 改造：Sim::Main 后台线程实现为 `std::thread::spawn` + `Arc<AtomicBool>` 停止标志，
//! 替代原版持久化 `Thread` 基类（运行时无差异，见规格 §5.2）。

use crate::frame_sync::FrameSync;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{JoinHandle, spawn};

/// Sim — 模拟主类（继承自 Thread 类）。
///
/// **内存布局**（源码 L6982-7052 + 运行时偏移反推）：
/// ```text
/// 0x000 ┌──────────────────────────┐
///       │ Thread 基类字段            │  80B（vtable + handle + running + ...）
/// 0x050 ├──────────────────────────┤
///       │ SimFrameManager 子对象    │  约 288B（Phase 1 用 reserved 占位）
/// 0x170 ├──────────────────────────┤
///       │ 其他字段（待 Phase 3+ 定位）│  约 176B
/// 0x220 ├──────────────────────────┤
///       │ struct FrameSync *frameSync │  8B
/// 0x228 └──────────────────────────┘
/// ```
#[repr(C)]
pub struct Sim {
    /// 0x00..0x50 (80B) —— Thread 基类占位。
    ///
    /// 源码 decompiler 显示为 longlong padding，实际包含：
    /// - vtable 指针（8B，Thread 虚函数表）
    /// - thread_handle（Win32 HANDLE，8B）
    /// - running（bool，1B + 7B padding）
    /// - 其他 Thread 内部字段（待 Phase 3+ 定位）
    pub reserved_head: [u8; 0x50],
    /// 0x50..0x220 (464B) —— SimFrameManager 子对象 + 其他字段占位。
    ///
    /// 实际包含（运行时偏移验证）：
    /// - 0x50..0x170: SimFrameManager 子对象（约 288B，sizeof 见 sim_frame_manager.rs）
    /// - 0x170..0x220: 其他字段（约 176B，Phase 3+ 定位）
    pub reserved_middle: [u8; 0x1D0],
    /// 0x220..0x228 (8B) —— FrameSync back-pointer（源码 L7051）。
    ///
    /// 由 `Sim::Sim(this, &gFrameSync)` 构造时设置（01_sim_api.c L332）。
    /// SimFrameManager 通过此指针回调 FrameSync::SimSync（08_sim_frame_manager.c L2165）。
    pub frame_sync: *mut FrameSync,
}

impl Sim {
    /// 构造零初始化的 Sim。
    pub fn new_zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }

    /// Phase 6+ TODO: 真实实现（源码位置：02_save_load.c L252）。
    /// Phase 3 stub：仅记日志，不启动线程。
    pub fn start(&mut self) {
        tracing::debug!("Thread::start stub called");
    }

    /// Phase 6+ TODO: 真实实现（源码位置：02_save_load.c L638）。
    /// Phase 3 stub：仅记日志，不等待线程。
    pub fn join(&mut self) {
        tracing::debug!("Thread::join stub called");
    }

    /// Phase 6+ TODO: 真实实现（源码位置：02_save_load.c L633）。
    /// Phase 3 stub：返回 false（线程未启动）。
    pub fn joinable(&self) -> bool {
        tracing::debug!("Thread::joinable stub called, returning false");
        false
    }

    /// 启动 Sim::Main 后台线程。
    ///
    /// 对照源码 02_save_load.c L252 `Thread::Start`。
    /// **改造**：用 `std::thread::spawn` 替代持久化 Thread 基类。
    pub fn start_main_thread(&mut self) {
        let mut mt = G_SIM_MAIN_THREAD.lock().unwrap_or_else(|e| e.into_inner());
        if mt.join_handle.is_some() {
            tracing::warn!("Sim::Main thread already running, skip");
            return;
        }
        mt.stop_flag.store(false, Ordering::Relaxed);
        let stop_flag = Arc::clone(&mt.stop_flag);
        let handle = spawn(move || {
            sim_main_loop(stop_flag);
        });
        mt.join_handle = Some(handle);
        tracing::info!("Sim::Main background thread started");
    }

    /// 停止 Sim::Main 后台线程（同步等待退出）。
    ///
    /// 对照源码 02_save_load.c L633 `Thread::Stop` + L638 `Thread::Join`。
    pub fn stop_main_thread(&mut self) {
        let mut mt = G_SIM_MAIN_THREAD.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = mt.join_handle.take() {
            mt.stop_flag.store(true, Ordering::Relaxed);
            // 释放锁后 join（避免死锁，因为 main_loop 也可能要锁 G_SIM_MAIN_THREAD）
            drop(mt);
            let _ = handle.join();
            tracing::info!("Sim::Main background thread stopped");
        }
    }
}

/// `Sim::Main` 主循环（独立函数，spawn 启动）。
///
/// 对照源码 08_sim_frame_manager.c L2086-2168。
/// Phase 7 简化版（T6 接入完整 ProcessFrame 调用）：
/// - 锁 G_SIM_FRAME_MANAGER
/// - begin_frame_processing
/// - 若 count < 1：update_components_data_list_only
/// - 否则循环 process_next_frame
/// - end_frame_processing
fn sim_main_loop(stop_flag: Arc<AtomicBool>) {
    while !stop_flag.load(Ordering::Relaxed) {
        let count = {
            let mut mgr = crate::sim_frame_manager::G_SIM_FRAME_MANAGER
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            mgr.begin_frame_processing()
        };
        if count < 1 {
            // 无待处理帧：仅更新组件数据列表
            // T6 接入 sim_data.update_components_data_list_only()
            std::thread::sleep(std::time::Duration::from_millis(1));
        } else {
            for _ in 0..count {
                let _elapsed = {
                    let mut mgr = crate::sim_frame_manager::G_SIM_FRAME_MANAGER
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    mgr.process_next_frame(std::ptr::null_mut())
                };
                // T14 接入 sim_base_update_data 多线程温度
            }
            {
                let mut mgr = crate::sim_frame_manager::G_SIM_FRAME_MANAGER
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                mgr.end_frame_processing();
            }
        }
    }
    tracing::info!("Sim::Main loop exiting (stop_flag=true)");
}

// Safety: Sim 含裸指针，跨线程共享需 unsafe（Phase 3+ 接入业务逻辑时审视）
unsafe impl Send for Sim {}
unsafe impl Sync for Sim {}

/// Phase 7：Sim::Main 后台线程的运行时状态（不在 Sim struct 内，避免改变 sizeof(Sim) = 0x228）。
///
/// 对照源码 `Thread` 基类（80B）+ `Sim::Main` 主循环。
/// **改造**：原版用 Thread 基类的 `m_running` + `m_handle`；Rust 用 `Arc<AtomicBool>` + `Option<JoinHandle>`。
pub struct SimMainThread {
    /// 停止标志：true 时主循环退出
    pub stop_flag: Arc<AtomicBool>,
    /// 线程句柄：Some 时线程在运行，None 时未启动或已停止
    pub join_handle: Option<JoinHandle<()>>,
}

impl SimMainThread {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            join_handle: None,
        }
    }
}

// 全局 SimMainThread 单例（与 G_SIM 并列）
use once_cell::sync::Lazy;
use std::sync::Mutex;
pub static G_SIM_MAIN_THREAD: Lazy<Mutex<SimMainThread>> =
    Lazy::new(|| Mutex::new(SimMainThread::new()));

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    /// 跨模块共享的测试序列化锁：sim::tests 新增 main_thread 测试操作全局状态
    /// G_SIM_MAIN_THREAD（且后台线程会间接锁 G_SIM_FRAME_MANAGER），需串行化。
    use crate::LIB_TESTS_LOCK as PHASE4_TEST_LOCK;

    #[test]
    fn sim_size_matches_source() {
        // operator_new(0x228) 是源码硬编码值（01_sim_api.c L330）
        assert_eq!(size_of::<Sim>(), 0x228);  // 552B
    }

    #[test]
    fn sim_new_zeroed() {
        let s = Sim::new_zeroed();
        assert!(s.frame_sync.is_null());
    }

    #[test]
    fn stub_start_does_not_panic() {
        let mut s = Sim::new_zeroed();
        s.start();
    }

    #[test]
    fn stub_join_does_not_panic() {
        let mut s = Sim::new_zeroed();
        s.join();
    }

    #[test]
    fn stub_joinable_returns_false() {
        let s = Sim::new_zeroed();
        assert!(!s.joinable());
    }

    #[test]
    fn start_main_thread_spawns_and_stops() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Sim::new_zeroed();
        s.start_main_thread();
        // 验证线程已启动
        {
            let mt = G_SIM_MAIN_THREAD.lock().unwrap();
            assert!(mt.join_handle.is_some());
        }
        // 立即停止
        s.stop_main_thread();
        // 验证线程已停止
        {
            let mt = G_SIM_MAIN_THREAD.lock().unwrap();
            assert!(mt.join_handle.is_none());
        }
    }

    #[test]
    fn start_main_thread_idempotent_when_already_running() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Sim::new_zeroed();
        s.start_main_thread();
        s.start_main_thread();  // 第二次应 warn 并跳过
        s.stop_main_thread();
    }
}
