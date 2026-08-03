# ElementChunk（物质碎片）温度组件实现计划

> **面向 AI 代理的工作者：** 必须子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现本计划。步骤使用复选框（`- [ ]`）语法跟踪进度。

**目标：** 实现 ElementChunk（物质碎片）温度组件——注册表（版本句柄）、Add/Modify/Move/ModifyEnergy/ModifyAdjuster/Remove 消息、Update（adjuster / 世界换热：本格 + 下方固体格）、相变事件、GDU 输出，使碎片与世界正常换热。

**架构：** 规格 `docs/superpowers/specs/2026-08-03-element-chunk-design.md`（提交 `be8eb8f`）。新建 `c_simulation/element_chunk.rs`（全局单例管理器，模式同 `building_temperature.rs`）；`ExchangeHeatEnergyWithWorld` 与 Update 核心也放该文件；frame_processor 消费消息；copy_sim_data_to_game 补输出交换；update_data 组件阶段挂载。

**技术栈：** Rust / cargo / 严格对照 `OxygenNotIncluded_Source/SimDLL_Source.c`（L139789-140202、L139972-140075、L116653-116682）。全量测试用 `cargo test -- --test-threads=1`。

---

## 文件清单

| 文件 | 职责 | 动作 |
|------|------|------|
| `simDLL_RS/src/c_simulation/element_chunk.rs` | ElementChunkData、管理器（句柄/注册/消息/Update/输出） | 新建 |
| `simDLL_RS/src/c_simulation/mod.rs` | 注册模块 | 修改 |
| `simDLL_RS/src/c_simulation/frame_processor.rs` | process_element_chunk_messages（adds→modifies→energy→adjuster→move→removes） | 修改 |
| `simDLL_RS/src/c_simulation/sim_data_ops.rs` | copy_sim_data_to_game 交换 element_chunk_info | 修改 |
| `simDLL_RS/src/c2_physics/mod.rs` | update_data 组件阶段挂载 update_element_chunks | 修改 |

## 全局约束

- 严格对照原版行号（规格 2.2/2.3/3.1-3.3），**禁止照抄** `my_Project\simDLL_RS_Kit\simDLL_RS`（神秘版本）。
- TDD：每个新函数先写失败测试 → 验证失败 → 最少实现 → 验证通过 → commit。
- 断言（原版 DebugBreak/崩溃）按项目惯例转 `tracing::error/warn` + 继续。
- 单测须持 `LIB_TESTS_LOCK`；元素表用独立最小表（参照 `c2_physics/temperature.rs` 的 `init_pair_table` 风格）。
- 句柄模式与 `building_temperature.rs` 一致：`handle = (version<<24)|index`、free list、swap-remove。

---

### 任务 1：ElementChunkManager 注册表 + Add/Modify/Move/ModifyEnergy/ModifyAdjuster/Remove

**文件：** `simDLL_RS/src/c_simulation/element_chunk.rs`（新建）、`c_simulation/mod.rs`

- [ ] **步骤 1：写失败测试（Register 派生 + 消息语义）**

```rust
#[test]
fn register_derives_fields_from_message() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_elem_table(); // elem5：SHC=2.0、TC=4.0、low=200、high=5000
    let mut mgr = ElementChunkManager::new();
    // 6×6 sim：game cell 4（row1,col1）→ sim (2,2) = 2×6+2 = 14
    let msg = AddElementChunkMsg {
        game_cell: 4,
        callback_idx: 7,
        mass: 100.0,
        temperature: 300.0,
        surface_area: 2.0,
        thickness: 0.5,
        ground_transfer_scale: 0.5,
        element_idx: 5,
        pad: [0; 2],
    };
    let h = mgr.add(6, &msg);
    assert_eq!(h & 0xffffff, 0);
    let d = mgr.record(h).unwrap();
    assert_eq!(d.cell, 14);
    assert_eq!(d.temperature, 300.0);
    assert!((d.heat_capacity - 200.0).abs() < 1e-3); // SHC×mass = 2×100
    assert_eq!(d.thermal_conductivity, 4.0);
    assert!((d.max_energy_transfer_scale_factor - 4.0).abs() < 1e-3); // 2/0.5
    assert_eq!(d.ground_transfer_scale, 0.5);
    assert_eq!(d.high_temp, 5000.0);
    assert_eq!(d.low_temp, 200.0);
    assert_eq!(d.adjuster.heat_capacity, 0.0);
}

#[test]
fn modify_energy_and_adjuster_and_move() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_elem_table();
    let mut mgr = ElementChunkManager::new();
    let h = mgr.add(6, &add_msg(4, 300.0));
    // ModifyEnergy：ΔT = deltaKJ/heatCapacity
    assert!(mgr.modify_energy(h, 200.0)); // 200/200 = +1℃
    assert!((mgr.record(h).unwrap().temperature - 301.0).abs() < 1e-3);
    // ModifyAdjuster
    assert!(mgr.modify_adjuster(h, 350.0, 50.0, 2.0));
    assert_eq!(mgr.record(h).unwrap().adjuster.temperature, 350.0);
    assert_eq!(mgr.record(h).unwrap().adjuster.heat_capacity, 50.0);
    assert_eq!(mgr.record(h).unwrap().adjuster.thermal_conductivity, 2.0);
    // Move：game cell 9（row2,col1）→ sim (3,2) = 3×6+2 = 20
    assert!(mgr.move_chunk(h, 9, 6));
    assert_eq!(mgr.record(h).unwrap().cell, 20);
    // Modify
    assert!(mgr.modify(h, 400.0, 500.0));
    assert_eq!(mgr.record(h).unwrap().temperature, 400.0);
    assert_eq!(mgr.record(h).unwrap().heat_capacity, 500.0);
    // Remove
    assert!(mgr.remove(h));
    assert!(mgr.record(h).is_none());
}
```

