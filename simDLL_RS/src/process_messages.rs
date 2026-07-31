//! ProcessMessages — Phase 7 消息处理（应用 C# 格子改造请求）。
//!
//! 对照源码 06_process_messages.c。
//! T7-T10 实现 4 个 Process* 函数；T6 期间为桩。

use crate::sim_data::SimData;
use crate::sim_events::SimFrameInfo;

/// `ProcessCellProperties` — 设置/清除格子属性标志。
///
/// 对照源码 06_process_messages.c L221-322。
/// 遍历 frame.set_cell_properties（每条 12B），game_cell → cell_idx，
/// set_or_clear == 0 → properties[idx] &= !flags，set_or_clear != 0 → properties[idx] |= flags。
/// 若格子非空 → 检查置换需求（DisplaceGas/DisplaceLiquid 桩返回 false，不处理）。
///
/// **运行时影响**：C# 设置格子属性（如 solid/insulated 等）时，对应标志位会真实写入
/// `updated_cells.properties`。游戏画面上对应格子的视觉/物理行为会随属性生效。
pub fn process_cell_properties(frame: &SimFrameInfo, sim_data: &mut SimData) {
    let width = sim_data.width;
    let height_minus_2 = sim_data.height - 2;
    if width <= 2 || height_minus_2 <= 0 {
        return;
    }
    // UniquePtr<CellSOA> 只有 ptr 字段，需 unsafe 解引用
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    let properties = updated_cells.properties.as_mut_slice();
    let mass = updated_cells.mass.as_slice();
    for cp in frame.set_cell_properties.as_slice() {
        if let Some(idx) = crate::sim_frame_manager::game_cell_to_idx(
            cp.game_cell,
            width,
            height_minus_2,
        ) {
            let i = idx as usize;
            if i < properties.len() {
                if cp.set_or_clear == 0 {
                    properties[i] &= !cp.property_flags;
                } else {
                    properties[i] |= cp.property_flags;
                }
                // 若格子非空（mass > 0）→ 检查 DisplaceGas/DisplaceLiquid（桩，跳过）
                if i < mass.len() && mass[i] > 0.0 {
                    // Phase 7：DisplaceGas/DisplaceLiquid 桩返回 false，不处理
                }
            }
        }
    }
}

/// `ProcessCellModifications` — 添加/替换/置换格子元素。
///
/// 对照源码 06_process_messages.c L1252-1502。
/// 遍历 frame.cell_modifications（每条 28B），按 replace_mode 分派：
/// - 0 (Add)：按元素状态调 add_gas/add_liquid/add_solid
/// - 1 (Replace)：replace_element（覆盖 element_idx + mass + temp + disease）
/// - 2 (ReplaceAndDisplace)：replace_and_displace_element（先 Displace 旧元素[桩]再 Replace）
///
/// **运行时影响**：C# 添加/替换格子物质（如放氧气、放水、放石头）时，对应格子的
/// 元素类型、质量、温度、病菌会真实写入 `updated_cells`。游戏画面格子外观会变化。
pub fn process_cell_modifications(frame: &SimFrameInfo, sim_data: &mut SimData) {
    let width = sim_data.width;
    let height_minus_2 = sim_data.height - 2;
    if width <= 2 || height_minus_2 <= 0 {
        return;
    }
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    let elem_idx_arr = updated_cells.element_idx.as_mut_slice();
    let temp_arr = updated_cells.temperature.as_mut_slice();
    let mass_arr = updated_cells.mass.as_mut_slice();
    let disease_idx_arr = updated_cells.disease_idx.as_mut_slice();
    let disease_count_arr = updated_cells.disease_count.as_mut_slice();

    for cm in frame.cell_modifications.as_slice() {
        let idx = match crate::sim_frame_manager::game_cell_to_idx(
            cm.game_cell,
            width,
            height_minus_2,
        ) {
            Some(i) => i as usize,
            None => continue,
        };
        // 越界保护
        if idx >= elem_idx_arr.len() {
            continue;
        }
        let temp = cm.temperature.clamp(0.0, 10000.0);
        let mass = if cm.mass < 0.0 { 0.0 } else { cm.mass };

        match cm.replace_mode {
            0 => {
                // Add：按元素状态调 add_gas/add_liquid/add_solid
                let elem_state = get_element_state(cm.element_idx);
                match elem_state {
                    ElementState::Gas => add_gas(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr),
                    ElementState::Liquid => add_liquid(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr),
                    ElementState::Solid => add_solid(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr),
                    ElementState::Vacuum => {
                        // 真空元素：直接 replace
                        replace_element(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr);
                    }
                }
            }
            1 => {
                // Replace
                replace_element(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr);
            }
            2 => {
                // ReplaceAndDisplace：先 Displace（桩返回 false）再 Replace
                replace_and_displace_element(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr);
            }
            other => {
                tracing::warn!(replace_mode = other, "ProcessCellModifications: unknown replace_mode");
            }
        }
    }
}

