//! GameData — simDLL 游戏数据 struct（每帧同步给 C# 的数据）。
//!
//! 字段对照源码：00_types_reference.c L3225-3269。
//! 25 个 *Info/*Callback 子结构 + Handle 辅助结构，全部基于源码实际字段定义
//! （非估算占位）。MsvcVector<T> 大小与 T 无关（4 指针 = 32B），故 *Info 子结构
//! 大小不影响 GameData 总大小，GameData 精确 = 1024B。

use crate::sim_data::{BackwallSOA, CellSOA};
use crate::stl_shim::{MsvcVector, UniquePtr};
use crate::vector_math::{Vector2f, Vector4f};

// ===== Handle（游戏句柄，4B）=====

/// Handle — 游戏对象句柄。
///
/// 源码 00_types_reference.c L1801-1803：`struct Handle { int value; }` = 4B。
/// 被 MeltedInfo / BuildingTemperatureInfo / ConsumedMassInfo 引用。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Handle {
    pub value: i32, // L1802
}

// ===== 25 个 *Info/*Callback 子结构（按源码实际字段）=====

// --- 4B 子结构（7 个）---

/// CallbackInfo — 源码 L12206-12208。int callbackIdx = 4B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CallbackInfo {
    pub callback_idx: i32, // L12207
}

/// LiquidChangeInfo — 源码 L9784-9786。int cellIdx = 4B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LiquidChangeInfo {
    pub cell_idx: i32, // L9785
}

/// SolidSubstanceChangeInfo — 源码 L12748-12750。int cellIdx = 4B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SolidSubstanceChangeInfo {
    pub cell_idx: i32, // L12749
}

/// MeltedInfo — 源码 L7183-7185。struct Handle handle = 4B。
/// 被 GameData 的 4 个 vector 字段复用（elementChunkMeltedInfo /
/// buildingMeltedInfo / buildingOverheatInfo / buildingNoLongerOverheatedInfo）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MeltedInfo {
    pub handle: Handle, // L7184
}

/// CellMeltedInfo — 源码 L11929-11931。uint gameCell = 4B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CellMeltedInfo {
    pub game_cell: u32, // L11930
}

/// BackwallElementChangedInfo — 源码 L8737-8739。uint gameCell = 4B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BackwallElementChangedInfo {
    pub game_cell: u32, // L8738
}

/// BackwallShouldTransitionInfo — 源码 L12758-12760。uint gameCell = 4B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BackwallShouldTransitionInfo {
    pub game_cell: u32, // L12759
}

// --- 8B 子结构（8 个）---

/// SubstanceChangeInfo — 源码 L1847-1851。int + ushort + ushort = 8B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SubstanceChangeInfo {
    pub cell_idx: i32,        // L1848
    pub old_element_idx: u16, // L1849 oldElementIdx
    pub new_element_idx: u16, // L1850 newElementIdx
}

/// WorldDamageInfo — 源码 L7438-7441。int + int = 8B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WorldDamageInfo {
    pub cell_idx: i32,               // L7439
    pub damage_source_cell_idx: i32, // L7440
}

/// BuildingTemperatureInfo — 源码 L9481-9484。Handle + float = 8B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BuildingTemperatureInfo {
    pub handle: Handle,   // L9482
    pub temperature: f32, // L9483
}

/// SolidInfo — 源码 L12451-12454。int + int = 8B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SolidInfo {
    pub cell_idx: i32, // L12452
    pub solid: i32,    // L12453
}

/// ElementChunkInfo — 源码 L2064-2067。float + float = 8B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementChunkInfo {
    pub temperature: f32, // L2065
    pub delta_kj: f32,    // L2066 deltaKJ
}

/// DiseaseEmittedInfo — 源码 L12762-12765。uchar + pad[3] + int = 8B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DiseaseEmittedInfo {
    pub disease_idx: u8,    // L12763
    pub padding: [u8; 3],   // L12764 padding[3]
    pub count: i32,         // L12765
}

/// DiseaseConsumedInfo — 源码 L1041-1045。uchar + pad[3] + int = 8B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DiseaseConsumedInfo {
    pub disease_idx: u8,    // L1042
    pub padding: [u8; 3],   // L1043 padding[3]
    pub count: i32,         // L1044
}

