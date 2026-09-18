//! BuildingTemperatureManager —— 建筑温度句柄管理器
//! ============================================================
//! 对照原版 SimDLL 的 BuildingManager / buildingHeatExchangeMessages 处理链：
//!
//! C# StructureTemperature.SimRegister（StructureTemperatureComponents.cs）在建筑
//! OnSpawn 时发送 AddBuildingHeatExchange（0x67A75D28，44B，带 callbackIdx），sim 分配
//! 建筑温度句柄后经 componentStateChangedMessages 回调 C#（OnSimRegistered），C# 随即
//! 触发管道入网（Conduit.OnStructureTemperatureRegistered → AddToNetworks）。
//!
//! 本模块实现：
//! - 版本化句柄分配（handle = (version<<24)|index，槽位复用递增版本）
//! - Add/Modify/Remove 记录维护（帧处理器逐帧消费 buildingHeatExchangeMessages）
//! - 每帧输出 building_temperature_info（handle + temperature，按 handle index 索引），
//!   经 GDU buildingTemperatures 供 C# 读取（StructureTemperature 温度权威 + 管道换热输入）
//!
//! 换热本体（建筑 ↔ 周围格）与过热/熔化检测留待后续；本阶段目标是打通
//! "建筑注册 → 回调 C# → 管道入网" 链路，使 ConduitFlow 能注册管道温度句柄。

use crate::a_framework::game_data::{BuildingTemperatureInfo, GameData, Handle, MeltedInfo};
use crate::a_framework::sim_data::SimData;
use crate::a_framework::sim_events::AddBuildingHeatExchangeMsg;
use crate::a_framework::sim_events::SimEvents;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// 单条建筑温度记录（对应原版 BuildingTemperatureManager::Data 的核心字段）。
#[derive(Clone, Copy, Debug)]
pub struct BuildingTemperatureRecord {
    pub elem_idx: u16,
    pub mass: f32,
    pub temperature: f32,
    pub thermal_conductivity: f32,
    pub overheat_temperature: f32,
    pub operating_kilowatts: f32,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub callback_idx: i32,
    pub total_heat_capacity: f32,
    pub per_cell_heat_capacity: f32,
    pub high_temp: f32,
    pub sim_min_x: i32,
    pub sim_min_y: i32,
    pub sim_max_x: i32,
    pub sim_max_y: i32,
}

impl Default for BuildingTemperatureRecord {
    fn default() -> Self {
        Self {
            elem_idx: 0xffff,
            mass: 0.0,
            temperature: 0.0,
            thermal_conductivity: 0.0,
            overheat_temperature: 0.0,
            operating_kilowatts: 0.0,
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
            callback_idx: -1,
            total_heat_capacity: 0.0,
            per_cell_heat_capacity: 0.0,
            high_temp: 0.0,
            sim_min_x: 0,
            sim_min_y: 0,
            sim_max_x: 0,
            sim_max_y: 0,
        }
    }
}

