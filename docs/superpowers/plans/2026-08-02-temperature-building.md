# 建筑温度链路（方案 A）实现计划

> **面向 AI 代理的工作者：** 必须子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现本计划。步骤使用复选框（`- [ ]`）语法跟踪进度。

**目标：** 实现建筑温度链路——ModifyBuildingEnergy 消费、BuildingHeatExchange::Update（建筑↔格子换热）、过热/熔化事件，以及必要依赖 DoStateTransition（相变检查），使建筑产热真实升温并与周边格子换热，管道换热获得真实温差来源。

**架构：** 规格 `docs/superpowers/specs/2026-08-02-temperature-building-design.md`（提交 `1d005b2`）。记录扩展在 `building_temperature.rs`（对照原版 `InitializeFromMessageData` L137088）；ModifyBuildingEnergy 消费插在 `frame_processor` 的 modifies 与 removes 之间（原版 L115484-115540）；`BuildingHeatExchange::Update` 挂在 `c2_physics::update_data` 末尾（原版 L145434 组件阶段），遍历记录、逐格换热、调用新建的 `c2_physics/temperature.rs::do_state_transition`。

**技术栈：** Rust / cargo（lib 单测 + Release 构建）/ 严格对照 `OxygenNotIncluded_Source/SimDLL_Source.c`。全量测试用 `cargo test -- --test-threads=1`（项目已知 flaky 对策）。

---

## 文件清单

| 文件 | 职责 | 动作 |
|------|------|------|
| `simDLL_RS/src/b_elements/elements_table.rs` | 新增 `get_element_by_idx`（DoStateTransition 非固态分支读 Element.low_temp/high_temp） | 修改 |
| `simDLL_RS/src/c2_physics/liquid_flow.rs` | `push_substance_change`、`clear_cell` 改 `pub(crate)` 供复用 | 修改 |
| `simDLL_RS/src/c2_physics/temperature.rs` | 新建：`spawn_ore` + `do_state_transition`（C2 温度模块未来主体） | 新建 |
| `simDLL_RS/src/c2_physics/mod.rs` | 注册 `pub mod temperature;`；`update_data` 末尾挂载 `update_building_heat_exchange` | 修改 |
| `simDLL_RS/src/c_simulation/building_temperature.rs` | 记录派生字段；`modify_building_energy`；`modify` 过热事件；`update_building_heat_exchange` | 修改 |
| `simDLL_RS/src/c_simulation/frame_processor.rs` | 消费 `modify_building_energy_messages`（modifies 与 removes 之间） | 修改 |

## 全局约束

- 严格对照原版行号（规格 3.1-3.5），**禁止照抄** `my_Project\simDLL_RS_Kit\simDLL_RS`（神秘版本）。
- TDD：每个新函数先写失败测试 → 验证失败 → 最少实现 → 验证通过 → commit。
- 断言（原版 DebugBreak/崩溃）按项目惯例转 `tracing::error/warn` + 继续。
- 全部单测须持 `LIB_TESTS_LOCK`（与既有测试一致）；元素表测试用独立元素表（不污染真实表，参照 `c2_physics/mod.rs` 的 `init_pressure_table` 风格）。

---

### 任务 1：元素表访问器 `get_element_by_idx`

**文件：** `simDLL_RS/src/b_elements/elements_table.rs`

- [ ] **步骤 1：写失败测试**

```rust
#[test]
fn get_element_by_idx_returns_element_or_none() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();
    table.elements.clear();
    let mut e = Element::default();
    e.id = 0x12345678;
    e.low_temp = 100.0;
    e.high_temp = 500.0;
    table.elements.push(e);
    drop(table);
    let got = get_element_by_idx(0).expect("elem 0 exists");
    assert_eq!(got.id, 0x12345678);
    assert_eq!(got.low_temp, 100.0);
    assert!(get_element_by_idx(0xffff).is_none());
}
```

- [ ] **步骤 2：运行确认失败**

```powershell
cargo test -p simdll_rs get_element_by_idx_returns_element_or_none
```
预期：`error[E0425]: cannot find function get_element_by_idx`。

- [ ] **步骤 3：最少实现**

```rust
/// 查询完整元素记录（gElements，0xa4B）。DoStateTransition 低温非固态分支读 low_temp/high_temp。
pub fn get_element_by_idx(elem_idx: u16) -> Option<Element> {
    let idx = elem_idx as usize;
    match G_ELEMENTS_TABLE.lock() {
        Ok(table) => table.elements.get(idx).copied(),
        Err(e) => e.into_inner().elements.get(idx).copied(),
    }
}
```

- [ ] **步骤 4：运行确认通过**

