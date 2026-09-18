//! C2 阶段物理模拟桩实现。
//!
//! C1 阶段：所有函数为空桩，不执行任何物理计算。
//! C2 阶段：逐步替换为真实物理计算（温度/液体/气体/辐射/疾病）。

#![allow(dead_code)]

use crate::a_framework::sim_data::SimData;

pub mod liquid_flow;
pub mod property_texture;
pub mod temperature;
pub mod gas_flow;
pub mod radiation;
pub mod disease;
pub mod region_events;
pub mod region_rng;

/// 阶段 A：区域网格物理（d3 三段式；parallel=true 时温度内联 + 区域局部 CopyFrom）。
/// 串行路径（parallel=false）行为与原版/现状逐字节一致。
pub(crate) fn run_region_physics_a(
    sim_data: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
    cosmic_intensity: f32,
    parallel: bool,
) {
    if bounds.is_empty() {
        return;
    }
    // 自然固体强度推导（原版在固体生成/加载时赋予质量型抗压字节；每帧按活动区域兜底，
    // 覆盖岩浆凝固/沙盒生成等运行时新出现的固体，随后压力分支据此计算抗压阈值）。
    crate::c_simulation::sim_data_ops::derive_natural_solid_strength(sim_data, bounds);
    // 温度段（含并行/串行两分支与队列调度开销——都归属温度阶段成本）
    crate::sim_stage_timed!(crate::a_framework::stage_profiler::STAGE_TEMPERATURE, {
        if parallel {
            // d3 并行路径：温度在区域任务内联整区（不经全局共享队列，避免嵌套并行争抢）。
            crate::c2_physics::temperature::run_temperature_task(sim_data, bounds);
        } else {
            // 串行路径：按 worker 数切行带提交到 ParallelTaskQueue（原版 L144430-144527），
            // 队列未初始化（测试环境）时内联整区执行。
            // 2026-08-10 修复（D-7）：守卫存变量持锁到使用完毕——此前 `.lock().0` 的守卫
            // 是临时量，语句结束即释放，后续 submit/wait_all 无锁使用裸指针（UAF 隐患）。
            let queue_guard = crate::globals::G_PARALLEL_TASK_QUEUE.lock();
            let queue_ptr = queue_guard.0;
            if !queue_ptr.is_null() {
                let queue = unsafe { &*queue_ptr };
                let n_workers = queue.worker_count().max(1);
                let total_rows = bounds.max_y - bounds.min_y;
                let base = total_rows / n_workers;
                let extra = total_rows % n_workers;
                let mut row = bounds.min_y;
                let mut submitted = 0usize;
                for i in 0..n_workers {
                    let band_rows = base + if i < extra { 1 } else { 0 };
                    if band_rows == 0 {
                        break;
                    }
                    let row_end = row + band_rows;
                    let ptr = crate::globals::SendSyncPtr(sim_data as *mut SimData);
                    let b = bounds;
                    queue.submit(Box::new(move || {
                        crate::c2_physics::temperature::run_temperature_band_task(ptr, b, row, row_end);
                    }));
                    row = row_end;
                    submitted += 1;
                }
                if submitted > 0 {
                    queue.wait_all();
                }
            } else {
                crate::c2_physics::temperature::run_temperature_task(sim_data, bounds);
            }
        }
    });
    crate::sim_stage_timed!(
        crate::a_framework::stage_profiler::STAGE_COPY,
        if parallel {
            crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells_region(sim_data, &bounds);
        } else {
            crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);
        }
    );
    crate::sim_stage_timed!(crate::a_framework::stage_profiler::STAGE_GAS_PRESSURE, {
        crate::c2_physics::gas_flow::run_gas_pressure_task(sim_data, bounds);
        crate::c2_physics::gas_flow::run_gas_displacement_task(sim_data, bounds);
    });
    crate::sim_stage_timed!(
        crate::a_framework::stage_profiler::STAGE_COPY,
        if parallel {
            crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells_region(sim_data, &bounds);
        } else {
            crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);
        }
    );
    crate::sim_stage_timed!(crate::a_framework::stage_profiler::STAGE_LIQUID, {
        crate::c2_physics::liquid_flow::update_liquid_loop(sim_data, bounds);
        crate::c2_physics::liquid_flow::run_liquid_displacement_task(sim_data, bounds);
    });
    crate::sim_stage_timed!(
        crate::a_framework::stage_profiler::STAGE_COPY,
        if parallel {
            crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells_region(sim_data, &bounds);
        } else {
            crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);
        }
    );
    crate::sim_stage_timed!(crate::a_framework::stage_profiler::STAGE_DISEASE_DIFFUSE, {
        crate::c2_physics::disease::run_disease_diffusion_task(sim_data, bounds);
    });
    crate::sim_stage_timed!(crate::a_framework::stage_profiler::STAGE_RADIATION, {
        crate::c2_physics::radiation::run_radiation_task(sim_data, bounds, cosmic_intensity);
    });
}

