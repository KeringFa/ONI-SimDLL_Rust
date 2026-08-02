//! 系统信息桩
//!
//! 对照源码 01_sim_api.c SYSINFO_Acquire/Release。
//! A1 阶段返回最小 JSON，不收集真实系统信息。

use std::os::raw::c_char;
use super::abi::SYSINFO_STUB_JSON;

/// 获取系统信息 JSON 字符串。
///
/// 返回 SYSINFO_STUB_JSON 的裸指针。调用方通过 SYSINFO_Release 释放。
/// 对照源码 SYSINFO_Acquire，A1 阶段返回桩 JSON。
pub fn acquire() -> *mut c_char {
    // 分配堆内存并拷贝桩 JSON
    let json = SYSINFO_STUB_JSON;
    unsafe {
        let layout = std::alloc::Layout::from_size_align(json.len(), 1).unwrap();
        let ptr = std::alloc::alloc(layout) as *mut c_char;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(json.as_ptr() as *const c_char, ptr, json.len());
        ptr
    }
}

/// 释放系统信息字符串。
///
/// 对照源码 SYSINFO_Release，A1 阶段释放 acquire 分配的内存。
///
/// # Safety
/// ptr 必须是 acquire 返回的指针。
pub unsafe fn release(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    // 使用 SYSINFO_STUB_JSON 的长度（含 NUL）
    let layout = std::alloc::Layout::from_size_align(SYSINFO_STUB_JSON.len(), 1).unwrap();
    std::alloc::dealloc(ptr as *mut u8, layout);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_returns_non_null() {
        let ptr = acquire();
        assert!(!ptr.is_null());
        unsafe { release(ptr); }
    }

    #[test]
    fn release_null_is_safe_no_op() {
        unsafe { release(std::ptr::null_mut()); }
    }
}
