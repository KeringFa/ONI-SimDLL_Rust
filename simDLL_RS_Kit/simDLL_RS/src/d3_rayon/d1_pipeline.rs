//! 方向 1（1 星 1 线程）：D1 执行器。
//!
//! 持久线程执行器：每组一个常驻 `std::thread`（组数 = 物理核组 + 超线程组，
//! ≤ 逻辑核数），线程只在分组变化时重建；每帧经 mpsc 信号 + Barrier 同步，
//! 执行阶段 A/C 的组内区域物理。阶段 B（组件）保持串行（与 D2 一致）。
//! 事件经 region sink 路由后合并。亲和性绑定已移除（OS 自由调度，实测与
//! rayon 持平；低配机器 4C8T 下线程数=组数更充分，作为玩家可选优化保留）。

use crate::a_framework::sim_data::SimData;
use crate::c2_physics::region_events::{with_region_sink, RegionEvents};
use crate::c2_physics::region_rng::{with_region_rng, RegionRng};
use crate::d1_activity::RegionBounds;
use crate::d3_rayon::affinity::{assign_groups, AffinityGroup, CpuTopology};
use crate::globals::SendSyncPtr;
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::thread::JoinHandle;

/// 太空员舱集群识别：HabitatModuleMedium/Small 世界 32×32 = 1024 tiles。
/// region bounds 为区域矩形（compute_region_bounds 后与区域格子数一致）。
pub fn is_module_cluster(bounds: &RegionBounds) -> bool {
    let w = bounds.max_x.saturating_sub(bounds.min_x);
    let h = bounds.max_y.saturating_sub(bounds.min_y);
    w * h == 32 * 32
}

/// 构建 D1 分组：普通世界走 `assign_groups`（物理核独占 + 超线程两两合并），
/// 太空员舱集群单独一组（固定超线程）。
/// `reserve`：预留物理核数（1..=4，接口 mod 可配置）。
pub fn build_d1_groups(
    bounds: &[RegionBounds],
    topo: &CpuTopology,
    reserve: usize,
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
    let mut groups = assign_groups(&world_tiles, topo, reserve)?;
    // 把 assign_groups 的组内索引（world_tiles 内）映射回真实 region 索引。
    for g in groups.iter_mut() {
        g.region_indices = g.region_indices.iter().map(|&wi| world_idx[wi]).collect();
    }
    // 集群组：单独超线程组（优先复用最小物理核组的空闲逻辑核，即 ht 序号 0）。
    if !cluster_idx.is_empty() {
        let mask = ht_idle_mask(topo, 0, reserve)?;
        groups.push(AffinityGroup {
            region_indices: cluster_idx,
            affinity_mask: mask,
        });
    }
    Some(groups)
}

/// 第 `ht_index` 个超线程组的亲和掩码：从最小物理核组（索引 p−1）的空闲逻辑核
/// 开始轮转（与 `assign_groups` 内部逻辑一致）。
fn ht_idle_mask(topo: &CpuTopology, ht_index: usize, reserve: usize) -> Option<u64> {
    if !topo.has_hyper_threading() {
        return None;
    }
    let p = topo.physical_cores.saturating_sub(reserve).max(1);
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
/// 【实验 2026-08-09】亲和性绑定已移除（OS 自由调度、线程数=组数，
/// 实测 +5 FPS 来自并行度而非超线程规划）；如要恢复绑定，在
/// worker_loop 开头调用本函数即可。
#[allow(dead_code)]
#[cfg(windows)]
fn set_thread_affinity(mask: u64) {
    unsafe {
        let _ = windows::Win32::System::Threading::SetThreadAffinityMask(
            windows::Win32::System::Threading::GetCurrentThread(),
            mask as usize,
        );
    }
}

/// Linux 占位（亲和绑定已移除；如恢复需用 libc sched_setaffinity）。
#[allow(dead_code)]
#[cfg(target_os = "linux")]
fn set_thread_affinity(_mask: u64) {}

/// 每帧线程相位：A（网格物理）或 C（收尾）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    A,
    C,
}

/// 每帧共享上下文（主线程写，worker 锁内读取后释放锁再执行物理）。
struct FrameContext {
    sim: SendSyncPtr<SimData>,
    bounds: Vec<RegionBounds>,
    pp: Vec<RegionBounds>,
    cosmic: Vec<f32>,
    rng_ptrs: Vec<SendSyncPtr<RegionRng>>,
    events_ptrs: Vec<SendSyncPtr<RegionEvents>>,
    phase: Phase,
}

/// 持久线程执行器：每组一个常驻线程 + mpsc 信号 + Barrier 帧同步。
/// 线程只在分组变化时重建（避免每帧 spawn 开销——rayon 为持久池，D1 需同量级）。
struct D1Executor {
    groups: Vec<AffinityGroup>,
    senders: Vec<mpsc::Sender<()>>,
    handles: Vec<JoinHandle<()>>,
    frame: Arc<Mutex<FrameContext>>,
    barrier: Arc<Barrier>,
}

