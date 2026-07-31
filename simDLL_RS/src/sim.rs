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
use crate::sim_data::SimData;
use crate::sim_events::SimEvents;
use crate::temperature::{
    consolidate_events, do_state_transition, update_temperature,
    update_temperature_for_backwall_exchange,
};
use crate::element::ElementTemperatureData;
use crate::elements_table::G_ELEMENTS_TABLE;
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
                // T14: 多线程温度派发（对照源码 SimBase::UpdateData）
                // 锁 G_SIM_DATA 调 sim_base_update_data 完成：
                // - iterate_direction 翻转
                // - cells = updated_cells 同步
                // - 各 active_region 并行温度交换 + 状态转换
                // - consolidate_events 合并 per-thread SimEvents
                {
                    let mut g = crate::save_load::G_SIM_DATA
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    sim_base_update_data(&mut **g);
                }
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

/// `SimBase::UpdateData` — 模拟主循环的温度派发函数。
///
/// 严格对照源码 11_msvcrt_ignored.c L41811-41930+（SimBase::UpdateData）+
/// L41693-41807（SimUpdateTemperatureTask::InternalDoTask）。
///
/// **改造**：原版用 taskQueue 派发到固定数量 worker 线程；Rust 用 `std::thread::scope`
/// 按可用并行度派发到 scope 内的临时线程。每 worker 拥有独立 SimEvents，scope
/// 结束后调 `consolidate_events` 合并到主 `sim_events`。
///
/// # 算法概述（对照源码 L41901-42036 + L42393）
/// 1. `iterate_direction = -iterate_direction`（L41901）
/// 2. `cells = updated_cells`（L41903：同步状态）
/// 3. 对每个 active_region：
///    - num_threads = `available_parallelism`
///    - 创建 `Vec<SimEvents>`（num_threads 个空 SimEvents）
///    - 快照 `G_ELEMENTS_TABLE` 的 `temperature_data` Vec（lock once, clone）
///    - `std::thread::scope`：每个线程处理 y-strip `[start_y, end_y)`
///      - Phase 1 温度交换：遍历 [min_x,max_x)×[start_y,end_y)，
///        调 `update_temperature_for_backwall_exchange` + `update_temperature(cell, cell+1)`
///        + `update_temperature(cell, cell+width)`
///      - Phase 2 状态转换：同范围遍历，查 element_idx → ElementTemperatureData，
///        调 `do_state_transition`
/// 4. scope 后：`cells = updated_cells`（L42036：同步状态）
/// 5. 调 `consolidate_events(main_sim_events, &per_thread_sim_events)`（L42393）
///
/// # 多线程 `&mut SimData` 难题
/// `update_temperature` / `do_state_transition` 等签名要求 `&mut SimData`，但多线程
/// 无法共享 `&mut`。本函数把 `sim_data` 转 `*mut SimData`，scope 内各线程
/// `unsafe { &mut *ptr }` 借用。
///
/// # Safety
/// - `sim_data` 必须存活至本函数返回（调用方保证：来自 `G_SIM_DATA.lock()`）
/// - 多线程写 `updated_cells` 不同 y-strip 不相交（除 vertical 边界 cell
///   `cell+width` 可能落入下一线程 strip——与原版一致，benign race：
///   两侧计算结果近似，最终值任一即可，原版同样行为）
/// - per-thread SimEvents 各自独立，无共享写入
/// - `G_ELEMENTS_TABLE` 快照仅在 scope 外 lock 一次，scope 内只读 `td_snapshot`
pub fn sim_base_update_data(sim_data: &mut SimData) {
    // L41901: iterate_direction = -iterate_direction
    sim_data.iterate_direction = -sim_data.iterate_direction;

    // L41903: cells = updatedCells（同步状态）
    if !sim_data.cells.ptr.is_null() && !sim_data.updated_cells.ptr.is_null() {
        let cells = unsafe { &mut *sim_data.cells.ptr };
        let updated = unsafe { &*sim_data.updated_cells.ptr };
        cells.copy_from(updated);
    }

    // 源码用 taskQueue worker 数；Rust 用 available_parallelism
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);

    // 快照 ElementTemperatureData 表（lock once, clone Vec）
    // 避免在 scope 内每线程 lock G_ELEMENTS_TABLE 造成锁竞争
    let td_snapshot: Vec<ElementTemperatureData> = {
        let table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        table.temperature_data.clone()
    };

    // 转裸指针为 usize 以满足 scope::spawn 闭包的 Send bound。
    // 裸指针 *mut T 默认 !Send；Rust 2021 disjoint capture 会穿透 newtype wrapper
    // 直接捕获字段，故用 usize（Copy + Send）在闭包间传递，闭包内再 cast 回 *mut SimData。
    let sd_addr: usize = sim_data as *mut SimData as usize;

    let num_regions = sim_data.active_regions.len();
    for i in 0..num_regions {
        let region = sim_data.active_regions.as_slice()[i];
        if region.max_x <= region.min_x || region.max_y <= region.min_y {
            continue;
        }

        // 创建 per-thread SimEvents
        let mut per_thread_events: Vec<SimEvents> = (0..num_threads)
            .map(|_| SimEvents::default())
            .collect();

        // 计算每线程的 y-strip 大小（向上取整）
        let total_y = region.max_y - region.min_y;
        let strip_size = (total_y + num_threads as i32 - 1) / num_threads as i32;

        std::thread::scope(|s| {
            for (t, events) in per_thread_events.iter_mut().enumerate() {
                let start_y = region.min_y + (t as i32) * strip_size;
                let end_y = (start_y + strip_size).min(region.max_y);
                if start_y >= end_y {
                    continue;
                }

                let sd_addr = sd_addr;
                let td_snap = &td_snapshot;
                let region_local = region;

                s.spawn(move || {
                    // SAFETY: Each thread writes to disjoint y-strips in updated_cells.
                    // Boundary cells (vertical neighbors at y-strip edges, i.e. cell+width
                    // where cell is at y=end_y-1) may be written by multiple threads —
                    // benign race consistent with source code behavior (last writer wins,
                    // both compute similar temperature values from read-only `cells`).
                    let sd = unsafe { &mut *(sd_addr as *mut SimData) };
                    let width = sd.width;

                    // Phase 1: 温度交换（对照源码 InternalDoTask 温度交换阶段）
                    for y in start_y..end_y {
                        for x in region_local.min_x..region_local.max_x {
                            let cell_idx = y * width + x;
                            update_temperature_for_backwall_exchange(sd, events, cell_idx);
                            // 水平：cell & cell+1（同 y，都在本线程 strip 内）
                            update_temperature(sd, events, cell_idx, cell_idx + 1);
                            // 垂直：cell & cell+width（下一 y，可能落入下一线程 strip）
                            update_temperature(sd, events, cell_idx, cell_idx + width);
                        }
                    }

                    // Phase 2: 状态转换（对照源码 InternalDoTask DoStateTransition 阶段）
                    if sd.updated_cells.ptr.is_null() {
                        return;
                    }
                    let updated_cells = unsafe { &*sd.updated_cells.ptr };
                    let elem_slice = updated_cells.element_idx.as_slice();
                    let mass_slice = updated_cells.mass.as_slice();

                    for y in start_y..end_y {
                        for x in region_local.min_x..region_local.max_x {
                            let cell_idx = (y * width + x) as usize;
                            if cell_idx >= elem_slice.len() || cell_idx >= mass_slice.len() {
                                continue;
                            }
                            if mass_slice[cell_idx] <= 0.0 {
                                continue;
                            }
                            let elem_idx = elem_slice[cell_idx];
                            let td_idx = elem_idx as usize;
                            if td_idx >= td_snap.len() {
                                continue;
                            }
                            let td: &ElementTemperatureData = &td_snap[td_idx];
                            do_state_transition(sd, events, cell_idx as i32, td);
                        }
                    }
                });
            }
        });

        // L42036: cells = updated_cells（同步状态）
        if !sim_data.cells.ptr.is_null() && !sim_data.updated_cells.ptr.is_null() {
            let cells = unsafe { &mut *sim_data.cells.ptr };
            let updated = unsafe { &*sim_data.updated_cells.ptr };
            cells.copy_from(updated);
        }

        // L42393: consolidate_events
        if !sim_data.sim_events.ptr.is_null() {
            let main_events = unsafe { &mut *sim_data.sim_events.ptr };
            consolidate_events(main_events, &per_thread_events);
        }
    }
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

    // ===== T14: sim_base_update_data 集成测试 =====

    /// 释放 SimData 内 Box::into_raw 分配的 cells/updated_cells/backwall/sim_events。
    /// 测试结束前手动调以避免内存泄漏（SimData 无 Drop）。
    fn cleanup_simdata(sd: &mut SimData) {
        unsafe {
            if !sd.cells.ptr.is_null() {
                drop(Box::from_raw(sd.cells.ptr));
                sd.cells.ptr = std::ptr::null_mut();
            }
            if !sd.updated_cells.ptr.is_null() {
                drop(Box::from_raw(sd.updated_cells.ptr));
                sd.updated_cells.ptr = std::ptr::null_mut();
            }
            if !sd.backwall.ptr.is_null() {
                drop(Box::from_raw(sd.backwall.ptr));
                sd.backwall.ptr = std::ptr::null_mut();
            }
            if !sd.sim_events.ptr.is_null() {
                drop(Box::from_raw(sd.sim_events.ptr));
                sd.sim_events.ptr = std::ptr::null_mut();
            }
        }
    }

    #[test]
    fn sim_base_update_data_zeroed_no_panic() {
        // 全零 SimData：无 cells、无 active_regions、无 sim_events
        // 应不 panic（所有指针 null 检查防御）
        let mut sd = SimData::new_zeroed();
        sim_base_update_data(&mut sd);
        // iterate_direction 应被翻转：0 -> -0 = 0
        assert_eq!(sd.iterate_direction, 0);
    }

    #[test]
    fn sim_base_update_data_flips_iterate_direction() {
        // iterate_direction = 1 -> -1
        let mut sd = SimData::new_zeroed();
        sd.iterate_direction = 1;
        sim_base_update_data(&mut sd);
        assert_eq!(sd.iterate_direction, -1);
        // 再次调用应翻回 1
        sim_base_update_data(&mut sd);
        assert_eq!(sd.iterate_direction, 1);
    }

    #[test]
    fn sim_base_update_data_skips_degenerate_regions() {
        // max_x <= min_x 或 max_y <= min_y 的 region 应被跳过
        let mut sd = SimData::new_zeroed();
        sd.width = 4;
        sd.height = 4;
        sd.active_regions.resize(2, crate::sim_data::ActiveRegion::default());
        // region 0：退化（max_x == min_x）
        let regions = sd.active_regions.as_mut_slice();
        regions[0].min_x = 1;
        regions[0].max_x = 1;  // max_x <= min_x → 跳过
        regions[0].min_y = 1;
        regions[0].max_y = 3;
        // region 1：退化（max_y < min_y）
        regions[1].min_x = 1;
        regions[1].max_x = 3;
        regions[1].min_y = 2;
        regions[1].max_y = 1;  // max_y < min_y → 跳过
        // 无 cells 分配，但退化 region 不会触达 → 不 panic
        sim_base_update_data(&mut sd);
    }

    #[test]
    fn sim_base_update_data_syncs_cells_from_updated_cells() {
        // 验证 L41903：cells = updated_cells（函数开头 copy_from）
        let mut sd = SimData::new_zeroed();
        sd.width = 4;
        sd.height = 4;

        // 分配 cells / updated_cells
        let cells = Box::new(crate::sim_data::CellSOA::new());
        let mut updated = Box::new(crate::sim_data::CellSOA::new());
        // 在 updated_cells 写入测试数据
        updated.element_idx.resize(4, 7);
        updated.temperature.resize(4, 300.0);
        let temp_slice = updated.temperature.as_mut_slice();
        temp_slice[0] = 100.0;
        temp_slice[1] = 200.0;
        temp_slice[2] = 300.0;
        temp_slice[3] = 400.0;

        sd.cells.ptr = Box::into_raw(cells);
        sd.updated_cells.ptr = Box::into_raw(updated);

        // 不设 active_regions → 跳过温度派发，但开头的 copy_from 仍执行
        sim_base_update_data(&mut sd);

        // 验证 cells 已同步 updated_cells 的数据
        let cells_ref = unsafe { &*sd.cells.ptr };
        assert_eq!(cells_ref.element_idx.len(), 4);
        assert_eq!(cells_ref.element_idx.as_slice()[0], 7);
        assert_eq!(cells_ref.temperature.len(), 4);
        assert_eq!(cells_ref.temperature.as_slice()[2], 300.0);

        cleanup_simdata(&mut sd);
    }

    #[test]
    fn sim_base_update_data_with_active_region_no_panic() {
        // 完整设置：分配 cells + updated_cells + sim_events + 一个 active region
        // 验证多线程温度派发不 panic（无断言具体温度值，仅冒烟）
        let mut sd = SimData::new_zeroed();
        sd.width = 6;  // 含边界的内部宽
        sd.height = 6;
        sd.num_game_cells = 36;
        sd.iterate_direction = 1;

        // 分配 cells / updated_cells / sim_events
        let cells = Box::new(crate::sim_data::CellSOA::new());
        let mut updated = Box::new(crate::sim_data::CellSOA::new());
        // 初始化 36 个 cell：element_idx=0（vacuum 假设）、temperature=300K、mass=1.0
        updated.element_idx.resize(36, 0);
        updated.temperature.resize(36, 300.0);
        updated.mass.resize(36, 1.0);
        sd.cells.ptr = Box::into_raw(cells);
        sd.updated_cells.ptr = Box::into_raw(updated);
        sd.sim_events.ptr = Box::into_raw(Box::new(crate::sim_events::SimEvents::default()));

        // 一个 active region：覆盖内部 4x4 区域（去掉边界）
        sd.active_regions.resize(1, crate::sim_data::ActiveRegion::default());
        let region = &mut sd.active_regions.as_mut_slice()[0];
        region.min_x = 1;
        region.min_y = 1;
        region.max_x = 5;
        region.max_y = 5;

        // 调用：应执行多线程温度交换 + 状态转换（element_idx=0 若超出 td_snapshot 范围则跳过）
        sim_base_update_data(&mut sd);

        // 验证 iterate_direction 翻转
        assert_eq!(sd.iterate_direction, -1);
        // 验证 cells 已同步
        let cells_ref = unsafe { &*sd.cells.ptr };
        assert_eq!(cells_ref.element_idx.len(), 36);
        assert_eq!(cells_ref.mass.len(), 36);

        cleanup_simdata(&mut sd);
    }

    #[test]
    fn sim_base_update_data_multiple_regions_no_panic() {
        // 多个 active region 应顺序处理，不 panic
        let mut sd = SimData::new_zeroed();
        sd.width = 8;
        sd.height = 8;
        sd.num_game_cells = 64;

        let cells = Box::new(crate::sim_data::CellSOA::new());
        let mut updated = Box::new(crate::sim_data::CellSOA::new());
        updated.element_idx.resize(64, 0);
        updated.temperature.resize(64, 295.0);
        updated.mass.resize(64, 0.0);  // mass=0 → 状态转换 phase 跳过
        sd.cells.ptr = Box::into_raw(cells);
        sd.updated_cells.ptr = Box::into_raw(updated);

        // 两个不相交 region
        sd.active_regions.resize(2, crate::sim_data::ActiveRegion::default());
        let regions = sd.active_regions.as_mut_slice();
        regions[0].min_x = 1; regions[0].min_y = 1; regions[0].max_x = 4; regions[0].max_y = 4;
        regions[1].min_x = 4; regions[1].min_y = 4; regions[1].max_x = 7; regions[1].max_y = 7;

        sim_base_update_data(&mut sd);

        // 两次调用确保稳定性
        sim_base_update_data(&mut sd);

        cleanup_simdata(&mut sd);
    }
}
