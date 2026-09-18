//! FFI 集成测试：模拟 C# P/Invoke 调用 9 个导出函数。

use SimDLL::a_framework::sim_api::*;
use std::os::raw::{c_char, c_int, c_schar, c_void};

#[test]
fn sim_initialize_with_null_does_not_crash() {
    SIM_Initialize(None);
}

#[test]
fn sim_shutdown_does_not_crash() {
    SIM_Shutdown();
}

#[test]
fn sim_handle_message_null_returns_null() {
    let result = SIM_HandleMessage(0, 0, std::ptr::null_mut());
    assert!(result.is_null());
}

#[test]
fn sim_handle_messages_null_returns_null() {
    let result = SIM_HandleMessages(0, 0, 0, std::ptr::null_mut());
    assert!(result.is_null());
}

#[test]
fn sim_begin_save_returns_null_and_zero_size() {
    let mut size: c_int = -1;
    let result = SIM_BeginSave(&mut size, 0, 0);
    assert!(result.is_null());
    assert_eq!(size, 0);
}

#[test]
fn sim_begin_save_null_out_size_does_not_crash() {
    let result = SIM_BeginSave(std::ptr::null_mut(), 0, 0);
    assert!(result.is_null());
}

#[test]
fn sim_end_save_does_not_crash() {
    SIM_EndSave();
}

#[test]
fn sim_debug_crash_does_not_crash() {
    SIM_DebugCrash();
}

#[test]
fn sysinfo_acquire_returns_non_null() {
    let ptr = SYSINFO_Acquire();
    assert!(!ptr.is_null());
    SYSINFO_Release();
}

#[test]
fn sysinfo_release_null_is_safe() {
    SYSINFO_Release();
}

#[test]
fn sim_handle_message_with_data_returns_null() {
    // 构造一个简单的数据 buffer
    let data: Vec<u8> = vec![1, 2, 3, 4];
    let result = SIM_HandleMessage(42, 4, data.as_ptr() as *mut c_schar);
    assert!(result.is_null());
}

#[test]
fn binary_buffer_round_trip_integration() {
    use SimDLL::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
    let mut w = BinaryBufferWriter::new();
    w.write_int(42);
    w.write_float(3.14);
    w.write_string("test");
    let bytes = w.into_bytes();
    let mut r = BinaryBufferReader::new(&bytes);
    assert_eq!(r.read_int().unwrap(), 42);
    assert_eq!(r.read_float().unwrap(), 3.14);
    assert_eq!(r.read_string().unwrap(), "test");
    assert!(r.is_at_end());
}
