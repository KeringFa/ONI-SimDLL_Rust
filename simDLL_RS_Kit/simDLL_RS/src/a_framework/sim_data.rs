//! SimData / CellSOA / BackwallSOA — simDLL 核心数据 struct。
//!
//! 字段对照源码：
//! - SimData: 00_types_reference.c L8389-8433（原版 0x890 = 2192 字节）
//! - CellSOA: 00_types_reference.c L11014-11026（11 个 vector，352 字节）
//! - BackwallSOA: 00_types_reference.c L9458-9464（u16 + i16 pad + 3 vector，104 字节）
//!
//! 8 个 *Emitter/*Consumer/*Chunk/*HeatExchange 子结构内部 padding 内容 A2
//! 不深究，用 `reserved: [u8; N]` 占位（N 来自 ~SimData dtor 偏移差值）。

use crate::a_framework::sim_events::SimEvents;
use crate::a_framework::stl_shim::{MsvcVector, UniquePtr};
use crate::a_framework::vector_math::Vector4f;
use crate::b_elements::elements_table;

// ===== 辅助子结构 =====

/// SimComponent 前向声明（A2 占位 8B）。
#[repr(C)]
pub struct SimComponent {
    pub reserved: [u8; 8],
}

/// DebugProperties — SimData 的调试属性（12B）。
/// 源码 00_types_reference.c L3992-3997。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DebugProperties {
    pub building_temperature_scale: f32,
    pub building_to_building_temperature_scale: f32,
    pub is_debug_editing: bool,
    pub pad: [bool; 3],
}

/// WorldOffsetData — 世界偏移数据（16B）。
/// 源码 00_types_reference.c L35013-35018。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WorldOffsetData {
    pub offset_x: i32,
    pub offset_y: i32,
    pub width: i32,
    pub height: i32,
}

/// ActiveRegion — 活跃区域（24B）。
/// 源码 00_types_reference.c L3512-3516。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ActiveRegion {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub current_sunlight_intensity: f32,
    pub current_cosmic_radiation_intensity: f32,
}

/// Timers — 计时器数组元素（1B）。
/// 源码 00_types_reference.c L49187-49189。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Timers {
    pub stable_cell_ticks: u8, // bitfield:5，仅低 5 bits 有效
}

// ===== 8 个 padding 子结构 =====
// 大小来自 ~SimData dtor 偏移差值（01_sim_api.c L6109-6174）：
// ElementConsumer: 312B, ElementEmitter: 312B, RadiationEmitter: 176B,
// ElementChunk: 208B, BuildingHeatExchange: 208B,
// BuildingToBuildingHeatExchange: 176B, DiseaseEmitter: 312B, DiseaseConsumer: 208B

#[repr(C)]
/// ElementConsumerData — 消费者注册条目（16B，原版 SimDLL_Source.c L3592+）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementConsumerEntry {
    pub consumption_rate: f32,   // @0
    pub max_depth: u8,           // @4 半径
    pub configuration: u8,       // @5 0=指定元素 1=任意液体 2=任意气体
    pub element_idx: u16,        // @6
    pub offset_idx: u8,          // @8（每次消费+1，选元素轮转）
    pub field_0x9: u8,           // @9
    pub cell_y: u16,             // @10
    pub cell_x: u16,             // @12（0xFFFF = 墓碑，槽位可复用）
    pub field_0xe: u16,          // @14
}

/// ElementConsumer — 元素消费者 sim 组件（原版 L3592-3618）。
/// 2026-08-02 实现：水泵/涡轮等消费者注册表 + 逐帧 BFS 消费 + consumedMassInfo 回传。
#[repr(C)]
pub struct ElementConsumer {
    pub registry: MsvcVector<ElementConsumerEntry>,
    pub consumed_mass_info: MsvcVector<crate::a_framework::game_data::ConsumedMassInfo>,
    // 保持与原版 SimData 布局总尺寸一致（0x890 固定）：
    // 32（registry）+ 32（consumed_mass_info）+ 248 填充 = 312B
    pub _pad: [u8; 248],
}

/// ElementEmitter — 元素发射器 sim 组件（原版 L3621+，喷口/排液口）。
/// 2026-08-02：先补充 emittedMassInfo 容器供事件交换；发射逻辑后续实现。
#[repr(C)]
pub struct ElementEmitter {
    pub emitted_mass_info: MsvcVector<crate::a_framework::game_data::EmittedMassInfo>,
    // 32 + 280 填充 = 312B（保持 SimData 布局尺寸一致）
    pub _pad: [u8; 280],
}

#[repr(C)]
pub struct RadiationEmitter { pub reserved: [u8; 176] }

#[repr(C)]
pub struct ElementChunk { pub reserved: [u8; 208] }

#[repr(C)]
pub struct BuildingHeatExchange { pub reserved: [u8; 208] }

#[repr(C)]
pub struct BuildingToBuildingHeatExchange { pub reserved: [u8; 176] }

#[repr(C)]
pub struct DiseaseEmitter { pub reserved: [u8; 312] }

#[repr(C)]
pub struct DiseaseConsumer { pub reserved: [u8; 208] }

// ===== BackwallSOA =====

