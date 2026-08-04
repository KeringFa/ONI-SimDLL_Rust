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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::BinaryBufferWriter;
    use crate::a_framework::sim_data::SimData;
    use crate::LIB_TESTS_LOCK;

    fn make_sim() -> SimData {
        SimData::new_for_allocate(8, 8, 1, true, false)
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
