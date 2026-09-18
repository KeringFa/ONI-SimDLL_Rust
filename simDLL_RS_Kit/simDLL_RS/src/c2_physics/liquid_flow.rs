//! 液体流动物理。对照源码 05_liquid_flow.c。
//!
//! 入口 update_liquids（由 c2_physics::update_data 调用）：
//! 遍历 active region 内每个"液体格"（元素 state & 3 == 2 且 mass > 0），
//! 调 update_liquid 做下落/转移，最后调 post_process_cell 做挤压与后处理。
//!
//! 本任务仅实现两个基础判定函数；液体物理在后续任务（6/7/8）追加。

use crate::a_framework::sim_data::CellSOA;
use crate::a_framework::sim_data::SimData;
use crate::a_framework::vector_math::Vector4f;
use crate::b_elements::elements_table;

/// IsLiquidPermeable — 该格元素是否允许液体渗透。
/// 对照源码 05_liquid_flow.c L599-626。
/// 可渗透 ⟺ properties[cell] & 2 == 0 且 (state & 3) <= 1（真空/气体）。
/// ⚠️ 液体（state&3==2）与固体（state&3==3）都**不可渗透**（原版 L615: `1 < (state&3)` → false）。
/// 元素表缺失（测试环境）时返回 false。
pub fn is_liquid_permeable(cells: &CellSOA, cell_idx: usize) -> bool {
    // 原版 L607：properties bit1（值 2，固体标志）置位 → 不可渗透
    if (cells.properties.get(cell_idx) & 2) != 0 {
        return false;
    }
    let e = cells.element_idx.get(cell_idx);
    match elements_table::get_element_liquid_data(e) {
        Some(d) => (d.state & 3) <= 1, // 真空/气体可渗透；液体/固体不可渗透（原版 L615）
        None => false,
    }
}

/// IsSolid — 该格元素是否固体。
/// 对照源码 05_liquid_flow.c L627-655。
/// 固体 ⟺ (state & 3) == 3 或 properties[cell] & 2 置位（原版 L639-644）。
/// 元素表缺失（测试环境）时返回 false。
pub fn is_solid(cells: &CellSOA, cell_idx: usize) -> bool {
    let e = cells.element_idx.get(cell_idx);
    let state_solid = match elements_table::get_element_liquid_data(e) {
        Some(d) => (d.state & 3) == 3,
        None => false,
    };
    state_solid || (cells.properties.get(cell_idx) & 2) != 0
}

// ===== 任务 6：UpdateLiquid + UpdateNeighbourLiquidMass =====
// 对照源码 05_liquid_flow.c L656-1212（UpdateLiquid）+ L1213-1274（UpdateNeighbourLiquidMass）。

use crate::a_framework::game_data::{SpawnFXInfo, SubstanceChangeInfo, UnstableCellInfo};

/// UpdateLiquidData 入口 — 遍历内部格，对"液体格"（state&3==2 且 mass>0）调 update_liquid。
/// 对照源码 11_msvcrt_ignored.c L42408-42435（遍历循环）：
/// 从 cells（源缓冲）读元素/质量，写 updatedCells（目标缓冲）。
/// 元素表为空（测试环境）时安全跳过。
pub fn update_liquids(sim: &mut crate::a_framework::sim_data::SimData) {
    let bounds = crate::d1_activity::full_grid_bounds(sim);
    update_liquid_loop(sim, bounds);
    post_process_loop(sim, bounds);
}

/// UpdateLiquid 遍历循环（原版 11_msvcrt_ignored.c L42408-42435）。
/// **自下而上**遍历（行从 min_y → max_y，原版 fVar34 = fStack_204 递增）。
/// 2026-08-06 对照审查还原：86d7512 曾改为自上而下（误判——认为"液2 先合并
/// 5kg"需要上方的液2 先处理）。实际用户布局（气1 左侧、液1、液2 在液1 正上方）
/// 中液1 被水平阻挡不动，**自下而上同样满足"液2 先分支 1 合并 5kg，再阶段 E
/// 对角换位"**（液2 处理时其分支 1 目标仍是液1）；原版循环确为自下而上。
/// 2026-08-04 拆分：与 post_process_loop 分离，供 update_data 在两者之间插入
/// 太空真空任务（原版 worldZones 特判位于 UpdateLiquid 之后、PostProcessCell 之前，
/// L145466-145520）。
pub(crate) fn update_liquid_loop(
    sim: &mut crate::a_framework::sim_data::SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    let width = sim.width as usize;
    let height = sim.height as usize;
    if width < 3 || height < 3 || sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
    for row in bounds.min_y..bounds.max_y {
    for col in bounds.min_x..bounds.max_x {
            if row >= height - 1 || col >= width - 1 {
                continue;
            }
            let idx = row * width + col;
            let cells = unsafe { &*sim.cells.ptr };
            let e = cells.element_idx.get(idx);
            // 原版 L42423：state&3==2 且 mass>0 才调 UpdateLiquid
            let is_liquid = match elements_table::get_element_liquid_data(e) {
                Some(d) => (d.state & 3) == 2,
                None => false,
            };
            if is_liquid && cells.mass.get(idx) > 0.0 {
                update_liquid(sim, idx);
            }
        }
    }
}

/// PostProcessCell 遍历循环（原版 UpdateData L43023 对每个内部格无条件调
/// PostProcessCell）。post_process_cell 内部按真空/气体/固体/液体四分支处理，
/// 元素表缺失时安全返回，故此处对全部内部格无条件调用。
/// 挤压位移 / 密度分层 / 升华 accumulated_flow 累加由此在生产中真正执行。
pub(crate) fn post_process_loop(
    sim: &mut crate::a_framework::sim_data::SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    let width = sim.width as usize;
    let height = sim.height as usize;
    if width < 3 || height < 3 || sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
    for row in bounds.min_y..bounds.max_y {
    for col in bounds.min_x..bounds.max_x {
            if row >= height - 1 || col >= width - 1 {
                continue;
            }
            post_process_cell(sim, row * width + col);
        }
    }
}

/// 太空真空特判（原版 UpdateData L145466-145520，split 11_msvcrt_ignored.c L42954-43020）。
///
/// 条件：worldZones[cell] == -1（0xFF，世界外部/太空格）且背墙为真空。
/// 行为（读 updatedCells 元素 state，postProcessData.state & 3）：
/// - 气体（state==1）：mass -= 1.0 × 0.02（每帧 0.02kg）
/// - 液体（state==2）：mass -= 1000.0 × 0.02（每帧 20kg）
/// - 固体/真空：跳过
/// - mass 钳制 >= 0，写回 updatedCells；flow[cell] 重置 {-0.3, 0.3, 0.3, -0.3}
///
/// 2026-08-04 实现：此前缺失 → 太空暴露的液体/气体不被持续删除
/// （用户实测：原版 Space exposure 持续删除至空，项目版不删）。
pub(crate) fn run_space_vacuum_task(
    sim: &mut crate::a_framework::sim_data::SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    if sim.world_zones.ptr.is_null()
        || sim.backwall.ptr.is_null()
        || sim.updated_cells.ptr.is_null()
        || sim.flow.ptr.is_null()
    {
        return;
    }
    let width = sim.width as usize;
    let height = sim.height as usize;
    if width < 3 || height < 3 {
        return;
    }
    let total = width * height;
    let world_zones = unsafe { std::slice::from_raw_parts(sim.world_zones.ptr, total) };
    let backwall = unsafe { &*sim.backwall.ptr };
    // 原版 L145470：backwall.elementIdx[cell] == *(short*)backwall（backwall 首 2 字节）。
    // BackwallSOA 构造（11_msvcrt_ignored.c L21506-21507：this->vacuumElementIdx = param_2）
    // 确认首字段 = vacuumElementIdx。元素表定性诊断（2026-08-04）：
    //   count=212、zero_idx=None（无 id=0）、vac_idx=211、vac_table_idx=211 →
    //   原版真空索引 = 211，原版只删"背墙 == 真空(211)"的太空格；
    //   原版存档太空格背墙 hash=0 → GetElementIndex(0)=65535 → 原版对该存档不删。
    //   此前 element_idx[0]（=65535 加载值）判定把原版不删的格也删了（过度宽松；
    //   old 版因无 vacuumElementIdx 字段才用 element_idx[0]）。修正回 vacuumElementIdx。
    let vacuum = backwall.vacuum_element_idx;
    unsafe {
        let updated = &mut *sim.updated_cells.ptr;
        let flow = std::slice::from_raw_parts_mut(sim.flow.ptr, total);
    for row in bounds.min_y..bounds.max_y {
    for col in bounds.min_x..bounds.max_x {
                if row >= height - 1 || col >= width - 1 {
                    continue;
                }
                let cell = row * width + col;
                // 原版 L42962：worldZones[cell] == -1（0xFF）
                if world_zones[cell] != 0xFF {
                    continue;
                }
                // 原版 L42967-42970：背墙元素 == backwall.vacuumElementIdx（真空 211）
                if backwall.element_idx.get(cell) != vacuum {
                    continue;
                }
                // 原版 L42979-42987：读 updatedCells 元素 state & 3
                let elem = updated.element_idx.get(cell);
                let state = match elements_table::get_element_post_process_data(elem) {
                    Some(ppd) => ppd.state & 3,
                    None => continue,
                };
                // 原版 L42989-42995：气体 1.0 / 液体 1000.0 / 固体跳过
                let rate = match state {
                    1 => 1.0,
                    2 => 1000.0,
                    _ => continue,
                };
                // 原版 L43000-43004：mass -= rate×0.02，钳制 >= 0
                let mut m = updated.mass.get(cell) - rate * 0.02;
                if m <= 0.0 {
                    m = 0.0;
                }
                updated.mass.set(cell, m);
                // 原版 L43005-43008：flow 重置 {-0.3, 0.3, 0.3, -0.3}
                flow[cell] = crate::a_framework::vector_math::Vector4f {
                    x: -0.3,
                    y: 0.3,
                    z: 0.3,
                    w: -0.3,
                };
            }
        }
    }
}

/// UpdateLiquid — 液体下落与质量转移。
/// 对照源码 05_liquid_flow.c L656-1212（向下 + 左/右水平扩散；斜向流动属后续任务）。
///
/// 三个向下分支（按原版顺序）：
/// 1. 同元素下落（src_elem == 下方格元素）：计算转移量 fVar22/fVar23，
///    调 update_neighbour_liquid_mass，源格减质量、flow[DOWN] 累加。
/// 2. 下方是 void → 源格转 vacuum（质量/温度/病菌清零），产生事件。
/// 3. 下方真空/气体 → 角落检查（SpawnFallingLiquid）或 SwapCells 交换 + 事件。
/// 然后水平扩散（左 L849-945 → 右 L946-1042）：判定链失败时跳过（fall-through，
/// 不 return），转移量基于前序分支递减后的局部剩余质量，flow[LEFT]/flow[RIGHT]
/// 分别累加到 flow 的 x/y 分量。
pub fn update_liquid(sim: &mut crate::a_framework::sim_data::SimData, cell: usize) {
    let width = sim.width as usize;
    if width == 0 || cell < width {
        return; // 防止下方格索引下溢
    }
    let below = cell - width;
    if sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }

    // —— 读源格数据（cells 缓冲，原版 L686-705）——
    // mass_self/disease_count_self 声明 mut：向下分支转移成功后按原版 L751/L764 递减，
    // 左分支用递减后的值计算扩散量（fVar22 = (fVar25 - left_mass) * 0.25，原版 L876）。
    let (src_elem, mut mass_self, mut disease_count_self, disease_idx_self) = {
        let cells = unsafe { &*sim.cells.ptr };
        if cell >= cells.element_idx.len() {
            return;
        }
        let src_elem = cells.element_idx.get(cell);
        match elements_table::get_element_liquid_data(src_elem) {
            Some(_) => {}
            None => return,
        }
        (
            src_elem,
            cells.mass.get(cell),
            cells.disease_count.get(cell),
            cells.disease_idx.get(cell),
        )
    };

    // —— 读下方格数据 ——
    let (below_elem_cells, below_mass_cells, below_props) = {
        let cells = unsafe { &*sim.cells.ptr };
        (
            cells.element_idx.get(below),
            cells.mass.get(below),
            cells.properties.get(below),
        )
    };
    // 下方格元素（updatedCells 缓冲，原版 L711 uVar9）—— 用于非固体判断
    let below_elem_updated = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        updated.element_idx.get(below)
    };

    // 原版 L712/L715：下方格**非固体**（updatedCells 元素 state&3!=3 且 cells properties&2==0）
    // 时才进入向下处理块；否则**跳过**向下处理（不是 return）——原版 L712 if 为 false 时
    // 直接落到 L849 左分支（本函数此前误实现为 return，挡住了左分支执行）。
    let below_updated_state = match elements_table::get_element_liquid_data(below_elem_updated) {
        Some(d) => d.state & 3,
        None => return,
    };
    if below_updated_state != 3 && (below_props & 2 == 0) {
        // 原版 L716：下方格元素状态（cells，bVar11）
        let below_state = match elements_table::get_element_liquid_data(below_elem_cells) {
            Some(d) => d.state & 3,
            None => return,
        };

        if src_elem == below_elem_cells {
            // ===== 分支 1：同元素下落（原版 L717-773）=====
            // fVar23 = 下方质量（若下方是液体 state==2）否则 0（原版 L718-724）
            let below_mass = if below_state == 2 { below_mass_cells } else { 0.0 };
            let src_ld = match elements_table::get_element_liquid_data(src_elem) {
                Some(d) => d,
                None => return,
            };
            // fVar22 = max(mass_self*1.01, max_mass) - below_mass，clamp>=0（原版 L725-732）。
            // ⚠️ 2026-08-02 根因修复：原版 `if (mass*1.01 <= max_mass) fVar22 = max_mass`
            // 即 fVar22 = **max**(mass*1.01, max_mass)（目标容量恒 ≥ 满格值，上层水可快速灌满下层，
            // 且超满格（如 1020>1000）时 f22 = 1030.2-1000 > 0 能向下排出）。
            // 旧实现误用 min → 接近平衡时滴漏 ≈0.1kg/帧（底层难满）、超满格排不下去（卡在 1020）。
            let mut f22 = (mass_self * 1.01).max(src_ld.max_mass) - below_mass;
            if f22 < 0.0 {
                f22 = 0.0;
            }
            // fVar23 = min(mass_self, 流率上限)，再 min(fVar22*0.5)（原版 L733-739）
            // 流率上限 = liquid_data +8 = viscosity（原版 L734 读 lVar1+8）。
            // ⚠️ 2026-08-02 根因修复：生产数据里 liquid.flow=0（YAML 无 flow 字段）、
            // viscosity=speed（水=125），旧实现误用 flow → f23 恒 0 → 液体不扩散/水压水不转移。
            let mut f23 = mass_self.min(src_ld.viscosity);
            f23 = f23.min(f22 * 0.5);
            // 原版 L740-741 条件：(mass_self<=min_vertical_flow || min_vertical_flow<=f23) && f23>0
            let min_vertical = src_ld.min_vertical_flow;
            if (mass_self <= min_vertical || min_vertical <= f23) && f23 > 0.0 {
                // 病菌转移量 = (f23/mass_self)*disease_count（原版 L742）
                let disease_to_transfer = ((f23 / mass_self) * disease_count_self as f32) as i32;
                let ok = update_neighbour_liquid_mass(
                    sim, cell, src_elem, below, below_elem_cells, f23, disease_idx_self,
                    disease_to_transfer,
                );
                if ok {
                    // 源格局部变量递减（原版 L751 local_res8 -= iVar14、L764 fVar25 -= fVar23）——
                    // 左分支计算扩散量须用递减后的质量/病菌（原版 L876/L889）
                    disease_count_self -= disease_to_transfer;
                    mass_self -= f23;
                    // 源格病菌减少（原版 L751/L758-762）
                    let updated = unsafe { &mut *sim.updated_cells.ptr };
                    if updated.disease_idx.get(cell) == disease_idx_self {
                        modify_disease_count(updated, cell, -disease_to_transfer);
                    }
                    // 源格质量减少（原版 L768）
                    updated.mass.set(cell, updated.mass.get(cell) - f23);
                    // flow[DOWN] 累加（原版 L769-770，Vector4f offset 8 = z）
                    if !sim.flow.ptr.is_null() {
                        let flow = unsafe { &mut *sim.flow.ptr };
                        let flow_slice = unsafe {
                            std::slice::from_raw_parts_mut(
                                flow,
                                sim.width as usize * sim.height as usize,
                            )
                        };
                        flow_slice[cell].z += f23;
                    }
                }
            }
        } else if below_state < 2 {
            // ===== 分支 2+3：下方是真空/气体（原版 L774-846）=====
            // 分支 2：下方是 void → 源格转 vacuum（原版 L775-792）
            if below_elem_cells == sim.void_element_idx {
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                updated.element_idx.set(cell, sim.vacuum_element_idx);
                updated.temperature.set(cell, 0.0);
                updated.mass.set(cell, 0.0);
                clear_disease(updated, cell);
                push_substance_change(sim, cell);
                return; // 原版 L787 goto LAB_180049ebd：不执行左分支
            }
            // 分支 3a：角落可下落 → SpawnFallingLiquid（原版 L793-841）
            let updated = unsafe { &*sim.updated_cells.ptr };
            let perm_left_up = is_liquid_permeable(updated, cell - 1);
            let perm_left_down = is_liquid_permeable(updated, below - 1);
            let perm_right_up = is_liquid_permeable(updated, cell + 1);
            let perm_right_down = is_liquid_permeable(updated, below + 1);
            let corner = (perm_left_up && !perm_left_down) || (perm_right_up && !perm_right_down);
            if corner {
                let cells = unsafe { &*sim.cells.ptr };
                let src_temp = cells.temperature.get(cell);
                if spawn_falling_liquid(
                    sim,
                    cell,
                    src_elem,
                    mass_self,
                    src_temp,
                    disease_idx_self,
                    disease_count_self,
                ) {
                    let updated = unsafe { &mut *sim.updated_cells.ptr };
                    updated.element_idx.set(cell, sim.vacuum_element_idx);
                    updated.temperature.set(cell, 0.0);
                    updated.mass.set(cell, 0.0);
                    clear_disease(updated, cell);
                    push_substance_change(sim, cell);
                    return; // 原版 L834 goto LAB_180049ebd：不执行左分支
                }
                // 原版 L817-818：SpawnFallingLiquid 失败 → return
                return;
            }
            // 分支 3b：SwapCells 交换 + 双事件（原版 L842-845）
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            swap_cells(updated, cell, below);
            push_substance_change(sim, cell);
            push_substance_change(sim, below);
            return; // 原版 L845 goto LAB_180049ebd：swap 后不执行左分支
        }
    }

    // —— 左分支（原版 L849-945）：水平向左扩散 ——
    // 原版 L849-851：fVar25 <= 0 → 直接返回（向下转移已耗尽源格质量）
    if mass_self <= 0.0 {
        return;
    }
    // 任务 2 修正（对齐原版）：左分支判定链是 **if 块**（原版 L864-866）——判定失败
    // （左邻固体/异液体/元素表缺失/左边界无左邻格）时**不 return**，而是 fall-through
    // 继续右分支（LAB_18004a18f L946）。故用嵌套 if 控制转移块，而非提前 return。
    if cell % width != 0 {
        let left = cell - 1;
        // 判定链（原版 L852-866）：
        // 1. 左邻元素（cells 快照）：== 本格元素 或 state&3 < 2
        let (left_elem_cells, left_mass_cells, left_props) = {
            let cells = unsafe { &*sim.cells.ptr };
            (
                cells.element_idx.get(left),
                cells.mass.get(left),
                cells.properties.get(left),
            )
        };
        let left_elem_updated = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            updated.element_idx.get(left)
        };
        let left_state_updated = match elements_table::get_element_liquid_data(left_elem_updated) {
            Some(d) => d.state & 3,
            None => 0xFF, // 元素表缺失 → 判定失败（跳过左转移，fall-through 到右分支）
        };
        let left_state_cells = match elements_table::get_element_liquid_data(left_elem_cells) {
            Some(d) => d.state & 3,
            None => 0xFF,
        };
        let same_or_gas = src_elem == left_elem_cells || left_state_cells < 2;
        // 原版 L864-866：(same_or_gas) && (updated 非固体)；L869：properties&2==0
        if same_or_gas && left_state_updated != 3 && (left_props & 2 == 0) {
            // 扩散量计算（原版 L867-882）：
            let left_mass = if left_state_cells == 2 { left_mass_cells } else { 0.0 };
            let src_ld = match elements_table::get_element_liquid_data(src_elem) {
                Some(d) => d,
                None => return,
            };
            let mut f23 = (mass_self - left_mass) * 0.25; // fVar22（原版 L876）
            // fVar23 = min(mass, 流率上限)（原版 L877-880 读 lVar1+8 = viscosity；
            // 生产数据 flow=0、viscosity=speed，2026-08-02 根因修复）
            let mut f23_flow = mass_self.min(src_ld.viscosity);
            f23_flow = f23_flow.min(f23);
            f23 = f23_flow;
            // 门限（原版 L884）：f23 >= min_horizontal_flow 且 f23 > 0
            if (f23 >= src_ld.min_horizontal_flow) && f23 > 0.0 {
                // 病菌按比例（原版 L886）
                let disease_to_transfer = ((f23 / mass_self) * disease_count_self as f32) as i32;
                // 原版 L887-909：IsSolid(下方) && IsLiquidPermeable(左下方) → 先试 SpawnFallingLiquid
                //（液滴轨道，2026-08-02 实现：质量离开格子由 C# 渲染+落地回注，悬崖处不再有实体方格滑落）；
                // 成功则不转邻格；失败回退原版 L917-919 的 UpdateNeighbourLiquidMass(左邻)。
                let (below_solid, src_temp, src_disease_idx) = {
                    let cells = unsafe { &*sim.cells.ptr };
                    let bs = is_solid(cells, below);
                    let (t, d) = if bs {
                        (cells.temperature.get(cell), cells.disease_idx.get(cell))
                    } else {
                        (0.0f32, 0xFFu8)
                    };
                    (bs, t, d)
                };
                let diagonal_permeable = if below_solid {
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    is_liquid_permeable(updated, left - width)
                } else {
                    false
                };
                let mut transferred = false;
                if below_solid && diagonal_permeable {
                    transferred = spawn_falling_liquid(
                        sim,
                        left,
                        src_elem,
                        f23,
                        src_temp,
                        src_disease_idx,
                        disease_to_transfer,
                    );
                }
                if !transferred {
                    transferred = update_neighbour_liquid_mass(
                        sim, cell, src_elem, left, left_elem_cells, f23, disease_idx_self,
                        disease_to_transfer,
                    );
                }
                if transferred {
                    // 源格局部变量递减（原版 L923 local_res8 -= iVar15、L936 fVar25 -= fVar23）——
                    // 右分支计算扩散量须用左分支递减后的剩余质量/病菌
                    disease_count_self -= disease_to_transfer;
                    mass_self -= f23;
                    // 源格病菌减少（原版 L930-934）
                    let updated = unsafe { &mut *sim.updated_cells.ptr };
                    if updated.disease_idx.get(cell) == disease_idx_self {
                        modify_disease_count(updated, cell, -disease_to_transfer);
                    }
                    // 源格质量减少（原版 L936）
                    updated.mass.set(cell, updated.mass.get(cell) - f23);
                    // flow[LEFT] 累加（原版 L941-942，Vector4f offset 0 = x）
                    if !sim.flow.ptr.is_null() {
                        let flow = unsafe { &mut *sim.flow.ptr };
                        let flow_slice = unsafe {
                            std::slice::from_raw_parts_mut(
                                flow,
                                sim.width as usize * sim.height as usize,
                            )
                        };
                        flow_slice[cell].x += f23;
                    }
                }
            }
        }
    }

    // —— 右分支（原版 L946-1042）：水平向右扩散 ——
    // 原版 L947-948：fVar25 <= 0 → 直接返回（左分支已耗尽源格质量）
    if mass_self <= 0.0 {
        return;
    }
    if cell % width != width - 1 {
        let right = cell + 1;
        // 判定链（原版 L950-966）：与左分支对称
        let (right_elem_cells, right_mass_cells, right_props) = {
            let cells = unsafe { &*sim.cells.ptr };
            (
                cells.element_idx.get(right),
                cells.mass.get(right),
                cells.properties.get(right),
            )
        };
        let right_elem_updated = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            updated.element_idx.get(right)
        };
        let right_state_updated = match elements_table::get_element_liquid_data(right_elem_updated) {
            Some(d) => d.state & 3,
            None => 0xFF, // 元素表缺失 → 判定失败（跳过右转移，函数至此结束）
        };
        let right_state_cells = match elements_table::get_element_liquid_data(right_elem_cells) {
            Some(d) => d.state & 3,
            None => 0xFF,
        };
        let same_or_gas = src_elem == right_elem_cells || right_state_cells < 2;
        // 原版 L961-963：(same_or_gas) && (updated 非固体)；L966：properties&2==0
        if same_or_gas && right_state_updated != 3 && (right_props & 2 == 0) {
            // 扩散量计算（原版 L967-983）：mass_self 为左分支递减后的剩余质量
            let right_mass = if right_state_cells == 2 { right_mass_cells } else { 0.0 };
            let src_ld = match elements_table::get_element_liquid_data(src_elem) {
                Some(d) => d,
                None => return,
            };
            let mut f23 = (mass_self - right_mass) * 0.25; // fVar22（原版 L976）
            // fVar23 = min(mass, 流率上限)（原版 L977-980 读 lVar1+8 = viscosity；
            // 生产数据 flow=0、viscosity=speed，2026-08-02 根因修复）
            let mut f23_flow = mass_self.min(src_ld.viscosity);
            f23_flow = f23_flow.min(f23);
            f23 = f23_flow;
            // 门限（原版 L984）：f23 >= min_horizontal_flow 且 f23 > 0
            if (f23 >= src_ld.min_horizontal_flow) && f23 > 0.0 {
                // 病菌按比例（原版 L986）
                let disease_to_transfer = ((f23 / mass_self) * disease_count_self as f32) as i32;
                // 原版 L987-1007：IsSolid(下方) && IsLiquidPermeable(右下方) → 先试 SpawnFallingLiquid
                //（液滴轨道，2026-08-02 实现，与左分支对称）；失败回退 L1014-1016 UpdateNeighbourLiquidMass(右邻)。
                let (below_solid, src_temp, src_disease_idx) = {
                    let cells = unsafe { &*sim.cells.ptr };
                    let bs = is_solid(cells, below);
                    let (t, d) = if bs {
                        (cells.temperature.get(cell), cells.disease_idx.get(cell))
                    } else {
                        (0.0f32, 0xFFu8)
                    };
                    (bs, t, d)
                };
                let diagonal_permeable = if below_solid {
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    is_liquid_permeable(updated, right - width)
                } else {
                    false
                };
                let mut transferred = false;
                if below_solid && diagonal_permeable {
                    transferred = spawn_falling_liquid(
                        sim,
                        right,
                        src_elem,
                        f23,
                        src_temp,
                        src_disease_idx,
                        disease_to_transfer,
                    );
                }
                if !transferred {
                    transferred = update_neighbour_liquid_mass(
                        sim, cell, src_elem, right, right_elem_cells, f23, disease_idx_self,
                        disease_to_transfer,
                    );
                }
                if transferred {
                    // 注：原版 L1020/L1033 在此递减局部变量 local_res8/fVar25，但右分支是
                    // UpdateLiquid 最后一个消费方（其后无其他分支读取），Rust 省略等价。
                    // 源格病菌减少（原版 L1027-1031）
                    let updated = unsafe { &mut *sim.updated_cells.ptr };
                    if updated.disease_idx.get(cell) == disease_idx_self {
                        modify_disease_count(updated, cell, -disease_to_transfer);
                    }
                    // 源格质量减少（原版 L1033）
                    updated.mass.set(cell, updated.mass.get(cell) - f23);
                    // flow[RIGHT] 累加（原版 L1038-1039，Vector4f offset 4 = y）
                    if !sim.flow.ptr.is_null() {
                        let flow = unsafe { &mut *sim.flow.ptr };
                        let flow_slice = unsafe {
                            std::slice::from_raw_parts_mut(
                                flow,
                                sim.width as usize * sim.height as usize,
                            )
                        };
                        flow_slice[cell].y += f23;
                    }
                }
            }
        }
    }

    // —— 上推分支（原版 L1043-1110，LAB_18004a4b2）：压力回流 ——
    // 原版 L1044-1045：fVar25 <= 0 → 直接返回（前序分支已耗尽源格质量）。
    // 2026-08-02 实现：此前缺失 → 超满格（如 2000kg）无法向上回流 → 过压 200%+。
    // 原版三层水时底层封顶 ~1020kg（threshold = max(中层×1.01, maxMass)）。
    if mass_self <= 0.0 {
        return;
    }
    let up = cell + width;
    let total = width * sim.height as usize;
    if up < total {
        // 判定链（原版 L1047-1064）：与左右分支对称
        let (up_elem_cells, up_mass_cells, up_props) = {
            let cells = unsafe { &*sim.cells.ptr };
            (
                cells.element_idx.get(up),
                cells.mass.get(up),
                cells.properties.get(up),
            )
        };
        let up_elem_updated = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            updated.element_idx.get(up)
        };
        let up_state_updated = match elements_table::get_element_liquid_data(up_elem_updated) {
            Some(d) => d.state & 3,
            None => 0xFF,
        };
        let up_state_cells = match elements_table::get_element_liquid_data(up_elem_cells) {
            Some(d) => d.state & 3,
            None => 0xFF,
        };
        let same_or_gas = src_elem == up_elem_cells || up_state_cells < 2;
        if same_or_gas && up_state_updated != 3 && (up_props & 2 == 0) {
            // 压力公式（原版 L1066-1077）：
            let up_mass = if up_state_cells == 2 { up_mass_cells } else { 0.0 };
            let src_ld = match elements_table::get_element_liquid_data(src_elem) {
                Some(d) => d,
                None => return,
            };
            // threshold = max(up_mass×1.01, max_mass)（原版 L1066-1069，与下落分支同款 max 公式）
            let threshold = (up_mass * 1.01).max(src_ld.max_mass);
            let mut transfer = (mass_self - threshold).max(0.0) * 0.5;
            // 原版 L1070-1075：mass < 2×threshold → 受 viscosity / mass 上限约束
            if mass_self < threshold * 2.0 {
                transfer = transfer.min(src_ld.viscosity).min(mass_self);
            }
            // 原版 L1077：0.01 < transfer 才执行
            if transfer > 0.01 {
                let disease_to_transfer =
                    ((transfer / mass_self) * disease_count_self as f32) as i32;
                let ok = update_neighbour_liquid_mass(
                    sim, cell, src_elem, up, up_elem_cells, transfer, disease_idx_self,
                    disease_to_transfer,
                );
                if ok {
                    // 注：原版 L1087/L1093 在此递减局部变量 local_res8/fVar25，但上推分支是
                    // UpdateLiquid 最后一个分支（其后无其他分支读取），Rust 省略等价。
                    let updated = unsafe { &mut *sim.updated_cells.ptr };
                    if updated.disease_idx.get(cell) == disease_idx_self {
                        modify_disease_count(updated, cell, -disease_to_transfer);
                    }
                    updated.mass.set(cell, updated.mass.get(cell) - transfer);
                    // flow[UP] 累加（原版 L1090-1091，Vector4f offset 0xc = w）
                    if !sim.flow.ptr.is_null() {
                        let flow = unsafe { &mut *sim.flow.ptr };
                        let flow_slice = unsafe {
                            std::slice::from_raw_parts_mut(
                                flow,
                                sim.width as usize * sim.height as usize,
                            )
                        };
                        flow_slice[cell].w += transfer;
                    }
                }
            }
        }
    }

    // —— 阶段 E：对角换位（原版 L1128-1205）——
    // 正交方向走不通且质量未耗尽时，尝试与对角格（下右/下左）的**气体**整格换位。
    // 全部从 updatedCells 读取（原版 uVar4 = updatedCells），SwapCells + 双事件。
    // 2026-08-06 实现：此前"斜向流动属后续任务"从未实现 → 用户实测
    // [气1(1,1) 左][液1(2,1) 中][液2(2,2) 上] 砖封布局，液2 与左下对角气体换位缺失
    // （原版 1 秒后：气1→(2,2)、液2→(1,1)、液1 不动；液2 位置为真空则气1 不动）。
    if mass_self <= 0.0 {
        return;
    }
    let below = cell - width;
    let below_right = below + 1;
    let mut swap_target: Option<usize> = None;
    {
        let updated = unsafe { &*sim.updated_cells.ptr };
        // 优先"下右"对角（原版 L1130-1156）
        if below_right < updated.element_idx.len() {
            let br_state = match elements_table::get_element_liquid_data(
                updated.element_idx.get(below_right),
            ) {
                Some(d) => d.state & 3,
                None => 0xFF,
            };
            if br_state == 1 {
                // 原版 L1139-1145：right（cell+1）非固体且可渗透(bit0==0) → 放弃右下
                //（与左下分支检查 left 对称；2026-08-06 修正：此前误用 below 导致
                // 镜像布局（气体在右下对角）不触发）
                let right = cell + 1;
                let right_state = match elements_table::get_element_liquid_data(
                    updated.element_idx.get(right),
                ) {
                    Some(d) => d.state & 3,
                    None => 0xFF,
                };
                let mut ok = true;
                if right_state != 3 {
                    if updated.properties.get(right) & 1 == 0 {
                        ok = false;
                    }
                }
                let below_state = match elements_table::get_element_liquid_data(
                    updated.element_idx.get(below),
                ) {
                    Some(d) => d.state & 3,
                    None => 0xFF,
                };
                if ok && below_state == 3 {
                    ok = false; // 原版 L1146：below 固体 → 放弃右下
                }
                if ok && updated.properties.get(cell) & 1 != 0 {
                    ok = false; // 原版 L1150
                }
                if ok && updated.properties.get(below_right) & 2 != 0 {
                    ok = false; // 原版 L1153
                }
                if ok && updated.properties.get(below) & 3 != 0 {
                    ok = false; // 原版 L1156
                }
                if ok {
                    swap_target = Some(below_right);
                }
            }
        }
    }
    if swap_target.is_none() {
        // 回退"下左"对角（原版 L1158-1189）
        let below_left = below.wrapping_sub(1);
        let updated = unsafe { &*sim.updated_cells.ptr };
        if below_left >= updated.element_idx.len() {
            return;
        }
        let bl_state = match elements_table::get_element_liquid_data(
            updated.element_idx.get(below_left),
        ) {
            Some(d) => d.state & 3,
            None => 0xFF,
        };
        if bl_state != 1 {
            return; // 左下不是气体 → 返回（原版 L1162-1164）
        }
        let left = cell.wrapping_sub(1);
        let left_state = match elements_table::get_element_liquid_data(updated.element_idx.get(left))
        {
            Some(d) => d.state & 3,
            None => 0xFF,
        };
        if left_state != 3 {
            if updated.properties.get(left) & 1 == 0 {
                return; // left 可渗透 → 返回（原版 L1166-1170）
            }
        }
        let below_state = match elements_table::get_element_liquid_data(
            updated.element_idx.get(below),
        ) {
            Some(d) => d.state & 3,
            None => 0xFF,
        };
        if below_state == 3 {
            return; // 原版 L1173
        }
        if updated.properties.get(cell) & 1 != 0 {
            return; // 原版 L1176
        }
        if updated.properties.get(below_left) & 2 != 0 {
            return; // 原版 L1179
        }
        if updated.properties.get(below) & 3 != 0 {
            return; // 原版 L1182
        }
        swap_target = Some(below_left);
    }
    if let Some(t) = swap_target {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        swap_cells(updated, cell, t);
        push_substance_change(sim, cell);
        push_substance_change(sim, t);
    }
}

/// UpdateNeighbourLiquidMass — 邻格质量转移辅助。
/// 对照源码 05_liquid_flow.c L1213-1274。
///
/// 参数：src_cell 源格、src_elem 源元素、dst_cell 目标格、dst_elem 目标元素、
/// amount 转移量、disease_idx 病菌索引、disease_count 转移病菌数。
/// 返回 false 表示转移被拒绝（目标为不同液体 / 气体无法置换 / 索引越界）。
fn update_neighbour_liquid_mass(
    sim: &mut crate::a_framework::sim_data::SimData,
    src_cell: usize,
    src_elem: u16,
    dst_cell: usize,
    _dst_elem: u16,
    amount: f32,
    disease_idx: u8,
    disease_count: i32,
) -> bool {
    if sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return false;
    }
    // fVar1 = 源格温度（从 cells 读，原版 L1227-1230）
    let src_temp = {
        let cells = unsafe { &*sim.cells.ptr };
        cells.temperature.get(src_cell)
    };
    // 目标格当前元素（原版 L1235 uVar2）——共享借用读取，避免跨 &mut SimData 调用
    let dst_elem_now = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        if dst_cell >= updated.element_idx.len() {
            return false;
        }
        updated.element_idx.get(dst_cell)
    };
    // 原版 L1236：目标已是同元素 → AddMassAndUpdateTemperature + return true
    if src_elem == dst_elem_now {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        add_mass_and_update_temperature(updated, dst_cell, amount, src_temp, disease_idx, disease_count);
        return true;
    }
    let elem_count = elements_table::get_element_count_pub();
    if (dst_elem_now as usize) >= elem_count {
        return false;
    }
    // 原版 L1243：目标是 void → 目标格质量清零
    if dst_elem_now == sim.void_element_idx {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.mass.set(dst_cell, 0.0);
        return true;
    }
    let dst_state = match elements_table::get_element_liquid_data(dst_elem_now) {
        Some(d) => d.state & 3,
        None => return false,
    };
    // 原版 L1252-1257：目标是液体（不同元素）→ false；目标是气体且无法置换 → false
    if dst_state == 2 {
        return false;
    }
    if dst_state == 1 {
        // 原版 L1252-1257：目标气体 → 先 DisplaceGas 把气体**挤走**（单方向全量转移
        // 到 [下/左/右/上] 候选），挤不走（四周均非目标元素/真空）→ 液体无法流入，返回 false。
        // 2026-08-04 修复：此前直接 return false（YAGNI 占位）→ 液体永不水平推挤气体。
        if !displace_gas(sim, dst_cell, dst_elem_now) {
            return false;
        }
        // DisplaceGas 已把目标格清空（元素=真空、质量=0），继续按原版 L1261-1265 改回源液体元素。
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.element_idx.set(dst_cell, src_elem);
        add_mass_and_update_temperature(updated, dst_cell, amount, src_temp, disease_idx, disease_count);
        push_substance_change(sim, dst_cell);
        return true;
    }
    // 目标格元素改为源元素（原版 L1261）
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    updated.element_idx.set(dst_cell, src_elem);
    add_mass_and_update_temperature(updated, dst_cell, amount, src_temp, disease_idx, disease_count);
    // 事件：ChangeSubstance（原版 L1265）—— 元素类型变化必须通知 C# 刷新画面
    push_substance_change(sim, dst_cell);
    true
}

