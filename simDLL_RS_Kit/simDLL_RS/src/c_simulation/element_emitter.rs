//! ElementEmitter（元素发射器）组件。
//!
//! 对照原版 `SimDLL_Source.c`：
//! - `ElementEmitter::Emit`（04_temperature.c L1495-1609）、`Modify`（L1610）、
//!   `Register`（L1679）、`TryEmit`（L1792）、`Update`（L1966）；
//! - `GetReachableCells`（11_msvcrt_ignored.c L34451）；
//! - CopySimDataToGame emittedMassInfo 拷贝+清零（11_msvcrt_ignored.c L31626-31648）。

use crate::a_framework::game_data::{CallbackInfo, EmittedMassInfo};
use crate::a_framework::sim_data::SimData;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// ElementEmitterData（原版 44B = 0x2c，L10551）。
#[derive(Clone, Copy, Debug)]
pub struct ElementEmitterData {
    pub elapsed_time: f32,
    pub emit_interval: f32,
    pub emit_mass: f32,
    pub emit_temperature: f32,
    pub max_pressure: f32,
    pub emit_disease_count: i32,
    pub cell: [u16; 2], // [row, col]
    pub elem_idx: u16,
    pub max_depth: u8,
    pub offset_idx: u8,
    pub blocked_state: u8,
    pub disease_idx: u8,
    pub blocked_cb_idx: u32,
    pub unblocked_cb_idx: u32,
}

impl Default for ElementEmitterData {
    fn default() -> Self {
        Self {
            elapsed_time: 0.0,
            emit_interval: 0.0,
            emit_mass: 0.0,
            emit_temperature: 0.0,
            max_pressure: 0.0,
            emit_disease_count: 0,
            cell: [0, 0],
            elem_idx: 0xFFFF,
            max_depth: 0,
            offset_idx: 0,
            blocked_state: 0,
            disease_idx: 0xFF,
            blocked_cb_idx: 0xFFFFFFFF,
            unblocked_cb_idx: 0xFFFFFFFF,
        }
    }
}

/// AddElementEmitterMessage（16B，原版 L14712）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AddElementEmitterMsg {
    pub max_pressure: f32,
    pub callback_idx: i32,
    pub on_blocked_cb: i32,
    pub on_unblocked_cb: i32,
}

/// ModifyElementEmitterMessage（32B，原版 L13733）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ModifyElementEmitterMsg {
    pub handle: i32,
    pub cell_idx: i32,
    pub emit_interval: f32,
    pub emit_mass: f32,
    pub emit_temperature: f32,
    pub max_pressure: f32,
    pub disease_count: i32,
    pub element_idx: u16,
    pub max_depth: u8,
    pub disease_idx: u8,
}

/// RemoveElementEmitterMessage（8B，原版 L7452）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RemoveElementEmitterMsg {
    pub handle: i32,
    pub callback_idx: i32,
}

/// game→sim 坐标（原版 Modify：`(gc/(w-2)+1)*w + gc%(w-2)+1`）。
fn game_to_sim_cell(game_cell: i32, width: i32) -> i32 {
    let gw = width - 2;
    (game_cell / gw + 1) * width + game_cell % gw + 1
}

/// ElementEmitter 管理器（模式同 ElementChunkManager：版本句柄 + free list）。
pub struct ElementEmitterManager {
    records: Vec<ElementEmitterData>,
    items: Vec<i32>,
    versions: Vec<u8>,
    free_handles: Vec<i32>,
}

impl ElementEmitterManager {
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

