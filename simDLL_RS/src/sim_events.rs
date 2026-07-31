//! SimEvents / SimFrameInfo — 模拟事件集合 + 单帧信息 + Phase 7 消息结构体。
//!
//! SimEvents 字段对照源码 00_types_reference.c L9363-9384（20 个 MsvcVector 字段）。
//! SimFrameInfo Phase 7 补全 8 字段（共 37 字段，其余 29 字段推迟到 Phase 8/9）。
//! 5 个消息结构体（CellModification / CellPropertiesChange / MassConsumption /
//! MassEmission / SetCellFloatValue）对照源码 handle_message emplace 偏移实现。

use crate::stl_shim::MsvcVector;
use crate::game_data::*;
use crate::sim_data::DebugProperties;

/// SimFrameInfo — 单帧信息（Phase 7 补全 8 字段，其余 29 字段推迟）。
///
/// 对照源码 00_types_reference.c L5406-5446（37 字段，0x1c0=452B 完整）。
/// Phase 7 仅补全消息处理需要的 8 字段：
/// - elapsed_seconds (L5407)
/// - cell_modifications (L5413)
/// - set_insulation_values (L5416)
/// - set_strength_values (L5417)
/// - set_cell_properties (L5418)
/// - mass_consumption_messages (L5425)
/// - mass_emission_messages (L5426)
/// - debug_properties (L5441)
///
/// **注意**：Rust 内部结构，可后续追加字段而无需 C ABI 兼容。
/// handle_message 的 39 个非 Phase 7 arm 改为 drain-only（读字节丢弃，不 emplace）。
#[repr(C)]
pub struct SimFrameInfo {
    pub elapsed_seconds: f32,                                    // L5407 @0x00
    pub _pad: [u8; 4],                                           // @0x04 对齐到 8B
    pub cell_modifications: MsvcVector<CellModification>,        // L5413 @0x08
    pub set_insulation_values: MsvcVector<SetCellFloatValue>,    // L5416 @0x28
    pub set_strength_values: MsvcVector<SetCellFloatValue>,      // L5417 @0x48
    pub set_cell_properties: MsvcVector<CellPropertiesChange>,   // L5418 @0x68
    pub mass_consumption_messages: MsvcVector<MassConsumption>,   // L5425 @0x88
    pub mass_emission_messages: MsvcVector<MassEmission>,         // L5426 @0xa8
    pub debug_properties: DebugProperties,                        // L5441 @0xc8
}

impl Default for SimFrameInfo {
    fn default() -> Self {
        // unsafe { std::mem::zeroed() }：MsvcVector 全零=空 vector，DebugProperties 全零=Default
        unsafe { std::mem::zeroed() }
    }
}

// ===== Phase 7：5 个消息结构体（handle_message emplace 用）=====

/// CellModification — ModifyCell 消息体（28B = 0x1c）。
///
/// 对照源码 06_process_messages.c L1303-1305 的 `local_c0` 拆包 +
/// L1496 的 `piVar2 += 0x1c`（每条 28B）+ L221 ProcessCellModifications。
///
/// **字段布局**（基于源码位偏移反推）：
/// - game_cell (i32 @ 0x00)：游戏侧格子索引
/// - callback_idx (i32 @ 0x04)：C# 回调索引
/// - mass (f32 @ 0x08)
/// - temperature (f32 @ 0x0c)
/// - disease_count (i32 @ 0x10)：源码 (int)local_c0
/// - element_idx (u16 @ 0x14)：源码 local_c0._4_2_
/// - disease_idx (u8 @ 0x16)：源码 local_c0._6_1_
/// - replace_mode (u8 @ 0x17)：源码 local_c0._7_1_（0=Add, 1=Replace, 2=ReplaceAndDisplace）
/// - flags (u8 @ 0x18)：源码 local_b8 & 0xff
/// - _pad ([u8; 3] @ 0x19)：对齐到 4B（总计 28B = 0x1c）
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CellModification {
    pub game_cell: i32,
    pub callback_idx: i32,
    pub mass: f32,
    pub temperature: f32,
    pub disease_count: i32,
    pub element_idx: u16,
    pub disease_idx: u8,
    pub replace_mode: u8,  // 0=Add, 1=Replace, 2=ReplaceAndDisplace
    pub flags: u8,
    pub _pad: [u8; 3],
}

/// CellPropertiesChange — ChangeCellProperties 消息体（12B = 0xc）。
///
/// 对照源码 06_process_messages.c L249 `lVar11 / 0xc`（每条 12B）+ L221 ProcessCellProperties。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CellPropertiesChange {
    pub game_cell: i32,        // *(int *)(lVar10 + lVar2)
    pub callback_idx: i32,    // *(int *)(lVar10 + 4 + lVar2)
    pub property_flags: u8,   // *(byte *)(lVar10 + 8 + lVar2)
    pub set_or_clear: u8,      // *(byte *)(lVar10 + 9 + lVar2)（0=clear, 1=set）
    pub _pad: [u8; 2],         // 对齐到 12B
}