- [ ] **步骤 2：运行确认失败** — `cargo test -p simdll_rs --lib element_chunk::`；预期：模块/函数未定义。

- [ ] **步骤 3：最少实现**

```rust
//! ElementChunk（物质碎片）温度组件（原版 L139789-140202）。
use crate::a_framework::game_data::{ElementChunkInfo, Handle, MeltedInfo};
use crate::a_framework::sim_data::SimData;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// ExternalAdjuster（原版 ElementChunkData.adjuster，16B）。
#[derive(Clone, Copy, Debug)]
pub struct ElementChunkAdjuster {
    pub handle: i32,
    pub temperature: f32,
    pub heat_capacity: f32,
    pub thermal_conductivity: f32,
}

impl Default for ElementChunkAdjuster {
    fn default() -> Self {
        Self { handle: -1, temperature: 0.0, heat_capacity: 0.0, thermal_conductivity: 0.0 }
    }
}

/// ElementChunkData（原版 44B = 0x2c，L10107）。
#[derive(Clone, Copy, Debug)]
pub struct ElementChunkData {
    pub cell: i32,
    pub temperature: f32,
    pub heat_capacity: f32,
    pub thermal_conductivity: f32,
    pub max_energy_transfer_scale_factor: f32,
    pub ground_transfer_scale: f32,
    pub high_temp: f32,
    pub low_temp: f32,
    pub adjuster: ElementChunkAdjuster,
}

impl Default for ElementChunkData {
    fn default() -> Self {
        Self {
            cell: 0,
            temperature: 0.0,
            heat_capacity: 0.0,
            thermal_conductivity: 0.0,
            max_energy_transfer_scale_factor: 1.0,
            ground_transfer_scale: 0.0,
            high_temp: f32::MAX,
            low_temp: 0.0,
            adjuster: ElementChunkAdjuster::default(),
        }
    }
}

/// AddElementChunkMessage（32B，原版 L7694）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AddElementChunkMsg {
    pub game_cell: i32,
    pub callback_idx: i32,
    pub mass: f32,
    pub temperature: f32,
    pub surface_area: f32,
    pub thickness: f32,
    pub ground_transfer_scale: f32,
    pub element_idx: u16,
    pub pad: [u8; 2],
}

/// game→sim 坐标（原版 Register L140019：`(gameCell/(w-2)+1)*w + gameCell%(w-2)+1`）。
fn game_to_sim_cell(game_cell: i32, width: i32) -> i32 {
    let gw = width - 2;
    (game_cell / gw + 1) * width + game_cell % gw + 1
}

pub struct ElementChunkManager {
    records: Vec<ElementChunkData>,
    items: Vec<i32>,
    versions: Vec<u8>,
    free_handles: Vec<i32>,
    /// 每帧输出 {temperature, delta_kj}（按 handle index 索引后压缩；原版 Update `_padding_` 数组）
    output: Vec<ElementChunkInfo>,
}

impl ElementChunkManager {
    pub fn new() -> Self {
        Self { records: Vec::new(), items: Vec::new(), versions: Vec::new(), free_handles: Vec::new(), output: Vec::new() }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.items.clear();
        self.versions.clear();
        self.free_handles.clear();
        self.output.clear();
    }

    pub fn record(&self, handle: i32) -> Option<&ElementChunkData> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let ri = self.items[index];
        if ri < 0 {
            return None;
        }
        self.records.get(ri as usize)
    }

    fn get_mut(&mut self, handle: i32) -> Option<&mut ElementChunkData> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let ri = self.items[index];
        if ri < 0 {
            return None;
        }
        self.records.get_mut(ri as usize)
    }

    /// Register（原版 L139972-140075）。
    pub fn add(&mut self, width: i32, msg: &AddElementChunkMsg) -> i32 {
        let Some(etd) = crate::b_elements::elements_table::get_element_temperature_data(msg.element_idx) else {
            return -1;
        };
        let rec = ElementChunkData {
            cell: game_to_sim_cell(msg.game_cell, width),
            temperature: msg.temperature,
            heat_capacity: etd.specific_heat_capacity * msg.mass,
            thermal_conductivity: etd.thermal_conductivity,
            max_energy_transfer_scale_factor: if msg.thickness != 0.0 { msg.surface_area / msg.thickness } else { 0.0 },
            ground_transfer_scale: msg.ground_transfer_scale,
            high_temp: etd.high_temp,
            low_temp: etd.low_temp,
            adjuster: ElementChunkAdjuster::default(),
        };
        let ri = self.records.len();
        let handle = if let Some(h) = self.free_handles.pop() {
            let index = (h & 0xffffff) as usize;
            self.items[index] = ri as i32;
            h
        } else {
            let index = self.items.len();
            self.items.push(ri as i32);
            self.versions.push(0);
            index as i32
        };
        self.records.push(rec);
        handle
    }

    /// Modify（原版 L139894）。
    pub fn modify(&mut self, handle: i32, temperature: f32, heat_capacity: f32) -> bool {
        let Some(d) = self.get_mut(handle) else { return false; };
        d.temperature = temperature;
        d.heat_capacity = heat_capacity;
        true
    }

    /// ModifyEnergy（原版 L139947）：temp += deltaKJ/heatCapacity，钳制 ≥0。
    pub fn modify_energy(&mut self, handle: i32, delta_kj: f32) -> bool {
        let Some(d) = self.get_mut(handle) else { return false; };
        if d.heat_capacity > 0.0 {
            let mut t = d.temperature + delta_kj / d.heat_capacity;
            if t <= 0.0 {
                t = 0.0;
            }
            d.temperature = t;
        }
        true
    }

    /// ModifyAdjuster（原版 L139926）。
    pub fn modify_adjuster(&mut self, handle: i32, temperature: f32, heat_capacity: f32, thermal_conductivity: f32) -> bool {
        let Some(d) = self.get_mut(handle) else { return false; };
        d.adjuster.temperature = temperature;
        d.adjuster.heat_capacity = heat_capacity;
        d.adjuster.thermal_conductivity = thermal_conductivity;
        true
    }

    /// Move（原版 MoveElementChunkMessage {handle, gameCell}）。
    pub fn move_chunk(&mut self, handle: i32, game_cell: i32, width: i32) -> bool {
        let Some(d) = self.get_mut(handle) else { return false; };
        d.cell = game_to_sim_cell(game_cell, width);
        true
    }

    /// Remove（swap-remove + 版本递增）。
    pub fn remove(&mut self, handle: i32) -> bool {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return false;
        }
        let ri = self.items[index];
        if ri < 0 {
            return false;
        }
        let new_version = version.wrapping_add(1);
        self.versions[index] = new_version;
        self.free_handles.push(((new_version as i32) << 24) | (index as i32 & 0xffffff));
        self.items[index] = -1;
        let ri = ri as usize;
        let last = self.records.len() - 1;
        if ri != last {
            self.records.swap(ri, last);
            // 更新被移动记录的 items 映射（需按 handle index 反查——简化：遍历 items）
            let moved = &self.records[ri];
            for it in self.items.iter_mut() {
                if *it == last as i32 {
                    *it = ri as i32;
                    let _ = moved.cell; // keep borrow clean
                    break;
                }
            }
        }
        self.records.pop();
        true
    }
}

static ELEMENT_CHUNK_MANAGER: Lazy<Mutex<ElementChunkManager>> =
    Lazy::new(|| Mutex::new(ElementChunkManager::new()));

pub fn clear_element_chunks() {
    ELEMENT_CHUNK_MANAGER.lock().clear();
}

// 顶层包装（frame_processor / update_data 调用）
pub fn add_element_chunk(width: i32, msg: &AddElementChunkMsg) -> i32 {
    ELEMENT_CHUNK_MANAGER.lock().add(width, msg)
}
pub fn modify_element_chunk(handle: i32, temperature: f32, heat_capacity: f32) -> bool {
    ELEMENT_CHUNK_MANAGER.lock().modify(handle, temperature, heat_capacity)
}
pub fn modify_element_chunk_energy(handle: i32, delta_kj: f32) -> bool {
    ELEMENT_CHUNK_MANAGER.lock().modify_energy(handle, delta_kj)
}
pub fn modify_element_chunk_adjuster(handle: i32, t: f32, hc: f32, tc: f32) -> bool {
    ELEMENT_CHUNK_MANAGER.lock().modify_adjuster(handle, t, hc, tc)
}
pub fn move_element_chunk(handle: i32, game_cell: i32, width: i32) -> bool {
    ELEMENT_CHUNK_MANAGER.lock().move_chunk(handle, game_cell, width)
}
pub fn remove_element_chunk(handle: i32) -> bool {
    ELEMENT_CHUNK_MANAGER.lock().remove(handle)
}
```

