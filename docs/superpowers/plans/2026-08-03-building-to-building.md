# BuildingToBuildingHeatExchange（导热板）实现计划

> **面向 AI 代理的工作者：** 必须子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现本计划。步骤使用复选框（`- [ ]`）语法跟踪进度。

**目标：** 实现 BuildingToBuildingHeatExchange（导热板）——注册表、Register/AddContact/RemoveContact/Unregister 消息、Update 换热（totalHC×TC×0.005 + 能量守恒 + 接触方过热事件）、帧路由修正与组件挂载。

**架构：** 规格 `docs/superpowers/specs/2026-08-03-building-to-building-design.md`（提交 `1eb865f`）。新建 `c_simulation/building_to_building.rs`（管理器）；`building_temperature.rs` 新增 `change_building_temperature`/只读访问；frame_manager 路由 4 条消息（修 AddInContact 8→12B）；frame_processor 消费；update_data 挂载。

**技术栈：** Rust / cargo / 严格对照 `SimDLL_Source.c`（L138358-138640、L138121/137831/138193/138337、L116540-116650、L136853）。全量测试用 `cargo test -- --test-threads=1`。

---

## 文件清单

| 文件 | 职责 | 动作 |
|------|------|------|
| `simDLL_RS/src/c_simulation/building_to_building.rs` | 管理器（注册表/消息/Update） | 新建 |
| `simDLL_RS/src/c_simulation/mod.rs` | 注册模块 | 修改 |
| `simDLL_RS/src/c_simulation/building_temperature.rs` | `change_building_temperature` + 只读记录访问 | 修改 |
| `simDLL_RS/src/a_framework/sim_frame_manager.rs` | 路由 Register/RemoveInContact/AddInContact/Remove | 修改 |
| `simDLL_RS/src/c_simulation/frame_processor.rs` | process_building_to_building_messages | 修改 |
| `simDLL_RS/src/c2_physics/mod.rs` | update_data 挂载 | 修改 |

## 全局约束

- 严格对照原版行号（规格 2.1-3.4），**禁止照抄** `my_Project\simDLL_RS_Kit\simDLL_RS`（神秘版本）。
- TDD：先写失败测试 → 验证失败 → 最少实现 → 验证通过 → commit。
- 断言按项目惯例转 `tracing::error/warn` + 继续；单测持 `LIB_TESTS_LOCK`。
- record@0（heatExchange_handle）写入点在实现期按 L138121/L138470 核对（Update 将 record@0 作为建筑句柄查 buildingHeatExchange）。
- 锁序：B2B_MANAGER → BUILDING_TEMPERATURE_MANAGER（Register 校验与 Update 写回都持 b2b 锁再取 temp 锁；building_temperature 侧不反向访问 b2b，无死锁）。

---

### 任务 1：管理器注册表 + Register/AddContact/RemoveContact/Unregister

**文件：** `simDLL_RS/src/c_simulation/building_to_building.rs`（新建）、`mod.rs`

- [ ] **步骤 1：写失败测试**

```rust
#[test]
fn register_and_contacts_and_unregister() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_building_table(); // 元素表 + 注册建筑句柄 0..8（经 building_temperature manager）
    let mut mgr = BuildingToBuildingManager::new();
    // Register：校验建筑句柄存在（callbackIdx 回调由 frame_processor 负责，见任务 4）
    let h = mgr.register(-1, 7);
    assert_eq!(h & 0xffffff, 0);
    assert_eq!(mgr.record(h).unwrap().heat_exchange_handle, 7);
    // AddContact
    assert!(mgr.add_contact(h, 8, 3));
    assert_eq!(mgr.record(h).unwrap().contacts.len(), 1);
    assert_eq!(mgr.record(h).unwrap().contacts[0].handle, 8);
    assert_eq!(mgr.record(h).unwrap().contacts[0].cells_in_contact, 3);
    // RemoveContact
    assert!(mgr.remove_contact(h, 8));
    assert_eq!(mgr.record(h).unwrap().contacts.len(), 0);
    // Unregister
    assert!(mgr.unregister(h));
    assert!(mgr.record(h).is_none());
    // 无效建筑句柄 → 注册失败
    assert_eq!(mgr.register(-1, 0x12345678), -1);
}
```

