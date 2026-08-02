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
/// 对照源码 09_crashdump_logger.c 中的崩溃通知逻辑。
/// 2 个指针字段 = 16 字节。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CrashNotifyPayload {
    pub message: *const c_char,
    pub stack_trace: *const c_char,
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
    fn crash_notify_payload_size_is_16() {
        assert_eq!(std::mem::size_of::<CrashNotifyPayload>(), 16);
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
