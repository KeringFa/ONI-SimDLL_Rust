//! ElementEmitter（元素发射器）组件。
//!
//! 对照原版 `SimDLL_Source.c`：
//! - `ElementEmitter::Emit`（04_temperature.c L1495-1609）、`Modify`（L1610）、
//!   `Register`（L1679）、`TryEmit`（L1792）、`Update`（L1966）；
//! - `GetReachableCells`（11_msvcrt_ignored.c L34451）；
//! - CopySimDataToGame emittedMassInfo 拷贝+清零（11_msvcrt_ignored.c L31626-31648）。

use crate::a_framework::game_data::{ComponentStateChangedMessage, EmittedMassInfo};
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

    /// Register（原版 L1679）：分配记录 + 返回 handle。
    pub fn add(&mut self, msg: &AddElementEmitterMsg) -> i32 {
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
        ((self.versions[index] as i32) << 24) | (index as i32 & 0xffffff)
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
pub fn add_element_emitter(msg: &AddElementEmitterMsg) -> i32 {
    ELEMENT_EMITTER_MANAGER.lock().add(msg)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;

    #[test]
    fn add_returns_versioned_handle_and_modify_writes_fields() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        reset_for_test();
        let msg = AddElementEmitterMsg {
            max_pressure: 2.0,
            callback_idx: 7,
            on_blocked_cb: 8,
            on_unblocked_cb: 9,
        };
        let h = add_element_emitter(&msg);
        assert_eq!(h & 0xffffff, 0, "首个 handle index 0");
        let sim = SimData::new_for_allocate(6, 6, 1, false, false);
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        reset_for_test();
        let h = add_element_emitter(&AddElementEmitterMsg::default());
        let sim = SimData::new_for_allocate(6, 6, 1, false, false);
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        reset_for_test();
        let h = add_element_emitter(&AddElementEmitterMsg::default());
        assert!(remove_element_emitter(h));
        assert!(!remove_element_emitter(h), "重复移除应失败");
        let h2 = add_element_emitter(&AddElementEmitterMsg::default());
        assert_ne!(h2, h, "版本应递增");
        assert_eq!(h2 & 0xffffff, h & 0xffffff, "index 复用");
    }
}
