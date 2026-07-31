//! Temperature — 温度模拟模块（Phase 7 T11-T13 实现）。
//!
//! 对照源码 04_temperature.c。
//! T2 期间提供 do_load_time_state_transition 桩以编译通过。
//! T11 实现：
//! - effective_conductivity / get_insulation_value（格子-格子热交换的导热系数计算）
//! - update_temperature（两相邻格子热交换）
//! - update_temperature_for_backwall_exchange（格子与背景墙热交换）
//! T12-T13 将实现 do_state_transition / do_load_time_state_transition / partial_heat_transition 等。

use crate::sim_data::SimData;
use crate::sim_events::SimEvents;
use crate::elements_table::get_element_temperature_data;
use crate::vector_math::{calculate_temperature_exchange, calculate_temperature_exchange_precise};

/// 1.53787e-5 常量（源码 04_temperature.c L542, L546, 11_msvcrt_ignored.c L33226）。
///
/// 用于把 byte insulation 值（0-255）转换为 effective insulation factor：
/// `insulation_factor = insulation_byte² × 1.53787e-5`
/// 当 insulation_byte = 255 时，insulation_factor ≈ 1.0（255² × 1.53787e-5 ≈ 0.9999）。
/// 即 insulation_byte = 255 表示"完全绝缘"（factor ≈ 1）。
const INSULATION_FACTOR_CONSTANT: f32 = 1.53787e-5;

/// `CellSOA::GetInsulationValue` — 计算 insulation factor。
///
/// 严格对照源码 11_msvcrt_ignored.c L33216-33230（CellSOA::GetInsulationValue）。
///
/// 公式：`insulation_byte² × 1.53787e-5`
///
/// **运行时影响**：与原版完全等价（同样公式）。
fn get_insulation_value(insulation_byte: u8) -> f32 {
    let ins = insulation_byte as f32;
    ins * ins * INSULATION_FACTOR_CONSTANT
}

/// `effective_conductivity` — 格子-格子热交换的 effective conductivity。
///
/// 严格对照源码 04_temperature.c L540-543（cell A 部分）和 L544-549（cell B 部分）的内联公式。
///
/// 公式：`insulation_byte² × 1.53787e-5 × thermal_conductivity`
/// 即 `GetInsulationValue(insulation_byte) × thermal_conductivity`
///
/// **运行时影响**：与原版等价（原版是内联计算，本函数抽取出来便于测试与复用）。
fn effective_conductivity(thermal_conductivity: f32, insulation_byte: u8) -> f32 {
    get_insulation_value(insulation_byte) * thermal_conductivity
}

/// 取两数最小值（与源码 `if (a <= b) b = a` 等价）。
///
/// 仅在两侧 insulation_factor 任一 < 1.0 时使用（取 min）。
/// 当两侧都 >= 1.0 时，源码用 log/exp 计算乘积（exp(log(a) + log(b)) = a × b），
/// 本实现直接用乘法（数学等价，对非负数无差异）。
fn combine_conductivity(
    cond_a: f32,
    insulation_factor_a: f32,
    cond_b: f32,
    insulation_factor_b: f32,
) -> f32 {
    // 源码 04_temperature.c L570-580：
    // if (fVar16 < 1.0) || (fVar19 < 1.0): take min(fVar25, fVar21)
    // else: logf(); logf(); expf(); → exp(log(fVar25) + log(fVar21)) = fVar25 × fVar21
    if insulation_factor_a < 1.0 || insulation_factor_b < 1.0 {
        cond_a.min(cond_b)
    } else {
        // 双方都接近完全绝缘：conductivity 取乘积（log/exp 等价转换）
        cond_a * cond_b
    }
}

