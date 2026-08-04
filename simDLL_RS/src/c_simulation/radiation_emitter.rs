//! RadiationEmitter 组件（阶段 B：Constant/Pulsing/PulsingAveraged）。
//!
//! 对照原版 11_msvcrt_ignored.c：
//! - AddRadiationEmitterMessage 36B / Modify 40B / Remove 8B（C# SimMessages）；
//! - RadiationEmitterData 44B（00_types_reference.c L8583-8600）；
//! - Register（L39374）/ Modify（L39168）/ Update（L39693）/ tickConstant（L40172）/
//!   tickPulsing（L40263）/ inRadialRange（L40033）/ RadiationAbsorptionAlongLine（L39249）。
//!
//! DLC 门控：Update 整体包在 `radiationEnabled`（L39721）；单星零执行。
//! 覆盖：发光虫/冰息萝卜/蜜蜂（Constant）、蜂巢（Pulsing）、人力粒子发生器（Constant）、
//! 核反应堆含熔毁态（Constant，radius 25×25）。
use crate::a_framework::game_data::ComponentStateChangedMessage;
use crate::a_framework::sim_data::SimData;
use crate::d1_activity::RegionBounds;

/// RadiationEmitterType（00_types_reference.c L8574-8581）。
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(i32)]
pub enum RadiationEmitterType {
    Constant = 0,
    Pulsing = 1,
    PulsingAveraged = 2,
    SimplePulse = 3,
    RadialBeams = 4,
    Attractor = 5,
}

/// RadiationEmitterData（44B）。
#[derive(Clone, Copy, Default, Debug)]
pub struct RadiationEmitterData {
    pub cell: i32,
    pub emit_radius_x: u16,
    pub emit_radius_y: u16,
    pub emit_rads: f32,
    pub emit_rate: f32,
    pub emit_speed: f32,
    pub emit_direction: f32,
    pub emit_angle: f32,
    pub emit_timer: f32,
    pub emit_step_timer: f32,
    pub emit_type: i32,
    pub emit_step: i32,
}

impl RadiationEmitterData {
    fn from_add(msg: &AddRadiationEmitterMsg) -> Self {
        Self {
            cell: msg.cell,
            emit_radius_x: msg.emit_radius_x.max(0) as u16,
            emit_radius_y: msg.emit_radius_y.max(0) as u16,
            emit_rads: msg.emit_rads,
            emit_rate: msg.emit_rate,
            emit_speed: msg.emit_speed,
            emit_direction: msg.emit_direction,
            emit_angle: msg.emit_angle,
            emit_type: msg.emit_type,
            ..Default::default()
        }
    }
}

/// AddRadiationEmitterMessage（36B）。
#[derive(Clone, Copy, Default, Debug)]
pub struct AddRadiationEmitterMsg {
    pub callback_idx: i32,
    pub cell: i32,
    pub emit_radius_x: i16,
    pub emit_radius_y: i16,
    pub emit_rads: f32,
    pub emit_rate: f32,
    pub emit_speed: f32,
    pub emit_direction: f32,
    pub emit_angle: f32,
    pub emit_type: i32,
}

impl AddRadiationEmitterMsg {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 36 {
            return None;
        }
        Some(Self {
            callback_idx: i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            cell: i32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            emit_radius_x: i16::from_le_bytes([b[8], b[9]]),
            emit_radius_y: i16::from_le_bytes([b[10], b[11]]),
            emit_rads: f32::from_le_bytes([b[12], b[13], b[14], b[15]]),
            emit_rate: f32::from_le_bytes([b[16], b[17], b[18], b[19]]),
            emit_speed: f32::from_le_bytes([b[20], b[21], b[22], b[23]]),
            emit_direction: f32::from_le_bytes([b[24], b[25], b[26], b[27]]),
            emit_angle: f32::from_le_bytes([b[28], b[29], b[30], b[31]]),
            emit_type: i32::from_le_bytes([b[32], b[33], b[34], b[35]]),
        })
    }
}

