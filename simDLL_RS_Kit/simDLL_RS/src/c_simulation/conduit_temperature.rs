//! ConduitTemperatureManager —— 管道温度管理器
//! ============================================================
//! 严格对照原版 SimDLL_Source.c L94814-95886（conduittemperaturemanager.cpp）。
//!
//! C# ConduitTemperatureManager（Assembly-CSharp\ConduitTemperatureManager.cs）通过 7 个
//! FFI 函数驱动本管理器：
//! - Initialize / Shutdown / Clear：生命周期
//! - Add：为每根管道分配温度句柄（管道建成/加载时）
//! - Set：每帧把管道内容温度/质量/元素写回 sim（ConduitFlow EndFrame）
//! - Remove：管道拆除时注销句柄（延迟两轮释放）
//! - Update：每 200ms 计算管道内容物 ↔ 管道结构的换热，返回温度数组 + 冻结/熔化句柄
//!
//! ## 句柄系统（原版 CompactedVector 语义）
//! `handle = (version << 24) | index`，version 为 8bit 复用计数。
//! - Add：优先复用 free list（已递增版本），否则追加新槽（版本 0）
//! - Remove：把句柄推进延迟释放双缓冲，两轮 sim 循环后（ReleaseQueuedHandles）
//!   版本 +1 并回收数据槽（swap-remove），防止遍历中复用导致状态错乱
//! - ReleaseQueuedHandles 由 sim 线程在每轮 Sim::Main 循环开头调用（L2113-2116）
//!
//! ## Update 换热（原版 L95549-95886）
//! 1. 有效导热系数：insulated → min(contentsTC, conduitTC)；否则 → avg
//! 2. 热流 = effectiveK × ΔT × 50.0（THERMAL_FLOW_SCALE）
//! 3. 能量 = 热流 × dt × 0.001（TIME_SCALE）
//! 4. 候选新温度钳制到 [min(src,dst), max(src,dst)] 防过冲
//! 5. 能量守恒：取 |Δcontents|×HC 与 |Δbuilding|×HC 的较小值
//! 6. 交叉检测：方向反转时用热容加权平均
//! 7. NaN 检测：回退原温度，能量归零
//! 8. 显著能量（>1e-6）→ 发 MODIFY_BUILDING_ENERGY（0xAF9B1296）16B 消息
//! 9. 相变检测：±3.0 滞回，超出 low/high 过渡温度 → frozen/melted 句柄
//!
//! ## 已确认的 C# 消费方式
//! - C# 自己的 `temperatures[]` 数组按 handle index 索引，Sim200ms 时被
//!   Update 返回的数组整体覆盖（Marshal.Copy）——因此 sim 侧温度是权威。
//! - Update 第二参数 `building_conductivity_data` 即 GDU 的
//!   `buildingTemperatures` 指针（Game.cs L1086），长度 = GDU.num_building_temperature_info。
//! - frozen/melted 数组内容为 handle index（int），C# 经 Sim.GetHandleIndex 读取。

use crate::a_framework::buffer::BinaryBufferReader;
use crate::a_framework::game_data::BuildingTemperatureInfo;
use crate::a_framework::message_handler::MessageType;
use crate::a_framework::sim_frame_manager::SimFrameManager;
use crate::b_elements::element::{ElementTemperatureData, INVALID_ELEMENT_INDEX};
use crate::b_elements::elements_table::{
    get_element_default_temperature, get_element_index_pub, get_element_temperature_data,
};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::panic::{catch_unwind, AssertUnwindSafe};

// ===== 常量（原版 conduitTemperatureManager 硬编码值）=====

/// 热流缩放系数（原版 L95699 `* 50.0`）。
const THERMAL_FLOW_SCALE: f32 = 50.0;

/// 时间缩放系数（原版 L95702 `* 0.001`）。
const TIME_SCALE: f32 = 0.001;

/// 相变滞回（原版 L95815-95816 `- 3.0` / `+ 3.0`）。
const PHASE_HYSTERESIS: f32 = 3.0;

/// 热容下限（原版 L95665-95666 `< 0.0001`），低于此值跳过换热。
const HEAT_CAPACITY_EPSILON: f32 = 0.0001;

/// 能量上报阈值（原版 L95803 `< 1e-06`）。
const ENERGY_REPORT_THRESHOLD: f32 = 1e-6;

/// 温度上限（原版 SIM_MAX_TEMPERATURE = 10000.0）。
const SIM_MAX_TEMPERATURE: f32 = 10000.0;

/// Handle index 有效上限（原版 GetData L95659 `< 0x100000`）。
const HANDLE_INDEX_MAX: usize = 0x100000;

/// ModifyBuildingEnergy 消息 ID（原版 -0x5064ed6a == 0xAF9B1296）。
const MSG_MODIFY_BUILDING_ENERGY: u32 = MessageType::ModifyBuildingEnergy as u32;

// ===== 数据结构 =====