/// 元素状态枚举（对照源码 elements.c state 字段，state & 3 决定状态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementState {
    Gas,
    Liquid,
    Solid,
    Vacuum,
}

/// 辅助：查询元素状态（对照源码 g_elements 表）。
///
/// 调用 elements_table 模块查询 state_data。若元素表未初始化或 idx 越界，默认 Solid。
fn get_element_state(elem_idx: u16) -> ElementState {
    crate::elements_table::get_element_state_by_idx(elem_idx).unwrap_or(ElementState::Solid)
}

/// 辅助：AddGas（添加气体到格子）。
///
/// 对照源码 06_process_messages.c AddGas。
/// 共享 add_element_common 逻辑。
fn add_gas(
    idx: usize,
    cm: &crate::sim_events::CellModification,
    mass: f32,
    temp: f32,
    mass_arr: &mut [f32],
    temp_arr: &mut [f32],
    elem_idx_arr: &mut [u16],
    disease_idx_arr: &mut [u8],
    disease_count_arr: &mut [i32],
) {
    add_element_common(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr);
}

/// 辅助：AddLiquid（添加液体到格子）。
fn add_liquid(
    idx: usize,
    cm: &crate::sim_events::CellModification,
    mass: f32,
    temp: f32,
    mass_arr: &mut [f32],
    temp_arr: &mut [f32],
    elem_idx_arr: &mut [u16],
    disease_idx_arr: &mut [u8],
    disease_count_arr: &mut [i32],
) {
    add_element_common(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr);
}

/// 辅助：AddSolid（添加固体到格子）。
fn add_solid(
    idx: usize,
    cm: &crate::sim_events::CellModification,
    mass: f32,
    temp: f32,
    mass_arr: &mut [f32],
    temp_arr: &mut [f32],
    elem_idx_arr: &mut [u16],
    disease_idx_arr: &mut [u8],
    disease_count_arr: &mut [i32],
) {
    add_element_common(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr);
}

/// 辅助：AddElement 通用逻辑（gas/liquid/solid 共享）。
///
/// 对照源码 06_process_messages.c AddElement 通用部分。
/// 若目标格子是真空或同元素 → 叠加 mass + 加权平均温度；
/// 不同元素 → fallback 调 replace_element（DisplaceGas/DisplaceLiquid 桩返回 false 后的行为）。
fn add_element_common(
    idx: usize,
    cm: &crate::sim_events::CellModification,
    mass: f32,
    temp: f32,
    mass_arr: &mut [f32],
    temp_arr: &mut [f32],
    elem_idx_arr: &mut [u16],
    disease_idx_arr: &mut [u8],
    disease_count_arr: &mut [i32],
) {
    let current_elem = elem_idx_arr[idx];
    if current_elem == cm.element_idx || mass_arr[idx] <= 0.0 {
        // 叠加（同元素或目标为空）
        let new_mass = mass_arr[idx] + mass;
        let new_temp = if new_mass > 0.0 {
            calculate_combined_temperatures(mass_arr[idx], temp_arr[idx], mass, temp)
        } else {
            temp
        };
        mass_arr[idx] = new_mass;
        temp_arr[idx] = new_temp;
        elem_idx_arr[idx] = cm.element_idx;
        // 病菌：Disease::AddDiseaseToCell（桩 no-op）
        if cm.disease_count > 0 {
            // Phase 7：桩，不添加病菌
        }
    } else {
        // 不同元素：气体/液体置换（桩返回 false），固体排放失败
        // Phase 7：直接覆盖（与桩 DisplaceGas/DisplaceLiquid 返回 false 后的 fallback 一致）
        replace_element(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr);
    }
}

