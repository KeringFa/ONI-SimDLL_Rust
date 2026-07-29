//! `SimFrameManager` 桩。对应原版 `Sim::HandleMessage` 的 dispatch。
//! Phase 6 实现。

#![allow(dead_code)]

use std::os::raw::{c_int, c_schar};

/// 对应原版 `SimFrameManager`。Phase 0 仅占位。
pub struct SimFrameManager;

impl SimFrameManager {
    /// 对应原版 `SimFrameManager::HandleMessage(this, id, size, data, &out)`。
    /// Phase 0 永远返回 false（未处理）。
    pub fn handle_message(
        &mut self,
        _id: c_int,
        _size: c_int,
        _data: *const c_schar,
        _out: &mut *mut std::ffi::c_void,
    ) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_message_always_returns_false_in_stub() {
        let mut mgr = SimFrameManager;
        let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
        let result = mgr.handle_message(0, 0, std::ptr::null(), &mut out);
        assert!(!result);
    }
}
