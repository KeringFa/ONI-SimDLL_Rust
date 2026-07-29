# simDLL_RS Phase 1：数据层 + 序列化设计规格

> **状态**：已批准（待用户最终审查）
> **日期**：2026-07-29
> **范围**：Phase 1 子项目规格 — BinaryBufferReader/Writer + STL 容器建模 + 所有数据 struct 字段定义 + 类骨架
> **前置**：Phase 0（已完成，HEAD = `1624795`，37/37 测试通过）

---

## 目标

为 Phase 3+ 的存档加载、元素系统、模拟逻辑提供完整的数据结构基础。Phase 1 只定义数据结构（含 size_of 验证），不实现任何业务方法。

### 关键约束（沿用 Phase 0）

- **平台**：仅 x86_64-pc-windows-msvc
- **C ABI**：`#[no_mangle] pub extern "C" fn`；跨 ABI 结构体 `#[repr(C)]`
- **panic**：release abort / test unwind；导出函数必 `catch_unwind` 兜底
- **依赖最小化**：沿用 Phase 0 的 7 个 crate，不新增依赖
- **严格对照源码**：原理不变，允许改造结构但需说明运行时差异
- **代码小白用户**：所有设计需向用户解释清楚

### 验收标准

- ✅ BinaryBufferReader/Writer round-trip 单测通过
- ✅ 所有 struct 的 `size_of` 断言通过（与原版字节数一致）
- ✅ `cargo test` 全部通过（Phase 0 的 37 个 + Phase 1 新增 ~50+ 个）
- ✅ `cargo build --release` 产出 SimDLL.dll（与 Phase 0 行为一致）
- ✅ 用户部署后游戏仍能进主菜单（无回归）

---

## §1 总体架构 + 模块布局

### 修改的模块（替换 Phase 0 桩）

| 模块 | 原状态 | Phase 1 改动 |
|---|---|---|
| `buffer.rs` | 简化桩（2 字段） | BinaryBufferReader/Writer 完整实现 |
| `sim.rs` | 空 struct | Sim 类完整字段骨架（仅字段，无方法） |
| `sim_frame_manager.rs` | 空 struct | SimFrameManager 完整字段骨架 |
| `frame_sync.rs` | 空 struct | FrameSync 完整字段 |
| `message_handler.rs` | 空表 | Type enum + MessageHandlerEntry + 空 gSimMessageHandlers |

### 新增模块