/// `UpdateTemperature` — 两相邻格子热交换。
///
/// 严格对照源码 04_temperature.c L407-670（UpdateTemperature）。
///
/// # 参数
/// - `sim_data`：可变 SimData 引用（读 cells 原始数据，写 updated_cells）
/// - `sim_events`：SimEvents 引用（T11 未使用，T12/T13 触发相变时用）
/// - `cell_a`：格子 A 的内部索引（不是 game_cell）
/// - `cell_b`：格子 B 的内部索引
///
/// # 算法概述
/// 1. 边界检查：cell_a, cell_b 在 temperature 数组范围内
/// 2. 温度合法性检查：若 mass > 0 但 temperature <= 0，记日志（源码 KCrashReporter）
/// 3. 上限检查：temperature <= 10000.0
/// 4. 查 ElementTemperatureData for 双方 element_idx
/// 5. 计算 effective_conductivity（双方）
/// 6. 选择 surface_area_multiplier（基于对方 state）
/// 7. 用 combine_conductivity 合并 conductivity（min 或乘积）
/// 8. 计算 heat_capacity = mass × specific_heat_capacity（双方）
/// 9. 调用 calculate_temperature_exchange_precise（含 swap 处理）
/// 10. 写回 updated_cells.temperature[A], updated_cells.temperature[B]
/// 11. 钳制到 [1.0, 10000.0]
///
/// # 运行时影响
/// 与原版完全等价。两格热交换：高温降温、低温升温，趋向热平衡。
/// 调用方（ProcessFrame 遍历相邻格子对）控制调用顺序。
pub fn update_temperature(
    sim_data: &mut SimData,
    _sim_events: &mut SimEvents,
    cell_a: i32,
    cell_b: i32,
) {
    // 源码 L439-440：读 cells（原始只读）+ updatedCells（写入目标）
    if sim_data.cells.ptr.is_null() || sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let cells = unsafe { &*sim_data.cells.ptr };
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };

    let cells_temp = cells.temperature.as_slice();
    let cells_mass = cells.mass.as_slice();
    let cells_elem = cells.element_idx.as_slice();
    let cells_insul = cells.insulation.as_slice();

    let updated_temp = updated_cells.temperature.as_mut_slice();

    let total = cells_temp.len() as i32;
    if cell_a < 0 || cell_a >= total || cell_b < 0 || cell_b >= total {
        return;
    }
    let a = cell_a as usize;
    let b = cell_b as usize;

    // 源码 L446-466, L471-491：温度合法性检查（mass != 0 但 temp <= 0 → 警告）
    // Rust 实现仅记日志，不 panic（与源码 KCrashReporterReportMessage 行为等价）
    if cells_temp[a] <= 0.0 && cells_mass[a] != 0.0 {
        tracing::warn!(
            cell = cell_a, temp = cells_temp[a], mass = cells_mass[a],
            "Invalid temperature: T<=0 with non-zero mass"
        );
        return;
    }
    if cells_temp[b] <= 0.0 && cells_mass[b] != 0.0 {
        tracing::warn!(
            cell = cell_b, temp = cells_temp[b], mass = cells_mass[b],
            "Invalid temperature: T<=0 with non-zero mass"
        );
        return;
    }

    // 源码 L496-510, L513-526：上限检查（temp > 10000 → 警告）
    if cells_temp[a] > 10000.0 {
        tracing::warn!(cell = cell_a, temp = cells_temp[a], "Invalid temperature: T>10000");
    }
    if cells_temp[b] > 10000.0 {
        tracing::warn!(cell = cell_b, temp = cells_temp[b], "Invalid temperature: T>10000");
    }

    // 源码 L527-534：边界检查 element_idx
    let elem_a = cells_elem[a];
    let elem_b = cells_elem[b];
    let td_a = match get_element_temperature_data(elem_a) {
        Some(td) => td,
        None => return,
    };
    let td_b = match get_element_temperature_data(elem_b) {
        Some(td) => td,
        None => return,
    };

    // 源码 L538-543：cell A effective conductivity = insulation²×1.53787e-5 × thermal_conductivity
    let insul_a_byte = cells_insul[a];
    let insulation_factor_a = get_insulation_value(insul_a_byte);
    let cond_a = insulation_factor_a * td_a.thermal_conductivity;

    // 源码 L544-549：cell B effective conductivity
    let insul_b_byte = cells_insul[b];
    let insulation_factor_b = get_insulation_value(insul_b_byte);
    let cond_b = insulation_factor_b * td_b.thermal_conductivity;

    // 源码 L550-558：surface_area_multiplier of A, chosen by B's state
    // (源码 fVar18 = ElementTemperatureData::GetSurfaceAreaMultiplier(td_a, td_b))
    let sam_a = get_surface_area_multiplier(&td_a, td_b.state);
    // 源码 L559-568：surface_area_multiplier of B, chosen by A's state
    // (源码 fVar24 = ElementTemperatureData::GetSurfaceAreaMultiplier(td_b, td_a))
    let sam_b = get_surface_area_multiplier(&td_b, td_a.state);

    // 源码 L570-580：combine_conductivity
    let combined_conductivity = combine_conductivity(
        cond_a,
        insulation_factor_a,
        cond_b,
        insulation_factor_b,
    );

    // 源码 L581-595：计算 heat_capacity = mass × specific_heat_capacity
    let cap_a = cells_mass[a] * td_a.specific_heat_capacity;
    let cap_b = cells_mass[b] * td_b.specific_heat_capacity;

    // 源码 L597-608：调用 CalculateTemperatureExchange_precise
    // - heat_factor = 0.25（源码 param_1）
    // - surface_area = combined_conductivity × sam_a × sam_b（源码 dVar20 = fVar25 * fVar24 * fVar18）
    //   注意：源码 fVar25 是 combined_conductivity, fVar24 是 sam_b, fVar18 是 sam_a
    //   推导：源码 fVar25 = min(fVar25, fVar21) 或 fVar25*fVar21 → 是 combined 后的 conductivity
    //         fVar24 = ElementTemperatureData::GetSurfaceAreaMultiplier(td_b, td_a) = sam_b
    //         fVar18 = ElementTemperatureData::GetSurfaceAreaMultiplier(td_a, td_b) = sam_a
    //   所以 surface_area = combined_conductivity × sam_b × sam_a（顺序无关）
    // - cap_factor = 0.2（源码 param_3, `0.20000000298023224`）
    let heat_factor: f32 = 0.25;
    let cap_factor: f32 = 0.2;
    let surface_area = combined_conductivity * sam_a * sam_b;

    // 源码 L597-608 直接调用 _precise（不用 wrapper），自己处理 swap：
    // - if temp_a <= temp_b: 直接调 _precise(temp_a, cap_a, temp_b, cap_b)
    //   输出 (final_temp_hi=final_b, final_temp_lo=final_a, delta_heat)
    // - else: 调 _precise(temp_b, cap_b, temp_a, cap_a)，swap 输出
    // 等价于直接调用 wrapper calculate_temperature_exchange：
    let result = calculate_temperature_exchange(
        heat_factor,
        surface_area,
        cap_factor,
        cells_temp[a],
        cap_a,
        cells_temp[b],
        cap_b,
    );
    // wrapper 返回：
    // - final_temp_hi = final_temp_b
    // - final_temp_lo = final_temp_a
    // - delta_heat = energy_lost_by_a（不用）

    // 源码 L615-618: updated_cells_temp[a] += final_temp_a - cells_temp[a]
    // 源码 L625-628: updated_cells_temp[b] += final_temp_b - cells_temp[b]
    // 注意：源码中 if temp_a <= temp_b 时 local_74 = field_0x0 = final_temp_b, local_78 = field_0x4 = final_temp_a
    //       else 时 swap：local_74 = field_0x0 = final_temp_a, local_78 = field_0x4 = final_temp_b
    // 但 wrapper 已经统一了语义：final_temp_hi=B, final_temp_lo=A
    let delta_temp_a = result.final_temp_lo - cells_temp[a];
    let delta_temp_b = result.final_temp_hi - cells_temp[b];
    updated_temp[a] += delta_temp_a;
    updated_temp[b] += delta_temp_b;

    // 源码 L632-650：钳制温度到 [1.0, 10000.0]
    let clamped_a = updated_temp[a].clamp(1.0, 10000.0);
    updated_temp[a] = clamped_a;
    let clamped_b = updated_temp[b].clamp(1.0, 10000.0);
    updated_temp[b] = clamped_b;
}

