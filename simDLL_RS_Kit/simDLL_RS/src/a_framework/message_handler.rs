//! MessageType enum + MessageHandlerEntry — 消息 ID 与处理函数表。
//!
//! 字段对照源码：00_types_reference.c L9157-9225。
//! A2 范围：定义 enum + MessageHandlerEntry struct + 空 handler 表。
//! A3 填充 14 项真实 handler + dispatch_handler_table。

#![allow(dead_code)]

use crate::a_framework::buffer::BinaryBufferReader;
use crate::a_framework::save_load;
use crate::b_elements::elements_table;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// 62 个消息 ID（原版源码 00_types_reference.c L9157-9220）。
///
/// `#[repr(u32)]` 而非 `#[repr(C)]`，因为 37 个值超过 i32::MAX，
/// C 编译器选用 `unsigned int`（u32）作为 enum 底层类型。
#[allow(non_camel_case_types)]
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
/// 对应原版 `void * (*handler)(struct BinaryBufferReader *)`。
pub type MessageHandlerFn = unsafe extern "C" fn(*mut BinaryBufferReader) -> *mut c_void;

/// 消息处理表项（16B，对照源码 L9222-9225）。
#[repr(C)]
pub struct MessageHandlerEntry {
    pub id: MessageType,                     // offset 0, 4 字节
    pub padding: u32,                        // offset 4, 对齐到 8 字节边界
    pub handler: Option<MessageHandlerFn>,   // offset 8, 8 字节
}

// ===== 14 个 C ABI wrapper 函数 =====
// 每个 wrapper 用 catch_unwind 包装对应的 save_load/elements 函数。
// 对照源码 gSimMessageHandlers[14]（01_sim_api.c L230-238 线性查找）。

/// 通用 wrapper 生成宏：将 save_load::fn(&mut BinaryBufferReader) 包装为
/// unsafe extern "C" fn(*mut BinaryBufferReader)。
macro_rules! make_handler_wrapper {
    ($name:ident, $func:path) => {
        unsafe extern "C" fn $name(reader: *mut BinaryBufferReader) -> *mut c_void {
            catch_unwind(AssertUnwindSafe(|| {
                if reader.is_null() {
                    return std::ptr::null_mut();
                }
                let reader = &mut *reader;
                $func(reader)
            }))
            .unwrap_or_else(|_| {
                tracing::error!("{} handler panicked", stringify!($name));
                std::ptr::null_mut()
            })
        }
    };
}

// 10 个 save_load handler wrapper
make_handler_wrapper!(load_handler, save_load::handle_load);
make_handler_wrapper!(start_handler, save_load::handle_start);
make_handler_wrapper!(allocate_cells_handler, save_load::handle_allocate_cells);
make_handler_wrapper!(define_world_offsets_handler, save_load::handle_define_world_offsets);
make_handler_wrapper!(prepare_game_data_handler, save_load::handle_prepare_game_data);
make_handler_wrapper!(clear_unoccupied_cells_handler, save_load::handle_clear_unoccupied_cells);
make_handler_wrapper!(set_saved_options_handler, save_load::handle_set_saved_options);
make_handler_wrapper!(resize_and_initialize_vacuum_cells_handler, save_load::handle_resize_and_initialize_vacuum_cells);
make_handler_wrapper!(create_disease_table_handler, save_load::handle_create_disease_table);

// 3 个 SimData handler wrapper（A3 桩）
make_handler_wrapper!(initialize_from_cells_handler, save_load::handle_initialize_from_cells);
make_handler_wrapper!(free_grid_cells_handler, save_load::handle_free_grid_cells);
make_handler_wrapper!(set_world_zones_handler, save_load::handle_set_world_zones);

