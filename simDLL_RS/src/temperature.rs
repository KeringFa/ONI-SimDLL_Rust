//! Temperature — 温度模拟模块（Phase 7 T11-T13 实现）。
//!
//! 对照源码 04_temperature.c。
//! T2 期间提供 do_load_time_state_transition 桩以编译通过。
//! T11 实现：
//! - effective_conductivity / get_insulation_value（格子-格子热交换的导热系数计算）
//! - update_temperature（两相邻格子热交换）
//! - update_temperature_for_backwall_exchange（格子与背景墙热交换）
//! T12 实现：
//! - do_load_time_state_transition（加载时一次性相变检查，对照源码 L7-73）
//! - do_state_transition（运行时相变触发，对照源码 L78-403）
//!   含 SpawnOre / ChangeSubstance / ClearCell / ModifyDiseaseCount 辅助函数
//! T13 将实现 partial_heat_transition 等。

use crate::sim_data::SimData;
use crate::sim_events::SimEvents;
use crate::elements_table::get_element_temperature_data;
use crate::vector_math::calculate_temperature_exchange;
use crate::element::ElementTemperatureData;
use crate::game_data::{
    CellMeltedInfo, SpawnFallingLiquidInfo, SpawnOreInfo, SubstanceChangeInfo,
};

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
/// 严格对照源码 04_temperature.c L7-73（DoLoadTimeStateTransition）。
///
/// # 参数
/// - `sim_data`：可变 SimData 引用（读 + 写 `updated_cells`，不读 `cells`）
/// - `cell_idx`：内部格子索引
///
/// # 算法
/// 读 `updated_cells.element_idx[cell_idx]` → 查 `ElementTemperatureData` →
/// 读 `updated_cells.temperature[cell_idx]` →
/// - 若 `temp >= low_temp - 3.0` OR `low_temp_transition_idx == 0xffff`：
///   - 若 `temp > high_temp + 3.0` AND `high_temp_transition_idx != 0xffff`：
///     - `temperature -= 1.5`（让温度退出相变带）
///     - `element_idx = high_temp_transition_idx`
/// - 否则：触发低温相变
///   - `temperature += 1.5`
///   - `element_idx = low_temp_transition_idx`
///
/// # 运行时影响
/// 与原版完全等价。仅修改 `updated_cells`，不触发 SimEvents 事件（无 SpawnOre /
/// ChangeSubstance 等）。用于存档加载时一次性"补"已发生的相变（如岩浆冷却为岩石）。
pub fn do_load_time_state_transition(sim_data: &mut SimData, cell_idx: i32) {
    // 源码 L20：uVar3 = param_1->updatedCells（注意：updatedCells 不是 cells！）
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };

    // 源码 L22-23：uVar9 < element_idx.len()
    let elem_idx_slice = updated_cells.element_idx.as_slice();
    if cell_idx < 0 || cell_idx as usize >= elem_idx_slice.len() {
        return;
    }
    let elem_idx = elem_idx_slice[cell_idx as usize];

    // 源码 L25-26：查 ElementTemperatureData，确认 idx 在范围内
    let td = match get_element_temperature_data(elem_idx) {
        Some(td) => td,
        None => return,
    };

    // 源码 L28-31：uVar9 < temperature.len()
    let temp_slice = updated_cells.temperature.as_slice();
    if cell_idx as usize >= temp_slice.len() {
        return;
    }
    let cur_temp = temp_slice[cell_idx as usize];

    // 源码 L32-43：if (low_temp - 3.0 <= temp) || (low_temp_transition_idx == 0xffff)
    //              → 跳过低温相变，进入高温相变检查
    // 否则 → 触发低温相变
    if cur_temp >= td.low_temp - 3.0 || td.low_temp_transition_idx == 0xffff {
        // 高温相变检查（源码 L34-42）
        // 源码 L35：(high_temp + 3.0 < temp) AND (high_temp_transition_idx != 0xffff)
        if cur_temp > td.high_temp + 3.0 && td.high_temp_transition_idx != 0xffff {
            // 源码 L36：temperature -= 1.5（让温度退出相变带）
            let new_temp = cur_temp - 1.5;
            let temp_mut = updated_cells.temperature.as_mut_slice();
            temp_mut[cell_idx as usize] = new_temp;
            // 源码 L40-41：element_idx = high_temp_transition_idx
            let elem_mut = updated_cells.element_idx.as_mut_slice();
            elem_mut[cell_idx as usize] = td.high_temp_transition_idx;
        }
    } else {
        // 触发低温相变（源码 L44-50）
        // 源码 L46：temperature += 1.5
        let new_temp = cur_temp + 1.5;
        let temp_mut = updated_cells.temperature.as_mut_slice();
        temp_mut[cell_idx as usize] = new_temp;
        // 源码 L50：element_idx = low_temp_transition_idx
        let elem_mut = updated_cells.element_idx.as_mut_slice();
        elem_mut[cell_idx as usize] = td.low_temp_transition_idx;
    }

    // 源码 L52-66：debug assert temperature <= 10000（原版 KCrashReporter）
    // Rust 实现：仅记日志（不 panic）
    let temp_after = updated_cells.temperature.as_slice()[cell_idx as usize];
    if temp_after > 10000.0 {
        tracing::warn!(
            cell = cell_idx,
            temp = temp_after,
            "DoLoadTimeStateTransition: temperature > 10000 after transition"
        );
    }
}

