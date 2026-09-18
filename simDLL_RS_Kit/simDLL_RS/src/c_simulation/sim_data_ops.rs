//! SimData 操作函数。
//!
//! 对照源码 11_msvcrt_ignored.c 中的 ClearCell/ClearBackwall/CopySimDataToGame。
//!
//! **注意**：SimData 中的 cells/updated_cells/backwall/sim_events 字段均为
//! `UniquePtr<T>`，需通过 `.ptr` 解引用后访问内部字段。
//! GameData 中的 cells/backwalls 同样为 `UniquePtr<T>`。

use crate::a_framework::sim_data::SimData;
use crate::a_framework::game_data::{EmittedMassInfo, GameData};
use std::sync::atomic::{AtomicU32, Ordering};

/// CopySimDataToGame 内局部静态 tick_count 的镜像（原版 11_msvcrt_ignored.c L32494-32498）：
/// 每次 CopySimDataToGame 调用 +1，超过 14 后清零 SimData.accumulatedFlow 并归零。
/// 用途：accumulated_flow 是"3 秒窗口累积量"（15 帧 × 0.2s = 3s，C# 悬停显示
/// `AccumulatedFlow / 3` 折算每秒速率），窗口结束必须重置源缓冲，否则无限增长。
static ACCUMULATED_FLOW_RESET_COUNT: AtomicU32 = AtomicU32::new(0);

#[cfg(test)]
fn reset_accumulated_flow_reset_count_for_tests() {
    ACCUMULATED_FLOW_RESET_COUNT.store(0, Ordering::SeqCst);
}

/// SimData::ClearCell：清除单元格元素。
///
/// 对照源码 11_msvcrt_ignored.c L23677-23723。
/// 清零 7 个字段：elementIdx/mass/temperature/diseaseIdx/diseaseCount/
/// diseaseInfestationTickCount/diseaseGrowthAccumulatedError。
///
/// **注意**：参数 `cell_idx` 为内部 cell 索引（含边界，即 simCell）。
pub fn clear_cell(sim_data: &mut SimData, cell_idx: i32) {
    if sim_data.updated_cells.ptr.is_null() {
        tracing::warn!("clear_cell: updated_cells.ptr is null");
        return;
    }
    let idx = cell_idx as usize;
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };

    // elementIdx → vacuumElementIdx
    updated_cells.element_idx.set(idx, sim_data.vacuum_element_idx);

    // mass → 0
    updated_cells.mass.set(idx, 0.0f32);

    // temperature → 0
    updated_cells.temperature.set(idx, 0.0f32);

    // diseaseIdx → 0xFF (255，表示"无疾病")
    updated_cells.disease_idx.set(idx, 0xFFu8);

    // diseaseCount → 0
    updated_cells.disease_count.set(idx, 0i32);

    // diseaseInfestationTickCount → 0
    updated_cells.disease_infestation_tick_count.set(idx, 0u8);

    // diseaseGrowthAccumulatedError → 0
    updated_cells.disease_growth_accumulated_error.set(idx, 0.0f32);
}

/// SimData::ClearBackwall：清除后墙元素。
///
/// 对照源码 11_msvcrt_ignored.c L23648-23673。
/// 清零 3 个字段：elementIdx/mass/temperature。
///
/// **注意**：参数 `cell_idx` 为内部 cell 索引（含边界，即 simCell）。
pub fn clear_backwall(sim_data: &mut SimData, cell_idx: i32) {
    if sim_data.backwall.ptr.is_null() {
        tracing::warn!("clear_backwall: backwall.ptr is null");
        return;
    }
    let idx = cell_idx as usize;
    let backwall = unsafe { &mut *sim_data.backwall.ptr };

    // elementIdx → backwall 默认值（vacuum）
    backwall.element_idx.set(idx, sim_data.vacuum_element_idx);

    // mass → 0
    backwall.mass.set(idx, 0.0f32);

    // temperature → 0
    backwall.temperature.set(idx, 0.0f32);
}

/// SimBase::CopyUpdatedCellsToCells：将 updated_cells 拷贝到 cells。
///
/// 对照源码 11_msvcrt_ignored.c。
/// 用于 elapsed_seconds <= 0 时直接同步双缓冲。
/// 自然固体 strength_info 运行时推导。
///
/// 原版在固体生成/加载后会为自然固体写入质量型抗压字节：
///   `0x80 | (结构强度系数 × 4 & 0x7F)`
/// 自然固体的结构强度系数固定为 1.0，因此字节恒为 `0x80 | 4 = 0x84`。
///
/// 元素强度（`ElementPostProcessData::strength`，如玄武岩 1.2、火成岩 1.0）不写入此字节，
/// 而是由 `DoPressureBreak` 单独乘入。若这里写成「元素强度×4」，会在 DoPressureBreak 里
/// 被二次乘入（变成 strength²），与原版/维基公式不符。
///
/// 只覆盖 `strength_info == 0` 的格子：人造砖（0x06）、地基（0x04）等 C# 显式写入的非零值
/// 不受影响。判定条件与原版压力分支一致：`state & 3 == 3`（固体）且 `mass > 0`。
pub fn derive_natural_solid_strength(
    sim_data: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    if bounds.is_empty() {
        return;
    }
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim_data.width as usize;
    let updated = unsafe { &mut *sim_data.updated_cells.ptr };
    let len = updated.strength_info.len();
    for row in bounds.min_y..bounds.max_y {
        for col in bounds.min_x..bounds.max_x {
            let cell = row * width + col;
            if cell >= len {
                continue;
            }
            if updated.strength_info.get(cell) != 0 {
                continue;
            }
            let elem = updated.element_idx.get(cell);
            let Some(ppd) =
                crate::b_elements::elements_table::get_element_post_process_data(elem)
            else {
                continue;
            };
            if (ppd.state & 3) == 3 && updated.mass.get(cell) > 0.0 {
                // 0x80 = 质量型标志(bit7)；低 7 位 = 1.0×4 = 4。
                updated.strength_info.set(cell, 0x80u8 | ((1.0f32 * 4.0) as u8 & 0x7f));
            }
        }
    }
}

pub fn copy_updated_cells_to_cells(sim_data: &mut SimData) {
    if sim_data.cells.ptr.is_null() || sim_data.updated_cells.ptr.is_null() {
        tracing::warn!("copy_updated_cells_to_cells: cells or updated_cells ptr is null");
        return;
    }
    let cells = unsafe { &mut *sim_data.cells.ptr };
    let updated_cells = unsafe { &*sim_data.updated_cells.ptr };
    cells.copy_from(updated_cells);
}