/// ModifyRadiationEmitterMessage（40B）。
#[derive(Clone, Copy, Default, Debug)]
pub struct ModifyRadiationEmitterMsg {
    pub handle: i32,
    pub cell: i32,
    pub callback_idx: i32,
    pub emit_radius_x: i16,
    pub emit_radius_y: i16,
    pub emit_rads: f32,
    pub emit_rate: f32,
    pub emit_speed: f32,
    pub emit_direction: f32,
    pub emit_angle: f32,
    pub emit_type: i32,
}

impl ModifyRadiationEmitterMsg {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 40 {
            return None;
        }
        Some(Self {
            handle: i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            cell: i32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            callback_idx: i32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            emit_radius_x: i16::from_le_bytes([b[12], b[13]]),
            emit_radius_y: i16::from_le_bytes([b[14], b[15]]),
            emit_rads: f32::from_le_bytes([b[16], b[17], b[18], b[19]]),
            emit_rate: f32::from_le_bytes([b[20], b[21], b[22], b[23]]),
            emit_speed: f32::from_le_bytes([b[24], b[25], b[26], b[27]]),
            emit_direction: f32::from_le_bytes([b[28], b[29], b[30], b[31]]),
            emit_angle: f32::from_le_bytes([b[32], b[33], b[34], b[35]]),
            emit_type: i32::from_le_bytes([b[36], b[37], b[38], b[39]]),
        })
    }
}

/// RemoveRadiationEmitterMessage（8B）。
#[derive(Clone, Copy, Default, Debug)]
pub struct RemoveRadiationEmitterMsg {
    pub handle: i32,
    pub callback_idx: i32,
}

impl RemoveRadiationEmitterMsg {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 8 {
            return None;
        }
        Some(Self {
            handle: i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            callback_idx: i32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        })
    }
}

/// game→sim 坐标（+1 边界）。
fn game_to_sim_cell(game_cell: i32, width: i32) -> i32 {
    let gw = width - 2;
    (game_cell / gw + 1) * width + game_cell % gw + 1
}

/// RadiationEmitter 注册表（版本句柄 + free list + 延迟释放）。
pub struct RadiationEmitterRegistry {
    records: Vec<RadiationEmitterData>,
    items: Vec<i32>,
    versions: Vec<u8>,
    free_handles: Vec<i32>,
    pending_remove: Vec<(i32, i32)>, // (handle, callback_idx) 延迟释放缓冲
}

