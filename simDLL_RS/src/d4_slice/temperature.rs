//! 切片化的温度阶段。
//!
//! 本模块目前只有**串行执行版** —— 目的是先把「切片化之后与原版逐位一致」这件事
//! 证明清楚，再在任务 5 接并行。若两者同时引入，测试一红就分不清是逻辑复刻错
//! 还是调度错。

use crate::a_framework::sim_data::SimData;
use crate::b_elements::elements_table::get_element_temperature_data;
use crate::c2_physics::temperature::{
    apply_delta, compute_pair_delta, do_state_transition, update_temperature_for_backwall,
};
use crate::d1_activity::RegionBounds;
use crate::d4_slice::slice_grid::SliceGrid;

/// 格 `cell` 是否落在任一区域边界内（即是否会被某个切片覆盖）。
///
/// 由任务 3 的覆盖性测试可知，切片恰好无重叠、无缝地平铺每个区域，
/// 因此「被某切片覆盖」⟺「落在某区域边界内」，无需查切片表。
///
/// 用途：判断配对的邻居那半该由谁写。
/// - 上/左邻居被覆盖 → 那一对在原版里存在，本格那半要写
/// - 上/左邻居未被覆盖 → 原版根本不算这一对，本格那半**也不能写**
/// - 右/下邻居未被覆盖 → 原版照样写了它（边缘 halo），本格必须**代写**
///
/// 若后续 profile 显示这个 O(区域数) 判定是热点，再换成缓存位图；先按 YAGNI 走简单版。
fn is_owned(cell: usize, w: usize, regions: &[RegionBounds]) -> bool {
    let x = cell % w;
    let y = cell / w;
    regions
        .iter()
        .any(|r| x >= r.min_x && x < r.max_x && y >= r.min_y && y < r.max_y)
}

/// 单个格子的完整处理序列（顺序严格复刻串行）。
///
/// `neighbor_owned` 返回某邻居是否归某个切片所有（不归属 → 视方向决定是否代写，
/// 见下方不对称规则的推导）。
fn process_cell(
    sim: &mut SimData,
    cell: usize,
    w: usize,
    h: usize,
    neighbor_owned: impl Fn(usize) -> bool,
) {
    let r = cell / w;
    let c = cell % w;

    // ---- 计算（顺序无关，只读快照；保留完整配对结果，halo 代写需要另一侧）----
    //
    // 四个门控分别严格复刻「那一对在原版里由谁处理、处理时带什么门控」：
    //   pair_up   ← 原版在格 (cell-w) 处算下对，需 (cell-w) 通过循环守卫
    //   pair_left ← 原版在格 (cell-1) 处算右对，需 (cell-1) 通过循环守卫
    //   pair_right← 原版在本格算右对
    //   pair_down ← 原版在本格算下对
    let pair_up = if r >= 1 && r < h - 1 && c < w - 1 {
        compute_pair_delta(sim, cell - w, cell, true)
    } else {
        None
    };
    let pair_left = if c >= 1 && c < w - 1 && r < h - 1 {
        compute_pair_delta(sim, cell - 1, cell, true)
    } else {
        None
    };
    let pair_right = if c + 1 < w - 1 && r < h - 1 {
        compute_pair_delta(sim, cell, cell + 1, true)
    } else {
        None
    };
    let pair_down = if r + 1 < h - 1 && r < h - 1 && c < w - 1 {
        compute_pair_delta(sim, cell, cell + w, true)
    } else {
        None
    };

    // ---- 应用（顺序严格：上 → 左 → 背墙 → 右 → 下 → 状态转换）----
    //
    // ⚠️ **上/左 与 右/下 的写入规则不对称**，根源是本格在配对中所处的位置不同。
    //
    // 串行版（`run_temperature_band`）遍历 `row ∈ [row_start,row_end)`、
    // `col ∈ [min_x,max_x)`，跳过 `row ≥ h-1 || col ≥ w-1`；每格只处理「右」「下」
    // 两对，本格是配对的**第一**元素，且 `update_temperature_pair` 一次写满两半。
    // 于是「一对 (A,B) 是否存在」**只由 A 是否被遍历到决定，与 B 无关**。
    // 对本格 X 的四个配对：
    //   - (X-w, X) 与 (X-1, X)：X 是**第二**元素。这一对只有在「那个邻居被遍历到、
    //     它自己那轮会算」时才存在。邻居在区域外 → 串行版根本不算这一对，
    //     **X 那半也不能写**，否则就是多出来的贡献（区域顶行/左列必挂）。
    //     邻居那半永不代写：在区域内时它自己那轮会写，不在时这一对不存在。
    //   - (X, X+1) 与 (X, X+w)：X 是**第一**元素，只要 X 被遍历到就无条件算
    //     （门控只看 |ΔT|/TC/边界，不看邻居归属）。X 那半永远写；邻居那半串行版
    //     也会写（哪怕邻居在区域外 —— 那就是右/下边缘的 halo），所以邻居在区域外时
    //     必须由本格代写。邻居在区域内时由它自己的上/左对代劳，此处跳过以免重复计。
    //
    // 这条不对称规则同时保证并行安全：区域内格的写入只落在自己身上，
    // 跨格写入仅发生在无人拥有的 halo 上 —— 不存在两个切片写同一格。
    if let Some((_d_nb, d_self)) = pair_up {
        if neighbor_owned(cell - w) {
            apply_delta(sim, cell, d_self);
        }
    }
    if let Some((_d_nb, d_self)) = pair_left {
        if neighbor_owned(cell - 1) {
            apply_delta(sim, cell, d_self);
        }
    }

    // 背墙：复刻串行门控（cells.mass>0 且 backwall.mass>0）
    {
        let do_bw = unsafe {
            let cc = &*sim.cells.ptr;
            let bw = &*sim.backwall.ptr;
            cell < cc.mass.len()
                && cell < bw.mass.len()
                && cc.mass.get(cell) > 0.0
                && bw.mass.get(cell) > 0.0
        };
        if do_bw {
            update_temperature_for_backwall(sim, cell);
        }
    }

    if let Some((d_self, d_nb)) = pair_right {
        apply_delta(sim, cell, d_self);
        if !neighbor_owned(cell + 1) {
            apply_delta(sim, cell + 1, d_nb); // 右边缘 halo，无人会写 → 代写
        }
    }
    if let Some((d_self, d_nb)) = pair_down {
        apply_delta(sim, cell, d_self);
        if !neighbor_owned(cell + w) {
            apply_delta(sim, cell + w, d_nb); // 下边缘 halo，无人会写 → 代写
        }
    }

    // 状态转换
    let elem = unsafe { (*sim.cells.ptr).element_idx.get(cell) };
    if let Some(etd) = get_element_temperature_data(elem) {
        do_state_transition(sim, cell, &etd);
    }
}