/// AddMassAndUpdateTemperature（原版 11_msvcrt_ignored.c L33887-33994）。
/// 目标格质量 += amount；温度按质量加权混合并钳制到 [min, max]；
/// 病菌按 disease_idx/disease_count 转移（AddDiseaseToCell）。
pub(crate) fn add_mass_and_update_temperature(
    cells: &mut CellSOA,
    cell: usize,
    amount: f32,
    src_temp: f32,
    disease_idx: u8,
    disease_count: i32,
) {
    let cur_mass = cells.mass.get(cell);
    let new_mass = cur_mass + amount;
    if new_mass <= 0.0 {
        // 原版 L33913-33919：质量清零 → 温度清零 + ClearDisease
        cells.temperature.set(cell, 0.0);
        clear_disease(cells, cell);
        return;
    }
    let cur_temp = cells.temperature.get(cell);
    // 原版 L33925-33947：质量加权平均温度，钳制到 [min(cur,src), max(cur,src)]
    let avg = (cur_mass * cur_temp + src_temp * amount) / new_mass;
    let lo = cur_temp.min(src_temp);
    let hi = cur_temp.max(src_temp);
    let mixed = avg.clamp(lo, hi);
    cells.mass.set(cell, new_mass);
    cells.temperature.set(cell, mixed);
    add_disease_to_cell(cells, cell, disease_idx, disease_count);
}

/// Disease::AddDiseaseToCell（原版 11_msvcrt_ignored.c L26163-26263）。
///
/// 病菌强度混合，逐分支对照原版：
/// - 同病菌：count 累加（L26173-26174）；
/// - 异病菌：默认取输入（L26175-26177），但格内已有真实病菌时"保留当前"
///   先执行（L26178 逗号表达式 iVar9=iVar6, bVar7=bVar1）→
///   干净输入（new==0xff）不清除现有病菌；
/// - 双方真实病菌（cur!=0xff 且 new!=0xff）→ 从"保留当前"起点做强度混合
///   （L26180-26219）：
///     f10 = cur_count × strength[cur]，f11 = new_count × strength[new]
///     f10<=f11：cur<0 → result=-cur, idx=new；否则保留当前
///     f10>f11：d=(int)(new - (f10/f11)×cur)；d<0 → idx=cur；
///              result=-d（wrapping，对应 MSVC 无符号回绕），-1<d → result=d
/// - result>0：idx 变化 → infestation_tick_count 清零（L26225-26231）；
/// - result<=0：清除病菌四字段（L26234-26251）。
///
/// 防御：gDisease 表缺失或索引越界（原版越界断言/UB）→ 保留当前。
pub(crate) fn add_disease_to_cell(
    cells: &mut CellSOA,
    cell: usize,
    disease_idx: u8,
    disease_count: i32,
) {
    let cur_idx = cells.disease_idx.get(cell);
    let cur_count = cells.disease_count.get(cell);
    let (new_idx, new_count) = if cur_idx == disease_idx {
        (disease_idx, cur_count + disease_count)
    } else {
        // 原版 L26175-26178：先默认取输入；但格内已有真实病菌时"保留当前"
        // 先执行（L26178 逗号表达式 iVar9=iVar6, bVar7=bVar1），
        // 因此输入 0xff（干净质量）流入带菌格不会清除病菌。
        // 2026-08-06 修复：此前输入 0xff 走"替换" → 孢子兰格被补充的
        // 干净 CO2 每子步清空病菌（用户实测：流动补充环境病菌消失，
        // 高压→低压无补充则正常；原版保留病菌）。
        let mut result_idx = cur_idx;
        let mut result_count = cur_count;
        if cur_idx == 0xFF {
            // 格内无病菌：输入（含 0xff）直接生效（原版 L26175-26177 默认值）
            result_idx = disease_idx;
            result_count = disease_count;
        } else if disease_idx != 0xFF {
            // 双方真实病菌 → 强度混合（原版 L26178-26219，起点为保留当前）
            let table_ptr = crate::globals::G_DISEASE.lock().0;
            let strengths = if table_ptr.is_null() {
                None
            } else {
                let t = unsafe { &*table_ptr };
                Some(t.diseases.as_slice())
            };
            match strengths {
                Some(s) if (cur_idx as usize) < s.len() && (disease_idx as usize) < s.len() => {
                    let f10 = cur_count as f32 * s[cur_idx as usize].strength;
                    let f11 = disease_count as f32 * s[disease_idx as usize].strength;
                    if f10 <= f11 {
                        if cur_count < 0 {
                            result_count = cur_count.wrapping_neg();
                            result_idx = disease_idx;
                        }
                    } else {
                        let d = (disease_count as f32 - (f10 / f11) * cur_count as f32) as i32;
                        if d < 0 {
                            // 混合差值为负 → 当前病菌胜出（原版 param_3 = bVar1）
                            result_idx = cur_idx;
                            result_count = d.wrapping_neg();
                        } else {
                            // d>=0 → 新病菌（原版 bVar7 = param_3 未被改写）
                            result_idx = disease_idx;
                            result_count = d;
                        }
                    }
                }
                _ => {
                    // 表缺失 / 索引越界（理论不发生的脏数据）→ 保留当前
                }
            }
        }
        (result_idx, result_count)
    };
    cells.disease_idx.set(cell, new_idx);
    cells.disease_count.set(cell, new_count);
    if new_count > 0 {
        // 原版 L26225-26231：病菌类型变化 → 侵扰 tick count 清零
        if new_idx != cur_idx {
            cells.disease_infestation_tick_count.set(cell, 0);
        }
        return;
    }
    // 原版 L26234-26251：count<=0 → 清除病菌
    clear_disease(cells, cell);
}

/// CellSOA::ModifyDiseaseCount（原版 11_msvcrt_ignored.c L43462-43485）。
/// disease_count += delta；<=0 时清除病菌四字段。
fn modify_disease_count(cells: &mut CellSOA, cell: usize, delta: i32) {
    let new = cells.disease_count.get(cell) + delta;
    cells.disease_count.set(cell, new);
    if new > 0 {
        return;
    }
    clear_disease(cells, cell);
}

/// CellSOA::ClearDisease（原版 11_msvcrt_ignored.c L43186-43214）。
pub(crate) fn clear_disease(cells: &mut CellSOA, cell: usize) {
    cells.disease_idx.set(cell, 0xFF);
    cells.disease_count.set(cell, 0);
    cells.disease_infestation_tick_count.set(cell, 0);
    cells.disease_growth_accumulated_error.set(cell, 0.0);
}

/// CellSOA::SwapCells（原版 11_msvcrt_ignored.c L33397-33461）。
/// 交换两格 7 个字段：element_idx/mass/temperature/disease_idx/disease_count/
/// disease_infestation_tick_count/disease_growth_accumulated_error。
fn swap_cells(cells: &mut CellSOA, a: usize, b: usize) {
    let ea = cells.element_idx.get(a);
    cells.element_idx.set(a, cells.element_idx.get(b));
    cells.element_idx.set(b, ea);
    let ma = cells.mass.get(a);
    cells.mass.set(a, cells.mass.get(b));
    cells.mass.set(b, ma);
    let ta = cells.temperature.get(a);
    cells.temperature.set(a, cells.temperature.get(b));
    cells.temperature.set(b, ta);
    let dia = cells.disease_idx.get(a);
    cells.disease_idx.set(a, cells.disease_idx.get(b));
    cells.disease_idx.set(b, dia);
    let dca = cells.disease_count.get(a);
    cells.disease_count.set(a, cells.disease_count.get(b));
    cells.disease_count.set(b, dca);
    let ita = cells.disease_infestation_tick_count.get(a);
    cells.disease_infestation_tick_count.set(a, cells.disease_infestation_tick_count.get(b));
    cells.disease_infestation_tick_count.set(b, ita);
    let ga = cells.disease_growth_accumulated_error.get(a);
    cells.disease_growth_accumulated_error.set(a, cells.disease_growth_accumulated_error.get(b));
    cells.disease_growth_accumulated_error.set(b, ga);
}

/// SimEvents::ChangeSubstance（原版 11_msvcrt_ignored.c L33178-33212）。
/// simCell → gameCell 坐标转换，push substance_change_info
/// （old/new 填 0xFFFF 占位，由 copy_sim_data_to_game 后处理回填），
/// 并标记 timers[simCell] |= 0x1F。
/// 这是 C# 画面刷新的关键：液体转移后格子元素变化必须产生事件。
pub(crate) fn push_substance_change(sim: &mut crate::a_framework::sim_data::SimData, sim_cell: usize) {
    if sim.sim_events.ptr.is_null() {
        return;
    }
    let width = sim.width;
    let game_w = width - 2;
    let internal = sim_cell as i32;
    let game_cell = (internal % width - 1) + (internal / width - 1) * game_w;
    if game_cell >= 0 && game_cell < sim.num_game_cells {
        crate::c2_physics::region_events::emit_substance_change(
            sim,
            SubstanceChangeInfo {
                cell_idx: game_cell,
                old_element_idx: 0xFFFF,
                new_element_idx: 0xFFFF,
            },
        );
    }
    if !sim.timers.ptr.is_null() {
        unsafe {
            let timers_ptr = sim.timers.ptr as *mut u8;
            let old = std::ptr::read(timers_ptr.add(sim_cell));
            std::ptr::write(timers_ptr.add(sim_cell), old | 0x1F);
        }
    }
}

/// SimEvents::SpawnFallingLiquid（原版 11_msvcrt_ignored.c L43489-43554）简化版。
/// 液体从壁边下落时产生 spawn_liquid_info 事件（下落粒子效果）。
/// headless 模式返回 false；visible_grid 为 null（未分配）时保守返回 false。
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_falling_liquid(
    sim: &mut crate::a_framework::sim_data::SimData,
    cell: usize,
    elem_idx: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
) -> bool {
    if sim.headless {
        return false;
    }
    let width = sim.width as usize;
    if sim.cells.ptr.is_null() || sim.visible_grid.ptr.is_null() {
        return false;
    }
    if cell < width {
        return false;
    }
    let cells = unsafe { &*sim.cells.ptr };
    // 原版 L43510-43512：上方格 properties & 2 == 0（上方非固体）
    if cells.properties.get(cell - width) & 2 != 0 {
        return false;
    }
    // 原版 L43513-43516：game cell 计算 + 可见性检查
    let i4 = ((cell / width) as i32 - 1) * (sim.width - 2) + (cell % width) as i32;
    let callback = i4 - 1;
    if callback < 0 || callback >= sim.num_game_cells {
        return false;
    }
    let visible = unsafe { std::ptr::read(sim.visible_grid.ptr.add(i4 as usize - 1)) };
    if visible == 0 && !sim.debug_properties.is_debug_editing {
        return false;
    }
    // 原版 L43524-43526：温度范围检查 low_temp-3 <= temp <= high_temp+3
    match elements_table::get_element_temperature_data(elem_idx) {
        Some(td) => {
            if temperature < td.low_temp - 3.0 || temperature > td.high_temp + 3.0 {
                return false;
            }
        }
        None => return false,
    }
    if sim.sim_events.ptr.is_null() {
        return false;
    }
    crate::c2_physics::region_events::emit_spawn_liquid(
        sim,
        crate::a_framework::game_data::SpawnFallingLiquidInfo {
            cell_idx: callback,
            element_idx: elem_idx,
            disease_idx,
            pad: 0,
            mass,
            temperature,
            disease_count,
        },
    );
    true
}

// ===== 任务 7：UpdatePressure（压强平衡）=====
// 对照源码 05_liquid_flow.c L1436-1629（UpdatePressure）。
//
// 语义：单邻格质量转移——本格与 `neighbor` 之间的质量差 × 压力系数（上限 12.5%），
// 使相邻液体质量趋于平衡。原版由 UpdateData（11_msvcrt_ignored.c L42108/42144/42192）
// 对每个方向各调一次，并用**返回值**在调用点累加 flow 分量（任务 9 挂接时处理）；
// 本函数只实现函数本体，返回 clamp 后的带符号转移量 fVar18。

/// UpdatePressure — 向单个邻格转移质量差 × 压力系数，使两格液体质量趋于平衡。
/// 对照源码 05_liquid_flow.c L1436-1629。
///
/// 参数：
/// - `cell`：本格（原版 param_5）
/// - `neighbor`：邻格索引（原版 param_9，方向由调用方决定）
///
/// 算法（逐行对照原版）：
/// 1. 转移量 `f18 = flow_coeff × (mass[cell] − mass[neighbor])`（原版 L1478）；
///    flow_coeff = 邻格元素 pressure_data.flow（原版 L1476，param_7 != 1 时覆盖）。
/// 2. clamp 上限 +mass[cell]×12.5%（L1479-1481）、下限 −mass[neighbor]×12.5%（L1482-1484）。
/// 3. 方向：f18 >= 0（本格→邻格）来源=本格/接收=邻格；f18 < 0（邻格→本格）反之（L1485-1496）。
/// 4. 实际转移量 = min(|f18|, 来源格质量)（L1497-1500）。
/// 5. 病菌按质量比例转移（L1501-1513），来源格病菌 idx/温度从 updatedCells 读（L1504-1518）。
/// 6. 接收格质量 += 转移量 + 温度加权混合 + 病菌（L1519-1556，复用 add_mass_and_update_temperature）。
/// 7. 来源格质量/病菌减少（L1559-1580）。
/// 8. 元素段（L1581-1608）：仅当接收格元素 pressure state & 3 == 0（真空）时，
///    接收格元素改为来源元素并产生 ChangeSubstance 事件（复用 push_substance_change）；
///    接收格原为 void 时清零质量/温度/病菌。
///
/// 返回值：clamp 后的带符号转移量 fVar18，供调用点累加 flow 分量（原版 L42121-42126）。
/// 元素表缺失（测试环境）或索引越界时安全返回 0.0。
pub fn update_pressure(sim: &mut crate::a_framework::sim_data::SimData, cell: usize, neighbor: usize) -> f32 {
    if sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return 0.0;
    }
    let cells = unsafe { &*sim.cells.ptr };
    if cell >= cells.mass.len() || neighbor >= cells.mass.len() {
        return 0.0;
    }
    // —— 元素与压力数据（原版 L1463-1471）——
    // uVar14 = elementIdx[param_9]（邻格元素），bVar12 = 邻格元素 pressure state & 3
    let cell_elem = cells.element_idx.get(cell);
    let neighbor_elem = cells.element_idx.get(neighbor);
    let neighbor_pd = match elements_table::get_element_pressure_data(neighbor_elem) {
        Some(p) => p,
        None => return 0.0,
    };
    let cell_pd = match elements_table::get_element_pressure_data(cell_elem) {
        Some(p) => p,
        None => return 0.0,
    };
    update_pressure_with(
        sim, cell, neighbor, cell_elem, neighbor_elem, &cell_pd, &neighbor_pd,
    )
}

/// #18 查表复用版（2026-09-06）：gas_flow 主循环在 gas_pair_ok 里已查过
/// 邻格（dst）的 pd、循环开头已查过源格（src）的 pd——传递进来免重查，
/// 每对省 2 次 get_element_pressure_data（AtomicPtr 加载 + 索引检查）。
/// 门控与计算顺序与 update_pressure 逐位一致。
pub(crate) fn update_pressure_with(
    sim: &mut crate::a_framework::sim_data::SimData,
    cell: usize,
    neighbor: usize,
    cell_elem: u16,
    neighbor_elem: u16,
    cell_pd: &crate::b_elements::element::ElementPressureData,
    neighbor_pd: &crate::b_elements::element::ElementPressureData,
) -> f32 {
    if sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return 0.0;
    }
    let cells = unsafe { &*sim.cells.ptr };
    if cell >= cells.mass.len() || neighbor >= cells.mass.len() {
        return 0.0;
    }
    let mut bvar12 = neighbor_pd.state & 3;
    let cell_state = cell_pd.state & 3; // 原版 param_7
    // 2026-08-03 修复：**液体**源格禁止向异元素非真空目标转移。
    // 原版压力只在同元素格或真空目标间转移（UpdateData 门控 L144628-144655）；
    // 液体格向异元素气体/液体转移时接收格元素不变，源格质量被"吸收"成目标元素
    // —— 实测 500kg 污染水 + 上方 0.5kg 污染氧 → 每子步 ~60kg 水变成污染氧，
    // 直至 250/250 质量均衡（用户观察到的"一次性分出自身一半质量"）。
    // 2026-08-04（气体模块）：真空(0)/气体(1)源格**允许**异元素 —— 气体↔真空压差
    // 由真空源格驱动（气体扩散入真空），原版 UpdatePressure 对 state<2 源格不门控。
    if cell_state == 2 && bvar12 != 0 && neighbor_elem != cell_elem {
        return 0.0;
    }
    // —— 质量（原版 L1469-1474：fVar15 = mass[cell]，fVar19 = mass[neighbor]）——
    let mass_cell = cells.mass.get(cell);
    let mass_neighbor = cells.mass.get(neighbor);
    // K1 修复（方案 A）：系数选择从"本格状态"判定改为"邻格状态"判定。
    // 原版 L1475-1477：param_7（驱动格状态）!= 1 时取 param_9（邻格）的 flow ——
    // 原版从真空/气体格驱动、液体为邻格，驱动格非气体时取**液体邻格**的 flow。
    // Rust 由液体格驱动（mod.rs 压力循环 L42108-42192 语义）、真空/气体为邻格，
    // 驱动格/邻格角色对调 → 邻格为气体（state==1）时用邻格（气体）flow，
    // 邻格为真空/液体/固体时用本格（液体）flow。修复前取真空 flow≈0 → 横向不蔓延。
    let flow_coeff = if bvar12 == 1 { neighbor_pd.flow } else { cell_pd.flow };
    // —— 转移量 + clamp（原版 L1478-1484）——
    let mut f18 = flow_coeff * (mass_cell - mass_neighbor);
    if mass_cell * 0.125 <= f18 {
        f18 = mass_cell * 0.125;
    }
    if f18 <= mass_neighbor * -0.125 {
        f18 = mass_neighbor * -0.125;
    }
    // —— 方向：来源格/接收格/新元素（原版 L1485-1496）——
    // uVar13 = 接收格新元素，uVar14 = 接收格当前元素（void 判断）
    let mut uvar13 = cell_elem;
    let mut uvar14 = neighbor_elem;
    let (src_cell, dst_cell, src_mass) = if f18 < 0.0 {
        // 邻格 → 本格：来源=邻格，接收=本格
        bvar12 = cell_state; // 原版 L1488：bVar12 = param_7（接收格=本格）
        uvar13 = neighbor_elem; // 原版 L1489：uVar13 = 邻格元素
        uvar14 = cell_elem; // 原版 L1490：uVar14 = 本格元素
        (neighbor, cell, mass_neighbor)
    } else {
        // 本格 → 邻格：来源=本格，接收=邻格
        (cell, neighbor, mass_cell)
    };
    // —— 实际转移量 = min(|f18|, 来源格质量)（原版 L1497-1500）——
    let transfer = f18.abs().min(src_mass);
    if transfer <= 0.0 {
        return f18; // 无质量转移（避免 0/0 NaN；原版此场景为边界，质量/病菌转移量均 0）
    }
    // —— 病菌：来源格病菌数从 cells 读（原版 L1501-1513）——
    let src_disease_count = cells.disease_count.get(src_cell);
    let ratio = (transfer / src_mass).clamp(0.0, 1.0);
    let disease_to_transfer = (ratio * src_disease_count as f32) as i32;
    // —— 来源格病菌 idx / 温度（原版 L1504-1518：从 updatedCells 读）——
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if src_cell >= updated.mass.len() || dst_cell >= updated.mass.len() {
        return f18;
    }
    let src_disease_idx = updated.disease_idx.get(src_cell);
    let src_temp = updated.temperature.get(src_cell);
    // —— 接收格质量/温度/病菌（原版 L1519-1556，复用质量加权混合）——
    add_mass_and_update_temperature(
        updated,
        dst_cell,
        transfer,
        src_temp,
        src_disease_idx,
        disease_to_transfer,
    );
    // —— 来源格质量减少（原版 L1559-1567，clamp >= 0）——
    let src_mass_new = (updated.mass.get(src_cell) - transfer).max(0.0);
    updated.mass.set(src_cell, src_mass_new);
    // —— 来源格病菌减少（原版 L1568-1580）——
    modify_disease_count(updated, src_cell, -disease_to_transfer);
    // —— 元素段（原版 L1581-1608）：仅接收格非真空才产生事件 ——
    if bvar12 != 0 {
        return f18; // 接收格已是气体/液体/固体：不改变元素、不产生事件（原版 L1581-1583）
    }
    if uvar14 == sim.void_element_idx {
        // 接收格原为 void → 质量/温度清零 + 清病菌（原版 L1584-1599）
        updated.mass.set(dst_cell, 0.0);
        updated.temperature.set(dst_cell, 0.0);
        clear_disease(updated, dst_cell);
        return f18;
    }
    // 接收格元素改为来源元素 + 事件（原版 L1600-1607）
    updated.element_idx.set(dst_cell, uvar13);
    push_substance_change(sim, dst_cell);
    f18
}

// ===== 任务 8：PostProcessCell + DoDensityDisplacement + 挤压位移 =====
// 对照源码 05_liquid_flow.c L2320-2867（PostProcessCell）、L1755-1840（DoDensityDisplacement）、
// 11_msvcrt_ignored.c L34035（DisplaceGas）、L34136（DisplaceLiquid）、
// L34206（DisplaceLiquidSimple）、L34357（DoDisplacement）。
//
// 用户观察的两条挤压链路（设计文档 3.4 节）：
// - 液体被固体挤压 → 质量**平均分配**给上下左右可容纳格（异类液体也接收）→ displace_liquid
// - 气体被固体挤压 → **单方向**转移全部质量（方向由 tickCount 旋转候选序驱动）→ displace_gas
// ⚠️ 事件产生必须完整（push_substance_change）：挤压位移改变格子内容，C# 画面靠事件更新。

/// 原版 LCG 伪随机数（L1786/L2474/L2746）：`seed = seed * 0x343fd + 0x269ec3`，
/// 返回 `((seed >> 16) & 0x7fff) * 3.051851e-05`，∈ [0, 1)。
fn next_random(sim: &mut SimData) -> f32 {
    // T4 路由：区域 RNG 存在（d3 并行）→ 用区域持久种子；无 → sim 全局（串行不变）。
    if let Some(rng) = crate::c2_physics::region_rng::region_rng_ptr() {
        let rng = unsafe { &mut *rng };
        rng.random_seed = rng.random_seed.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        ((rng.random_seed >> 16) & 0x7fff) as f32 * 3.051851e-05
    } else {
        sim.random_seed = sim.random_seed.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        ((sim.random_seed >> 16) & 0x7fff) as f32 * 3.051851e-05
    }
}

/// CellSOA::ClearCell（原版 11_msvcrt_ignored.c L23677-23723）。
/// 清空格子：元素 → vacuumElementIdx、质量/温度清零、病菌四字段清除。
pub(crate) fn clear_cell(sim: &SimData, cells: &mut CellSOA, cell: usize) {
    cells.element_idx.set(cell, sim.vacuum_element_idx);
    cells.mass.set(cell, 0.0);
    cells.temperature.set(cell, 0.0);
    clear_disease(cells, cell);
}

/// FastCalculateCombinedTemperature（原版 DoDensityDisplacement L1820 调用）——
/// 两格同元素液体质量加权平均温度。
pub(crate) fn fast_combined_temperature(mass_a: f32, temp_a: f32, mass_b: f32, temp_b: f32) -> f32 {
    (mass_a * temp_a + mass_b * temp_b) / (mass_a + mass_b)
}

/// CalculateCombinedTemperature（原版 SimDLL_Source.c L140988，ElementEmitter 用）：
/// 加权平均后**钳制到 [min(temp_a,temp_b), max(temp_a,temp_b)]**；总质量<=0 返回 0。
/// 此前 element_emitter 误用 fast_combined_temperature（无钳制）→ 合并温度可能越界。
pub(crate) fn calculate_combined_temperature(
    mass_a: f32,
    temp_a: f32,
    mass_b: f32,
    temp_b: f32,
) -> f32 {
    if mass_a + mass_b <= 0.0 {
        return 0.0;
    }
    let avg = (mass_a * temp_a + mass_b * temp_b) / (mass_a + mass_b);
    avg.clamp(temp_a.min(temp_b), temp_a.max(temp_b))
}

/// 清空格子 flow 四分量（原版 L2455-2458/L2552-2555，flow 为 Vector4f 数组）。
fn clear_flow(sim: &mut SimData, cell: usize) {
    if !sim.flow.ptr.is_null() {
        let total = sim.width as usize * sim.height as usize;
        let flow = unsafe { std::slice::from_raw_parts_mut(sim.flow.ptr, total) };
        if cell < total {
            flow[cell] = Vector4f::default();
        }
    }
}

/// Evaporate（原版 05_liquid_flow.c L2194-2208）：清空格子 + substance 事件。
/// 调用 SimData::ClearCell（11_msvcrt L23677：元素→真空、质量/温度清零、病菌四字段）。
/// 2026-08-02 实现：此前留桩 → 表层微克级残水（≤0.01kg / 气体 <1e-9）永不清理。
pub(crate) fn do_evaporate(sim: &mut SimData, cell: usize) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if cell >= updated.element_idx.len() {
        return;
    }
    clear_cell(sim, updated, cell);
    push_substance_change(sim, cell);
}

/// SimEvents::SpawnFX（原版 L149835+）：sim→game 坐标转换 + 推 spawn_fx_info。
/// SpawnFXInfo 12B = {cell_idx: i32, fx_id: i32, rotation: f32}（L7251-7253）。
/// fx_id = 元素 sublimateFX；rotation = 方向角（镜像原版常量，C# 渲染端解释）。
pub(crate) fn push_spawn_fx(sim: &mut SimData, sim_cell: usize, fx_id: i32, rotation: f32) {
    if sim.sim_events.ptr.is_null() {
        return;
    }
    let width = sim.width;
    let game_w = width - 2;
    let internal = sim_cell as i32;
    let game_cell = (internal % width - 1) + (internal / width - 1) * game_w;
    if game_cell >= 0 && game_cell < sim.num_game_cells {
        crate::c2_physics::region_events::emit_spawn_fx(
            sim,
            SpawnFXInfo {
                cell_idx: game_cell,
                fx_id,
                rotation,
            },
        );
    }
}

/// 液体 offgas 产气辅助（原版 L149810 匿名 lambda 的可观测语义）：
/// 源格扣 amount 质量（病菌按比例），目标格获得 sublimateIndex 气体
/// （真空/异种 → 换元素 + substance 事件；同种 → 质量合并），温度加权；
/// merge_disease=true 时按 add_mass_and_update_temperature 合并病菌，否则只合质量/温度。
/// src_cell == dst_cell 时等价于"就地转气体"（耗尽路径）。
fn emit_sublimate_gas(
    sim: &mut SimData,
    src_cell: usize,
    dst_cell: usize,
    amount: f32,
    merge_disease: bool,
    sublimate_index: u16,
) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if src_cell >= updated.mass.len() || dst_cell >= updated.mass.len() {
        return;
    }
    // 源格扣质量/病菌（比例），病菌 idx 先读
    let src_mass = updated.mass.get(src_cell);
    let src_disease_idx = updated.disease_idx.get(src_cell);
    let src_disease_count = updated.disease_count.get(src_cell);
    let d = if src_mass > 0.0 {
        (src_disease_count as f32 * (amount / src_mass)) as i32
    } else {
        0
    };
    updated.mass.set(src_cell, src_mass - amount);
    let remain = updated.disease_count.get(src_cell) - d;
    if remain < 1 {
        clear_disease(updated, src_cell);
    } else {
        updated.disease_count.set(src_cell, remain);
    }
    // 目标格：真空/异种 → 换元素 + 事件
    let dst_elem = updated.element_idx.get(dst_cell);
    if dst_elem != sublimate_index {
        updated.element_idx.set(dst_cell, sublimate_index);
        push_substance_change(sim, dst_cell);
    }
    let src_temp = updated.temperature.get(src_cell);
    if merge_disease {
        add_mass_and_update_temperature(updated, dst_cell, amount, src_temp, src_disease_idx, d);
    } else {
        let cur_mass = updated.mass.get(dst_cell);
        let new_mass = cur_mass + amount;
        let cur_temp = updated.temperature.get(dst_cell);
        let avg = (cur_mass * cur_temp + src_temp * amount) / new_mass;
        let mixed = avg.clamp(cur_temp.min(src_temp), cur_temp.max(src_temp));
        updated.mass.set(dst_cell, new_mass);
        updated.temperature.set(dst_cell, mixed);
    }
}

/// DoSublimation（原版 SimDLL_Source.c L148591-148880）——自然固体格升华。
/// 由真空/气体格驱动（PostProcessCell 真空/气体分支末尾调用）：检查四邻固体砖
/// （上/右/左/下），概率门通过后把砖块质量以升华产物气体形式转移到调用格。
fn do_sublimation(sim: &mut SimData, cell: usize) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim.width as usize;
    let height = sim.height as usize;
    if width < 3 || height < 3 || cell < width {
        return;
    }
    let total = width * height;
    // 入口（L148633）：调用格质量 < 1.8（mass <= 1.8 && mass != 1.8）
    let entry_mass = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        if cell >= updated.mass.len() {
            return;
        }
        updated.mass.get(cell)
    };
    if entry_mass >= 1.8 {
        return;
    }
    // 入口快照（L148650 sVar4 读一次；param_4->state 用入口 ppd）
    let (entry_elem, entry_state) = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        let elem = updated.element_idx.get(cell);
        match elements_table::get_element_post_process_data(elem) {
            Some(p) => (elem, p.state & 3),
            None => return,
        }
    };
    let mut bvar16 = false;
    // 四邻顺序 [cell+width, cell+1, cell-1, cell-width]（L148649-148652）
    let neighbors = [
        cell + width,
        cell + 1,
        cell.wrapping_sub(1),
        cell.wrapping_sub(width),
    ];
    for &nb in &neighbors {
        if nb >= total {
            continue;
        }
        // 邻格：固体（state&3==3）且 sublimateIndex 有效才消耗 randomSeed（L148656-148662）
        let nb_ppd = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            if nb >= updated.element_idx.len() {
                continue;
            }
            match elements_table::get_element_post_process_data(updated.element_idx.get(nb)) {
                Some(p) => p,
                None => continue,
            }
        };
        if (nb_ppd.state & 3) != 3 || nb_ppd.sublimate_index == 0xffff {
            continue;
        }
        let rnd = next_random(sim);
        if rnd > nb_ppd.sublimate_probability {
            continue; // 原版 random <= probability（fVar18 < p || fVar18 == p）
        }
        let f3 = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            updated.mass.get(nb)
        };
        let f18 = nb_ppd.sublimate_rate * 0.2;
        let mut f21 = f18.min(f3);
        if f18 <= f3 - f21 {
            // —— 部分转移分支（L148666-148791）——
            let (disease_nb, temp_nb, disease_idx_nb) = {
                let updated = unsafe { &*sim.updated_cells.ptr };
                (
                    updated.disease_count.get(nb),
                    updated.temperature.get(nb),
                    updated.disease_idx.get(nb),
                )
            };
            let mut i15 = ((f21 / f3) * disease_nb as f32) as i32;
            if entry_elem == nb_ppd.sublimate_index {
                // 同种气体：1.8 上限缩放（L148681-148702）
                let mut f20 = f21 * nb_ppd.sublimate_efficiency;
                let room = {
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    1.8 - updated.mass.get(cell)
                };
                if room < f20 {
                    f21 = (room / f20) * f21;
                    i15 = ((f21 / f3) * disease_nb as f32) as i32;
                    f20 = room;
                }
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                add_mass_and_update_temperature(updated, cell, f20, temp_nb, disease_idx_nb, i15);
                bvar16 = true;
            } else {
                // 异种/真空目标（L148705-148758）
                let mut set_element = false;
                if entry_state == 0 {
                    set_element = true;
                } else {
                    let cur_elem = {
                        let updated = unsafe { &*sim.updated_cells.ptr };
                        updated.element_idx.get(cell)
                    };
                    if displace_gas(sim, cell, cur_elem) {
                        set_element = true;
                    }
                }
                if !set_element && !bvar16 {
                    continue; // 无先前成功：跳过该邻格（L148758）
                }
                if set_element {
                    // 换元素段（L148719-148753）
                    let updated = unsafe { &mut *sim.updated_cells.ptr };
                    updated.element_idx.set(cell, nb_ppd.sublimate_index);
                    updated.mass.set(
                        cell,
                        updated.mass.get(cell) + f21 * nb_ppd.sublimate_efficiency,
                    );
                    updated.temperature.set(cell, temp_nb);
                    updated.disease_idx.set(cell, disease_idx_nb);
                    updated.disease_count.set(cell, i15);
                    push_substance_change(sim, cell);
                    bvar16 = true;
                }
                // 怪癖（L148758 fall-through）：displace 失败但 bvar16==true →
                // 仍执行下方扣减段，目标格不产气（逐行镜像）。
            }
            // —— 共同扣减段（L148761-148791）——
            {
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                updated.mass.set(nb, updated.mass.get(nb) - f21);
                let remain = updated.disease_count.get(nb) - i15;
                if remain < 1 {
                    clear_disease(updated, nb);
                } else {
                    updated.disease_count.set(nb, remain);
                }
            }
            if !sim.accumulated_flow.ptr.is_null() {
                let acc = unsafe { std::slice::from_raw_parts_mut(sim.accumulated_flow.ptr, total) };
                acc[nb] += f21;
            }
            let dir = if nb == cell + width {
                180.0
            } else if nb == cell + 1 {
                90.0
            } else if nb == cell.wrapping_sub(1) {
                -90.0
            } else {
                0.0
            };
            push_spawn_fx(sim, nb, nb_ppd.sublimate_fx, dir);
            {
                let updated = unsafe { &*sim.updated_cells.ptr };
                if updated.mass.get(nb) <= 1e-09 {
                    do_evaporate(sim, nb);
                }
            }
        } else {
            // —— 整格消耗分支（L148848-148866）：砖块就地转气体 ——
            {
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                updated.element_idx.set(nb, nb_ppd.sublimate_index);
                updated.mass.set(nb, f3 * nb_ppd.sublimate_efficiency);
            }
            push_substance_change(sim, nb);
            push_spawn_fx(sim, nb, nb_ppd.sublimate_fx, 0.0);
            bvar16 = true;
        }
    }
}

/// DoUnstableCheck 分派（原版 L2537-2545）：savedOptions&1 → WithDiagonals，
/// 否则 Basic。当前 C# 从不设置 ENABLE_DIAGONAL_FALLING_SAND → 恒走 Basic。
fn do_unstable_check(sim: &mut SimData, cell: usize) {
    if (sim.saved_options & 1) != 0 {
        do_unstable_check_with_diagonals(sim, cell);
    } else {
        do_unstable_check_basic(sim, cell);
    }
}

/// SimData::GetStableTicksRemaining（SimDLL_Source.c L123214-123250）。
///
/// timers[cell] 低 5 位稳定 tick：
/// - == 0x1F（"刚变化"标记）：LCG 重掷 `3 + (seed>>16 & 0x7fff)×9.155553e-05`
///   ∈ [3,6)，写回低 5 位，返回新值（3-5 tick 稳定期，塌方延迟）；
/// - != 0：减 1 写回，返回递减后值；
/// - == 0：返回 0（稳定，可塌方）。
pub(crate) fn get_stable_ticks_remaining(sim: &mut SimData, cell: usize) -> u8 {
    if sim.timers.ptr.is_null() {
        return 0;
    }
    let timers_ptr = sim.timers.ptr as *mut u8;
    let cur = unsafe { std::ptr::read(timers_ptr.add(cell)) };
    let low = cur & 0x1f;
    if low == 0x1f {
        // next_random 归一化 [0,1) ×3 → [0,3)，截断 +3 → {3,4,5}（等价原版算式）
        let new_low = ((next_random(sim) * 3.0) as u8).saturating_add(3) & 0x1f;
        let new_byte = (cur & !0x1f) | new_low;
        unsafe { std::ptr::write(timers_ptr.add(cell), new_byte); }
        return new_low;
    }
    if low != 0 {
        let new_low = low - 1;
        let new_byte = (cur & !0x1f) | new_low;
        unsafe { std::ptr::write(timers_ptr.add(cell), new_byte); }
        return new_low;
    }
    0
}

/// 推 UnstableCellInfo（20B）——塌方事件（原版 DoUnstableCheckBasic L1878-1903 /
/// WithDiagonals L2061-2095 内联 push）。
///
/// src_cell：元素/质量/病菌/温度来源（塌方块）；pos_cell：事件 game 坐标格
/// （Basic=方块自身；WithDiagonals=对角候选格）；weight：转移比例
/// （Basic=1.0 全量后 Evaporate；对角=0.5，源格保留余量）。
/// UnstableCellInfo 布局 {cellIdx i32, elemIdx u16, fallingInfo u8, diseaseIdx u8,
/// mass f32, temperature f32, diseaseCount i32} = 20B（00_types_reference.c L12524）。
fn push_unstable_cell_info(sim: &mut SimData, src_cell: usize, pos_cell: usize, weight: f32) {
    if sim.sim_events.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim.width as i32;
    let game_w = width - 2;
    let internal = pos_cell as i32;
    let game_cell = (internal % width - 1) + (internal / width - 1) * game_w;
    if game_cell < 0 || game_cell >= sim.num_game_cells {
        return;
    }
    let updated = unsafe { &*sim.updated_cells.ptr };
    if src_cell >= updated.element_idx.len() || src_cell >= updated.mass.len() {
        return;
    }
    crate::c2_physics::region_events::emit_unstable_cell(
        sim,
        UnstableCellInfo {
            cell_idx: game_cell,
            elem_idx: updated.element_idx.get(src_cell),
            falling_info: 0,
            disease_idx: updated.disease_idx.get(src_cell),
            mass: updated.mass.get(src_cell) * weight,
            temperature: updated.temperature.get(src_cell),
            disease_count: (updated.disease_count.get(src_cell) as f32 * weight) as i32,
        },
    );
}

