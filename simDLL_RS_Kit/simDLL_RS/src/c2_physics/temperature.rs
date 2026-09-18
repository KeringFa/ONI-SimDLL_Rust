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
    crate::c2_physics::region_events::emit_spawn_ore(
        sim,
        SpawnOreInfo {
            cell_idx: game_cell,
            elem_idx,
            disease_idx,
            pad: 0,
            mass,
            temperature,
            disease_count,
        },
    );
    true
}

/// DoLoadTimeStateTransition — 加载时温度状态修正（原版 04_temperature.c L7-73）。
/// 读取 updated_cells：温度越界 [lowTemp−3, highTemp+3] 时 ±1.5 换元素，
/// 使存档中轻微越界的格子回到相变窗口内。
pub(crate) fn do_load_time_state_transition(sim: &mut SimData, cell: usize) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if cell >= updated.element_idx.len() || cell >= updated.temperature.len() {
        return;
    }
    let elem = updated.element_idx.get(cell);
    let Some(etd) = get_element_temperature_data(elem) else {
        return;
    };
    let mut temp = updated.temperature.get(cell);
    // 原版 L22-24：temp >= lowTemp−3 或无低温转换目标 → 只查高温侧
    if temp >= etd.low_temp - 3.0 || etd.low_temp_transition_idx == INVALID_ELEMENT_INDEX {
        // 原版 L25-32：highTemp+3 < temp 且有高温转换目标 → 高温转换（temp −1.5、换元素）
        if etd.high_temp + 3.0 < temp && etd.high_temp_transition_idx != INVALID_ELEMENT_INDEX {
            if get_element_temperature_data(etd.high_temp_transition_idx).is_none() {
                tracing::warn!(
                    cell,
                    target = etd.high_temp_transition_idx,
                    "DoLoadTimeStateTransition: invalid high transition target"
                );
                return;
            }
            temp -= 1.5;
            updated.temperature.set(cell, temp);
            updated.element_idx.set(cell, etd.high_temp_transition_idx);
        }
    } else {
        // 原版 L33-39：低温转换（temp +1.5、换元素）
        if get_element_temperature_data(etd.low_temp_transition_idx).is_none() {
            tracing::warn!(
                cell,
                target = etd.low_temp_transition_idx,
                "DoLoadTimeStateTransition: invalid low transition target"
            );
            return;
        }
        temp += 1.5;
        updated.temperature.set(cell, temp);
        updated.element_idx.set(cell, etd.low_temp_transition_idx);
    }
    if temp > 10000.0 {
        tracing::warn!(
            cell,
            temp,
            "DoLoadTimeStateTransition: temperature exceeds SIM_MAX_TEMPERATURE"
        );
    }
}

/// DoPartialHeatTransition — 部分沸腾/分批相变（原版 04_temperature.c L2105-2471）。
/// 大质量格（≥5kg）被极热邻格加热：抽邻格热量 heat=(high+3−本格温)×TC×5，
/// 把恰好 5kg 转化为高温相变目标（气体→排下方格；液体→SpawnFallingLiquid）；
/// 失败全额恢复（质量/病菌/邻格温度）并返回 false。
pub(crate) fn do_partial_heat_transition(sim: &mut SimData, cell: usize, neighbor: usize) -> bool {
    let width = sim.width as usize;
    let total = width * sim.height as usize;
    if cell >= total || neighbor >= total {
        return false;
    }
    if sim.updated_cells.ptr.is_null() || sim.sim_events.ptr.is_null() {
        return false;
    }
    let updated = unsafe { &*sim.updated_cells.ptr };
    // 原版 L2138：mass < 5.0 → false（== 5.0 放行）
    let mass_cell = updated.mass.get(cell);
    if mass_cell < 5.0 {
        return false;
    }
    let cell_elem = updated.element_idx.get(cell);
    let neigh_elem = updated.element_idx.get(neighbor);
    let (Some(cell_el), Some(neigh_el)) = (
        get_element_by_idx(cell_elem),
        get_element_by_idx(neigh_elem),
    ) else {
        return false;
    };
    // 原版 L2146：cell 高温转换目标（Element offset 6）
    let target = cell_el.high_temp_transition_idx;
    if target == INVALID_ELEMENT_INDEX {
        return false;
    }
    let Some(target_el) = get_element_by_idx(target) else {
        return false;
    };
    let cell_high = cell_el.high_temp;
    let neigh_low = neigh_el.low_temp;
    // Original L2156/L2160 reads Element@0x0c = specificHeatCapacity
    // (NOT thermalConductivity@0x10). 2026-08-10 fix: using TC made the
    // heat-affordability gate far too permissive, so the forced transition
    // fired while the oil was still ~220C; original waits until ~350C.
    let cell_shc = cell_el.specific_heat_capacity;
    let neigh_shc = neigh_el.specific_heat_capacity;
    let cell_temp = updated.temperature.get(cell);
    let neigh_temp = updated.temperature.get(neighbor);
    let f22 = cell_high + 3.0;
    // 原版 L2147-2152：邻格温度 > cell_high+3；邻格温度 >= neigh_low+13；本格温度 < cell_high−3
    if neigh_temp <= f22 {
        return false;
    }
    if neigh_temp < neigh_low + 13.0 {
        return false;
    }
    if cell_high - 3.0 <= cell_temp {
        return false;
    }
    // 原版 L2156：heat = (f22 − cell_temp) × cell_SHC × 5.0
    let heat = (f22 - cell_temp) * cell_shc * 5.0;
    // 原版 L2158-2162：f21 = mass[neighbor] × neigh_SHC；f21×10 < heat → false
    let mass_neigh = updated.mass.get(neighbor);
    let mut f21 = mass_neigh * neigh_shc;
    if f21 * 10.0 < heat {
        return false;
    }
    // 原版 L2165：邻格新温度
    f21 = (neigh_temp * f21 - heat) / f21;
    let disease_cell = updated.disease_count.get(cell);
    let disease_idx_cell = updated.disease_idx.get(cell);
    let i16 = (disease_cell as f32 * (5.0 / mass_cell)) as i32;
    let target_state = target_el.state & 3;

    // —— 目标为气体（原版 L2171-2290）——
    if target_state == 1 {
        // 扣减段：本格 −5kg、病菌按比例、邻格降温
        {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.mass.set(cell, mass_cell - 5.0);
            let remain = disease_cell - i16;
            if remain < 1 {
                crate::c2_physics::liquid_flow::clear_disease(updated, cell);
            } else {
                updated.disease_count.set(cell, remain);
            }
            updated.temperature.set(neighbor, f21);
        }
        let below = cell + width;
        let below_elem = if below < total {
            unsafe { (*sim.updated_cells.ptr).element_idx.get(below) }
        } else {
            INVALID_ELEMENT_INDEX
        };
        let below_is_gas = matches!(
            get_element_by_idx(below_elem),
            Some(e) if (e.state & 3) == 1
        );
        if below_is_gas {
            if crate::c2_physics::liquid_flow::displace_gas(sim, below, below_elem) {
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                updated.element_idx.set(below, target);
                updated.mass.set(below, 5.0);
                updated.temperature.set(below, f22);
                crate::c2_physics::liquid_flow::push_substance_change(sim, below);
                return true;
            }
            // 原版此处进崩溃标签（L2286）；安全镜像：走恢复分支
        } else if crate::c2_physics::liquid_flow::displace_liquid(sim, cell, cell_elem) {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.element_idx.set(cell, target);
            updated.mass.set(cell, 5.0);
            updated.temperature.set(cell, f22);
            crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
            return true;
        }
        // 恢复段（原版 L2264-2268 同款：质量/病菌/邻格温度还原）
        {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.mass.set(cell, mass_cell);
            updated.disease_count.set(cell, disease_cell);
            updated.temperature.set(neighbor, neigh_temp);
        }
        return false;
    }

    // —— 目标为液体（原版 L2291-2370）——
    if target_state == 2 {
        {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.mass.set(cell, mass_cell - 5.0);
            let remain = disease_cell - i16;
            if remain < 1 {
                crate::c2_physics::liquid_flow::clear_disease(updated, cell);
            } else {
                updated.disease_count.set(cell, remain);
            }
            updated.temperature.set(neighbor, f21);
        }
        if crate::c2_physics::liquid_flow::displace_liquid(sim, cell, cell_elem) {
            if crate::c2_physics::liquid_flow::spawn_falling_liquid(
                sim,
                cell,
                target,
                5.0,
                f22,
                disease_idx_cell,
                i16,
            ) {
                return true;
            }
            // 液滴失败 → 就地转化（原版 L2342-2350）
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.element_idx.set(cell, target);
            updated.mass.set(cell, 5.0);
            updated.temperature.set(cell, f22);
            crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
            return true;
        }
        {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.mass.set(cell, mass_cell);
            updated.disease_count.set(cell, disease_cell);
            updated.temperature.set(neighbor, neigh_temp);
        }
        return false;
    }

    tracing::warn!(target_state, "DoPartialHeatTransition: unexpected target state");
    false
}

