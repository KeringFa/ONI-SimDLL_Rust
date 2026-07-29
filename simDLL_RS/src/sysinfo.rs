//! SYSINFO 实现。对应原版 `GetJSONSysInfo` + `SYSINFO_Acquire`/`Release`。
//! Phase 0 返回最小 stub JSON。

#![allow(dead_code)]

use std::os::raw::c_char;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::abi::SYSINFO_STUB_JSON;

/// 全局缓存：最近一次 `Acquire` 泄漏的字符串指针。
/// 对应原版 `QUERIED_SYSTEM_INFO` 全局变量。
static QUERIED_SYSINFO: AtomicUsize = AtomicUsize::new(0);

/// 对应原版 `SYSINFO_Acquire`。返回 NUL 终止的 C 字符串。
///
/// 内存策略：每次调用都 `leak()` 一段堆内存。
/// 不释放前一次的（原版 `operator_delete` 旧值，桩简化为不释放）。
/// 游戏会话内调用次数有限（主菜单 + 设置页打开），泄漏可忽略。
///
/// # Safety
/// 返回的指针指向堆内存，调用方不得 free。
pub unsafe fn acquire() -> *const c_char {
    // 复制 SYSINFO_STUB_JSON 到堆，泄漏之
    let boxed: Box<[u8]> = SYSINFO_STUB_JSON.into();
    let ptr = Box::into_raw(boxed) as *const u8 as *const c_char;
    // 记录到全局（覆盖旧值，旧值不释放——与原版简化语义一致）
    let old = QUERIED_SYSINFO.swap(ptr as usize, Ordering::SeqCst);
    let _ = old; // 不 free 旧值
    ptr
}

/// 对应原版 `SYSINFO_Release`。桩为 no-op。
/// 内存由进程退出回收。
pub unsafe fn release() {
    let _ = QUERIED_SYSINFO.swap(0, Ordering::SeqCst);
    // 不 free（原版用 operator_delete，Rust 端为简化省略）
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_returns_non_null_pointer() {
        unsafe {
            let ptr = acquire();
            assert!(!ptr.is_null());
            // 验证内容是 stub JSON
            let bytes = std::slice::from_raw_parts(ptr as *const u8, SYSINFO_STUB_JSON.len());
            assert_eq!(bytes, SYSINFO_STUB_JSON);
            // 最后一个字节是 NUL
            assert_eq!(*bytes.last().unwrap(), 0u8);
        }
    }

    #[test]
    fn release_is_safe_no_op() {
        unsafe {
            release();
            release(); // 多次调用安全
        }
    }
}