/// 阶段 B：区域组件（★4 注册顺序；串行。d3 并行路径也按区域序串行执行，
/// 事件直接写全局，不经过区域 sink）。
pub(crate) fn run_region_emitters(
    sim_data: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    if bounds.is_empty() {
        return;
    }
    crate::sim_stage_timed!(crate::a_framework::stage_profiler::STAGE_EMITTERS, {
        crate::c_simulation::frame_processor::process_element_consumers(sim_data, bounds);
        crate::c_simulation::element_emitter::update_element_emitters(
            sim_data,
            0.2,
            sim_data.width,
            bounds,
        );
        crate::c_simulation::radiation_emitter::update_radiation_emitters_global(sim_data, 0.2, bounds);
        crate::c_simulation::element_chunk::update_element_chunks(
            sim_data,
            0.2,
            sim_data.width,
            bounds,
        );
        crate::c_simulation::building_temperature::update_building_heat_exchange(sim_data, bounds);
        crate::c_simulation::building_to_building::update_building_to_building(sim_data, 0.2, bounds);
        crate::c_simulation::disease_component::update_disease_emitters(sim_data, 0.2, bounds);
    });
}

/// 阶段 C：区域收尾（太空真空 → PostProcess → 病菌生长）。
pub(crate) fn run_region_physics_c(
    sim_data: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
    pp_bounds: crate::d1_activity::RegionBounds,
) {
    if bounds.is_empty() {
        return;
    }
    // 阶段 C 开头的自然固体强度推导：A 阶段温度/相变与 B 阶段发射器新生成的固体，
    // 在进入 PostProcess（含 DoPressureBreak 超压检查）前补上强度，做到同 tick 生效。
    // 用 bounds（区域边界）而非 pp_bounds：相邻区域的 post-process 边界会重叠 1 格，
    // 并行下并发写同一格不安全；区域边界互不重叠。
    crate::c_simulation::sim_data_ops::derive_natural_solid_strength(sim_data, bounds);
    // 2026-08-10 修复（顶部格层气体残留 bug）：原版 L145445-145460 太空删除的
    // 遍历边界用 **PostProcess 边界**（32 格块对齐 + max+1），即世界上方/右方
    // 1 格（太空格）也在删除范围内；此前误用普通区域边界 → 顶部格层气体扩散到
    // 世界上方太空格后不被删除，删除主格后残留随机闪现、缓慢消失。
    let _t0 = std::time::Instant::now();
    crate::sim_stage_timed!(crate::a_framework::stage_profiler::STAGE_POSTPROCESS, {
        crate::c2_physics::liquid_flow::run_space_vacuum_task(sim_data, pp_bounds);
        crate::c2_physics::liquid_flow::post_process_loop(sim_data, pp_bounds);
        crate::c2_physics::disease::run_disease_growth_task(sim_data, bounds);
    });
}

