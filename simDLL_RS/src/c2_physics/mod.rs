//! C2 阶段物理模拟桩实现。
//!
//! C1 阶段：所有函数为空桩，不执行任何物理计算。
//! C2 阶段：逐步替换为真实物理计算（温度/液体/气体/辐射/疾病）。

#![allow(dead_code)]

use crate::a_framework::sim_data::SimData;

pub mod liquid_flow;
pub mod property_texture;
pub mod temperature;

/// SimBase::UpdateData 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c L41811。
/// 原版调用 UpdateTemperature/UpdatePressure/UpdateLiquid 等物理更新。
/// C1 桩：不执行任何物理计算，cell 状态保持不变。
pub fn update_data(sim_data: &mut SimData) {
    // 2026-08-04 D1 探针（临时）：首帧统计整网格 vs 活动区域覆盖格数，
    // 量化"只算已发现世界矩形"的裁剪收益。
    static UPDATE_DATA_REGION_DIAG: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    if !UPDATE_DATA_REGION_DIAG.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let total = (sim_data.width as usize).saturating_mul(sim_data.height as usize);
        let regions = sim_data.active_regions.as_slice();
        let mut region_cells = 0u64;
        let mut region_list: Vec<(i32, i32, i32, i32, u64)> = Vec::new();
        for r in regions {
            let w = (r.max_x - r.min_x + 1).max(0) as u64;
            let h = (r.max_y - r.min_y + 1).max(0) as u64;
            region_cells += w * h;
            if region_list.len() < 8 {
                region_list.push((r.min_x, r.min_y, r.max_x, r.max_y, w * h));
            }
        }
        tracing::info!(
            total,
            region_count = regions.len(),
            region_cells,
            ?region_list,
            "update_data: grid vs active-region coverage diag"
        );
    }
    // 2026-08-04 D1 探针（临时）：验证
    // ① 活动区域之外的格（未发现星球/太空缝隙）状态不变化（原版"UNKNOWN 永不变化"）；
    // ② 活动区域内（含已发现世界里的 UNKNOWN 迷雾格）状态应变化（区域=整块世界矩形，
    //    已发现世界内没有逐格可见性门控）。
    // 区域外 2 样本 + 区域内 2 样本（优先选液体/气体格），第 1 次与第 60 次各记录比较。
    {
        use std::sync::atomic::{AtomicU32, Ordering};
        static FROZEN_PROBE_COUNT: AtomicU32 = AtomicU32::new(0);
        static FROZEN_PROBE_RECORDED: std::sync::Mutex<Option<Vec<(usize, u16, f32, f32)>>> =
            std::sync::Mutex::new(None);
        let n = FROZEN_PROBE_COUNT.fetch_add(1, Ordering::Relaxed);
        if n == 0 || n == 60 {
            // 采样：活动区域之外的格（取区域矩形外固定偏移，含世界之间缝隙/未发现世界）。
            let bounds_list: Vec<crate::d1_activity::RegionBounds> = sim_data
                .active_regions
                .as_slice()
                .iter()
                .map(|r| crate::d1_activity::compute_region_bounds(sim_data, r))
                .collect();
            let w = sim_data.width as usize;
            let h = sim_data.height as usize;
            let mut samples: Vec<(usize, u16, f32, f32)> = Vec::new();
            let candidates = [
                (w / 4, h / 2),
                (w / 2, h / 4),
                (w * 3 / 4, h / 2),
            ];
            // 区域外样本
            for (x, y) in candidates {
                let cell = y * w + x;
                let in_region = bounds_list.iter().any(|b| {
                    x >= b.min_x && x <= b.max_x && y >= b.min_y && y <= b.max_y
                });
                if !in_region && samples.len() < 2 && !sim_data.updated_cells.ptr.is_null() {
                    let c = unsafe { &*sim_data.updated_cells.ptr };
                    if cell < c.element_idx.len() {
                        samples.push((cell, c.element_idx.get(cell), c.mass.get(cell), c.temperature.get(cell)));
                    }
                }
            }
            // 区域内样本：优先选液体(state&3==2)/气体(state&3==1)格
            if let Some(first) = bounds_list.first() {
                let mid_x = (first.min_x + first.max_x) / 2;
                let mid_y = (first.min_y + first.max_y) / 2;
                let mut picked = Vec::new();
                for dy in 0..8i64 {
                    for dx in 0..8i64 {
                        let x = mid_x as i64 + dx * 16;
                        let y = mid_y as i64 + dy * 16;
                        if x > first.max_x as i64 || y > first.max_y as i64 {
                            continue;
                        }
                        let cell = (y as usize) * w + (x as usize);
                        if !sim_data.updated_cells.ptr.is_null() {
                            let c = unsafe { &*sim_data.updated_cells.ptr };
                            if cell < c.element_idx.len() {
                                let e = c.element_idx.get(cell);
                                let state = crate::b_elements::elements_table::get_element_post_process_data(e)
                                    .map(|p| p.state & 3)
                                    .unwrap_or(0);
                                if (state == 1 || state == 2) && c.mass.get(cell) > 0.0 {
                                    picked.push((cell, e, c.mass.get(cell), c.temperature.get(cell)));
                                    if picked.len() >= 2 {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if picked.len() >= 2 {
                        break;
                    }
                }
                if picked.is_empty() {
                    // 回退：区域内任意格（含固体）
                    let cell = mid_y * w + mid_x;
                    if !sim_data.updated_cells.ptr.is_null() {
                        let c = unsafe { &*sim_data.updated_cells.ptr };
                        if cell < c.element_idx.len() {
                            picked.push((cell, c.element_idx.get(cell), c.mass.get(cell), c.temperature.get(cell)));
                        }
                    }
                }
                samples.extend(picked);
            }
            if n == 0 {
                if samples.is_empty() {
                    tracing::info!("frozen-probe: no cells outside active regions (single-star or full coverage)");
                } else {
                    *FROZEN_PROBE_RECORDED.lock().unwrap() = Some(samples.clone());
                    tracing::info!(
                        ?samples,
                        "frozen-probe: first sample (first 2 = outside region, rest = inside region)"
                    );
                }
            } else if n == 60 {
                let first = FROZEN_PROBE_RECORDED.lock().unwrap().clone();
                let mut changed = Vec::new();
                if let Some(f) = &first {
                    for (a, s) in f.iter().zip(samples.iter()) {
                        let same = a.0 == s.0
                            && a.1 == s.1
                            && (a.2 - s.2).abs() < 1e-6
                            && (a.3 - s.3).abs() < 1e-6;
                        changed.push((s.0, !same));
                    }
                }
                tracing::info!(
                    ?samples,
                    ?changed,
                    "frozen-probe: 60th sample vs first (outside expect false=unchanged; inside expect true=changed)"
                );
            }
        }
    }
    // 原版 UpdateData L144415：每子步开头 CopyFrom(cells, updatedCells)
    crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);

    // 2026-08-04 D1：主物理循环按活动区域（已发现世界矩形）裁剪。
    // 原版 L144410-144430：遍历 activeRegions，每个区域内依次执行
    // 温度任务 → CopyFrom → 压力 → 液体 → 太空真空 → PostProcess。
    // 无活动区域时回退整张内部网格（测试环境无区域数据；原版此时区域循环不执行，
    // 但生产环境始终 ≥1 个已发现世界，回退不会改变生产行为）。
    let region_bounds: Vec<crate::d1_activity::RegionBounds> = {
        let regions = sim_data.active_regions.as_slice();
        if regions.is_empty() {
            vec![crate::d1_activity::full_grid_bounds(sim_data)]
        } else {
            regions
                .iter()
                .map(|r| crate::d1_activity::compute_region_bounds(sim_data, r))
                .collect()
        }
    };
    for bounds in region_bounds {
        if bounds.is_empty() {
            continue;
        }
        // 温度任务（原版 L144430-144548：读 cells 稳定缓冲、写 updatedCells 增量；
        // 含背墙换热、右/下邻换热、逐格 DoStateTransition）
        crate::c2_physics::temperature::run_temperature_task(sim_data, bounds);
        // 原版 L144548：温度后 CopyFrom，供后续压力/液体阶段读取新温度
        crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);

        // 压力循环（原版顺序：温度任务 → CopyFrom → **压力循环** L42030-42320 →
        // CopyFrom → 液体任务 L42408+ → PostProcessCell；2026-08-03 对照源码修正，
        // 置于 update_liquids 之前——此前液体→压力顺序与原版相反，同帧内先后不同）
        run_pressure_task(sim_data, bounds);

        // C2 阶段实现：液体流动（原版 L42408+）
        crate::c2_physics::liquid_flow::update_liquid_loop(sim_data, bounds);
        // 太空真空特判（原版 L145466-145520：worldZones==-1 且背墙真空 → 液体/气体
        // 持续删除至空 + flow 重置）。2026-08-04 实现：位于 UpdateLiquid 之后、
        // PostProcessCell 之前（原版顺序），此前缺失 → 太空暴露不删除。
        crate::c2_physics::liquid_flow::run_space_vacuum_task(sim_data, bounds);
        // PostProcessCell（原版 L43023：挤压/密度分层/升华）
        crate::c2_physics::liquid_flow::post_process_loop(sim_data, bounds);
    }
    // ElementConsumer 逐帧消费（2026-08-02：水泵/涡轮取水；原版 UpdateData 调组件 Update）
    crate::c_simulation::frame_processor::process_element_consumers(sim_data);

    // BuildingHeatExchange::Update（原版 L145434 组件阶段：建筑↔格子换热，dt=0.2）
    crate::c_simulation::building_temperature::update_building_heat_exchange(sim_data);
    // ElementChunk::Update（原版 L145434 同一组件阶段：物质碎片换热）
    crate::c_simulation::element_chunk::update_element_chunks(sim_data, 0.2, sim_data.width);
    // ElementEmitter::Update（原版 L145434 同一组件阶段：发射器逐帧发射）
    crate::c_simulation::element_emitter::update_element_emitters(sim_data, 0.2, sim_data.width);
    // BuildingToBuildingHeatExchange::Update（原版 L145434 同一组件阶段：导热板建筑↔建筑换热）
    crate::c_simulation::building_to_building::update_building_to_building(sim_data, 0.2);
}

/// UpdateData 压力循环（对照源码 11_msvcrt_ignored.c L42030-42320 的 UpdatePressure 调用）。
///
/// 遍历内部格，对液体格向 4 方向调 update_pressure，用返回值累加 flow 分量。
/// 原版语义（L42121-42126 水平 / L42157-42160 垂直）：
///   flow[本格].y += dir×f39、flow[邻格].x −= dir×f39（水平，dir=±1）
///   flow[本格].z += f39、flow[邻格].w −= f39（垂直）
///
/// 提取为独立函数以便回归测试直接隔离门控（update_liquid 的水平扩散会干扰同场景断言）。
pub(crate) fn run_pressure_task(
    sim_data: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    let sim_w = sim_data.width as usize;
    let sim_h = sim_data.height as usize;
    if sim_w < 3 || sim_h < 3 || sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let flow_ptr = sim_data.flow.ptr;
    if flow_ptr.is_null() {
        return; // flow 未分配（测试/异常场景）：无 flow 分量可累加
    }
    let total = sim_w * sim_h;
    let flow = unsafe { std::slice::from_raw_parts_mut(flow_ptr, total) };
    for row in bounds.min_y..=bounds.max_y {
        for col in bounds.min_x..=bounds.max_x {
            if row >= sim_h - 1 || col >= sim_w - 1 {
                continue;
            }
            let cell = row * sim_w + col;
            // 仅液体格（state & 3 == 2 且 mass > 0）——与 update_liquids 同判定
            let is_liquid = {
                let updated = unsafe { &*sim_data.updated_cells.ptr };
                crate::b_elements::elements_table::get_element_liquid_data(updated.element_idx.get(cell))
                    .map(|d| (d.state & 3) == 2)
                    .unwrap_or(false)
                    && updated.mass.get(cell) > 0.0
            };
            if !is_liquid {
                continue;
            }
            // 邻格状态（从 cells 读——update_pressure 读取的缓冲区）：
            // 原版门控（L42064-42076 水平 / L42126+ 垂直）：邻格须满足
            //   ① pressure state & 3 <= 1（真空/气体；液体/固体邻格不参与压差），
            //   ② properties & 1 == 0（可渗透），
            //   ③ updatedCells 元素 == cells 元素（快照一致），
            //   ④ 同元素或异元素但状态不同（液体源格对真空/气体邻格恒成立）。
            // 2026-08-03 修正：此前只跳固体（state==3），会把同元素液体邻格也纳入压差，
            // 与原版不符（原版液体↔液体压差不存在于压力循环，液体质量均衡由液体任务处理）。
            let [below, above, right, left] = [cell + sim_w, cell - sim_w, cell + 1, cell - 1];
            let neighbor_ok = {
                let cells = unsafe { &*sim_data.cells.ptr };
                let updated = unsafe { &*sim_data.updated_cells.ptr };
                [below, above, right, left].map(|n| {
                    let nb_elem = cells.element_idx.get(n);
                    crate::b_elements::elements_table::get_element_pressure_data(nb_elem)
                        .map(|pd| {
                            (pd.state & 3) <= 1
                                && cells.properties.get(n) & 1 == 0
                                && updated.element_idx.get(n) == nb_elem
                        })
                        .unwrap_or(false)
                })
            };
            // 下（+w）、上（-w）垂直：flow[cell].z += f39、flow[邻格].w −= f39
            if neighbor_ok[0] {
                let f39 = crate::c2_physics::liquid_flow::update_pressure(sim_data, cell, below);
                flow[cell].z += f39;
                flow[below].w -= f39;
            }
            if neighbor_ok[1] {
                let f39 = crate::c2_physics::liquid_flow::update_pressure(sim_data, cell, above);
                flow[cell].z += f39;
                flow[above].w -= f39;
            }
            // 右（+1，dir=+1）、左（-1，dir=-1）水平：
            // flow[cell].y += dir×f39、flow[邻格].x −= dir×f39
            if neighbor_ok[2] {
                let f39 = crate::c2_physics::liquid_flow::update_pressure(sim_data, cell, right);
                flow[cell].y += f39;
                flow[right].x -= f39;
            }
            if neighbor_ok[3] {
                let f39 = crate::c2_physics::liquid_flow::update_pressure(sim_data, cell, left);
                flow[cell].y -= f39;
                flow[left].x += f39;
            }
        }
    }
}

/// SimData::UpdateComponentsDataListOnly 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c。
/// 原版更新组件数据列表（ElementConsumer/Emitter 等）。
/// C1 桩：不执行任何操作。
pub fn update_components_data_list_only(_sim_data: &mut SimData) {
    // ElementChunk::UpdateDataListOnly（原版 L140202）：无时间流逝帧刷新输出，
    // 保证 GDU elementChunkInfos 非空（否则 C# OnSimRegistered 读 elementChunks 时 NRE）
    crate::c_simulation::element_chunk::update_element_chunks_data_list_only();
}

/// ConsolidateEvents 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c。
/// 原版合并 sim_events 中的重复事件。
/// C1 桩：不执行任何操作。
pub fn consolidate_events(_sim_data: &mut SimData) {
    // C2 阶段实现
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::{CellSOA, SimData};
    use crate::b_elements::element::{
        Element, ElementLiquidData, ElementPostProcessData, ElementPressureData, ElementStateData,
        ElementTemperatureData, INVALID_ELEMENT_INDEX,
    };
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::LIB_TESTS_LOCK;

    /// 建 3 元素最小表：0=真空(state=0, flow=0)、1=液体(state=2, flow=50, viscosity=50)、2=固体(state=3)。
    /// 调用方需持 LIB_TESTS_LOCK。
    fn init_pressure_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.element_names.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        for (id, state, flow) in [(0i32, 0u8, 0.0f32), (1i32, 2u8, 50.0f32), (2i32, 3u8, 0.0f32)] {
            let mut elem = Element::default();
            elem.id = id;
            elem.state = state;
            elem.flow = flow;
            table.elements.push(elem);
            table.state_data.push(ElementStateData { state });
            // viscosity=50：UpdateLiquid 流率上限读 viscosity（原版 +8），
            // 生产数据 flow=0、viscosity=speed（2026-08-02 根因修复后测试表须给 viscosity）。
            // max_mass=1000：上推分支阈值 max(up×1.01, max_mass) 需要（2026-08-02 追加）。
            table.liquid_data.push(ElementLiquidData {
                state,
                flow,
                viscosity: 50.0,
                max_mass: 1000.0,
                ..Default::default()
            });
            table.pressure_data.push(ElementPressureData { state, flow });
            // post_process 字段默认：sublimate_index=0xffff → 升华路径不触发
            table.post_process_data.push(ElementPostProcessData { state, ..Default::default() });
        }
    }

    /// 同 init_pressure_table，另补安全 temperature_data（elem1 水、elem2 砖，相变惰性）。
    /// 调用方需持 LIB_TESTS_LOCK。
    fn init_temp_task_table() {
        init_pressure_table();
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
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
        table.temperature_data.clear();
        table.temperature_data.push(ElementTemperatureData::default()); // 0
        table.temperature_data.push(water); // 1
        table.temperature_data.push(rock); // 2
    }

    /// 6×6（内部 4×4）SimData，cells/updated_cells/flow/accumulated_flow/backwall/sim_events 已分配。
    fn make_sim_data() -> SimData {
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, true);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF; // 真空元素 0 不是 void → 走元素变化分支
        sd
    }

    /// 填充 cells 与 updated_cells 双缓冲的辅助。
    fn fill_both(sd: &mut SimData, f: impl Fn(&mut CellSOA)) {
        unsafe {
            f(&mut *sd.cells.ptr);
            f(&mut *sd.updated_cells.ptr);
        }
    }

    /// K1 生产路径测试（RED 依据）：液体格对真空邻格产生**水平**压力转移。
    /// 场景（6×6，内部格）：
    /// - cell14 (row2,col2) 液体 mass=100；下方格 cell8 (row1,col2) **固体** → 液体不下落，
    ///   且压力循环跳过固体邻格（K1 门控）；右/左/上方格（15/13/20）真空。
    /// update_data 帧末期望（任务 2 修正：右分支已实现，右扩散叠加到真空右邻）：
    /// - update_liquid 左分支（原版 L849-945）：下方固体→跳过下落，向左扩散
    ///   min(min(100,50),100×0.25)=25 到 cell13（flow[14].x += 25）；
    /// - update_liquid 右分支（任务 2 新增，原版 L946-1042）：左分支递减后剩余 75，
    ///   向右扩散 min(min(75,50),75×0.25)=18.75 到 cell15（flow[14].y += 18.75）；
    /// - update_pressure 从 cells 帧初快照读质量（源格 100），4 方向各 clamp 12.5%，
    ///   跳过上方固体（cell8）→ 15/20/13 各得 12.5（cell13 叠加左扩散得 25+12.5=37.5）；
    /// - cell15 = 右扩散 18.75 + 压力 12.5 = 31.25；
    /// - 源格最终 = 100 − 25（左扩散）− 18.75（右扩散）− 3×12.5（压力）= 18.75。
    #[test]
    fn update_data_spreads_liquid_to_vacuum_neighbor() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pressure_table();
        let mut sd = make_sim_data();
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 1); // 液体
            b.mass.set(14, 100.0);
            b.temperature.set(14, 300.0);
            b.element_idx.set(8, 2); // 下方固体
            b.mass.set(8, 500.0);
            b.temperature.set(8, 300.0);
            // 13/15/20 为真空（质量 0，元素 0）
        });
        update_data(&mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // 水平蔓延：右邻真空格被液体填充（右扩散 18.75 + 压力 12.5 = 31.25）
            assert_eq!(updated.element_idx.get(15), 1, "真空右邻格应变为液体元素");
            assert!(
                (updated.mass.get(15) - 31.25).abs() < 1e-3,
                "cell15 应得右扩散 18.75 + 压力 12.5 = 31.25，got {}",
                updated.mass.get(15)
            );
            assert!(
                (updated.temperature.get(15) - 300.0).abs() < 1e-3,
                "温度应整体转移为 300"
            );
            // 上方真空邻格得 12.5（update_pressure 从 cells 帧初快照读质量）
            assert!((updated.mass.get(20) - 12.5).abs() < 1e-3);
            // 左邻：左扩散 25 + 压力 12.5 = 37.5
            assert!(
                (updated.mass.get(13) - 37.5).abs() < 1e-3,
                "cell13 应得左扩散 25 + 压力 12.5 = 37.5，got {}",
                updated.mass.get(13)
            );
            assert!(
                (updated.mass.get(14) - 18.75).abs() < 1e-3,
                "源格应 100−25（左）−18.75（右）−3×12.5（压力）= 18.75，got {}",
                updated.mass.get(14)
            );
            // 固体邻格（cell8）不受压力转移影响（K1 门控）
            assert_eq!(updated.element_idx.get(8), 2, "固体邻格不应被侵蚀");
            assert!((updated.mass.get(8) - 500.0).abs() < 1e-3, "固体质量不变");
            // flow 分量累加：flow[15].x -= f39；flow[14].x 含左扩散 +25
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 36);
            assert!(
                (flow[15].x - (-12.5)).abs() < 1e-3,
                "flow[15].x 应 -12.5，got {}",
                flow[15].x
            );
            assert!(
                (flow[14].x - 25.0).abs() < 1e-3,
                "flow[14].x 应含左扩散 +25，got {}",
                flow[14].x
            );
            // 真空格被填充 → substance 事件（sim15 → game cell 6）
            let events = &*sd.sim_events.ptr;
            let has_cell6 = events.substance_change_info.as_slice().iter().any(|e| e.cell_idx == 6);
            assert!(has_cell6, "应产生 sim15（game6）的 substance 事件");
        }
    }

    /// K1 门控补充：液体格 4 方向全是固体时，压力循环不应转移任何质量/不产生事件。
    #[test]
    fn update_data_skips_pressure_when_all_neighbors_solid() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pressure_table();
        let mut sd = make_sim_data();
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 1);
            b.mass.set(14, 100.0);
            b.temperature.set(14, 300.0);
            // 4 方向（8 下 / 20 上 / 15 右 / 13 左）全固体
            for c in [8usize, 20, 15, 13] {
                b.element_idx.set(c, 2);
                b.mass.set(c, 500.0);
            }
        });
        update_data(&mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            for c in [8usize, 20, 15, 13] {
                assert_eq!(updated.element_idx.get(c), 2, "固体邻格 {} 不应被侵蚀", c);
                assert!((updated.mass.get(c) - 500.0).abs() < 1e-3);
            }
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 0, "全固体邻格不应产生转移事件");
        }
    }

    /// 原版压力循环门控（L42064-42076）：邻格 pressure state & 3 <= 1（真空/气体）才调
    /// UpdatePressure。同元素液体邻格不参与压差——液体↔液体质量均衡由液体任务处理。
    /// 2026-08-03 修正前只跳固体（state==3），此场景旧实现会转移 min(12.5%×500, 400)=62.5kg。
    #[test]
    fn pressure_task_skips_same_element_liquid_neighbor() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pressure_table();
        let mut sd = make_sim_data();
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 1);
            b.mass.set(14, 500.0);
            b.temperature.set(14, 300.0);
            // 右邻同元素液体 100（质量差 400 → 旧实现会转移 62.5）
            b.element_idx.set(15, 1);
            b.mass.set(15, 100.0);
            b.temperature.set(15, 300.0);
            // cell15 自身也是液体格，会向自己的真空邻格转移——把 cell14/cell15 的
            // 全部 6 个外邻（8/20/13 + 9/16/21）设为固体，隔离"同元素液体邻格"关系。
            for c in [8usize, 20, 13, 9, 16, 21] {
                b.element_idx.set(c, 2);
                b.mass.set(c, 500.0);
                b.temperature.set(c, 300.0);
            }
        });
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_pressure_task(&mut sd, bounds);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(14) - 500.0).abs() < 1e-3,
                "同元素液体邻格不应触发压差：cell14 应保持 500，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.mass.get(15) - 100.0).abs() < 1e-3,
                "同元素液体邻格不应触发压差：cell15 应保持 100，got {}",
                updated.mass.get(15)
            );
            assert_eq!(updated.element_idx.get(15), 1, "邻格仍为液体");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 0, "不应产生转移事件");
        }
    }

    /// 门控正例：真空邻格（state&3==0）仍参与压差转移（原版 L42064-42076 通过路径）。
    /// cell14 液体 100 → 右邻真空格得 min(12.5%×100, 100)=12.5，flow 分量累加。
    #[test]
    fn pressure_task_transfers_to_vacuum_neighbor() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pressure_table();
        let mut sd = make_sim_data();
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 1);
            b.mass.set(14, 100.0);
            b.temperature.set(14, 300.0);
            // 15 真空（elem0, mass 0）；其余 3 方向固体
            for c in [8usize, 20, 13] {
                b.element_idx.set(c, 2);
                b.mass.set(c, 500.0);
                b.temperature.set(c, 300.0);
            }
        });
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_pressure_task(&mut sd, bounds);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(15), 1, "真空邻格应变为液体元素");
            assert!(
                (updated.mass.get(15) - 12.5).abs() < 1e-3,
                "cell15 应得 12.5，got {}",
                updated.mass.get(15)
            );
            assert!(
                (updated.mass.get(14) - 87.5).abs() < 1e-3,
                "cell14 应减至 87.5，got {}",
                updated.mass.get(14)
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 36);
            assert!(
                (flow[14].y - 12.5).abs() < 1e-3,
                "flow[14].y 应 +12.5，got {}",
                flow[14].y
            );
            assert!(
                (flow[15].x + 12.5).abs() < 1e-3,
                "flow[15].x 应 -12.5，got {}",
                flow[15].x
            );
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1, "真空格填充应产生 1 条事件");
        }
    }

    #[test]
    fn update_data_syncs_cells_before_temperature_each_substep() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_temp_task_table();
        let mut sd = make_sim_data();
        // cell14 水 300、cell15 砖 400（相邻换热）；两者四周全是同温固体 → 液体流/压力惰性，
        // 隔离验证"温度任务基于上一子步结果"（原版 L144415 每子步开头 CopyFrom）。
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 1);
            b.mass.set(14, 100.0);
            b.temperature.set(14, 300.0);
            b.insulation.set(14, 255);
            b.element_idx.set(15, 2);
            b.mass.set(15, 200.0);
            b.temperature.set(15, 400.0);
            b.insulation.set(15, 255);
            for (cell, temp) in [(8usize, 300.0f32), (20usize, 300.0f32), (13usize, 300.0f32)] {
                b.element_idx.set(cell, 2);
                b.mass.set(cell, 1000.0);
                b.temperature.set(cell, temp);
                b.insulation.set(cell, 255);
            }
            for (cell, temp) in [(9usize, 400.0f32), (21usize, 400.0f32), (16usize, 400.0f32)] {
                b.element_idx.set(cell, 2);
                b.mass.set(cell, 1000.0);
                b.temperature.set(cell, temp);
                b.insulation.set(cell, 255);
            }
        });
        update_data(&mut sd);
        let after_1 = unsafe { (*sd.updated_cells.ptr).temperature.get(14) };
        update_data(&mut sd); // 第二个子步
        let after_2 = unsafe { (*sd.updated_cells.ptr).temperature.get(14) };
        // opening CopyFrom 使第二子步基于第一子步结果：温度继续收敛（而非重复相同 delta 过冲）
        assert!(
            after_2 > after_1,
            "第二子步应继续升温收敛：{after_1} → {after_2}"
        );
        assert!(after_2 < 400.0, "不得越过热侧 400：{after_2}");
    }
}
