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
use once_cell::sync::Lazy;
use parking_lot::Mutex;

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
    release_rounds: u32,
}

impl RadiationEmitterRegistry {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            items: Vec::new(),
            versions: Vec::new(),
            free_handles: Vec::new(),
            pending_remove: Vec::new(),
            release_rounds: 0,
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.items.clear();
        self.versions.clear();
        self.free_handles.clear();
        self.pending_remove.clear();
        self.release_rounds = 0;
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
        // 原版 Modify（L39168-39220）：emitSpeed = min(emitSpeed, emitRate)；
        // 且不重置 emitTimer/emitStep/emitStepTimer（OnSimDeactivate 全零 Modify 后
        // 重新激活时计时状态延续，与 Update 内 gating 语义一致）。
        rec.emit_speed = msg.emit_speed.min(msg.emit_rate);
        rec.emit_direction = msg.emit_direction;
        rec.emit_angle = msg.emit_angle;
        rec.emit_type = msg.emit_type;
        true
    }

    /// Remove：入延迟释放缓冲，两轮后真正回收（防遍历中复用）。
    pub fn remove(&mut self, handle: i32, callback_idx: i32) {
        self.pending_remove.push((handle, callback_idx));
    }

    /// 延迟释放：每帧调用一次；记录已排队两轮 → 释放 + 回调推 -1。
    pub fn release_queued_handles(&mut self, sim: &mut SimData) {
        if self.pending_remove.is_empty() {
            return;
        }
        self.release_rounds += 1;
        if self.release_rounds < 2 {
            return;
        }
        self.release_rounds = 0;
        let pending = std::mem::take(&mut self.pending_remove);
        for (handle, callback_idx) in pending {
            let index = (handle & 0xffffff) as usize;
            let version = ((handle >> 24) & 0xff) as u8;
            // 版本校验：旧存档的延迟 Remove 在新存档复用同 index 后到达时，
            // 不能误杀新注册的发射器（新世界分配时注册表已整体清空）。
            if index < self.versions.len() && self.versions[index] == version {
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

// ===== 全局管理器（仿 element_emitter）=====

static RADIATION_EMITTER_MANAGER: Lazy<Mutex<RadiationEmitterRegistry>> =
    Lazy::new(|| Mutex::new(RadiationEmitterRegistry::new()));

pub fn clear_radiation_emitters() {
    RADIATION_EMITTER_MANAGER.lock().clear();
}

pub fn radiation_emitter_count() -> usize {
    RADIATION_EMITTER_MANAGER.lock().active_count()
}

/// 处理帧内 radiation_emitter_messages：adds → modifies → removes（含延迟释放）。
pub fn process_radiation_emitter_messages(frame: &crate::a_framework::sim_frame_manager::SimFrameInfo, sim: &mut SimData) {
    let mut mgr = RADIATION_EMITTER_MANAGER.lock();
    for b in frame.radiation_emitter_messages.adds.as_slice().chunks(36) {
        if let Some(msg) = AddRadiationEmitterMsg::from_bytes(b) {
            mgr.register(sim, &msg);
        }
    }
    for b in frame.radiation_emitter_messages.modifies.as_slice().chunks(40) {
        if let Some(msg) = ModifyRadiationEmitterMsg::from_bytes(b) {
            mgr.modify(sim, &msg);
        }
    }
    for b in frame.radiation_emitter_messages.removes.as_slice().chunks(8) {
        if let Some(msg) = RemoveRadiationEmitterMsg::from_bytes(b) {
            mgr.remove(msg.handle, msg.callback_idx);
        }
    }
}

/// 延迟释放（sim 线程每帧调用一次）。
pub fn release_queued_handles_global(sim: &mut SimData) {
    RADIATION_EMITTER_MANAGER.lock().release_queued_handles(sim);
}

/// 组件 Update（全局版）。
pub fn update_radiation_emitters_global(sim: &mut SimData, dt: f32, bounds: RegionBounds) {
    let mut mgr = RADIATION_EMITTER_MANAGER.lock();
    update_radiation_emitters(sim, &mut mgr, dt, bounds);
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
    // 原版 L40280-40288：随机值 = (seed>>16 & 0x7fff)，tickConstant 抖动公式
    // 再乘 7.629627e-06（=1/131072）与 amount。此前归一化到 [0,1] 导致随机项
    // 再缩 1/32768 ≈ 0，抖动退化为恒定 −12.5%。
    ((sim.random_seed >> 16) & 0x7fff) as f32
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
/// **半步 DDA** 走查发射器→目标（非整数 Bresenham——|dy|>|dx| 斜率下两者
/// 访问的格子集合不同），透射率 = Π(1 − block)，block 用宇宙遮挡公式
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
    let abs_dx = (tx - ex).abs();
    let abs_dy = (ty - ey).abs();
    // 主轴选择（原版 bVar1 = |dy| <= |dx|，主轴为 X；否则为 Y）。
    let walk_x = abs_dy <= abs_dx;
    // 原版排序后恒从**较小主轴坐标**走到较大者（主轴 +1 步进）；
    // 透射率是乘积、与方向无关，故等价。主轴/副轴端点按同一端点配对。
    let (dom0, minor0, dom1, minor1) = if walk_x {
        if ex <= tx {
            (ex, ey, tx, ty)
        } else {
            (tx, ty, ex, ey)
        }
    } else {
        if ey <= ty {
            (ey, ex, ty, tx)
        } else {
            (ty, tx, ey, ex)
        }
    };
    let (mut dom, mut minor) = (dom0, minor0);
    let dom_end = dom1;
    let minor_step = if minor1 < minor0 { -1 } else { 1 };
    let (dom_span, minor_span) = if walk_x {
        (abs_dx, abs_dy)
    } else {
        (abs_dy, abs_dx)
    };
    // 半步误差（原版 L39280：fVar13 = (fVar10 - fVar11) * 0.5）
    let mut error = 0.5 * dom_span as f32;
    loop {
        let (x, y) = if walk_x { (dom, minor) } else { (minor, dom) };
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
        if dom == dom_end {
            break;
        }
        // 原版 L39820-39826：error -= 副轴跨度；<0 → error += 主轴跨度、副轴步进
        error -= minor_span as f32;
        if error < 0.0 {
            error += dom_span as f32;
            minor += minor_step;
        }
        dom += 1;
    }
    transmission.clamp(0.0, 1.0)
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
    if y < bounds.min_y as i32 || y >= bounds.max_y as i32 || y < 1 || y >= h as i32 - 1 {
            continue;
        }
        for dx in -rx..=rx {
            let x = ex + dx;
    if x < bounds.min_x as i32 || x >= bounds.max_x as i32 || x < 1 || x >= w as i32 - 1 {
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

/// tickPulsing（原版 L40255-40350）：脉冲环扩散。
/// - step 0：中心格 += emitRads（PulsingAveraged 为 emitRads/面积）；
/// - step 1+：SetCircleAA 抗锯齿椭圆轮廓（1~2px 环，非填充圆盘），
///   每像素强度 = roundf(frac × emitRads)，**无 linger 除**（区别于 Constant）；
/// - rx/ry = ceil(radius×step_f)（原版 movmskps 取整 hack：正值上取整），min 1。
fn tick_pulsing(sim: &mut SimData, bounds: RegionBounds, rec: &mut RadiationEmitterData, _dt: f32) {
    let w = sim.width as i32;
    let ex = rec.cell % w;
    let ey = rec.cell / w;
    let max_r = rec.emit_radius_x.max(rec.emit_radius_y) as f32;
    if max_r <= 0.0 {
        return;
    }
    let step_f = (rec.emit_step as f32 + 1.0) / max_r;
    let rx = (rec.emit_radius_x as f32 * step_f).ceil().max(1.0) as i32;
    let ry = (rec.emit_radius_y as f32 * step_f).ceil().max(1.0) as i32;
    let mut emit_rads = rec.emit_rads;
    if rec.emit_type == RadiationEmitterType::PulsingAveraged as i32 {
        let area = ((ry - 1) * (rx - 1)).max(1) as f32;
        emit_rads /= area;
    }
    if rec.emit_step == 0 {
        let cell = rec.cell as usize;
        let u = unsafe { &mut *sim.updated_cells.ptr };
        u.radiation.set(cell, u.radiation.get(cell) + emit_rads);
        return;
    }
    if rx <= 1 && ry <= 1 {
        return;
    }
    set_circle_aa(sim, bounds, rec, ex, ey, rx, ry, emit_rads);
}

/// SetCircleAA（原版 L39482-39596）：抗锯齿椭圆轮廓。
/// 两个循环（x 向扫描 + y 向扫描）各画边界像素 + 内侧补像素，
/// 强度按边界亚像素位置插值：inten = roundf(frac × rads)，补像素 = rads − inten。
fn set_circle_aa(
    sim: &mut SimData,
    bounds: RegionBounds,
    rec: &RadiationEmitterData,
    cx: i32,
    cy: i32,
    rx: i32,
    ry: i32,
    rads: f32,
) {
    if rx < 1 || ry < 1 {
        return;
    }
    let rx2 = (rx * rx) as f32;
    let ry2 = (ry * ry) as f32;
    let inv_hyp = 1.0 / (rx2 + ry2).sqrt();
    // 第一循环：dx 0..round(rx²/hypot)，dy = floor(ry×sqrt(1−(dx/rx)²))
    let dx_max = (inv_hyp * rx2).round() as i32;
    for dx in 0..=dx_max {
        let y_exact = (1.0 - (dx * dx) as f32 / rx2).sqrt() * ry as f32;
        let dy = y_exact.floor() as i32;
        let inten = ((y_exact - dy as f32) * rads).round();
        if inten > 0.0 {
            set_pulsing_pixel(sim, bounds, rec, cx, cy, dx, dy, inten);
        }
        let comp = rads - inten;
        if comp > 0.0 {
            set_pulsing_pixel(sim, bounds, rec, cx, cy, dx, dy - 1, comp);
        }
    }
    // 第二循环：dy 0..round(ry²/hypot)，dx = floor(rx×sqrt(1−(dy/ry)²))
    let dy_max = (inv_hyp * ry2).round() as i32;
    for dy in 0..=dy_max {
        let x_exact = (1.0 - (dy * dy) as f32 / ry2).sqrt() * rx as f32;
        let dx = x_exact.floor() as i32;
        let inten = ((x_exact - dx as f32) * rads).round();
        if inten > 0.0 {
            set_pulsing_pixel(sim, bounds, rec, cx, cy, dx, dy, inten);
        }
        let comp = rads - inten;
        if comp > 0.0 {
            set_pulsing_pixel(sim, bounds, rec, cx, cy, dx - 1, dy, comp);
        }
    }
}

/// setPulsing（原版 L40110-40142）：四象限镜像写辐射 += absorption × intensity；
/// 强度 ≤0 / 越界 / 扇形外 / 源格跳过。
fn set_pulsing_pixel(
    sim: &mut SimData,
    bounds: RegionBounds,
    rec: &RadiationEmitterData,
    cx: i32,
    cy: i32,
    dx: i32,
    dy: i32,
    intensity: f32,
) {
    let w = sim.width as i32;
    let h = sim.height as i32;
    for (sx, sy) in [(1i32, 1i32), (-1, 1), (1, -1), (-1, -1)] {
        let x = cx + sx * dx;
        let y = cy + sy * dy;
        if x < 1 || y < 1 || x >= w - 1 || y >= h - 1 {
            continue;
        }
        if x < bounds.min_x as i32
                    || x >= bounds.max_x as i32
            || y < bounds.min_y as i32
                    || y >= bounds.max_y as i32
        {
            continue;
        }
        if x == cx && y == cy {
            continue;
        }
        if !in_radial_range(x, y, cx, cy, rec.emit_angle, rec.emit_direction) {
            continue;
        }
        let line_abs = radiation_absorption_along_line(sim, cx, cy, x, y);
        let cell = (y as usize) * (w as usize) + x as usize;
        let u = unsafe { &mut *sim.updated_cells.ptr };
        u.radiation.set(cell, u.radiation.get(cell) + line_abs * intensity);
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
                    && cx < bounds.max_x as i32
                    && cy >= bounds.min_y as i32
                    && cy < bounds.max_y as i32,
                rec.emit_rads > 0.0 && (rec.emit_radius_x != 0 || rec.emit_radius_y != 0),
            )
        };
        if !in_bounds || !active {
            continue;
        }
        // 时序（原版 Update L39693-39881 反编译语义）：
        // 1) emitTimer += dt；rate_ok = rate==0 || timer>=rate；
        //    timer<=speed 恒可发（rate_ok 时重置 timer/step）；
        //    timer>speed 且 !rate_ok → skip；timer>speed 且 rate_ok → 重置后发。
        // 2) 发射次数 n = (int)(dt / step_time)，step_time = emitSpeed / max_r；
        //    rate==0 恒 n=1；n<=1 且 step_time <= emitStepTimer（小数累加器）
        //    → 补发 1 次；否则 n<1 不发（等累加器攒够一个 step_time）。
        // 3) 每次发射 emitStep = (emitStep+1) % max_r，emitStepTimer -= step_time；
        //    帧末 emitStepTimer += dt。
        let rec = reg.record_mut(handle).unwrap();
        rec.emit_timer += dt;
        let rate_ok = rec.emit_rate == 0.0 || rec.emit_timer >= rec.emit_rate;
        if rec.emit_timer <= rec.emit_speed {
            if rate_ok {
                rec.emit_timer = 0.0;
                rec.emit_step = 0;
            }
        } else {
            if !rate_ok {
                continue;
            }
            rec.emit_timer = 0.0;
            rec.emit_step = 0;
        }
        let rec = reg.record_mut(handle).unwrap();
        let max_r = rec.emit_radius_x.max(rec.emit_radius_y) as f32;
        if max_r <= 0.0 {
            continue;
        }
        let step_time = rec.emit_speed / max_r;
        let mut n = if step_time <= 0.0 {
            1.0
        } else {
            dt / step_time
        };
        if rec.emit_rate == 0.0 || (n <= 1.0 && step_time <= rec.emit_step_timer) {
            n = 1.0;
        }
        let count = n as i32;
        if count > 0 {
            for _ in 0..count {
                let rec = reg.record_mut(handle).unwrap();
                match rec.emit_type {
                    0 => tick_constant(sim, bounds, rec, dt),
                    1 | 2 => tick_pulsing(sim, bounds, rec, dt),
                    _ => {} // SimplePulse/RadialBeams/Attractor：阶段 B 范围外，跳过
                }
                let rec = reg.record_mut(handle).unwrap();
                rec.emit_step = (rec.emit_step + 1) % max_r as i32;
                rec.emit_step_timer -= step_time;
            }
        }
        let rec = reg.record_mut(handle).unwrap();
        rec.emit_step_timer += dt;
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
        let _lock = LIB_TESTS_LOCK.lock();
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

    /// 原版半步 DDA 线走查（L39249）：(2,2)→(3,5)（|dy|>|dx|）经过
    /// (2,2),(2,3),(3,4),(3,5)；整数 Bresenham 会经过 (2,4)。把 (2,4)
    /// 设为全遮挡 → DDA 透射率必须为 1.0。
    #[test]
    fn radiation_absorption_line_dda_visits_original_cells() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_table();
        let mut sd = make_sim();
        fill_both(&mut sd, |b| {
            for i in 0..64 {
                b.element_idx.set(i, 0);
                b.mass.set(i, 0.0);
            }
            // cell 34 = (2,4)：仅整数 Bresenham 路径经过
            b.element_idx.set(34, 1);
            b.mass.set(34, 1e6);
        });
        let t = radiation_absorption_along_line(&sd, 2, 2, 3, 5);
        assert!(
            (t - 1.0).abs() < 1e-6,
            "DDA 路径避开 (2,4) 全遮挡格，透射率应为 1.0，got {t}"
        );
        DestroyElementsTable();
    }

    /// next_random 返回原始 (seed>>16 & 0x7fff)（原版 L40280-40288），
    /// 不做 [0,1] 归一化——归一化会让 tickConstant 抖动随机项趋零。
    #[test]
    fn next_random_returns_raw_rand_value() {
        let mut sd = make_sim();
        sd.random_seed = 12345;
        let s1 = 12345u32.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        let expected = ((s1 >> 16) & 0x7fff) as f32;
        let got = next_random(&mut sd);
        assert!(
            (got - expected).abs() < 1e-6,
            "raw rand = {expected}，got {got}"
        );
        assert!(got >= 0.0 && got <= 32767.0, "rand ∈ [0, 32767]");
    }

    /// tickConstant 低 falloff 抖动（原版 L40280-40291）：amount +=
    /// rand×amount×7.629627e-06 − amount×0.125（±12.5%）。
    /// radius 3 发射器在 sim cell 12（cx=4, cy=1）：按 dy→dx 行主序，
    /// 仅 (dy=0,dx=-3) 轴向格（falloff=0）先抽取 1 次（(dy=0,dx=3) 在 x=7
    /// 边界被跳过），故 cell 18（dx=-2, dy=1，falloff=2/9、amount=22.222…）
    /// 是**第 2 次**随机抽取，用 seed 确定性断言公式。
    #[test]
    fn tick_constant_low_falloff_jitter_matches_original() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_table();
        let mut sd = make_sim();
        sd.random_seed = 12345;
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
        w.write_short(3);
        w.write_short(3);
        w.write_float(110.0);
        w.write_float(0.0); // rate=0
        w.write_float(1.0);
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(0); // Constant
        let msg = AddRadiationEmitterMsg::from_bytes(&w.into_bytes()).unwrap();
        reg.register(&mut sd, &msg);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        update_radiation_emitters(&mut sd, &mut reg, 1.0, bounds);
        // 复算前两次 LCG 抽取
        let mut s = 12345u32;
        s = s.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        s = s.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        let rand2 = ((s >> 16) & 0x7fff) as f32;
        // cell 18 = (2,2)，相对发射器 dx=-2、dy=1
        let falloff = (1.0 - 2.0 / 3.0) - (1.0 - (1.0 * 2.0) / 3.0) / 3.0;
        let base = falloff * 110.0 / 1.1;
        let expected = base + rand2 * base * 7.629627e-06 - base * 0.125;
        unsafe {
            let got = (*sd.updated_cells.ptr).radiation.get(18);
            assert!(
                (got - expected).abs() < 1e-3,
                "cell18 抖动 = {expected}，got {got}（base={base}, rand2={rand2}）"
            );
        }
        DestroyElementsTable();
    }

    /// 任务 3：Pulsing——step 0 中心 +emitRads；step 1 环 = SetCircleAA 抗锯齿
    /// 椭圆轮廓（rx=ry=2：轴向外侧像素强度 0 被跳过，环落在内侧一格），
    /// 无 linger 除；dt=0.5=step_time(1/2)，每次更新发射 1 次；
    /// rate=1 边界（timer>=1s）重置 emitStep。
    #[test]
    fn emitter_pulsing_expands_ring() {
        let _lock = LIB_TESTS_LOCK.lock();
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
        w.write_float(1.0);  // rate=1：step 随 1s 边界重置
        w.write_float(1.0);  // speed=1
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(1); // Pulsing
        let msg = AddRadiationEmitterMsg::from_bytes(&w.into_bytes()).unwrap();
        let h = reg.register(&mut sd, &msg);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        // 更新 1（dt=0.5=step_time）：step 0 → 中心 +110
        update_radiation_emitters(&mut sd, &mut reg, 0.5, bounds);
        unsafe {
            assert!(
                ((*sd.updated_cells.ptr).radiation.get(12) - 110.0).abs() < 1e-3,
                "step0 中心 +110"
            );
        }
        // 更新 2（dt=0.5）：timer=1.0=rate → 重置 emitStep=0 → step 0 再次 +110
        update_radiation_emitters(&mut sd, &mut reg, 0.5, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                (u.radiation.get(12) - 220.0).abs() < 1e-3,
                "rate 重置后中心 110+110=220，got {}",
                u.radiation.get(12)
            );
        }
        // 更新 3（dt=0.5）：step 1 → 环（rx=ry=2）轮廓
        update_radiation_emitters(&mut sd, &mut reg, 0.5, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                (u.radiation.get(12) - 220.0).abs() < 1e-3,
                "环步不写中心，220，got {}",
                u.radiation.get(12)
            );
            // 半径 2 圆环（rx=ry=2）：边界恰落整格 → 外部像素强度 0 被跳过，
            // 环像素落在内侧：轴向 (1,0)=278（原版 SetPixel4 四象限在 dy=0 时
            // 两次写同一格：(29+110)×2）、(0,1)=278 同理、对角 (1,1)=162（81+81）。
            assert!(
                (u.radiation.get(13) - 278.0).abs() < 1e-3,
                "环内右邻 (29+110)×2=278，got {}",
                u.radiation.get(13)
            );
            assert!(
                (u.radiation.get(20) - 278.0).abs() < 1e-3,
                "环内上方 (110+29)×2=278，got {}",
                u.radiation.get(20)
            );
            assert!(
                (u.radiation.get(21) - 162.0).abs() < 1e-3,
                "环内对角 81+81=162，got {}",
                u.radiation.get(21)
            );
            assert_eq!(u.radiation.get(14), 0.0, "半径 2 环外侧轴向 (2,0) 强度 0");
            assert_eq!(u.radiation.get(28), 0.0, "半径 2 环外侧轴向 (0,2) 强度 0");
        }
        DestroyElementsTable();
    }

    /// 任务 3c：Pulsing 多步发射 + emitStep 在 max_r 后回环。
    #[test]
    fn emitter_pulsing_multistep_wraps_step() {
        let _lock = LIB_TESTS_LOCK.lock();
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
        w.write_float(10.0); // rate=10：测试期间不重置
        w.write_float(1.0);  // speed=1
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(1); // Pulsing
        let msg = AddRadiationEmitterMsg::from_bytes(&w.into_bytes()).unwrap();
        let h = reg.register(&mut sd, &msg);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        // dt=1.0 → n=(int)(1.0/0.5)=2：step 0 → 中心 +110，step 1 → 环轮廓，
        // emitStep 2 → max_r=2 → 环绕 0。
        update_radiation_emitters(&mut sd, &mut reg, 1.0, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                (u.radiation.get(12) - 110.0).abs() < 1e-3,
                "step0 中心 110，got {}",
                u.radiation.get(12)
            );
            assert!((u.radiation.get(13) - 278.0).abs() < 1e-3, "环右邻 278");
            assert!((u.radiation.get(20) - 278.0).abs() < 1e-3, "环上方 278");
            assert!((u.radiation.get(21) - 162.0).abs() < 1e-3, "环对角 162");
        }
        let rec = reg.record(h).unwrap();
        assert_eq!(rec.emit_step, 0, "emitStep 在 max_r=2 后回环");
        DestroyElementsTable();
    }

    /// 任务 3e：Pulsing 环是**轮廓**而非填充圆盘——radius 3×3 的 step 2 环
    /// （rx=ry=3）：内部 (1,0) 不受辐射、环上 (2,0)=110、外侧 (3,0)=0。
    #[test]
    fn emitter_pulsing_ring_outline_not_filled() {
        let _lock = LIB_TESTS_LOCK.lock();
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
        w.write_short(3);
        w.write_short(3);
        w.write_float(110.0);
        w.write_float(10.0); // rate=10：测试期间不重置
        w.write_float(1.0);  // speed=1 → step_time = 1/3
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(1); // Pulsing
        let msg = AddRadiationEmitterMsg::from_bytes(&w.into_bytes()).unwrap();
        let h = reg.register(&mut sd, &msg);
        // 直接置 step=2（step 0/1 已发射过），只测 step 2 环（rx=ry=3）。
        reg.record_mut(h).unwrap().emit_step = 2;
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        update_radiation_emitters(&mut sd, &mut reg, 1.0 / 3.0, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.radiation.get(12), 0.0, "中心未被环步写入");
            assert_eq!(u.radiation.get(13), 0.0, "内部 (1,0) 不受辐射（环非填充）");
            assert!(
                (u.radiation.get(14) - 220.0).abs() < 1e-3,
                "环上轴向 (2,0) = Loop B comp 110×2（SetPixel4 轴向重复写），got {}",
                u.radiation.get(14)
            );
            assert_eq!(u.radiation.get(15), 0.0, "外侧轴向 (3,0) 强度 0（整格边界）");
            assert!(
                (u.radiation.get(21) - 38.0).abs() < 1e-3,
                "对角内侧 (1,1) = 19+19 = 38，got {}",
                u.radiation.get(21)
            );
            assert!(
                (u.radiation.get(22) - 175.0).abs() < 1e-3,
                "对角 (2,1) = 84+91 = 175，got {}",
                u.radiation.get(22)
            );
            assert!(
                (u.radiation.get(30) - 52.0).abs() < 1e-3,
                "对角 (2,2) = 26+26 = 52，got {}",
                u.radiation.get(30)
            );
        }
        DestroyElementsTable();
    }

    /// 任务 3d：stepTimer 小数累加器定时——dt<step_time 时不发射，
    /// 累加够一个 step_time 后补发 1 次（每帧 0~1 次）。
    #[test]
    fn emitter_constant_ticks_paced_by_step_timer() {
        let _lock = LIB_TESTS_LOCK.lock();
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
        w.write_float(1.0); // rate=1
        w.write_float(1.0); // speed=1 → step_time = 1/2 = 0.5
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(0); // Constant
        let msg = AddRadiationEmitterMsg::from_bytes(&w.into_bytes()).unwrap();
        reg.register(&mut sd, &msg);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        // dt=0.2 < step_time=0.5：前 3 帧不发（stepTimer 0.2/0.4/0.6），
        // 第 4 帧 stepTimer=0.6 ≥ 0.5 → 补发 1 次 → 中心 +100。
        for _ in 0..3 {
            update_radiation_emitters(&mut sd, &mut reg, 0.2, bounds);
        }
        unsafe {
            assert_eq!(
                (*sd.updated_cells.ptr).radiation.get(12),
                0.0,
                "stepTimer 未攒够 step_time 前不发射"
            );
        }
        update_radiation_emitters(&mut sd, &mut reg, 0.2, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                (u.radiation.get(12) - 100.0).abs() < 1e-3,
                "stepTimer 攒够后补发 1 次 = 100，got {}",
                u.radiation.get(12)
            );
        }
        DestroyElementsTable();
    }

    /// 任务 3b：PulsingAveraged——step 0 中心 = emitRads/面积（radius 2×2 → 面积1 → 110）。
    #[test]
    fn emitter_pulsing_averaged_center_step() {
        let _lock = LIB_TESTS_LOCK.lock();
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

    /// 任务 4a：DLC 门控——radiationEnabled=false → update 零执行。
    #[test]
    fn emitter_gated_by_enabled_flag() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_table();
        let mut sd = make_sim();
        sd.radiation_enabled = false;
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
        w.write_float(1.0);
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(0);
        let msg = AddRadiationEmitterMsg::from_bytes(&w.into_bytes()).unwrap();
        reg.register(&mut sd, &msg);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        update_radiation_emitters(&mut sd, &mut reg, 1.0, bounds);
        unsafe {
            assert_eq!(
                (*sd.updated_cells.ptr).radiation.get(12),
                0.0,
                "单星门控：发射器零执行"
            );
        }
        DestroyElementsTable();
    }

    /// 任务 4b：帧消息处理——adds 36B 注册进全局管理器。
    #[test]
    fn emitter_frame_messages_register_global() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_table();
        let mut sd = make_sim();
        clear_radiation_emitters();
        let mut w = BinaryBufferWriter::new();
        w.write_int(-1);
        w.write_int(3);
        w.write_short(2);
        w.write_short(2);
        w.write_float(110.0);
        w.write_float(0.0);
        w.write_float(1.0);
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(0);
        let bytes = w.into_bytes();
        let mut frame = crate::a_framework::sim_frame_manager::SimFrameInfo::default();
        for &b in &bytes {
            frame.radiation_emitter_messages.adds.push_unchecked(b);
        }
        process_radiation_emitter_messages(&frame, &mut sd);
        assert_eq!(radiation_emitter_count(), 1, "36B Add 应注册 1 个发射器");
        clear_radiation_emitters();
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
        let _lock = LIB_TESTS_LOCK.lock();
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
        // 原版 Modify：emitSpeed = min(emitSpeed, emitRate)（rate=0 → 有效 speed=0），
        // 且不重置 emitTimer/emitStep/emitStepTimer。
        assert_eq!(reg.record(h).unwrap().emit_speed, 0.0, "speed 钳制 min(speed,rate)");
        // Remove（延迟两轮）→ 记录失效 + 回调推 -1
        reg.remove(h, 9);
        reg.release_queued_handles(&mut sd);
        assert!(reg.record(h).is_some(), "第一轮不释放");
        reg.release_queued_handles(&mut sd);
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
