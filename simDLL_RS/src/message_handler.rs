//! MessageType enum + MessageHandlerEntry — 消息 ID 与处理函数表。
//!
//! 字段对照源码：00_types_reference.c L9157-9225。
//! Phase 1 范围：定义 enum + MessageHandlerEntry struct + 空 handler 表。
//! Phase 6 才填充真实 handler。

#![allow(dead_code)]

use crate::buffer::BinaryBufferReader;
use std::ffi::c_void;

/// 62 个消息 ID（原版源码 00_types_reference.c L9157-9220）。
///
/// 注意：`#[repr(u32)]` 而非 `#[repr(C)]`，因为 62 个值中有 37 个超过 i32::MAX
/// （如 `MoveElementChunk=3920055938`），C 编译器实际选用 `unsigned int`（u32）
/// 作为 enum 底层类型。`#[repr(C)]` 默认 isize 在 x86_64 是 8 字节，会破坏
/// C ABI 兼容性与 size_of 断言。
#[allow(non_camel_case_types)]
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    AddDiseaseConsumer = 348345681,
    RadiationParamsModification = 377112707,
    ModifyElementEmitter = 403589164,
    RemoveDiseaseEmitter = 468135926,
    RemoveBuildingToBuildingHeatExchange = 697100730,
    MassEmission = 797274363,
    ModifyCellEnergy = 818320644,
    Disease_CreateTable = 825301935,
    Dig = 833038498,
    RemoveElementConsumer = 894417742,
    ModifyElementChunkEnergy = 1020555667,
    PrepareGameData = 1078620451,
    AllocateCells = 1092408308,
    Elements_CreateTable = 1108437482,
    SetSavedOptions = 1154135737,
    AddElementChunk = 1445724082,
    AddDiseaseEmitter = 1486783027,
    SetElementConsumerData = 1575539738,
    SetStrengthValue = 1593243982,
    MassConsumption = 1727657959,
    AddBuildingHeatExchange = 1739021608,
    ModifyBuildingHeatExchange = 1818001569,
    AddElementConsumer = 2024405073,
    SimData_InitializeFromCells = 2062421945,
    RemoveBuildingInContactFromBuildingToBuildingHeatExchange = 2301110083,
    CellRadiationModification = 2380089499,
    ModifyDiseaseEmitter = 2395843372,
    CellDiseaseModification = 2441296022,
    ClearUnoccupiedCells = 2458763021,
    ModifyDiseaseConsumer = 2471979672,
    SetDebugProperties = 2611848804,
    AddInContactBuildingToBuildingToBuildingHeatExchange = 2708242975,
    RemoveElementEmitter = 2770849014,
    AddRadiationEmitter = 2789071982,
    ModifyChunkTemperatureAdjuster = 2907365917,
    ModifyBuildingEnergy = 2946175638,
    AddBuildingToBuildingHeatExchange = 2956249079,
    ModifyCell = 3042046492,
    SimData_FreeCells = 3127174375,
    ConsumeDisease = 3275125760,
    Start = 3363520610,
    Elements_CreateInteractions = 3364677509,
    RemoveElementChunk = 3382058741,
    SetInsulationValue = 3396194175,
    DefineWorldOffsets = 3399120745,
    RemoveDiseaseConsumer = 3513325646,
    SimFrameManager_NewGameFrame = 3519640899,
    SimData_ResizeAndInitializeVacuumCells = 3542291143,
    RadiationSickness = 3567220694,
    RemoveRadiationEmitter = 3590707377,
    Load = 3622429126,
    ModifyBackwallData = 3689545781,
    SetVisibleCells = 3731910273,
    AddElementEmitter = 3789496115,
    ModifyRadiationEmitter = 3791001831,
    ChangeCellProperties = 3825655653,
    SetWorldZones = 3837658903,
    RemoveBuildingHeatExchange = 3838850667,
    ModifyCellWorldZone = 3845249282,
    SetElementChunkData = 3859851389,
    ToggleProfiler = 3885002365,
    MoveElementChunk = 3920055938,
}

/// 消息处理函数指针类型。
///
/// 对应原版 `void * (*handler)(struct BinaryBufferReader *)`（L9224）。
/// 与 `abi.rs::GameMessageHandler` 是不同签名：
/// - `GameMessageHandler`：`fn(c_int, *mut *mut c_void)`，C# → Rust 全局通知
/// - `MessageHandlerFn`：`fn(*mut BinaryBufferReader) -> *mut c_void`，单条消息处理
pub type MessageHandlerFn = unsafe extern "C" fn(*mut BinaryBufferReader) -> *mut c_void;

/// 消息处理表项（对照源码 L9222-9225）。
///
/// 源码 struct 实际仅 2 字段（id + handler），此处显式声明 `padding: u32`
/// 使隐式 padding 对读者可见（Rust `#[repr(C)]` 不会再额外插入 padding，
/// 因为显式 padding 已对齐到 8 字节边界，最终 struct 大小 16 字节与源码一致）。
#[repr(C)]
pub struct MessageHandlerEntry {
    pub id: MessageType,                     // offset 0, 4 字节
    pub padding: u32,                        // offset 4, 对齐到 8 字节边界
    pub handler: Option<MessageHandlerFn>,   // offset 8, 8 字节（Option<fn> null-optimized）
}

/// Phase 1：空表（所有 handler 都是 None）。
/// Phase 6 才填充真实 handler（源码 `gSimMessageHandlers` 实际条目数待 Phase 6 核实）。
pub static G_SIM_MESSAGE_HANDLERS: &[MessageHandlerEntry] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn message_type_size_is_4() {
        // #[repr(u32)] 显式指定底层 4 字节（37 个值超 i32::MAX，不能用 #[repr(C)] 默认 isize=8B）
        assert_eq!(size_of::<MessageType>(), 4);
    }

    #[test]
    fn message_handler_entry_size_is_16() {
        // 4 (id) + 4 (padding) + 8 (Option<fn>) = 16 字节
        assert_eq!(size_of::<MessageHandlerEntry>(), 16);
    }

    #[test]
    fn handlers_table_is_empty() {
        assert!(G_SIM_MESSAGE_HANDLERS.is_empty());
    }

    #[test]
    fn message_type_max_value_fits_u32() {
        // 验证最大值能正确表示（防止后续重构误用 #[repr(C)] 或 #[repr(i32)] 导致溢出）
        assert_eq!(MessageType::MoveElementChunk as u32, 3920055938);
    }
}
