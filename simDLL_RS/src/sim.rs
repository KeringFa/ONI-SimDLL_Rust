//! Sim — 模拟主类（仅字段，无方法）。
//!
//! 字段对照源码 00_types_reference.c L6982-7052（`struct Sim`）。
//! 运行时：
//! - `operator_new(0x228)` 分配 Sim（01_sim_api.c L330）→ sizeof(Sim) = 552B
//! - `Sim::Sim(this, &gFrameSync)` 构造（01_sim_api.c L332）
//! - SimFrameManager 子对象位于 offset 0x50（01_sim_api.c L227）
//! - Thread 基类位于 offset 0..0x50（Thread::Start/Join/Stop 调用 gSim 强转）
//! - frameSync 指针位于 offset 0x220（最后 8B，源码 L7051）
//! Phase 1 用占位字段，Phase 3+ 补全 Thread 基类与 SimFrameManager 子对象布局。

use crate::frame_sync::FrameSync;

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
}

// Safety: Sim 含裸指针，跨线程共享需 unsafe（Phase 3+ 接入业务逻辑时审视）
unsafe impl Send for Sim {}
unsafe impl Sync for Sim {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

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
}
