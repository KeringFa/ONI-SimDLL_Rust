//! Sim — 模拟主类。
//!
//! 字段对照源码 00_types_reference.c L6982-7052。
//! 内存布局：
//! ```text
//! 0x000 ┌──────────────────────────┐
//!       │ Thread 基类字段            │  80B（vtable + handle + running + ...）
//! 0x050 ├──────────────────────────┤
//!       │ SimFrameManager 子对象    │  288B
//! 0x170 ├──────────────────────────┤
//!       │ 其他字段（padding）       │  176B
//! 0x220 ├──────────────────────────┤
//!       │ struct FrameSync *frameSync │  8B
//! 0x228 └──────────────────────────┘
//! ```
//! C1 实现：Sim::new 初始化 SimFrameManager + start/join sim 线程。

use crate::a_framework::frame_sync::FrameSync;
use crate::a_framework::sim_frame_manager::SimFrameManager;

/// Sim — 模拟主类（继承自 Thread 类，0x228 = 552B）。
///
/// **布局**：
/// - reserved_head (80B): Thread 基类占位（vtable + handle + mExitRequested 等）
/// - frame_manager (288B): SimFrameManager 子对象，位于 offset 0x50
/// - reserved_tail (176B): 其他字段 padding，填充到 0x220
/// - frame_sync (8B): FrameSync back-pointer，位于 offset 0x220
#[repr(C)]
pub struct Sim {
    /// 0x00..0x50 (80B) —— Thread 基类占位。
    pub reserved_head: [u8; 0x50],
    /// 0x50..0x170 (288B) —— SimFrameManager 子对象。
    /// C1 阶段在 Sim::new 中通过 SimFrameManager::new_zeroed() 初始化。
    pub frame_manager: SimFrameManager,
    /// 0x170..0x220 (176B) —— 其他字段 padding。
    pub reserved_tail: [u8; 0xB0],
    /// 0x220..0x228 (8B) —— FrameSync back-pointer。
    pub frame_sync: *mut FrameSync,
}

impl Sim {
    /// 构造零初始化的 Sim（SimFrameManager 未初始化，current_frame 为 null）。
    pub fn new_zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }

    /// Sim::Sim(this, frameSync) — 构造 Sim 对象。
    /// 对照源码 SIM_Initialize L330-333: operator_new(0x228) + Sim::Sim(this, &gFrameSync)。
    ///
    /// 参数：
    /// - `frame_sync`：指向全局 gFrameSync 的指针（G_FRAME_SYNC 内部 FrameSync 的地址）
    ///
    /// C1 阶段：
    /// - 设置 frame_sync 字段
    /// - 初始化 SimFrameManager（位于 offset 0x50）：
    ///   SimFrameManager::new_zeroed() 分配初始 current_frame (SimFrameInfo)
    pub fn new(frame_sync: *mut FrameSync) -> Self {
        let mut sim = Self::new_zeroed();
        sim.frame_sync = frame_sync;
        // C1: 初始化 SimFrameManager（分配初始 current_frame）
        // SimFrameManager::new_zeroed() 内部调用 new_frame() 分配 SimFrameInfo
        sim.frame_manager = SimFrameManager::new_zeroed();
        tracing::debug!(
            "Sim::new created, frame_sync={:p}, current_frame={:p}",
            frame_sync, sim.frame_manager.current_frame
        );
        sim
    }

    /// Thread::start — 启动 sim 线程（C1 实现）。
    ///
    /// 委托给 sim_thread::start_sim_thread。
    pub fn start(&mut self) {
        let sim_ptr = self as *mut Sim;
        crate::a_framework::sim_thread::start_sim_thread(sim_ptr);
    }

    /// Thread::join — 等待 sim 线程退出（C1 实现）。
    ///
    /// 委托给 sim_thread::join_sim_thread。
    pub fn join(&mut self) {
        crate::a_framework::sim_thread::join_sim_thread();
    }

    /// Thread::joinable — 返回 sim 线程是否在运行（C1 实现）。
    pub fn joinable(&self) -> bool {
        if let Some(holder) = crate::a_framework::sim_thread::G_SIM_THREAD.get() {
            holder.lock().is_some()
        } else {
            false
        }
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        // C1: 清理 SimFrameManager 的 current_frame（防止内存泄漏）
        // SimFrameManager 本身没有 Drop impl，手动清理关键资源。
        let fm = &mut self.frame_manager;
        if !fm.current_frame.is_null() {
            unsafe {
                // 释放 current_frame (Box<SimFrameInfo>)
                let _ = Box::from_raw(fm.current_frame);
            }
            fm.current_frame = std::ptr::null_mut();
        }
        // 释放 frame_pool 中的帧
        let pool_slice = fm.frame_pool.as_slice();
        for &ptr in pool_slice {
            if !ptr.is_null() {
                unsafe { let _ = Box::from_raw(ptr); }
            }
        }
        // 释放 queued_frames 中的帧
        let queued_slice = fm.queued_frames.as_slice();
        for &ptr in queued_slice {
            if !ptr.is_null() {
                unsafe { let _ = Box::from_raw(ptr); }
            }
        }
        // 释放 active_frames 中的帧
        let active_slice = fm.active_frames.as_slice();
        for &ptr in active_slice {
            if !ptr.is_null() {
                unsafe { let _ = Box::from_raw(ptr); }
            }
        }
        // 释放 processed_frames 中的帧
        let processed_slice = fm.processed_frames.as_slice();
        for &ptr in processed_slice {
            if !ptr.is_null() {
                unsafe { let _ = Box::from_raw(ptr); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn sim_size_is_0x228() {
        assert_eq!(size_of::<Sim>(), 0x228, "Sim size should be 0x228 (552B)");
    }

    #[test]
    fn sim_new_zeroed() {
        let sim = Sim::new_zeroed();
        assert!(sim.frame_sync.is_null());
        assert!(sim.frame_manager.current_frame.is_null());
    }

    #[test]
    fn sim_new_initializes_frame_manager() {
        let sim = Sim::new(std::ptr::null_mut());
        // SimFrameManager 应已初始化（current_frame 非 null）
        assert!(
            !sim.frame_manager.current_frame.is_null(),
            "Sim::new should initialize SimFrameManager with a current_frame"
        );
        // frame_sync 应为传入的值（null）
        assert!(sim.frame_sync.is_null());
    }

    #[test]
    fn sim_drop_cleans_up_frame_manager() {
        let sim = Sim::new(std::ptr::null_mut());
        // Sim drop 时应清理 current_frame（不 panic）
        drop(sim);
    }

    #[test]
    fn sim_joinable_returns_false_without_start() {
        let sim = Sim::new(std::ptr::null_mut());
        assert!(!sim.joinable());
        drop(sim);
    }
}
