//! 方向 1（1 星 1 线程）：D1 执行器。
//!
//! 每帧按组 spawn `std::thread`（组数 = 物理核组 + 超线程组，≤ 逻辑核数），
//! 线程闭包内 `SetThreadAffinityMask` 绑定亲和性后执行阶段 A/C 的组内区域物理。
//! 阶段 B（组件）保持串行（与 D2 一致）。事件经 region sink 路由后合并。
//! 说明：线程采用每帧创建而非持久线程——sim 帧 200ms，创建开销（~1ms）可忽略，
//! 换取实现简单可靠；std::thread + 亲和性 + 组并行的目标不变。

use crate::a_framework::sim_data::SimData;
use crate::c2_physics::region_events::{with_region_sink, RegionEvents};
use crate::c2_physics::region_rng::{with_region_rng, RegionRng};
use crate::d1_activity::RegionBounds;
use crate::d3_rayon::affinity::{assign_groups, AffinityGroup, CpuTopology};
use crate::globals::SendSyncPtr;

/// 太空员舱集群识别：HabitatModuleMedium/Small 世界 32×32 = 1024 tiles。
/// region bounds 为区域矩形（compute_region_bounds 后与区域格子数一致）。
pub fn is_module_cluster(bounds: &RegionBounds) -> bool {
    let w = bounds.max_x.saturating_sub(bounds.min_x);
    let h = bounds.max_y.saturating_sub(bounds.min_y);
    w * h == 32 * 32
}

/// 构建 D1 分组：普通世界走 `assign_groups`（物理核独占 + 超线程两两合并），
/// 太空员舱集群单独一组（固定超线程）。
pub fn build_d1_groups(
    bounds: &[RegionBounds],
    topo: &CpuTopology,
) -> Option<Vec<AffinityGroup>> {
    if !topo.has_hyper_threading() {
        return None;
    }
    let area = |b: &RegionBounds| (b.max_x - b.min_x) as u64 * (b.max_y - b.min_y) as u64;
    let mut world_idx: Vec<usize> = Vec::new();
    let mut cluster_idx: Vec<usize> = Vec::new();
    for (i, b) in bounds.iter().enumerate() {
        if is_module_cluster(b) {
            cluster_idx.push(i);
        } else {
            world_idx.push(i);
        }
    }
    let world_tiles: Vec<u64> = world_idx.iter().map(|&i| area(&bounds[i])).collect();
    let mut groups = assign_groups(&world_tiles, topo)?;
    // 把 assign_groups 的组内索引（world_tiles 内）映射回真实 region 索引。
    for g in groups.iter_mut() {
        g.region_indices = g.region_indices.iter().map(|&wi| world_idx[wi]).collect();
    }
    // 集群组：单独超线程组（优先复用最小物理核组的空闲逻辑核，即 ht 序号 0）。
    if !cluster_idx.is_empty() {
        let mask = ht_idle_mask(topo, 0)?;
        groups.push(AffinityGroup {
            region_indices: cluster_idx,
            affinity_mask: mask,
        });
    }
    Some(groups)
}

/// 第 `ht_index` 个超线程组的亲和掩码：从最小物理核组（索引 p−1）的空闲逻辑核
/// 开始轮转（与 `assign_groups` 内部逻辑一致）。
fn ht_idle_mask(topo: &CpuTopology, ht_index: usize) -> Option<u64> {
    if !topo.has_hyper_threading() {
        return None;
    }
    let p = topo.physical_cores.saturating_sub(1).max(1);
    let core_idx = (p - 1 - (ht_index % p)) % p;
    let start: u64 = topo
        .logical_per_core
        .iter()
        .take(core_idx)
        .map(|&n| n as u64)
        .sum();
    Some(1u64 << (start + 1))
}

/// 当前线程设置亲和掩码（失败静默，回退 OS 调度）。
fn set_thread_affinity(mask: u64) {
    unsafe {
        let _ = windows::Win32::System::Threading::SetThreadAffinityMask(
            windows::Win32::System::Threading::GetCurrentThread(),
            mask as usize,
        );
    }
}