> 建筑句柄有效性通过 `building_temperature::record(handle)` 校验（测试表注册句柄 7、8）。

本模块 tests 辅助（任务 1 与任务 3/5 共用）：

```rust
fn init_building_table() {
    // 元素表：elem5 SHC=2.0、TC=4.0、highTemp=5000、lowTemp=200（同 building_temperature::tests）
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();
    table.elements.clear();
    table.element_names.clear();
    table.state_data.clear();
    table.liquid_data.clear();
    table.pressure_data.clear();
    table.post_process_data.clear();
    table.temperature_data.clear();
    let mut etd = ElementTemperatureData::default();
    etd.state = 2;
    etd.specific_heat_capacity = 2.0;
    etd.thermal_conductivity = 4.0;
    etd.high_temp = 5000.0;
    etd.low_temp = 200.0;
    table.elements.resize(6, Element::default());
    table.temperature_data.resize(6, ElementTemperatureData::default());
    table.temperature_data[5] = etd;
    drop(table);
    // 注册 9 个建筑（句柄 0..8）：供热记录 totalHC=SHC×mass、TC=元素TC×msgTC
    for _ in 0..9 {
        add_building(&bldg_msg(300.0, 1000.0, 4.0));
    }
}

/// 面积 1（min(1,2) max(2,3)）：mass=total_hc/2（SHC=2）→ totalHC 精确；
/// msgTC=tc/4（元素 TC=4）→ 记录 TC 精确；overheat=2000。
fn bldg_msg(temp: f32, total_hc: f32, tc: f32) -> AddBuildingHeatExchangeMsg {
    AddBuildingHeatExchangeMsg {
        callback_idx: -1,
        elem_idx: 5,
        pad0: 0,
        pad1: 0,
        mass: total_hc / 2.0,
        temperature: temp,
        thermal_conductivity: tc / 4.0,
        overheat_temperature: 2000.0,
        operating_kilowatts: 0.0,
        min_x: 1,
        min_y: 2,
        max_x: 2,
        max_y: 3,
    }
}

fn make_sim() -> SimData {
    let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
    sd.vacuum_element_idx = 0;
    sd.void_element_idx = 0xFFFF;
    sd.debug_properties.building_to_building_temperature_scale = 100.0;
    sd
}
```

> 测试 imports：`AddBuildingHeatExchangeMsg`、`SimData`、`Element`/`ElementTemperatureData`、`G_ELEMENTS_TABLE`、
> `building_temperature::{add_building, record, clear_building_temperature}`、`LIB_TESTS_LOCK`。

- [ ] **步骤 2：运行确认失败** — `cargo test -p simdll_rs --lib building_to_building`；预期：模块未定义。

- [ ] **步骤 3：最少实现**

