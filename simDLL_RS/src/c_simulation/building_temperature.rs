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

use crate::a_framework::game_data::{BuildingTemperatureInfo, GameData, Handle};
use crate::a_framework::sim_events::AddBuildingHeatExchangeMsg;
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
        }
    }
}

/// 建筑温度管理器（全局单例，非 SimData 成员；仅在 sim 线程访问）。
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
        let record = BuildingTemperatureRecord {
            elem_idx: msg.elem_idx,
            mass: msg.mass,
            temperature: msg.temperature,
            thermal_conductivity: msg.thermal_conductivity,
            overheat_temperature: msg.overheat_temperature,
            operating_kilowatts: msg.operating_kilowatts,
            min_x: msg.min_x,
            min_y: msg.min_y,
            max_x: msg.max_x,
            max_y: msg.max_y,
            callback_idx: msg.callback_idx,
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

    /// 更新记录（原版 ModifyBuildingHeatExchange 处理；msg.callback_idx 即 handle）。
    pub fn modify(&mut self, handle: i32, msg: &AddBuildingHeatExchangeMsg) -> bool {
        let Some(record) = self.get_mut(handle) else {
            tracing::warn!(
                handle,
                "BuildingTemperatureManager::modify: bad handle"
            );
            return false;
        };
        record.elem_idx = msg.elem_idx;
        record.mass = msg.mass;
        record.temperature = msg.temperature;
        record.thermal_conductivity = msg.thermal_conductivity;
        record.overheat_temperature = msg.overheat_temperature;
        record.operating_kilowatts = msg.operating_kilowatts;
        record.min_x = msg.min_x;
        record.min_y = msg.min_y;
        record.max_x = msg.max_x;
        record.max_y = msg.max_y;
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

/// 处理 ModifyBuildingHeatExchange 消息。
pub fn modify_building(handle: i32, msg: &AddBuildingHeatExchangeMsg) -> bool {
    BUILDING_TEMPERATURE_MANAGER.lock().modify(handle, msg)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::game_data::GameData;
    use crate::a_framework::sim_events::RemoveBuildingHeatExchangeMsg;
    use crate::LIB_TESTS_LOCK;
    use std::mem::size_of;

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
        let mut mgr = BuildingTemperatureManager::new();
        let h0 = mgr.add(&add_msg(10, 300.0));
        let h1 = mgr.add(&add_msg(11, 310.0));
        assert_eq!(h0, 0);
        assert_eq!(h1, 1);
        assert_eq!(mgr.get_mut(h0).unwrap().temperature, 300.0);
        assert_eq!(mgr.get_mut(h1).unwrap().callback_idx, 11);
    }

    #[test]
    fn modify_updates_record() {
        let mut mgr = BuildingTemperatureManager::new();
        let h = mgr.add(&add_msg(10, 300.0));
        assert!(mgr.modify(h, &add_msg(10, 350.0)));
        assert_eq!(mgr.get_mut(h).unwrap().temperature, 350.0);
    }

    #[test]
    fn remove_frees_and_reuses_with_version_bump() {
        let mut mgr = BuildingTemperatureManager::new();
        let h0 = mgr.add(&add_msg(10, 300.0));
        let h1 = mgr.add(&add_msg(11, 310.0));
        assert!(mgr.remove(h0));
        assert!(mgr.get_mut(h0).is_none());
        assert!(mgr.get_mut(h1).is_some());
        // 复用 index 0，版本递增
        let h2 = mgr.add(&add_msg(12, 320.0));
        assert_eq!(h2 & 0xffffff, 0);
        assert_ne!(h2, h0);
        assert_eq!((h2 >> 24) & 0xff, 1);
        assert!(mgr.get_mut(h2).is_some());
        // 旧句柄仍无效
        assert!(mgr.get_mut(h0).is_none());
    }

    #[test]
    fn write_output_is_indexed_by_handle() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
}