/// 单条管道温度记录（原版 `ConduitTemperatureManager::Data`，0x24 = 36B）。
///
/// | 原版字段 | 类型 | Rust 字段 |
/// |------|------|------|
/// | temperature | f32 | temperature |
/// | thermalConductivity | f32 | thermal_conductivity |
/// | heatCapacity | f32 | heat_capacity |
/// | conduitBuildingTemperatureHandle | Handle(i32) | conduit_building_temperature_handle |
/// | conduitHeatCapacity | f32 | conduit_heat_capacity |
/// | conduitThermalConductivity | f32 | conduit_thermal_conductivity |
/// | conduitInsulated | bool | conduit_insulated |
/// | lowStateTransitionTemperature | f32 | low_state_transition_temperature |
/// | highStateTransitionTemperature | f32 | high_state_transition_temperature |
#[derive(Clone, Copy, Debug)]
pub struct ConduitTemperatureData {
    pub temperature: f32,
    /// 内容物导热系数（来自元素表）
    pub thermal_conductivity: f32,
    /// 内容物热容 = specificHeatCapacity × mass
    pub heat_capacity: f32,
    /// 管道结构的 BuildingManager 温度句柄（查 building_temps 用）
    pub conduit_building_temperature_handle: i32,
    /// 管道结构热容
    pub conduit_heat_capacity: f32,
    /// 管道结构导热系数
    pub conduit_thermal_conductivity: f32,
    /// 是否隔热（insulated → 取 min 导热系数，否则取平均）
    pub conduit_insulated: bool,
    /// 低温相变阈值（内容物温度 < low - 3.0 → frozen）
    pub low_state_transition_temperature: f32,
    /// 高温相变阈值（内容物温度 > high + 3.0 → melted）
    pub high_state_transition_temperature: f32,
}

impl Default for ConduitTemperatureData {
    fn default() -> Self {
        Self {
            temperature: 0.0,
            thermal_conductivity: 0.0,
            heat_capacity: 0.0,
            conduit_building_temperature_handle: -1,
            conduit_heat_capacity: 0.0,
            conduit_thermal_conductivity: 0.0,
            conduit_insulated: false,
            low_state_transition_temperature: 0.0,
            high_state_transition_temperature: f32::MAX,
        }
    }
}

/// Update 返回的指针结构（原版 L11251-11258，C# `[StructLayout(Pack=4)]`）。
#[repr(C, packed(4))]
pub struct ConduitTemperatureUpdateData {
    pub num_entries: c_int,
    pub temperatures: *const f32,
    pub num_frozen_handles: c_int,
    pub frozen_handles: *const c_int,
    pub num_melted_handles: c_int,
    pub melted_handles: *const c_int,
}

impl Default for ConduitTemperatureUpdateData {
    fn default() -> Self {
        Self {
            num_entries: 0,
            temperatures: std::ptr::null(),
            num_frozen_handles: 0,
            frozen_handles: std::ptr::null(),
            num_melted_handles: 0,
            melted_handles: std::ptr::null(),
        }
    }
}

/// 管道温度管理器（全局单例，非 SimData 成员，对应原版 gConduitTemperatureManager）。
pub struct ConduitTemperatureManager {
    /// 数据槽（swap-remove，与 data_handle_indices 平行）
    data: Vec<ConduitTemperatureData>,
    /// data[i] 对应的完整 handle
    data_handle_indices: Vec<i32>,
    /// handle index → data index
    items: Vec<i32>,
    /// handle index → version（8bit）
    versions: Vec<u8>,
    /// 可复用的完整 handle（版本已递增）
    free_handles: Vec<i32>,
    /// 延迟释放双缓冲：Remove 推进 [cur]，下一轮 Release 切换后回收 [cur^1]
    frame_delayed_released_handles: [Vec<i32>; 2],
    /// 当前释放缓冲索引（0 或 1）
    cur_release_list_idx: usize,
    /// 输出温度数组（按 handle index 索引，C# Marshal.Copy 整体读走）
    temperatures: Vec<f32>,
    /// 每帧清空重填：frozen 相变句柄（handle index）
    frozen_content_handles: Vec<c_int>,
    /// 每帧清空重填：melted 相变句柄（handle index）
    melted_content_handles: Vec<c_int>,
    /// Update 返回结构（指针供 C# 同步读取）
    update_data: ConduitTemperatureUpdateData,
    /// 本帧待发的 MODIFY_BUILDING_ENERGY 消息（handle, deltaKJ, minTemp, maxTemp）
    pending_building_energy: Vec<(i32, f32, f32, f32)>,
}

