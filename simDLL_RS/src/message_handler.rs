//! MessageType enum + MessageHandlerEntry — 消息 ID 与处理函数表。
//!
//! 字段对照源码：00_types_reference.c L9157-9225。
//! Phase 1 范围：定义 enum + MessageHandlerEntry struct + 空 handler 表。
//! Phase 6 才填充真实 handler。

#![allow(dead_code)]

use crate::buffer::{BinaryBufferReader, BufferError};
use crate::save_load;
use std::ffi::c_void;

/// 62 个消息 ID（原版源码 00_types_reference.c L9157-9220）。
///
/// 注意：`#[repr(u32)]` 而非 `#[repr(C)]`，因为 62 个值中有 37 个超过 i32::MAX
/// （如 `MoveElementChunk=3920055938`），C 编译器实际选用 `unsigned int`（u32）
/// 作为 enum 底层类型。`#[repr(C)]` 默认 isize 在 x86_64 是 8 字节，会破坏
/// C ABI 兼容性与 size_of 断言。
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

/// Phase 6：14 项 handler 表（9 save_load 真实 + 5 非 save_load 桩）。
///
/// 对照源码 `01_sim_api.c` L230-238 的 `gSimMessageHandlers[14]` 表（Ghidra 未反编译
/// `.rdata` 静态初始化代码，无法从源码精确确认 5 个非 save_load 项的 ID 与函数）。
///
/// **9 个 save_load 真实 handler**：调用 `save_load::*` 函数（Phase 3 已实现）。
///
/// **5 个非 save_load 桩 handler**：候选 ID 基于 MessageType enum 排除法
/// （未归入 HandleMessage 路由的 43 项 + 未归入 9 个 save_load 的剩余 5 个）。
/// 桩函数仅 tracing log + 返回 null。用户部署后通过日志验证候选是否正确
/// （见 spec §4）。若日志显示某候选 ID 实际未被调用，或某消息 ID 未命中任何
/// handler，需调整候选列表。
///
/// **表项顺序**：与原版表项顺序无关（线性查找，原版也线性查找）。Rust 按可读性
/// 分组（9 save_load 在前，5 桩在后）。
pub static G_SIM_MESSAGE_HANDLERS: &[MessageHandlerEntry] = &[
    // === 9 个 save_load 真实 handler（Phase 3 实现，这里只注册函数指针）===
    MessageHandlerEntry {
        id: MessageType::Load,
        padding: 0,
        handler: Some(save_load_shim::load),
    },
    MessageHandlerEntry {
        id: MessageType::Start,
        padding: 0,
        handler: Some(save_load_shim::start),
    },
    MessageHandlerEntry {
        id: MessageType::AllocateCells,
        padding: 0,
        handler: Some(save_load_shim::allocate_cells),
    },
    MessageHandlerEntry {
        id: MessageType::DefineWorldOffsets,
        padding: 0,
        handler: Some(save_load_shim::define_world_offsets),
    },
    MessageHandlerEntry {
        id: MessageType::PrepareGameData,
        padding: 0,
        handler: Some(save_load_shim::prepare_game_data),
    },
    MessageHandlerEntry {
        id: MessageType::ClearUnoccupiedCells,
        padding: 0,
        handler: Some(save_load_shim::clear_unoccupied_cells),
    },
    MessageHandlerEntry {
        id: MessageType::SetSavedOptions,
        padding: 0,
        handler: Some(save_load_shim::set_saved_options),
    },
    MessageHandlerEntry {
        id: MessageType::Disease_CreateTable,
        padding: 0,
        handler: Some(save_load_shim::create_disease_table),
    },
    MessageHandlerEntry {
        id: MessageType::SimData_ResizeAndInitializeVacuumCells,
        padding: 0,
        handler: Some(save_load_shim::resize_and_initialize_vacuum_cells),
    },
    // === 5 个非 save_load 桩 handler（候选列表，部署后日志验证）===
    MessageHandlerEntry {
        id: MessageType::SimData_InitializeFromCells,
        padding: 0,
        handler: Some(non_save_load_handlers::sim_data_initialize_from_cells),
    },
    MessageHandlerEntry {
        id: MessageType::SimData_FreeCells,
        padding: 0,
        handler: Some(non_save_load_handlers::sim_data_free_cells),
    },
    MessageHandlerEntry {
        id: MessageType::SetVisibleCells,
        padding: 0,
        handler: Some(non_save_load_handlers::set_visible_cells),
    },
    MessageHandlerEntry {
        id: MessageType::SetWorldZones,
        padding: 0,
        handler: Some(non_save_load_handlers::set_world_zones),
    },
    MessageHandlerEntry {
        id: MessageType::RadiationSickness,
        padding: 0,
        handler: Some(non_save_load_handlers::radiation_sickness),
    },
];

