//! SimEvents / 消息结构体 — 模拟事件集合 + 消息体定义。
//!
//! SimEvents 字段对照源码 00_types_reference.c L9363-9384（20 个 MsvcVector 字段）。
//! SimFrameInfo 已移至 sim_frame_manager.rs（C1 阶段扩展为完整版 0x5b8）。
//! 6 个消息结构体对照源码 handle_message emplace 偏移实现。

use crate::a_framework::stl_shim::MsvcVector;
use crate::a_framework::game_data::*;

// ===== 6 个消息结构体（handle_message emplace 用）=====

/// CellModification — ModifyCell 消息体（28B = 0x1c）。
/// 对照 C# SimMessages.cs L434-451 ModifyCellMessage：
/// @0 cellIdx, @4 callbackIdx, @8 temperature, @12 mass,
/// @16 diseaseCount, @20 elementIdx, @22 replaceType, @23 diseaseIdx, @24 flags。
/// （注意：此前 Rust 版 mass/temperature 与 disease_idx/replace_mode 两对字段
/// 顺序写反，已按 C# 布局修正。）
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CellModification {
    pub game_cell: i32,
    pub callback_idx: i32,
    pub temperature: f32,
    pub mass: f32,
    pub disease_count: i32,
    pub element_idx: u16,
    pub replace_mode: u8,  // 0=Add, 1=Replace, 2=ReplaceAndDisplace
    pub disease_idx: u8,
    pub flags: u8,
    pub _pad: [u8; 3],
}

/// CellPropertiesChange — ChangeCellProperties 消息体（12B = 0xc）。
/// 对照源码 06_process_messages.c L249。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CellPropertiesChange {
    pub game_cell: i32,
    pub callback_idx: i32,
    pub property_flags: u8,
    pub set_or_clear: u8,  // 0=clear, 1=set
    pub _pad: [u8; 2],
}

/// MassConsumption — MassConsumption 消息体（16B）。
/// 对照 C# SimMessages.cs MassConsumptionMessage：cellIdx, callbackIdx, mass,
/// elementIdx, radius, height（4+4+4+2+1+1 = 16B）。
/// ⚠️ 2026-08-02 修复：旧实现 12B 且字段错位（把 mass 当成 elem_idx），
/// radius/height 完全丢失 → MassConsumption 从未被正确处理（Flood 系列缺失的根因之一）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MassConsumption {
    pub game_cell: i32,
    pub callback_idx: i32,
    pub mass: f32,
    pub element_idx: u16,
    pub radius: u8,
    pub height: u8, // 0=FloodRemoved（半径模式），>0=RectangularRemoved（宽×高）
}

/// AddElementConsumerMessage（12B，C# SimMessages.AddElementConsumerMessage）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AddElementConsumerMsg {
    pub cell_idx: i32,
    pub callback_idx: i32,
    pub radius: u8,
    pub configuration: u8, // 0=指定元素 1=任意液体 2=任意气体
    pub element_idx: u16,
}

/// SetElementConsumerDataMessage（12B，C# SimMessages.SetElementConsumerDataMessage）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SetElementConsumerDataMsg {
    pub handle: i32,
    pub cell: i32,
    pub consumption_rate: f32,
}

/// RemoveElementConsumerMessage（8B，C# SimMessages.RemoveElementConsumerMessage）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RemoveElementConsumerMsg {
    pub handle: i32,
    pub callback_idx: i32,
}

/// AddBuildingHeatExchangeMessage (0x67A75D28, 44B, C# SimMessages.cs L218-235).
/// ModifyBuildingHeatExchangeMessage shares this layout (0x6C5C80A1, 44B) with
/// callbackIdx carrying the sim building-temperature handle (C# L917-931).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AddBuildingHeatExchangeMsg {
    pub callback_idx: i32,
    pub elem_idx: u16,
    pub pad0: u8,
    pub pad1: u8,
    pub mass: f32,
    pub temperature: f32,
    pub thermal_conductivity: f32,
    pub overheat_temperature: f32,
    pub operating_kilowatts: f32,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// RemoveBuildingHeatExchangeMessage (0xE4D1E06B, 8B, C# SimMessages.cs L944-952).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RemoveBuildingHeatExchangeMsg {
    pub handle: i32,
    pub callback_idx: i32,
}

/// MassEmission — MassEmission 消息体（24B = 6 × i32）。
/// 对照源码 06_process_messages.c L1242。
///
/// 字段 4+4+4+4+4+2+1+1 = 24B，#[repr(C)] 对齐到 4B → 24B（无需 padding）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MassEmission {
    pub game_cell: i32,
    pub callback_idx_or_temp: f32,
    pub mass: f32,
    pub temperature: f32,
    pub disease_count: i32,
    pub element_idx: u16,
    pub disease_idx: u8,
    pub emitted: u8,
}

