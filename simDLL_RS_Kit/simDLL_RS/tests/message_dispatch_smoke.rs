//! FFI 集成测试：消息分派 smoke 测试。
//!
//! 验证两阶段消息分派（对照源码 01_sim_api.c L208-244）：
//! 1. SimFrameManager::handle_message（A3 桩，返回 false）
//! 2. handler 表线性查找（14 项 gSimMessageHandlers）
//!
//! 以及 SIM_HandleMessage / SIM_HandleMessages 的路由行为。

use SimDLL::a_framework::buffer::BinaryBufferReader;
use SimDLL::a_framework::message_handler::{
    dispatch_handler_table, G_SIM_MESSAGE_HANDLERS, MessageType,
};
use SimDLL::a_framework::sim_api::*;
use std::os::raw::{c_int, c_schar};

/// 串行化所有修改全局状态的测试。
static DISPATCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ===== 静态表测试（不依赖全局状态，无需锁）=====

#[test]
fn handler_table_has_14_entries() {
    assert_eq!(G_SIM_MESSAGE_HANDLERS.len(), 14);
}

#[test]
fn all_14_handlers_are_non_null() {
    for entry in G_SIM_MESSAGE_HANDLERS {
        assert!(entry.handler.is_some(), "handler for {:?} is null", entry.id);
    }
}

#[test]
fn handler_table_contains_all_save_load_ids() {
    let ids: Vec<u32> = G_SIM_MESSAGE_HANDLERS.iter().map(|e| e.id as u32).collect();
    // 10 个 save_load handler
    assert!(ids.contains(&(MessageType::Load as u32)));
    assert!(ids.contains(&(MessageType::Start as u32)));
    assert!(ids.contains(&(MessageType::AllocateCells as u32)));
    assert!(ids.contains(&(MessageType::DefineWorldOffsets as u32)));
    assert!(ids.contains(&(MessageType::PrepareGameData as u32)));
    assert!(ids.contains(&(MessageType::ClearUnoccupiedCells as u32)));
    assert!(ids.contains(&(MessageType::Elements_CreateInteractions as u32)));
    assert!(ids.contains(&(MessageType::SetSavedOptions as u32)));
    assert!(ids.contains(&(MessageType::SimData_ResizeAndInitializeVacuumCells as u32)));
    assert!(ids.contains(&(MessageType::Disease_CreateTable as u32)));
}

#[test]
fn handler_table_contains_non_save_load_ids() {
    let ids: Vec<u32> = G_SIM_MESSAGE_HANDLERS.iter().map(|e| e.id as u32).collect();
    // 4 个非 save_load handler
    assert!(ids.contains(&(MessageType::Elements_CreateTable as u32)));
    assert!(ids.contains(&(MessageType::SimData_InitializeFromCells as u32)));
    assert!(ids.contains(&(MessageType::SimData_FreeCells as u32)));
    assert!(ids.contains(&(MessageType::SetWorldZones as u32)));
}

// ===== dispatch_handler_table 测试 =====

#[test]
fn dispatch_unknown_id_returns_none() {
    let data = [0u8; 4];
    let mut reader = BinaryBufferReader::new(&data);
    assert!(dispatch_handler_table(0xDEADBEEF, &mut reader).is_none());
}

#[test]
fn dispatch_known_id_returns_some() {
    let data = [0u8; 16];
    let mut reader = BinaryBufferReader::new(&data);
    // AllocateCells 是已知 ID，即使数据不完整也应找到 handler
    let result = dispatch_handler_table(MessageType::AllocateCells as u32, &mut reader);
    assert!(result.is_some(), "AllocateCells should be found in handler table");
}

// ===== SIM_HandleMessage 路由测试（依赖全局状态，需锁）=====

/// 构造 AllocateCells 消息数据
fn make_allocate_cells_data(width: i32, height: i32) -> Vec<u8> {
    let mut data = Vec::with_capacity(10);
    data.extend_from_slice(&width.to_le_bytes());
    data.extend_from_slice(&height.to_le_bytes());
    data.push(0u8);
    data.push(0u8);
    data
}

