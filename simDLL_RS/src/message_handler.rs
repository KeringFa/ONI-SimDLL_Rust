//! `MessageHandlerEntry` 表桩。对应原版 `gSimMessageHandlers` 数组（14 个条目）。
//! Phase 6 实现真实 handler 注册。

#![allow(dead_code)]

use std::os::raw::{c_int, c_void};

/// 对应原版 `MessageHandlerEntry { id, handler }`。
#[repr(C)]
pub struct MessageHandlerEntry {
    pub id: c_int,
    pub handler: unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut c_void,
}

/// Phase 0：空表。原版 14 个条目在 Phase 6 填充。
pub static G_SIM_MESSAGE_HANDLERS: &[MessageHandlerEntry] = &[];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_table_is_empty_in_stub() {
        assert!(G_SIM_MESSAGE_HANDLERS.is_empty());
    }
}
