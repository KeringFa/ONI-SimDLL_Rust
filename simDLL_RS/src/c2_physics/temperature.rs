//! 温度模块（C2）：相变与建筑换热依赖。
//!
//! 对照原版 `SimDLL_Source.c`：
//! - `SimEvents::SpawnOre`（L133682-133737）：矿渣事件；
//! - `DoStateTransition`（L133261-133737，任务 4）；
//! - `UpdateTemperature` / `UpdateTemperatureForBackwallExchange`（C2 后续）。

use crate::a_framework::game_data::SpawnOreInfo;
use crate::a_framework::sim_data::SimData;

/// SimEvents::SpawnOre（原版 L133682-133737）。只推事件，不扣质量/疾病（调用方扣）。
///
/// - mass ≤ 0 → false；
/// - gameCell = (row-1)×(width-2) + (col-1)，越界 → false；
/// - 非 debugEditing 且 visibleGrid[gameCell]==0 → false；
/// - 推 `spawn_ore_info`，返回 true。
pub(crate) fn spawn_ore(
    sim: &mut SimData,
    cell: usize,
    elem_idx: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
    is_debug_editing: bool,
) -> bool {
    if mass <= 0.0 {
        return false;
    }
    let width = sim.width as usize;
    let row = cell / width;
    let col = cell % width;
    let game_cell = (row as i32 - 1) * (width as i32 - 2) + col as i32 - 1;
    if game_cell < 0 || game_cell >= sim.num_game_cells {
        return false;
    }
    if !is_debug_editing && !sim.visible_grid.ptr.is_null() {
        if unsafe { *sim.visible_grid.ptr.add(game_cell as usize) } == 0 {
            return false;
        }
    }
    let events = unsafe { &mut *sim.sim_events.ptr };
    events.spawn_ore_info.push(SpawnOreInfo {
        cell_idx: game_cell,
        elem_idx,
        disease_idx,
        pad: 0,
        mass,
        temperature,
        disease_count,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;
    use crate::LIB_TESTS_LOCK;

    fn make_sim() -> SimData {
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, true);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd
    }

    #[test]
    fn spawn_ore_pushes_event_with_game_cell() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        let mut sd = make_sim();
        // cell14 = sim(row2,col2) → game = 1×4+1 = 5
        assert!(spawn_ore(&mut sd, 14, 7, 5.0, 300.0, 0xFF, 0, false));
        let info = {
            let events = unsafe { &*sd.sim_events.ptr };
            events.spawn_ore_info.as_slice().to_vec()
        };
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].cell_idx, 5);
        assert_eq!(info[0].elem_idx, 7);
        assert_eq!(info[0].mass, 5.0);
        assert_eq!(info[0].temperature, 300.0);
        // mass ≤ 0 → false
        assert!(!spawn_ore(&mut sd, 14, 7, 0.0, 300.0, 0xFF, 0, false));
    }

    #[test]
    fn spawn_ore_skips_invisible_cell_unless_debug_editing() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        let mut sd = make_sim();
        // game cell 5 置为不可见（visible_grid 初始全 0xFF 兜底）
        unsafe {
            *sd.visible_grid.ptr.offset(5) = 0;
        }
        assert!(!spawn_ore(&mut sd, 14, 7, 5.0, 300.0, 0xFF, 0, false));
        assert!(spawn_ore(&mut sd, 14, 7, 5.0, 300.0, 0xFF, 0, true));
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.spawn_ore_info.len(), 1);
    }

    #[test]
    fn spawn_ore_rejects_out_of_bounds_game_cell() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        let mut sd = make_sim();
        // sim 尺寸 6×6（含边界）：内部行 1..4；cell0 = row0 边界 → game = -5 → false
        assert!(!spawn_ore(&mut sd, 0, 7, 5.0, 300.0, 0xFF, 0, true));
    }
}