impl ConduitTemperatureManager {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            data_handle_indices: Vec::new(),
            items: Vec::new(),
            versions: Vec::new(),
            free_handles: Vec::new(),
            frame_delayed_released_handles: [Vec::new(), Vec::new()],
            cur_release_list_idx: 0,
            temperatures: Vec::new(),
            frozen_content_handles: Vec::new(),
            melted_content_handles: Vec::new(),
            update_data: ConduitTemperatureUpdateData::default(),
            pending_building_energy: Vec::new(),
        }
    }

    /// 清空全部状态（原版 Clear：回收 CompactedVector 的 5 个 vector）。
    pub fn clear(&mut self) {
        self.data.clear();
        self.data_handle_indices.clear();
        self.items.clear();
        self.versions.clear();
        self.free_handles.clear();
        self.frame_delayed_released_handles[0].clear();
        self.frame_delayed_released_handles[1].clear();
        self.cur_release_list_idx = 0;
        self.temperatures.clear();
        self.frozen_content_handles.clear();
        self.melted_content_handles.clear();
        self.update_data = ConduitTemperatureUpdateData::default();
        self.pending_building_energy.clear();
    }

    /// 校验 handle 并返回只读数据。
    pub fn get(&self, handle: i32) -> Option<&ConduitTemperatureData> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let data_index = self.items[index];
        if data_index < 0 {
            return None;
        }
        self.data.get(data_index as usize)
    }

    /// 校验 handle 并返回可变数据。
    fn get_mut(&mut self, handle: i32) -> Option<&mut ConduitTemperatureData> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let data_index = self.items[index];
        if data_index < 0 {
            return None;
        }
        self.data.get_mut(data_index as usize)
    }

    /// 分配句柄并存入数据（原版 Add L94979-95202）。
    ///
    /// free list 优先复用（版本已在释放时递增）；否则追加新槽（版本 0）。
    /// 返回 `handle = (version << 24) | index`。
    pub fn add(&mut self, data: ConduitTemperatureData) -> i32 {
        let data_index = self.data.len();
        let handle = if let Some(h) = self.free_handles.pop() {
            let index = (h & 0xffffff) as usize;
            debug_assert!(index < self.items.len());
            self.items[index] = data_index as i32;
            h
        } else {
            let index = self.items.len();
            self.items.push(data_index as i32);
            self.versions.push(0);
            index as i32
        };
        self.data.push(data);
        self.data_handle_indices.push(handle);
        handle
    }

    /// 注销句柄（原版 Remove L96244-96290）。
    ///
    /// 先把数据标记为失效（conduit 字段置 -1），再推进延迟释放缓冲；
    /// 真正的槽位回收在 sim 线程的 ReleaseQueuedHandles 中执行（两轮后）。
    pub fn remove(&mut self, handle: i32) -> bool {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return false;
        }
        let data_index = self.items[index];
        if data_index < 0 || (data_index as usize) >= self.data.len() {
            return false;
        }
        {
            let d = &mut self.data[data_index as usize];
            d.conduit_heat_capacity = -1.0;
            d.conduit_thermal_conductivity = -1.0;
            d.conduit_building_temperature_handle = -1;
        }
        self.frame_delayed_released_handles[self.cur_release_list_idx].push(handle);
        true
    }

    /// 回收延迟释放缓冲（原版 ReleaseQueuedHandles L95203-95278，sim 线程每轮调用）。
    ///
    /// 先切换缓冲索引，再回收上一轮切走的那一侧（保证 Remove 与真正释放之间
    /// 至少隔一轮，避免遍历中槽位被复用）。
    pub fn release_queued_handles(&mut self) {
        self.cur_release_list_idx ^= 1;
        let releases =
            std::mem::take(&mut self.frame_delayed_released_handles[self.cur_release_list_idx]);
        for handle in releases {
            self.actual_release(handle);
        }
    }

    /// 真正释放一个句柄：版本 +1、入 free list、swap-remove 数据槽。
    fn actual_release(&mut self, handle: i32) {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return;
        }
        // 版本 +1（8bit 回绕），新句柄入 free list（原版 L95232-95239）
        let new_version = version.wrapping_add(1);
        self.versions[index] = new_version;
        self.free_handles
            .push(((new_version as i32) << 24) | (index as i32 & 0xffffff));

        // swap-remove 数据槽（原版 L95240-95275）
        let data_index = self.items[index];
        self.items[index] = 0;
        if data_index < 0 || (data_index as usize) >= self.data.len() {
            return;
        }
        let data_index = data_index as usize;
        let last = self.data.len() - 1;
        if data_index != last {
            self.data[data_index] = self.data[last];
            let moved_handle = self.data_handle_indices[last];
            let moved_index = (moved_handle & 0xffffff) as usize;
            self.items[moved_index] = data_index as i32;
            self.data_handle_indices[data_index] = moved_handle;
        }
        self.data.pop();
        self.data_handle_indices.pop();
        if index < self.temperatures.len() {
            self.temperatures[index] = 0.0;
        }
    }

    /// 更新已有句柄的温度/质量/元素（原版 Set L95346-95454）。
    ///
    /// 按元素表重算 heatCapacity/thermalConductivity/low/high 过渡温度；
    /// handle 无效时仅记录警告（原版 KCrashReporterReportMessage，不崩溃）。
    pub fn set(
        &mut self,
        handle: i32,
        contents_temperature: f32,
        contents_mass: f32,
        contents_element_hash: i32,
    ) -> bool {
        let elem = match lookup_element_temperature(contents_element_hash) {
            Some(e) => e,
            None => {
                tracing::warn!(
                    handle,
                    element_hash = contents_element_hash,
                    "ConduitTemperatureManager::Set: unknown element"
                );
                return false;
            }
        };
        let entry = match self.get_mut(handle) {
            Some(e) => e,
            None => {
                tracing::warn!(
                    handle,
                    index = handle & 0xffffff,
                    version = (handle >> 24) & 0xff,
                    array_size = self.versions.len(),
                    "ConduitTemperatureManager::Set called with bad handle"
                );
                return false;
            }
        };
        let mut temp = contents_temperature;
        if temp > SIM_MAX_TEMPERATURE {
            if let Some(default_temp) = lookup_element_default_temperature(contents_element_hash) {
                temp = default_temp;
            }
        }
        entry.temperature = temp;
        entry.heat_capacity = elem.specific_heat_capacity * contents_mass;
        entry.thermal_conductivity = elem.thermal_conductivity;
        entry.low_state_transition_temperature = if elem.low_temp_transition_idx == 0xffff {
            0.0
        } else {
            elem.low_temp
        };
        entry.high_state_transition_temperature = if elem.high_temp_transition_idx == 0xffff {
            f32::MAX
        } else {
            elem.high_temp
        };
        true
    }

    /// 每帧更新所有管道换热（原版 Update L95456-95886）。
    ///
    /// - `dt`：C# 传入的时间步长（通常 0.2 或 0.0）
    /// - `building_temps`：BuildingTemperatureInfo 切片（GDU buildingTemperatures，
    ///   按 handle index 索引）。None/越界/热容不足 → 仅复制当前温度（不换热）。
    ///
    /// 返回 `&ConduitTemperatureUpdateData`（指针供 C# 读取）。
    pub fn update(
        &mut self,
        dt: f32,
        building_temps: Option<&[BuildingTemperatureInfo]>,
    ) -> &ConduitTemperatureUpdateData {
        // 原版 L95485-95487：清空每帧缓冲
        self.melted_content_handles.clear();
        self.frozen_content_handles.clear();
        self.pending_building_energy.clear();

        // 原版 L95488-95491：temperatures 对齐 handle index 空间（items 长度）
        if self.temperatures.len() < self.items.len() {
            self.temperatures.resize(self.items.len(), 0.0);
        }

        // 原版 L95492-95548 / L95549-95853：主循环
        for i in 0..self.data.len() {
            let handle = self.data_handle_indices[i];
            let handle_index = (handle & 0xffffff) as usize;
            let entry = &mut self.data[i];

            // 跳过条件（原版 L95504-95513）：
            // 建筑温度句柄 index 越界 / 内容物热容不足 / 结构热容不足。
            // 注意：dest_temp 用 entry.conduit_building_temperature_handle 索引
            // buildingTemps（原版 L95510-95519），不是 conduit 自身句柄索引——
            // 两个句柄空间独立分配，索引必然错位。
            let building_handle = entry.conduit_building_temperature_handle;
            let building_index = (building_handle & 0xffffff) as usize;
            let mut skip = building_index >= HANDLE_INDEX_MAX
                || entry.heat_capacity <= HEAT_CAPACITY_EPSILON
                || entry.conduit_heat_capacity <= HEAT_CAPACITY_EPSILON;
            if !skip {
                if let Some(bt) = building_temps {
                    if building_index >= bt.len() {
                        skip = true;
                    }
                } else {
                    skip = true;
                }
            }

            if skip {
                if handle_index < self.temperatures.len() {
                    self.temperatures[handle_index] = entry.temperature;
                }
                continue;
            }

            let bt = building_temps.unwrap();
            let dest_temp = bt[building_index].temperature;
            // 原版 L95515：结构温度 <= 0 → 无有效温度，仅复制
            if dest_temp <= 0.0 {
                if handle_index < self.temperatures.len() {
                    self.temperatures[handle_index] = entry.temperature;
                }
                continue;
            }

            let src_temp = entry.temperature;

            // 有效导热系数（原版 L95520-95524）
            let effective_k = if !entry.conduit_insulated {
                (entry.thermal_conductivity + entry.conduit_thermal_conductivity) * 0.5
            } else if entry.conduit_thermal_conductivity <= entry.thermal_conductivity {
                entry.conduit_thermal_conductivity
            } else {
                entry.thermal_conductivity
            };

            // 热流与能量（原版 L95525-95529）
            let heat_flow = effective_k * (src_temp - dest_temp) * THERMAL_FLOW_SCALE;
            let energy = heat_flow * dt * TIME_SCALE;

            // 温度边界（防过冲，原版 L95530-95534）
            let min_temp = src_temp.min(dest_temp);
            let max_temp = src_temp.max(dest_temp);

            // 候选新温度（原版 L95535-95546）
            let mut new_contents_temp = src_temp - energy / entry.heat_capacity;
            if new_contents_temp > max_temp {
                new_contents_temp = max_temp;
            }
            let mut new_building_temp = dest_temp + energy / entry.conduit_heat_capacity;
            if new_building_temp > max_temp {
                new_building_temp = max_temp;
            }
            if new_contents_temp < min_temp {
                new_contents_temp = min_temp;
            }
            if new_building_temp < min_temp {
                new_building_temp = min_temp;
            }

            // 能量守恒（原版 L95547-95558）：取两侧实际能量较小值
            let energy_from_contents = (new_contents_temp - src_temp).abs() * entry.heat_capacity;
            let energy_to_building = (new_building_temp - dest_temp).abs() * entry.conduit_heat_capacity;
            let sign = if heat_flow >= 0.0 { 1.0 } else { -1.0 };
            let conserved_energy = energy_from_contents.min(energy_to_building) * sign;

            // 最终温度（原版 L95559-95568）
            let mut final_contents_temp = src_temp - conserved_energy / entry.heat_capacity;
            if final_contents_temp <= 0.0 {
                final_contents_temp = 0.0;
            }
            let mut final_building_temp =
                dest_temp + conserved_energy / entry.conduit_heat_capacity;
            if final_building_temp <= 0.0 {
                final_building_temp = 0.0;
            }

            // 交叉检测（原版 L95569-95575）：方向反转 → 热容加权平均
            if (final_contents_temp - final_building_temp) * (src_temp - dest_temp) < 0.0 {
                let total_hc = entry.heat_capacity + entry.conduit_heat_capacity;
                if total_hc > 0.0 {
                    final_contents_temp = (entry.heat_capacity * src_temp
                        + entry.conduit_heat_capacity * dest_temp)
                        / total_hc;
                }
            }

            // NaN 检测（原版 L95576-95587）：回退原温度，能量归零
            let mut actual_energy = conserved_energy;
            if final_contents_temp.is_nan() {
                final_contents_temp = src_temp;
                actual_energy = 0.0;
            } else {
                entry.temperature = final_contents_temp;
            }

            if handle_index < self.temperatures.len() {
                self.temperatures[handle_index] = final_contents_temp;
            }

            // 显著能量 → 发 MODIFY_BUILDING_ENERGY（原版 L95588-95601）
            if actual_energy.abs() > ENERGY_REPORT_THRESHOLD {
                let msg_heat_flow = (src_temp - final_contents_temp) * entry.heat_capacity;
                self.pending_building_energy.push((
                    entry.conduit_building_temperature_handle,
                    msg_heat_flow,
                    min_temp,
                    max_temp,
                ));
            }

            // 相变检测（原版 L95602-95622，±3.0 滞回）
            if final_contents_temp < entry.low_state_transition_temperature - PHASE_HYSTERESIS {
                self.frozen_content_handles.push(handle_index as c_int);
            } else if final_contents_temp
                > entry.high_state_transition_temperature + PHASE_HYSTERESIS
            {
                self.melted_content_handles.push(handle_index as c_int);
            }
        }

        // 原版 L95623-95628：Update 末尾不回收延迟释放（那是 sim 线程的职责），
        // 但本模块把消息发送放在换热循环后统一执行，避免热路径反复加锁。
        self.flush_pending_messages();

        // 填充返回结构（原版 L95629-95649）
        self.update_data.num_entries = self.temperatures.len() as c_int;
        self.update_data.temperatures = if self.temperatures.is_empty() {
            std::ptr::null()
        } else {
            self.temperatures.as_ptr()
        };
        self.update_data.num_frozen_handles = self.frozen_content_handles.len() as c_int;
        self.update_data.frozen_handles = if self.frozen_content_handles.is_empty() {
            std::ptr::null()
        } else {
            self.frozen_content_handles.as_ptr()
        };
        self.update_data.num_melted_handles = self.melted_content_handles.len() as c_int;
        self.update_data.melted_handles = if self.melted_content_handles.is_empty() {
            std::ptr::null()
        } else {
            self.melted_content_handles.as_ptr()
        };

        &self.update_data
    }

    /// 把本帧缓冲的 MODIFY_BUILDING_ENERGY 消息发到 SimFrameManager。
    ///
    /// 消息 16B：[i32 handle][f32 deltaKJ][f32 minTemp][f32 maxTemp]，
    /// 与 GDU 的 elementChunkInfos 等消息同样经 SIM_HandleMessage 路径入帧。
    fn flush_pending_messages(&mut self) {
        if self.pending_building_energy.is_empty() {
            return;
        }
        let sim_ptr = crate::globals::G_SIM.lock().0;
        if sim_ptr.is_null() {
            // 测试环境无 gSim，丢弃缓冲
            self.pending_building_energy.clear();
            return;
        }
        let mut buf = [0u8; 16];
        for &(handle, delta_kj, min_temp, max_temp) in &self.pending_building_energy {
            buf[0..4].copy_from_slice(&handle.to_le_bytes());
            buf[4..8].copy_from_slice(&delta_kj.to_le_bytes());
            buf[8..12].copy_from_slice(&min_temp.to_le_bytes());
            buf[12..16].copy_from_slice(&max_temp.to_le_bytes());
            let mut reader = BinaryBufferReader::new(&buf);
            let mut result: *mut c_void = std::ptr::null_mut();
            unsafe {
                let frame_manager_ptr =
                    (sim_ptr as *mut u8).add(0x50) as *mut SimFrameManager;
                let frame_manager = &mut *frame_manager_ptr;
                frame_manager.handle_message(MSG_MODIFY_BUILDING_ENERGY, &mut reader, &mut result);
            }
        }
        self.pending_building_energy.clear();
    }
}