/// 建筑温度管理器（全局单例，非 SimData 成员；仅在 sim 线程访问）。
/// 由 Add/Modify 消息构建记录并计算派生字段（原版 InitializeFromMessageData L137088-137186）。
/// 元素表缺失/面积<1/温度越界时返回 None（原版断言/崩溃，本项目转日志 + 拒绝）。
pub fn build_record_from_msg(msg: &AddBuildingHeatExchangeMsg) -> Option<BuildingTemperatureRecord> {
    let etd =
        crate::b_elements::elements_table::get_element_temperature_data(msg.elem_idx)?;
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

pub struct BuildingTemperatureManager {
    /// 数据槽（非压缩：释放只标记不复位）
    records: Vec<BuildingTemperatureRecord>,
    /// handle index → record index；-1 = 空闲槽
    items: Vec<i32>,
    /// handle index → version（8bit）
    versions: Vec<u8>,
    /// 可复用的完整 handle（版本已递增）
    free_handles: Vec<i32>,
}

impl BuildingTemperatureManager {
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

    /// 分配句柄并存入记录（原版 AddBuildingHeatExchange 处理）。
    pub fn add(&mut self, msg: &AddBuildingHeatExchangeMsg) -> i32 {
        let Some(record) = build_record_from_msg(msg) else {
            return -1;
        };
        let record_index = self.records.len();
        let handle = if let Some(h) = self.free_handles.pop() {
            let index = (h & 0xffffff) as usize;
            debug_assert!(index < self.items.len());
            self.items[index] = record_index as i32;
            h
        } else {
            let index = self.items.len();
            self.items.push(record_index as i32);
            self.versions.push(0);
            index as i32
        };
        self.records.push(record);
        handle
    }

    /// 校验 handle 并返回记录可变引用。
    fn get_mut(&mut self, handle: i32) -> Option<&mut BuildingTemperatureRecord> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let record_index = self.items[index];
        if record_index < 0 {
            return None;
        }
        self.records.get_mut(record_index as usize)
    }

    /// 校验 handle 并返回记录只读引用（测试/只读场景）。
    pub fn record(&self, handle: i32) -> Option<&BuildingTemperatureRecord> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let record_index = self.items[index];
        if record_index < 0 {
            return None;
        }
        self.records.get(record_index as usize)
    }

    /// 更新记录（原版 ModifyBuildingHeatExchange 处理；msg.callback_idx 即 handle）。
    pub fn modify(&mut self, handle: i32, msg: &AddBuildingHeatExchangeMsg) -> bool {
        let Some(new_record) = build_record_from_msg(msg) else {
            return false;
        };
        let Some(record) = self.get_mut(handle) else {
            tracing::warn!(
                handle,
                "BuildingTemperatureManager::modify: bad handle"
            );
            return false;
        };
        // 原版 Modify 消息 data 无 callbackIdx 字段，记录保留 Add 时的 callback（L137245 写回不含 callback）。
        let callback_idx = record.callback_idx;
        *record = new_record;
        record.callback_idx = callback_idx;
        true
    }

    /// 写建筑温度记录（原版 ChangeBuildingTemperature L136853 的记录写路径）。
    /// temperatureInfo 由每帧 write_building_temperature_info 重建，行为等价。
    pub fn set_temperature(&mut self, handle: i32, temperature: f32) -> bool {
        let Some(record) = self.get_mut(handle) else {
            tracing::warn!(
                handle,
                "BuildingTemperatureManager::set_temperature: bad handle"
            );
            return false;
        };
        record.temperature = temperature;
        true
    }

    /// ModifyBuildingEnergy 消费（原版 L115484-115540）。
    /// 返回 false 仅表示消息被跳过（坏句柄或钳制范围越出 [0,10000]）；
    /// 非法终温时原版仅告警不写回（消息已消费，返回 true）。
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
            } else {
                record.temperature = f2;
            }
            return true;
        }
        false
    }

    /// Modify + 过热事件（原版 L137187-137203）：old≥overheat 且 new<overheat →
    /// 推 `buildingNoLongerOverheatedInfo`（条件是 ≥，事件是 no-longer-overheated）。
    pub fn modify_with_events(
        &mut self,
        handle: i32,
        msg: &AddBuildingHeatExchangeMsg,
        events: &mut SimEvents,
    ) -> bool {
        let Some(new_record) = build_record_from_msg(msg) else {
            return false;
        };
        let Some(record) = self.get_mut(handle) else {
            return false;
        };
        let old_temp = record.temperature;
        let old_overheat = record.overheat_temperature;
        let callback_idx = record.callback_idx;
        if old_temp >= old_overheat && new_record.temperature < old_overheat {
            events.building_no_longer_overheated_info.push(MeltedInfo {
                handle: Handle { value: handle },
            });
        }
        *record = new_record;
        record.callback_idx = callback_idx;
        true
    }

    /// 释放句柄（原版 RemoveBuildingHeatExchange 处理；swap-remove 数据槽）。
    pub fn remove(&mut self, handle: i32) -> bool {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return false;
        }
        let record_index = self.items[index];
        if record_index < 0 {
            return false;
        }
        // 版本 +1（8bit 回绕），句柄入 free list
        let new_version = version.wrapping_add(1);
        self.versions[index] = new_version;
        self.free_handles
            .push(((new_version as i32) << 24) | (index as i32 & 0xffffff));
        self.items[index] = -1;
        true
    }

    /// 每帧把建筑温度（handle + temperature，按 handle index 索引）写入 GameData，
    /// 供 GDU buildingTemperatures 暴露给 C#。空闲槽位写 (0, -1.0)
    /// （原版 Unregister L137245 写 0xbf800000 = -1.0f）。
    pub fn write_building_temperature_info(&self, game_data: &mut GameData) {
        game_data.building_temperature_info.clear_keep_capacity();
        for index in 0..self.items.len() {
            let record_index = self.items[index];
            let (handle_value, temperature) = if record_index >= 0 {
                let version = self.versions[index] as i32;
                let record = self.records[record_index as usize];
                (
                    (version << 24) | (index as i32 & 0xffffff),
                    record.temperature,
                )
            } else {
                (0, -1.0)
            };
            game_data.building_temperature_info.push(BuildingTemperatureInfo {
                handle: Handle {
                    value: handle_value,
                },
                temperature,
            });
        }
    }
}

