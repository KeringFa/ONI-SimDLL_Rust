# simDLL_RS Phase 2 — 元素系统设计规格

> 起草日期：2026-07-30
> 起始 commit：`d7e42b5`（Phase 1 收官）
> 上游源码：`OxygenNotIncluded_Source/simDLL_edit/03_elements.c`（5 个函数）+ `00_types_reference.c`（12 个 struct 定义）
> 状态：草案，待用户审查后进入 writing-plans

---

## 1. 范围与目标

### 1.1 范围

实现 `03_elements.c` 中的 5 个函数，封装 15 个全局变量为 `ElementsTable` struct：

| 函数 | 源码行号 | 签名（原版） |
|---|---|---|
| `CreateElementsTable` | L27-429 | `void* __cdecl (BinaryBufferReader*)` |
| `DestroyElementsTable` | L430-453 | `void __cdecl (void)` |
| `GetElementIndex` | L456-495 | `ushort __cdecl (uint hash)` |
| `CreateElementInteractions` | L517-569 | `void* __cdecl (BinaryBufferReader*)` |
| `CreateElementInteractionsLocked` | L7-23 | `void* __cdecl (BinaryBufferReader*)` |

### 1.2 目标

- 5 个函数作为 `#[no_mangle] extern "C"` 导出，可被 Phase 6 消息分派调用
- 12 个新 struct + 1 个 type alias（`ElementPhysicsData = PhysicsData`），全部 `#[repr(C)]` 精确对照源码字节布局
- 单元测试覆盖：struct 大小 + 反序列化 round-trip
- 集成测试覆盖：5 个导出函数不 panic

### 1.3 非目标

- **不**实现消息分派（Phase 6 才将函数指针注册到 `gSimMessageHandlers` 表）
- **不**实现 yaml 读取（simDLL 不直接读 yaml，C# 侧读 yaml 后序列化成 BinaryBuffer 传给 simDLL——见 §3.1）
- **不**替换 Phase 1 的 `MsvcMutex` 字节数组占位（Phase 3+ 统一处理）
- **不**实现 `mSimMutex` 真实 lock（用独立 `parking_lot::Mutex`——见 §6.2）

### 1.4 部署后预期行为（告知用户）

- **无可见变化**——这些是消息处理函数，要等 Phase 6 消息分派才会被 C# 触发
- 游戏进主菜单、加载存档行为与 Phase 1 完全一致
- 日志无新增条目（除非 C# 主动调用这些函数，Phase 2 之前不会发生）
- 验收靠单元测试 + 集成测试 + 部署无回归（与 Phase 1 相同）

---

## 2. 上游源码研究结论

### 2.1 数据来源澄清（修正 roadmap.md）

`roadmap.md` L61 当前写"加载 `OxygenNotIncluded_Data/StreamingAssets/elements/gas.yaml`"，这是误导。

**实际流程**：
1. C# 侧读 yaml 文件（`gas.yaml` / `liquid.yaml` / `solid.yaml` / `special.yaml`）
2. C# 序列化成 `BinaryBuffer`（含 count + Element 数据 + Element 名称 + 9 个附属数据源）
3. C# 通过 `SIM_HandleMessage(Elements_CreateTable, ...)` 把 buffer 传给 simDLL
4. simDLL 的 `CreateElementsTable` 从 `BinaryBufferReader` 反序列化

simDLL **不直接读 yaml**。本 Phase 2 的实现目标是步骤 4。

### 2.2 5 个函数源码行为分析

#### 2.2.1 `CreateElementsTable`（4 阶段 + 后处理）

源码 `03_elements.c` L27-429：

```
阶段 0 (L79): 调用 DestroyElementsTable() 清空旧数据
阶段 1 (L80-100):
    - read_int(count)  // L80: (**(code**)(param_1->_padding_ + 0x38))(param_1, &local_c8)
    - 11 个 vector::resize(count)  // L86-96
    - local_c4 = buffer_data + offset  // L81: 指向 Element 数据起点
    - memcpy(gElements, local_c4, count * 0xa4)  // L99
    - buffer.skip(count * 0xa4)  // L100
阶段 2 (L101-179): 循环 count 次，建立 gElementIndices:
    - local_b8 = Element[i].id  // L110: *(uint*)(i * 0xa4 + gElements._8_8_)
    - find(local_b8) in gElementIndices  // L111
    - if not found: insert new node  // L122-156
    - list_node.value@0x14 = uVar19 (current index)  // L175
阶段 3 (L180-250): 循环 count 次，读 gElementNames:
    - read_int(len)
    - read string (len bytes) → assign to gElementNames[i]
    - buffer.skip(len)  // SkipBytes(L244)
阶段 4 (L259-392): 循环 count 次，逐字段拷贝到 9 个附属 vector
    - 字段映射见 §2.3
后处理 (L396-425): 循环 count 次，填充 ElementTemperatureData 的:
    - lowTempTransitionOreIdx@6 = GetElementIndex(Element.lowTempTransitionOreID) or 0xffff
    - highTempTransitionOreIdx@8 = GetElementIndex(Element.highTempTransitionOreID) or 0xffff
    - 无效哈希常量: 0x2d39bf75 (oreID == 此值 → 结果 0xffff)
返回: (void*)(longlong)local_c8  // 即 count
```

#### 2.2.2 `DestroyElementsTable`（L430-453）

```
- std::_Destroy_range(gElementNames)  // 析构每个 MSVC string
- 11 个 vector: _Last = _First  // clear 语义，不释放容量
- gElementIndices.list::clear()
- gElementIndices.hash::_Init(8)  // 重置 hash 表
```

#### 2.2.3 `GetElementIndex`（L456-495）

```
- uVar4 = 0xffff  // 默认未找到
- find(hash) in gElementIndices
- if iterator valid && iterator != end:
    - uVar4 = *(ushort*)(iter + 0x14)  // 读 list_node 的 value@0x14
- return uVar4
```

#### 2.2.4 `CreateElementInteractions`（L517-569）

```
- gLiquidObliterations._Last = _First  // 清空（不释放容量）
- gLiquidConversions._Last = _First
- gGasObliterations._Last = _First
- count = *(int*)(buffer + offset)
- 循环 count 次，每次读 0x20=32 字节 entry:
    - if entry.signature == 0x6826f67d (GasObliteration 签名):
        - 提取 8 个字段 → push 到 gGasObliterations
- return null
```

**注意**：源码中 `gLiquidObliterations` / `gLiquidConversions` 只清空不填充。可能是 Ghidra 反编译缺失分支，或源码确实如此。**Phase 2 严格按源码实现**，只清空不填充这两个 vector。

#### 2.2.5 `CreateElementInteractionsLocked`（L7-23）