/// BackwallSOA — 后墙 Structure-of-Arrays（104B）。
/// 源码 00_types_reference.c L9458-9464。
#[repr(C)]
pub struct BackwallSOA {
    pub vacuum_element_idx: u16,        // L9459
    pub padding: i16,                    // L9460 _padding_
    // 4B 隐式 padding（对齐 MsvcVector 到 8B）
    pub element_idx: MsvcVector<u16>,   // L9461
    pub mass: MsvcVector<f32>,          // L9462
    pub temperature: MsvcVector<f32>,   // L9463
}

impl BackwallSOA {
    /// 构造空 BackwallSOA（3 个 MsvcVector 全空 + vacuum_element_idx=0）。
    pub fn new() -> Self {
        Self {
            vacuum_element_idx: 0,
            padding: 0,
            element_idx: MsvcVector::new(),
            mass: MsvcVector::new(),
            temperature: MsvcVector::new(),
        }
    }

    /// 构造具有 `n` 个元素的 BackwallSOA。
    /// element_idx 每个填 `fill`（原版默认 0xffff = 真空元素），mass/temperature 清零。
    /// 2026-08-04 修复：原版 BackwallSOA::BackwallSOA(this, count, vacuumElementIdx)
    /// 会把 vacuumElementIdx 存入首字段（11_msvcrt_ignored.c L21506-21507）；
    /// 此前漏设 → vacuum_element_idx 恒 0（太空判定/背墙逻辑依赖该字段）。
    pub fn with_size(n: usize, fill: u16) -> Self {
        let mut s = BackwallSOA::new();
        s.vacuum_element_idx = fill;
        s.element_idx.resize(n, fill);
        s.mass.resize(n, 0.0f32);
        s.temperature.resize(n, 0.0f32);
        s
    }
}

impl Default for BackwallSOA {
    fn default() -> Self { Self::new() }
}

impl BackwallSOA {
    /// CopyFrom — 从 source 拷贝所有字段到 self。
    /// 对照源码 CellSOA::CopyFrom 的模式（11_msvcrt_ignored.c L3352-3410）。
    ///
    /// 对每个 MsvcVector：resize 到 source 的长度，然后 memcpy 数据。
    pub fn copy_from(&mut self, source: &BackwallSOA) {
        // element_idx (u16)
        let len = source.element_idx.len();
        self.element_idx.resize(len, 0u16);
        unsafe {
            std::ptr::copy_nonoverlapping(source.element_idx.begin, self.element_idx.begin, len);
        }
        // mass (f32)
        let len = source.mass.len();
        self.mass.resize(len, 0.0f32);
        unsafe {
            std::ptr::copy_nonoverlapping(source.mass.begin, self.mass.begin, len);
        }
        // temperature (f32)
        let len = source.temperature.len();
        self.temperature.resize(len, 0.0f32);
        unsafe {
            std::ptr::copy_nonoverlapping(source.temperature.begin, self.temperature.begin, len);
        }
        // vacuum_element_idx 也一并拷贝
        self.vacuum_element_idx = source.vacuum_element_idx;
    }

    /// 逐行拷贝（去掉边界）—— CopySimDataToGame 专用。
    ///
    /// 对照源码 11_msvcrt_ignored.c L31441-31476（CopySimDataToGame 的 copyBackwallTasks）。
    /// 逻辑与 CellSOA::copy_rows_from_stripped 相同，但只处理 3 个字段。
    pub fn copy_rows_from_stripped(
        &mut self,
        source: &BackwallSOA,
        game_w: usize,
        game_h: usize,
        sim_w: usize,
    ) {
        let total_game_cells = game_w * game_h;
        self.element_idx.resize(total_game_cells, 0u16);
        self.mass.resize(total_game_cells, 0.0f32);
        self.temperature.resize(total_game_cells, 0.0f32);
        self.vacuum_element_idx = source.vacuum_element_idx;

        for row in 0..game_h {
            let src_start = (sim_w + 1) + row * sim_w;
            let dst_start = row * game_w;

            unsafe {
                std::ptr::copy_nonoverlapping(
                    source.element_idx.begin.add(src_start),
                    self.element_idx.begin.add(dst_start),
                    game_w,
                );
                std::ptr::copy_nonoverlapping(
                    source.mass.begin.add(src_start),
                    self.mass.begin.add(dst_start),
                    game_w,
                );
                std::ptr::copy_nonoverlapping(
                    source.temperature.begin.add(src_start),
                    self.temperature.begin.add(dst_start),
                    game_w,
                );
            }
        }
    }
}

impl Drop for BackwallSOA {
    fn drop(&mut self) {
        self.element_idx.clear();
        self.mass.clear();
        self.temperature.clear();
    }
}

// ===== CellSOA =====

/// CellSOA — 单元格 Structure-of-Arrays（352B = 11 × 32B）。
/// 源码 00_types_reference.c L11014-11026。
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

impl CellSOA {
    /// 构造空 CellSOA（11 个 MsvcVector 全空）。
    pub fn new() -> Self {
        Self {
            element_idx: MsvcVector::new(),
            temperature: MsvcVector::new(),
            mass: MsvcVector::new(),
            properties: MsvcVector::new(),
            insulation: MsvcVector::new(),
            strength_info: MsvcVector::new(),
            disease_idx: MsvcVector::new(),
            disease_count: MsvcVector::new(),
            disease_infestation_tick_count: MsvcVector::new(),
            disease_growth_accumulated_error: MsvcVector::new(),
            radiation: MsvcVector::new(),
        }
    }

