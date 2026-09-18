//! BuildingToBuildingHeatExchange（导热板）——原版 L138358-138640。
//! C# 侧 StructureToStructureTemperature 注册/增删接触；sim 侧做建筑↔建筑换热与过热事件。
//! 任务 1：注册表（Register/AddContact/RemoveContact/Unregister）；任务 3：Update 换热。
use crate::a_framework::game_data::{Handle, MeltedInfo};
use crate::a_framework::sim_data::SimData;
use crate::c_simulation::building_temperature::BuildingTemperatureRecord;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// 接触条目：{buildingInContact handle, cellsInContact}（原版 InContactBuildingData，8B）。
#[derive(Clone, Copy, Debug)]
pub struct ContactEntry {
    pub handle: i32,
    pub cells_in_contact: i32,
}

/// 单条导热板记录（原版 BuildingToBuildingHeatExchangeData 40B）：
/// heatExchange_handle 关联建筑温度句柄；contacts 为接触建筑列表。
#[derive(Clone, Debug)]
pub struct BuildingToBuildingRecord {
    pub heat_exchange_handle: i32,
    pub contacts: Vec<ContactEntry>,
}

/// 导热板管理器（全局单例，非 SimData 成员；仅 sim 线程访问）。
/// 注册表模式同 BuildingTemperatureManager：版本句柄、free list、swap-remove。
pub struct BuildingToBuildingManager {
    records: Vec<BuildingToBuildingRecord>,
    items: Vec<i32>,
    versions: Vec<u8>,
    free_handles: Vec<i32>,
}