/// ComponentStateChangedMessage — 源码 L7528-7531。int + int = 8B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ComponentStateChangedMessage {
    pub callback_idx: i32, // L7529
    pub sim_handle: i32,   // L7530 simHandle
}

// --- 12B 子结构（3 个）---

/// DiseaseConsumedCallback — 源码 L1855-1860。int + uchar + pad[3] + int = 12B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DiseaseConsumedCallback {
    pub callback_idx: i32,  // L1856
    pub disease_idx: u8,    // L1857
    pub pad: [u8; 3],       // L1858 pad[3]
    pub disease_count: i32, // L1859
}

/// RadiationConsumedCallback — 源码 L12752-12756。int + int + float = 12B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RadiationConsumedCallback {
    pub callback_idx: i32, // L12753
    pub game_cell: i32,    // L12754
    pub radiation: f32,    // L12755
}

/// SpawnFXInfo — 源码 L7250-7254。int + int + float = 12B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SpawnFXInfo {
    pub cell_idx: i32, // L7251
    pub fx_id: i32,    // L7252 fxID
    pub rotation: f32, // L7253
}

// --- 16B 子结构（1 个）---

/// EmittedMassInfo — 源码 L11935-11942。ushort + uchar + uchar + float + float + int = 16B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EmittedMassInfo {
    pub elem_idx: u16,       // L11936
    pub disease_idx: u8,     // L11937
    pub pad: u8,             // L11938
    pub mass: f32,           // L11939
    pub temperature: f32,    // L11940
    pub disease_count: i32,  // L11941
}

// --- 20B 子结构（6 个）---

/// SpawnFallingLiquidInfo — 源码 L12545-12553。int + ushort + uchar + uchar + float + float + int = 20B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SpawnFallingLiquidInfo {
    pub cell_idx: i32,       // L12546
    pub element_idx: u16,    // L12547
    pub disease_idx: u8,     // L12548
    pub pad: u8,             // L12549
    pub mass: f32,           // L12550
    pub temperature: f32,    // L12551
    pub disease_count: i32,  // L12552
}

/// SpawnOreInfo — 源码 L5973-5981。int + ushort + uchar + uchar + float + float + int = 20B。
/// 被 GameData 的 spawnOreInfo 和 digInfo 两个字段复用。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SpawnOreInfo {
    pub cell_idx: i32,       // L5974
    pub elem_idx: u16,       // L5975
    pub disease_idx: u8,     // L5976
    pub pad: u8,             // L5977
    pub mass: f32,           // L5978
    pub temperature: f32,    // L5979
    pub disease_count: i32,  // L5980
}

/// UnstableCellInfo — 源码 L12523-12531。int + ushort + uchar + uchar + float + float + int = 20B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct UnstableCellInfo {
    pub cell_idx: i32,        // L12524
    pub elem_idx: u16,        // L12525
    pub falling_info: u8,     // L12526 fallingInfo
    pub disease_idx: u8,      // L12527
    pub mass: f32,            // L12528
    pub temperature: f32,     // L12529
    pub disease_count: i32,   // L12530
}

/// MassConsumedCallback — 源码 L10127-10135。int + ushort + uchar + uchar + float + float + int = 20B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MassConsumedCallback {
    pub callback_idx: i32,   // L10128
    pub elem_idx: u16,        // L10129
    pub disease_idx: u8,       // L10130
    pub pad: u8,               // L10131
    pub mass: f32,             // L10132
    pub temperature: f32,     // L10133
    pub disease_count: i32,   // L10134
}

/// MassEmittedCallback — 源码 L12738-12746。int + ushort + uchar + uchar + float + float + int = 20B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MassEmittedCallback {
    pub callback_idx: i32,   // L12739
    pub elem_idx: u16,        // L12740
    pub emitted: u8,           // L12741
    pub disease_idx: u8,       // L12742
    pub mass: f32,             // L12743
    pub temperature: f32,     // L12744
    pub disease_count: i32,   // L12745
}