impl D1Executor {
    fn new(
        groups: Vec<AffinityGroup>,
        frame: Arc<Mutex<FrameContext>>,
        barrier: Arc<Barrier>,
    ) -> Self {
        let mut senders = Vec::with_capacity(groups.len());
        let mut handles = Vec::with_capacity(groups.len());
        for (gi, g) in groups.iter().enumerate() {
            let (tx, rx) = mpsc::channel::<()>();
            let frame = frame.clone();
            let barrier = barrier.clone();
            let g = g.clone();
            let handle = std::thread::Builder::new()
                .name(format!("simdll-d1-{gi}"))
                .spawn(move || worker_loop(rx, g, frame, barrier))
                .expect("d1: spawn worker failed");
            senders.push(tx);
            handles.push(handle);
        }
        Self {
            groups,
            senders,
            handles,
            frame,
            barrier,
        }
    }
}

impl Drop for D1Executor {
    fn drop(&mut self) {
        // 先 drop 所有 sender → worker recv 返回 Err → 退出；再 join。
        self.senders.clear();
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

/// worker 主循环：等待 tick → 锁内取本组数据 → 释放锁 → 执行物理 → Barrier。
/// panic 用 catch_unwind 包裹并照常 Barrier，避免主线程卡死。
fn worker_loop(
    rx: mpsc::Receiver<()>,
    group: AffinityGroup,
    frame: Arc<Mutex<FrameContext>>,
    barrier: Arc<Barrier>,
) {
    // 实验：不绑定亲和性，交给 OS 调度（线程数 = 组数，天然用满超线程核）。
    while let Ok(()) = rx.recv() {
        let (sim, phase, jobs) = {
            let f = frame.lock().unwrap_or_else(|e| e.into_inner());
            let sim = f.sim;
            let phase = f.phase;
            let jobs: Vec<_> = group
                .region_indices
                .iter()
                .map(|&i| {
                    let bounds = f.bounds[i];
                    let cosmic = f.cosmic[i];
                    let pp = f.pp[i];
                    let rng = f.rng_ptrs[i].get();
                    let events = f.events_ptrs[i].get();
                    (bounds, cosmic, pp, rng, events)
                })
                .collect();
            (sim, phase, jobs)
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for (bounds, cosmic, pp, rng, events) in jobs {
                with_region_rng(rng, || {
                    with_region_sink(events, || unsafe {
                        match phase {
                            Phase::A => crate::c2_physics::run_region_physics_a(
                                &mut *sim.get(),
                                bounds,
                                cosmic,
                                true,
                            ),
                            Phase::C => crate::c2_physics::run_region_physics_c(
                                &mut *sim.get(),
                                bounds,
                                pp,
                            ),
                        }
                    });
                });
            }
        }));
        barrier.wait();
    }
}

static G_D1_EXECUTOR: Mutex<Option<D1Executor>> = Mutex::new(None);

/// D1 管线：阶段 A（组并行）→ 合并 A 事件 → 阶段 B（组件串行）→
/// 阶段 C（组并行）→ 合并 C 事件。与 D2 `run_parallel_pipeline` 语义一致，
/// 线程组织为持久线程 + 亲和性（分组变化时重建执行器）。
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

    let mut executor_guard = G_D1_EXECUTOR.lock().unwrap_or_else(|e| e.into_inner());
    // 分组变化（世界增删/尺寸变化）→ 重建执行器（线程重新创建并绑定亲和性）。
    let needs_rebuild = match &*executor_guard {
        Some(ex) => ex.groups != groups,
        None => true,
    };
    if needs_rebuild {
        let frame = Arc::new(Mutex::new(FrameContext {
            sim: sim_ptr,
            bounds: Vec::new(),
            pp: Vec::new(),
            cosmic: Vec::new(),
            rng_ptrs: Vec::new(),
            events_ptrs: Vec::new(),
            phase: Phase::A,
        }));
        let barrier = Arc::new(Barrier::new(groups.len() + 1));
        *executor_guard = Some(D1Executor::new(groups.to_vec(), frame, barrier));
    }
    let executor = executor_guard.as_mut().unwrap();

    // 更新每帧上下文（主线程独占写入；worker 锁内读取）。
    {
        let mut f = executor.frame.lock().unwrap_or_else(|e| e.into_inner());
        f.sim = sim_ptr;
        f.bounds = region_bounds.to_vec();
        f.pp = region_pp_bounds.to_vec();
        f.cosmic = region_cosmic.to_vec();
        f.rng_ptrs = rng_ptrs;
        f.events_ptrs = events_ptrs;
        f.phase = Phase::A;
    }

    // —— 阶段 A：组并行 ——
    for tx in &executor.senders {
        let _ = tx.send(());
    }
    executor.barrier.wait();
    for ev in region_events.iter_mut() {
        ev.merge_into(sim);
    }

    // —— 阶段 B：组件串行（同 D2）——
    for i in 0..region_bounds.len() {
        crate::c2_physics::run_region_emitters(sim, region_bounds[i]);
    }

    // —— 阶段 C：组并行 ——
    {
        let mut f = executor.frame.lock().unwrap_or_else(|e| e.into_inner());
        f.phase = Phase::C;
    }
    for tx in &executor.senders {
        let _ = tx.send(());
    }
    executor.barrier.wait();
    for ev in region_events.iter_mut() {
        ev.merge_into(sim);
    }
}

/// 销毁 D1 执行器（回收常驻线程）——模式切换离开 D1 时由 scheduler 调用。
pub fn shutdown_executor() {
    let mut guard = G_D1_EXECUTOR.lock().unwrap_or_else(|e| e.into_inner());
    *guard = None;
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