`c_simulation/mod.rs` 注册 `pub mod element_chunk;`。

- [ ] **步骤 4：运行确认通过（含全量）** — `cargo test -p simdll_rs -- --test-threads=1`。

- [ ] **步骤 5：Commit** — `git commit -m "feat(element_chunk): registry + Add/Modify/Move/ModifyEnergy/ModifyAdjuster/Remove"`

---

### 任务 2：`ExchangeHeatEnergyWithWorld`（原版 L139789-139890）

**文件：** `simDLL_RS/src/c_simulation/element_chunk.rs`

- [ ] **步骤 1：写失败测试（本格换热 / 门控）**

```rust
#[test]
fn exchange_heat_energy_with_world_exchanges_cell_and_chunk() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_elem_table(); // elem5 液态水 SHC=2 TC=4 ins 由测试设
    let mut sd = make_sim(); // 6×6，headless=false
    // 格 14：水 300°C mass 100 ins 255；碎片：400°C HC 200 TC 4，scale=1，factor=1
    unsafe {
        let u = &mut *sd.updated_cells.ptr;
        u.element_idx.set(14, 5);
        u.mass.set(14, 100.0);
        u.temperature.set(14, 300.0);
        u.insulation.set(14, 255);
    }
    let mut chunk = ElementChunkData {
        cell: 14,
        temperature: 400.0,
        heat_capacity: 200.0,
        thermal_conductivity: 4.0,
        max_energy_transfer_scale_factor: 1.0,
        ..Default::default()
    };
    let heat = exchange_heat_energy_with_world(&mut sd, &mut chunk, 14, 1.0, 0.2);
    assert!(heat > 0.0, "碎片（热）应失去热量给格子");
    assert!(chunk.temperature < 400.0);
    unsafe {
        assert!((*sd.updated_cells.ptr).temperature.get(14) > 300.0);
    }
}

#[test]
fn exchange_heat_energy_with_world_gates() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_elem_table();
    let mut sd = make_sim();
    let mut chunk = ElementChunkData { cell: 14, temperature: 400.0, heat_capacity: 200.0, thermal_conductivity: 4.0, max_energy_transfer_scale_factor: 1.0, ..Default::default() };
    // mass=0 → 0
    assert_eq!(exchange_heat_energy_with_world(&mut sd, &mut chunk, 14, 1.0, 0.2), 0.0);
    // 元素 TemperatureInsulated(0x10) → 0
    unsafe {
        let u = &mut *sd.updated_cells.ptr;
        u.element_idx.set(14, 5);
        u.mass.set(14, 100.0);
        u.temperature.set(14, 300.0);
    }
    {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.temperature_data[5].state = 2 | 0x10;
    }
    assert_eq!(exchange_heat_energy_with_world(&mut sd, &mut chunk, 14, 1.0, 0.2), 0.0);
}
```