/// `save_load_shim` —— save_load handler 函数的 ABI 适配层。
///
/// `save_load::*` 函数签名是 `fn(&mut BinaryBufferReader) -> Result<(), BufferError>`，
/// 但 `gSimMessageHandlers` 表需要 `unsafe extern "C" fn(*mut BinaryBufferReader) -> *mut c_void`
/// （`MessageHandlerFn` 类型）。shim 函数调用 save_load 函数、转换结果为指针、
/// 用 `catch_unwind` 隔离 panic（不跨 C ABI 边界）。
mod save_load_shim {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// Load 消息 handler shim。
    pub unsafe extern "C" fn load(reader: *mut BinaryBufferReader) -> *mut c_void {
        run_save_load(reader, save_load::load, "Load")
    }

    /// Start 消息 handler shim。
    pub unsafe extern "C" fn start(reader: *mut BinaryBufferReader) -> *mut c_void {
        run_save_load(reader, save_load::start, "Start")
    }

    /// AllocateCells 消息 handler shim。
    pub unsafe extern "C" fn allocate_cells(reader: *mut BinaryBufferReader) -> *mut c_void {
        run_save_load(reader, save_load::allocate_cells, "AllocateCells")
    }

    /// DefineWorldOffsets 消息 handler shim。
    pub unsafe extern "C" fn define_world_offsets(reader: *mut BinaryBufferReader) -> *mut c_void {
        run_save_load(reader, save_load::define_world_offsets, "DefineWorldOffsets")
    }

    /// PrepareGameData 消息 handler shim。
    pub unsafe extern "C" fn prepare_game_data(reader: *mut BinaryBufferReader) -> *mut c_void {
        run_save_load(reader, save_load::prepare_game_data, "PrepareGameData")
    }

    /// ClearUnoccupiedCells 消息 handler shim。
    pub unsafe extern "C" fn clear_unoccupied_cells(reader: *mut BinaryBufferReader) -> *mut c_void {
        run_save_load(reader, save_load::clear_unoccupied_cells, "ClearUnoccupiedCells")
    }

    /// SetSavedOptions 消息 handler shim。
    pub unsafe extern "C" fn set_saved_options(reader: *mut BinaryBufferReader) -> *mut c_void {
        run_save_load(reader, save_load::set_saved_options, "SetSavedOptions")
    }

    /// Disease_CreateTable 消息 handler shim。
    pub unsafe extern "C" fn create_disease_table(reader: *mut BinaryBufferReader) -> *mut c_void {
        run_save_load(reader, save_load::create_disease_table, "Disease_CreateTable")
    }

    /// SimData_ResizeAndInitializeVacuumCells 消息 handler shim。
    pub unsafe extern "C" fn resize_and_initialize_vacuum_cells(
        reader: *mut BinaryBufferReader,
    ) -> *mut c_void {
        run_save_load(
            reader,
            save_load::resize_and_initialize_vacuum_cells,
            "SimData_ResizeAndInitializeVacuumCells",
        )
    }

    /// 通用 save_load handler 调用器：构造引用、调 save_load 函数、转结果为指针。
    ///
    /// # Safety
    /// `reader` 必须是有效的 `BinaryBufferReader` 指针（C# FFI 契约默认满足）。
    unsafe fn run_save_load(
        reader: *mut BinaryBufferReader,
        f: fn(&mut BinaryBufferReader) -> Result<(), BufferError>,
        name: &'static str,
    ) -> *mut c_void {
        let result = catch_unwind(AssertUnwindSafe(|| {
            if reader.is_null() {
                tracing::error!(handler = name, "save_load handler: null reader");
                return std::ptr::null_mut();
            }
            match f(&mut *reader) {
                Ok(()) => std::ptr::null_mut(),
                Err(e) => {
                    tracing::error!(handler = name, error = ?e, "save_load handler returned error");
                    std::ptr::null_mut()
                }
            }
        }));
        match result {
            Ok(ptr) => ptr,
            Err(_) => {
                tracing::error!(handler = name, "save_load handler panicked, returning null");
                std::ptr::null_mut()
            }
        }
    }
}

