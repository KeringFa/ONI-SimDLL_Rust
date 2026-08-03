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
use crate::a_framework::game_data::{
    BackwallShouldTransitionInfo, CellMeltedInfo, SpawnFallingLiquidInfo,
};

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

/// 合并导率（原版 UpdateTemperature L133833-133847）：任一侧 insulation 值
/// （ins²×1.53787e-05）<1.0 → min(k_a,k_b)；否则几何平均。
/// 注：ins=255 时 f32 下 iv 舍入为 1.0（非 <1）→ 几何平均分支可达。
fn combine_conductivity(ins_a: u8, tc_a: f32, ins_b: u8, tc_b: f32) -> f32 {
    let ins_a = ins_a as f32;
    let ins_b = ins_b as f32;
    let iv_a = ins_a * ins_a * 1.53787e-05;
    let iv_b = ins_b * ins_b * 1.53787e-05;
    let k_a = iv_a * tc_a;
    let k_b = iv_b * tc_b;
    if iv_a < 1.0 || iv_b < 1.0 {
        k_a.min(k_b)
    } else {
        (k_a * k_b).sqrt() // 原版 logf+logf+expf = 几何平均
    }
}

/// 表面倍率（邻格路径，原版 L133801-133823 内联）：state&3==0 时保持 1.0（无分支命中）。
fn surface_multiplier_pair(this: &ElementTemperatureData, other_state: u8) -> f32 {
    match other_state & 3 {
        1 => this.gas_surface_area_multiplier,
        2 => this.liquid_surface_area_multiplier,
        3 => this.solid_surface_area_multiplier,
        _ => 1.0,
    }
}

/// 表面倍率（背墙路径，原版 GetSurfaceAreaMultiplier L133620）：state&3==0 落 liquid 倍率。
fn surface_multiplier_backwall(this: &ElementTemperatureData, other_state: u8) -> f32 {
    match other_state & 3 {
        1 => this.gas_surface_area_multiplier,
        2 => this.liquid_surface_area_multiplier,
        3 => this.solid_surface_area_multiplier,
        _ => this.liquid_surface_area_multiplier,
    }
}

/// UpdateTemperature（原版 L133737-134004）：读 `cells` 稳定缓冲，写 `updatedCells` 增量 + [1,10000] 钳制。
pub(crate) fn update_temperature_pair(sim: &mut SimData, cell_a: usize, cell_b: usize) {
    if sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
    let (t_a, t_b, hc_a, hc_b, k_total) = {
        let c = unsafe { &*sim.cells.ptr };
        if cell_a >= c.temperature.len() || cell_b >= c.temperature.len() {
            return;
        }
        let (elem_a, elem_b) = (c.element_idx.get(cell_a), c.element_idx.get(cell_b));
        let (Some(etd_a), Some(etd_b)) = (
            get_element_temperature_data(elem_a),
            get_element_temperature_data(elem_b),
        ) else {
            return;
        };
        let (hc_a, hc_b) = (
            c.mass.get(cell_a) * etd_a.specific_heat_capacity,
            c.mass.get(cell_b) * etd_b.specific_heat_capacity,
        );
        if hc_a <= 0.0 || hc_b <= 0.0 {
            return;
        }
        let k = combine_conductivity(
            c.insulation.get(cell_a),
            etd_a.thermal_conductivity,
            c.insulation.get(cell_b),
            etd_b.thermal_conductivity,
        );
        let mult_a = surface_multiplier_pair(&etd_a, etd_b.state);
        let mult_b = surface_multiplier_pair(&etd_b, etd_a.state);
        (
            c.temperature.get(cell_a),
            c.temperature.get(cell_b),
            hc_a,
            hc_b,
            k * mult_a * mult_b,
        )
    };
    // 原版约定：冷侧在前（T_a ≤ T_b）调用 precise；否则 swapped 后映射回 (new_cell, new_neighbor)
    let (new_a, new_b) = if t_a <= t_b {
        calculate_temperature_exchange_precise(0.25, k_total, 0.2, t_a, hc_a, t_b, hc_b)
    } else {
        let (nb, na) =
            calculate_temperature_exchange_precise(0.25, k_total, 0.2, t_b, hc_b, t_a, hc_a);
        (na, nb)
    };
    unsafe {
        let u = &mut *sim.updated_cells.ptr;
        let c = &*sim.cells.ptr;
        let mut va = (new_a - c.temperature.get(cell_a)) + u.temperature.get(cell_a);
        va = va.max(1.0).min(10000.0);
        u.temperature.set(cell_a, va);
        let mut vb = (new_b - c.temperature.get(cell_b)) + u.temperature.get(cell_b);
        vb = vb.max(1.0).min(10000.0);
        u.temperature.set(cell_b, vb);
    }
}