- [ ] **步骤 2：运行确认失败** — `exchange_heat_energy_with_world` 未定义。

- [ ] **步骤 3：最少实现**

```rust
/// ExchangeHeatEnergyWithWorld（原版 L139789-139890）。返回热流（正 = 碎片失热给格子）。
pub(crate) fn exchange_heat_energy_with_world(
    sim: &mut SimData,
    chunk: &mut ElementChunkData,
    cell: usize,
    scale: f32,
    dt: f32,
) -> f32 {
    if sim.updated_cells.ptr.is_null() {
        return 0.0;
    }
    let (cell_mass, cell_temp, cell_ins, cell_hc, cell_etd) = {
        let u = unsafe { &*sim.updated_cells.ptr };
        if cell >= u.mass.len() || u.mass.get(cell) <= 0.0 {
            return 0.0;
        }
        let elem = u.element_idx.get(cell);
        let Some(etd) = crate::b_elements::elements_table::get_element_temperature_data(elem) else {
            return 0.0;
        };
        if etd.state & 0x10 != 0 {
            return 0.0; // TemperatureInsulated
        }
        (
            u.mass.get(cell),
            u.temperature.get(cell),
            u.insulation.get(cell) as f32,
            u.mass.get(cell) * etd.specific_heat_capacity,
            etd,
        )
    };
    if !(0.0..=10000.0).contains(&chunk.temperature) {
        tracing::error!(t = chunk.temperature, "ElementChunk bad item temperature");
        return 0.0;
    }
    if cell_hc <= 0.0 || chunk.heat_capacity <= 0.0 {
        return 0.0;
    }
    let ins_val = cell_ins * cell_ins * 1.53787e-05;
    let k_cell = ins_val * cell_etd.thermal_conductivity;
    let k = chunk.thermal_conductivity.min(k_cell)
        * scale
        * chunk.max_energy_transfer_scale_factor
        * 0.001;
    // CalculateTemperatureExchange wrapper（冷侧在前）→ (new_热侧, new_冷侧, heat)
    let (new_chunk, new_cell) = if chunk.temperature <= cell_temp {
        let (nc, nn) = crate::c2_physics::temperature::calculate_temperature_exchange_precise(
            1.0, k, dt, chunk.temperature, chunk.heat_capacity, cell_temp, cell_hc,
        );
        (nc, nn)
    } else {
        let (nn, nc) = crate::c2_physics::temperature::calculate_temperature_exchange_precise(
            1.0, k, dt, cell_temp, cell_hc, chunk.temperature, chunk.heat_capacity,
        );
        (nc, nn)
    };
    let heat = (chunk.temperature - new_chunk) * chunk.heat_capacity;
    chunk.temperature = new_chunk;
    unsafe {
        (*sim.updated_cells.ptr).temperature.set(cell, new_cell);
    }
    heat
}
```