```
- _Mtx_lock(&gFrameSync.mSimMutex)
- if lock failed: std::_Throw_C_error
- CreateElementInteractions(reader)
- _Mtx_unlock(&gFrameSync.mSimMutex)
- if unlock failed: std::_Throw_C_error
- return result
```

### 2.3 9 个附属 struct 字段映射表

字段映射基于源码 L259-392 的逐字段拷贝逻辑反推 + `00_types_reference.c` struct 定义验证。

#### 2.3.1 `ElementTemperatureData`（stride 0x38=56B）

源码 struct: `00_types_reference.c` L15462-15481（16 字段）。

| dest 偏移 | dest 类型 | dest 字段 | source 偏移（Element） | source 字段 | 备注 |
|---|---|---|---|---|---|
| 0 | u8 | state | 0xa | state | |
| 2 | u16 | lowTempTransitionIdx | 4 | lowTempTransitionIdx | 直接拷贝（非 hash 查找） |
| 4 | u16 | highTempTransitionIdx | 6 | highTempTransitionIdx | 直接拷贝 |
| 6 | u16 | lowTempTransitionOreIdx | — | — | 后处理阶段填入（见下） |
| 8 | u16 | highTempTransitionOreIdx | — | — | 后处理阶段填入 |
| 0xc | f32 | specificHeatCapacity | 0xc | specificHeatCapacity | |
| 0x10 | f32 | thermalConductivity | 0x10 | thermalConductivity | |
| 0x14 | f32 | defaultMass | 0x9c | defaultValues.mass | PhysicsData.mass |
| 0x18 | f32 | massAreaScale | — | — | **运行时计算**：state&3==1 ? 1.0 : 0.001 |
| 0x1c | f32 | gasSurfaceAreaMultiplier | 0x20 | gasSurfaceAreaMultiplier | |
| 0x20 | f32 | liquidSurfaceAreaMultiplier | 0x1c | liquidSurfaceAreaMultiplier | |
| 0x24 | f32 | solidSurfaceAreaMultiplier | 0x18 | solidSurfaceAreaMultiplier | |
| 0x28 | f32 | lowTemp | 0x38 | lowTemp | |
| 0x2c | f32 | highTemp | 0x3c | highTemp | |
| 0x30 | f32 | lowTempTransitionOreMassConversion | 0x48 | lowTempTransitionOreMassConversion | |
| 0x34 | f32 | highTempTransitionOreMassConversion | 0x50 | highTempTransitionOreMassConversion | |

**后处理阶段（L396-425）**：
```rust
lowTempTransitionOreIdx = if Element.lowTempTransitionOreID == 0x2d39bf75 {
    0xffff
} else {
    get_element_index(Element.lowTempTransitionOreID as u32)
};
highTempTransitionOreIdx = if Element.highTempTransitionOreID == 0x2d39bf75 {
    0xffff
} else {
    get_element_index(Element.highTempTransitionOreID as u32)
};
```

`0x2d39bf75` 是无效元素 ID 的哈希常量（推测是 "Invalid" 或 "Vacuum" 等占位元素的哈希）。源码中遇到此值表示"无相变"，结果索引设为 `0xffff`（65535，表示无效索引）。

#### 2.3.2 `ElementPostProcessData`（stride 0x2c=44B）

源码 struct: `00_types_reference.c` L10144-10157（12 字段）。

| dest 偏移 | dest 类型 | dest 字段 | source 偏移 | source 字段 |
|---|---|---|---|---|
| 0 | u16 | sublimateIndex | 0x54 | sublimateIndex |
| 2 | u16 | convertIndex | 0x56 | convertIndex |
| 4 | u8 | state | 0xa | state |
| 8 | f32 | sublimateRate | 0x7c | sublimateRate |
| 0xc | f32 | sublimateEfficiency | 0x80 | sublimateEfficiency |
| 0x10 | f32 | sublimateProbability | 0x84 | sublimateProbability |
| 0x14 | f32 | offGasPercentage | 0x88 | offGasPercentage |
| 0x18 | f32 | molarMass | 0x14 | molarMass |
| 0x1c | f32 | strength | 0x40 | strength |
| 0x20 | f32 | maxMass | 0x34 | maxMass |
| 0x24 | f32 | minHorizontalFlow | 0x2c | minHorizontalFlow |
| 0x28 | i32 | sublimateFX | 0x78 | sublimateFX |

注：state@4 + 3 字节 padding @5-7。

#### 2.3.3 `ElementLiquidData`（stride 0x18=24B）

源码 struct: `00_types_reference.c` L8802-8809（6 字段）。

| dest 偏移 | dest 类型 | dest 字段 | source 偏移 | source 字段 |
|---|---|---|---|---|
| 0 | u8 | state | 0xa | state |
| 4 | f32 | flow | 0x24 | flow |
| 8 | f32 | viscosity | 0x28 | viscosity |
| 0xc | f32 | minHorizontalFlow | 0x2c | minHorizontalFlow |
| 0x10 | f32 | minVerticalFlow | 0x30 | minVerticalFlow |
| 0x14 | f32 | maxMass | 0x34 | maxMass |

注：state@0 + 3 字节 padding @1-3。

#### 2.3.4 `ElementPhysicsData`（stride 0xc=12B）— 复用 `PhysicsData`

源码 struct: `00_types_reference.c` 无独立定义。源码 L320-328 直接拷贝 Element.defaultValues 的 12 字节。

```rust
pub type ElementPhysicsData = PhysicsData;  // 12B = temperature + mass + pressure
```

字段映射：直接拷贝 `Element.defaultValues`（PhysicsData@0x98）的 12 字节。

#### 2.3.5 `ElementPressureData`（stride 0x8=8B）

源码 struct: `00_types_reference.c` L5474-5477（2 字段）。

| dest 偏移 | dest 类型 | dest 字段 | source 偏移 | source 字段 |
|---|---|---|---|---|
| 0 | u8 | state | 0xa | state |
| 4 | f32 | flow | 0x24 | flow |

注：state@0 + 3 字节 padding @1-3。

#### 2.3.6 `ElementPropertyTextureData`（stride 0x24=36B）

源码 struct: `00_types_reference.c` L9598-9604（5 字段，含 [u32;6] 数组）。

| dest 偏移 | dest 类型 | dest 字段 | source 偏移 | source 字段 |
|---|---|---|---|---|
| 0 | u32 | colour | 0x5c | colour |
| 4 | [u32;6] | gradient | 0x60 | gradient_colours[6] |
| 0x1c | u32 | materialProperties | 0x58 | materialProperties |
| 0x20 | u8 | state | 0xa | state |
| 0x21 | u8 | numberOfGradientColors | 0xb | numberOfGradientColors |

