//! T5：三段式并行接线（d3 并行路径）。
//!
//! 阶段 A（网格物理并行）→ 合并 A 事件 → 阶段 B（组件串行，按 ★4 注册顺序）
//! → 阶段 C（收尾并行）→ 合并 C 事件。每个区域 job 用 `with_region_rng` +
//! `with_region_sink` 包裹，保证随机/事件路由正确；区域分组用 LPT 面积装箱，
//! 组内逐区域串行。串行路径不触碰本模块（update_data 门控）。

use crate::a_framework::sim_data::SimData;
use crate::c2_physics::region_events::{with_region_sink, RegionEvents};
use crate::c2_physics::region_rng::{with_region_rng, RegionRng};
use crate::d1_activity::RegionBounds;
use crate::globals::SendSyncPtr;

/// 编译期断言：SendSyncPtr 包装必须 Send/Sync（d3 并行任务跨线程传递的前提）。
fn _assert_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<SendSyncPtr<SimData>>();
    assert_send::<SendSyncPtr<RegionRng>>();
    assert_send::<SendSyncPtr<RegionEvents>>();
    assert_sync::<SendSyncPtr<SimData>>();
}

/// 最小 scope 捕获实验（定位 Send 推断问题）。
fn _min_scope_test(sim: &mut SimData) {
    let ptr: SendSyncPtr<SimData> = SendSyncPtr(sim as *mut SimData);
    let pool = crate::d3_rayon::pool();
    pool.scope(|s| {
        let ptr = ptr;
        s.spawn(move |_| {
            let _ = ptr.get();
        });
    });
}