```rust
//! BuildingToBuildingHeatExchange（导热板）——原版 L138358-138640。
use crate::a_framework::sim_data::SimData;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

#[derive(Clone, Copy, Debug)]
pub struct ContactEntry {
    pub handle: i32,
    pub cells_in_contact: i32,
}

#[derive(Clone, Debug)]
pub struct BuildingToBuildingRecord {
    pub heat_exchange_handle: i32,
    pub contacts: Vec<ContactEntry>,
}

pub struct BuildingToBuildingManager {
    records: Vec<BuildingToBuildingRecord>,
    items: Vec<i32>,
    versions: Vec<u8>,
    free_handles: Vec<i32>,
}

impl BuildingToBuildingManager {
    pub fn new() -> Self {
        Self { records: Vec::new(), items: Vec::new(), versions: Vec::new(), free_handles: Vec::new() }
    }
    pub fn clear(&mut self) {
        self.records.clear(); self.items.clear(); self.versions.clear(); self.free_handles.clear();
    }
    pub fn record(&self, handle: i32) -> Option<&BuildingToBuildingRecord> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let ri = self.items[index];
        if ri < 0 { return None; }
        self.records.get(ri as usize)
    }
    fn get_mut(&mut self, handle: i32) -> Option<&mut BuildingToBuildingRecord> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let ri = self.items[index];
        if ri < 0 { return None; }
        self.records.get_mut(ri as usize)
    }

    /// Register（原版 L138121）：校验建筑句柄 → 分配空记录。
    pub fn register(&mut self, callback_idx: i32, heat_exchange_handle: i32) -> i32 {
        if crate::c_simulation::building_temperature::record(heat_exchange_handle).is_none() {
            return -1;
        }
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
        self.records.push(BuildingToBuildingRecord {
            heat_exchange_handle,
            contacts: Vec::new(),
        });
        let _ = callback_idx;
        handle
    }

    /// Add（原版 L137831）：追加接触 {buildingInContact, cellsInContact}。
    pub fn add_contact(&mut self, self_handle: i32, building_in_contact: i32, cells_in_contact: i32) -> bool {
        let Some(rec) = self.get_mut(self_handle) else { return false; };
        rec.contacts.push(ContactEntry { handle: building_in_contact, cells_in_contact });
        true
    }

    /// RemoveInContact（原版 L138193）：从接触列表移除。
    pub fn remove_contact(&mut self, self_handle: i32, building_in_contact: i32) -> bool {
        let Some(rec) = self.get_mut(self_handle) else { return false; };
        rec.contacts.retain(|c| c.handle != building_in_contact);
        true
    }

    /// Unregister（原版 L138337）：版本+1、free list、swap-remove。
    pub fn unregister(&mut self, handle: i32) -> bool {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return false;
        }
        let ri = self.items[index];
        if ri < 0 { return false; }
        let new_version = version.wrapping_add(1);
        self.versions[index] = new_version;
        self.free_handles.push(((new_version as i32) << 24) | (index as i32 & 0xffffff));
        self.items[index] = -1;
        let ri = ri as usize;
        let last = self.records.len() - 1;
        if ri != last {
            self.records.swap(ri, last);
            for it in self.items.iter_mut() {
                if *it == last as i32 { *it = ri as i32; break; }
            }
        }
        self.records.pop();
        true
    }
}

static B2B_MANAGER: Lazy<Mutex<BuildingToBuildingManager>> =
    Lazy::new(|| Mutex::new(BuildingToBuildingManager::new()));

pub fn clear_building_to_building() { B2B_MANAGER.lock().clear(); }
pub fn register_building_to_building(callback_idx: i32, heat_exchange_handle: i32) -> i32 {
    B2B_MANAGER.lock().register(callback_idx, heat_exchange_handle)
}
pub fn add_building_to_building_contact(self_handle: i32, building_in_contact: i32, cells_in_contact: i32) -> bool {
    B2B_MANAGER.lock().add_contact(self_handle, building_in_contact, cells_in_contact)
}
pub fn remove_building_in_contact(self_handle: i32, building_in_contact: i32) -> bool {
    B2B_MANAGER.lock().remove_contact(self_handle, building_in_contact)
}
pub fn remove_building_to_building(handle: i32) -> bool {
    B2B_MANAGER.lock().unregister(handle)
}
```

`building_temperature.rs` 新增只读访问（供句柄校验与 Update 读数据）：

```rust
/// 只读访问建筑温度记录（供 BuildingToBuilding 等校验/读取）。
pub fn record(handle: i32) -> Option<BuildingTemperatureRecord> {
    BUILDING_TEMPERATURE_MANAGER.lock().record(handle).copied()
}
```

`mod.rs` 注册 `pub mod building_to_building;`。