注：state@0x20 + numberOfGradientColors@0x21 + 2 字节 padding @0x22-0x23。

源码 L347-351 用 `undefined8`（8 字节）一次拷贝 gradient[0..1]、gradient[2..3]、gradient[4..5] 共 24 字节。Rust 实现可逐 u32 拷贝或用 `copy_from_slice`，结果等价。

#### 2.3.7 `ElementLightAbsorptionData`（stride 0x8=8B）

源码 struct: `00_types_reference.c` L13306-13309（2 字段）。

| dest 偏移 | dest 类型 | dest 字段 | source 偏移 | source 字段 | 备注 |
|---|---|---|---|---|---|
| 0 | f32 | factor | 0x8c | lightAbsorptionFactor | |
| 4 | f32 | massScale | — | — | **运行时计算** |

**massScale 计算式**（源码 L362-368）：
```rust
let mass_scale = if element.state & 3 == 3 {
    // solid state (state&3==3): INFINITY
    f32::INFINITY
} else {
    1.0 / element.default_values.mass
};
```

注：`state & 3` 是元素状态的低 2 位（state enum 的低 3 位编码 state 类型）。`state & 3 == 3` 表示固态，固态不参与 mass scale 计算（返回 ∞）。

#### 2.3.8 `ElementRadiationData`（stride 0x8=8B）

源码 struct: `00_types_reference.c` L7624-7627（2 字段）。

| dest 偏移 | dest 类型 | dest 字段 | source 偏移 | source 字段 |
|---|---|---|---|---|
| 0 | f32 | factor | 0x90 | radiationAbsorptionFactor |
| 4 | f32 | rads_per_1000 | 0x94 | radiationPer1000Mass |

#### 2.3.9 `ElementStateData`（stride 0x1=1B）

源码 struct: `00_types_reference.c` L13476-13478（1 字段）。

| dest 偏移 | dest 类型 | dest 字段 | source 偏移 | source 字段 |
|---|---|---|---|---|
| 0 | u8 | state | 0xa | state |

### 2.4 3 个交互 struct 字段定义

#### 2.4.1 `GasObliteration`（stride 0x1c=28B）

源码 struct: `00_types_reference.c` L12220-12229（8 字段）。

| dest 偏移 | dest 类型 | dest 字段 | source entry 偏移 |
|---|---|---|---|
| 0 | u16 | elemIdx1 | 4 |
| 2 | u16 | elemIdx2 | 6 |
| 4 | u16 | elemResultIdx | 8 |
| 8 | f32 | interactionProbability | 0x10 |
| 0xc | f32 | minMass | 0xc |
| 0x10 | f32 | elem1MassDestructionPercent | 0x14 |
| 0x14 | f32 | elem2MassRequiredMultiplier | 0x18 |
| 0x18 | f32 | elemResultMassCreationMultiplier | 0x1c |

注：elemIdx1@0 + elemIdx2@2 + elemResultIdx@4 + 2 字节 padding @6-7。

#### 2.4.2 `LiquidObliteration`（stride 0x10=16B）

源码 struct: `00_types_reference.c` L10516-10522（5 字段）。

| dest 偏移 | dest 类型 | dest 字段 |
|---|---|---|
| 0 | u16 | elemIdx1 |
| 2 | u16 | elemIdx2 |
| 4 | i16 | prefabIdx |
| 8 | f32 | massThreshold |
| 0xc | f32 | probability |

注：elemIdx1@0 + elemIdx2@2 + prefabIdx@4 + 2 字节 padding @6-7。

#### 2.4.3 `LiquidConversion`（stride 0xc=12B）

源码 struct: `00_types_reference.c` L8963-8968（4 字段）。

| dest 偏移 | dest 类型 | dest 字段 |
|---|---|---|
| 0 | u16 | elemIdx1 |
| 2 | u16 | elemIdx2 |
| 4 | f32 | massRatio |
| 8 | f32 | probability |

### 2.5 15 个全局变量清单

| 全局变量 | 类型 | stride | Phase 2 Rust 类型 |
|---|---|---|---|
| gElements | vector<Element> | 0xa4=164B | `Vec<Element>` |
| gElementNames | vector<basic_string> | 0x28=40B | `Vec<String>` |
| gElementTemperatureData | vector<ElementTemperatureData> | 0x38=56B | `Vec<ElementTemperatureData>` |
| gElementPostProcessData | vector<ElementPostProcessData> | 0x2c=44B | `Vec<ElementPostProcessData>` |
| gElementLiquidData | vector<ElementLiquidData> | 0x18=24B | `Vec<ElementLiquidData>` |
| gElementPhysicsData | vector<PhysicsData> | 0xc=12B | `Vec<PhysicsData>` |
| gElementPressureData | vector<ElementPressureData> | 0x8=8B | `Vec<ElementPressureData>` |
| gElementPropertyTextureData | vector<ElementPropertyTextureData> | 0x24=36B | `Vec<ElementPropertyTextureData>` |
| gElementLightAbsorptionData | vector<ElementLightAbsorptionData> | 0x8=8B | `Vec<ElementLightAbsorptionData>` |
| gElementRadiationData | vector<ElementRadiationData> | 0x8=8B | `Vec<ElementRadiationData>` |
| gElementStateData | vector<ElementStateData> | 0x1=1B | `Vec<ElementStateData>` |
| gElementIndices | std::_Hash<uint,ushort> | — | `HashMap<u32, u16>` |
| gGasObliterations | vector<GasObliteration> | 0x1c=28B | `Vec<GasObliteration>` |
| gLiquidObliterations | vector<LiquidObliteration> | 0x10=16B | `Vec<LiquidObliteration>` |
| gLiquidConversions | vector<LiquidConversion> | 0xc=12B | `Vec<LiquidConversion>` |

---

## 3. 设计方案

### 3.1 数据来源（决策已确认）

按源码实际逻辑，从 `BinaryBufferReader` 反序列化。**不引入 yaml 库**。复用 Phase 1 已实现的 `BinaryBufferReader`（buffer.rs）。

### 3.2 架构方案（决策已确认：方案 B）

封装为 `ElementsTable` struct，用 `once_cell::sync::Lazy<Mutex<ElementsTable>>` 全局持有。13+ 个全局变量集中到一个 struct。

**与原版差异（原理不变，仅实现差异）**：
- 原版 `gElementNames` 是 `vector<basic_string>`（MSVC SSO 32B/元素），Rust 用 `Vec<String>`（24B/元素）——仅内存表示不同，运行时数据完全等价
- 原版 `gElementIndices` 是 `std::_Hash`（基于 `std::list`），Rust 用 `HashMap<u32, u16>`——查找语义相同（key→value），不暴露迭代顺序。**无运行时影响**，因为代码只做 `find(key)`，从不迭代
- 原版 13 个全局变量分散，Rust 集中到一个 struct——逻辑等价，便于生命周期管理

