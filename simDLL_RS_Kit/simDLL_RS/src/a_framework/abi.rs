//! C ABI 类型定义
//!
//! 对照源码 01_sim_api.c 和 09_crashdump_logger.c 中的类型。

use std::os::raw::{c_char, c_int, c_void};

/// 原版 schar 类型（signed char，1 字节）。
pub type SChar = c_char;

/// C# 注册的消息回调函数指针类型。
///
/// 对照源码 01_sim_api.c L98515 SIM_Initialize 的参数类型。
/// 原版签名：`void (*)(int, void**)`（消息 ID + 返回数据指针的指针）。
pub type GameMessageHandler = unsafe extern "C" fn(c_int, *mut *mut c_void);

/// 崩溃通知 payload（传递给 GameMessageHandler）。
///
/// 对照源码 09_crashdump_logger.c L186-210/L430-443 与 C# Sim.cs
/// `DLLReportMessageMessage { IntPtr callstack; IntPtr message; IntPtr file; int line; }`：
/// 4 字段 28B，对齐到 32B。此前 {message, stack_trace} 16B 字段错位 → C# 读
/// callstack@0/message@8/file@16/line@24 时内容对调且越界读 16B。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CrashNotifyPayload {
    pub callstack: *const c_char, // @0
    pub message: *const c_char,   // @8
    pub file: *const c_char,      // @16
    pub line: c_int,              // @24
}

/// SYSINFO_Acquire 返回的最小 JSON（NUL 终止）。
///
/// 对照源码 01_sim_api.c SYSINFO_Acquire 的返回值。
/// A1 阶段返回最小桩 JSON，后续阶段可扩展。
pub const SYSINFO_STUB_JSON: &[u8] = b"{\"version\":\"simDLL_RS v2 A1\"}\0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schar_is_signed_char() {
        assert_eq!(std::mem::size_of::<SChar>(), 1);
    }

    #[test]
    fn crash_notify_payload_size_is_32() {
        // C# DLLReportMessageMessage：3×IntPtr + int，对齐后 32B
        assert_eq!(std::mem::size_of::<CrashNotifyPayload>(), 32);
    }

    #[test]
    fn sysinfo_stub_json_ends_with_nul() {
        assert_eq!(*SYSINFO_STUB_JSON.last().unwrap(), 0);
    }

    #[test]
    fn sysinfo_stub_json_is_valid_utf8() {
        // 去掉 NUL 终止符后应是有效 UTF-8
        let json = &SYSINFO_STUB_JSON[..SYSINFO_STUB_JSON.len() - 1];
        std::str::from_utf8(json).expect("should be valid UTF-8");
    }

    #[test]
    fn game_message_handler_is_function_pointer() {
        // 验证 GameMessageHandler 是函数指针（8 字节）
        assert_eq!(std::mem::size_of::<Option<GameMessageHandler>>(), 8);
    }
}