/// `ElementTemperatureData::GetSurfaceAreaMultiplier` — 选择 surface_area_multiplier。
///
/// 严格对照源码 11_msvcrt_ignored.c L33234-33252
/// （ElementTemperatureData::GetSurfaceAreaMultiplier）。
///
/// # 算法
/// 根据 `other_state`（对方元素的 state）选择本元素的 SAM：
/// - state & 3 == 1 (Gas): 用 self.gas_surface_area_multiplier
/// - state & 3 == 2 (Liquid): 用 self.liquid_surface_area_multiplier
/// - state & 3 == 3 (Solid): 用 self.solid_surface_area_multiplier
/// - 其他（如 0 Vacuum）: 默认 1.0
///
/// 注意：源码使用 param_1->state（即对方元素）的 state 字段，本实现把对方 state 作为参数。
///
/// **运行时影响**：与原版完全等价。
fn get_surface_area_multiplier(td_self: &crate::element::ElementTemperatureData, other_state: u8) -> f32 {
    match other_state & 3 {
        1 => td_self.gas_surface_area_multiplier,
        2 => td_self.liquid_surface_area_multiplier,
        3 => td_self.solid_surface_area_multiplier,
        _ => 1.0,
    }
}

/// `UpdateTemperatureForBackwallExchange` — 格子与背景墙热交换。
///
/// 严格对照源码 04_temperature.c L674-813（UpdateTemperatureForBackwallExchange）。
///
/// # 参数
/// - `sim_data`：可变 SimData 引用（读 cells + backwall，写 updated_cells + backwall.temperature）
/// - `sim_events`：SimEvents 引用（T11 不使用，T12/T13 触发 backwall transition 时用）
/// - `cell`：格子内部索引
///
/// # 算法概述
/// 1. 边界检查：cell 在 element_idx / backwall.element_idx 范围内
/// 2. 跳过 vacuum/void 元素（双方任一是 vacuum 或 void_element_idx → 直接返回）
/// 3. 查 ElementTemperatureData for cells[A] 和 backwall[A]
/// 4. 计算 effective conductivity：
///    - cell 侧：`GetInsulationValue(cells, A) × thermal_conductivity_cell`
///      （即 `insulation²×1.53787e-5 × thermal_conductivity_cell`，因 GetInsulationValue 内部已平方）
///    - backwall 侧：直接用 `thermal_conductivity_backwall`（无 insulation 调整！）
/// 5. 合并 conductivity：
///    - 如果 `GetInsulationValue(cells, A) >= 1.0`（cell 完全绝缘）：conductivity = cell_cond × backwall_cond
///    - 否则 if `cell_cond <= backwall_cond`：conductivity = cell_cond（取 min）
///    - 否则：conductivity = backwall_cond（保留 backwall_cond）
/// 6. 计算 surface_area_multiplier（双方，基于对方 state）
/// 7. 调用 CalculateTemperatureExchange wrapper（含 swap 处理）
/// 8. 写回 updated_cells.temperature[A] += delta_temp_cell
/// 9. 写回 backwall.temperature[A] = final_temp_backwall（直接覆盖，不累加）
/// 10. T11 不实现 backwall transition 触发（T12/T13 处理）
///
/// # 运行时影响
/// 与原版等价（除 transition 触发部分留给 T12/T13）。
/// 格子温度变化通过 updated_cells 增量累加（与格子-格子交换一致）；
/// 背景墙温度直接覆盖（源码 `*backwall_temp = local_88._0_4_`，非累加）。
pub fn update_temperature_for_backwall_exchange(
    sim_data: &mut SimData,
    _sim_events: &mut SimEvents,
    cell: i32,
) {
    // 源码 L699-701：读 cells + updatedCells + backwall
    if sim_data.cells.ptr.is_null()
        || sim_data.updated_cells.ptr.is_null()
        || sim_data.backwall.ptr.is_null()
    {
        return;
    }
    let cells = unsafe { &*sim_data.cells.ptr };
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    let backwall = unsafe { &mut *sim_data.backwall.ptr };

    let cells_elem = cells.element_idx.as_slice();
    let cells_temp = cells.temperature.as_slice();
    let cells_mass = cells.mass.as_slice();
    let cells_insul = cells.insulation.as_slice();
    let updated_temp = updated_cells.temperature.as_mut_slice();

    let backwall_elem = backwall.element_idx.as_slice();
    let backwall_temp = backwall.temperature.as_mut_slice();
    let backwall_mass = backwall.mass.as_slice();

    let total = cells_elem.len() as i32;
    let total_bw = backwall_elem.len() as i32;
    if cell < 0 || cell >= total || cell >= total_bw {
        return;
    }
    let c = cell as usize;

    // 源码 L708-715：vacuum/void 检查
    let elem_cell = cells_elem[c];
    let elem_backwall = backwall_elem[c];
    // 源码：if cell_elem == vacuum_element_idx OR backwall_elem == vacuum_element_idx
    //       OR cell_elem == void_element_idx OR backwall_elem == void_element_idx → return
    // gSimData + 0x1c = void_element_idx (2B), + 0x1e = vacuum_element_idx (2B)
    if elem_cell == sim_data.vacuum_element_idx
        || elem_backwall == sim_data.vacuum_element_idx
        || elem_cell == sim_data.void_element_idx
        || elem_backwall == sim_data.void_element_idx
    {
        return;
    }

    // 源码 L716-721：查 ElementTemperatureData for 双方
    let td_cell = match get_element_temperature_data(elem_cell) {
        Some(td) => td,
        None => return,
    };
    let td_backwall = match get_element_temperature_data(elem_backwall) {
        Some(td) => td,
        None => return,
    };

    // 源码 L722-725：cell 侧 effective conductivity
    // fVar11 = GetInsulationValue(cells, A) × thermal_conductivity_cell
    // fVar12 = GetInsulationValue(cells, A)  ← 注意是同一个值，不是 cells.insulation[A]
    // fVar15 = thermal_conductivity_backwall（直接用，无 insulation 调整）
    let insulation_factor = get_insulation_value(cells_insul[c]);
    let cell_cond = insulation_factor * td_cell.thermal_conductivity;
    let backwall_cond = td_backwall.thermal_conductivity;

    // 源码 L726-727：surface_area_multiplier
    // fVar13 = GetSurfaceAreaMultiplier(td_cell, td_backwall) ← 基于 backwall state 选择
    // fVar14 = GetSurfaceAreaMultiplier(td_backwall, td_cell) ← 基于 cell state 选择
    let sam_cell = get_surface_area_multiplier(&td_cell, td_backwall.state);
    let sam_backwall = get_surface_area_multiplier(&td_backwall, td_cell.state);

    // 源码 L728-736：合并 conductivity
    // if (fVar12 >= 1.0): fVar15 = exp(log(fVar11) + log(fVar15)) = fVar11 * fVar15
    // else if (fVar11 <= fVar15): fVar15 = fVar11 (min)
    // else: fVar15 保持不变（即取 backwall_cond）
    let combined_conductivity = if insulation_factor >= 1.0 {
        // 双方都接近完全绝缘 → 取乘积
        cell_cond * backwall_cond
    } else if cell_cond <= backwall_cond {
        cell_cond  // min
    } else {
        backwall_cond  // 保留原 backwall_cond
    };

    // 源码 L749-756：调用 wrapper CalculateTemperatureExchange
    // - heat_factor = 0.01（源码 param_1）
    // - surface_area = combined_conductivity × sam_cell × sam_backwall（fVar15 * fVar13 * fVar14）
    // - cap_factor = 0.2
    // - temp_a = cells_temp[c], cap_a = cells_mass[c] * specific_heat_capacity_cell
    // - temp_b = backwall_temp[c], cap_b = backwall_mass[c] * specific_heat_capacity_backwall
    let heat_factor: f32 = 0.01;
    let cap_factor: f32 = 0.2;
    let surface_area = combined_conductivity * sam_cell * sam_backwall;
    let cap_cell = cells_mass[c] * td_cell.specific_heat_capacity;
    let cap_backwall = backwall_mass[c] * td_backwall.specific_heat_capacity;

    let result = calculate_temperature_exchange(
        heat_factor,
        surface_area,
        cap_factor,
        cells_temp[c],
        cap_cell,
        backwall_temp[c],
        cap_backwall,
    );
    // wrapper 返回：
    // - final_temp_hi = final_temp_backwall
    // - final_temp_lo = final_temp_cell
    // - delta_heat = energy_lost_by_cell（不用）

    // 源码 L762-765: updated_cells_temp[c] += final_temp_cell - cells_temp[c]
    // （源码 local_88._4_4_ = field_0x4 = final_temp of param_4 side = cell side）
    let delta_temp_cell = result.final_temp_lo - cells_temp[c];
    updated_temp[c] += delta_temp_cell;

    // 源码 L768-769: backwall_temp[c] = final_temp_backwall（直接覆盖，不累加）
    // （源码 local_88._0_4_ = field_0x0 = final_temp of param_6 side = backwall side）
    backwall_temp[c] = result.final_temp_hi;

    // 源码 L770-803：backwall transition 检查（T12/T13 实现，T11 跳过）
    // 当 backwall_temp 超出 [low_temp - 3, high_temp + 3] 范围时，
    // 应 push 一个 BackwallShouldTransitionInfo 到 simEvents.backwall_should_transition_info
    // 当前实现：暂不处理，留待 T12/T13
}