---

## 4. 数据结构层

### 4.1 新增 struct（追加到 `element.rs`）

新增 9 个附属 struct + 3 个交互 struct + 1 个 type alias：

```rust
// 9 个附属 struct（按 §2.3 字段映射定义，全部 #[repr(C)] + Clone, Copy, Default）

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementTemperatureData {
    pub state: u8,
    _pad1: u8,                              // @1
    pub low_temp_transition_idx: u16,       // @2
    pub high_temp_transition_idx: u16,       // @4
    pub low_temp_transition_ore_idx: u16,    // @6
    pub high_temp_transition_ore_idx: u16,    // @8
    _pad2: u16,                              // @0xa
    pub specific_heat_capacity: f32,         // @0xc
    pub thermal_conductivity: f32,           // @0x10
    pub default_mass: f32,                   // @0x14
    pub mass_area_scale: f32,               // @0x18
    pub gas_surface_area_multiplier: f32,    // @0x1c
    pub liquid_surface_area_multiplier: f32, // @0x20
    pub solid_surface_area_multiplier: f32,  // @0x24
    pub low_temp: f32,                       // @0x28
    pub high_temp: f32,                      // @0x2c
    pub low_temp_transition_ore_mass_conversion: f32,  // @0x30
    pub high_temp_transition_ore_mass_conversion: f32, // @0x34
}
// size_of = 56 (0x38)

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementPostProcessData {
    pub sublimate_index: u16,            // @0
    pub convert_index: u16,              // @2
    pub state: u8,                       // @4
    _pad1: [u8; 3],                      // @5-7
    pub sublimate_rate: f32,             // @8
    pub sublimate_efficiency: f32,       // @0xc
    pub sublimate_probability: f32,      // @0x10
    pub off_gas_percentage: f32,         // @0x14
    pub molar_mass: f32,                // @0x18
    pub strength: f32,                  // @0x1c
    pub max_mass: f32,                  // @0x20
    pub min_horizontal_flow: f32,        // @0x24
    pub sublimate_fx: i32,              // @0x28
}
// size_of = 44 (0x2c)

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementLiquidData {
    pub state: u8,                  // @0
    _pad1: [u8; 3],                 // @1-3
    pub flow: f32,                  // @4
    pub viscosity: f32,            // @8
    pub min_horizontal_flow: f32,   // @0xc
    pub min_vertical_flow: f32,    // @0x10
    pub max_mass: f32,             // @0x14
}
// size_of = 24 (0x18)

pub type ElementPhysicsData = PhysicsData;  // 12B，复用 Phase 1

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementPressureData {
    pub state: u8,        // @0
    _pad1: [u8; 3],       // @1-3
    pub flow: f32,        // @4
}
// size_of = 8

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementPropertyTextureData {
    pub colour: u32,                       // @0
    pub gradient: [u32; 6],                // @4
    pub material_properties: u32,         // @0x1c
    pub state: u8,                         // @0x20
    pub number_of_gradient_colors: u8,     // @0x21
    _pad1: [u8; 2],                        // @0x22-0x23
}
// size_of = 36 (0x24)

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementLightAbsorptionData {
    pub factor: f32,      // @0
    pub mass_scale: f32,  // @4
}
// size_of = 8

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementRadiationData {
    pub factor: f32,         // @0
    pub rads_per_1000: f32,  // @4
}
// size_of = 8

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementStateData {
    pub state: u8,  // @0
}
// size_of = 1

// 3 个交互 struct
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GasObliteration {
    pub elem_idx1: u16,                              // @0
    pub elem_idx2: u16,                              // @2
    pub elem_result_idx: u16,                        // @4
    _pad1: u16,                                      // @6
    pub interaction_probability: f32,                // @8
    pub min_mass: f32,                               // @0xc
    pub elem1_mass_destruction_percent: f32,         // @0x10
    pub elem2_mass_required_multiplier: f32,          // @0x14
    pub elem_result_mass_creation_multiplier: f32,   // @0x18
}
// size_of = 28 (0x1c)

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LiquidObliteration {
    pub elem_idx1: u16,        // @0
    pub elem_idx2: u16,        // @2
    pub prefab_idx: i16,       // @4
    _pad1: u16,                // @6
    pub mass_threshold: f32,   // @8
    pub probability: f32,     // @0xc
}
// size_of = 16 (0x10)

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LiquidConversion {
    pub elem_idx1: u16,    // @0
    pub elem_idx2: u16,    // @2
    pub mass_ratio: f32,   // @4
    pub probability: f32, // @8
}
// size_of = 12 (0xc)
```

### 4.2 常量定义

```rust
// 在 element.rs 或 elements_table.rs 顶部

/// 无效元素 ID 哈希常量（对应源码 0x2d39bf75）。
/// oreID == 此值时表示"无相变"，GetElementIndex 结果设为 0xffff。
pub const INVALID_ELEMENT_HASH: u32 = 0x2d39bf75;

/// 无效元素索引（对应源码 0xffff = 65535）。
pub const INVALID_ELEMENT_INDEX: u16 = 0xffff;

/// GasObliteration 签名常量（对应源码 0x6826f67d）。
/// CreateElementInteractions 中 entry.signature == 此值才提取 GasObliteration。
pub const GAS_OBLITERATION_SIGNATURE: i32 = 0x6826f67d_u32 as i32;
```

---

## 5. ElementsTable 全局状态

### 5.1 新建 `elements_table.rs` 模块