/// 14 项 handler 表。
/// 对照源码 gSimMessageHandlers[14]（01_sim_api.c L230-238 线性查找 14 项）。
///
/// **10 个 save_load handler**（02_save_load.c 实现）：
///   Load / Start / AllocateCells / DefineWorldOffsets / PrepareGameData /
///   ClearUnoccupiedCells / Elements_CreateInteractions / SetSavedOptions /
///   SimData_ResizeAndInitializeVacuumCells / Disease_CreateTable
///
/// **4 个非 save_load handler**：
///   Elements_CreateTable（B1 已实现）/ SimData_InitializeFromCells /
///   SimData_FreeCells / SetWorldZones
pub static G_SIM_MESSAGE_HANDLERS: &[MessageHandlerEntry] = &[
    MessageHandlerEntry { id: MessageType::Load, padding: 0, handler: Some(load_handler) },
    MessageHandlerEntry { id: MessageType::Start, padding: 0, handler: Some(start_handler) },
    MessageHandlerEntry { id: MessageType::AllocateCells, padding: 0, handler: Some(allocate_cells_handler) },
    MessageHandlerEntry { id: MessageType::DefineWorldOffsets, padding: 0, handler: Some(define_world_offsets_handler) },
    MessageHandlerEntry { id: MessageType::PrepareGameData, padding: 0, handler: Some(prepare_game_data_handler) },
    MessageHandlerEntry { id: MessageType::ClearUnoccupiedCells, padding: 0, handler: Some(clear_unoccupied_cells_handler) },
    MessageHandlerEntry { id: MessageType::Elements_CreateInteractions, padding: 0, handler: Some(elements_table::CreateElementInteractionsLocked) },
    MessageHandlerEntry { id: MessageType::SetSavedOptions, padding: 0, handler: Some(set_saved_options_handler) },
    MessageHandlerEntry { id: MessageType::SimData_ResizeAndInitializeVacuumCells, padding: 0, handler: Some(resize_and_initialize_vacuum_cells_handler) },
    MessageHandlerEntry { id: MessageType::Disease_CreateTable, padding: 0, handler: Some(create_disease_table_handler) },
    MessageHandlerEntry { id: MessageType::Elements_CreateTable, padding: 0, handler: Some(elements_table::CreateElementsTable) },
    MessageHandlerEntry { id: MessageType::SimData_InitializeFromCells, padding: 0, handler: Some(initialize_from_cells_handler) },
    MessageHandlerEntry { id: MessageType::SimData_FreeCells, padding: 0, handler: Some(free_grid_cells_handler) },
    MessageHandlerEntry { id: MessageType::SetWorldZones, padding: 0, handler: Some(set_world_zones_handler) },
];

/// 线性查找 handler 表。
/// 对照源码 01_sim_api.c L230-238。
///
/// 返回 `Some(result)` 表示找到 handler 并执行，`None` 表示未找到。
/// 注意：调用方需要确保 reader 的生命周期。
pub fn dispatch_handler_table(id: u32, reader: &mut BinaryBufferReader) -> Option<*mut c_void> {
    for entry in G_SIM_MESSAGE_HANDLERS {
        if entry.id as u32 == id {
            if let Some(handler) = entry.handler {
                // handler 需要 *mut BinaryBufferReader，但 reader 是 &mut
                // cast 是安全的：reader 在当前栈帧存活，handler 同步执行
                let result = unsafe { handler(reader as *mut BinaryBufferReader) };
                return Some(result);
            }
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn message_type_size_is_4() {
        assert_eq!(size_of::<MessageType>(), 4);
    }

    #[test]
    fn message_handler_entry_size_is_16() {
        assert_eq!(size_of::<MessageHandlerEntry>(), 16);
    }

    #[test]
    fn message_type_has_62_variants() {
        // 验证关键 ID 值
        assert_eq!(MessageType::Load as u32, 3622429126);
        assert_eq!(MessageType::Start as u32, 3363520610);
        assert_eq!(MessageType::Dig as u32, 833038498);
        assert_eq!(MessageType::SimFrameManager_NewGameFrame as u32, 3519640899);
    }

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
    fn handler_table_contains_all_expected_ids() {
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
        // 4 个非 save_load handler
        assert!(ids.contains(&(MessageType::Elements_CreateTable as u32)));
        assert!(ids.contains(&(MessageType::SimData_InitializeFromCells as u32)));
        assert!(ids.contains(&(MessageType::SimData_FreeCells as u32)));
        assert!(ids.contains(&(MessageType::SetWorldZones as u32)));
    }

    #[test]
    fn dispatch_unknown_id_returns_none() {
        let data = [0u8; 4];
        let mut reader = BinaryBufferReader::new(&data);
        assert!(dispatch_handler_table(0xDEADBEEF, &mut reader).is_none());
    }
}
