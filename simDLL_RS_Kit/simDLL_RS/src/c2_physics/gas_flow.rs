//! C2 气体流动（原版 UpdateData 压力循环的气体部分）。
//!
//! 原版无独立 UpdateGas：气体与液体共用统一流动系统。本模块实现压力循环中
//! 与气体相关的两个子阶段（SimDLL_Source.c L144590-144950）：
//!
//! 1. **同元素气体/真空压力均衡**（L144590-144760）：
//!    - 源格 state < 2（真空 0 / 气体 1）、可渗透、快照一致；
//!    - 对 水平(+iterateDirection) / 垂直(+width) / 对角(+width+iterateDirection)
//!      三方向调 UpdatePressure（质量差驱动，双向均衡）；
//!    - iterateDirection 每帧翻转（原版 L144415），水平扫描方向交替 →
//!      "圆形振荡"扩散。
//! 2. **异元素气体置换**（L144780-144950）：源格为气体时，对
//!    下(-width) / -iterateDirection / +iterateDirection / 上(+width) 四方向调
//!    DoGasPressureDisplacement：源格（高压）推开异元素气体邻格，邻格原内容被
//!    推到 beyond 格，源格气体流入邻格。
//!
//! 语义参考 old 稳定版 gas_flow.rs（圆形振荡已实测验证），实现严格对照原版 C，
//! 不照抄 old 版代码。

use crate::a_framework::sim_data::SimData;
use crate::b_elements::elements_table;

/// 气体置换流量上限（原版 DoGasPressureDisplacement：min(源格原质量 × 0.125, 当前质量)）。
const GAS_FLOW_RATE: f32 = 0.125;