/// 切片化的温度阶段（**串行执行版**，用于验证逻辑复刻正确性）。
///
/// 与 `run_temperature_task(sim, region)` 逐位等价；任务 5 接并行时只改外层调度，
/// `process_cell` 不用动。
pub fn run_temperature_sliced_serial(
    sim: &mut SimData,
    regions: &[RegionBounds],
    slice_size: usize,
) {
    // 与 `run_temperature_band` 的守卫保持一致（含 backwall / sim_events，
    // `process_cell` 会解引用它们）。
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
    let grid = SliceGrid::build(regions, slice_size);
    // 注意：owned 判定的是「邻居是否被**任何**切片覆盖」，不是「是否属于本切片」。
    let owned = |cell: usize| is_owned(cell, w, regions);
    for s in &grid.slices {
        if s.is_empty() {
            continue;
        }
        for y in s.min_y..s.max_y {
            for x in s.min_x..s.max_x {
                if y >= h - 1 || x >= w - 1 {
                    continue; // 复刻串行循环守卫
                }
                process_cell(sim, y * w + x, w, h, &owned);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;
    use crate::b_elements::element::{Element, ElementTemperatureData};
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::d4_slice::DEFAULT_SLICE_SIZE;
    use crate::LIB_TESTS_LOCK;

    /// 元素表：dummy(0) / 水(1) / 冰(2) / 蒸汽(3)。
    /// 与 `c2_physics/temperature.rs` 测试中的 `init_phase_table` 一致。
    fn init_phase_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.temperature_data.clear();

        // ⚠️ 必须显式设比热容 / 导热率 / 三个表面倍率。`ElementTemperatureData` 是
        // `#[derive(Default)]`，未设的字段一律 0.0，于是：
        //   - TC = 0            → `pair_allowed` 直接否决，换热恒为 0
        //   - 表面倍率 = 0       → `k_total = k * mult_a * mult_b` 恒为 0
        // 任一为 0，串行与切片两侧就都是空操作，逐位比对照样全绿，却什么都没验证。
        // 本简报原版 `init_phase_table` 正漏了这三项 —— 已由变异测试实测为假绿，故补齐。
        let mut water = ElementTemperatureData::default();
        water.state = 2;
        water.specific_heat_capacity = 4.179;
        water.thermal_conductivity = 0.609;
        water.gas_surface_area_multiplier = 1.0;
        water.liquid_surface_area_multiplier = 1.0;
        water.solid_surface_area_multiplier = 1.0;
        water.default_mass = 1000.0;
        water.low_temp = 0.0;
        water.high_temp = 100.0;
        water.low_temp_transition_idx = 2;
        water.high_temp_transition_idx = 3;
        let mut ice = ElementTemperatureData::default();
        ice.state = 3;
        ice.specific_heat_capacity = 2.05;
        ice.thermal_conductivity = 2.18;
        ice.gas_surface_area_multiplier = 1.0;
        ice.liquid_surface_area_multiplier = 1.0;
        ice.solid_surface_area_multiplier = 1.0;
        ice.default_mass = 1000.0;
        let mut steam = ElementTemperatureData::default();
        steam.state = 1;
        steam.specific_heat_capacity = 1.0;
        steam.thermal_conductivity = 0.1;
        steam.gas_surface_area_multiplier = 1.0;
        steam.liquid_surface_area_multiplier = 1.0;
        steam.solid_surface_area_multiplier = 1.0;
        steam.default_mass = 1.0;
        steam.low_temp = 100.0;
        steam.high_temp = 999.0;
        steam.low_temp_transition_idx = 1;

        table.temperature_data.push(ElementTemperatureData::default());
        table.temperature_data.push(water);
        table.temperature_data.push(ice);
        table.temperature_data.push(steam);

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
        table.elements.push(Element::default());
        table.elements.push(e_water);
        table.elements.push(e_ice);
        table.elements.push(e_steam);
    }

    /// 构造一份可复现的 SimData：内部格铺满水并带温度梯度（左冷右热、上冷下热），
    /// 保证 `|ΔT| >= 1` 门控大量命中，四个方向的配对都会被走到。
    fn make_sim(w: i32, h: i32) -> SimData {
        init_phase_table();
        let mut sd = SimData::new_for_allocate(w, h, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;

        let ww = w as usize;
        let hh = h as usize;
        for y in 1..hh.saturating_sub(1) {
            for x in 1..ww.saturating_sub(1) {
                let cell = y * ww + x;
                let u = unsafe { &mut *sd.updated_cells.ptr };
                u.element_idx.set(cell, 1); // 水
                u.mass.set(cell, 50.0);
                // 温度场刻意取**不整齐**的数值（素数码伪随机），而非简报的平滑梯度。
                // 平滑梯度下相邻格温差整齐，浮点加法的最低位舍入碰巧可交换，
                // 导致「贡献施加顺序」无法被任何断言区分（实测：把上/左两块对调，
                // 全部用例照绿）。换不规则场才能让 `to_bits()` 真正咬住顺序。
                let t = 20.0 + ((x * 7919 + y * 104729) % 997) as f32 * 0.137;
                u.temperature.set(cell, t);
                // insulation 也很关键：默认 0 → iv = 0 → `combine_conductivity`
                // 走 `min(k_a, k_b)` 分支得到 0 → 换热恒 0，测试同样假绿。
                // 取 255（与 `c2_physics` 既有 pair 测试同值）：f32 下 iv 舍入为 ≥1.0，
                // 走几何平均分支，导率非零。
                u.insulation.set(cell, 255);
            }
        }
        // 温度阶段读 `cells`（不可变快照），必须先同步一次
        crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd);
        sd
    }

    /// 逐位比对两个 SimData 的 `updated_cells`（温度阶段只写这里）。
    /// `f32` 一律走 `to_bits()` —— 浮点加法不可结合，任何一次写入顺序或
    /// 归属判定的偏差都会在某个格子上暴露成位模式差异。
    fn bit_eq_cells(a: &SimData, b: &SimData) {
        let ua = unsafe { &*a.updated_cells.ptr };
        let ub = unsafe { &*b.updated_cells.ptr };
        let n = ua.temperature.len();
        assert_eq!(n, ub.temperature.len(), "两份 SimData 尺寸不同");
        for i in 0..n {
            assert_eq!(
                ua.temperature.get(i).to_bits(),
                ub.temperature.get(i).to_bits(),
                "updated_cells.temperature 第 {i} 格不一致"
            );
            assert_eq!(
                ua.mass.get(i).to_bits(),
                ub.mass.get(i).to_bits(),
                "mass 第 {i} 格"
            );
            assert_eq!(ua.element_idx.get(i), ub.element_idx.get(i), "element_idx 第 {i} 格");
            assert_eq!(
                ua.disease_count.get(i),
                ub.disease_count.get(i),
                "disease_count 第 {i} 格"
            );
        }
    }

    /// 反假绿闸门：确认这一趟温度阶段**真的改动了格子**。
    ///
    /// 若元素表漏设 TC / 表面倍率，或 insulation 为 0，换热恒为 0 —— 串行与切片
    /// 两侧都是空操作，逐位比对照样全绿，却一个字节的有效逻辑都没验证到。
    /// 这正是本任务初次实现踩到的坑（见报告「自审」一节），故固化成断言。
    fn assert_work_happened(before: &SimData, after: &SimData, min_changed: usize) {
        let b = unsafe { &*before.updated_cells.ptr };
        let a = unsafe { &*after.updated_cells.ptr };
        let mut changed = 0usize;
        for i in 0..a.temperature.len() {
            if a.temperature.get(i).to_bits() != b.temperature.get(i).to_bits()
                || a.element_idx.get(i) != b.element_idx.get(i)
            {
                changed += 1;
            }
        }
        assert!(
            changed >= min_changed,
            "温度阶段几乎没改动任何格子（仅 {changed} 格，期望 ≥{min_changed}）——\
             逐位一致很可能是因为两侧都是空操作"
        );
    }

    #[test]
    fn sliced_serial_matches_original_bitwise() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut serial = make_sim(40, 40);
        let mut sliced = make_sim(40, 40);
        let pristine = make_sim(40, 40);
        let bounds = RegionBounds { min_x: 1, min_y: 1, max_x: 39, max_y: 39 };

        crate::c2_physics::temperature::run_temperature_task(&mut serial, bounds);
        run_temperature_sliced_serial(&mut sliced, &[bounds], DEFAULT_SLICE_SIZE);

        assert_work_happened(&pristine, &serial, 500);
        bit_eq_cells(&serial, &sliced);
    }

    /// 参数化：切片尺寸 16 / 32 / 64 各跑一遍逐位一致，
    /// 同时覆盖「能被整除」与「不能被整除」两种切片收窄。
    #[test]
    fn sliced_serial_matches_for_all_slice_sizes() {
        let _lock = LIB_TESTS_LOCK.lock();
        for size in [16usize, 32, 64] {
            let mut serial = make_sim(70, 70);
            let mut sliced = make_sim(70, 70);
            let pristine = make_sim(70, 70);
            let bounds = RegionBounds { min_x: 1, min_y: 1, max_x: 69, max_y: 69 };
            crate::c2_physics::temperature::run_temperature_task(&mut serial, bounds);
            run_temperature_sliced_serial(&mut sliced, &[bounds], size);
            assert_work_happened(&pristine, &serial, 500);
            bit_eq_cells(&serial, &sliced);
        }
    }

    /// 反假绿：上面两个用例的区域都紧贴水域外沿（min=1 恰是 `make_sim` 铺水的起点），
    /// 于是区域外一圈全是 dummy 格（TC=0）—— `pair_up`/`pair_left` 直接被
    /// `pair_allowed` 挡掉，**上/左的归属判定根本没被走到**。
    /// 本用例把区域收进水域内部（5..35），区域外一圈是可换热的真水，
    /// 从而同时压到不对称规则的两个方向：
    ///   - 上/左邻居在水里但**不属于**区域 → 串行不算这一对 → 本格那半必须跳过
    ///   - 右/下邻居在水里但**不属于**区域 → 串行照样写了它 → 必须代写 halo
    /// 缺任何一半，这里必红。
    #[test]
    fn sliced_serial_matches_for_interior_region() {
        let _lock = LIB_TESTS_LOCK.lock();
        for size in [16usize, 32] {
            let mut serial = make_sim(40, 40);
            let mut sliced = make_sim(40, 40);
            let pristine = make_sim(40, 40);
            let bounds = RegionBounds { min_x: 5, min_y: 5, max_x: 35, max_y: 35 };
            crate::c2_physics::temperature::run_temperature_task(&mut serial, bounds);
            run_temperature_sliced_serial(&mut sliced, &[bounds], size);
            assert_work_happened(&pristine, &serial, 500);
            bit_eq_cells(&serial, &sliced);
        }
    }

    /// 空区域与退化尺寸不得 panic，且不得改动任何格子。
    #[test]
    fn sliced_serial_handles_empty_and_tiny_inputs() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 空区域：无切片
        let mut sd = make_sim(40, 40);
        let mut expect = make_sim(40, 40);
        let empty = RegionBounds { min_x: 1, min_y: 1, max_x: 0, max_y: 0 };
        run_temperature_sliced_serial(&mut sd, &[empty], DEFAULT_SLICE_SIZE);
        bit_eq_cells(&expect, &sd);

        // 退化尺寸（w<3）：直接返回
        let mut tiny = make_sim(2, 2);
        let tiny_expect = make_sim(2, 2);
        let b = RegionBounds { min_x: 0, min_y: 0, max_x: 2, max_y: 2 };
        run_temperature_sliced_serial(&mut tiny, &[b], DEFAULT_SLICE_SIZE);
        bit_eq_cells(&tiny_expect, &tiny);
    }

    /// 多区域（互不相邻）：`is_owned` 跨区域判定的常规情形。
    /// 简报只给了单区域用例，而 `run_temperature_sliced_serial` 的签名收的是
    /// `&[RegionBounds]` —— 补上多区域覆盖。
    #[test]
    fn sliced_serial_matches_for_two_separated_regions() {
        let _lock = LIB_TESTS_LOCK.lock();
        let a = RegionBounds { min_x: 5, min_y: 5, max_x: 15, max_y: 35 };
        let b = RegionBounds { min_x: 18, min_y: 5, max_x: 28, max_y: 35 };
        let mut serial = make_sim(40, 40);
        let mut sliced = make_sim(40, 40);
        let pristine = make_sim(40, 40);
        crate::c2_physics::temperature::run_temperature_task(&mut serial, a);
        crate::c2_physics::temperature::run_temperature_task(&mut serial, b);
        run_temperature_sliced_serial(&mut sliced, &[a, b], DEFAULT_SLICE_SIZE);
        assert_work_happened(&pristine, &serial, 200);
        bit_eq_cells(&serial, &sliced);
    }

    /// 多区域且**紧贴相邻**（A 的右边界 x=15 就是 B 的左边界）—— 已知最微妙的情形。
    ///
    /// 此时跨区域配对 (14,15) 存在，且两侧施加顺序**确实不同**：
    ///   - 串行：区域 A 先跑，格 14 的右对把两半都写了（含格 15）；
    ///     随后区域 B 跑到格 15 时再叠加它自己的上/左贡献。
    ///   - 切片：格 15 在 B 的切片里，先加上贡献、再加来自格 14 的左贡献。
    /// 即「右 halo 那半」与「左那半」的先后与串行相反。
    ///
    /// 实测两者位模式仍一致 —— 因为贡献量级远小于格温度，两次 f32 加法
    /// 的中间舍入恰好不落到舍入边界上（把上/左两块对调，全部用例照绿，
    /// 可见此处顺序差异在 f32 下不可观测）。
    ///
    /// ⚠️ 因此本用例是**回归护栏**而非顺序正确性的证明：它保证多区域路径
    /// 不写错格、不漏写，但不保证与串行逐位相同的原因不是「碰巧」。
    /// 任务 5 若改为并行/乱序调度，此处的顺序假设需重新论证。
    #[test]
    fn sliced_serial_matches_for_two_adjacent_regions() {
        let _lock = LIB_TESTS_LOCK.lock();
        let a = RegionBounds { min_x: 5, min_y: 5, max_x: 15, max_y: 35 };
        let b = RegionBounds { min_x: 15, min_y: 5, max_x: 25, max_y: 35 };
        let mut serial = make_sim(40, 40);
        let mut sliced = make_sim(40, 40);
        let pristine = make_sim(40, 40);
        crate::c2_physics::temperature::run_temperature_task(&mut serial, a);
        crate::c2_physics::temperature::run_temperature_task(&mut serial, b);
        run_temperature_sliced_serial(&mut sliced, &[a, b], DEFAULT_SLICE_SIZE);
        assert_work_happened(&pristine, &serial, 200);
        bit_eq_cells(&serial, &sliced);
    }
}