/// DoPartialMelt — 部分熔化（原版 04_temperature.c L2471-2662）。
/// 极热气体格把相邻固体 5kg 熔为液滴（SpawnFallingLiquid，温度=固体熔点+3）；
/// 液滴失败则气体格就地转为熔融液体；固体格扣质量/病菌，扣尽则 Evaporate。
pub(crate) fn do_partial_melt(sim: &mut SimData, cell: usize, neighbor: usize) -> bool {
    let width = sim.width as usize;
    let total = width * sim.height as usize;
    if cell >= total || neighbor >= total {
        return false;
    }
    if sim.updated_cells.ptr.is_null() || sim.sim_events.ptr.is_null() {
        return false;
    }
    let updated = unsafe { &*sim.updated_cells.ptr };
    let cell_elem = updated.element_idx.get(cell);
    let neigh_elem = updated.element_idx.get(neighbor);
    let (Some(cell_el), Some(neigh_el)) = (
        get_element_by_idx(cell_elem),
        get_element_by_idx(neigh_elem),
    ) else {
        return false;
    };
    // 原版 L2495-2499：cell state&3 <= 1（真空/气体），neighbor state&3 == 3（固体）
    if (cell_el.state & 3) > 1 || (neigh_el.state & 3) != 3 {
        return false;
    }
    // 原版 L2500-2502：neighbor 质量 > 5.0
    let mass_neigh = updated.mass.get(neighbor);
    if mass_neigh <= 5.0 {
        return false;
    }
    // 原版 L2503-2506：neighbor props & 0x48 == 0
    if updated.properties.get(neighbor) & 0x48 != 0 {
        return false;
    }
    let cell_temp = updated.temperature.get(cell);
    let neigh_temp = updated.temperature.get(neighbor);
    let f19 = neigh_el.high_temp + 3.0;
    // 原版 L2510-2514：cell_temp > neigh_high+3；neigh_temp < neigh_high−3
    if cell_temp <= f19 {
        return false;
    }
    if neigh_el.high_temp - 3.0 <= neigh_temp {
        return false;
    }
    // 原版 L2516-2519：neighbor 高温转换目标（Element offset 6）有效
    let target = neigh_el.high_temp_transition_idx;
    if target == INVALID_ELEMENT_INDEX {
        return false;
    }
    let Some(target_el) = get_element_by_idx(target) else {
        return false;
    };
    // 原版 L2527-2529：目标必须为液体
    if (target_el.state & 3) != 2 {
        return false;
    }
    // 原版 L2521：heat = (f19 − neigh_temp) × 目标SHC × 5.0
    let heat = (f19 - neigh_temp) * target_el.specific_heat_capacity * 5.0;
    // 原版 L2523-2525：f17 = mass[cell] × cell_SHC；(cell_temp − (cell_low+6)) × f17 < heat → false
    let cell_shc = cell_el.specific_heat_capacity;
    let mut f17 = updated.mass.get(cell) * cell_shc;
    if (cell_temp - (cell_el.low_temp + 6.0)) * f17 < heat {
        return false;
    }
    // 原版 L2531：气体格新温度
    f17 = (f17 * cell_temp - heat) / f17;
    let disease_neigh = updated.disease_count.get(neighbor);
    let disease_idx_neigh = updated.disease_idx.get(neighbor);
    // 原版 L2533：i14 = (int)((5.0 / mass[neighbor]) × disease[neighbor])
    let i14 = ((5.0 / mass_neigh) * disease_neigh as f32) as i32;

    if crate::c2_physics::liquid_flow::spawn_falling_liquid(
        sim,
        cell,
        target,
        5.0,
        f19,
        disease_idx_neigh,
        i14,
    ) {
        // 原版 L2535-2545：成功 → 气体格降温、固体格扣 5kg/病菌
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.temperature.set(cell, f17);
        updated.mass.set(neighbor, mass_neigh - 5.0);
        let remain = disease_neigh - i14;
        if remain < 1 {
            crate::c2_physics::liquid_flow::clear_disease(updated, neighbor);
        } else {
            updated.disease_count.set(neighbor, remain);
        }
        if updated.mass.get(neighbor) <= 1.1754944e-38 {
            crate::c2_physics::liquid_flow::do_evaporate(sim, neighbor);
        }
        return true;
    }
    // 原版 L2546-2562：液滴失败 → DisplaceGas(cell) 后就地转化 + 固体扣减
    if !crate::c2_physics::liquid_flow::displace_gas(sim, cell, cell_elem) {
        return false;
    }
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.element_idx.set(cell, target);
        updated.mass.set(cell, 5.0);
        updated.temperature.set(cell, f19);
        crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
    }
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.mass.set(neighbor, mass_neigh - 5.0);
        let remain = disease_neigh - i14;
        if remain < 1 {
            crate::c2_physics::liquid_flow::clear_disease(updated, neighbor);
        } else {
            updated.disease_count.set(neighbor, remain);
        }
        if updated.mass.get(neighbor) <= 1.1754944e-38 {
            crate::c2_physics::liquid_flow::do_evaporate(sim, neighbor);
        }
    }
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
                crate::c2_physics::region_events::emit_cell_melted(
                    sim,
                    CellMeltedInfo { game_cell: gc as u32 },
                );
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
                    // 原版 L345-349：范围检查用**已写入的新温度**（max(0, t+1.5)），
                    // 不是原始 t——此前用旧温度导致低温相变窗口判断偏严：
                    // t < 目标低变下限-3 但新温度落在窗口内时，原版会 push 液滴，
                    // Rust 却保格换元素（质量不清、无液滴）。2026-08-07 CRITICAL-2 修复。
                    let new_t = updated.temperature.get(cell);
                    if elem.low_temp - 3.0 <= new_t && new_t <= elem.high_temp + 3.0 {
                        crate::c2_physics::region_events::emit_spawn_liquid(
                            sim,
                            SpawnFallingLiquidInfo {
                                cell_idx: gc,
                                element_idx: etd.low_temp_transition_idx,
                                disease_idx: updated.disease_idx.get(cell),
                                pad: 0,
                                mass: updated.mass.get(cell),
                                temperature: updated.temperature.get(cell),
                                disease_count: updated.disease_count.get(cell),
                            },
                        );
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
/// 新温度 = ±heat/HC + 原温度，钳制不越过加权均衡。
/// 返回 (new_第一参, new_第二参)。
/// 2026-09-06 对齐修正：原版全程 float（SimDLL_Source.c L133833+ 的 fVar 序列），
/// 此前 Rust 用 f64 中间计算属于精度偏差（非增强）——改回 f32 与原版一致，
/// 同时消除 6 次/调用的 cvt 转换并解除 SIMD 向量化阻碍。
pub(crate) fn calculate_temperature_exchange_precise(
    rate: f32,
    k: f32,
    dt: f32,
    t_a: f32,
    hc_a: f32,
    t_b: f32,
    hc_b: f32,
) -> (f32, f32) {
    if hc_a <= 0.0 || hc_b <= 0.0 {
        return (t_a, t_b);
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
    (new_a, new_b)
}

/// insulation 中间值 LUT（#16，2026-09-06）：iv = ins² × 1.53787e-05。
/// insulation 是 u8（256 种输入），热路径每对邻格重复计算——预计算查表。
/// const 求值与运行时 IEEE 乘法逐位一致（同表达式同顺序），下方单测护栏。
static INSULATION_IV_LUT: [f32; 256] = {
    let mut lut = [0.0f32; 256];
    let mut i = 0usize;
    while i < 256 {
        let ins = i as f32;
        lut[i] = ins * ins * 1.53787e-05;
        i += 1;
    }
    lut
};

/// 合并导率（原版 UpdateTemperature L133833-133847）：任一侧 insulation 值
/// （ins²×1.53787e-05）<1.0 → min(k_a,k_b)；否则几何平均。
/// 注：ins=255 时 f32 下 iv 舍入为 1.0（非 <1）→ 几何平均分支可达。
fn combine_conductivity(ins_a: u8, tc_a: f32, ins_b: u8, tc_b: f32) -> f32 {
    let iv_a = INSULATION_IV_LUT[ins_a as usize];
    let iv_b = INSULATION_IV_LUT[ins_b as usize];
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
    let (etd_a, etd_b) = {
        let c = unsafe { &*sim.cells.ptr };
        if cell_a >= c.temperature.len() || cell_b >= c.temperature.len() {
            return;
        }
        match (
            get_element_temperature_data(c.element_idx.get(cell_a)),
            get_element_temperature_data(c.element_idx.get(cell_b)),
        ) {
            (Some(a), Some(b)) => (a, b),
            _ => return,
        }
    };
    update_temperature_pair_with(sim, cell_a, cell_b, &etd_a, &etd_b);
}

/// #13 查表复用版：调用方（run_temperature_band）已持有双方 ETD 时免重查。
/// 门控与写入顺序与原 update_temperature_pair 逐位一致——只是省掉重复的
/// 2 次 get_element_temperature_data（AtomicPtr 加载 + 索引检查）。
fn update_temperature_pair_with(
    sim: &mut SimData,
    cell_a: usize,
    cell_b: usize,
    etd_a: &ElementTemperatureData,
    etd_b: &ElementTemperatureData,
) {
    let (t_a, t_b, hc_a, hc_b, k_total) = {
        let c = unsafe { &*sim.cells.ptr };
        if cell_a >= c.temperature.len() || cell_b >= c.temperature.len() {
            return;
        }
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
        let mult_a = surface_multiplier_pair(etd_a, etd_b.state);
        let mult_b = surface_multiplier_pair(etd_b, etd_a.state);
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
    // 原版 L134079-134099：实参 (cell, back) 调用，A 侧（偏移4）更新格子、B 侧（偏移0）写背墙。
    // calculate_temperature_exchange_precise 返回逻辑序 (new_a, new_b)（第一实参侧, 第二实参侧）。
    // 2026-08-07 修复：此前两个分支都对调返回值 → 格子拿到背墙温度、背墙拿到格子温度
    // （全图背墙换热方向性错乱；测试仅不等式断言未能捕获）。
    let (new_back, new_cell) = if t_cell <= t_back {
        // a=cell, b=back → (new_cell, new_back)；映射回 (new_back, new_cell)
        let (nc, nb) = calculate_temperature_exchange_precise(
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
        // a=back, b=cell → (new_back, new_cell)
        let (nb, nc) = calculate_temperature_exchange_precise(
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
                    crate::c2_physics::region_events::emit_backwall_should_transition(
                        sim,
                        BackwallShouldTransitionInfo {
                            game_cell: game_cell as u32,
                        },
                    );
                }
            }
        }
    }
}

/// 邻格换热门控（原版 L144246-144258）：双方元素 ETD 有效、TC>0、state 无 TemperatureInsulated(0x10)。
fn pair_allowed(sim: &SimData, a: usize, b: usize) -> bool {
    let c = unsafe { &*sim.cells.ptr };
    match (
        get_element_temperature_data(c.element_idx.get(a)),
        get_element_temperature_data(c.element_idx.get(b)),
    ) {
        (Some(etd_a), Some(etd_b)) => pair_allowed_etd(&etd_a, &etd_b),
        _ => false,
    }
}

/// 纯判定版（#13 查表复用）：调用方已持有双方 ETD 时零查表判定。
/// 门控顺序与原 pair_allowed 逐位一致。
fn pair_allowed_etd(etd_a: &ElementTemperatureData, etd_b: &ElementTemperatureData) -> bool {
    etd_a.thermal_conductivity > 0.0
        && etd_b.thermal_conductivity > 0.0
        && (etd_a.state | etd_b.state) & 0x10 == 0
}

/// SimUpdateTemperatureTask::InternalDoTask 单线程等价（原版 L144205-144310）。
/// 遍历内部格：背墙换热 → 右邻（ΔT≥1 且 TC>0 且无 0x10）→ 下邻（同门控）→ DoStateTransition。
/// 读 `cells` 稳定缓冲、写 `updatedCells` 增量（opening CopyFrom 后 == 子步初态）。
pub(crate) fn run_temperature_task(
    sim: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
            run_temperature_band(sim, bounds, bounds.min_y, bounds.max_y);
}

/// 温度行带任务（D2：原版 UpdateData L144430 把每个区域的温度按 worker 数切行带，
/// 提交到 ParallelTaskQueue 执行）。行范围 [row_start, row_end)，列范围取区域边界。
pub(crate) fn run_temperature_band(
    sim: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
    row_start: usize,
    row_end: usize,
) {
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
    for row in row_start..row_end {
                for col in bounds.min_x..bounds.max_x {
            if row >= h - 1 || col >= w - 1 {
                continue;
            }
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
            // #13 查表复用：|ΔT| 门控通过后才查 ETD（保持惰性——温差小的格零查表），
            // 每方向查到的 ETD 同时用于 pair_allowed_etd 判定与 update_temperature_pair_with，
            // 消除原 update_temperature_pair 内部的 2 次重复查表。
            if col + 1 < w - 1 && (t_self - t_right).abs() >= 1.0 {
                let c = unsafe { &*sim.cells.ptr };
                if let (Some(ea), Some(eb)) = (
                    get_element_temperature_data(c.element_idx.get(cell)),
                    get_element_temperature_data(c.element_idx.get(cell + 1)),
                ) {
                    if pair_allowed_etd(&ea, &eb) {
                        update_temperature_pair_with(sim, cell, cell + 1, &ea, &eb);
                    }
                }
            }
            if row + 1 < h - 1 && (t_self - t_below).abs() >= 1.0 {
                let c = unsafe { &*sim.cells.ptr };
                if let (Some(ea), Some(eb)) = (
                    get_element_temperature_data(c.element_idx.get(cell)),
                    get_element_temperature_data(c.element_idx.get(cell + w)),
                ) {
                    if pair_allowed_etd(&ea, &eb) {
                        update_temperature_pair_with(sim, cell, cell + w, &ea, &eb);
                    }
                }
            }
            // 4. DoStateTransition（每格，原版 L144293-144306）
            let elem = unsafe { (*sim.cells.ptr).element_idx.get(cell) };
            if let Some(etd) = get_element_temperature_data(elem) {
                do_state_transition(sim, cell, &etd);
            }
        }
    }
}

/// D2 温度行带任务入口：经 SendSyncPtr 捕获整个 SimData 指针（避免 edition-2021
/// disjoint capture 只捕获 `ptr.0` 裸字段导致闭包 !Send）。worker 内 1 个 &mut。
pub(crate) fn run_temperature_band_task(
    sim_ptr: crate::globals::SendSyncPtr<SimData>,
    bounds: crate::d1_activity::RegionBounds,
    row_start: usize,
    row_end: usize,
) {
    let sim = unsafe { &mut *sim_ptr.0 };
    run_temperature_band(sim, bounds, row_start, row_end);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::b_elements::element::Element;
    use crate::LIB_TESTS_LOCK;

    /// #16 LUT 一致性护栏：256 个预计算值必须与现算表达式逐位全等
    /// （ins*ins*1.53787e-05，同乘法顺序）。任何一侧改写导致舍入差异即红。
    #[test]
    fn insulation_iv_lut_matches_inline_computation() {
        for i in 0..=255usize {
            let ins = i as f32;
            let expected = ins * ins * 1.53787e-05;
            assert_eq!(
                INSULATION_IV_LUT[i], expected,
                "LUT[{i}] 与现算不一致（逐位）"
            );
        }
    }

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
        let _lock = LIB_TESTS_LOCK.lock();
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

    /// 2026-08-10 玩家报告回归：原油 CrudeOil 高温相变不成石油 Petroleum。
    /// 液体→液体（无 ore）走与原版一致的高温分支：温度 −1.5、元素切换、质量不变。
    /// CrudeOil 真实参数：highTemp=673K、highTempTransitionTarget=Petroleum、无 ore。
    #[test]
    fn high_transition_liquid_to_liquid_switches_element_keeps_mass() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_phase_table();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.resize(6, Default::default());
            table.elements.resize(6, Element::default());
            // 原油(4) → 石油(5)
            table.temperature_data[4].state = 2;
            table.temperature_data[4].low_temp = 233.0;
            table.temperature_data[4].high_temp = 673.0;
            table.temperature_data[4].high_temp_transition_idx = 5;
            table.temperature_data[5].state = 2;
            table.temperature_data[5].low_temp = 216.0;
            table.temperature_data[5].high_temp = 812.0;
            table.elements[4].state = 2;
            table.elements[5].state = 2;
        }
        let mut sd = make_sim();
        fill(&mut sd, 14, 4, 870.0, 677.0); // 原油 677K > 673+3
        let etd = get_element_temperature_data(4).unwrap();
        assert!(do_state_transition(&mut sd, 14, &etd));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 5, "原油应→石油");
        assert!((u.temperature.get(14) - 675.5).abs() < 1e-4, "温度 -1.5 → 675.5，got {}", u.temperature.get(14));
        assert!((u.mass.get(14) - 870.0).abs() < 1e-4, "液体→液体质量不变，got {}", u.mass.get(14));
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.substance_change_info.len(), 1);
    }

    #[test]
    fn low_transition_solid_clears_cell_when_mass_ratio_small() {
        let _lock = LIB_TESTS_LOCK.lock();
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
        let _lock = LIB_TESTS_LOCK.lock();
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

    /// CRITICAL-2 回归（2026-08-07）：低温相变液滴的范围检查必须用**新温度**
    /// max(0, t+1.5)（原版 L345-349），不是原始 t。
    /// 蒸汽 t=-4℃：旧温度 -4 < 水低变窗口下界 -3 → 检查失败（旧实现保格换元素）；
    /// 新温度 max(0, -4+1.5)=0 ∈ [-3, 103] → 原版 push 液滴（T=0、整格质量、清格）。
    #[test]
    fn low_transition_range_check_uses_new_temperature() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 20, 3, 5.0, -4.0); // 蒸汽 → 低变=水（非固态）
        let etd = get_element_temperature_data(3).unwrap();
        assert!(do_state_transition(&mut sd, 20, &etd));
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.spawn_liquid_info.len(), 1, "新温度通过窗口 → 液滴");
        let drop = events.spawn_liquid_info.get(0);
        assert_eq!(drop.element_idx, 1, "目标=水");
        assert!((drop.mass - 5.0).abs() < 1e-4, "液滴质量=整格质量");
        assert!(
            (drop.temperature - 0.0).abs() < 1e-4,
            "push 温度 = max(0, t+1.5) = 0，got {}",
            drop.temperature
        );
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(20), sd.vacuum_element_idx, "push 后清格");
    }

    #[test]
    fn transition_gate_keeps_within_plus_minus_three() {
        let _lock = LIB_TESTS_LOCK.lock();
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
    fn load_time_transition_high_converts_element_and_cools() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 14, 1, 10.0, 105.0); // 水 105℃ > high+3(103)
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 3, "应转为蒸汽");
        assert!((u.temperature.get(14) - 103.5).abs() < 1e-4, "温度应 -1.5 → 103.5");
    }

    #[test]
    fn load_time_transition_low_converts_element_and_warms() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 14, 1, 10.0, -5.0); // 水 -5℃ < low-3(-3)
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 2, "应转为冰");
        assert!((u.temperature.get(14) + 3.5).abs() < 1e-4, "温度应 +1.5 → -3.5");
    }

    #[test]
    fn load_time_transition_within_window_and_boundary_is_noop() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 14, 1, 10.0, 97.0); // 窗口内
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 1);
        assert_eq!(u.temperature.get(14), 97.0);
        fill(&mut sd, 14, 1, 10.0, -3.0); // 恰在 low-3 边界 → 不触发
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 1);
        assert_eq!(u.temperature.get(14), -3.0);
        fill(&mut sd, 14, 1, 10.0, 103.0); // 恰在 high+3 边界 → 不触发
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 1);
        assert_eq!(u.temperature.get(14), 103.0);
    }

    #[test]
    fn load_time_transition_without_target_is_noop() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_phase_table();
        // 冰(2) 无转换目标：手动置 0xFFFF
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data[2].low_temp_transition_idx = 0xFFFF;
            table.temperature_data[2].high_temp_transition_idx = 0xFFFF;
        }
        let mut sd = make_sim();
        fill(&mut sd, 14, 2, 10.0, 2000.0); // 冰 2000℃ 但无高温目标
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 2);
        assert_eq!(u.temperature.get(14), 2000.0);
        fill(&mut sd, 14, 2, 10.0, -100.0); // 冰 无低温目标 → 也不动
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 2);
        assert_eq!(u.temperature.get(14), -100.0);
    }

    /// 部分沸腾测试表：dummy(0) / 水 water(1, state2, high=100, high_trans=3) /
    /// 冰 ice(2) / 蒸汽 steam(3, state1) / 热固体 hot(4, state3, tc=1, low=-100, high=500) /
    /// 熔岩 lava(5, state2, tc=1)。
    fn init_partial_heat_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.temperature_data.clear();
        let mut mk = |state: u8, low: f32, high: f32, tc: f32, shc: f32, high_trans: u16| {
            let mut e = Element::default();
            e.state = state;
            e.low_temp = low;
            e.high_temp = high;
            e.thermal_conductivity = tc;
            e.specific_heat_capacity = shc;
            e.high_temp_transition_idx = high_trans;
            e
        };
        table.elements.push(Element::default()); // 0 dummy
        table.elements.push(mk(2, 0.0, 100.0, 1.0, 1.0, 3)); // 1 水 → 蒸汽
        table.elements.push(Element::default()); // 2 冰
        table.elements.push(mk(1, 100.0, 999.0, 1.0, 1.0, 0)); // 3 蒸汽
        table.elements.push(mk(3, -100.0, 500.0, 1.0, 1.0, 0xFFFF)); // 4 热固体
        table.elements.push(mk(2, 0.0, 500.0, 1.0, 1.0, 0)); // 5 熔岩
        // temperature_data 与 elements 同序（液滴温度范围检查会读 low/high）
        let td = [
            (0.0f32, 0.0f32),   // 0 dummy
            (0.0, 100.0),       // 1 水
            (0.0, 0.0),         // 2 冰
            (100.0, 999.0),     // 3 蒸汽
            (-100.0, 500.0),    // 4 热固体
            (0.0, 500.0),       // 5 熔岩
        ];
        for (low, high) in td {
            let mut d = crate::b_elements::element::ElementTemperatureData::default();
            d.low_temp = low;
            d.high_temp = high;
            table.temperature_data.push(d);
        }
        // post_process_data / liquid_data / state_data 补齐 state（displace 系列检查用）
        let states = [0u8, 2, 3, 1, 3, 2];
        table.post_process_data.clear();
        table.liquid_data.clear();
        table.state_data.clear();
        for &s in &states {
            let mut ppd = crate::b_elements::element::ElementPostProcessData::default();
            ppd.state = s;
            table.post_process_data.push(ppd);
            let mut ld = crate::b_elements::element::ElementLiquidData::default();
            ld.state = s;
            table.liquid_data.push(ld);
            let mut sd = crate::b_elements::element::ElementStateData::default();
            sd.state = s;
            table.state_data.push(sd);
        }
    }

    #[test]
    fn partial_heat_gas_target_places_below() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_partial_heat_table();
        let mut sd = make_sim();
        // cell14=水 10kg 20℃；neighbor15=热固体 100kg 600℃；below20=蒸汽
        fill(&mut sd, 14, 1, 10.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 600.0);
        fill(&mut sd, 20, 3, 1.0, 300.0);
        assert!(do_partial_heat_transition(&mut sd, 14, 15));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!(
            (u.mass.get(14) - 5.0).abs() < 1e-4,
            "本格应剩 5kg，got {}",
            u.mass.get(14)
        );
        assert_eq!(u.element_idx.get(14), 1, "本格仍是水");
        assert_eq!(u.element_idx.get(20), 3, "下方格变蒸汽");
        assert!((u.mass.get(20) - 5.0).abs() < 1e-4);
        assert!(
            (u.temperature.get(20) - 103.0).abs() < 1e-4,
            "下方蒸汽应 103℃（high+3）"
        );
        assert!(
            (u.temperature.get(15) - 595.85).abs() < 1e-2,
            "热固体应降温，got {}",
            u.temperature.get(15)
        );
    }

    #[test]
    fn partial_heat_liquid_target_spawns_droplet() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_partial_heat_table();
        let mut sd = make_sim();
        // 水 → 目标改为熔岩(5)：水(1).high_trans=5
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements[1].high_temp_transition_idx = 5;
        }
        fill(&mut sd, 14, 1, 10.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 600.0);
        assert!(do_partial_heat_transition(&mut sd, 14, 15));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!(
            u.mass.get(14) < 0.01,
            "液体目标分支 DisplaceLiquid 后排空本格，got {}",
            u.mass.get(14)
        );
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.spawn_liquid_info.len(), 1, "应生成 1 条液滴事件");
        let drop = events.spawn_liquid_info.get(0);
        assert_eq!(drop.element_idx, 5);
        assert!((drop.mass - 5.0).abs() < 1e-4);
        assert!((drop.temperature - 103.0).abs() < 1e-4);
    }

    /// 2026-08-10 玩家报告：原油 CrudeOil 强制相变生成的石油温度不对（334℃ 而非 403℃）。
    /// 用真实参数验证：CrudeOil(6) high=673K → Petroleum(7)；Magma(8) low=1683K。
    #[test]
    fn partial_heat_crude_oil_to_petroleum_droplet_at_high_plus_3() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_partial_heat_table();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.resize(9, Element::default());
            table.temperature_data.resize(9, Default::default());
            // CrudeOil(6)：液体、high=673K → Petroleum(7)、TC=2
            table.elements[6].state = 2;
            table.elements[6].low_temp = 233.0;
            table.elements[6].high_temp = 673.0;
            table.elements[6].thermal_conductivity = 2.0;
            table.elements[6].high_temp_transition_idx = 7;
            table.temperature_data[6].state = 2;
            table.temperature_data[6].low_temp = 233.0;
            table.temperature_data[6].high_temp = 673.0;
            // Petroleum(7)：液体
            table.elements[7].state = 2;
            table.elements[7].low_temp = 216.0;
            table.elements[7].high_temp = 812.0;
            table.temperature_data[7].state = 2;
            table.temperature_data[7].low_temp = 216.0;
            table.temperature_data[7].high_temp = 812.0;
            // Magma(8)：液体、low=1683K、TC=1
            table.elements[8].state = 2;
            table.elements[8].low_temp = 1683.0;
            table.elements[8].high_temp = 2630.0;
            table.elements[8].thermal_conductivity = 1.0;
            table.temperature_data[8].state = 2;
            table.temperature_data[8].low_temp = 1683.0;
            table.temperature_data[8].high_temp = 2630.0;
            // post_process/liquid/state 补齐 state
            for idx in [6usize, 7, 8] {
                table.post_process_data.resize(idx + 1, Default::default());
                table.liquid_data.resize(idx + 1, Default::default());
                table.state_data.resize(idx + 1, Default::default());
                table.post_process_data[idx].state = 2;
                table.liquid_data[idx].state = 2;
                table.state_data[idx].state = 2;
            }
        }
        let mut sd = make_sim();
        // 原油 100kg @ 600K（327℃），岩浆 1000kg @ 1800K
        fill(&mut sd, 14, 6, 100.0, 600.0);
        fill(&mut sd, 15, 8, 1000.0, 1800.0);
        assert!(do_partial_heat_transition(&mut sd, 14, 15));
        let u = unsafe { &*sd.updated_cells.ptr };
        // 液体目标分支 DisplaceLiquid 排空本格（原版同）
        assert!(u.mass.get(14) < 0.01, "本格应被排空，got {}", u.mass.get(14));
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.spawn_liquid_info.len(), 1);
        let drop = events.spawn_liquid_info.get(0);
        assert_eq!(drop.element_idx, 7, "液滴应为石油");
        assert!((drop.mass - 5.0).abs() < 1e-4);
        assert!(
            (drop.temperature - 676.0).abs() < 1e-3,
            "石油应 676K（high+3=673+3），got {}K",
            drop.temperature
        );
    }

    #[test]
    fn partial_heat_affordability_gate_uses_specific_heat_capacity() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_partial_heat_table();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.resize(9, Element::default());
            table.temperature_data.resize(9, Default::default());
            // CrudeOil(6): liquid, high=673K -> Petroleum(7); SHC=1.69, TC=2
            table.elements[6].state = 2;
            table.elements[6].low_temp = 233.0;
            table.elements[6].high_temp = 673.0;
            table.elements[6].specific_heat_capacity = 1.69;
            table.elements[6].thermal_conductivity = 2.0;
            table.elements[6].high_temp_transition_idx = 7;
            table.temperature_data[6].state = 2;
            table.temperature_data[6].low_temp = 233.0;
            table.temperature_data[6].high_temp = 673.0;
            // Petroleum(7): liquid
            table.elements[7].state = 2;
            table.elements[7].low_temp = 216.0;
            table.elements[7].high_temp = 812.0;
            table.elements[7].specific_heat_capacity = 1.69;
            table.temperature_data[7].state = 2;
            table.temperature_data[7].low_temp = 216.0;
            table.temperature_data[7].high_temp = 812.0;
            // Hot solid(8): 100kg scenario, SHC=1, TC=2 (like Igneous Rock)
            table.elements[8].state = 3;
            table.elements[8].low_temp = 0.0;
            table.elements[8].high_temp = 10000.0;
            table.elements[8].specific_heat_capacity = 1.0;
            table.elements[8].thermal_conductivity = 2.0;
            table.temperature_data[8].state = 3;
            table.temperature_data[8].low_temp = 0.0;
            table.temperature_data[8].high_temp = 10000.0;
            for idx in [6usize, 7, 8] {
                table.post_process_data.resize(idx + 1, Default::default());
                table.liquid_data.resize(idx + 1, Default::default());
                table.state_data.resize(idx + 1, Default::default());
                table.post_process_data[idx].state = table.elements[idx].state;
                table.liquid_data[idx].state = table.elements[idx].state;
                table.state_data[idx].state = table.elements[idx].state;
            }
        }
        let mut sd = make_sim();
        // Oil 220C (493K) + 100kg solid at 416C (689K):
        // original gate mass*SHC*10 >= (676-T)*SHC_cell*5
        //   1000 >= 8.45*(676-493)=1546  -> REJECT (old TC bug accepted: 2000 >= 1830)
        fill(&mut sd, 14, 6, 100.0, 493.0);
        fill(&mut sd, 15, 8, 100.0, 689.0);
        assert!(
            !do_partial_heat_transition(&mut sd, 14, 15),
            "oil at 220C must be rejected by the SHC affordability gate"
        );
        // Oil 300C (573K): heat=8.45*(676-573)=870 <= 1000 -> PASS
        fill(&mut sd, 14, 6, 100.0, 573.0);
        assert!(
            do_partial_heat_transition(&mut sd, 14, 15),
            "oil at 300C must pass the SHC affordability gate"
        );
    }

    #[test]
    fn partial_heat_precondition_gates_and_restore() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_partial_heat_table();
        let mut sd = make_sim();
        // 质量 < 5 → false
        fill(&mut sd, 14, 1, 4.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 600.0);
        assert!(!do_partial_heat_transition(&mut sd, 14, 15));
        // 邻格不够热（neigh_temp > high+3 不满足）→ false
        fill(&mut sd, 14, 1, 10.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 102.0);
        assert!(!do_partial_heat_transition(&mut sd, 14, 15));
        // 无高温转换目标（冰）→ false
        fill(&mut sd, 14, 2, 10.0, 20.0);
        assert!(!do_partial_heat_transition(&mut sd, 14, 15));
        // displace 候选 [右15, 左13, 下20, 上8] 全固体 → DisplaceLiquid 失败 → 恢复并返回 false
        fill(&mut sd, 14, 1, 10.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 600.0);
        for n in [8usize, 13, 20] {
            fill(&mut sd, n, 4, 100.0, 300.0); // 固体邻居（8/13/20 = 上/左/下）
        }
        assert!(!do_partial_heat_transition(&mut sd, 14, 15));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!(
            (u.mass.get(14) - 10.0).abs() < 1e-4,
            "失败应恢复质量，got {}",
            u.mass.get(14)
        );
        assert!(
            (u.temperature.get(15) - 600.0).abs() < 1e-4,
            "失败应恢复邻格温度"
        );
    }

    /// 端到端：盐水（水，100kg 20°C）被热源缓慢加热 → 分批 5kg 蒸发全程
    /// **不产生矿渣（盐）**——partial 分支无 SpawnOre，且本格温度保持
    /// < high−3 时不会触发 DoStateTransition（产盐路径）。原版同款行为。
    #[test]
    fn partial_evaporation_completes_without_ore() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_partial_heat_table();
        let mut sd = make_sim();
        // cell14=盐水(水 1：high=100 → 蒸汽 3) 100kg 20°C；cell15=热固体 600°C；
        // below20=蒸汽（排放目标，DisplaceGas 用）
        fill(&mut sd, 14, 1, 100.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 600.0);
        fill(&mut sd, 20, 3, 1.0, 300.0);
        let events_before = unsafe { &*sd.sim_events.ptr }.spawn_ore_info.len();
        // 模拟 30 tick：热源缓慢加热（每 tick +1°C，远低于蒸发完所需），
        // 每 tick 先 partial（温度 < high−3=97 时触发），温度超限后停止。
        for _ in 0..30 {
            let t = unsafe { (*sd.updated_cells.ptr).temperature.get(14) };
            unsafe { (*sd.updated_cells.ptr).temperature.set(14, t + 1.0) };
            let etd = crate::b_elements::elements_table::get_element_temperature_data(1).unwrap();
            if do_state_transition(&mut sd, 14, &etd) {
                break; // 温度超过 high+3 → 整格相变（产盐路径）
            }
            if !do_partial_heat_transition(&mut sd, 14, 15) {
                break;
            }
        }
        let u = unsafe { &*sd.updated_cells.ptr };
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(
            events.spawn_ore_info.len(),
            events_before,
            "分批蒸发全程不得产生矿渣（盐）"
        );
        assert!(
            u.mass.get(14) < 0.01,
            "100kg 盐水应经 partial 全部蒸发为蒸汽，剩 {}kg",
            u.mass.get(14)
        );
        assert_eq!(u.element_idx.get(14), 1, "本格仍为盐水（未整格相变）");
        // partial 每次把新 5kg 蒸汽放入下方格、旧蒸汽被 DisplaceGas 挤到更下方，
        // 下方格始终 ~5kg；总蒸汽量体现在被挤走的格子。此处只断言盐水耗尽 + 无盐。
        assert_eq!(u.element_idx.get(20), 3, "下方格为蒸汽排放口");
    }

    #[test]
    fn partial_melt_spawns_droplet_and_deducts_solid() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_partial_heat_table();
        // 热固体(4) 需 high_temp_transition_idx → 熔岩(5)；蒸汽(3) 作为高温气体格
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements[4].high_temp_transition_idx = 5; // 固体 → 熔岩
            table.elements[4].low_temp = -100.0;
            table.elements[4].high_temp = 500.0;
            table.elements[4].thermal_conductivity = 1.0;
        }
        let mut sd = make_sim();
        // cell14=蒸汽(气体格) 100kg 800℃；neighbor15=热固体 100kg 20℃
        fill(&mut sd, 14, 3, 100.0, 800.0);
        fill(&mut sd, 15, 4, 100.0, 20.0);
        assert!(do_partial_melt(&mut sd, 14, 15));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!(
            (u.mass.get(15) - 95.0).abs() < 1e-4,
            "固体应扣 5kg，got {}",
            u.mass.get(15)
        );
        assert!(
            (u.temperature.get(14) - 775.85).abs() < 1e-2,
            "气体格应降温到 775.85，got {}",
            u.temperature.get(14)
        );
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.spawn_liquid_info.len(), 1, "应生成 1 条液滴");
        let drop = events.spawn_liquid_info.get(0);
        assert_eq!(drop.element_idx, 5);
        assert!((drop.mass - 5.0).abs() < 1e-4);
        assert!(
            (drop.temperature - 503.0).abs() < 1e-4,
            "液滴温度应为固体熔点+3"
        );
    }

    #[test]
    fn partial_melt_precondition_gates() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_partial_heat_table();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements[4].high_temp_transition_idx = 5;
        }
        let mut sd = make_sim();
        // 邻格非固体 → false
        fill(&mut sd, 14, 3, 100.0, 800.0);
        fill(&mut sd, 15, 3, 100.0, 20.0); // 蒸汽
        assert!(!do_partial_melt(&mut sd, 14, 15));
        // 固体质量 <= 5 → false
        fill(&mut sd, 15, 4, 5.0, 20.0);
        assert!(!do_partial_melt(&mut sd, 14, 15));
        // 气体格不够热（cell_temp <= 熔点+3）→ false
        fill(&mut sd, 15, 4, 100.0, 20.0);
        fill(&mut sd, 14, 3, 100.0, 300.0);
        assert!(!do_partial_melt(&mut sd, 14, 15));
    }

    #[test]
    fn spawn_ore_pushes_event_with_game_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
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
        let _lock = LIB_TESTS_LOCK.lock();
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
        let _lock = LIB_TESTS_LOCK.lock();
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
        let _lock = LIB_TESTS_LOCK.lock();
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
        let _lock = LIB_TESTS_LOCK.lock();
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
        let _lock = LIB_TESTS_LOCK.lock();
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

    /// 2026-08-07 背墙换热对调回归：格子与背墙热容悬殊时，交换方向可精确区分。
    /// 格 14：水 300℃（mass=100 → HC=417.9）；背墙：砖 500℃（mass=100 → HC=80）。
    /// 修复前两个分支对调返回值 → 格子拿到背墙温度（~499）、背墙拿到格子温度（~300）。
    /// 修复后：格子微升（<310）、背墙微降（>490）。
    #[test]
    fn backwall_exchange_cell_gets_cell_temp_not_backwall() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_pair_table();
        let mut sd = make_sim();
        unsafe {
            let c = &mut *sd.cells.ptr;
            let u = &mut *sd.updated_cells.ptr;
            let bw = &mut *sd.backwall.ptr;
            // 格 14：水 300℃；背墙：砖 500℃
            for buf in [&mut *c, &mut *u] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                buf.insulation.set(14, 255);
            }
            bw.element_idx.set(14, 2);
            bw.mass.set(14, 100.0);
            bw.temperature.set(14, 500.0);
        }
        update_temperature_for_backwall(&mut sd, 14);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            let bw = &*sd.backwall.ptr;
            let new_cell = u.temperature.get(14);
            let new_back = bw.temperature.get(14);
            assert!(
                new_cell < 310.0,
                "格子只应微升（水 HC 大），got {new_cell}（若≈499 说明拿到背墙温度=写反）"
            );
            assert!(
                new_cell > 300.0,
                "格子应从 300 微升，got {new_cell}"
            );
            assert!(
                new_back > 490.0,
                "背墙只应微降（砖 HC 小），got {new_back}（若≈300 说明拿到格子温度=写反）"
            );
            assert!(new_back < 500.0, "背墙应从 500 微降，got {new_back}");
        }
    }

    #[test]
    fn backwall_exchange_skips_vacuum_or_void() {
        let _lock = LIB_TESTS_LOCK.lock();
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
        let _lock = LIB_TESTS_LOCK.lock();
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
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_temperature_task(&mut sd, bounds);
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
        let _lock = LIB_TESTS_LOCK.lock();
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
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_temperature_task(&mut sd, bounds);
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
