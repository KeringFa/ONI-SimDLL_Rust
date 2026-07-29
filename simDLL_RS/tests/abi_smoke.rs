//! Phase 0 冒烟测试：模拟 C# P/Invoke 调用 9 个导出函数。
//!
//! 运行：cargo test --test abi_smoke

use SimDLL::sim_api::*;
use std::os::raw::{c_char, c_int, c_schar, c_void};

#[test]
fn sim_initialize_with_null_handler_smoke() {
    // 模拟 C# 在主菜单启动时调用 SIM_Initialize(null)
    // 应不崩、不返回（void）
    SIM_Initialize(None);
    // 二次调用应安全（幂等）
    SIM_Initialize(None);
}

#[test]
fn sim_handle_message_smoke() {
    // 模拟 C# 发送一条空消息
    let result: *mut c_void = SIM_HandleMessage(0, 0, std::ptr::null_mut());
    assert!(result.is_null(), "Phase 0 stub should return null");
}

#[test]
fn sim_handle_messages_smoke() {
    // 模拟 C# 批量发送
    let result = SIM_HandleMessages(0, 0, 0, std::ptr::null_mut());
    assert!(result.is_null());
}

#[test]
fn sim_begin_save_smoke() {
    // 模拟 C# 触发存档
    let mut size: c_int = 0xDEAD; // 初始化为哨兵值
    let result = SIM_BeginSave(&mut size as *mut c_int, 0, 0);
    assert!(result.is_null());
    assert_eq!(size, 0, "out_size should be set to 0 by stub");
}

#[test]
fn sim_end_save_smoke() {
    // 模拟 C# 结束存档
    SIM_EndSave();
}

#[test]
fn sim_shutdown_smoke() {
    // 模拟 C# 退出游戏
    SIM_Shutdown();
}

#[test]
fn sim_debug_crash_smoke() {
    // 验证桩不真正崩溃
    SIM_DebugCrash();
}

#[test]
fn sysinfo_acquire_returns_valid_c_string_smoke() {
    // 模拟 C# 读取 SYSINFO
    unsafe {
        let ptr: *mut c_char = SYSINFO_Acquire();
        assert!(!ptr.is_null());
        // 用 CStr 验证是合法的 NUL 终止字符串
        let cstr = std::ffi::CStr::from_ptr(ptr);
        let s = cstr.to_str().expect("should be valid UTF-8");
        assert!(s.contains("stub"));
        assert!(s.contains("windows"));
        assert!(s.contains("x86_64"));
    }
}

#[test]
fn sysinfo_release_smoke() {
    // 释放应安全
    SYSINFO_Release();
}

#[test]
fn full_lifecycle_smoke() {
    // 模拟游戏完整会话：启动 → 发消息 → 查 SYSINFO → 关闭
    SIM_Initialize(None);
    let _ = SIM_HandleMessage(42, 0, std::ptr::null_mut());
    let _ = SYSINFO_Acquire();
    SYSINFO_Release();
    SIM_Shutdown();
}