- [ ] **步骤 4：运行确认通过（含全量）**
- [ ] **步骤 5：Commit** — `git commit -m "feat(building_to_building): registry + Register/AddContact/RemoveContact/Unregister"`

---

### 任务 2：`change_building_temperature`（原版 L136853）

**文件：** `simDLL_RS/src/c_simulation/building_temperature.rs`

- [ ] **步骤 1：写失败测试**

```rust
#[test]
fn change_building_temperature_updates_record() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_temp_table();
    let h = add_building(&add_msg(10, 300.0));
    assert!(change_building_temperature(h, 350.0));
    assert_eq!(record(h).unwrap().temperature, 350.0);
    assert!(!change_building_temperature(h + (1 << 24), 350.0));
}
```

- [ ] **步骤 2：运行确认失败** — `change_building_temperature` 未定义。
- [ ] **步骤 3：最少实现**

```rust
/// ChangeBuildingTemperature（原版 L136853）：写建筑温度记录（temperatureInfo 由每帧 write 刷新，等价）。
pub fn change_building_temperature(handle: i32, temperature: f32) -> bool {
    BUILDING_TEMPERATURE_MANAGER.lock().set_temperature(handle, temperature)
}
```

manager 内新增（get_mut 校验后写 record.temperature；temperatureInfo 由 copy_sim_data_to_game 的
write_building_temperature_info 每帧刷新，与原版 ChangeBuildingTemperature 写 temperatureInfo 等价）：

```rust
impl BuildingTemperatureManager {
    pub fn set_temperature(&mut self, handle: i32, temperature: f32) -> bool {
        let Some(record) = self.get_mut(handle) else {
            tracing::warn!(handle, "BuildingTemperatureManager::set_temperature: bad handle");
            return false;
        };
        record.temperature = temperature;
        true
    }
}
```

- [ ] **步骤 4：运行确认通过（含全量）**
- [ ] **步骤 5：Commit** — `git commit -m "feat(building_temperature): change_building_temperature (L136853)"`

---

### 任务 3：`update_building_to_building`（原版 L138358-138640）

**文件：** `simDLL_RS/src/c_simulation/building_to_building.rs`

- [ ] **步骤 1：写失败测试（换热数值 / 能量守恒 / 事件）**

```rust
#[test]
fn update_exchanges_between_building_and_contact() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_building_table();
    // 建筑 A：totalHC=1000、TC=4、300°C；接触 B：totalHC=2000、TC=2、400°C
    let a = add_building(&bldg_msg(300.0, 1000.0, 4.0));
    let b = add_building(&bldg_msg(400.0, 2000.0, 2.0));
    let mut mgr = BuildingToBuildingManager::new();
    let h = mgr.register(-1, a);
    mgr.add_contact(h, b, 1);
    let mut sd = make_sim();
    update_building_to_building_inner(&mut mgr, &mut sd, 0.2);
    // 建筑 A 升温、B 降温，能量守恒
    assert!(record(a).unwrap().temperature > 300.0);
    assert!(record(b).unwrap().temperature < 400.0);
    let da = (record(a).unwrap().temperature - 300.0) * 1000.0;
    let db = (record(b).unwrap().temperature - 400.0) * 2000.0;
    assert!((da + db).abs() < 1e-2);
}

#[test]
fn update_emits_overheat_on_contact() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_building_table();
    let a = add_building(&bldg_msg(300.0, 1000.0, 4.0));
    let mut bmsg = bldg_msg(400.0, 2000.0, 2.0);
    bmsg.overheat_temperature = 350.0; // scale=100 单子步平衡至 366.67°C ≥ 350 → 过热
    let b = add_building(&bmsg);
    let mut mgr = BuildingToBuildingManager::new();
    let h = mgr.register(-1, a);
    mgr.add_contact(h, b, 1);
    let mut sd = make_sim();
    update_building_to_building_inner(&mut mgr, &mut sd, 0.2);
    let events = unsafe { &*sd.sim_events.ptr };
    assert_eq!(events.building_overheat_info.len(), 1, "接触方 B 过热");
    assert_eq!(events.building_overheat_info.get(0).handle.value, b);
}

#[test]
fn update_emits_no_longer_overheated_on_contact() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_building_table();
    let a = add_building(&bldg_msg(300.0, 1000.0, 4.0));
    let mut bmsg = bldg_msg(400.0, 2000.0, 2.0);
    bmsg.overheat_temperature = 380.0; // old 400 ≥ 380，new 366.67 < 380 → no-longer
    let b = add_building(&bmsg);
    let mut mgr = BuildingToBuildingManager::new();
    let h = mgr.register(-1, a);
    mgr.add_contact(h, b, 1);
    let mut sd = make_sim();
    update_building_to_building_inner(&mut mgr, &mut sd, 0.2);
    let events = unsafe { &*sd.sim_events.ptr };
    assert_eq!(events.building_no_longer_overheated_info.len(), 1);
    assert_eq!(events.building_no_longer_overheated_info.get(0).handle.value, b);
}
```

