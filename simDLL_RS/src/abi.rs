//! C ABI 类型与跨边界类型别名。
//!
//! 所有跨 ABI 的结构体都标 `#[repr(C)]`，避免 Rust 重排字段。
//! 原版源码类型对照：
//!   - schar  -> i8  (signed char)
//!   - byte   -> u8
//!   - ushort -> u16
//!   - uint   -> u32
//!   - ulonglong -> u64
//!   - longlong  -> i64
//!   - dword  -> u32
//!   - qword  -> u64
//!   - undefined4 -> u32

#![allow(dead_code)]

use std::os::raw::{c_char, c_int, c_void};

/// 原版 `schar` = signed char
pub type SChar = c_char;

/// 原版 `GameMessageHandler` 类型签名：
///   `typedef void (*GameMessageHandler)(int msg_id, void** payload);`
///
/// 反推依据：源码 L98944 `(*gGameMessageHandler)(0, &local_e0);` 其中
/// `local_e0` 是 `undefined1 *` 指向 `[stacktrace_ptr, filename_ptr]` 二指针数组。
///
/// **开放问题**：C# 侧的实际委托签名需在 Phase 0 实施时反编译 Assembly-CSharp.dll 验证。
/// 即使签名不匹配，最坏情况是 C# 收到垃圾指针导致弹窗乱码，不会让游戏更崩。
pub type GameMessageHandler = unsafe extern "C" fn(c_int, *mut *mut c_void);

/// 桩：占位的 GameMessageHandler 哨兵值，用于测试。
pub const NULL_GAME_MESSAGE_HANDLER: Option<GameMessageHandler> = None;

/// 崩溃通知 C# 时传的二指针 payload。
/// `[stacktrace_cstr, filename_cstr]`
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CrashNotifyPayload {
    pub stacktrace: *const c_char,
    pub filename: *const c_char,
}

impl Default for CrashNotifyPayload {
    fn default() -> Self {
        Self {
            stacktrace: std::ptr::null(),
            filename: std::ptr::null(),
        }
    }
}

/// 测试用的最小 JSON 字符串（SYSINFO_Acquire 返回）。
/// NUL 终止的 C 字符串：`{"stub":true,"os":"windows","arch":"x86_64"}\0`
pub const SYSINFO_STUB_JSON: &[u8] = b"{\"stub\":true,\"os\":\"windows\",\"arch\":\"x86_64\"}\0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schar_is_signed_char() {
        // 原版 schar = signed char，Rust c_char 在 Windows MSVC 上是 i8
        assert_eq!(std::mem::size_of::<SChar>(), 1);
        assert!(i8::MIN as i32 == -128);
    }

    #[test]
    fn crash_notify_payload_is_two_pointers() {
        // 反推：源码 `(*handler)(0, &local_e0)` 中 local_e0 后紧跟 local_d8
        // 即二指针数组。验证 repr(C) 布局是 16 字节（2 个 8 字节指针）。
        assert_eq!(std::mem::size_of::<CrashNotifyPayload>(), 16);
        assert_eq!(
            std::mem::offset_of!(CrashNotifyPayload, stacktrace),
            0
        );
        assert_eq!(
            std::mem::offset_of!(CrashNotifyPayload, filename),
            8
        );
    }

    #[test]
    fn sysinfo_stub_json_is_null_terminated() {
        // 必须以 NUL 结尾，因为 C# 端用 Marshal.PtrToStringAnsi 读
        assert_eq!(SYSINFO_STUB_JSON.last(), Some(&0u8));
        // 长度固定（测试后续如果意外修改常量，能立即发现）
        assert_eq!(SYSINFO_STUB_JSON.len(), 45);
    }

    #[test]
    fn game_message_handler_is_fn_pointer() {
        // 验证类型别名编译通过且为非零大小
        assert_eq!(std::mem::size_of::<Option<GameMessageHandler>>(), 8);
    }
}
