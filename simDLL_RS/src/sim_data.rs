//! SimData / CellSOA / BackwallSOA — simDLL 核心数据 struct。
//!
//! 字段对照源码：
//! - SimData: 00_types_reference.c L8389-8433（原版 0x890 = 2192 字节）
//! - CellSOA: 00_types_reference.c L11014-11026（11 个 vector，352 字节）
//! - BackwallSOA: 00_types_reference.c L9458-9464（u16 + i16 pad + 3 vector，104 字节）
//!
//! **关键修订**：MsvcVector 已在 commit 618b457 修订为 32B（MSVC iterator debugging
//! 头部），本文件所有 size 计算基于 32B MsvcVector，期望 SimData 精确达 0x890。
//!
//! 8 个 *Emitter/*Consumer/*Chunk/*HeatExchange 子结构内部 padding 内容 Phase 1
//! 不深究，用 `reserved: [u8; N]` 占位（N 来自 ~SimData dtor 偏移差值，详见各
//! struct 注释）。Phase 3 接入业务逻辑时再细化字段。

use crate::sim_events::SimEvents;
use crate::stl_shim::{MsvcVector, UniquePtr};
use crate::vector_math::Vector4f;

// ===== 辅助子结构（公开供 SimData 引用）=====

/// SimComponent 前向声明（Phase 6+ 实现）。
///
/// 源码 00_types_reference.c L7589-7598 有 8 个 undefined 字段 = 8B。
/// 本任务仅在 SimData.components 中作为 `*mut SimComponent` 的向量元素
/// 类型，SimData 内是 `MsvcVector<*mut SimComponent>` 32B，SimComponent
/// 自身大小不影响 SimData 布局。
#[repr(C)]
pub struct SimComponent {
    pub reserved: [u8; 8], // 占位，Phase 6+ 补全
}

/// DebugProperties — SimData 的调试属性。
///
/// 源码 00_types_reference.c L3992-3997：
/// - building_temperature_scale: f32
/// - building_to_building_temperature_scale: f32
/// - is_debug_editing: bool
/// - pad: bool[3]
///
/// 大小 = 4 + 4 + 1 + 3 = 12B，对齐 = 4（f32 对齐），尾 padding 0B（12 已是 4 倍数）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DebugProperties {
    pub building_temperature_scale: f32,                   // L3993
    pub building_to_building_temperature_scale: f32,       // L3994
    pub is_debug_editing: bool,                            // L3995
    pub pad: [bool; 3],                                    // L3996 bool pad[3]
}

/// WorldOffsetData — 世界偏移数据。
///
/// 源码 00_types_reference.c L35013-35018：4 个 int = 16B。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WorldOffsetData {
    pub offset_x: i32, // L35014 offsetX
    pub offset_y: i32, // L35015 offsetY
    pub width: i32,    // L35016 width
    pub height: i32,   // L35017 height
}

/// ActiveRegion — 活跃区域。
///
/// 源码 00_types_reference.c L3512-3516：
/// - region: Region（2 × Vector2<i32> = 16B，源码 L3508-3511）
/// - current_sunlight_intensity: f32
/// - current_cosmic_radiation_intensity: f32
///
/// 大小 = 16 + 4 + 4 = 24B。
///
/// **设计决策**：将 Region 内联为 4 个 i32 字段（min_x/min_y/max_x/max_y）
/// 而非单独定义 Region struct，因为 Region 仅在 ActiveRegion 内使用一次，
/// 单独定义会增加不必要的类型层次。Phase 3+ 若 Region 出现在其他位置可
/// 抽出为独立 struct。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ActiveRegion {
    pub min_x: i32,                              // L3509 region.minimum.x
    pub min_y: i32,                              // L3509 region.minimum.y
    pub max_x: i32,                              // L3510 region.maximum.x
    pub max_y: i32,                              // L3510 region.maximum.y
    pub current_sunlight_intensity: f32,         // L3514
    pub current_cosmic_radiation_intensity: f32, // L3515
}

