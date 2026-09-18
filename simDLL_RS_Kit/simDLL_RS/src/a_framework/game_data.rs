//! GameData — simDLL 游戏数据 struct（每帧同步给 C# 的数据）。
//!
//! 字段对照源码：00_types_reference.c L3225-3269。
//! 25 个 *Info/*Callback 子结构 + Handle 辅助结构。
//! MsvcVector<T> 大小与 T 无关（4 指针 = 32B），故 *Info 子结构大小不影响 GameData 总大小。
//! GameData 精确 = 1024B = 0x400。

use crate::a_framework::sim_data::{BackwallSOA, CellSOA};
use crate::a_framework::stl_shim::{MsvcVector, UniquePtr};
use crate::a_framework::vector_math::{Vector2f, Vector4f};

// ===== Handle（4B）=====

/// Handle — 游戏对象句柄。源码 L1801-1803。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Handle {
    pub value: i32,
}

// ===== 25 个 *Info/*Callback 子结构 =====

// --- 4B 子结构（7 个）---

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CallbackInfo { pub callback_idx: i32 }  // L12207

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LiquidChangeInfo { pub cell_idx: i32 }  // L9785

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SolidSubstanceChangeInfo { pub cell_idx: i32 }  // L12749

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MeltedInfo { pub handle: Handle }  // L7184

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CellMeltedInfo { pub game_cell: u32 }  // L11930

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BackwallElementChangedInfo { pub game_cell: u32 }  // L8738

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BackwallShouldTransitionInfo { pub game_cell: u32 }  // L12759