/// D1 管线：阶段 A（组并行）→ 合并 A 事件 → 阶段 B（组件串行）→
/// 阶段 C（组并行）→ 合并 C 事件。与 D2 `run_parallel_pipeline` 语义一致，
/// 仅线程组织不同（每组一个 std::thread + 亲和性）。
pub fn run_d1_pipeline(
    sim: &mut SimData,
    region_bounds: &[RegionBounds],
    region_pp_bounds: &[RegionBounds],
    region_cosmic: &[f32],
    groups: &[AffinityGroup],
) {
    if region_bounds.is_empty() || groups.is_empty() {
        return;
    }
    // T4：区域 RNG 对齐（同 D2）。
    crate::c2_physics::region_rng::realign_region_rngs(sim, sim.active_regions.as_slice());
    let rng_ptrs: Vec<SendSyncPtr<RegionRng>> = crate::c2_physics::region_rng::region_rng_ptrs()
        .into_iter()
        .map(SendSyncPtr)
        .collect();
    let mut region_events: Vec<RegionEvents> =
        (0..region_bounds.len()).map(|_| Default::default()).collect();
    let events_ptrs: Vec<SendSyncPtr<RegionEvents>> = (0..region_bounds.len())
        .map(|i| SendSyncPtr(&mut region_events[i] as *mut RegionEvents))
        .collect();
    let sim_ptr = SendSyncPtr(sim as *mut SimData);

    // —— 阶段 A：组并行（scoped 线程：可借用局部 region 数据，语义等价）——
    std::thread::scope(|s| {
        for (gi, g) in groups.iter().enumerate() {
            let sim_ptr = sim_ptr.clone();
            let rng_ptrs = rng_ptrs.clone();
            let events_ptrs = events_ptrs.clone();
            let g = g;
            s.spawn(move || {
                set_thread_affinity(g.affinity_mask);
                let sim_raw = sim_ptr.get();
                for &i in &g.region_indices {
                    let rng_raw = rng_ptrs[i].get();
                    let events_raw = events_ptrs[i].get();
                    with_region_rng(rng_raw, move || {
                        with_region_sink(events_raw, move || unsafe {
                            crate::c2_physics::run_region_physics_a(
                                &mut *sim_raw,
                                region_bounds[i],
                                region_cosmic[i],
                                true,
                            );
                        });
                    });
                }
            });
        }
    });
    for ev in region_events.iter_mut() {
        ev.merge_into(sim);
    }

    // —— 阶段 B：组件串行（同 D2）——
    for i in 0..region_bounds.len() {
        crate::c2_physics::run_region_emitters(sim, region_bounds[i]);
    }

    // —— 阶段 C：组并行 ——
    std::thread::scope(|s| {
        for g in groups.iter() {
            let sim_ptr = sim_ptr.clone();
            let rng_ptrs = rng_ptrs.clone();
            let events_ptrs = events_ptrs.clone();
            s.spawn(move || {
                set_thread_affinity(g.affinity_mask);
                let sim_raw = sim_ptr.get();
                for &i in &g.region_indices {
                    let rng_raw = rng_ptrs[i].get();
                    let events_raw = events_ptrs[i].get();
                    with_region_rng(rng_raw, move || {
                        with_region_sink(events_raw, move || unsafe {
                            crate::c2_physics::run_region_physics_c(
                                &mut *sim_raw,
                                region_bounds[i],
                                region_pp_bounds[i],
                            );
                        });
                    });
                }
            });
        }
    });
    for ev in region_events.iter_mut() {
        ev.merge_into(sim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;
    use crate::b_elements::element::{
        Element, ElementLiquidData, ElementPostProcessData, ElementPressureData, ElementStateData,
        ElementTemperatureData,
    };
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::LIB_TESTS_LOCK;

    fn init_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.element_names.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        table.radiation_data.clear();
        for (id, state) in [(0i32, 0u8), (1i32, 2u8), (2i32, 1u8), (3i32, 3u8)] {
            let mut elem = Element::default();
            elem.id = id;
            elem.state = state;
            table.elements.push(elem);
            table.state_data.push(ElementStateData { state });
            table.liquid_data.push(ElementLiquidData {
                state,
                flow: 50.0,
                viscosity: 50.0,
                max_mass: 1000.0,
                min_horizontal_flow: 0.0,
                min_vertical_flow: 0.0,
            });
            table.pressure_data.push(ElementPressureData { state, flow: 0.0 });
            table
                .post_process_data
                .push(ElementPostProcessData { state, ..Default::default() });
            table.temperature_data.push(ElementTemperatureData {
                state,
                low_temp: 0.0,
                high_temp: 10000.0,
                ..Default::default()
            });
            table.radiation_data.push(crate::b_elements::element::ElementRadiationData {
                factor: if state == 3 { 0.5 } else { 0.0 },
                rads_per_1000: 0.0,
            });
        }
    }

    /// 构造 2 个活动区域、各放 100kg 液体的最小 sim（含元素表初始化）。
    fn make_two_region_sim() -> SimData {
        init_table();
        let mut sd = SimData::new_for_allocate(14, 12, 7, true, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            let cells = &mut *sd.cells.ptr;
            let updated = &mut *sd.updated_cells.ptr;
            for buf in [&mut *cells, &mut *updated] {
                for i in 0..(14 * 12) {
                    buf.element_idx.set(i, 0);
                    buf.mass.set(i, 0.0);
                    buf.temperature.set(i, 300.0);
                }
                buf.element_idx.set(1 * 14 + 1, 1);
                buf.mass.set(1 * 14 + 1, 100.0);
                buf.element_idx.set(1 * 14 + 7, 1);
                buf.mass.set(1 * 14 + 7, 100.0);
            }
        }
        sd.active_regions.push(crate::a_framework::sim_data::ActiveRegion {
            min_x: 1,
            min_y: 1,
            max_x: 6,
            max_y: 5,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        });
        sd.active_regions.push(crate::a_framework::sim_data::ActiveRegion {
            min_x: 7,
            min_y: 1,
            max_x: 12,
            max_y: 5,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        });
        sd
    }

    /// D1 管线基本可用性：2 区域各放液体，跑 run_d1_pipeline 不 panic，
    /// 液体总质量守恒。
    #[test]
    fn d1_pipeline_preserves_mass() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        let mut sd = make_two_region_sim();
        let bounds: Vec<RegionBounds> = sd
            .active_regions
            .as_slice()
            .iter()
            .map(|r| crate::d1_activity::compute_region_bounds(&sd, r))
            .collect();
        let pp: Vec<RegionBounds> = sd
            .active_regions
            .as_slice()
            .iter()
            .map(|r| crate::d1_activity::compute_post_process_bounds(&sd, r))
            .collect();
        let cosmic = vec![0.0f32; 2];
        // 2 组各 1 区域，mask 用本机逻辑核 0/1（测试环境不要求真实拓扑）。
        let groups = vec![
            AffinityGroup {
                region_indices: vec![0],
                affinity_mask: 1,
            },
            AffinityGroup {
                region_indices: vec![1],
                affinity_mask: 2,
            },
        ];
        run_d1_pipeline(&mut sd, &bounds, &pp, &cosmic, &groups);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            let mut total = 0.0f32;
            let mut count = 0usize;
            for i in 0..(14 * 12) {
                if u.element_idx.get(i) == 1 {
                    total += u.mass.get(i);
                    count += 1;
                }
            }
            assert!(count >= 1, "液体应存在（可能扩散为多格）");
            assert!(
                (total - 200.0).abs() < 0.5,
                "液体总量应守恒 200kg，got {total}"
            );
        }
        crate::b_elements::elements_table::DestroyElementsTable();
    }

    #[test]
    fn module_cluster_detected_by_32x32_bounds() {
        let b = RegionBounds {
            min_x: 10,
            min_y: 20,
            max_x: 42,
            max_y: 52,
        };
        assert!(is_module_cluster(&b), "32×32 区域应识别为太空员舱集群");
        let b2 = RegionBounds {
            min_x: 1,
            min_y: 1,
            max_x: 40,
            max_y: 52,
        };
        assert!(!is_module_cluster(&b2), "38×50 非集群");
    }
}