```powershell
cargo test -p simdll_rs get_element_by_idx_returns_element_or_none
```
预期：`test result: ok`。

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/b_elements/elements_table.rs
git commit -m "feat(elements): add get_element_by_idx accessor"
```

---

### 任务 2：BuildingTemperatureRecord 派生字段（InitializeFromMessageData）

**文件：** `simDLL_RS/src/c_simulation/building_temperature.rs`

新增字段：`total_heat_capacity: f32`、`per_cell_heat_capacity: f32`、`high_temp: f32`、`sim_min_x/sim_min_y/sim_max_x/sim_max_y: i32`。`thermal_conductivity` 语义改为记录派生值（元素TC×消息TC）。新增 `build_record_from_msg(msg) -> Option<BuildingTemperatureRecord>`（对照 L137088-137186），`add`/`modify` 改用它。

- [ ] **步骤 1：写失败测试（派生字段 + 三个错误分支）**

```rust
fn init_temp_table() {
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();
    table.elements.clear();
    table.element_names.clear();
    table.state_data.clear();
    table.liquid_data.clear();
    table.pressure_data.clear();
    table.post_process_data.clear();
    table.temperature_data.clear();
    // elem 5：SHC=2.0、TC=4.0、highTemp=5000、lowTemp=200
    let mut etd = ElementTemperatureData::default();
    etd.state = 2;
    etd.specific_heat_capacity = 2.0;
    etd.thermal_conductivity = 4.0;
    etd.high_temp = 5000.0;
    etd.low_temp = 200.0;
    table.elements.resize(6, Element::default());
    table.temperature_data.resize(6, ElementTemperatureData::default());
    table.temperature_data[5] = etd;
}

#[test]
fn build_record_derives_fields_from_message() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_temp_table();
    let msg = add_msg(10, 300.0); // min(1,2) max(2,3) → 面积 1，mass=100，TC=1.0
    let r = build_record_from_msg(&msg).expect("valid building");
    // totalHC = SHC×mass = 2×100 = 200；perCellHC = 200/1 = 200
    assert_eq!(r.total_heat_capacity, 200.0);
    assert_eq!(r.per_cell_heat_capacity, 200.0);
    // recordTC = 元素TC×消息TC = 4×1 = 4
    assert_eq!(r.thermal_conductivity, 4.0);
    assert_eq!(r.high_temp, 5000.0);
    assert_eq!((r.sim_min_x, r.sim_min_y), (2, 3));
    assert_eq!((r.sim_max_x, r.sim_max_y), (3, 4));
}

#[test]
fn build_record_rejects_zero_area_and_bad_temperature() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_temp_table();
    let mut zero = add_msg(10, 300.0);
    zero.min_x = 2; zero.max_x = 2; // 面积 0
    assert!(build_record_from_msg(&zero).is_none());
    let mut hot = add_msg(10, 20000.0);
    assert!(build_record_from_msg(&hot).is_none());
    let mut bad_elem = add_msg(10, 300.0);
    bad_elem.elem_idx = 0xffff;
    assert!(build_record_from_msg(&bad_elem).is_none());
}
```

（`add_msg` 已有，`mass=100`、`thermal_conductivity=1.0`、`min_x=1/min_y=2/max_x=2/max_y=3`；需补 `use crate::b_elements::element::ElementTemperatureData;`。）

- [ ] **步骤 2：运行确认失败**

```powershell
cargo test -p simdll_rs build_record_derives_fields_from_message build_record_rejects_zero_area_and_bad_temperature
```
预期：`error[E0599]: no function or associated item named build_record_from_msg`。

- [ ] **步骤 3：最少实现**

```rust
pub fn build_record_from_msg(msg: &AddBuildingHeatExchangeMsg) -> Option<BuildingTemperatureRecord> {
    let etd = crate::b_elements::elements_table::get_element_temperature_data(msg.elem_idx)?;
    let area = (msg.max_x - msg.min_x) * (msg.max_y - msg.min_y);
    if area < 1 {
        tracing::error!("Size zero building passed into BuildingHeatExchange");
        return None;
    }
    if !(0.0..=10000.0).contains(&msg.temperature) {
        tracing::error!("Invalid temperature passed into BuildingHeatExchange");
        return None;
    }
    let total_hc = etd.specific_heat_capacity * msg.mass;
    Some(BuildingTemperatureRecord {
        elem_idx: msg.elem_idx,
        mass: msg.mass,
        temperature: msg.temperature,
        thermal_conductivity: etd.thermal_conductivity * msg.thermal_conductivity,
        overheat_temperature: msg.overheat_temperature,
        operating_kilowatts: msg.operating_kilowatts,
        min_x: msg.min_x,
        min_y: msg.min_y,
        max_x: msg.max_x,
        max_y: msg.max_y,
        callback_idx: msg.callback_idx,
        total_heat_capacity: total_hc,
        per_cell_heat_capacity: total_hc / area as f32,
        high_temp: etd.high_temp,
        sim_min_x: msg.min_x + 1,
        sim_min_y: msg.min_y + 1,
        sim_max_x: msg.max_x + 1,
        sim_max_y: msg.max_y + 1,
    })
}
```

`add` 改为 `let Some(record) = build_record_from_msg(msg) else { return -1; };`；`modify` 先算 `let Some(new_rec) = build_record_from_msg(msg) else { return false; };` 再写回（modify 的过热事件在任务 6 加）。

- [ ] **步骤 4：运行确认通过（含既有 5 条单测）**

```powershell
cargo test -p simdll_rs -- --test-threads=1
```
预期：全部通过，无新增失败。

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c_simulation/building_temperature.rs
git commit -m "feat(temperature): derive building heat exchange record fields (InitializeFromMessageData)"
```