/// SimBase::UpdateData —— 主物理更新入口。
///
/// 对照源码 SimDLL_Source.c L144400+（UpdateData）与 11_msvcrt_ignored.c L41811。
/// 每子步：翻转 iterateDirection → 全网格 CopyFrom → 按活动区域执行
/// 温度/气体/液体/病菌/辐射 → 组件（阶段 B）→ 太空真空 → PostProcess（阶段 C）。
/// （C1 阶段曾为空桩，2026-08 起已完整实现。）
pub fn update_data(sim_data: &mut SimData) {
    // 原版 UpdateData L144415：iterateDirection 每子步翻转（气体/液体压力扫描方向
    // 交替 → 圆形振荡扩散）。
    sim_data.iterate_direction = -sim_data.iterate_direction;

    // 原版 UpdateData L144415：每子步开头 CopyFrom(cells, updatedCells)
    crate::sim_stage_timed!(crate::a_framework::stage_profiler::STAGE_COPY, {
        crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);
    });

    // 2026-08-04 D1：主物理循环按活动区域（已发现世界矩形）裁剪。
    // 原版 L144410-144430：遍历 activeRegions，每个区域内依次执行
    // 温度任务 → CopyFrom → 压力 → 液体 → 太空真空 → PostProcess。
    // 无活动区域时回退整张内部网格（测试环境无区域数据；原版此时区域循环不执行，
    // 但生产环境始终 ≥1 个已发现世界，回退不会改变生产行为）。
    let region_bounds: Vec<(
        crate::d1_activity::RegionBounds,
        crate::d1_activity::RegionBounds,
        f32,
    )> = {
        let regions = sim_data.active_regions.as_slice();
        if regions.is_empty() {
            vec![(
                crate::d1_activity::full_grid_bounds(sim_data),
                crate::d1_activity::full_grid_bounds(sim_data),
                0.0,
            )]
        } else {
            regions
                .iter()
                .map(|r| {
                    (
                        crate::d1_activity::compute_region_bounds(sim_data, r),
                        crate::d1_activity::compute_post_process_bounds(sim_data, r),
                        r.current_cosmic_radiation_intensity,
                    )
                })
                .collect()
        }
    };
    let bounds_list: Vec<_> = region_bounds.iter().map(|x| x.0).collect();
    let pp_list: Vec<_> = region_bounds.iter().map(|x| x.1).collect();
    let cosmic_list: Vec<_> = region_bounds.iter().map(|x| x.2).collect();
    let region_count = bounds_list.len();
    // d3 并行分支（feature "d3-rayon" 门控）：多区域 → 三段式并行；否则串行。
    #[cfg(feature = "d3-rayon")]
    {
        let _perf_t0 = std::time::Instant::now();
        use crate::d3_rayon::scheduler::SchedulerMode;
        let mut perf_threads = 1usize;
        match crate::d3_rayon::scheduler::current_mode() {
            SchedulerMode::D2 => {
                perf_threads = crate::d3_rayon::pool().current_num_threads();
                crate::d3_rayon::pipeline::run_parallel_pipeline(
                    sim_data,
                    bounds_list,
                    pp_list,
                    cosmic_list,
                );
            }
            SchedulerMode::D1 => {
                if region_count <= 1 {
                    // 单区域/单星永远走串行（与原版一致）。
                    run_serial_regions(sim_data, bounds_list, pp_list, cosmic_list);
                } else {
                    let topo = crate::d3_rayon::affinity::CpuTopology::cached();
                    let reserve = crate::d3_rayon::reserve_cores();
                    if let Some(groups) =
                        crate::d3_rayon::d1_pipeline::build_d1_groups(&bounds_list, topo, reserve)
                    {
                        perf_threads = groups.len();
                        crate::d3_rayon::d1_pipeline::run_d1_pipeline(
                            sim_data,
                            &bounds_list,
                            &pp_list,
                            &cosmic_list,
                            &groups,
                        );
                    } else {
                        // 分组失败（无 HT/拓扑异常）→ 回退 D2。
                        crate::d3_rayon::pipeline::run_parallel_pipeline(
                            sim_data,
                            bounds_list,
                            pp_list,
                            cosmic_list,
                        );
                    }
                }
            }
            SchedulerMode::Auto => {
                if crate::d3_rayon::should_parallelize(region_count) {
                    perf_threads = crate::d3_rayon::pool().current_num_threads();
                    crate::d3_rayon::pipeline::run_parallel_pipeline(
                        sim_data,
                        bounds_list,
                        pp_list,
                        cosmic_list,
                    );
                } else {
                    run_serial_regions(sim_data, bounds_list, pp_list, cosmic_list);
                }
            }
        }
        let perf_mode = match crate::d3_rayon::scheduler::current_mode() {
            SchedulerMode::Auto => 0,
            SchedulerMode::D2 => 1,
            SchedulerMode::D1 => 2,
        };
        crate::d3_rayon::perf_probe::frame_done(
            region_count,
            perf_mode,
            perf_threads,
            _perf_t0.elapsed().as_micros() as u64,
        );
    }
    #[cfg(not(feature = "d3-rayon"))]
    {
        run_serial_regions(sim_data, bounds_list, pp_list, cosmic_list);
    }
    // 原版 UpdateData L43033：`param_1->tickCount = param_1->tickCount + 1`（每子步一次）。
    // DisplaceGas / DisplaceLiquid 的候选方向按 tickCount 旋转（"随机单方向"确定性实现）；
    // 2026-08-12 修正：此前从未递增 → 方向恒为 tick 0（右格固定真空/左格固定累积），
    // 与透气砖高压存储场景的原版行为（方向随帧轮转）不符。
    sim_data.tick_count = sim_data.tick_count.wrapping_add(1);
}

