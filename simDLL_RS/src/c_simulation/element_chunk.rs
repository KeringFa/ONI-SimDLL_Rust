//! ElementChunk（物质碎片）温度组件。
//!
//! 对照原版 `SimDLL_Source.c`：
//! - `ElementChunk::Register`（L139972-140075）、`Modify`（L139894）、`ModifyAdjuster`（L139926）、
//!   `ModifyEnergy`（L139947）、`Unregister`（L140077）；
//! - `ExchangeHeatEnergyWithWorld`（L139789-139890）、`ElementChunk::Update`（L140092-140202）。

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
        Self {
            handle: -1,
            temperature: 0.0,
            heat_capacity: 0.0,
            thermal_conductivity: 0.0,
        }
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

/// ElementChunk 管理器（模式同 BuildingTemperatureManager：版本句柄 + free list + swap-remove）。
pub struct ElementChunkManager {
    records: Vec<ElementChunkData>,
    items: Vec<i32>,
    versions: Vec<u8>,
    free_handles: Vec<i32>,
    /// 每帧输出 {temperature, delta_kj}（按 handle index；copy_sim_data_to_game swap 消费）。
    pub(crate) output: Vec<ElementChunkInfo>,
}

impl ElementChunkManager {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            items: Vec::new(),
            versions: Vec::new(),
            free_handles: Vec::new(),
            output: Vec::new(),
        }
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
        let Some(etd) = crate::b_elements::elements_table::get_element_temperature_data(msg.element_idx)
        else {
            return -1;
        };
        let rec = ElementChunkData {
            cell: game_to_sim_cell(msg.game_cell, width),
            temperature: msg.temperature,
            heat_capacity: etd.specific_heat_capacity * msg.mass,
            thermal_conductivity: etd.thermal_conductivity,
            max_energy_transfer_scale_factor: if msg.thickness != 0.0 {
                msg.surface_area / msg.thickness
            } else {
                0.0
            },
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
        let Some(d) = self.get_mut(handle) else {
            return false;
        };
        d.temperature = temperature;
        d.heat_capacity = heat_capacity;
        true
    }

    /// ModifyEnergy（原版 L139947）：temp += deltaKJ/heatCapacity，钳制 ≥0。
    pub fn modify_energy(&mut self, handle: i32, delta_kj: f32) -> bool {
        let Some(d) = self.get_mut(handle) else {
            return false;
        };
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
    pub fn modify_adjuster(
        &mut self,
        handle: i32,
        temperature: f32,
        heat_capacity: f32,
        thermal_conductivity: f32,
    ) -> bool {
        let Some(d) = self.get_mut(handle) else {
            return false;
        };
        d.adjuster.temperature = temperature;
        d.adjuster.heat_capacity = heat_capacity;
        d.adjuster.thermal_conductivity = thermal_conductivity;
        true
    }

    /// Move（原版 MoveElementChunkMessage {handle, gameCell}）。
    pub fn move_chunk(&mut self, handle: i32, game_cell: i32, width: i32) -> bool {
        let Some(d) = self.get_mut(handle) else {
            return false;
        };
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

    /// 测试/输出访问。
    pub fn output_snapshot(&self) -> &[ElementChunkInfo] {
        &self.output
    }
}

static ELEMENT_CHUNK_MANAGER: Lazy<Mutex<ElementChunkManager>> =
    Lazy::new(|| Mutex::new(ElementChunkManager::new()));

pub fn clear_element_chunks() {
    ELEMENT_CHUNK_MANAGER.lock().clear();
}

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
    let (cell_mass, cell_temp, cell_ins, cell_hc, cell_tc) = {
        let u = unsafe { &*sim.updated_cells.ptr };
        if cell >= u.mass.len() || u.mass.get(cell) <= 0.0 {
            return 0.0;
        }
        let elem = u.element_idx.get(cell);
        let Some(etd) = crate::b_elements::elements_table::get_element_temperature_data(elem) else {
            return 0.0;
        };
        if etd.state & 0x10 != 0 {
            return 0.0; // TemperatureInsulated → 不换热
        }
        (
            u.mass.get(cell),
            u.temperature.get(cell),
            u.insulation.get(cell) as f32,
            u.mass.get(cell) * etd.specific_heat_capacity,
            etd.thermal_conductivity,
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
    let k_cell = ins_val * cell_tc;
    let k = chunk.thermal_conductivity.min(k_cell)
        * scale
        * chunk.max_energy_transfer_scale_factor
        * 0.001;
    // CalculateTemperatureExchange wrapper（冷侧在前）→ (new_chunk, new_cell)
    let (new_chunk, new_cell) = if chunk.temperature <= cell_temp {
        crate::c2_physics::temperature::calculate_temperature_exchange_precise(
            1.0,
            k,
            dt,
            chunk.temperature,
            chunk.heat_capacity,
            cell_temp,
            cell_hc,
        )
    } else {
        let (nn, nc) = crate::c2_physics::temperature::calculate_temperature_exchange_precise(
            1.0,
            k,
            dt,
            cell_temp,
            cell_hc,
            chunk.temperature,
            chunk.heat_capacity,
        );
        (nc, nn)
    };
    let heat = (chunk.temperature - new_chunk) * chunk.heat_capacity;
    chunk.temperature = new_chunk;
    unsafe {
        (*sim.updated_cells.ptr).temperature.set(cell, new_cell);
    }
    let _ = cell_mass;
    heat
}

/// ElementChunk::Update（原版 L140092-140202）。
/// 逐碎片：adjuster 换热（rate 1.0，不写格子）或世界换热（本格 1.0 + 下方固体 groundTransferScale）；
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
    let w = width as usize;
    for index in 0..mgr.items.len() {
        let ri = mgr.items[index];
        if ri < 0 {
            continue;
        }
        let handle = ((mgr.versions[index] as i32) << 24) | (index as i32 & 0xffffff);
        let chunk = &mut mgr.records[ri as usize];
        if chunk.heat_capacity <= 0.0 {
            continue;
        }
        let mut delta = 0.0f32;
        if chunk.adjuster.heat_capacity > 0.0 {
            // adjuster 分支（原版 L140114-140120）：rate 1.0，不写格子；
            // 原版走 CalculateTemperatureExchange wrapper（冷侧在前），负温差下 min 取负会过冲。
            let nc = if chunk.temperature <= chunk.adjuster.temperature {
                let (nc, _na) =
                    crate::c2_physics::temperature::calculate_temperature_exchange_precise(
                        1.0,
                        chunk.adjuster.thermal_conductivity,
                        dt,
                        chunk.temperature,
                        chunk.heat_capacity,
                        chunk.adjuster.temperature,
                        chunk.adjuster.heat_capacity,
                    );
                nc
            } else {
                let (_na, nc) =
                    crate::c2_physics::temperature::calculate_temperature_exchange_precise(
                        1.0,
                        chunk.adjuster.thermal_conductivity,
                        dt,
                        chunk.adjuster.temperature,
                        chunk.adjuster.heat_capacity,
                        chunk.temperature,
                        chunk.heat_capacity,
                    );
                nc
            };
            chunk.temperature = nc;
        } else {
            let cell = chunk.cell as usize;
            let h1 = exchange_heat_energy_with_world(sim, chunk, cell, 1.0, dt);
            delta += h1;
            // 下方格为固体 → groundTransferScale 换热（原版 L140159-140167）
            if cell >= w {
                let below = cell - w;
                let below_elem = unsafe { (*sim.updated_cells.ptr).element_idx.get(below) };
                if let Some(etd) =
                    crate::b_elements::elements_table::get_element_temperature_data(below_elem)
                {
                    if etd.state & 3 == 3 {
                        let h2 = exchange_heat_energy_with_world(
                            sim,
                            chunk,
                            below,
                            chunk.ground_transfer_scale,
                            dt,
                        );
                        delta += h2;
                    }
                }
            }
        }
        let t = chunk.temperature;
        mgr.output.push(ElementChunkInfo {
            temperature: t,
            delta_kj: delta,
        });
        // 相变（原版 L140185-140196）：T ≥ highTemp+3 或 < lowTemp−3 → elementChunkMeltedInfo
        if chunk.high_temp + 3.0 <= t || t < chunk.low_temp - 3.0 {
            let events = unsafe { &mut *sim.sim_events.ptr };
            events
                .element_chunk_melted_info
                .push(MeltedInfo {
                    handle: Handle { value: handle },
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;
    use crate::b_elements::element::ElementTemperatureData;
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::LIB_TESTS_LOCK;

    /// 元素表：elem2 固体砖（SHC=0.8、TC=2.0）、elem5 液态水（SHC=2.0、TC=4.0、low=200、high=5000）。
    fn init_elem_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.temperature_data.clear();
        let mut rock = ElementTemperatureData::default();
        rock.state = 3;
        rock.specific_heat_capacity = 0.8;
        rock.thermal_conductivity = 2.0;
        let mut water = ElementTemperatureData::default();
        water.state = 2;
        water.specific_heat_capacity = 2.0;
        water.thermal_conductivity = 4.0;
        water.low_temp = 200.0;
        water.high_temp = 5000.0;
        table.temperature_data.push(ElementTemperatureData::default()); // 0
        table.temperature_data.push(ElementTemperatureData::default()); // 1
        table.temperature_data.push(rock); // 2
        table.temperature_data.push(ElementTemperatureData::default()); // 3
        table.temperature_data.push(ElementTemperatureData::default()); // 4
        table.temperature_data.push(water); // 5
    }

    fn make_sim() -> SimData {
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd
    }

    fn add_msg(game_cell: i32, temperature: f32) -> AddElementChunkMsg {
        AddElementChunkMsg {
            game_cell,
            callback_idx: 7,
            mass: 100.0,
            temperature,
            surface_area: 2.0,
            thickness: 0.5,
            ground_transfer_scale: 0.5,
            element_idx: 5,
            pad: [0; 2],
        }
    }

    #[test]
    fn register_derives_fields_from_message() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_elem_table();
        let mut mgr = ElementChunkManager::new();
        // 6×6 sim（内部 4×4）：game cell 5 = row1,col1 → sim (2,2) = 2×6+2 = 14
        let msg = add_msg(5, 300.0);
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
        // ModifyEnergy：ΔT = deltaKJ/heatCapacity = 200/200 = +1℃
        assert!(mgr.modify_energy(h, 200.0));
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

    #[test]
    fn exchange_heat_energy_with_world_exchanges_cell_and_chunk() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_elem_table();
        let mut sd = make_sim();
        // 格 14：水 300°C mass 100 ins 255（HC=200）；碎片：400°C HC=200 TC=4，factor=1000
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
            max_energy_transfer_scale_factor: 1000.0,
            ..Default::default()
        };
        let heat = exchange_heat_energy_with_world(&mut sd, &mut chunk, 14, 1.0, 0.2);
        assert!(heat > 0.0, "碎片（热）应失热给格子");
        assert!(chunk.temperature < 400.0, "碎片应降温，got {}", chunk.temperature);
        unsafe {
            assert!(
                (*sd.updated_cells.ptr).temperature.get(14) > 300.0,
                "格子应升温"
            );
        }
    }

    #[test]
    fn exchange_heat_energy_with_world_gates() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_elem_table();
        let mut sd = make_sim();
        let mut chunk = ElementChunkData {
            cell: 14,
            temperature: 400.0,
            heat_capacity: 200.0,
            thermal_conductivity: 4.0,
            max_energy_transfer_scale_factor: 1000.0,
            ..Default::default()
        };
        // mass=0 → 0
        assert_eq!(
            exchange_heat_energy_with_world(&mut sd, &mut chunk, 14, 1.0, 0.2),
            0.0
        );
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
        assert_eq!(
            exchange_heat_energy_with_world(&mut sd, &mut chunk, 14, 1.0, 0.2),
            0.0
        );
    }

    #[test]
    fn update_uses_adjuster_when_present() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_elem_table();
        let mut sd = make_sim();
        let mut mgr = ElementChunkManager::new();
        let h = mgr.add(6, &add_msg(4, 300.0)); // game 4 → sim 13
        mgr.modify_adjuster(h, 500.0, 100.0, 2.0);
        // adjuster 分支不依赖格子质量
        update_element_chunks_inner(&mut mgr, &mut sd, 0.2, 6);
        let d = mgr.record(h).unwrap();
        assert!(
            d.temperature > 300.0,
            "adjuster 应加热碎片，got {}",
            d.temperature
        );
        assert!(d.temperature < 500.0);
    }

    #[test]
    fn update_exchanges_with_solid_cell_below() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_elem_table();
        let mut sd = make_sim();
        let mut mgr = ElementChunkManager::new();
        // game 9 → sim (3,2) = 20；下方格 14 为固体砖 300°C
        let h = mgr.add(6, &add_msg(9, 5000.0));
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(14, 2);
            u.mass.set(14, 1000.0);
            u.temperature.set(14, 300.0);
            u.insulation.set(14, 255);
        }
        update_element_chunks_inner(&mut mgr, &mut sd, 0.2, 6);
        let d = mgr.record(h).unwrap();
        assert!(
            d.temperature < 5000.0,
            "碎片应降温给下方固体，got {}",
            d.temperature
        );
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.element_chunk_melted_info.len(), 0, "无相变");
        assert!(!mgr.output_snapshot().is_empty());
    }

    #[test]
    fn update_emits_melted_event_on_phase_cross() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_elem_table(); // elem5 highTemp=5000 → >5003 触发
        let mut sd = make_sim();
        let mut mgr = ElementChunkManager::new();
        let h = mgr.add(6, &add_msg(4, 6000.0)); // game 4 → sim 13（无质量格 → 世界分支不换热）
        update_element_chunks_inner(&mut mgr, &mut sd, 0.2, 6);
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.element_chunk_melted_info.len(), 1);
        assert_eq!(events.element_chunk_melted_info.get(0).handle.value, h);
    }
}