> 测试表：bldg_msg(temp, totalHC, TC) 经 building_temperature 构造；overheat 由测试就地改写。

- [ ] **步骤 2：运行确认失败** — `update_building_to_building_inner` 未定义。
- [ ] **步骤 3：最少实现**（核心公式对照 L138505-138600）

```rust
/// BuildingToBuildingHeatExchange::Update（原版 L138358-138640）。
pub(crate) fn update_building_to_building_inner(
    mgr: &mut BuildingToBuildingManager,
    sim: &mut SimData,
    dt: f32,
) {
    if sim.sim_events.ptr.is_null() { return; }
    let scale = sim.debug_properties.building_to_building_temperature_scale;
    for index in 0..mgr.items.len() {
        let ri = mgr.items[index];
        if ri < 0 { continue; }
        let handle = ((mgr.versions[index] as i32) << 24) | (index as i32 & 0xffffff);
        let rec = mgr.records[ri as usize].clone();
        let Some(bldg) = crate::c_simulation::building_temperature::record(rec.heat_exchange_handle) else {
            continue;
        };
        // min/max_seen 遍历接触
        let mut min_seen = bldg.temperature;
        let mut max_seen = bldg.temperature;
        let mut contacts: Vec<(i32, BuildingTemperatureRecord)> = Vec::new();
        for c in &rec.contacts {
            if let Some(b) = crate::c_simulation::building_temperature::record(c.handle) {
                min_seen = min_seen.min(b.temperature);
                max_seen = max_seen.max(b.temperature);
                contacts.push((c.handle, b));
            }
        }
        let bldg_hc = bldg.total_heat_capacity;
        let bldg_tc = bldg.thermal_conductivity;
        for (ch, c) in contacts {
            let c_hc = c.total_heat_capacity;
            if c_hc <= 0.0 { continue; }
            let f21 = if c.temperature < bldg.temperature { bldg_hc } else { c_hc };
            let mut k = dt * scale * c.thermal_conductivity * bldg_tc
                * (c.temperature - bldg.temperature) * f21 * 0.005;
            let mut new_b = k / bldg_hc + bldg.temperature;
            new_b = new_b.max(min_seen).min(max_seen);
            let mut new_c = c.temperature - k / c_hc;
            new_c = new_c.max(min_seen).min(max_seen);
            let e1 = (new_b - bldg.temperature).abs() * bldg_hc;
            let e2 = (new_c - c.temperature).abs() * c_hc;
            let mut ec = e1.min(e2);
            if k < 0.0 { ec = -ec; }
            new_b = ec / bldg_hc + bldg.temperature;
            new_c = c.temperature - ec / c_hc;
            if (new_b - new_c) * (bldg.temperature - c.temperature) < 0.0 {
                new_b = (c.temperature * c_hc + bldg.temperature * bldg_hc) / (c_hc + bldg_hc);
                new_b = new_b.max(min_seen).min(max_seen);
                new_c = new_b;
            }
            // 事件（接触方）
            if c.overheat_temperature <= new_c {
                let events = unsafe { &mut *sim.sim_events.ptr };
                events.building_overheat_info.push(MeltedInfo { handle: Handle { value: ch } });
            }
            if c.overheat_temperature <= c.temperature && new_c < c.overheat_temperature {
                let events = unsafe { &mut *sim.sim_events.ptr };
                events.building_no_longer_overheated_info.push(MeltedInfo { handle: Handle { value: ch } });
            }
            crate::c_simulation::building_temperature::change_building_temperature(ch, new_c);
            crate::c_simulation::building_temperature::change_building_temperature(
                rec.heat_exchange_handle, new_b);
        }
    }
}

/// 顶层：update_data 组件阶段调用。
pub fn update_building_to_building(sim: &mut SimData, dt: f32) {
    let mut mgr = B2B_MANAGER.lock();
    update_building_to_building_inner(&mut mgr, sim, dt);
}
```