/// `DoStateTransition` — 运行时相变触发。
///
/// 严格对照源码 04_temperature.c L78-403（DoStateTransition）。
///
/// # 参数
/// - `sim_data`：可变 SimData 引用（读 + 写 `updated_cells`）
/// - `sim_events`：可变 SimEvents 引用（push 各类事件）
/// - `cell_idx`：内部格子索引
/// - `td`：当前格子元素的 `ElementTemperatureData`（调用方预先查表，避免函数内重复查）
///
/// # 返回
/// - `true`：触发了相变（已修改温度/元素/质量，并 push 了相关事件）
/// - `false`：未触发相变（温度在范围内、无相变元素、质量为 0 等）
///
/// # 算法概述
/// 1. 边界检查 mass[cell_idx] > 0（无质量则不触发）
/// 2. 读 temperature[cell_idx]
/// 3. **高温分支**：`temp >= low_temp - 3.0` OR `low_temp_transition_idx == 0xffff`：
///    - 若 `temp <= high_temp + 3.0` → 在范围内，返回 false
///    - 若 `high_temp_transition_idx == 0xffff` → 无相变定义，返回 false
///    - 否则触发高温相变：
///      - `temperature = max(0, temp - 1.5)`
///      - `element_idx = high_temp_transition_idx`
///      - 若有 `high_temp_transition_ore_idx`：spawn ore（质量按 conversion 比例转换）
///      - 若 `properties & 0x40 != 0`：push CellMeltedInfo + 设置 timer bits
///      - 调用 `change_substance` 通知 C#
///      - 返回 true
/// 4. **低温分支**：触发低温相变：
///    - `temperature = max(0, temp + 1.5)`
///    - 若有 `low_temp_transition_ore_idx`：spawn ore（同上）
///    - 查新元素的 ElementTemperatureData：
///      - 若新元素是 Solid（state & 3 == 3）：
///        - 若密度（mass / specific_heat_capacity）<= 0.8：spawn 整格为 ore + ClearCell
///        - 否则：仅 `element_idx = low_temp_transition_idx`
///      - 若新元素是 Gas/Liquid：
///        - 若非 headless 且新元素液态范围包含当前温度：push 到 spawn_liquid_info + ClearCell
///        - 否则：仅 `element_idx = low_temp_transition_idx`
///    - 调用 `change_substance` 通知 C#
///    - 返回 true
///
/// # 运行时影响
/// 与原版完全等价。相变时：
/// - 高温相变：温度降 1.5K（避免下一帧再次触发），元素替换
/// - 低温相变：温度升 1.5K，元素替换
/// - 含 ore 转换的相变：spawn 矿石实体（push 到 SimEvents，由 C# 侧处理）
/// - 液态相变：spawn 液体实体（同上）
/// - Solid 低密度相变：整格清空（ClearCell）+ spawn ore
pub fn do_state_transition(
    sim_data: &mut SimData,
    sim_events: &mut SimEvents,
    cell_idx: i32,
    td: &ElementTemperatureData,
) -> bool {
    // 源码 L106-113：local_80.cells = updatedCells; bounds check mass
    if sim_data.updated_cells.ptr.is_null() {
        return false;
    }
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    let mass_slice = updated_cells.mass.as_slice();

    if cell_idx < 0 || cell_idx as usize >= mass_slice.len() {
        return false;
    }
    // 源码 L112-114：if mass[cell_idx] <= 0.0 → return false
    if mass_slice[cell_idx as usize] <= 0.0 {
        return false;
    }

    // 源码 L115-118：bounds check temperature, read fVar19
    let temp_slice = updated_cells.temperature.as_slice();
    if cell_idx as usize >= temp_slice.len() {
        return false;
    }
    let cur_temp = temp_slice[cell_idx as usize];

    // 源码 L119：if (low_temp - 3.0 <= temp) || (low_temp_transition_idx == 0xffff)
    //            → 跳过低温相变，进入高温相变检查
    if cur_temp >= td.low_temp - 3.0 || td.low_temp_transition_idx == 0xffff {
        // 高温相变检查
        // 源码 L121-122：if (temp <= high_temp + 3.0) return false
        if cur_temp <= td.high_temp + 3.0 {
            return false;
        }
        // 源码 L124-126：if (high_temp_transition_idx == 0xffff) return false
        if td.high_temp_transition_idx == 0xffff {
            return false;
        }

        // ===== 触发高温相变 =====
        // 源码 L127-135：temperature = max(0.0, temp - 1.5)
        let new_temp = (cur_temp - 1.5).max(0.0);
        updated_cells.temperature.as_mut_slice()[cell_idx as usize] = new_temp;

        // 源码 L136-141：element_idx = high_temp_transition_idx
        let elem_slice = updated_cells.element_idx.as_slice();
        if cell_idx as usize >= elem_slice.len() {
            return true;  // 已修改温度，但 element_idx 越界 → 退出
        }
        updated_cells.element_idx.as_mut_slice()[cell_idx as usize] = td.high_temp_transition_idx;

        // 源码 L142-181：high_temp_transition_ore_idx 的 SpawnOre 逻辑
        if td.high_temp_transition_ore_idx != 0xffff {
            spawn_ore_for_transition(
                sim_events,
                sim_data,
                cell_idx,
                td.high_temp_transition_ore_idx,
                td.high_temp_transition_ore_mass_conversion,
                /* force_spawn */ true,
            );
        }

        // 源码 L182-207：if (properties & 0x40 != 0) → push CellMeltedInfo + set timer bits
        let properties_slice = updated_cells.properties.as_slice();
        if (cell_idx as usize) < properties_slice.len()
            && (properties_slice[cell_idx as usize] & 0x40) != 0
        {
            push_cell_melted_and_set_timers(sim_events, sim_data, cell_idx);
        }

        // 源码 L208-210：LAB_18003616f: ChangeSubstance + return true
        change_substance(sim_events, sim_data, cell_idx);
        return true;
    }

    // ===== 触发低温相变 =====
    // 源码 L217-225：temperature = max(0.0, temp + 1.5)
    let new_temp = (cur_temp + 1.5).max(0.0);
    updated_cells.temperature.as_mut_slice()[cell_idx as usize] = new_temp;

    // 源码 L226-262：low_temp_transition_ore_idx 的 SpawnOre 逻辑
    if td.low_temp_transition_ore_idx != 0xffff {
        spawn_ore_for_transition(
            sim_events,
            sim_data,
            cell_idx,
            td.low_temp_transition_ore_idx,
            td.low_temp_transition_ore_mass_conversion,
            /* force_spawn */ true,
        );
    }

    // 源码 L263-312：查新元素的 ElementTemperatureData，按 state 分支处理
    let new_elem_idx = td.low_temp_transition_idx;
    let new_td = get_element_temperature_data(new_elem_idx);

    if let Some(new_td) = new_td {
        // 源码 L269：if ((new_td.state & 3) == 3) — 新元素是 Solid
        if (new_td.state & 3) == 3 {
            // 源码 L270-301：密度检查 mass / specific_heat_capacity <= 0.8
            let mass_now = updated_cells.mass.as_slice()[cell_idx as usize];
            let density = if new_td.specific_heat_capacity > 0.0 {
                mass_now / new_td.specific_heat_capacity
            } else {
                mass_now  // 避免除零，退化为 mass 比较
            };

            if density <= 0.8 {
                // 源码 L287-300：spawn 整格为 ore（mass = 当前 mass），成功则 ClearCell
                let disease_idx = updated_cells.disease_idx.as_slice()[cell_idx as usize];
                let disease_count = updated_cells.disease_count.as_slice()[cell_idx as usize];
                let temp_now = updated_cells.temperature.as_slice()[cell_idx as usize];
                let spawned = spawn_ore(
                    sim_events,
                    sim_data,
                    cell_idx,
                    new_elem_idx,
                    mass_now,
                    temp_now,
                    disease_idx,
                    disease_count,
                    sim_data.debug_properties.is_debug_editing,
                );
                if spawned {
                    clear_cell(sim_data, cell_idx);
                    change_substance(sim_events, sim_data, cell_idx);
                    return true;
                }
            }
            // 源码 L302-310：非低密度或 spawn 失败 → 仅替换 element_idx
            let elem_slice = updated_cells.element_idx.as_slice();
            if (cell_idx as usize) < elem_slice.len() {
                updated_cells.element_idx.as_mut_slice()[cell_idx as usize] = new_elem_idx;
                change_substance(sim_events, sim_data, cell_idx);
                return true;
            }
        } else {
            // 源码 L313-393：新元素是 Gas 或 Liquid
            // 源码 L318-322：读 disease_count、disease_idx、temperature、mass
            let disease_count_slice = updated_cells.disease_count.as_slice();
            let disease_idx_slice = updated_cells.disease_idx.as_slice();
            let mass_slice = updated_cells.mass.as_slice();
            let temp_slice = updated_cells.temperature.as_slice();

            if cell_idx as usize >= disease_count_slice.len()
                || cell_idx as usize >= disease_idx_slice.len()
                || cell_idx as usize >= mass_slice.len()
                || cell_idx as usize >= temp_slice.len()
            {
                // 越界 → 仅替换 element_idx
                let elem_slice = updated_cells.element_idx.as_slice();
                if (cell_idx as usize) < elem_slice.len() {
                    updated_cells.element_idx.as_mut_slice()[cell_idx as usize] = new_elem_idx;
                    change_substance(sim_events, sim_data, cell_idx);
                    return true;
                }
                return true;
            }

            let disease_count_val = disease_count_slice[cell_idx as usize];
            let disease_idx_val = disease_idx_slice[cell_idx as usize];
            let mass_val = mass_slice[cell_idx as usize];
            let temp_val = temp_slice[cell_idx as usize];

            // 源码 L331-382：非 headless 且上方格子不是 solid wall 时，spawn liquid
            let mut did_spawn_liquid = false;
            if !sim_data.headless {
                let width = sim_data.width;
                if width > 0 && cell_idx >= width {
                    let above_idx = cell_idx - width;
                    let props_slice = updated_cells.properties.as_slice();
                    if (above_idx as usize) < props_slice.len()
                        && (props_slice[above_idx as usize] & 2) == 0
                    {
                        // 上方格子不是 solid wall（property bit 0x02 = solid_wall）
                        // 源码 L337-342：检查上方格子是否在可见区域内
                        let game_cell = internal_to_game_cell(sim_data, above_idx);
                        if game_cell >= 0 && game_cell < sim_data.num_game_cells {
                            let visible = if sim_data.visible_grid.ptr.is_null() {
                                false
                            } else {
                                unsafe {
                                    *sim_data.visible_grid.ptr.add(game_cell as usize) != 0
                                }
                            };
                            if visible || sim_data.debug_properties.is_debug_editing {
                                // 源码 L345-349：检查温度是否在新元素的液态范围内
                                // 源码使用 gElements[new_elem_idx].low_temp / high_temp
                                // 我们用 ElementTemperatureData 的 low_temp / high_temp（等价）
                                if temp_val >= new_td.low_temp - 3.0
                                    && temp_val <= new_td.high_temp + 3.0
                                {
                                    // 源码 L350-378：push 到 spawn_liquid_info + ClearCell
                                    push_spawn_liquid(
                                        sim_events,
                                        game_cell,
                                        new_elem_idx,
                                        disease_idx_val,
                                        mass_val,
                                        temp_val,
                                        disease_count_val,
                                    );
                                    clear_cell(sim_data, cell_idx);
                                    did_spawn_liquid = true;
                                }
                            }
                        }
                    }
                }
            }

            if !did_spawn_liquid {
                // 源码 L384-390：非液态或 headless 模式 → 仅替换 element_idx
                let elem_slice = updated_cells.element_idx.as_slice();
                if (cell_idx as usize) < elem_slice.len() {
                    updated_cells.element_idx.as_mut_slice()[cell_idx as usize] = new_elem_idx;
                }
            }

            // 源码 L389-390 + L208-210：goto LAB_18003616f → ChangeSubstance + return
            change_substance(sim_events, sim_data, cell_idx);
            return true;
        }
    } else {
        // 源码未明确处理 new_td 不存在的情况（原版会 _invalid_parameter_noinfo_noreturn）
        // Rust 实现：仅替换 element_idx（保守降级）
        let elem_slice = updated_cells.element_idx.as_slice();
        if (cell_idx as usize) < elem_slice.len() {
            updated_cells.element_idx.as_mut_slice()[cell_idx as usize] = new_elem_idx;
        }
        change_substance(sim_events, sim_data, cell_idx);
        return true;
    }

    // 不应到达此处（所有分支都已 return），保守返回 true（温度已修改）
    true
}