/// Timers — 计时器数组元素。
///
/// 源码 00_types_reference.c L49187-49189：
/// - stable_cell_ticks: uchar bitfield:5（5 bits，1B 存储）
///
/// 大小 = 1B。本任务 SimData 内是 `UniquePtr<Timers>` 8B 指针，
/// Timers struct 自身大小不影响 SimData 布局。
///
/// **注意**：bitfield 在 Rust 中无原生支持，用 `u8` 存储 + 文档标注
/// 仅低 5 bits 有效。Phase 3+ 接入业务逻辑时若需位操作可改用
/// `bitfield` crate 或手写 getter/setter。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Timers {
    /// bitfield:5 —— 仅低 5 bits 有效（stableCellTicks）。
    pub stable_cell_ticks: u8, // L49188 uchar stableCellTicks:5
}

// ===== 8 个 *Emitter/*Consumer/*Chunk/*HeatExchange 子结构 =====
//
// 大小来自 ~SimData dtor 偏移差值（01_sim_api.c L6109-6174）：
// - ElementConsumer: 0x218 - 0xe0 = 312B
// - ElementEmitter: 0x350 - 0x218 = 312B
// - RadiationEmitter: 0x400 - 0x350 = 176B
// - ElementChunk: 0x4d0 - 0x400 = 208B
// - BuildingHeatExchange: 0x5a0 - 0x4d0 = 208B
// - BuildingToBuildingHeatExchange: 0x650 - 0x5a0 = 176B
// - DiseaseEmitter: 0x788 - 0x650 = 312B
// - DiseaseConsumer: 后续至 SimData 末尾 = 208B
//
// 子结构内部字段 Phase 1 不深究（源码显示 22 longlong padding + N vector 模式，
// 或 176 undefined 字段），统一用 `reserved: [u8; N]` 占位。

/// ElementConsumer — 元素消费者。
///
/// 源码 00_types_reference.c L3598-3624 有 22 longlong padding + 4 vector。
/// 实际大小 312B（dtor 偏移 0xe0=224 → 0x218=536，差值 312）。
#[repr(C)]
pub struct ElementConsumer {
    /// 占位 312B。Phase 3 接入业务逻辑时细化字段。
    pub reserved: [u8; 312],
}

/// ElementEmitter — 元素发射器。
///
/// 源码 00_types_reference.c L7803-7830 同 ElementConsumer 结构。
/// 实际大小 312B（dtor 偏移 0x218=536 → 0x350=848，差值 312）。
#[repr(C)]
pub struct ElementEmitter {
    /// 占位 312B。Phase 3 接入业务逻辑时细化字段。
    pub reserved: [u8; 312],
}

/// RadiationEmitter — 辐射发射器。
///
/// 源码 00_types_reference.c L7832-8009 有 176 undefined 字段。
/// 实际大小 176B（dtor 偏移 0x350=848 → 0x400=1024，差值 176）。
#[repr(C)]
pub struct RadiationEmitter {
    /// 占位 176B。Phase 3 接入业务逻辑时细化字段。
    pub reserved: [u8; 176],
}

/// ElementChunk — 元素块。
///
/// 源码 00_types_reference.c L8317-8341 有 22 longlong padding + 1 vector。
/// 实际大小 208B（dtor 偏移 0x400=1024 → 0x4d0=1232，差值 208）。
#[repr(C)]
pub struct ElementChunk {
    /// 占位 208B。Phase 3 接入业务逻辑时细化字段。
    pub reserved: [u8; 208],
}

/// BuildingHeatExchange — 建筑热交换。
///
/// 源码 00_types_reference.c L8225-8249 有 22 longlong padding + 1 vector。
/// 实际大小 208B（dtor 偏移 0x4d0=1232 → 0x5a0=1440，差值 208）。
#[repr(C)]
pub struct BuildingHeatExchange {
    /// 占位 208B。Phase 3 接入业务逻辑时细化字段。
    pub reserved: [u8; 208],
}