/// DoUnstableCheckBasic（原版 05_liquid_flow.c L1841-1937）——不稳定方块塌方（基本变体）。
///
/// 读 updatedCells：下方格（cell−width）元素为气体/液体/真空（state&3<3）且
/// 非 void 且 `properties&4==0`（非 SolidImpermeable）且稳定 tick 耗尽
/// （GetStableTicksRemaining==0）→ 塌方：
/// - 非 headless：推 UnstableCellInfo（fallingInfo=0，C# Spawn 下落实体）
///   + Evaporate（清格 + substance 事件）；
/// - headless：HeadlessUnstableFallSimpleSwap（逐格向下交换，世界生成用）。
/// 否则 `timers[cell] |= 0x1F`（标记"刚变化"）。
pub(crate) fn do_unstable_check_basic(sim: &mut SimData, cell: usize) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim.width as usize;
    if width == 0 || cell < width {
        return; // 下方索引下溢（原版越界断言）
    }
    let below = cell - width;
    let (below_elem, below_props) = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        if below >= updated.element_idx.len() {
            return;
        }
        (updated.element_idx.get(below), updated.properties.get(below))
    };
    let below_state = match elements_table::get_element_post_process_data(below_elem) {
        Some(p) => p.state & 3,
        None => return,
    };
    // 原版 L1854-1875：下方为气体/液体/真空且非 void 且非 SolidImpermeable
    if below_state < 3 && below_elem != sim.void_element_idx && (below_props & 4) == 0 {
        let ticks = get_stable_ticks_remaining(sim, cell);
        if ticks == 0 {
            if sim.headless {
                headless_unstable_fall_simple_swap(sim, cell);
            } else {
                push_unstable_cell_info(sim, cell, cell, 1.0);
                do_evaporate(sim, cell);
            }
        }
        return;
    }
    // 原版 L1910-1913：不塌方 → timers[cell] |= 0x1F
    if !sim.timers.ptr.is_null() {
        let timers_ptr = sim.timers.ptr as *mut u8;
        let cur = unsafe { std::ptr::read(timers_ptr.add(cell)) };
        unsafe { std::ptr::write(timers_ptr.add(cell), cur | 0x1F) };
    }
}

/// HeadlessUnstableFallSimpleSwap（原版 L2209-2322）——headless（世界生成）下落。
///
/// 从方块下方第一格（cell−width）开始向下走：若当前格为固体 →
/// ChangeSubstance(方块当前格) 返回；否则交换当前格 ↔ 方块当前格
/// （7 字段，固体下移一格）+ substance 事件 + timers|=0x1F，继续向下。
pub(crate) fn headless_unstable_fall_simple_swap(sim: &mut SimData, cell: usize) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim.width as isize;
    if width <= 0 {
        return;
    }
    let total = sim.width as usize * sim.height as usize;
    let mut cur = cell as isize - width; // 方块下方第一格
    loop {
        if cur < 0 || cur as usize >= total {
            return;
        }
        let below_cell = cur as usize;
        let elem = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            if below_cell >= updated.element_idx.len() {
                return;
            }
            updated.element_idx.get(below_cell)
        };
        let is_solid = match elements_table::get_element_post_process_data(elem) {
            Some(p) => (p.state & 3) == 3,
            None => true, // 表缺失防御：按固体处理（不再下落）
        };
        if is_solid {
            // 原版 L2223-2227：ChangeSubstance(方块当前格)（iVar11 + iVar13）
            push_substance_change(sim, (cur + width) as usize);
            return;
        }
        let solid_cell = (cur + width) as usize;
        // 交换下方格 ↔ 方块当前格（7 字段，原版 L2228-2283）
        {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            swap_cells(updated, below_cell, solid_cell);
        }
        // 事件 + timers（原版 L2284-2315）
        push_substance_change(sim, solid_cell);
        cur -= width;
    }
}

/// DoUnstableCheckWithDiagonals（原版 L1937-2208）——斜向塌方变体。
///
/// 休眠选项：由 `savedOptions & 1`（ENABLE_DIAGONAL_FALLING_SAND）门控，
/// 当前 C# 从不调用 SetSavedOptionValue → 实际恒走 DoUnstableCheckBasic。
/// 按原版结构实现三方向扫描：
/// - 正下（offset=−width、权重 1.0、阈值 0、正交 false）；
/// - 左下（offset=−width−1、权重 0.5、阈值 1.0×minHorizontalFlow、正交 true）；
/// - 右下（offset=1−width、权重 0.5、阈值 1.0×minHorizontalFlow、正交 true）。
/// 门控 `(mass − 阈值×minHorizontalFlow)×权重 > 0`；候选格须 gas/liquid/vacuum、
/// 非 void、props&4==0；正交标志时额外要求候选下方格（candidate+width）非固体
/// 且 props&4==0。稳定 tick 耗尽 → 推事件（坐标=候选格、mass/disease×权重）+
/// 源格扣减（headless 走搬移+继续下落）。
/// 注：原版栈布局逆向的方向表存在少量不确定性（休眠路径，不影响当前游戏行为）。
pub(crate) fn do_unstable_check_with_diagonals(sim: &mut SimData, cell: usize) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim.width as i32;
    if width <= 0 || (cell as i32) < width {
        return;
    }
    let directions: [(i32, f32, f32, bool); 3] = [
        (-width, 1.0, 0.0, false),
        (-width - 1, 0.5, 1.0, true),
        (1 - width, 0.5, 1.0, true),
    ];
    // 源格快照（原版 L2005-2012：fVar4=mass、iVar7=diseaseCount）
    let (src_mass, src_count, src_elem) = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        if cell >= updated.mass.len() || cell >= updated.disease_count.len() {
            return;
        }
        (
            updated.mass.get(cell),
            updated.disease_count.get(cell),
            updated.element_idx.get(cell),
        )
    };
    let min_horizontal_flow = match elements_table::get_element_post_process_data(src_elem) {
        Some(p) => p.min_horizontal_flow,
        None => return,
    };
    let mut bvar15 = false;
    let mut bvar14 = false;
    let mut uvar16: u8 = 0xff;
    for &(offset, weight, threshold, orthogonal) in &directions {
        let candidate = (cell as i32 + offset) as usize;
        // 门控（原版 L2013-2016）：(mass − threshold×minHorizontalFlow)×weight > 0
        if (src_mass - threshold * min_horizontal_flow) * weight <= 0.0 {
            continue;
        }
        // 候选格数据（updatedCells，原版 L2018-2045）
        let (below_cand_state, below_cand_props) = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            if candidate >= updated.element_idx.len() || candidate >= updated.properties.len() {
                continue;
            }
            let cand_elem = updated.element_idx.get(candidate);
            let cand_state = match elements_table::get_element_post_process_data(cand_elem) {
                Some(p) => p.state & 3,
                None => continue,
            };
            let cand_props = updated.properties.get(candidate);
            let below_cand = (candidate as i32 + width) as usize;
            if below_cand >= updated.element_idx.len() || below_cand >= updated.properties.len() {
                continue;
            }
            let below_cand_elem = updated.element_idx.get(below_cand);
            let below_cand_state =
                match elements_table::get_element_post_process_data(below_cand_elem) {
                    Some(p) => p.state & 3,
                    None => continue,
                };
            let below_cand_props = updated.properties.get(below_cand);
            if cand_state >= 3 || cand_elem == sim.void_element_idx || (cand_props & 4) != 0 {
                continue;
            }
            (below_cand_state, below_cand_props)
        };
        // 正交标志：候选下方格非固体且 props&4==0（原版 L2167-2170）
        if orthogonal && (below_cand_state == 3 || (below_cand_props & 4) != 0) {
            continue;
        }
        // 塌方（原版 LAB_18004d432）：稳定 tick 每调用只掷一次
        bvar15 = true;
        if !bvar14 {
            bvar14 = true;
            uvar16 = get_stable_ticks_remaining(sim, cell);
        }
        if uvar16 != 0 {
            continue;
        }
        // timers[cell] |= 0x1F（原版 L2057-2059）
        if !sim.timers.ptr.is_null() {
            let timers_ptr = sim.timers.ptr as *mut u8;
            let cur = unsafe { std::ptr::read(timers_ptr.add(cell)) };
            unsafe { std::ptr::write(timers_ptr.add(cell), cur | 0x1F) };
        }
        if !sim.headless {
            // 非 headless：推事件（坐标=候选格、mass/disease×**原始**质量）+ 源格扣减
            push_unstable_cell_info(sim, cell, candidate, weight);
            {
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                let mass_new = (updated.mass.get(cell) - src_mass * weight).max(0.0);
                updated.mass.set(cell, mass_new);
                let remain = updated.disease_count.get(cell) - (src_count as f32 * weight) as i32;
                if remain < 1 {
                    clear_disease(updated, cell);
                } else {
                    updated.disease_count.set(cell, remain);
                }
            }
            push_substance_change(sim, cell);
            let mass_now = {
                let updated = unsafe { &*sim.updated_cells.ptr };
                updated.mass.get(cell)
            };
            if mass_now <= 0.0 {
                do_evaporate(sim, cell);
            }
        } else {
            // headless：候选格置换后搬入 + 源格按**当前**质量扣减 + 继续下落
            headless_diagonal_fall(sim, cell, candidate, weight);
        }
    }
    if !bvar15 {
        // 原版 L2187-2191：无方向通过 → timers[cell] |= 0x1F
        if !sim.timers.ptr.is_null() {
            let timers_ptr = sim.timers.ptr as *mut u8;
            let cur = unsafe { std::ptr::read(timers_ptr.add(cell)) };
            unsafe { std::ptr::write(timers_ptr.add(cell), cur | 0x1F) };
        }
    }
}

/// WithDiagonals headless 分支（原版 L2103-2180）：候选格
/// DisplaceGas/DisplaceLiquid 腾位后，把源格元素/质量/温度/病菌搬入候选格，
/// 源格按权重扣减，再 HeadlessUnstableFallSimpleSwap(candidate)。
fn headless_diagonal_fall(sim: &mut SimData, cell: usize, candidate: usize, weight: f32) {
    let cand_state = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        let e = updated.element_idx.get(candidate);
        match elements_table::get_element_post_process_data(e) {
            Some(p) => p.state & 3,
            None => return,
        }
    };
    let cand_elem = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        updated.element_idx.get(candidate)
    };
    if cand_state == 1 {
        displace_gas(sim, candidate, cand_elem);
    } else if cand_state == 2 {
        displace_liquid(sim, candidate, cand_elem);
    }
    let (cur_mass, cur_count, cur_elem, cur_temp, cur_d_idx) = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        (
            updated.mass.get(cell),
            updated.disease_count.get(cell),
            updated.element_idx.get(cell),
            updated.temperature.get(cell),
            updated.disease_idx.get(cell),
        )
    };
    let move_mass = cur_mass * weight;
    let move_count = (cur_count as f32 * weight) as i32;
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.element_idx.set(candidate, cur_elem);
        updated.mass.set(candidate, move_mass);
        updated.temperature.set(candidate, cur_temp);
        updated.disease_idx.set(candidate, cur_d_idx);
        updated.disease_count.set(candidate, move_count);
        let src_new = (updated.mass.get(cell) - move_mass).max(0.0);
        updated.mass.set(cell, src_new);
        let remain = updated.disease_count.get(cell) - move_count;
        if remain < 1 {
            clear_disease(updated, cell);
        } else {
            updated.disease_count.set(cell, remain);
        }
    }
    push_substance_change(sim, candidate);
    push_substance_change(sim, cell);
    let mass_now = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        updated.mass.get(cell)
    };
    if mass_now <= 1.1754944e-38 {
        do_evaporate(sim, cell);
    }
    headless_unstable_fall_simple_swap(sim, candidate);
}

/// DoDensityDisplacement — 密度分层（原版 05_liquid_flow.c L1755-1840）。
///
/// 对照原版逐行：
/// 1. 当前格元素状态 != 下方格元素状态 → false（不同物质不相邻置换，L1783-1785）。
/// 2. 随机门：`next_random() <= param_5` → false（L1786-1789）。
///    液体调用方传 0.3（70% 通过）、气体传 0.99（1% 通过）；测试可传 0.0 强制通过。
/// 3. 下方格更轻（below.molar_mass <= cell.molar_mass 且不等）→ 当前格更重 → 交换下沉
///    （L1791-1797）+ 2 条事件。
/// 4. 同元素分支（L1798-1833）：元素不同 → false；mass[below] <= mass[cell] → false；
///    temp[below] <= temp[cell] → false；非液体（气体）→ 直接交换；
///    液体 → 温度质量加权合并（FastCalculateCombinedTemperature），两格同温。
///
/// 返回 true 表示发生了交换/合并（调用方不再做后续处理，原版 L2560-2562）。
/// 元素表缺失或索引越界时安全返回 false。
pub fn do_density_displacement(sim: &mut SimData, cell: usize, param_5: f32) -> bool {
    if sim.updated_cells.ptr.is_null() {
        return false;
    }
    let width = sim.width as usize;
    if width == 0 || cell < width {
        return false; // 防止下方格索引下溢
    }
    let below = cell - width;
    let (cell_elem, below_elem) = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        (updated.element_idx.get(cell), updated.element_idx.get(below))
    };
    // 原版 L1781-1785：param_4->state != 下方格元素 Element.state → false。
    // ppd.state 与 Element.state 同源（03_elements.c L296），故两格都用 ppd.state 比较。
    let (cell_ppd, below_ppd) = match (
        elements_table::get_element_post_process_data(cell_elem),
        elements_table::get_element_post_process_data(below_elem),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    if cell_ppd.state != below_ppd.state {
        return false;
    }
    // 原版 L1786-1789：随机门
    if next_random(sim) <= param_5 {
        return false;
    }
    // 原版 L1791-1797：当前格更重 → 交换下沉
    if below_ppd.molar_mass <= cell_ppd.molar_mass && cell_ppd.molar_mass != below_ppd.molar_mass {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        swap_cells(updated, cell, below);
        push_substance_change(sim, cell);
        push_substance_change(sim, below);
        return true;
    }
    // —— 同元素分支（原版 L1798-1833）——
    if cell_elem != below_elem {
        return false;
    }
    let (mass_below, mass_cell, temp_below, temp_cell) = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        (
            updated.mass.get(below),
            updated.mass.get(cell),
            updated.temperature.get(below),
            updated.temperature.get(cell),
        )
    };
    // 原版分支 B（SimDLL_Source.c L147862-147900，CellSOA 布局 L11008-11020：
    // 0x28=temperature、0x48=mass）：唯一门控 = **temp[below] > temp[cell]**
    // （下方更暖才处理），**无 mass 门控**。气体 → 交换（暖上升）；液体 → 合并温度。
    // 2026-08-04 审查纠正：基线自带的是 mass 检查（多余，误读 0x28/0x48），
    // 温度检查才是原版语义——此前一度误删温度检查，已恢复。
    if temp_below <= temp_cell {
        return false;
    }
    if (cell_ppd.state & 3) != 2 {
        // 原版 L1812-1815：非液体（气体）→ 直接交换
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        swap_cells(updated, cell, below);
        return true;
    }
    // 原版 L1816-1831：液体 → 温度质量加权合并
    let combined = fast_combined_temperature(mass_cell, temp_cell, mass_below, temp_below);
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    updated.temperature.set(cell, combined);
    updated.temperature.set(below, combined);
    true
}

/// DoDisplacement（原版 11_msvcrt_ignored.c L34357-34419）—— 把源格**全部**内容
/// 移到目标格：质量/温度/病菌整体累加、目标格元素改为源格元素、源格 ClearCell，
/// 产生 2 条 substance 事件（源 + 目标）。DisplaceGas 与挤压链路共用。
pub(crate) fn do_displacement(sim: &mut SimData, src: usize, dst: usize) {
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    let (mass, temp, d_idx, d_count) = (
        updated.mass.get(src),
        updated.temperature.get(src),
        updated.disease_idx.get(src),
        updated.disease_count.get(src),
    );
    add_mass_and_update_temperature(updated, dst, mass, temp, d_idx, d_count);
    updated.element_idx.set(dst, updated.element_idx.get(src));
    clear_cell(sim, updated, src);
    push_substance_change(sim, src);
    push_substance_change(sim, dst);
}

/// DisplaceGas — 气体被挤压 → **单方向转移全部质量**（原版 11_msvcrt_ignored.c L34035-34129）。
///
/// 源格必须质量 > 0 且元素为气体（postProcessData.state & 3 == 1）。
/// 候选顺序（原版 L34068-34074）`[上, 左, 右, 下]`（本项目约定 +width=上），按 `tickCount` 旋转
/// （原版 `local_60[uVar4 + tickCount & 3]`）——这是"随机单方向"的确定性实现
/// （每帧 tickCount 递增，起始方向交替变化）。
///
/// 循环 1（主方向 L34075-34092）：候选元素 == 目标气体 或 == vacuum 且
/// `properties & 1 == 0` → DoDisplacement 全量转移，返回 true。
/// 循环 2（对角 L34093-34121）：候选为 左上/右上（按 tickCount 取一），
/// 候选元素 == 目标气体 且 同侧平格（updatedCells）非固体（liquidData.state & 3 != 3）
/// 且 `properties & 1 == 0` → DoDisplacement。
///
/// 返回 false 表示无方向可去（原版返回 false，调用方决定是否放弃）。
pub fn displace_gas(sim: &mut SimData, cell: usize, gas_elem: u16) -> bool {
    if sim.updated_cells.ptr.is_null() {
        return false;
    }
    let width = sim.width as usize;
    let updated = unsafe { &*sim.updated_cells.ptr };
    if cell >= updated.mass.len() || updated.mass.get(cell) <= 0.0 {
        return false;
    }
    // 原版 L34062-34067：源格元素必须为气体
    let cell_elem = updated.element_idx.get(cell);
    let is_gas = match elements_table::get_element_post_process_data(cell_elem) {
        Some(ppd) => (ppd.state & 3) == 1,
        None => false,
    };
    if !is_gas {
        return false;
    }
    let len = updated.element_idx.len();
    // 原版 L34068-34074：候选 = [上(+width), 左(-1), 右(+1), 下(-width)]
    //（原版 L136841-136844 local_60[0]=+width、[3]=-width；本项目约定 +width=上）
    let candidates = [cell + width, cell.wrapping_sub(1), cell + 1, cell.wrapping_sub(width)];
    let tick = sim.tick_count as usize;
    // 循环 1（L34075-34092）
    for i in 0..4 {
        let c = candidates[(tick + i) & 3];
        if c >= len {
            continue;
        }
        let e = updated.element_idx.get(c);
        let is_target = e == gas_elem || e == sim.vacuum_element_idx;
        if is_target && (updated.properties.get(c) & 1) == 0 {
            do_displacement(sim, cell, c);
            return true;
        }
    }
    // 循环 2（L34093-34121）：对角（+width 侧 = 上方）
    let diag = [cell + width - 1, cell + width + 1]; // 左上、右上
    let sides = [cell.wrapping_sub(1), cell + 1]; // 左、右
    for i in 0..2 {
        let dir = (tick + i) & 1;
        let c = diag[dir];
        if c >= len {
            continue;
        }
        if updated.element_idx.get(c) != gas_elem {
            continue;
        }
        // 同侧平格（updatedCells）非固体才允许对角移动（原版 L34102-34109）
        let side = sides[dir];
        let side_ok = match elements_table::get_element_liquid_data(updated.element_idx.get(side)) {
            Some(ld) => (ld.state & 3) != 3,
            None => false,
        };
        if side_ok && (updated.properties.get(c) & 1) == 0 {
            do_displacement(sim, cell, c);
            return true;
        }
    }
    false
}

/// DisplaceLiquidSimple（原版 11_msvcrt_ignored.c L34206-34352）——
/// 把源格液体质量**平均分配**给候选格。
///
/// 对照原版：
/// 1. 源格质量 >= 0.01（L34256）且候选非空。
/// 2. 收集候选：元素 == 目标液体 或 == vacuum，且 `properties & 2 == 0`
///    （非固体标志，L34262-34282）——**异类液体也接收**（原版检查的是元素
///    相等或真空，不是"同类才收"）。
/// 3. 每格 AddMassAndUpdateTemperature(mass/n) + 元素改为目标液体 + substance 事件
///    + timers |= 0x1F（L34309-34342）；病菌按 `(count + n - 1) / n` 分配。
/// 4. 全部派发后源格 ClearCell + 1 条事件（L34295-34297）。
///
/// 返回 true 表示派发完成。
fn displace_liquid_simple(sim: &mut SimData, cell: usize, elem: u16, candidates: &[usize]) -> bool {
    if sim.updated_cells.ptr.is_null() {
        return false;
    }
    let updated = unsafe { &*sim.updated_cells.ptr };
    if cell >= updated.mass.len() || candidates.is_empty() {
        return false;
    }
    let mass_cell = updated.mass.get(cell);
    if mass_cell < 0.01 {
        return false; // 原版 L34256
    }
    // 收集可容纳格（L34262-34282）
    let mut valid: Vec<usize> = Vec::new();
    for &c in candidates {
        if c >= updated.element_idx.len() {
            continue;
        }
        let e = updated.element_idx.get(c);
        let is_target = e == elem || e == sim.vacuum_element_idx;
        if is_target && (updated.properties.get(c) & 2) == 0 {
            valid.push(c);
        }
    }
    let n = valid.len();
    if n == 0 {
        return false;
    }
    let fvar4 = updated.mass.get(cell);
    let disease_count = updated.disease_count.get(cell);
    let disease_idx = updated.disease_idx.get(cell);
    let temp = updated.temperature.get(cell);
    let per = fvar4 / n as f32;
    // 原版 L34314：病菌按 `(count + n - 1) / n` 均分（整除，可能略超）
    let disease_per = (disease_count + n as i32 - 1) / n as i32;
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    for &c in &valid {
        add_mass_and_update_temperature(updated, c, per, temp, disease_idx, disease_per);
        updated.element_idx.set(c, elem);
        // 目标格元素类型变化必须产生事件（原版 L34322-34339）+ timers（push 内处理）
        push_substance_change(sim, c);
    }
    // 源格 ClearCell + 事件（原版 L34295-34297）
    clear_cell(sim, updated, cell);
    push_substance_change(sim, cell);
    true
}

/// DisplaceLiquid — 液体被固体/异类挤压 → 质量平均分配（原版 11_msvcrt_ignored.c L34136-34199）。
///
/// 对照原版：
/// 1. 源格质量 > 0 且元素为液体（postProcessData.state & 3 == 2，L34154-34161）。
/// 2. 先 DisplaceLiquidSimple 尝试 4 方向均分（右/左/下/上，L34162-34168）。
/// 3. 失败则按 tickCount 旋转顺序找气体邻居 → DisplaceGas 把气体挤走（L34170-34188），
///    成功后重试 DisplaceLiquidSimple。
///
/// `elem` 是被挤压液体的元素索引（原版 param_5，来自调用点读到的源格元素）。
pub fn displace_liquid(sim: &mut SimData, cell: usize, elem: u16) -> bool {
    if sim.updated_cells.ptr.is_null() {
        return false;
    }
    let width = sim.width as usize;
    let updated = unsafe { &*sim.updated_cells.ptr };
    if cell >= updated.mass.len() || updated.mass.get(cell) <= 0.0 {
        return false;
    }
    // 原版 L34156-34161：源格元素必须为液体
    let cell_elem = updated.element_idx.get(cell);
    let is_liquid = match elements_table::get_element_post_process_data(cell_elem) {
        Some(ppd) => (ppd.state & 3) == 2,
        None => false,
    };
    if !is_liquid {
        return false;
    }
    // 原版 L34162-34165：候选 = [右, 左, 下, 上]
    let candidates = [cell + 1, cell.wrapping_sub(1), cell + width, cell.wrapping_sub(width)];
    if displace_liquid_simple(sim, cell, elem, &candidates) {
        return true;
    }
    // 原版 L34170-34191：气体邻居置换后重试
    let len = updated.element_idx.len();
    let tick = sim.tick_count as usize;
    for i in 0..4 {
        let c = candidates[(tick + i) & 3];
        if c >= len {
            continue;
        }
        let e = updated.element_idx.get(c);
        let is_gas = match elements_table::get_element_post_process_data(e) {
            Some(ppd) => (ppd.state & 3) == 1,
            None => false,
        };
        if is_gas && displace_gas(sim, c, e) {
            return displace_liquid_simple(sim, cell, elem, &candidates);
        }
    }
    false
}

// ===== DoLiquidPressureDisplacement（原版 11_msvcrt_ignored.c L43219-43460）=====
// 液体↔液体压力位移：液体格挤压异元素液体邻格（左/右/下），把邻格内容推到
// beyond 格，再把本格质量的 12.5% 以本格元素灌入邻格。原版位于 UpdateData
// UpdateLiquid 之后、CopyFrom 之前的位移循环（L42480-42557）。
// 2026-08-06 补齐（此前缺失——run_pressure_task 只做液体→气体/真空质量转移，
// 不覆盖液体↔液体挤压）。

/// DisplaceLiquidDirectional（原版 L43219-43325）——把邻格（src_cell，被挤压的
/// 异元素液体）内容移到目标格（dst_cell）：
/// - dst 为气体 → DisplaceGas 挤走气体后 SwapCells(src, dst)（液体整体搬入）；
/// - dst 为真空 → DisplaceGas 因质量 0 失败 → 返回 false（原版怪癖，真空 beyond 挤不动）；
/// - dst 为液体 → **仅当 src 与 dst 同元素**才合并进 dst
///   （原版 L146461-146464：`updated[src] != updated[dst] → false`；
///   否则污染水小格会被挤进水格 → 质量变水消失）；
/// - dst 为固体 → false。
fn displace_liquid_directional(
    sim: &mut SimData,
    src_cell: usize,
    dst_cell: usize,
) -> bool {
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if src_cell >= updated.mass.len() || dst_cell >= updated.mass.len() {
        return false;
    }
    if updated.mass.get(src_cell) <= 0.0 {
        return false;
    }
    // 原版 L43230-43233：dst 必须可渗透（properties & 2 == 0）
    if updated.properties.get(dst_cell) & 2 != 0 {
        return false;
    }
    let dst_state = match elements_table::get_element_liquid_data(
        updated.element_idx.get(dst_cell),
    ) {
        Some(ld) => ld.state & 3,
        None => return false,
    };
    if dst_state < 2 {
        // dst 为气体(1)/真空(0)：DisplaceGas 把 dst 内容挤走（真空质量 0 → 失败）
        if !displace_gas(sim, dst_cell, updated.element_idx.get(dst_cell)) {
            return false;
        }
        // DisplaceGas 后 dst 已清空；SwapCells 把邻格液体整体搬入 dst
        swap_cells(updated, src_cell, dst_cell);
        push_substance_change(sim, src_cell);
        push_substance_change(sim, dst_cell);
        return true;
    }
    if dst_state != 2 {
        return false; // dst 为固体
    }
    // dst 为液体（异元素）：仅当邻格与 dst 同元素才合并（原版 L146461-146464）。
    // 旧实现误用邻格自身快照元素 → 污染水被挤进水格后质量变水消失。
    if updated.element_idx.get(src_cell) != updated.element_idx.get(dst_cell) {
        return false;
    }
    let (mass, temp, d_idx, d_count) = (
        updated.mass.get(src_cell),
        updated.temperature.get(src_cell),
        updated.disease_idx.get(src_cell),
        updated.disease_count.get(src_cell),
    );
    add_mass_and_update_temperature(updated, dst_cell, mass, temp, d_idx, d_count);
    clear_cell(sim, updated, src_cell);
    // 原版 L146481：合并路径只对 src（被清空的邻格）发 ChangeSubstance。
    push_substance_change(sim, src_cell);
    true
}

/// DoLiquidPressureDisplacement（原版 L43326-43460）——单次压力位移。
/// 返回实际位移量（调用方用于 flow 记账）；条件不满足返回 0。
///
/// 语义：源液体格（src_cell，元素 src_elem）挤压异元素液体邻格（dst_cell），
/// 邻格内容经 DisplaceLiquidDirectional 推到 beyond_cell；随后
/// displacement = (mass[src] − flow[src]四分量和) × 0.125 灌入邻格并改元素、
/// 温度/病菌按 12.5% 转移，源格扣减。
pub(crate) fn do_liquid_pressure_displacement(
    sim: &mut SimData,
    src_elem: u16,
    src_cell: usize,
    dst_cell: usize,
    beyond_cell: usize,
) -> f32 {
    if sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return 0.0;
    }
    let cells = unsafe { &*sim.cells.ptr };
    if src_cell >= cells.mass.len()
        || dst_cell >= cells.mass.len()
        || beyond_cell >= cells.mass.len()
    {
        return 0.0;
    }
    // 原版 L43339-43344：dst 可渗透（properties & 2 == 0）
    if cells.properties.get(dst_cell) & 2 != 0 {
        return 0.0;
    }
    let dst_elem_cells = cells.element_idx.get(dst_cell);
    // 原版 L43345-43346：dst 元素 == 源液体元素 → 跳过
    if dst_elem_cells == src_elem {
        return 0.0;
    }
    {
        let updated = unsafe { &*sim.updated_cells.ptr };
        // 原版 L43349-43351：updated[src] 仍为源液体（本帧变化后的当前元素）
        if updated.element_idx.get(src_cell) != src_elem {
            return 0.0;
        }
        // 原版 L43353-43354：updated[dst] == cells[dst]（快照一致）
        if updated.element_idx.get(dst_cell) != dst_elem_cells {
            return 0.0;
        }
        // 原版 L43357-43359：dst 必须是液体（pressure state & 3 == 2）
        let dst_state = match elements_table::get_element_pressure_data(dst_elem_cells) {
            Some(pd) => pd.state & 3,
            None => return 0.0,
        };
        if dst_state != 2 {
            return 0.0;
        }
    }
    // 原版 L43361-43365：fVar15 = mass[src] − (flow[src].x+y+z+w)
    let flow_sum = if sim.flow.ptr.is_null() {
        0.0
    } else {
        unsafe {
            let total = sim.width as usize * sim.height as usize;
            let flow = std::slice::from_raw_parts(sim.flow.ptr, total);
            let f = flow[src_cell];
            f.x + f.y + f.z + f.w
        }
    };
    let mass_src = cells.mass.get(src_cell);
    let mut f15 = mass_src - flow_sum;
    // 原版 L43367-43368：cells.mass[beyond] + cells.mass[dst] < f15 才位移
    if cells.mass.get(beyond_cell) + cells.mass.get(dst_cell) >= f15 {
        return 0.0;
    }
    if !displace_liquid_directional(sim, dst_cell, beyond_cell) {
        return 0.0;
    }
    f15 *= 0.125;
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        // 原版 L43372-43375：dst.mass += f15
        updated.mass.set(dst_cell, updated.mass.get(dst_cell) + f15);
        // 原版 L43377-43380：dst.temp = cells.temp[src]
        updated.temperature.set(dst_cell, cells.temperature.get(src_cell));
        // 原版 L43381-43384：dst.element = 源液体元素 + 事件
        updated.element_idx.set(dst_cell, src_elem);
        push_substance_change(sim, dst_cell);
        // 原版 L43386-43391：src.mass −= f15（钳零）
        updated.mass.set(src_cell, (updated.mass.get(src_cell) - f15).max(0.0));
        // 原版 L43393-43410：病菌 12.5% 转移（无 0 守卫，原版恒调）
        let src_disease_count = cells.disease_count.get(src_cell);
        let disease_transfer = (src_disease_count as f32 * 0.125) as i32;
        let src_disease_idx = cells.disease_idx.get(src_cell);
        add_disease_to_cell(updated, dst_cell, src_disease_idx, disease_transfer);
        modify_disease_count(updated, src_cell, -disease_transfer);
    }
    f15
}

/// 液体压力位移循环（原版 11_msvcrt_ignored.c L42480-42557）——UpdateLiquid 之后、
/// CopyFrom 之前：对液体格向 左(±iterateDirection)/右/下 挤压异元素液体邻格，
/// 并用返回值记账 flow 分量。
///
/// 区域收缩到 [3, width−4]×[3, height−4]（原版 L42520-42528：beyond 需 2 格
/// 余量，边界 2 格内不参与），与活动区域取交集。
pub(crate) fn run_liquid_displacement_task(
    sim: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    let w = sim.width as i32;
    let h = sim.height as i32;
    if w < 7 || h < 7 || sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
    let x_min = bounds.min_x.max(3) as i32;
    let x_max = (bounds.max_x as i32).min(w - 3);
    let y_min = bounds.min_y.max(3) as i32;
    let y_max = (bounds.max_y as i32).min(h - 3);
    // 排他上界（原版 L144726-144738 钳到 [3, w-3] 后用 `< max`；D-M2 2026-08-10
    // 此前 ..= 多处理 1 行 1 列）。
    if x_min >= x_max || y_min >= y_max {
        return;
    }
    let flow = if sim.flow.ptr.is_null() {
        None
    } else {
        Some(unsafe { std::slice::from_raw_parts_mut(sim.flow.ptr, (w * h) as usize) })
    };
    let mut flow = flow;
    let dir = sim.iterate_direction as i32;
    for y in y_min..y_max {
        for x in x_min..x_max {
            let cell = (y * w + x) as usize;
            let cell_elem = unsafe { (*sim.cells.ptr).element_idx.get(cell) };
            // 原版 L42480：本格 pressure state & 3 == 2（液体）
            let is_liquid = match elements_table::get_element_pressure_data(cell_elem) {
                Some(pd) => (pd.state & 3) == 2,
                None => continue,
            };
            if !is_liquid {
                continue;
            }
            // —— 左邻（cell − dir，beyond = cell − 2·dir）——
            let left = cell as i32 - dir;
            {
                let cells = unsafe { &*sim.cells.ptr };
                if cells.element_idx.get(left as usize) != cell_elem {
                    let f = do_liquid_pressure_displacement(
                        sim,
                        cell_elem,
                        cell,
                        left as usize,
                        (left - dir) as usize,
                    );
                    if let Some(fl) = flow.as_deref_mut() {
                        fl[cell].x += f;
                        fl[left as usize].y -= f;
                    }
                }
            }
            // —— 右邻（cell + dir，beyond = cell + 2·dir）——
            let right = cell as i32 + dir;
            {
                let cells = unsafe { &*sim.cells.ptr };
                if cells.element_idx.get(right as usize) != cell_elem {
                    let f = do_liquid_pressure_displacement(
                        sim,
                        cell_elem,
                        cell,
                        right as usize,
                        (right + dir) as usize,
                    );
                    if let Some(fl) = flow.as_deref_mut() {
                        fl[cell].x -= f;
                        fl[right as usize].y += f;
                    }
                }
            }
            // —— 下方（cell + width，beyond = cell + 2·width）——
            let below = cell + w as usize;
            {
                let cells = unsafe { &*sim.cells.ptr };
                if cells.element_idx.get(below) != cell_elem {
                    // 原版 L42530-42534：maxMass 查 updatedCells 当前元素；质量也读 updated
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    let updated_elem = updated.element_idx.get(cell);
                    let max_mass = match elements_table::get_element_post_process_data(updated_elem)
                    {
                        Some(ppd) => ppd.max_mass,
                        None => continue,
                    };
                    let mass = updated.mass.get(cell);
                    if max_mass <= mass {
                        let f = do_liquid_pressure_displacement(
                            sim,
                            cell_elem,
                            cell,
                            below,
                            below + w as usize,
                        );
                        if let Some(fl) = flow.as_deref_mut() {
                            fl[cell].z += f;
                            fl[below].w -= f;
                        }
                    }
                }
            }
        }
    }
}

// ===== Add 系列（cellmodifications.cpp，原版 SimDLL_Source.c L134150-134789）=====
// 供 frame_processor process_cell_modifications mode 0（ReplaceType.None / Add）分派。
// 语义：把 (mass, temp, disease) 添加到目标格，按目标格元素状态处理：
// 同元素 → 合并质量/温度（吸收为一格）；真空 → 直接放置；
// 异形态 → Displace 挤走原内容后放置（失败则四邻找同元素/真空格）；固体 → 四邻找非固体格。
// 2026-08-02 实现：液滴落地回注（C# AddToSim → AddRemoveSubstance → ModifyCell Add）的核心。

/// 查询 updated 缓冲中某格的元素状态（state & 3；元素表缺失返回 0xFF）。
fn elem_state(updated: &CellSOA, idx: usize) -> u8 {
    let e = updated.element_idx.get(idx);
    elements_table::get_element_post_process_data(e)
        .map(|p| p.state & 3)
        .unwrap_or(0xFF)
}

/// 原版 L134837-134897 的"原位放置/替换 + 剩余合并"（LAB_18003784e）：
/// 目标格已有质量 > 传入质量 → 只减质量；否则清空目标格，剩余质量合并进去。
fn place_subtract(
    sim: &mut SimData,
    target: usize,
    elem: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if target >= updated.mass.len() {
        return;
    }
    let cur_mass = updated.mass.get(target);
    if mass < cur_mass {
        updated.mass.set(target, cur_mass - mass);
        return;
    }
    let leftover = mass - cur_mass;
    updated.element_idx.set(target, elem);
    updated.mass.set(target, 0.0);
    updated.temperature.set(target, 0.0);
    let disease_to_add = if mass > 0.0 {
        ((leftover / mass) * disease_count as f32) as i32
    } else {
        0
    };
    add_mass_and_update_temperature(updated, target, leftover, temperature, disease_idx, disease_to_add);
    if updated.mass.get(target) <= 1.1754944e-38 {
        updated.element_idx.set(target, sim.vacuum_element_idx);
    }
    push_substance_change(sim, target);
}

/// 四邻（右/左/上/下）找同元素格 → 合并并返回 true。
fn merge_into_same_element_neighbor(
    sim: &mut SimData,
    target: usize,
    elem: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
) -> bool {
    if sim.updated_cells.ptr.is_null() {
        return false;
    }
    let width = sim.width as usize;
    let found = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        let mut found_n = None;
        for n in [target + 1, target.wrapping_sub(1), target + width, target.wrapping_sub(width)] {
            if n < updated.element_idx.len() && updated.element_idx.get(n) == elem {
                found_n = Some(n);
                break;
            }
        }
        found_n
    };
    if let Some(n) = found {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        add_mass_and_update_temperature(updated, n, mass, temperature, disease_idx, disease_count);
        return true;
    }
    false
}