// ===== T12 辅助函数（对照源码 11_msvcrt_ignored.c）=====

/// `SimEvents::SpawnOre` — push 一个 SpawnOreInfo 到 sim_events.spawn_ore_info。
///
/// 严格对照源码 11_msvcrt_ignored.c L33307-33359（SimEvents::SpawnOre）。
///
/// # 算法
/// 1. 若 `mass <= 0.0` → 返回 false（不 spawn）
/// 2. 计算 game_cell = internal_to_game_cell(cell_idx)
/// 3. 若 `game_cell` 越界 OR（非 debug 模式 AND 不可见）→ 返回 false
/// 4. push `SpawnOreInfo` 到 `sim_events.spawn_ore_info`
/// 5. 返回 true
///
/// # 运行时影响
/// 与原版等价。`spawn_ore_info` 由 GameDataUpdate 转发给 C# 侧创建掉落的矿石实体。
fn spawn_ore(
    sim_events: &mut SimEvents,
    sim_data: &SimData,
    cell_idx: i32,
    ore_elem_idx: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
    force_spawn: bool,
) -> bool {
    // 源码 L33319-33321：if (mass <= 0.0) return false
    if mass <= 0.0 {
        return false;
    }

    // 源码 L33326-33331：计算 game_cell 并检查可见性
    let game_cell = internal_to_game_cell(sim_data, cell_idx);
    if game_cell < 0 || game_cell >= sim_data.num_game_cells {
        return false;
    }
    if !force_spawn {
        // 源码 L33331：!param_8 && visible_grid[game_cell] == 0 → 返回 false
        if sim_data.visible_grid.ptr.is_null() {
            return false;
        }
        let visible = unsafe { *sim_data.visible_grid.ptr.add(game_cell as usize) };
        if visible == 0 {
            return false;
        }
    }

    // 源码 L33335-33354：push 到 spawn_ore_info
    // 源码中 SpawnOreInfo 的第一个字段（callbackIdx/cell_idx）存储 game_cell
    sim_events.spawn_ore_info.push(SpawnOreInfo {
        cell_idx: game_cell,
        elem_idx: ore_elem_idx,
        disease_idx,
        pad: 0,
        mass,
        temperature,
        disease_count,
    });
    true
}