```rust
//! ElementsTable — 元素表全局状态（封装 15 个全局变量）。
//!
//! 对照源码 03_elements.c 中的 15 个全局变量：
//! gElements / gElementNames / gElementTemperatureData / gElementPostProcessData /
//! gElementLiquidData / gElementPhysicsData / gElementPressureData /
//! gElementPropertyTextureData / gElementLightAbsorptionData / gElementRadiationData /
//! gElementStateData / gElementIndices / gGasObliterations / gLiquidObliterations /
//! gLiquidConversions。

use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;

use crate::element::*;

/// 元素表全局状态——封装原版 13+ 个分散全局变量。
pub struct ElementsTable {
    pub elements: Vec<Element>,
    pub element_names: Vec<String>,
    pub temperature_data: Vec<ElementTemperatureData>,
    pub post_process_data: Vec<ElementPostProcessData>,
    pub liquid_data: Vec<ElementLiquidData>,
    pub physics_data: Vec<PhysicsData>,
    pub pressure_data: Vec<ElementPressureData>,
    pub property_texture_data: Vec<ElementPropertyTextureData>,
    pub light_absorption_data: Vec<ElementLightAbsorptionData>,
    pub radiation_data: Vec<ElementRadiationData>,
    pub state_data: Vec<ElementStateData>,
    pub element_indices: HashMap<u32, u16>,
    pub gas_obliterations: Vec<GasObliteration>,
    pub liquid_obliterations: Vec<LiquidObliteration>,
    pub liquid_conversions: Vec<LiquidConversion>,
}

impl Default for ElementsTable {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            element_names: Vec::new(),
            temperature_data: Vec::new(),
            post_process_data: Vec::new(),
            liquid_data: Vec::new(),
            physics_data: Vec::new(),
            pressure_data: Vec::new(),
            property_texture_data: Vec::new(),
            light_absorption_data: Vec::new(),
            radiation_data: Vec::new(),
            state_data: Vec::new(),
            element_indices: HashMap::new(),
            gas_obliterations: Vec::new(),
            liquid_obliterations: Vec::new(),
            liquid_conversions: Vec::new(),
        }
    }
}

/// 全局 ElementsTable 单例。
pub static G_ELEMENTS_TABLE: Lazy<Mutex<ElementsTable>> =
    Lazy::new(|| Mutex::new(ElementsTable::default()));
```

### 5.2 在 `lib.rs` 添加模块声明

```rust
pub mod elements_table;
```

### 5.3 在 `lib.rs` 的 `globals` 子模块补充

不需要新增（`G_ELEMENTS_TABLE` 在 `elements_table.rs` 内部定义）。

---

## 6. 5 个函数实现

### 6.1 `create_elements_table`（内部函数）

```rust
fn create_elements_table(reader: &mut BinaryBufferReader) -> Result<i32, BufferError> {
    // 阶段 0: 清空旧数据
    destroy_elements_table();

    let mut table = G_ELEMENTS_TABLE.lock().unwrap();

    // 阶段 1: 读 count + memcpy Element 数据 + skip
    let count = reader.read_int()?;
    if count < 0 || count > 0xfffe {
        // 源码 L82: if (0xfffe < local_c8) && !gAssertsDisabled && IsDebuggerPresent() DebugBreak()
        tracing::warn!(count, "CreateElementsTable: count exceeds 0xfffe, may indicate corrupted data");
    }
    let count_usize = count.max(0) as usize;

    // 预分配所有 vector
    table.elements.resize(count_usize, Element::default());  // 注意：Element 需要 Default
    table.element_names.resize(count_usize, String::new());
    table.temperature_data.resize(count_usize, ElementTemperatureData::default());
    // ... 其余 9 个 vector 同样 resize

    // memcpy Element 数据（直接读 count * 164 字节）
    for i in 0..count_usize {
        let mut bytes = [0u8; 164];
        reader.read_bytes(&mut bytes)?;
        // 安全转换：Element 是 #[repr(C)] 且 164 字节
        table.elements[i] = unsafe { std::ptr::read(bytes.as_ptr() as *const Element) };
    }

    // 阶段 2: 建立 element_indices（Element.id → index）
    table.element_indices.clear();
    for (i, elem) in table.elements.iter().enumerate() {
        table.element_indices.insert(elem.id as u32, i as u16);
        // 源码 L175: list_node.value = uVar19（当前 index）。重复 id 后插入覆盖前一个。
    }

    // 阶段 3: 读 element_names
    for i in 0..count_usize {
        let len = reader.read_int()? as usize;
        let mut buf = vec![0u8; len];
        reader.read_bytes(&mut buf)?;
        table.element_names[i] = String::from_utf8(buf)
            .map_err(|_| BufferError::InvalidUtf8)?;
    }

    // 阶段 4: 逐字段填充 9 个附属 vector
    for i in 0..count_usize {
        let elem = &table.elements[i];

        // ElementTemperatureData（按 §2.3.1 字段映射）
        let mut td = ElementTemperatureData::default();
        td.state = elem.state;
        td.low_temp_transition_idx = elem.low_temp_transition_idx;
        td.high_temp_transition_idx = elem.high_temp_transition_idx;
        td.specific_heat_capacity = elem.specific_heat_capacity;
        td.thermal_conductivity = elem.thermal_conductivity;
        td.default_mass = elem.default_values.mass;
        td.mass_area_scale = if elem.state & 3 == 1 { 1.0 } else { 0.001 };
        td.gas_surface_area_multiplier = elem.gas_surface_area_multiplier;
        td.liquid_surface_area_multiplier = elem.liquid_surface_area_multiplier;
        td.solid_surface_area_multiplier = elem.solid_surface_area_multiplier;
        td.low_temp = elem.low_temp;
        td.high_temp = elem.high_temp;
        td.low_temp_transition_ore_mass_conversion = elem.low_temp_transition_ore_mass_conversion;
        td.high_temp_transition_ore_mass_conversion = elem.high_temp_transition_ore_mass_conversion;
        // low_temp_transition_ore_idx / high_temp_transition_ore_idx 在后处理阶段填入
        table.temperature_data[i] = td;

        // ElementPostProcessData / ElementLiquidData / ElementPhysicsData /
        // ElementPressureData / ElementPropertyTextureData / ElementLightAbsorptionData /
        // ElementRadiationData / ElementStateData —— 同样逐字段映射
        // （省略，详见 §2.3.2-2.3.9）
    }

    // 后处理阶段：填充 low_temp_transition_ore_idx / high_temp_transition_ore_idx
    // 需要先释放 table 锁以避免 GetElementIndex 死锁（GetElementIndex 也要锁）
    drop(table);
    for i in 0..count_usize {
        let elem = {
            let table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements[i]
        };

        let low_idx = if elem.low_temp_transition_ore_id as u32 == INVALID_ELEMENT_HASH {
            INVALID_ELEMENT_INDEX
        } else {
            get_element_index(elem.low_temp_transition_ore_id as u32)
        };
        let high_idx = if elem.high_temp_transition_ore_id as u32 == INVALID_ELEMENT_HASH {
            INVALID_ELEMENT_INDEX
        } else {
            get_element_index(elem.high_temp_transition_ore_id as u32)
        };

        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.temperature_data[i].low_temp_transition_ore_idx = low_idx;
        table.temperature_data[i].high_temp_transition_ore_idx = high_idx;
    }

    Ok(count)
}
```