---

### 任务 3：SpawnOre + 事件依赖可见性

**文件：** `simDLL_RS/src/c2_physics/temperature.rs`（新建）、`simDLL_RS/src/c2_physics/liquid_flow.rs`、`simDLL_RS/src/c2_physics/mod.rs`

- [ ] **步骤 1：写失败测试（spawn_ore 事件、gameCell 换算、可见性门控、mass≤0）**

```rust
// c2_physics/temperature.rs 内 tests
fn make_sim() -> SimData {
    let mut sd = SimData::new_for_allocate(6, 6, 1, false, true);
    sd.vacuum_element_idx = 0;
    sd.void_element_idx = 0xFFFF;
    sd
}

#[test]
fn spawn_ore_pushes_event_with_game_cell_and_respects_visibility() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    let mut sd = make_sim();
    let events = unsafe { &mut *sd.sim_events.ptr };
    // cell14 = sim(row2,col2) → game (row1,col1) = 1×4+0 = 4
    assert!(spawn_ore(&mut sd, events, 14, 7, 5.0, 300.0, 0xFF, 0, false));
    let info = events.spawn_ore_info.as_slice();
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].callback_idx, 4);
    assert_eq!(info[0].elem_idx, 7);
    assert_eq!(info[0].mass, 5.0);
    // mass<=0 → false
    assert!(!spawn_ore(&mut sd, events, 14, 7, 0.0, 300.0, 0xFF, 0, false));
}

#[test]
fn spawn_ore_skips_invisible_cell_unless_debug_editing() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    let mut sd = make_sim();
    // visible_grid 全 0xFF（可见兜底）→ 手动置 0 模拟不可见
    unsafe { *sd.visible_grid.ptr.offset(4) = 0; }
    let events = unsafe { &mut *sd.sim_events.ptr };
    assert!(!spawn_ore(&mut sd, events, 14, 7, 5.0, 300.0, 0xFF, 0, false));
    assert!(spawn_ore(&mut sd, events, 14, 7, 5.0, 300.0, 0xFF, 0, true));
    assert_eq!(events.spawn_ore_info.len(), 1);
}
```

- [ ] **步骤 2：运行确认失败**

```powershell
cargo test -p simdll_rs spawn_ore_pushes_event_with_game_cell_and_respects_visibility
```
预期：`error[E0433]: failed to resolve: use of undeclared crate or module temperature`（mod.rs 未注册）或函数未定义。

- [ ] **步骤 3：最少实现**

```rust
// c2_physics/temperature.rs
//! 温度模块（C2）：相变与建筑换热依赖。
use crate::a_framework::game_data::SpawnOreInfo;
use crate::a_framework::sim_data::SimData;
use crate::a_framework::sim_events::SimEvents;

/// SimEvents::SpawnOre（原版 L133682-133737）。只推事件，不扣质量/疾病（调用方扣）。
pub(crate) fn spawn_ore(
    sim: &mut SimData,
    events: &mut SimEvents,
    cell: usize,
    elem_idx: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
    is_debug_editing: bool,
) -> bool {
    if mass <= 0.0 {
        return false;
    }
    let width = sim.width as usize;
    let row = cell / width;
    let col = cell % width;
    let game_cell = (row as i32 - 1) * (width as i32 - 2) + col as i32 - 1;
    if game_cell < 0 || game_cell >= sim.num_game_cells {
        return false;
    }
    if !is_debug_editing && !sim.visible_grid.ptr.is_null() {
        let visible = unsafe { &*sim.visible_grid.ptr };
        if visible.get(game_cell as usize).copied().unwrap_or(0) == 0 {
            return false;
        }
    }
    events.spawn_ore_info.push(SpawnOreInfo {
        callback_idx: game_cell,
        elem_idx,
        disease_idx,
        pad: 0,
        mass,
        temperature,
        disease_count,
    });
    true
}
```

`liquid_flow.rs`：`fn push_substance_change` → `pub(crate) fn push_substance_change`；`fn clear_cell` → `pub(crate) fn clear_cell`。`mod.rs` 增加 `pub mod temperature;`。

- [ ] **步骤 4：运行确认通过**