> 注意：bldg/contact 记录取自 building_temperature（快照拷贝）；写回经 change_building_temperature。
> 测试用本地 `BuildingToBuildingManager::new()` 实例直接调 inner，避免触碰全局 B2B_MANAGER 锁。

- [ ] **步骤 4：运行确认通过（含全量）**
- [ ] **步骤 5：Commit** — `git commit -m "feat(building_to_building): Update exchange (L138358-138640)"`

---

### 任务 4：frame_manager 路由 + frame_processor 消费

**文件：** `simDLL_RS/src/a_framework/sim_frame_manager.rs`、`simDLL_RS/src/c_simulation/frame_processor.rs`

- [ ] **步骤 1：写失败测试（路由尺寸 8/8/12/8）**

```rust
#[test]
fn handle_message_building_to_building_routes() {
    let mut mgr = SimFrameManager::new_zeroed();
    let mut result: *mut c_void = std::ptr::null_mut();
    // Register 8B
    let mut reg = vec![0u8; 8];
    reg[0..4].copy_from_slice(&(-1i32).to_le_bytes());
    reg[4..8].copy_from_slice(&7i32.to_le_bytes());
    let mut r = BinaryBufferReader::new(&reg);
    assert!(mgr.handle_message(MessageType::AddBuildingToBuildingHeatExchange as u32, &mut r, &mut result));
    assert_eq!(unsafe { &*mgr.current_frame }.building_to_building_heat_exchange_messages.adds.len(), 8);
    // AddInContact 12B（原 8B 错误）
    let mut ac = vec![0u8; 12];
    let mut r = BinaryBufferReader::new(&ac);
    assert!(mgr.handle_message(MessageType::AddInContactBuildingToBuildingToBuildingHeatExchange as u32, &mut r, &mut result));
    assert_eq!(unsafe { &*mgr.current_frame }.add_building_in_contact_messages.len(), 12);
    // RemoveInContact 8B → modifies
    let mut rc = vec![0u8; 8];
    let mut r = BinaryBufferReader::new(&rc);
    assert!(mgr.handle_message(MessageType::RemoveBuildingInContactFromBuildingToBuildingHeatExchange as u32, &mut r, &mut result));
    assert_eq!(unsafe { &*mgr.current_frame }.building_to_building_heat_exchange_messages.modifies.len(), 8);
    // Remove 8B → removes
    let mut rm = vec![0u8; 8];
    let mut r = BinaryBufferReader::new(&rm);
    assert!(mgr.handle_message(MessageType::RemoveBuildingToBuildingHeatExchange as u32, &mut r, &mut result));
    assert_eq!(unsafe { &*mgr.current_frame }.building_to_building_heat_exchange_messages.removes.len(), 8);
    unsafe { let _ = Box::from_raw(mgr.current_frame); }
}
```

- [ ] **步骤 2：运行确认失败** — 路由未实现/尺寸错。
- [ ] **步骤 3：最少实现**