    pub fn record(&self, handle: i32) -> Option<&ElementEmitterData> {
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

    fn record_mut(&mut self, handle: i32) -> Option<&mut ElementEmitterData> {
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

    pub fn active_count(&self) -> usize {
        self.items.iter().filter(|&&ri| ri >= 0).count()
    }

    /// Register（原版 L1679）：分配记录 + **同步 resize/初始化输出向量**（原版 L1761-1769
    /// Register 直接写 emittedMassInfo[handleIndex]，保证无时间帧（elapsed<=0，update 不跑）
    /// 时 C# 也能在回调同帧拿到非空 emittedMassEntries）。
    pub fn add(&mut self, sim: &mut SimData, msg: &AddElementEmitterMsg) -> i32 {
        let record = ElementEmitterData {
            max_pressure: msg.max_pressure,
            blocked_cb_idx: msg.on_blocked_cb as u32,
            unblocked_cb_idx: msg.on_unblocked_cb as u32,
            ..ElementEmitterData::default()
        };
        let index;
        if let Some(free) = self.free_handles.pop() {
            index = free as usize;
            let ri = self.records.len() as i32;
            self.items[index] = ri;
            self.records.push(record);
            self.versions[index] = self.versions[index].wrapping_add(1);
        } else {
            index = self.items.len();
            let ri = self.records.len() as i32;
            self.items.push(ri);
            self.records.push(record);
            self.versions.push(0);
        }
        let handle = ((self.versions[index] as i32) << 24) | (index as i32 & 0xffffff);
        // 输出向量 resize 到句柄数并初始化为空（原版 Register 写入 0xffff 条目）
        let n = self.items.len();
        let out = &mut sim.element_emitter.emitted_mass_info;
        out.resize(n, EmittedMassInfo::default());
        for idx in 0..n {
            out.set(
                idx,
                EmittedMassInfo {
                    elem_idx: 0xFFFF,
                    disease_idx: 0xFF,
                    pad: 0,
                    mass: 0.0,
                    temperature: 0.0,
                    disease_count: 0,
                },
            );
        }
        handle
    }

    /// Modify（原版 L1610）：校验 + 写发射参数；emitMass<=0 → blockedState=0xFF（停用标记）。
    pub fn modify(&mut self, sim: &SimData, msg: &ModifyElementEmitterMsg) -> bool {
        let Some(rec) = self.record_mut(msg.handle) else {
            return false;
        };
        let sim_cell = game_to_sim_cell(msg.cell_idx, sim.width);
        rec.cell = [(sim_cell / sim.width) as u16, (sim_cell % sim.width) as u16];
        rec.emit_interval = msg.emit_interval;
        rec.emit_mass = msg.emit_mass;
        rec.emit_temperature = msg.emit_temperature;
        rec.max_pressure = msg.max_pressure;
        rec.emit_disease_count = msg.disease_count;
        rec.elem_idx = msg.element_idx;
        rec.max_depth = msg.max_depth;
        rec.disease_idx = msg.disease_idx;
        rec.elapsed_time = 0.0;
        // 原版 Modify L1661：offsetIdx（byte 0x1f）每次置 0 —— 激活后轮转起点归零
        rec.offset_idx = 0;
        if msg.emit_mass <= 0.0 {
            rec.blocked_state = 0xFF;
        }
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
        self.versions[index] = self.versions[index].wrapping_add(1);
        self.items[index] = -1;
        self.free_handles.push(index as i32);
        let last = self.records.len() - 1;
        if ri as usize != last {
            self.records.swap(ri as usize, last);
            for item in self.items.iter_mut() {
                if *item == last as i32 {
                    *item = ri;
                    break;
                }
            }
        }
        self.records.pop();
        true
    }
}

static ELEMENT_EMITTER_MANAGER: Lazy<Mutex<ElementEmitterManager>> =
    Lazy::new(|| Mutex::new(ElementEmitterManager::new()));

pub fn clear_element_emitters() {
    ELEMENT_EMITTER_MANAGER.lock().clear();
}
pub fn add_element_emitter(sim: &mut SimData, msg: &AddElementEmitterMsg) -> i32 {
    ELEMENT_EMITTER_MANAGER.lock().add(sim, msg)
}
pub fn modify_element_emitter(sim: &SimData, msg: &ModifyElementEmitterMsg) -> bool {
    ELEMENT_EMITTER_MANAGER.lock().modify(sim, msg)
}
pub fn remove_element_emitter(handle: i32) -> bool {
    ELEMENT_EMITTER_MANAGER.lock().remove(handle)
}
pub fn element_emitter_count() -> usize {
    ELEMENT_EMITTER_MANAGER.lock().active_count()
}
#[cfg(test)]
pub fn reset_for_test() {
    ELEMENT_EMITTER_MANAGER.lock().clear();
}

/// GetReachableCells BFS（原版 11_msvcrt_ignored.c L34451 + Flood L33466，param_5=true）：
/// 从 (row,col) 出发，maxDepth 内穿越**非固体**格，收集所有可达格（含起点）。
fn get_reachable_cells(
    sim: &SimData,
    row: usize,
    col: usize,
    max_depth: usize,
    width: usize,
) -> Vec<usize> {
    let mut out = Vec::new();
    if max_depth == 0 {
        return out;
    }
    let total = width * sim.height as usize;
    let start = row * width + col;
    if start >= total {
        return out;
    }
    let is_solid = |c: usize| -> bool {
        let e = unsafe { (*sim.updated_cells.ptr).element_idx.get(c) };
        matches!(
            crate::b_elements::elements_table::get_element_post_process_data(e),
            Some(p) if (p.state & 3) == 3
        )
    };
    // visited 用线程本地世代戳池复用（省每次 218KB 分配+清零，行为等价）。
    crate::c_simulation::bfs_scratch::with_visited_scratch(total, |visited, generation| {
        let mut queue = std::collections::VecDeque::new();
        visited[start] = generation;
        queue.push_back((start, 0usize));
        while let Some((c, depth)) = queue.pop_front() {
            if is_solid(c) {
                continue; // 原版 Flood param_5=true：固体格不收集不穿越
            }
            out.push(c);
            if depth + 1 >= max_depth {
                continue;
            }
            // 原版 Flood（11_msvcrt_ignored.c L33623-33638）入队顺序：下(y-1)、左(x-1)、
            // 右(x+1)、上(y+1)。2026-08-12 对照审查修正：此前为 [下,右,左,上]（右/左互换），
            // 影响判定点被堵/满压后第二候选的落点（火山熔岩喷发 中下右左上 vs 原版 中下左右上）。
            for nb in [
                c.wrapping_sub(width),
                c.wrapping_sub(1),
                c + 1,
                c + width,
            ] {
                if nb >= total || visited[nb] == generation {
                    continue;
                }
                visited[nb] = generation;
                queue.push_back((nb, depth + 1));
            }
        }
    });
    out
}

/// Emit（原版 L1495-1609）：目标格置为发射元素 + 温度质量合并 + 病菌 + 事件 + timers。
fn emit_into_cell(
    sim: &mut SimData,
    cell: usize,
    elem_idx: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
) {
    let updated = unsafe { &*sim.updated_cells.ptr };
    let cur = updated.element_idx.get(cell);
    if cur != elem_idx && cur != sim.vacuum_element_idx {
        return; // 不应发生（TryEmit 已 displace）；原版 DebugBreak
    }
    let was_vacuum = cur == sim.vacuum_element_idx;
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.element_idx.set(cell, elem_idx);
        crate::c2_physics::liquid_flow::add_mass_and_update_temperature(
            updated,
            cell,
            mass,
            temperature,
            disease_idx,
            disease_count,
        );
    }
    if was_vacuum {
        crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
        // timers[cell] |= 0x1F（原版 L1557）
        if !sim.timers.ptr.is_null() {
            let timers_ptr = sim.timers.ptr as *mut u8;
            let old = unsafe { std::ptr::read(timers_ptr.add(cell)) };
            unsafe { std::ptr::write(timers_ptr.add(cell), old | 0x1F) };
        }
    }
}

impl ElementEmitterManager {
    /// Update（原版 L1966-2104）。
    pub fn update(
        &mut self,
        sim: &mut SimData,
        dt: f32,
        width: i32,
        bounds: crate::d1_activity::RegionBounds,
    ) {
        if sim.updated_cells.ptr.is_null() || sim.sim_events.ptr.is_null() {
            return;
        }
        let n_items = self.items.len();
        // 输出数组按 handle index 布局，add() 注册时已 resize 并初始化为空条目
        // （0xFFFF/0，原版 Register 语义）；copy_sim_data_to_game 每帧拷贝后把
        // sim 侧重置为空（原版 CopySimDataToGame L131564-131570 语义）。
        // 这里**不再重置**：多 region（多星）下每子步按 region 逐个调用本函数，
        // 若每个 region 都清空会把前面 region 的排放量冲掉（Geo Vent 喷发正常但
        // 内部物质不扣减的根因，2026-08-18）。仅兜底扩容（不缩容、不清内容）。
        sim.element_emitter
            .emitted_mass_info
            .resize(n_items, EmittedMassInfo::default());
        let w = width as usize;
        for idx in 0..self.items.len() {
            let ri = self.items[idx];
            if ri < 0 {
                continue;
            }
            let rec = &mut self.records[ri as usize];
            // 原版 ElementEmitter::Update L141535-141537：仅处理落在 region 矩形内的
            // 发射器（cell = [row, col]；★4）。max 为**包含**（`<= region.max`，
            // 且原版 elapsedTime 累加在 region 检查通过后——此处过滤先于累加，一致）。
            if rec.cell[1] < bounds.min_x as u16
                || rec.cell[0] < bounds.min_y as u16
                || rec.cell[1] > bounds.max_x as u16
                || rec.cell[0] > bounds.max_y as u16
            {
                continue;
            }
            rec.elapsed_time += dt;
            if rec.elapsed_time < rec.emit_interval {
                continue;
            }
            // 输出条目空或已是发射元素 → 尝试（原版 L2020-2023）
            let entry_ok = {
                let out = &sim.element_emitter.emitted_mass_info;
                let e = out.get(idx).elem_idx;
                e == 0xFFFF || e == sim.vacuum_element_idx || e == rec.elem_idx
            };
            if entry_ok {
                // rec.cell = [row, col]（modify 时存入）
                let cell = rec.cell[0] as usize * w + rec.cell[1] as usize;
                let reachable = get_reachable_cells(
                    sim,
                    rec.cell[0] as usize,
                    rec.cell[1] as usize,
                    rec.max_depth as usize,
                    w,
                );
                // 堵/通判定（任一可达格 mass < maxPressure → 有空间）
                let has_room = {
                    let u = unsafe { &*sim.updated_cells.ptr };
                    reachable.iter().any(|&c| {
                        let m = u.mass.get(c);
                        m < rec.max_pressure && rec.max_pressure != m
                    })
                };
                if has_room {
                    if rec.blocked_state != 0 {
                        let cb = rec.unblocked_cb_idx;
                        rec.blocked_state = 0;
                        if cb != 0xFFFFFFFF {
                            let events = unsafe { &mut *sim.sim_events.ptr };
                            // 诊断：记录 unblocked 回调值（C# 以 callbackIdx 查句柄池，
                            // -1/超限会 ArgumentOutOfRangeException——新建游戏 EMBARK 后偶发崩溃嫌疑）
                            tracing::info!(
                                cb = cb,
                                cb_signed = cb as i32,
                                event = "element_emitter_unblocked_callback",
                                "element emitter callback_info push"
                            );
                            // 原版 ElementEmitter::Update（SimDLL_Source.c L141577-141610）：
                            // blocked/unblocked 回调推送到 **callbackInfo**（simEvents+0x120，
                            // 4B CallbackInfo），C# 只登记不 Release（Game.cs L1135-1144
                            // callbackInfo.Add，可重复触发）。此前误推 componentStateChangedMessages
                            // → C# 首次触发即 Release 回调句柄 → 版本飙升 + 旧句柄残留 →
                            // "mismatched handle version" 崩溃。此为崩溃根因修复。
                            events.callback_info.push(CallbackInfo {
                                callback_idx: cb as i32,
                            });
                        }
                    }
                } else if rec.blocked_state != 1 {
                    let cb = rec.blocked_cb_idx;
                    rec.blocked_state = 1;
                    if cb != 0xFFFFFFFF {
                        let events = unsafe { &mut *sim.sim_events.ptr };
                        // 诊断：记录 blocked 回调值
                        tracing::info!(
                            cb = cb,
                            cb_signed = cb as i32,
                            event = "element_emitter_blocked_callback",
                            "element emitter callback_info push"
                        );
                        events.callback_info.push(CallbackInfo {
                            callback_idx: cb as i32,
                        });
                    }
                }
                // TryEmit
                let emitted = try_emit(sim, rec, &reachable);
                if emitted.mass > 0.0 {
                    let out = &mut sim.element_emitter.emitted_mass_info;
                    let cur = out.get(idx);
                    // 原版 ElementEmitter::Emit/Update 调 CalculateCombinedTemperature
                    // （SimDLL_Source.c L141073/L141607）：加权平均后钳制到 [min,max]。
                    let combined = crate::c2_physics::liquid_flow::calculate_combined_temperature(
                        cur.mass,
                        cur.temperature,
                        emitted.mass,
                        emitted.temperature,
                    );
                    out.set(
                        idx,
                        EmittedMassInfo {
                            elem_idx: emitted.elem_idx,
                            disease_idx: 0xFF,
                            pad: 0,
                            mass: cur.mass + emitted.mass,
                            temperature: combined,
                            disease_count: 0,
                        },
                    );
                }
            }
            rec.elapsed_time -= rec.emit_interval;
        }
    }
}

/// TryEmit（原版 L1792-1965）：轮转可达格，气/液入格、固体推回调。
fn try_emit(
    sim: &mut SimData,
    rec: &mut ElementEmitterData,
    reachable: &[usize],
) -> EmittedMassInfo {
    let mut result = EmittedMassInfo::default();
    if reachable.is_empty() {
        return result;
    }
    let count = reachable.len();
    // 原版 TryEmit 的 param_3 = offsetIdx，但原版 **从不递增** offsetIdx（Update 只读、
    // TryEmit 只读，Modify 归零）→ 轮转起点恒为 0，候选永远从判定点（BFS 首格）开始。
    // 2026-08-12 对照审查修正：此前每次发射 +1 轮转 → 中下右左上依次生成，与
    // 原版"固定从判定点生成"不符（玩家实测：钨火山熔融钨生成到上格熔化金汞齐轨道）。
    let start = rec.offset_idx as usize % count;
    let emit_state = match crate::b_elements::elements_table::get_element_by_idx(rec.elem_idx) {
        Some(e) => e.state & 3,
        None => return result,
    };
    // 原版 L1796-1799：emitTemperature < 0 → 用元素 defaultValues.temperature
    let emit_temp = if rec.emit_temperature < 0.0 {
        match crate::b_elements::elements_table::get_element_by_idx(rec.elem_idx) {
            Some(e) => e.default_values.temperature,
            None => rec.emit_temperature,
        }
    } else {
        rec.emit_temperature
    };
    for k in 0..count {
        let c = reachable[(start + k) % count];
        let (mass_c, cur_elem) = {
            let u = unsafe { &*sim.updated_cells.ptr };
            (u.mass.get(c), u.element_idx.get(c))
        };
        if !(mass_c < rec.max_pressure && rec.max_pressure != mass_c) {
            continue; // 压力门
        }
        if emit_state == 0 {
            break;
        }
        match emit_state {
            1 => {
                if cur_elem != rec.elem_idx && cur_elem != sim.vacuum_element_idx {
                    if !crate::c2_physics::liquid_flow::displace_gas(sim, c, cur_elem) {
                        continue;
                    }
                }
            }
            2 => {
                if cur_elem != rec.elem_idx && cur_elem != sim.vacuum_element_idx {
                    if !crate::c2_physics::liquid_flow::displace_liquid(sim, c, cur_elem) {
                        let cur2 = unsafe { (*sim.updated_cells.ptr).element_idx.get(c) };
                        if cur2 != sim.vacuum_element_idx {
                            if !crate::c2_physics::liquid_flow::displace_gas(sim, c, cur2) {
                                continue;
                            }
                        }
                    }
                }
            }
            3 => {
                // 固体：推 MassConsumedCallback（含可见性检查，原版 L1816-1844）
                if rec.emit_mass > 0.0 {
                    let width = sim.width as usize;
                    let game_cell =
                        ((c / width) as i32 - 1) * (sim.width - 2) + (c % width) as i32 - 1;
                    if game_cell >= 0 && game_cell < sim.num_game_cells {
                        let visible = if sim.visible_grid.ptr.is_null() {
                            0xFF
                        } else {
                            unsafe { std::ptr::read(sim.visible_grid.ptr.add(game_cell as usize)) }
                        };
                        if sim.debug_properties.is_debug_editing || visible != 0 {
                            let events = unsafe { &mut *sim.sim_events.ptr };
                            events.mass_consumed_callbacks.push(
                                crate::a_framework::game_data::MassConsumedCallback {
                                    callback_idx: game_cell,
                                    elem_idx: rec.elem_idx,
                                    disease_idx: 0xFF,
                                    pad: 0,
                                    mass: rec.emit_mass,
                                    temperature: emit_temp,
                                    disease_count: 0,
                                },
                            );
                        }
                    }
                }
                continue; // 原版固体分支 goto 下一候选
            }
            _ => continue,
        }
        // Emit 入格
        emit_into_cell(
            sim,
            c,
            rec.elem_idx,
            rec.emit_mass,
            emit_temp,
            rec.disease_idx,
            rec.emit_disease_count,
        );
        result.elem_idx = rec.elem_idx;
        result.mass = rec.emit_mass;
        result.temperature = emit_temp;
        result.disease_idx = 0xFF;
        result.disease_count = 0;
        break;
    }
    result
}

/// 顶层 Update（update_data 组件阶段调用）。
pub fn update_element_emitters(
    sim: &mut SimData,
    dt: f32,
    width: i32,
    bounds: crate::d1_activity::RegionBounds,
) {
    ELEMENT_EMITTER_MANAGER.lock().update(sim, dt, width, bounds);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;
    use crate::b_elements::element::{
        Element, ElementLiquidData, ElementPostProcessData, ElementPressureData, ElementStateData,
    };
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;

    /// 4 元素表：0=真空(state0) / 1=液体(state2) / 2=固体(state3) / 3=气体(state1)。
    fn init_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.element_names.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        for (id, state) in [(0i32, 0u8), (1i32, 2u8), (2i32, 3u8), (3i32, 1u8)] {
            let mut elem = Element::default();
            elem.id = id;
            elem.state = state;
            table.elements.push(elem);
            table.state_data.push(ElementStateData { state });
            table.liquid_data.push(ElementLiquidData {
                state,
                flow: 0.0,
                ..Default::default()
            });
            table.pressure_data.push(ElementPressureData { state, flow: 0.0 });
            table.post_process_data.push(ElementPostProcessData {
                state,
                ..Default::default()
            });
            table
                .temperature_data
                .push(crate::b_elements::element::ElementTemperatureData { state, ..Default::default() });
        }
    }