    /// 构造具有 `n` 个元素的 CellSOA，所有字段清零。
    pub fn with_size(n: usize) -> Self {
        let mut s = CellSOA::new();
        s.element_idx.resize(n, 0u16);
        s.temperature.resize(n, 0.0f32);
        s.mass.resize(n, 0.0f32);
        s.properties.resize(n, 0u8);
        s.insulation.resize(n, 0u8);
        s.strength_info.resize(n, 0u8);
        s.disease_idx.resize(n, 0u8);
        s.disease_count.resize(n, 0i32);
        s.disease_infestation_tick_count.resize(n, 0u8);
        s.disease_growth_accumulated_error.resize(n, 0.0f32);
        s.radiation.resize(n, 0.0f32);
        s
    }
}

impl Default for CellSOA {
    fn default() -> Self { Self::new() }
}

impl CellSOA {
    /// CopyFrom — 从 source 拷贝所有字段到 self。
    /// 对照源码 11_msvcrt_ignored.c L3352-3410（CellSOA::CopyFrom）。
    ///
    /// __thiscall CopyFrom(this, param_1)：this=dest, param_1=source。
    /// 对每个 MsvcVector：resize 到 source 的长度，然后 memcpy 数据。
    pub fn copy_from(&mut self, source: &CellSOA) {
        // element_idx (u16)
        let len = source.element_idx.len();
        self.element_idx.resize(len, 0u16);
        unsafe {
            std::ptr::copy_nonoverlapping(source.element_idx.begin, self.element_idx.begin, len);
        }
        // temperature (f32)
        let len = source.temperature.len();
        self.temperature.resize(len, 0.0f32);
        unsafe {
            std::ptr::copy_nonoverlapping(source.temperature.begin, self.temperature.begin, len);
        }
        // mass (f32)
        let len = source.mass.len();
        self.mass.resize(len, 0.0f32);
        unsafe {
            std::ptr::copy_nonoverlapping(source.mass.begin, self.mass.begin, len);
        }
        // properties (u8)
        let len = source.properties.len();
        self.properties.resize(len, 0u8);
        unsafe {
            std::ptr::copy_nonoverlapping(source.properties.begin, self.properties.begin, len);
        }
        // insulation (u8)
        let len = source.insulation.len();
        self.insulation.resize(len, 0u8);
        unsafe {
            std::ptr::copy_nonoverlapping(source.insulation.begin, self.insulation.begin, len);
        }
        // strength_info (u8)
        let len = source.strength_info.len();
        self.strength_info.resize(len, 0u8);
        unsafe {
            std::ptr::copy_nonoverlapping(source.strength_info.begin, self.strength_info.begin, len);
        }
        // disease_idx (u8)
        let len = source.disease_idx.len();
        self.disease_idx.resize(len, 0u8);
        unsafe {
            std::ptr::copy_nonoverlapping(source.disease_idx.begin, self.disease_idx.begin, len);
        }
        // disease_count (i32)
        let len = source.disease_count.len();
        self.disease_count.resize(len, 0i32);
        unsafe {
            std::ptr::copy_nonoverlapping(source.disease_count.begin, self.disease_count.begin, len);
        }
        // disease_infestation_tick_count (u8)
        let len = source.disease_infestation_tick_count.len();
        self.disease_infestation_tick_count.resize(len, 0u8);
        unsafe {
            std::ptr::copy_nonoverlapping(
                source.disease_infestation_tick_count.begin,
                self.disease_infestation_tick_count.begin,
                len,
            );
        }
        // disease_growth_accumulated_error (f32)
        let len = source.disease_growth_accumulated_error.len();
        self.disease_growth_accumulated_error.resize(len, 0.0f32);
        unsafe {
            std::ptr::copy_nonoverlapping(
                source.disease_growth_accumulated_error.begin,
                self.disease_growth_accumulated_error.begin,
                len,
            );
        }
        // radiation (f32)
        let len = source.radiation.len();
        self.radiation.resize(len, 0.0f32);
        unsafe {
            std::ptr::copy_nonoverlapping(source.radiation.begin, self.radiation.begin, len);
        }
    }

