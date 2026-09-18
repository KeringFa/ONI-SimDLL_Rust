//! 病菌格子模拟：扩散（Disease::UpdateCells L26646）+ 生长（Disease::PostProcess L26424）。
//!
//! 对照原版 11_msvcrt_ignored.c + SimDLL_Source.c（阶段 2）：
//! - GetDiffusionScale（L26329）：扩散门控（minDiffusionCount / minDiffusionInfestationTickCount / diffusionScale）；
//! - UpdateCells（L26646-26812）：相邻两格病菌扩散（读 cells 稳定缓冲，写 updatedCells，经 AddDiseaseToCell）；
//! - PostProcess（L26424-26640）：逐格生长（温度因子 + 种群因子 + 辐射杀伤 + 误差结转 + 清除 + tick 递增）。
//!
//! 公式对照 C# Klei.AI.Disease（HalfLifeToGrowthRate / CalculateRangeHalfLife）与
//! Klei.AI.DiseaseGrowthRules.ElemGrowthInfo.CalculateDiseaseCountDelta；dt 固定 0.2。
use crate::a_framework::sim_data::{CellSOA, SimData};
use crate::b_elements::disease::{Disease, RangeInfo};
use crate::d1_activity::RegionBounds;

/// HalfLifeToGrowthRate — 半活期转生长率（C# Klei.AI.Disease L314-322）。
///
/// half_life==0 → 0；half_life==+INF → 1；否则 `powf(2, −dt/half_life)`。
pub(crate) fn half_life_to_growth_rate(half_life: f32, dt: f32) -> f32 {
    if half_life == 0.0 {
        return 0.0;
    }
    if half_life == f32::INFINITY {
        return 1.0;
    }
    f32::powf(2.0, -dt / half_life)
}

/// CalculateRangeHalfLife — 温度带间插值半活期（C# Klei.AI.Disease L348-381）。
///
/// 找第一个 `temp <= range[i]`（range = minViable/minGrowth/maxGrowth/maxViable），
/// lower = max(i−1, 0)、upper = i；无匹配 → lower=upper=3；
/// 最优带 [minGrowth,maxGrowth]（lower==1 && upper==2）或任一半活期 INF → INF；
/// 否则按 t=(temp−range[lower])/(range[upper]−range[lower])（分母≤0 → t=0）插值。
pub(crate) fn calculate_range_half_life(
    temp: f32,
    range: &RangeInfo,
    half_lives: &RangeInfo,
) -> f32 {
    // 找第一个 temp <= range[i]（range 顺序：minViable/minGrowth/maxGrowth/maxViable）
    let vals = [
        range.min_viable,
        range.min_growth,
        range.max_growth,
        range.max_viable,
    ];
    let hls = [
        half_lives.min_viable,
        half_lives.min_growth,
        half_lives.max_growth,
        half_lives.max_viable,
    ];
    let mut lower = 3usize;
    let mut upper = 3usize;
    for i in 0..4usize {
        if temp <= vals[i] {
            lower = i.saturating_sub(1);
            upper = i;
            break;
        }
    }
    // 最优带 [minGrowth, maxGrowth]（lower==1 && upper==2）→ INF（生长率 1.0）
    if lower == 1 && upper == 2 {
        return f32::INFINITY;
    }
    if hls[lower] == f32::INFINITY || hls[upper] == f32::INFINITY {
        return f32::INFINITY;
    }
    let mut t = 0.0;
    let span = vals[upper] - vals[lower];
    if span > 0.0 {
        t = (temp - vals[lower]) / span;
    }
    (1.0 - t) * hls[lower] + t * hls[upper]
}

/// GetDiffusionScale — 病菌扩散比例（原版 L26329）。
///
/// 门控：disease_idx/elem 越界 → 0.0（原版断言，防御语义）；
/// count < minDiffusionCount[elem] → 0.0；
/// tick < minDiffusionInfestationTickCount[elem] → 0.0；
/// 否则返回 diffusionScale[elem]。
pub(crate) fn get_diffusion_scale(disease: &Disease, sim: &SimData, cell: usize) -> f32 {
    let cells = unsafe { &*sim.cells.ptr };
    if cell >= cells.disease_idx.len()
        || cell >= cells.element_idx.len()
        || cell >= cells.disease_count.len()
        || cell >= cells.disease_infestation_tick_count.len()
    {
        return 0.0;
    }
    let d_idx = cells.disease_idx.get(cell);
    let diseases = disease.diseases.as_slice();
    if (d_idx as usize) >= diseases.len() {
        return 0.0;
    }
    let eg = &diseases[d_idx as usize].elem_growth_info;
    let elem = cells.element_idx.get(cell) as usize;
    if elem >= eg.min_diffusion_count.len()
        || elem >= eg.min_diffusion_infestation_tick_count.len()
        || elem >= eg.diffusion_scale.len()
    {
        return 0.0;
    }
    if cells.disease_count.get(cell) < eg.min_diffusion_count.get(elem) {
        return 0.0;
    }
    if cells.disease_infestation_tick_count.get(cell) < eg.min_diffusion_infestation_tick_count.get(elem) {
        return 0.0;
    }
    eg.diffusion_scale.get(elem)
}