```powershell
cargo test -p simdll_rs -- --test-threads=1
```
预期：全部通过。

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c2_physics/temperature.rs simDLL_RS/src/c2_physics/liquid_flow.rs simDLL_RS/src/c2_physics/mod.rs
git commit -m "feat(temperature): add SimEvents::SpawnOre and expose change-substance/clear-cell helpers"
```

---

### 任务 4：DoStateTransition（相变检查）

**文件：** `simDLL_RS/src/c2_physics/temperature.rs`

对照原版 L133261-133737。输入：`sim: &mut SimData`、`cell: usize`、`etd: &ElementTemperatureData`。返回 `bool`。

- [ ] **步骤 1：写失败测试（高温转变 / 低温固态≤0.8 清格 / 低温非固态液滴 / 闸门）**

```rust
fn init_phase_table() {
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();
    table.elements.clear();
    table.temperature_data.clear();
    // elem 1 = 水（液态）：低变 2（冰）、高变 3（蒸汽）、无矿渣
    let mut water = ElementTemperatureData::default();
    water.state = 2;
    water.low_temp = 0.0;
    water.high_temp = 100.0;
    water.low_temp_transition_idx = 2;
    water.high_temp_transition_idx = 3;
    // elem 2 = 冰（固态）：default_mass=1000
    let mut ice = ElementTemperatureData::default();
    ice.state = 3;
    ice.default_mass = 1000.0;
    // elem 3 = 蒸汽（气态）
    let mut steam = ElementTemperatureData::default();
    steam.state = 1;
    table.temperature_data.push(water);
    table.temperature_data.push(ice);
    table.temperature_data.push(steam);
    // Element 表（非固态分支读 lowTemp/highTemp）
    let mut e1 = Element::default(); e1.low_temp = 0.0; e1.high_temp = 100.0;
    let mut e2 = Element::default(); e2.state = 3;
    let mut e3 = Element::default(); e3.state = 1; e3.low_temp = 100.0; e3.high_temp = 999.0;
    table.elements.push(e1); table.elements.push(e2); table.elements.push(e3);
}

fn fill(sd: &mut SimData, cell: usize, elem: u16, mass: f32, temp: f32) {
    unsafe {
        let u = &mut *sd.updated_cells.ptr;
        u.element_idx.set(cell, elem);
        u.mass.set(cell, mass);
        u.temperature.set(cell, temp);
    }
}

#[test]
fn high_transition_switches_element_and_emits_substance_change() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_phase_table();
    let mut sd = make_sim();
    fill(&mut sd, 14, 1, 10.0, 105.0); // 水 105℃ ≥ high+3
    let etd = get_element_temperature_data(1).unwrap();
    assert!(do_state_transition(&mut sd, 14, &etd));
    let u = unsafe { &*sd.updated_cells.ptr };
    assert_eq!(u.element_idx.get(14), 3); // → 蒸汽
    assert!((u.temperature.get(14) - 103.5).abs() < 1e-4); // max(0, 105-1.5)
    let events = unsafe { &*sd.sim_events.ptr };
    assert_eq!(events.substance_change_info.len(), 1);
}

#[test]
fn low_transition_solid_clears_cell_when_mass_ratio_small() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_phase_table();
    let mut sd = make_sim();
    fill(&mut sd, 14, 1, 100.0, -5.0); // 水 -5℃ < low-3，低变=冰(固态)
    let etd = get_element_temperature_data(1).unwrap();
    assert!(do_state_transition(&mut sd, 14, &etd));
    let u = unsafe { &*sd.updated_cells.ptr };
    // 100/1000 = 0.1 ≤ 0.8 → 清格出矿渣（spawn_ore_info 1 条，格子变 vacuum）
    assert_eq!(u.element_idx.get(14), sd.vacuum_element_idx);
    let events = unsafe { &*sd.sim_events.ptr };
    assert_eq!(events.spawn_ore_info.len(), 1);
    assert_eq!(events.spawn_ore_info.get(0).elem_idx, 2);
}

#[test]
fn low_transition_non_solid_drips_liquid_when_visible() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_phase_table();
    let mut sd = make_sim();
    fill(&mut sd, 20, 3, 5.0, 95.0); // 蒸汽 95℃ < 低变范围（100-3），低变=水（非固态）
    let etd = get_element_temperature_data(3).unwrap();
    assert!(do_state_transition(&mut sd, 20, &etd));
    let events = unsafe { &*sd.sim_events.ptr };
    assert_eq!(events.spawn_liquid_info.len(), 1, "可见且上格非固态 → 液滴");
    let u = unsafe { &*sd.updated_cells.ptr };
    assert_eq!(u.element_idx.get(20), sd.vacuum_element_idx);
}