impl Default for ConduitTemperatureManager {
    fn default() -> Self {
        Self::new()
    }
}

// ConduitTemperatureUpdateData 含裸指针（指向自身 Vec 的分配），需手动声明 Send。
// 安全性：仅在同一时刻单线程经 Mutex 访问；C# 在 Update 返回后同步读取指针。
// 与 GameDataUpdateHolder 的 Send/Sync 模式一致。
unsafe impl Send for ConduitTemperatureManager {}

// ===== 全局单例 =====

static CONDUIT_MANAGER: Lazy<Mutex<ConduitTemperatureManager>> =
    Lazy::new(|| Mutex::new(ConduitTemperatureManager::new()));

/// sim 线程每轮循环开头调用（原版 Sim::Main L2113-2116）。
pub fn release_queued_handles_global() {
    CONDUIT_MANAGER.lock().release_queued_handles();
}

// ===== 元素表辅助 =====

/// 按元素 hash 查询温度数据（原版 GetElementIndex + gElements 字段读取）。
fn lookup_element_temperature(hash: i32) -> Option<ElementTemperatureData> {
    let index = get_element_index_pub(hash as u32);
    if index == INVALID_ELEMENT_INDEX {
        return None;
    }
    get_element_temperature_data(index)
}

/// 按元素 hash 查询默认温度（原版 Add/Set 超温重置用，Element.defaultValues.temperature）。
fn lookup_element_default_temperature(hash: i32) -> Option<f32> {
    let index = get_element_index_pub(hash as u32);
    if index == INVALID_ELEMENT_INDEX {
        return None;
    }
    get_element_default_temperature(index)
}