/// 辅助：ReplaceElement（覆盖格子元素）。
///
/// 对照源码 06_process_messages.c ReplaceElement。
fn replace_element(
    idx: usize,
    cm: &crate::sim_events::CellModification,
    mass: f32,
    temp: f32,
    mass_arr: &mut [f32],
    temp_arr: &mut [f32],
    elem_idx_arr: &mut [u16],
    disease_idx_arr: &mut [u8],
    disease_count_arr: &mut [i32],
) {
    elem_idx_arr[idx] = cm.element_idx;
    mass_arr[idx] = mass;
    temp_arr[idx] = temp;
    disease_idx_arr[idx] = if cm.disease_count > 0 { cm.disease_idx } else { 0 };
    disease_count_arr[idx] = cm.disease_count;
    // 触发 ChangeSubstance 事件（Phase 7：simEvents 暂不写）
}

/// 辅助：ReplaceAndDisplaceElement（先 Displace 旧元素[桩]再 Replace）。
///
/// 对照源码 06_process_messages.c ReplaceAndDisplaceElement。
/// DisplaceGas/DisplaceLiquid 桩返回 false（不处理），fallback 直接 Replace。
fn replace_and_displace_element(
    idx: usize,
    cm: &crate::sim_events::CellModification,
    mass: f32,
    temp: f32,
    mass_arr: &mut [f32],
    temp_arr: &mut [f32],
    elem_idx_arr: &mut [u16],
    disease_idx_arr: &mut [u8],
    disease_count_arr: &mut [i32],
) {
    // DisplaceGas/DisplaceLiquid 桩返回 false（不处理）
    // Phase 7：直接 Replace（与原版 fallback 一致）
    replace_element(idx, cm, mass, temp, mass_arr, temp_arr, elem_idx_arr, disease_idx_arr, disease_count_arr);
}

/// 辅助：混合温度计算（质量加权平均）。
///
/// 对照源码 04_temperature.c L852 CalculateCombinedTemperatures。
/// T = (m1 * t1 + m2 * t2) / (m1 + m2)
/// 若 m1+m2 == 0，返回 t1（防除零）。
fn calculate_combined_temperatures(m1: f32, t1: f32, m2: f32, t2: f32) -> f32 {
    let total_mass = m1 + m2;
    if total_mass <= 0.0 {
        return t1;
    }
    (m1 * t1 + m2 * t2) / total_mass
}

/// `ProcessMassConsumption` — 消耗质量（挖掘等）。
///
/// 对照源码 06_process_messages.c L920-1038。
/// 遍历 frame.mass_consumption_messages（每条 16B），按 use_rectangle 分派：
/// - 0 → flood_removed_stub（退化为 rectangular_removed 用 loop_radius 作半径）
/// - 1 → rectangular_removed（矩形区域内清零 mass + 设置为 vacuum 元素）
///
/// **运行时影响**：C# 挖掘命令（如复制人挖矿）会清空指定矩形区域内格子的质量，
/// 游戏画面上对应格子变为真空（黑色）。Phase 7 简化：FloodRemoved 退化为 RectangularRemoved。
pub fn process_mass_consumption(frame: &SimFrameInfo, sim_data: &mut SimData) {
    let width = sim_data.width;
    let height_minus_2 = sim_data.height - 2;
    if width <= 2 || height_minus_2 <= 0 {
        return;
    }
    for mc in frame.mass_consumption_messages.as_slice() {
        let center_idx = match crate::sim_frame_manager::game_cell_to_idx(
            mc.game_cell,
            width,
            height_minus_2,
        ) {
            Some(i) => i,
            None => continue,
        };
        let radius = mc.loop_radius as i32;
        if mc.use_rectangle != 0 {
            rectangular_removed(sim_data, center_idx, radius, mc.removed_elem_idx);
        } else {
            // Phase 7：FloodRemoved 退化为 RectangularRemoved
            flood_removed_stub(sim_data, center_idx, radius, mc.removed_elem_idx);
        }
        // 回调：emplace MassConsumedCallback 到 simEvents（Phase 7：暂不写）
    }
}

