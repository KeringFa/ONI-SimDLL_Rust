//! SimEvents / SimFrameInfo — 模拟事件集合 + 单帧信息。
//!
//! SimEvents 字段对照源码 00_types_reference.c L9363-9384（20 个 MsvcVector 字段）。
//! SimFrameInfo 仍为占位（Phase 3+ 补全）。

use crate::stl_shim::MsvcVector;
use crate::game_data::*;

/// SimFrameInfo — 单帧信息（Phase 1 占位，Phase 3+ 补全）。
#[repr(C)]
pub struct SimFrameInfo {
    pub reserved: [u8; 0],
}

/// SimEvents — 模拟事件集合（20 个 MsvcVector 字段）。
///
/// 字段对照源码 00_types_reference.c L9363-9384。
/// 大小 = 20 × 32B = 640B = 0x280。
#[repr(C)]
pub struct SimEvents {
    pub substance_change_info: MsvcVector<SubstanceChangeInfo>,                // L9364 @0x00
    pub spawn_liquid_info: MsvcVector<SpawnFallingLiquidInfo>,                 // L9365 @0x20
    pub spawn_ore_info: MsvcVector<SpawnOreInfo>,                              // L9366 @0x40
    pub unstable_cell_info: MsvcVector<UnstableCellInfo>,                      // L9367 @0x60
    pub element_chunk_melted_info: MsvcVector<MeltedInfo>,                     // L9368 @0x80
    pub building_melted_info: MsvcVector<MeltedInfo>,                          // L9369 @0xa0
    pub building_overheat_info: MsvcVector<MeltedInfo>,                        // L9370 @0xc0
    pub building_no_longer_overheated_info: MsvcVector<MeltedInfo>,            // L9371 @0xe0
    pub cell_melted_info: MsvcVector<CellMeltedInfo>,                          // L9372 @0x100
    pub callback_info: MsvcVector<CallbackInfo>,                               // L9373 @0x120
    pub world_damage_info: MsvcVector<WorldDamageInfo>,                        // L9374 @0x140
    pub mass_consumed_callbacks: MsvcVector<MassConsumedCallback>,             // L9375 @0x160
    pub radiation_consumed_callbacks: MsvcVector<RadiationConsumedCallback>,   // L9376 @0x180
    pub mass_emitted_callbacks: MsvcVector<MassEmittedCallback>,               // L9377 @0x1a0
    pub disease_consumed_callbacks: MsvcVector<DiseaseConsumedCallback>,       // L9378 @0x1c0
    pub spawn_fx_info: MsvcVector<SpawnFXInfo>,                                // L9379 @0x1e0
    pub component_state_changed_messages: MsvcVector<ComponentStateChangedMessage>, // L9380 @0x200
    pub dig_info: MsvcVector<SpawnOreInfo>,                                    // L9381 @0x220
    pub backwall_element_changed_info: MsvcVector<BackwallElementChangedInfo>, // L9382 @0x240
    pub backwall_should_transition_info: MsvcVector<BackwallShouldTransitionInfo>, // L9383 @0x260
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
}