> 注：`calculate_temperature_exchange_precise` 返回 (new_第一参, new_第二参)；此处直接映射 (new_chunk, new_cell)。

- [ ] **步骤 4：运行确认通过（含全量）**

- [ ] **步骤 5：Commit** — `git commit -m "feat(element_chunk): ExchangeHeatEnergyWithWorld"`

---

### 任务 3：`ElementChunk::Update`（原版 L140092-140202）

**文件：** `simDLL_RS/src/c_simulation/element_chunk.rs`

- [ ] **步骤 1：写失败测试（adjuster 分支 / 下方固体格 / 相变事件 / 输出）**

```rust
#[test]
fn update_uses_adjuster_when_present() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_elem_table();
    let mut sd = make_sim();
    let mut mgr = ElementChunkManager::new();
    let h = mgr.add(6, &add_msg(4, 300.0));
    mgr.modify_adjuster(h, 500.0, 100.0, 2.0);
    // 格 14 无质量 → 若走世界分支 heat=0；adjuster 分支不依赖格子
    update_element_chunks_inner(&mut mgr, &mut sd, 0.2, 6);
    let d = mgr.record(h).unwrap();
    assert!(d.temperature > 300.0, "adjuster 应加热碎片，got {}", d.temperature);
    assert!(d.temperature < 500.0);
}

#[test]
fn update_exchanges_with_solid_cell_below() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_elem_table();
    let mut sd = make_sim();
    let mut mgr = ElementChunkManager::new();
    // 碎片在格 20（row3,col2），下方格 14（row2,col2）为固体
    let mut msg = add_msg(0, 400.0); // game cell 8 → sim (3,2)=20
    msg.game_cell = 8;
    let h = mgr.add(6, &msg);
    unsafe {
        let u = &mut *sd.updated_cells.ptr;
        u.element_idx.set(14, 2); // 固体（init_elem_table 的 elem2）
        u.mass.set(14, 1000.0);
        u.temperature.set(14, 300.0);
        u.insulation.set(14, 255);
    }
    update_element_chunks_inner(&mut mgr, &mut sd, 0.2, 6);
    let d = mgr.record(h).unwrap();
    assert!(d.temperature < 400.0, "碎片应降温给下方固体，got {}", d.temperature);
    let events = unsafe { &*sd.sim_events.ptr };
    assert_eq!(events.element_chunk_melted_info.len(), 0, "无相变");
    // 输出：temperature + delta_kj
    assert!(!mgr.output_snapshot().is_empty());
}

#[test]
fn update_emits_melted_event_on_phase_cross() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_elem_table(); // elem5 highTemp=5000 → 触发需要 >5003
    let mut sd = make_sim();
    let mut mgr = ElementChunkManager::new();
    let mut msg = add_msg(4, 6000.0);
    let h = mgr.add(6, &msg);
    update_element_chunks_inner(&mut mgr, &mut sd, 0.2, 6);
    let events = unsafe { &*sd.sim_events.ptr };
    assert_eq!(events.element_chunk_melted_info.len(), 1);
    assert_eq!(events.element_chunk_melted_info.get(0).handle.value, h);
}
```