/// BuildingToBuildingHeatExchange — 建筑间热交换。
///
/// 源码 00_types_reference.c L8011-8188 有 176 undefined 字段。
/// 实际大小 176B（dtor 偏移 0x5a0=1440 → 0x650=1616，差值 176）。
#[repr(C)]
pub struct BuildingToBuildingHeatExchange {
    /// 占位 176B。Phase 3 接入业务逻辑时细化字段。
    pub reserved: [u8; 176],
}

/// DiseaseEmitter — 病原体发射器。
///
/// 源码 00_types_reference.c L8288-8315 同 ElementEmitter 结构。
/// 实际大小 312B（dtor 偏移 0x650=1616 → 0x788=1928，差值 312）。
#[repr(C)]
pub struct DiseaseEmitter {
    /// 占位 312B。Phase 3 接入业务逻辑时细化字段。
    pub reserved: [u8; 312],
}

/// DiseaseConsumer — 病原体消费者。
///
/// 源码 00_types_reference.c L8251-8275 有 22 longlong padding + 1 vector。
/// 实际大小 208B（dtor 偏移 0x788=1928 → SimData 末尾，差值 208）。
#[repr(C)]
pub struct DiseaseConsumer {
    /// 占位 208B。Phase 3 接入业务逻辑时细化字段。
    pub reserved: [u8; 208],
}

// ===== BackwallSOA — 后墙 SOA 数组 =====

/// BackwallSOA — 后墙 Structure-of-Arrays。
///
/// 字段对照源码 00_types_reference.c L9458-9464：
/// - vacuum_element_idx: u16 (2B @ 0)
/// - _padding_: i16 (2B @ 2，源码显式声明)
/// - (4B 隐式 padding 至 8B 对齐，因 MsvcVector 对齐 = 8)
/// - element_idx: MsvcVector<u16> (32B @ 8)
/// - mass: MsvcVector<f32> (32B @ 40)
/// - temperature: MsvcVector<f32> (32B @ 72)
///
/// 大小 = 8 + 96 = 104B。
#[repr(C)]
pub struct BackwallSOA {
    pub vacuum_element_idx: u16,        // L9459
    pub padding: i16,                    // L9460 short _padding_
    // 4B 隐式 padding（#[repr(C)] 自动对齐 MsvcVector 到 8B）
    pub element_idx: MsvcVector<u16>,   // L9461
    pub mass: MsvcVector<f32>,          // L9462
    pub temperature: MsvcVector<f32>,   // L9463
}

// ===== CellSOA — 主格子 SOA 数组 =====

/// CellSOA — 单元格 Structure-of-Arrays。
///
/// 字段对照源码 00_types_reference.c L11014-11026（11 个 vector，全部 32B）：
/// - element_idx: vector<u16>
/// - temperature: vector<float>
/// - mass: vector<float>
/// - properties: vector<u8>
/// - insulation: vector<u8>
/// - strength_info: vector<u8>
/// - disease_idx: vector<u8>
/// - disease_count: vector<int>
/// - disease_infestation_tick_count: vector<u8>
/// - disease_growth_accumulated_error: vector<float>
/// - radiation: vector<float>
///
/// 大小 = 11 × 32 = 352B。
#[repr(C)]
pub struct CellSOA {
    pub element_idx: MsvcVector<u16>,                       // L11015
    pub temperature: MsvcVector<f32>,                       // L11016
    pub mass: MsvcVector<f32>,                              // L11017
    pub properties: MsvcVector<u8>,                         // L11018
    pub insulation: MsvcVector<u8>,                         // L11019
    pub strength_info: MsvcVector<u8>,                      // L11020
    pub disease_idx: MsvcVector<u8>,                        // L11021
    pub disease_count: MsvcVector<i32>,                     // L11022
    pub disease_infestation_tick_count: MsvcVector<u8>,     // L11023
    pub disease_growth_accumulated_error: MsvcVector<f32>,  // L11024
    pub radiation: MsvcVector<f32>,                         // L11025
}

