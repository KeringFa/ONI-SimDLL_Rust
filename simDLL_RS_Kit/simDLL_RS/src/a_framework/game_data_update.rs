//! GameDataUpdate — 每帧从 GameData 提取的可观察数据。
//!
//! 对照源码 02_save_load.c L688-1059（PrepareGameDataUpdate）。
//! `#[repr(C, packed(4))]` 严格匹配 C# `[StructLayout(LayoutKind.Sequential, Pack=4)]`。
//!
//! **packed(4) 安全约束**：禁止对 packed 字段取引用（UB），只允许整体取引用或直接写。

use crate::a_framework::game_data::*;
use crate::a_framework::vector_math::Vector2f;

/// GameDataUpdate — PrepareGameDataUpdate 的目标 struct（496B）。
///
/// 1 个 int + 12 个 SOA 裸指针 + 29 对 num+ptr + 6 个 property texture 指针 = 496B。
/// C# Pack=4 让指针 4 字节对齐，每对 num(4)+ptr(8)=12B 紧接无 pad。
#[repr(C, packed(4))]
pub struct GameDataUpdate {
    // 1 个标量（L702）—— offset 0
    pub num_frames_processed: i32,

    // 12 个 SOA 裸指针（offset 4 起，每个 8B，4 字节对齐）
    pub element_idx: *const u16,                             // L705 cells.element_idx._First
    pub temperature: *const f32,                             // L710 cells.temperature._First
    pub mass: *const f32,                                    // L720 cells.mass._First
    pub properties: *const u8,                               // L725 cells.properties._First
    pub insulation: *const u8,                               // L730 cells.insulation._First
    pub strength_info: *const u8,                            // L735 cells.strength_info._First
    pub radiation: *const f32,                               // L715 cells.radiation._First
    pub disease_idx: *const u8,                              // L740 cells.disease_idx._First
    pub disease_count: *const i32,                           // L745 cells.disease_count._First
    pub backwall_element_idx: *const u16,                    // L751 backwall.element_idx._First
    pub backwall_masses: *const f32,                         // L756 backwall.mass._First
    pub backwall_temperatures: *const f32,                   // L761 backwall.temperature._First

    // 29 对 num+ptr（offset 100 起，每对 12B：i32 count + *const T pointer，无 pad）
    pub num_solid_info: i32,                                 // L767
    pub solid_info: *const SolidInfo,                        // L770

    pub num_liquid_change_info: i32,                         // L775
    pub liquid_change_info: *const LiquidChangeInfo,         // L778

    pub num_solid_substance_change_info: i32,                // L802
    pub solid_substance_change_info: *const SolidSubstanceChangeInfo, // L806

    pub num_substance_change_info: i32,                      // L811
    pub substance_change_info: *const SubstanceChangeInfo,   // L815

    pub num_callback_info: i32,                              // L820
    pub callback_info: *const CallbackInfo,                  // L823

    pub num_spawn_falling_liquid_info: i32,                  // L828
    pub spawn_falling_liquid_info: *const SpawnFallingLiquidInfo, // L832

    pub num_dig_info: i32,                                   // L839
    pub dig_info: *const SpawnOreInfo,                       // L842

    pub num_spawn_ore_info: i32,                             // L848
    pub spawn_ore_info: *const SpawnOreInfo,                 // L851

    pub num_spawn_fx_info: i32,                              // L857
    pub spawn_fx_info: *const SpawnFXInfo,                   // L860

    pub num_unstable_cell_info: i32,                         // L866
    pub unstable_cell_info: *const UnstableCellInfo,         // L869

    pub num_world_damage_info: i32,                          // L876
    pub world_damage_info: *const WorldDamageInfo,           // L879

    pub num_building_temperature_info: i32,                  // L884
    pub building_temperature_info: *const BuildingTemperatureInfo, // L888

    pub num_mass_consumed_callbacks: i32,                    // L893
    pub mass_consumed_callbacks: *const MassConsumedCallback, // L897

    pub num_mass_emitted_callbacks: i32,                     // L904
    pub mass_emitted_callbacks: *const MassEmittedCallback,  // L908

    pub num_disease_consumed_callbacks: i32,                 // L915
    pub disease_consumed_callbacks: *const DiseaseConsumedCallback, // L919

    pub num_component_state_changed_messages: i32,           // L926
    pub component_state_changed_messages: *const ComponentStateChangedMessage, // L930

    pub num_removed_mass_entries: i32,                       // L935
    pub removed_mass_entries: *const ConsumedMassInfo,       // L938

    pub num_emitted_mass_entries: i32,                       // L945
    pub emitted_mass_entries: *const EmittedMassInfo,        // L948

    pub num_element_chunk_infos: i32,                        // L953
    pub element_chunk_infos: *const ElementChunkInfo,        // L956

    pub num_element_chunk_melted_infos: i32,                 // L961
    pub element_chunk_melted_infos: *const MeltedInfo,       // L965

    pub num_building_overheat_infos: i32,                    // L970
    pub building_overheat_infos: *const MeltedInfo,          // L974

    pub num_building_no_longer_overheated_infos: i32,        // L979
    pub building_no_longer_overheated_infos: *const MeltedInfo, // L983

    pub num_building_melted_infos: i32,                      // L988
    pub building_melted_infos: *const MeltedInfo,            // L992

    pub num_cell_melted_infos: i32,                          // L996
    pub cell_melted_infos: *const CellMeltedInfo,            // L999

    pub num_backwall_element_changed_infos: i32,             // L1004
    pub backwall_element_changed_infos: *const BackwallElementChangedInfo, // L1008