/// AddLiquid（原版 L134362-134555）。
pub(crate) fn add_liquid(
    sim: &mut SimData,
    cell: usize,
    elem: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if cell >= updated.element_idx.len() {
        return;
    }
    let cur = updated.element_idx.get(cell);
    // 原版 L134375-134377：同元素 → 合并（吸收为一格）
    if cur == elem {
        add_mass_and_update_temperature(updated, cell, mass, temperature, disease_idx, disease_count);
        return;
    }
    let width = sim.width as usize;
    // 原版 L134379-134411：目标固体 → 四邻找第一个非固体格（左/右/下/上）
    let mut target = cell;
    if elem_state(updated, cell) == 3 {
        for n in [
            cell.wrapping_sub(1),
            cell + 1,
            cell.wrapping_sub(width),
            cell + width,
        ] {
            if n < updated.element_idx.len() && elem_state(updated, n) != 3 {
                target = n;
                break;
            }
        }
    }
    let target_state = elem_state(updated, target);
    // 原版 L134412-134425：目标固体/void → 原位减质量或替换+剩余合并
    if target_state == 3 || updated.element_idx.get(target) == sim.void_element_idx {
        place_subtract(sim, target, elem, mass, temperature, disease_idx, disease_count);
        return;
    }
    if target_state == 0 {
        // 原版 L134426-134431：真空 → 直接放置
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.element_idx.set(target, elem);
        add_mass_and_update_temperature(updated, target, mass, temperature, disease_idx, disease_count);
        push_substance_change(sim, target);
        return;
    }
    if target_state == 1 {
        // 原版 L134456-134495：气体 → DisplaceGas；失败 → 下方液体交换 + 四邻同元素合并/放置
        let gas = updated.element_idx.get(target);
        if displace_gas(sim, target, gas) {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.element_idx.set(target, elem);
            add_mass_and_update_temperature(updated, target, mass, temperature, disease_idx, disease_count);
            push_substance_change(sim, target);
            return;
        }
        // 原版 L1191：`iVar11 = width + iVar14` —— 与**上格**液体交换（C# Grid.CellAbove
        // = cell+width）；2026-08-12 对照审查修正：此前误用 target-width（下格），
        // 与 AddLiquid 气体分支语义相反。
        let above = target + width;
        if above < updated.element_idx.len() && elem_state(updated, above) == 2 {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            swap_cells(updated, above, target);
            push_substance_change(sim, above);
            push_substance_change(sim, target);
        }
        if merge_into_same_element_neighbor(sim, target, elem, mass, temperature, disease_idx, disease_count) {
            return;
        }
        place_subtract(sim, target, elem, mass, temperature, disease_idx, disease_count);
        return;
    }
    // 原版 L134446-134455：异液体 → DisplaceLiquid 挤走后，四邻同元素合并/放置
    let liq = updated.element_idx.get(target);
    // 2026-08-12 修正：原版 `bVar15 = DisplaceLiquid(...)`，**成功**才把新液体写入
    // 目标格（LAB_180037959 ChangeSubstance）；此前忽略返回值，成功时误并入同元素
    // 邻格、目标格留真空，与 AddLiquid 语义不符。
    if displace_liquid(sim, target, liq) {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.element_idx.set(target, elem);
        add_mass_and_update_temperature(
            updated,
            target,
            mass,
            temperature,
            disease_idx,
            disease_count,
        );
        push_substance_change(sim, target);
        return;
    }
    if merge_into_same_element_neighbor(sim, target, elem, mass, temperature, disease_idx, disease_count) {
        return;
    }
    place_subtract(sim, target, elem, mass, temperature, disease_idx, disease_count);
}

/// AddGas（原版 L134150-134360）。
pub(crate) fn add_gas(
    sim: &mut SimData,
    cell: usize,
    elem: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if cell >= updated.element_idx.len() {
        return;
    }
    let cur = updated.element_idx.get(cell);
    if cur == elem {
        add_mass_and_update_temperature(updated, cell, mass, temperature, disease_idx, disease_count);
        return;
    }
    let width = sim.width as usize;
    let target_state = elem_state(updated, cell);
    // 原版 L134186-134207：真空直接放置；异形态 Displace 后放置
    let displace_ok = match target_state {
        0 => true,
        1 => displace_gas(sim, cell, cur),
        // 2026-08-12 修正：原版 `bVar10 = DisplaceLiquid(...)`——液体挤不动（如水格
        // 四周液体不可渗透的透气砖）时**不得**覆写水格；此前无条件 true → 水整格
        // 瞬间消失（高压存储场景玩家反馈）。
        2 => displace_liquid(sim, cell, cur),
        _ => false,
    };
    if displace_ok {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.element_idx.set(cell, elem);
        add_mass_and_update_temperature(updated, cell, mass, temperature, disease_idx, disease_count);
        push_substance_change(sim, cell);
        return;
    }
    // 原版 L134208-134249：Displace 失败 → 邻格 [左, 右, 上] **逐格**先查真空再查
    // 同元素（位置优先：左格真空优先于右格同元素）。2026-08-12 修正：此前分两轮
    // 收集"所有真空优先于同元素"，与原版逐格判断的优先级不符。
    for n in [cell.wrapping_sub(1), cell + 1, cell + width] {
        let ne = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            if n >= updated.element_idx.len() {
                continue;
            }
            updated.element_idx.get(n)
        };
        if ne == sim.vacuum_element_idx {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.element_idx.set(n, elem);
            updated.mass.set(n, mass);
            updated.temperature.set(n, temperature);
            updated.disease_idx.set(n, disease_idx);
            updated.disease_count.set(n, disease_count);
            push_substance_change(sim, n);
            return;
        }
        if ne == elem {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            add_mass_and_update_temperature(
                updated,
                n,
                mass,
                temperature,
                disease_idx,
                disease_count,
            );
            return;
        }
    }
    // 原版 L134249-134277：兜底原位减质量或替换+剩余合并
    place_subtract(sim, cell, elem, mass, temperature, disease_idx, disease_count);
}

/// AddSolid（原版 SimDLL_Source.c L134555-134789）。
/// `add_sub_type`（ModifyCell 消息 addSubType）：0 = DoVerticalDisplacement（默认，
/// 流星/AddRemoveSubstance 固体）；1 = OnlyIfSameElement；其他 → no-op。
pub(crate) fn add_solid(
    sim: &mut SimData,
    cell: usize,
    elem: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
    add_sub_type: u8,
) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim.width as usize;
    let total = width * sim.height as usize;
    if cell >= total {
        return;
    }
    // —— Type 0：DoVerticalDisplacement（L134573-134743）——
    if add_sub_type == 0 {
        let row = cell / width;
        let col = cell % width;
        let mut remaining = mass;
        // 阶段 1：同列 4 格（row−1/0/+1/+2）同元素 → 按 ppd.maxMass 补满
        for dr in [-1i32, 0, 1, 2] {
            let r = row as i32 + dr;
            if r < 0 || r as usize >= sim.height as usize {
                continue;
            }
            let c = r as usize * width + col;
            let updated = unsafe { &*sim.updated_cells.ptr };
            if updated.element_idx.get(c) != elem {
                continue;
            }
            let Some(max_mass) = elements_table::get_element_post_process_data(elem)
                .map(|p| p.max_mass)
            else {
                continue;
            };
            let room = max_mass - updated.mass.get(c);
            if room <= 0.0 {
                continue;
            }
            let f16 = room.min(remaining);
            let d = (f16 / remaining * disease_count as f32) as i32;
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            add_mass_and_update_temperature(updated, c, f16, temperature, disease_idx, d);
            remaining -= f16;
            if remaining <= 0.0 {
                return;
            }
        }
        // 阶段 2：从目标格向下走，跳过固体、挤走气/液、落到第一个非固体格
        let mut cur = cell;
        loop {
            if cur >= total {
                return;
            }
            let updated = unsafe { &*sim.updated_cells.ptr };
            let cur_elem = updated.element_idx.get(cur);
            if cur_elem == sim.unobtanium_element_idx {
                return;
            }
            let state = match elements_table::get_element_post_process_data(cur_elem) {
                Some(p) => p.state & 3,
                None => return,
            };
            match state {
                3 => {
                    cur += width;
                    continue;
                }
                1 => {
                    crate::c2_physics::liquid_flow::displace_gas(sim, cur, cur_elem);
                }
                2 => {
                    crate::c2_physics::liquid_flow::displace_liquid(sim, cur, cur_elem);
                }
                _ => {}
            }
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.element_idx.set(cur, elem);
            updated.mass.set(cur, remaining);
            updated.temperature.set(cur, temperature);
            updated.disease_idx.set(cur, disease_idx);
            updated.disease_count.set(cur, disease_count);
            push_substance_change(sim, cur);
            return;
        }
    }
    // —— Type 1：OnlyIfSameElement（L134744-134789）——
    if add_sub_type == 1 {
        let updated = unsafe { &*sim.updated_cells.ptr };
        let cur_elem = updated.element_idx.get(cell);
        let state = match elements_table::get_element_post_process_data(cur_elem) {
            Some(p) => p.state & 3,
            None => return,
        };
        match state {
            3 => {
                if cur_elem != elem {
                    return;
                }
            }
            1 => {
                crate::c2_physics::liquid_flow::displace_gas(sim, cell, cur_elem);
            }
            2 => {
                crate::c2_physics::liquid_flow::displace_liquid(sim, cell, cur_elem);
            }
            _ => {}
        }
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        add_mass_and_update_temperature(updated, cell, mass, temperature, disease_idx, disease_count);
        push_substance_change(sim, cell);
        return;
    }
    // —— 其他 Type → no-op（原版直接 return）——
}

/// DoPressureBreak（原版 04_temperature.c L2663-2792）：超压液体沿一个方向尝试突破。
///
/// 从 `cell` 出发沿 `offset` 走最多 3 格：每遇到一格固体，累加其"抗压阻力"：
/// `resistance += ((mass/maxMass)*bit7 + (1-bit7)) * strength * (strengthInfo&0x7f) * 0.25 + 0.1`
/// （bit7=strengthInfo 最高位；不可破坏固体 state&4 或 properties&8 → 直接挡住）。
/// 非固体格 → 停止走；若累计阻力 < 压强比（mass/maxMass）→ 突破成功：
/// 对被穿过的固体格产生 worldDamageInfo，返回 true。
/// 2026-08-02 实现：原版超压第一优先是 4 方向突破（缺失时总是向下倾倒 → 上下合并异常）。
fn do_pressure_break(sim: &mut SimData, cell: usize, ratio: f32, offset: isize) -> bool {
    if sim.updated_cells.ptr.is_null() || sim.sim_events.ptr.is_null() {
        return false;
    }
    let width = sim.width as usize;
    let total = width * sim.height as usize;
    let mut cur = cell as isize;
    let mut solid_count: usize = 0;
    let mut resistance = 1.0f32;
    let mut iter = 0usize;
    while iter < 3 {
        cur += offset;
        if cur < 0 || cur as usize >= total {
            return false; // 越界（原版崩溃，Rust 安全返回）
        }
        let c = cur as usize;
        let updated = unsafe { &*sim.updated_cells.ptr };
        let elem = updated.element_idx.get(c);
        let ppd = match elements_table::get_element_post_process_data(elem) {
            Some(p) => p,
            None => return false,
        };
        if (ppd.state & 3) != 3 {
            break; // 非固体 → 停止走（原版 L2686）
        }
        if (ppd.state & 4) != 0 {
            return false; // 不可破坏固体（原版 L2688）
        }
        if (updated.properties.get(c) & 8) != 0 {
            return false; // properties bit3 挡住（原版 L2693）
        }
        solid_count += 1;
        let sb = updated.strength_info.get(c);
        let fvar14 = if (sb >> 7) == 1 { 1.0f32 } else { 0.0f32 };
        let fvar13 = if fvar14 >= 1.0 { 0.0f32 } else { 0.100000024 };
        let mass_c = updated.mass.get(c);
        let strength = ppd.strength;
        resistance += ((mass_c / ppd.max_mass) * fvar14 + (1.0 - fvar14))
            * strength
            * ((sb & 0x7f) as f32)
            * 0.25
            + fvar13;
        if ratio < resistance {
            return false; // 压强不足 → 被挡（原版 L2707）
        }
        iter += 1;
    }
    // 原版 L2716：`(uVar9 - 1 < 2)`，uVar9 为 uint —— 0 格固体时 0u32-1 回绕为
    // 0xFFFFFFFF，判断为 false（不突破），仅 1~2 格固体可突破。
    // 修复：此前 `(solid_count as i32 - 1) < 2` 在 0 时得 -1<2=true，超压液体
    // 遇开放方向（真空/气体）会误报“突破成功”提前返回，跳过其余方向的墙壁
    // 破坏与下方倾倒（原版会继续尝试 → 与用户实测液体行为不符）。
    if (solid_count == 1 || solid_count == 2) && resistance < ratio {
        let c = cur as usize;
        if c < total {
            let updated = unsafe { &*sim.updated_cells.ptr };
            let elem = updated.element_idx.get(c);
            if let Some(ppd2) = elements_table::get_element_post_process_data(elem) {
                // 原版 L2720-2728：边界格是液体且有质量 → 从压强比中扣除
                // 原版 L2742-2752：边界格为液体且有质量 → 从压力比中扣除 mass/maxMass；
                // 随后**无条件**按 (resistance < ratio) 判定突破。此前 Rust 把判定嵌进
                // `if m > 0.0` 内，mass<=0 时漏判 → 不炸墙泄压（待对照 P2-2）。
                let mut ratio_eff = ratio;
                if (ppd2.state & 3) == 2 {
                    let m = updated.mass.get(c);
                    if m > 0.0 {
                        ratio_eff = ratio - m / ppd2.max_mass;
                    }
                }
                if resistance < ratio_eff {
                    push_world_damage(sim, cell, offset, solid_count);
                    return true;
                }
            } else if resistance < ratio {
                push_world_damage(sim, cell, offset, solid_count);
                return true;
            }
        } else if resistance < ratio {
            push_world_damage(sim, cell, offset, solid_count);
            return true;
        }
    }
    false
}

/// 对被穿过的固体格产生 worldDamageInfo（原版 L2730-2765）。
fn push_world_damage(sim: &mut SimData, cell: usize, offset: isize, solid_count: usize) {
    let width = sim.width as usize;
    let game_w = sim.width - 2;
    let src_game = ((cell % width) as i32 - 1) + ((cell / width) as i32 - 1) * game_w;
    let mut pos = cell as isize;
    for _ in 0..solid_count {
        pos += offset;
        if pos < 0 {
            continue;
        }
        let p = pos as usize;
        let game_cell = ((p % width) as i32 - 1) + ((p / width) as i32 - 1) * game_w;
        if game_cell >= 0 && game_cell < sim.num_game_cells {
            crate::c2_physics::region_events::emit_world_damage(
                sim,
                crate::a_framework::game_data::WorldDamageInfo {
                    cell_idx: game_cell,
                    damage_source_cell_idx: src_game,
                },
            );
        }
    }
}