**注意**：
- `Element` 当前在 element.rs 中只 derive `Clone, Copy`，**未 derive `Default`**。需要在 Phase 2 给 Element 添加 `Default`（用 `#[derive(Default)]` 或手动实现——i32/u16/u8/f32 都有 Default，应该可以自动 derive）
- 后处理阶段的锁释放/重获是为了避免 `get_element_index` 死锁。源码中 `GetElementIndex` 读取 `gElementIndices` 不需要锁（C++ 全局变量无锁保护），Rust 实现需要考虑并发安全。**或者**：把 `get_element_index` 改成内部函数 `fn get_element_index_inner(table: &ElementsTable, hash: u32) -> u16`，避免重复加锁。

### 6.2 `create_element_interactions`（内部函数）

```rust
fn create_element_interactions(reader: &mut BinaryBufferReader) -> Result<(), BufferError> {
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();

    // 清空 3 个交互 vector
    table.liquid_obliterations.clear();
    table.liquid_conversions.clear();
    table.gas_obliterations.clear();

    let count = reader.read_int()?;
    if count < 0 {
        return Ok(());  // 防御性：负 count 视为 0
    }

    for _ in 0..count {
        // 每个 entry 0x20=32 字节
        let signature = reader.read_int()?;
        let elem_idx1 = reader.read_ushort()?;
        let elem_idx2 = reader.read_ushort()?;
        let elem_result_idx = reader.read_ushort()?;
        let _padding = reader.read_ushort()?;  // @0xa-0xb（源码未使用）
        let min_mass = reader.read_float()?;
        let interaction_probability = reader.read_float()?;
        let elem1_mass_destruction_percent = reader.read_float()?;
        let elem2_mass_required_multiplier = reader.read_float()?;
        let elem_result_mass_creation_multiplier = reader.read_float()?;
        // 共读 4 + 2*3 + 2 + 5*4 = 30 字节，剩余 2 字节对齐 padding（@0x1e-0x1f）

        // 跳过对齐 padding（如源码 entry stride 0x20=32 而我们读了 30 字节）
        // 实际：源码每 entry 32 字节，我们已读 30 字节，需 skip 2 字节
        // 但 reader 已自动前进，这里需要确认对齐
        // 简化：reader.skip(2)?;

        if signature == GAS_OBLITERATION_SIGNATURE {
            table.gas_obliterations.push(GasObliteration {
                elem_idx1,
                elem_idx2,
                elem_result_idx,
                _pad1: 0,
                interaction_probability,
                min_mass,
                elem1_mass_destruction_percent,
                elem2_mass_required_multiplier,
                elem_result_mass_creation_multiplier,
            });
        }
        // else: 签名不匹配，源码中跳过此 entry（不 push）
    }

    Ok(())
}
```

**注意**：source entry stride 是 0x20=32 字节，但提取的字段总和是 30 字节（4+2+2+2+2+4+4+4+4+4）。剩余 2 字节是 padding。需要 `reader.skip(2)?` 或者在循环开始时按 32 字节步进读取。

### 6.3 `destroy_elements_table`（内部函数）

```rust
fn destroy_elements_table() {
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();
    table.elements.clear();
    table.element_names.clear();
    table.temperature_data.clear();
    table.post_process_data.clear();
    table.liquid_data.clear();
    table.physics_data.clear();
    table.pressure_data.clear();
    table.property_texture_data.clear();
    table.light_absorption_data.clear();
    table.radiation_data.clear();
    table.state_data.clear();
    table.element_indices.clear();
    // 注意：原版 DestroyElementsTable 不清空 3 个交互 vector
    // （gas_obliterations / liquid_obliterations / liquid_conversions）
    // 严格对照源码：本函数不清空这 3 个 vector
}
```

### 6.4 `get_element_index`（内部函数）

```rust
fn get_element_index(hash: u32) -> u16 {
    let table = G_ELEMENTS_TABLE.lock().unwrap();
    *table.element_indices.get(&hash).unwrap_or(&INVALID_ELEMENT_INDEX)
}
```

### 6.5 C ABI 导出（5 个 `#[no_mangle] extern "C"` 函数）

```rust
use std::os::raw::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};

#[no_mangle]
pub extern "C" fn CreateElementsTable(reader: *mut BinaryBufferReader) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        match unsafe { reader.as_mut() } {
            Some(reader) => match create_elements_table(reader) {
                Ok(count) => count as *mut c_void,  // 源码返回 (void*)(longlong)count
                Err(e) => {
                    tracing::error!(?e, "CreateElementsTable failed");
                    std::ptr::null_mut()
                }
            },
            None => std::ptr::null_mut(),
        }
    }))
    .unwrap_or_else(|_| {
        tracing::error!("CreateElementsTable panicked");
        std::ptr::null_mut()
    })
}

#[no_mangle]
pub extern "C" fn DestroyElementsTable() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        destroy_elements_table();
    }));
}

#[no_mangle]
pub extern "C" fn GetElementIndex(hash: u32) -> u16 {
    catch_unwind(AssertUnwindSafe(|| {
        get_element_index(hash)
    }))
    .unwrap_or(INVALID_ELEMENT_INDEX)
}

#[no_mangle]
pub extern "C" fn CreateElementInteractions(reader: *mut BinaryBufferReader) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        match unsafe { reader.as_mut() } {
            Some(reader) => match create_element_interactions(reader) {
                Ok(()) => std::ptr::null_mut(),
                Err(e) => {
                    // 决策 #1：错误处理策略 A —— 记日志 + 返回 null（不崩溃游戏）
                    tracing::error!(?e, "CreateElementInteractions failed, partial data may remain");
                    std::ptr::null_mut()
                }
            },
            None => std::ptr::null_mut(),
        }
    }))
    .unwrap_or_else(|_| {
        tracing::error!("CreateElementInteractions panicked");
        std::ptr::null_mut()
    })
}

#[no_mangle]
pub extern "C" fn CreateElementInteractionsLocked(reader: *mut BinaryBufferReader) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        // 决策 #2：用 parking_lot::Mutex 独立锁，不绑定到 gFrameSync.mSimMutex
        let _guard = G_INTERACTIONS_LOCK.lock();
        // 调用内部 create_element_interactions（不再单独包 catch_unwind，由本函数包）
        match unsafe { reader.as_mut() } {
            Some(reader) => match create_element_interactions(reader) {
                Ok(()) => std::ptr::null_mut(),
                Err(e) => {
                    tracing::error!(?e, "CreateElementInteractionsLocked failed");
                    std::ptr::null_mut()
                }
            },
            None => std::ptr::null_mut(),
        }
    }))
    .unwrap_or_else(|_| {
        tracing::error!("CreateElementInteractionsLocked panicked");
        std::ptr::null_mut()
    })
}
```

### 6.6 独立锁（决策 #2）

```rust
// 在 elements_table.rs 顶部
use parking_lot::Mutex;

/// CreateElementInteractionsLocked 用的独立锁，对应原版 gFrameSync.mSimMutex。
/// Phase 3+ 替换 MsvcMutex 时统一接入。
static G_INTERACTIONS_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
```