/// 辅助：RectangularRemoved — 矩形区域内清零 mass 并设为 vacuum 元素。
///
/// 对照源码 06_process_messages.c RectangularRemoved。
/// 区域计算：以 center_idx 为中心，radius 为半径的矩形（含边界保护）。
fn rectangular_removed(sim_data: &mut SimData, center_idx: i32, radius: i32, _removed_elem_idx: u16) {
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim_data.width;
    let height = sim_data.height;
    let center_x = center_idx % width;
    let center_y = center_idx / width;
    let min_x = (center_x - radius).max(0);
    let max_x = (center_x + radius).min(width - 1);
    let min_y = (center_y - radius).max(0);
    let max_y = (center_y + radius).min(height - 1);

    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    let mass_arr = updated_cells.mass.as_mut_slice();
    let elem_idx_arr = updated_cells.element_idx.as_mut_slice();
    let temp_arr = updated_cells.temperature.as_mut_slice();
    let vacuum_idx = sim_data.vacuum_element_idx;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let idx = (y * width + x) as usize;
            if idx < mass_arr.len() {
                mass_arr[idx] = 0.0;
                // 设置为真空元素（与原版一致：清空后是 vacuum）
                elem_idx_arr[idx] = vacuum_idx;
                temp_arr[idx] = 0.0;
                // 触发 ConsumedMassInfo 事件（Phase 7：暂不写）
            }
        }
    }
}

/// 辅助：FloodRemoved 桩 — 退化为 RectangularRemoved。
///
/// Phase 7：FloodRemoved（泛洪移除）退化为 RectangularRemoved（矩形移除）。
/// Phase 8/9 补全泛洪算法。
fn flood_removed_stub(sim_data: &mut SimData, center_idx: i32, radius: i32, removed_elem_idx: u16) {
    tracing::trace!(
        center_idx,
        radius,
        "FloodRemoved: degraded to RectangularRemoved (Phase 7)"
    );
    rectangular_removed(sim_data, center_idx, radius, removed_elem_idx);
}

/// 辅助：DisplaceGas 桩 — 返回 false。
///
/// Phase 8 补全真实流体置换。当前返回 false 让调用方走 fallback（直接覆盖）。
pub fn displace_gas(_sim_data: &mut SimData, _idx: usize, _new_elem: u16, _new_mass: f32, _new_temp: f32) -> bool {
    false
}

/// 辅助：DisplaceLiquid 桩 — 返回 false。
pub fn displace_liquid(_sim_data: &mut SimData, _idx: usize, _new_elem: u16, _new_mass: f32, _new_temp: f32) -> bool {
    false
}