/// UpdateTemperatureForBackwallExchange（原版 L134004-134079）。
/// 格子内容物 ↔ 背墙层元素，因子 0.01；cell 写 updatedCells 增量、背墙温度直接写；
/// 背墙温度越 [low−3, high+3] → backwallShouldTransitionInfo。
pub(crate) fn update_temperature_for_backwall(sim: &mut SimData, cell: usize) {
    if sim.cells.ptr.is_null()
        || sim.updated_cells.ptr.is_null()
        || sim.backwall.ptr.is_null()
        || sim.sim_events.ptr.is_null()
    {
        return;
    }
    let (back_elem, t_cell, hc_cell, t_back, hc_back, k_total) = {
        let c = unsafe { &*sim.cells.ptr };
        let bw = unsafe { &*sim.backwall.ptr };
        if cell >= c.element_idx.len() || cell >= bw.element_idx.len() {
            return;
        }
        let cell_elem = c.element_idx.get(cell);
        let back_elem = bw.element_idx.get(cell);
        // 双方元素非真空/void（原版 L134024-134033）
        if cell_elem == sim.vacuum_element_idx
            || cell_elem == sim.void_element_idx
            || back_elem == sim.vacuum_element_idx
            || back_elem == sim.void_element_idx
        {
            return;
        }
        let (Some(etd_cell), Some(etd_back)) = (
            get_element_temperature_data(cell_elem),
            get_element_temperature_data(back_elem),
        ) else {
            return;
        };
        let hc_cell = c.mass.get(cell) * etd_cell.specific_heat_capacity;
        let hc_back = bw.mass.get(cell) * etd_back.specific_heat_capacity;
        if hc_cell <= 0.0 || hc_back <= 0.0 {
            return;
        }
        let ins = c.insulation.get(cell) as f32;
        let iv = ins * ins * 1.53787e-05;
        let k_cell = iv * etd_cell.thermal_conductivity;
        // 原版 L134045-134053：iv ≥ 1.0 → 几何平均；否则 min(k_cell, backTC)
        let k = if iv >= 1.0 {
            (k_cell * etd_back.thermal_conductivity).sqrt()
        } else {
            k_cell.min(etd_back.thermal_conductivity)
        };
        let mult_cell = surface_multiplier_backwall(&etd_cell, etd_back.state);
        let mult_back = surface_multiplier_backwall(&etd_back, etd_cell.state);
        (
            back_elem,
            c.temperature.get(cell),
            hc_cell,
            bw.temperature.get(cell),
            hc_back,
            k * mult_cell * mult_back,
        )
    };
    // 原版 wrapper CalculateTemperatureExchange（L133079）自动按温度排序，返回 (new_back, new_cell)
    let (new_back, new_cell) = if t_cell <= t_back {
        let (nb, nc) = calculate_temperature_exchange_precise(
            0.01,
            k_total,
            0.2,
            t_cell,
            hc_cell,
            t_back,
            hc_back,
        );
        (nb, nc)
    } else {
        let (nc, nb) = calculate_temperature_exchange_precise(
            0.01,
            k_total,
            0.2,
            t_back,
            hc_back,
            t_cell,
            hc_cell,
        );
        (nb, nc)
    };
    unsafe {
        let u = &mut *sim.updated_cells.ptr;
        let c = &*sim.cells.ptr;
        let mut v = (new_cell - c.temperature.get(cell)) + u.temperature.get(cell);
        v = v.max(1.0).min(10000.0);
        u.temperature.set(cell, v);
        let bw = &mut *sim.backwall.ptr;
        bw.temperature.set(cell, new_back);
        // 背墙相变：越 [low−3, high+3] → backwallShouldTransitionInfo
        if let Some(etd_back) = get_element_temperature_data(back_elem) {
            if new_back < etd_back.low_temp - 3.0 || new_back > etd_back.high_temp + 3.0 {
                let w = sim.width as usize;
                let game_cell =
                    ((cell / w) as i32 - 1) * (w as i32 - 2) + (cell % w) as i32 - 1;
                if game_cell >= 0 && game_cell < sim.num_game_cells {
                    let events = unsafe { &mut *sim.sim_events.ptr };
                    events.backwall_should_transition_info.push(BackwallShouldTransitionInfo {
                        game_cell: game_cell as u32,
                    });
                }
            }
        }
    }
}