```rust
// sim_frame_manager.rs：从 C2 桩分支移出 + 修正尺寸
MSG_ADD_BUILDING_TO_BUILDING_HE => {
    emplace_bytes(&mut frame.building_to_building_heat_exchange_messages.adds, reader, 8)
}
MSG_REMOVE_BUILDING_IN_CONTACT => {
    emplace_bytes(&mut frame.building_to_building_heat_exchange_messages.modifies, reader, 8)
}
MSG_ADD_BUILDING_IN_CONTACT => {
    emplace_bytes(&mut frame.add_building_in_contact_messages, reader, 12)  // 原 8 修正为 12
}
MSG_REMOVE_BUILDING_TO_BUILDING_HE => {
    emplace_bytes(&mut frame.building_to_building_heat_exchange_messages.removes, reader, 8)
}
```

`frame_processor.rs` 新增 `process_building_to_building_messages`
（adds→modifies→addInContact→removes，对照 L116540-116650）：

```rust
fn process_building_to_building_messages(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    if sim_data.sim_events.ptr.is_null() {
        clear_component_messages(&mut frame.building_to_building_heat_exchange_messages);
        frame.add_building_in_contact_messages.clear_keep_capacity();
        return;
    }
    // adds：8B {callbackIdx, heatExchange_handle} → Register；callbackIdx≠-1 推 componentStateChanged
    {
        let adds = frame.building_to_building_heat_exchange_messages.adds.as_slice();
        let n = adds.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let callback_idx = i32::from_le_bytes(adds[off..off + 4].try_into().unwrap());
            let heat_exchange_handle =
                i32::from_le_bytes(adds[off + 4..off + 8].try_into().unwrap());
            let handle = crate::c_simulation::building_to_building::register_building_to_building(
                callback_idx, heat_exchange_handle,
            );
            if callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events.component_state_changed_messages.push(
                    crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx,
                        sim_handle: handle, // 注册失败时 handle=-1，与原版 Register 一致
                    },
                );
            }
        }
        frame.building_to_building_heat_exchange_messages.adds.clear_keep_capacity();
    }
    // modifies：8B {self_handle, buildingInContact} → RemoveInContact
    {
        let mods = frame.building_to_building_heat_exchange_messages.modifies.as_slice();
        let n = mods.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let self_handle = i32::from_le_bytes(mods[off..off + 4].try_into().unwrap());
            let building_in_contact =
                i32::from_le_bytes(mods[off + 4..off + 8].try_into().unwrap());
            crate::c_simulation::building_to_building::remove_building_in_contact(
                self_handle, building_in_contact,
            );
        }
        frame.building_to_building_heat_exchange_messages.modifies.clear_keep_capacity();
    }
    // addInContact：12B {self_handle, buildingInContact, cellsInContact} → Add
    {
        let bytes = frame.add_building_in_contact_messages.as_slice();
        let n = bytes.len() / 12;
        for i in 0..n {
            let off = i * 12;
            let self_handle = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            let building_in_contact =
                i32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            let cells_in_contact =
                i32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
            crate::c_simulation::building_to_building::add_building_to_building_contact(
                self_handle, building_in_contact, cells_in_contact,
            );
        }
        frame.add_building_in_contact_messages.clear_keep_capacity();
    }
    // removes：8B {callbackIdx, handle} → Unregister；callbackIdx≠-1 推 {callbackIdx, -1}
    {
        let removes = frame.building_to_building_heat_exchange_messages.removes.as_slice();
        let n = removes.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let callback_idx = i32::from_le_bytes(removes[off..off + 4].try_into().unwrap());
            let handle = i32::from_le_bytes(removes[off + 4..off + 8].try_into().unwrap());
            crate::c_simulation::building_to_building::remove_building_to_building(handle);
            if callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events.component_state_changed_messages.push(
                    crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx,
                        sim_handle: -1, // 原版 remove 回调 sim_handle=-1
                    },
                );
            }
        }
        frame.building_to_building_heat_exchange_messages.removes.clear_keep_capacity();
    }
}
```