    fn make_sim() -> SimData {
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd
    }

    #[test]
    fn add_returns_versioned_handle_and_modify_writes_fields() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        let mut sim = SimData::new_for_allocate(6, 6, 1, false, false);
        let msg = AddElementEmitterMsg {
            max_pressure: 2.0,
            callback_idx: 7,
            on_blocked_cb: 8,
            on_unblocked_cb: 9,
        };
        let h = add_element_emitter(&mut sim, &msg);
        assert_eq!(h & 0xffffff, 0, "首个 handle index 0");
        let m = ModifyElementEmitterMsg {
            handle: h,
            cell_idx: 0,
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: 350.0,
            max_pressure: 2.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 2,
            disease_idx: 0xFF,
        };
        assert!(modify_element_emitter(&sim, &m));
        {
            let mgr = ELEMENT_EMITTER_MANAGER.lock();
            let rec = mgr.record(h).unwrap();
            assert_eq!(rec.emit_interval, 0.2);
            assert_eq!(rec.emit_mass, 0.002);
            assert_eq!(rec.elem_idx, 3);
            assert_eq!(rec.max_depth, 2);
            // gameCell 0 → sim cell (0/4+1)*6 + 0%4 + 1 = 7 → row 1 col 1
            assert_eq!(rec.cell, [1, 1]);
        }
    }

    #[test]
    fn modify_with_zero_mass_sets_blocked_state_ff() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        let sim = SimData::new_for_allocate(6, 6, 1, false, false);
        let mut sim = sim;
        let h = add_element_emitter(&mut sim, &AddElementEmitterMsg::default());
        let m = ModifyElementEmitterMsg {
            handle: h,
            emit_mass: 0.0,
            ..ModifyElementEmitterMsg::default()
        };
        assert!(modify_element_emitter(&sim, &m));
        let mgr = ELEMENT_EMITTER_MANAGER.lock();
        assert_eq!(mgr.record(h).unwrap().blocked_state, 0xFF);
    }

    #[test]
    fn remove_frees_and_versions_bump() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        let mut sim = SimData::new_for_allocate(6, 6, 1, false, false);
        let h = add_element_emitter(&mut sim, &AddElementEmitterMsg::default());
        assert!(remove_element_emitter(h));
        assert!(!remove_element_emitter(h), "重复移除应失败");
        let h2 = add_element_emitter(&mut sim, &AddElementEmitterMsg::default());
        assert_ne!(h2, h, "版本应递增");
        assert_eq!(h2 & 0xffffff, h & 0xffffff, "index 复用");
    }

    #[test]
    fn add_sizes_output_vector_immediately() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        let mut sim = make_sim();
        assert_eq!(sim.element_emitter.emitted_mass_info.len(), 0);
        let h = add_element_emitter(
            &mut sim,
            &AddElementEmitterMsg {
                max_pressure: 2.0,
                ..AddElementEmitterMsg::default()
            },
        );
        // 注册即 resize + 初始化（无需 update()），保证无时间帧下 C# 也能拿到非空指针
        let out = &sim.element_emitter.emitted_mass_info;
        assert_eq!(out.len(), 1, "注册后输出向量应立即为 1");
        assert_eq!(out.get(h as usize & 0xffffff).elem_idx, 0xFFFF);
    }

    /// 端到端：注册（process_frame）→ copy_sim_data_to_game → game_sync 交换 →
    /// prepare_game_data_update 读 GDU —— emitted_mass_entries 必须非空。
    #[test]
    fn register_reaches_gdu_emitted_mass_entries() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        init_table();
        use crate::a_framework::frame_sync::FrameSync;
        use crate::a_framework::sim_frame_manager::SimFrameInfo;
        use crate::c_simulation::sim_data_ops::copy_sim_data_to_game;

        let mut sim = make_sim();
        // 模拟 frame_sync 双缓冲（m_sim_data / m_game_data）
        let mut fs = FrameSync::new_zeroed();
        let mut m_sim = crate::a_framework::game_data::GameData::new(4, 4);
        let mut m_game = crate::a_framework::game_data::GameData::new(4, 4);
        fs.m_sim_data = crate::a_framework::stl_shim::UniquePtr {
            ptr: Box::into_raw(Box::new(m_sim)),
        };
        fs.m_game_data = crate::a_framework::stl_shim::UniquePtr {
            ptr: Box::into_raw(Box::new(m_game)),
        };

        // 1. process_frame：AddElementEmitter 消息 → add() 注册 + resize 输出向量
        let mut frame = SimFrameInfo::default();
        let mut msg = Vec::new();
        msg.extend_from_slice(&2.0f32.to_le_bytes()); // maxPressure
        msg.extend_from_slice(&7i32.to_le_bytes()); // callbackIdx
        msg.extend_from_slice(&(-1i32).to_le_bytes()); // onBlockedCB
        msg.extend_from_slice(&(-1i32).to_le_bytes()); // onUnblockedCB
        for b in msg {
            frame.element_emitter_messages.adds.push(b);
        }
        crate::c_simulation::frame_processor::process_frame(&mut frame, &mut sim);
        assert_eq!(sim.element_emitter.emitted_mass_info.len(), 1, "注册后 sim 输出向量应为 1");

        // 2. copy_sim_data_to_game（写入 m_sim_data）
        unsafe {
            copy_sim_data_to_game(
                &mut sim,
                &mut *fs.m_sim_data.ptr,
                &*fs.m_game_data.ptr as *const _,
                1,
            );
            assert_eq!(
                (*fs.m_sim_data.ptr).emitted_mass_info.len(),
                1,
                "copy 后 m_sim_data 输出向量应为 1"
            );
        }

        // 3. game_sync 交换双缓冲
        unsafe {
            std::mem::swap(&mut fs.m_sim_data, &mut fs.m_game_data);
        }

        // 4. prepare_game_data_update 读 m_game_data
        unsafe {
            let gd = &*fs.m_game_data.ptr;
            assert_eq!(gd.emitted_mass_info.len(), 1, "交换后 m_game_data 应为 1");
            let gdu = crate::a_framework::game_data_update::GameDataUpdate::default();
            let _ = gdu;
            // 直接验证指针：模拟 save_load 的赋值
            let ptr = gd.emitted_mass_info.as_ptr();
            assert!(!ptr.is_null(), "emitted_mass_entries 不得为 null");
        }
    }

    #[test]
    fn update_emits_after_interval_and_accumulates_output() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        init_table();
        let mut sim = make_sim();
        let h = add_element_emitter(
            &mut sim,
            &AddElementEmitterMsg {
                max_pressure: 10.0,
                ..AddElementEmitterMsg::default()
            },
        );
        let m = ModifyElementEmitterMsg {
            handle: h,
            cell_idx: 0,
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: 350.0,
            max_pressure: 10.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 2,
            disease_idx: 0xFF,
        };
        assert!(modify_element_emitter(&sim, &m));
        let b = crate::d1_activity::full_grid_bounds(&sim);
        update_element_emitters(&mut sim, 0.2, 6, b);
        let out = &sim.element_emitter.emitted_mass_info;
        assert!(out.get(0).mass > 0.0, "应累积发射量，got {}", out.get(0).mass);
        assert_eq!(out.get(0).elem_idx, 3);
        // 发射格（sim 7）应已变成气体
        let u = unsafe { &*sim.updated_cells.ptr };
        assert_eq!(u.element_idx.get(7), 3);
    }

    /// 回归（2026-08-18）：多 region（多星）下每子步按 region 逐个调用
    /// update_element_emitters，**不得**把前面 region 已写入的排放量重置掉——
    /// 此前 update 开头无条件清空整个 emitted_mass_info，后处理 region 会把
    /// 前面 region 的排放冲掉 → Geo Vent 喷发正常但内部物质不扣减。
    #[test]
    fn update_multi_region_preserves_earlier_region_emissions() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        init_table();
        let mut sim = make_sim();
        let ha = add_element_emitter(
            &mut sim,
            &AddElementEmitterMsg {
                max_pressure: 10.0,
                ..AddElementEmitterMsg::default()
            },
        );
        let hb = add_element_emitter(
            &mut sim,
            &AddElementEmitterMsg {
                max_pressure: 10.0,
                ..AddElementEmitterMsg::default()
            },
        );
        // A 判定格 = sim 7（row1,col1），B 判定格 = sim 8（row1,col2）
        let ma = ModifyElementEmitterMsg {
            handle: ha,
            cell_idx: 0,
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: 350.0,
            max_pressure: 10.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 1,
            disease_idx: 0xFF,
        };
        let mb = ModifyElementEmitterMsg {
            handle: hb,
            cell_idx: 1,
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: 350.0,
            max_pressure: 10.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 1,
            disease_idx: 0xFF,
        };
        assert!(modify_element_emitter(&sim, &ma));
        assert!(modify_element_emitter(&sim, &mb));
        // region A：含 A（col<=1），不含 B（col2）
        let ra = crate::d1_activity::RegionBounds {
            min_x: 0,
            min_y: 0,
            max_x: 1,
            max_y: 5,
        };
        update_element_emitters(&mut sim, 0.2, 6, ra);
        assert!(
            sim.element_emitter.emitted_mass_info.get(0).mass > 0.0,
            "region A 排放应写入 out[0]"
        );
        // region B：不含 A（col1 < 2）；修复前会重置整个数组冲掉 A
        let rb = crate::d1_activity::RegionBounds {
            min_x: 2,
            min_y: 0,
            max_x: 5,
            max_y: 5,
        };
        update_element_emitters(&mut sim, 0.2, 6, rb);
        assert!(
            sim.element_emitter.emitted_mass_info.get(0).mass > 0.0,
            "region B 不得冲掉 region A 的排放（多星 Geo Vent 不扣减根因）"
        );
        assert!(
            sim.element_emitter.emitted_mass_info.get(1).mass > 0.0,
            "region B 排放应写入 out[1]"
        );
    }

    #[test]
    fn update_fires_blocked_callback_when_no_room() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        init_table();
        let mut sim = make_sim();
        let h = add_element_emitter(
            &mut sim,
            &AddElementEmitterMsg {
                max_pressure: 1.0,
                on_blocked_cb: 55,
                ..AddElementEmitterMsg::default()
            },
        );
        // maxDepth=1 → 可达格仅发射格自身；填满质量 >= maxPressure → 堵住
        unsafe {
            let u = &mut *sim.updated_cells.ptr;
            u.element_idx.set(7, 1);
            u.mass.set(7, 2.0);
            u.temperature.set(7, 300.0);
        }
        let m = ModifyElementEmitterMsg {
            handle: h,
            cell_idx: 0,
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: 350.0,
            max_pressure: 1.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 1,
            disease_idx: 0xFF,
        };
        assert!(modify_element_emitter(&sim, &m));
        let b = crate::d1_activity::full_grid_bounds(&sim);
        update_element_emitters(&mut sim, 0.2, 6, b);
        let events = unsafe { &*sim.sim_events.ptr };
        let cb = events
            .callback_info
            .as_slice()
            .iter()
            .find(|m| m.callback_idx == 55);
        assert!(cb.is_some(), "应推 blocked 回调");
    }

    /// 回归：发射器必须作用在**正确的非对称格**（modify 存 [row,col]，update 不得转置）。
    /// game cell 7 → sim 16（row2,col4）；若 row/col 写反会作用到 sim 26（row4,col2）。
    #[test]
    fn update_emits_into_correct_nonsymmetric_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        init_table();
        let mut sim = make_sim();
        let h = add_element_emitter(
            &mut sim,
            &AddElementEmitterMsg {
                max_pressure: 10.0,
                ..AddElementEmitterMsg::default()
            },
        );
        let m = ModifyElementEmitterMsg {
            handle: h,
            cell_idx: 7, // → sim 16 = row2 col4（非对称）
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: 350.0,
            max_pressure: 10.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 1,
            disease_idx: 0xFF,
        };
        assert!(modify_element_emitter(&sim, &m));
        {
            let mgr = ELEMENT_EMITTER_MANAGER.lock();
            let rec = mgr.record(h).unwrap();
            assert_eq!(rec.cell, [2, 4], "modify 应存 [row,col]");
        }
        let b = crate::d1_activity::full_grid_bounds(&sim);
        update_element_emitters(&mut sim, 0.2, 6, b);
        let u = unsafe { &*sim.updated_cells.ptr };
        assert_eq!(u.element_idx.get(16), 3, "sim 16（row2,col4）应变成气体");
        assert_ne!(
            u.element_idx.get(26),
            3,
            "转置格 sim 26（row4,col2）不应被错误写入"
        );
    }

    /// 回归（2026-08-12 修正）：原版 **从不递增** offsetIdx（Update/TryEmit 均只读、
    /// Modify 归零）→ 轮转起点恒为 0，候选永远从判定点开始；此前每次发射 +1 轮转
    /// 导致"中下右左上"依次生成（钨火山熔融钨喷到上格熔化金汞齐轨道）。
    #[test]
    fn modify_resets_offset_idx() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        init_table();
        let mut sim = make_sim();
        let h = add_element_emitter(
            &mut sim,
            &AddElementEmitterMsg {
                max_pressure: 10.0,
                ..AddElementEmitterMsg::default()
            },
        );
        let m = ModifyElementEmitterMsg {
            handle: h,
            cell_idx: 7,
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: 350.0,
            max_pressure: 10.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 2,
            disease_idx: 0xFF,
        };
        assert!(modify_element_emitter(&sim, &m));
        let b = crate::d1_activity::full_grid_bounds(&sim);
        update_element_emitters(&mut sim, 0.2, 6, b);
        {
            let mgr = ELEMENT_EMITTER_MANAGER.lock();
            assert_eq!(
                mgr.record(h).unwrap().offset_idx,
                0,
                "发射后 offset_idx 必须保持 0（原版从不轮转）"
            );
        }
        assert!(modify_element_emitter(&sim, &m));
        {
            let mgr = ELEMENT_EMITTER_MANAGER.lock();
            assert_eq!(mgr.record(h).unwrap().offset_idx, 0, "Modify 保持 offset_idx = 0");
        }
    }

    /// 回归：emitTemperature < 0 → 用元素 defaultValues.temperature（原版 L1796-1799）。
    #[test]
    fn try_emit_negative_temp_falls_back_to_default() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_for_test();
        init_table();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements[3].default_values.temperature = 300.0;
        }
        let mut sim = make_sim();
        let h = add_element_emitter(
            &mut sim,
            &AddElementEmitterMsg {
                max_pressure: 10.0,
                ..AddElementEmitterMsg::default()
            },
        );
        let m = ModifyElementEmitterMsg {
            handle: h,
            cell_idx: 0,
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: -1.0,
            max_pressure: 10.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 1,
            disease_idx: 0xFF,
        };
        assert!(modify_element_emitter(&sim, &m));
        let b = crate::d1_activity::full_grid_bounds(&sim);
        update_element_emitters(&mut sim, 0.2, 6, b);
        let u = unsafe { &*sim.updated_cells.ptr };
        assert_eq!(u.element_idx.get(7), 3);
        assert!(
            (u.temperature.get(7) - 300.0).abs() < 1e-3,
            "负温度应回退元素默认 300，got {}",
            u.temperature.get(7)
        );
        let out = &sim.element_emitter.emitted_mass_info;
        assert!((out.get(0).temperature - 300.0).abs() < 1e-3);
    }
}