/// 邻格换热门控（原版 L144246-144258）：双方元素 ETD 有效、TC>0、state 无 TemperatureInsulated(0x10)。
fn pair_allowed(sim: &SimData, a: usize, b: usize) -> bool {
    let c = unsafe { &*sim.cells.ptr };
    let (Some(etd_a), Some(etd_b)) = (
        get_element_temperature_data(c.element_idx.get(a)),
        get_element_temperature_data(c.element_idx.get(b)),
    ) else {
        return false;
    };
    etd_a.thermal_conductivity > 0.0
        && etd_b.thermal_conductivity > 0.0
        && (etd_a.state | etd_b.state) & 0x10 == 0
}

/// SimUpdateTemperatureTask::InternalDoTask 单线程等价（原版 L144205-144310）。
/// 遍历内部格：背墙换热 → 右邻（ΔT≥1 且 TC>0 且无 0x10）→ 下邻（同门控）→ DoStateTransition。
/// 读 `cells` 稳定缓冲、写 `updatedCells` 增量（opening CopyFrom 后 == 子步初态）。
pub(crate) fn run_temperature_task(sim: &mut SimData) {
    if sim.cells.ptr.is_null()
        || sim.updated_cells.ptr.is_null()
        || sim.backwall.ptr.is_null()
        || sim.sim_events.ptr.is_null()
    {
        return;
    }
    let w = sim.width as usize;
    let h = sim.height as usize;
    if w < 3 || h < 3 {
        return;
    }
    for row in 1..h - 1 {
        for col in 1..w - 1 {
            let cell = row * w + col;
            // 1. 背墙换热（cells.mass>0 且 backwall.mass>0，原版 L144222-144234）
            {
                let c = unsafe { &*sim.cells.ptr };
                let bw = unsafe { &*sim.backwall.ptr };
                if cell < c.mass.len()
                    && cell < bw.mass.len()
                    && c.mass.get(cell) > 0.0
                    && bw.mass.get(cell) > 0.0
                {
                    update_temperature_for_backwall(sim, cell);
                }
            }
            // 2/3. 右/下邻换热（门控：|ΔT|≥1、双方 TC>0、无 TemperatureInsulated）
            let (t_self, t_right, t_below) = {
                let c = unsafe { &*sim.cells.ptr };
                (
                    c.temperature.get(cell),
                    c.temperature.get(cell + 1),
                    c.temperature.get(cell + w),
                )
            };
            if col + 1 < w - 1 && (t_self - t_right).abs() >= 1.0 && pair_allowed(sim, cell, cell + 1)
            {
                update_temperature_pair(sim, cell, cell + 1);
            }
            if row + 1 < h - 1 && (t_self - t_below).abs() >= 1.0 && pair_allowed(sim, cell, cell + w)
            {
                update_temperature_pair(sim, cell, cell + w);
            }
            // 4. DoStateTransition（每格，原版 L144293-144306）
            let elem = unsafe { (*sim.cells.ptr).element_idx.get(cell) };
            if let Some(etd) = get_element_temperature_data(elem) {
                do_state_transition(sim, cell, &etd);
            }
        }
    }
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

    /// 元素表：elem1 液态水（SHC=4.179、TC=0.609）、elem2 固态砖（SHC=0.8、TC=2.0）。
    fn init_pair_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.temperature_data.clear();
        let mut water = ElementTemperatureData::default();
        water.state = 2;
        water.specific_heat_capacity = 4.179;
        water.thermal_conductivity = 0.609;
        water.gas_surface_area_multiplier = 1.0;
        water.liquid_surface_area_multiplier = 1.0;
        water.solid_surface_area_multiplier = 1.0;
        water.low_temp = 0.0;
        water.high_temp = 1000.0;
        water.low_temp_transition_idx = INVALID_ELEMENT_INDEX;
        water.high_temp_transition_idx = INVALID_ELEMENT_INDEX;
        let mut rock = ElementTemperatureData::default();
        rock.state = 3;
        rock.specific_heat_capacity = 0.8;
        rock.thermal_conductivity = 2.0;
        rock.gas_surface_area_multiplier = 1.0;
        rock.liquid_surface_area_multiplier = 1.0;
        rock.solid_surface_area_multiplier = 1.0;
        rock.low_temp = 0.0;
        rock.high_temp = 1000.0;
        rock.low_temp_transition_idx = INVALID_ELEMENT_INDEX;
        rock.high_temp_transition_idx = INVALID_ELEMENT_INDEX;
        table.temperature_data.push(ElementTemperatureData::default()); // 0 dummy
        table.temperature_data.push(water); // 1
        table.temperature_data.push(rock); // 2
    }

    fn fill_pair_cells(sd: &mut SimData, cells: &[(usize, u16, f32, f32, u8)]) {
        unsafe {
            let c = &mut *sd.cells.ptr;
            let u = &mut *sd.updated_cells.ptr;
            for &(cell, elem, mass, temp, ins) in cells {
                c.element_idx.set(cell, elem);
                c.mass.set(cell, mass);
                c.temperature.set(cell, temp);
                c.insulation.set(cell, ins);
                u.element_idx.set(cell, elem);
                u.mass.set(cell, mass);
                u.temperature.set(cell, temp);
                u.insulation.set(cell, ins);
            }
        }
    }

    #[test]
    fn update_temperature_pair_exchanges_and_conserves_energy() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pair_table();
        let mut sd = make_sim();
        // ins=255：f32 下 iv = 255²×1.53787e-05 舍入为 1.0（非 <1）→ 几何平均路径
        fill_pair_cells(&mut sd, &[(14, 1, 100.0, 300.0, 255), (15, 2, 200.0, 400.0, 255)]);
        update_temperature_pair(&mut sd, 14, 15);
        unsafe {
            let c = &*sd.cells.ptr;
            let u = &*sd.updated_cells.ptr;
            let new_a = u.temperature.get(14);
            let new_b = u.temperature.get(15);
            assert!(new_a > 300.0 && new_b < 400.0, "热格降温冷格升温：{new_a} / {new_b}");
            // 能量守恒：HC_a=417.9，HC_b=160
            let da = (new_a - c.temperature.get(14)) * 417.9;
            let db = (new_b - c.temperature.get(15)) * 160.0;
            assert!((da + db).abs() < 1e-2, "能量不守恒：{da} + {db}");
            // 数值核对：几何平均 k = sqrt(0.609×2.0)（iv=1.0），heat = 100×k×0.2
            let iv = 255f32 * 255f32 * 1.53787e-05;
            assert!(iv >= 1.0, "f32 下 iv 应舍入为 ≥1.0，got {iv}");
            let k = (iv * 0.609 * iv * 2.0).sqrt();
            let heat = 100.0 * k * 0.2;
            assert!((new_a - (300.0 + heat / 417.9)).abs() < 1e-2, "got {new_a}");
        }
    }

    #[test]
    fn update_temperature_pair_applies_surface_multiplier() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pair_table();
        let mut sd = make_sim();
        fill_pair_cells(&mut sd, &[(14, 1, 100.0, 300.0, 255), (15, 2, 200.0, 400.0, 255)]);
        update_temperature_pair(&mut sd, 14, 15);
        let d1 = unsafe { (*sd.updated_cells.ptr).temperature.get(14) } - 300.0;
        // 把水的 solid 倍率调高到 10 → 换热应放大约 10 倍
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data[1].solid_surface_area_multiplier = 10.0;
        }
        let mut sd2 = make_sim();
        fill_pair_cells(&mut sd2, &[(14, 1, 100.0, 300.0, 255), (15, 2, 200.0, 400.0, 255)]);
        update_temperature_pair(&mut sd2, 14, 15);
        let d2 = unsafe { (*sd2.updated_cells.ptr).temperature.get(14) } - 300.0;
        assert!(d1 > 0.0 && d2 > 0.0);
        assert!((d2 / d1 - 10.0).abs() < 0.5, "倍率未生效：d1={d1} d2={d2}");
    }

    #[test]
    fn backwall_exchange_writes_both_and_emits_transition() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pair_table();
        let mut sd = make_sim();
        unsafe {
            let c = &mut *sd.cells.ptr;
            let u = &mut *sd.updated_cells.ptr;
            let bw = &mut *sd.backwall.ptr;
            // 格 14：液态水 300°C；背墙：固态砖 500°C（elem2，默认 highTemp=0 → 越界触发事件）
            c.element_idx.set(14, 1);
            c.mass.set(14, 100.0);
            c.temperature.set(14, 300.0);
            c.insulation.set(14, 255);
            u.element_idx.set(14, 1);
            u.mass.set(14, 100.0);
            u.temperature.set(14, 300.0);
            bw.element_idx.set(14, 2);
            bw.mass.set(14, 200.0);
            bw.temperature.set(14, 500.0);
        }
        // 相变事件：把砖的 highTemp 调低到 0 → 背墙 500°C 越 high+3
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data[2].high_temp = 0.0;
        }
        update_temperature_for_backwall(&mut sd, 14);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            let bw = &*sd.backwall.ptr;
            let new_cell = u.temperature.get(14);
            let new_back = bw.temperature.get(14);
            assert!(new_cell > 300.0, "格升温（背墙更热），got {new_cell}");
            assert!(new_back < 500.0, "背墙降温，got {new_back}");
            assert!(new_back > 300.0);
            // 背墙温度被直接写（非增量）
            assert_eq!(bw.temperature.get(14), new_back);
            // 背墙 500°C > 0+3 → 相变事件
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.backwall_should_transition_info.len(), 1);
        }
    }

    #[test]
    fn backwall_exchange_skips_vacuum_or_void() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pair_table();
        let mut sd = make_sim();
        unsafe {
            let c = &mut *sd.cells.ptr;
            let u = &mut *sd.updated_cells.ptr;
            let bw = &mut *sd.backwall.ptr;
            c.element_idx.set(14, 0); // 真空
            c.mass.set(14, 0.0);
            c.temperature.set(14, 0.0);
            u.element_idx.set(14, 0);
            u.mass.set(14, 0.0);
            u.temperature.set(14, 0.0);
            bw.element_idx.set(14, 2);
            bw.mass.set(14, 200.0);
            bw.temperature.set(14, 500.0);
        }
        update_temperature_for_backwall(&mut sd, 14);
        unsafe {
            let bw = &*sd.backwall.ptr;
            assert_eq!(bw.temperature.get(14), 500.0, "真空格不换热");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.backwall_should_transition_info.len(), 0);
        }
    }

    #[test]
    fn temperature_task_exchanges_adjacent_pairs_with_gates() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pair_table();
        let mut sd = make_sim();
        // cell14(row2,col2 水 300) 与 cell15(row2,col3 砖 400) 相邻（ΔT=100 ≥ 1）→ 换热；
        // cell20(row3,col2 水 300) 与 cell26(row4,col2 水 300) 同温（ΔT=0）→ 不换热
        fill_pair_cells(
            &mut sd,
            &[
                (14, 1, 100.0, 300.0, 255),
                (15, 2, 200.0, 400.0, 255),
                (20, 1, 100.0, 300.0, 255),
                (26, 1, 100.0, 300.0, 255),
            ],
        );
        run_temperature_task(&mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(u.temperature.get(14) > 300.0, "水平相邻应换热");
            assert!(u.temperature.get(15) < 400.0, "水平相邻应换热");
            assert_eq!(u.temperature.get(20), 300.0, "ΔT=0 不换热");
            assert_eq!(u.temperature.get(26), 300.0, "ΔT=0 不换热");
        }
    }

    #[test]
    fn temperature_task_skips_temperature_insulated_state() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pair_table();
        // 砖加 TemperatureInsulated(0x10) 位
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            let mut rock = table.temperature_data[2];
            rock.state = 3 | 0x10;
            table.temperature_data[2] = rock;
        }
        let mut sd = make_sim();
        fill_pair_cells(&mut sd, &[(14, 1, 100.0, 300.0, 255), (15, 2, 200.0, 400.0, 255)]);
        run_temperature_task(&mut sd);
        unsafe {
            assert_eq!(
                (*sd.updated_cells.ptr).temperature.get(14),
                300.0,
                "绝缘元素不换热"
            );
            assert_eq!((*sd.updated_cells.ptr).temperature.get(15), 400.0);
        }
    }
}