接入点：
- `process_frame`：在 `process_building_heat_exchange_messages(frame, sim_data);` 与
  `process_element_chunk_messages(frame, sim_data);` 之间插入 `process_building_to_building_messages(frame, sim_data);`
  （原版 b2b 段位于 buildingHeatExchange 与 elementChunk 之间）。
- `process_component_messages_stubs`：删除 `building_to_building_heat_exchange_messages` 与
  `add_building_in_contact_messages` 两段清理（改由新函数消费）。

- [ ] **步骤 4：运行确认通过（含全量）**
- [ ] **步骤 5：Commit** — `git commit -m "feat(building_to_building): frame routing + ProcessFrame consumption"`

---

### 任务 5：update_data 挂载 + 集成

**文件：** `simDLL_RS/src/c2_physics/mod.rs`

- [ ] **步骤 1：写失败测试（组件阶段调用后事件/温度变化）**

```rust
#[test]
fn update_data_runs_building_to_building() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_building_table();
    crate::c_simulation::building_temperature::clear_building_temperature();
    crate::c_simulation::building_to_building::clear_building_to_building();
    let a = add_building(&bldg_msg(300.0, 1000.0, 4.0));
    let b = add_building(&bldg_msg(400.0, 2000.0, 2.0));
    let h = crate::c_simulation::building_to_building::register_building_to_building(-1, a);
    crate::c_simulation::building_to_building::add_building_to_building_contact(h, b, 1);
    let mut sd = make_sim();
    crate::c2_physics::update_data(&mut sd);
    let ra = record(a).unwrap();
    let rb = record(b).unwrap();
    assert!(ra.temperature > 300.0);
    assert!(rb.temperature < 400.0);
    let da = (ra.temperature - 300.0) * 1000.0;
    let db = (rb.temperature - 400.0) * 2000.0;
    assert!((da + db).abs() < 1e-2);
}
```

- [ ] **步骤 2：运行确认失败** — 组件阶段未挂载 → 温度不变，断言失败。
- [ ] **步骤 3：最少实现**

```rust
// update_data 组件阶段末尾（与建筑↔格子/碎片并列，原版 L145434 同一组件循环）
crate::c_simulation::building_to_building::update_building_to_building(sim_data, 0.2);
```

- [ ] **步骤 4：运行确认通过（含全量 + Release）**
- [ ] **步骤 5：Commit** — `git commit -m "feat(building_to_building): wire update_data component phase"`

---

### 任务 6：集成回归与文档

- [ ] **步骤 1：全量测试 + Release** — `cargo test -p simdll_rs -- --test-threads=1` + `cargo build --release`。
- [ ] **步骤 2：更新代办.txt**：导热板标为已实现；留"升华/持续蒸发、ElementEmitter 本体"为后续。
- [ ] **步骤 3：Commit** — `git commit -m "docs(todo): BuildingToBuildingHeatExchange implemented"`

---

## 自检

1. **规格覆盖度：** 规格 2.2/2.3→任务 1/4（Register 回调 componentStateChanged 在任务 4 处理器实现，含失败注册推 -1）；3.1→任务 1；3.2→任务 2；3.3→任务 3；3.4→任务 4/5；测试/验收→任务 6。无遗漏。
2. **占位符扫描：** 已修正——任务 1 增加 `init_building_table`/`bldg_msg`/`make_sim` 辅助定义；任务 2 改为 `set_temperature` 具体实现；任务 3 测试改用本地 manager 并修正过热阈值数值；任务 4 补全 `process_building_to_building_messages` 代码；任务 5 测试给出实际断言。无 TODO/待定。
3. **类型一致性：** `BuildingToBuildingRecord`/`ContactEntry` 字段在任务 1 定义并复用；`change_building_temperature`/`update_building_to_building`/`process_building_to_building_messages` 命名全文一致。