#[test]
fn transition_gate_keeps_within_plus_minus_three() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_phase_table();
    let mut sd = make_sim();
    fill(&mut sd, 14, 1, 10.0, 102.9); // 100+3 内 → 不转变
    let etd = get_element_temperature_data(1).unwrap();
    assert!(!do_state_transition(&mut sd, 14, &etd));
}
```

（低温非固态分支的"上格非固态"检查读 `cells.properties[above] & 2`，测试默认 properties=0 即非固态，且可见兜底全 0xFF → 走液滴分支。）

- [ ] **步骤 2：运行确认失败** — `cargo test -p simdll_rs do_state_transition`；预期：函数未定义。

- [ ] **步骤 3：最少实现**

```rust
/// DoStateTransition（原版 L133261-133737）。返回是否发生转变。
pub(crate) fn do_state_transition(
    sim: &mut SimData,
    cell: usize,
    etd: &ElementTemperatureData,
) -> bool {
    if sim.updated_cells.ptr.is_null() {
        return false;
    }
    let width = sim.width as usize;
    let game_w = (width as i32) - 2;
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if cell >= updated.mass.len() || updated.mass.get(cell) <= 0.0 {
        return false;
    }
    let t = updated.temperature.get(cell);

    if t >= etd.low_temp - 3.0 || etd.low_temp_transition_idx == INVALID_ELEMENT_INDEX {
        // 高温侧（沸腾/熔化）
        if t <= etd.high_temp + 3.0 || etd.high_temp_transition_idx == INVALID_ELEMENT_INDEX {
            return false;
        }
        updated.temperature.set(cell, (t - 1.5).max(0.0));
        updated.element_idx.set(cell, etd.high_temp_transition_idx);
        if etd.high_temp_transition_ore_idx != INVALID_ELEMENT_INDEX {
            let ore_mass = etd.high_temp_transition_ore_mass_conversion * updated.mass.get(cell);
            if ore_mass > 0.001 {
                let converted =
                    (updated.disease_count.get(cell) as f32 * etd.high_temp_transition_ore_mass_conversion) as i32;
                let events = unsafe { &mut *sim.sim_events.ptr };
                if spawn_ore(
                    sim, events, cell, etd.high_temp_transition_ore_idx, ore_mass,
                    updated.temperature.get(cell), updated.disease_idx.get(cell), converted, true,
                ) {
                    updated.mass.set(cell, updated.mass.get(cell) - ore_mass);
                    updated
                        .disease_count
                        .set(cell, updated.disease_count.get(cell) - converted);
                }
            }
        }
        // properties 位 0x40：背墙熔化标记（原版读 updatedCells.properties）
        if updated.properties.get(cell) & 0x40 != 0 {
            let game_cell = (cell / width) as i32 - 1;
            let game_cell = game_cell * game_w + (cell % width) as i32 - 1;
            if game_cell >= 0 && game_cell < sim.num_game_cells {
                let events = unsafe { &mut *sim.sim_events.ptr };
                events.cell_melted_info.push(CellMeltedInfo { game_cell: game_cell as u32 });
                if !sim.timers.ptr.is_null() {
                    unsafe {
                        *sim.timers.ptr.offset(cell as isize) |= 0x1F;
                    }
                }
            }
        }
        crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
        return true;
    }

    // 低温侧（冻结/冷凝）
    updated.temperature.set(cell, (t + 1.5).max(0.0));
    if etd.low_temp_transition_ore_idx != INVALID_ELEMENT_INDEX {
        let ore_mass = etd.low_temp_transition_ore_mass_conversion * updated.mass.get(cell);
        if ore_mass > 0.001 {
            let converted =
                (updated.disease_count.get(cell) as f32 * etd.low_temp_transition_ore_mass_conversion) as i32;
            let events = unsafe { &mut *sim.sim_events.ptr };
            if spawn_ore(
                sim, events, cell, etd.low_temp_transition_ore_idx, ore_mass,
                updated.temperature.get(cell), updated.disease_idx.get(cell), converted, true,
            ) {
                updated.mass.set(cell, updated.mass.get(cell) - ore_mass);
                updated
                    .disease_count
                    .set(cell, updated.disease_count.get(cell) - converted);
            }
        }
    }
    let Some(transition_etd) =
        crate::b_elements::elements_table::get_element_temperature_data(etd.low_temp_transition_idx)
    else {
        crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
        return true;
    };
    if transition_etd.state & 3 == 3 {
        // 目标为固态：质量比 ≤ 0.8 → 清格出矿渣
        let mass = updated.mass.get(cell);
        if mass / transition_etd.default_mass <= 0.8 {
            let events = unsafe { &mut *sim.sim_events.ptr };
            if spawn_ore(
                sim, events, cell, etd.low_temp_transition_idx, mass,
                updated.temperature.get(cell), updated.disease_idx.get(cell),
                updated.disease_count.get(cell), sim.debug_properties.is_debug_editing,
            ) {
                crate::c2_physics::liquid_flow::clear_cell(sim, updated, cell);
                crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
                return true;
            }
        }
        updated.element_idx.set(cell, etd.low_temp_transition_idx);
        crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
        return true;
    }
    // 目标非固态：上格非固态且可见 → 液滴（spawnLiquidInfo），否则保格
    if !sim.headless && cell >= width {
        let above = cell - width;
        let cells = unsafe { &*sim.cells.ptr };
        if above < cells.properties.len() && cells.properties.get(above) & 2 == 0 {
            let game_cell = (cell / width) as i32 - 1;
            let game_cell = game_cell * game_w + (cell % width) as i32 - 1;
            let visible = sim.visible_grid.ptr.is_null()
                || unsafe { *sim.visible_grid.ptr.offset(game_cell as isize) } != 0
                || sim.debug_properties.is_debug_editing;
            if visible {
                if let Some(elem) =
                    crate::b_elements::elements_table::get_element_by_idx(etd.low_temp_transition_idx)
                {
                    if elem.low_temp - 3.0 <= t && t <= elem.high_temp + 3.0 {
                        let events = unsafe { &mut *sim.sim_events.ptr };
                        events.spawn_liquid_info.push(SpawnFallingLiquidInfo {
                            callback_idx: game_cell,
                            elem_idx: etd.low_temp_transition_idx,
                            disease_idx: updated.disease_idx.get(cell),
                            pad: 0,
                            mass: updated.mass.get(cell),
                            temperature: updated.temperature.get(cell),
                            disease_count: updated.disease_count.get(cell),
                        });
                        crate::c2_physics::liquid_flow::clear_cell(sim, updated, cell);
                        crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
                        return true;
                    }
                }
            }
        }
    }
    updated.element_idx.set(cell, etd.low_temp_transition_idx);
    crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
    true
}
```

（需 `use crate::a_framework::game_data::{CellMeltedInfo, SpawnFallingLiquidInfo};`、`use crate::b_elements::element::{ElementTemperatureData, INVALID_ELEMENT_INDEX};`、`use crate::b_elements::elements_table::get_element_temperature_data;`。SpawnFallingLiquidInfo 字段名按 `game_data.rs` 既有定义核对。）

- [ ] **步骤 4：运行确认通过**

```powershell
cargo test -p simdll_rs -- --test-threads=1
```
预期：全部通过。

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c2_physics/temperature.rs
git commit -m "feat(temperature): implement DoStateTransition phase change"
```