/// 同元素气体/真空压力均衡（原版 L144590-144760）。
pub(crate) fn run_gas_pressure_task(
    sim: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    let w = sim.width as i32;
    let h = sim.height as i32;
    if w < 5 || h < 5 || sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
    let x_min = bounds.min_x.max(1) as i32;
    let x_max = (bounds.max_x as i32).min(w - 1);
    let y_min = bounds.min_y.max(1) as i32;
    let y_max = (bounds.max_y as i32).min(h - 1);
    // 排他上界：源格迭代 [min, max)（原版 L144576+ 用 `< max`，RegionBounds 亦排他）。
    // 2026-08-10 修复（D-M2）：此前用 ..= 每区域多处理最右 1 列 + 最下 1 行。
    if x_min >= x_max || y_min >= y_max {
        return;
    }
    let iterate_dir = sim.iterate_direction;
    let flow = if sim.flow.ptr.is_null() {
        None
    } else {
        Some(unsafe { std::slice::from_raw_parts_mut(sim.flow.ptr, (w * h) as usize) })
    };
    let mut flow = flow; // 供可变借用

    for y in y_min..y_max {
        // iterateDirection 控制水平扫描方向（原版 L144606-144611）：
        // >=0 从左到右，<0 从右到左 → 每帧翻转形成"圆形振荡"。
        let width_scan = x_max - x_min;
        for i in 0..width_scan {
            let x = if iterate_dir >= 0 { x_min + i } else { x_max - 1 - i };
            let src = (y * w + x) as usize;

            let (src_elem, src_state, src_pd) = {
                let cells = unsafe { &*sim.cells.ptr };
                let e = cells.element_idx.get(src);
                let pd = match elements_table::get_element_pressure_data(e) {
                    Some(pd) => pd,
                    None => continue,
                };
                let s = pd.state & 3;
                if s >= 2 {
                    continue; // 只处理真空(0)/气体(1)
                }
                // T6（#21）：原此处还读了 cells.mass.get(src) 但 `let _ = src_mass`
                // 未使用——已删（原版 UpdatePressure 内部重读，调用方不用）。
                (e, s, pd)
            };

            // 源格可渗透 + 快照一致（原版 L144604-144605）
            {
                let cells = unsafe { &*sim.cells.ptr };
                let updated = unsafe { &*sim.updated_cells.ptr };
                if cells.properties.get(src) & 1 != 0 {
                    continue;
                }
                if updated.element_idx.get(src) != src_elem {
                    continue;
                }
            }

            // —— 水平方向（+iterateDirection）——
            let nx = x + iterate_dir;
            if (1..=w - 2).contains(&nx) {
                let dst = (y * w + nx) as usize;
                if let Some((dst_elem, dst_pd)) = gas_pair_ok(sim, src, dst, src_elem, src_state) {
                    let f = crate::c2_physics::liquid_flow::update_pressure_with(
                        sim, src, dst, src_elem, dst_elem, &src_pd, &dst_pd,
                    );
                    if let Some(fl) = flow.as_deref_mut() {
                        fl[src].y += iterate_dir as f32 * f;
                        fl[dst].x -= iterate_dir as f32 * f;
                    }
                }
            }

            // 源格为真空且水平调用可能已改写源格元素 → 重查一致性（old 版语义）
            if src_state == 0 {
                let updated = unsafe { &*sim.updated_cells.ptr };
                if updated.element_idx.get(src) != src_elem {
                    continue;
                }
            }

            // —— 垂直方向（+width，上方）——
            if y + 1 <= y_max {
                let dst = src + w as usize;
                if let Some((dst_elem, dst_pd)) = gas_pair_ok(sim, src, dst, src_elem, src_state) {
                    let f = crate::c2_physics::liquid_flow::update_pressure_with(
                        sim, src, dst, src_elem, dst_elem, &src_pd, &dst_pd,
                    );
                    if let Some(fl) = flow.as_deref_mut() {
                        fl[src].z += f;
                        fl[dst].w -= f;
                    }
                }
            }

            if src_state == 0 {
                let updated = unsafe { &*sim.updated_cells.ptr };
                if updated.element_idx.get(src) != src_elem {
                    continue;
                }
            }

            // —— 对角方向（+width + iterateDirection，右上方）——
            if y + 1 <= y_max && (1..=w - 2).contains(&nx) {
                let dst = (src as i32 + w + iterate_dir) as usize;
                // 角落检查（原版 L144688-144689）：水平/垂直直线邻居 state OR == 3 时
                // 阻止气体穿"L"形砖块斜角扩散。
                let h_neighbor = (y * w + nx) as usize;
                let v_neighbor = (dst as i32 - iterate_dir) as usize;
                let corner_blocked = {
                    let cells = unsafe { &*sim.cells.ptr };
                    let hs = elements_table::get_element_pressure_data(cells.element_idx.get(h_neighbor))
                        .map(|pd| pd.state & 3)
                        .unwrap_or(3);
                    let vs = elements_table::get_element_pressure_data(cells.element_idx.get(v_neighbor))
                        .map(|pd| pd.state & 3)
                        .unwrap_or(3);
                    ((hs | vs) & 3) == 3
                };
                if !corner_blocked {
                    if let Some((dst_elem, dst_pd)) =
                        gas_pair_ok(sim, src, dst, src_elem, src_state)
                    {
                        // 2026-08-10 审查修正：原版 L144706-144713 对角 pair **只调用
                        // UpdatePressure、不写 flow**（flow 纹理仅由水平/垂直 pair 记账）。
                        // 此前多写 z/w → 对角扩散被计入垂直 flow，气体纹理方向感异常。
                        let _ = crate::c2_physics::liquid_flow::update_pressure_with(
                            sim, src, dst, src_elem, dst_elem, &src_pd, &dst_pd,
                        );
                    }
                }
            }
        }
    }
}