/// `spawn_ore_for_transition` — 相变时 spawn ore + 调整质量 + 修改 disease_count。
///
/// 对照源码 04_temperature.c L142-181（高温分支）和 L226-262（低温分支）的内联 SpawnOre 逻辑。
///
/// # 算法
/// 1. 计算 ore_mass = mass_conversion × mass[cell_idx]
/// 2. 若 `ore_mass <= 0.001` → 跳过（源码 L150 / L233）
/// 3. bounds check disease_count / disease_idx / temperature 数组
/// 4. 计算 disease_delta = (int)(disease_count × mass_conversion)
/// 5. 调 `spawn_ore(...)`，若返回 true：
///    - `mass[cell_idx] -= ore_mass`
///    - `disease_count[cell_idx] += -disease_delta`（ModifyDiseaseCount）
fn spawn_ore_for_transition(
    sim_events: &mut SimEvents,
    sim_data: &mut SimData,
    cell_idx: i32,
    ore_elem_idx: u16,
    mass_conversion: f32,
    force_spawn: bool,
) {
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };

    // 源码 L148-149：ore_mass = mass_conversion × mass[cell_idx]
    let mass_slice = updated_cells.mass.as_slice();
    if cell_idx < 0 || cell_idx as usize >= mass_slice.len() {
        return;
    }
    let ore_mass = mass_conversion * mass_slice[cell_idx as usize];

    // 源码 L150：if (ore_mass <= 0.001) skip
    if ore_mass <= 0.001 {
        return;
    }

    // 源码 L151-159：bounds check 多个数组（radiation / disease_count / temperature）
    let disease_count_slice = updated_cells.disease_count.as_slice();
    let disease_idx_slice = updated_cells.disease_idx.as_slice();
    let temp_slice = updated_cells.temperature.as_slice();
    if cell_idx as usize >= disease_count_slice.len()
        || cell_idx as usize >= disease_idx_slice.len()
        || cell_idx as usize >= temp_slice.len()
    {
        return;
    }

    // 源码 L161-163：disease_delta = (int)(disease_count × mass_conversion)
    let disease_count_val = disease_count_slice[cell_idx as usize];
    let disease_delta = (disease_count_val as f32 * mass_conversion) as i32;

    // 源码 L164-171：spawn_ore(force_spawn=true)
    let temp_val = temp_slice[cell_idx as usize];
    let disease_idx_val = disease_idx_slice[cell_idx as usize];
    let spawned = spawn_ore(
        sim_events,
        sim_data,
        cell_idx,
        ore_elem_idx,
        ore_mass,
        temp_val,
        disease_idx_val,
        disease_delta,
        force_spawn,
    );

    // 源码 L172-179：if (spawned) mass -= ore_mass; ModifyDiseaseCount(-disease_delta)
    if spawned {
        let mass_mut = updated_cells.mass.as_mut_slice();
        mass_mut[cell_idx as usize] -= ore_mass;

        let dc_mut = updated_cells.disease_count.as_mut_slice();
        dc_mut[cell_idx as usize] = dc_mut[cell_idx as usize].saturating_sub(disease_delta);
    }
}

/// `SimEvents::ChangeSubstance` — push SubstanceChangeInfo + 设置 timer bits。
///
/// 严格对照源码 11_msvcrt_ignored.c L33178-33212（SimEvents::ChangeSubstance）。
///
/// # 算法
/// 1. 计算 game_cell = internal_to_game_cell(cell_idx)
/// 2. 若 `0 <= game_cell < num_game_cells`：
///    - push `SubstanceChangeInfo { cell_idx: game_cell, old_elem: 0xffff, new_elem: 0xffff }`
///      （注：源码 push 时 old/new 都是 0xffff，由 C# 侧填充实际值）
/// 3. **无论是否 push**：`timers[cell_idx] |= 0x1f`（设置 5 个 timer bits）
///
/// # 运行时影响
/// 与原版等价。`substance_change_info` 通知 C# 侧更新格子渲染；
/// `timers[cell_idx] |= 0x1f` 触发该格子周围 5 类定时器重新检查
/// （如 stableCellTicks，用于检测格子是否稳定）。
fn change_substance(sim_events: &mut SimEvents, sim_data: &mut SimData, cell_idx: i32) {
    // 源码 L33188-33192：计算 game_cell
    let game_cell = internal_to_game_cell(sim_data, cell_idx);

    // 源码 L33192-33208：若 game_cell 在范围内，push SubstanceChangeInfo
    if game_cell >= 0 && game_cell < sim_data.num_game_cells {
        sim_events.substance_change_info.push(SubstanceChangeInfo {
            cell_idx: game_cell,
            old_element_idx: 0xffff,
            new_element_idx: 0xffff,
        });
    }

    // 源码 L33203-33204 + L33209-33210：timers[cell_idx] |= 0x1f（无论是否 push 都执行）
    set_timer_bits(sim_data, cell_idx, 0x1f);
}

