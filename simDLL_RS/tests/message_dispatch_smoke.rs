//! Phase 6 集成测试：消息分派端到端 FFI 验证。
//!
//! 通过 C ABI 的 `SIM_HandleMessage` / `SIM_HandleMessages` 入口验证两阶段分派路径：
//! - 3 个特殊处理 ID（HandleMessage 路由）通过 FFI 返回 null
//! - 42 个普通 emplace ID（HandleMessage 路由）通过 FFI 返回 null
//! - 9 个 save_load ID 通过 FFI 返回 null（不回归 Phase 3）
//! - 5 个非 save_load 桩 ID 通过 FFI 返回 null
//! - 未知 ID（id=0）通过 FFI 返回 null
//! - 批量消息（count=0/2/3）通过 FFI 返回 null
//!
//! 这些测试不验证日志输出（tracing subscriber 在测试中未配置），
//! 仅验证返回值 `is_null()` 与不 panic。日志验证留给部署后人工检查（spec §5）。
//!
//! 运行：cargo test --test message_dispatch_smoke

use SimDLL::message_handler::MessageType;
use SimDLL::sim_api::{SIM_HandleMessage, SIM_HandleMessages};
use std::os::raw::{c_int, c_schar, c_void};

/// 辅助：把 `MessageType` 转为 `c_int`（bit-pattern 保留，u32 → i32）。
///
/// 许多 enum 值超过 `i32::MAX`（如 `MoveElementChunk=3920055938`），
/// 必须用 `as u32 as c_int` 做位模式保留转换，否则直接 `as c_int` 会溢出。
fn id(mt: MessageType) -> c_int {
    mt as u32 as c_int
}

#[test]
fn handle_message_special_ids_return_null_via_ffi() {
    // 3 个特殊处理 ID（SimFrameManager_NewGameFrame / SetDebugProperties / ModifyBuildingEnergy）
    // 命中 HandleMessage 阶段 1 路由（log + return true），SIM_HandleMessage 返回 null。
    let specials = [
        MessageType::SimFrameManager_NewGameFrame,
        MessageType::SetDebugProperties,
        MessageType::ModifyBuildingEnergy,
    ];
    for mt in specials {
        let result: *mut c_void = SIM_HandleMessage(id(mt), 0, std::ptr::null_mut());
        assert!(
            result.is_null(),
            "special id {:?} should return null via FFI",
            mt
        );
    }
}

#[test]
fn handle_message_emplace_ids_return_null_via_ffi() {
    // 全部 42 个普通 emplace 路由 ID（T1 实际是 42 个，非简报的 41 个）。
    // 列表来源：sim_frame_manager.rs 中 T1 实现的 const 常量列表（排除 3 个特殊 ID）。
    // 每个 emplace ID 命中 HandleMessage 阶段 1 路由（log + return true），返回 null。
    let emplace_ids = [
        MessageType::ChangeCellProperties,
        MessageType::AddBuildingToBuildingHeatExchange,
        MessageType::RemoveBuildingInContactFromBuildingToBuildingHeatExchange,
        MessageType::CellRadiationModification,
        MessageType::ModifyDiseaseEmitter,
        MessageType::CellDiseaseModification,
        MessageType::ModifyDiseaseConsumer,
        MessageType::AddInContactBuildingToBuildingToBuildingHeatExchange,
        MessageType::RemoveElementEmitter,
        MessageType::AddRadiationEmitter,
        MessageType::ModifyChunkTemperatureAdjuster,
        MessageType::ModifyCell,
        MessageType::ConsumeDisease,
        MessageType::RemoveElementChunk,
        MessageType::SetInsulationValue,
        MessageType::RemoveDiseaseConsumer,
        MessageType::RemoveRadiationEmitter,
        MessageType::ModifyBackwallData,
        MessageType::AddElementEmitter,
        MessageType::ModifyRadiationEmitter,
        MessageType::RemoveBuildingHeatExchange,
        MessageType::ModifyCellWorldZone,
        MessageType::SetElementChunkData,
        MessageType::MoveElementChunk,
        MessageType::AddDiseaseConsumer,
        MessageType::ModifyElementEmitter,
        MessageType::RemoveDiseaseEmitter,
        MessageType::RemoveBuildingToBuildingHeatExchange,
        MessageType::MassEmission,
        MessageType::ModifyCellEnergy,
        MessageType::Dig,
        MessageType::RadiationParamsModification,
        MessageType::SetStrengthValue,
        MessageType::MassConsumption,
        MessageType::AddBuildingHeatExchange,
        MessageType::ModifyBuildingHeatExchange,
        MessageType::AddElementConsumer,
        MessageType::AddElementChunk,
        MessageType::AddDiseaseEmitter,
        MessageType::SetElementConsumerData,
        MessageType::ModifyElementChunkEnergy,
        MessageType::RemoveElementConsumer,
    ];
    assert_eq!(
        emplace_ids.len(),
        42,
        "普通 emplace 实际 42 项（T1 逐项核对源码后确认，含原计划遗漏的 ModifyElementChunkEnergy）"
    );
    for mt in emplace_ids {
        let result: *mut c_void = SIM_HandleMessage(id(mt), 0, std::ptr::null_mut::<c_schar>());
        assert!(
            result.is_null(),
            "emplace id {:?} should return null via FFI",
            mt
        );
    }
}