/// SetCellFloatValue — SetInsulationValue / SetStrengthValue 消息体（8B）。
/// 对照源码 06_process_messages.c L1230。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SetCellFloatValue {
    pub game_cell: i32,
    pub value: f32,
}

/// DigPoint — Dig 消息体（12B = 0xc）。
/// 对照 C# SimMessages.cs L388-398 DigMessage + 源码 08_sim_frame_manager.c L784-793。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DigPoint {
    pub game_cell: i32,
    pub callback_idx: i32,
    pub skip_event: u8,
    pub backwall: u8,
    pub _pad: [u8; 2],
}

/// SimEvents — 模拟事件集合（20 个 MsvcVector 字段 = 640B = 0x280）。
/// 字段对照源码 00_types_reference.c L9363-9384。
#[repr(C)]
pub struct SimEvents {
    pub substance_change_info: MsvcVector<SubstanceChangeInfo>,                // L9364
    pub spawn_liquid_info: MsvcVector<SpawnFallingLiquidInfo>,                 // L9365
    pub spawn_ore_info: MsvcVector<SpawnOreInfo>,                              // L9366
    pub unstable_cell_info: MsvcVector<UnstableCellInfo>,                      // L9367
    pub element_chunk_melted_info: MsvcVector<MeltedInfo>,                     // L9368
    pub building_melted_info: MsvcVector<MeltedInfo>,                          // L9369
    pub building_overheat_info: MsvcVector<MeltedInfo>,                        // L9370
    pub building_no_longer_overheated_info: MsvcVector<MeltedInfo>,            // L9371
    pub cell_melted_info: MsvcVector<CellMeltedInfo>,                          // L9372
    pub callback_info: MsvcVector<CallbackInfo>,                               // L9373
    pub world_damage_info: MsvcVector<WorldDamageInfo>,                        // L9374
    pub mass_consumed_callbacks: MsvcVector<MassConsumedCallback>,             // L9375
    pub radiation_consumed_callbacks: MsvcVector<RadiationConsumedCallback>,   // L9376
    pub mass_emitted_callbacks: MsvcVector<MassEmittedCallback>,               // L9377
    pub disease_consumed_callbacks: MsvcVector<DiseaseConsumedCallback>,       // L9378
    pub spawn_fx_info: MsvcVector<SpawnFXInfo>,                                // L9379
    pub component_state_changed_messages: MsvcVector<ComponentStateChangedMessage>, // L9380
    pub dig_info: MsvcVector<SpawnOreInfo>,                                    // L9381
    pub backwall_element_changed_info: MsvcVector<BackwallElementChangedInfo>, // L9382
    pub backwall_should_transition_info: MsvcVector<BackwallShouldTransitionInfo>, // L9383
}

impl Default for SimEvents {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn sim_events_size() {
        // 20 × MsvcVector (32B) = 640B = 0x280
        assert_eq!(size_of::<SimEvents>(), 640);
    }

    #[test]
    fn dig_point_size_is_12() { assert_eq!(size_of::<DigPoint>(), 12); }

    #[test]
    fn cell_modification_size_is_28() { assert_eq!(size_of::<CellModification>(), 28); }

    #[test]
    fn cell_properties_change_size_is_12() { assert_eq!(size_of::<CellPropertiesChange>(), 12); }

    #[test]
    fn mass_consumption_size_is_16() { assert_eq!(size_of::<MassConsumption>(), 16); }
    #[test]
    fn add_element_consumer_size_is_12() { assert_eq!(size_of::<AddElementConsumerMsg>(), 12); }
    #[test]
    fn set_element_consumer_data_size_is_12() { assert_eq!(size_of::<SetElementConsumerDataMsg>(), 12); }
    #[test]
    fn remove_element_consumer_size_is_8() { assert_eq!(size_of::<RemoveElementConsumerMsg>(), 8); }

    #[test]
    fn mass_emission_size_is_24() { assert_eq!(size_of::<MassEmission>(), 24); }

    #[test]
    fn set_cell_float_value_size_is_8() { assert_eq!(size_of::<SetCellFloatValue>(), 8); }

    #[test]
    fn sim_events_default_all_empty() {
        let ev = SimEvents::default();
        assert!(ev.substance_change_info.is_empty());
        assert!(ev.spawn_liquid_info.is_empty());
        assert!(ev.dig_info.is_empty());
    }
}