/// 串行区域循环（d3 关闭或单区域时执行；行为与原版逐帧一致）。
fn run_serial_regions(
    sim_data: &mut SimData,
    bounds_list: Vec<crate::d1_activity::RegionBounds>,
    pp_list: Vec<crate::d1_activity::RegionBounds>,
    cosmic_list: Vec<f32>,
) {
    for i in 0..bounds_list.len() {
        let bounds = bounds_list[i];
        if bounds.is_empty() {
            continue;
        }
        run_region_physics_a(sim_data, bounds, cosmic_list[i], false);
        run_region_emitters(sim_data, bounds);
        run_region_physics_c(sim_data, bounds, pp_list[i]);
    }
}

/// SimData::UpdateComponentsDataListOnly 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c。
/// 原版更新组件数据列表（ElementConsumer/Emitter 等）。
/// SimData::UpdateComponentsDataListOnly（原版 L124218-124238）。
///
/// 无时间流逝帧刷新组件输出。Rust 架构下各输出通道在 CopySimDataToGame
/// 每帧重建/交换（行为等价，详见漏洞.txt #6），此处仅需补 ElementChunk 输出。
pub fn update_components_data_list_only(_sim_data: &mut SimData) {
    // ElementChunk::UpdateDataListOnly（原版 L140202）：无时间流逝帧刷新输出，
    // 保证 GDU elementChunkInfos 非空（否则 C# OnSimRegistered 读 elementChunks 时 NRE）
    crate::c_simulation::element_chunk::update_element_chunks_data_list_only();
}