/// `ProcessMassEmission` — 排放质量（间歇泉等）。
///
/// 对照源码 06_process_messages.c L1042-1248。
/// 遍历 frame.mass_emission_messages（每条 24B），game_cell → cell_idx，
/// 若目标格子是真空或同元素 → 叠加 mass + 加权平均温度；
/// 若不同元素 → DisplaceGas/DisplaceLiquid（桩）或固体排放失败。
/// 病菌：Disease::AddDiseaseToCell（桩 no-op）。
///
/// **运行时影响**：C# 间歇泉排放、玩家放生物等会向格子添加物质。
/// 真空格子或同元素格子叠加质量 + 加权温度；不同元素 Phase 7 直接覆盖。
pub fn process_mass_emission(frame: &SimFrameInfo, sim_data: &mut SimData) {
    let width = sim_data.width;
    let height_minus_2 = sim_data.height - 2;
    if width <= 2 || height_minus_2 <= 0 {
        return;
    }
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    let elem_idx_arr = updated_cells.element_idx.as_mut_slice();
    let temp_arr = updated_cells.temperature.as_mut_slice();
    let mass_arr = updated_cells.mass.as_mut_slice();

    for me in frame.mass_emission_messages.as_slice() {
        let idx = match crate::sim_frame_manager::game_cell_to_idx(
            me.game_cell,
            width,
            height_minus_2,
        ) {
            Some(i) => i as usize,
            None => continue,
        };
        let temp = me.temperature.clamp(0.0, 10000.0);
        let mass = if me.mass < 0.0 { 0.0 } else { me.mass };

        if idx >= elem_idx_arr.len() {
            continue;
        }

        let current_elem = elem_idx_arr[idx];
        let current_mass = mass_arr[idx];

        if current_elem == me.element_idx || current_mass <= 0.0 {
            // 真空或同元素：叠加 mass + 加权平均温度
            let new_mass = current_mass + mass;
            let new_temp = if new_mass > 0.0 {
                calculate_combined_temperatures(current_mass, temp_arr[idx], mass, temp)
            } else {
                temp
            };
            mass_arr[idx] = new_mass;
            temp_arr[idx] = new_temp;
            elem_idx_arr[idx] = me.element_idx;
            // 病菌：Disease::AddDiseaseToCell（桩 no-op）
            // 元素变更：emplace SubstanceChangeInfo 到 simEvents（Phase 7：暂不写）
        } else {
            // 不同元素：气体/液体置换（桩返回 false）→ fallback 直接覆盖
            let elem_state = get_element_state(current_elem);
            let displaced = match elem_state {
                ElementState::Gas => displace_gas(sim_data, idx, me.element_idx, mass, temp),
                ElementState::Liquid => displace_liquid(sim_data, idx, me.element_idx, mass, temp),
                _ => false,  // 固体排放失败
            };
            if !displaced {
                // Phase 7：直接覆盖（fallback）
                elem_idx_arr[idx] = me.element_idx;
                mass_arr[idx] = mass;
                temp_arr[idx] = temp;
            }
        }
        // 回调：emplace MassEmittedCallback 到 simEvents（Phase 7：暂不写）
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_data::{CellSOA, SimData};
    use crate::sim_events::{CellPropertiesChange, SimFrameInfo};

    /// 辅助：构造测试用 SimData（width=6, height=6 → 内部 4×4，含边界 36 cells）
    /// 并分配 updated_cells（所有 SOA 字段长度 36，全 0）
    fn make_test_sim_data() -> SimData {
        let mut sd = SimData::new_zeroed();
        sd.width = 6;
        sd.height = 6;
        sd.num_game_cells = 16;  // 4×4 内部格子
        let mut cells = Box::new(CellSOA::new());
        cells.element_idx.resize(36, 0);
        cells.temperature.resize(36, 0.0);
        cells.mass.resize(36, 0.0);
        cells.properties.resize(36, 0);
        cells.insulation.resize(36, 0);
        cells.strength_info.resize(36, 0);
        cells.disease_idx.resize(36, 0);
        cells.disease_count.resize(36, 0);
        sd.updated_cells.ptr = Box::into_raw(cells);
        sd
    }

    /// 辅助：释放 SimData 中通过 Box::into_raw 分配的 CellSOA
    fn free_test_sim_data(sd: &mut SimData) {
        if !sd.updated_cells.ptr.is_null() {
            unsafe {
                drop(Box::from_raw(sd.updated_cells.ptr));
            }
            sd.updated_cells.ptr = std::ptr::null_mut();
        }
    }

    #[test]
    fn process_cell_properties_sets_flag() {
        let mut frame = SimFrameInfo::default();
        // game_cell=0 → idx=7（width=6, h-2=4）
        frame.set_cell_properties.push(CellPropertiesChange {
            game_cell: 0,
            callback_idx: 0,
            property_flags: 0x01,
            set_or_clear: 1,
            _pad: [0; 2],
        });
        let mut sd = make_test_sim_data();
        process_cell_properties(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert_eq!(cells.properties.get(7), 0x01, "set 应把 flag 位 |= 0x01");
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_cell_properties_clears_flag() {
        let mut frame = SimFrameInfo::default();
        // 预置 properties[7] = 0xff，再用 set_or_clear=0 清除 0xff
        frame.set_cell_properties.push(CellPropertiesChange {
            game_cell: 0,
            callback_idx: 0,
            property_flags: 0xff,
            set_or_clear: 0,
            _pad: [0; 2],
        });
        let mut sd = make_test_sim_data();
        unsafe {
            (*sd.updated_cells.ptr).properties.set(7, 0xff);
        }
        process_cell_properties(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert_eq!(cells.properties.get(7), 0x00, "clear 应把 flag 位 &= !0xff → 0");
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_cell_properties_skips_invalid_width() {
        let mut sd = SimData::new_zeroed();
        sd.width = 0;  // 无效 width
        let frame = SimFrameInfo::default();
        process_cell_properties(&frame, &mut sd);
        // 不 panic 即可
    }

    #[test]
    fn process_cell_properties_skips_null_updated_cells() {
        // updated_cells.ptr 为 null 应不 panic
        let mut sd = SimData::new_zeroed();
        sd.width = 6;
        sd.height = 6;
        // sd.updated_cells.ptr 保持 null
        let frame = SimFrameInfo::default();
        process_cell_properties(&frame, &mut sd);
        // 不 panic 即可
    }

    #[test]
    fn process_cell_properties_skips_out_of_range_game_cell() {
        // game_cell=9999 远超 num_game_cells，越界保护应跳过
        let mut frame = SimFrameInfo::default();
        frame.set_cell_properties.push(CellPropertiesChange {
            game_cell: 9999,
            callback_idx: 0,
            property_flags: 0x01,
            set_or_clear: 1,
            _pad: [0; 2],
        });
        let mut sd = make_test_sim_data();
        process_cell_properties(&frame, &mut sd);
        // 不 panic 且 properties[7..35] 仍为 0
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            for i in 0..36 {
                assert_eq!(cells.properties.get(i), 0, "idx {} 不应被修改", i);
            }
        }
        free_test_sim_data(&mut sd);
    }

    // ===== Phase 7 T8：process_cell_modifications + 辅助函数测试 =====

    #[test]
    fn calculate_combined_temperatures_weighted_average() {
        // m1=1, t1=100, m2=1, t2=200 → (100+200)/2 = 150
        assert!((calculate_combined_temperatures(1.0, 100.0, 1.0, 200.0) - 150.0).abs() < 0.01);
        // m1=0, t1=0, m2=2, t2=300 → 600/2 = 300
        assert!((calculate_combined_temperatures(0.0, 0.0, 2.0, 300.0) - 300.0).abs() < 0.01);
        // 防除零：m1+m2=0 时返回 t1
        assert!((calculate_combined_temperatures(0.0, 50.0, 0.0, 100.0) - 50.0).abs() < 0.01);
    }

    #[test]
    fn process_cell_modifications_replace_mode_writes_element() {
        // replace_mode=1：直接覆盖 element_idx/mass/temp/disease
        use crate::sim_events::CellModification;
        let mut frame = SimFrameInfo::default();
        frame.cell_modifications.push(CellModification {
            game_cell: 0,            // → idx=7
            callback_idx: 0,
            mass: 100.0,
            temperature: 300.0,
            disease_count: 5,
            element_idx: 1,
            disease_idx: 2,
            replace_mode: 1,         // Replace
            flags: 0,
            _pad: [0; 3],
        });
        let mut sd = make_test_sim_data();
        process_cell_modifications(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert_eq!(cells.element_idx.get(7), 1);
            assert!((cells.mass.get(7) - 100.0).abs() < 0.01);
            assert!((cells.temperature.get(7) - 300.0).abs() < 0.01);
            assert_eq!(cells.disease_idx.get(7), 2);
            assert_eq!(cells.disease_count.get(7), 5);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_cell_modifications_add_mode_stacks_when_same_element() {
        // replace_mode=0 (Add)，目标格已存在同元素 → 叠加 mass + 加权平均温度
        use crate::sim_events::CellModification;
        let mut frame = SimFrameInfo::default();
        frame.cell_modifications.push(CellModification {
            game_cell: 0,
            callback_idx: 0,
            mass: 50.0,
            temperature: 200.0,
            disease_count: 0,
            element_idx: 1,
            disease_idx: 0,
            replace_mode: 0,         // Add
            flags: 0,
            _pad: [0; 3],
        });
        let mut sd = make_test_sim_data();
        // 预置 idx=7 已有 element=1, mass=50, temp=100
        unsafe {
            (*sd.updated_cells.ptr).element_idx.set(7, 1);
            (*sd.updated_cells.ptr).mass.set(7, 50.0);
            (*sd.updated_cells.ptr).temperature.set(7, 100.0);
        }
        process_cell_modifications(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            // 叠加后 mass = 50+50 = 100
            assert!((cells.mass.get(7) - 100.0).abs() < 0.01);
            // 加权平均温度 = (50*100 + 50*200) / 100 = 150
            assert!((cells.temperature.get(7) - 150.0).abs() < 0.01);
            assert_eq!(cells.element_idx.get(7), 1);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_cell_modifications_add_mode_replaces_when_different_element() {
        // replace_mode=0 (Add)，目标格是不同元素 → fallback 到 replace
        use crate::sim_events::CellModification;
        let mut frame = SimFrameInfo::default();
        frame.cell_modifications.push(CellModification {
            game_cell: 0,
            callback_idx: 0,
            mass: 30.0,
            temperature: 250.0,
            disease_count: 0,
            element_idx: 2,           // 与目标格 element=1 不同
            disease_idx: 0,
            replace_mode: 0,
            flags: 0,
            _pad: [0; 3],
        });
        let mut sd = make_test_sim_data();
        unsafe {
            (*sd.updated_cells.ptr).element_idx.set(7, 1);
            (*sd.updated_cells.ptr).mass.set(7, 100.0);  // mass>0 → 不同元素
            (*sd.updated_cells.ptr).temperature.set(7, 50.0);
        }
        process_cell_modifications(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            // fallback replace → 完全覆盖
            assert_eq!(cells.element_idx.get(7), 2);
            assert!((cells.mass.get(7) - 30.0).abs() < 0.01);
            assert!((cells.temperature.get(7) - 250.0).abs() < 0.01);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_cell_modifications_skips_null_updated_cells() {
        use crate::sim_events::CellModification;
        let mut frame = SimFrameInfo::default();
        frame.cell_modifications.push(CellModification {
            game_cell: 0,
            callback_idx: 0,
            mass: 100.0,
            temperature: 300.0,
            disease_count: 0,
            element_idx: 1,
            disease_idx: 0,
            replace_mode: 1,
            flags: 0,
            _pad: [0; 3],
        });
        let mut sd = SimData::new_zeroed();
        sd.width = 6;
        sd.height = 6;
        // updated_cells.ptr 保持 null
        process_cell_modifications(&frame, &mut sd);
        // 不 panic 即可
    }

    #[test]
    fn process_cell_modifications_clamps_invalid_temp_and_mass() {
        // 越界保护：temp > 10000 clamp 到 10000；mass < 0 置 0
        use crate::sim_events::CellModification;
        let mut frame = SimFrameInfo::default();
        frame.cell_modifications.push(CellModification {
            game_cell: 0,
            callback_idx: 0,
            mass: -50.0,              // → clamp 到 0
            temperature: 99999.0,    // → clamp 到 10000
            disease_count: 0,
            element_idx: 1,
            disease_idx: 0,
            replace_mode: 1,
            flags: 0,
            _pad: [0; 3],
        });
        let mut sd = make_test_sim_data();
        process_cell_modifications(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert!((cells.mass.get(7) - 0.0).abs() < 0.01, "负 mass 应 clamp 到 0");
            assert!((cells.temperature.get(7) - 10000.0).abs() < 0.01, "超高温应 clamp 到 10000");
        }
        free_test_sim_data(&mut sd);
    }

    // ===== Phase 7 T9：process_mass_consumption + 辅助函数测试 =====

    #[test]
    fn process_mass_consumption_rectangular_removed_clears_mass() {
        // use_rectangle=1，半径 1：以 idx=7 (x=1,y=1) 为中心，覆盖 0..=2 × 0..=2 的 9 格
        use crate::sim_events::MassConsumption;
        let mut frame = SimFrameInfo::default();
        frame.mass_consumption_messages.push(MassConsumption {
            game_cell: 0,
            callback_idx: 0,
            removed_elem_idx: 0,
            loop_radius: 1,
            use_rectangle: 1,
        });
        let mut sd = make_test_sim_data();
        // 预置所有格子 mass=100, element=5
        unsafe {
            for i in 0..36 {
                (*sd.updated_cells.ptr).mass.set(i, 100.0);
                (*sd.updated_cells.ptr).element_idx.set(i, 5);
                (*sd.updated_cells.ptr).temperature.set(i, 300.0);
            }
        }
        process_mass_consumption(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            // 中心 (1,1)=7 应被清空
            assert!((cells.mass.get(7) - 0.0).abs() < 0.01);
            assert_eq!(cells.element_idx.get(7), 0);  // vacuum_element_idx=0 (默认)
            // 角落 (0,0)=0 也应被清空（在 radius=1 范围内）
            assert!((cells.mass.get(0) - 0.0).abs() < 0.01);
            // 边界外 (5,5)=35 应保持不变
            assert!((cells.mass.get(35) - 100.0).abs() < 0.01);
            assert_eq!(cells.element_idx.get(35), 5);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_mass_consumption_flood_degraded_to_rectangular() {
        // use_rectangle=0 (FloodRemoved)，应退化为 RectangularRemoved 不 panic
        use crate::sim_events::MassConsumption;
        let mut frame = SimFrameInfo::default();
        frame.mass_consumption_messages.push(MassConsumption {
            game_cell: 0,
            callback_idx: 0,
            removed_elem_idx: 0,
            loop_radius: 1,
            use_rectangle: 0,
        });
        let mut sd = make_test_sim_data();
        process_mass_consumption(&frame, &mut sd);
        // 中心 idx=7 应被清空（与 rectangular 一致）
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert!((cells.mass.get(7) - 0.0).abs() < 0.01);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn displace_gas_stub_returns_false() {
        let mut sd = make_test_sim_data();
        assert!(!displace_gas(&mut sd, 0, 1, 100.0, 300.0));
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn displace_liquid_stub_returns_false() {
        let mut sd = make_test_sim_data();
        assert!(!displace_liquid(&mut sd, 0, 1, 100.0, 300.0));
        free_test_sim_data(&mut sd);
    }

    // ===== Phase 7 T10：process_mass_emission 测试 =====

    #[test]
    fn process_mass_emission_emits_to_vacuum() {
        // 目标格 mass=0（真空）→ 叠加 + 设元素
        use crate::sim_events::MassEmission;
        let mut frame = SimFrameInfo::default();
        frame.mass_emission_messages.push(MassEmission {
            game_cell: 0,            // → idx=7
            callback_idx_or_temp: 0.0,
            mass: 100.0,
            temperature: 300.0,
            disease_count: 0,
            element_idx: 1,
            disease_idx: 0,
            emitted: 0,
            _pad: [0; 2],
        });
        let mut sd = make_test_sim_data();
        process_mass_emission(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert_eq!(cells.element_idx.get(7), 1);
            assert!((cells.mass.get(7) - 100.0).abs() < 0.01);
            assert!((cells.temperature.get(7) - 300.0).abs() < 0.01);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_mass_emission_stacks_when_same_element() {
        // 同元素叠加 + 加权平均温度
        use crate::sim_events::MassEmission;
        let mut frame = SimFrameInfo::default();
        frame.mass_emission_messages.push(MassEmission {
            game_cell: 0,
            callback_idx_or_temp: 0.0,
            mass: 50.0,
            temperature: 200.0,
            disease_count: 0,
            element_idx: 1,         // 与目标同元素
            disease_idx: 0,
            emitted: 0,
            _pad: [0; 2],
        });
        let mut sd = make_test_sim_data();
        unsafe {
            (*sd.updated_cells.ptr).element_idx.set(7, 1);
            (*sd.updated_cells.ptr).mass.set(7, 50.0);
            (*sd.updated_cells.ptr).temperature.set(7, 100.0);
        }
        process_mass_emission(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert!((cells.mass.get(7) - 100.0).abs() < 0.01);
            assert!((cells.temperature.get(7) - 150.0).abs() < 0.01);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_mass_emission_overwrites_when_different_element() {
        // 不同元素：DisplaceGas/DisplaceLiquid 桩返回 false → fallback 覆盖
        use crate::sim_events::MassEmission;
        let mut frame = SimFrameInfo::default();
        frame.mass_emission_messages.push(MassEmission {
            game_cell: 0,
            callback_idx_or_temp: 0.0,
            mass: 30.0,
            temperature: 250.0,
            disease_count: 0,
            element_idx: 2,         // 与目标 element=1 不同
            disease_idx: 0,
            emitted: 0,
            _pad: [0; 2],
        });
        let mut sd = make_test_sim_data();
        unsafe {
            (*sd.updated_cells.ptr).element_idx.set(7, 1);
            (*sd.updated_cells.ptr).mass.set(7, 100.0);  // mass>0
            (*sd.updated_cells.ptr).temperature.set(7, 50.0);
        }
        process_mass_emission(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert_eq!(cells.element_idx.get(7), 2);
            assert!((cells.mass.get(7) - 30.0).abs() < 0.01);
            assert!((cells.temperature.get(7) - 250.0).abs() < 0.01);
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_mass_emission_skips_invalid_width() {
        let mut sd = SimData::new_zeroed();
        sd.width = 0;
        let frame = SimFrameInfo::default();
        process_mass_emission(&frame, &mut sd);
        // 不 panic
    }
}
