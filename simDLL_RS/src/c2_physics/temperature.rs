//! 温度模块（C2）：相变与建筑换热依赖。
//!
//! 对照原版 `SimDLL_Source.c`：
//! - `SimEvents::SpawnOre`（L133682-133737）：矿渣事件；
//! - `DoStateTransition`（L133261-133737，任务 4）；
//! - `UpdateTemperature` / `UpdateTemperatureForBackwallExchange`（C2 后续）。

use crate::a_framework::game_data::SpawnOreInfo;
use crate::a_framework::sim_data::SimData;
use crate::b_elements::element::{ElementTemperatureData, INVALID_ELEMENT_INDEX};
use crate::b_elements::elements_table::{get_element_by_idx, get_element_temperature_data};
use crate::c2_physics::liquid_flow::{clear_cell, push_substance_change};
use crate::a_framework::game_data::{CellMeltedInfo, SpawnFallingLiquidInfo};

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

/// DoStateTransition（原版 L133261-133737）。每格换热/升温后调用，返回是否发生转变。
///
/// - 高温侧（T ≥ highTemp+3 且 highTempTransitionIdx 有效）：温度= max(0,T-1.5)，元素切换，
///   矿渣扣质量/疾病；`updated.properties[cell] & 0x40` → cellMeltedInfo + timers |= 0x1F；ChangeSubstance。
/// - 低温侧（T < lowTemp-3 且 lowTempTransitionIdx 有效）：温度= max(0,T+1.5)，矿渣扣减；
///   目标固态且 mass/default_mass ≤ 0.8 → 清格出矿渣；否则保格切换；
///   目标非固态且上格非固态+可见 → 液滴事件 + 清格；否则保格切换。
pub(crate) fn do_state_transition(
    sim: &mut SimData,
    cell: usize,
    etd: &ElementTemperatureData,
) -> bool {
    if sim.updated_cells.ptr.is_null() {
        return false;
    }
    let width = sim.width as usize;
    let game_w = (width as i32) - 2;
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if cell >= updated.mass.len() || updated.mass.get(cell) <= 0.0 {
        return false;
    }
    let t = updated.temperature.get(cell);
    let game_cell_of = |c: usize| -> i32 {
        let row = c / width;
        let col = c % width;
        (row as i32 - 1) * game_w + col as i32 - 1
    };

    if t >= etd.low_temp - 3.0 || etd.low_temp_transition_idx == INVALID_ELEMENT_INDEX {
        // 高温侧（沸腾/熔化）
        if t <= etd.high_temp + 3.0 || etd.high_temp_transition_idx == INVALID_ELEMENT_INDEX {
            return false;
        }
        updated.temperature.set(cell, (t - 1.5).max(0.0));
        updated.element_idx.set(cell, etd.high_temp_transition_idx);
        if etd.high_temp_transition_ore_idx != INVALID_ELEMENT_INDEX {
            let ore_mass = etd.high_temp_transition_ore_mass_conversion * updated.mass.get(cell);
            if ore_mass > 0.001 {
                let converted =
                    (updated.disease_count.get(cell) as f32 * etd.high_temp_transition_ore_mass_conversion)
                        as i32;
                if spawn_ore(
                    sim,
                    cell,
                    etd.high_temp_transition_ore_idx,
                    ore_mass,
                    updated.temperature.get(cell),
                    updated.disease_idx.get(cell),
                    converted,
                    true,
                ) {
                    updated.mass.set(cell, updated.mass.get(cell) - ore_mass);
                    updated.disease_count.set(cell, updated.disease_count.get(cell) - converted);
                }
            }
        }
        if updated.properties.get(cell) & 0x40 != 0 {
            let gc = game_cell_of(cell);
            if gc >= 0 && gc < sim.num_game_cells {
                let events = unsafe { &mut *sim.sim_events.ptr };
                events.cell_melted_info.push(CellMeltedInfo { game_cell: gc as u32 });
                if !sim.timers.ptr.is_null() {
                    unsafe {
                        (*sim.timers.ptr.offset(cell as isize)).stable_cell_ticks |= 0x1F;
                    }
                }
            }
        }
        push_substance_change(sim, cell);
        return true;
    }

    // 低温侧（冻结/冷凝）
    updated.temperature.set(cell, (t + 1.5).max(0.0));
    if etd.low_temp_transition_ore_idx != INVALID_ELEMENT_INDEX {
        let ore_mass = etd.low_temp_transition_ore_mass_conversion * updated.mass.get(cell);
        if ore_mass > 0.001 {
            let converted =
                (updated.disease_count.get(cell) as f32 * etd.low_temp_transition_ore_mass_conversion)
                    as i32;
            if spawn_ore(
                sim,
                cell,
                etd.low_temp_transition_ore_idx,
                ore_mass,
                updated.temperature.get(cell),
                updated.disease_idx.get(cell),
                converted,
                true,
            ) {
                updated.mass.set(cell, updated.mass.get(cell) - ore_mass);
                updated.disease_count.set(cell, updated.disease_count.get(cell) - converted);
            }
        }
    }
    let Some(transition_etd) = get_element_temperature_data(etd.low_temp_transition_idx) else {
        push_substance_change(sim, cell);
        return true;
    };
    if transition_etd.state & 3 == 3 {
        // 目标固态：质量比 ≤ 0.8 → 清格出矿渣；否则保格切换
        let mass = updated.mass.get(cell);
        if mass / transition_etd.default_mass <= 0.8
            && spawn_ore(
                sim,
                cell,
                etd.low_temp_transition_idx,
                mass,
                updated.temperature.get(cell),
                updated.disease_idx.get(cell),
                updated.disease_count.get(cell),
                sim.debug_properties.is_debug_editing,
            )
        {
            clear_cell(sim, updated, cell);
            push_substance_change(sim, cell);
            return true;
        }
        updated.element_idx.set(cell, etd.low_temp_transition_idx);
        push_substance_change(sim, cell);
        return true;
    }
    // 目标非固态：上格（稳定缓冲 cells.properties）非固态且可见 → 液滴事件 + 清格
    if !sim.headless && cell >= width {
        let above = cell - width;
        let cells = unsafe { &*sim.cells.ptr };
        if above < cells.properties.len() && cells.properties.get(above) & 2 == 0 {
            let gc = game_cell_of(cell);
            let visible = sim.visible_grid.ptr.is_null()
                || unsafe { *sim.visible_grid.ptr.add(gc as usize) } != 0
                || sim.debug_properties.is_debug_editing;
            if visible {
                if let Some(elem) = get_element_by_idx(etd.low_temp_transition_idx) {
                    if elem.low_temp - 3.0 <= t && t <= elem.high_temp + 3.0 {
                        let events = unsafe { &mut *sim.sim_events.ptr };
                        events.spawn_liquid_info.push(SpawnFallingLiquidInfo {
                            cell_idx: gc,
                            element_idx: etd.low_temp_transition_idx,
                            disease_idx: updated.disease_idx.get(cell),
                            pad: 0,
                            mass: updated.mass.get(cell),
                            temperature: updated.temperature.get(cell),
                            disease_count: updated.disease_count.get(cell),
                        });
                        clear_cell(sim, updated, cell);
                        push_substance_change(sim, cell);
                        return true;
                    }
                }
            }
        }
    }
    updated.element_idx.set(cell, etd.low_temp_transition_idx);
    push_substance_change(sim, cell);
    true
}