    /// 逐行拷贝（去掉边界）—— CopySimDataToGame 专用。
    ///
    /// 对照源码 11_msvcrt_ignored.c L31369-31376（CopySimDataToGame 的 copyToGameTasks）。
    /// 原版使用并行任务队列逐行拷贝，每个任务参数包含：
    /// - source (updatedCells，含边界 sim_w × sim_h)
    /// - dest (cells，不含边界 game_w × game_h)
    /// - 起始行（从 1 开始，跳过边界第一行）
    /// - sim_w（含边界宽度）
    /// - game_w（不含边界宽度）
    ///
    /// 本方法为单线程实现：对每一行 row (0..game_h)：
    /// - 源起始索引 = (row + 1) * sim_w + 1（跳过第一行和第一列边界）
    /// - 目标起始索引 = row * game_w
    /// - 对每个字段 memcpy game_w 个元素
    ///
    /// **C1 修复**：原 copy_from 直接 resize + memcpy 整个 SOA，导致
    /// game_data.cells 被错误 resize 到含边界尺寸，C# 读取数据错位。
    pub fn copy_rows_from_stripped(
        &mut self,
        source: &CellSOA,
        game_w: usize,
        game_h: usize,
        sim_w: usize,
    ) {
        // 确保目标 vector 尺寸正确（game_w × game_h，不含边界）
        let total_game_cells = game_w * game_h;
        self.element_idx.resize(total_game_cells, 0u16);
        self.temperature.resize(total_game_cells, 0.0f32);
        self.mass.resize(total_game_cells, 0.0f32);
        self.properties.resize(total_game_cells, 0u8);
        self.insulation.resize(total_game_cells, 0u8);
        self.strength_info.resize(total_game_cells, 0u8);
        self.disease_idx.resize(total_game_cells, 0u8);
        self.disease_count.resize(total_game_cells, 0i32);
        self.disease_infestation_tick_count.resize(total_game_cells, 0u8);
        self.disease_growth_accumulated_error.resize(total_game_cells, 0.0f32);
        self.radiation.resize(total_game_cells, 0.0f32);

        for row in 0..game_h {
            let src_start = (sim_w + 1) + row * sim_w; // 跳过第一行和第一列
            let dst_start = row * game_w;

            // 逐字段 memcpy 一行（game_w 个元素）
            macro_rules! copy_row {
                ($field:ident, $ty:ty) => {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            source.$field.begin.add(src_start),
                            self.$field.begin.add(dst_start),
                            game_w,
                        );
                    }
                };
            }
            copy_row!(element_idx, u16);
            copy_row!(temperature, f32);
            copy_row!(mass, f32);
            copy_row!(properties, u8);
            copy_row!(insulation, u8);
            copy_row!(strength_info, u8);
            copy_row!(disease_idx, u8);
            copy_row!(disease_count, i32);
            copy_row!(disease_infestation_tick_count, u8);
            copy_row!(disease_growth_accumulated_error, f32);
            copy_row!(radiation, f32);
        }
    }
}

impl Drop for CellSOA {
    fn drop(&mut self) {
        self.element_idx.clear();
        self.temperature.clear();
        self.mass.clear();
        self.properties.clear();
        self.insulation.clear();
        self.strength_info.clear();
        self.disease_idx.clear();
        self.disease_count.clear();
        self.disease_infestation_tick_count.clear();
        self.disease_growth_accumulated_error.clear();
        self.radiation.clear();
    }
}

// ===== SimData =====

/// SimData — 模拟核心数据（0x890 = 2192B）。
/// 字段对照源码 00_types_reference.c L8389-8433。
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
    pub radiation_linger_rate: f32,                         // L8427
    pub radiation_max_mass: f32,                            // L8428
    pub radiation_base_weight: f32,                         // L8429
    pub radiation_density_weight: f32,                      // L8430
    pub radiation_constructed_factor: f32,                  // L8431
    pub worlds: MsvcVector<WorldOffsetData>,                // L8432
}