/// `SimData::ClearCell` — 清空格子为真空状态。
///
/// 严格对照源码 11_msvcrt_ignored.c L23677-23723（SimData::ClearCell）。
///
/// # 算法
/// - `element_idx[cell_idx] = vacuum_element_idx`
/// - `mass[cell_idx] = 0`
/// - `temperature[cell_idx] = 0`
/// - `disease_idx[cell_idx] = 0xff`
/// - `disease_count[cell_idx] = 0`
/// - `disease_infestation_tick_count[cell_idx] = 0`
/// - `disease_growth_accumulated_error[cell_idx] = 0`
///
/// # 运行时影响
/// 与原版等价。整格物质清空，变为真空格子（可被后续气体/液体填充）。
fn clear_cell(sim_data: &mut SimData, cell_idx: i32) {
    if sim_data.updated_cells.ptr.is_null() || cell_idx < 0 {
        return;
    }
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    let i = cell_idx as usize;

    // 源码 L23685：element_idx = vacuum_element_idx
    let elem_slice = updated_cells.element_idx.as_slice();
    if i < elem_slice.len() {
        updated_cells.element_idx.as_mut_slice()[i] = sim_data.vacuum_element_idx;
    }
    // 源码 L23689：mass = 0
    let mass_slice = updated_cells.mass.as_slice();
    if i < mass_slice.len() {
        updated_cells.mass.as_mut_slice()[i] = 0.0;
    }
    // 源码 L23693：temperature = 0
    let temp_slice = updated_cells.temperature.as_slice();
    if i < temp_slice.len() {
        updated_cells.temperature.as_mut_slice()[i] = 0.0;
    }
    // 源码 L23697：disease_idx = 0xff
    let disease_idx_slice = updated_cells.disease_idx.as_slice();
    if i < disease_idx_slice.len() {
        updated_cells.disease_idx.as_mut_slice()[i] = 0xff;
    }
    // 源码 L23701：disease_count = 0
    let dc_slice = updated_cells.disease_count.as_slice();
    if i < dc_slice.len() {
        updated_cells.disease_count.as_mut_slice()[i] = 0;
    }
    // 源码 L23706：disease_infestation_tick_count = 0
    let dit_slice = updated_cells.disease_infestation_tick_count.as_slice();
    if i < dit_slice.len() {
        updated_cells.disease_infestation_tick_count.as_mut_slice()[i] = 0;
    }
    // 源码 L23712：disease_growth_accumulated_error = 0
    let dgae_slice = updated_cells.disease_growth_accumulated_error.as_slice();
    if i < dgae_slice.len() {
        updated_cells.disease_growth_accumulated_error.as_mut_slice()[i] = 0.0;
    }
}

/// 设置 timers[cell_idx] 的指定位（bitwise OR）。
///
/// 对照源码 04_temperature.c L206 `*pbVar13 = *pbVar13 | 0x1f` +
/// 11_msvcrt_ignored.c L33204 `*pbVar2 = *pbVar2 | 0x1f`。
///
/// Timers struct 是 1 字节 bitfield（仅低 5 bits 有效），
/// 源码用 `|= 0x1f` 一次性设置所有 5 个 timer bits。
fn set_timer_bits(sim_data: &mut SimData, cell_idx: i32, bits: u8) {
    if sim_data.timers.ptr.is_null() || cell_idx < 0 {
        return;
    }
    // Timers struct 是 1 字节，timers[cell_idx] 直接寻址
    // 源码：pbVar13 = (byte *)((longlong)param_1->timers + uVar11)
    let timers_ptr = sim_data.timers.ptr as *mut u8;
    unsafe {
        *timers_ptr.add(cell_idx as usize) |= bits;
    }
}

/// push 一个 CellMeltedInfo 到 sim_events.cell_melted_info。
///
/// 对照源码 04_temperature.c L193-203。
///
/// 源码先检查 `pBVar8 == capacity_end`（容量不足）来决定是否 reallocate，
/// Rust 实现用 `MsvcVector::push` 自动处理容量增长。
fn push_cell_melted_and_set_timers(
    sim_events: &mut SimEvents,
    sim_data: &mut SimData,
    cell_idx: i32,
) {
    let game_cell = internal_to_game_cell(sim_data, cell_idx);
    if game_cell >= 0 && game_cell < sim_data.num_game_cells {
        sim_events.cell_melted_info.push(CellMeltedInfo {
            game_cell: game_cell as u32,
        });
    }
    // 源码 L205-206：timers[cell_idx] |= 0x1f
    set_timer_bits(sim_data, cell_idx, 0x1f);
}

/// push 一个 SpawnFallingLiquidInfo 到 sim_events.spawn_liquid_info。
///
/// 对照源码 04_temperature.c L350-378。
fn push_spawn_liquid(
    sim_events: &mut SimEvents,
    game_cell: i32,
    elem_idx: u16,
    disease_idx: u8,
    mass: f32,
    temperature: f32,
    disease_count: i32,
) {
    sim_events.spawn_liquid_info.push(SpawnFallingLiquidInfo {
        cell_idx: game_cell,
        element_idx: elem_idx,
        disease_idx,
        pad: 0,
        mass,
        temperature,
        disease_count,
    });
}