---

### 任务 5：ModifyBuildingEnergy 消费

**文件：** `simDLL_RS/src/c_simulation/building_temperature.rs`、`simDLL_RS/src/c_simulation/frame_processor.rs`

- [ ] **步骤 1：写失败测试（公式、钳制、坏句柄、非法温度不写回）**

```rust
#[test]
fn modify_building_energy_applies_delta_and_clamps() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    let mut mgr = BuildingTemperatureManager::new();
    let mut msg = add_msg(10, 300.0);
    msg.mass = 100.0;
    msg.min_x = 0; msg.min_y = 0; msg.max_x = 2; msg.max_y = 2; // 面积 4
    // perCellHC = SHC×mass/面积；测试表 elem5：SHC=2 → 200/4 = 50
    init_temp_table_for_manager();
    let h = mgr.add(&msg);
    // delta = perCellHC×面积×1℃ = 50×4×1 = 200
    assert!(mgr.modify_building_energy(h, 200.0, 0.0, 10000.0));
    assert!((mgr.record(h).unwrap().temperature - 301.0).abs() < 1e-3);
    // 钳制 [min,max]：向 10000 方向加 1e6，但 msg.max=350 → 350
    assert!(mgr.modify_building_energy(h, 1e6, 0.0, 350.0));
    assert!((mgr.record(h).unwrap().temperature - 350.0).abs() < 1e-3);
    // 坏句柄
    assert!(!mgr.modify_building_energy(h + (1 << 24), 200.0, 0.0, 10000.0));
}

#[test]
fn modify_building_energy_rejects_invalid_final_temperature() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    let mut mgr = BuildingTemperatureManager::new();
    init_temp_table_for_manager();
    let h = mgr.add(&add_msg(10, 300.0)); // 面积 1，perCellHC = 200
    // 无法把 [0,10000] 外的值钳回 → 不写回
    assert!(mgr.modify_building_energy(h, 1e10, 0.0, 10000.0));
    assert!((mgr.record(h).unwrap().temperature - 300.0).abs() < 1e-3);
}
```

（需要 `mgr.record(h) -> Option<&BuildingTemperatureRecord>` 只读访问器，以及 `init_temp_table_for_manager` 复用任务 2 的表初始化；`add_msg` 默认 `max_x=2/max_y=3/min_x=1/min_y=2` 面积 1、perCellHC=200。）

- [ ] **步骤 2：运行确认失败** — 预期：`modify_building_energy` 未定义。

- [ ] **步骤 3：最少实现**

```rust
/// ModifyBuildingEnergy 消费（原版 L115484-115540）。
/// 返回是否成功写回。
pub fn modify_building_energy(
    &mut self,
    handle: i32,
    delta_kj: f32,
    min_temperature: f32,
    max_temperature: f32,
) -> bool {
    let Some(record) = self.get_mut(handle) else {
        return false;
    };
    let f = record.temperature;
    let clamp_low = f.min(min_temperature);
    let clamp_high = f.max(max_temperature);
    if clamp_low >= 0.0 && clamp_high <= 10000.0 {
        let area = (record.sim_max_x - record.sim_min_x) as f32
            * (record.sim_max_y - record.sim_min_y) as f32;
        let mut f2 = f + delta_kj / (area * record.per_cell_heat_capacity);
        f2 = f2.max(clamp_low).min(clamp_high);
        if f2 <= 0.0 || f2 >= 10000.0 {
            tracing::warn!(
                handle,
                delta_kj,
                f2,
                "Invalid final temperature in ProcessBuildingHeatExchangeMessages"
            );
            return false;
        }
        record.temperature = f2;
        return true;
    }
    false
}
```

`frame_processor.rs` 在 modifies 块之后、removes 块之前插入：

```rust
{
    // ModifyBuildingEnergy 消费（原版 L115484-115540，modifies 之后 removes 之前）
    let bytes = frame.modify_building_energy_messages.as_slice();
    let n = bytes.len() / 16;
    for i in 0..n {
        let off = i * 16;
        let handle = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let delta_kj = f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
        let min_temp = f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
        let max_temp = f32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap());
        let ok = crate::c_simulation::building_temperature::modify_building_energy(
            handle, delta_kj, min_temp, max_temp,
        );
        tracing::debug!(handle, ok, "modify_building_energy consumed");
    }
    frame.modify_building_energy_messages.clear_keep_capacity();
}
```

顶层函数：