impl SimData {
    /// 构造零初始化的 SimData。
    pub fn new_zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }

    /// AllocateCells 用的构造函数。
    /// 对照源码 11_msvcrt_ignored.c L22089-22592（SimData::SimData）。
    ///
    /// 参数说明（AllocateCells 传入时已加 2 边界）：
    /// - `width`/`height`：含边界的尺寸（原版 width+2, height+2）
    /// - `random_seed`：随机种子（_time64 返回值）
    /// - `radiation_enabled`：辐射是否启用
    /// - `headless`：无头模式
    pub fn new_for_allocate(
        width: i32,
        height: i32,
        random_seed: u32,
        radiation_enabled: bool,
        headless: bool,
    ) -> Self {
        let mut sd = SimData::new_zeroed();
        sd.width = width;
        sd.height = height;
        sd.num_game_cells = (width - 2) * (height - 2);
        sd.random_seed = random_seed;
        sd.iterate_direction = -1;
        sd.displacement_direction = -1;
        sd.radiation_enabled = radiation_enabled;
        sd.headless = headless;

        // 对照源码 L22385-22390：三个特殊元素索引
        sd.void_element_idx = elements_table::get_element_index_pub(0xa9360b34);
        sd.vacuum_element_idx = elements_table::get_element_index_pub(0x2d39bf75);
        sd.unobtanium_element_idx = elements_table::get_element_index_pub(0x6d95058c);

        // 对照源码 L22358-22362：辐射常数
        sd.radiation_linger_rate = 1.1;
        sd.radiation_max_mass = 2000.0;
        sd.radiation_base_weight = 0.3;
        sd.radiation_density_weight = 0.7;
        sd.radiation_constructed_factor = 0.8;
        // 对照源码 L22355：initSettleThermalBoundaries = false（new_zeroed 已为 false，显式写出以对齐原版）
        sd.init_settle_thermal_boundaries = false;

        let total_cells = (width as usize) * (height as usize);

        // 分配 cells（CellSOA::CellSOA(_, uVar27)）
        let cells = Box::new(CellSOA::with_size(total_cells));
        sd.cells = UniquePtr { ptr: Box::into_raw(cells) };

        // 分配 updated_cells
        let updated_cells = Box::new(CellSOA::with_size(total_cells));
        sd.updated_cells = UniquePtr { ptr: Box::into_raw(updated_cells) };

        // 默认绝缘 255（满导热）：原版建格默认即可导电；存档格式不含 insulation，
        // 加载后格子保留此默认；隔热砖/门经 SetInsulationValue(×255) 覆盖。
        // 此前默认 0 → k = ins²×1.53787e-05×TC ≈ 0 → 邻格/背墙/建筑↔格子换热全部失效
        //（2026-08-03 用户实测：加载存档后液体/气体/背墙均不换热）。
        {
            let cells = unsafe { &mut *sd.cells.ptr };
            let updated = unsafe { &mut *sd.updated_cells.ptr };
            for i in 0..total_cells {
                cells.insulation.set(i, 255);
                updated.insulation.set(i, 255);
            }
        }

        // 分配 flow（Vector4f × total_cells，4 方向流动量）—— 对照源码 L8406
        let flow = vec![crate::a_framework::vector_math::Vector4f::default(); total_cells];
        sd.flow = UniquePtr { ptr: flow.leak().as_mut_ptr() };

        // 分配 accumulated_flow（f32 × total_cells，纹理累积值）—— 对照源码 L8405
        let accumulated_flow = vec![0.0f32; total_cells];
        sd.accumulated_flow = UniquePtr { ptr: accumulated_flow.leak().as_mut_ptr() };

        // 分配 backwall（BackwallSOA::BackwallSOA(_, uVar27, vacuumElementIdx)）
        let backwall = Box::new(BackwallSOA::with_size(total_cells, sd.vacuum_element_idx));
        sd.backwall = UniquePtr { ptr: Box::into_raw(backwall) };

        // 分配 sim_events（对照源码 11_msvcrt_ignored.c L22374-22380）
        // operator_new(0x280) + SimEvents::SimEvents() → this->simEvents
        // C1 修复：原 A3 阶段遗漏了 sim_events 分配，导致 process_dig_points 和
        // copy_sim_data_to_game 检查 sim_events.ptr.is_null() 后跳过处理。
        let sim_events = Box::new(SimEvents::default());
        sd.sim_events = UniquePtr { ptr: Box::into_raw(sim_events) };

        // 分配 visible_grid（液滴轨道前置，2026-08-02）：每游戏格 1 字节，初始全 0xFF（可见兜底）。
        // 原版 SpawnFallingLiquid（11_msvcrt L43490）按 visibleGrid 门控液滴事件；
        // 之前保持 null → spawn_falling_liquid 恒 false → 液滴轨道从未激活（悬崖处质量走格子转移）。
        // 注：C# 反编译未见 SetVisibleCells 发送方，先全可见兜底；每帧与 GameData.visibleGrid 交换
        //（原版 GameData::swapVisibleGrid），若 C# 后续写入可见性则自动生效。
        let visible = vec![0xFFu8; sd.num_game_cells as usize];
        sd.visible_grid = UniquePtr { ptr: visible.leak().as_mut_ptr() };

        // 分配 timers（原版 11_msvcrt_ignored.c L22456-22484）：每格 1B，初始全部 |= 0x1F。
        // 2026-08-04 审查修正：此前 null（liquid_flow/temperature/element_emitter 均 null 守卫
        // 跳过 timers 写入）。原版初始全 0x1F（"格子内容刚变化"标记位域）。
        let timers = vec![Timers { stable_cell_ticks: 0x1F }; total_cells];
        sd.timers = UniquePtr { ptr: timers.leak().as_mut_ptr() };

        // world_zones：原版 SimData::SimData 初始 null（11_msvcrt L22138），
        // 由 SetWorldZones 消息分配（L24760）；此处保持 null，消息到达时分配。

        sd
    }

    /// ApplySaveSettings — 设置存档选项。
    /// 对照源码 11_msvcrt_ignored.c L23639-23644。
    pub fn apply_save_settings(&mut self, settings: u8) {
        self.saved_options = settings;
    }

    /// InitializeBoundary — 初始化顶/底边界行（原版 SimDLL_Source.c L123236+，
    /// 与 11_msvcrt_ignored.c L23949+ 同函数）。
    ///
    /// 逐列初始化：
    /// - 底行（row 0）= gSimData+0x1e = **unobtanium**；
    /// - 顶行（row height−1）= gSimData+0x1c = **vacuum**；
    /// - mass=9999.0（0x461C3C00）、temperature=0、病菌清零；
    /// - 背墙=vacuum 索引/0/0。
    /// cells 与 updated_cells 双缓冲都写。左右列**不处理**（原版仅顶/底两行，
    /// 左右边界来自存档地图数据）。
    pub fn initialize_boundary(&mut self) {
        let width = self.width as usize;
        let height = self.height as usize;
        let unobtanium = self.unobtanium_element_idx;
        let vacuum = self.vacuum_element_idx;

        // 辅助：写一个边界格（双缓冲 + 背墙）
        fn set_boundary_cell(cells: &mut CellSOA, updated: &mut CellSOA, backwall: &mut BackwallSOA, idx: usize, element: u16, bw_vacuum: u16) {
            for buf in [cells, updated] {
                buf.element_idx.set(idx, element);
                buf.mass.set(idx, 9999.0);
                buf.temperature.set(idx, 0.0);
                buf.disease_count.set(idx, 0);
                buf.disease_idx.set(idx, 0xff);
            }
            backwall.element_idx.set(idx, bw_vacuum);
            backwall.mass.set(idx, 0.0);
            backwall.temperature.set(idx, 0.0);
        }

        unsafe {
            let backwall = &mut *self.backwall.ptr;
            let bw_vacuum = backwall.vacuum_element_idx;

            // 底行（row 0）与顶行（row height-1）（对照 L23960-24073）
            for x in 0..width {
                let cells = &mut *self.cells.ptr;
                let updated = &mut *self.updated_cells.ptr;
                set_boundary_cell(cells, updated, backwall, x, unobtanium, bw_vacuum);
                let top = (height - 1) * width + x;
                set_boundary_cell(cells, updated, backwall, top, vacuum, bw_vacuum);
            }
        }
    }

    /// SettleThermalBoundaries — Void 格温度取邻居均值。
    /// 对照源码 11_msvcrt_ignored.c L24803-24898。
    ///
    /// 读 cells、写 updated_cells：内部格中元素为 0x3fe0146e 的格子，
    /// 温度 = 8 邻居中（排除 0x3fe0146e 与 unobtanium 0x6d95058c，且 mass>0）的温度均值。
    pub fn settle_thermal_boundaries(&mut self) {
        let width = self.width as usize;
        let height = self.height as usize;
        let void_idx = elements_table::get_element_index_pub(0x3fe0146e);
        let unobtanium_idx = elements_table::get_element_index_pub(0x6d95058c);
        let w = width as i32;
        let offsets: [i32; 8] = [-w - 1, -w, 1 - w, -1, 1, w - 1, w, w + 1];
        unsafe {
            let cells = &*self.cells.ptr;
            let updated = &mut *self.updated_cells.ptr;
            // 行 1..height-1、列 1..width-1（对照 C 循环 iVar14 ∈ [1, height-2]）
            for row in 1..height.saturating_sub(1) {
                for col in 1..width.saturating_sub(1) {
                    let idx = row * width + col;
                    if cells.element_idx.get(idx) != void_idx {
                        continue;
                    }
                    let mut sum = 0.0f32;
                    let mut count = 0.0f32;
                    for off in offsets {
                        let nidx = (idx as i32 + off) as usize;
                        let e = cells.element_idx.get(nidx);
                        if e == void_idx || e == unobtanium_idx {
                            continue;
                        }
                        if cells.mass.get(nidx) > 0.0 {
                            count += 1.0;
                            sum += cells.temperature.get(nidx);
                        }
                    }
                    if count > 0.0 {
                        updated.temperature.set(idx, sum / count);
                    }
                }
            }
        }
    }
}