/// T2：区域局部 CopyFrom——并行路径下只复制区域矩形 + halo 边界 1 格到 cells 快照，
/// 避免全网格拷贝（memory-bound，并行无收益且引入调度抖动）。
///
/// bounds 已钳制到内部格（compute_region_bounds），halo 落在区域间真空 buffer 内，
/// 跨区域读安全；边界处 clamp 到网格边缘。字段集与 `CellSOA::copy_from` 一致。
pub fn copy_updated_cells_to_cells_region(
    sim_data: &SimData,
    bounds: &crate::d1_activity::RegionBounds,
) {
    if sim_data.cells.ptr.is_null() || sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let w = sim_data.width as usize;
    let h = sim_data.height as usize;
    let halo = 1usize;
    let min_x = bounds.min_x.saturating_sub(halo);
    let min_y = bounds.min_y.saturating_sub(halo);
    let max_x = bounds.max_x.saturating_add(halo).min(w);
    let max_y = bounds.max_y.saturating_add(halo).min(h);
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    let cells = unsafe { &mut *sim_data.cells.ptr };
    let upd = unsafe { &*sim_data.updated_cells.ptr };
    // 行级 memcpy（2026-09-06 #1）：串行 copy_from 用 copy_nonoverlapping，
    // region 版此前逐格 .set()×11（每次边界检查）。改为每行每字段一次
    // copy_nonoverlapping——数据逐位一致，仅消除内层边界检查与循环开销。
    let row_len = max_x - min_x;
    macro_rules! copy_row_field {
        ($field:ident, $ty:ty) => {
            for row in min_y..max_y {
                let base = row * w + min_x;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        upd.$field.begin.add(base) as *const $ty,
                        cells.$field.begin.add(base) as *mut $ty,
                        row_len,
                    );
                }
            }
        };
    }
    copy_row_field!(element_idx, u16);
    copy_row_field!(temperature, f32);
    copy_row_field!(mass, f32);
    copy_row_field!(properties, u8);
    copy_row_field!(insulation, u8);
    copy_row_field!(strength_info, u8);
    copy_row_field!(disease_idx, u8);
    copy_row_field!(disease_count, i32);
    copy_row_field!(disease_infestation_tick_count, u8);
    copy_row_field!(disease_growth_accumulated_error, f32);
    copy_row_field!(radiation, f32);
}

/// 清零整张 flow 数组（Vector4f × width×height，含边界）。
///
/// 对照源码 Sim::Main（SimDLL_Source.c L2119）：`BeginFrameProcessing` 之后、
/// 帧循环之前有 `memset()`——每帧清零 flow，使 flow 成为**帧内增量累加器**。
/// C# Property.Flow（blend=0.25）把显示值逐帧拉向 raw：扩散中增量非零 → 动画；
/// 平衡后增量 ≈ 0 → 显示值指数衰减至静止。此前项目缺失此清零，flow 累积历史
/// 总量（平衡后 raw 纹理 ~30.6），C# 混合收敛到 30.6 → 动画永不停止。
///
/// 物理阶段只写 flow（update_liquid / run_pressure_task / gas_flow），不读它做
/// 决策，故每帧清零对物理零副作用，仅影响纹理。
pub fn clear_flow(sim_data: &mut SimData) {
    if sim_data.flow.ptr.is_null() {
        return;
    }
    let total = sim_data.width as usize * sim_data.height as usize;
    if total == 0 {
        return;
    }
    let flow = unsafe { std::slice::from_raw_parts_mut(sim_data.flow.ptr, total) };
    flow.fill(crate::a_framework::vector_math::Vector4f::default());
}

/// 按活动区域清 flow（替代整网格清零的性能优化）。
///
/// 收益：736×304 网格整清 = 3.41 MB memset/帧；只清活动区域省 ~37.2% 死区。
/// 安全性论证（2026-08-30 已验证）：
///   1. flow 在 new_for_allocate 零初始化；
///   2. save_load 不持久化 sim_data.flow；
///   3. 物理只按区域边界写 flow → 区域外 flow 恒 0，全清属浪费。
/// 回退：区域列表空或 > MAX_REGIONS_STACK 时退回全清（保守、行为不变）。
const MAX_REGIONS_STACK: usize = 64;

pub fn clear_flow_regions(sim_data: &mut SimData) {
    if sim_data.flow.ptr.is_null() {
        return;
    }
    let w = sim_data.width as usize;
    let h = sim_data.height as usize;
    let total = w * h;
    if total == 0 {
        return;
    }
    // 分两步：先借用 active_regions 算出边界，再取 flow 的可变切片。
    let empty = crate::d1_activity::RegionBounds {
        min_x: 0,
        min_y: 0,
        max_x: 0,
        max_y: 0,
    };
    let mut bounds = [empty; MAX_REGIONS_STACK];
    let mut n = 0usize;
    {
        let regions = sim_data.active_regions.as_slice();
        if regions.is_empty() || regions.len() > MAX_REGIONS_STACK {
            clear_flow(sim_data);
            return;
        }
        for r in regions {
            bounds[n] = crate::d1_activity::compute_region_bounds(sim_data, r);
            n += 1;
        }
    }
    let flow = unsafe { std::slice::from_raw_parts_mut(sim_data.flow.ptr, total) };
    for b in &bounds[..n] {
        if b.is_empty() {
            continue;
        }
        for y in b.min_y..b.max_y {
            let start = y * w + b.min_x;
            let end = y * w + b.max_x;
            if end <= total {
                flow[start..end].fill(crate::a_framework::vector_math::Vector4f::default());
            }
        }
    }
}