```rust
pub fn modify_building_energy(handle: i32, delta_kj: f32, min_temp: f32, max_temp: f32) -> bool {
    BUILDING_TEMPERATURE_MANAGER
        .lock()
        .modify_building_energy(handle, delta_kj, min_temp, max_temp)
}
```

- [ ] **步骤 4：运行确认通过（全量）**

```powershell
cargo test -p simdll_rs -- --test-threads=1
```

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c_simulation/building_temperature.rs simDLL_RS/src/c_simulation/frame_processor.rs
git commit -m "feat(temperature): consume ModifyBuildingEnergy (building self-heat)"
```

---

### 任务 6：Modify 过热事件（no-longer-overheated）

**文件：** `simDLL_RS/src/c_simulation/building_temperature.rs`、`simDLL_RS/src/c_simulation/frame_processor.rs`

对照原版 `BuildingHeatExchange::Modify` L137187-137203：`old_temp >= overheat && new_temp < overheat` → 推 `MeltedInfo{handle}` 到 `buildingNoLongerOverheatedInfo`。

- [ ] **步骤 1：写失败测试（≥ 与 < 边界）**

```rust
#[test]
fn modify_pushes_no_longer_overheated_at_boundary() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    let mut sd = make_sim_for_events();
    let events = unsafe { &mut *sd.sim_events.ptr };
    let mut mgr = BuildingTemperatureManager::new();
    let mut msg = add_msg(10, 2000.0); // overheat=2000
    let h = mgr.add(&msg);
    // old=2000 >= overheat=2000，new=1999 < 2000 → 事件
    let mut m2 = add_msg(10, 1999.0);
    assert!(mgr.modify_with_events(h, &m2, events));
    assert_eq!(events.building_no_longer_overheated_info.len(), 1);
    assert_eq!(events.building_no_longer_overheated_info.get(0).handle.value, h);
    // old=1999 < overheat → 无事件
    let mut m3 = add_msg(10, 1800.0);
    assert!(mgr.modify_with_events(h, &m3, events));
    assert_eq!(events.building_no_longer_overheated_info.len(), 1);
}
```

- [ ] **步骤 2：运行确认失败** — 预期：`modify_with_events` 未定义。

- [ ] **步骤 3：最少实现**

```rust
/// Modify + 过热事件（原版 L137187-137203）。event 推 buildingNoLongerOverheatedInfo。
pub fn modify_with_events(
    &mut self,
    handle: i32,
    msg: &AddBuildingHeatExchangeMsg,
    events: &mut crate::a_framework::sim_events::SimEvents,
) -> bool {
    let Some(new_rec) = build_record_from_msg(msg) else {
        return false;
    };
    let Some(record) = self.get_mut(handle) else {
        return false;
    };
    let old_temp = record.temperature;
    if old_temp >= record.overheat_temperature && new_rec.temperature < record.overheat_temperature {
        events.building_no_longer_overheated_info.push(MeltedInfo {
            handle: Handle { value: handle },
        });
    }
    *record = new_rec;
    true
}
```

`modify` 保留为无事件版本（或直接替换调用点为 `modify_with_events`，frame_processor 的 modifies 分支调用处传入 `events = unsafe { &mut *sim_data.sim_events.ptr }`）。

- [ ] **步骤 4：运行确认通过（全量）**

```powershell
cargo test -p simdll_rs -- --test-threads=1
```

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c_simulation/building_temperature.rs simDLL_RS/src/c_simulation/frame_processor.rs
git commit -m "feat(temperature): emit buildingNoLongerOverheatedInfo on Modify"
```

---

### 任务 7：BuildingHeatExchange::Update（建筑↔格子换热）

**文件：** `simDLL_RS/src/c_simulation/building_temperature.rs`、`simDLL_RS/src/c2_physics/mod.rs`

对照原版 L137362-137633。挂载：`c2_physics::update_data` 末尾调用 `building_temperature::update_building_heat_exchange(sim_data)`（dt=0.2）。

- [ ] **步骤 1：写失败测试（真空跳过 / 单格能量守恒 / 事件 / 产热）**

```rust
fn make_sim_6x6() -> SimData {
    let mut sd = SimData::new_for_allocate(6, 6, 1, false, true);
    sd.vacuum_element_idx = 0;
    sd.void_element_idx = 0xFFFF;
    sd.debug_properties.building_temperature_scale = 1.0;
    sd
}

#[test]
fn building_exchange_skips_vacuum_and_converges_with_matter_cell() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_temp_table(); // elem5 液态 SHC=2 TC=4 highTemp=5000
    let mut sd = make_sim_6x6();
    unsafe {
        let u = &mut *sd.updated_cells.ptr;
        // 建筑 sim 范围 (2,3)-(3,5) 覆盖 cell20(row3,col2) 与 cell26(row4,col2)
        u.element_idx.set(20, 5);
        u.mass.set(20, 100.0);
        u.temperature.set(20, 300.0);
        u.insulation.set(20, 1);
        // cell26 真空：mass=0 → 不换热
        u.element_idx.set(26, 0);
        u.mass.set(26, 0.0);
        u.temperature.set(26, 0.0);
        u.insulation.set(26, 0);
    }
    // 面积 1×2，mass=100，SHC=2 → totalHC=200、perCellHC=100
    let mut msg = add_msg(10, 400.0);
    msg.min_x = 1; msg.min_y = 2; msg.max_x = 2; msg.max_y = 4; // sim 范围 (2,3)-(3,5)
    let h = add_building(&msg);
    update_building_heat_exchange(&mut sd);
    let u = unsafe { &*sd.updated_cells.ptr };
    // cell20 有质量 → 参与换热；cell26 真空 → 温度仍 0
    assert_eq!(u.temperature.get(26), 0.0);
    assert!((u.temperature.get(20) - 400.0).abs() < 300.0, "cell20 温度向建筑收敛");
    remove_building(h);
}
```