/// `DoLoadTimeStateTransition` — 加载时一次性相变检查。
///
/// 对照源码 02_save_load.c L104 + 04_temperature.c L7-73（DoLoadTimeStateTransition）。
/// T12 替换为真实实现。当前桩：仅 log。
pub fn do_load_time_state_transition(_sim_data: &mut SimData, _cell_idx: i32) {
    // T12 替换为真实实现
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_data::{BackwallSOA, CellSOA, SimData};
    use crate::sim_events::SimEvents;
    use crate::elements_table::G_ELEMENTS_TABLE;
    use crate::element::ElementTemperatureData;

    /// 辅助：构造测试用 SimData（width=4, height=4 → 内部 2×2，含边界 16 cells）
    /// 并分配 updated_cells + cells + backwall（所有 SOA 字段长度 16，全 0）
    /// 同时在元素表中塞入 1 个 ElementTemperatureData（idx=0）
    fn make_test_sim_data_with_elements() -> SimData {
        // 先在元素表中塞入 1 个 ElementTemperatureData
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
            let mut td = ElementTemperatureData::default();
            td.thermal_conductivity = 1.0;
            td.specific_heat_capacity = 1.0;
            td.gas_surface_area_multiplier = 1.0;
            td.liquid_surface_area_multiplier = 1.0;
            td.solid_surface_area_multiplier = 1.0;
            td.state = 1;  // Gas
            table.temperature_data.push(td);
        }

        let mut sd = SimData::new_zeroed();
        sd.width = 4;
        sd.height = 4;
        sd.num_game_cells = 4;  // 2×2 内部格子
        sd.vacuum_element_idx = 0xfffe;
        sd.void_element_idx = 0xffff;

        // 分配 cells（原始只读）
        let mut cells = Box::new(CellSOA::new());
        cells.element_idx.resize(16, 0);
        cells.temperature.resize(16, 0.0);
        cells.mass.resize(16, 0.0);
        cells.properties.resize(16, 0);
        cells.insulation.resize(16, 0);
        cells.strength_info.resize(16, 0);
        cells.disease_idx.resize(16, 0);
        cells.disease_count.resize(16, 0);
        sd.cells.ptr = Box::into_raw(cells);

        // 分配 updated_cells（写入目标）
        let mut updated = Box::new(CellSOA::new());
        updated.element_idx.resize(16, 0);
        updated.temperature.resize(16, 0.0);
        updated.mass.resize(16, 0.0);
        updated.properties.resize(16, 0);
        updated.insulation.resize(16, 0);
        updated.strength_info.resize(16, 0);
        updated.disease_idx.resize(16, 0);
        updated.disease_count.resize(16, 0);
        sd.updated_cells.ptr = Box::into_raw(updated);

        // 分配 backwall
        let mut backwall = Box::new(BackwallSOA::new());
        backwall.element_idx.resize(16, 0);
        backwall.temperature.resize(16, 0.0);
        backwall.mass.resize(16, 0.0);
        sd.backwall.ptr = Box::into_raw(backwall);

        sd
    }

    /// 辅助：释放 SimData 中通过 Box::into_raw 分配的 CellSOA + BackwallSOA
    /// 并清空元素表（避免污染后续测试）
    fn free_test_sim_data(sd: &mut SimData) {
        unsafe {
            if !sd.cells.ptr.is_null() {
                drop(Box::from_raw(sd.cells.ptr));
                sd.cells.ptr = std::ptr::null_mut();
            }
            if !sd.updated_cells.ptr.is_null() {
                drop(Box::from_raw(sd.updated_cells.ptr));
                sd.updated_cells.ptr = std::ptr::null_mut();
            }
            if !sd.backwall.ptr.is_null() {
                drop(Box::from_raw(sd.backwall.ptr));
                sd.backwall.ptr = std::ptr::null_mut();
            }
        }
        // 清空元素表
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
        }
    }

    /// 跨模块共享的测试序列化锁（与 elements_table::tests 共享）
    use crate::LIB_TESTS_LOCK as PHASE4_TEST_LOCK;

    #[test]
    fn get_insulation_value_zero_byte() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // insulation_byte = 0 → factor = 0
        assert_eq!(get_insulation_value(0), 0.0);
    }

    #[test]
    fn get_insulation_value_max_byte() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // insulation_byte = 255 → factor ≈ 255² × 1.53787e-5 ≈ 0.9999
        let factor = get_insulation_value(255);
        assert!((factor - 1.0).abs() < 0.001, "255 should give factor ~1.0, got {}", factor);
    }

    #[test]
    fn effective_conductivity_basic() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // thermal_conductivity = 2.0, insulation_byte = 100
        // factor = 100² × 1.53787e-5 = 0.153787
        // effective = 0.153787 × 2.0 = 0.307574
        let eff = effective_conductivity(2.0, 100);
        assert!((eff - 0.307574).abs() < 1e-4, "got {}", eff);
    }

    #[test]
    fn combine_conductivity_min_when_one_low() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // insulation_factor_a = 0.5 (< 1.0) → take min
        // cond_a = 0.3, cond_b = 0.5 → min = 0.3
        let combined = combine_conductivity(0.3, 0.5, 0.5, 1.0);
        assert!((combined - 0.3).abs() < 1e-6, "got {}", combined);
    }

    #[test]
    fn combine_conductivity_product_when_both_high() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // insulation_factor_a = 1.0, insulation_factor_b = 1.0 → both >= 1.0 → product
        // cond_a = 0.5, cond_b = 0.7 → product = 0.35
        let combined = combine_conductivity(0.5, 1.0, 0.7, 1.0);
        assert!((combined - 0.35).abs() < 1e-6, "got {}", combined);
    }

    #[test]
    fn get_surface_area_multiplier_gas_state() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut td = ElementTemperatureData::default();
        td.gas_surface_area_multiplier = 1.5;
        td.liquid_surface_area_multiplier = 2.5;
        td.solid_surface_area_multiplier = 3.5;
        // other_state = 1 (Gas) → use gas_surface_area_multiplier
        assert_eq!(get_surface_area_multiplier(&td, 1), 1.5);
        // other_state = 2 (Liquid) → use liquid_surface_area_multiplier
        assert_eq!(get_surface_area_multiplier(&td, 2), 2.5);
        // other_state = 3 (Solid) → use solid_surface_area_multiplier
        assert_eq!(get_surface_area_multiplier(&td, 3), 3.5);
        // other_state = 0 (Vacuum) → default 1.0
        assert_eq!(get_surface_area_multiplier(&td, 0), 1.0);
    }

    #[test]
    fn update_temperature_null_cells_returns_early() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = SimData::new_zeroed();
        let mut events = SimEvents::default();
        // cells.ptr 和 updated_cells.ptr 都为 null
        update_temperature(&mut sd, &mut events, 0, 1);
        // 不 panic 即可
    }

    #[test]
    fn update_temperature_skips_out_of_range() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        let mut events = SimEvents::default();
        // 越界索引
        update_temperature(&mut sd, &mut events, 999, 0);
        update_temperature(&mut sd, &mut events, -1, 0);
        // 不 panic 且温度未被修改
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.temperature.get(0), 0.0);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn update_temperature_skips_when_mass_nonzero_but_temp_zero() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        let mut events = SimEvents::default();
        // 设置 cell 0: mass=1.0, temp=0.0（非法）
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.mass.set(0, 1.0);
            cells.temperature.set(0, 0.0);
            // cell 1: mass=1.0, temp=100.0
            cells.mass.set(1, 1.0);
            cells.temperature.set(1, 100.0);
        }
        update_temperature(&mut sd, &mut events, 0, 1);
        // 应跳过，updated_cells.temperature 不变
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.temperature.get(0), 0.0);
            assert_eq!(updated.temperature.get(1), 0.0);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn update_temperature_basic_heat_exchange() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        let mut events = SimEvents::default();
        // 设置 cell 0: temp=100, mass=1, elem=0, insulation=0（factor=0）
        // 设置 cell 1: temp=200, mass=1, elem=0, insulation=0
        // 注意 insulation=0 时 factor=0, 所以 combined_conductivity = min(0, 0) = 0
        // → surface_area = 0 → candidate_conductivity = 0 → min = 0 → 无热量交换
        // 这是源码的边界 case（insulation=0 完全不导热）
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(0, 0);
            cells.element_idx.set(1, 0);
            cells.mass.set(0, 1.0);
            cells.mass.set(1, 1.0);
            cells.temperature.set(0, 100.0);
            cells.temperature.set(1, 200.0);
            cells.insulation.set(0, 0);
            cells.insulation.set(1, 0);
            // 模拟真实帧初始化：updated_cells.temperature 镜像 cells.temperature
            // （源码每帧开始时 SimBase::UpdateData 会把 updated_cells 拷贝自 cells）
            let updated = &mut *sd.updated_cells.ptr;
            updated.temperature.set(0, 100.0);
            updated.temperature.set(1, 200.0);
        }
        update_temperature(&mut sd, &mut events, 0, 1);
        // 应无热量交换（conductivity = 0）：updated_cells.temperature 保持原值
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.temperature.get(0), 100.0, "no heat exchange: t0 should stay at 100");
            assert_eq!(updated.temperature.get(1), 200.0, "no heat exchange: t1 should stay at 200");
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn update_temperature_heat_exchange_with_insulation() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        let mut events = SimEvents::default();
        // cell 0: temp=100, mass=1, elem=0, insulation=100（factor≈0.154, cond=0.154×1.0=0.154）
        // cell 1: temp=200, mass=1, elem=0, insulation=100（同上）
        // 双方 insulation_factor 都 < 1.0 → combined = min(0.154, 0.154) = 0.154
        // surface_area = 0.154 × 1.0 × 1.0 = 0.154
        // candidates: [100×0.154×0.2=3.08, 100×0.25×1=25, 100×0.25×1=25] → min = 3.08
        // final_lo = 100 + 3.08/1 = 103.08
        // final_hi = 200 - 3.08/1 = 196.92
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(0, 0);
            cells.element_idx.set(1, 0);
            cells.mass.set(0, 1.0);
            cells.mass.set(1, 1.0);
            cells.temperature.set(0, 100.0);
            cells.temperature.set(1, 200.0);
            cells.insulation.set(0, 100);
            cells.insulation.set(1, 100);
        }
        update_temperature(&mut sd, &mut events, 0, 1);
        // 验证 updated_cells.temperature 有变化
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            let t0 = updated.temperature.get(0);
            let t1 = updated.temperature.get(1);
            // 期望 final_temp_lo ≈ 103.08, final_temp_hi ≈ 196.92
            // updated_temp[0] += 103.08 - 100 = 3.08 → updated_temp[0] = 0 + 3.08 = 3.08
            // updated_temp[1] += 196.92 - 200 = -3.08 → updated_temp[1] = 0 - 3.08 = -3.08
            // 但 clamp 后 [1.0, 10000.0]：3.08 保留，-3.08 → 1.0
            assert!((t0 - 3.08).abs() < 0.5, "t0 should be ~3.08, got {}", t0);
            assert_eq!(t1, 1.0, "t1 should be clamped to 1.0, got {}", t1);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn update_temperature_for_backwall_exchange_null_returns_early() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = SimData::new_zeroed();
        let mut events = SimEvents::default();
        // 所有 ptr 都为 null
        update_temperature_for_backwall_exchange(&mut sd, &mut events, 0);
        // 不 panic 即可
    }

    #[test]
    fn update_temperature_for_backwall_exchange_skips_vacuum() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        let mut events = SimEvents::default();
        // 设置 cell 0 元素为 vacuum_element_idx
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(0, sd.vacuum_element_idx);
            cells.temperature.set(0, 100.0);
            cells.mass.set(0, 1.0);
            let backwall = &mut *sd.backwall.ptr;
            backwall.element_idx.set(0, 0);
            backwall.temperature.set(0, 200.0);
            backwall.mass.set(0, 1.0);
        }
        update_temperature_for_backwall_exchange(&mut sd, &mut events, 0);
        // 应跳过，温度不变
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.temperature.get(0), 0.0);
            let backwall = &*sd.backwall.ptr;
            assert_eq!(backwall.temperature.get(0), 200.0);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn update_temperature_for_backwall_exchange_basic() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        let mut events = SimEvents::default();
        // cell: temp=100, mass=1, elem=0, insulation=0 → factor=0
        //   cell_cond = 0 × 1.0 = 0
        // backwall: temp=200, mass=1, elem=0, cond=1.0
        // 因 insulation_factor=0 < 1.0, cell_cond(0) <= backwall_cond(1.0) → combined=0
        // → surface_area=0 → no heat exchange
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(0, 0);
            cells.temperature.set(0, 100.0);
            cells.mass.set(0, 1.0);
            cells.insulation.set(0, 0);
            let backwall = &mut *sd.backwall.ptr;
            backwall.element_idx.set(0, 0);
            backwall.temperature.set(0, 200.0);
            backwall.mass.set(0, 1.0);
        }
        update_temperature_for_backwall_exchange(&mut sd, &mut events, 0);
        // 验证无热交换
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.temperature.get(0), 0.0, "no exchange expected");
            let backwall = &*sd.backwall.ptr;
            // backwall_temp 直接覆盖为 final_temp_backwall = 200（无交换）
            assert_eq!(backwall.temperature.get(0), 200.0);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn update_temperature_for_backwall_exchange_with_insulation() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        let mut events = SimEvents::default();
        // cell: temp=100, mass=1, elem=0, insulation=100
        //   insulation_factor = 100²×1.53787e-5 = 0.153787
        //   cell_cond = 0.153787 × 1.0 = 0.153787
        // backwall: temp=200, mass=1, elem=0, backwall_cond = 1.0
        // 因 insulation_factor(0.154) < 1.0, cell_cond(0.154) <= backwall_cond(1.0) → combined=0.154
        // surface_area = 0.154 × 1.0 × 1.0 = 0.154
        // heat_factor = 0.01
        // candidates: [100×0.154×0.2=3.08, 100×0.01×1=1.0, 100×0.01×1=1.0] → min = 1.0
        // final_temp_lo(cell) = 100 + 1.0/1 = 101
        // final_temp_hi(backwall) = 200 - 1.0/1 = 199
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(0, 0);
            cells.temperature.set(0, 100.0);
            cells.mass.set(0, 1.0);
            cells.insulation.set(0, 100);
            let backwall = &mut *sd.backwall.ptr;
            backwall.element_idx.set(0, 0);
            backwall.temperature.set(0, 200.0);
            backwall.mass.set(0, 1.0);
        }
        update_temperature_for_backwall_exchange(&mut sd, &mut events, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // updated_temp[0] += 101 - 100 = 1.0
            let t_cell = updated.temperature.get(0);
            assert!((t_cell - 1.0).abs() < 0.5, "cell temp should be ~1.0 (delta from 100→101), got {}", t_cell);
            let backwall = &*sd.backwall.ptr;
            // backwall_temp 直接覆盖为 199（无累加）
            let t_bw = backwall.temperature.get(0);
            assert!((t_bw - 199.0).abs() < 0.5, "backwall temp should be ~199, got {}", t_bw);
        }
        free_test_sim_data(&mut sd);
    }
}