/// ConsumedMassInfo — 源码 L1805-1813。Handle + ushort + uchar + uchar + float + float + int = 20B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ConsumedMassInfo {
    pub sim_handle: Handle,      // L1806 simHandle
    pub removed_elem_idx: u16,   // L1807 removedElemIdx
    pub disease_idx: u8,         // L1808
    pub pad: u8,                 // L1809
    pub mass: f32,               // L1810
    pub temperature: f32,        // L1811
    pub disease_count: i32,      // L1812
}

// ===== GameData =====

/// GameData — 每帧同步给 C# 的游戏数据。
///
/// 字段对照源码 00_types_reference.c L3225-3269。
///
/// 大小 = 4×i32 (16) + 29×MsvcVector (928) + 10×UniquePtr (80) = 1024B。
/// MsvcVector<T> 大小与 T 无关（4 指针 = 32B），故 *Info 子结构大小不影响 GameData 总大小。
#[repr(C)]
pub struct GameData {
    pub width: i32,                                                                  // L3226
    pub height: i32,                                                                 // L3227
    pub num_frames_processed: i32,                                                   // L3228 numFramesProcessed
    pub padding: i32,                                                                // L3229 _padding_
    pub substance_change_info: MsvcVector<SubstanceChangeInfo>,                      // L3230
    pub callback_info: MsvcVector<CallbackInfo>,                                     // L3231
    pub spawn_falling_liquid_info: MsvcVector<SpawnFallingLiquidInfo>,               // L3232
    pub spawn_ore_info: MsvcVector<SpawnOreInfo>,                                    // L3233
    pub dig_info: MsvcVector<SpawnOreInfo>,                                          // L3234 (复用 SpawnOreInfo)
    pub unstable_cell_info: MsvcVector<UnstableCellInfo>,                            // L3235
    pub world_damage_info: MsvcVector<WorldDamageInfo>,                              // L3236
    pub building_temperature_info: MsvcVector<BuildingTemperatureInfo>,              // L3237
    pub mass_consumed_callbacks: MsvcVector<MassConsumedCallback>,                   // L3238
    pub mass_emitted_callbacks: MsvcVector<MassEmittedCallback>,                     // L3239
    pub disease_consumed_callbacks: MsvcVector<DiseaseConsumedCallback>,             // L3240
    pub radiation_consumed_callbacks: MsvcVector<RadiationConsumedCallback>,         // L3241
    pub spawn_fx_info: MsvcVector<SpawnFXInfo>,                                      // L3242
    pub solid_info: MsvcVector<SolidInfo>,                                           // L3243
    pub liquid_change_info: MsvcVector<LiquidChangeInfo>,                            // L3244
    pub solid_substance_change_info: MsvcVector<SolidSubstanceChangeInfo>,           // L3245
    pub consumed_mass_info: MsvcVector<ConsumedMassInfo>,                            // L3246
    pub emitted_mass_info: MsvcVector<EmittedMassInfo>,                              // L3247
    pub element_chunk_info: MsvcVector<ElementChunkInfo>,                            // L3248
    pub disease_emitted_info: MsvcVector<DiseaseEmittedInfo>,                        // L3249
    pub disease_consumed_info: MsvcVector<DiseaseConsumedInfo>,                      // L3250
    pub element_chunk_melted_info: MsvcVector<MeltedInfo>,                           // L3251 (MeltedInfo)
    pub building_melted_info: MsvcVector<MeltedInfo>,                                // L3252 (MeltedInfo)
    pub building_overheat_info: MsvcVector<MeltedInfo>,                              // L3253 (MeltedInfo)
    pub building_no_longer_overheated_info: MsvcVector<MeltedInfo>,                  // L3254 (MeltedInfo)
    pub component_state_changed_messages: MsvcVector<ComponentStateChangedMessage>,  // L3255
    pub cell_melted_info: MsvcVector<CellMeltedInfo>,                                // L3256
    pub backwall_element_changed_info: MsvcVector<BackwallElementChangedInfo>,       // L3257
    pub backwall_should_transition_info: MsvcVector<BackwallShouldTransitionInfo>,   // L3258
    pub cells: UniquePtr<CellSOA>,                                                   // L3259
    pub backwalls: UniquePtr<BackwallSOA>,                                           // L3260
    pub flow: UniquePtr<Vector4f>,                                                   // L3261
    pub accumulated_flow: UniquePtr<f32>,                                            // L3262 accumulatedFlow
    pub visible_grid: UniquePtr<u8>,                                                 // L3263 visibleGrid
    pub property_texture_flow: UniquePtr<Vector2f>,                                  // L3264 propertyTextureFlow
    pub property_texture_liquid: UniquePtr<u32>,                                     // L3265 propertyTextureLiquid
    pub property_texture_liquid_data: UniquePtr<u32>,                                // L3266 propertyTextureLiquidData
    pub property_texture_material_data: UniquePtr<u32>,                              // L3267 propertyTextureMaterialData
    pub property_texture_exposed_to_sunlight: UniquePtr<u8>,                         // L3268 propertyTextureExposedToSunlight
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn handle_size() {
        // struct Handle { int value; } = 4B
        assert_eq!(size_of::<Handle>(), 4);
    }