/// UpdateCells — 相邻两格病菌扩散（原版 L26646-26812）。
///
/// 读 `cells` 稳定缓冲，写 `updatedCells`（经 AddDiseaseToCell 强度混合）。
/// - 元素相态门控：postProcessData[elem].state&3 必须相同（液/气/固/真空各自同类）；
/// - 同病菌：按 count 差 ×0.125×scale 从高向低流；
/// - 一方无病菌：单向流入（×0.125×scale）；
/// - 异真实病菌：强方（count×strength 大）流向弱方（相等 → b 为源）。
pub(crate) fn update_cells(disease: &Disease, sim: &mut SimData, cell_a: usize, cell_b: usize) {
    let cells = unsafe { &*sim.cells.ptr };
    let len = cells.element_idx.len();
    if cell_a >= len || cell_b >= len {
        return;
    }
    // 元素相态门控（原版 L26679-26681：(state_b ^ state_a) & 3 != 0 → return）
    let ea = cells.element_idx.get(cell_a);
    let eb = cells.element_idx.get(cell_b);
    let sa = crate::b_elements::elements_table::get_element_post_process_data(ea)
        .map(|p| p.state & 3);
    let sb = crate::b_elements::elements_table::get_element_post_process_data(eb)
        .map(|p| p.state & 3);
    if sa.is_none() || sb.is_none() || sa != sb {
        return;
    }
    let da = cells.disease_idx.get(cell_a);
    let db = cells.disease_idx.get(cell_b);
    let ca = cells.disease_count.get(cell_a);
    let cb = cells.disease_count.get(cell_b);
    let diseases = disease.diseases.as_slice();

    // transfer：正 = a 得 b 失；src_disease = 流入病菌（所有分支均赋值或 return）
    let transfer: i32;
    let src_disease: u8;
    if da == db {
        if da == 0xff {
            return;
        }
        // 同病菌：count 差 ×0.125×scale，高→低（原版 L26692-26703）
        let diff = cb - ca;
        let source = if diff > 0 { cell_b } else { cell_a };
        let f = get_diffusion_scale(disease, sim, source) * diff as f32 * 0.125;
        transfer = f as i32;
        src_disease = da;
    } else if da == 0xff {
        // a 无病菌：b → a 单向（原版 L26705-26712）
        let f = get_diffusion_scale(disease, sim, cell_b) * cb as f32 * 0.125;
        transfer = f as i32;
        src_disease = db;
    } else if db == 0xff {
        // b 无病菌：a → b 单向（原版 L26714-26724）
        let f = get_diffusion_scale(disease, sim, cell_a) * ca as f32 * 0.125;
        transfer = -(f as i32);
        src_disease = da;
    } else {
        // 异真实病菌：强方流向弱方（原版 L26726-26766）
        if (da as usize) >= diseases.len() || (db as usize) >= diseases.len() {
            return;
        }
        let sa_s = diseases[da as usize].strength;
        let sb_s = diseases[db as usize].strength;
        if cb as f32 * sb_s < ca as f32 * sa_s {
            // a 更强 → a 失 b 得
            let f = get_diffusion_scale(disease, sim, cell_a) * ca as f32 * 0.125;
            transfer = -(f as i32);
            src_disease = da;
        } else {
            // b 更强（或相等）→ b 失 a 得
            let f = get_diffusion_scale(disease, sim, cell_b) * cb as f32 * 0.125;
            transfer = f as i32;
            src_disease = db;
        }
    }
    if transfer == 0 {
        return;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    crate::c2_physics::liquid_flow::add_disease_to_cell(updated, cell_a, src_disease, transfer);
    crate::c2_physics::liquid_flow::add_disease_to_cell(updated, cell_b, src_disease, -transfer);
}

/// 病菌扩散区域循环（原版 UpdateData L145087-145100）。
///
/// 区域每格：若本格或右邻有病菌 → update_cells(cell, cell+1)；
/// 若本格或下邻有病菌 → update_cells(cell, cell+width)。
/// gDisease 缺失（单星/未建表）→ 跳过。
pub(crate) fn run_disease_diffusion_task(sim: &mut SimData, bounds: RegionBounds) {
    let disease_ptr = crate::globals::G_DISEASE.lock().0;
    if disease_ptr.is_null() {
        return;
    }
    let disease = unsafe { &*disease_ptr };
    let width = sim.width as usize;
    let height = sim.height as usize;
    if width < 3 || height < 3 || sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
                for row in bounds.min_y..bounds.max_y {
                for col in bounds.min_x..bounds.max_x {
            let cell = row * width + col;
            let (right_ok, down_ok) = {
                let cells = unsafe { &*sim.cells.ptr };
                let d = cells.disease_idx.get(cell);
                let right = cell + 1 < cells.disease_idx.len()
                    && (d != 0xff || cells.disease_idx.get(cell + 1) != 0xff);
                let down = cell + width < cells.disease_idx.len()
                    && (d != 0xff || cells.disease_idx.get(cell + width) != 0xff);
                (right, down)
            };
            if right_ok {
                update_cells(disease, sim, cell, cell + 1);
            }
            if down_ok {
                update_cells(disease, sim, cell, cell + width);
            }
        }
    }
}

/// PostProcess 单格生长（原版 L26424-26640 第一遍）。
///
/// 温度因子（calculate_range_half_life + half_life_to_growth_rate，dt=0.2）+
/// 种群因子（count<min → −underPopDeath×0.2；count>max → 过密半活期；否则种群半活期）+
/// radiationEnabled 时 −radiation×radiationKillRate；
/// 汇总后 acc_err 存小数、count 加整数部分。
fn post_process_cell(
    disease: &Disease,
    updated: &mut CellSOA,
    cell: usize,
    radiation_enabled: bool,
) {
    if cell >= updated.element_idx.len()
        || cell >= updated.temperature.len()
        || cell >= updated.mass.len()
        || cell >= updated.disease_idx.len()
        || cell >= updated.disease_count.len()
        || cell >= updated.disease_growth_accumulated_error.len()
        || cell >= updated.disease_infestation_tick_count.len()
    {
        return;
    }
    let d_idx = updated.disease_idx.get(cell);
    if d_idx == 0xff {
        return;
    }
    let diseases = disease.diseases.as_slice();
    if (d_idx as usize) >= diseases.len() {
        return;
    }
    let di = &diseases[d_idx as usize];
    let elem = updated.element_idx.get(cell) as usize;
    let eg = &di.elem_growth_info;
    if elem >= eg.min_count_per_kg.len()
        || elem >= eg.max_count_per_kg.len()
        || elem >= eg.under_population_death_rate.len()
        || elem >= eg.population_half_life.len()
        || elem >= eg.over_population_half_life.len()
    {
        return;
    }

    // A. 温度因子（对照 C# CalculateRangeHalfLife + HalfLifeToGrowthRate）
    let temp = updated.temperature.get(cell);
    let hl = calculate_range_half_life(temp, &di.temperature_range, &di.temperature_half_lives);
    let factor = half_life_to_growth_rate(hl, 0.2);

    // B. 种群因子（对照 C# ElemGrowthInfo.CalculateDiseaseCountDelta，dt=0.2）
    let count_f = updated.disease_count.get(cell) as f32;
    let mass = updated.mass.get(cell);
    let min_c = eg.min_count_per_kg.get(elem) * mass;
    let max_c = eg.max_count_per_kg.get(elem) * mass;
    let growth = if count_f < min_c {
        -eg.under_population_death_rate.get(elem) * 0.2
    } else if count_f > max_c {
        (half_life_to_growth_rate(eg.over_population_half_life.get(elem), 0.2) - 1.0) * count_f
    } else {
        (half_life_to_growth_rate(eg.population_half_life.get(elem), 0.2) - 1.0) * count_f
    };

    // C. 汇总：f25 = (count×factor + acc − count) + growth [− radiation×killRate]
    let acc = updated.disease_growth_accumulated_error.get(cell);
    let mut f25 = (count_f * factor + acc - count_f) + growth;
    if radiation_enabled && cell < updated.radiation.len() {
        f25 -= updated.radiation.get(cell) * di.radiation_kill_rate;
    }
    if !f25.is_finite() {
        return; // 防御（原版依赖数据不变量；越界/除零产生 NaN 时跳过本格）
    }
    let trunc = f25.trunc();
    updated.disease_growth_accumulated_error.set(cell, f25 - trunc);
    updated
        .disease_count
        .set(cell, updated.disease_count.get(cell) + trunc as i32);
}

/// PostProcess 区域循环（原版 L26424-26640，两遍）。
///
/// 第一遍：逐格生长（跳过 disease==0xff）；
/// 第二遍：count<1 → 清四字段；tick = min(tick+1, 0xfe)。
/// 注意原版怪癖：第二遍无条件执行，count<1（含无病菌格）先清零再 +1 → tick=1。
pub(crate) fn run_disease_growth_task(sim: &mut SimData, bounds: RegionBounds) {
    let disease_ptr = crate::globals::G_DISEASE.lock().0;
    if disease_ptr.is_null() {
        return;
    }
    let disease = unsafe { &*disease_ptr };
    let width = sim.width as usize;
    let height = sim.height as usize;
    if width < 3 || height < 3 || sim.updated_cells.ptr.is_null() {
        return;
    }
    let radiation_enabled = sim.radiation_enabled;
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        for row in bounds.min_y..bounds.max_y {
        for col in bounds.min_x..bounds.max_x {
                let cell = row * width + col;
                if cell >= updated.disease_count.len() {
                    continue;
                }
                post_process_cell(disease, updated, cell, radiation_enabled);
            }
        }
    }
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        for row in bounds.min_y..bounds.max_y {
        for col in bounds.min_x..bounds.max_x {
                let cell = row * width + col;
                if cell >= updated.disease_count.len() || cell >= updated.disease_idx.len() {
                    continue;
                }
                if updated.disease_count.get(cell) < 1 {
                    updated.disease_idx.set(cell, 0xff);
                    updated.disease_count.set(cell, 0);
                    updated.disease_infestation_tick_count.set(cell, 0);
                    updated.disease_growth_accumulated_error.set(cell, 0.0);
                }
                let tick = updated.disease_infestation_tick_count.get(cell);
                updated
                    .disease_infestation_tick_count
                    .set(cell, tick.saturating_add(1).min(0xfe));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
    use crate::a_framework::sim_data::SimData;
    use crate::b_elements::disease::{Disease, DiseaseInfo, ElemGrowthInfo};
    use crate::b_elements::element::Element;
    use crate::b_elements::elements_table::{CreateElementsTable, DestroyElementsTable};
    use crate::LIB_TESTS_LOCK;

    /// 建 3 元素最小表：0=气体(state=1)、1=液体(state=2)、2=固体(state=3)。
    /// 调用方需持 LIB_TESTS_LOCK，并在测试末尾 DestroyElementsTable() 清理。
    fn create_state_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(3);
        for (id, state) in [(0i32, 1u8), (1i32, 2u8), (2i32, 3u8)] {
            let mut elem = Element::default();
            elem.id = id;
            elem.state = state;
            elem.number_of_gradient_colors = 1;
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(&elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..3 {
            w.write_int(0); // 空名称
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 2 病菌表挂 G_DISEASE：
    /// - idx0: strength=2.0；idx1: strength=1.0
    /// ElemGrowthInfo（3 元素）：
    ///   diffusionScale=[1.0,0.5,0.0]
    ///   minDiffusionCount=[10,5,100]
    ///   minDiffusionInfestationTickCount=[2,1,3]
    fn install_disease_table() {
        let mut t = Disease::new();
        for (hash, strength) in [(0xAAAAu32, 2.0f32), (0xBBBB, 1.0)] {
            let mut eg = ElemGrowthInfo::default();
            eg.diffusion_scale.resize(3, 0.0);
            eg.diffusion_scale.set(0, 1.0);
            eg.diffusion_scale.set(1, 0.5);
            eg.diffusion_scale.set(2, 0.0);
            eg.min_diffusion_count.resize(3, 0);
            eg.min_diffusion_count.set(0, 10);
            eg.min_diffusion_count.set(1, 5);
            eg.min_diffusion_count.set(2, 100);
            eg.min_diffusion_infestation_tick_count.resize(3, 0);
            eg.min_diffusion_infestation_tick_count.set(0, 2);
            eg.min_diffusion_infestation_tick_count.set(1, 1);
            eg.min_diffusion_infestation_tick_count.set(2, 3);
            t.diseases.push_unchecked(DiseaseInfo {
                hash_id: hash,
                strength,
                elem_growth_info: eg,
                ..Default::default()
            });
        }
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
        }
        g.0 = Box::into_raw(Box::new(t));
    }

    fn uninstall_disease_table() {
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
            g.0 = std::ptr::null_mut();
        }
    }

    fn disease_table_ref() -> &'static Disease {
        unsafe { &*crate::globals::G_DISEASE.lock().0 }
    }

    #[test]
    fn get_diffusion_scale_gates() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        install_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let d = disease_table_ref();
        unsafe {
            let cells = &mut *sd.cells.ptr;
            // elem0（气体）：count=9 < minDiffusionCount=10 → 0.0
            cells.element_idx.set(7, 0);
            cells.disease_idx.set(7, 0);
            cells.disease_count.set(7, 9);
            cells.disease_infestation_tick_count.set(7, 5);
            assert_eq!(get_diffusion_scale(d, &sd, 7), 0.0, "count < minDiffusionCount");
            // elem0：count=100、tick=1 < minInfestationTick=2 → 0.0
            cells.disease_count.set(7, 100);
            cells.disease_infestation_tick_count.set(7, 1);
            assert_eq!(get_diffusion_scale(d, &sd, 7), 0.0, "tick < minInfestationTick");
            // elem0：count=100、tick=5 → diffusionScale[0]=1.0
            cells.disease_infestation_tick_count.set(7, 5);
            assert_eq!(get_diffusion_scale(d, &sd, 7), 1.0);
            // elem1（液体）：scale=0.5
            cells.element_idx.set(8, 1);
            cells.disease_idx.set(8, 0);
            cells.disease_count.set(8, 100);
            cells.disease_infestation_tick_count.set(8, 5);
            assert_eq!(get_diffusion_scale(d, &sd, 8), 0.5);
            // elem2（固体）：scale=0.0
            cells.element_idx.set(9, 2);
            cells.disease_idx.set(9, 0);
            cells.disease_count.set(9, 100);
            cells.disease_infestation_tick_count.set(9, 5);
            assert_eq!(get_diffusion_scale(d, &sd, 9), 0.0);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    /// 设置 cells 与 updatedCells 中指定格：元素/病菌 idx/count/tick。
    fn set_both(sd: &mut SimData, cell: usize, elem: u16, d_idx: u8, d_count: i32, tick: u8) {
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let cells = &mut *buf_ptr;
                cells.element_idx.set(cell, elem);
                cells.disease_idx.set(cell, d_idx);
                cells.disease_count.set(cell, d_count);
                cells.disease_infestation_tick_count.set(cell, tick);
            }
        }
    }

    #[test]
    fn update_cells_same_disease_equilibrates() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        install_disease_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let d = disease_table_ref();
        // a=7（elem0 气体）disease0 count100；b=8（elem0）disease0 count50
        set_both(&mut sd, 7, 0, 0, 100, 5);
        set_both(&mut sd, 8, 0, 0, 50, 5);
        update_cells(d, &mut sd, 7, 8);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 94, "a 高→低 流失 6");
            assert_eq!(u.disease_count.get(8), 56, "b 低→高 获得 6");
            assert_eq!(u.disease_idx.get(7), 0);
            assert_eq!(u.disease_idx.get(8), 0);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn update_cells_empty_a_absorbs_from_b() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        install_disease_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let d = disease_table_ref();
        set_both(&mut sd, 7, 0, 0xff, 0, 0);
        set_both(&mut sd, 8, 0, 0, 100, 5);
        update_cells(d, &mut sd, 7, 8);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 12, "a 空 → 单向流入 12");
            assert_eq!(u.disease_idx.get(7), 0);
            assert_eq!(u.disease_count.get(8), 88);
            assert_eq!(u.disease_idx.get(8), 0);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn update_cells_empty_b_receives_from_a() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        install_disease_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let d = disease_table_ref();
        set_both(&mut sd, 7, 0, 0, 100, 5);
        set_both(&mut sd, 8, 0, 0xff, 0, 0);
        update_cells(d, &mut sd, 7, 8);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 88);
            assert_eq!(u.disease_count.get(8), 12, "b 空 → 单向流入 12");
            assert_eq!(u.disease_idx.get(8), 0);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn update_cells_different_disease_stronger_flows() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        install_disease_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let d = disease_table_ref();
        // a=disease0(s2) count100；b=disease1(s1) count100 → a 更强，transfer=12
        set_both(&mut sd, 7, 0, 0, 100, 5);
        set_both(&mut sd, 8, 0, 1, 100, 5);
        update_cells(d, &mut sd, 7, 8);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 88, "a 强方流失 12");
            assert_eq!(u.disease_idx.get(7), 0);
            // b 收 +12(disease0)：混合 f10=100×1=100、f11=12×2=24、d=-404 → b=404 保持 disease1
            assert_eq!(u.disease_count.get(8), 404, "AddDiseaseToCell 混合结果");
            assert_eq!(u.disease_idx.get(8), 1);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn update_cells_element_state_gate_blocks() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        install_disease_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let d = disease_table_ref();
        // a=气体(elem0) b=液体(elem1) → state&3 不同 → 不扩散
        set_both(&mut sd, 7, 0, 0, 100, 5);
        set_both(&mut sd, 8, 1, 0, 50, 5);
        update_cells(d, &mut sd, 7, 8);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 100, "异相态不扩散");
            assert_eq!(u.disease_count.get(8), 50);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn run_disease_diffusion_task_loops_right_and_down() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        install_disease_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        // 区域仅 cell=14（row2 col2）；右邻 15、下邻 20（均在 bounds 外，只作邻居）
        set_both(&mut sd, 14, 0, 0, 100, 5);
        set_both(&mut sd, 15, 0, 0, 50, 5);
        set_both(&mut sd, 20, 0, 0, 50, 5);
        let bounds = crate::d1_activity::RegionBounds {
    min_x: 2,
    min_y: 2,
    max_x: 3,
    max_y: 3,
        };
        run_disease_diffusion_task(&mut sd, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(14), 88, "右+下各流出 6 → 88");
            assert_eq!(u.disease_count.get(15), 56);
            assert_eq!(u.disease_count.get(20), 56);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    // ===== 任务 3：PostProcess 生长 =====

    /// 1 病菌表（idx0）：
    /// tempRange(0,20,40,60)、tempHalfLives(5,2,2,5)、radiationKillRate=0.5
    /// elem0：minCountPerKG=100 maxCountPerKG=1000 popHL=10 overPopHL=10 underPopDeath=1.0
    fn install_growth_table() {
        let mut t = Disease::new();
        let mut eg = ElemGrowthInfo::default();
        eg.min_count_per_kg.resize(1, 0.0);
        eg.min_count_per_kg.set(0, 100.0);
        eg.max_count_per_kg.resize(1, 0.0);
        eg.max_count_per_kg.set(0, 1000.0);
        eg.population_half_life.resize(1, 0.0);
        eg.population_half_life.set(0, 10.0);
        eg.over_population_half_life.resize(1, 0.0);
        eg.over_population_half_life.set(0, 10.0);
        eg.under_population_death_rate.resize(1, 0.0);
        eg.under_population_death_rate.set(0, 1.0);
        t.diseases.push_unchecked(DiseaseInfo {
            hash_id: 0xD1,
            strength: 1.0,
            temperature_range: RangeInfo {
                min_viable: 0.0,
                min_growth: 20.0,
                max_growth: 40.0,
                max_viable: 60.0,
            },
            temperature_half_lives: RangeInfo {
                min_viable: 5.0,
                min_growth: 2.0,
                max_growth: 2.0,
                max_viable: 5.0,
            },
            elem_growth_info: eg,
            radiation_kill_rate: 0.5,
            ..Default::default()
        });
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
        }
        g.0 = Box::into_raw(Box::new(t));
    }

    /// 设置 updatedCells 指定格（elem0）：温度/质量/病菌 idx/count/tick/acc/radiation。
    #[allow(clippy::too_many_arguments)]
    fn set_updated(
        sd: &mut SimData,
        cell: usize,
        temp: f32,
        mass: f32,
        d_idx: u8,
        d_count: i32,
        tick: u8,
        acc: f32,
        rad: f32,
    ) {
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(cell, 0);
            u.temperature.set(cell, temp);
            u.mass.set(cell, mass);
            u.disease_idx.set(cell, d_idx);
            u.disease_count.set(cell, d_count);
            u.disease_infestation_tick_count.set(cell, tick);
            u.disease_growth_accumulated_error.set(cell, acc);
            u.radiation.set(cell, rad);
        }
    }

    #[test]
    fn post_process_optimal_temp_within_population_range() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_growth_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        // temp=30（最优带 → f=1.0）、count=500、mass=1.0（min=100≤500≤1000）、acc=0
        // growth=(2^-0.02-1)×500=-6.88365；f25=-6.88365 → count=494、acc=-0.88365、tick 5→6
        set_updated(&mut sd, 7, 30.0, 1.0, 0, 500, 5, 0.0, 0.0);
    let bounds = crate::d1_activity::RegionBounds { min_x: 1, min_y: 1, max_x: 4, max_y: 2 };
        run_disease_growth_task(&mut sd, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 494);
            assert!((u.disease_growth_accumulated_error.get(7) - (-0.8836478)).abs() < 1e-5);
            assert_eq!(u.disease_infestation_tick_count.get(7), 6, "存活格 tick+1");
        }
        uninstall_disease_table();
    }

    #[test]
    fn post_process_below_min_viable_decays() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_growth_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        // temp=-10 → hl[0]=5 → f=2^-0.04=0.97265495
        // f25=(500×0.97265495-500)+(-6.88365)=-20.5562 → count=480、acc=-0.5562
        set_updated(&mut sd, 7, -10.0, 1.0, 0, 500, 5, 0.0, 0.0);
    let bounds = crate::d1_activity::RegionBounds { min_x: 1, min_y: 1, max_x: 4, max_y: 2 };
        run_disease_growth_task(&mut sd, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 480);
            assert!((u.disease_growth_accumulated_error.get(7) - (-0.5561741)).abs() < 1e-5);
        }
        uninstall_disease_table();
    }

    #[test]
    fn post_process_under_population_constant_death() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_growth_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        // count=50 < min=100 → growth=-1.0×0.2=-0.2；temp 最优 → f=1.0
        // f25=-0.2 → count+=trunc(-0.2)=0 → 50；acc=-0.2
        set_updated(&mut sd, 7, 30.0, 1.0, 0, 50, 5, 0.0, 0.0);
    let bounds = crate::d1_activity::RegionBounds { min_x: 1, min_y: 1, max_x: 4, max_y: 2 };
        run_disease_growth_task(&mut sd, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 50);
            assert!((u.disease_growth_accumulated_error.get(7) - (-0.2)).abs() < 1e-6);
        }
        uninstall_disease_table();
    }

    #[test]
    fn post_process_over_population_half_life() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_growth_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        // count=2000 > max=1000 → growth=(2^-0.02-1)×2000=-27.5346
        // f25=-27.5346 → count=1973、acc=-0.5346
        set_updated(&mut sd, 7, 30.0, 1.0, 0, 2000, 5, 0.0, 0.0);
    let bounds = crate::d1_activity::RegionBounds { min_x: 1, min_y: 1, max_x: 4, max_y: 2 };
        run_disease_growth_task(&mut sd, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 1973);
            assert!((u.disease_growth_accumulated_error.get(7) - (-0.5346)).abs() < 1e-3);
        }
        uninstall_disease_table();
    }

    #[test]
    fn post_process_accumulated_error_carries() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_growth_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        // acc=0.4、count=500、temp 最优 → f25=0.4-6.88365=-6.48365 → count=494、acc=-0.48365
        set_updated(&mut sd, 7, 30.0, 1.0, 0, 500, 5, 0.4, 0.0);
    let bounds = crate::d1_activity::RegionBounds { min_x: 1, min_y: 1, max_x: 4, max_y: 2 };
        run_disease_growth_task(&mut sd, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 494);
            assert!((u.disease_growth_accumulated_error.get(7) - (-0.4836478)).abs() < 1e-5);
        }
        uninstall_disease_table();
    }

    #[test]
    fn post_process_radiation_kills_when_enabled() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_growth_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        // radiation_enabled=true、radiation=10、killRate=0.5
        // f25=-6.88365-5.0=-11.88365 → count=489、acc=-0.88365
        set_updated(&mut sd, 7, 30.0, 1.0, 0, 500, 5, 0.0, 10.0);
    let bounds = crate::d1_activity::RegionBounds { min_x: 1, min_y: 1, max_x: 4, max_y: 2 };
        run_disease_growth_task(&mut sd, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 489, "辐射杀伤 5");
            assert!((u.disease_growth_accumulated_error.get(7) - (-0.8836478)).abs() < 1e-5);
        }
        uninstall_disease_table();
    }

    #[test]
    fn post_process_second_pass_clears_and_increments_tick() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_growth_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        // count=0、tick=0、acc=0.5：第一遍 f25=0.3 → count 仍 0；
        // 第二遍 count<1 → 清四字段 → tick=0+1=1（原版怪癖）
        set_updated(&mut sd, 7, 30.0, 1.0, 0, 0, 0, 0.5, 0.0);
        // 无病菌格（0xff, count=0, tick=7）→ 第二遍同样 count<1 → 清除 → tick=1
        set_updated(&mut sd, 8, 30.0, 1.0, 0xff, 0, 7, 0.0, 0.0);
    let bounds = crate::d1_activity::RegionBounds { min_x: 1, min_y: 1, max_x: 4, max_y: 2 };
        run_disease_growth_task(&mut sd, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_idx.get(7), 0xff);
            assert_eq!(u.disease_count.get(7), 0);
            assert_eq!(u.disease_infestation_tick_count.get(7), 1, "清除后 tick=1 怪癖");
            assert_eq!(u.disease_growth_accumulated_error.get(7), 0.0);
            assert_eq!(u.disease_infestation_tick_count.get(8), 1, "无病菌格同样重置 tick=1");
            assert_eq!(u.disease_idx.get(8), 0xff);
        }
        uninstall_disease_table();
    }

    // ===== 任务 4：update_data 挂接集成 =====

    /// 集成用病菌表：固体(elem2) 扩散 scale=1.0；生长恒 0（popHL/overPopHL=INF、
    /// underPopDeath=0、min=0、max=INF、tempHL 全 INF → 温度因子 1.0）。
    fn install_integration_table() {
        let mut t = Disease::new();
        let mut eg = ElemGrowthInfo::default();
        eg.diffusion_scale.resize(1, 0.0);
        eg.diffusion_scale.set(0, 1.0);
        eg.min_diffusion_count.resize(1, 0);
        eg.min_diffusion_infestation_tick_count.resize(1, 0);
        eg.min_count_per_kg.resize(1, 0.0);
        eg.max_count_per_kg.resize(1, f32::INFINITY);
        eg.population_half_life.resize(1, f32::INFINITY);
        eg.over_population_half_life.resize(1, f32::INFINITY);
        eg.under_population_death_rate.resize(1, 0.0);
        t.diseases.push_unchecked(DiseaseInfo {
            hash_id: 0xE1,
            strength: 1.0,
            temperature_range: RangeInfo {
                min_viable: -f32::INFINITY,
                min_growth: -f32::INFINITY,
                max_growth: f32::INFINITY,
                max_viable: f32::INFINITY,
            },
            temperature_half_lives: RangeInfo {
                min_viable: f32::INFINITY,
                min_growth: f32::INFINITY,
                max_growth: f32::INFINITY,
                max_viable: f32::INFINITY,
            },
            elem_growth_info: eg,
            radiation_kill_rate: 0.0,
            ..Default::default()
        });
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
        }
        g.0 = Box::into_raw(Box::new(t));
    }

    /// 集成测试元素表：1 个固体元素（state=3），温度范围覆盖 300K
    /// （low=-273/high=1000 → 温度任务 DoStateTransition 不触发；
    /// 避免测试元素表默认 low/high=0 把固体相变成元素 0）。
    fn create_solid_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(1);
        let mut elem = Element::default();
        elem.id = 0;
        elem.state = 3;
        elem.low_temp = -273.0;
        elem.high_temp = 1000.0;
        elem.number_of_gradient_colors = 1;
        let elem_bytes = unsafe {
            std::slice::from_raw_parts(&elem as *const Element as *const u8, 164).to_vec()
        };
        w.write_bytes(&elem_bytes);
        w.write_int(0); // 空名称
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    #[test]
    fn update_data_hooks_disease_diffusion_after_liquid() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_solid_table();
        install_integration_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        // 既有 update_data 测试约定：真空=元素0、void=0xFFFF（测试元素表无真实真空 hash）
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        // 全固体世界（elem0 state3，不流动）；两相邻固体格 disease0 count100/50
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let c = &mut *buf_ptr;
                for i in 0..30usize {
                    c.element_idx.set(i, 0);
                    c.temperature.set(i, 300.0);
                    c.mass.set(i, 1000.0);
                    c.disease_idx.set(i, 0xff);
                    c.disease_count.set(i, 0);
                    c.disease_infestation_tick_count.set(i, 0);
                }
            }
        }
        set_both(&mut sd, 14, 0, 0, 100, 0);
        set_both(&mut sd, 15, 0, 0, 50, 0);
        // 下邻 20 同病菌等量（100）→ 14↔20 净转移 0；区域仅 cell14
        // （原版扩散按区域每格对右/下邻调用，区域外格只作邻居，不被迭代）。
        set_both(&mut sd, 20, 0, 0, 100, 0);
        sd.active_regions.push(crate::a_framework::sim_data::ActiveRegion {
            min_x: 2,
            min_y: 2,
            max_x: 3,
            max_y: 3,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        });
        crate::c2_physics::update_data(&mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(14), 94, "扩散 6 → 94");
            assert_eq!(u.disease_count.get(15), 56);
            assert_eq!(u.disease_count.get(20), 100, "下邻等量 → 无净转移");
            assert_eq!(u.disease_idx.get(14), 0);
            assert_eq!(u.disease_idx.get(15), 0);
            assert_eq!(u.disease_infestation_tick_count.get(14), 1, "生长第二遍 tick+1");
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn half_life_to_growth_rate_zero_is_zero() {
        assert_eq!(half_life_to_growth_rate(0.0, 0.2), 0.0);
    }

    #[test]
    fn half_life_to_growth_rate_infinity_is_one() {
        assert_eq!(half_life_to_growth_rate(f32::INFINITY, 0.2), 1.0);
    }

    #[test]
    fn half_life_to_growth_rate_matches_pow2() {
        // 2^(-0.2/1.0) = 0.870550563296124
        assert!((half_life_to_growth_rate(1.0, 0.2) - 0.870550563296124).abs() < 1e-6);
        // 2^(-0.2/10.0) = 2^(-0.02) = 0.9862327044935591
        assert!((half_life_to_growth_rate(10.0, 0.2) - 0.9862327044935591).abs() < 1e-6);
    }

    fn test_range() -> RangeInfo {
        RangeInfo {
            min_viable: 0.0,
            min_growth: 20.0,
            max_growth: 40.0,
            max_viable: 60.0,
        }
    }

    fn test_hl() -> RangeInfo {
        RangeInfo {
            min_viable: 100.0,
            min_growth: 200.0,
            max_growth: 300.0,
            max_viable: 400.0,
        }
    }

    #[test]
    fn calculate_range_half_life_optimal_band_is_infinity() {
        let _lock = LIB_TESTS_LOCK.lock();
        assert_eq!(calculate_range_half_life(30.0, &test_range(), &test_hl()), f32::INFINITY);
    }

    #[test]
    fn calculate_range_half_life_interpolates_band() {
        let _lock = LIB_TESTS_LOCK.lock();
        // temp=10 ∈ [0,20)：t=0.5 → lerp(100,200)=150
        assert_eq!(calculate_range_half_life(10.0, &test_range(), &test_hl()), 150.0);
    }

    #[test]
    fn calculate_range_half_life_clamps_below_and_above() {
        let _lock = LIB_TESTS_LOCK.lock();
        assert_eq!(calculate_range_half_life(-5.0, &test_range(), &test_hl()), 100.0);
        assert_eq!(calculate_range_half_life(80.0, &test_range(), &test_hl()), 400.0);
    }

    #[test]
    fn calculate_range_half_life_inf_propagates() {
        let _lock = LIB_TESTS_LOCK.lock();
        let hl = RangeInfo {
            min_viable: f32::INFINITY,
            ..test_hl()
        };
        assert_eq!(calculate_range_half_life(10.0, &test_range(), &hl), f32::INFINITY);
    }
}