- [ ] **步骤 2：运行确认失败** — `update_element_chunks_inner` / `output_snapshot` 未定义。

- [ ] **步骤 3：最少实现**

```rust
/// ElementChunk::Update（原版 L140092-140202）。
/// 逐碎片：adjuster 换热（rate 1.0）或世界换热（本格 1.0 + 下方固体 groundTransferScale）；
/// 输出 {temperature, delta_kj}；越相变 → elementChunkMeltedInfo。
pub(crate) fn update_element_chunks_inner(
    mgr: &mut ElementChunkManager,
    sim: &mut SimData,
    dt: f32,
    width: i32,
) {
    mgr.output.clear();
    if sim.updated_cells.ptr.is_null() || sim.sim_events.ptr.is_null() {
        return;
    }
    let total = mgr.records.len();
    let mut i = 0;
    while i < total {
        let handle = mgr.data_handle(i);
        let chunk = &mut mgr.records[i];
        if chunk.heat_capacity <= 0.0 {
            i += 1;
            continue;
        }
        let mut delta = 0.0f32;
        if chunk.adjuster.heat_capacity > 0.0 {
            // adjuster 分支：rate 1.0，不写格子
            let (nc, _na) = crate::c2_physics::temperature::calculate_temperature_exchange_precise(
                1.0, chunk.adjuster.thermal_conductivity, dt,
                chunk.adjuster.temperature, chunk.adjuster.heat_capacity,
                chunk.temperature, chunk.heat_capacity,
            );
            chunk.temperature = nc;
        } else {
            let cell = chunk.cell as usize;
            let h1 = exchange_heat_energy_with_world(sim, chunk, cell, 1.0, dt);
            delta += h1;
            let below = cell.wrapping_sub(width as usize);
            let below_elem = unsafe { (*sim.updated_cells.ptr).element_idx.get(below) };
            if let Some(etd) = crate::b_elements::elements_table::get_element_temperature_data(below_elem)
            {
                if etd.state & 3 == 3 {
                    let h2 = exchange_heat_energy_with_world(sim, chunk, below, chunk.ground_transfer_scale, dt);
                    delta += h2;
                }
            }
        }
        let t = chunk.temperature;
        mgr.output.push(ElementChunkInfo { temperature: t, delta_kj: delta });
        if chunk.high_temp + 3.0 <= t || t < chunk.low_temp - 3.0 {
            let events = unsafe { &mut *sim.sim_events.ptr };
            events.element_chunk_melted_info.push(MeltedInfo { handle: Handle { value: handle } });
        }
        i += 1;
    }
}

/// 顶层：update_data 组件阶段调用。
pub fn update_element_chunks(sim: &mut SimData, dt: f32, width: i32) {
    let mut mgr = ELEMENT_CHUNK_MANAGER.lock();
    update_element_chunks_inner(&mut mgr, sim, dt, width);
    mgr.take_output_into(game_data::element_chunk_info()); // 见任务 5 接线
}

pub fn output_snapshot(&self) -> &[ElementChunkInfo] { &self.output }
```

> `data_handle(i)`：从 items 反查第 i 条记录对应的 handle（实现时提供；或用 items 正向遍历）。
> 输出接线在任务 5 完成（copy_sim_data_to_game swap）；本任务先保证 output 填充正确。

- [ ] **步骤 4：运行确认通过（含全量）**

- [ ] **步骤 5：Commit** — `git commit -m "feat(element_chunk): Update (adjuster + world + below-solid + phase event)"`

---

### 任务 4：frame_processor 消息消费

**文件：** `simDLL_RS/src/c_simulation/frame_processor.rs`

- [ ] **步骤 1：写失败测试（Add 32B 解析与注册）**