**注**：Phase 1 已依赖 `parking_lot`（见 progress.md §全局约束"依赖最小化"清单），无需新增依赖。

---

## 7. 错误处理策略（决策 #1 已确认）

### 7.1 策略 A：记日志 + 尽力而为

| 错误场景 | 原版行为 | Rust 行为 | 理由 |
|---|---|---|---|
| `BinaryBufferReader` 越界 | abort（_invalid_parameter_noinfo_noreturn） | 记 error 日志 + 返回已读的部分数据 | 开发阶段保证日志可获取，不崩溃游戏 |
| `count` 超过 0xfffe | abort（DebugBreak，仅当 IsDebuggerPresent） | 记 warn 日志，继续处理 | 源码此检查仅 debug 用，release 模式不触发 |
| 字符串 UTF-8 解码失败 | 未定义（源码不检查） | 返回 `BufferError::InvalidUtf8`，记日志 | Rust 严格，C# 应保证传 UTF-8 |
| panic | abort（panic=abort in release） | catch_unwind 捕获 + 返回 null/默认值 | 防止 panic 跨 FFI 边界 |

### 7.2 日志格式

```rust
tracing::error!(?e, "CreateElementsTable failed at offset {}", reader.offset());
tracing::warn!(count, "CreateElementsTable: count exceeds 0xfffe");
```

**注**：`BinaryBufferReader` 当前无 `offset()` 公开方法，Phase 2 需添加（或用 `remaining()` 反推）。

---

## 8. 测试策略

### 8.1 单元测试（element.rs）

12 个 struct 的 `size_of` 精确断言：

```rust
#[test]
fn element_temperature_data_size() {
    assert_eq!(size_of::<ElementTemperatureData>(), 56);
}
// ... 其余 11 个类似
```

### 8.2 单元测试（elements_table.rs）

#### 8.2.1 `create_elements_table` round-trip

构造一个 2-3 元素的 BinaryBufferWriter（按源码格式）→ 喂给 `create_elements_table` → 验证：
- `elements.len() == count`
- `element_indices.get(&elem.id) == Some(idx)`
- `element_names[i] == expected_string`
- `temperature_data[i]` 字段全部正确（包括 mass_area_scale 计算值）
- `light_absorption_data[i].mass_scale` 计算正确（含 ∞ 测试用例）
- `temperature_data[i].low_temp_transition_ore_idx` 在后处理后正确（含 0x2d39bf75 → 0xffff 用例）

#### 8.2.2 `get_element_index`

```rust
#[test]
fn get_element_index_returns_inserted() {
    // 构造 table 含 id=42 → index=7
    assert_eq!(get_element_index(42), 7);
}

#[test]
fn get_element_index_returns_invalid_for_missing() {
    assert_eq!(get_element_index(999), INVALID_ELEMENT_INDEX);
}
```

#### 8.2.3 `destroy_elements_table`

```rust
#[test]
fn destroy_clears_all_vectors() {
    // 先 create_elements_table 填充数据
    destroy_elements_table();
    let table = G_ELEMENTS_TABLE.lock().unwrap();
    assert!(table.elements.is_empty());
    assert!(table.element_names.is_empty());
    // ... 11 个 vector 全部为空
    assert!(table.element_indices.is_empty());
    // 注意：3 个交互 vector 不应被清空（严格对照源码）
}
```

#### 8.2.4 `create_element_interactions`

```rust
#[test]
fn create_element_interactions_pushes_only_matching_signature() {
    // 构造 buffer：count=2，entry0 签名匹配，entry1 签名不匹配
    create_element_interactions(&mut reader).unwrap();
    let table = G_ELEMENTS_TABLE.lock().unwrap();
    assert_eq!(table.gas_obliterations.len(), 1);
    // 验证 entry0 字段
}
```

### 8.3 集成测试（tests/abi_smoke 风格）

新建 `tests/elements_smoke.rs` 或追加到 `abi_smoke.rs`：

```rust
#[test]
fn create_elements_table_with_null_reader_does_not_panic() {
    let result = CreateElementsTable(std::ptr::null_mut());
    assert!(result.is_null());
}

#[test]
fn destroy_elements_table_does_not_panic() {
    DestroyElementsTable();
}

#[test]
fn get_element_index_returns_invalid_for_empty_table() {
    let result = GetElementIndex(0);
    assert_eq!(result, INVALID_ELEMENT_INDEX);
}
// ... CreateElementInteractions / CreateElementInteractionsLocked 类似
```

---

## 9. 文件改动清单

### 9.1 新建文件

| 文件 | 用途 |
|---|---|
| `simDLL_RS/src/elements_table.rs` | ElementsTable struct + 全局单例 + 5 个 C ABI 导出函数 |
| `simDLL_RS/tests/elements_smoke.rs` | 集成测试（5 个导出函数不 panic） |

### 9.2 修改文件

| 文件 | 改动 |
|---|---|
| `simDLL_RS/src/element.rs` | 追加 9 个附属 struct + 3 个交互 struct + 1 个 type alias + 3 个常量 + Element 添加 Default derive |
| `simDLL_RS/src/lib.rs` | 追加 `pub mod elements_table;` |
| `simDLL_RS/src/buffer.rs` | 添加 `pub fn offset(&self) -> u64` 方法（用于错误日志） |
| `simDLL_RS/docs/roadmap.md` | 修正 L61：把"加载 gas.yaml"改为"从 BinaryBuffer 反序列化（C# 侧读 yaml 后序列化）" |
| `.superpowers/sdd/progress.md` | Phase 2 任务进度记录 |

### 9.3 不修改文件

- `Cargo.toml`（parking_lot 已是依赖，无需新增）
- `sim_api.rs`（Phase 2 不接入消息分派，Phase 6 才连接）
- `message_handler.rs`（同上）

---

## 10. 验收标准

### 10.1 编译

- `cargo build --release --target x86_64-pc-windows-msvc` exit 0
- `SimDLL.dll` 字节数与 Phase 1 基线（996864）的差异在合理范围（新增 5 个导出函数 + 12 个 struct 会增加 5-20 KB）

### 10.2 测试

- `cargo test` 全量通过（Phase 1 基线 93 + Phase 2 新增预计 20-30 个测试）
- 0 error，无新 warning（仅遗留的"crate SimDLL should have a snake case name"）

### 10.3 导出符号

- `dumpbin /exports SimDLL.dll` 应包含 5 个新符号：
  - `CreateElementsTable`
  - `DestroyElementsTable`
  - `GetElementIndex`
  - `CreateElementInteractions`
  - `CreateElementInteractionsLocked`