impl Default for BuildingTemperatureManager {
    fn default() -> Self {
        Self::new()
    }
}

// ===== 全局单例 =====

static BUILDING_TEMPERATURE_MANAGER: Lazy<Mutex<BuildingTemperatureManager>> =
    Lazy::new(|| Mutex::new(BuildingTemperatureManager::new()));

/// 处理 AddBuildingHeatExchange 消息，返回分配的句柄。
pub fn add_building(msg: &AddBuildingHeatExchangeMsg) -> i32 {
    BUILDING_TEMPERATURE_MANAGER.lock().add(msg)
}

/// 只读访问建筑温度记录快照（供 BuildingToBuilding 等校验/读取）。
pub fn record(handle: i32) -> Option<BuildingTemperatureRecord> {
    BUILDING_TEMPERATURE_MANAGER.lock().record(handle).copied()
}

/// ChangeBuildingTemperature（原版 L136853）：写建筑温度记录。
pub fn change_building_temperature(handle: i32, temperature: f32) -> bool {
    BUILDING_TEMPERATURE_MANAGER
        .lock()
        .set_temperature(handle, temperature)
}

/// 处理 ModifyBuildingHeatExchange 消息。
pub fn modify_building(handle: i32, msg: &AddBuildingHeatExchangeMsg) -> bool {
    BUILDING_TEMPERATURE_MANAGER.lock().modify(handle, msg)
}

/// 处理 ModifyBuildingHeatExchange 消息（带 no-longer-overheated 事件）。
pub fn modify_building_with_events(
    handle: i32,
    msg: &AddBuildingHeatExchangeMsg,
    events: &mut SimEvents,
) -> bool {
    BUILDING_TEMPERATURE_MANAGER
        .lock()
        .modify_with_events(handle, msg, events)
}

/// 处理 ModifyBuildingEnergy 消息（建筑产热）。
pub fn modify_building_energy(handle: i32, delta_kj: f32, min_temp: f32, max_temp: f32) -> bool {
    BUILDING_TEMPERATURE_MANAGER
        .lock()
        .modify_building_energy(handle, delta_kj, min_temp, max_temp)
}

/// 处理 RemoveBuildingHeatExchange 消息。
pub fn remove_building(handle: i32) -> bool {
    BUILDING_TEMPERATURE_MANAGER.lock().remove(handle)
}

/// 每帧填充 GameData.building_temperature_info。
pub fn write_building_temperature_info(game_data: &mut GameData) {
    BUILDING_TEMPERATURE_MANAGER
        .lock()
        .write_building_temperature_info(game_data);
}

/// 清空全部状态（CleanUp 时调用）。
pub fn clear_building_temperature() {
    BUILDING_TEMPERATURE_MANAGER.lock().clear();
}

/// 测试/诊断：当前活跃建筑温度记录数。
pub fn building_temperature_count() -> usize {
    BUILDING_TEMPERATURE_MANAGER
        .lock()
        .items
        .iter()
        .filter(|&&ri| ri >= 0)
        .count()
}