/// 内部格子索引 → 游戏格子索引转换。
///
/// 对照源码 04_temperature.c L189-191 + 11_msvcrt_ignored.c L33189-33191 + L33327-33328。
///
/// 公式：`game_cell = (cell_idx % width) - 1 + ((cell_idx / width) - 1) × (width - 2)`
///
/// 解释：内部网格含 1 格边界（四周各 1 格），游戏格子不含边界。
/// `cell_idx % width` 得到内部 X 坐标，减 1 得到游戏 X 坐标（0-based）
/// `cell_idx / width` 得到内部 Y 坐标，减 1 得到游戏 Y 坐标（0-based）
/// `(width - 2)` 是每行游戏格子数，乘以游戏 Y 得到行偏移
fn internal_to_game_cell(sim_data: &SimData, cell_idx: i32) -> i32 {
    let width = sim_data.width;
    if width <= 0 || cell_idx < 0 {
        return -1;
    }
    let cell = cell_idx as i32;
    // 源码：(cell % width) - 1 + ((cell / width) - 1) × (width - 2)
    (cell % width) - 1 + ((cell / width) - 1) * (width - 2)
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

    // ===== T12 测试 =====

    /// 辅助：为测试分配 timers 数组（num_cells 个字节，全零）。
    /// Timers struct 是 1 字节，所以直接用 u8 数组。
    fn alloc_test_timers(sd: &mut SimData, num_cells: usize) {
        use std::alloc::{alloc_zeroed, Layout};
        let layout = Layout::array::<u8>(num_cells).unwrap();
        let ptr = unsafe { alloc_zeroed(layout) } as *mut crate::sim_data::Timers;
        sd.timers.ptr = ptr;
    }

    /// 辅助：释放 timers 数组。
    fn free_test_timers(sd: &mut SimData, num_cells: usize) {
        use std::alloc::{dealloc, Layout};
        if !sd.timers.ptr.is_null() {
            let layout = Layout::array::<u8>(num_cells).unwrap();
            unsafe { dealloc(sd.timers.ptr as *mut u8, layout); }
            sd.timers.ptr = std::ptr::null_mut();
        }
    }

    /// 辅助：在元素表中设置 idx=0 的元素的相变参数。
    fn set_element0_transitions(
        low_temp: f32,
        high_temp: f32,
        low_transition_idx: u16,
        high_transition_idx: u16,
    ) {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        if table.temperature_data.is_empty() {
            table.temperature_data.push(ElementTemperatureData::default());
        }
        let td = &mut table.temperature_data[0];
        td.low_temp = low_temp;
        td.high_temp = high_temp;
        td.low_temp_transition_idx = low_transition_idx;
        td.high_temp_transition_idx = high_transition_idx;
    }

    #[test]
    fn internal_to_game_cell_basic() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = SimData::new_zeroed();
        sd.width = 4;
        sd.height = 4;
        sd.num_game_cells = 4;  // 2×2 内部格子
        // 内部网格 4×4（含边界），内部坐标 (1,1) → game_cell 0
        // cell_idx=5: (5 % 4)=1, (5 / 4)=1 → (1-1) + (1-1)*(4-2) = 0 ✓
        assert_eq!(internal_to_game_cell(&sd, 5), 0);
        // cell_idx=6: (6 % 4)=2, (6 / 4)=1 → (2-1) + (1-1)*(4-2) = 1
        assert_eq!(internal_to_game_cell(&sd, 6), 1);
        // cell_idx=9: (9 % 4)=1, (9 / 4)=2 → (1-1) + (2-1)*(4-2) = 2
        assert_eq!(internal_to_game_cell(&sd, 9), 2);
        // cell_idx=10: (10 % 4)=2, (10 / 4)=2 → (2-1) + (2-1)*(4-2) = 3
        assert_eq!(internal_to_game_cell(&sd, 10), 3);
        // 边界格子 cell_idx=0: (0 % 4)=0 → (0-1) + ((0/4)-1)*(4-2) = -1 + (-1)*2 = -3（边界外，负值即可）
        assert_eq!(internal_to_game_cell(&sd, 0), -3);
    }

    #[test]
    fn do_load_time_state_transition_null_returns_early() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = SimData::new_zeroed();
        // updated_cells.ptr 为 null
        do_load_time_state_transition(&mut sd, 0);
        // 不 panic 即可
    }

    #[test]
    fn do_load_time_state_transition_high_temp() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        // 设置元素 0：low_temp=100, high_temp=200, high_temp_transition_idx=1
        set_element0_transitions(100.0, 200.0, 0xffff, 1);
        // 设置 cell 0：温度 250（高于 high_temp + 3.0 = 203）
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(0, 0);
            updated.temperature.set(0, 250.0);
        }
        do_load_time_state_transition(&mut sd, 0);
        // 应触发高温相变：温度 -= 1.5 → 248.5，element_idx = 1
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.temperature.get(0) - 248.5).abs() < 0.001,
                "temp should be 248.5, got {}", updated.temperature.get(0));
            assert_eq!(updated.element_idx.get(0), 1,
                "element_idx should be 1 (high_temp_transition_idx)");
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_load_time_state_transition_low_temp() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        // 设置元素 0：low_temp=100, high_temp=200, low_temp_transition_idx=2
        set_element0_transitions(100.0, 200.0, 2, 0xffff);
        // 设置 cell 0：温度 50（低于 low_temp - 3.0 = 97）
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(0, 0);
            updated.temperature.set(0, 50.0);
        }
        do_load_time_state_transition(&mut sd, 0);
        // 应触发低温相变：温度 += 1.5 → 51.5，element_idx = 2
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.temperature.get(0) - 51.5).abs() < 0.001,
                "temp should be 51.5, got {}", updated.temperature.get(0));
            assert_eq!(updated.element_idx.get(0), 2,
                "element_idx should be 2 (low_temp_transition_idx)");
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_load_time_state_transition_within_range_no_change() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        // 设置元素 0：low_temp=100, high_temp=200, 都有 transition_idx
        set_element0_transitions(100.0, 200.0, 2, 1);
        // 设置 cell 0：温度 150（在范围内）
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(0, 0);
            updated.temperature.set(0, 150.0);
        }
        do_load_time_state_transition(&mut sd, 0);
        // 不应触发相变
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.temperature.get(0), 150.0, "temp should be unchanged");
            assert_eq!(updated.element_idx.get(0), 0, "element_idx should be unchanged");
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_load_time_state_transition_no_transition_idx_skip() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        // 元素 0：low/high_transition_idx = 0xffff → 无相变定义
        // 设置 cell 0：温度 250（高于 high_temp + 3.0，但 high_temp_transition_idx == 0xffff）
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data[0].low_temp = 100.0;
            table.temperature_data[0].high_temp = 200.0;
            table.temperature_data[0].low_temp_transition_idx = 0xffff;
            table.temperature_data[0].high_temp_transition_idx = 0xffff;
        }
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(0, 0);
            updated.temperature.set(0, 250.0);
        }
        do_load_time_state_transition(&mut sd, 0);
        // 不应触发相变（虽然温度超出范围，但 transition_idx == 0xffff）
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.temperature.get(0), 250.0, "temp should be unchanged");
            assert_eq!(updated.element_idx.get(0), 0, "element_idx should be unchanged");
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_state_transition_null_returns_false() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = SimData::new_zeroed();
        let mut events = SimEvents::default();
        let td = ElementTemperatureData::default();
        // updated_cells.ptr 为 null
        let result = do_state_transition(&mut sd, &mut events, 0, &td);
        assert!(!result, "should return false for null updated_cells");
    }

    #[test]
    fn do_state_transition_no_mass_returns_false() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        let mut events = SimEvents::default();
        // 设置 cell 0：mass=0（不触发相变）
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.mass.set(0, 0.0);
            updated.temperature.set(0, 250.0);
        }
        let mut td = ElementTemperatureData::default();
        td.low_temp = 100.0;
        td.high_temp = 200.0;
        td.high_temp_transition_idx = 1;
        let result = do_state_transition(&mut sd, &mut events, 0, &td);
        assert!(!result, "should return false when mass <= 0");
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_state_transition_within_range_returns_false() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        let mut events = SimEvents::default();
        // 设置 cell 0：temp=150（在 [low_temp-3, high_temp+3] = [97, 203] 范围内）
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.mass.set(0, 1.0);
            updated.temperature.set(0, 150.0);
            updated.element_idx.set(0, 0);
        }
        let mut td = ElementTemperatureData::default();
        td.low_temp = 100.0;
        td.high_temp = 200.0;
        td.high_temp_transition_idx = 1;
        let result = do_state_transition(&mut sd, &mut events, 0, &td);
        assert!(!result, "should return false when temp is within range");
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_state_transition_high_temp_basic() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        alloc_test_timers(&mut sd, 16);
        let mut events = SimEvents::default();
        // 设置元素 0：low_temp=100, high_temp=200, high_temp_transition_idx=1
        set_element0_transitions(100.0, 200.0, 0xffff, 1);
        // 设置 cell 5（内部坐标 5 → game_cell 0）
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.mass.set(5, 1.0);
            updated.temperature.set(5, 250.0);
            updated.element_idx.set(5, 0);
        }
        let mut td = ElementTemperatureData::default();
        td.low_temp = 100.0;
        td.high_temp = 200.0;
        td.high_temp_transition_idx = 1;
        td.low_temp_transition_idx = 0xffff;
        let result = do_state_transition(&mut sd, &mut events, 5, &td);
        assert!(result, "should return true for high temp transition");
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // 温度应降 1.5：250 - 1.5 = 248.5
            assert!((updated.temperature.get(5) - 248.5).abs() < 0.001,
                "temp should be 248.5, got {}", updated.temperature.get(5));
            // element_idx 应替换为 1
            assert_eq!(updated.element_idx.get(5), 1,
                "element_idx should be 1");
        }
        // 应 push 一个 SubstanceChangeInfo 到 sim_events
        assert_eq!(events.substance_change_info.len(), 1,
            "should push 1 SubstanceChangeInfo");
        free_test_timers(&mut sd, 16);
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_state_transition_high_temp_with_melted_property() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        alloc_test_timers(&mut sd, 16);
        let mut events = SimEvents::default();
        set_element0_transitions(100.0, 200.0, 0xffff, 1);
        // 设置 cell 5：properties |= 0x40（melted 通知位）
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.mass.set(5, 1.0);
            updated.temperature.set(5, 250.0);
            updated.element_idx.set(5, 0);
            updated.properties.set(5, 0x40);
        }
        let mut td = ElementTemperatureData::default();
        td.low_temp = 100.0;
        td.high_temp = 200.0;
        td.high_temp_transition_idx = 1;
        td.low_temp_transition_idx = 0xffff;
        let result = do_state_transition(&mut sd, &mut events, 5, &td);
        assert!(result);
        // 应 push CellMeltedInfo 到 cell_melted_info
        assert_eq!(events.cell_melted_info.len(), 1,
            "should push 1 CellMeltedInfo");
        // timers[5] 应被设置 |= 0x1f
        unsafe {
            let timers_ptr = sd.timers.ptr as *const u8;
            let timer_val = *timers_ptr.add(5);
            assert_eq!(timer_val, 0x1f, "timers[5] should be 0x1f, got {}", timer_val);
        }
        free_test_timers(&mut sd, 16);
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_state_transition_low_temp_gas_to_solid_replace() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        alloc_test_timers(&mut sd, 16);
        let mut events = SimEvents::default();
        // 设置元素 0：low_temp=100, high_temp=200, low_temp_transition_idx=1
        // 元素 1：state=3 (Solid)，specific_heat_capacity=1.0（密度 mass/SHC = 1.0 > 0.8 → 不 spawn ore）
        set_element0_transitions(100.0, 200.0, 1, 0xffff);
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            // 推入元素 1 作为相变目标（Solid，密度高 → 仅替换 element_idx）
            let mut td1 = ElementTemperatureData::default();
            td1.state = 3;  // Solid
            td1.specific_heat_capacity = 1.0;
            if table.temperature_data.len() < 2 {
                table.temperature_data.push(td1);
            } else {
                table.temperature_data[1] = td1;
            }
        }
        // 设置 cell 5：temp=50（低于 low_temp - 3.0 = 97）
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.mass.set(5, 1.0);
            updated.temperature.set(5, 50.0);
            updated.element_idx.set(5, 0);
        }
        let mut td = ElementTemperatureData::default();
        td.low_temp = 100.0;
        td.high_temp = 200.0;
        td.low_temp_transition_idx = 1;
        td.high_temp_transition_idx = 0xffff;
        let result = do_state_transition(&mut sd, &mut events, 5, &td);
        assert!(result, "should return true for low temp transition");
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // 温度应升 1.5：50 + 1.5 = 51.5
            assert!((updated.temperature.get(5) - 51.5).abs() < 0.001,
                "temp should be 51.5, got {}", updated.temperature.get(5));
            // element_idx 应替换为 1（Solid 目标）
            assert_eq!(updated.element_idx.get(5), 1,
                "element_idx should be 1 (solid transition target)");
        }
        assert_eq!(events.substance_change_info.len(), 1);
        free_test_timers(&mut sd, 16);
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_state_transition_low_temp_gas_to_solid_low_density_spawn_ore() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        alloc_test_timers(&mut sd, 16);
        let mut events = SimEvents::default();
        // 元素 0：low_temp=100, high_temp=200, low_temp_transition_idx=1
        set_element0_transitions(100.0, 200.0, 1, 0xffff);
        // 元素 1：state=3 (Solid)，specific_heat_capacity=10.0（密度 mass/SHC = 1/10 = 0.1 <= 0.8 → spawn ore）
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            let mut td1 = ElementTemperatureData::default();
            td1.state = 3;  // Solid
            td1.specific_heat_capacity = 10.0;
            if table.temperature_data.len() < 2 {
                table.temperature_data.push(td1);
            } else {
                table.temperature_data[1] = td1;
            }
        }
        // 设置 cell 5：mass=1.0, temp=50
        // 同时开启 is_debug_editing（让 spawn_ore 跳过 visible_grid 检查，force_spawn=true）
        sd.debug_properties.is_debug_editing = true;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.mass.set(5, 1.0);
            updated.temperature.set(5, 50.0);
            updated.element_idx.set(5, 0);
            updated.disease_idx.set(5, 0);
            updated.disease_count.set(5, 0);
        }
        let mut td = ElementTemperatureData::default();
        td.low_temp = 100.0;
        td.high_temp = 200.0;
        td.low_temp_transition_idx = 1;
        td.high_temp_transition_idx = 0xffff;
        let result = do_state_transition(&mut sd, &mut events, 5, &td);
        assert!(result);
        // 应 spawn ore（push 到 spawn_ore_info）+ ClearCell
        assert_eq!(events.spawn_ore_info.len(), 1, "should spawn 1 ore");
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // ClearCell 应把 mass/temp/element_idx 都清零
            assert_eq!(updated.mass.get(5), 0.0, "mass should be 0 after ClearCell");
            assert_eq!(updated.temperature.get(5), 0.0, "temp should be 0 after ClearCell");
            assert_eq!(updated.element_idx.get(5), sd.vacuum_element_idx,
                "element_idx should be vacuum after ClearCell");
        }
        free_test_timers(&mut sd, 16);
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn do_state_transition_high_temp_with_ore_spawn() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        alloc_test_timers(&mut sd, 16);
        let mut events = SimEvents::default();
        // 元素 0：high_temp_transition_idx=1, high_temp_transition_ore_idx=2,
        // high_temp_transition_ore_mass_conversion=0.5（一半质量转为矿石）
        set_element0_transitions(100.0, 200.0, 0xffff, 1);
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data[0].high_temp_transition_ore_idx = 2;
            table.temperature_data[0].high_temp_transition_ore_mass_conversion = 0.5;
        }
        // 设置 cell 5：mass=2.0, temp=250, disease_count=100
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.mass.set(5, 2.0);
            updated.temperature.set(5, 250.0);
            updated.element_idx.set(5, 0);
            updated.disease_idx.set(5, 0);
            updated.disease_count.set(5, 100);
        }
        let mut td = ElementTemperatureData::default();
        td.low_temp = 100.0;
        td.high_temp = 200.0;
        td.low_temp_transition_idx = 0xffff;
        td.high_temp_transition_idx = 1;
        td.high_temp_transition_ore_idx = 2;
        td.high_temp_transition_ore_mass_conversion = 0.5;
        let result = do_state_transition(&mut sd, &mut events, 5, &td);
        assert!(result);
        // 应 spawn ore：ore_mass = 0.5 × 2.0 = 1.0
        assert_eq!(events.spawn_ore_info.len(), 1, "should spawn 1 ore");
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // mass 应减去 ore_mass：2.0 - 1.0 = 1.0
            assert!((updated.mass.get(5) - 1.0).abs() < 0.001,
                "mass should be 1.0 after ore spawn, got {}", updated.mass.get(5));
            // disease_count 应减去 disease_delta = (int)(100 × 0.5) = 50
            assert_eq!(updated.disease_count.get(5), 50,
                "disease_count should be 50, got {}", updated.disease_count.get(5));
        }
        free_test_timers(&mut sd, 16);
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn clear_cell_zeros_all_fields() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        // 设置 cell 0：所有字段非零
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(0, 5);
            updated.mass.set(0, 100.0);
            updated.temperature.set(0, 500.0);
            updated.disease_idx.set(0, 7);
            updated.disease_count.set(0, 999);
            updated.disease_infestation_tick_count.set(0, 42);
            updated.disease_growth_accumulated_error.set(0, 1.5);
        }
        clear_cell(&mut sd, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(0), sd.vacuum_element_idx);
            assert_eq!(updated.mass.get(0), 0.0);
            assert_eq!(updated.temperature.get(0), 0.0);
            assert_eq!(updated.disease_idx.get(0), 0xff);
            assert_eq!(updated.disease_count.get(0), 0);
            assert_eq!(updated.disease_infestation_tick_count.get(0), 0);
            assert!((updated.disease_growth_accumulated_error.get(0)).abs() < 0.001);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn change_substance_pushes_event_and_sets_timers() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = make_test_sim_data_with_elements();
        alloc_test_timers(&mut sd, 16);
        let mut events = SimEvents::default();
        // cell 5 → game_cell 0
        change_substance(&mut events, &mut sd, 5);
        // 应 push SubstanceChangeInfo
        assert_eq!(events.substance_change_info.len(), 1);
        let sci = events.substance_change_info.get(0);
        assert_eq!(sci.cell_idx, 0, "game_cell should be 0");
        assert_eq!(sci.old_element_idx, 0xffff);
        assert_eq!(sci.new_element_idx, 0xffff);
        // timers[5] 应被设置 |= 0x1f
        unsafe {
            let timers_ptr = sd.timers.ptr as *const u8;
            assert_eq!(*timers_ptr.add(5), 0x1f);
        }
        free_test_timers(&mut sd, 16);
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn set_timer_bits_or_not_overwrite() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sd = SimData::new_zeroed();
        sd.width = 4;
        sd.height = 4;
        alloc_test_timers(&mut sd, 16);
        // 先设置 timers[0] = 0x01
        unsafe {
            let timers_ptr = sd.timers.ptr as *mut u8;
            *timers_ptr = 0x01;
        }
        // OR 0x1f → 应变为 0x1f
        set_timer_bits(&mut sd, 0, 0x1f);
        unsafe {
            let timers_ptr = sd.timers.ptr as *const u8;
            assert_eq!(*timers_ptr, 0x1f, "should be 0x1f after OR");
        }
        // 再次 OR 0x1f → 应保持 0x1f（幂等）
        set_timer_bits(&mut sd, 0, 0x1f);
        unsafe {
            let timers_ptr = sd.timers.ptr as *const u8;
            assert_eq!(*timers_ptr, 0x1f, "should still be 0x1f (idempotent)");
        }
        free_test_timers(&mut sd, 16);
    }
}