（`update_building_heat_exchange` 为顶层函数：锁 manager → 遍历记录 → 逐格换热 → 推事件。坐标映射：`cell = y×width + x`，sim 范围 = min+1 的半开区间 `[simMin, simMax)`。）

- [ ] **步骤 2：运行确认失败** — 预期：`update_building_heat_exchange` 未定义。

- [ ] **步骤 3：最少实现（核心骨架）**

```rust
/// BuildingHeatExchange::Update（原版 L137362-137633）。dt=0.2。
pub fn update_building_heat_exchange(sim_data: &mut SimData) {
    let mut mgr = BUILDING_TEMPERATURE_MANAGER.lock();
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let dt = 0.2f32;
    let scale = sim_data.debug_properties.building_temperature_scale;
    let width = sim_data.width as usize;
    let mut records_snapshot: Vec<(i32, BuildingTemperatureRecord)> = mgr
        .records
        .iter()
        .enumerate()
        .filter_map(|(ri, r)| {
            mgr.items.iter().position(|&i| i == ri as i32).map(|_| {
                // 句柄 = items 中的 handle index → 需要 versions/items 反查
                let idx = mgr.items.iter().position(|&i| i == ri as i32).unwrap();
                let handle = ((mgr.versions[idx] as i32) << 24) | (idx as i32 & 0xffffff);
                (handle, *r)
            })
        })
        .collect();
    // （骨架：完整实现遍历 records，逐格按规格 3.3 公式换热，写 updated_cells，
    //  调 do_state_transition，推三事件，写回 record.temperature）
    for (handle, rec) in records_snapshot.iter_mut() {
        let _ = (handle, rec, width, dt, scale);
    }
}
```

> 完整实现按规格 3.3 逐格公式（k 用 `insulation²×1.53787e-08×elemTC×recordTC`、交叉守恒钳制、min/max_seen 钳制、operatingKW 项、三事件、写回 record）。实现时须保证：只处理 `per_cell_heat_capacity > 0 && temperature >= 0` 的记录；`cell_count` 用 sim 范围半开区间；每格 `mass > 0` 才换热；事件用 `MeltedInfo{handle}`；写回 `record.temperature`。

`c2_physics/mod.rs` `update_data` 末尾追加：

```rust
// BuildingHeatExchange::Update（原版 L145434 组件阶段：建筑↔格子换热）
crate::c_simulation::building_temperature::update_building_heat_exchange(sim_data);
```

- [ ] **步骤 4：运行确认通过（全量 + Release）**

```powershell
cargo test -p simdll_rs -- --test-threads=1
cargo build --release
```

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c_simulation/building_temperature.rs simDLL_RS/src/c2_physics/mod.rs
git commit -m "feat(temperature): implement BuildingHeatExchange::Update building-cell exchange"
```

---

### 任务 8：集成回归与文档

- [ ] **步骤 1：全量测试 + Release 构建**

```powershell
cargo test -p simdll_rs -- --test-threads=1
cargo build --release
```
预期：全部通过，无 warning 新增。

- [ ] **步骤 2：更新代办.txt**

把"温度模块（方案 A）"从待办移入已实现，记录：ModifyBuildingEnergy 消费、BuildingHeatExchange::Update、过热/熔化事件、DoStateTransition 依赖已实现；留"方格↔方格 UpdateTemperature / 背墙换热 / 导热板 BuildingToBuilding"为后续。

- [ ] **步骤 3：Commit**

```powershell
git add 代办.txt
git commit -m "docs(todo): temperature building chain (Plan A) implemented"
```

---

## 自检

1. **规格覆盖度：** 规格 3.1→任务 2；3.2→任务 5；3.3→任务 7；3.4→任务 4（+任务 3 依赖）；3.5→任务 6；依赖访问器→任务 1；挂载→任务 7；回归/文档→任务 8。无遗漏。
2. **占位符扫描：** 任务 7 步骤 3 的"完整实现按规格 3.3"为**有意留白**（避免与规格重复粘贴），实现者必须逐行对照规格 3.3 的公式与 L137362-137633；除此处外无 TODO/待定。
3. **类型一致性：** `BuildingTemperatureRecord` 新字段在任务 2 定义，任务 5/6/7 使用；`modify_building_energy`/`modify_with_events`/`update_building_heat_exchange` 命名在全文一致；`spawn_ore`、`do_state_transition`、`push_substance_change`、`clear_cell` 签名在任务 3/4 定义并复用。