/// BuildingHeatExchange::Update（原版 L137362-137633）。每子步由 `update_data` 末尾调用（dt=0.2）。
///
/// 逐建筑、逐格换热：mass>0 才参与（真空跳过）；k = f24×ΔT×元素TC×记录TC×insulation²
/// ×7.68935e-08×dt×buildingTemperatureScale；能量守恒交叉钳制；建筑终温 = Σk/(cellCount×perCellHC)
/// + bldgT，钳制 [min_seen,max_seen] 后 + dt×operatingKW/(cellCount×perCellHC)；
/// 推 buildingOverheatInfo / buildingNoLongerOverheatedInfo / buildingMeltedInfo。
pub fn update_building_heat_exchange(
    sim_data: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    if sim_data.updated_cells.ptr.is_null() || sim_data.sim_events.ptr.is_null() {
        return;
    }
    let mut mgr = BUILDING_TEMPERATURE_MANAGER.lock();
    let dt = 0.2f32;
    let scale = sim_data.debug_properties.building_temperature_scale;
    let width = sim_data.width as usize;

    for index in 0..mgr.items.len() {
        let record_index = mgr.items[index];
        if record_index < 0 {
            continue;
        }
        let handle = ((mgr.versions[index] as i32) << 24) | (index as i32 & 0xffffff);
        let rec = mgr.records[record_index as usize];
        // 原版 BuildingHeatExchange::Update L137416-137419：仅处理完全落在
        // region 矩形内的建筑（region.min <= simMin 且 simMax <= region.max）。
        // 旧实现无 bounds → 未发现世界/区域外建筑也被逐帧换热（★4）。
        if rec.sim_min_x < bounds.min_x as i32
            || rec.sim_min_y < bounds.min_y as i32
            || rec.sim_max_x > bounds.max_x as i32
            || rec.sim_max_y > bounds.max_y as i32
        {
            continue;
        }
        if rec.per_cell_heat_capacity <= 0.0 || rec.temperature < 0.0 {
            continue;
        }
        let old_temperature = rec.temperature;
        let bldg_t = rec.temperature;
        let per_cell_hc = rec.per_cell_heat_capacity;
        let record_tc = rec.thermal_conductivity;
        let operating_kw = rec.operating_kilowatts;
        let overheat = rec.overheat_temperature;
        let high_temp = rec.high_temp;
        let cell_count =
            (rec.sim_max_y - rec.sim_min_y) * (rec.sim_max_x - rec.sim_min_x);
        if cell_count < 1 {
            continue;
        }
        let cell_count_f = cell_count as f32;
        let mut min_seen = bldg_t;
        let mut max_seen = bldg_t;
        let mut sum_k = 0.0f32;

        for row in rec.sim_min_y..rec.sim_max_y {
            for col in rec.sim_min_x..rec.sim_max_x {
                let cell = row as usize * width + col as usize;
                let (cell_mass, cell_temp, insul, etd) = {
                    let u = unsafe { &*sim_data.updated_cells.ptr };
                    if cell >= u.mass.len() {
                        continue;
                    }
                    let m = u.mass.get(cell);
                    if m <= 0.0 {
                        continue;
                    }
                    let t = u.temperature.get(cell);
                    let elem = u.element_idx.get(cell);
                    let Some(etd) =
                        crate::b_elements::elements_table::get_element_temperature_data(elem)
                    else {
                        continue;
                    };
                    (m, t, u.insulation.get(cell), etd)
                };
                let cell_hc = cell_mass * etd.specific_heat_capacity;
                if cell_hc <= 0.0 {
                    continue;
                }
                let f24 = if cell_temp < bldg_t { per_cell_hc } else { cell_hc };
                let insul_f = insul as f32;
                let mut k = f24 * (cell_temp - bldg_t) * etd.thermal_conductivity
                    * record_tc
                    * insul_f
                    * insul_f
                    * 7.68935e-08
                    * dt
                    * scale;
                let low = bldg_t.min(cell_temp);
                let high = bldg_t.max(cell_temp);
                let mut new_cell_t = (cell_temp - k / cell_hc).max(low).min(high);
                // 原版交叉分支不重算 new_bldg_t（L137451-137458 仅更新 k 与 new_cell_t；
                // 审查 Minor ③，2026-08-03 修正：删除多余重算）。
                let new_bldg_t = (k / per_cell_hc + bldg_t).max(low).min(high);
                if (new_bldg_t - new_cell_t) * (bldg_t - cell_temp) < 0.0 {
                    // 交叉：能量守恒均衡（原版 L137451-137458）
                    let inv = 1.0 / (per_cell_hc + cell_hc);
                    let balanced =
                        (per_cell_hc * bldg_t + cell_hc * cell_temp) * inv;
                    let balanced = balanced.max(low).min(high);
                    k = (balanced - bldg_t) * per_cell_hc;
                    new_cell_t = balanced;
                }
                // 原版 L35649-35655：`fVar25 < fVar9 || fVar24 < fVar25` 严格越界 →
                // 仅上报并**跳过写入 / DoStateTransition / sum_k**（写与累加在 else 分支内）。
                // 注意：NaN 时两个严格比较均为 false → 走 else 照写（与原版一致，
                // 不能用 `!(low <= x && x <= high)`——那会把 NaN 判为越界而跳过，产生分歧）。
                if new_cell_t < low || high < new_cell_t {
                    tracing::error!(
                        cell,
                        new_cell_t,
                        low,
                        high,
                        "BuildingHeatExchange has invalid values"
                    );
                    continue;
                }
                if !(0.0 <= new_cell_t && new_cell_t <= 10000.0) {
                    tracing::error!(
                        cell,
                        new_cell_t,
                        "BuildingHeatExchange has generated invalid final temperature"
                    );
                }
                unsafe {
                    (*sim_data.updated_cells.ptr).temperature.set(cell, new_cell_t);
                }
                crate::c2_physics::temperature::do_state_transition(sim_data, cell, &etd);
                sum_k += k;
                max_seen = max_seen.max(high);
                min_seen = min_seen.min(low);
            }
        }

        let inv_count_hc = 1.0 / (cell_count_f * per_cell_hc);
        let mut final_t = inv_count_hc * sum_k + bldg_t;
        final_t = final_t.min(max_seen);
        final_t = final_t.max(min_seen);
        final_t = final_t + dt * operating_kw * inv_count_hc;
        if !(0.0 <= final_t && final_t <= 10000.0) {
            tracing::error!(
                handle,
                final_t,
                "Assert failed: 0 <= final_temperature <= SIM_MAX_TEMPERATURE"
            );
        }
        {
            let events = unsafe { &mut *sim_data.sim_events.ptr };
            if overheat <= final_t {
                events.building_overheat_info.push(MeltedInfo {
                    handle: Handle { value: handle },
                });
            }
            if overheat <= old_temperature && final_t < overheat {
                events.building_no_longer_overheated_info.push(MeltedInfo {
                    handle: Handle { value: handle },
                });
            }
            if high_temp <= final_t {
                events.building_melted_info.push(MeltedInfo {
                    handle: Handle { value: handle },
                });
            }
        }
        mgr.records[record_index as usize].temperature = final_t;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::game_data::GameData;
    use crate::a_framework::sim_data::SimData;
    use crate::a_framework::sim_events::RemoveBuildingHeatExchangeMsg;
    use crate::a_framework::sim_events::SimEvents;
    use crate::b_elements::element::ElementTemperatureData;
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::LIB_TESTS_LOCK;
    use std::mem::size_of;

    /// 建最小元素表：elem5 液态 SHC=2.0、TC=4.0、highTemp=5000、lowTemp=200。
    /// 调用方需持 LIB_TESTS_LOCK。
    fn init_temp_table() {
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
        table.elements.resize(6, crate::b_elements::element::Element::default());
        table.temperature_data.resize(6, ElementTemperatureData::default());
        table.temperature_data[5] = etd;
    }

    fn add_msg(callback_idx: i32, temp: f32) -> AddBuildingHeatExchangeMsg {
        AddBuildingHeatExchangeMsg {
            callback_idx,
            elem_idx: 5,
            pad0: 0,
            pad1: 0,
            mass: 100.0,
            temperature: temp,
            thermal_conductivity: 1.0,
            overheat_temperature: 2000.0,
            operating_kilowatts: 0.0,
            min_x: 1,
            min_y: 2,
            max_x: 2,
            max_y: 3,
        }
    }

    #[test]
    fn message_sizes_match_csharp() {
        assert_eq!(size_of::<AddBuildingHeatExchangeMsg>(), 44);
        assert_eq!(size_of::<RemoveBuildingHeatExchangeMsg>(), 8);
    }

    #[test]
    fn add_allocates_versioned_handles() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut mgr = BuildingTemperatureManager::new();
        let h0 = mgr.add(&add_msg(10, 300.0));
        let h1 = mgr.add(&add_msg(11, 310.0));
        assert_eq!(h0, 0);
        assert_eq!(h1, 1);
        assert_eq!(mgr.record(h0).unwrap().temperature, 300.0);
        assert_eq!(mgr.record(h1).unwrap().callback_idx, 11);
    }

    #[test]
    fn modify_updates_record() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut mgr = BuildingTemperatureManager::new();
        let h = mgr.add(&add_msg(10, 300.0));
        assert!(mgr.modify(h, &add_msg(10, 350.0)));
        assert_eq!(mgr.record(h).unwrap().temperature, 350.0);
    }

    #[test]
    fn remove_frees_and_reuses_with_version_bump() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut mgr = BuildingTemperatureManager::new();
        let h0 = mgr.add(&add_msg(10, 300.0));
        let h1 = mgr.add(&add_msg(11, 310.0));
        assert!(mgr.remove(h0));
        assert!(mgr.record(h0).is_none());
        assert!(mgr.record(h1).is_some());
        // 复用 index 0，版本递增
        let h2 = mgr.add(&add_msg(12, 320.0));
        assert_eq!(h2 & 0xffffff, 0);
        assert_ne!(h2, h0);
        assert_eq!((h2 >> 24) & 0xff, 1);
        assert!(mgr.record(h2).is_some());
        // 旧句柄仍无效
        assert!(mgr.record(h0).is_none());
    }

    #[test]
    fn write_output_is_indexed_by_handle() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut mgr = BuildingTemperatureManager::new();
        let h0 = mgr.add(&add_msg(10, 300.0));
        let h1 = mgr.add(&add_msg(11, 310.0));
        mgr.remove(h0);
        mgr.remove(h1);
        mgr.add(&add_msg(12, 320.0)); // 复用 index 1
        mgr.add(&add_msg(13, 330.0)); // 新 index 2

        let mut gd: GameData = unsafe { std::mem::zeroed() };
        mgr.write_building_temperature_info(&mut gd);
        let slice = gd.building_temperature_info.as_slice();
        assert_eq!(slice.len(), 2);
        assert_eq!(slice[0].handle.value & 0xffffff, 0);
        assert_eq!((slice[0].handle.value >> 24) & 0xff, 1);
        assert_eq!(slice[0].temperature, 330.0);
        assert_eq!(slice[1].handle.value & 0xffffff, 1);
        assert_eq!(slice[1].temperature, 320.0);
    }

    #[test]
    fn write_output_freed_slot_is_minus_one() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut mgr = BuildingTemperatureManager::new();
        let h0 = mgr.add(&add_msg(10, 300.0));
        mgr.remove(h0);
        let mut gd: GameData = unsafe { std::mem::zeroed() };
        mgr.write_building_temperature_info(&mut gd);
        let slice = gd.building_temperature_info.as_slice();
        assert_eq!(slice.len(), 1);
        // 原版 Unregister 把空闲槽温度写 -1.0f（L137245 0xbf800000）
        assert_eq!(slice[0].temperature, -1.0);
    }

    #[test]
    fn build_record_derives_fields_from_message() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let msg = add_msg(10, 300.0); // min(1,2) max(2,3) → 面积 1，mass=100
        let r = build_record_from_msg(&msg).expect("valid building");
        // totalHC = SHC×mass = 2×100 = 200；perCellHC = 200/1 = 200
        assert_eq!(r.total_heat_capacity, 200.0);
        assert_eq!(r.per_cell_heat_capacity, 200.0);
        // recordTC = 元素TC×消息TC = 4×1 = 4
        assert_eq!(r.thermal_conductivity, 4.0);
        assert_eq!(r.high_temp, 5000.0);
        assert_eq!((r.sim_min_x, r.sim_min_y), (2, 3));
        assert_eq!((r.sim_max_x, r.sim_max_y), (3, 4));
        assert_eq!(r.callback_idx, 10);
    }

    #[test]
    fn build_record_rejects_zero_area_and_bad_temperature() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut zero = add_msg(10, 300.0);
        zero.min_x = 2;
        zero.max_x = 2; // 面积 0
        assert!(build_record_from_msg(&zero).is_none());
        let mut hot = add_msg(10, 20000.0);
        assert!(build_record_from_msg(&hot).is_none());
        let mut bad_elem = add_msg(10, 300.0);
        bad_elem.elem_idx = 0xffff;
        assert!(build_record_from_msg(&bad_elem).is_none());
    }

    #[test]
    fn modify_building_energy_applies_delta_and_clamps() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut mgr = BuildingTemperatureManager::new();
        let mut msg = add_msg(10, 300.0);
        msg.mass = 100.0;
        msg.min_x = 0;
        msg.min_y = 0;
        msg.max_x = 2;
        msg.max_y = 2; // 面积 4 → perCellHC = 2×100/4 = 50
        let h = mgr.add(&msg);
        // delta = perCellHC×面积×1℃ = 50×4×1 = 200 → +1℃
        assert!(mgr.modify_building_energy(h, 200.0, 0.0, 10000.0));
        assert!((mgr.record(h).unwrap().temperature - 301.0).abs() < 1e-3);
        // 钳制到 msg.max=350
        assert!(mgr.modify_building_energy(h, 1e6, 0.0, 350.0));
        assert!((mgr.record(h).unwrap().temperature - 350.0).abs() < 1e-3);
        // 坏句柄（版本不符）
        assert!(!mgr.modify_building_energy(h + (1 << 24), 200.0, 0.0, 10000.0));
    }

    #[test]
    fn modify_building_energy_rejects_invalid_final_temperature() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut mgr = BuildingTemperatureManager::new();
        let h = mgr.add(&add_msg(10, 300.0)); // 面积 1，perCellHC = 200
        // 钳到 [0,10000] 上界 → 非法温度 → 不写回
        assert!(mgr.modify_building_energy(h, 1e10, 0.0, 10000.0));
        assert!((mgr.record(h).unwrap().temperature - 300.0).abs() < 1e-3);
    }

    #[test]
    fn modify_pushes_no_longer_overheated_at_boundary() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut events = SimEvents::default();
        let mut mgr = BuildingTemperatureManager::new();
        let msg = add_msg(10, 2000.0); // overheat = 2000，温度 = 2000
        let h = mgr.add(&msg);
        // old=2000 >= overheat=2000，new=1999 < 2000 → 事件
        let m2 = add_msg(10, 1999.0);
        assert!(mgr.modify_with_events(h, &m2, &mut events));
        assert_eq!(events.building_no_longer_overheated_info.len(), 1);
        assert_eq!(
            events.building_no_longer_overheated_info.get(0).handle.value,
            h
        );
        // old=1999 < overheat → 无事件
        let m3 = add_msg(10, 1800.0);
        assert!(mgr.modify_with_events(h, &m3, &mut events));
        assert_eq!(events.building_no_longer_overheated_info.len(), 1);
    }

    #[test]
    fn change_building_temperature_updates_record() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let h = add_building(&add_msg(10, 300.0));
        assert!(change_building_temperature(h, 350.0));
        assert_eq!(record(h).unwrap().temperature, 350.0);
        // 坏句柄（版本不符）→ 拒绝
        assert!(!change_building_temperature(h + (1 << 24), 350.0));
    }

    fn make_exchange_sim() -> SimData {
        let mut sd = crate::a_framework::sim_data::SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd.debug_properties.building_temperature_scale = 1.0;
        sd
    }

    /// ★4 回归：组件按区域分派——完全落在 region 内的建筑换热，区域外建筑跳过。
    /// 原版 BuildingHeatExchange::Update L137416-137419（region.min <= simMin 且
    /// simMax <= region.max）。
    #[test]
    fn building_exchange_only_processes_in_region() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table(); // elem5：SHC=2、TC=4、highTemp=5000
        let mut sd = make_exchange_sim();
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            // 建筑 A（game (1,2)-(2,3)）→ sim (2,3)-(3,4)：cell 20/21/26/27
            // 建筑 B（game (3,2)-(4,3)）→ sim (4,3)-(5,4)：cell 22/23/28/29
            for c in [20usize, 21, 26, 27, 22, 23, 28, 29] {
                u.element_idx.set(c, 5);
                u.mass.set(c, 100.0);
                u.temperature.set(c, 300.0);
                u.insulation.set(c, 1);
            }
        }
        let ha = add_building(&add_msg(10, 2000.0)); // (1,2)-(2,3)
        let mut msg_b = add_msg(11, 2000.0);
        msg_b.min_x = 3;
        msg_b.max_x = 4;
        let hb = add_building(&msg_b); // (3,2)-(4,3)
        // region（sim 坐标）：min(2,3)-max(4,6)（排他）→ 建筑 A（simMaxX=3）完全在内，
        // 建筑 B（simMinX=4）在右边界外
        let bounds = crate::d1_activity::RegionBounds {
            min_x: 2,
            min_y: 3,
            max_x: 4,
            max_y: 6,
        };
        update_building_heat_exchange(&mut sd, bounds);
        let ta = BUILDING_TEMPERATURE_MANAGER.lock().record(ha).unwrap().temperature;
        let tb = BUILDING_TEMPERATURE_MANAGER.lock().record(hb).unwrap().temperature;
        assert!(ta < 2000.0, "区域内建筑 A 应换热降温，got {ta}");
        assert!(
            (tb - 2000.0).abs() < 1e-3,
            "区域外建筑 B 不应被处理，got {tb}"
        );
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(u.temperature.get(20) > 300.0, "A 覆盖格应升温");
            assert!(
                (u.temperature.get(22) - 300.0).abs() < 1e-3,
                "B 覆盖格不应换热，got {}",
                u.temperature.get(22)
            );
        }
        remove_building(ha);
        remove_building(hb);
    }

    #[test]
    fn building_exchange_skips_vacuum_and_exchanges_with_matter_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table(); // elem5：SHC=2、TC=4、highTemp=5000
        let mut sd = make_exchange_sim();
        // 建筑 sim 范围 (2,3)-(3,5)：cell20(row3,col2) 有质量、cell26(row4,col2) 真空
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(20, 5);
            u.mass.set(20, 100.0);
            u.temperature.set(20, 300.0);
            u.insulation.set(20, 1);
            u.element_idx.set(26, 0);
            u.mass.set(26, 0.0);
            u.temperature.set(26, 0.0);
            u.insulation.set(26, 0);
        }
        let mut msg = add_msg(10, 2000.0); // 大温差让单子步换热在 f32 下可观测
        msg.min_x = 1;
        msg.min_y = 2;
        msg.max_x = 2;
        msg.max_y = 4; // 面积 1×2 → perCellHC = 200/2 = 100
        let h = add_building(&msg);
        let b = crate::d1_activity::full_grid_bounds(&sd);
        update_building_heat_exchange(&mut sd, b);
        let u = unsafe { &*sd.updated_cells.ptr };
        // 真空格不换热
        assert_eq!(u.temperature.get(26), 0.0);
        // 有质量格从建筑获得热量（cellT 300 < bldgT 2000 → 升温）
        assert!(u.temperature.get(20) > 300.0, "cell20 应升温，got {}", u.temperature.get(20));
        // 建筑温度下降
        assert!(
            BUILDING_TEMPERATURE_MANAGER.lock().record(h).unwrap().temperature < 2000.0
        );
        remove_building(h);
    }

    #[test]
    fn building_exchange_emits_overheat_and_melted_events() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut sd = make_exchange_sim();
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(20, 5); // 默认建筑覆盖 cell20(row3,col2)
            u.mass.set(20, 100.0);
            u.temperature.set(20, 2000.0);
            u.insulation.set(20, 1);
        }
        let mut msg = add_msg(10, 2000.0); // overheat=2000，温度=2000
        msg.operating_kilowatts = 2000.0;
        let h = add_building(&msg);
        let b = crate::d1_activity::full_grid_bounds(&sd);
        update_building_heat_exchange(&mut sd, b);
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.building_overheat_info.len(), 1, "自热使建筑温度超过 overheat");
        assert_eq!(
            events.building_overheat_info.get(0).handle.value,
            h
        );
        // 建筑终温 = 2000 + 0.2×2000/(1×perCellHC=200) = 2002，highTemp=5000 → 无 melted
        assert_eq!(events.building_melted_info.len(), 0);
        assert!(
            (BUILDING_TEMPERATURE_MANAGER.lock().record(h).unwrap().temperature - 2002.0).abs() < 0.01
        );
        remove_building(h);
    }

    #[test]
    fn building_exchange_emits_no_longer_overheated_when_cooled() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_temp_table();
        let mut sd = make_exchange_sim();
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(20, 5);
            u.mass.set(20, 100.0);
            u.temperature.set(20, 100.0);
            u.insulation.set(20, 1);
        }
        let h = add_building(&add_msg(10, 2000.0)); // overheat=2000，建筑 2000℃，周围 100℃
        let b = crate::d1_activity::full_grid_bounds(&sd);
        update_building_heat_exchange(&mut sd, b);
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.building_no_longer_overheated_info.len(), 1);
        assert!(BUILDING_TEMPERATURE_MANAGER.lock().record(h).unwrap().temperature < 2000.0);
        remove_building(h);
    }
}