/// SimBase::CopySimDataToGame：将 SimData 拷贝到 GameData。
///
/// 对照源码 11_msvcrt_ignored.c L31168-32200。
/// 1. 拷贝 cells SOA（updated_cells → game_data.cells）——**逐行拷贝去掉边界**
/// 2. 拷贝 backwalls SOA（backwall → game_data.backwalls）——**逐行拷贝去掉边界**
/// 3. 搬移 sim_events 的 20 个事件 vector（swap 后清空 sim_events 源，对照
///    源码 L132195-132214 的 end=begin 清空，避免事件乒乓重复投递）
/// 4. **SubstanceChangeInfo 后处理**（对照源码 L132194-132373）：
///    排序去重 → 用 mGameData(旧)/mSimData(新) 的 cells 回填 old/new 元素 →
///    派生 solidInfo/solidSubstanceChangeInfo/liquidChangeInfo。
///    C# 依赖这些事件刷新格子画面（MarkDirty）和更新 Grid.Solid
///    （挖掘任务结束）；缺失会导致"挖掘后格子不消失、小人重复挖掘"。
/// 5. 更新 num_frames_processed
///
/// **C1 修复**：原实现使用 copy_from（整个 SOA memcpy），导致
/// game_data.cells 被错误 resize 到含边界尺寸（227752 vs 225040），
/// C# 按 1160×194 布局读取 1162×196 布局的数据，导致数据错位。
/// 现改为 copy_rows_from_stripped（逐行拷贝去掉边界），对照源码
/// CopySimDataToGame 的 copyToGameTasks/copyBackwallTasks 并行逐行拷贝逻辑。
///
/// **注意**：此函数在 sim 线程中调用，操作的是 FrameSync.m_sim_data 指向的 GameData。
/// `old_game_data` 为 FrameSync.m_game_data（旧可见缓冲），只读访问。
pub fn copy_sim_data_to_game(
    sim_data: &mut SimData,
    game_data: &mut GameData,
    old_game_data: *const GameData,
    frame_count: i32,
) {
    // 计算不含边界的游戏尺寸
    // sim_data.width/height 含边界，game_data.width/height 不含边界
    let sim_w = sim_data.width as usize;
    let game_w = game_data.width as usize;
    let game_h = game_data.height as usize;

    // 1. 拷贝 cells SOA（逐行去掉边界）
    if !game_data.cells.ptr.is_null() && !sim_data.updated_cells.ptr.is_null() {
        let game_cells = unsafe { &mut *game_data.cells.ptr };
        let sim_updated_cells = unsafe { &*sim_data.updated_cells.ptr };
        game_cells.copy_rows_from_stripped(sim_updated_cells, game_w, game_h, sim_w);
    } else {
        tracing::warn!("copy_sim_data_to_game: cells or updated_cells ptr is null, skip cells copy");
    }

    // 2. 拷贝 backwalls SOA（逐行去掉边界）
    if !game_data.backwalls.ptr.is_null() && !sim_data.backwall.ptr.is_null() {
        let game_backwalls = unsafe { &mut *game_data.backwalls.ptr };
        let sim_backwall = unsafe { &*sim_data.backwall.ptr };
        game_backwalls.copy_rows_from_stripped(sim_backwall, game_w, game_h, sim_w);
    } else {
        tracing::warn!("copy_sim_data_to_game: backwalls or backwall ptr is null, skip backwalls copy");
    }

    // 1.5 交换 visible_grid（原版 GameData::swapVisibleGrid，SimDLL_Source.c L127019/L132390）——
    // 液滴轨道前置（2026-08-02）：C# 侧可见性写入 GameData.visibleGrid 后，经此交换进入
    // SimData.visibleGrid，供 spawn_falling_liquid 门控读取。任一为空则跳过（安全）。
    if !game_data.visible_grid.ptr.is_null() && !sim_data.visible_grid.ptr.is_null() {
        std::mem::swap(&mut game_data.visible_grid, &mut sim_data.visible_grid);
    } else {
        tracing::warn!("copy_sim_data_to_game: visible_grid ptr is null, skip visible grid swap");
    }

    // 1.6 交换 consumed_mass_info / emitted_mass_info（原版 L131486 交换
    // elementConsumer.consumedMassInfo → GameData.consumedMassInfo；emittedMassInfo 同理）。
    // 2026-08-02 修复：此前缺失 → C# 泵收不到 removedMassEntries（无法吸水/进水管）。
    // 2026-08-02 修复：与其他 20 个事件 vector 一样**无条件交换指针**。
    // 旧实现加了"GameData 侧 begin 非空"检查——GameData 的向量零初始化（begin=null）
    // 从未分配 → 每次都被跳过 → 水泵吸走的水到不了 C#（实测"只有动画在动"）。
    // swap 本身对 null↔有效指针交换是安全的（与 substance_change_info 等一致）。
    std::mem::swap(
        &mut game_data.consumed_mass_info,
        &mut sim_data.element_consumer.consumed_mass_info,
    );
    sim_data.element_consumer.consumed_mass_info.clear_keep_capacity();
    if game_data.consumed_mass_info.len() > 0 {
        tracing::info!(
            n = game_data.consumed_mass_info.len(),
            "copy_sim_data_to_game: consumed_mass_info delivered"
        );
    }
    // 原版 L31626-31648：**不是交换**——resize game 到 sim 长度 → 逐条拷贝 sim→game →
    // sim 条目原地重置（elemIdx=vacuum、mass=0，**保持向量长度**）。
    // 关键：sim 侧向量长度恒定（Register 时建立，重置不清长度），无时间帧（elapsed<=0，
    // update 不跑）时 sim 条目为空但长度仍在 → 拷贝后 game 向量非空 → GDU emittedMassEntries
    // 永不为 null → C# BuildingElementEmitter 无条件读取不会 NRE（建造肥料合成器瞬间复现）。
    let sim_len = sim_data.element_emitter.emitted_mass_info.len();
    game_data.emitted_mass_info.resize(sim_len, EmittedMassInfo::default());
    for idx in 0..sim_len {
        game_data.emitted_mass_info.set(idx, sim_data.element_emitter.emitted_mass_info.get(idx));
        sim_data.element_emitter.emitted_mass_info.set(
            idx,
            EmittedMassInfo {
                elem_idx: 0xFFFF,
                disease_idx: 0xFF,
                pad: 0,
                mass: 0.0,
                temperature: 0.0,
                disease_count: 0,
            },
        );
    }

    // 病菌发射信息交付（2026-08-07 补齐：此前缺失 → C# diseaseEmittedInfos 恒空）
    crate::c_simulation::disease_component::swap_disease_emitted_output(game_data);

    // ElementChunk 每帧输出（原版 CopySimDataToGame elementChunkInfo 交换）
    crate::c_simulation::element_chunk::swap_element_chunk_output(game_data);

    // 3. 搬移 sim_events 的 20 个事件 vector（swap 后清空 sim_events 源）
    // 对照源码 L31791-L32174（搬移）+ L132195-132214（源清空，end=begin）
    if sim_data.sim_events.ptr.is_null() {
        tracing::warn!("copy_sim_data_to_game: sim_events ptr is null, skip events swap");
    } else {
        let sim_events = unsafe { &mut *sim_data.sim_events.ptr };

        // 按 SimEvents 字段顺序 swap（与 GameData 对应字段名一致或相近）
        std::mem::swap(&mut game_data.substance_change_info, &mut sim_events.substance_change_info);
        std::mem::swap(&mut game_data.spawn_falling_liquid_info, &mut sim_events.spawn_liquid_info);
        std::mem::swap(&mut game_data.spawn_ore_info, &mut sim_events.spawn_ore_info);
        std::mem::swap(&mut game_data.unstable_cell_info, &mut sim_events.unstable_cell_info);
        std::mem::swap(&mut game_data.element_chunk_melted_info, &mut sim_events.element_chunk_melted_info);
        std::mem::swap(&mut game_data.building_melted_info, &mut sim_events.building_melted_info);
        std::mem::swap(&mut game_data.building_overheat_info, &mut sim_events.building_overheat_info);
        std::mem::swap(&mut game_data.building_no_longer_overheated_info, &mut sim_events.building_no_longer_overheated_info);
        std::mem::swap(&mut game_data.cell_melted_info, &mut sim_events.cell_melted_info);
        std::mem::swap(&mut game_data.callback_info, &mut sim_events.callback_info);
        std::mem::swap(&mut game_data.world_damage_info, &mut sim_events.world_damage_info);
        std::mem::swap(&mut game_data.mass_consumed_callbacks, &mut sim_events.mass_consumed_callbacks);
        std::mem::swap(&mut game_data.radiation_consumed_callbacks, &mut sim_events.radiation_consumed_callbacks);
        std::mem::swap(&mut game_data.mass_emitted_callbacks, &mut sim_events.mass_emitted_callbacks);
        std::mem::swap(&mut game_data.disease_consumed_callbacks, &mut sim_events.disease_consumed_callbacks);
        std::mem::swap(&mut game_data.spawn_fx_info, &mut sim_events.spawn_fx_info);
        std::mem::swap(&mut game_data.component_state_changed_messages, &mut sim_events.component_state_changed_messages);
        std::mem::swap(&mut game_data.dig_info, &mut sim_events.dig_info);
        std::mem::swap(&mut game_data.backwall_element_changed_info, &mut sim_events.backwall_element_changed_info);
        std::mem::swap(&mut game_data.backwall_should_transition_info, &mut sim_events.backwall_should_transition_info);

        // 搬移后清空 sim_events 源（对照源码 L132195-132214 的 end=begin）。
        // 换入 sim_events 的旧 vector 是 C# 已消费的内容，直接丢弃，避免
        // 同一事件在 sim↔game 之间乒乓、被 C# 每帧重复消费。
        sim_events.substance_change_info.clear_keep_capacity();
        sim_events.spawn_liquid_info.clear_keep_capacity();
        sim_events.spawn_ore_info.clear_keep_capacity();
        sim_events.unstable_cell_info.clear_keep_capacity();
        sim_events.element_chunk_melted_info.clear_keep_capacity();
        sim_events.building_melted_info.clear_keep_capacity();
        sim_events.building_overheat_info.clear_keep_capacity();
        sim_events.building_no_longer_overheated_info.clear_keep_capacity();
        sim_events.cell_melted_info.clear_keep_capacity();
        sim_events.callback_info.clear_keep_capacity();
        sim_events.world_damage_info.clear_keep_capacity();
        sim_events.mass_consumed_callbacks.clear_keep_capacity();
        sim_events.radiation_consumed_callbacks.clear_keep_capacity();
        sim_events.mass_emitted_callbacks.clear_keep_capacity();
        sim_events.disease_consumed_callbacks.clear_keep_capacity();
        sim_events.spawn_fx_info.clear_keep_capacity();
        sim_events.component_state_changed_messages.clear_keep_capacity();
        sim_events.dig_info.clear_keep_capacity();
        sim_events.backwall_element_changed_info.clear_keep_capacity();
        sim_events.backwall_should_transition_info.clear_keep_capacity();
    }

    // 3b. Refresh building temperature output every frame (handle + temperature,
    //     indexed by handle index) -> GDU buildingTemperatures for C#.
    crate::c_simulation::building_temperature::write_building_temperature_info(game_data);

    // 4. SubstanceChangeInfo 后处理（对照源码 L132194-132373）
    post_process_substance_change_info(game_data, old_game_data);

    // 5. 更新 num_frames_processed
    game_data.num_frames_processed = frame_count;

    // 5a. flow 逐行拷贝（原版 copyFlowTasks，11_msvcrt_ignored.c L31263-31350）：
    //     按行（去掉边界）SimData.flow → GameData.flow（Vector4f，16B/格）。
    //     2026-08-04 审查修正：此前缺失 → game.flow 恒 null（C# 不消费该字段，
    //     行为等价，但按原版补全构造/拷贝链）。
    if !sim_data.flow.ptr.is_null() && !game_data.flow.ptr.is_null() {
        let sim_total = sim_w * sim_data.height as usize;
        let game_total = game_w * game_h;
        if game_total > 0 {
            unsafe {
                let src = std::slice::from_raw_parts(
                    sim_data.flow.ptr,
                    sim_total,
                );
                let dst = std::slice::from_raw_parts_mut(game_data.flow.ptr, game_total);
                for row in 0..game_h {
                    let sim_row_start = (row + 1) * sim_w + 1;
                    dst[row * game_w..(row + 1) * game_w]
                        .copy_from_slice(&src[sim_row_start..sim_row_start + game_w]);
                }
            }
        }
    }

    // 5b. accumulated_flow 同步（原版 11_msvcrt_ignored.c L32491-32498）：
    //     按行 memcpy（去掉边界）SimData.accumulatedFlow → GameData.accumulatedFlow，
    //     随后局部静态 tick_count 超过 14 时 memset 清零 SimData 源缓冲。
    //     C# 悬停显示 "Emitting {Element}: {FlowRate}" 读 GameData.accumulatedFlow
    //     （SelectToolHoverTextCard L745：AccumulatedFlow/3），缺失此同步 → 恒显示 0。
    if !sim_data.accumulated_flow.ptr.is_null() && !game_data.accumulated_flow.ptr.is_null() {
        let sim_total = sim_w * sim_data.height as usize;
        let game_total = game_w * game_h;
        if game_total > 0 {
            unsafe {
                let src = std::slice::from_raw_parts(sim_data.accumulated_flow.ptr, sim_total);
                let dst = std::slice::from_raw_parts_mut(
                    game_data.accumulated_flow.ptr,
                    game_total,
                );
                for row in 0..game_h {
                    let sim_row_start = (row + 1) * sim_w + 1;
                    dst[row * game_w..(row + 1) * game_w]
                        .copy_from_slice(&src[sim_row_start..sim_row_start + game_w]);
                }
            }
        }
        // 原版：tick_count = tick_count + 1; if (0xe < tick_count) { memset(...); tick_count = 0; }
        // memset 清零 SimData 源缓冲（写入侧），窗口语义：显示维持至多 15 帧后归零。
        let tick = ACCUMULATED_FLOW_RESET_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if tick > 14 {
            unsafe {
                let src = std::slice::from_raw_parts_mut(sim_data.accumulated_flow.ptr, sim_total);
                src.fill(0.0);
            }
            ACCUMULATED_FLOW_RESET_COUNT.store(0, Ordering::Relaxed);
        }
    }

    // 6. 液体纹理更新（对照源码 11_msvcrt_ignored.c L32496-32500：UpdateFlowTexture
    //    + UpdateExposedToSunPropertyTexture + UpdateLiquidPropertyTexture）
    //    必须放在 cells 拷贝之后（纹理函数读 sim.updated_cells 对比 cells）
    crate::c2_physics::property_texture::update_flow_texture(sim_data, game_data);
    crate::c2_physics::property_texture::update_exposed_to_sun_property_texture(sim_data, game_data);
    crate::c2_physics::property_texture::update_liquid_property_texture(sim_data, game_data);
}