/// CalculateTemperatureExchange_precise（原版 L133110-133170）。
///
/// heat = min((T_b−T_a)×k×dt, (T_b−T_a)×rate×HC_a, (T_b−T_a)×rate×HC_b)；
/// 新温度 = ±heat/HC + 原温度，钳制不越过加权均衡；double 计算、f32 输出。
/// 返回 (new_第一参, new_第二参)。
pub(crate) fn calculate_temperature_exchange_precise(
    rate: f32,
    k: f32,
    dt: f32,
    t_a: f32,
    hc_a: f32,
    t_b: f32,
    hc_b: f32,
) -> (f32, f32) {
    let (t_a, hc_a) = (t_a as f64, hc_a as f64);
    let (t_b, hc_b) = (t_b as f64, hc_b as f64);
    let (rate, k, dt) = (rate as f64, k as f64, dt as f64);
    if hc_a <= 0.0 || hc_b <= 0.0 {
        return (t_a as f32, t_b as f32);
    }
    let equilibrium = (t_a * hc_a + t_b * hc_b) / (hc_a + hc_b);
    let d = t_b - t_a;
    let heat = (d * k * dt).min(d * rate * hc_a).min(d * rate * hc_b);
    let mut new_a = heat / hc_a + t_a;
    if equilibrium <= new_a {
        new_a = equilibrium;
    }
    let mut new_b = -heat / hc_b + t_b;
    if new_b <= equilibrium {
        new_b = equilibrium;
    }
    (new_a as f32, new_b as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::b_elements::element::Element;
    use crate::LIB_TESTS_LOCK;

    fn make_sim() -> SimData {
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd
    }

    /// 元素表：dummy(0) / 水 water(1) / 冰 ice(2) / 蒸汽 steam(3)。
    fn init_phase_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.temperature_data.clear();

        let mut water = ElementTemperatureData::default();
        water.state = 2;
        water.low_temp = 0.0;
        water.high_temp = 100.0;
        water.low_temp_transition_idx = 2; // → 冰
        water.high_temp_transition_idx = 3; // → 蒸汽

        let mut ice = ElementTemperatureData::default();
        ice.state = 3;
        ice.default_mass = 1000.0;

        let mut steam = ElementTemperatureData::default();
        steam.state = 1;
        steam.low_temp_transition_idx = 1; // → 水
        steam.low_temp = 100.0;
        steam.high_temp = 999.0;

        table.temperature_data.push(ElementTemperatureData::default()); // 0 dummy
        table.temperature_data.push(water); // 1
        table.temperature_data.push(ice); // 2
        table.temperature_data.push(steam); // 3

        let mut e_water = Element::default();
        e_water.state = 2;
        e_water.low_temp = 0.0;
        e_water.high_temp = 100.0;
        let mut e_ice = Element::default();
        e_ice.state = 3;
        let mut e_steam = Element::default();
        e_steam.state = 1;
        e_steam.low_temp = 100.0;
        e_steam.high_temp = 999.0;
        table.elements.push(Element::default()); // 0 dummy
        table.elements.push(e_water); // 1
        table.elements.push(e_ice); // 2
        table.elements.push(e_steam); // 3
    }

    fn fill(sd: &mut SimData, cell: usize, elem: u16, mass: f32, temp: f32) {
        let u = unsafe { &mut *sd.updated_cells.ptr };
        u.element_idx.set(cell, elem);
        u.mass.set(cell, mass);
        u.temperature.set(cell, temp);
    }

    #[test]
    fn high_transition_switches_element_and_emits_substance_change() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 14, 1, 10.0, 105.0); // 水 105℃ ≥ high+3
        let etd = get_element_temperature_data(1).unwrap();
        assert!(do_state_transition(&mut sd, 14, &etd));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 3); // → 蒸汽
        assert!((u.temperature.get(14) - 103.5).abs() < 1e-4); // max(0, 105-1.5)
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.substance_change_info.len(), 1);
    }

    #[test]
    fn low_transition_solid_clears_cell_when_mass_ratio_small() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 14, 1, 100.0, -5.0); // 水 -5℃ < low-3，低变=冰（固态）
        let etd = get_element_temperature_data(1).unwrap();
        assert!(do_state_transition(&mut sd, 14, &etd));
        let u = unsafe { &*sd.updated_cells.ptr };
        // 100/1000 = 0.1 ≤ 0.8 → 清格出矿渣（spawn_ore_info 1 条，格子变 vacuum）
        assert_eq!(u.element_idx.get(14), sd.vacuum_element_idx);
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.spawn_ore_info.len(), 1);
        assert_eq!(events.spawn_ore_info.get(0).elem_idx, 2);
    }

    #[test]
    fn low_transition_non_solid_drips_liquid_when_visible() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 20, 3, 5.0, 95.0); // 蒸汽 95℃ < low-3，低变=水（非固态）
        let etd = get_element_temperature_data(3).unwrap();
        assert!(do_state_transition(&mut sd, 20, &etd));
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.spawn_liquid_info.len(), 1, "可见且上格非固态 → 液滴");
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(20), sd.vacuum_element_idx);
    }

    #[test]
    fn transition_gate_keeps_within_plus_minus_three() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 14, 1, 10.0, 102.9); // 100+3 内 → 不转变
        let etd = get_element_temperature_data(1).unwrap();
        assert!(!do_state_transition(&mut sd, 14, &etd));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 1);
        assert_eq!(u.temperature.get(14), 102.9);
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

    #[test]
    fn exchange_precise_conserves_energy_and_clamps_to_equilibrium() {
        // T_a=300 HC=100，T_b=400 HC=100，k=1，rate=0.25，dt=0.2
        let (na, nb) =
            calculate_temperature_exchange_precise(0.25, 1.0, 0.2, 300.0, 100.0, 400.0, 100.0);
        // heat = min(100×1×0.2, 100×0.25×100, 100×0.25×100) = 20
        assert!((na - 300.2).abs() < 1e-3, "a 得热 20/100 → 300.2，got {na}");
        assert!((nb - 399.8).abs() < 1e-3, "b 失热 20/100 → 399.8，got {nb}");
        // 能量守恒：0.2×100 == 0.2×100
        assert!(((na - 300.0) * 100.0 + (nb - 400.0) * 100.0).abs() < 1e-3);
    }

    #[test]
    fn exchange_precise_caps_heat_by_k_and_hc() {
        // 原版约定：precise 只在"冷侧在前"（T_a ≤ T_b）下调用，d ≥ 0。
        // k 极小 → k×dt 上限生效：heat = 100×0.01×0.2 = 0.2
        let (na, nb) =
            calculate_temperature_exchange_precise(0.25, 0.01, 0.2, 300.0, 100.0, 400.0, 100.0);
        assert!((na - 300.002).abs() < 1e-3, "got {na}");
        assert!((nb - 399.998).abs() < 1e-3, "got {nb}");
        // k 极大 → 0.25×HC 上限生效：heat = 100×0.25×100 = 2500 → 325/375，不越过均衡 350
        let (na2, nb2) =
            calculate_temperature_exchange_precise(0.25, 1e9, 0.2, 300.0, 100.0, 400.0, 100.0);
        assert!((na2 - 325.0).abs() < 1e-3, "got {na2}");
        assert!((nb2 - 375.0).abs() < 1e-3, "got {nb2}");
        assert!(na2 <= 350.0 && nb2 >= 350.0);
    }
}