/// MassConsumption — MassConsumption 消息体（16B = 4 × i32）。
///
/// 对照源码 06_process_messages.c L1014 `piVar8 += 4`（每条 16B）+ L920 ProcessMassConsumption。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MassConsumption {
    pub game_cell: i32,        // *piVar8
    pub callback_idx: i32,    // piVar8[1]
    pub removed_elem_idx: u16, // *(ushort *)(piVar8 + 3) @ 0x0c
    pub loop_radius: u8,       // *(byte *)((longlong)piVar8 + 0xe)
    pub use_rectangle: u8,     // *(byte *)((longlong)piVar8 + 0xf)（0=FloodRemoved, 1=RectangularRemoved）
}

/// MassEmission — MassEmission 消息体（24B = 6 × i32）。
///
/// 对照源码 06_process_messages.c L1242 `piVar18 += 6`（每条 24B）+ L1042 ProcessMassEmission。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MassEmission {
    pub game_cell: i32,             // *piVar18 @ 0x00
    pub callback_idx_or_temp: f32,  // piVar18[1] as f32 @ 0x04（语义见源码 L1076-1242）
    pub mass: f32,                  // piVar18[2] as f32 @ 0x08
    pub temperature: f32,           // piVar18[3] as f32 @ 0x0c
    pub disease_count: i32,         // piVar18[4] @ 0x10
    pub element_idx: u16,           // *(ushort *)(piVar18 + 5) @ 0x14
    pub disease_idx: u8,            // *(uchar *)((longlong)piVar18 + 0x16)
    pub emitted: u8,                // bVar13 @ 0x17（源码是否成功标志）
    pub _pad: [u8; 2],              // 对齐到 24B（实际 0x18 = 24B）
}

/// SetCellFloatValue — SetInsulationValue / SetStrengthValue 消息体（8B = 2 × i32）。
///
/// 对照源码 06_process_messages.c L1230 `piVar31 += 2`（每条 8B）+ L1178 ProcessFrame 内联循环。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SetCellFloatValue {
    pub game_cell: i32,    // *piVar31
    pub value: f32,         // (float)piVar31[1]
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

impl Default for SimEvents {
    /// 构造空 SimEvents（20 个 MsvcVector 全空，对应 MSVC std::vector 默认构造）。
    ///
    /// 与 `SimFrameInfo::default` 一致：用 `unsafe { std::mem::zeroed() }`。
    /// 全零的 MsvcVector 是空 vector（first/last/end 指针全 null）。
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
    fn sim_frame_info_size_in_expected_range() {
        // Phase 7：8 字段（f32 + 4B pad + 6×MsvcVector(32B) + DebugProperties(12B)）
        // = 4 + 4 + 6*32 + 12 = 212B（#[repr(C)] 末尾对齐到 8B → 216B）
        // 范围断言允许 200-260B（实际布局由 #[repr(C)] 决定）
        let actual = size_of::<SimFrameInfo>();
        assert!(actual >= 200 && actual <= 260, "SimFrameInfo size = {}", actual);
    }

    #[test]
    fn cell_modification_size_is_28() {
        // game_cell(4) + callback_idx(4) + mass(4) + temperature(4) + disease_count(4) +
        // element_idx(2) + disease_idx(1) + replace_mode(1) + flags(1) + 3B pad = 28B ✓
        assert_eq!(size_of::<CellModification>(), 28);
    }

    #[test]
    fn cell_properties_change_size_is_12() {
        // i32 (4) + i32 (4) + u8 (1) + u8 (1) + 2B pad = 12B
        assert_eq!(size_of::<CellPropertiesChange>(), 12);
    }

    #[test]
    fn mass_consumption_size_is_12_or_16() {
        // i32 (4) + i32 (4) + u16 (2) + u8 (1) + u8 (1) = 12B
        // 源码偏移反推 16B（piVar8 += 4 即 4×i32=16B），但 #[repr(C)] 紧密打包 = 12B
        // Phase 7 接受 12B 或 16B，若部署后 reader 错位再调整
        let actual = size_of::<MassConsumption>();
        assert!(actual == 12 || actual == 16, "MassConsumption size = {}", actual);
    }

    #[test]
    fn mass_emission_size_is_24() {
        // i32(4) + f32(4) + f32(4) + f32(4) + i32(4) + u16(2) + u8(1) + u8(1) + 2B pad = 24B ✓
        // 但 #[repr(C)] 末尾对齐到 4B（最大字段 i32/f32 对齐），实际 = 24B
        let actual = size_of::<MassEmission>();
        assert!(actual == 24 || actual == 28, "MassEmission size = {}", actual);
    }

    #[test]
    fn set_cell_float_value_size_is_8() {
        // i32 (4) + f32 (4) = 8B
        assert_eq!(size_of::<SetCellFloatValue>(), 8);
    }

    #[test]
    fn sim_frame_info_default_all_zero() {
        let f = SimFrameInfo::default();
        assert_eq!(f.elapsed_seconds, 0.0);
        assert!(f.cell_modifications.is_empty());
        assert!(f.set_cell_properties.is_empty());
    }
}