// ===== SimData — simDLL 核心数据 struct =====

/// SimData — 模拟核心数据。
///
/// 字段对照源码 00_types_reference.c L8389-8433（44 字段，末尾 = 0x890=2192B）。
///
/// **大小验证**：基于 32B MsvcVector（commit 618b457 修订），所有偏移与
/// 运行时访问模式（02_save_load.c）+ dtor 偏移（01_sim_api.c L6109-6174）
/// 交叉验证，期望精确达 0x890。
#[repr(C)]
pub struct SimData {
    pub width: i32,                                          // L8390
    pub height: i32,                                         // L8391
    pub num_game_cells: i32,                                 // L8392
    pub random_seed: u32,                                    // L8393
    pub iterate_direction: i32,                              // L8394
    pub displacement_direction: i32,                         // L8395
    pub saved_options: u8,                                   // L8396
    // 1B 隐式 padding（对齐 u16 到 2B）
    pub void_element_idx: u16,                               // L8397
    pub vacuum_element_idx: u16,                             // L8398
    pub unobtanium_element_idx: u16,                         // L8399
    pub cells: UniquePtr<CellSOA>,                           // L8400
    pub updated_cells: UniquePtr<CellSOA>,                   // L8401
    pub backwall: UniquePtr<BackwallSOA>,                    // L8402
    pub world_zones: UniquePtr<u8>,                          // L8403
    pub cosmic_radiation_occlusion: MsvcVector<f32>,         // L8404
    pub accumulated_flow: UniquePtr<f32>,                    // L8405
    pub flow: UniquePtr<Vector4f>,                           // L8406
    pub timers: UniquePtr<Timers>,                           // L8407
    pub active_regions: MsvcVector<ActiveRegion>,            // L8408
    pub visible_grid: UniquePtr<u8>,                         // L8409
    pub tick_count: u16,                                     // L8410
    // 2B 隐式 padding（对齐 i32 到 4B）
    pub padding_1: i32,                                      // L8411 _padding_
    pub sim_events: UniquePtr<SimEvents>,                    // L8412
    pub debug_properties: DebugProperties,                   // L8413
    pub padding_2: i32,                                      // L8414 _padding_
    pub components: MsvcVector<*mut SimComponent>,           // L8415
    pub element_consumer: ElementConsumer,                   // L8416
    pub element_emitter: ElementEmitter,                     // L8417
    pub radiation_emitter: RadiationEmitter,                // L8418
    pub element_chunk: ElementChunk,                         // L8419
    pub building_heat_exchange: BuildingHeatExchange,        // L8420
    pub building_to_building_heat_exchange: BuildingToBuildingHeatExchange, // L8421
    pub disease_emitter: DiseaseEmitter,                    // L8422
    pub disease_consumer: DiseaseConsumer,                  // L8423
    pub init_settle_thermal_boundaries: bool,                // L8424
    pub headless: bool,                                     // L8425
    pub radiation_enabled: bool,                            // L8426
    // 1B 隐式 padding（对齐 f32 到 4B）
    pub radiation_linger_rate: f32,                         // L8427 RADIATION_LINGER_RATE
    pub radiation_max_mass: f32,                            // L8428 RADIATION_MAX_MASS
    pub radiation_base_weight: f32,                         // L8429 RADIATION_BASE_WEIGHT
    pub radiation_density_weight: f32,                      // L8430 RADIATION_DENSITY_WEIGHT
    pub radiation_constructed_factor: f32,                  // L8431 RADIATION_CONSTRUCTED_FACTOR
    pub worlds: MsvcVector<WorldOffsetData>,                // L8432
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn debug_properties_size_matches_source() {
        // 2×f32 (8) + bool (1) + bool[3] (3) = 12B
        assert_eq!(size_of::<DebugProperties>(), 12);
    }

    #[test]
    fn world_offset_data_size_matches_source() {
        // 4 × i32 = 16B
        assert_eq!(size_of::<WorldOffsetData>(), 16);
    }