/// SubstanceChangeInfo 后处理（对照源码 L132194-132373）。
///
/// 1. 按 cellIdx 排序 + 相邻同 cellIdx 去重（源码 L132227-132290）
/// 2. 清空 game_data 的 solidInfo/liquidChangeInfo/solidSubstanceChangeInfo
///    （源码 L132215-132219）
/// 3. 遍历每条 entry：old = old_game_data.cells（C# 上次可见状态），
///    new = game_data.cells（新状态），回填 old/new 元素索引
///    （源码 L132321-132337）
/// 4. old≠new 时派生事件（源码 L132338-132371）：
///    - 固态性变化 → solidInfo（C# Grid.SetSolid：挖掘任务能否结束的关键）
///    - 任一侧为固体 → solidSubstanceChangeInfo（C# MarkDirty：画面刷新）
///    - 任一侧为液体 → liquidChangeInfo
fn post_process_substance_change_info(game_data: &mut GameData, old_game_data: *const GameData) {
    use crate::a_framework::game_data::{LiquidChangeInfo, SolidInfo, SolidSubstanceChangeInfo};

    // 清空派生向量（对照源码 L132215-132219，无论后续是否生成都先清空）
    game_data.solid_info.clear_keep_capacity();
    game_data.liquid_change_info.clear_keep_capacity();
    game_data.solid_substance_change_info.clear_keep_capacity();

    if game_data.cells.ptr.is_null() || old_game_data.is_null() {
        return;
    }
    let old_cells_ptr = unsafe { (*old_game_data).cells.ptr };
    if old_cells_ptr.is_null() {
        return;
    }

    // 1. 取出 entries，按 cellIdx 排序 + 相邻去重（保留首次出现）
    let mut entries: Vec<crate::a_framework::game_data::SubstanceChangeInfo> =
        game_data.substance_change_info.as_slice().to_vec();
    if entries.is_empty() {
        return;
    }
    entries.sort_by_key(|e| e.cell_idx);
    entries.dedup_by_key(|e| e.cell_idx);

    let new_cells = unsafe { &*game_data.cells.ptr };
    let old_cells = unsafe { &*old_cells_ptr };
    let cell_count = new_cells.element_idx.len().min(old_cells.element_idx.len());

    // 3-4. 回填 old/new 并派生事件
    for entry in entries.iter_mut() {
        let cell = entry.cell_idx as usize;
        if cell >= cell_count {
            continue;
        }
        let old_elem = old_cells.element_idx.get(cell);
        let new_elem = new_cells.element_idx.get(cell);
        entry.old_element_idx = old_elem;
        entry.new_element_idx = new_elem;

        if old_elem == new_elem {
            continue;
        }

        // 状态判定：0=Vacuum, 1=Gas, 2=Liquid, 3=Solid
        let state_of = |elem: u16| -> u8 {
            crate::b_elements::elements_table::get_element_state_by_idx(elem).unwrap_or(0)
        };
        let solid_old = state_of(old_elem) == 3;
        let solid_new = state_of(new_elem) == 3;
        let liquid_old = state_of(old_elem) == 2;
        let liquid_new = state_of(new_elem) == 2;

        if solid_old != solid_new {
            game_data.solid_info.push(SolidInfo {
                cell_idx: entry.cell_idx,
                solid: solid_new as i32,
            });
        }
        if solid_old || solid_new {
            game_data.solid_substance_change_info.push(SolidSubstanceChangeInfo {
                cell_idx: entry.cell_idx,
            });
        }
        if liquid_old || liquid_new {
            game_data.liquid_change_info.push(LiquidChangeInfo {
                cell_idx: entry.cell_idx,
            });
        }
    }

    // 5. 写回 game_data.substance_change_info
    game_data.substance_change_info.clear_keep_capacity();
    for entry in entries {
        game_data.substance_change_info.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::{CellSOA, BackwallSOA};
    use crate::a_framework::sim_events::SimEvents;
    use crate::a_framework::stl_shim::UniquePtr;
    use crate::LIB_TESTS_LOCK;

    /// 构造测试用 SimData（4×4 含边界 = 6×6 = 36 个 cell）
    fn make_test_sim_data() -> SimData {
        let mut sd = SimData::new_zeroed();
        sd.width = 6;
        sd.height = 6;
        sd.num_game_cells = 4 * 4;
        sd.vacuum_element_idx = 1; // 测试用 vacuum idx = 1

        let total_cells = (sd.width as usize) * (sd.height as usize);

        // 分配 cells（填 0）
        let cells = Box::new(CellSOA::with_size(total_cells));
        sd.cells = UniquePtr { ptr: Box::into_raw(cells) };

        // 分配 updated_cells（填测试数据）
        let mut updated_cells = CellSOA::with_size(total_cells);
        // 在 cell_idx=5 处填非零数据
        updated_cells.element_idx.set(5, 100u16);
        updated_cells.mass.set(5, 50.0f32);
        updated_cells.temperature.set(5, 300.0f32);
        updated_cells.disease_idx.set(5, 0x05u8);
        updated_cells.disease_count.set(5, 42i32);
        updated_cells.disease_infestation_tick_count.set(5, 7u8);
        updated_cells.disease_growth_accumulated_error.set(5, 1.5f32);
        sd.updated_cells = UniquePtr { ptr: Box::into_raw(Box::new(updated_cells)) };

        // 分配 backwall
        let mut backwall = BackwallSOA::with_size(total_cells, sd.vacuum_element_idx);
        backwall.element_idx.set(5, 200u16);
        backwall.mass.set(5, 99.0f32);
        backwall.temperature.set(5, 250.0f32);
        sd.backwall = UniquePtr { ptr: Box::into_raw(Box::new(backwall)) };

        // 分配 sim_events
        let sim_events = Box::new(SimEvents::default());
        sd.sim_events = UniquePtr { ptr: Box::into_raw(sim_events) };

        sd
    }

    /// 构造测试用 GameData（4×4 不含边界 = 16 个 cell）
    fn make_test_game_data() -> GameData {
        GameData::new(4, 4)
    }

    #[test]
    fn clear_cell_zeros_seven_fields() {
        let mut sd = make_test_sim_data();

        // 验证测试数据已设置
        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.element_idx.get(5), 100u16);
        assert_eq!(updated_cells.mass.get(5), 50.0f32);

        // 执行 clear_cell
        clear_cell(&mut sd, 5);

        // 验证 7 个字段已清零
        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.element_idx.get(5), sd.vacuum_element_idx, "element_idx should be vacuum");
        assert_eq!(updated_cells.mass.get(5), 0.0f32, "mass should be 0");
        assert_eq!(updated_cells.temperature.get(5), 0.0f32, "temperature should be 0");
        assert_eq!(updated_cells.disease_idx.get(5), 0xFFu8, "disease_idx should be 0xFF");
        assert_eq!(updated_cells.disease_count.get(5), 0i32, "disease_count should be 0");
        assert_eq!(updated_cells.disease_infestation_tick_count.get(5), 0u8, "disease_infestation_tick_count should be 0");
        assert_eq!(updated_cells.disease_growth_accumulated_error.get(5), 0.0f32, "disease_growth_accumulated_error should be 0");
    }

    #[test]
    fn clear_backwall_zeros_three_fields() {
        let mut sd = make_test_sim_data();

        // 验证测试数据已设置
        let backwall = unsafe { &*sd.backwall.ptr };
        assert_eq!(backwall.element_idx.get(5), 200u16);
        assert_eq!(backwall.mass.get(5), 99.0f32);

        // 执行 clear_backwall
        clear_backwall(&mut sd, 5);

        // 验证 3 个字段已清零
        let backwall = unsafe { &*sd.backwall.ptr };
        assert_eq!(backwall.element_idx.get(5), sd.vacuum_element_idx, "element_idx should be vacuum");
        assert_eq!(backwall.mass.get(5), 0.0f32, "mass should be 0");
        assert_eq!(backwall.temperature.get(5), 0.0f32, "temperature should be 0");
    }

    #[test]
    fn copy_updated_cells_to_cells_copies_all_fields() {
        let mut sd = make_test_sim_data();

        // 执行 copy
        copy_updated_cells_to_cells(&mut sd);

        // 验证 cells 已被 updated_cells 覆盖
        let cells = unsafe { &*sd.cells.ptr };
        assert_eq!(cells.element_idx.get(5), 100u16);
        assert_eq!(cells.mass.get(5), 50.0f32);
        assert_eq!(cells.temperature.get(5), 300.0f32);
    }

    /// T2 回归：区域局部 CopyFrom 只复制区域矩形 + halo 边界 1 格（并行路径免全网格拷贝）。
    #[test]
    fn copy_region_copies_rectangle_with_halo() {
        let mut sd = make_test_sim_data();
        unsafe {
            let upd = &mut *sd.updated_cells.ptr;
            upd.element_idx.set(7, 111); // region interior (1,1)
            upd.mass.set(14, 4.0);       // region interior (2,2)
            upd.element_idx.set(0, 222); // halo (0,0) = (min-1, min-1)
        }
        let bounds = crate::d1_activity::RegionBounds {
            min_x: 1,
            min_y: 1,
            max_x: 3,
            max_y: 3,
        };
        copy_updated_cells_to_cells_region(&sd, &bounds);
        unsafe {
            let cells = &*sd.cells.ptr;
            assert_eq!(cells.element_idx.get(7), 111, "region interior 应复制");
            assert_eq!(cells.mass.get(14), 4.0, "region interior mass 应复制");
            assert_eq!(cells.element_idx.get(0), 222, "halo (min-1) 应复制");
            // region 外（如 cell 35 = row5 col5）不应被本函数触碰（保持 cells 初始 0）
            assert_eq!(cells.element_idx.get(35), 0);
        }
    }

    #[test]
    fn copy_region_empty_bounds_is_noop() {
        let mut sd = make_test_sim_data();
        let bounds = crate::d1_activity::RegionBounds {
            min_x: 3,
            min_y: 3,
            max_x: 2,
            max_y: 2,
        };
        copy_updated_cells_to_cells_region(&sd, &bounds); // 不 panic，不复制
        unsafe {
            let cells = &*sd.cells.ptr;
            assert_eq!(cells.element_idx.get(7), 0, "空区域不应复制");
        }
    }

    #[test]
    fn copy_sim_data_to_game_swaps_events_and_updates_frame_count() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = make_test_sim_data();
        let mut gd = make_test_game_data();

        // 在 sim_events.dig_info 中放入一个测试事件
        let sim_events = unsafe { &mut *sd.sim_events.ptr };
        let test_dig = crate::a_framework::game_data::SpawnOreInfo {
            cell_idx: 42,
            elem_idx: 5,
            disease_idx: 0,
            pad: 0,
            mass: 10.0,
            temperature: 300.0,
            disease_count: 0,
        };
        sim_events.dig_info.push(test_dig);

        // 执行 copy（old_game_data 传 null：跳过后处理，仅验证搬移与帧数）
        copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 7);

        // 验证 num_frames_processed 已更新
        assert_eq!(gd.num_frames_processed, 7);

        // 验证 dig_info 已 swap 到 game_data
        assert_eq!(gd.dig_info.len(), 1);
        let dig = gd.dig_info.get(0);
        assert_eq!(dig.cell_idx, 42);
        assert_eq!(dig.mass, 10.0);

        // 验证 sim_events.dig_info 已被 swap 为空（原 game_data 的空 vector）
        let sim_events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(sim_events.dig_info.len(), 0);
    }

    #[test]
    fn clear_cell_with_null_ptr_does_not_panic() {
        let mut sd = SimData::new_zeroed();
        // updated_cells.ptr 为 null
        clear_cell(&mut sd, 0); // 应该只记录警告，不 panic
    }

    /// 验证 copy_sim_data_to_game 逐行拷贝（去掉边界）的正确性。
    ///
    /// SimData: 6×6（含边界），updated_cells 尺寸 = 36
    /// GameData: 4×4（不含边界），cells 尺寸 = 16
    ///
    /// 在 updated_cells 的内部 cell（row=1, col=1 到 row=4, col=4）中填入测试数据，
    /// 验证 game_data.cells 中对应位置的数据正确，且尺寸不会被错误扩展。
    #[test]
    fn copy_sim_data_to_game_strips_border_correctly() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = make_test_sim_data();
        let mut gd = make_test_game_data();

        // SimData: width=6, height=6（含边界）
        // GameData: width=4, height=4（不含边界）
        // 内部 cell 的 sim 坐标：row=1..4, col=1..4
        // 对应 game 坐标：row=0..3, col=0..3

        // 在 updated_cells 的内部 cell (sim row=1, col=1) = sim_idx = 1*6+1 = 7 处填测试数据
        let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
        updated_cells.element_idx.set(7, 111u16);
        updated_cells.mass.set(7, 222.0f32);
        updated_cells.temperature.set(7, 333.0f32);

        // 在 updated_cells 的内部 cell (sim row=2, col=3) = sim_idx = 2*6+3 = 15 处填测试数据
        updated_cells.element_idx.set(15, 444u16);
        updated_cells.mass.set(15, 555.0f32);

        // 执行 copy（old_game_data 传 null：跳过后处理，仅验证去边界拷贝）
        copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 1);

        // 验证 game_data.cells 尺寸为 16（4×4），没有被错误扩展
        let game_cells = unsafe { &*gd.cells.ptr };
        assert_eq!(game_cells.element_idx.len(), 16, "game_data.cells should be 16 (4x4)");

        // 验证 (sim row=1, col=1) → (game row=0, col=0) = game_idx = 0
        assert_eq!(game_cells.element_idx.get(0), 111u16, "game[0,0] element_idx");
        assert_eq!(game_cells.mass.get(0), 222.0f32, "game[0,0] mass");
        assert_eq!(game_cells.temperature.get(0), 333.0f32, "game[0,0] temperature");

        // 验证 (sim row=2, col=3) → (game row=1, col=2) = game_idx = 1*4+2 = 6
        assert_eq!(game_cells.element_idx.get(6), 444u16, "game[1,2] element_idx");
        assert_eq!(game_cells.mass.get(6), 555.0f32, "game[1,2] mass");
    }

    /// 验证 copy_sim_data_to_game 把 SimData.accumulated_flow 逐行（去掉边界）
    /// 同步到 GameData.accumulated_flow —— C# 悬停显示 "Emitting {Element}: {FlowRate}"
    /// 读取的就是该缓冲（SelectToolHoverTextCard L745：AccumulatedFlow / 3）。
    ///
    /// SimData: 6×6（含边界）→ 内部格 sim 坐标 row=1..4, col=1..4
    /// GameData: 4×4（不含边界）→ game 坐标 row=0..3, col=0..3
    #[test]
    fn copy_sim_data_to_game_copies_accumulated_flow_stripped() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_accumulated_flow_reset_count_for_tests();
        let mut sd = make_test_sim_data();
        let mut gd = make_test_game_data();

        // 分配 SimData.accumulated_flow（36 格），在内部格填入升华累积量
        let acc: Vec<f32> = vec![0.0f32; 36];
        sd.accumulated_flow = UniquePtr { ptr: acc.leak().as_mut_ptr() };
        unsafe {
            let src = std::slice::from_raw_parts_mut(sd.accumulated_flow.ptr, 36);
            src[7] = 0.5;  // sim (row=1, col=1) → game (0,0)
            src[15] = 0.25; // sim (row=2, col=3) → game (1,2)
            src[0] = 9.0;  // 边界格不应被拷贝
        }

        copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 1);

        let dst = unsafe { std::slice::from_raw_parts(gd.accumulated_flow.ptr, 16) };
        assert!((dst[0] - 0.5).abs() < 1e-6, "game[0,0] 应为 sim 内部格累积量 0.5");
        assert!((dst[6] - 0.25).abs() < 1e-6, "game[1,2] 应为 0.25");
        assert_eq!(dst[3], 0.0, "game 边界行（sim 边界格 9.0）不应被拷贝");
    }

    /// 2026-08-04 审查修正：copy_sim_data_to_game 逐行拷贝 sim.flow → game.flow
    /// （原版 copyFlowTasks，11_msvcrt_ignored.c L31263-31350，Vector4f 16B/格）。
    /// 此前 game.flow 恒 null；此测试验证去边界拷贝。
    #[test]
    fn copy_sim_data_to_game_copies_flow_stripped() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = make_test_sim_data();
        let mut gd = make_test_game_data();

        // 分配 SimData.flow（36 格 Vector4f）
        let flow_vec: Vec<crate::a_framework::vector_math::Vector4f> =
            vec![crate::a_framework::vector_math::Vector4f::default(); 36];
        sd.flow = UniquePtr { ptr: flow_vec.leak().as_mut_ptr() };
        unsafe {
            let src = std::slice::from_raw_parts_mut(sd.flow.ptr, 36);
            src[7] = crate::a_framework::vector_math::Vector4f {
                x: 1.0, y: 2.0, z: 3.0, w: 4.0,
            }; // sim (row=1,col=1) → game (0,0)
            src[15] = crate::a_framework::vector_math::Vector4f {
                x: 5.0, y: 6.0, z: 7.0, w: 8.0,
            }; // sim (row=2,col=3) → game (1,2)
            src[0] = crate::a_framework::vector_math::Vector4f {
                x: 9.0, y: 9.0, z: 9.0, w: 9.0,
            }; // 边界格不应被拷贝
        }

        copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 1);

        let dst = unsafe { std::slice::from_raw_parts(gd.flow.ptr, 16) };
        assert!((dst[0].x - 1.0).abs() < 1e-6, "game[0,0].x 应为 1.0");
        assert!((dst[0].w - 4.0).abs() < 1e-6, "game[0,0].w 应为 4.0");
        assert!((dst[6].y - 6.0).abs() < 1e-6, "game[1,2].y 应为 6.0");
        assert_eq!(dst[3].x, 0.0, "game 边界行（sim 边界格 9.0）不应被拷贝");
    }

    /// 2026-08-06 气体纹理根因：Sim::Main 每帧清零 flow（帧内增量累加器）。
    /// 整张数组（含边界）清为 Vector4f::default()，物理只写不读 flow，零副作用。
    #[test]
    fn clear_flow_zeroes_entire_array() {
        let mut sd = make_test_sim_data();
        // 分配 flow（36 格 Vector4f）
        let flow_vec: Vec<crate::a_framework::vector_math::Vector4f> =
            vec![crate::a_framework::vector_math::Vector4f::default(); 36];
        sd.flow = UniquePtr { ptr: flow_vec.leak().as_mut_ptr() };
        unsafe {
            let src = std::slice::from_raw_parts_mut(sd.flow.ptr, 36);
            for v in src.iter_mut() {
                *v = crate::a_framework::vector_math::Vector4f {
                    x: 1.0, y: 2.0, z: 3.0, w: 4.0,
                };
            }
        }
        clear_flow(&mut sd);
        unsafe {
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 36);
            for v in flow {
                assert_eq!(*v, crate::a_framework::vector_math::Vector4f::default());
            }
        }
    }

    /// 按区域清 flow 的对照基准：先造一份「全脏」的 flow 缓冲（36 格非零）。
    fn make_dirty_flow(sd: &mut SimData) -> crate::a_framework::vector_math::Vector4f {
        let flow_vec: Vec<crate::a_framework::vector_math::Vector4f> =
            vec![crate::a_framework::vector_math::Vector4f::default(); 36];
        sd.flow = UniquePtr {
            ptr: flow_vec.leak().as_mut_ptr(),
        };
        let dirty = crate::a_framework::vector_math::Vector4f {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            w: 4.0,
        };
        unsafe {
            for v in std::slice::from_raw_parts_mut(sd.flow.ptr, 36).iter_mut() {
                *v = dirty;
            }
        }
        dirty
    }

    /// clear_flow_regions 只清活动区域内的格子，区域外原样保留。
    ///
    /// 这是相对 clear_flow 的核心差异，也是省下死区 memset 的依据：
    /// 区域外的 flow 恒为 0（分配时零初始化 + 物理只在区域内写），
    /// 每帧重复清零纯属浪费。
    ///
    /// 6×6 网格，区域 (0,0)-(3,3) 经 compute_region_bounds 钳制为
    /// x∈[1,3)、y∈[1,3) → 索引 7、8、13、14。
    #[test]
    fn clear_flow_regions_only_touches_active_region() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = make_test_sim_data();
        let dirty = make_dirty_flow(&mut sd);

        sd.active_regions.push(crate::a_framework::sim_data::ActiveRegion {
            min_x: 0,
            min_y: 0,
            max_x: 3,
            max_y: 3,
            ..Default::default()
        });

        clear_flow_regions(&mut sd);

        let zero = crate::a_framework::vector_math::Vector4f::default();
        unsafe {
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 36);
            for i in [7usize, 8, 13, 14] {
                assert_eq!(flow[i], zero, "区域内 cell {i} 应被清零");
            }
            for i in 0..36 {
                if matches!(i, 7 | 8 | 13 | 14) {
                    continue;
                }
                assert_eq!(flow[i], dirty, "区域外 cell {i} 不应被动");
            }
        }
    }

    /// 无活动区域（首帧 / 刚 AllocateCells）时退回全网格清零，
    /// 行为与 clear_flow 完全一致——保证改造不引入行为漂移。
    #[test]
    fn clear_flow_regions_falls_back_to_full_clear_without_regions() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = make_test_sim_data();
        let _dirty = make_dirty_flow(&mut sd);

        assert!(
            sd.active_regions.is_empty(),
            "前置条件：本用例的 active_regions 应为空"
        );

        clear_flow_regions(&mut sd);

        unsafe {
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 36);
            for v in flow {
                assert_eq!(
                    *v,
                    crate::a_framework::vector_math::Vector4f::default(),
                    "无区域时应退回全清"
                );
            }
        }
    }

    /// 验证 accumulated_flow 的 15 帧清零窗口（原版 CopySimDataToGame 局部静态
    /// tick_count：tick_count+1 后 > 0xe 时 memset 清零 SimData 源缓冲）。
    /// 语义：第 15 次调用仍把"窗口内累积量"交给 GameData，随后源归零；
    /// 第 16 次调用起 GameData 读到 0 —— 即 C# 显示只维持一个窗口。
    #[test]
    fn copy_sim_data_to_game_resets_accumulated_flow_after_15_calls() {
        let _lock = LIB_TESTS_LOCK.lock();
        reset_accumulated_flow_reset_count_for_tests();
        let mut sd = make_test_sim_data();
        let mut gd = make_test_game_data();

        let acc: Vec<f32> = vec![0.0f32; 36];
        sd.accumulated_flow = UniquePtr { ptr: acc.leak().as_mut_ptr() };
        unsafe {
            let src = std::slice::from_raw_parts_mut(sd.accumulated_flow.ptr, 36);
            src[7] = 0.5;
        }

        // 前 14 次：GameData 一直读到 0.5，SimData 源保持不变
        for _ in 0..14 {
            copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 1);
            let dst = unsafe { std::slice::from_raw_parts(gd.accumulated_flow.ptr, 16) };
            assert!((dst[0] - 0.5).abs() < 1e-6, "窗口内 GameData 应持续读到 0.5");
            let src = unsafe { std::slice::from_raw_parts(sd.accumulated_flow.ptr, 36) };
            assert!((src[7] - 0.5).abs() < 1e-6, "第 14 次前 SimData 源不应被清零");
        }

        // 第 15 次：先拷贝（GameData 仍读到 0.5），随后 SimData 源被 memset 清零
        copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 1);
        let dst = unsafe { std::slice::from_raw_parts(gd.accumulated_flow.ptr, 16) };
        assert!((dst[0] - 0.5).abs() < 1e-6, "第 15 次拷贝应交付窗口值 0.5");
        let src = unsafe { std::slice::from_raw_parts(sd.accumulated_flow.ptr, 36) };
        assert_eq!(src[7], 0.0, "第 15 次调用后 SimData 源应被清零");

        // 第 16 次：GameData 读到 0
        copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 1);
        let dst = unsafe { std::slice::from_raw_parts(gd.accumulated_flow.ptr, 16) };
        assert_eq!(dst[0], 0.0, "清零后 GameData 应读到 0");
    }

    /// 回归：emitted_mass_info 清空必须**逐条目重置保持长度**（原版 L31641-31648）。
    /// 若用 clear_keep_capacity()（长度→0），无时间帧（update 不跑）后 sim 侧向量为空，
    /// 下一次交换把空向量给 game → GDU emittedMassEntries=null → C# BuildingElementEmitter NRE。
    #[test]
    fn copy_sim_data_to_game_keeps_emitted_mass_vector_length() {
        let _lock = LIB_TESTS_LOCK.lock();
        crate::c_simulation::element_emitter::reset_for_test();
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd.vacuum_element_idx = 0;
        // 注册 1 个发射器：add() 会把 sim 输出向量 resize 到 1
        let msg = crate::c_simulation::element_emitter::AddElementEmitterMsg {
            max_pressure: 2.0,
            ..Default::default()
        };
        crate::c_simulation::element_emitter::add_element_emitter(&mut sd, &msg);
        assert_eq!(sd.element_emitter.emitted_mass_info.len(), 1);

        let mut gd = GameData::new(4, 4);
        // 第一次 copy：game 拿到长度 1
        copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 1);
        assert_eq!(gd.emitted_mass_info.len(), 1, "首次 copy 后 game 应为 1");

        // 第二次 copy（模拟无时间帧，update 不跑，sim 侧未重新 resize）：
        // 清空必须保持长度，game 仍拿到 1 且指针非空
        copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 2);
        assert_eq!(
            gd.emitted_mass_info.len(),
            1,
            "无时间帧后 game 输出向量长度必须保持（否则 GDU null → C# NRE）"
        );
        assert!(!gd.emitted_mass_info.as_ptr().is_null());
    }

    /// 验证 SimData::new_for_allocate 分配了 sim_events。
    /// C1 修复：原 A3 阶段遗漏了 sim_events 分配。
    #[test]
    fn new_for_allocate_allocates_sim_events() {
        let _lock = LIB_TESTS_LOCK.lock();
        let sd = SimData::new_for_allocate(4, 4, 12345, false, false);
        assert!(!sd.sim_events.ptr.is_null(), "sim_events should be allocated");
        assert!(!sd.updated_cells.ptr.is_null(), "updated_cells should be allocated");
        assert!(!sd.backwall.ptr.is_null(), "backwall should be allocated");
    }

    /// 验证 SubstanceChangeInfo 后处理（对照源码 L132194-132373）：
    /// 固体 → 真空的变化应派生 solidInfo(isSolid=0) 和 solidSubstanceChangeInfo，
    /// 并回填 old/new 元素索引。
    #[test]
    fn post_process_derives_solid_events_for_dug_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 准备元素状态表：idx 100 = 固体(3)，idx 1 = 真空(0)
        {
            let mut table = crate::b_elements::elements_table::G_ELEMENTS_TABLE.lock().unwrap();
            table.state_data.clear();
            table.state_data.resize(128, crate::b_elements::element::ElementStateData::default());
            table.state_data[100].state = 3; // 固体
            table.state_data[1].state = 0;   // 真空
        }

        let mut sd = make_test_sim_data();
        let mut gd = make_test_game_data();
        let mut old_gd = make_test_game_data();

        // 旧可见缓冲：game cell 0 是固体 100
        unsafe { &mut *old_gd.cells.ptr }.element_idx.set(0, 100u16);

        // sim 侧：内部 cell (1,1) → sim_idx 7 → game cell 0，已被挖为真空 1
        let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
        updated_cells.element_idx.set(7, 1u16);

        // sim_events 中放入一条占位 substance_change_info（0xFFFF 占位，模仿 process_dig_points）
        let sim_events = unsafe { &mut *sd.sim_events.ptr };
        sim_events.substance_change_info.push(crate::a_framework::game_data::SubstanceChangeInfo {
            cell_idx: 0,
            old_element_idx: 0xFFFF,
            new_element_idx: 0xFFFF,
        });

        copy_sim_data_to_game(&mut sd, &mut gd, &old_gd as *const GameData, 1);

        // 验证回填：old=100（旧可见），new=1（新状态）
        assert_eq!(gd.substance_change_info.len(), 1);
        let entry = gd.substance_change_info.get(0);
        assert_eq!(entry.old_element_idx, 100, "old 应回填为旧可见元素");
        assert_eq!(entry.new_element_idx, 1, "new 应回填为新元素");

        // 验证派生：固体→真空，应有 solidInfo(solid=0) 和 solidSubstanceChangeInfo
        assert_eq!(gd.solid_info.len(), 1, "应派生 1 条 solidInfo");
        assert_eq!(gd.solid_info.get(0).cell_idx, 0);
        assert_eq!(gd.solid_info.get(0).solid, 0, "挖掘后 isSolid 应为 0");
        assert_eq!(gd.solid_substance_change_info.len(), 1, "应派生 1 条 solidSubstanceChangeInfo");
        assert_eq!(gd.liquid_change_info.len(), 0, "不涉及液体");

        // 清理全局元素表，避免影响其他测试
        let mut table = crate::b_elements::elements_table::G_ELEMENTS_TABLE.lock().unwrap();
        table.state_data.clear();
    }

    /// 验证事件搬移后 sim_events 源被清空（对照源码 L132195-132214），
    /// 防止事件乒乓重复投递。
    #[test]
    fn copy_sim_data_to_game_clears_event_source() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = make_test_sim_data();
        let mut gd = make_test_game_data();

        // game_data 中预置一条"已被 C# 消费"的旧 dig_info（模拟上轮残留）
        gd.dig_info.push(crate::a_framework::game_data::SpawnOreInfo {
            cell_idx: 99, elem_idx: 1, disease_idx: 0, pad: 0,
            mass: 1.0, temperature: 300.0, disease_count: 0,
        });

        copy_sim_data_to_game(&mut sd, &mut gd, std::ptr::null(), 1);

        // 搬移后 sim_events 应为空（旧内容被丢弃，不会乒乓回 game_data）
        let sim_events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(sim_events.dig_info.len(), 0, "sim_events 源应被清空");
        assert_eq!(gd.dig_info.len(), 0, "残留事件不应再次出现");
    }
}