```rust
#[test]
fn process_element_chunk_add_message_registers_chunk() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_elem_table();
    // 构造 32B Add 消息字节（gameCell=4, callbackIdx=7, mass=100, temp=300, area=2, thickness=0.5, ground=0.5, elem=5, pad）
    let mut bytes = Vec::new();
    for v in [4i32, 7, 100, 300] { bytes.extend_from_slice(&v.to_le_bytes()); }
    for v in [2.0f32, 0.5, 0.5] { bytes.extend_from_slice(&v.to_le_bytes()); }
    bytes.extend_from_slice(&5u16.to_le_bytes());
    bytes.extend_from_slice(&[0u8, 0]);
    let mut frame = SimFrameInfo::default();
    frame.element_chunk_messages.adds.extend_from_slice(&bytes);
    let mut sd = SimData::new_for_allocate(6, 6, 1, false, true);
    process_element_chunk_messages(&mut frame, &mut sd);
    // 通过全局管理器验证注册（暴露测试钩子或经顶层函数）
    assert!(crate::c_simulation::element_chunk::element_chunk_count() >= 1);
}
```

- [ ] **步骤 2：运行确认失败** — `process_element_chunk_messages` 未定义。

- [ ] **步骤 3：最少实现**（frame_processor 新增，按原版 L116653-116682 顺序）

```rust
/// 原版 ProcessFrame ElementChunk 段（L116653-116682）：adds → modifies → modifyEnergy →
/// modifyAdjuster → move → removes。
fn process_element_chunk_messages(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    use crate::c_simulation::element_chunk::*;
    let width = sim_data.width;
    // adds（32B）
    {
        let adds = frame.element_chunk_messages.adds.as_slice();
        let n = adds.len() / 32;
        for i in 0..n {
            let off = i * 32;
            let msg = AddElementChunkMsg {
                game_cell: i32::from_le_bytes(adds[off..off+4].try_into().unwrap()),
                callback_idx: i32::from_le_bytes(adds[off+4..off+8].try_into().unwrap()),
                mass: f32::from_le_bytes(adds[off+8..off+12].try_into().unwrap()),
                temperature: f32::from_le_bytes(adds[off+12..off+16].try_into().unwrap()),
                surface_area: f32::from_le_bytes(adds[off+16..off+20].try_into().unwrap()),
                thickness: f32::from_le_bytes(adds[off+20..off+24].try_into().unwrap()),
                ground_transfer_scale: f32::from_le_bytes(adds[off+24..off+28].try_into().unwrap()),
                element_idx: u16::from_le_bytes(adds[off+28..off+30].try_into().unwrap()),
                pad: [adds[off+30], adds[off+31]],
            };
            let h = add_element_chunk(width, &msg);
            if msg.callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events.component_state_changed_messages.push(
                    crate::a_framework::game_data::ComponentStateChangedMessage { callback_idx: msg.callback_idx, sim_handle: h },
                );
            }
        }
        frame.element_chunk_messages.adds.clear_keep_capacity();
    }
    // modifies（12B）{handle, temperature, heatCapacity}
    {
        let mods = frame.element_chunk_messages.modifies.as_slice();
        let n = mods.len() / 12;
        for i in 0..n {
            let off = i * 12;
            let handle = i32::from_le_bytes(mods[off..off+4].try_into().unwrap());
            let t = f32::from_le_bytes(mods[off+4..off+8].try_into().unwrap());
            let hc = f32::from_le_bytes(mods[off+8..off+12].try_into().unwrap());
            modify_element_chunk(handle, t, hc);
        }
        frame.element_chunk_messages.modifies.clear_keep_capacity();
    }
    // modifyEnergy（8B）{handle, deltaKJ}
    {
        let bytes = frame.modify_element_chunk_energy_messages.as_slice();
        let n = bytes.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let handle = i32::from_le_bytes(bytes[off..off+4].try_into().unwrap());
            let dk = f32::from_le_bytes(bytes[off+4..off+8].try_into().unwrap());
            modify_element_chunk_energy(handle, dk);
        }
        frame.modify_element_chunk_energy_messages.clear_keep_capacity();
    }
    // modifyAdjuster（16B）{handle, temperature, heatCapacity, thermalConductivity}
    {
        let bytes = frame.modify_element_chunk_adjuster_messages.as_slice();
        let n = bytes.len() / 16;
        for i in 0..n {
            let off = i * 16;
            let handle = i32::from_le_bytes(bytes[off..off+4].try_into().unwrap());
            let t = f32::from_le_bytes(bytes[off+4..off+8].try_into().unwrap());
            let hc = f32::from_le_bytes(bytes[off+8..off+12].try_into().unwrap());
            let tc = f32::from_le_bytes(bytes[off+12..off+16].try_into().unwrap());
            modify_element_chunk_adjuster(handle, t, hc, tc);
        }
        frame.modify_element_chunk_adjuster_messages.clear_keep_capacity();
    }
    // move（8B）{handle, gameCell}
    {
        let bytes = frame.move_element_chunk_messages.as_slice();
        let n = bytes.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let handle = i32::from_le_bytes(bytes[off..off+4].try_into().unwrap());
            let gc = i32::from_le_bytes(bytes[off+4..off+8].try_into().unwrap());
            move_element_chunk(handle, gc, width);
        }
        frame.move_element_chunk_messages.clear_keep_capacity();
    }
    // removes（8B）{handle, callbackIdx}
    {
        let removes = frame.element_chunk_messages.removes.as_slice();
        let n = removes.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let handle = i32::from_le_bytes(removes[off..off+4].try_into().unwrap());
            let cb = i32::from_le_bytes(removes[off+4..off+8].try_into().unwrap());
            remove_element_chunk(handle);
            if cb != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events.component_state_changed_messages.push(
                    crate::a_framework::game_data::ComponentStateChangedMessage { callback_idx: cb, sim_handle: -1 },
                );
            }
        }
        frame.element_chunk_messages.removes.clear_keep_capacity();
    }
}
```

