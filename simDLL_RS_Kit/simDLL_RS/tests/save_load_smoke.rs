//! FFI 集成测试：存档加载/保存 smoke 测试。
//!
//! 模拟 C# 端的完整加载流程：
//! SIM_Initialize → AllocateCells → Load → Start → BeginSave → EndSave
//!
//! 对照源码 02_save_load.c 的 handler 链。

use SimDLL::a_framework::message_handler::MessageType;
use SimDLL::a_framework::sim_api::*;
use std::os::raw::{c_int, c_schar};

/// 串行化所有修改全局状态的测试，避免并行竞争。
/// 集成测试在同一个 binary 内共享全局状态（gSim/gSimData/gFrameSync 等）。
static SAVE_LOAD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 构造 AllocateCells 消息数据：width(i32) + height(i32) + flag1(bool) + flag2(bool)
/// 对照源码 02_save_load.c L259-293。
fn make_allocate_cells_data(width: i32, height: i32) -> Vec<u8> {
    let mut data = Vec::with_capacity(10);
    data.extend_from_slice(&width.to_le_bytes());
    data.extend_from_slice(&height.to_le_bytes());
    data.push(0u8); // flag1 = false (radiation_enabled)
    data.push(0u8); // flag2 = false (headless)
    data
}

/// 构造最小存档数据（version 15）。
///
/// 格式对照源码 02_save_load.c L7-207：
/// - 8B magic "SIMSAVE\0"
/// - 4B version (15 = 0xf)
/// - 4B save_width
/// - 4B save_height
/// - 4B offset (version > 13)
/// - 4B skip (version > 13)
/// - 1B flag (version > 12)
/// - cell 数据：save_width × save_height × 16B (element_hash + temp + mass + radiation)
/// - disease 数据：save_width × save_height × 8B (disease_hash + disease_count)
/// - backwall 数据：save_width × save_height × 12B (element_hash + mass + temp)
fn make_save_data(save_width: i32, save_height: i32) -> Vec<u8> {
    let total_cells = (save_width as usize) * (save_height as usize);
    let mut data = Vec::new();

    // Header (29 bytes)
    data.extend_from_slice(b"SIMSAVE\0");
    data.extend_from_slice(&15i32.to_le_bytes()); // version = 0xf
    data.extend_from_slice(&save_width.to_le_bytes());
    data.extend_from_slice(&save_height.to_le_bytes());
    data.extend_from_slice(&0i32.to_le_bytes()); // offset
    data.extend_from_slice(&0i32.to_le_bytes()); // skip
    data.push(0u8); // flag

    // Cell data: element_hash(u32) + temp(f32) + mass(f32) + radiation(f32)
    for _ in 0..total_cells {
        data.extend_from_slice(&0x2d39bf75u32.to_le_bytes()); // vacuum hash
        data.extend_from_slice(&300.0f32.to_le_bytes()); // temperature
        data.extend_from_slice(&100.0f32.to_le_bytes()); // mass
        data.extend_from_slice(&0.0f32.to_le_bytes()); // radiation
    }

    // Disease data: disease_hash(u32) + disease_count(i32)
    for _ in 0..total_cells {
        data.extend_from_slice(&0u32.to_le_bytes()); // no disease
        data.extend_from_slice(&0i32.to_le_bytes()); // count = 0
    }

    // Backwall data: element_hash(u32) + mass(f32) + temp(f32)
    for _ in 0..total_cells {
        data.extend_from_slice(&0x2d39bf75u32.to_le_bytes()); // vacuum hash
        data.extend_from_slice(&0.0f32.to_le_bytes()); // mass
        data.extend_from_slice(&0.0f32.to_le_bytes()); // temp
    }

    data
}

/// 发送 AllocateCells 消息并验证返回非 null。
fn do_allocate_cells(width: i32, height: i32) {
    let data = make_allocate_cells_data(width, height);
    let result = SIM_HandleMessage(
        MessageType::AllocateCells as c_int,
        data.len() as c_int,
        data.as_ptr() as *mut c_schar,
    );
    assert!(!result.is_null(), "AllocateCells({},{}) should return non-null", width, height);
}

#[test]
fn allocate_cells_returns_non_null_after_init() {
    let _guard = SAVE_LOAD_LOCK.lock().unwrap();
    SIM_Initialize(None);
    do_allocate_cells(4, 4);
}