// ===== FFI 导出（C# ConduitTemperatureManager 7 函数）=====

/// ConduitTemperatureManager_Initialize —— 初始化（原版 operator_new + 构造）。
/// 幂等；Lazy 单例首次访问即创建。
#[no_mangle]
pub extern "C" fn ConduitTemperatureManager_Initialize() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _ = CONDUIT_MANAGER.lock();
        tracing::trace!("ConduitTemperatureManager_Initialize");
    }));
}

/// ConduitTemperatureManager_Shutdown —— 关闭（原版释放全局实例）。
/// Rust 侧清空全部状态（保留单例）。
#[no_mangle]
pub extern "C" fn ConduitTemperatureManager_Shutdown() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        CONDUIT_MANAGER.lock().clear();
        tracing::trace!("ConduitTemperatureManager_Shutdown");
    }));
}

/// ConduitTemperatureManager_Clear —— 清空（原版 5 个 vector end = begin）。
#[no_mangle]
pub extern "C" fn ConduitTemperatureManager_Clear() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        CONDUIT_MANAGER.lock().clear();
        tracing::trace!("ConduitTemperatureManager_Clear");
    }));
}

/// ConduitTemperatureManager_Add —— 注册管道温度句柄，返回完整 handle。
///
/// C# bool（conduit_insulated）经 DllImport 默认按 4 字节 BOOL 传递，故用 c_int。
#[no_mangle]
pub extern "C" fn ConduitTemperatureManager_Add(
    contents_temperature: f32,
    contents_mass: f32,
    contents_element_hash: c_int,
    conduit_structure_temperature_handle: c_int,
    conduit_heat_capacity: f32,
    conduit_thermal_conductivity: f32,
    conduit_insulated: c_int,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let elem = match lookup_element_temperature(contents_element_hash) {
            Some(e) => e,
            None => {
                tracing::warn!(
                    element_hash = contents_element_hash,
                    "ConduitTemperatureManager_Add: unknown element"
                );
                return -1;
            }
        };
        let mut temp = contents_temperature;
        if temp > SIM_MAX_TEMPERATURE {
            if let Some(default_temp) =
                lookup_element_default_temperature(contents_element_hash)
            {
                temp = default_temp;
            }
        }
        let data = ConduitTemperatureData {
            temperature: temp,
            thermal_conductivity: elem.thermal_conductivity,
            heat_capacity: elem.specific_heat_capacity * contents_mass,
            conduit_building_temperature_handle: conduit_structure_temperature_handle,
            conduit_heat_capacity,
            conduit_thermal_conductivity,
            conduit_insulated: conduit_insulated != 0,
            low_state_transition_temperature: if elem.low_temp_transition_idx == 0xffff {
                0.0
            } else {
                elem.low_temp
            },
            high_state_transition_temperature: if elem.high_temp_transition_idx == 0xffff {
                f32::MAX
            } else {
                elem.high_temp
            },
        };
        let handle = CONDUIT_MANAGER.lock().add(data);
        tracing::info!(
            handle,
            index = handle & 0xffffff,
            version = (handle >> 24) & 0xff,
            mass = contents_mass,
            element_hash = contents_element_hash,
            "ConduitTemperatureManager_Add"
        );
        handle
    }))
    .unwrap_or(-1)
}