    pub num_backwall_should_transition_infos: i32,           // L1013
    pub backwall_should_transition_infos: *const BackwallShouldTransitionInfo, // L1017

    pub num_disease_emitted_infos: i32,                      // L1022
    pub disease_emitted_infos: *const DiseaseEmittedInfo,    // L1026

    pub num_disease_consumed_infos: i32,                     // L1031
    pub disease_consumed_infos: *const DiseaseConsumedInfo,  // L1035

    pub num_radiation_consumed_callbacks: i32,               // L1040
    pub radiation_consumed_callbacks: *const RadiationConsumedCallback, // L1044

    // 6 个 property texture 指针（L1051-1057，offset 448-488）
    pub accumulated_flow: *const f32,                         // L1051
    pub property_texture_flow: *const Vector2f,               // L1052
    pub property_texture_liquid: *const u32,                  // L1053
    pub property_texture_liquid_data: *const u32,             // L1054
    pub property_texture_material_data: *const u32,           // L1055
    pub property_texture_exposed_to_sunlight: *const u8,      // L1056
}

impl Default for GameDataUpdate {
    fn default() -> Self {
        Self {
            num_frames_processed: 0,
            element_idx: std::ptr::null(),
            temperature: std::ptr::null(),
            mass: std::ptr::null(),
            properties: std::ptr::null(),
            insulation: std::ptr::null(),
            strength_info: std::ptr::null(),
            radiation: std::ptr::null(),
            disease_idx: std::ptr::null(),
            disease_count: std::ptr::null(),
            backwall_element_idx: std::ptr::null(),
            backwall_masses: std::ptr::null(),
            backwall_temperatures: std::ptr::null(),
            num_solid_info: 0, solid_info: std::ptr::null(),
            num_liquid_change_info: 0, liquid_change_info: std::ptr::null(),
            num_solid_substance_change_info: 0, solid_substance_change_info: std::ptr::null(),
            num_substance_change_info: 0, substance_change_info: std::ptr::null(),
            num_callback_info: 0, callback_info: std::ptr::null(),
            num_spawn_falling_liquid_info: 0, spawn_falling_liquid_info: std::ptr::null(),
            num_dig_info: 0, dig_info: std::ptr::null(),
            num_spawn_ore_info: 0, spawn_ore_info: std::ptr::null(),
            num_spawn_fx_info: 0, spawn_fx_info: std::ptr::null(),
            num_unstable_cell_info: 0, unstable_cell_info: std::ptr::null(),
            num_world_damage_info: 0, world_damage_info: std::ptr::null(),
            num_building_temperature_info: 0, building_temperature_info: std::ptr::null(),
            num_mass_consumed_callbacks: 0, mass_consumed_callbacks: std::ptr::null(),
            num_mass_emitted_callbacks: 0, mass_emitted_callbacks: std::ptr::null(),
            num_disease_consumed_callbacks: 0, disease_consumed_callbacks: std::ptr::null(),
            num_component_state_changed_messages: 0, component_state_changed_messages: std::ptr::null(),
            num_removed_mass_entries: 0, removed_mass_entries: std::ptr::null(),
            num_emitted_mass_entries: 0, emitted_mass_entries: std::ptr::null(),
            num_element_chunk_infos: 0, element_chunk_infos: std::ptr::null(),
            num_element_chunk_melted_infos: 0, element_chunk_melted_infos: std::ptr::null(),
            num_building_overheat_infos: 0, building_overheat_infos: std::ptr::null(),
            num_building_no_longer_overheated_infos: 0, building_no_longer_overheated_infos: std::ptr::null(),
            num_building_melted_infos: 0, building_melted_infos: std::ptr::null(),
            num_cell_melted_infos: 0, cell_melted_infos: std::ptr::null(),
            num_backwall_element_changed_infos: 0, backwall_element_changed_infos: std::ptr::null(),
            num_backwall_should_transition_infos: 0, backwall_should_transition_infos: std::ptr::null(),
            num_disease_emitted_infos: 0, disease_emitted_infos: std::ptr::null(),
            num_disease_consumed_infos: 0, disease_consumed_infos: std::ptr::null(),
            num_radiation_consumed_callbacks: 0, radiation_consumed_callbacks: std::ptr::null(),
            accumulated_flow: std::ptr::null(),
            property_texture_flow: std::ptr::null(),
            property_texture_liquid: std::ptr::null(),
            property_texture_liquid_data: std::ptr::null(),
            property_texture_material_data: std::ptr::null(),
            property_texture_exposed_to_sunlight: std::ptr::null(),
        }
    }
}

// GameDataUpdate 包含裸指针（SOA 指针），这些指针指向 SimData 中的数据。
// 全局 G_GAME_DATA_UPDATE 受 OnceCell 保护，初始化后只读，可安全跨线程共享。
unsafe impl Send for GameDataUpdate {}
unsafe impl Sync for GameDataUpdate {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn game_data_update_size() {
        // 1×i32 (4) + 12×ptr (96) + 29×(i32+ptr) (348) + 6×ptr (48) = 496B
        let actual = size_of::<GameDataUpdate>();
        assert_eq!(actual, 496, "GameDataUpdate size = {}, 期望 496 (C# Pack=4)", actual);
    }

    #[test]
    fn game_data_update_default_all_null() {
        let gdu = GameDataUpdate::default();
        assert_eq!(gdu.num_frames_processed, 0);
        assert!(gdu.element_idx.is_null());
        assert!(gdu.temperature.is_null());
        assert!(gdu.accumulated_flow.is_null());
        assert_eq!(gdu.num_solid_info, 0);
        assert!(gdu.solid_info.is_null());
    }
}