并在 `process_frame` 的组件消息段调用 `process_element_chunk_messages(frame, sim_data)`。

- [ ] **步骤 4：运行确认通过（含全量）**

- [ ] **步骤 5：Commit** — `git commit -m "feat(element_chunk): consume component messages in ProcessFrame"`

---

### 任务 5：输出交换 + update_data 挂载

**文件：** `simDLL_RS/src/c_simulation/sim_data_ops.rs`、`simDLL_RS/src/c2_physics/mod.rs`

- [ ] **步骤 1：写失败测试（element_chunk_info 交换）**

```rust
#[test]
fn copy_sim_data_to_game_swaps_element_chunk_info() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    // 在 manager 输出放 1 条，调用 copy_sim_data_to_game，验证 game_data.element_chunk_info 收到
    crate::c_simulation::element_chunk::push_test_output(ElementChunkInfo { temperature: 300.0, delta_kj: 1.0 });
    // ...（参照既有 copy_sim_data_to_game 测试构造 sim/game 数据）
    assert_eq!(gd.element_chunk_info.len(), 1);
}
```

- [ ] **步骤 2：运行确认失败** — swap 未实现。

- [ ] **步骤 3：最少实现**

```rust
// sim_data_ops.rs copy_sim_data_to_game：与 20 事件交换并列
std::mem::swap(
    &mut game_data.element_chunk_info,
    &mut crate::c_simulation::element_chunk::element_chunk_output_mut(),
);
crate::c_simulation::element_chunk::element_chunk_output_mut().clear_keep_capacity();

// c2_physics/mod.rs update_data 组件阶段（与 building heat exchange 并列）
crate::c_simulation::element_chunk::update_element_chunks(sim_data, 0.2, sim_data.width);
```

> 管理器输出字段改为 `pub(crate)` 或提供 `element_chunk_output_mut()` / `take_output_into()` 访问器；
> `update_element_chunks` 顶层函数改为"锁管理器 + 内部 Update + 输出留待 swap"（swap 由 copy_sim_data_to_game 消费）。

- [ ] **步骤 4：运行确认通过（含全量 + Release）**

- [ ] **步骤 5：Commit** — `git commit -m "feat(element_chunk): wire output swap + update_data integration"`

---

### 任务 6：集成回归与文档

- [ ] **步骤 1：全量测试 + Release** — `cargo test -p simdll_rs -- --test-threads=1` + `cargo build --release`。

- [ ] **步骤 2：更新代办.txt**：ElementChunk 标为已实现；留"透气砖/网格砖特殊机制、碎片 disease/辐射"为后续。

- [ ] **步骤 3：Commit** — `git commit -m "docs(todo): ElementChunk temperature component implemented"`

---

## 自检

1. **规格覆盖度：** 规格 2.2/2.3→任务 1/4；3.1→任务 2；3.2→任务 3；3.3→任务 5；测试/验收→任务 6。无遗漏。
2. **占位符扫描：** 无 TODO/待定；任务 3 的 `data_handle` 反查与任务 5 的访问器为**实现细节**（在任务内给出具体做法）。
3. **类型一致性：** `ElementChunkData`/`AddElementChunkMsg`/`ElementChunkInfo` 字段在任务 1/2/3 定义并复用；
   `exchange_heat_energy_with_world`、`update_element_chunks_inner`、`process_element_chunk_messages` 命名全文一致。