impl RadiationEmitterRegistry {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            items: Vec::new(),
            versions: Vec::new(),
            free_handles: Vec::new(),
            pending_remove: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.items.clear();
        self.versions.clear();
        self.free_handles.clear();
        self.pending_remove.clear();
    }

    pub fn record(&self, handle: i32) -> Option<&RadiationEmitterData> {
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

    pub fn record_mut(&mut self, handle: i32) -> Option<&mut RadiationEmitterData> {
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

    /// Register（L39374）：分配记录 + game→sim cell 换算 + 回调推句柄。
    pub fn register(&mut self, sim: &mut SimData, msg: &AddRadiationEmitterMsg) -> i32 {
        let mut record = RadiationEmitterData::from_add(msg);
        record.cell = game_to_sim_cell(msg.cell, sim.width);
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
        if msg.callback_idx != -1 {
            push_state_changed(sim, msg.callback_idx, handle);
        }
        handle
    }

    /// Modify（L39168）：校验 + 更新字段（handle 无效 → false）。
    pub fn modify(&mut self, sim: &SimData, msg: &ModifyRadiationEmitterMsg) -> bool {
        let Some(rec) = self.record_mut(msg.handle) else {
            return false;
        };
        rec.cell = game_to_sim_cell(msg.cell, sim.width);
        rec.emit_radius_x = msg.emit_radius_x.max(0) as u16;
        rec.emit_radius_y = msg.emit_radius_y.max(0) as u16;
        rec.emit_rads = msg.emit_rads;
        rec.emit_rate = msg.emit_rate;
        rec.emit_speed = msg.emit_speed;
        rec.emit_direction = msg.emit_direction;
        rec.emit_angle = msg.emit_angle;
        rec.emit_type = msg.emit_type;
        rec.emit_timer = 0.0;
        rec.emit_step_timer = 0.0;
        rec.emit_step = 0;
        true
    }

    /// Remove：入延迟释放缓冲，两轮后真正回收（防遍历中复用）。
    pub fn remove(&mut self, handle: i32, callback_idx: i32) {
        self.pending_remove.push((handle, callback_idx));
    }

    /// 延迟释放：每帧调用；记录已排队两轮 → 释放 + 回调推 -1。
    pub fn release_queued_handles(&mut self, sim: &mut SimData, rounds: &mut u32) {
        if self.pending_remove.is_empty() {
            return;
        }
        *rounds += 1;
        if *rounds < 2 {
            return;
        }
        *rounds = 0;
        let pending = std::mem::take(&mut self.pending_remove);
        for (handle, callback_idx) in pending {
            let index = (handle & 0xffffff) as usize;
            if index < self.versions.len() {
                self.versions[index] = self.versions[index].wrapping_add(1);
                self.items[index] = -1;
                self.free_handles.push(index as i32);
            }
            if callback_idx != -1 {
                push_state_changed(sim, callback_idx, -1);
            }
        }
    }
}

fn push_state_changed(sim: &mut SimData, callback_idx: i32, sim_handle: i32) {
    if sim.sim_events.ptr.is_null() {
        return;
    }
    let events = unsafe { &mut *sim.sim_events.ptr };
    events.component_state_changed_messages.push(ComponentStateChangedMessage {
        callback_idx,
        sim_handle,
    });
}

/// 同元素表 LCG（与 liquid_flow::next_random 同款）。
fn next_random(sim: &mut SimData) -> f32 {
    sim.random_seed = sim.random_seed.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
    ((sim.random_seed >> 16) & 0x7fff) as f32 * 3.051851e-05
}

/// 扇形判定（原版 inRadialRange L40033）。
fn in_radial_range(tx: i32, ty: i32, ex: i32, ey: i32, angle: f32, direction: f32) -> bool {
    if angle == 360.0 || (tx == ex && ty == ey) {
        return true;
    }
    let min_a = (direction - angle * 0.5).rem_euclid(360.0);
    let max_a = (angle * 0.5 + direction).rem_euclid(360.0);
    let target = ((ty - ey) as f32).atan2((tx - ex) as f32).to_degrees().rem_euclid(360.0);
    if max_a <= min_a {
        target >= min_a || target <= max_a
    } else {
        target >= min_a && target <= max_a
    }
}

/// 沿直线逐格辐射吸收（原版 RadiationAbsorptionAlongLine L39249）：
/// Bresenham 走查发射器→目标，透射率 = Π(1 − block)，block 用宇宙遮挡公式
/// （构造格 constructedFactor / 质量加权）。
fn radiation_absorption_along_line(sim: &SimData, ex: i32, ey: i32, tx: i32, ty: i32) -> f32 {
    let w = sim.width as i32;
    let h = sim.height as i32;
    let max_mass = sim.radiation_max_mass;
    let base_w = sim.radiation_base_weight;
    let dens_w = sim.radiation_density_weight;
    let const_f = sim.radiation_constructed_factor;
    let cells = unsafe { &*sim.updated_cells.ptr };
    let mut transmission = 1.0f32;
    let mut x = ex;
    let mut y = ey;
    let dx = (tx - ex).abs();
    let dy = (ty - ey).abs();
    let sx = if ex < tx { 1 } else { -1 };
    let sy = if ey < ty { 1 } else { -1 };
    let mut err = dx - dy;
    loop {
        if x >= 1 && y >= 1 && x < w - 1 && y < h - 1 {
            let cell = (y * w + x) as usize;
            let elem = cells.element_idx.get(cell);
            let factor = match crate::b_elements::elements_table::get_element_radiation_data(elem) {
                Some(d) => d.factor,
                None => 0.0,
            };
            let props = cells.properties.get(cell);
            let block = if props & 0x80 != 0 {
                factor * const_f
            } else {
                (cells.mass.get(cell) / max_mass) * factor * dens_w + factor * base_w
            };
            transmission *= 1.0 - block.clamp(0.0, 1.0);
        }
        if x == tx && y == ty {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
    }
    transmission
}

/// tickConstant（原版 L40172）：椭圆区域逐格 falloff × emitRads / lingerRate，
/// 低 falloff（<0.25）加 ±12.5% 随机抖动。
fn tick_constant(sim: &mut SimData, bounds: RegionBounds, rec: &mut RadiationEmitterData, _dt: f32) {
    let w = sim.width as usize;
    let h = sim.height as usize;
    let ex = (rec.cell % w as i32) as i32;
    let ey = (rec.cell / w as i32) as i32;
    let rx = rec.emit_radius_x.max(1) as i32;
    let ry = rec.emit_radius_y.max(1) as i32;
    let linger = sim.radiation_linger_rate;
    for dy in -ry..=ry {
        let y = ey + dy;
        if y < bounds.min_y as i32 || y > bounds.max_y as i32 || y < 1 || y >= h as i32 - 1 {
            continue;
        }
        for dx in -rx..=rx {
            let x = ex + dx;
            if x < bounds.min_x as i32 || x > bounds.max_x as i32 || x < 1 || x >= w as i32 - 1 {
                continue;
            }
            if !in_radial_range(x, y, ex, ey, rec.emit_angle, rec.emit_direction) {
                continue;
            }
            let rx2 = (rx * rx) as f32;
            let ry2 = (ry * ry) as f32;
            if (dy * dy) as f32 / ry2 + (dx * dx) as f32 / rx2 > 1.0 {
                continue;
            }
            let ax = dx.abs() as f32;
            let ay = dy.abs() as f32;
            let falloff =
                (1.0 - ax / rx as f32) - (ay - (ay * ax) / rx as f32) / ry as f32;
            let line_abs = radiation_absorption_along_line(sim, ex, ey, x, y);
            let mut amount = line_abs * (falloff * rec.emit_rads / linger);
            if falloff < 0.25 {
                let r = next_random(sim);
                amount += r * amount * 7.629627e-06 - amount * 0.125;
            }
            let cell = (y as usize) * w + x as usize;
            let u = unsafe { &mut *sim.updated_cells.ptr };
            u.radiation.set(cell, u.radiation.get(cell) + amount);
        }
    }
}

/// tickPulsing（原版 L40263）：脉冲环扩散；PulsingAveraged 均摊 emitRads。
fn tick_pulsing(sim: &mut SimData, bounds: RegionBounds, rec: &mut RadiationEmitterData, _dt: f32) {
    let w = sim.width as usize;
    let ex = (rec.cell % w as i32) as i32;
    let ey = (rec.cell / w as i32) as i32;
    let max_r = rec.emit_radius_x.max(rec.emit_radius_y).max(1) as f32;
    let step_f = (rec.emit_step as f32 + 1.0) / max_r;
    let rx = ((rec.emit_radius_x as f32) * step_f).round().max(1.0) as i32;
    let ry = ((rec.emit_radius_y as f32) * step_f).round().max(1.0) as i32;
    let mut emit_rads = rec.emit_rads;
    if rec.emit_type == RadiationEmitterType::PulsingAveraged as i32 {
        let area = ((ry - 1) * (rx - 1)).max(1) as f32;
        emit_rads /= area;
    }
    let linger = sim.radiation_linger_rate;
    if rec.emit_step == 0 {
        let cell = rec.cell as usize;
        let u = unsafe { &mut *sim.updated_cells.ptr };
        u.radiation.set(cell, u.radiation.get(cell) + emit_rads);
        return;
    }
    for dy in -ry..=ry {
        let y = ey + dy;
        if y < bounds.min_y as i32 || y > bounds.max_y as i32 || y < 1 {
            continue;
        }
        for dx in -rx..=rx {
            let x = ex + dx;
            if x < bounds.min_x as i32 || x > bounds.max_x as i32 || x < 1 {
                continue;
            }
            if !in_radial_range(x, y, ex, ey, rec.emit_angle, rec.emit_direction) {
                continue;
            }
            let rx2 = (rx * rx) as f32;
            let ry2 = (ry * ry) as f32;
            if (dy * dy) as f32 / ry2 + (dx * dx) as f32 / rx2 > 1.0 {
                continue;
            }
            let line_abs = radiation_absorption_along_line(sim, ex, ey, x, y);
            let amount = line_abs * (emit_rads / linger);
            let cell = (y as usize) * w + x as usize;
            let u = unsafe { &mut *sim.updated_cells.ptr };
            u.radiation.set(cell, u.radiation.get(cell) + amount);
        }
    }
}

/// 组件 Update（原版 RadiationEmitter::Update L39693）：门控 + 时序 + 类型分发。
pub fn update_radiation_emitters(
    sim: &mut SimData,
    reg: &mut RadiationEmitterRegistry,
    dt: f32,
    bounds: RegionBounds,
) {
    if !sim.radiation_enabled {
        return;
    }
    let w = sim.width as i32;
    let handles: Vec<i32> = (0..reg.items.len() as i32)
        .filter(|&i| reg.items[i as usize] >= 0)
        .map(|i| ((reg.versions[i as usize] as i32) << 24) | i)
        .collect();
    for handle in handles {
        let (in_bounds, active) = {
            let Some(rec) = reg.record(handle) else { continue };
            let cy = rec.cell / w;
            let cx = rec.cell % w;
            (
                cx >= bounds.min_x as i32
                    && cx <= bounds.max_x as i32
                    && cy >= bounds.min_y as i32
                    && cy <= bounds.max_y as i32,
                rec.emit_rads > 0.0 && (rec.emit_radius_x != 0 || rec.emit_radius_y != 0),
            )
        };
        if !in_bounds || !active {
            continue;
        }
        let emit = {
            let rec = reg.record_mut(handle).unwrap();
            rec.emit_timer += dt;
            if rec.emit_rate == 0.0 {
                true
            } else {
                rec.emit_timer >= rec.emit_rate
            }
        };
        if !emit {
            continue;
        }
        let rec = reg.record_mut(handle).unwrap();
        let max_r = rec.emit_radius_x.max(rec.emit_radius_y).max(1) as f32;
        let step_time = rec.emit_speed / max_r;
        let n = if rec.emit_rate == 0.0 || step_time <= 0.0 {
            1
        } else {
            ((dt / step_time).ceil() as i32).max(1)
        };
        for _ in 0..n {
            let rec = reg.record_mut(handle).unwrap();
            match rec.emit_type {
                0 => tick_constant(sim, bounds, rec, dt),
                1 | 2 => tick_pulsing(sim, bounds, rec, dt),
                _ => {} // SimplePulse/RadialBeams/Attractor：阶段 B 范围外，跳过
            }
            let rec = reg.record_mut(handle).unwrap();
            rec.emit_step += 1;
        }
        let rec = reg.record_mut(handle).unwrap();
        if rec.emit_timer > rec.emit_speed {
            rec.emit_timer = 0.0;
            rec.emit_step = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::BinaryBufferWriter;
    use crate::a_framework::sim_data::{CellSOA, SimData};
    use crate::b_elements::element::Element;
    use crate::b_elements::elements_table::{DestroyElementsTable, G_ELEMENTS_TABLE};
    use crate::LIB_TESTS_LOCK;

    fn make_sim() -> SimData {
        let mut sd = SimData::new_for_allocate(8, 8, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd
    }

    /// 元素表：0=真空(factor0)、1=固体(factor0.5)。
    fn init_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        table.elements.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        table.radiation_data.clear();
        for (id, state, factor) in [(0i32, 0u8, 0.0f32), (1i32, 3u8, 0.5f32)] {
            let mut e = Element::default();
            e.id = id;
            e.state = state;
            table.elements.push(e);
            table.state_data.push(crate::b_elements::element::ElementStateData { state });
            table.liquid_data.push(crate::b_elements::element::ElementLiquidData {
                state,
                ..Default::default()
            });
            table.pressure_data.push(crate::b_elements::element::ElementPressureData {
                state,
                ..Default::default()
            });
            table.post_process_data.push(crate::b_elements::element::ElementPostProcessData {
                state,
                ..Default::default()
            });
            table.radiation_data.push(crate::b_elements::element::ElementRadiationData {
                factor,
                rads_per_1000: 0.0,
            });
        }
        table
            .element_indices
            .insert(0u32, 0u16);
        table.element_indices.insert(1u32, 1u16);
    }

    fn fill_both(sd: &mut SimData, f: impl Fn(&mut CellSOA)) {
        unsafe {
            f(&mut *sd.cells.ptr);
            f(&mut *sd.updated_cells.ptr);
        }
    }

    /// 任务 2：Constant 发射器——椭圆中心 falloff=1.0、线吸收=1.0（真空）→
    /// emitRads/linger = 110/1.1 = 100；radius 2×2 边界外不受影响。
    #[test]
    fn emitter_constant_ticks_ellipse() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_table();
        let mut sd = make_sim();
        fill_both(&mut sd, |b| {
            for i in 0..64 {
                b.element_idx.set(i, 0);
                b.mass.set(i, 0.0);
            }
        });
        let mut reg = RadiationEmitterRegistry::new();
        // 发射器在 sim cell 14（行1 列6，gameCell=5 换算）：radius 2×2、emitRads=110、rate=0（恒发）、
        // speed=1、dir=0、angle=360（全向）、type=Constant
        let mut w = BinaryBufferWriter::new();
        w.write_int(-1); // callback
        w.write_int(5);  // gameCell → sim 14
        w.write_short(2);
        w.write_short(2);
        w.write_float(110.0);
        w.write_float(0.0);
        w.write_float(1.0);
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(0);
        let msg = AddRadiationEmitterMsg::from_bytes(&w.into_bytes()).unwrap();
        reg.register(&mut sd, &msg);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        update_radiation_emitters(&mut sd, &mut reg, 1.0, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                (u.radiation.get(14) - 100.0).abs() < 1e-3,
                "中心格 falloff1×110/1.1=100，got {}",
                u.radiation.get(14)
            );
            // radius 2 → 椭圆内最远 2 格；cell 13+8=21（上方2格）在椭圆内 → falloff>0
            assert!(u.radiation.get(22) > 0.0, "椭圆内上方格应收到辐射");
            // cell 14+16=30（上方3格）超出 radiusY=2 → 不受影响
            assert_eq!(u.radiation.get(30), 0.0, "椭圆外不受影响");
        }
        DestroyElementsTable();
    }

    /// 任务 3：Pulsing——step 0 中心 +emitRads；step 1 环（radius×2/2=2）内每格
    /// +emitRads/linger（线吸收 1.0）；emitStep 随 tick 递增。
    #[test]
    fn emitter_pulsing_expands_ring() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_table();
        let mut sd = make_sim();
        fill_both(&mut sd, |b| {
            for i in 0..64 {
                b.element_idx.set(i, 0);
                b.mass.set(i, 0.0);
            }
        });
        let mut reg = RadiationEmitterRegistry::new();
        let mut w = BinaryBufferWriter::new();
        w.write_int(-1);
        w.write_int(3); // → sim 12（行1 列4，居中）
        w.write_short(2);
        w.write_short(2);
        w.write_float(110.0);
        w.write_float(0.0);
        w.write_float(10.0); // speed=10 → 测试期间不重置 emitStep
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(1); // Pulsing
        let msg = AddRadiationEmitterMsg::from_bytes(&w.into_bytes()).unwrap();
        reg.register(&mut sd, &msg);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        // 更新 1：step 0 → 中心 +110
        update_radiation_emitters(&mut sd, &mut reg, 0.5, bounds);
        unsafe {
            assert!(
                ((*sd.updated_cells.ptr).radiation.get(12) - 110.0).abs() < 1e-3,
                "step0 中心 +110"
            );
        }
        // 更新 2：step 1 → 环（rx=ry=2）内每格 +110/1.1=100
        update_radiation_emitters(&mut sd, &mut reg, 0.5, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                (u.radiation.get(12) - 210.0).abs() < 1e-3,
                "中心 110+100=210，got {}",
                u.radiation.get(12)
            );
            assert!((u.radiation.get(13) - 100.0).abs() < 1e-3, "环内右邻 +100");
            assert!((u.radiation.get(20) - 100.0).abs() < 1e-3, "环内上方 +100");
        }
        DestroyElementsTable();
    }

    /// 任务 3b：PulsingAveraged——step 0 中心 = emitRads/面积（radius 2×2 → 面积1 → 110）。
    #[test]
    fn emitter_pulsing_averaged_center_step() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_table();
        let mut sd = make_sim();
        fill_both(&mut sd, |b| {
            for i in 0..64 {
                b.element_idx.set(i, 0);
                b.mass.set(i, 0.0);
            }
        });
        let mut reg = RadiationEmitterRegistry::new();
        let mut w = BinaryBufferWriter::new();
        w.write_int(-1);
        w.write_int(3);
        w.write_short(2);
        w.write_short(2);
        w.write_float(110.0);
        w.write_float(0.0);
        w.write_float(10.0);
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(2); // PulsingAveraged
        let msg = AddRadiationEmitterMsg::from_bytes(&w.into_bytes()).unwrap();
        reg.register(&mut sd, &msg);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        update_radiation_emitters(&mut sd, &mut reg, 0.5, bounds);
        unsafe {
            assert!(
                ((*sd.updated_cells.ptr).radiation.get(12) - 110.0).abs() < 1e-3,
                "Averaged step0 中心 = 110/面积1 = 110"
            );
        }
        DestroyElementsTable();
    }

    #[test]
    fn emitter_message_sizes() {
        assert_eq!(std::mem::size_of::<AddRadiationEmitterMsg>(), 36);
        assert_eq!(std::mem::size_of::<ModifyRadiationEmitterMsg>(), 40);
        assert_eq!(std::mem::size_of::<RemoveRadiationEmitterMsg>(), 8);
        assert_eq!(std::mem::size_of::<RadiationEmitterData>(), 44);
    }

    #[test]
    fn emitter_register_modify_remove_and_handle_versions() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        let mut sd = make_sim();
        let mut reg = RadiationEmitterRegistry::new();
        // Add：callbackIdx=3, gameCell=4（→sim cell = (4/6+1)*8 + 4%6+1 = 1*8+5 = 13），
        // radiusX=3, radiusY=2, emitRads=60, rate=0, speed=1, dir=90, angle=360, type=Constant
        let mut w = BinaryBufferWriter::new();
        w.write_int(3);
        w.write_int(4);
        w.write_short(3);
        w.write_short(2);
        w.write_float(60.0);
        w.write_float(0.0);
        w.write_float(1.0);
        w.write_float(90.0);
        w.write_float(360.0);
        w.write_int(0);
        let bytes = w.into_bytes();
        let msg = AddRadiationEmitterMsg::from_bytes(&bytes).unwrap();
        let h = reg.register(&mut sd, &msg);
        assert!(h != -1);
        {
            let rec = reg.record(h).expect("handle 有效");
            assert_eq!(rec.cell, 13, "game→sim 换算");
            assert_eq!(rec.emit_radius_x, 3);
            assert_eq!(rec.emit_radius_y, 2);
            assert!((rec.emit_rads - 60.0).abs() < 1e-6);
            assert_eq!(rec.emit_type, 0);
        }
        // Modify：radiusX→5
        let mut w2 = BinaryBufferWriter::new();
        w2.write_int(h);
        w2.write_int(4);
        w2.write_int(3);
        w2.write_short(5);
        w2.write_short(2);
        w2.write_float(60.0);
        w2.write_float(0.0);
        w2.write_float(1.0);
        w2.write_float(90.0);
        w2.write_float(360.0);
        w2.write_int(0);
        let bytes2 = w2.into_bytes();
        let mmsg = ModifyRadiationEmitterMsg::from_bytes(&bytes2).unwrap();
        assert!(reg.modify(&sd, &mmsg));
        assert_eq!(reg.record(h).unwrap().emit_radius_x, 5);
        // Remove（延迟两轮）→ 记录失效 + 回调推 -1
        reg.remove(h, 9);
        let mut rounds = 0u32;
        reg.release_queued_handles(&mut sd, &mut rounds);
        assert!(reg.record(h).is_some(), "第一轮不释放");
        reg.release_queued_handles(&mut sd, &mut rounds);
        assert!(reg.record(h).is_none(), "第二轮释放");
        unsafe {
            let events = &*sd.sim_events.ptr;
            let cbs = events.component_state_changed_messages.as_slice();
            assert_eq!(cbs[0].callback_idx, 3, "Register 回调");
            assert_eq!(cbs[0].sim_handle, h, "Register 回调推句柄");
            assert_eq!(cbs[1].callback_idx, 9, "Remove 回调");
            assert_eq!(cbs[1].sim_handle, -1, "Remove 回调推 -1");
        }
        // 句柄复用：再注册同索引 → 版本递增
        let h2 = reg.register(&mut sd, &msg);
        assert_ne!(h2, h, "版本递增后句柄不同");
    }
}