#[test]
fn load_returns_non_null_after_allocate_cells() {
    let _guard = SAVE_LOAD_LOCK.lock().unwrap();
    SIM_Initialize(None);
    do_allocate_cells(4, 4);

    let save_data = make_save_data(4, 4);
    let result = SIM_HandleMessage(
        MessageType::Load as c_int,
        save_data.len() as c_int,
        save_data.as_ptr() as *mut c_schar,
    );
    assert!(!result.is_null(), "Load should return non-null");
}

#[test]
fn start_returns_non_null_after_load() {
    let _guard = SAVE_LOAD_LOCK.lock().unwrap();
    SIM_Initialize(None);
    do_allocate_cells(4, 4);

    let save_data = make_save_data(4, 4);
    let _ = SIM_HandleMessage(
        MessageType::Load as c_int,
        save_data.len() as c_int,
        save_data.as_ptr() as *mut c_schar,
    );

    // Start handler 不读取 reader 数据，传最小 buffer 即可
    let start_data = vec![0u8; 4];
    let result = SIM_HandleMessage(
        MessageType::Start as c_int,
        start_data.len() as c_int,
        start_data.as_ptr() as *mut c_schar,
    );
    assert!(!result.is_null(), "Start should return non-null GameDataUpdate pointer");

    // Start 消息会启动 sim 线程，必须 Shutdown join 线程，避免后续测试死锁
    SIM_Shutdown();
}

#[test]
fn begin_save_returns_correct_size_after_allocate() {
    let _guard = SAVE_LOAD_LOCK.lock().unwrap();
    SIM_Initialize(None);
    // AllocateCells(4, 4) → SimData(6, 6), total_cells = 36
    do_allocate_cells(4, 4);

    let mut size: c_int = 0;
    let result = SIM_BeginSave(&mut size, 0, 0);

    assert!(!result.is_null(), "BeginSave should return non-null pointer");
    // total_size = width × height × 0x24 + 0x1d = 6 × 6 × 36 + 29 = 1325
    assert_eq!(size, 1325, "BeginSave size should be 6×6×36+29 = 1325");

    SIM_EndSave();
}

#[test]
fn end_save_is_idempotent() {
    let _guard = SAVE_LOAD_LOCK.lock().unwrap();
    SIM_Initialize(None);
    do_allocate_cells(2, 2);

    let mut size: c_int = 0;
    let _ = SIM_BeginSave(&mut size, 0, 0);
    SIM_EndSave();
    // Double EndSave should be safe (no-op)
    SIM_EndSave();
}

#[test]
fn reinitialize_cleans_up_properly() {
    let _guard = SAVE_LOAD_LOCK.lock().unwrap();

    // First init + allocate
    SIM_Initialize(None);
    do_allocate_cells(2, 2);

    // Second init should clean up and not crash
    SIM_Initialize(None);

    // Should be able to allocate again with different dimensions
    do_allocate_cells(3, 3);
}

#[test]
fn load_with_small_dimensions_works() {
    let _guard = SAVE_LOAD_LOCK.lock().unwrap();
    SIM_Initialize(None);
    do_allocate_cells(2, 2);

    let save_data = make_save_data(2, 2);
    let result = SIM_HandleMessage(
        MessageType::Load as c_int,
        save_data.len() as c_int,
        save_data.as_ptr() as *mut c_schar,
    );
    assert!(!result.is_null(), "Load with 2×2 should return non-null");
}

#[test]
fn full_load_to_start_pipeline() {
    let _guard = SAVE_LOAD_LOCK.lock().unwrap();
    SIM_Initialize(None);
    do_allocate_cells(4, 4);

    // Load
    let save_data = make_save_data(4, 4);
    let load_result = SIM_HandleMessage(
        MessageType::Load as c_int,
        save_data.len() as c_int,
        save_data.as_ptr() as *mut c_schar,
    );
    assert!(!load_result.is_null(), "Load should succeed");

    // Start
    let start_data = vec![0u8; 4];
    let start_result = SIM_HandleMessage(
        MessageType::Start as c_int,
        start_data.len() as c_int,
        start_data.as_ptr() as *mut c_schar,
    );
    assert!(!start_result.is_null(), "Start should return GDU pointer");

    // BeginSave
    let mut size: c_int = 0;
    let save_result = SIM_BeginSave(&mut size, 0, 0);
    assert!(!save_result.is_null(), "BeginSave should succeed after Start");
    assert_eq!(size, 1325);
    SIM_EndSave();

    // Start 消息会启动 sim 线程，必须 Shutdown join 线程，避免后续测试死锁
    SIM_Shutdown();
}