- 或用集成测试调用等价证明

### 10.4 部署无回归

- 用户部署后游戏进主菜单（与 Phase 1 一致）
- 加载存档行为与 Phase 1 一致（仍会崩溃，Phase 3 才实现存档加载）
- 日志文件无新增条目（除非 C# 主动调用本 Phase 函数，Phase 2 之前不会发生）

---

## 11. 风险与未决项

### 11.1 已决项

| 决策 | 选项 | 理由 |
|---|---|---|
| 数据来源 | 从 BinaryBuffer 反序列化 | 严格对照源码 |
| 架构方案 | 方案 B（ElementsTable struct） | 集中管理 15 个全局变量 |
| 错误处理 | 决策 #1：A（记日志 + 部分返回） | 开发阶段保证日志可获取 |
| Locked 函数 mutex | 决策 #2：A（parking_lot::Mutex 独立锁） | Phase 3+ 替换 MsvcMutex 时无需回头改 |
| roadmap.md 修正 | 决策 #3：是 | 避免后续 phase 误解 |

### 11.2 遗留观察项（Phase 6+ 复核）

1. **`GetElementIndex` 并发安全**：原版无锁（C++ 全局变量直接访问），Rust 用 `Mutex` 保护。若 Phase 6 性能基准显示锁竞争，可改用 `RwLock` 或 `AtomicPtr` + 无锁哈希表
2. **`element_indices` 迭代顺序**：原版 `std::_Hash` 按 bucket 顺序，Rust `HashMap` 随机。当前代码不依赖迭代顺序（只做 find），但 Phase 6+ 若新增迭代需注意
3. **`gElementNames` 字符串所有权**：原版 MSVC string SSO 32B，Rust String 24B。若 Phase 6+ 通过 `*mut` 暴露给 C#，需注意 ABI 兼容（当前不暴露）
4. **`GAS_OBLITERATION_SIGNATURE` 符号**：`0x6826f67d` 是 i32 还是 u32？源码用 `*(int*)` 比较，Rust 用 `i32` 比较可能负数。需测试覆盖负数场景
5. **`INVALID_ELEMENT_HASH` 符号**：同上，`0x2d39bf75` 作为 u32 是 759025269，作为 i32 是正数，无歧义
6. **`Element` 添加 Default**：Phase 1 只 derive `Clone, Copy`。Phase 2 需添加 Default。i32/u16/u8/f32 都有 Default，应能自动 derive

### 11.3 不在范围内

- `CreateElementInteractions` 中 `gLiquidObliterations` / `gLiquidConversions` 的填充逻辑（源码缺失，Phase 6+ 若发现需要再补）
- `gFrameSync.mSimMutex` 真实实现（Phase 3+）
- 消息分派（Phase 6）

---

## 12. 参考资源

- 源码：`OxygenNotIncluded_Source/simDLL_edit/03_elements.c`（5 个函数，571 行）
- 类型参考：`OxygenNotIncluded_Source/simDLL_edit/00_types_reference.c`（12 个 struct 定义）
- Phase 1 已实现：
  - `simDLL_RS/src/element.rs`（Element 164B / PhysicsData 12B / ElementState 1B）
  - `simDLL_RS/src/buffer.rs`（BinaryBufferReader 24B / BinaryBufferWriter）
  - `simDLL_RS/src/frame_sync.rs`（FrameSync 含 mSimMutex 占位）
  - `simDLL_RS/src/stl_shim.rs`（MsvcVector/MsvcMutex 等占位）
- yaml 数据（仅供理解字段含义，simDLL 不直接读）：`OxygenNotIncluded_Data/StreamingAssets/elements/{gas,liquid,solid,special}.yaml`
- BASE commit：`d7e42b5`（Phase 1 收官）

---

## 附录 A：Element 字段偏移表（用于字段映射验证）

| 字段 | 类型 | 偏移 | 大小 |
|---|---|---|---|
| id | i32 | 0 | 4 |
| low_temp_transition_idx | u16 | 4 | 2 |
| high_temp_transition_idx | u16 | 6 | 2 |
| elements_table_idx | u16 | 8 | 2 |
| state | u8 | 10 (0xa) | 1 |
| number_of_gradient_colors | u8 | 11 (0xb) | 1 |
| specific_heat_capacity | f32 | 12 (0xc) | 4 |
| thermal_conductivity | f32 | 16 (0x10) | 4 |
| molar_mass | f32 | 20 (0x14) | 4 |
| solid_surface_area_multiplier | f32 | 24 (0x18) | 4 |
| liquid_surface_area_multiplier | f32 | 28 (0x1c) | 4 |
| gas_surface_area_multiplier | f32 | 32 (0x20) | 4 |
| flow | f32 | 36 (0x24) | 4 |
| viscosity | f32 | 40 (0x28) | 4 |
| min_horizontal_flow | f32 | 44 (0x2c) | 4 |
| min_vertical_flow | f32 | 48 (0x30) | 4 |
| max_mass | f32 | 52 (0x34) | 4 |
| low_temp | f32 | 56 (0x38) | 4 |
| high_temp | f32 | 60 (0x3c) | 4 |
| strength | f32 | 64 (0x40) | 4 |
| low_temp_transition_ore_id | i32 | 68 (0x44) | 4 |
| low_temp_transition_ore_mass_conversion | f32 | 72 (0x48) | 4 |
| high_temp_transition_ore_id | i32 | 76 (0x4c) | 4 |
| high_temp_transition_ore_mass_conversion | f32 | 80 (0x50) | 4 |
| sublimate_index | u16 | 84 (0x54) | 2 |
| convert_index | u16 | 86 (0x56) | 2 |
| material_properties | u32 | 88 (0x58) | 4 |
| colour | u32 | 92 (0x5c) | 4 |
| gradient_colours | [u32;6] | 96 (0x60) | 24 |
| sublimate_fx | i32 | 120 (0x78) | 4 |
| sublimate_rate | f32 | 124 (0x7c) | 4 |
| sublimate_efficiency | f32 | 128 (0x80) | 4 |
| sublimate_probability | f32 | 132 (0x84) | 4 |
| off_gas_percentage | f32 | 136 (0x88) | 4 |
| light_absorption_factor | f32 | 140 (0x8c) | 4 |
| radiation_absorption_factor | f32 | 144 (0x90) | 4 |
| radiation_per_1000_mass | f32 | 148 (0x94) | 4 |
| default_values.temperature | f32 | 152 (0x98) | 4 |
| default_values.mass | f32 | 156 (0x9c) | 4 |
| default_values.pressure | f32 | 160 (0xa0) | 4 |
| **总大小** | | | **164 (0xa4)** |