    #[test]
    fn info_4byte_structs() {
        // 7 个 4B 子结构
        assert_eq!(size_of::<CallbackInfo>(), 4);
        assert_eq!(size_of::<LiquidChangeInfo>(), 4);
        assert_eq!(size_of::<SolidSubstanceChangeInfo>(), 4);
        assert_eq!(size_of::<MeltedInfo>(), 4);
        assert_eq!(size_of::<CellMeltedInfo>(), 4);
        assert_eq!(size_of::<BackwallElementChangedInfo>(), 4);
        assert_eq!(size_of::<BackwallShouldTransitionInfo>(), 4);
    }

    #[test]
    fn info_8byte_structs() {
        // 8 个 8B 子结构
        assert_eq!(size_of::<SubstanceChangeInfo>(), 8);
        assert_eq!(size_of::<WorldDamageInfo>(), 8);
        assert_eq!(size_of::<BuildingTemperatureInfo>(), 8);
        assert_eq!(size_of::<SolidInfo>(), 8);
        assert_eq!(size_of::<ElementChunkInfo>(), 8);
        assert_eq!(size_of::<DiseaseEmittedInfo>(), 8);
        assert_eq!(size_of::<DiseaseConsumedInfo>(), 8);
        assert_eq!(size_of::<ComponentStateChangedMessage>(), 8);
    }

    #[test]
    fn info_12byte_structs() {
        // 3 个 12B 子结构
        assert_eq!(size_of::<DiseaseConsumedCallback>(), 12);
        assert_eq!(size_of::<RadiationConsumedCallback>(), 12);
        assert_eq!(size_of::<SpawnFXInfo>(), 12);
    }

    #[test]
    fn info_16byte_structs() {
        // 1 个 16B 子结构
        assert_eq!(size_of::<EmittedMassInfo>(), 16);
    }

    #[test]
    fn info_20byte_structs() {
        // 6 个 20B 子结构
        assert_eq!(size_of::<SpawnFallingLiquidInfo>(), 20);
        assert_eq!(size_of::<SpawnOreInfo>(), 20);
        assert_eq!(size_of::<UnstableCellInfo>(), 20);
        assert_eq!(size_of::<MassConsumedCallback>(), 20);
        assert_eq!(size_of::<MassEmittedCallback>(), 20);
        assert_eq!(size_of::<ConsumedMassInfo>(), 20);
    }

    #[test]
    fn game_data_size_matches_source() {
        // 4×i32 (16) + 29×MsvcVector (928) + 10×UniquePtr (80) = 1024B
        // MsvcVector<T> 大小与 T 无关（4 指针 = 32B）
        let actual = size_of::<GameData>();
        if actual != 1024 {
            panic!(
                "GameData size = {} (0x{:x}), 期望 1024 (0x400)。\n\
                 差异：{} 字节。请检查：\n\
                 1. MsvcVector 是否 32B（commit 618b457 已修订）\n\
                 2. GameData 是否有 29 个 MsvcVector 字段 + 10 个 UniquePtr 字段\n\
                 3. 4 个 i32 头部字段是否存在",
                actual, actual, actual as i64 - 1024
            );
        }
    }
}