#[test]
fn sim_handle_message_unknown_id_returns_null_after_init() {
    let _guard = DISPATCH_LOCK.lock().unwrap();
    SIM_Initialize(None);

    let data = vec![0u8; 4];
    let result = SIM_HandleMessage(0xDEADBEEFu32 as c_int, 4, data.as_ptr() as *mut c_schar);
    assert!(result.is_null(), "Unknown message ID should return null");
}

#[test]
fn sim_handle_message_allocate_cells_returns_non_null() {
    let _guard = DISPATCH_LOCK.lock().unwrap();
    SIM_Initialize(None);

    let data = make_allocate_cells_data(4, 4);
    let result = SIM_HandleMessage(
        MessageType::AllocateCells as c_int,
        data.len() as c_int,
        data.as_ptr() as *mut c_schar,
    );
    assert!(!result.is_null(), "AllocateCells via SIM_HandleMessage should return non-null");
}

#[test]
fn sim_handle_messages_batch_returns_last_result() {
    let _guard = DISPATCH_LOCK.lock().unwrap();
    SIM_Initialize(None);

    // 2 条 AllocateCells 消息，每条 10 字节
    let msg_size = 10;
    let mut data = Vec::new();
    // Message 1: width=2, height=2
    data.extend_from_slice(&2i32.to_le_bytes());
    data.extend_from_slice(&2i32.to_le_bytes());
    data.push(0u8);
    data.push(0u8);
    // Message 2: width=3, height=3
    data.extend_from_slice(&3i32.to_le_bytes());
    data.extend_from_slice(&3i32.to_le_bytes());
    data.push(0u8);
    data.push(0u8);

    let result = SIM_HandleMessages(
        MessageType::AllocateCells as c_int,
        msg_size,
        2,
        data.as_ptr() as *mut c_schar,
    );
    assert!(!result.is_null(), "SIM_HandleMessages should return last message result");
}

#[test]
fn sim_handle_messages_zero_count_returns_null() {
    let _guard = DISPATCH_LOCK.lock().unwrap();
    SIM_Initialize(None);

    let data = vec![0u8; 4];
    let result = SIM_HandleMessages(
        MessageType::AllocateCells as c_int,
        4,
        0,
        data.as_ptr() as *mut c_schar,
    );
    assert!(result.is_null(), "SIM_HandleMessages with count=0 should return null");
}

#[test]
fn sim_handle_message_null_data_is_dispatched_safely() {
    // 原版 01_sim_api.c L225 不检查 data=null/size<=0，直接构造 reader。
    // C# 的 Sim.Start() 传入 size=0, data=null（Start 消息不需要 reader 数据），
    // 原版用空 reader 继续分派，handler 自行决定是否读取。
    //
    // 此测试验证 null data 不导致崩溃，且消息正常分派到 handler 表。
    let _guard = DISPATCH_LOCK.lock().unwrap();
    SIM_Initialize(None);

    // null data + size=0：应安全分派（不崩溃），AllocateCells 返回非 null
    let result = SIM_HandleMessage(MessageType::AllocateCells as c_int, 0, std::ptr::null_mut());
    assert!(!result.is_null(), "AllocateCells with null data should still be dispatched and return non-null");

    // null data + size>0：同样应安全分派（空 reader 读默认值）
    let result2 = SIM_HandleMessage(MessageType::AllocateCells as c_int, 10, std::ptr::null_mut());
    assert!(!result2.is_null(), "AllocateCells with null data + size>0 should still be dispatched safely");
}

#[test]
fn two_phase_dispatch_routes_to_handler_table() {
    // 验证两阶段分派：SimFrameManager 返回 false → handler 表处理
    // AllocateCells 不在 SimFrameManager 的 45 个 arm 中（它属于 handler 表）
    let _guard = DISPATCH_LOCK.lock().unwrap();
    SIM_Initialize(None);

    let data = make_allocate_cells_data(2, 2);
    let result = SIM_HandleMessage(
        MessageType::AllocateCells as c_int,
        data.len() as c_int,
        data.as_ptr() as *mut c_schar,
    );
    // 如果两阶段分派正常，AllocateCells 会被 handler 表处理并返回非 null
    assert!(!result.is_null(), "Two-phase dispatch should route AllocateCells to handler table");
}