#[test]
fn handle_message_save_load_ids_return_null_via_ffi() {
    // 9 个 save_load ID 不在 HandleMessage 路由表（阶段 1 返回 false），
    // 回退到 gSimMessageHandlers 表（阶段 2）命中 save_load_shim。
    // 空 buffer（null data → empty slice）让 save_load handler 返回 Err，
    // shim 转 null（行为与 Phase 3 一致，验证升级不破坏 save_load 路径）。
    let save_load_ids = [
        MessageType::Load,
        MessageType::Start,
        MessageType::AllocateCells,
        MessageType::DefineWorldOffsets,
        MessageType::PrepareGameData,
        MessageType::ClearUnoccupiedCells,
        MessageType::SetSavedOptions,
        MessageType::Disease_CreateTable,
        MessageType::SimData_ResizeAndInitializeVacuumCells,
    ];
    for mt in save_load_ids {
        let result: *mut c_void = SIM_HandleMessage(id(mt), 0, std::ptr::null_mut::<c_schar>());
        assert!(
            result.is_null(),
            "save_load id {:?} should return null via FFI",
            mt
        );
    }
}

#[test]
fn handle_message_non_save_load_stub_ids_return_null_via_ffi() {
    // 5 个非 save_load 桩 ID 不在 HandleMessage 路由表（阶段 1 返回 false），
    // 回退到 gSimMessageHandlers 表（阶段 2）命中 non_save_load stub handler。
    // 桩 handler 只 log + 返回 null（不读取 reader，不修改全局状态）。
    let stub_ids = [
        MessageType::SimData_InitializeFromCells,
        MessageType::SimData_FreeCells,
        MessageType::SetVisibleCells,
        MessageType::SetWorldZones,
        MessageType::RadiationSickness,
    ];
    for mt in stub_ids {
        let result: *mut c_void = SIM_HandleMessage(id(mt), 0, std::ptr::null_mut::<c_schar>());
        assert!(
            result.is_null(),
            "non_save_load stub id {:?} should return null via FFI",
            mt
        );
    }
}

#[test]
fn handle_message_unknown_id_returns_null_via_ffi() {
    // id=0 不在任何路由表（阶段 1 返回 false，阶段 2 表查找无匹配），
    // SIM_HandleMessage 返回 null。
    let result: *mut c_void = SIM_HandleMessage(0, 0, std::ptr::null_mut::<c_schar>());
    assert!(result.is_null(), "unknown id=0 should return null via FFI");
}

#[test]
fn handle_messages_count_zero_returns_null_via_ffi() {
    // count=0 直接返回 null（与原版 L266 `if (0 < param_3)` 一致，循环不执行）。
    let result: *mut c_void =
        SIM_HandleMessages(0, 0, 0, std::ptr::null_mut::<c_schar>());
    assert!(
        result.is_null(),
        "count=0 should return null via FFI (loop not executed)"
    );
}

#[test]
fn handle_messages_count_two_returns_null_via_ffi() {
    // count=2 + NewGameFrame id：每次都命中 HandleMessage 阶段 1 路由 → return null。
    // 验证批量消息循环不 panic，返回 null（最后一次调用的结果）。
    let id_v = id(MessageType::SimFrameManager_NewGameFrame);
    let result: *mut c_void =
        SIM_HandleMessages(id_v, 0, 2, std::ptr::null_mut::<c_schar>());
    assert!(
        result.is_null(),
        "count=2 with NewGameFrame id should return null via FFI"
    );
}

#[test]
fn handle_messages_count_three_returns_null_via_ffi() {
    // count=3 + Load id：每次都不命中 HandleMessage（阶段 1 返回 false），
    // 回退到阶段 2 命中 save_load shim → empty buffer Err → null。
    // 验证批量消息走阶段 2 分派不 panic，返回 null。
    let id_v = id(MessageType::Load);
    let result: *mut c_void =
        SIM_HandleMessages(id_v, 0, 3, std::ptr::null_mut::<c_schar>());
    assert!(
        result.is_null(),
        "count=3 with Load id should return null via FFI (stage 2 dispatch)"
    );
}