    #[test]
    fn active_region_size_matches_source() {
        // 4 × i32 (16) + 2 × f32 (8) = 24B
        assert_eq!(size_of::<ActiveRegion>(), 24);
    }

    #[test]
    fn timers_size_matches_source() {
        // uchar bitfield:5 = 1B（仅 5 bits 有效，存储为 1B）
        assert_eq!(size_of::<Timers>(), 1);
    }

    #[test]
    fn sim_component_size_matches_source() {
        // 8 undefined 字段 = 8B
        assert_eq!(size_of::<SimComponent>(), 8);
    }

    #[test]
    fn element_consumer_size_matches_dtor_offset() {
        // dtor 0xe0=224 → 0x218=536，差值 312B
        assert_eq!(size_of::<ElementConsumer>(), 312);
    }

    #[test]
    fn element_emitter_size_matches_dtor_offset() {
        // dtor 0x218=536 → 0x350=848，差值 312B
        assert_eq!(size_of::<ElementEmitter>(), 312);
    }

    #[test]
    fn radiation_emitter_size_matches_dtor_offset() {
        // dtor 0x350=848 → 0x400=1024，差值 176B
        assert_eq!(size_of::<RadiationEmitter>(), 176);
    }

    #[test]
    fn element_chunk_size_matches_dtor_offset() {
        // dtor 0x400=1024 → 0x4d0=1232，差值 208B
        assert_eq!(size_of::<ElementChunk>(), 208);
    }

    #[test]
    fn building_heat_exchange_size_matches_dtor_offset() {
        // dtor 0x4d0=1232 → 0x5a0=1440，差值 208B
        assert_eq!(size_of::<BuildingHeatExchange>(), 208);
    }

    #[test]
    fn building_to_building_heat_exchange_size_matches_dtor_offset() {
        // dtor 0x5a0=1440 → 0x650=1616，差值 176B
        assert_eq!(size_of::<BuildingToBuildingHeatExchange>(), 176);
    }

    #[test]
    fn disease_emitter_size_matches_dtor_offset() {
        // dtor 0x650=1616 → 0x788=1928，差值 312B
        assert_eq!(size_of::<DiseaseEmitter>(), 312);
    }

    #[test]
    fn disease_consumer_size_matches_dtor_offset() {
        // dtor 0x788=1928 → SimData 末尾 2136，差值 208B
        assert_eq!(size_of::<DiseaseConsumer>(), 208);
    }

    #[test]
    fn cell_soa_size_matches_source() {
        // 11 × MsvcVector (32) = 352B
        // （原计划简报写 264B 基于 24B vector，commit 618b457 修订后改 352B）
        assert_eq!(size_of::<CellSOA>(), 352);
    }

    #[test]
    fn backwall_soa_size_matches_source() {
        // u16 (2) + i16 (2) + 4B pad + 3 × MsvcVector (96) = 104B
        assert_eq!(size_of::<BackwallSOA>(), 104);
    }

    #[test]
    fn sim_data_size_matches_original() {
        // 原版 0x890 = 2192 字节
        // 基于 32B MsvcVector（commit 618b457）+ 8 子结构实际 dtor 偏移差值
        // 已交叉验证 02_save_load.c 运行时访问模式 + 01_sim_api.c dtor 偏移
        let actual = size_of::<SimData>();
        if actual != 0x890 {
            panic!(
                "SimData size = {:#x} ({}), 期望 0x890 (2192)。\n\
                 差异：{} 字节。请检查：\n\
                 1. MsvcVector 是否 32B（commit 618b457 已修订）\n\
                 2. 8 子结构 reserved 数组大小是否与 dtor 偏移差值一致\n\
                 3. BackwallSOA u16+i16+pad+3×MsvcVector 是否 104B\n\
                 4. DebugProperties 是否 12B\n\
                 5. CellSOA 11 个 MsvcVector 是否 352B",
                actual, actual, actual as i64 - 0x890
            );
        }
    }
}