impl Drop for SimData {
    fn drop(&mut self) {
        // 释放 cells（Box::from_raw 会调用 CellSOA::Drop，释放 MsvcVector 内存）
        if !self.cells.ptr.is_null() {
            unsafe { let _ = Box::from_raw(self.cells.ptr); }
            self.cells.ptr = std::ptr::null_mut();
        }
        // 释放 updated_cells
        if !self.updated_cells.ptr.is_null() {
            unsafe { let _ = Box::from_raw(self.updated_cells.ptr); }
            self.updated_cells.ptr = std::ptr::null_mut();
        }
        // 释放 backwall
        if !self.backwall.ptr.is_null() {
            unsafe { let _ = Box::from_raw(self.backwall.ptr); }
            self.backwall.ptr = std::ptr::null_mut();
        }
        // 释放 sim_events
        if !self.sim_events.ptr.is_null() {
            unsafe { let _ = Box::from_raw(self.sim_events.ptr); }
            self.sim_events.ptr = std::ptr::null_mut();
        }
        // 释放 flow（vec![...].leak() 分配，len == cap == total_cells）
        if !self.flow.ptr.is_null() {
            let total = (self.width as usize) * (self.height as usize);
            unsafe { let _ = Vec::from_raw_parts(self.flow.ptr, total, total); }
            self.flow.ptr = std::ptr::null_mut();
        }
        // 释放 accumulated_flow
        if !self.accumulated_flow.ptr.is_null() {
            let total = (self.width as usize) * (self.height as usize);
            unsafe { let _ = Vec::from_raw_parts(self.accumulated_flow.ptr, total, total); }
            self.accumulated_flow.ptr = std::ptr::null_mut();
        }
        // 释放 timers（2026-08-04 起分配）
        if !self.timers.ptr.is_null() {
            let total = (self.width as usize) * (self.height as usize);
            unsafe { let _ = Vec::from_raw_parts(self.timers.ptr, total, total); }
            self.timers.ptr = std::ptr::null_mut();
        }
        // 释放 visible_grid（2026-08-02 起分配）
        if !self.visible_grid.ptr.is_null() {
            let total = (self.num_game_cells as usize).max(0);
            unsafe { let _ = Vec::from_raw_parts(self.visible_grid.ptr, total, total); }
            self.visible_grid.ptr = std::ptr::null_mut();
        }
        // 释放 world_zones（SetWorldZones 消息分配后；null 时跳过）
        if !self.world_zones.ptr.is_null() {
            let total = (self.width as usize) * (self.height as usize);
            unsafe { let _ = Vec::from_raw_parts(self.world_zones.ptr, total, total); }
            self.world_zones.ptr = std::ptr::null_mut();
        }
        // MsvcVector 字段（cosmic_radiation_occlusion/active_regions/components/worlds）
        // 的释放在后续阶段实现。
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn debug_properties_size_is_12() {
        assert_eq!(size_of::<DebugProperties>(), 12);
    }

    #[test]
    fn world_offset_data_size_is_16() {
        assert_eq!(size_of::<WorldOffsetData>(), 16);
    }

    #[test]
    fn active_region_size_is_24() {
        assert_eq!(size_of::<ActiveRegion>(), 24);
    }

    #[test]
    fn timers_size_is_1() {
        assert_eq!(size_of::<Timers>(), 1);
    }

    #[test]
    fn padding_substruct_sizes() {
        assert_eq!(size_of::<ElementConsumer>(), 312);
        assert_eq!(size_of::<ElementEmitter>(), 312);
        assert_eq!(size_of::<RadiationEmitter>(), 176);
        assert_eq!(size_of::<ElementChunk>(), 208);
        assert_eq!(size_of::<BuildingHeatExchange>(), 208);
        assert_eq!(size_of::<BuildingToBuildingHeatExchange>(), 176);
        assert_eq!(size_of::<DiseaseEmitter>(), 312);
        assert_eq!(size_of::<DiseaseConsumer>(), 208);
    }

    #[test]
    fn backwall_soa_size_is_104() {
        assert_eq!(size_of::<BackwallSOA>(), 104);
    }

    #[test]
    fn cell_soa_size_is_352() {
        assert_eq!(size_of::<CellSOA>(), 352);
    }

    #[test]
    fn sim_data_size_matches_original() {
        let actual = size_of::<SimData>();
        if actual != 0x890 {
            panic!(
                "SimData size = {:#x} ({}), 期望 0x890 (2192)。\n\
                差异：{} 字节。请检查：\n\
                1. MsvcVector 是否 32B\n\
                2. 8 子结构 reserved 数组大小是否与 dtor 偏移差值一致\n\
                3. BackwallSOA u16+i16+pad+3×MsvcVector 是否 104B\n\
                4. DebugProperties 是否 12B\n\
                5. CellSOA 11 个 MsvcVector 是否 352B",
                actual, actual, actual as i64 - 0x890
            );
        }
    }

    #[test]
    fn backwall_soa_with_size_fills_element_idx() {
        let bw = BackwallSOA::with_size(5, 0xffff);
        assert_eq!(bw.element_idx.len(), 5);
        assert_eq!(bw.element_idx.get(0), 0xffff);
        assert_eq!(bw.element_idx.get(4), 0xffff);
        assert_eq!(bw.mass.len(), 5);
        assert_eq!(bw.temperature.len(), 5);
    }

    #[test]
    fn cell_soa_with_size_resizes_all_vectors() {
        let cells = CellSOA::with_size(10);
        assert_eq!(cells.element_idx.len(), 10);
        assert_eq!(cells.temperature.len(), 10);
        assert_eq!(cells.radiation.len(), 10);
    }

    #[test]
    fn sim_data_new_zeroed() {
        let sd = SimData::new_zeroed();
        assert_eq!(sd.width, 0);
        assert_eq!(sd.height, 0);
        assert_eq!(sd.num_game_cells, 0);
    }

    #[test]
    fn new_for_allocate_sets_special_indices_and_radiation_constants() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        let sd = SimData::new_for_allocate(6, 5, 42, true, true);
        // 对照源码 L22385-22390
        assert_eq!(sd.void_element_idx, elements_table::get_element_index_pub(0xa9360b34));
        assert_eq!(sd.vacuum_element_idx, elements_table::get_element_index_pub(0x2d39bf75));
        assert_eq!(sd.unobtanium_element_idx, elements_table::get_element_index_pub(0x6d95058c));
        // 对照源码 L22358-22362
        assert_eq!(sd.radiation_linger_rate, 1.1);
        assert_eq!(sd.radiation_max_mass, 2000.0);
        assert_eq!(sd.radiation_base_weight, 0.3);
        assert_eq!(sd.radiation_density_weight, 0.7);
        assert_eq!(sd.radiation_constructed_factor, 0.8);
        assert!(!sd.init_settle_thermal_boundaries);
    }