impl BuildingToBuildingManager {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            items: Vec::new(),
            versions: Vec::new(),
            free_handles: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.items.clear();
        self.versions.clear();
        self.free_handles.clear();
    }

    /// 校验 handle 并返回只读记录。
    pub fn record(&self, handle: i32) -> Option<&BuildingToBuildingRecord> {
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

    fn get_mut(&mut self, handle: i32) -> Option<&mut BuildingToBuildingRecord> {
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
    pub fn add_contact(
        &mut self,
        self_handle: i32,
        building_in_contact: i32,
        cells_in_contact: i32,
    ) -> bool {
        let Some(rec) = self.get_mut(self_handle) else {
            return false;
        };
        rec.contacts.push(ContactEntry {
            handle: building_in_contact,
            cells_in_contact,
        });
        true
    }

    /// RemoveInContact（原版 L138193）：从接触列表移除。
    pub fn remove_contact(&mut self, self_handle: i32, building_in_contact: i32) -> bool {
        let Some(rec) = self.get_mut(self_handle) else {
            return false;
        };
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
        if ri < 0 {
            return false;
        }
        let new_version = version.wrapping_add(1);
        self.versions[index] = new_version;
        self.free_handles
            .push(((new_version as i32) << 24) | (index as i32 & 0xffffff));
        self.items[index] = -1;
        let ri = ri as usize;
        let last = self.records.len() - 1;
        if ri != last {
            self.records.swap(ri, last);
            for it in self.items.iter_mut() {
                if *it == last as i32 {
                    *it = ri as i32;
                    break;
                }
            }
        }
        self.records.pop();
        true
    }
}

impl Default for BuildingToBuildingManager {
    fn default() -> Self {
        Self::new()
    }
}

// ===== 全局单例 =====

static B2B_MANAGER: Lazy<Mutex<BuildingToBuildingManager>> =
    Lazy::new(|| Mutex::new(BuildingToBuildingManager::new()));

/// 清空全部状态（CleanUp 时调用）。
pub fn clear_building_to_building() {
    B2B_MANAGER.lock().clear();
}

/// 测试/诊断：当前活跃建筑-建筑换热记录数。
pub fn building_to_building_count() -> usize {
    B2B_MANAGER
        .lock()
        .items
        .iter()
        .filter(|&&ri| ri >= 0)
        .count()
}

/// 处理 RegisterBuildingToBuildingHeatExchange 消息（原版 L138121）。
pub fn register_building_to_building(callback_idx: i32, heat_exchange_handle: i32) -> i32 {
    B2B_MANAGER
        .lock()
        .register(callback_idx, heat_exchange_handle)
}

/// 处理 AddInContactBuildingToBuildingToBuildingHeatExchange 消息（原版 L137831）。
pub fn add_building_to_building_contact(
    self_handle: i32,
    building_in_contact: i32,
    cells_in_contact: i32,
) -> bool {
    B2B_MANAGER
        .lock()
        .add_contact(self_handle, building_in_contact, cells_in_contact)
}

/// 处理 RemoveBuildingInContactFromBuildingToBuildingHeatExchange 消息（原版 L138193）。
pub fn remove_building_in_contact(self_handle: i32, building_in_contact: i32) -> bool {
    B2B_MANAGER
        .lock()
        .remove_contact(self_handle, building_in_contact)
}

/// 处理 RemoveBuildingToBuildingHeatExchange 消息（原版 L138337）。
pub fn remove_building_to_building(handle: i32) -> bool {
    B2B_MANAGER.lock().unregister(handle)
}

/// BuildingToBuildingHeatExchange::Update（原版 L138358-138640）。
/// 逐记录：building 数据经 heatExchange_handle 查询；遍历接触求 min/max_seen；
/// k = dt×scale×contactTC×bldgTC×(contactT−bldgT)×暖侧HC×0.005；
/// 能量守恒取 min(|Δ|×HC)（带 k 符号）→ 交叉则热容加权平均钳制；
/// 过热事件只发接触方；写回经 change_building_temperature。
pub(crate) fn update_building_to_building_inner(
    mgr: &mut BuildingToBuildingManager,
    sim: &mut SimData,
    dt: f32,
    bounds: crate::d1_activity::RegionBounds,
) {
    if sim.sim_events.ptr.is_null() {
        return;
    }
    let scale = sim.debug_properties.building_to_building_temperature_scale;
    for index in 0..mgr.items.len() {
        let ri = mgr.items[index];
        if ri < 0 {
            continue;
        }
        let rec = mgr.records[ri as usize].clone();
        let Some(bldg) =
            crate::c_simulation::building_temperature::record(rec.heat_exchange_handle)
        else {
            continue;
        };
        // 原版 BuildingToBuildingHeatExchange::Update L138440-138443：仅处理
        // 完全落在 region 矩形内的建筑（★4）。
        if bldg.sim_min_x < bounds.min_x as i32
            || bldg.sim_min_y < bounds.min_y as i32
            || bldg.sim_max_x > bounds.max_x as i32
            || bldg.sim_max_y > bounds.max_y as i32
        {
            continue;
        }
        // min/max_seen 遍历接触（含自身）
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
        // 原版 L36659 每轮重读 `*pfVar20`（建筑实时温度，前一接触已写回）→ 逐接触累积。
        // 2026-08-07 修复：此前用循环前快照 → 多接触时只有最后一个接触对建筑生效。
        let mut cur_bldg_temp = bldg.temperature;
        for (ch, c) in contacts {
            let c_hc = c.total_heat_capacity;
            if c_hc <= 0.0 {
                continue;
            }
            // 暖侧 HC：contactT < bldgT → bldgHC，否则 contactHC
            let f21 = if c.temperature < cur_bldg_temp {
                bldg_hc
            } else {
                c_hc
            };
            let mut k = dt
                * scale
                * c.thermal_conductivity
                * bldg_tc
                * (c.temperature - cur_bldg_temp)
                * f21
                * 0.005;
            let mut new_b = k / bldg_hc + cur_bldg_temp;
            new_b = new_b.max(min_seen).min(max_seen);
            let mut new_c = c.temperature - k / c_hc;
            new_c = new_c.max(min_seen).min(max_seen);
            // 能量守恒：min(|Δbldg|×bldgHC, |Δcontact|×contactHC)，带 k 符号
            let e1 = (new_b - cur_bldg_temp).abs() * bldg_hc;
            let e2 = (new_c - c.temperature).abs() * c_hc;
            let mut ec = e1.min(e2);
            if k < 0.0 {
                ec = -ec;
            }
            new_b = ec / bldg_hc + cur_bldg_temp;
            new_c = c.temperature - ec / c_hc;
            // 交叉（方向反转）→ 热容加权平均钳制
            if (new_b - new_c) * (cur_bldg_temp - c.temperature) < 0.0 {
                new_b = (c.temperature * c_hc + cur_bldg_temp * bldg_hc) / (c_hc + bldg_hc);
                new_b = new_b.max(min_seen).min(max_seen);
                new_c = new_b;
            }
            // 事件（接触方）
            if c.overheat_temperature <= new_c {
                let events = unsafe { &mut *sim.sim_events.ptr };
                events
                    .building_overheat_info
                    .push(MeltedInfo { handle: Handle { value: ch } });
            }
            if c.overheat_temperature <= c.temperature && new_c < c.overheat_temperature {
                let events = unsafe { &mut *sim.sim_events.ptr };
                events
                    .building_no_longer_overheated_info
                    .push(MeltedInfo { handle: Handle { value: ch } });
            }
            // 写回（contact 先、building 后，与原版一致）
            crate::c_simulation::building_temperature::change_building_temperature(ch, new_c);
            crate::c_simulation::building_temperature::change_building_temperature(
                rec.heat_exchange_handle,
                new_b,
            );
            cur_bldg_temp = new_b;
        }
    }
}

/// 顶层：update_data 组件阶段调用。
pub fn update_building_to_building(
    sim: &mut SimData,
    dt: f32,
    bounds: crate::d1_activity::RegionBounds,
) {
    let mut mgr = B2B_MANAGER.lock();
    update_building_to_building_inner(&mut mgr, sim, dt, bounds);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;
    use crate::a_framework::sim_events::AddBuildingHeatExchangeMsg;
    use crate::b_elements::element::{Element, ElementTemperatureData};
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::c_simulation::building_temperature::{add_building, record};
    use crate::LIB_TESTS_LOCK;

    /// 元素表：elem5 SHC=2.0、TC=4.0、highTemp=5000、lowTemp=200（同 building_temperature::tests）。
    /// 注册 9 个建筑（句柄 0..8）。
    fn init_building_table() {
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
        for _ in 0..9 {
            add_building(&bldg_msg(300.0, 1000.0, 4.0));
        }
    }

    /// 面积 1：mass=total_hc/2（SHC=2）→ totalHC 精确；msgTC=tc/4（元素 TC=4）→ 记录 TC 精确。
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

    #[test]
    fn register_and_contacts_and_unregister() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_building_table();
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

    #[test]
    fn update_exchanges_between_building_and_contact() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_building_table();
        // 建筑 A：totalHC=1000、TC=4、300°C；接触 B：totalHC=2000、TC=2、400°C
        let a = add_building(&bldg_msg(300.0, 1000.0, 4.0));
        let b = add_building(&bldg_msg(400.0, 2000.0, 2.0));
        let mut mgr = BuildingToBuildingManager::new();
        let h = mgr.register(-1, a);
        mgr.add_contact(h, b, 1);
        let mut sd = make_sim();
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        update_building_to_building_inner(&mut mgr, &mut sd, 0.2, bounds);
        // 建筑 A 升温、B 降温，能量守恒
        assert!(record(a).unwrap().temperature > 300.0);
        assert!(record(b).unwrap().temperature < 400.0);
        let da = (record(a).unwrap().temperature - 300.0) * 1000.0;
        let db = (record(b).unwrap().temperature - 400.0) * 2000.0;
        // f32 舍入：能量量级 ~6.7e4，守恒偏差 ~0.03（相对 < 5e-7），容差取 0.1
        assert!((da + db).abs() < 0.1);
    }

    #[test]
    fn update_emits_overheat_on_contact() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_building_table();
        let a = add_building(&bldg_msg(300.0, 1000.0, 4.0));
        let mut bmsg = bldg_msg(400.0, 2000.0, 2.0);
        bmsg.overheat_temperature = 350.0; // scale=100 单子步平衡至 366.67°C ≥ 350 → 过热
        let b = add_building(&bmsg);
        let mut mgr = BuildingToBuildingManager::new();
        let h = mgr.register(-1, a);
        mgr.add_contact(h, b, 1);
        let mut sd = make_sim();
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        update_building_to_building_inner(&mut mgr, &mut sd, 0.2, bounds);
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.building_overheat_info.len(), 1, "接触方 B 过热");
        assert_eq!(events.building_overheat_info.get(0).handle.value, b);
    }

    #[test]
    fn update_emits_no_longer_overheated_on_contact() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_building_table();
        let a = add_building(&bldg_msg(300.0, 1000.0, 4.0));
        let mut bmsg = bldg_msg(400.0, 2000.0, 2.0);
        bmsg.overheat_temperature = 380.0; // old 400 ≥ 380，new 366.67 < 380 → no-longer
        let b = add_building(&bmsg);
        let mut mgr = BuildingToBuildingManager::new();
        let h = mgr.register(-1, a);
        mgr.add_contact(h, b, 1);
        let mut sd = make_sim();
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        update_building_to_building_inner(&mut mgr, &mut sd, 0.2, bounds);
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.building_no_longer_overheated_info.len(), 1);
        assert_eq!(events.building_no_longer_overheated_info.get(0).handle.value, b);
    }

    #[test]
    fn update_data_runs_building_to_building() {
        let _lock = LIB_TESTS_LOCK.lock();
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
        assert!(ra.temperature > 300.0, "建筑 A 应升温，got {}", ra.temperature);
        assert!(rb.temperature < 400.0, "建筑 B 应降温，got {}", rb.temperature);
        let da = (ra.temperature - 300.0) * 1000.0;
        let db = (rb.temperature - 400.0) * 2000.0;
        assert!((da + db).abs() < 0.1);
    }
}
