//! `Sim` 类桩。对应原版 `Sim::Sim(Sim*, FrameSync*)`。
//! Phase 1 实现。

#![allow(dead_code)]

use crate::frame_sync::FrameSync;

/// 对应原版 `Sim` 类（构造时占 0x228 字节）。
/// Phase 0 不分配任何字段，只提供构造/析构。
pub struct Sim {
    _frame_sync: *mut FrameSync,
}

impl Sim {
    /// 对应原版 `Sim::Sim(this, &gFrameSync)`。
    /// # Safety
    /// 调用方需保证 frame_sync 指针在 Sim 存活期间有效。
    pub unsafe fn new(frame_sync: *mut FrameSync) -> Self {
        Self { _frame_sync: frame_sync }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_can_be_constructed() {
        let mut fs = FrameSync::default();
        let _sim = unsafe { Sim::new(&mut fs as *mut _) };
    }
}