/// ConsolidateEvents（原版 SimDLL_Source.c L143311 SimBase::ConsolidateEvents）。
///
/// ⚠️ 描述更正（2026-08-10）：原版**不是事件去重**，而是把每个 worker 各自的
/// SimEvents 缓冲逐字段 append 合并进 SimData->simEvents（多 worker 事件聚合）。
/// Rust 在 region_events.rs 把合并内联到产生点（无 sink 时直接写全局），
/// 1 worker 下结果等价；D2 worker >1 时需要 per-worker 隔离（见 D-11）。
/// 空函数体 = 单 worker 模型下无功能缺失。
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
    /// - 原版压力循环（L42030-42320）**只处理气体/真空源格**（源格 state&3<2），
    ///   液体源不进压力循环 → 液体↔真空只由 update_liquid 水平扩散处理：
    ///   cell15 = 右扩散 18.75、cell13 = 左扩散 25、cell20（上方真空）不转移
    ///   （上推分支阈值 max_mass，mass<1000 → 0）；
    /// - 源格最终 = 100 − 25（左扩散）− 18.75（右扩散）= 56.25。
    /// - 2026-08-06 修正：此前有非原版 run_pressure_task（液体源→真空 12.5% 压力
    ///   转移），会提前把真空填成液体，破坏 update_liquid 分支 3 角落判定（已移除）。
    #[test]
    fn update_data_spreads_liquid_to_vacuum_neighbor() {
        let _lock = LIB_TESTS_LOCK.lock();
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
            // 水平蔓延：右邻真空格被液体填充（右扩散 18.75，原版无液体源压力）
            assert_eq!(updated.element_idx.get(15), 1, "真空右邻格应变为液体元素");
            assert!(
                (updated.mass.get(15) - 18.75).abs() < 1e-3,
                "cell15 应得右扩散 18.75，got {}",
                updated.mass.get(15)
            );
            assert!(
                (updated.temperature.get(15) - 300.0).abs() < 1e-3,
                "温度应整体转移为 300"
            );
            // 上方真空邻格不转移（原版上推阈值 max_mass=1000，mass 100 < 1000 → 0）
            assert_eq!(updated.element_idx.get(20), 0, "上方真空格不应被填充");
            assert!((updated.mass.get(20) - 0.0).abs() < 1e-6);
            // 左邻：左扩散 25（无压力叠加）
            assert!(
                (updated.mass.get(13) - 25.0).abs() < 1e-3,
                "cell13 应得左扩散 25，got {}",
                updated.mass.get(13)
            );
            assert!(
                (updated.mass.get(14) - 56.25).abs() < 1e-3,
                "源格应 100−25（左）−18.75（右）= 56.25，got {}",
                updated.mass.get(14)
            );
            // 固体邻格（cell8）不受影响
            assert_eq!(updated.element_idx.get(8), 2, "固体邻格不应被侵蚀");
            assert!((updated.mass.get(8) - 500.0).abs() < 1e-3, "固体质量不变");
            // flow 分量累加：仅水平扩散——flow[14].x += 25（左）、flow[14].y += 18.75（右）
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 36);
            assert!((flow[15].x - 0.0).abs() < 1e-6, "flow[15].x 应 0（无压力转移）");
            assert!(
                (flow[14].x - 25.0).abs() < 1e-3,
                "flow[14].x 应含左扩散 +25，got {}",
                flow[14].x
            );
            assert!(
                (flow[14].y - 18.75).abs() < 1e-3,
                "flow[14].y 应含右扩散 +18.75，got {}",
                flow[14].y
            );
            // 真空格被填充 → substance 事件（sim15 → game cell 6）
            let events = &*sd.sim_events.ptr;
            let has_cell6 = events.substance_change_info.as_slice().iter().any(|e| e.cell_idx == 6);
            assert!(has_cell6, "应产生 sim15（game6）的 substance 事件");
        }
    }

    /// 液体格 4 方向全是固体时，update_data 不应转移任何质量/不产生事件
    /// （原版无液体源压力循环；update_liquid 对全固体邻格惰性）。
    #[test]
    fn update_data_liquid_all_solid_neighbors_inert() {
        let _lock = LIB_TESTS_LOCK.lock();
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

    #[test]
    fn update_data_syncs_cells_before_temperature_each_substep() {
        let _lock = LIB_TESTS_LOCK.lock();
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

    /// 回归（原版 UpdateData L43033）：每子步 tick_count + 1（DisplaceGas/DisplaceLiquid
    /// 候选方向的旋转驱动）。2026-08-12 修正前从未递增 → 方向恒为 tick 0（透气砖高压
    /// 存储场景"真空固定右格/左格固定累积"，与原版随帧轮转不符）。
    #[test]
    fn update_data_increments_tick_count_once() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_pressure_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.tick_count = 0;
        update_data(&mut sd);
        assert_eq!(sd.tick_count, 1, "第一个子步 tick_count 应为 1");
        update_data(&mut sd);
        assert_eq!(sd.tick_count, 2, "第二个子步 tick_count 应为 2");
        crate::b_elements::elements_table::DestroyElementsTable();
    }
}