/// PostProcessCell — 格后处理（原版 05_liquid_flow.c L2320-2867）。
///
/// 按元素状态分支：
/// - 真空（state&3==0）：properties&1 置位 → return；否则 DoSublimation（留桩）。
/// - 气体（state&3==1）：
///   1. `displacementDirection` 每格交替反转（L2382-2384）。
///   2. 质量 ∈ [1e-9, 0.001) 时检查挤压邻居（顺序受 displacementDirection 驱动：
///      [cell+dir, cell-dir, cell-width, cell+width]），任一邻居质量 >= 1.0 且为气体
///      → 清 flow + Evaporate（L2392-2460）。
///   3. DoDensityDisplacement(cell, 0.99)（L2461）；失败且非固体属性时，
///      随机 > 0.9（10%）→ 找 [cell-dir, cell+dir, cell-width] 中最轻的气体邻居交换
///      （L2469-2526，molar_mass 轻者优先，首次迭代不进入更重气体）。
/// - 固体（state&3==3）：state & 0xb == 0xb 时 DoUnstableCheck（留桩）。
/// - 液体（state&3==2）：
///   1. 质量 <= 0.01 → 清 flow + Evaporate（L2551-2558）。
///   2. DoDensityDisplacement(cell, 0.3)（L2559）；true → return。
///   3. 质量 > maxMass → 过压释放（L2566-2704）：下方格是液体且更轻时，
///      DisplaceLiquid 挤走下方液体腾出空间，再转移质量 × 0.49751243 到下方
///      （含温度/元素/病菌复制 + 事件）。
///   4. DoPartialHeatTransition（留桩）+ DoSublimation（相变留桩，仅保留
///      accumulatedFlow 累加 L2831）。
///
/// 元素表缺失（测试环境）时安全返回。
pub fn post_process_cell(sim: &mut SimData, cell: usize) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim.width as usize;
    let height = sim.height as usize;
    if width < 3 || height < 3 || cell < width {
        return;
    }
    // —— 读元素 + 后处理数据（原版 L2361-2371）——
    let (elem, ppd, props_cell) = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        if cell >= updated.element_idx.len() {
            return;
        }
        let elem = updated.element_idx.get(cell);
        let ppd = match elements_table::get_element_post_process_data(elem) {
            Some(p) => p,
            None => return,
        };
        (elem, ppd, updated.properties.get(cell))
    };
    let state3 = ppd.state & 3;

    // ===== 真空（原版 L2373-2380）=====
    if state3 == 0 {
        if props_cell & 1 != 0 {
            return;
        }
        do_sublimation(sim, cell);
        return;
    }

    // ===== 气体（原版 L2381-2531）=====
    if state3 == 1 {
        // 原版 L2382-2384：displacementDirection 每格交替反转。
        // T4 路由：区域 RNG 存在 → 用区域方向；无 → sim 全局（串行不变）。
        let dir = if let Some(rng) = crate::c2_physics::region_rng::region_rng_ptr() {
            let rng = unsafe { &mut *rng };
            let d = rng.displacement_direction;
            rng.displacement_direction = -d;
            d
        } else {
            let d = sim.displacement_direction;
            sim.displacement_direction = -d;
            d
        };
        let mass_cell = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            updated.mass.get(cell)
        };
        if mass_cell >= 1e-09 {
            if mass_cell < 0.001 {
                // 挤压邻居检查（L2392-2450）：顺序 [cell+dir, cell-dir, cell-width, cell+width]
                let total = width * height;
                let order = [
                    cell as isize + dir as isize,
                    cell as isize - dir as isize,
                    cell as isize - width as isize,
                    cell as isize + width as isize,
                ];
                let mut squeezed = false;
                for cand in order {
                    if cand < 0 {
                        continue;
                    }
                    let cand = cand as usize;
                    if cand >= total {
                        continue;
                    }
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    if updated.mass.get(cand) >= 1.0 {
                        if let Some(ppd2) =
                            elements_table::get_element_post_process_data(updated.element_idx.get(cand))
                        {
                            if (ppd2.state & 3) == 1 {
                                squeezed = true;
                                break;
                            }
                        }
                    }
                }
                if squeezed {
                    clear_flow(sim, cell);
                    do_evaporate(sim, cell);
                }
            }
        } else {
            // 原版 L2454-2460：mass < 1e-09 → 清 flow + Evaporate
            clear_flow(sim, cell);
            do_evaporate(sim, cell);
        }
        // 原版 L2461：DoDensityDisplacement(cell, ppd, 0.99)
        if !do_density_displacement(sim, cell, 0.99) {
            // 原版 L2469-2475：门控 = (props&2 != 0 || 四方向 DoPartialMelt 全 false) && random > 0.9。
            // 注意 props&2 置位（固体标志）的气体格**也**进入置换——原版是 OR 关系，
            // 旧实现 `(props&2)==0` 把置位格排除，与本修正冲突。
            // DoPartialMelt 完整版（温度.rs）：真实执行熔化副作用（液滴/就地转化），
            // 短路语义与原版一致——任一方向熔化成功即停止其余方向并阻断气体置换。
            let melt_blocked =
                crate::c2_physics::temperature::do_partial_melt(sim, cell, cell.wrapping_sub(width))
                    || crate::c2_physics::temperature::do_partial_melt(sim, cell, cell.wrapping_sub(1))
                    || crate::c2_physics::temperature::do_partial_melt(sim, cell, cell + 1)
                    || crate::c2_physics::temperature::do_partial_melt(sim, cell, cell + width);
            if ((props_cell & 2) != 0 || !melt_blocked) && next_random(sim) > 0.9 {
                // 候选顺序（原版 L2484-2486）：L2384 先反转 dir、L2484 再读新值，
                // 净效果 = [cell-旧dir, cell+旧dir, cell-width]（旧实现顺序相反，已修正）
                let candidates = [
                    cell as isize - dir as isize,
                    cell as isize + dir as isize,
                    cell as isize - width as isize,
                ];
                let total = width * height;
                let mut best: Option<(usize, f32)> = None;
                for (idx, &cand) in candidates.iter().enumerate() {
                    if cand < 0 {
                        continue;
                    }
                    let cand = cand as usize;
                    if cand >= total {
                        continue;
                    }
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    // 原版 L149467：fVar24 读 updatedCells + 0x28 = **temperature** 数组
                    //（CellSOA 布局 L11008-11020：elementIdx/temperature/mass/properties...）
                    // → 候选选择"最低温度"气体格，非最低质量；old 版亦取温度。
                    // 2026-08-04 审查修正：此前误读为 mass。
                    let cand_temp = updated.temperature.get(cand);
                    if let Some((_, best_mass)) = best {
                        if cand_temp > best_mass {
                            continue; // 原版 L2496：fVar24 <= fVar23（温度不优于当前最优）
                        }
                    }
                    if next_random(sim) <= 0.5 {
                        continue; // 原版 L2497-2498：0.5 < random
                    }
                    let cand_elem = updated.element_idx.get(cand);
                    let cand_ppd = elements_table::get_element_post_process_data(cand_elem);
                    let is_gas = matches!(cand_ppd, Some(p) if (p.state & 3) == 1);
                    if !is_gas {
                        continue;
                    }
                    // 原版 L2506-2510 标志位 [false,false,true]（L149459 CONCAT13 0x10000）：
                    // 摩尔质量约束只作用于**第 3 个候选（下方格 cell-width）**——
                    // 防止 O2 与下方更重 CO2 交换（CO2 上浮）；水平候选不受约束。
                    // 2026-08-04 修正：此前误挂在 idx==0（水平候选），导致分层失效。
                    if idx == 2 {
                        if let Some(cp) = cand_ppd {
                            if ppd.molar_mass <= cp.molar_mass {
                                continue;
                            }
                        }
                    }
                    best = Some((cand, cand_temp));
                }
                if let Some((b, _)) = best {
                    let updated = unsafe { &mut *sim.updated_cells.ptr };
                    swap_cells(updated, b, cell);
                    push_substance_change(sim, b);
                    push_substance_change(sim, cell);
                }
            }
        }
        do_sublimation(sim, cell);
        return;
    }

    // ===== 固体（原版 L2533-2545）=====
    if state3 == 3 {
        if (ppd.state & 0xb) != 0xb {
            return;
        }
        do_unstable_check(sim, cell);
        return;
    }

    // ===== 液体（原版 L2546-2863）=====
    let mass_cell = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        updated.mass.get(cell)
    };
    // 原版 L2551-2558：质量 <= 0.01 → 清 flow + Evaporate
    if mass_cell <= 0.01 && mass_cell != 0.01 {
        clear_flow(sim, cell);
        do_evaporate(sim, cell);
        return;
    }
    // 原版 L2559-2562：DoDensityDisplacement(cell, ppd, 0.3)
    if do_density_displacement(sim, cell, 0.3) {
        return;
    }
    // —— 过压释放（原版 L2566-2704）——
    let mass_cell = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        updated.mass.get(cell)
    };
    // 2026-08-03 防御：max_mass <= 0 时跳过（原版依赖数据不变量 maxMass>0，
    // 反编译无守卫；此处避免 0/0 → pressure_ratio=inf 传播进 DoPressureBreak）。
    if ppd.max_mass > 0.0 && ppd.max_mass < mass_cell {
        // 原版 L2565-2583：先按压强比 mass/maxMass 尝试 4 方向突破（DoPressureBreak）。
        // 任一方向突破成功 → 释放超压（worldDamage），不再向下倾倒。
        // 2026-08-02 实现：此前留桩 → 超压总是向下倾倒，造成上下合并异常。
        let pressure_ratio = mass_cell / ppd.max_mass;
        if do_pressure_break(sim, cell, pressure_ratio, -(width as isize)) {
            return;
        }
        if do_pressure_break(sim, cell, pressure_ratio, -1) {
            return;
        }
        if do_pressure_break(sim, cell, pressure_ratio, 1) {
            return;
        }
        if do_pressure_break(sim, cell, pressure_ratio, width as isize) {
            return;
        }
        let below = cell + width;
        let (mass_below, below_elem) = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            (updated.mass.get(below), updated.element_idx.get(below))
        };
        // 原版 L2589：maxMass × 1.5 < mass[cell]；L2597：mass[below] < mass[cell]
        if ppd.max_mass * 1.5 < mass_cell && mass_below < mass_cell {
            let below_state = match elements_table::get_element_liquid_data(below_elem) {
                Some(ld) => ld.state & 3,
                None => 0,
            };
            if below_state == 2 {
                // 原版 L2606-2613：threshold = max(maxMass, mass_below × 1.01)，
                // threshold <= mass[cell] 且不等 → DisplaceLiquid 挤走下方液体腾空间。
                // ⚠️ 2026-08-02 修复：旧实现误用 min——下方快满（below×1.01 > mass[cell]）时
                // 原版不往下倒（无处容纳），旧实现却挤压下方再倾倒 → 上下合并异常。
                let threshold = (mass_below * 1.01).max(ppd.max_mass);
                if threshold <= mass_cell && mass_cell != threshold {
                    if displace_liquid(sim, below, below_elem) {
                        // 原版 L2617-2698：转移 mass × 0.49751243（≈ ÷2.01）到下方格
                        let updated = unsafe { &mut *sim.updated_cells.ptr };
                        let fvar23 = updated.mass.get(cell);
                        let fvar24 = fvar23 * 0.49751243;
                        let d_transfer = (updated.disease_count.get(cell) as f32 * (fvar24 / fvar23)) as i32;
                        updated.mass.set(below, fvar24);
                        updated.temperature.set(below, updated.temperature.get(cell));
                        updated.element_idx.set(below, updated.element_idx.get(cell));
                        updated.disease_count.set(below, d_transfer);
                        updated.disease_idx.set(below, updated.disease_idx.get(cell));
                        updated.disease_infestation_tick_count.set(below, 0);
                        updated.disease_growth_accumulated_error.set(below, 0.0);
                        push_substance_change(sim, below);
                        updated.mass.set(cell, updated.mass.get(cell) - fvar24);
                        let remain = updated.disease_count.get(cell) - d_transfer;
                        if remain > 0 {
                            updated.disease_count.set(cell, remain);
                            return;
                        }
                        clear_disease(updated, cell);
                        return;
                    }
                }
            }
        }
    }
    // DoPartialHeatTransition 4 方向（原版 L2705-2721）：任一成功 → 提前返回
    if crate::c2_physics::temperature::do_partial_heat_transition(sim, cell, cell.wrapping_sub(width))
        || crate::c2_physics::temperature::do_partial_heat_transition(sim, cell, cell.wrapping_sub(1))
        || crate::c2_physics::temperature::do_partial_heat_transition(sim, cell, cell + 1)
        || crate::c2_physics::temperature::do_partial_heat_transition(sim, cell, cell + width)
    {
        return;
    }
    // —— DoSublimation（L2722-2863）：液体 offgas 主路径 + 耗尽路径 ——
    if !sim.accumulated_flow.ptr.is_null() {
        let below = cell + width;
        let (mass_below, below_elem, props_below) = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            (
                updated.mass.get(below),
                updated.element_idx.get(below),
                updated.properties.get(below),
            )
        };
        // 原版 L2731/2734/2742：前三项前置条件（下方质量 < 1.8、sublimateIndex 有效、下方真空/气体）
        let below_vac_or_gas = match elements_table::get_element_post_process_data(below_elem) {
            Some(p) => (p.state & 3) <= 1,
            None => false,
        };
        if mass_below < 1.8 && ppd.sublimate_index != 0xffff && below_vac_or_gas {
            // 原版 L2746-2747：在 props&1 检查**之前**消耗 randomSeed——
            // 旧实现把概率门放进 below_ok 的 && 短路里，在 props 检查后才消耗，已对齐。
            let random = next_random(sim);
            // 原版 L2752/2755：props&1 检查 + 升华概率门
            if props_below & 1 == 0 && ppd.sublimate_probability > random {
                let mass_cell = {
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    updated.mass.get(cell)
                };
                // 原版 L2765-2769：fVar24 = min(offGasPercentage × mass[cell], 1.0)
                let mut fvar24 = ppd.off_gas_percentage * mass_cell;
                if fvar24 >= 1.0 {
                    fvar24 = 1.0;
                }
                // 原版 L2770-2785：**上方格**（cell-width，原版 L2758 local_104 = cell-width）
                // 同元素且更重 → 用上方质量重算 fVar24（bVar9 = 更重标记，耗尽路径第二段判定用）。
                // ⚠️ 修复轮：旧实现误用下方格（below=cell+width），原版读的是 cell-width，已修正。
                let mut heavier_up = false;
                let up = cell.wrapping_sub(width);
                {
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    if updated.element_idx.get(up) == elem {
                        let mass_up = updated.mass.get(up);
                        if mass_cell < mass_up {
                            heavier_up = true;
                            fvar24 = ppd.off_gas_percentage * mass_up;
                            if fvar24 >= 1.0 {
                                fvar24 = 1.0;
                            }
                        }
                    }
                }
                // 原版 L2788-2790：剩余质量 >= 0.01（fVar24 <= 0 时原版直接 return，无 FX）
                if mass_cell - fvar24 >= 0.01 {
                    if fvar24 <= 0.0 {
                        return;
                    }
                    // 原版 L2792-2804：下方格元素非升华目标且非真空 → 先 DisplaceGas
                    // 把下方异类气体挤走；挤不走 → return（不累积，跳过 SpawnFX）。
                    // ⚠️ 修复轮：此项为原版 L2799 前置（旧实现缺失）。
                    let below_elem = {
                        let updated = unsafe { &*sim.updated_cells.ptr };
                        updated.element_idx.get(below)
                    };
                    if below_elem != ppd.sublimate_index {
                        if below_elem != sim.vacuum_element_idx {
                            if !displace_gas(sim, below, below_elem) {
                                return;
                            }
                        }
                    }
                    // 原版 L2805-2812：(1.8 - mass[below]) / sublimateEfficiency 缩放。
                    // ⚠️ 修复轮：此项为原版 L2809-2812 修正（旧实现缺失）。
                    let mass_below_now = {
                        let updated = unsafe { &*sim.updated_cells.ptr };
                        updated.mass.get(below)
                    };
                    let scaled = fvar24 * ppd.sublimate_efficiency;
                    let room = 1.8 - mass_below_now;
                    if room < scaled {
                        fvar24 = fvar24 * (room / scaled);
                    }
                    // 病菌合并判定（L149806-149819）：同菌种或目标无菌 → 合并
                    let merge = {
                        let updated = unsafe { &*sim.updated_cells.ptr };
                        let src_idx = updated.disease_idx.get(cell);
                        let dst_idx = updated.disease_idx.get(below);
                        src_idx == dst_idx || dst_idx == 0xff
                    };
                    // 原版 L149810：产气（源格扣质量/病菌，目标格得升华气体）
                    emit_sublimate_gas(sim, cell, below, fvar24, merge, ppd.sublimate_index);
                    // 原版 L2831-2832：accumulated_flow[cell] += fVar24
                    let total = sim.width as usize * sim.height as usize;
                    let acc = unsafe { std::slice::from_raw_parts_mut(sim.accumulated_flow.ptr, total) };
                    if cell < total {
                        acc[cell] += fvar24;
                    }
                } else {
                    // 耗尽路径（L149821-149833）：液体格就地转气体；差额从 cell-width 补足
                    emit_sublimate_gas(sim, cell, cell, mass_cell, true, ppd.sublimate_index);
                    if fvar24 - mass_cell > 0.0 && heavier_up {
                        let down = cell.wrapping_sub(width);
                        let merge = {
                            let updated = unsafe { &*sim.updated_cells.ptr };
                            let src_idx = updated.disease_idx.get(down);
                            let cell_idx = updated.disease_idx.get(cell);
                            src_idx == cell_idx || cell_idx == 0xff
                        };
                        emit_sublimate_gas(
                            sim,
                            down,
                            cell,
                            fvar24 - mass_cell,
                            merge,
                            ppd.sublimate_index,
                        );
                    }
                }
                // 原版 LAB_18004e400：SpawnFX(cell, sublimateFX, 0.0)
                push_spawn_fx(sim, cell, ppd.sublimate_fx, 0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
    use crate::a_framework::sim_data::SimData;
    use crate::a_framework::stl_shim::UniquePtr;
    use crate::b_elements::element::Element;
    use crate::b_elements::elements_table::{CreateElementsTable, DestroyElementsTable};
    use crate::LIB_TESTS_LOCK;

    /// 建一张 3 元素最小表：0=气体(state=1)、1=液体(state=2)、2=固体(state=3)。
    /// 调用方需持 LIB_TESTS_LOCK，并在测试末尾 DestroyElementsTable() 清理。
    /// 序列化格式对照 03_elements.c L27-429：
    /// count + count×164B 元素（连续）+ count×(4B 名称长度 + 名称)。
    fn create_state_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(3);
        for (id, state) in [(0i32, 1u8), (1i32, 2u8), (2i32, 3u8)] {
            let mut elem = Element::default();
            elem.id = id;
            elem.state = state;
            elem.number_of_gradient_colors = 1; // 避免 texture 相关除法异常
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(&elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..3 {
            w.write_int(0); // 空名称（长度=0），全部元素之后读
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    fn create_crude_oil_boil_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(5);
        let elems = [
            Element::default(),
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.molar_mass = 52.9;
                e.max_mass = 870.0;
                e.min_vertical_flow = 0.0;
                e.specific_heat_capacity = 1.69;
                e.thermal_conductivity = 2.0;
                e.low_temp = 233.0;
                e.high_temp = 673.0;
                e.high_temp_transition_idx = 2;
                e
            },
            {
                let mut e = Element::default();
                e.id = 2;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.molar_mass = 44.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.specific_heat_capacity = 1.69;
                e.thermal_conductivity = 2.0;
                e.low_temp = 216.0;
                e.high_temp = 812.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 3;
                e.state = 3;
                e.number_of_gradient_colors = 1;
                e.max_mass = 1000.0;
                e.specific_heat_capacity = 1.0;
                e.thermal_conductivity = 2.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 4;
                e.state = 1;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e.molar_mass = 29.0;
                e
            },
        ];
        for elem in &elems {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..5 {
            w.write_int(0);
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    #[test]
    fn crude_oil_partial_transition_full_pipeline_produces_676k_petroleum() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_crude_oil_boil_table();
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                for i in 0..36usize {
                    buf.element_idx.set(i, 0);
                    buf.mass.set(i, 0.0);
                    buf.temperature.set(i, 300.0);
                }
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 800.0);
                buf.temperature.set(14, 607.0);
                buf.element_idx.set(15, 3);
                buf.mass.set(15, 1000.0);
                buf.temperature.set(15, 1000.0);
                // solid below (cell-width = 8) keeps the oil from falling away
                buf.element_idx.set(8, 3);
                buf.mass.set(8, 1000.0);
                buf.temperature.set(8, 600.0);
            }
        }
        let mut petroleum_drops_676k = 0usize;
        let mut petroleum_drops_other: Vec<(f32, f32)> = Vec::new();
        for _tick in 0..20 {
            crate::c2_physics::update_data(&mut sd);
            let spawns: Vec<(u16, f32, f32)> = unsafe {
                let events = &*sd.sim_events.ptr;
                events
                    .spawn_liquid_info
                    .as_slice()
                    .iter()
                    .map(|d| (d.element_idx, d.mass, d.temperature))
                    .collect()
            };
            unsafe {
                let events = &mut *sd.sim_events.ptr;
                events.spawn_liquid_info.clear();
            }
            for (elem, mass, temp) in spawns {
                if elem == 2 {
                    if (temp - 676.0).abs() < 1.0 {
                        petroleum_drops_676k += 1;
                    } else {
                        petroleum_drops_other.push((mass, temp));
                    }
                }
            }
            let oil_mass = unsafe { (*sd.updated_cells.ptr).mass.get(14) };
            if oil_mass <= 0.01 && !petroleum_drops_other.is_empty() {
                break;
            }
        }
        assert!(
            petroleum_drops_676k > 0,
            "pipeline should emit petroleum droplet at 676K; non-676K drops: {:?}",
            petroleum_drops_other
        );
        assert!(
            petroleum_drops_other.is_empty(),
            "found petroleum droplet not at 676K (mass,temp): {:?}",
            petroleum_drops_other
        );
        DestroyElementsTable();
    }
    /// IsLiquidPermeable：真空/气体（state&3 <= 1）可渗透；
    /// 液体（state&3 == 2）与固体（state&3 == 3）都**不可渗透**（原版 L615: `1 < (state&3)` → false）；
    /// properties bit1（&2）置位时不可渗透（原版 L607）。
    #[test]
    fn is_liquid_permeable_basic() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(0, 0); // 气体 state=1
            cells.element_idx.set(1, 1); // 液体 state=2
            cells.element_idx.set(2, 2); // 固体 state=3
            assert!(is_liquid_permeable(cells, 0), "真空/气体应可渗透");
            assert!(!is_liquid_permeable(cells, 1), "液体不可渗透（state&3==2）");
            assert!(!is_liquid_permeable(cells, 2), "固体不可渗透");
            // properties bit1 置位 → 即使气体元素也不可渗透（原版 L607）
            cells.element_idx.set(3, 0); // 气体
            cells.properties.set(3, 2); // 固体标志
            assert!(!is_liquid_permeable(cells, 3), "properties&2 置位时不可渗透");
        }
        DestroyElementsTable();
    }

    /// IsSolid：固体（state & 3 == 3）被识别；气体不是固体；
    /// properties bit1（&2）置位时，非固体元素也算固体（原版 L639-644）。
    #[test]
    fn is_solid_detects_unobtanium() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(0, 2); // 固体
            cells.element_idx.set(1, 0); // 气体
            assert!(is_solid(cells, 0), "固体应被识别");
            assert!(!is_solid(cells, 1), "气体不是固体");
            // properties&2 置位的气体 → 固体（原版 L639-644）
            cells.element_idx.set(2, 0); // 气体
            cells.properties.set(2, 2); // 固体标志
            assert!(is_solid(cells, 2), "properties&2 置位时非固体元素也算固体");
        }
        DestroyElementsTable();
    }

    /// 太空真空特判（2026-08-04 实现，原版 UpdateData L145466-145520）：
    /// worldZones[cell]==0xFF（世界外部/太空）且背墙真空 →
    /// 液体每帧减 1000×0.02=20kg、气体每帧减 1.0×0.02=0.02kg、固体跳过；
    /// flow 重置 {-0.3, 0.3, 0.3, -0.3}。
    #[test]
    fn run_space_vacuum_task_removes_liquid_gas_skips_solid() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        // 模拟 SetWorldZones 后：内部格 cell14/15/16 标记太空（0xFF）
        let mut wz = vec![0u8; 30];
        wz[14] = 0xFF;
        wz[15] = 0xFF;
        wz[16] = 0xFF;
        sd.world_zones = UniquePtr { ptr: wz.leak().as_mut_ptr() };
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1); // 液体 state=2
            updated.mass.set(14, 500.0);
            updated.element_idx.set(15, 0); // 气体 state=1
            updated.mass.set(15, 1.0);
            updated.element_idx.set(16, 2); // 固体 state=3
            updated.mass.set(16, 100.0);
            // backwall 默认填 vacuum（new_for_allocate with_size(vacuum_element_idx)）
        }

        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_space_vacuum_task(&mut sd, bounds);

        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(14) - 480.0).abs() < 1e-3,
                "液体 500 应减 20 → 480，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.mass.get(15) - 0.98).abs() < 1e-3,
                "气体 1.0 应减 0.02 → 0.98，got {}",
                updated.mass.get(15)
            );
            assert!(
                (updated.mass.get(16) - 100.0).abs() < 1e-3,
                "固体不应被删除"
            );
            // flow 重置 ±0.3
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert_eq!(flow[14].x, -0.3);
            assert_eq!(flow[14].y, 0.3);
            assert_eq!(flow[14].z, 0.3);
            assert_eq!(flow[14].w, -0.3);
        }
        DestroyElementsTable();
    }

    /// 太空真空特判的门控：非太空格（worldZones != 0xFF）与背墙非真空格不删除。
    #[test]
    fn run_space_vacuum_task_gates_on_world_zone_and_backwall() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let mut wz = vec![0u8; 30];
        // cell14 太空；cell15 非太空（0）；cell16 太空但背墙非真空
        wz[14] = 0xFF;
        wz[16] = 0xFF;
        sd.world_zones = UniquePtr { ptr: wz.leak().as_mut_ptr() };
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1); // 液体，背墙真空 + 太空 → 应删除
            updated.mass.set(14, 200.0);
            updated.element_idx.set(15, 1); // 液体，非太空 → 不应删除
            updated.mass.set(15, 200.0);
            // cell16 太空但背墙非真空 → 不应删除
            updated.element_idx.set(16, 1);
            updated.mass.set(16, 200.0);
            let bw = &mut *sd.backwall.ptr;
            bw.element_idx.set(16, 5); // 非真空背墙
        }

        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_space_vacuum_task(&mut sd, bounds);

        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(14) - 180.0).abs() < 1e-3, "太空+背墙真空应删除");
            assert!((updated.mass.get(15) - 200.0).abs() < 1e-3, "非太空格不应删除");
            assert!((updated.mass.get(16) - 200.0).abs() < 1e-3, "背墙非真空不应删除");
        }
        DestroyElementsTable();
    }

    /// 元素表缺失时两函数安全返回 false，不 panic。
    /// 持 LIB_TESTS_LOCK 并先销毁全局表，确保与并行建表测试无竞态。
    #[test]
    fn empty_table_returns_safe_defaults() {
        let _lock = LIB_TESTS_LOCK.lock();
        DestroyElementsTable(); // 确保表为空（测试间可能残留）
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &*sd.cells.ptr;
            assert!(!is_liquid_permeable(cells, 0), "空表下应安全返回 false");
            assert!(!is_solid(cells, 0), "空表下应安全返回 false");
            // vacuum/unobtanium 索引字段由 new_for_allocate 设置（任务 2）
            let _ = sd.vacuum_element_idx;
            let _ = sd.unobtanium_element_idx;
        }
    }

    // ===== 任务 6：UpdateLiquid + UpdateNeighbourLiquidMass =====

    /// 自建 2 元素最小表：0=真空(state=0)，1=液体(state=2, flow=50, max_mass=1000, min_vertical_flow=0)。
    /// 调用方需持 LIB_TESTS_LOCK，并在测试末尾 DestroyElementsTable() 清理。
    fn create_liquid_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(2);
        for elem in [
            Element::default(), // 0=真空
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 2; // 液体
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0; // 流率上限（原版 UpdateLiquid 读 viscosity，2026-08-02 修复后）
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0; // 温度范围放宽（液滴 spawn 温度检查，测试用 300K）
                e.high_temp = 10000.0;
                e
            },
        ] {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(&elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..2 {
            w.write_int(0); // 空名称
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 简报基线：空元素表下 update_liquids 不 panic。
    /// 强行为断言在下方自建表测试中覆盖。
    #[test]
    fn update_liquid_moves_mass_down_to_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        DestroyElementsTable(); // 元素表为空
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        update_liquids(&mut sd); // 空表下不应 panic
    }

    /// 同元素下落 + 左扩散：源格液体(100g) 下方同液体(10g)，左邻真空。
    /// 向下：fVar22 = max(100*1.01, 1000) - 10 = 990（原版 L725-732 是 max，2026-08-02 修复）；
    ///       fVar23 = min(min(100, 50), 990*0.5) = 50（viscosity 上限）→ 源格递减到 50。
    /// 左扩散（原版 L849-945）：left(13) 真空 → fVar23 = min(min(50, 50), 50×0.25) = 12.5；
    ///       源格再减 12.5 → 最终 37.5；左邻变液体 mass=12.5；flow[LEFT] 累加 12.5。
    /// 事件：左邻元素 0→1 产生 1 条 substance 事件（sim13 → game 4）。
    #[test]
    fn update_liquid_same_element_transfers_mass_and_flow() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                // 源格 (row2,col2)=14：液体 mass=100
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                // 下方格 (row1,col2)=8：液体 mass=10
                buf.element_idx.set(8, 1);
                buf.mass.set(8, 10.0);
                buf.temperature.set(8, 300.0);
                // cell13（左邻）= 真空（元素 0，质量 0，默认）
                // cell15（右邻）= 固体（任务 2 修正：右分支实现后真空会触发右扩散，
                //   源格 40.875 断言会被破坏 → 补固体阻断右分支）
                buf.element_idx.set(15, 4);
                buf.mass.set(15, 500.0);
                buf.temperature.set(15, 300.0);
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // 向下转移 50 + 左扩散 12.5
            assert!(
                (updated.mass.get(14) - 37.5).abs() < 1e-3,
                "源格质量应 100-50-12.5=37.5，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.mass.get(8) - 60.0).abs() < 1e-3,
                "下方格质量应 10+50=60，got {}",
                updated.mass.get(8)
            );
            // 左扩散（原版 L849-945）：左邻得 min(min(50,50),50×0.25)=12.5
            assert_eq!(updated.element_idx.get(13), 1, "左邻应变为液体元素");
            assert!(
                (updated.mass.get(13) - 12.5).abs() < 1e-3,
                "左邻应得 12.5，got {}",
                updated.mass.get(13)
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!(
                (flow[14].z - 50.0).abs() < 1e-3,
                "flow[DOWN] 应累加 50，got {}",
                flow[14].z
            );
            assert!(
                (flow[14].x - 12.5).abs() < 1e-3,
                "flow[LEFT] 应累加 12.5，got {}",
                flow[14].x
            );
            // 左邻元素 0→1 变化 → 1 条 substance 事件（sim13 → game (13%6-1)+(13/6-1)*4 = 4）
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1, "左扩散应产生 1 条事件");
            assert_eq!(events.substance_change_info.get(0).cell_idx, 4);
        }
        DestroyElementsTable();
    }

    /// 复现（2026-08-06 用户实测）：液体水平挤入气体格时，气体正交四路全堵，
    /// 应沿**右上方对角**逃逸（原版 DisplaceGas 循环 2，11_msvcrt_ignored.c
    /// L34093-34121：diag = cell+width±1，同侧平格非固体 + properties&1==0）。
    /// 布局（6×5，+width=上，源格 14）：
    ///   row3: [18 固] [19 固] [20 气] [21 ..]
    ///   row2: [12 固] [13 气] [14 液] [15 固]
    ///   row1: [ 7 固] [ 8 液] [ 9 ..]
    /// 左扩散 12.5kg → 目标 13 是气体 → displace_gas：正交（19/12/14/7）全堵 →
    /// 对角 18 固体跳过 → 20（同气体、旁格 14 非固体）→ 气体整体移到 20。
    #[test]
    fn liquid_spreads_into_gas_and_gas_escapes_diagonally() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 表：0=真空(state0)、1=液体(state2)、2=气体(state1)、3=固体(state3)
        let mut w = BinaryBufferWriter::new();
        w.write_int(4);
        for elem in [
            Element::default(),
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 2;
                e.state = 1; // 气体
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 3;
                e.state = 3; // 固体
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
        ] {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(&elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..4 {
            w.write_int(0); // 空名称
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null());

        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                // 源格液体 14 (row2,col2)
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                // 下方同液体 8 (row1,col2)——同元素下落分支，随后水平扩散
                buf.element_idx.set(8, 1);
                buf.mass.set(8, 10.0);
                buf.temperature.set(8, 300.0);
                // 左邻气体 13 (row2,col1)
                buf.element_idx.set(13, 2);
                buf.mass.set(13, 10.0);
                buf.temperature.set(13, 300.0);
                // 气体 13 的逃逸路：上 19 / 左 12 / 下 7 = 固体；右 14 = 液体（源）
                for c in [19usize, 12, 7] {
                    buf.element_idx.set(c, 3);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
                // 对角：左上 18 = 固体（强制走右上）；右上 20 = 同气体（逃逸目标）
                buf.element_idx.set(18, 3);
                buf.mass.set(18, 500.0);
                buf.temperature.set(18, 300.0);
                buf.element_idx.set(20, 2);
                buf.mass.set(20, 0.0);
                buf.temperature.set(20, 300.0);
                // 右邻 15 = 固体（阻断右扩散）
                buf.element_idx.set(15, 3);
                buf.mass.set(15, 500.0);
                buf.temperature.set(15, 300.0);
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(
                updated.element_idx.get(13),
                1,
                "目标气体格 13 应被液体占据（挤走后变液体）"
            );
            assert!(updated.mass.get(13) > 0.0, "13 应获得左扩散的液体质量");
            assert_eq!(
                updated.element_idx.get(20),
                2,
                "气体应逃逸到右上对角 20"
            );
            assert!(
                (updated.mass.get(20) - 10.0).abs() < 1e-3,
                "20 应收到原 13 的 10kg 气体，got {}",
                updated.mass.get(20)
            );
        }
        DestroyElementsTable();
    }

    /// 液体压力位移测试表：0=真空(state0)、1=液体A(state2,max_mass 1000)、
    /// 2=液体B(state2,max_mass 1000)、3=气体(state1)、4=固体(state3)。
    fn create_liquid_displacement_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(5);
        let elems = [
            Element::default(),
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 2;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 3;
                e.state = 1; // 气体
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 4;
                e.state = 3; // 固体
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
        ];
        for elem in &elems {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..5 {
            w.write_int(0);
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 无 off-gas 的密度分层表：0=真空、1=重液(molar 200)、2=轻液(molar 18)、3=气体、4=固体。
    /// sublimate_probability 默认 0 → 重液不会 off-gas，聚焦质量消失 bug。
    fn create_clean_density_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(5);
        let elems = [
            Element::default(),
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.molar_mass = 200.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 2;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.molar_mass = 18.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 3;
                e.state = 1;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 4;
                e.state = 3;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
        ];
        for elem in &elems {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..5 {
            w.write_int(0); // 空名称
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 回归：DisplaceLiquidDirectional 合并分支必须要求 src 与 dst（beyond）**同元素**
    /// （原版 L146461-146464）。旧实现误用 src 自身快照元素 → 污染水小格被水挤压时
    /// 并进水格（add_mass 不改元素）→ 污染水质量变水消失。
    #[test]
    fn displace_liquid_directional_merge_requires_same_element() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_clean_density_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                buf.element_idx.set(13, 1); // 被挤压格：重液（污染水）
                buf.mass.set(13, 100.0);
                buf.temperature.set(13, 300.0);
                buf.element_idx.set(12, 2); // beyond：轻液（水）
                buf.mass.set(12, 100.0);
                buf.temperature.set(12, 300.0);
            }
        }
        // 异元素 beyond → 不得合并（旧实现会合并 → 污染水变水）
        let ok = displace_liquid_directional(&mut sd, 13, 12);
        assert!(!ok, "beyond 为水时不得把污染水并入");
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.element_idx.get(13), 1, "污染水应留在原格");
            assert!((u.mass.get(13) - 100.0).abs() < 1e-3);
            assert_eq!(u.element_idx.get(12), 2, "水格元素不变");
            assert!((u.mass.get(12) - 100.0).abs() < 1e-3);
        }
        // 同元素 beyond → 正常合并（污染水并入污染水）
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(12, 1);
            u.mass.set(12, 50.0);
        }
        let ok2 = displace_liquid_directional(&mut sd, 13, 12);
        assert!(ok2, "beyond 同为污染水时应合并");
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.element_idx.get(12), 1);
            assert!((u.mass.get(12) - 150.0).abs() < 1e-3, "100+50 合并进 beyond");
            assert_eq!(u.element_idx.get(13), 0, "被挤压格清空为真空");
        }
        DestroyElementsTable();
    }

    /// 回归：污染水（重液）从高空落入大水池，扩散出的薄格被水压力挤压时
    /// 不再消失（旧 bug：被挤进水格 → 质量变水）。seed=17 曾丢失 950kg。
    /// 400 tick 后污染水总质量必须守恒为 1000kg。
    #[test]
    fn polluted_sinking_conserves_mass() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_clean_density_table();
        let mut sd = SimData::new_for_allocate(20, 14, 17, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                for i in 0..280usize {
                    buf.element_idx.set(i, 0);
                    buf.mass.set(i, 0.0);
                    buf.temperature.set(i, 300.0);
                }
                // 固体底 row1 cols 3..16
                for c in 3..17usize {
                    let idx = 1 * 20 + c;
                    buf.element_idx.set(idx, 4);
                    buf.mass.set(idx, 1000.0);
                }
                // 水池 rows 2..8 cols 3..16（水面 row8）
                for r in 2..9usize {
                    for c in 3..17usize {
                        let idx = r * 20 + c;
                        buf.element_idx.set(idx, 2);
                        buf.mass.set(idx, 1000.0);
                    }
                }
                // 污染水 row10 col9（idx 209）；row9 col9（idx 189）真空间隔
                buf.element_idx.set(209, 1);
                buf.mass.set(209, 1000.0);
            }
        }
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        let mut min_pt = 1000.0f32;
        for _tick in 0..400 {
            unsafe {
                crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd);
            }
            update_liquid_loop(&mut sd, bounds);
            run_liquid_displacement_task(&mut sd, bounds);
            unsafe {
                crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd);
            }
            post_process_loop(&mut sd, bounds);
            unsafe {
                let u = &*sd.updated_cells.ptr;
                let mut pt = 0.0f32;
                for i in 0..280usize {
                    if u.element_idx.get(i) == 1 {
                        pt += u.mass.get(i);
                    }
                }
                min_pt = min_pt.min(pt);
            }
        }
        assert!(
            min_pt >= 999.5,
            "污染水总质量不应丢失（最小 {min_pt}，seed=17 旧实现会掉到 50）"
        );
        unsafe {
            let u = &*sd.updated_cells.ptr;
            let mut pt = 0.0f32;
            for i in 0..280usize {
                if u.element_idx.get(i) == 1 {
                    pt += u.mass.get(i);
                }
            }
            assert!(
                (pt - 1000.0).abs() < 0.2,
                "400 tick 后污染水应守恒为 1000kg，got {pt}"
            );
        }
        DestroyElementsTable();
    }

    /// DoLiquidPressureDisplacement 核心（beyond 为液体合并路径）：
    /// src(14)=液体A 1000kg、dst(13)=液体B 100kg、beyond(12)=液体B 50kg。
    /// f15 = 1000 − 0 = 1000；1000 > 50+100 → 位移；
    /// displacement = 1000×0.125 = 125 → dst 变液体A 125、src 875、
    /// beyond 合并为 150（100 并入 50）、dst 温度覆盖为源格温度。
    #[test]
    fn do_liquid_pressure_displacement_squeezes_to_liquid_beyond() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1); // 液体A 源
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(13, 2); // 液体B 邻格
                buf.mass.set(13, 100.0);
                buf.temperature.set(13, 200.0);
                buf.element_idx.set(12, 2); // 液体B beyond
                buf.mass.set(12, 50.0);
                buf.temperature.set(12, 200.0);
            }
        }
        let r = do_liquid_pressure_displacement(&mut sd, 1, 14, 13, 12);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((r - 125.0).abs() < 1e-3, "位移量应 125，got {r}");
            assert_eq!(updated.element_idx.get(13), 1, "邻格应变液体A");
            assert!(
                (updated.mass.get(13) - 125.0).abs() < 1e-3,
                "邻格应得 125，got {}",
                updated.mass.get(13)
            );
            assert!(
                (updated.mass.get(14) - 875.0).abs() < 1e-3,
                "源格应扣 125 → 875，got {}",
                updated.mass.get(14)
            );
            assert_eq!(updated.element_idx.get(12), 2, "beyond 保持液体B");
            assert!(
                (updated.mass.get(12) - 150.0).abs() < 1e-3,
                "beyond 应合并 100 → 150，got {}",
                updated.mass.get(12)
            );
            assert!(
                (updated.temperature.get(13) - 300.0).abs() < 1e-3,
                "邻格温度应覆盖为源格 300，got {}",
                updated.temperature.get(13)
            );
        }
        DestroyElementsTable();
    }

    /// DoLiquidPressureDisplacement（beyond 为气体）：DisplaceGas 把气体挤到逃逸格
    /// （18=上方同气体），SwapCells 把邻格液体B 整体搬入 beyond(12)，随后 125 灌入邻格。
    #[test]
    fn do_liquid_pressure_displacement_gas_beyond_displaces_gas() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(13, 2);
                buf.mass.set(13, 100.0);
                buf.temperature.set(13, 200.0);
                buf.element_idx.set(12, 3); // beyond = 气体
                buf.mass.set(12, 10.0);
                buf.temperature.set(12, 250.0);
                buf.element_idx.set(18, 3); // 气体逃逸格（上方同气体）
                buf.mass.set(18, 0.0);
                buf.temperature.set(18, 250.0);
                // 气体 12 的正交候选：左 11 / 右 13（液体B）/ 下 6 全堵 → 走上方 18
                for c in [11usize, 6] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let r = do_liquid_pressure_displacement(&mut sd, 1, 14, 13, 12);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((r - 125.0).abs() < 1e-3, "位移量应 125，got {r}");
            assert_eq!(updated.element_idx.get(18), 3, "气体应逃逸到 18");
            assert!(
                (updated.mass.get(18) - 10.0).abs() < 1e-3,
                "18 应收到 10kg 气体，got {}",
                updated.mass.get(18)
            );
            assert_eq!(updated.element_idx.get(12), 2, "beyond 应收到被挤的液体B");
            assert!(
                (updated.mass.get(12) - 100.0).abs() < 1e-3,
                "12 应收到 100kg 液体B，got {}",
                updated.mass.get(12)
            );
            assert_eq!(updated.element_idx.get(13), 1, "邻格应变液体A");
            assert!((updated.mass.get(13) - 125.0).abs() < 1e-3);
            assert!((updated.mass.get(14) - 875.0).abs() < 1e-3);
        }
        DestroyElementsTable();
    }

    /// 门控：dst 同元素 / dst 非液体（气体）/ 质量不足 / dst 不可渗透 → 全部返回 0 且无副作用。
    #[test]
    fn do_liquid_pressure_displacement_gates() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(13, 1); // 与源同元素
                buf.mass.set(13, 100.0);
                buf.temperature.set(13, 200.0);
                buf.element_idx.set(12, 2);
                buf.mass.set(12, 50.0);
            }
        }
        // ① dst 同元素 → 0
        let r1 = do_liquid_pressure_displacement(&mut sd, 1, 14, 13, 12);
        assert!((r1 - 0.0).abs() < 1e-6, "同元素邻格不位移，got {r1}");
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 1);
            assert!((updated.mass.get(14) - 1000.0).abs() < 1e-6);
        }
        // ② dst 为气体（非液体）→ 0
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(13, 3);
                buf.mass.set(13, 10.0);
            }
        }
        let r2 = do_liquid_pressure_displacement(&mut sd, 1, 14, 13, 12);
        assert!((r2 - 0.0).abs() < 1e-6, "气体邻格不位移，got {r2}");
        // ③ 质量不足：beyond(50) + dst(100) >= f15（源改为 100）
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(13, 2);
                buf.mass.set(13, 100.0);
                buf.mass.set(14, 100.0);
            }
        }
        let r3 = do_liquid_pressure_displacement(&mut sd, 1, 14, 13, 12);
        assert!((r3 - 0.0).abs() < 1e-6, "质量不足不位移，got {r3}");
        // ④ dst 不可渗透（properties bit1）
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.mass.set(14, 1000.0);
                buf.properties.set(13, 2);
            }
        }
        let r4 = do_liquid_pressure_displacement(&mut sd, 1, 14, 13, 12);
        assert!((r4 - 0.0).abs() < 1e-6, "不可渗透邻格不位移，got {r4}");
        DestroyElementsTable();
    }

    /// 位移循环集成：12×10 sim，src(53)=液体A 1000、左邻(52)=液体B 100、
    /// beyond(51)=液体B 50；iterate_direction=+1 → 左挤压，flow 记账
    /// flow[53].x += 125、flow[52].y -= 125。
    #[test]
    fn run_liquid_displacement_task_squeezes_and_accounts_flow() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(12, 10, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.iterate_direction = 1;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(53, 1); // (row4,col5) 液体A
                buf.mass.set(53, 1000.0);
                buf.temperature.set(53, 300.0);
                buf.element_idx.set(52, 2); // (row4,col4) 液体B
                buf.mass.set(52, 100.0);
                buf.temperature.set(52, 200.0);
                buf.element_idx.set(51, 2); // (row4,col3) beyond 液体B
                buf.mass.set(51, 50.0);
                buf.temperature.set(51, 200.0);
                // 阻断右/下：54 固体、41 固体
                for c in [54usize, 41] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_liquid_displacement_task(&mut sd, b);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(52), 1, "左邻应变液体A");
            assert!(
                (updated.mass.get(52) - 125.0).abs() < 1e-3,
                "左邻应得 125，got {}",
                updated.mass.get(52)
            );
            assert_eq!(updated.element_idx.get(51), 2, "beyond 保持液体B");
            assert!(
                (updated.mass.get(51) - 150.0).abs() < 1e-3,
                "beyond 应合并 150，got {}",
                updated.mass.get(51)
            );
            assert!(
                (updated.mass.get(53) - 875.0).abs() < 1e-3,
                "源格应 875，got {}",
                updated.mass.get(53)
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 12 * 10);
            assert!(
                (flow[53].x - 125.0).abs() < 1e-3,
                "flow[53].x(LEFT) 应 +125，got {}",
                flow[53].x
            );
            assert!(
                (flow[52].y - (-125.0)).abs() < 1e-3,
                "flow[52].y(RIGHT) 应 −125，got {}",
                flow[52].y
            );
        }
        DestroyElementsTable();
    }

    /// 病菌 12.5% 转移：src 病菌 800 → 转移 100，dst 得 100、src 剩 700。
    #[test]
    fn do_liquid_pressure_displacement_transfers_disease() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
                buf.disease_idx.set(14, 0x05);
                buf.disease_count.set(14, 800);
                buf.element_idx.set(13, 2);
                buf.mass.set(13, 100.0);
                buf.temperature.set(13, 200.0);
                buf.disease_idx.set(13, 0x08);
                buf.disease_count.set(13, 0);
                buf.element_idx.set(12, 2);
                buf.mass.set(12, 50.0);
                buf.temperature.set(12, 200.0);
            }
        }
        let r = do_liquid_pressure_displacement(&mut sd, 1, 14, 13, 12);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((r - 125.0).abs() < 1e-3);
            assert_eq!(updated.disease_count.get(13), 100, "dst 应得 100 病菌");
            assert_eq!(updated.disease_idx.get(13), 0x05, "dst 病菌 idx 应为源病菌");
            assert_eq!(updated.disease_count.get(14), 700, "src 应剩 700 病菌");
        }
        DestroyElementsTable();
    }

    /// 复现（2026-08-06 用户实测）：密封竖井 [气1 下][液1 中][液2 上]（同元素液体），
    /// 恢复运行时气体应被**传送**到液2位置（原版 update_liquid 分支 3b SwapCells
    /// 级联，跨两帧：帧1 液1↔气1 交换（气体升到中部）；帧2 液2↔气体 交换
    /// （气体升到顶部=液2原位）。分支判定读 cells 稳定快照，故单帧内不级联）。
    /// 布局（6×5，+width=上）：
    ///   row3: [19 固] [20 液2] [21 固]
    ///   row2: [13 固] [14 液1] [15 固]
    ///   row1: [ 7 固] [ 8 气1] [ 9 固]
    ///   row0: [     ] [ 2 固] [    ]
    #[test]
    fn enclosed_vertical_shaft_gas_teleports_to_top() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                // 竖井液体柱：8=气体、14=液1、20=液2
                buf.element_idx.set(8, 3);
                buf.mass.set(8, 10.0);
                buf.temperature.set(8, 250.0);
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
                // 砖块封闭：左右侧 + 井底
                for c in [7usize, 9, 13, 15, 19, 21, 2] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let b = crate::d1_activity::full_grid_bounds(&sd);
        // 帧 1：液1↔气1 交换
        update_liquid_loop(&mut sd, b);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 3, "帧1后气体应在中部 14");
            assert_eq!(updated.element_idx.get(8), 1, "帧1后底部应为液体");
        }
        // 同步双缓冲后进入帧 2：液2↔气体 交换
        unsafe { crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd); }
        update_liquid_loop(&mut sd, b);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(
                updated.element_idx.get(20),
                3,
                "气体应被传送到顶部液2位置 20"
            );
            assert_eq!(updated.element_idx.get(14), 1, "中部应为液体");
            assert_eq!(updated.element_idx.get(8), 1, "底部应为液体");
        }
        DestroyElementsTable();
    }

    /// 用户字面布局（水平相邻）：气1 在 (1,1)，液1 在其右边 (1,2)，液2 在液1上方
    /// (2,2)，三格被砖块封闭。阶段 E（对角换位）实现后：液2 与左下对角气1 整格
    /// 换位 → 气体到 14、液2 到 7（用户实测原版行为）。
    #[test]
    fn horizontal_gas_liquid_corner_enclosed_behavior() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(7, 3); // (1,1) 气1
                buf.mass.set(7, 10.0);
                buf.temperature.set(7, 250.0);
                buf.element_idx.set(8, 1); // (1,2) 液1
                buf.mass.set(8, 100.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(14, 1); // (2,2) 液2（液1 上方，同元素）
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                // 封闭砖块：气1 上/左/下，液1 上/下，液2 左/右/上
                for c in [13usize, 15, 9, 3, 2, 1, 6, 12] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let b = crate::d1_activity::full_grid_bounds(&sd);
        for _ in 0..20 {
            update_liquid_loop(&mut sd, b);
            unsafe { crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd); }
        }
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            let gas_at = (0..30)
                .filter(|&c| updated.element_idx.get(c) == 3)
                .collect::<Vec<_>>();
            assert!(
                gas_at.len() == 1 && gas_at[0] == 14,
                "阶段 E 对角换位后气体应在 14（液2 原位），got {gas_at:?}"
            );
        }
        DestroyElementsTable();
    }

    /// 正确布局复现（2026-08-06 用户精确坐标 (x,y)，y 向上）：
    /// 气1(1,1)=1kg、液1(1,2)=1000kg（液1 在气1 **正上方**）、液2(2,2)=300kg
    /// （液2 在液1 右侧），三格隔热砖封闭。
    /// 原版机制 = update_liquid 分支 3 角落检查 + SwapCells：
    /// - 液2 为液体（不可渗透）→ 液1 右下无角落 → SwapCells(液1,气1) → 气体上升到
    ///   液1 原位；液2 为真空（可渗透）→ 右下形成角落 → SpawnFallingLiquid 液滴 →
    ///   气体**不动**（用户实测"真空时不转移"）。
    #[test]
    fn gas_above_correct_layout_swap_vs_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let w = 12usize;
        let gas = 1 * w + 1; // (x=1,y=1) 气1
        let liq1 = gas + w; // (x=1,y=2) 液1（气1 正上方）
        let liq2 = liq1 + 1; // (x=2,y=2) 液2（液1 右侧）
        // 隔热砖封闭（气1/液1/液2 三格周围；气1=13、液1=25、液2=26）
        let bricks = [
            liq1 - 1,     // (2,0) 液1 左
            liq2 + 1,     // (2,3) 液2 右
            liq1 + w,     // (3,1) 液1 上
            liq2 + w,     // (3,2) 液2 上
            liq1 + w - 1, // (3,0) 液1 左上
            gas - 1,      // (1,0) 气1 左
            gas - w,      // (0,1) 气1 下
            gas - w - 1,  // (0,0) 气1 左下
            gas - w + 1,  // (0,2) 气1 右下
            liq2 - w,     // (1,2) 液2 下
            liq2 - w + 1, // (1,3) 液2 右下
            liq2 + w + 1, // (3,3) 液2 右上
        ];
        for (liq2_elem, liq2_mass, expect_gas_at) in
            [(1u16, 300.0f32, liq1), (0u16, 0.0f32, gas)]
        {
            let mut sd = SimData::new_for_allocate(12, 10, 12345, false, true);
            sd.vacuum_element_idx = 0;
            sd.void_element_idx = 0xFFFF;
            unsafe {
                for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                    buf.element_idx.set(gas, 3);
                    buf.mass.set(gas, 1.0);
                    buf.temperature.set(gas, 250.0);
                    buf.element_idx.set(liq1, 1);
                    buf.mass.set(liq1, 1000.0); // 满格
                    buf.temperature.set(liq1, 300.0);
                    buf.element_idx.set(liq2, liq2_elem);
                    buf.mass.set(liq2, liq2_mass);
                    buf.temperature.set(liq2, 300.0);
                    for &c in &bricks {
                        buf.element_idx.set(c, 4);
                        buf.mass.set(c, 500.0);
                        buf.temperature.set(c, 300.0);
                    }
                }
            }
            sd.active_regions.push(crate::a_framework::sim_data::ActiveRegion {
            min_x: 0,
            min_y: 0,
            max_x: 12,
            max_y: 10,
                current_sunlight_intensity: 0.0,
                current_cosmic_radiation_intensity: 0.0,
            });
            for _ in 0..5 {
                crate::c2_physics::update_data(&mut sd);
            }
            unsafe {
                let updated = &*sd.updated_cells.ptr;
                let gas_cells = (0..(12 * 10))
                    .filter(|&c| updated.element_idx.get(c) == 3)
                    .collect::<Vec<_>>();
                assert!(
                    gas_cells.len() == 1 && gas_cells[0] == expect_gas_at,
                    "liq2_elem={liq2_elem}: 气体应在 {expect_gas_at}，got {gas_cells:?}"
                );
            }
        }
        DestroyElementsTable();
    }

    /// 竖井 [气1 下][液1 中][液2 上]，液1=液2=1000kg（满格）、气1=1kg。
    /// 自下而上遍历：帧1 液1 分支 3 与气1 交换（气体升到中部）；帧2 液2 分支 3
    /// 与气体交换（气体升到顶部）。注：竖井内液2 的分支 1 目标格（中部）已在
    /// 帧1 变气体 → displace_gas 失败 → 竖井场景**无 5kg 合并**（合并只出现在
    /// 用户 L 布局——液1 被水平阻挡不动，见 diagonal_swap_liquid_swaps_with_below_left_gas）。
    #[test]
    fn full_column_merge_then_gas_swap_cascade() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(8, 3); // (1,2) 气1
                buf.mass.set(8, 1.0);
                buf.temperature.set(8, 250.0);
                buf.element_idx.set(14, 1); // (2,2) 液1（满格）
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1); // (3,2) 液2（满格）
                buf.mass.set(20, 1000.0);
                buf.temperature.set(20, 300.0);
                for c in [7usize, 9, 13, 15, 19, 21, 2] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let b = crate::d1_activity::full_grid_bounds(&sd);
        // 帧 1：液1（row2）先处理——分支 3 与气1 交换；液2（row3）后处理——
        // 分支 1 判定读 cells（稳定快照）仍是液1 → 转移 5kg 到 updated 里的气体格？？
        update_liquid_loop(&mut sd, b);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            let gas_cells = (0..30)
                .filter(|&c| updated.element_idx.get(c) == 3)
                .collect::<Vec<_>>();
            assert_eq!(updated.element_idx.get(14), 3, "帧1后液1↔气1 交换，气体在中部 14");
        }
        // 帧 2：液2 的分支判定（cells 已同步）看到下方是气体 → 分支 3 交换
        unsafe { crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd); }
        update_liquid_loop(&mut sd, b);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            let gas_cells = (0..30)
                .filter(|&c| updated.element_idx.get(c) == 3)
                .collect::<Vec<_>>();
            assert_eq!(updated.element_idx.get(20), 3, "气体应被顶到顶部 20（液2 原位）");
        }
        DestroyElementsTable();
    }

    /// 用户精确布局（2026-08-06，原版 1 秒后实测）：
    /// 气1(1,1)=7、液1(2,1)=8、液2(2,2)=14、砖块(1,2)=13，三格砖封。
    /// 原版：液2 与**左下对角**气1 整格换位（update_liquid 阶段 E L1128-1205）→
    /// 气1→(2,2)、液2→(1,1)、液1 不动；液2 位置为真空则气1 不动。
    /// 满格 1000kg：液2 先分支 1 合并 5kg（→995），随后阶段 E 对角换位到 7。
    /// 此测试直接调 update_liquid(14) 验证"合并 + 对角换位"一步到位。
    #[test]
    fn diagonal_swap_liquid_swaps_with_below_left_gas() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(7, 3); // (1,1) 气1
                buf.mass.set(7, 1.0);
                buf.temperature.set(7, 250.0);
                buf.element_idx.set(8, 1); // (2,1) 液1
                buf.mass.set(8, 1000.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(14, 1); // (2,2) 液2
                buf.mass.set(14, 1000.0); // 满格 → 先合并 5kg
                buf.temperature.set(14, 300.0);
                // 砖封：气1 上(13)/左(6)/下(1)；液1 下(2)/右(9)；液2 右(15)/上(20)/左(13)
                for c in [13usize, 6, 1, 2, 9, 15, 20, 0, 3, 21, 19] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 3, "气1 应换位到 14（液2 原位）");
            assert_eq!(updated.element_idx.get(7), 1, "液2 应换位到 7（气1 原位）");
            assert_eq!(updated.element_idx.get(8), 1, "液1 不动");
            assert!((updated.mass.get(14) - 1.0).abs() < 1e-3, "14 应有 1kg 气体");
            assert!(
                (updated.mass.get(7) - 995.0).abs() < 1e-3,
                "7 应含 995kg 液2（先合并 5kg 给液1），got {}",
                updated.mass.get(7)
            );
            assert!(
                (updated.mass.get(8) - 1005.0).abs() < 1e-3,
                "液1 应收到 5kg → 1005，got {}",
                updated.mass.get(8)
            );
        }
        DestroyElementsTable();
    }

    /// 全管线（update_data 5 帧）用户精确布局：300kg 液2 直接对角换位（位置断言）。
    #[test]
    fn diagonal_swap_full_pipeline_300kg() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(7, 3);
                buf.mass.set(7, 1.0);
                buf.temperature.set(7, 250.0);
                buf.element_idx.set(8, 1);
                buf.mass.set(8, 1000.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 1000.0); // 用户"液2=1000kg"用例
                buf.temperature.set(14, 300.0);
                for c in [13usize, 6, 1, 2, 9, 15, 20, 0, 3, 21, 19] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let b = crate::d1_activity::full_grid_bounds(&sd);
        for _ in 0..5 {
            crate::c2_physics::update_data(&mut sd);
        }
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 3, "气1 应到 14");
            assert_eq!(updated.element_idx.get(7), 1, "液2 应到 7");
            assert_eq!(updated.element_idx.get(8), 1, "液1 不动");
        }
        DestroyElementsTable();
    }

    /// 满格 1000/1000 变体（位置断言；后续帧水平转移会再分配质量，不做精确质量断言）。
    #[test]
    fn diagonal_swap_full_pipeline_1000kg_merge_then_swap() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(7, 3);
                buf.mass.set(7, 1.0);
                buf.temperature.set(7, 250.0);
                buf.element_idx.set(8, 1);
                buf.mass.set(8, 1000.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 1000.0); // 满格
                buf.temperature.set(14, 300.0);
                for c in [13usize, 6, 1, 2, 9, 15, 20, 0, 3, 21, 19] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let b = crate::d1_activity::full_grid_bounds(&sd);
        for _ in 0..5 {
            crate::c2_physics::update_data(&mut sd);
        }
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 3, "气1 应到 14");
            assert_eq!(updated.element_idx.get(7), 1, "液2 应到 7");
            assert_eq!(updated.element_idx.get(8), 1, "液1 不动");
        }
        DestroyElementsTable();
    }

    /// 液2 位置为真空 → 无液体执行对角换位 → 气1 不动（用户实测"真空不转移"）。
    #[test]
    fn diagonal_swap_vacuum_at_liquid2_no_transfer() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(7, 3);
                buf.mass.set(7, 1.0);
                buf.temperature.set(7, 250.0);
                buf.element_idx.set(8, 1);
                buf.mass.set(8, 1000.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(14, 0); // 液2 位置 = 真空
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
                for c in [13usize, 6, 1, 2, 9, 15, 20, 0, 3, 21, 19] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let b = crate::d1_activity::full_grid_bounds(&sd);
        for _ in 0..5 {
            crate::c2_physics::update_data(&mut sd);
        }
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(7), 3, "气1 应保持原位 7（无液2 执行换位）");
            assert_eq!(updated.element_idx.get(14), 0, "14 保持真空");
        }
        DestroyElementsTable();
    }

    /// 镜像布局：气体在液2 的**右下对角**（(3,1)=9），液1(2,1)=8、液2(2,2)=14、
    /// 砖块(3,2)=15，三格砖封。阶段 E 右下分支（cell−width+1）应与左下分支对称触发。
    #[test]
    fn diagonal_swap_mirror_below_right_gas() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(9, 3); // (3,1) 气1（液2 右下对角）
                buf.mass.set(9, 1.0);
                buf.temperature.set(9, 250.0);
                buf.element_idx.set(8, 1); // (2,1) 液1
                buf.mass.set(8, 1000.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(14, 1); // (2,2) 液2
                buf.mass.set(14, 300.0);
                buf.temperature.set(14, 300.0);
                // 砖封：气1 上(15)/右(10)/下(3)；液1 左(7)/下(2)；液2 左(13)/右(15)/上(20)
                for c in [15usize, 10, 3, 7, 2, 13, 20, 0, 1, 6, 16, 21, 19] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let b = crate::d1_activity::full_grid_bounds(&sd);
        for _ in 0..5 {
            crate::c2_physics::update_data(&mut sd);
        }
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 3, "气1 应换位到 14（液2 原位）");
            assert_eq!(updated.element_idx.get(9), 1, "液2 应换位到 9（气1 原位）");
            assert_eq!(updated.element_idx.get(8), 1, "液1 不动");
        }
        DestroyElementsTable();
    }

    /// 满格 1000/1000 回归（2026-08-06 用户实测"液2 换位后达 1200kg"根因）：
    /// 自下而上遍历时，换位帧（帧 0）液2 先分支 1 合并 5kg → 995，再阶段 E 换位；
    /// 液1（同帧先处理）左邻仍是 cells 快照中的气体 → displace_gas 被挡 → **帧内
    /// 无扩散**。帧 1 起液1 才开始水平均质化（2.5kg/帧 → 1000/1000）。
    /// 此前自上而下遍历：液2 先换位 → 液1 左分支从 cells 快照读气体质量 0 →
    /// f23=(1005−0)×0.25 被 viscosity 截断 → 灌入已换位的液2（同元素合并）→
    /// 液2 膨胀（测试表 viscosity=50 → 1045；游戏水=125 → 1120+，用户实测 1200）。
    #[test]
    fn diagonal_swap_no_mass_inflation_bottom_up() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_displacement_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(7, 3);
                buf.mass.set(7, 1.0);
                buf.temperature.set(7, 250.0);
                buf.element_idx.set(8, 1);
                buf.mass.set(8, 1000.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
                for c in [13usize, 6, 1, 2, 9, 15, 20, 0, 3, 21, 19] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                    buf.temperature.set(c, 300.0);
                }
            }
        }
        let b = crate::d1_activity::full_grid_bounds(&sd);
        crate::c2_physics::update_data(&mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 3, "换位帧内气体应已到 14");
            assert_eq!(updated.element_idx.get(7), 1, "液2 应已到 7");
            // 换位帧：无扩散膨胀（自上而下时此处为 1045）
            assert!(
                (updated.mass.get(7) - 995.0).abs() < 1e-3,
                "帧 0 液2 应恰为 995（合并 5kg 后换位，无帧内扩散），got {}",
                updated.mass.get(7)
            );
            assert!(
                (updated.mass.get(8) - 1005.0).abs() < 1e-3,
                "帧 0 液1 应恰为 1005，got {}",
                updated.mass.get(8)
            );
        }
        // 帧 1 起液1 才开始均质化（2.5kg/帧）
        crate::c2_physics::update_data(&mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(7) - 997.5).abs() < 1e-3,
                "帧 1 液2 应 997.5（液1 开始均质化），got {}",
                updated.mass.get(7)
            );
        }
        DestroyElementsTable();
    }

    /// 下落压缩修复（2026-08-02）：fVar22 = max(mass×1.01, max_mass) - below_mass。
    /// ① 上层 100 → 下层 990：f22 = max(101,1000)-990 = 10 → f23 = min(min(100,50),5) = 5
    ///    （下层逐步逼近满格 1000，而非 0.1kg 滴漏）。
    /// ② 超满格 1020 → 下层 100：f22 = max(1030.2,1000)-100 = 930.2 → f23 = 50
    ///    （旧 min 公式 f22 = min(1030.2,1000)-100 = 900 → 也非 0……等下，旧公式在 1020 时
    ///    min(1030.2,1000)=1000 → 1000-100=900 → 也能排。真正卡住的是"下层已满"场景：
    ///    1020 上层 + 1000 下层：旧 f22 = min(1030.2,1000)-1000 = 0 → 排不动；新 f22 = 30.2 → 排 15.1）。
    /// 本测试验证 ①（逼近满格）与 ②（下层满格时超满上层仍可排出）。
    #[test]
    fn update_liquid_drains_down_toward_full_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(8, 1);
                buf.mass.set(8, 990.0); // 下层接近满格
                buf.temperature.set(8, 300.0);
                // 左右均固体，隔离水平分支
                buf.element_idx.set(13, 4);
                buf.mass.set(13, 500.0);
                buf.element_idx.set(15, 4);
                buf.mass.set(15, 500.0);
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(8) - 995.0).abs() < 1e-3,
                "下层应 +5 到 995（逼近满格），got {}",
                updated.mass.get(8)
            );
            assert!(
                (updated.mass.get(14) - 95.0).abs() < 1e-3,
                "上层应 -5，got {}",
                updated.mass.get(14)
            );
        }
        // ② 超满格上层 1020 + 下层满格 1000 → 仍可向下排出（旧 min 公式 f22=0 卡死）
        let mut sd2 = SimData::new_for_allocate(6, 5, 1, true, false);
        sd2.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd2.cells.ptr;
            let updated = &mut *sd2.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 1020.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(8, 1);
                buf.mass.set(8, 1000.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(13, 4);
                buf.mass.set(13, 500.0);
                buf.element_idx.set(15, 4);
                buf.mass.set(15, 500.0);
                // cell20（上层）= 固体（隔离上推分支，只验证下落）
                buf.element_idx.set(20, 4);
                buf.mass.set(20, 500.0);
            }
        }
        update_liquid(&mut sd2, 14);
        unsafe {
            let updated = &*sd2.updated_cells.ptr;
            // f22 = max(1030.2,1000)-1000 = 30.2 → f23 = min(min(1020,50), 15.1) = 15.1
            assert!(
                (updated.mass.get(8) - 1015.1).abs() < 1e-3,
                "下层满格时上层 1020 仍应排出 15.1（旧 min 公式卡死），got {}",
                updated.mass.get(8)
            );
            assert!(
                (updated.mass.get(14) - 1004.9).abs() < 1e-3,
                "上层应 -15.1 到 1004.9，got {}",
                updated.mass.get(14)
            );
        }
        DestroyElementsTable();
    }

    /// 下方真空格 → SwapCells 交换 + 2 条 substance 事件（原版 L842-845）。
    #[test]
    fn update_liquid_vacuum_below_swaps_and_fires_events() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                buf.element_idx.set(14, 1); // 源格液体
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(8, 0); // 下方真空
                buf.mass.set(8, 0.0);
                buf.temperature.set(8, 0.0);
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 0, "源格应变为真空");
            assert_eq!(updated.element_idx.get(8), 1, "下方格应获得液体");
            assert!((updated.mass.get(8) - 100.0).abs() < 1e-3);
            assert!((updated.mass.get(14) - 0.0).abs() < 1e-3);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 2, "交换应产生 2 条事件");
            // game cell：sim14 → (2-1)+(2-1)*4 = 5；sim8 → (2-1)+(1-1)*4 = 1
            let mut cells: Vec<i32> = events
                .substance_change_info
                .as_slice()
                .iter()
                .map(|e| e.cell_idx)
                .collect();
            cells.sort();
            assert_eq!(cells, vec![1, 5]);
        }
        DestroyElementsTable();
    }

    /// 左扩散（设计测试 1）：液体格 mass=100，左邻真空，下方固体。
    /// 期望：左邻变液体、mass=25（100×0.25）、flow[cell].x=25、源格减 25。
    #[test]
    fn update_liquid_spreads_left_to_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        // 内部格坐标：cell14 = (row2,col2)。下方 cell8 = (row1,col2) 固体。
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1); // 液体元素 1（flow=50, max_mass=1000）
            cells.mass.set(14, 100.0);
            cells.temperature.set(14, 300.0);
            cells.element_idx.set(8, 4); // 固体元素 4
            cells.mass.set(8, 500.0);
            // cell15（右邻）= 固体（任务 2 修正：阻断右分支，保持左扩散 25/75 断言）
            cells.element_idx.set(15, 4);
            cells.mass.set(15, 500.0);
            // cell7（左下方对角）= 固体（液滴轨道：阻断液滴路径，保持格子转移断言）
            cells.element_idx.set(7, 4);
            cells.mass.set(7, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
            updated.element_idx.set(15, 4);
            updated.mass.set(15, 500.0);
            updated.element_idx.set(7, 4);
            updated.mass.set(7, 500.0);
            // cell13（左邻）= 真空（元素 0，质量 0，默认）
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 1, "左邻应变为液体元素");
            assert!(
                (updated.mass.get(13) - 25.0).abs() < 1e-3,
                "左邻应得 25（100×0.25），got {}",
                updated.mass.get(13)
            );
            assert!(
                (updated.mass.get(14) - 75.0).abs() < 1e-3,
                "源格应减 25，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.temperature.get(13) - 300.0).abs() < 1e-3,
                "温度应随质量转移为 300"
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!(
                (flow[14].x - 25.0).abs() < 1e-3,
                "flow[14].x(LEFT) 应 +25，got {}",
                flow[14].x
            );
        }
        DestroyElementsTable();
    }

    /// 左扩散事件（设计测试 6）：左扩散到真空格应产生 substance 事件。
    #[test]
    fn update_liquid_spread_left_fires_event() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.element_idx.set(8, 4);
            // cell15（右邻）= 固体（任务 2 修正：阻断右分支）
            cells.element_idx.set(15, 4);
            cells.mass.set(15, 500.0);
            // cell7（左下方对角）= 固体（液滴轨道：阻断液滴路径，保持 substance 事件断言）
            cells.element_idx.set(7, 4);
            cells.mass.set(7, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.element_idx.set(15, 4);
            updated.mass.set(15, 500.0);
            updated.element_idx.set(7, 4);
            updated.mass.set(7, 500.0);
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let events = &*sd.sim_events.ptr;
            // sim13（内部格）→ game 坐标：col=(13%6)-1=0, row=(13/6)-1=1 → game idx = 1*4+0 = 4
            let has_cell4 = events
                .substance_change_info
                .as_slice()
                .iter()
                .any(|e| e.cell_idx == 4);
            assert!(has_cell4, "应产生 sim13（game4）的 substance 事件");
        }
        DestroyElementsTable();
    }

    /// 邻格固体拒绝（设计测试 5）：左邻固体 → 不转移。
    #[test]
    fn update_liquid_skips_spread_to_solid_neighbor() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.element_idx.set(8, 4);
            cells.element_idx.set(13, 4); // 左邻固体
            cells.mass.set(13, 500.0);
            // cell15（右邻）= 固体（任务 2 修正：阻断右分支，保持源格 100 断言）
            cells.element_idx.set(15, 4);
            cells.mass.set(15, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.element_idx.set(13, 4);
            updated.mass.set(13, 500.0);
            updated.element_idx.set(15, 4);
            updated.mass.set(15, 500.0);
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 4, "左邻固体不应被替换");
            assert!((updated.mass.get(13) - 500.0).abs() < 1e-3);
            assert!((updated.mass.get(14) - 100.0).abs() < 1e-3, "源格质量不变");
        }
        DestroyElementsTable();
    }

    /// 右扩散（设计测试 2）：液体格 mass=100，右邻真空，下方固体。
    /// 任务 2 裁决修正：左邻设为异种液体（元素 2，state=2 但元素 idx 不同），
    /// 使左分支判定链"== 本格元素 或 state<2"不满足 → 左分支跳过，隔离右扩散。
    /// 期望：右邻变液体、mass=25、flow[cell].y(RIGHT)=25、源格减 25。
    #[test]
    fn update_liquid_spreads_right_to_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.temperature.set(14, 300.0);
            cells.element_idx.set(8, 4); // 下方固体
            cells.mass.set(8, 500.0);
            // 左邻 13 = 异种液体（元素 2，state=2 但 idx≠1）→ 阻断左分支
            cells.element_idx.set(13, 2);
            cells.mass.set(13, 0.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
            updated.element_idx.set(13, 2);
            updated.mass.set(13, 0.0);
            // cell9（右下方对角）= 固体（液滴轨道：阻断右液滴路径，保持右扩散 25/75 断言）
            cells.element_idx.set(9, 4);
            cells.mass.set(9, 500.0);
            updated.element_idx.set(9, 4);
            updated.mass.set(9, 500.0);
            // cell15（右邻）= 真空（默认）
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(15), 1, "右邻应变为液体元素");
            assert!(
                (updated.mass.get(15) - 25.0).abs() < 1e-3,
                "右邻应得 25，got {}",
                updated.mass.get(15)
            );
            assert!(
                (updated.mass.get(14) - 75.0).abs() < 1e-3,
                "源格应减 25，got {}",
                updated.mass.get(14)
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!(
                (flow[14].y - 25.0).abs() < 1e-3,
                "flow[14].y(RIGHT) 应 +25，got {}",
                flow[14].y
            );
        }
        DestroyElementsTable();
    }

    /// 双向扩散（设计测试 3）：左右都真空 → 左各得 25、右得 (100-25)×0.25=18.75、源格 56.25。
    /// 注意：fVar25 局部变量在左分支转移后递减（原版 L936 fVar25 -= fVar23），
    /// 右分支基于剩余质量计算（75×0.25=18.75）——依赖左分支递减后右分支用剩余值。
    #[test]
    fn update_liquid_spreads_both_directions() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.element_idx.set(8, 4);
            cells.mass.set(8, 500.0);
            // cell7/cell9（左右下方对角）= 固体（液滴轨道：阻断液滴路径，保持双向格子转移断言）
            cells.element_idx.set(7, 4);
            cells.mass.set(7, 500.0);
            cells.element_idx.set(9, 4);
            cells.mass.set(9, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
            updated.element_idx.set(7, 4);
            updated.mass.set(7, 500.0);
            updated.element_idx.set(9, 4);
            updated.mass.set(9, 500.0);
            // cell13（左）、cell15（右）都真空
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 1);
            assert_eq!(updated.element_idx.get(15), 1);
            assert!(
                (updated.mass.get(13) - 25.0).abs() < 1e-3,
                "左邻 25，got {}",
                updated.mass.get(13)
            );
            assert!(
                (updated.mass.get(15) - 18.75).abs() < 1e-3,
                "右邻 18.75（左分支后剩余 75×0.25），got {}",
                updated.mass.get(15)
            );
            assert!(
                (updated.mass.get(14) - 56.25).abs() < 1e-3,
                "源格 56.25，got {}",
                updated.mass.get(14)
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!((flow[14].x - 25.0).abs() < 1e-3, "flow.x 应 25");
            assert!((flow[14].y - 18.75).abs() < 1e-3, "flow.y 应 18.75");
        }
        DestroyElementsTable();
    }

    /// 水平门限（设计测试 4）：左邻同液体、质量差小（f23 < 门限）→ 不转移。
    /// 语义等价版本：create_displace_table 元素 1 未显式设 min_horizontal_flow（默认 0.0），
    /// 门限拦截路径在单测中不可达 → 本测试验证"质量差极小仍转移"（对照原版语义）：
    /// 质量差 1 → f22=0.25，f23=min(min(100,50),0.25)=0.25 ≥ 0 且 > 0 → 转移 0.25。
    #[test]
    fn update_liquid_horizontal_flow_threshold() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.element_idx.set(8, 4);
            cells.element_idx.set(13, 1); // 左邻同液体，质量接近
            cells.mass.set(13, 99.0);
            // cell7（左下方对角）= 固体（液滴轨道：阻断液滴路径，保持 0.25 转移断言）
            cells.element_idx.set(7, 4);
            cells.mass.set(7, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.element_idx.set(13, 1);
            updated.mass.set(13, 99.0);
            updated.element_idx.set(7, 4);
            updated.mass.set(7, 500.0);
        }
        // min_horizontal_flow=0（create_displace_table 元素 1 默认）→ 无门限，质量差 1 也转移 0.25
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(13) - 99.25).abs() < 1e-3,
                "左邻应 +0.25，got {}",
                updated.mass.get(13)
            );
        }
        DestroyElementsTable();
    }

    /// 生产环境元素表（liquid.yaml + ElementLoader.CopyEntryToElement + Sim.Element 映射）：
    /// 液体元素 flow=0（YAML 无 flow 字段 → 默认 0）、viscosity=speed（水=125）、
    /// min_horizontal_flow=0.01、min_vertical_flow=0.01、max_mass=1000。
    /// 用于复现 2026-08-02 实测 bug：Rust 实现把 flow（=0）当流率上限 → 水平扩散全死。
    fn create_production_like_liquid_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(3);
        for elem in [
            Element::default(), // 0=真空
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 2; // 液体（水）
                e.number_of_gradient_colors = 1;
                e.flow = 0.0; // 生产值：YAML 无 flow → 0
                e.viscosity = 125.0; // 生产值：liquid.yaml speed=125 → viscosity
                e.max_mass = 1000.0;
                e.min_horizontal_flow = 0.01;
                e.min_vertical_flow = 0.01;
                e.low_temp = 272.5; // 生产值：liquid.yaml Water lowTemp
                e.high_temp = 372.5; // 生产值：liquid.yaml Water highTemp
                e
            },
            {
                let mut e = Element::default();
                e.id = 4;
                e.state = 3; // 固体
                e.number_of_gradient_colors = 1;
                e
            },
        ] {
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

    /// 生产复现测试（2026-08-02 实测根因）：真实游戏数据下液体元素 flow=0、viscosity=125。
    /// 原版 UpdateLiquid 读 +8（viscosity）当流率上限 → 正常扩散；
    /// Rust 旧实现读 flow（=0）→ f23=0 → 不扩散（用户实测"水堆成柱"）。
    #[test]
    fn update_liquid_spreads_with_production_like_viscosity_cap() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_production_like_liquid_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.temperature.set(14, 300.0);
            cells.element_idx.set(8, 2); // 下方固体
            cells.mass.set(8, 500.0);
            cells.element_idx.set(15, 2); // 右邻固体（阻断右分支，隔离左扩散）
            cells.mass.set(15, 500.0);
            // cell7（左下方对角）= 固体（液滴轨道：阻断液滴路径，保持格子转移断言）
            cells.element_idx.set(7, 2);
            cells.mass.set(7, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 2);
            updated.mass.set(8, 500.0);
            updated.element_idx.set(15, 2);
            updated.mass.set(15, 500.0);
            updated.element_idx.set(7, 2);
            updated.mass.set(7, 500.0);
            // cell13（左邻）= 真空（默认）
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(
                updated.element_idx.get(13),
                1,
                "生产 flow=0 时液体仍应水平扩散（原版用 viscosity=speed 当流率上限）"
            );
            assert!(
                (updated.mass.get(13) - 25.0).abs() < 1e-3,
                "左邻应得 25（100×0.25），got {}",
                updated.mass.get(13)
            );
            assert!(
                (updated.mass.get(14) - 75.0).abs() < 1e-3,
                "源格应减 25，got {}",
                updated.mass.get(14)
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!(
                (flow[14].x - 25.0).abs() < 1e-3,
                "flow[14].x(LEFT) 应 +25，got {}",
                flow[14].x
            );
        }
        DestroyElementsTable();
    }

    /// 液滴轨道（左悬崖，2026-08-02）：下方固体 + 左下方对角可渗透 → 边缘液滴事件，
    /// 质量离开格子（源格扣 25、flow.x+25），左邻保持真空（不再有实体方格滑落）。
    #[test]
    fn update_liquid_spawns_droplet_at_left_ledge() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.temperature.set(14, 300.0);
            cells.element_idx.set(8, 4); // 下方固体
            cells.mass.set(8, 500.0);
            cells.element_idx.set(15, 4); // 右邻固体（阻断右分支）
            cells.mass.set(15, 500.0);
            // cell7（左下方对角）= 真空（可渗透）→ 触发液滴路径
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
            updated.element_idx.set(15, 4);
            updated.mass.set(15, 500.0);
            // cell13（左邻）= 真空；cell7 对角 = 真空
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 0, "左邻不应变为液体（液滴轨道）");
            assert_eq!(updated.mass.get(13), 0.0, "左邻质量应为 0");
            assert!(
                (updated.mass.get(14) - 75.0).abs() < 1e-3,
                "源格应扣 25 给液滴，got {}",
                updated.mass.get(14)
            );
            let events = &*sd.sim_events.ptr;
            let drop = events
                .spawn_liquid_info
                .as_slice()
                .iter()
                .find(|e| e.cell_idx == 4); // sim13（左邻）→ game4
            assert!(drop.is_some(), "应产生左边缘液滴事件（game4）");
            assert!(
                (drop.unwrap().mass - 25.0).abs() < 1e-3,
                "液滴质量应 25，got {}",
                drop.unwrap().mass
            );
            assert_eq!(drop.unwrap().element_idx, 1);
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!((flow[14].x - 25.0).abs() < 1e-3, "flow.x 应 +25");
        }
        DestroyElementsTable();
    }

    /// 液滴轨道（右悬崖，2026-08-02）：与左悬崖对称。
    /// 左邻设异种液体（元素 2）阻断左分支；右下方对角真空 → 右边缘液滴。
    #[test]
    fn update_liquid_spawns_droplet_at_right_ledge() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.temperature.set(14, 300.0);
            cells.element_idx.set(8, 4); // 下方固体
            cells.mass.set(8, 500.0);
            cells.element_idx.set(13, 2); // 左邻异种液体（阻断左分支）
            cells.mass.set(13, 0.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
            updated.element_idx.set(13, 2);
            updated.mass.set(13, 0.0);
            // cell15（右邻）= 真空；cell9（右下方对角）= 真空 → 触发液滴路径
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(15), 0, "右邻不应变为液体（液滴轨道）");
            assert!(
                (updated.mass.get(14) - 75.0).abs() < 1e-3,
                "源格应扣 25 给液滴，got {}",
                updated.mass.get(14)
            );
            let events = &*sd.sim_events.ptr;
            let drop = events
                .spawn_liquid_info
                .as_slice()
                .iter()
                .find(|e| e.cell_idx == 6); // sim15（右邻）→ game6
            assert!(drop.is_some(), "应产生右边缘液滴事件（game6）");
            assert!((drop.unwrap().mass - 25.0).abs() < 1e-3);
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!((flow[14].y - 25.0).abs() < 1e-3, "flow.y 应 +25");
        }
        DestroyElementsTable();
    }

    /// 液滴门控（不可见）：visibleGrid 全 0 且非 debug 编辑 → spawn 失败 → 回退格子转移。
    #[test]
    fn update_liquid_invisible_ledge_falls_back_to_cell_transfer() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            // 全部不可见
            let vg = std::slice::from_raw_parts_mut(sd.visible_grid.ptr, 12);
            for b in vg.iter_mut() {
                *b = 0;
            }
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.temperature.set(14, 300.0);
            cells.element_idx.set(8, 4);
            cells.mass.set(8, 500.0);
            cells.element_idx.set(15, 4);
            cells.mass.set(15, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
            updated.element_idx.set(15, 4);
            updated.mass.set(15, 500.0);
            // cell13（左邻）/ cell7（对角）真空
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 1, "不可见时回退格子转移，左邻应变液体");
            assert!((updated.mass.get(13) - 25.0).abs() < 1e-3);
            let events = &*sd.sim_events.ptr;
            assert_eq!(
                events.spawn_liquid_info.len(),
                0,
                "不可见时不应产生液滴事件"
            );
        }
        DestroyElementsTable();
    }

    /// AddLiquid 同元素合并（原版 L134375-134377）：液滴落在同液体格 → 质量吸收为一格。
    #[test]
    fn add_liquid_merges_into_same_element() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
            }
        }
        crate::c2_physics::liquid_flow::add_liquid(&mut sd, 14, 1, 25.0, 300.0, 0xFF, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 1);
            assert!(
                (updated.mass.get(14) - 125.0).abs() < 1e-3,
                "同元素应合并：100+25=125，got {}",
                updated.mass.get(14)
            );
            assert!((updated.temperature.get(14) - 300.0).abs() < 1e-3);
        }
        DestroyElementsTable();
    }

    /// AddLiquid 真空放置（原版 L134426-134431）。
    #[test]
    fn add_liquid_places_into_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        // cell14 默认真空
        crate::c2_physics::liquid_flow::add_liquid(&mut sd, 14, 1, 25.0, 300.0, 0xFF, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 1, "真空格应变为液体");
            assert!((updated.mass.get(14) - 25.0).abs() < 1e-3);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1, "应产生 substance 事件");
        }
        DestroyElementsTable();
    }

    /// AddLiquid 异液体挤压（原版 L134446-134455）：目标格是异液体 → DisplaceLiquid 挤走后放置。
    #[test]
    fn add_liquid_displaces_different_liquid() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 2); // 异种液体（元素 2）
                buf.mass.set(14, 50.0);
                buf.temperature.set(14, 280.0);
            }
        }
        crate::c2_physics::liquid_flow::add_liquid(&mut sd, 14, 1, 25.0, 300.0, 0xFF, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 1, "目标格应变新液体");
            assert!(
                (updated.mass.get(14) - 25.0).abs() < 1e-3,
                "目标格 25，got {}",
                updated.mass.get(14)
            );
            // 原异液体 50 被挤压到四邻真空格（各 12.5）
            let mut displaced = 0.0f32;
            for c in [13usize, 15, 8, 20] {
                displaced += updated.mass.get(c);
            }
            assert!(
                (displaced - 50.0).abs() < 1e-2,
                "异液体应挤到邻格：合计 50，got {}",
                displaced
            );
        }
        DestroyElementsTable();
    }

    /// 回归（玩家反馈）：水格四周 4 个液体不可渗透的透气砖格（props=4|2），排气口
    /// （AddRemoveSubstance → ModifyCell Add → add_gas）向水格注气时，DisplaceLiquid
    /// 失败 → **水必须保留**，气体落到 [左,右,上] 兜底（左格真空 → 放气）。
    /// 2026-08-12 修复前：add_gas 忽略 DisplaceLiquid 返回值，水整格被覆写成气体。
    #[test]
    fn add_gas_preserves_liquid_when_displace_fails_and_places_in_neighbor() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1); // 水
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
            }
            // 四周透气砖：真空 + 液体不可渗透(2)|固体不可渗透(4)
            for c in [8usize, 13, 15, 20] {
                let u = &mut *sd.updated_cells.ptr;
                u.properties.set(c, 6);
            }
        }
        crate::c2_physics::liquid_flow::add_gas(&mut sd, 14, 3, 1.0, 300.0, 0xFF, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 1, "水不能被覆写");
            assert!(
                (updated.mass.get(14) - 1000.0).abs() < 1e-3,
                "水质量必须保留，got {}",
                updated.mass.get(14)
            );
            assert_eq!(updated.element_idx.get(13), 3, "气体应落到左格（兜底首选）");
            assert!((updated.mass.get(13) - 1.0).abs() < 1e-3);
        }
        DestroyElementsTable();
    }

    /// 原版特性（用户实测）：透气砖格全是异类气体且**斜角位移被封死**时，add_gas
    /// 兜底无处可放 → place_subtract 从液体里扣掉发射质量（水多时保留，只减质量）。
    /// 注意：正十字布局 + 全同气体时，DisplaceGas 对角子循环必然把一格气体推到
    /// 另一格（"透气砖短暂真空"的来源）→ 兜底放进腾出的真空格，不扣水；只有斜角
    /// 被封死（如上侧是实墙）才真正走 place_subtract。此处布局：左/右/下 = 气体 5，
    /// 上 = 实墙，外围全固体。
    #[test]
    fn add_gas_hetero_against_occupied_neighbors_subtracts_liquid_mass() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1); // 水
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
            }
            // 除水格外的全部格先填固体（含边界），封死外围
            for c in 0..30 {
                if c == 14 {
                    continue;
                }
                let u = &mut *sd.updated_cells.ptr;
                u.element_idx.set(c, 4);
                u.mass.set(c, 100.0);
                u.temperature.set(c, 300.0);
                u.properties.set(c, 6);
            }
            // 透气砖：左(13)/右(15)/下(8) = 气体 5（1kg，异类），液体不可渗透；
            // 上(20) 保持实墙 → 斜角位移被封死
            for c in [8usize, 13, 15] {
                let u = &mut *sd.updated_cells.ptr;
                u.element_idx.set(c, 5);
                u.mass.set(c, 1.0);
                u.temperature.set(c, 300.0);
                u.properties.set(c, 6);
            }
        }
        // 气体 3 注入水格（与左/右/下 5 异类；上 20 是实墙，兜底 [左,右,上] 无处可放）
        crate::c2_physics::liquid_flow::add_gas(&mut sd, 14, 3, 1.0, 300.0, 0xFF, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 1, "水保留");
            assert!(
                (updated.mass.get(14) - 999.0).abs() < 1e-3,
                "兜底应从水里扣 1kg → 999，got {}",
                updated.mass.get(14)
            );
        }
        DestroyElementsTable();
    }

    /// 兜底优先级（原版逐格判断，位置优先）：左格同元素、右格真空 → 气体并入左格，
    /// 而非放进右格真空（2026-08-12 修正前的"种类优先"会放右格）。
    #[test]
    fn add_gas_fallback_prefers_left_same_element_over_right_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1); // 水（挤不动）
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
            }
            // 外围封死，防止 DisplaceLiquid 的气体子循环把左格气体推走腾出真空
            for c in 0..30 {
                if c == 14 {
                    continue;
                }
                let u = &mut *sd.updated_cells.ptr;
                u.element_idx.set(c, 4);
                u.mass.set(c, 100.0);
                u.temperature.set(c, 300.0);
                u.properties.set(c, 6);
            }
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(13, 3); // 左格：同元素气体
            u.mass.set(13, 1.0);
            u.temperature.set(13, 300.0);
            u.element_idx.set(15, 0); // 右格：真空（错误实现会优先放这里）
            u.mass.set(15, 0.0);
            u.properties.set(15, 6);
        }
        crate::c2_physics::liquid_flow::add_gas(&mut sd, 14, 3, 1.0, 300.0, 0xFF, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 1, "水保留");
            assert!(
                (updated.mass.get(13) - 2.0).abs() < 1e-3,
                "同元素左格应合并：1+1=2，got {}",
                updated.mass.get(13)
            );
            assert_eq!(updated.element_idx.get(15), 0, "右格真空不应被放置");
            assert!((updated.mass.get(15) - 0.0).abs() < 1e-6);
        }
        DestroyElementsTable();
    }

    /// add_liquid 异液体分支成功路径（原版 L134446-134455）：DisplaceLiquid 成功 →
    /// 新液体**写入目标格**，而非并入同元素邻格（2026-08-12 修正前并入上格 8）。
    #[test]
    fn add_liquid_writes_into_target_after_successful_displacement() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 2); // 异种液体（轻液）
                buf.mass.set(14, 50.0);
                buf.temperature.set(14, 280.0);
            }
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(15, 0); // 右格真空（唯一可挤压去向）
            u.element_idx.set(13, 4); // 左/下固体
            u.mass.set(13, 100.0);
            u.element_idx.set(20, 4);
            u.mass.set(20, 100.0);
            u.element_idx.set(8, 1); // 下格：新液体同元素（陷阱：修正前并入这里）
            u.mass.set(8, 10.0);
            u.temperature.set(8, 300.0);
        }
        crate::c2_physics::liquid_flow::add_liquid(&mut sd, 14, 1, 25.0, 300.0, 0xFF, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 1, "新液体应写入目标格");
            assert!(
                (updated.mass.get(14) - 25.0).abs() < 1e-3,
                "目标格质量 25，got {}",
                updated.mass.get(14)
            );
            assert_eq!(updated.element_idx.get(15), 2, "原异液体应挤到右格");
            assert!((updated.mass.get(15) - 50.0).abs() < 1e-2);
            assert!(
                (updated.mass.get(8) - 10.0).abs() < 1e-3,
                "同元素上格不应被并入，got {}",
                updated.mass.get(8)
            );
        }
        DestroyElementsTable();
    }

    /// add_liquid 气体分支交换方向（原版 L1191 `iVar11 = width + iVar14`，C# Grid.CellAbove
    /// = cell+width）：DisplaceGas 失败 → 与**上格**液体交换（气体上移、上格液体下来）。
    /// 2026-08-12 对照审查修正：此前误用下格（target-width），方向与原版相反。
    #[test]
    fn add_liquid_gas_swap_uses_cell_above() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 3); // 目标格：气体（挤不动）
                buf.mass.set(14, 1.0);
                buf.temperature.set(14, 300.0);
            }
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(20, 1); // 上格：液体（原版应与之交换）
            u.mass.set(20, 50.0);
            u.temperature.set(20, 300.0);
            u.element_idx.set(8, 2); // 下格：异类液体（陷阱：错误实现会换这里）
            u.mass.set(8, 50.0);
            u.temperature.set(8, 300.0);
            u.element_idx.set(13, 4); // 左/右/对角固体，封死 DisplaceGas
            u.mass.set(13, 100.0);
            u.element_idx.set(15, 4);
            u.mass.set(15, 100.0);
            for c in [7usize, 9, 12, 16, 19, 21] {
                u.element_idx.set(c, 4);
                u.mass.set(c, 100.0);
            }
        }
        crate::c2_physics::liquid_flow::add_liquid(&mut sd, 14, 2, 25.0, 300.0, 0xFF, 0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 1, "上格液体应交换到目标格");
            assert!(
                (updated.mass.get(14) - 50.0).abs() < 1e-3,
                "目标格承接上格液体 50，got {}",
                updated.mass.get(14)
            );
            assert_eq!(updated.element_idx.get(20), 3, "气体应上移到上格");
            assert!((updated.mass.get(20) - 1.0).abs() < 1e-3);
            assert_eq!(updated.element_idx.get(8), 2, "下格不应被交换");
            assert!(
                (updated.mass.get(8) - 75.0).abs() < 1e-3,
                "新增液体应并入下格同元素：50+25=75，got {}",
                updated.mass.get(8)
            );
        }
        DestroyElementsTable();
    }

    /// AddSolid Type 0（DoVerticalDisplacement）：先同列上方补满，剩余落到目标格。
    /// 元素表：0=气体 / 1=液体 / 2=固体（max_mass=1000）。
    #[test]
    fn add_solid_vertical_displacement_fills_above_and_places_down() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        {
            let mut table = crate::b_elements::elements_table::G_ELEMENTS_TABLE.lock().unwrap();
            table.post_process_data[2].max_mass = 1000.0;
        }
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(8, 2); // row1 col2 同元素固体
            u.mass.set(8, 900.0);
            u.temperature.set(8, 300.0);
            u.element_idx.set(14, 0); // row2 col2 气体
            u.mass.set(14, 0.1);
            u.temperature.set(14, 300.0);
            u.element_idx.set(20, 0); // row3 col2 气体
            u.mass.set(20, 0.1);
            u.temperature.set(20, 300.0);
        }
        crate::c2_physics::liquid_flow::add_solid(&mut sd, 14, 2, 150.0, 400.0, 0xFF, 0, 0);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                (u.mass.get(8) - 1000.0).abs() < 1e-3,
                "上方应补满到 1000，got {}",
                u.mass.get(8)
            );
            assert_eq!(u.element_idx.get(14), 2, "目标格应变成固体");
            assert!(
                (u.mass.get(14) - 50.0).abs() < 1e-3,
                "剩余 50kg 应落到目标格，got {}",
                u.mass.get(14)
            );
        }
        DestroyElementsTable();
    }

    /// AddSolid Type 0：目标格已是同元素固体 → 跳过，继续向下落到气体格。
    #[test]
    fn add_solid_vertical_displacement_skips_solid_and_displaces_gas() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 4 元素表：0=气体 / 1=液体 / 2=固体A（max_mass 1000）/ 3=固体B（不同元素）
        let mut w = BinaryBufferWriter::new();
        w.write_int(4);
        for (id, state) in [(0i32, 1u8), (1i32, 2u8), (2i32, 3u8), (3i32, 3u8)] {
            let mut e = Element::default();
            e.id = id;
            e.state = state;
            e.max_mass = 1000.0;
            e.number_of_gradient_colors = 1;
            let bytes =
                unsafe { std::slice::from_raw_parts(&e as *const Element as *const u8, 164).to_vec() };
            w.write_bytes(&bytes);
        }
        for _ in 0..4 {
            w.write_int(0);
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        assert!(!CreateElementsTable(&mut reader).is_null());
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(8, 2); // row1 固体A（可补满）
            u.mass.set(8, 900.0);
            u.temperature.set(8, 300.0);
            u.element_idx.set(14, 3); // row2 固体B（不同元素，阶段 1 跳过 + 行走跳过）
            u.mass.set(14, 100.0);
            u.temperature.set(14, 300.0);
            u.element_idx.set(20, 0); // row3 气体 → 挤走并放置
            u.mass.set(20, 0.5);
            u.temperature.set(20, 300.0);
        }
        crate::c2_physics::liquid_flow::add_solid(&mut sd, 14, 2, 150.0, 400.0, 0xFF, 0, 0);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                (u.mass.get(8) - 1000.0).abs() < 1e-3,
                "上方补满，got {}",
                u.mass.get(8)
            );
            assert_eq!(u.element_idx.get(20), 2, "剩余落到 row3（跳过 row2 固体）");
            assert!(
                (u.mass.get(20) - 50.0).abs() < 1e-3,
                "剩余 50kg，got {}",
                u.mass.get(20)
            );
        }
        DestroyElementsTable();
    }

    /// AddSolid Type 1（OnlyIfSameElement）：目标固体不同元素 → no-op；同元素 → 合并。
    #[test]
    fn add_solid_only_same_element_solid_noop_and_merge() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(14, 2);
            u.mass.set(14, 100.0);
            u.temperature.set(14, 300.0);
        }
        // 不同元素 → no-op
        crate::c2_physics::liquid_flow::add_solid(&mut sd, 14, 0, 50.0, 400.0, 0xFF, 0, 1);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.element_idx.get(14), 2, "不应替换");
            assert!((u.mass.get(14) - 100.0).abs() < 1e-3);
        }
        // 同元素 → 合并
        crate::c2_physics::liquid_flow::add_solid(&mut sd, 14, 2, 50.0, 400.0, 0xFF, 0, 1);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                (u.mass.get(14) - 150.0).abs() < 1e-3,
                "应合并到 150，got {}",
                u.mass.get(14)
            );
        }
        DestroyElementsTable();
    }

    /// AddSolid 其他 Type → no-op。
    #[test]
    fn add_solid_other_type_is_noop() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_state_table();
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(14, 0);
            u.mass.set(14, 0.1);
            u.temperature.set(14, 300.0);
        }
        crate::c2_physics::liquid_flow::add_solid(&mut sd, 14, 2, 50.0, 400.0, 0xFF, 0, 9);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.element_idx.get(14), 0, "不应放置");
        }
        DestroyElementsTable();
    }

    /// 超压释放·开放方向（原版 DoPressureBreak L2663-2792）：cell 上方（cell-6）真空 →
    /// **0 格固体不突破**（原版 uVar9 为 uint，0-1 回绕 → 判断 false，液体边界调整
    /// 也在该守卫内，同样不生效）→ 4 方向全失败 → 进入下方倾倒。
    #[test]
    fn post_process_overpressure_releases_through_open_direction() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 2000.0); // > maxMass(1000)×1.5
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1); // 倾倒目标（cell+6），保持较轻
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
                // cell8（cell-6，第一突破方向）= 真空（开放）
            }
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // DisplaceLiquid(20)：100 均分给 19/21/14/26（各 25）→ 20 清空；
            // 随后转移 2025×0.49751243 ≈ 1007.46 到 20
            let expect_dump = 2025.0 * 0.49751243;
            assert!(
                (updated.mass.get(14) - (2025.0 - expect_dump)).abs() < 1.0,
                "0 格固体不突破 → 源格下方倾倒剩 ~1017.5，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.mass.get(20) - expect_dump).abs() < 1.0,
                "0 格固体不突破 → cell20 应得 ~1007.5，got {}",
                updated.mass.get(20)
            );
            assert_eq!(updated.element_idx.get(20), 1);
        }
        DestroyElementsTable();
    }

    /// 超压开放方向不得误报突破（回归 2026-08-06）：cell 上/左/右均为真空（0 格固体），
    /// 下方 1100kg 重液（ratio_after=2.0-1.1=0.9 < resistance=1.0，向下也挡）。
    /// 原版 4 方向全失败 → 进入下方倾倒（DisplaceLiquid + 转移 ×0.49751243）。
    /// 此前 `(solid_count as i32 - 1) < 2` 使 0 格固体时在“上”方向误报突破成功，
    /// 提前 return，水既不突破也不倾倒（与用户实测液体行为不符）。
    #[test]
    fn post_process_overpressure_open_direction_falls_through_to_dump() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 2000.0); // > maxMass(1000)×1.5
                buf.temperature.set(14, 300.0);
                // 下方（cell+6=20）液体 1100：向下突破 ratio_after=0.9 < 1.0 → 挡
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 1100.0);
                buf.temperature.set(20, 300.0);
                // 上（8）/左（13）/右（15）= 真空（0 格固体，不突破）
            }
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // DisplaceLiquid(20)：1100 均分给 19/21/14/26（各 275）→ 20 清空；
            // 随后转移 2275×0.49751243 ≈ 1131.84 到 20
            let expect_dump = 2275.0 * 0.49751243;
            assert!(
                (updated.mass.get(20) - expect_dump).abs() < 1.0,
                "cell20 应得 ~1131.8（下方倾倒），got {}",
                updated.mass.get(20)
            );
            assert_eq!(updated.element_idx.get(20), 1);
            assert!(
                (updated.mass.get(14) - (2275.0 - expect_dump)).abs() < 1.0,
                "源格应剩 ~1143.2，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.temperature.get(20) - 300.0).abs() < 1e-3,
                "倾倒温度应随源格，got {}",
                updated.temperature.get(20)
            );
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.world_damage_info.len(), 0, "开放方向无墙壁 damage");
        }
        DestroyElementsTable();
    }

    /// 超压倾倒（原版 L2585-2698）：4 方向突破全被挡、下方液体够满 →
    /// DisplaceLiquid 挤走下方液体后转移 mass×0.49751243。
    #[test]
    fn post_process_overpressure_dumps_down_when_all_blocked() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 2000.0);
                buf.temperature.set(14, 300.0);
                // 下方（cell+6=20）液体 1100：DoPressureBreak 被其自身压强挡住（ratio 2.0-1.1=0.9 < 1.0）
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 1100.0);
                buf.temperature.set(20, 300.0);
                // 上方两格固体 + 越界（8、2）→ 突破失败
                buf.element_idx.set(8, 4);
                buf.mass.set(8, 500.0);
                buf.element_idx.set(2, 4);
                buf.mass.set(2, 500.0);
                // 左（13、12、11）右（15、16、17）各 3 格固体 → 突破失败
                for c in [13usize, 12, 11, 15, 16, 17] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                }
            }
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // DisplaceLiquid(20)：1100 均分给 19/21/14/26（各 275）→ 20 清空；
            // 随后转移 2275×0.49751243 ≈ 1131.84 到 20
            let expect_dump = 2275.0 * 0.49751243;
            assert!(
                (updated.mass.get(20) - expect_dump).abs() < 1.0,
                "cell20 应得 ~1131.8，got {}",
                updated.mass.get(20)
            );
            assert_eq!(updated.element_idx.get(20), 1);
            assert!(
                (updated.mass.get(14) - (2275.0 - expect_dump)).abs() < 1.0,
                "源格应剩 ~1143.2，got {}",
                updated.mass.get(14)
            );
            // 挤出的 1100 应出现在邻格
            let mut displaced = 0.0f32;
            for c in [19usize, 21, 26] {
                displaced += updated.mass.get(c);
            }
            assert!(
                (displaced - 275.0 * 3.0).abs() < 1.0,
                "挤压邻格合计应 825，got {}",
                displaced
            );
        }
        DestroyElementsTable();
    }

    /// 超压阈值修复（2026-08-02）：下方接近满（below×1.01 > mass[cell]）时**不**倾倒。
    /// 原版 threshold = max(maxMass, below×1.01)；旧实现 min → 错误倾倒。
    #[test]
    fn post_process_overpressure_skips_dump_when_below_nearly_full() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 2000.0);
                buf.temperature.set(14, 300.0);
                // 下方 1990：threshold=max(1000, 1990×1.01=2009.9)=2009.9 > 2000 → 不倒
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 1990.0);
                buf.temperature.set(20, 300.0);
                for c in [8usize, 2, 13, 12, 11, 15, 16, 17] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                }
            }
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(14) - 2000.0).abs() < 1e-3,
                "下方接近满 → 源格不应倾倒，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.mass.get(20) - 1990.0).abs() < 1e-3,
                "下方接近满 → 目标格不变，got {}",
                updated.mass.get(20)
            );
            assert_eq!(updated.element_idx.get(20), 1);
        }
        DestroyElementsTable();
    }

    /// 上推分支（原版 L1043-1110）：1020 格上方 1000 → threshold = max(1010, 1000) = 1010，
    /// 超出 10 → 推 5（验证原版 ~1020 封顶原理：三层水底层最多比满格多 ~20kg）。
    #[test]
    fn update_liquid_pushes_excess_up() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 1020.0); // 超满格
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1); // 上层液体 1000（cell+width）
                buf.mass.set(20, 1000.0);
                buf.temperature.set(20, 300.0);
                // 下/左/右固体，隔离其他分支
                for c in [8usize, 13, 15] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                }
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(20) - 1005.0).abs() < 1e-3,
                "上层应 +5 到 1005，got {}",
                updated.mass.get(20)
            );
            assert!(
                (updated.mass.get(14) - 1015.0).abs() < 1e-3,
                "源格应 -5 到 1015，got {}",
                updated.mass.get(14)
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!(
                (flow[14].w - 5.0).abs() < 1e-3,
                "flow[UP].w 应 +5，got {}",
                flow[14].w
            );
        }
        DestroyElementsTable();
    }

    /// 上推受流速上限约束（原版 L1070-1075）：2000 格上方 1000 → 超出 990×0.5=495，
    /// 但 mass(2000) < 2×threshold(2020) → min(495, viscosity=50) = 50 → 每次推 50。
    #[test]
    fn update_liquid_pushes_excess_up_viscosity_capped() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 2000.0); // 200% 超满
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 1000.0);
                buf.temperature.set(20, 300.0);
                for c in [8usize, 13, 15] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                }
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(20) - 1050.0).abs() < 1e-3,
                "上层应 +50（流速上限），got {}",
                updated.mass.get(20)
            );
            assert!(
                (updated.mass.get(14) - 1950.0).abs() < 1e-3,
                "源格应 -50 到 1950，got {}",
                updated.mass.get(14)
            );
        }
        DestroyElementsTable();
    }

    /// 未超阈值不推：1000 格上方 1000 → threshold = 1010 > 1000 → 无超出。
    #[test]
    fn update_liquid_does_not_push_when_under_threshold() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 1000.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 1000.0);
                buf.temperature.set(20, 300.0);
                for c in [8usize, 13, 15] {
                    buf.element_idx.set(c, 4);
                    buf.mass.set(c, 500.0);
                }
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(20) - 1000.0).abs() < 1e-3,
                "未超阈值不应上推，got {}",
                updated.mass.get(20)
            );
            assert!((updated.mass.get(14) - 1000.0).abs() < 1e-3);
        }
        DestroyElementsTable();
    }

    /// 蒸发清理（原版 L2551-2558 + Evaporate L2194）：液体质量 ≤0.01（≠0.01）→
    /// 清成真空 + substance 事件（修复表层 0.0mcg 残留）。
    #[test]
    fn post_process_evaporates_tiny_liquid() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 0.005); // 微克级残留
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(8, 4); // 下方固体
            updated.mass.set(8, 500.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(
                updated.element_idx.get(14),
                0,
                "≤0.01kg 液体应蒸发清空（元素→真空）"
            );
            assert_eq!(updated.mass.get(14), 0.0);
            let events = &*sd.sim_events.ptr;
            assert!(
                events.substance_change_info.len() >= 1,
                "蒸发应产生 substance 事件"
            );
        }
        DestroyElementsTable();
    }

    /// 原版守卫：质量恰好 0.01 → 不蒸发（`mass <= 0.01 && mass != 0.01`），
    /// 但会走 offgas 耗尽路径就地转升华气体（原版 L149821：fVar23−fVar24 < 0.01 分支）。
    #[test]
    fn post_process_keeps_exact_0_01() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 0.01);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 3, "0.01 不蒸发，但耗尽路径就地转升华气体");
            assert!((updated.mass.get(14) - 0.01).abs() < 1e-9);
        }
        DestroyElementsTable();
    }

    /// 下方 void → 源格转 vacuum + 清病菌 + 1 条事件（原版 L775-792）。
    #[test]
    fn update_liquid_void_below_clears_source_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0; // 下方真空格视作 void
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                buf.element_idx.set(14, 1); // 源格液体
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                buf.disease_idx.set(14, 0x05);
                buf.disease_count.set(14, 42);
                buf.element_idx.set(8, 0); // 下方 void
                buf.mass.set(8, 0.0);
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), sd.vacuum_element_idx, "源格应转 vacuum");
            assert_eq!(updated.mass.get(14), 0.0, "源格质量清零");
            assert_eq!(updated.temperature.get(14), 0.0, "源格温度清零");
            assert_eq!(updated.disease_idx.get(14), 0xFF, "源格病菌应清除");
            assert_eq!(updated.disease_count.get(14), 0);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1, "应产生 1 条事件");
            assert_eq!(events.substance_change_info.get(0).cell_idx, 5);
        }
        DestroyElementsTable();
    }

    // ===== 任务 7：UpdatePressure（压强平衡）=====

    /// 简报基线：空元素表下 update_pressure 不 panic（函数存在性验证）。
    /// 元素表为空 → get_element_pressure_data 返回 None → 函数安全返回 0.0。
    #[test]
    fn update_pressure_no_panic_on_empty_table() {
        let _lock = LIB_TESTS_LOCK.lock();
        DestroyElementsTable(); // 元素表为空
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.mass.set(14, 2000.0);
            cells.mass.set(15, 100.0);
        }
        let r = update_pressure(&mut sd, 14, 15); // 空表 → 0.0，不 panic
        assert_eq!(r, 0.0, "空元素表下应返回 0.0");
    }

    /// 两液体格质量平衡：本格(100g) vs 邻格(900g)，flow=50。
    /// f18 = 50*(100-900) = -40000 → clamp 下限 -900*0.125 = -112.5（负=邻格→本格）
    /// 实际转移 = min(|-112.5|, 来源质量 900) = 112.5。
    /// 断言：本格 +112.5、邻格 -112.5、返回 -112.5、同元素液体无事件。
    #[test]
    fn update_pressure_balances_neighbor_masses() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                // 本格 cell14 (row2,col2) 液体 mass=100；邻格 cell13 (row2,col1) 液体 mass=900
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(13, 1);
                buf.mass.set(13, 900.0);
                buf.temperature.set(13, 300.0);
            }
        }
        let r = update_pressure(&mut sd, 14, 13);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (r - (-112.5)).abs() < 1e-3,
                "返回转移量应 -112.5，got {}",
                r
            );
            assert!(
                (updated.mass.get(14) - 212.5).abs() < 1e-3,
                "本格应得 112.5 → 212.5，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.mass.get(13) - 787.5).abs() < 1e-3,
                "邻格应减 112.5 → 787.5，got {}",
                updated.mass.get(13)
            );
            // 两液体同元素 → 接收格非真空（bVar12=2≠0）→ 无元素变化事件（原版 L1581-1583）
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 0, "两液体平衡不应产生事件");
        }
        DestroyElementsTable();
    }

    /// 液体本格向真空邻格填充 + 事件：本格（cell=13，液体 mass=900）vs 邻格（cell=14，真空 mass=0）。
    /// 生产路径（mod.rs 压力循环）由液体格驱动，真空/气体为邻格。
    /// 系数（K1 方案 A）：邻格真空 state==0 ≠ 1 → 取本格液体 flow=50；
    /// f18 = 50×(900-0) = 45000 → clamp 上限 900×0.125 = 112.5 → 转移 112.5。
    /// 邻格真空（bVar12=0）→ 元素变为液体 + 1 条 substance 事件（原版 L1601-1607）。
    #[test]
    fn update_pressure_fills_vacuum_cell_with_event() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.void_element_idx = 0xFFFF; // 真空元素 0 不是 void → 走元素变化分支（原版 L1584）
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                // 本格 cell13 液体 mass=900；邻格 cell14 真空 mass=0
                buf.element_idx.set(13, 1);
                buf.mass.set(13, 900.0);
                buf.temperature.set(13, 300.0);
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
            }
        }
        let r = update_pressure(&mut sd, 13, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (r - 112.5).abs() < 1e-3,
                "返回转移量应 112.5，got {}",
                r
            );
            assert!(
                (updated.mass.get(14) - 112.5).abs() < 1e-3,
                "真空邻格应得 112.5，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.temperature.get(14) - 300.0).abs() < 1e-3,
                "温度混合后应 300（源温 300×112.5/112.5），got {}",
                updated.temperature.get(14)
            );
            assert_eq!(updated.element_idx.get(14), 1, "真空邻格应变为液体元素");
            assert!(
                (updated.mass.get(13) - 787.5).abs() < 1e-3,
                "本格应减 112.5 → 787.5，got {}",
                updated.mass.get(13)
            );
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1, "真空格被填充应产生 1 条事件");
            assert_eq!(events.substance_change_info.get(0).cell_idx, 5, "sim14 → game cell 5");
        }
        DestroyElementsTable();
    }

    /// 病菌按质量比例转移：转移 112.5 / 来源 900 = 0.125 → 病菌 1000×0.125 = 125。
    /// 断言：来源格 1000-125=875、接收格 100+125=225、病菌 idx 不变。
    #[test]
    fn update_pressure_transfers_disease() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                buf.disease_idx.set(14, 0x05);
                buf.disease_count.set(14, 100);
                buf.element_idx.set(13, 1);
                buf.mass.set(13, 900.0);
                buf.temperature.set(13, 300.0);
                buf.disease_idx.set(13, 0x05);
                buf.disease_count.set(13, 1000);
            }
        }
        let r = update_pressure(&mut sd, 14, 13);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((r - (-112.5)).abs() < 1e-3, "返回转移量应 -112.5，got {}", r);
            assert_eq!(updated.disease_count.get(13), 875, "来源格病菌 1000-125");
            assert_eq!(updated.disease_count.get(14), 225, "接收格病菌 100+125");
            assert_eq!(updated.disease_idx.get(14), 0x05, "病菌 idx 不变");
            assert_eq!(updated.disease_idx.get(13), 0x05);
        }
        DestroyElementsTable();
    }

    #[test]
    fn update_pressure_clean_replenishment_keeps_disease() {
        // 2026-08-06 孢子兰"补充质量删病菌"端到端回归：
        // 高压干净气体补充进带菌低压格 → 质量流入、病菌保留（原版 L26178）。
        // 此前 add_disease_to_cell(0xff,0) 走替换 → 病菌被清空（用户实测）。
        let _lock = LIB_TESTS_LOCK.lock();
        create_liquid_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0); // 高压干净气体
                buf.temperature.set(14, 300.0);
                buf.disease_idx.set(14, 0xff);
                buf.disease_count.set(14, 0);
                buf.element_idx.set(13, 1);
                buf.mass.set(13, 2.0); // 低压带菌格（孢子兰判定格）
                buf.temperature.set(13, 300.0);
                buf.disease_idx.set(13, 0x05);
                buf.disease_count.set(13, 1000);
            }
        }
        let r = update_pressure(&mut sd, 14, 13);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(r > 0.0, "干净高压气体应流入带菌格，r={}", r);
            assert!(updated.mass.get(13) > 2.0, "带菌格应收到补充质量");
            assert_eq!(
                updated.disease_count.get(13),
                1000,
                "干净补充质量不清除带菌格病菌"
            );
            assert_eq!(updated.disease_idx.get(13), 0x05, "病菌 idx 保留");
        }
        DestroyElementsTable();
    }

    /// 自建 3 元素压力表：0=真空、1=液体(state=2, flow=50)、2=气体(state=1, flow=0.12)。
    fn create_pressure_gas_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(3);
        let elems = [
            Element::default(),
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 2;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.flow = 0.12; // 类污染氧气体 flow
                e.molar_mass = 44.0;
                e
            },
        ];
        for elem in &elems {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..3 {
            w.write_int(0);
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 2026-08-03 回归：液体格不得向**异元素气体**格转移质量。
    /// 实测 bug：500kg 污染水 + 上方 0.5kg 污染氧 → 压力每子步把 ~60kg 水质量
    /// 塞进污染氧格（元素不变 → 水质量变成污染氧），直到 250/250 均衡。
    /// 原版压力只在同元素格或真空目标间转移。修复后 update_pressure 应返回 0、质量不变。
    #[test]
    fn update_pressure_does_not_transfer_into_heterogeneous_gas() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_pressure_gas_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1); // 液体
                buf.mass.set(14, 500.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(13, 2); // 异元素气体
                buf.mass.set(13, 0.5);
                buf.temperature.set(13, 300.0);
            }
        }
        let r = update_pressure(&mut sd, 14, 13);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(r, 0.0, "异元素气体目标不应发生压力转移");
            assert!(
                (updated.mass.get(14) - 500.0).abs() < 1e-4,
                "液体质量不得减少，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.mass.get(13) - 0.5).abs() < 1e-4,
                "气体质量不得增加，got {}",
                updated.mass.get(13)
            );
            assert_eq!(updated.element_idx.get(13), 2, "气体元素不变");
        }
        DestroyElementsTable();
    }

    // ===== 任务 8：PostProcessCell + DoDensityDisplacement + 挤压位移 =====

    /// 自建 6 元素最小表（任务 8 挤压/密度/升华链路）：
    /// - 0：真空（state=0）
    /// - 1：重液体（state=2，molar_mass=200，max_mass=1000，flow=50，viscosity=1，
    ///      sublimate_index=3，sublimate_probability=1.0，off_gas_percentage=0.5，sublimate_efficiency=1.0）
    /// - 2：轻液体（state=2，molar_mass=18，max_mass=1000）
    /// - 3：气体（state=1，molar_mass=16）
    /// - 4：固体（state=3）
    /// - 5：轻气体（state=1，molar_mass=8）——气体邻居置换/异类气体测试用
    fn create_displace_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(6);
        let elems = [
            Element::default(), // 0=真空
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 200.0;
                e.flow = 50.0;
                e.viscosity = 50.0; // 流率上限（原版 UpdateLiquid 读 viscosity，2026-08-02 修复后）
                e.max_mass = 1000.0;
                // ppd.max_mass 映射自 elem.max_mass（03_elements.c L305 @0x20 ← elem@0x34，
                // 2026-08-02 修正；超压/DoPressureBreak 用）。
                e.solid_surface_area_multiplier = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                // 升华字段（accumulatedFlow 累加测试用，对照 L297-302 映射到 ppd）
                e.sublimate_index = 3;
                e.sublimate_probability = 1.0;
                e.off_gas_percentage = 0.5;
                e.sublimate_efficiency = 1.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 2;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 18.0;
                e.max_mass = 1000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 3;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 16.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 4;
                e.state = 3;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 1000.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 5;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 8.0; // 比气体 3 更轻，可被置换候选选中
                e
            },
        ];
        for elem in &elems {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..6 {
            w.write_int(0); // 空名称
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 自建 8 元素升华表：
    /// - 0：真空（state=0）
    /// - 1：固体 A（state=3，sublimate_index=4→气体4，rate=0.4，eff=0.5，prob=1.0，fx=101）
    /// - 2：固体 B（state=3，sublimate_index=5→气体5，rate=0.1，eff=0.2，prob=1.0，fx=102）
    /// - 3：气体 X（state=1，molar=16）
    /// - 4：气体 Y（state=1，molar=44）
    /// - 5：气体 Z（state=1，molar=28）
    /// - 6：普通固体（state=3，sublimate_index=0xFFFF，不升华）
    /// - 7：液体（state=2，sublimate_index=5，prob=1.0，off_gas=0.5，eff=1.0，fx=107）
    fn create_sublimate_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(8);
        let elems = [
            Element::default(),
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 3;
                e.number_of_gradient_colors = 1;
                e.sublimate_index = 4;
                e.sublimate_rate = 0.4;
                e.sublimate_efficiency = 0.5;
                e.sublimate_probability = 1.0;
                e.sublimate_fx = 101;
                e
            },
            {
                let mut e = Element::default();
                e.id = 2;
                e.state = 3;
                e.number_of_gradient_colors = 1;
                e.sublimate_index = 5;
                e.sublimate_rate = 0.1;
                e.sublimate_efficiency = 0.2;
                e.sublimate_probability = 1.0;
                e.sublimate_fx = 102;
                e
            },
            {
                let mut e = Element::default();
                e.id = 3;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 16.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 4;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 44.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 5;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 28.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 6;
                e.state = 3;
                e.number_of_gradient_colors = 1;
                e.sublimate_index = 0xffff;
                e
            },
            {
                let mut e = Element::default();
                e.id = 7;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e.sublimate_index = 5;
                e.sublimate_probability = 1.0;
                e.off_gas_percentage = 0.5;
                e.sublimate_efficiency = 1.0;
                e.sublimate_fx = 107;
                e
            },
        ];
        for elem in &elems {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..8 {
            w.write_int(0);
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 简报基线：空元素表下 post_process_cell 不 panic（函数存在性验证）。
    #[test]
    fn post_process_cell_no_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        DestroyElementsTable(); // 元素表为空
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        post_process_cell(&mut sd, 14);
    }

    /// DoSublimation 入口（L148633）：调用格质量 >= 1.8 → 整函数跳过，randomSeed 不消耗。
    #[test]
    fn do_sublimation_skips_when_target_full() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 4);
                buf.mass.set(14, 1.8);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1); // 有效升华邻格
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
            }
        }
        let seed_before = sd.random_seed;
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(20) - 100.0).abs() < 1e-4);
        }
        assert_eq!(sd.random_seed, seed_before, "满格不消耗 randomSeed");
        DestroyElementsTable();
    }

    /// 同种气体合并（L148681-148702）：cell14=气体4 mass 1.0，上方砖 A（rate0.4→f18=0.08，eff0.5）。
    /// 目标 += 0.08×0.5=0.04；砖格 -= 0.08；accumulated_flow[砖] += 0.08；
    /// SpawnFX(砖, fx=101, rotation=180 上方)；同元素合并无 substance 事件；温度质量加权。
    #[test]
    fn do_sublimation_merges_same_element() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 4);
                buf.mass.set(14, 1.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 250.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(14) - 1.04).abs() < 1e-4, "目标 += 0.04");
            assert!((updated.mass.get(20) - 99.92).abs() < 1e-4, "砖格 -= 0.08");
            assert!((updated.temperature.get(14) - 298.0769).abs() < 1e-3, "温度加权");
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!((acc[20] - 0.08).abs() < 1e-4, "accumulated_flow 累积在砖格");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 0, "同元素合并无 substance 事件");
            assert_eq!(events.spawn_fx_info.len(), 1);
            let fx = events.spawn_fx_info.get(0);
            assert_eq!(fx.cell_idx, 9, "sim20 → game 9");
            assert_eq!(fx.fx_id, 101);
            assert_eq!(fx.rotation, 180.0, "上方邻格方向角");
        }
        DestroyElementsTable();
    }

    /// 1.8 上限缩放（L148692-148702）：目标 1.79，room=0.01 < f20=0.04 →
    /// f21 = 0.08×(0.01/0.04) = 0.02；目标恰好到 1.8，砖格只扣 0.02。
    #[test]
    fn do_sublimation_caps_at_1_8() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 4);
                buf.mass.set(14, 1.79);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(14) - 1.8).abs() < 1e-5, "目标恰好 1.8");
            assert!((updated.mass.get(20) - 99.98).abs() < 1e-4, "砖格扣缩放后的 0.02");
        }
        DestroyElementsTable();
    }

    /// 真空换元素（L148719-148753）：cell14 真空 → 元素=4、质量 += 0.04、温度/病菌拷贝、
    /// substance 事件；砖格扣 0.08、acc[20]+=0.08、SpawnFX 方向 180。
    #[test]
    fn do_sublimation_sets_element_in_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 250.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 4, "真空格换成升华产物");
            assert!((updated.mass.get(14) - 0.04).abs() < 1e-5);
            assert!((updated.temperature.get(14) - 250.0).abs() < 1e-4, "温度拷贝自砖格");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1);
            assert_eq!(events.substance_change_info.get(0).cell_idx, 5, "sim14 → game 5");
        }
        DestroyElementsTable();
    }

    /// 病菌按比例（L148676-148679）：砖格 mass=0.5、病菌 100 → i15 = (0.08/0.5)×100 = 16。
    /// 目标得病菌 idx 拷贝 + count=16；砖格病菌剩 84。
    #[test]
    fn do_sublimation_transfers_disease_proportionally() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 0.5);
                buf.temperature.set(20, 250.0);
                buf.disease_idx.set(20, 3);
                buf.disease_count.set(20, 100);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(20) - 0.42).abs() < 1e-5);
            assert_eq!(updated.disease_count.get(20), 84);
            assert_eq!(updated.disease_idx.get(14), 3, "病菌 idx 拷贝");
            assert_eq!(updated.disease_count.get(14), 16, "病菌按比例转移");
        }
        DestroyElementsTable();
    }

    /// 异种气体 DisplaceGas 成功（L148705-148753）：cell14=气体X mass 0.5，上方砖 A（产物气体4）。
    /// tick0 候选 [20,13,15,8]：20 固体、13 真空 → 气体 X 全量移到 13；
    /// 目标换元素 4、质量 += 0.04；砖 A 扣 0.08。
    #[test]
    fn do_sublimation_displaces_heterogeneous_gas() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.tick_count = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 3); // 气体 X
                buf.mass.set(14, 0.5);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1); // 固体 A（产物 4）
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 250.0);
                buf.element_idx.set(13, 0); // 真空（DisplaceGas 目的地）
                buf.mass.set(13, 0.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 3, "异类气体被挤到 13");
            assert!((updated.mass.get(13) - 0.5).abs() < 1e-4);
            assert_eq!(updated.element_idx.get(14), 4, "目标换成升华产物");
            assert!((updated.mass.get(14) - 0.04).abs() < 1e-5, "挤走后只剩新增 0.04");
            assert!((updated.temperature.get(14) - 250.0).abs() < 1e-4);
            assert!((updated.mass.get(20) - 99.92).abs() < 1e-4);
        }
        DestroyElementsTable();
    }

    /// DisplaceGas 失败且无先前成功（L148758）：四邻全固体/无真空 → 跳过，砖格无损、无事件。
    #[test]
    fn do_sublimation_skips_when_displace_fails_and_no_prior_success() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 3);
                buf.mass.set(14, 0.5);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
                for nb in [15usize, 13, 8] {
                    buf.element_idx.set(nb, 6); // 普通固体，无升华
                    buf.mass.set(nb, 500.0);
                    buf.temperature.set(nb, 300.0);
                }
            }
        }
        let seed_before = sd.random_seed;
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(14) - 0.5).abs() < 1e-4);
            assert!((updated.mass.get(20) - 100.0).abs() < 1e-4);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 0);
        }
        let expected = seed_before.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        assert_eq!(sd.random_seed, expected, "仅 1 个有效邻格 → 恰好消耗 1 次");
        DestroyElementsTable();
    }

    /// 原版怪癖（L148758 fall-through）：前一邻格成功（bvar16=true）后，
    /// 后续邻格 DisplaceGas 失败 → 仍扣该砖质量/播 FX，但目标不产气。
    /// cell14=气体X 0.5；20=砖A(产物4)→先成功（挤到13）；15=砖B(产物5)→挤 4 失败（四邻堵死）。
    #[test]
    fn do_sublimation_quirk_mirrors_mass_loss_after_prior_success() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.tick_count = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 3);
                buf.mass.set(14, 0.5);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1); // 产物 4 ≠ 3 → 挤 3（成功，13 真空）
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
                buf.element_idx.set(15, 2); // 产物 5 → 挤当前 4（失败，四邻无真空/同气体）
                buf.mass.set(15, 100.0);
                buf.temperature.set(15, 300.0);
                buf.element_idx.set(13, 0);
                buf.mass.set(13, 0.0);
                buf.element_idx.set(8, 6);
                buf.mass.set(8, 500.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 3, "气体 X 被挤到 13");
            assert_eq!(updated.element_idx.get(14), 4, "第一邻格成功换元素");
            assert!((updated.mass.get(14) - 0.04).abs() < 1e-5);
            assert!(
                (updated.mass.get(15) - 99.98).abs() < 1e-4,
                "怪癖：失败邻格仍扣 0.02（f18=0.1×0.2）"
            );
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!((acc[15] - 0.02).abs() < 1e-5, "怪癖：flow 仍累积");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 2, "两个邻格都有 FX");
        }
        DestroyElementsTable();
    }

    /// 整格消耗分支（L148848-148866）：砖质量 0.05 <= f18=0.08 → 砖格就地转气体、
    /// 质量 ×eff、substance 事件、SpawnFX(0.0)；调用格（真空）不变。
    #[test]
    fn do_sublimation_full_consumption_converts_in_place() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 0.05);
                buf.temperature.set(20, 250.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(20), 4, "砖格就地转气体");
            assert!((updated.mass.get(20) - 0.025).abs() < 1e-5, "0.05×0.5");
            assert_eq!(updated.element_idx.get(14), 0, "调用格不变");
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert_eq!(acc[20], 0.0, "整格消耗分支不累积 flow");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1);
            assert_eq!(events.substance_change_info.get(0).cell_idx, 9);
            let fx = events.spawn_fx_info.get(0);
            assert_eq!(fx.rotation, 0.0);
        }
        DestroyElementsTable();
    }

    /// 入口快照语义（L148650/param_4->state）：入口真空 → 后续邻格走换元素段**不尝试 DisplaceGas**，
    /// 即使目标已被前一邻格换成气体 4。最终元素 = 后邻格产物 5。
    #[test]
    fn do_sublimation_uses_entry_snapshot_for_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
                buf.element_idx.set(20, 1); // 产物 4
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
                buf.element_idx.set(15, 2); // 产物 5
                buf.mass.set(15, 100.0);
                buf.temperature.set(15, 300.0);
                buf.element_idx.set(13, 6);
                buf.mass.set(13, 500.0);
                buf.element_idx.set(8, 6);
                buf.mass.set(8, 500.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 5, "后邻格覆盖（入口快照驱动）");
            assert!((updated.mass.get(14) - 0.044).abs() < 1e-5, "0.04 + 0.02×0.2");
            assert!((updated.mass.get(20) - 99.92).abs() < 1e-4);
            assert!((updated.mass.get(15) - 99.98).abs() < 1e-4);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 2, "两次换元素各一次事件");
            assert_eq!(events.substance_change_info.get(0).cell_idx, 5);
            assert_eq!(events.substance_change_info.get(1).cell_idx, 5);
        }
        DestroyElementsTable();
    }

    /// 随机种子只对有效邻格消耗（L148656-148662）：四邻全为普通固体 → 种子不变。
    #[test]
    fn do_sublimation_consumes_seed_only_for_valid_neighbors() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                for nb in [20usize, 15, 13, 8] {
                    buf.element_idx.set(nb, 6);
                    buf.mass.set(nb, 500.0);
                }
            }
        }
        let seed_before = sd.random_seed;
        do_sublimation(&mut sd, 14);
        assert_eq!(sd.random_seed, seed_before, "无有效邻格不消耗");
        DestroyElementsTable();
    }

    /// 挤压链路（用户观察）：液体格被固体挤压 → 质量**平均分配**给上下左右 4 格。
    /// 对照 DisplaceLiquid L34136 / DisplaceLiquidSimple L34206：
    /// 4 个候选（右/左/下/上）均为真空 → 各得 100/4 = 25；
    /// 源格 ClearCell（元素→vacuum、质量/温度清零）；5 条 substance 事件（4 目标 + 1 源）。
    #[test]
    fn displace_liquid_evenly_splits_to_4_neighbors() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            // 源格 cell14 (row2,col2)：重液体元素 1，mass=100
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            // 4 邻格（13=左,15=右,8=下,20=上）全为真空
            for c in [13usize, 15, 8, 20] {
                updated.element_idx.set(c, 0);
                updated.mass.set(c, 0.0);
                updated.temperature.set(c, 0.0);
            }
        }
        let ok = displace_liquid(&mut sd, 14, 1);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(ok, "displace_liquid 应成功");
            for c in [13usize, 15, 8, 20] {
                assert!(
                    (updated.mass.get(c) - 25.0).abs() < 1e-3,
                    "邻格 {} 应得 100/4=25，got {}",
                    c,
                    updated.mass.get(c)
                );
                assert_eq!(updated.element_idx.get(c), 1, "邻格 {} 应变为液体元素", c);
            }
            // 源格被 ClearCell（L34295：元素→vacuum、质量/温度清零）
            assert_eq!(updated.element_idx.get(14), 0, "源格应清空为真空");
            assert_eq!(updated.mass.get(14), 0.0, "源格质量清零");
            assert_eq!(updated.temperature.get(14), 0.0, "源格温度清零");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 5, "应产生 4 目标 + 1 源 = 5 条事件");
            let mut cells: Vec<i32> = events
                .substance_change_info
                .as_slice()
                .iter()
                .map(|e| e.cell_idx)
                .collect();
            cells.sort();
            // game cell：sim13→4, sim15→6, sim8→1, sim20→9, sim14→5
            assert_eq!(cells, vec![1, 4, 5, 6, 9]);
        }
        DestroyElementsTable();
    }

    /// 挤压链路：部分方向被固体堵塞 → 只分给可容纳格。
    /// 左格 13 的 properties bit1（&2，固体标志）置位 → 排除；
    /// 右/下/上（15/8/20）各得 100/3 ≈ 33.333。
    #[test]
    fn displace_liquid_splits_to_available_neighbors_only() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            for c in [15usize, 8, 20] {
                updated.element_idx.set(c, 0);
                updated.mass.set(c, 0.0);
            }
            // 左格 13：真空元素但 properties&2（固体标志）→ DisplaceLiquidSimple L34271 排除
            updated.element_idx.set(13, 0);
            updated.mass.set(13, 0.0);
            updated.properties.set(13, 2);
        }
        let ok = displace_liquid(&mut sd, 14, 1);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(ok, "displace_liquid 应成功");
            for c in [15usize, 8, 20] {
                assert!(
                    (updated.mass.get(c) - (100.0 / 3.0)).abs() < 1e-3,
                    "邻格 {} 应得 100/3，got {}",
                    c,
                    updated.mass.get(c)
                );
            }
            assert_eq!(updated.mass.get(13), 0.0, "被堵塞格不应收到质量");
            assert_eq!(updated.element_idx.get(14), 0, "源格应清空");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 4, "3 目标 + 1 源 = 4 条事件");
        }
        DestroyElementsTable();
    }

    /// 挤压链路（用户观察）：气体被固体挤压 → **随机单方向转移全部质量**。
    /// 对照 DisplaceGas L34035：候选顺序 [下,左,右,上] 按 tickCount 旋转；
    /// tick_count=0 时首个候选 = 下方格 20（真空）→ DoDisplacement 全量转移。
    /// 事件：源格 1 + 目标格 1 = 2 条。
    #[test]
    fn displace_gas_moves_all_to_single_direction() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.tick_count = 0; // 候选顺序 [下(20),左(13),右(15),上(8)] 首个 = 20
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            // 源格 cell14：气体元素 3，mass=50
            updated.element_idx.set(14, 3);
            updated.mass.set(14, 50.0);
            updated.temperature.set(14, 300.0);
            // 下方格 20 真空可接收；其余方向（13/15/8）properties&1 置位 → 排除
            updated.element_idx.set(20, 0);
            updated.mass.set(20, 0.0);
            updated.temperature.set(20, 0.0);
            for c in [13usize, 15, 8] {
                updated.element_idx.set(c, 0);
                updated.mass.set(c, 0.0);
                updated.properties.set(c, 1);
            }
        }
        let ok = displace_gas(&mut sd, 14, 3);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(ok, "displace_gas 应成功");
            assert!(
                (updated.mass.get(20) - 50.0).abs() < 1e-3,
                "目标格应收到全部 50，got {}",
                updated.mass.get(20)
            );
            assert_eq!(updated.element_idx.get(20), 3, "目标格应变为气体元素");
            assert!(
                (updated.temperature.get(20) - 300.0).abs() < 1e-3,
                "温度应整体转移"
            );
            assert_eq!(updated.element_idx.get(14), 0, "源格应清空为真空");
            assert_eq!(updated.mass.get(14), 0.0, "源格质量清零");
            // 其余方向不受影响
            for c in [13usize, 15, 8] {
                assert_eq!(updated.mass.get(c), 0.0, "{} 不应收到质量", c);
            }
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 2, "源 + 目标 = 2 条事件");
            // sim14→game5，sim20→game9
            let mut cells: Vec<i32> = events
                .substance_change_info
                .as_slice()
                .iter()
                .map(|e| e.cell_idx)
                .collect();
            cells.sort();
            assert_eq!(cells, vec![5, 9]);
        }
        DestroyElementsTable();
    }

    /// 气液水平置换（2026-08-04 修复，用户实测 Bug 3）：
    /// 液体格 cell14（元素1）左侧 cell13 是气体（元素3）。
    /// 原版 UpdateNeighbourLiquidMass 对气体目标先 DisplaceGas 把气体挤走，
    /// 再把目标格改为液体元素并灌入流量 —— 此前直接 return false（液体不挤气体）。
    /// 期望：cell13 变为液体（元素1）、气体（元素3）全量移到候选格、源格质量减少。
    #[test]
    fn update_liquid_displaces_gas_horizontally() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        // tick=0 → displace_gas 候选起始方向 = 下（cell7）
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                // cell14 = 液体（元素1）mass 100
                buf.element_idx.set(14, 1);
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                // cell13 = 气体（元素3）mass 10
                buf.element_idx.set(13, 3);
                buf.mass.set(13, 10.0);
                buf.temperature.set(13, 300.0);
                // cell8 = 下方固体（阻止液体下落）；cell7 = 左下固体（阻止液滴斜落路径）
                buf.element_idx.set(8, 4);
                buf.mass.set(8, 1000.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(7, 4);
                buf.mass.set(7, 1000.0);
                buf.temperature.set(7, 300.0);
                // cell12 = 气体左侧真空（气体逃逸候选）
                buf.element_idx.set(12, 0);
                buf.mass.set(12, 0.0);
                buf.temperature.set(12, 0.0);
            }
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(
                updated.element_idx.get(13),
                1,
                "气体目标格应被液体占据（元素1）"
            );
            assert!(
                updated.mass.get(13) > 0.0,
                "液体应流入目标格，got {}",
                updated.mass.get(13)
            );
            assert_eq!(
                updated.element_idx.get(19),
                3,
                "气体应被挤到候选格（cell19 上方真空，tick=0 起始候选）"
            );
            assert!((updated.mass.get(19) - 10.0).abs() < 1e-3, "气体全量转移");
            assert!(
                updated.mass.get(14) < 100.0,
                "源格质量应减少，got {}",
                updated.mass.get(14)
            );
        }
        DestroyElementsTable();
    }

    /// 密度分层：重液体（molar_mass=200）压在轻液体（18）上方 → 交换下沉。
    /// 对照 DoDensityDisplacement L1791-1797；param_5=0.0 强制跳过随机门
    /// （原版液体走 0.3 概率门，测试直接调用函数本体）。
    #[test]
    fn do_density_displacement_swaps_heavy_above_light() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            // cell14 = 重液体(1)，下方 cell8 = 轻液体(2)
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(8, 2);
            updated.mass.set(8, 100.0);
            updated.temperature.set(8, 300.0);
        }
        let swapped = do_density_displacement(&mut sd, 14, 0.0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(swapped, "重液体应下沉交换");
            assert_eq!(updated.element_idx.get(14), 2, "上方应变轻液体");
            assert_eq!(updated.element_idx.get(8), 1, "下方应变重液体");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 2, "交换产生 2 条事件");
        }
        DestroyElementsTable();
    }

    /// 密度分层：同元素液体，下方更重（mass=200）且更热（400）→ 温度按质量加权合并。
    /// 对照 DoDensityDisplacement L1816-1831：
    /// combined = (100×300 + 200×400) / 300 = 366.67，两格同温、元素不变、无事件。
    #[test]
    fn do_density_displacement_same_element_combines_temperature() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(8, 1);
            updated.mass.set(8, 200.0);
            updated.temperature.set(8, 400.0);
        }
        let swapped = do_density_displacement(&mut sd, 14, 0.0);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(swapped, "同元素液体温度合并且返回 true（原版 L1828）");
            let expected = (100.0 * 300.0 + 200.0 * 400.0) / 300.0;
            assert!(
                (updated.temperature.get(14) - expected).abs() < 1e-3,
                "cell14 温度应合并为 {}，got {}",
                expected,
                updated.temperature.get(14)
            );
            assert!(
                (updated.temperature.get(8) - expected).abs() < 1e-3,
                "cell8 温度应合并为 {}，got {}",
                expected,
                updated.temperature.get(8)
            );
            assert_eq!(updated.element_idx.get(14), 1, "元素不变");
            assert_eq!(updated.element_idx.get(8), 1, "元素不变");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 0, "同元素温度合并无事件");
        }
        DestroyElementsTable();
    }

    /// accumulatedFlow 累加（原版 L2831）：液体格满足升华路径条件时
    /// accumulated_flow[cell] += min(offGasPercentage × mass, 1.0)。
    /// cell14 重液体 mass=100、sublimate_probability=1.0、off_gas_percentage=0.5
    /// → fVar24 = min(0.5×100, 1.0) = 1.0，下方格 20 真空（mass<1.8）。
    #[test]
    fn post_process_cell_accumulates_flow_on_sublimation_path() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1); // 重液体（含升华字段）
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(20, 0); // 下方格（cell+width）真空
            updated.mass.set(20, 0.0);
            updated.temperature.set(20, 0.0);
            updated.element_idx.set(8, 0); // 上方格（cell-width）真空，跳过同元素重算
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!(
                (acc[14] - 1.0).abs() < 1e-3,
                "accumulated_flow[14] 应累加 1.0，got {}",
                acc[14]
            );
        }
        DestroyElementsTable();
    }

    /// 升华缩放修正（原版 L2809-2812）：fVar25 = 1.8 - mass[below] < fVar24 × sublimateEfficiency
    /// → fVar24 ×= (1.8 - mass[below]) / (fVar24 × efficiency)。
    /// 下方格 20 真空 mass=1.0：fVar24 = min(0.5×100,1)=1.0；scaled = 1.0×1.0 = 1.0；
    /// room = 1.8 - 1.0 = 0.8 < 1.0 → fVar24 = 1.0 × (0.8/1.0) = 0.8 → acc[14] = 0.8。
    #[test]
    fn post_process_cell_accumulates_scaled_flow_when_below_mass_high() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(20, 0); // 下方真空，但质量 1.0 → 触发缩放
            updated.mass.set(20, 1.0);
            updated.temperature.set(20, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!(
                (acc[14] - 0.8).abs() < 1e-3,
                "accumulated_flow[14] 应缩放为 0.8，got {}",
                acc[14]
            );
        }
        DestroyElementsTable();
    }

    /// 升华 DisplaceGas 前置（原版 L2799）：下方格是异类气体（非升华目标、非真空）时，
    /// 先 DisplaceGas 把气体挤走，再累积。cell20 = 轻气体 5（sublimate_index=3 → 异类）。
    /// tick=0 候选 [下26,左19,右21,上14]，26 真空 → 气体从 20 移到 26；acc[14] 仍 += 1.0。
    #[test]
    fn post_process_cell_displaces_heterogeneous_gas_below() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.tick_count = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(20, 5); // 下方异类气体（sublimate 目标=3）
            updated.mass.set(20, 0.1);
            updated.temperature.set(20, 300.0);
            updated.element_idx.set(8, 0); // 上方真空，跳过同元素重算
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!(
                (acc[14] - 1.0).abs() < 1e-3,
                "挤走异类气体后仍应累积 1.0，got {}",
                acc[14]
            );
            // 气体被挤到 cell26（下方格 20 的下方），随后产气使 cell20 变成升华气体（原版 L149810）
            assert_eq!(updated.element_idx.get(26), 5, "异类气体应被挤到 cell26");
            assert!((updated.mass.get(26) - 0.1).abs() < 1e-3, "cell26 应收到 0.1 质量");
            assert_eq!(updated.element_idx.get(20), 3, "挤走后产气 → cell20 变升华气体");
            assert!((updated.mass.get(20) - 1.0).abs() < 1e-3, "cell20 收到 1.0 产气");
            assert!((updated.mass.get(14) - 99.0).abs() < 1e-3, "液体扣 1.0");
        }
        DestroyElementsTable();
    }

    /// 升华上方格重算（原版 L2770-2785）：上方格（cell-width）同元素且更重 →
    /// 用上方质量重算 fVar24。cell14 液体 mass=0.6，上方格 8 同液体 mass=1.0。
    /// 初始 fVar24 = min(0.5×0.6,1)=0.3；重算后 = min(0.5×1.0,1)=0.5；
    /// 剩余 = 0.6-0.5 = 0.1 ≥ 0.01 → acc[14] = 0.5（若误用下方格则得 0.3）。
    #[test]
    fn post_process_cell_recomputes_flow_from_up_cell_mass() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 0.6);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(8, 1); // 上方格同液体，更重（mass=1.0）
            updated.mass.set(8, 1.0);
            updated.temperature.set(8, 300.0);
            updated.element_idx.set(20, 0); // 下方真空
            updated.mass.set(20, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!(
                (acc[14] - 0.5).abs() < 1e-3,
                "上方格重算后应累积 0.5，got {}",
                acc[14]
            );
        }
        DestroyElementsTable();
    }

    /// 气体格 post_process：元素表非空、无异常（气体分支跑 density displacement +
    /// 随机气体邻居置换，质量 1.0 不在 [1e-9, 0.001) 挤压区间，不蒸发不清格）。
    #[test]
    fn post_process_cell_gas_cell_no_corruption() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 3); // 气体
            updated.mass.set(14, 1.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(8, 0); // 下方真空（density displacement 状态不同 → false）
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 3, "气体格不应被清空");
            assert!((updated.mass.get(14) - 1.0).abs() < 1e-6, "气体质量不变");
        }
        DestroyElementsTable();
    }

    /// 气体邻居置换门控（原版 L2469-2475）：props&2 置位（固体标志）的气体格**也**进入置换。
    /// 旧实现 `(props&2)==0` 会把置位格排除（与门控的 OR 语义冲突——审查发现的重要问题）。
    /// cell14 = 气体(3)，props=2；cell15 = 轻气体(5, molar=8)，mass=0.5。
    /// displacement_direction=-1 → 候选 [cell-dir=15, cell+dir=13, cell-width=8]；
    /// random_seed=9020：门控 random=0.9001>0.9、候选 idx0 random=0.9339>0.5。
    /// idx0=15 是气体且 molar(8)<molar(16) → 交换。断言：cell14→轻气体 5、cell15→气体 3、2 条事件。
    #[test]
    fn post_process_cell_gas_with_solid_flag_still_displaces() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.tick_count = 0;
        sd.random_seed = 9020;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 3); // 气体
            updated.mass.set(14, 1.0);
            updated.temperature.set(14, 300.0);
            updated.properties.set(14, 2); // 固体标志（props&2 置位）
            updated.element_idx.set(15, 5); // 轻气体（molar=8，比 3 的 16 更轻）
            updated.mass.set(15, 0.5);
            updated.temperature.set(15, 300.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 5, "props&2 置位气体格也应进入置换");
            assert_eq!(updated.element_idx.get(15), 3, "候选轻气体应上移到 gas 格");
            assert!((updated.mass.get(14) - 0.5).abs() < 1e-3, "交换后 mass[14] 应为 0.5");
            assert!((updated.mass.get(15) - 1.0).abs() < 1e-3, "交换后 mass[15] 应为 1.0");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 2, "交换产生 2 条事件");
        }
        DestroyElementsTable();
    }

    // ===== 任务 9 修复（K2-A）：update_liquids 生产路径挂接 post_process_cell =====

    /// K2-A 间接验证：update_liquids 帧末应对每个内部格调 post_process_cell，
    /// 使升华路径（原版 L2831 accumulatedFlow 累加）在生产中真正执行。
    /// 场景（create_displace_table，液体元素 1 含升华字段 sublimate_index=3/probability=1.0/off_gas=0.5）：
    /// - cell14 液体 mass=100；update_liquid 视角下方格（cell-width=8）**固体** → 不下落；
    /// - post_process 视角下方格（cell+width=20）真空 mass=0 → 升华路径累积；
    ///   同时 cell8 为固体 → 不触发过压 displace 干扰 cell20 的真空条件。
    /// 期望：update_liquids 返回后 accumulated_flow[14] ≈ 1.0
    /// （min(offGas×mass, 1.0) = 1.0）。修复前 update_liquids 不调 post_process_cell → 恒 0。
    #[test]
    fn update_liquids_hooks_post_process_cell_sublimation_path() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 1); // 液体（升华源）
                buf.mass.set(14, 100.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(8, 4); // update_liquid 视角下方格（cell-width）固体 → 不下落
                buf.mass.set(8, 500.0);
                buf.temperature.set(8, 300.0);
                buf.element_idx.set(20, 0); // post_process 视角下方格（cell+width）真空
                buf.mass.set(20, 0.0);
                buf.temperature.set(20, 0.0);
            }
        }
        update_liquids(&mut sd);
        unsafe {
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!(
                (acc[14] - 1.0).abs() < 1e-3,
                "update_liquids 后 accumulated_flow[14] 应累积 1.0（升华路径经 post_process_cell 执行），got {}",
                acc[14]
            );
        }
        DestroyElementsTable();
    }

    /// SimEvents::SpawnFX（原版 L149835+）：sim→game 坐标转换 + 推 spawn_fx_info。
    /// sim14（row2,col2）→ game 5；字段 = {cell_idx, fx_id, rotation}（12B，L7251-7253）。
    #[test]
    fn push_spawn_fx_converts_cell_and_fields() {
        let _lock = LIB_TESTS_LOCK.lock();
        DestroyElementsTable();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let events = &mut *sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 0);
        }
        push_spawn_fx(&mut sd, 14, 101, 180.0);
        push_spawn_fx(&mut sd, 20, 102, 0.0); // sim20（row3,col2）→ game 9
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 2);
            let fx0 = events.spawn_fx_info.get(0);
            assert_eq!(fx0.cell_idx, 5);
            assert_eq!(fx0.fx_id, 101);
            assert_eq!(fx0.rotation, 180.0);
            let fx1 = events.spawn_fx_info.get(1);
            assert_eq!(fx1.cell_idx, 9);
            assert_eq!(fx1.fx_id, 102);
            assert_eq!(fx1.rotation, 0.0);
        }
    }

    /// 液体 offgas 主路径产气（L149806-149819）：cell14 液体 mass=100@300、病菌 100，
    /// 目标 20 真空。f24 = min(0.5×100, 1.0) = 1.0；目标换元素 5、质量 1.0、温度 300、
    /// 病菌按比例 1；液体格扣 1.0/病菌 1；acc[14] += 1.0；SpawnFX(14, 107, 0.0)。
    #[test]
    fn offgas_main_path_emits_into_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7); // 液体
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.disease_idx.set(14, 3);
            updated.disease_count.set(14, 100);
            updated.element_idx.set(20, 0); // 目标真空
            updated.mass.set(20, 0.0);
            updated.temperature.set(20, 0.0);
            updated.disease_idx.set(20, 0xff); // 无菌哨兵（clear_disease 同款，原版 L149816 判定）
            updated.element_idx.set(8, 0); // cell-width 真空，跳过同元素重算
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(20), 5, "目标换成污染氧");
            assert!((updated.mass.get(20) - 1.0).abs() < 1e-4);
            assert!((updated.temperature.get(20) - 300.0).abs() < 1e-4);
            assert_eq!(updated.disease_idx.get(20), 3);
            assert_eq!(updated.disease_count.get(20), 1, "病菌按 1/100 比例");
            assert!((updated.mass.get(14) - 99.0).abs() < 1e-4, "液体扣 1.0");
            assert_eq!(updated.disease_count.get(14), 99);
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!((acc[14] - 1.0).abs() < 1e-4);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1);
            assert_eq!(events.substance_change_info.get(0).cell_idx, 9, "sim20 → game 9");
            assert_eq!(events.spawn_fx_info.len(), 1);
            let fx = events.spawn_fx_info.get(0);
            assert_eq!(fx.cell_idx, 5);
            assert_eq!(fx.fx_id, 107);
            assert_eq!(fx.rotation, 0.0);
        }
        DestroyElementsTable();
    }

    /// 同种气体合并 + 病菌合并（同菌种）：目标 20 = 气体5 mass 0.5@200、病菌 10。
    /// f24 = 1.0；room = 1.8-0.5 = 1.3 ≥ 1.0 → 不缩放；合并后质量 1.5、温度 266.67、
    /// 病菌 10+1=11；无 substance 事件（同元素）。
    #[test]
    fn offgas_main_path_merges_same_gas_and_disease() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.disease_idx.set(14, 3);
            updated.disease_count.set(14, 100);
            updated.element_idx.set(20, 5); // 同种气体
            updated.mass.set(20, 0.5);
            updated.temperature.set(20, 200.0);
            updated.disease_idx.set(20, 3);
            updated.disease_count.set(20, 10);
            updated.element_idx.set(8, 0);
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(20) - 1.5).abs() < 1e-4);
            assert!((updated.temperature.get(20) - 266.667).abs() < 1e-2, "温度加权");
            assert_eq!(updated.disease_count.get(20), 11, "同菌种合并");
            assert!((updated.mass.get(14) - 99.0).abs() < 1e-4);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 0, "同元素合并无事件");
        }
        DestroyElementsTable();
    }

    /// room 缩放（L2809-2812）：目标 20 = 气体5 mass 1.0；scaled = 1.0，room = 0.8 →
    /// f24 = 0.8；目标恰好 1.8；液体扣 0.8；acc[14] = 0.8。
    #[test]
    fn offgas_main_path_scales_to_room() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(20, 5);
            updated.mass.set(20, 1.0);
            updated.temperature.set(20, 300.0);
            updated.element_idx.set(8, 0);
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(20) - 1.8).abs() < 1e-4, "目标恰好 1.8");
            assert!((updated.mass.get(14) - 99.2).abs() < 1e-4);
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!((acc[14] - 0.8).abs() < 1e-4);
        }
        DestroyElementsTable();
    }

    /// 异类气体 DisplaceGas 失败 → 直接 return：无产气、无扣质量、无 acc、无 FX。
    /// 目标 20 = 气体3（≠产物5），其四邻 [26,19,21,14] 全为固体/液体 → 挤不走。
    #[test]
    fn offgas_main_path_returns_when_displace_fails() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.tick_count = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(20, 3); // 异类气体
            updated.mass.set(20, 0.1);
            updated.temperature.set(20, 300.0);
            updated.element_idx.set(8, 0);
            updated.mass.set(8, 0.0);
            for nb in [26usize, 19, 21] {
                updated.element_idx.set(nb, 6); // 固体堵死
                updated.mass.set(nb, 500.0);
            }
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(14) - 100.0).abs() < 1e-4, "挤不走则不产气");
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert_eq!(acc[14], 0.0);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 0);
        }
        DestroyElementsTable();
    }

    /// 真实参数回归（对照游戏元素表 DirtyWater：offGasPercentage=0.001、prob=0.01、
    /// eff=1.0、sublimate→污染氧）：500kg 液体单次产气应为 0.5kg（用户实测原版 500g），
    /// 而非 250kg（0.5×mass 的错误量级）。
    #[test]
    fn offgas_realistic_dirty_water_emits_500g_from_500kg() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 自建 3 元素表：0=真空、1=液体（offGas=0.001/eff=1.0/prob=1.0，prob 用 1.0 使测试确定）、2=气体
        let mut w = BinaryBufferWriter::new();
        w.write_int(3);
        let elems = [
            Element::default(),
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e.sublimate_index = 2;
                e.sublimate_probability = 1.0;
                e.off_gas_percentage = 0.001; // 真实 DirtyWater 值
                e.sublimate_efficiency = 1.0;
                e.sublimate_fx = 107;
                e
            },
            {
                let mut e = Element::default();
                e.id = 2;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 44.0;
                e
            },
        ];
        for elem in &elems {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..3 {
            w.write_int(0);
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 500.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(20, 0);
            updated.mass.set(20, 0.0);
            updated.temperature.set(20, 0.0);
            updated.disease_idx.set(20, 0xff);
            updated.element_idx.set(8, 0);
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(20), 2, "目标变成污染氧");
            assert!(
                (updated.mass.get(20) - 0.5).abs() < 1e-4,
                "500kg 液体应产 0.5kg 污染氧（offGas 0.001×500），got {}",
                updated.mass.get(20)
            );
            assert!(
                (updated.mass.get(14) - 499.5).abs() < 1e-4,
                "液体应剩 499.5kg，got {}",
                updated.mass.get(14)
            );
        }
        DestroyElementsTable();
    }

    /// 耗尽路径 + 差额补足（L149821-149833）：cell14 液体 mass=0.015，cell8（cell-width）
    /// 同液体更重（mass=10）→ f24 = min(0.5×10, 1.0) = 1.0；剩余 <0.01 →
    /// cell14 就地转气体 5（质量 0.015、病菌保留）；差额 0.985 从 cell8 补足 → cell14 质量 1.0；
    /// 耗尽路径**不累积** accumulated_flow；SpawnFX(14, 107, 0.0)。
    #[test]
    fn offgas_depleted_path_converts_in_place_with_top_up() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 0.015);
            updated.temperature.set(14, 300.0);
            updated.disease_idx.set(14, 3);
            updated.disease_count.set(14, 100);
            updated.element_idx.set(8, 7); // cell-width 同液体，更重
            updated.mass.set(8, 10.0);
            updated.temperature.set(8, 300.0);
            updated.disease_idx.set(8, 0xff); // 无菌
            updated.element_idx.set(20, 0); // 目标真空
            updated.mass.set(20, 0.0);
            updated.temperature.set(20, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 5, "液体格就地转气体");
            assert!((updated.mass.get(14) - 1.0).abs() < 1e-4, "剩余 + 补足差额");
            assert!((updated.mass.get(8) - 9.015).abs() < 1e-4, "cell8 扣差额");
            assert_eq!(updated.disease_idx.get(14), 3, "就地转化保留病菌");
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert_eq!(acc[14], 0.0, "耗尽路径不累积 flow");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 1);
            assert_eq!(events.spawn_fx_info.get(0).cell_idx, 5);
        }
        DestroyElementsTable();
    }

    /// 耗尽路径无补足：cell8 非同元素 → heavier_up=false；f24 = min(0.5×0.015,1)=0.0075；
    /// 只就地转化（质量 0.015），无第二段调用。
    #[test]
    fn offgas_depleted_path_without_top_up() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 0.015);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(8, 0); // 非液体 → 不重算
            updated.mass.set(8, 0.0);
            updated.element_idx.set(20, 0);
            updated.mass.set(20, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 5);
            assert!((updated.mass.get(14) - 0.015).abs() < 1e-5, "就地转化全部剩余");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 1);
        }
        DestroyElementsTable();
    }

    // ===== 任务 3：AddDiseaseToCell 病菌强度混合 =====

    /// 安装病菌表到 G_DISEASE：
    /// - idx0: strength = 2.0（当前病菌 A）
    /// - idx1: strength = 4.0（强病菌 B，用于 f10<=f11 分支）
    /// - idx2: strength = 1.0（弱病菌 C，用于 f10>f11 分支）
    fn install_disease_table() {
        use crate::b_elements::disease::{Disease, DiseaseInfo};
        let mut t = Disease::new();
        t.diseases.push_unchecked(DiseaseInfo { hash_id: 0xAAAA, strength: 2.0, ..Default::default() });
        t.diseases.push_unchecked(DiseaseInfo { hash_id: 0xBBBB, strength: 4.0, ..Default::default() });
        t.diseases.push_unchecked(DiseaseInfo { hash_id: 0xCCCC, strength: 1.0, ..Default::default() });
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

    #[test]
    fn add_disease_to_cell_same_disease_accumulates() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0);
            cells.disease_count.set(0, 100);
            cells.disease_infestation_tick_count.set(0, 7);
            add_disease_to_cell(cells, 0, 0, 50);
            assert_eq!(cells.disease_idx.get(0), 0);
            assert_eq!(cells.disease_count.get(0), 150);
            assert_eq!(cells.disease_infestation_tick_count.get(0), 7, "同病菌不清 infestation");
        }
        uninstall_disease_table();
    }

    #[test]
    fn add_disease_to_cell_one_side_invalid_replaces() {
        let _lock = LIB_TESTS_LOCK.lock();
        uninstall_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0xff);
            add_disease_to_cell(cells, 0, 1, 42);
            assert_eq!(cells.disease_idx.get(0), 1);
            assert_eq!(cells.disease_count.get(0), 42);
        }
        uninstall_disease_table();
    }

    #[test]
    fn add_disease_to_cell_clean_inflow_keeps_current() {
        // 原版 L26178 逗号表达式：bVar1 != 0xff 时先"保留当前"(iVar9=iVar6, bVar7=bVar1)，
        // 再判 param_3 != 0xff —— 干净质量（0xff,0）流入已有病菌格 → 保留当前病菌。
        // 2026-08-06 孢子兰"补充质量删病菌"根因：此前输入 0xff 走替换 → 病菌被清空。
        let _lock = LIB_TESTS_LOCK.lock();
        install_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0);
            cells.disease_count.set(0, 1000);
            cells.disease_infestation_tick_count.set(0, 5);
            // 干净质量流入（如无病菌气体补充进已带菌格）
            add_disease_to_cell(cells, 0, 0xff, 0);
            assert_eq!(cells.disease_idx.get(0), 0, "干净输入不覆盖当前病菌 idx");
            assert_eq!(cells.disease_count.get(0), 1000, "干净输入不清除当前病菌");
            assert_eq!(
                cells.disease_infestation_tick_count.get(0),
                5,
                "idx 未变化 → infestation 不清零"
            );
        }
        uninstall_disease_table();
    }

    #[test]
    fn add_disease_to_cell_mixing_keep_current_when_weaker_new() {
        // cur(0,100,s2) + new(1,50,s4): f10=200<=f11=200, cur>=0 → 保留当前 (0,100)
        let _lock = LIB_TESTS_LOCK.lock();
        install_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0);
            cells.disease_count.set(0, 100);
            add_disease_to_cell(cells, 0, 1, 50);
            assert_eq!(cells.disease_idx.get(0), 0, "f10<=f11 且 cur>=0 → 保留当前");
            assert_eq!(cells.disease_count.get(0), 100);
        }
        uninstall_disease_table();
    }

    #[test]
    fn add_disease_to_cell_mixing_negative_cur_count_switches() {
        // cur(0,-100,s2) + new(1,50,s4): f10=-200<=200, cur<0 → (-cur=100, idx=1)
        let _lock = LIB_TESTS_LOCK.lock();
        install_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0);
            cells.disease_count.set(0, -100);
            add_disease_to_cell(cells, 0, 1, 50);
            assert_eq!(cells.disease_idx.get(0), 1, "f10<=f11 且 cur<0 → 切换为新病菌");
            assert_eq!(cells.disease_count.get(0), 100);
        }
        uninstall_disease_table();
    }

    #[test]
    fn add_disease_to_cell_mixing_negative_d_keeps_current() {
        // cur(0,100,s2) + new(2,100,s1): f10=200>100, d=100-(200/100)*100=-100 → (100, idx=0)
        let _lock = LIB_TESTS_LOCK.lock();
        install_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0);
            cells.disease_count.set(0, 100);
            add_disease_to_cell(cells, 0, 2, 100);
            assert_eq!(cells.disease_idx.get(0), 0, "f10>f11 且 d<0 → 当前病菌胜出");
            assert_eq!(cells.disease_count.get(0), 100, "result = -d = 100");
        }
        uninstall_disease_table();
    }

    #[test]
    fn add_disease_to_cell_mixing_positive_d_replaces() {
        // cur(0,100,s2) + new(2,150,s1): f10=200>150, d=150-(200/150)*100=16 → (16, idx=2)
        let _lock = LIB_TESTS_LOCK.lock();
        install_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0);
            cells.disease_count.set(0, 100);
            add_disease_to_cell(cells, 0, 2, 150);
            assert_eq!(cells.disease_idx.get(0), 2, "f10>f11 且 d>=0 → 新病菌");
            assert_eq!(cells.disease_count.get(0), 16, "result = d = 16");
        }
        uninstall_disease_table();
    }

    #[test]
    fn add_disease_to_cell_mixing_changes_idx_resets_infestation() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0);
            cells.disease_count.set(0, 100);
            cells.disease_infestation_tick_count.set(0, 9);
            add_disease_to_cell(cells, 0, 2, 150); // idx 0 → 2
            assert_eq!(cells.disease_idx.get(0), 2);
            assert_eq!(cells.disease_infestation_tick_count.get(0), 0, "idx 变化 → infestation 清零");
        }
        uninstall_disease_table();
    }

    #[test]
    fn add_disease_to_cell_result_nonpositive_clears() {
        let _lock = LIB_TESTS_LOCK.lock();
        install_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0);
            cells.disease_count.set(0, 5);
            cells.disease_infestation_tick_count.set(0, 3);
            cells.disease_growth_accumulated_error.set(0, 1.5);
            add_disease_to_cell(cells, 0, 0, -5); // 5 + (-5) = 0
            assert_eq!(cells.disease_idx.get(0), 0xff, "count<=0 → 清 idx");
            assert_eq!(cells.disease_count.get(0), 0);
            assert_eq!(cells.disease_infestation_tick_count.get(0), 0);
            assert_eq!(cells.disease_growth_accumulated_error.get(0), 0.0);
        }
        uninstall_disease_table();
    }

    #[test]
    fn add_disease_to_cell_without_table_keeps_current() {
        // G_DISEASE 为 null（无病菌表）时双真实病菌 → 防御保留当前（原版越界断言语义）
        let _lock = LIB_TESTS_LOCK.lock();
        uninstall_disease_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.disease_idx.set(0, 0);
            cells.disease_count.set(0, 100);
            add_disease_to_cell(cells, 0, 1, 50);
            assert_eq!(cells.disease_idx.get(0), 0, "无表时保留当前病菌");
            assert_eq!(cells.disease_count.get(0), 100);
        }
        uninstall_disease_table();
    }

    /// 塌方测试表：0=真空(state0)、1=气体(state1)、2=不稳定固体(state 0x0b=11)。
    /// 固体 state 含 bit3（不稳定标记），供 post_process_loop 的 0xb 门控使用。
    fn create_unstable_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(3);
        for (id, state) in [(0i32, 0u8), (1i32, 1u8), (2i32, 0x0bu8)] {
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
            w.write_int(0);
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    #[test]
    fn stable_ticks_reroll_decrement_and_zero() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let timers_ptr = sd.timers.ptr as *mut u8;
            // 0x1F：重掷 3-5（原版 L123220-123233）
            std::ptr::write(timers_ptr.add(3), 0x1F);
            let v = get_stable_ticks_remaining(&mut sd, 3);
            assert!((3..=5).contains(&v), "0x1F 应重掷 3-5，got {}", v);
            let stored = std::ptr::read(timers_ptr.add(3)) & 0x1f;
            assert_eq!(stored, v, "重掷值应写回低 5 位");
            // 非 0：递减
            std::ptr::write(timers_ptr.add(4), 5);
            assert_eq!(get_stable_ticks_remaining(&mut sd, 4), 4);
            assert_eq!(std::ptr::read(timers_ptr.add(4)) & 0x1f, 4);
            // 0：保持
            std::ptr::write(timers_ptr.add(5), 0);
            assert_eq!(get_stable_ticks_remaining(&mut sd, 5), 0);
        }
    }

    #[test]
    fn unstable_check_gas_below_falls_and_pushes_event() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_unstable_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(19, 2); // 不稳定固体
                buf.mass.set(19, 100.0);
                buf.temperature.set(19, 300.0);
                buf.disease_idx.set(19, 3);
                buf.disease_count.set(19, 42);
                buf.element_idx.set(13, 1); // 下方气体
                buf.mass.set(13, 1.0);
                buf.temperature.set(13, 300.0);
            }
            // 稳定 tick = 0 → 立即塌方
            let timers_ptr = sd.timers.ptr as *mut u8;
            std::ptr::write(timers_ptr.add(19), 0);
        }
        do_unstable_check_basic(&mut sd, 19);
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.unstable_cell_info.len(), 1, "应推 1 条塌方事件");
            let info = events.unstable_cell_info.get(0);
            assert_eq!(info.elem_idx, 2);
            assert_eq!(info.cell_idx, 8, "game(19) = (19%6-1) + 4×(19/6-1) = 8");
            assert_eq!(info.falling_info, 0);
            assert_eq!(info.disease_idx, 3);
            assert!((info.mass - 100.0).abs() < 1e-3, "全量质量");
            assert_eq!(info.disease_count, 42);
            // 源格 Evaporate：元素→真空、质量/温度清零、病菌清除
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(19), 0);
            assert_eq!(updated.mass.get(19), 0.0);
            assert_eq!(updated.temperature.get(19), 0.0);
            assert_eq!(updated.disease_idx.get(19), 0xff);
        }
        DestroyElementsTable();
    }

    #[test]
    fn unstable_check_solid_below_no_fall_sets_timer() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_unstable_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(19, 2);
                buf.mass.set(19, 100.0);
                buf.temperature.set(19, 300.0);
                buf.element_idx.set(13, 2); // 下方固体
                buf.mass.set(13, 100.0);
            }
            let timers_ptr = sd.timers.ptr as *mut u8;
            std::ptr::write(timers_ptr.add(19), 5);
        }
        do_unstable_check_basic(&mut sd, 19);
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.unstable_cell_info.len(), 0, "固体下方不塌方");
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(19), 2, "源格保留");
            let timers_ptr = sd.timers.ptr as *mut u8;
            let t = std::ptr::read(timers_ptr.add(19));
            assert_eq!(t & 0x1f, 0x1f, "不塌方 → timers |= 0x1F");
        }
        DestroyElementsTable();
    }

    #[test]
    fn unstable_check_void_below_no_fall() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_unstable_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 1; // 显式：下方=void 元素
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(19, 2);
                buf.mass.set(19, 100.0);
                buf.temperature.set(19, 300.0);
                buf.element_idx.set(13, 1); // void 元素（气体态但 hash=void）
            }
            let timers_ptr = sd.timers.ptr as *mut u8;
            std::ptr::write(timers_ptr.add(19), 0);
        }
        do_unstable_check_basic(&mut sd, 19);
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.unstable_cell_info.len(), 0, "void 下方不塌方");
        }
        DestroyElementsTable();
    }

    #[test]
    fn unstable_check_stable_ticks_delay_blocks_fall() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_unstable_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(19, 2);
                buf.mass.set(19, 100.0);
                buf.temperature.set(19, 300.0);
                buf.element_idx.set(13, 1);
                buf.mass.set(13, 1.0);
            }
            let timers_ptr = sd.timers.ptr as *mut u8;
            std::ptr::write(timers_ptr.add(19), 5); // 稳定期未耗尽
        }
        do_unstable_check_basic(&mut sd, 19);
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.unstable_cell_info.len(), 0, "稳定期未耗尽不塌方");
            let timers_ptr = sd.timers.ptr as *mut u8;
            let t = std::ptr::read(timers_ptr.add(19)) & 0x1f;
            assert_eq!(t, 4, "稳定 tick 递减 5→4");
        }
        DestroyElementsTable();
    }

    #[test]
    fn unstable_check_headless_swaps_down() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_unstable_table();
        let mut sd = SimData::new_for_allocate(8, 8, 1, false, true); // headless
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(27, 2); // (3,3) 不稳定固体 100kg
                buf.mass.set(27, 100.0);
                buf.temperature.set(27, 300.0);
                buf.element_idx.set(19, 1); // (2,3) 气体
                buf.element_idx.set(11, 1); // (1,3) 气体
                buf.element_idx.set(3, 2); // (0,3) 固体 → 停住
                buf.mass.set(3, 100.0);
            }
            let timers_ptr = sd.timers.ptr as *mut u8;
            std::ptr::write(timers_ptr.add(27), 0);
        }
        do_unstable_check_basic(&mut sd, 27);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(11), 2, "固体应落到 (1,3)");
            assert_eq!(updated.mass.get(11), 100.0, "质量守恒");
            assert_eq!(updated.element_idx.get(19), 1, "气体被顶到 (2,3)");
            assert_eq!(updated.element_idx.get(27), 1, "气体被顶到 (3,3)");
        }
        DestroyElementsTable();
    }

    #[test]
    fn unstable_check_with_diagonals_below_empty_falls_full() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_unstable_table();
        let mut sd = SimData::new_for_allocate(8, 8, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd.saved_options = 1; // ENABLE_DIAGONAL_FALLING_SAND（休眠选项测试）
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(27, 2); // (3,3) 不稳定固体
                buf.mass.set(27, 100.0);
                buf.temperature.set(27, 300.0);
                buf.element_idx.set(19, 1); // (2,3) 下方气体 → 正下分支
                buf.mass.set(19, 1.0);
                // 挡住对角分支：左下(18)/右下(20) 候选格设固体
                buf.element_idx.set(18, 2);
                buf.mass.set(18, 100.0);
                buf.element_idx.set(20, 2);
                buf.mass.set(20, 100.0);
            }
            let timers_ptr = sd.timers.ptr as *mut u8;
            std::ptr::write(timers_ptr.add(27), 0);
        }
        do_unstable_check_with_diagonals(&mut sd, 27);
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.unstable_cell_info.len(), 1, "正下分支应塌方");
            let info = events.unstable_cell_info.get(0);
            assert_eq!(info.cell_idx, 8, "事件坐标 = game(下方 19)");
            assert!((info.mass - 100.0).abs() < 1e-3, "权重 1.0 全量");
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.mass.get(27), 0.0, "全量转移后源格清空");
            assert_eq!(updated.element_idx.get(27), 0, "Evaporate");
        }
        DestroyElementsTable();
    }

    #[test]
    fn unstable_check_with_diagonals_side_empty_half_falls() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_unstable_table();
        let mut sd = SimData::new_for_allocate(8, 8, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd.saved_options = 1;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(27, 2); // (3,3) 不稳定固体 100kg
                buf.mass.set(27, 100.0);
                buf.temperature.set(27, 300.0);
                buf.element_idx.set(19, 2); // (3,2) 下方固体 → 挡住正下
                buf.mass.set(19, 100.0);
                buf.element_idx.set(18, 1); // (2,2) 左下气体 → 对角分支
                buf.mass.set(18, 1.0);
                buf.element_idx.set(26, 1); // (2,3) 左邻气体（对角正交检查格）
                buf.mass.set(26, 1.0);
                buf.element_idx.set(28, 2); // (4,3) 右下正交格固体 → 挡住右下分支
                buf.mass.set(28, 100.0);
            }
            let timers_ptr = sd.timers.ptr as *mut u8;
            std::ptr::write(timers_ptr.add(27), 0);
        }
        do_unstable_check_with_diagonals(&mut sd, 27);
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.unstable_cell_info.len(), 1, "仅左下分支应塌方");
            let info = events.unstable_cell_info.get(0);
            assert_eq!(info.cell_idx, 7, "事件坐标 = game(左下 18)");
            assert!((info.mass - 50.0).abs() < 1e-3, "对角权重 0.5");
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(27) - 50.0).abs() < 1e-3, "源格保留 50%");
            assert_eq!(updated.element_idx.get(27), 2, "源格仍是固体（未清空）");
        }
        DestroyElementsTable();
    }

    #[test]
    fn post_process_cell_unstable_solid_falls_via_gate() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_unstable_table();
        let mut sd = SimData::new_for_allocate(8, 8, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(27, 2); // 不稳定固体（state 0x0b 过 0xb 门控）
                buf.mass.set(27, 100.0);
                buf.temperature.set(27, 300.0);
                buf.element_idx.set(19, 1); // 下方气体
                buf.mass.set(19, 1.0);
                buf.temperature.set(19, 300.0);
            }
            let timers_ptr = sd.timers.ptr as *mut u8;
            std::ptr::write(timers_ptr.add(27), 0);
        }
        post_process_cell(&mut sd, 27);
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.unstable_cell_info.len(), 1, "0xb 门控后应塌方");
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(27), 0, "塌方后清格");
        }
        DestroyElementsTable();
    }
}