| 模块 | 内容 |
|---|---|
| `stl_shim.rs` | C++ STL 容器的 Rust 等价物（MsvcVector、UniquePtr、MsvcString、MsvcFunction） |
| `sim_data.rs` | SimData + CellSOA + BackwallSOA + WorldOffsetData + 子结构 |
| `game_data.rs` | GameData + 27 个 *Info/*Callback 子结构 |
| `element.rs` | Element + PhysicsData + ElementState |
| `disease.rs` | Disease + DiseaseInfo |
| `sim_events.rs` | SimEvents + SimFrameInfo + 各种事件子结构 |
| `vector_math.rs` | Vector2<f32>、Vector3<f32>、Vector4<f32> |

### 不变的模块

`lib.rs`、`sim_api.rs`、`abi.rs`、`logger.rs`、`crashdump.rs`、`sysinfo.rs`

> 注：`lib.rs` 仅追加 `mod` 声明与全局静态变量类型调整，不算"修改模块"。

### 模块依赖关系（从底层到上层）

```
abi.rs / vector_math.rs / stl_shim.rs   ← 基础类型层（无依赖）
        ↓
buffer.rs / element.rs / disease.rs      ← 数据载体
        ↓
sim_data.rs / game_data.rs / sim_events.rs ← 大型聚合 struct
        ↓
frame_sync.rs / sim.rs / sim_frame_manager.rs / message_handler.rs ← 顶层骨架
```

### 运行时差异

无。Phase 1 不实现任何业务逻辑，只定义数据结构。游戏运行时行为与 Phase 0 完全一致（仍走桩函数）。

---

## §2 BinaryBufferReader / BinaryBufferWriter 设计

### 关键决策

**决策 #1**：BinaryBufferReader 用**纯 Rust 结构体 + 直接方法**（不保留 C++ vtable）。
- 理由：BinaryBufferReader 只在 simDLL 内部构造（C# 通过 `SIM_HandleMessage(id, size, data)` 只传原始字节数组），无外部代码触碰，不需要 ABI 兼容。
- 代价：`size_of` 不等于原版 32 字节（实际 24 字节，少 8 字节 vtable 指针）。
- 收益：代码简洁、易读、易调试，符合 Rust 习惯。

**决策 #2**：所有读取方法返回 `Result<T, BufferError>`。
- 理由：存档文件可能损坏，明确处理错误比直接崩溃更友好。
- 调用者可选用 `?` 传播到 `catch_unwind` 边界，或显式 `match` 处理。

### BinaryBufferReader struct

```rust
#[repr(C)]
pub struct BinaryBufferReader {
    m_offset: u64,            // 原版 mOffset
    m_buffer_data: *const i8, // 原版 mBufferData（schar*）
    m_buffer_size: u64,       // 原版 mBufferSize
}
// size_of = 24 字节（原版 32 字节，差 8 字节 vtable 指针）
```

**构造**：`pub fn new(buffer: &[u8]) -> Self`

### 读取方法清单

| 方法签名 | 原版 vtable 偏移 | 用途 |
|---|---|---|
| `read_int(&mut self) -> Result<i32, BufferError>` | +0x38 | 4 字节有符号整数 |
| `read_uint(&mut self) -> Result<u32, BufferError>` | +0x38 | 4 字节无符号整数 |
| `read_short(&mut self) -> Result<i16, BufferError>` | +0x68 | 2 字节短整数 |
| `read_ushort(&mut self) -> Result<u16, BufferError>` | +0x68 | 2 字节无符号短整数 |
| `read_float(&mut self) -> Result<f32, BufferError>` | +0x18 | 4 字节浮点 |
| `read_byte(&mut self) -> Result<u8, BufferError>` | +0x50 | 1 字节 |
| `read_bool(&mut self) -> Result<bool, BufferError>` | +0x50 | 1 字节布尔（仅 0/1 合法） |
| `read_bytes(&mut self, out: &mut [u8]) -> Result<(), BufferError>` | +0x30 | N 字节填入调用者缓冲 |
| `read_string(&mut self) -> Result<String, BufferError>` | — | UTF-8 字符串（先读 uint 长度，再读字节） |
| `skip(&mut self, count: u64) -> Result<(), BufferError>` | `SkipBytes` | 跳过 N 字节 |
| `remaining(&self) -> u64` | — | 剩余可读字节数 |
| `is_at_end(&self) -> bool` | — | 是否读到末尾 |

> 源码 vtable 偏移来源：02_save_load.c 中 `_padding_ + 0x18 / 0x30 / 0x38 / 0x50 / 0x68` 反推。

### BinaryBufferWriter struct

```rust
pub struct BinaryBufferWriter {
    buf: Vec<u8>,  // 不需要 #[repr(C)]，Writer 完全是 Rust 内部使用
}
```

**写入方法**（与 Reader 对称）：
- `write_int(&mut self, v: i32) -> Result<(), BufferError>`
- `write_uint(&mut self, v: u32) -> Result<(), BufferError>`
- `write_short(&mut self, v: i16) -> Result<(), BufferError>`
- `write_ushort(&mut self, v: u16) -> Result<(), BufferError>`
- `write_float(&mut self, v: f32) -> Result<(), BufferError>`
- `write_byte(&mut self, v: u8) -> Result<(), BufferError>`
- `write_bytes(&mut self, bytes: &[u8]) -> Result<(), BufferError>`
- `write_string(&mut self, s: &str) -> Result<(), BufferError>` —— 先写 uint 长度，再写 UTF-8 字节
- `into_bytes(self) -> Vec<u8>` —— 取出最终字节

### BufferError enum

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferError {
    EndOfBuffer,        // 读取越界
    InvalidUtf8,        // 字符串解码失败
    InvalidBool(u8),    // 布尔值非 0/1
    Io,                 // 写入失败（Writer 用，实际几乎不发生）
}
```

### 运行时差异（BinaryBufferReader/Writer）

| 维度 | 原版 | Rust 实现 | 用户感知差异 |
|---|---|---|---|
| 调用方式 | vtable 间接调用 | 直接方法调用 | 无（性能略好，约 1-2ns/次） |
| 越界处理 | `_invalid_parameter_noinfo_noreturn()` 立即终止进程 | 返回 `Err`，调用者决定 | 存档损坏时不崩游戏，被 `catch_unwind` 捕获后优雅恢复 |
| 布尔值 | 任意非零为真 | 仅 0/1 合法，其他返回 `Err(InvalidBool)` | 可能导致读存档失败（若存档有"2"作为布尔值），但实际存档布尔值都是 0/1 |
| 字符串编码 | 假设 ASCII/UTF-8 | 严格 UTF-8 验证 | 存档若含非 UTF-8 字符串会读失败，但原版 ONI 字符串都是 ASCII |

### 不在范围

- **EndianSwappedBinaryBufferReader**：YAGNI（Windows x86_64 始终小端序，存档永远小端序）。

---

## §3 Struct 布局策略 + 核心 struct 字段清单

### 关键决策

**决策 #3**：所有**数据 struct**（SimData/CellSOA/GameData/Disease/Element 等）用 `#[repr(C)]` + 与原版相同字段顺序 + `size_of` 断言等于原版字节数。

**例外**：BinaryBufferReader/Writer 不要求匹配原版字节数（§2 决策 #1 已决定放弃 vtable，BinaryBufferReader = 24 字节而非原版 32 字节，BinaryBufferWriter 用 `Vec<u8>` 不需要 `#[repr(C)]`）。

- 优点：编译期自动验证字段完整性，漏字段立即编译失败。
- 代价：需要把 C++ STL 容器（`std::vector`/`std::unique_ptr`/`std::string`/`std::function`）建模为 Rust 等价 struct。

### STL 容器建模（`stl_shim.rs`）

```rust
/// std::vector<T> = 24 字节（begin + end + capacity_end 三个指针）
#[repr(C)]
pub struct MsvcVector<T> {
    pub begin: *mut T,         // offset 0
    pub end: *mut T,           // offset 8
    pub capacity_end: *mut T,  // offset 16
}

/// std::unique_ptr<T> = 8 字节（单一指针，default_delete 无状态）
#[repr(C)]
pub struct UniquePtr<T> {
    pub ptr: *mut T,
}

/// std::string (MSVC SSO) = 32 字节
#[repr(C)]
pub struct MsvcString {
    pub buffer_or_inline: [u8; 16],  // SSO inline 或堆指针
    pub size: u64,
    pub capacity: u64,
}

/// std::function<...> = 64 字节（典型大小，含 vtable + small storage）
#[repr(C)]
pub struct MsvcFunction {
    pub vtable: *const u8,        // offset 0
    pub storage: [u8; 56],        // offset 8-63
}
```

**约束**：
- 所有 STL 容器等价 struct 都用 `#[repr(C)]`
- **不实现任何方法**（push/pop/etc）—— Phase 1 只做内存布局占位
- 在 `stl_shim.rs` 加单元测试断言 `size_of` 与原版一致

### size_of 验证策略

每个 struct 在定义文件底部加单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn simdata_size_matches_original() {
        assert_eq!(size_of::<SimData>(), 0x890);  // 2192 字节
    }
    #[test]
    fn disease_size_matches_original() {
        assert_eq!(size_of::<Disease>(), 0x40);  // 64 字节
    }
    // ...每个 struct 一个断言
}
```

### 核心 struct 字段清单（按复杂度排序）

| struct | 原版字节数 | 主要字段 | 难点 |
|---|---|---|---|
| `Element` | ~120 | 30+ POD 字段 + `PhysicsData` 子结构 | 字段顺序严格对照源码 L11104-11143 |
| `Disease` | 64 (`0x40`) | 2 个 `MsvcVector` (48B) + 16B 未识别字段 | Ghidra 可能漏字段，需在实施时定位剩余 16B |
| `CellSOA` | 264-336+ | 11 个 `MsvcVector` (264B) | 源码访问到 0x150 偏移，可能漏字段 |
| `SimData` | 2192 (`0x890`) | 4 int + 3 ushort + 4 unique_ptr + 3 vector + 8 个子结构 + 5 float + 3 bool | 子结构大小（ElementConsumer/Emitter 等）待定 |
| `GameData` | ~736+ | 4 int + 27 vector + 9 unique_ptr | 27 种 `*Info/*Callback` 子结构也要定义 |

### SimData 字段对照（源码 00_types_reference.c L8389-8433）

```rust
#[repr(C)]
pub struct SimData {
    pub width: i32,                              // L8390
    pub height: i32,                             // L8391
    pub num_game_cells: i32,                     // L8392
    pub random_seed: u32,                        // L8393
    pub iterate_direction: i32,                  // L8394
    pub displacement_direction: i32,              // L8395
    pub saved_options: u8,                        // L8396
    pub void_element_idx: u16,                   // L8397
    pub vacuum_element_idx: u16,                 // L8398
    pub unobtanium_element_idx: u16,             // L8399
    pub cells: UniquePtr<CellSOA>,               // L8400
    pub updated_cells: UniquePtr<CellSOA>,       // L8401
    pub backwall: UniquePtr<BackwallSOA>,        // L8402
    pub world_zones: UniquePtr<u8>,               // L8403
    pub cosmic_radiation_occlusion: MsvcVector<f32>, // L8404
    pub accumulated_flow: UniquePtr<f32>,         // L8405
    pub flow: UniquePtr<Vector4f>,                // L8406
    pub timers: UniquePtr<Timers>,                // L8407
    pub active_regions: MsvcVector<ActiveRegion>, // L8408
    pub visible_grid: UniquePtr<u8>,              // L8409
    pub tick_count: u16,                          // L8410
    pub padding_1: i32,                           // L8411 _padding_
    pub sim_events: UniquePtr<SimEvents>,         // L8412
    pub debug_properties: DebugProperties,        // L8413
    pub padding_2: i32,                           // L8414 _padding_
    pub components: MsvcVector<*mut SimComponent>, // L8415
    pub element_consumer: ElementConsumer,        // L8416
    pub element_emitter: ElementEmitter,          // L8417
    pub radiation_emitter: RadiationEmitter,      // L8418
    pub element_chunk: ElementChunk,              // L8419
    pub building_heat_exchange: BuildingHeatExchange, // L8420
    pub building_to_building_heat_exchange: BuildingToBuildingHeatExchange, // L8421
    pub disease_emitter: DiseaseEmitter,          // L8422
    pub disease_consumer: DiseaseConsumer,        // L8423
    pub init_settle_thermal_boundaries: bool,     // L8424
    pub headless: bool,                           // L8425
    pub radiation_enabled: bool,                  // L8426
    pub radiation_linger_rate: f32,               // L8427
    pub radiation_max_mass: f32,                  // L8428
    pub radiation_base_weight: f32,               // L8429
    pub radiation_density_weight: f32,            // L8430
    pub radiation_constructed_factor: f32,        // L8431
    pub worlds: MsvcVector<WorldOffsetData>,      // L8432
}
```

### CellSOA 字段对照（源码 00_types_reference.c L11014-11026）

```rust
#[repr(C)]
pub struct CellSOA {
    pub element_idx: MsvcVector<u16>,             // L11015
    pub temperature: MsvcVector<f32>,             // L11016
    pub mass: MsvcVector<f32>,                   // L11017
    pub properties: MsvcVector<u8>,              // L11018
    pub insulation: MsvcVector<u8>,              // L11019
    pub strength_info: MsvcVector<u8>,           // L11020
    pub disease_idx: MsvcVector<u8>,             // L11021
    pub disease_count: MsvcVector<i32>,           // L11022
    pub disease_infestation_tick_count: MsvcVector<u8>, // L11023
    pub disease_growth_accumulated_error: MsvcVector<f32>, // L11024
    pub radiation: MsvcVector<f32>,              // L11025
}
// 注意：源码访问到 0x150 偏移，可能还有未识别字段，待实施时验证
```

### GameData 字段对照（源码 00_types_reference.c L3225-3269）

```rust
#[repr(C)]
pub struct GameData {
    pub width: i32,                                            // L3226
    pub height: i32,                                          // L3227
    pub num_frames_processed: i32,                            // L3228
    pub padding: i32,                                         // L3229 _padding_
    pub substance_change_info: MsvcVector<SubstanceChangeInfo>, // L3230
    pub callback_info: MsvcVector<CallbackInfo>,              // L3231
    pub spawn_falling_liquid_info: MsvcVector<SpawnFallingLiquidInfo>, // L3232
    pub spawn_ore_info: MsvcVector<SpawnOreInfo>,             // L3233
    pub dig_info: MsvcVector<SpawnOreInfo>,                   // L3234 (复用 SpawnOreInfo)
    pub unstable_cell_info: MsvcVector<UnstableCellInfo>,     // L3235
    pub world_damage_info: MsvcVector<WorldDamageInfo>,       // L3236
    pub building_temperature_info: MsvcVector<BuildingTemperatureInfo>, // L3237
    pub mass_consumed_callbacks: MsvcVector<MassConsumedCallback>, // L3238
    pub mass_emitted_callbacks: MsvcVector<MassEmittedCallback>, // L3239
    pub disease_consumed_callbacks: MsvcVector<DiseaseConsumedCallback>, // L3240
    pub radiation_consumed_callbacks: MsvcVector<RadiationConsumedCallback>, // L3241
    pub spawn_fx_info: MsvcVector<SpawnFXInfo>,              // L3242
    pub solid_info: MsvcVector<SolidInfo>,                    // L3243
    pub liquid_change_info: MsvcVector<LiquidChangeInfo>,    // L3244
    pub solid_substance_change_info: MsvcVector<SolidSubstanceChangeInfo>, // L3245
    pub consumed_mass_info: MsvcVector<ConsumedMassInfo>,    // L3246
    pub emitted_mass_info: MsvcVector<EmittedMassInfo>,      // L3247
    pub element_chunk_info: MsvcVector<ElementChunkInfo>,    // L3248
    pub disease_emitted_info: MsvcVector<DiseaseEmittedInfo>, // L3249
    pub disease_consumed_info: MsvcVector<DiseaseConsumedInfo>, // L3250
    pub element_chunk_melted_info: MsvcVector<MeltedInfo>,    // L3251
    pub building_melted_info: MsvcVector<MeltedInfo>,        // L3252
    pub building_overheat_info: MsvcVector<MeltedInfo>,      // L3253
    pub building_no_longer_overheated_info: MsvcVector<MeltedInfo>, // L3254
    pub component_state_changed_messages: MsvcVector<ComponentStateChangedMessage>, // L3255
    pub cell_melted_info: MsvcVector<CellMeltedInfo>,        // L3256
    pub backwall_element_changed_info: MsvcVector<BackwallElementChangedInfo>, // L3257
    pub backwall_should_transition_info: MsvcVector<BackwallShouldTransitionInfo>, // L3258
    pub cells: UniquePtr<CellSOA>,                           // L3259
    pub backwalls: UniquePtr<BackwallSOA>,                   // L3260
    pub flow: UniquePtr<Vector4f>,                            // L3261
    pub accumulated_flow: UniquePtr<f32>,                    // L3262
    pub visible_grid: UniquePtr<u8>,                          // L3263
    pub property_texture_flow: UniquePtr<Vector2f>,           // L3264
    pub property_texture_liquid: UniquePtr<u32>,             // L3265
    pub property_texture_liquid_data: UniquePtr<u32>,        // L3266
    pub property_texture_material_data: UniquePtr<u32>,       // L3267
    pub property_texture_exposed_to_sunlight: UniquePtr<u8>,  // L3268
}
```

### Disease 字段对照（源码 00_types_reference.c L9347-9350）

```rust
#[repr(C)]
pub struct Disease {
    pub diseases: MsvcVector<DiseaseInfo>,        // L9348 (24B)
    pub disease_names: MsvcVector<MsvcString>,    // L9349 (24B)
    pub reserved: [u8; 16],                        // 待实施时定位剩余 16B 字段
}
// size_of 必须等于 0x40 (64)
```

### Element 字段对照（源码 00_types_reference.c L11104-11143）

```rust
#[repr(C)]
pub struct Element {
    pub id: i32,                                  // L11105
    pub low_temp_transition_idx: u16,             // L11106
    pub high_temp_transition_idx: u16,            // L11107
    pub elements_table_idx: u16,                  // L11108
    pub state: u8,                                // L11109
    pub number_of_gradient_colors: u8,            // L11110
    pub specific_heat_capacity: f32,              // L11111
    pub thermal_conductivity: f32,                // L11112
    pub molar_mass: f32,                          // L11113
    pub solid_surface_area_multiplier: f32,       // L11114
    pub liquid_surface_area_multiplier: f32,      // L11115
    pub gas_surface_area_multiplier: f32,          // L11116
    pub flow: f32,                                 // L11117
    pub viscosity: f32,                           // L11118
    pub min_horizontal_flow: f32,                 // L11119
    pub min_vertical_flow: f32,                   // L11120
    pub max_mass: f32,                            // L11121
    pub low_temp: f32,                            // L11122
    pub high_temp: f32,                           // L11123
    pub strength: f32,                            // L11124
    pub low_temp_transition_ore_id: i32,          // L11125
    pub low_temp_transition_ore_mass_conversion: f32, // L11126
    pub high_temp_transition_ore_id: i32,         // L11127
    pub high_temp_transition_ore_mass_conversion: f32, // L11128
    pub sublimate_index: u16,                    // L11129
    pub convert_index: u16,                      // L11130
    pub material_properties: u32,                 // L11131
    pub colour: u32,                             // L11132
    pub gradient_colours: [u32; 6],               // L11133
    pub sublimate_fx: i32,                       // L11134
    pub sublimate_rate: f32,                     // L11135
    pub sublimate_efficiency: f32,                // L11136
    pub sublimate_probability: f32,              // L11137
    pub off_gas_percentage: f32,                  // L11138
    pub light_absorption_factor: f32,             // L11139
    pub radiation_absorption_factor: f32,         // L11140
    pub radiation_per_1000_mass: f32,             // L11141
    pub default_values: PhysicsData,              // L11142
}
```

### 子结构（也属于 Phase 1 范围）

#### SimData 嵌套子结构
- `PhysicsData`（Element 默认值）
- `DebugProperties`（SimData 调试属性）
- `WorldOffsetData`、`ActiveRegion`（SimData 内部）
- `Timers`（SimData 计时器数组）
- `ElementConsumer`、`ElementEmitter`、`RadiationEmitter`
- `ElementChunk`、`BuildingHeatExchange`、`BuildingToBuildingHeatExchange`
- `DiseaseEmitter`、`DiseaseConsumer`
- `BackwallSOA`（与 CellSOA 类似）
- `SimComponent`（前向声明）

#### GameData 嵌套子结构（27 个）
- `SubstanceChangeInfo`、`CallbackInfo`、`SpawnFallingLiquidInfo`
- `SpawnOreInfo`、`UnstableCellInfo`、`WorldDamageInfo`
- `BuildingTemperatureInfo`、`MassConsumedCallback`、`MassEmittedCallback`
- `DiseaseConsumedCallback`、`RadiationConsumedCallback`、`SpawnFXInfo`
- `SolidInfo`、`LiquidChangeInfo`、`SolidSubstanceChangeInfo`
- `ConsumedMassInfo`、`EmittedMassInfo`、`ElementChunkInfo`
- `DiseaseEmittedInfo`、`DiseaseConsumedInfo`、`MeltedInfo`
- `CellMeltedInfo`、`ComponentStateChangedMessage`
- `BackwallElementChangedInfo`、`BackwallShouldTransitionInfo`

每个子结构需在实施时从源码定位字段定义并加 `size_of` 断言。

---

## §4 辅助 struct + 类骨架 + MessageHandlerEntry

### SimEvents / SimFrameInfo（`sim_events.rs`）

```rust
#[repr(C)]
pub struct SimEvents {
    // 字段待实施时从源码定位
}

#[repr(C)]
pub struct SimFrameInfo {
    pub frame_id: u32,
    pub time_delta: f32,
    // 字段待实施时从源码定位
}
```

### FrameSync（`frame_sync.rs`，替换 Phase 0 桩）

```rust
#[repr(C)]
pub struct FrameSync {
    pub m_game_data: UniquePtr<GameData>,
    pub m_prev_game_data: UniquePtr<GameData>,
    pub m_sim_frame_info: SimFrameInfo,
    // 字段待实施时从源码 L127147+ FrameSync::GameSync/SimSync/clear 反推
}
```

### Sim 类骨架（`sim.rs`，仅字段，无方法）

```rust
#[repr(C)]
pub struct Sim {
    pub frame_manager: SimFrameManager,
    pub thread_handle: *mut c_void,      // std::thread 句柄
    pub running: bool,
    // 字段待实施时从源码 L130556 Sim::Sim 对照
}
```

### SimFrameManager 类骨架（`sim_frame_manager.rs`）

```rust
#[repr(C)]
pub struct SimFrameManager {
    pub current_frame: u32,
    pub frames: MsvcVector<SimFrameInfo>,
    // 字段待实施时从源码 L113784 对照
}
```

### MessageHandlerEntry 表（`message_handler.rs`）

```rust
/// 60+ 个消息 ID 枚举（源码 00_types_reference.c L9158-9221）
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    AddDiseaseConsumer = 348345681,
    RadiationParamsModification = 377112707,
    ModifyElementEmitter = 403589164,
    RemoveDiseaseEmitter = 468135926,
    // ... 全部 60+ 个值按源码顺序
}

/// 消息处理函数指针类型
pub type MessageHandlerFn =
    unsafe extern "C" fn(*mut BinaryBufferReader) -> *mut c_void;

#[repr(C)]
pub struct MessageHandlerEntry {
    pub id: MessageType,                    // offset 0, 4 字节
    pub padding: u32,                       // offset 4, 对齐到 8 字节边界
    pub handler: Option<MessageHandlerFn>,   // offset 8, 8 字节（函数指针）
}
// size_of = 16（与原版一致）

/// Phase 1 范围：空表（所有 handler 都是 None）
/// Phase 6 才填充真实 handler
pub static G_SIM_MESSAGE_HANDLERS: &[MessageHandlerEntry] = &[];
```

### 运行时差异

无。Phase 1 不实现任何业务方法，只定义 struct + size_of 断言。游戏运行时与 Phase 0 一致。

---

## §5 测试与验收策略

### 单元测试（每模块内 `#[cfg(test)] mod tests`）

| 模块 | 测试内容 |
|---|---|
| `buffer.rs` | BinaryBufferReader/Writer round-trip（int/float/short/byte/bytes/string）+ 越界返回 Err |
| `stl_shim.rs` | MsvcVector/UniquePtr/MsvcString/MsvcFunction `size_of` 断言 |
| `vector_math.rs` | Vector2/3/4 `size_of` 断言 |
| `sim_data.rs` | SimData/CellSOA/BackwallSOA + 子结构 `size_of` 断言 |
| `game_data.rs` | GameData + 27 个 *Info `size_of` 断言 |
| `element.rs` | Element/PhysicsData `size_of` 断言 |
| `disease.rs` | Disease/DiseaseInfo `size_of` 断言 |
| `sim_events.rs` | SimEvents/SimFrameInfo `size_of` 断言 |
| `frame_sync.rs` | FrameSync `size_of` 断言 |
| `sim.rs` / `sim_frame_manager.rs` | 类骨架 `size_of` 断言 |
| `message_handler.rs` | MessageType 枚举值 + MessageHandlerEntry `size_of == 16` |

### 集成测试（`tests/abi_smoke.rs` 扩展）

- Phase 0 的 10 个测试不破坏
- 新增 1 个测试：BinaryBufferReader/Writer round-trip（写入多种类型 → 读取验证）

### 验收标准

- ✅ `cargo test` 全部通过（Phase 0 的 37 个 + Phase 1 新增 ~50+ 个）
- ✅ BinaryBufferReader/Writer round-trip 单测通过
- ✅ 所有 struct 的 `size_of` 断言通过
- ✅ `cargo build --release` 产出 SimDLL.dll（与 Phase 0 行为一致）
- ✅ 用户部署后游戏仍能进主菜单（与 Phase 0 一致，无回归）

---

## §6 范围排除项

| 排除项 | 理由 | 何时做 |
|---|---|---|
| `EndianSwappedBinaryBufferReader` | YAGNI（Windows x86_64 始终小端序） | 永不（或跨平台移植时） |
| 真实 message handler 注册 | 只定义表结构，handler 是 Phase 6 | Phase 6 |
| `Load`/`Start`/`AllocateCells` 等存档加载函数 | Phase 3 范围 | Phase 3 |
| `CreateElementsTable`/`GetElementIndex` | Phase 2 元素系统 | Phase 2 |
| `Sim::Main` / `SimFrameManager::ProcessFrame` 等方法 | Phase 6+ 模拟逻辑 | Phase 6+ |
| `Sim` 类的实例化与运行 | Phase 6+ | Phase 6+ |
| 任何 `Process*` / `Update*` 业务逻辑 | 不在 Phase 1 | Phase 7-9 |
| `std::function` 真实 binder 复现 | Phase 1 只建模内存布局 | 不需要（用 MsvcFunction 占位） |

---

## §7 Phase 0 技术债顺手清理

| 编号 | 问题 | 处理 |
|---|---|---|
| P0-2 | `Cargo.toml [profile.test] panic="unwind"` 冗余 warning | 在 Phase 1 第一次改 Cargo.toml 时移除（test 默认就是 unwind） |
| P0-3 | `tests/abi_smoke.rs:6 unused c_schar import` | 在 Phase 1 扩展 abi_smoke.rs 时清理（若该 import 仍无用） |
| P0-4 | logger init 时打 "Log file: <full_path>" | **不在 Phase 1 范围**（与数据层无关），保持现状 |

---

## §8 任务拆分粒度（参考 Phase 0 模式）

预计 Phase 1 拆成 10-12 个子任务，每个子任务由子代理实现，主代理审查：

1. 创建 `stl_shim.rs`（MsvcVector/UniquePtr/MsvcString/MsvcFunction）+ size_of 单测
2. 创建 `vector_math.rs`（Vector2/3/4）+ size_of 单测
3. 重写 `buffer.rs`（BinaryBufferReader/Writer + BufferError + round-trip 单测）
4. 创建 `element.rs`（Element + PhysicsData + ElementState）+ size_of 单测
5. 创建 `disease.rs`（Disease + DiseaseInfo）+ size_of 单测
6. 创建 `sim_data.rs`（SimData + CellSOA + BackwallSOA + 8 个子结构）+ size_of 单测
7. 创建 `game_data.rs`（GameData + 27 个 *Info）+ size_of 单测
8. 创建 `sim_events.rs`（SimEvents + SimFrameInfo）+ size_of 单测
9. 扩展 `frame_sync.rs` + `sim.rs` + `sim_frame_manager.rs`（替换桩 + size_of 单测）
10. 扩展 `message_handler.rs`（Type enum 60+ 值 + MessageHandlerEntry + 空 gSimMessageHandlers）
11. 扩展 `lib.rs`（声明新模块 + 调整全局静态变量类型）
12. 扩展 `tests/abi_smoke.rs`（新增 round-trip 测试 + 清理 P0-3）+ Cargo.toml 清理 P0-2

---

## §9 开放问题（实施时需验证，brainstorming 阶段不深挖）

1. **MSVC std::vector 字段偏移**：Ghidra 显示 24 字节但访问模式表明 begin 可能在 offset 8 而非 offset 0。
2. **CellSOA 实际大小**：Ghidra 显示 264 字节但访问到 0x150 偏移，可能更大。
3. **Disease 完整字段**：Ghidra 显示 48 字节但 `operator_new(0x40)` = 64 字节，剩余 16B 待定位。
4. **std::function 实际大小**：因签名而异，64B 是估计值。
5. **8 个 SimData 子结构大小**：需在实施时分析其字段定义。
6. **27 个 GameData *Info 子结构大小**：需在实施时分析。
7. **`read_string` 字符串格式**：源码 `02_save_load.c::CreateDiseaseTable` 调用 `Disease::Disease` 传 StreamReader，但 Disease 构造函数在 00_types_reference.c 中（无源码）。先长度后字节是 ONI 序列化常见模式（与 KSerialization 一致）。实施时需确认。

这些开放问题不影响 brainstorming 推进——整体策略（严格匹配 + STL 建模 + size_of 断言）已定。具体字段偏移在实施时通过逐字段对照源码确定，遇问题以 BLOCKED 状态上报。

---

## §10 关键决策汇总

| # | 决策 | 理由 |
|---|---|---|
| 1 | BinaryBufferReader 用纯 Rust struct（不保留 vtable） | 只在 simDLL 内部构造，无 ABI 兼容需求；代码简洁 |
| 2 | 读取方法返回 `Result<T, BufferError>` | 存档损坏时不崩游戏，被 catch_unwind 捕获后优雅恢复 |
| 3 | 所有 struct 用 `#[repr(C)]` + 严格 `size_of` 断言 | 编译期自动验证字段完整性，漏字段立即编译失败 |
| 4 | STL 容器用 Rust POD struct 占位（不实现方法） | Phase 1 只做内存布局，不做容器逻辑 |
| 5 | 不实现 EndianSwappedBinaryBufferReader | YAGNI（Windows x86_64 始终小端序） |
| 6 | Phase 1 不实现任何业务方法 | 只定义数据结构，业务逻辑在 Phase 2+ |
| 7 | P0-2/P0-3 顺手清理，P0-4 推后 | 与 Phase 1 改动相关的技术债顺手清，不相关的推后 |
| 8 | 任务拆 10-12 个子任务，沿用 Phase 0 子代理驱动开发 | Phase 0 模式已验证可行 |

---

## 参考

- Phase 0 设计规格：`docs/superpowers/specs/2026-07-29-simdll-rs-foundation-design.md`
- 总路线图：`simDLL_RS/docs/roadmap.md`
- 源码分段索引：`OxygenNotIncluded_Source/simDLL_edit/INDEX.md`
- 关键源码位置：
  - BinaryBufferReader struct 定义：`00_types_reference.c` L9047-9052
  - SimData struct 定义：`00_types_reference.c` L8389-8433
  - CellSOA struct 定义：`00_types_reference.c` L11014-11026
  - GameData struct 定义：`00_types_reference.c` L3225-3269
  - Disease struct 定义：`00_types_reference.c` L9347-9350
  - Element struct 定义：`00_types_reference.c` L11104-11143
  - Type enum 定义：`00_types_reference.c` L9158-9221
  - MessageHandlerEntry struct 定义：`00_types_reference.c` L9223-9226
  - 存档加载函数（Phase 3 范围）：`02_save_load.c` L96334+
- 用户规则：助手不帮部署 simDLL.dll；用户是代码小白；严格对照源码原理，允许改造结构但需说明运行时差异