    /// 2026-08-04 审查修正：timers 分配（初始全 0x1F，原版 L22456-22484）、
    /// visible_grid 分配（全 0xFF 可见兜底）、world_zones 初始 null（原版 L22138，
    /// 由 SetWorldZones 消息分配）。
    #[test]
    fn new_for_allocate_allocates_timers_visible_grid_keeps_world_zones_null() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        let sd = SimData::new_for_allocate(6, 5, 0, false, false);
        assert!(!sd.timers.ptr.is_null(), "timers 应分配");
        assert!(!sd.visible_grid.ptr.is_null(), "visible_grid 应分配");
        assert!(sd.world_zones.ptr.is_null(), "world_zones 初始应 null（原版 L22138）");
        unsafe {
            // timers 初始全部 |= 0x1F（原版 L22475-22484 循环）
            let timers = std::slice::from_raw_parts(sd.timers.ptr, 30);
            for t in timers {
                assert_eq!(
                    t.stable_cell_ticks & 0x1F,
                    0x1F,
                    "timers 初始应 0x1F"
                );
            }
            // visible_grid 初始全 0xFF（可见兜底，已知偏差记录）
            let vg = std::slice::from_raw_parts(sd.visible_grid.ptr, sd.num_game_cells as usize);
            for v in vg {
                assert_eq!(*v, 0xFF);
            }
        }
    }

    #[test]
    fn initialize_boundary_sets_top_bottom_rows_both_buffers() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        let mut sd = SimData::new_for_allocate(6, 5, 0, false, false);
        sd.initialize_boundary();
        let unob = elements_table::get_element_index_pub(0x6d95058c);
        let vacuum = elements_table::get_element_index_pub(0x2d39bf75);
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let buf = &*buf_ptr;
                for x in 0..6usize {
                    // 底行（row 0）= unobtanium
                    assert_eq!(buf.element_idx.get(x), unob, "bottom x={}", x);
                    assert_eq!(buf.mass.get(x), 9999.0);
                    assert_eq!(buf.temperature.get(x), 0.0);
                    assert_eq!(buf.disease_count.get(x), 0);
                    assert_eq!(buf.disease_idx.get(x), 0xff);
                    // 顶行（row height-1）= vacuum
                    let top = 4 * 6 + x;
                    assert_eq!(buf.element_idx.get(top), vacuum, "top x={}", x);
                    assert_eq!(buf.mass.get(top), 9999.0);
                }
                // 左右列不处理（原版仅顶/底两行）——左列 (1,0) 保持零值
                assert_eq!(buf.element_idx.get(6), 0, "left col row1 不受影响");
                assert_eq!(buf.mass.get(6), 0.0);
                // 内部格不受影响（with_size 零值）
                assert_eq!(buf.element_idx.get(8), 0);
                assert_eq!(buf.mass.get(8), 0.0);
            }
            let bw = &*sd.backwall.ptr;
            for x in 0..6usize {
                assert_eq!(bw.element_idx.get(x), bw.vacuum_element_idx);
                assert_eq!(bw.mass.get(x), 0.0);
                assert_eq!(bw.temperature.get(x), 0.0);
            }
        }
    }

    #[test]
    fn settle_thermal_boundaries_averages_neighbor_temperatures() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        let mut sd = SimData::new_for_allocate(6, 5, 0, false, false);
        let void_idx = elements_table::get_element_index_pub(0x3fe0146e);
        let unob_idx = elements_table::get_element_index_pub(0x6d95058c);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            // 目标格 (row=2, col=2) → idx = 2*6+2 = 14，设为 Void
            cells.element_idx.set(14, void_idx);
            // 8 邻居：7, 8, 9（上排 row1 col1-3）；13, 15（左右）；19, 20, 21（下排 row3 col1-3）
            let neighbors: [(usize, f32); 8] = [
                (7, 100.0), (8, 200.0), (9, 300.0),
                (13, 400.0), (15, 500.0),
                (19, 600.0), (20, 700.0), (21, 800.0),
            ];
            for (idx, temp) in neighbors {
                cells.element_idx.set(idx, 1); // 非 void/unob 普通元素
                cells.mass.set(idx, 1.0);
                cells.temperature.set(idx, temp);
            }
            // idx 8 无质量 → 应被排除
            cells.mass.set(8, 0.0);
            // idx 9 是 unobtanium → 应被排除
            cells.element_idx.set(9, unob_idx);
        }
        sd.settle_thermal_boundaries();
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // 测试环境 void_idx == unob_idx == 0xFFFF，故 idx 9（设为 unob）与 idx 8（mass=0）
            // 都被排除：均值 = (100+400+500+600+700+800) / 6 = 3100/6
            // （简报原写 3400/7 未扣除 idx 9，与 void_idx==unob_idx 前提矛盾，已修正）
            let expected = (100.0f32 + 400.0 + 500.0 + 600.0 + 700.0 + 800.0) / 6.0;
            let got = updated.temperature.get(14);
            assert!((got - expected).abs() < 0.01, "expected {}, got {}", expected, got);
        }
    }

    #[test]
    fn new_for_allocate_allocates_flow_buffers() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        let sd = SimData::new_for_allocate(6, 5, 0x12345678, true, false);
        assert!(!sd.flow.ptr.is_null(), "flow 应已分配");
        assert!(!sd.accumulated_flow.ptr.is_null(), "accumulated_flow 应已分配");
        let total = (6 * 5) as usize;
        unsafe {
            let flow = std::slice::from_raw_parts(sd.flow.ptr, total);
            for v in flow {
                assert_eq!(*v, crate::a_framework::vector_math::Vector4f::default());
            }
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, total);
            for a in acc {
                assert_eq!(*a, 0.0f32);
            }
        }
    }
}