// --- 8B 子结构（8 个）---

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SubstanceChangeInfo {  // L1848-1850
    pub cell_idx: i32,
    pub old_element_idx: u16,
    pub new_element_idx: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WorldDamageInfo {  // L7439-7440
    pub cell_idx: i32,
    pub damage_source_cell_idx: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BuildingTemperatureInfo {  // L9482-9483
    pub handle: Handle,
    pub temperature: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SolidInfo {  // L12452-12453
    pub cell_idx: i32,
    pub solid: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementChunkInfo {  // L2065-2066
    pub temperature: f32,
    pub delta_kj: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DiseaseEmittedInfo {  // L12763-12765
    pub disease_idx: u8,
    pub padding: [u8; 3],
    pub count: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DiseaseConsumedInfo {  // L1042-1044
    pub disease_idx: u8,
    pub padding: [u8; 3],
    pub count: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ComponentStateChangedMessage {  // L7529-7530
    pub callback_idx: i32,
    pub sim_handle: i32,
}

// --- 12B 子结构（3 个）---

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DiseaseConsumedCallback {  // L1856-1859
    pub callback_idx: i32,
    pub disease_idx: u8,
    pub pad: [u8; 3],
    pub disease_count: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RadiationConsumedCallback {  // L12753-12755
    pub callback_idx: i32,
    pub game_cell: i32,
    pub radiation: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SpawnFXInfo {  // L7251-7253
    pub cell_idx: i32,
    pub fx_id: i32,
    pub rotation: f32,
}

// --- 16B 子结构（1 个）---

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EmittedMassInfo {  // L11936-11941
    pub elem_idx: u16,
    pub disease_idx: u8,
    pub pad: u8,
    pub mass: f32,
    pub temperature: f32,
    pub disease_count: i32,
}

// --- 20B 子结构（6 个）---

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SpawnFallingLiquidInfo {  // L12546-12552
    pub cell_idx: i32,
    pub element_idx: u16,
    pub disease_idx: u8,
    pub pad: u8,
    pub mass: f32,
    pub temperature: f32,
    pub disease_count: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SpawnOreInfo {  // L5974-5980（dig_info 也复用此类型）
    pub cell_idx: i32,
    pub elem_idx: u16,
    pub disease_idx: u8,
    pub pad: u8,
    pub mass: f32,
    pub temperature: f32,
    pub disease_count: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct UnstableCellInfo {  // L12524-12530
    pub cell_idx: i32,
    pub elem_idx: u16,
    pub falling_info: u8,
    pub disease_idx: u8,
    pub mass: f32,
    pub temperature: f32,
    pub disease_count: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MassConsumedCallback {  // L10128-10134
    pub callback_idx: i32,
    pub elem_idx: u16,
    pub disease_idx: u8,
    pub pad: u8,
    pub mass: f32,
    pub temperature: f32,
    pub disease_count: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MassEmittedCallback {  // L12739-12745
    pub callback_idx: i32,
    pub elem_idx: u16,
    pub emitted: u8,
    pub disease_idx: u8,
    pub mass: f32,
    pub temperature: f32,
    pub disease_count: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ConsumedMassInfo {  // L1806-1812
    pub sim_handle: Handle,
    pub removed_elem_idx: u16,
    pub disease_idx: u8,
    pub pad: u8,
    pub mass: f32,
    pub temperature: f32,
    pub disease_count: i32,
}

// ===== GameData =====

/// GameData — 每帧同步给 C# 的游戏数据（1024B = 0x400）。
/// 字段对照源码 00_types_reference.c L3225-3269。
#[repr(C)]
pub struct GameData {
    pub width: i32,                                                                  // L3226
    pub height: i32,                                                                 // L3227
    pub num_frames_processed: i32,                                                   // L3228
    pub padding: i32,                                                                // L3229
    pub substance_change_info: MsvcVector<SubstanceChangeInfo>,                      // L3230
    pub callback_info: MsvcVector<CallbackInfo>,                                     // L3231
    pub spawn_falling_liquid_info: MsvcVector<SpawnFallingLiquidInfo>,               // L3232
    pub spawn_ore_info: MsvcVector<SpawnOreInfo>,                                    // L3233
    pub dig_info: MsvcVector<SpawnOreInfo>,                                          // L3234（复用 SpawnOreInfo）
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
    pub element_chunk_melted_info: MsvcVector<MeltedInfo>,                           // L3251
    pub building_melted_info: MsvcVector<MeltedInfo>,                                // L3252
    pub building_overheat_info: MsvcVector<MeltedInfo>,                              // L3253
    pub building_no_longer_overheated_info: MsvcVector<MeltedInfo>,                  // L3254
    pub component_state_changed_messages: MsvcVector<ComponentStateChangedMessage>,  // L3255
    pub cell_melted_info: MsvcVector<CellMeltedInfo>,                                // L3256
    pub backwall_element_changed_info: MsvcVector<BackwallElementChangedInfo>,       // L3257
    pub backwall_should_transition_info: MsvcVector<BackwallShouldTransitionInfo>,   // L3258
    pub cells: UniquePtr<CellSOA>,                                                   // L3259
    pub backwalls: UniquePtr<BackwallSOA>,                                           // L3260
    pub flow: UniquePtr<Vector4f>,                                                   // L3261
    pub accumulated_flow: UniquePtr<f32>,                                            // L3262
    pub visible_grid: UniquePtr<u8>,                                                 // L3263
    pub property_texture_flow: UniquePtr<Vector2f>,                                  // L3264
    pub property_texture_liquid: UniquePtr<u32>,                                     // L3265
    pub property_texture_liquid_data: UniquePtr<u32>,                                // L3266
    pub property_texture_material_data: UniquePtr<u32>,                              // L3267
    pub property_texture_exposed_to_sunlight: UniquePtr<u8>,                         // L3268
}

impl GameData {
    /// 构造函数 — 对照源码 11_msvcrt_ignored.c L27089-27480（GameData::GameData）。
    ///
    /// 参数：game_width = 宽度（不含边界），game_height = 高度（不含边界）。
    /// 分配 CellSOA（game_width × game_height 个 cell，清零）+
    /// BackwallSOA（同尺寸，element_idx 填 0xffff）。
    /// 29 个事件 MsvcVector 初始化为空。flow/accumulatedFlow/visibleGrid 等 UniquePtr 置 null。
    pub fn new(game_width: i32, game_height: i32) -> Self {
        let mut gd: GameData = unsafe { std::mem::zeroed() };
        gd.width = game_width;
        gd.height = game_height;
        gd.num_frames_processed = 0;

        let total_cells = (game_width as usize) * (game_height as usize);

        // 分配 CellSOA（源码 L27413-27423: operator_new(0x160) + CellSOA::CellSOA(_, count)）
        let cells = Box::new(CellSOA::with_size(total_cells));
        gd.cells = UniquePtr { ptr: Box::into_raw(cells) };

        // 分配 BackwallSOA（源码 L27424-27433: operator_new(100) + BackwallSOA::BackwallSOA(_, count, 0xffff)）
        let backwalls = Box::new(BackwallSOA::with_size(total_cells, 0xffff));
        gd.backwalls = UniquePtr { ptr: Box::into_raw(backwalls) };

        // 分配 property texture 内存（C# PropertyTextures.LoadRawTextureData 需要有效指针）
        // 大小对照 C# PropertyTextures.cs L507-519：
        //   Flow: 8 * W * H 字节（Vector2f = 8 字节）
        //   Liquid/LiquidData/MaterialData: 4 * W * H 字节
        //   ExposedToSunlight: 1 * W * H 字节
        //   AccumulatedFlowValues: 4 * W * H 字节
        // 零初始化，C# 端 LoadRawTextureData 读取后 Apply 到纹理。
        // 2026-08-04 审查修正：分配 flow（Vector4f，16B/格）——原版 GameData::GameData
        // 11_msvcrt_ignored.c L27433-27458 分配 0x10×count 并零初始化。C# 不消费该字段
        // （死字段），但按原版构造补齐。
        let flow_vec = vec![crate::a_framework::vector_math::Vector4f::default(); total_cells];
        gd.flow = UniquePtr { ptr: flow_vec.leak().as_mut_ptr() };
        let flow_vec = vec![crate::a_framework::vector_math::Vector2f::default(); total_cells];
        gd.property_texture_flow = UniquePtr {
            ptr: flow_vec.leak().as_mut_ptr()
        };
        let liquid_tex: Vec<u32> = vec![0; total_cells];
        gd.property_texture_liquid = UniquePtr {
            ptr: liquid_tex.leak().as_mut_ptr()
        };
        let liquid_data: Vec<u32> = vec![0; total_cells];
        gd.property_texture_liquid_data = UniquePtr {
            ptr: liquid_data.leak().as_mut_ptr()
        };
        let material_data: Vec<u32> = vec![0; total_cells];
        gd.property_texture_material_data = UniquePtr {
            ptr: material_data.leak().as_mut_ptr()
        };
        let sunlight: Vec<u8> = vec![0; total_cells];
        gd.property_texture_exposed_to_sunlight = UniquePtr {
            ptr: sunlight.leak().as_mut_ptr()
        };
        let accumulated: Vec<f32> = vec![0.0; total_cells];
        gd.accumulated_flow = UniquePtr {
            ptr: accumulated.leak().as_mut_ptr()
        };
        // visible_grid（液滴轨道前置，2026-08-02）：与 SimData.visible_grid 每帧交换
        //（原版 GameData::swapVisibleGrid，L127019/L132390）。初始全 0xFF（可见兜底）。
        let visible: Vec<u8> = vec![0xFF; total_cells];
        gd.visible_grid = UniquePtr {
            ptr: visible.leak().as_mut_ptr()
        };

        gd
    }
}

impl Default for GameData {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl Drop for GameData {
    fn drop(&mut self) {
        // 释放 CellSOA
        if !self.cells.ptr.is_null() {
            unsafe { let _ = Box::from_raw(self.cells.ptr); }
            self.cells.ptr = std::ptr::null_mut();
        }
        // 释放 BackwallSOA
        if !self.backwalls.ptr.is_null() {
            unsafe { let _ = Box::from_raw(self.backwalls.ptr); }
            self.backwalls.ptr = std::ptr::null_mut();
        }
        // 释放 new() 中 vec![...].leak() 分配的缓冲区（len == cap == width × height）。
        // 2026-08-03 修正：此前仅释放 cells/backwalls，7 个 texture/visible/accumulated
        // 缓冲区泄漏（旧注释"保持 null"已过时——new() 已实际分配）。
        // 与 SimData::Drop 的 Vec::from_raw_parts 释放模式一致（sim_data.rs L745-754）。
        let total = (self.width as usize).saturating_mul(self.height as usize);
        macro_rules! free_leaked_vec {
            ($field:ident) => {
                if !self.$field.ptr.is_null() {
                    unsafe { let _ = Vec::from_raw_parts(self.$field.ptr, total, total); }
                    self.$field.ptr = std::ptr::null_mut();
                }
            };
        }
        free_leaked_vec!(property_texture_flow);
        free_leaked_vec!(property_texture_liquid);
        free_leaked_vec!(property_texture_liquid_data);
        free_leaked_vec!(property_texture_material_data);
        free_leaked_vec!(property_texture_exposed_to_sunlight);
        free_leaked_vec!(flow);
        free_leaked_vec!(accumulated_flow);
        free_leaked_vec!(visible_grid);
        // 29 个事件 MsvcVector 的内存在 resize 时分配，Drop 时需要 clear
        // 但 GameData 通常通过 Box::from_raw 释放，CellSOA/BackwallSOA 的 Drop 会清理 MsvcVector
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn handle_size_is_4() { assert_eq!(size_of::<Handle>(), 4); }

    #[test]
    fn info_4byte_structs() {
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
        assert_eq!(size_of::<DiseaseConsumedCallback>(), 12);
        assert_eq!(size_of::<RadiationConsumedCallback>(), 12);
        assert_eq!(size_of::<SpawnFXInfo>(), 12);
    }

    #[test]
    fn info_16byte_structs() { assert_eq!(size_of::<EmittedMassInfo>(), 16); }

    #[test]
    fn info_20byte_structs() {
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
        let actual = size_of::<GameData>();
        assert_eq!(actual, 1024, "GameData size = {} (0x{:x}), 期望 1024 (0x400)", actual, actual);
    }

    /// 2026-08-04 审查修正：GameData::GameData 分配 flow（Vector4f，16B/格，
    /// 原版 11_msvcrt_ignored.c L27433-27458）。此前 new() 未分配 → flow 恒 null。
    #[test]
    fn new_allocates_flow_buffer() {
        let gd = GameData::new(4, 3);
        assert!(!gd.flow.ptr.is_null(), "flow 应分配（Vector4f × 12）");
        unsafe {
            let flow = std::slice::from_raw_parts(gd.flow.ptr, 12);
            for f in flow {
                assert_eq!(f.x, 0.0);
                assert_eq!(f.y, 0.0);
                assert_eq!(f.z, 0.0);
                assert_eq!(f.w, 0.0);
            }
        }
        // Drop 释放所有缓冲区（含 flow）——此处仅验证指针已分配
        drop(gd);
    }
}