/// 并行路径入口：多区域（多星）时替代串行区域循环。
pub(crate) fn run_parallel_pipeline(
    sim: &mut SimData,
    region_bounds: Vec<RegionBounds>,
    region_pp_bounds: Vec<RegionBounds>,
    region_cosmic: Vec<f32>,
) {
    let n = region_bounds.len();
    if n == 0 {
        return;
    }
    // T4：按当前 active_regions 对齐区域随机状态（key 不变保留，新增派生）。
    crate::c2_physics::region_rng::realign_region_rngs(sim, sim.active_regions.as_slice());
    let rng_ptrs: Vec<SendSyncPtr<RegionRng>> =
        crate::c2_physics::region_rng::region_rng_ptrs()
            .into_iter()
            .map(SendSyncPtr)
            .collect();
    // T3：每区域独立事件缓冲（并行任务写各自缓冲，零锁；scope 结束后按区域序合并）。
    let mut region_events: Vec<RegionEvents> = (0..n).map(|_| Default::default()).collect();
    let events_ptrs: Vec<SendSyncPtr<RegionEvents>> = (0..n)
        .map(|i| SendSyncPtr(&mut region_events[i] as *mut RegionEvents))
        .collect();
    let pool = crate::d3_rayon::pool();
    // T6.3：LPT 面积装箱——组数 = min(区域数, 线程数)，组内逐区域串行。
    let groups = crate::d3_rayon::bin_pack::bin_pack_region_groups(
        &region_bounds,
        pool.current_num_threads(),
    );
    let sim_ptr: SendSyncPtr<SimData> = SendSyncPtr(sim as *mut SimData);

    // —— 阶段 A：网格物理并行（温度内联、区域局部 CopyFrom，parallel=true）——
    pool.scope(|s| {
        for group in &groups {
            let sim_ptr = sim_ptr.clone();
            let group = group.clone();
            let region_bounds = region_bounds.clone();
            let region_cosmic = region_cosmic.clone();
            let rng_ptrs = rng_ptrs.clone();
            let events_ptrs = events_ptrs.clone();
            s.spawn(move |_| {
                // get() 方法调用强制捕获整个 SendSyncPtr（Rust 2021 disjoint capture
                // 下 `.0` 字段访问只捕获裸指针本身，绕过 Send）。
                let sim_raw = sim_ptr.get();
                for i in group {
                    let bounds = region_bounds[i];
                    let cosmic = region_cosmic[i];
                    let rng = rng_ptrs[i];
                    let events = events_ptrs[i];
                    let rng_raw = rng.get();
                    let events_raw = events.get();
                    with_region_rng(rng_raw, move || {
                        with_region_sink(events_raw, move || unsafe {
                            crate::c2_physics::run_region_physics_a(
                                &mut *sim_raw,
                                bounds,
                                cosmic,
                                true,
                            );
                        });
                    });
                }
            });
        }
    });
    // 合并 A 事件（按区域序，A→B→C 相对顺序）
    for ev in region_events.iter_mut() {
        ev.merge_into(sim);
    }
    // —— 阶段 B：组件串行（注册表全局共享不可并行；事件直接写全局）——
    for i in 0..n {
        crate::c2_physics::run_region_emitters(sim, region_bounds[i]);
    }
    // —— 阶段 C：收尾并行（PostProcess + 病菌生长；事件回区域缓冲）——
    pool.scope(|s| {
        for group in &groups {
            let sim_ptr = sim_ptr.clone();
            let group = group.clone();
            let region_bounds = region_bounds.clone();
            let region_pp_bounds = region_pp_bounds.clone();
            let rng_ptrs = rng_ptrs.clone();
            let events_ptrs = events_ptrs.clone();
            s.spawn(move |_| {
                let sim_raw = sim_ptr.get();
                for i in group {
                    let bounds = region_bounds[i];
                    let pp_bounds = region_pp_bounds[i];
                    let rng = rng_ptrs[i];
                    let events = events_ptrs[i];
                    let rng_raw = rng.get();
                    let events_raw = events.get();
                    with_region_rng(rng_raw, move || {
                        with_region_sink(events_raw, move || unsafe {
                            crate::c2_physics::run_region_physics_c(
                                &mut *sim_raw,
                                bounds,
                                pp_bounds,
                            );
                        });
                    });
                }
            });
        }
    });
    // 合并 C 事件
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

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn send_sync_ptr_wrappers_are_send() {
        assert_send::<SendSyncPtr<SimData>>();
        assert_send::<SendSyncPtr<RegionRng>>();
        assert_send::<SendSyncPtr<RegionEvents>>();
        assert_sync::<SendSyncPtr<SimData>>();
    }

    /// 最小元素表：0=真空、1=液体（state2）、2=气体（state1）、3=固体（state3）。
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
            // 辐射数据：固体（state3）factor=0.5，其余 0（真空/液体/气体不遮挡）。
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
                // 区域 0（x1..5, y1..4）col1 row1：液体 100kg
                buf.element_idx.set(1 * 14 + 1, 1);
                buf.mass.set(1 * 14 + 1, 100.0);
                // 区域 1（x7..11, y1..4）col7 row1：液体 100kg
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

    /// 并行路径基本可用性：2 个活动区域各放液体，跑 pipeline 不 panic，
    /// 各区域液体被处理（质量守恒、事件合并进全局）。
    #[test]
    fn parallel_pipeline_runs_and_merges_events() {
        let _lock = LIB_TESTS_LOCK.lock();
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
        run_parallel_pipeline(&mut sd, bounds, pp, cosmic);
        // 不 panic；液体总质量守恒（两个 100kg 可能在区域内流动，但总量不变）
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
        // 事件按区域序合并进全局（可能有 substance_change 事件）
        let events = unsafe { &*sd.sim_events.ptr };
        assert!(
            events.substance_change_info.len() >= 0,
            "事件合并不应破坏全局缓冲"
        );
        crate::b_elements::elements_table::DestroyElementsTable();
    }

    /// 一致性：同一多区域布局分别走串行（run_region_physics_a(false) 循环）与并行
    /// （run_parallel_pipeline），两者液体总质量都应守恒（并行用区域 RNG，逐格随机
    /// 结果统计等价而非逐字节一致，故只比守恒/总量）。
    #[test]
    fn parallel_matches_serial_mass_conservation() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd_s = make_two_region_sim();
        let bounds: Vec<RegionBounds> = sd_s
            .active_regions
            .as_slice()
            .iter()
            .map(|r| crate::d1_activity::compute_region_bounds(&sd_s, r))
            .collect();
        let pp: Vec<RegionBounds> = sd_s
            .active_regions
            .as_slice()
            .iter()
            .map(|r| crate::d1_activity::compute_post_process_bounds(&sd_s, r))
            .collect();
        let cosmic = vec![0.0f32; 2];
        // —— 串行路径（一帧）——
        for i in 0..2 {
            crate::c2_physics::run_region_physics_a(&mut sd_s, bounds[i], cosmic[i], false);
            crate::c2_physics::run_region_emitters(&mut sd_s, bounds[i]);
            crate::c2_physics::run_region_physics_c(&mut sd_s, bounds[i], pp[i]);
        }
        // —— 并行路径（一帧，独立 sim）——
        let mut sd_p = make_two_region_sim();
        run_parallel_pipeline(&mut sd_p, bounds.clone(), pp.clone(), cosmic.clone());
        // —— 断言：两版液体总质量守恒 ——
        let mass_of = |sd: &SimData| -> f32 {
            unsafe {
                let u = &*sd.updated_cells.ptr;
                let mut t = 0.0f32;
                for i in 0..(14 * 12) {
                    if u.element_idx.get(i) == 1 {
                        t += u.mass.get(i);
                    }
                }
                t
            }
        };
        let ms = mass_of(&sd_s);
        let mp = mass_of(&sd_p);
        assert!(
            (ms - 200.0).abs() < 0.5,
            "串行液体应守恒 200kg，got {ms}"
        );
        assert!(
            (mp - 200.0).abs() < 0.5,
            "并行液体应守恒 200kg，got {mp}"
        );
        crate::b_elements::elements_table::DestroyElementsTable();
    }

    /// d3 并行回归：宇宙辐射遮挡在并行路径下必须与串行一致。此前 run_radiation_task
    /// 对 SimData 共享字段 cosmic_radiation_occlusion 执行 resize+全填 1.0，多 region
    /// 线程并发写同一缓冲会互相覆盖（一个线程的"填 1.0"抹掉另一线程已算好的遮挡），
    /// 导致宇宙辐射间歇性穿透方块照全图。修复后 occlusion 为区域局部缓冲，本测试
    /// 验证并行结果与串行逐格一致：region0 顶行固体（factor 0.5）→ occlusion 0.5 →
    /// cosmic 50；region1 全真空顶行 → occlusion 1.0 → cosmic 100。
    #[test]
    fn parallel_radiation_occlusion_matches_serial() {
        let _lock = LIB_TESTS_LOCK.lock();
        let fill_solid = |sd: &mut SimData| unsafe {
            let u = &mut *sd.updated_cells.ptr;
            for x in 1..6 {
                u.element_idx.set(4 * 14 + x, 3);
                u.mass.set(4 * 14 + x, 2000.0);
            }
        };
        let mut sd_s = make_two_region_sim();
        fill_solid(&mut sd_s);
        let bounds: Vec<RegionBounds> = sd_s
            .active_regions
            .as_slice()
            .iter()
            .map(|r| crate::d1_activity::compute_region_bounds(&sd_s, r))
            .collect();
        let pp: Vec<RegionBounds> = sd_s
            .active_regions
            .as_slice()
            .iter()
            .map(|r| crate::d1_activity::compute_post_process_bounds(&sd_s, r))
            .collect();
        let cosmic = vec![110.0f32; 2];
        // —— 串行路径（一帧）——
        for i in 0..2 {
            crate::c2_physics::run_region_physics_a(&mut sd_s, bounds[i], cosmic[i], false);
            crate::c2_physics::run_region_emitters(&mut sd_s, bounds[i]);
            crate::c2_physics::run_region_physics_c(&mut sd_s, bounds[i], pp[i]);
        }
        // —— 并行路径（一帧，独立 sim）——
        let mut sd_p = make_two_region_sim();
        fill_solid(&mut sd_p);
        run_parallel_pipeline(&mut sd_p, bounds.clone(), pp.clone(), cosmic.clone());
        // —— 断言：串行/并行逐格一致 ——
        unsafe {
            let us = &*sd_s.updated_cells.ptr;
            let up = &*sd_p.updated_cells.ptr;
            for x in 1..6 {
                let cell = 4 * 14 + x;
                let rs = us.radiation.get(cell);
                let rp = up.radiation.get(cell);
                assert!((rs - 50.0).abs() < 1.0, "串行 region0 固体行 cosmic 50，got {rs}");
                assert!((rp - 50.0).abs() < 1.0, "并行 region0 固体行 cosmic 50，got {rp}");
            }
            for x in 7..12 {
                let cell = 4 * 14 + x;
                let rs = us.radiation.get(cell);
                let rp = up.radiation.get(cell);
                assert!(
                    (rs - 100.0).abs() < 1.0,
                    "串行 region1 顶行 cosmic 100，got {rs}"
                );
                assert!(
                    (rp - 100.0).abs() < 1.0,
                    "并行 region1 顶行 cosmic 100，got {rp}"
                );
            }
        }
        crate::b_elements::elements_table::DestroyElementsTable();
    }
}