/// ConduitTemperatureManager_Set —— 更新已有句柄（C# 每帧 EndFrame 调用）。
#[no_mangle]
pub extern "C" fn ConduitTemperatureManager_Set(
    handle: c_int,
    contents_temperature: f32,
    contents_mass: f32,
    contents_element_hash: c_int,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let ok = CONDUIT_MANAGER
            .lock()
            .set(handle, contents_temperature, contents_mass, contents_element_hash);
        tracing::trace!(
            handle,
            ok,
            mass = contents_mass,
            "ConduitTemperatureManager_Set"
        );
        if ok { 0 } else { -1 }
    }))
    .unwrap_or(-1)
}

/// ConduitTemperatureManager_Remove —— 注销句柄（延迟两轮释放）。
#[no_mangle]
pub extern "C" fn ConduitTemperatureManager_Remove(handle: c_int) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let ok = CONDUIT_MANAGER.lock().remove(handle);
        tracing::trace!(handle, ok, "ConduitTemperatureManager_Remove");
    }));
}

/// ConduitTemperatureManager_Update —— 每帧换热 + 返回温度数组。
///
/// `building_conductivity_data` 为 GDU buildingTemperatures 指针（Game.cs L1086），
/// 长度取 G_GAME_DATA_UPDATE.num_building_temperature_info。
#[no_mangle]
pub extern "C" fn ConduitTemperatureManager_Update(
    dt: f32,
    building_conductivity_data: *mut c_void,
) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        let building_len = crate::globals::G_GAME_DATA_UPDATE
            .get()
            .and_then(|m| m.try_lock())
            .map(|g| g.num_building_temperature_info)
            .unwrap_or(0);
        let building_temps: Option<&[BuildingTemperatureInfo]> = unsafe {
            if building_conductivity_data.is_null() || building_len <= 0 {
                None
            } else {
                Some(std::slice::from_raw_parts(
                    building_conductivity_data as *const BuildingTemperatureInfo,
                    building_len as usize,
                ))
            }
        };
        let mut mgr = CONDUIT_MANAGER.lock();
        let result = mgr.update(dt, building_temps);
        tracing::info!(
            entries = result.num_entries,
            building_len,
            "ConduitTemperatureManager_Update"
        );
        result as *const ConduitTemperatureUpdateData as *mut c_void
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data(
        temp: f32,
        hc: f32,
        tc: f32,
        building_handle: i32,
        conduit_hc: f32,
        conduit_tc: f32,
        insulated: bool,
        low: f32,
        high: f32,
    ) -> ConduitTemperatureData {
        ConduitTemperatureData {
            temperature: temp,
            thermal_conductivity: tc,
            heat_capacity: hc,
            conduit_building_temperature_handle: building_handle,
            conduit_heat_capacity: conduit_hc,
            conduit_thermal_conductivity: conduit_tc,
            conduit_insulated: insulated,
            low_state_transition_temperature: low,
            high_state_transition_temperature: high,
        }
    }

    /// 从原版 Add 语义构造数据（等价于元素表查询结果）。
    fn make_water_data(temp: f32, mass: f32) -> ConduitTemperatureData {
        // 水：SHC=4.179, TC=0.609, low/high 过渡索引有效
        ConduitTemperatureData {
            temperature: temp,
            thermal_conductivity: 0.609,
            heat_capacity: 4.179 * mass,
            conduit_building_temperature_handle: 0,
            conduit_heat_capacity: 100.0,
            conduit_thermal_conductivity: 1.0,
            conduit_insulated: false,
            low_state_transition_temperature: 0.0,
            high_state_transition_temperature: 2000.0,
        }
    }

    #[test]
    fn add_returns_versioned_handle() {
        let mut mgr = ConduitTemperatureManager::new();
        let h0 = mgr.add(make_water_data(300.0, 1.0));
        let h1 = mgr.add(make_water_data(310.0, 2.0));
        assert_eq!(h0, 0);
        assert_eq!(h1, 1);
        assert_eq!(h0 & 0xffffff, 0);
        assert_eq!((h0 >> 24) & 0xff, 0);
        assert_eq!(mgr.get(h0).unwrap().temperature, 300.0);
        assert_eq!(mgr.get(h1).unwrap().heat_capacity, 4.179 * 2.0);
    }

    #[test]
    fn remove_is_delayed_two_release_calls() {
        let mut mgr = ConduitTemperatureManager::new();
        let h0 = mgr.add(make_water_data(300.0, 1.0));
        let h1 = mgr.add(make_water_data(310.0, 1.0));

        assert!(mgr.remove(h0));
        // 移除后立即释放一次：切换缓冲，处理的是另一侧（空），h0 仍有效
        mgr.release_queued_handles();
        assert!(mgr.get(h0).is_some());
        // 再释放一次：回到 h0 所在的缓冲，真正回收
        mgr.release_queued_handles();
        assert!(mgr.get(h0).is_none());
        assert!(mgr.get(h1).is_some());

        // 复用 h0 的 index，版本递增
        let h2 = mgr.add(make_water_data(320.0, 1.0));
        assert_eq!(h2 & 0xffffff, 0);
        assert_ne!(h2, h0);
        assert_eq!((h2 >> 24) & 0xff, 1);
        assert!(mgr.get(h0).is_none());
        assert!(mgr.get(h2).is_some());
    }

    #[test]
    fn remove_invalid_handle_returns_false() {
        let mut mgr = ConduitTemperatureManager::new();
        assert!(!mgr.remove(0x42));
    }

    #[test]
    fn update_without_building_data_copies_temperatures() {
        let mut mgr = ConduitTemperatureManager::new();
        mgr.add(make_water_data(300.0, 1.0));
        mgr.add(make_water_data(310.0, 1.0));
        let result = mgr.update(0.2, None);
        assert_eq!(result.num_entries, 2);
        unsafe {
            assert_eq!(*result.temperatures.add(0), 300.0);
            assert_eq!(*result.temperatures.add(1), 310.0);
        }
        assert_eq!(result.num_frozen_handles, 0);
        assert_eq!(result.num_melted_handles, 0);
    }

    #[test]
    fn update_heat_exchange_matches_original_math() {
        // 内容物 300K, HC=100, TC=1.0；结构 200K, HC=200, TC=1.0，不隔热，dt=0.2
        // effectiveK = avg(1,1) = 1.0
        // heat_flow = 1.0 * (300-200) * 50 = 5000
        // energy = 5000 * 0.2 * 0.001 = 1.0
        // new_contents = 300 - 1/100 = 299.99
        // new_building = 200 + 1/200 = 200.005
        // conserved = min(1.0, 1.0) = 1.0
        // final_contents = 300 - 1/100 = 299.99
        let mut mgr = ConduitTemperatureManager::new();
        mgr.add(make_data(300.0, 100.0, 1.0, 0, 200.0, 1.0, false, 0.0, f32::MAX));
        let bt = [BuildingTemperatureInfo {
            handle: crate::a_framework::game_data::Handle { value: 0 },
            temperature: 200.0,
        }];
        let result = mgr.update(0.2, Some(&bt));
        assert_eq!(result.num_entries, 1);
        unsafe {
            let t = *result.temperatures.add(0);
            assert!((t - 299.99).abs() < 0.001, "expected ~299.99, got {}", t);
        }
        // 内部温度已更新
        assert!((mgr.get(0).unwrap().temperature - 299.99).abs() < 0.001);
    }

    #[test]
    fn update_insulated_uses_min_conductivity() {
        // contentsTC=2.0, conduitTC=1.0
        // insulated → min = 1.0；非隔热 → avg = 1.5
        let mut m1 = ConduitTemperatureManager::new();
        m1.add(make_data(300.0, 100.0, 2.0, 0, 200.0, 1.0, true, 0.0, f32::MAX));
        let bt = [BuildingTemperatureInfo {
            handle: crate::a_framework::game_data::Handle { value: 0 },
            temperature: 200.0,
        }];
        let r1 = m1.update(0.2, Some(&bt));

        let mut m2 = ConduitTemperatureManager::new();
        m2.add(make_data(300.0, 100.0, 2.0, 0, 200.0, 1.0, false, 0.0, f32::MAX));
        let r2 = m2.update(0.2, Some(&bt));

        unsafe {
            let t1 = *r1.temperatures.add(0);
            let t2 = *r2.temperatures.add(0);
            assert!(t1 > t2, "insulated ({}) cools slower than non-insulated ({})", t1, t2);
        }
    }

    #[test]
    fn update_phase_transition_detection() {
        let mut mgr = ConduitTemperatureManager::new();
        // low=100, high=500；内容物 50 < 100-3 → frozen
        mgr.add(make_data(50.0, 100.0, 1.0, 0, 200.0, 1.0, false, 100.0, 500.0));
        let bt = [BuildingTemperatureInfo {
            handle: crate::a_framework::game_data::Handle { value: 0 },
            temperature: 50.0,
        }];
        let r = mgr.update(0.2, Some(&bt));
        assert_eq!(r.num_frozen_handles, 1);
        assert_eq!(r.num_melted_handles, 0);
        unsafe {
            assert_eq!(*r.frozen_handles.add(0), 0);
        }

        let mut mgr2 = ConduitTemperatureManager::new();
        mgr2.add(make_data(600.0, 100.0, 1.0, 0, 200.0, 1.0, false, 100.0, 500.0));
        let bt2 = [BuildingTemperatureInfo {
            handle: crate::a_framework::game_data::Handle { value: 0 },
            temperature: 600.0,
        }];
        let r2 = mgr2.update(0.2, Some(&bt2));
        assert_eq!(r2.num_melted_handles, 1);
        assert_eq!(r2.num_frozen_handles, 0);
    }

    #[test]
    fn update_skips_low_heat_capacity() {
        let mut mgr = ConduitTemperatureManager::new();
        mgr.add(make_data(300.0, 0.00005, 1.0, 0, 200.0, 1.0, false, 0.0, f32::MAX));
        let bt = [BuildingTemperatureInfo {
            handle: crate::a_framework::game_data::Handle { value: 0 },
            temperature: 200.0,
        }];
        let r = mgr.update(0.2, Some(&bt));
        unsafe {
            assert_eq!(*r.temperatures.add(0), 300.0);
        }
    }

    #[test]
    fn update_crossover_uses_weighted_average() {
        // 大导热系数 + 大 dt → 交叉检测 → 加权平均
        // weighted = (100*300 + 200*200) / 300 = 233.33
        let mut mgr = ConduitTemperatureManager::new();
        mgr.add(make_data(300.0, 100.0, 1000.0, 0, 200.0, 1000.0, false, 0.0, f32::MAX));
        let bt = [BuildingTemperatureInfo {
            handle: crate::a_framework::game_data::Handle { value: 0 },
            temperature: 200.0,
        }];
        let r = mgr.update(1000.0, Some(&bt));
        unsafe {
            let t = *r.temperatures.add(0);
            assert!((t - 233.33).abs() < 1.0, "expected ~233.33, got {}", t);
        }
    }

    #[test]
    fn update_writes_only_active_handles_after_release() {
        let mut mgr = ConduitTemperatureManager::new();
        let h0 = mgr.add(make_water_data(300.0, 1.0));
        mgr.add(make_water_data(310.0, 1.0));
        mgr.remove(h0);
        mgr.release_queued_handles();
        mgr.release_queued_handles(); // h0 真正释放，槽位被 swap-remove
        let r = mgr.update(0.2, None);
        // 只剩 1 条活动数据；温度数组按 handle index 布局（h0 槽为 0.0）
        assert_eq!(r.num_entries, 2);
        unsafe {
            assert_eq!(*r.temperatures.add(0), 0.0);
            assert_eq!(*r.temperatures.add(1), 310.0);
        }
    }

    #[test]
    fn clear_resets_all_state() {
        let mut mgr = ConduitTemperatureManager::new();
        mgr.add(make_water_data(300.0, 1.0));
        mgr.remove(0);
        mgr.release_queued_handles();
        mgr.clear();
        assert!(mgr.data.is_empty());
        assert!(mgr.items.is_empty());
        assert!(mgr.versions.is_empty());
        assert!(mgr.free_handles.is_empty());
        assert!(mgr.temperatures.is_empty());
        assert!(mgr.get(0).is_none());
    }

    #[test]
    fn update_indexes_building_temps_by_building_handle_not_conduit_handle() {
        // 原版 L95510-95519：dest_temp = buildingTemps[entry.conduit_building_temperature_handle & 0xffffff]
        // （conduit 句柄与建筑温度句柄是独立分配空间，索引必然错位）。
        let mut mgr = ConduitTemperatureManager::new();
        let mut data = make_water_data(95.0, 10.0);
        data.conduit_building_temperature_handle = 7; // 本管道的建筑温度句柄 index = 7
        data.conduit_heat_capacity = 80.0;
        data.conduit_thermal_conductivity = 2.0;
        let h = mgr.add(data);
        assert_eq!(h & 0xffffff, 0); // conduit 句柄 index = 0

        // buildingTemps：index 0 是"别的建筑"（900°C），index 7 才是本管道（20°C）
        let mut bts: Vec<BuildingTemperatureInfo> = (0..8)
            .map(|i| BuildingTemperatureInfo {
                handle: crate::a_framework::game_data::Handle { value: i as i32 },
                temperature: 900.0,
            })
            .collect();
        bts[7].temperature = 20.0;

        mgr.update(0.2, Some(&bts));

        // 正确语义：dest = 管道 20°C → 内容物 95°C 应略降温
        let after = mgr.get(h).unwrap().temperature;
        assert!(
            after < 95.0,
            "内容物应从 95°C 向管道 20°C 降温（got {after}；若按 conduit 句柄 0 读到 900°C 会升温）"
        );
        assert!(after > 20.0);
    }
}