/// `non_save_load_handlers` —— 5 个非 save_load 桩 handler 函数。
///
/// 对照原版 `gSimMessageHandlers` 表的 5 个非 save_load 项（Ghidra 未反编译
/// `.rdata` 静态初始化代码，候选 ID 基于 enum 排除法，见 spec §4）。
///
/// **每个桩函数行为**：tracing log + 返回 null。不读取 reader，不修改全局状态。
/// Phase 7+ 接入真实业务逻辑时替换函数体。
mod non_save_load_handlers {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// `SimData_InitializeFromCells` (0x7AD66E59) 桩 handler。
    ///
    /// 候选理由：命名暗示从 cells 初始化 SimData（与 save_load 流程并列）。
    /// 部署后验证：若游戏启动时日志显示此 handler 被调用，候选正确；
    /// 若日志显示此 ID 未被调用或被 HandleMessage 路由，需调整。
    pub unsafe extern "C" fn sim_data_initialize_from_cells(
        _reader: *mut BinaryBufferReader,
    ) -> *mut c_void {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            tracing::trace!(
                id = MessageType::SimData_InitializeFromCells as u32,
                "non_save_load handler: SimData_InitializeFromCells (stub, log + return null)"
            );
        }));
        std::ptr::null_mut()
    }

    /// `SimData_FreeCells` (0xBA6B9E17) 桩 handler。
    ///
    /// 候选理由：命名暗示释放 SimData 的 cells（与 InitializeFromCells 对称）。
    pub unsafe extern "C" fn sim_data_free_cells(_reader: *mut BinaryBufferReader) -> *mut c_void {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            tracing::trace!(
                id = MessageType::SimData_FreeCells as u32,
                "non_save_load handler: SimData_FreeCells (stub, log + return null)"
            );
        }));
        std::ptr::null_mut()
    }

    /// `SetVisibleCells` (0xDE847A61) 桩 handler。
    ///
    /// 候选理由：命名暗示设置可见 cell 范围。
    pub unsafe extern "C" fn set_visible_cells(_reader: *mut BinaryBufferReader) -> *mut c_void {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            tracing::trace!(
                id = MessageType::SetVisibleCells as u32,
                "non_save_load handler: SetVisibleCells (stub, log + return null)"
            );
        }));
        std::ptr::null_mut()
    }

    /// `SetWorldZones` (0xE4C0C9F7) 桩 handler。
    ///
    /// 候选理由：命名暗示设置世界区域。
    pub unsafe extern "C" fn set_world_zones(_reader: *mut BinaryBufferReader) -> *mut c_void {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            tracing::trace!(
                id = MessageType::SetWorldZones as u32,
                "non_save_load handler: SetWorldZones (stub, log + return null)"
            );
        }));
        std::ptr::null_mut()
    }

    /// `RadiationSickness` (0xD4855076) 桩 handler。
    ///
    /// 候选理由：命名暗示辐射病事件。
    pub unsafe extern "C" fn radiation_sickness(_reader: *mut BinaryBufferReader) -> *mut c_void {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            tracing::trace!(
                id = MessageType::RadiationSickness as u32,
                "non_save_load handler: RadiationSickness (stub, log + return null)"
            );
        }));
        std::ptr::null_mut()
    }
}

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
    fn handlers_table_has_14_entries() {
        // Phase 6 升级：从空表升级为 14 项（9 save_load + 5 非 save_load 桩）
        assert_eq!(G_SIM_MESSAGE_HANDLERS.len(), 14, "gSimMessageHandlers should have 14 entries");
    }

    #[test]
    fn handlers_table_contains_all_9_save_load_ids() {
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
        for id in save_load_ids {
            let found = G_SIM_MESSAGE_HANDLERS.iter().any(|e| e.id == id);
            assert!(found, "save_load id {:?} should be in handler table", id);
        }
    }

    #[test]
    fn handlers_table_contains_all_5_non_save_load_candidate_ids() {
        let candidate_ids = [
            MessageType::SimData_InitializeFromCells,
            MessageType::SimData_FreeCells,
            MessageType::SetVisibleCells,
            MessageType::SetWorldZones,
            MessageType::RadiationSickness,
        ];
        for id in candidate_ids {
            let found = G_SIM_MESSAGE_HANDLERS.iter().any(|e| e.id == id);
            assert!(found, "non_save_load candidate id {:?} should be in handler table", id);
        }
    }

    #[test]
    fn handlers_table_all_handlers_are_non_null() {
        // Phase 6：14 项 handler 全部非 null（save_load 真实 + 桩）
        for entry in G_SIM_MESSAGE_HANDLERS {
            assert!(entry.handler.is_some(), "handler for id {:?} should be non-null", entry.id);
        }
    }

    #[test]
    fn message_type_max_value_fits_u32() {
        // 验证最大值能正确表示（防止后续重构误用 #[repr(C)] 或 #[repr(i32)] 导致溢出）
        assert_eq!(MessageType::MoveElementChunk as u32, 3920055938);
    }
}