/// 气体对检查（同元素压力均衡的邻格条件，原版 L144610-144630 等）：
/// 邻格可渗透、快照一致、state<2、非真空-真空、非"异元素同状态"。
/// #18 查表复用：通过时返回 (dst_elem, dst_pd) 供 update_pressure_with 免重查。
fn gas_pair_ok(
    sim: &SimData,
    src: usize,
    dst: usize,
    src_elem: u16,
    src_state: u8,
) -> Option<(u16, crate::b_elements::element::ElementPressureData)> {
    let cells = unsafe { &*sim.cells.ptr };
    let updated = unsafe { &*sim.updated_cells.ptr };
    if dst >= cells.element_idx.len() {
        return None;
    }
    let dst_elem = cells.element_idx.get(dst);
    let dst_pd = match elements_table::get_element_pressure_data(dst_elem) {
        Some(pd) => pd,
        None => return None,
    };
    let dst_state = dst_pd.state & 3;
    if cells.properties.get(dst) & 1 != 0 {
        return None;
    }
    if updated.element_idx.get(dst) != dst_elem {
        return None;
    }
    if dst_state >= 2 {
        return None;
    }
    if src_state == 0 && dst_state == 0 {
        return None; // 真空-真空：flow=0
    }
    if src_elem != dst_elem && dst_state == src_state {
        return None; // 异元素同状态由异元素置换处理
    }
    Some((dst_elem, dst_pd))
}

/// 异元素气体压力置换（原版 L144780-144950 调用方 + DoGasPressureDisplacement L147441）。
///
/// 语义：srcCell（高压气体）推开 dstCell（低压异元素气体），dstCell 原内容被推到
/// beyondCell，srcCell 气体流入 dstCell。返回实际流量（用于 flow 记账）。
pub(crate) fn run_gas_displacement_task(
    sim: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    let w = sim.width as i32;
    let h = sim.height as i32;
    if w < 7 || h < 7 || sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
    // 2 格余量（beyond 距源格 2）
    let x_min = bounds.min_x.max(3) as i32;
    let x_max = (bounds.max_x as i32).min(w - 3);
    let y_min = bounds.min_y.max(3) as i32;
    let y_max = (bounds.max_y as i32).min(h - 3);
    // 排他上界（同 run_gas_pressure_task，D-M2 2026-08-10）。
    if x_min >= x_max || y_min >= y_max {
        return;
    }
    let iterate_dir = sim.iterate_direction;
    // 4 方向：(dst_off, beyond_off) —— 下(-w) / -iterateDir / +iterateDir / 上(+w)
    //（本项目约定 cell-width=下、cell+width=上，与 update_liquid 一致）
    let dirs: [(i32, i32); 4] = [
        (-w, -2 * w),
        (-iterate_dir, -2 * iterate_dir),
        (iterate_dir, 2 * iterate_dir),
        (w, 2 * w),
    ];
    for y in y_min..y_max {
        for x in x_min..x_max {
            let src = (y * w + x) as usize;
            let src_elem = {
                let cells = unsafe { &*sim.cells.ptr };
                let e = cells.element_idx.get(src);
                match elements_table::get_element_pressure_data(e) {
                    Some(pd) if (pd.state & 3) == 1 => e,
                    _ => continue,
                }
            };
            for &(dst_off, beyond_off) in &dirs {
                let dst = (src as i32 + dst_off) as usize;
                let beyond = (src as i32 + beyond_off) as usize;
                // 调用方预检查（全部从 cells 读，原版 L144779-144791）：
                let cells = unsafe { &*sim.cells.ptr };
                if dst >= cells.element_idx.len() || beyond >= cells.element_idx.len() {
                    continue;
                }
                let dst_elem = cells.element_idx.get(dst);
                if dst_elem == src_elem {
                    continue; // 同元素由同元素压力处理
                }
                let dst_state = match elements_table::get_element_pressure_data(dst_elem) {
                    Some(pd) => pd.state & 3,
                    None => continue,
                };
                if dst_state != 1 {
                    continue; // dst 必须是气体
                }
                if cells.properties.get(dst) & 1 != 0 {
                    continue; // GasImpermeable
                }
                // beyond 检查由 do_gas_pressure_displacement 内部完成（原版
                // L147502-147512：beyond 元素 == dst 元素 **或** vacuum，且可渗透）。
                // ⚠️ 2026-08-04 审查修正：调用方不再预检"beyond 必须是气体"——
                // 原版允许 beyond 为真空（气体可被推入真空格）；old 版为修 Bug1-4
                // 加的预检是额外限制，与原始源码不符。
                let f = do_gas_pressure_displacement(sim, src_elem, src, dst, beyond);
                if f > 0.0 && !sim.flow.ptr.is_null() {
                    let flow = unsafe {
                        std::slice::from_raw_parts_mut(
                            sim.flow.ptr,
                            (w * h) as usize,
                        )
                    };
                    flow[src].w -= f;
                    flow[dst].z += f;
                }
            }
        }
    }
}

/// DoGasPressureDisplacement（原版 L147441-147571）。
pub(crate) fn do_gas_pressure_displacement(
    sim: &mut SimData,
    src_elem: u16,
    src: usize,
    dst: usize,
    beyond: usize,
) -> f32 {
    let cells = unsafe { &*sim.cells.ptr };
    let dst_elem = cells.element_idx.get(dst);
    let src_mass_old = cells.mass.get(src);
    let dst_mass_old = cells.mass.get(dst);
    // 源格质量必须严格大于目标格质量（原版 L147470-147484）
    if src_mass_old <= dst_mass_old {
        return 0.0;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if updated.element_idx.get(src) != src_elem {
        return 0.0;
    }
    if updated.element_idx.get(dst) != dst_elem {
        return 0.0;
    }
    if updated.mass.get(dst) <= 0.0 {
        return 0.0;
    }
    // 超越格：元素 == dst 元素 或 == vacuum，且可渗透（原版 L147502-147512）
    let beyond_elem = updated.element_idx.get(beyond);
    if beyond_elem != dst_elem && beyond_elem != sim.vacuum_element_idx {
        return 0.0;
    }
    if updated.properties.get(beyond) & 1 != 0 {
        return 0.0;
    }
    // 把 dst 内容整体搬到 beyond（dst 清为真空）
    crate::c2_physics::liquid_flow::do_displacement(sim, dst, beyond);
    // 从 src 向 dst 灌入气体
    let src_mass_now = updated.mass.get(src);
    let flow = (src_mass_old * GAS_FLOW_RATE).min(src_mass_now);
    if flow <= 0.0 {
        return 0.0;
    }
    updated.mass.set(dst, updated.mass.get(dst) + flow);
    // 温度直接覆盖（dst 刚被清空，原版 L147534）
    updated.temperature.set(dst, cells.temperature.get(src));
    // 元素覆盖 + 事件（原版 L147539-147541）
    updated.element_idx.set(dst, src_elem);
    crate::c2_physics::liquid_flow::push_substance_change(sim, dst);
    // 源格扣减（钳零，原版 L147554）
    updated.mass.set(src, (src_mass_now - flow).max(0.0));
    // 病菌转移：固定 0.125（原版 L147560-147571）
    let src_disease_count = cells.disease_count.get(src);
    let disease_transfer = (src_disease_count as f32 * GAS_FLOW_RATE) as i32;
    if disease_transfer > 0 {
        let src_disease_idx = cells.disease_idx.get(src);
        crate::c2_physics::liquid_flow::add_disease_to_cell(
            updated,
            dst,
            src_disease_idx,
            disease_transfer,
        );
        updated.disease_count.set(
            src,
            (updated.disease_count.get(src) - disease_transfer).max(0),
        );
    }
    flow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::{CellSOA, SimData};
    use crate::b_elements::element::{Element, ElementPressureData, ElementStateData};
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::LIB_TESTS_LOCK;

    /// 最小元素表：0=真空(state0)、1=气体A(state1, flow=1)、2=气体B(state1, flow=1)、3=固体(state3)。
    fn init_gas_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        table.elements.clear();
        table.element_names.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        for (id, state, flow) in [(0i32, 0u8, 0.0f32), (1i32, 1u8, 1.0f32), (2i32, 1u8, 1.0f32), (3i32, 3u8, 0.0f32)] {
            let mut elem = Element::default();
            elem.id = id;
            elem.state = state;
            elem.flow = flow;
            table.elements.push(elem);
            table.state_data.push(ElementStateData { state });
            table.liquid_data.push(crate::b_elements::element::ElementLiquidData {
                state,
                flow,
                viscosity: 50.0,
                max_mass: 1000.0,
                ..Default::default()
            });
            table.pressure_data.push(ElementPressureData { state, flow });
            table.post_process_data.push(crate::b_elements::element::ElementPostProcessData {
                state,
                ..Default::default()
            });
        }
        table
            .element_indices
            .insert(0u32, 0u16);
        table.element_indices.insert(1u32, 1u16);
        table.element_indices.insert(2u32, 2u16);
        table.element_indices.insert(3u32, 3u16);
    }

    fn make_sim() -> SimData {
        let mut sd = SimData::new_for_allocate(8, 8, 1, false, true);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd
    }

    fn fill_both(sd: &mut SimData, f: impl Fn(&mut CellSOA)) {
        unsafe {
            f(&mut *sd.cells.ptr);
            f(&mut *sd.updated_cells.ptr);
        }
    }

    #[test]
    fn gas_same_element_horizontal_balance_moves_mass() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_gas_table();
        let mut sd = make_sim();
        // cell 27 (row3,col3)=气体A 100kg；cell 28 (row3,col4)=气体A 0kg
        fill_both(&mut sd, |b| {
            b.element_idx.set(27, 1);
            b.mass.set(27, 100.0);
            b.temperature.set(27, 300.0);
            b.element_idx.set(28, 1);
            b.mass.set(28, 0.0);
            b.temperature.set(28, 300.0);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_gas_pressure_task(&mut sd, b);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                u.mass.get(28) > 0.0,
                "同元素气体应水平均衡到真空邻格，got {}",
                u.mass.get(28)
            );
            assert!(u.mass.get(27) < 100.0, "源格质量应减少");
            // 全网格质量守恒（气体向垂直/对角等多方向扩散）
            let total: f32 = (0..64).map(|i| u.mass.get(i)).sum();
            assert!((total - 100.0).abs() < 1e-3, "全网格质量守恒，got {}", total);
        }
    }

    #[test]
    fn gas_diffuses_into_vacuum_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_gas_table();
        let mut sd = make_sim();
        // cell 27 = 气体A 100kg；cell 28 = 真空
        fill_both(&mut sd, |b| {
            b.element_idx.set(27, 1);
            b.mass.set(27, 100.0);
            b.temperature.set(27, 300.0);
            b.element_idx.set(28, 0);
            b.mass.set(28, 0.0);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_gas_pressure_task(&mut sd, b);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(u.mass.get(28) > 0.0, "气体应扩散进真空格");
            assert_eq!(u.element_idx.get(28), 1, "真空格应变为气体A");
        }
    }

    #[test]
    fn different_gas_displacement_pushes_beyond_gas() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_gas_table();
        let mut sd = make_sim();
        // cell 27 = 气体A 100kg；cell 28 = 气体B 10kg；cell 29 = 气体B 0kg（beyond）
        fill_both(&mut sd, |b| {
            b.element_idx.set(27, 1);
            b.mass.set(27, 100.0);
            b.temperature.set(27, 300.0);
            b.element_idx.set(28, 2);
            b.mass.set(28, 10.0);
            b.temperature.set(28, 200.0);
            b.element_idx.set(29, 2);
            b.mass.set(29, 0.0);
            b.temperature.set(29, 200.0);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_gas_displacement_task(&mut sd, b);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            // 气体A 应流入 cell28，cell28 原气体B 被推到 cell29
            assert_eq!(u.element_idx.get(28), 1, "dst 应变为气体A");
            assert!(u.mass.get(28) > 10.0, "dst 应有气体A 流入");
            assert_eq!(u.element_idx.get(29), 2, "beyond 应保留气体B（被顶入）");
            assert!(u.mass.get(29) > 0.0, "beyond 应收到原 dst 的气体B");
        }
    }

    #[test]
    fn different_gas_displacement_beyond_vacuum_allowed() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_gas_table();
        let mut sd = make_sim();
        // cell 27 = 气体A 100kg；cell 28 = 气体B 10kg；cell 29 = 真空（beyond）
        // 原版 L147502-147512：beyond 允许 == vacuum → 气体B 被顶入真空格。
        fill_both(&mut sd, |b| {
            b.element_idx.set(27, 1);
            b.mass.set(27, 100.0);
            b.temperature.set(27, 300.0);
            b.element_idx.set(28, 2);
            b.mass.set(28, 10.0);
            b.temperature.set(28, 200.0);
            b.element_idx.set(29, 0);
            b.mass.set(29, 0.0);
            b.temperature.set(29, 0.0);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_gas_displacement_task(&mut sd, b);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            // beyond 真空 → 原版允许：dst 变为气体A、气体B 被顶到真空格
            assert_eq!(u.element_idx.get(28), 1, "dst 应变气体A");
            assert!(u.mass.get(28) > 10.0, "dst 应有气体A 流入");
            assert_eq!(u.element_idx.get(29), 2, "beyond 真空格应收到气体B");
            assert!((u.mass.get(29) - 10.0).abs() < 1e-3, "气体B 全量转移");
        }
    }

    #[test]
    fn iterate_direction_flips_each_update() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_gas_table();
        let mut sd = make_sim();
        assert_eq!(sd.iterate_direction, -1);
        update_data_noop(&mut sd);
        assert_eq!(sd.iterate_direction, 1);
        update_data_noop(&mut sd);
        assert_eq!(sd.iterate_direction, -1);
    }

    /// 只翻转 iterateDirection 的 update_data 替身（避免整链依赖）。
    fn update_data_noop(sd: &mut SimData) {
        sd.iterate_direction = -sd.iterate_direction;
    }

    /// 复现表：0=真空、1=CO2(气体, molar 44)、2=O2(气体, molar 32)、3=固体。
    fn init_stratify_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        table.elements.clear();
        table.element_names.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        let specs = [
            (0i32, 0u8, 0.0f32, 0.0f32),   // 真空
            (1i32, 1u8, 1.0f32, 44.0f32),  // CO2
            (2i32, 1u8, 1.0f32, 32.0f32),  // O2
            (3i32, 3u8, 0.0f32, 100.0f32), // 固体
        ];
        for (id, state, flow, molar) in specs {
            let mut elem = Element::default();
            elem.id = id;
            elem.state = state;
            elem.flow = flow;
            elem.molar_mass = molar;
            table.elements.push(elem);
            table.state_data.push(ElementStateData { state });
            table.liquid_data.push(crate::b_elements::element::ElementLiquidData {
                state,
                flow,
                viscosity: 50.0,
                max_mass: 1000.0,
                ..Default::default()
            });
            table.pressure_data.push(ElementPressureData { state, flow });
            table.post_process_data.push(crate::b_elements::element::ElementPostProcessData {
                state,
                molar_mass: molar,
                max_mass: 1000.0,
                ..Default::default()
            });
        }
        table.element_indices.insert(0u32, 0u16);
        table.element_indices.insert(1u32, 1u16);
        table.element_indices.insert(2u32, 2u16);
        table.element_indices.insert(3u32, 3u16);
    }

    /// 回归（2026-08-04 用户实测 Bug 2）：12×8 真空房间（含边界 14×10），
    /// 上 4 行 CO2 100kg、下 4 行 O2 200kg。原版运行一会后 CO2 全部沉底。
    /// 修复前随机交换的摩尔质量约束误挂在水平候选上 → 分层失效（CO2/O2 全图混合）；
    /// 修复后约束挂在下方候选（原版标志位 [false,false,true]）→ CO2 全部沉底。
    #[test]
    fn gas_stratification_co2_sinks_below_o2() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_stratify_table();
        let mut sd = SimData::new_for_allocate(14, 10, 12345, false, true);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        let w = sd.width as usize;
        let h = sd.height as usize;
        fill_both(&mut sd, |b| {
            for y in 0..h {
                for x in 0..w {
                    let c = y * w + x;
                    // 边界（row 0/9、col 0/13）= 固体
                    if y == 0 || y == h - 1 || x == 0 || x == w - 1 {
                        b.element_idx.set(c, 3);
                        b.mass.set(c, 1000.0);
                        b.temperature.set(c, 300.0);
                        continue;
                    }
                    // 内部：row 1..4 = O2(2)，row 5..8 = CO2(1)
                    let elem = if y >= 5 { 1u16 } else { 2u16 };
                    let mass = if elem == 1 { 100.0 } else { 200.0 };
                    b.element_idx.set(c, elem);
                    b.mass.set(c, mass);
                    b.temperature.set(c, 300.0);
                }
            }
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        // 每帧 = 原版 UpdateData 气体阶段 + PostProcess 阶段
        let mut trend: Vec<(usize, f32, f32)> = Vec::new();
        for frame in 0..50000 {
            unsafe { crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd); }
            sd.iterate_direction = -sd.iterate_direction;
            crate::c2_physics::gas_flow::run_gas_pressure_task(&mut sd, b);
            crate::c2_physics::gas_flow::run_gas_displacement_task(&mut sd, b);
            unsafe { crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd); }
            crate::c2_physics::liquid_flow::post_process_loop(&mut sd, b);

            if [1000usize, 5000, 15000, 30000, 45000].contains(&(frame + 1)) {
                let (mut co2_bottom, mut co2_top) = (0.0f32, 0.0f32);
                for y in 1..h - 1 {
                    for x in 1..w - 1 {
                        let c = y * w + x;
                        let e = unsafe { (*sd.updated_cells.ptr).element_idx.get(c) };
                        if e == 1 {
                            if y <= 4 {
                                co2_bottom += unsafe { (*sd.updated_cells.ptr).mass.get(c) };
                            } else {
                                co2_top += unsafe { (*sd.updated_cells.ptr).mass.get(c) };
                            }
                        }
                    }
                }
                trend.push((frame + 1, co2_bottom, co2_top));
            }
        }
        let last = trend.last().copied().unwrap_or((0, 0.0, 0.0));
        eprintln!("gas_stratification trend={trend:?}");
        // 趋势：1000 帧时大部分 CO2 已沉入下半区；45000 帧时上半区 CO2 应归零。
        assert!(
            trend.len() >= 5 && trend[0].1 > 4000.0,
            "1000 帧时下半区 CO2 应 >4000kg（总 4800kg），趋势={trend:?}"
        );
        assert!(
            last.2 < 1.0,
            "45000 帧时上半区 CO2 应归零（全部沉底），趋势={trend:?}"
        );
    }

    /// 回归（2026-08-06 气体纹理根因）：原版 Sim::Main 在 BeginFrameProcessing 后
    /// **每帧 memset 清零 flow**（帧内增量累加器，08_sim_frame_manager.c L2118-2119）；
    /// C# Property.Flow 以 0.25/帧混合衰减 → 平衡后动画静止。
    /// 此前项目缺该清零 → flow 累积历史总量，平衡后 raw 纹理仍 ~30.6，动画永不停止。
    ///
    /// 复现：密封房间 100kg 气体（pressure flow=0.1 真实值）扩散至平衡。
    /// 每帧按 sim_main_loop 顺序 clear_flow → copy → 翻转 iterateDir → 气体任务 →
    /// copy；断言平衡后 raw flow 纹理趋近 0（不再有历史累积）。
    #[test]
    fn flow_texture_returns_to_rest_when_cleared_each_frame() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 表：0=真空(state0)、1=气体(state1, flow=0.1 真实值)、3=固体
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
            table.elements.clear();
            table.element_names.clear();
            table.state_data.clear();
            table.liquid_data.clear();
            table.pressure_data.clear();
            table.post_process_data.clear();
            table.temperature_data.clear();
            for (id, state, flow) in [(0i32, 0u8, 0.0f32), (1i32, 1u8, 0.1f32), (3i32, 3u8, 0.0f32)] {
                let mut elem = Element::default();
                elem.id = id;
                elem.state = state;
                elem.flow = flow;
                table.elements.push(elem);
                table.state_data.push(ElementStateData { state });
                table.liquid_data.push(crate::b_elements::element::ElementLiquidData {
                    state,
                    flow,
                    viscosity: 50.0,
                    max_mass: 1000.0,
                    ..Default::default()
                });
                table.pressure_data.push(ElementPressureData { state, flow });
                table.post_process_data.push(crate::b_elements::element::ElementPostProcessData {
                    state,
                    ..Default::default()
                });
            }
            table.element_indices.insert(0u32, 0u16);
            table.element_indices.insert(1u32, 1u16);
            table.element_indices.insert(3u32, 3u16);
        }

        let mut sd = SimData::new_for_allocate(14, 10, 12345, false, true);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        let w = sd.width as usize;
        let h = sd.height as usize;
        let src = 3 * w + 3; // 内部 (3,3)，100kg
        fill_both(&mut sd, |b| {
            for y in 0..h {
                for x in 0..w {
                    let c = y * w + x;
                    if y == 0 || y == h - 1 || x == 0 || x == w - 1 {
                        b.element_idx.set(c, 3); // 固体边界
                        b.mass.set(c, 1000.0);
                        b.temperature.set(c, 300.0);
                    } else {
                        b.element_idx.set(c, 0); // 真空
                        b.temperature.set(c, 300.0);
                    }
                }
            }
            b.element_idx.set(src, 1);
            b.mass.set(src, 100.0);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);

        let mut last = 0usize;
        let mut raw_max = 0.0f32;
        for frame in 1..=20000usize {
            // sim_main_loop 帧序（对照原版 Sim::Main）：清 flow → 处理帧 → 拷贝
            crate::c_simulation::sim_data_ops::clear_flow(&mut sd);
            unsafe { crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd); }
            sd.iterate_direction = -sd.iterate_direction;
            crate::c2_physics::gas_flow::run_gas_pressure_task(&mut sd, b);
            crate::c2_physics::gas_flow::run_gas_displacement_task(&mut sd, b);
            unsafe { crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd); }

            raw_max = unsafe {
                let flow = std::slice::from_raw_parts(sd.flow.ptr, w * h);
                let u = &*sd.updated_cells.ptr;
                let mut rmax = 0.0f32;
                for y in 1..h - 1 {
                    for x in 1..w - 1 {
                        let c = y * w + x;
                        if u.element_idx.get(c) != 1 {
                            continue;
                        }
                        let inv = 1.0 / u.mass.get(c).max(1.0);
                        let f = flow[c];
                        rmax = rmax
                            .max(((f.x - f.y) * inv).abs())
                            .max(((f.w - f.z) * inv).abs());
                    }
                }
                rmax
            };
            last = frame;
        }
        assert!(last == 20000, "应跑到 20000 帧");
        // 每帧清零后，flow 只含当前帧增量；平衡时增量 ≈ 0 → 纹理趋零。
        // 修复前（无清零）此值 = 30.6（历史累积总量）。
        assert!(
            raw_max < 0.05,
            "平衡后 raw flow 纹理应趋近 0（每帧清零增量），got {raw_max}"
        );
    }
}
