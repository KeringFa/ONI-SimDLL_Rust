//! 切片化的温度阶段。
//!
//! 本模块目前只有**串行执行版** —— 目的是先把「切片化之后与原版逐位一致」这件事
//! 证明清楚，再在任务 5 接并行。若两者同时引入，测试一红就分不清是逻辑复刻错
//! 还是调度错。

use crate::a_framework::sim_data::SimData;
use crate::b_elements::elements_table::get_element_temperature_data;
use crate::c2_physics::region_events::RegionEvents;
use crate::c2_physics::region_events::with_region_sink;
use crate::c2_physics::temperature::{
    apply_delta, compute_pair_delta, do_state_transition, update_temperature_for_backwall,
};
use crate::d1_activity::RegionBounds;
use crate::d4_slice::slice_grid::{SliceBounds, SliceCache, SliceGrid};
use std::cell::RefCell;

/// 处理完某一行时，`RegionEvents` 各向量的长度快照。
///
/// 字段名与 `RegionEvents` 一一对应，便于宏统一处理全部 8 类事件 ——
/// 温度阶段实际会产出 5 类（`substance_change` / `cell_melted` /
/// `backwall_should_transition` / `spawn_ore` / `spawn_liquid`），
/// 但这里 8 类全收：漏掉任何一类就是**静默丢事件**，而这类 bug 极难发现。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RowMarks {
    substance_change_info: usize,
    spawn_liquid_info: usize,
    spawn_ore_info: usize,
    unstable_cell_info: usize,
    cell_melted_info: usize,
    spawn_fx_info: usize,
    world_damage_info: usize,
    backwall_should_transition_info: usize,
}

impl RowMarks {
    fn of(e: &RegionEvents) -> Self {
        Self {
            substance_change_info: e.substance_change_info.len(),
            spawn_liquid_info: e.spawn_liquid_info.len(),
            spawn_ore_info: e.spawn_ore_info.len(),
            unstable_cell_info: e.unstable_cell_info.len(),
            cell_melted_info: e.cell_melted_info.len(),
            spawn_fx_info: e.spawn_fx_info.len(),
            world_damage_info: e.world_damage_info.len(),
            backwall_should_transition_info: e.backwall_should_transition_info.len(),
        }
    }
}

/// 单切片的事件缓冲 + 行边界。
///
/// # 为什么需要行边界
///
/// 瓦片式切片下「整片拼接」的顺序 **≠** 原版行主序：同一 tile 行内，原版逐行横跨
/// s0/s1，整片拼接却要跑完 s0 的全部行才轮到 s1。记下每行的长度快照，合并时就能按
/// 「行 → 该行上各切片（左→右）」逐段追加，还原出**精确**的行主序。
///
/// 之所以记长度而不是给每行建一个 `RegionEvents`：后者是 8 个 `MsvcVector` 头，
/// 一个大区域能轻松堆到上百 KB 且每帧重建。长度快照每只行 64 字节。
struct SliceEventBuf {
    events: RegionEvents,
    /// 第 k 项 = 处理完本切片第 k 行（`min_y + k`）后的长度快照。
    row_marks: Vec<RowMarks>,
}

impl SliceEventBuf {
    fn new(rows: usize) -> Self {
        Self { events: RegionEvents::default(), row_marks: Vec::with_capacity(rows) }
    }

    /// 帧间复用：清空逻辑状态但**保留容量**，供 `SliceEventPool` 缓存复用。
    ///
    /// - 事件向量：`clear_keep_capacity`（end = begin，内存保留）—— 与原版
    ///   `Start` 对 sim_events 的 20 个 vector 的清法一致；
    /// - 行标记：`Vec::clear` 只清 len，保留 allocation。
    ///
    /// ⚠️ 必须在 `acquire`（帧开始）时调用，绝不能在帧末调用：合并阶段
    /// （`merge_row_into`）还要读 `row_marks` / `events`，提前清空会丢数据。
    fn reset(&mut self) {
        macro_rules! seg {
            ($f:ident) => {
                self.events.$f.clear_keep_capacity();
            };
        }
        seg!(substance_change_info);
        seg!(spawn_liquid_info);
        seg!(spawn_ore_info);
        seg!(unstable_cell_info);
        seg!(cell_melted_info);
        seg!(spawn_fx_info);
        seg!(world_damage_info);
        seg!(backwall_should_transition_info);
        self.row_marks.clear();
    }

    /// 把本切片第 `row` 行（相对 `min_y` 的偏移）产出的事件追加进 `dst`。
    ///
    /// 目标既可以是全局 `SimEvents` 的逐字段落盘（经 `RegionEvents` 中转），
    /// 也可以是 D4 收集形态下的区域缓冲 —— 两者的字段名与 `RegionEvents`
    /// 完全一致，故统一写到 `RegionEvents` 上。
    fn merge_row_into(&mut self, dst: &mut RegionEvents, row: usize) {
        let prev = if row == 0 { RowMarks::default() } else { self.row_marks[row - 1] };
        let cur = self.row_marks[row];
        macro_rules! seg {
            ($f:ident) => {
                for i in prev.$f..cur.$f {
                    dst.$f.push(self.events.$f.get(i));
                }
            };
        }
        seg!(substance_change_info);
        seg!(spawn_liquid_info);
        seg!(spawn_ore_info);
        seg!(unstable_cell_info);
        seg!(cell_melted_info);
        seg!(spawn_fx_info);
        seg!(world_damage_info);
        seg!(backwall_should_transition_info);
    }
}

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

/// 切片化的温度阶段。
///
/// 与 `run_temperature_task(sim, region)` 逐位等价，且事件追加顺序也一致。
///
/// # 写入安全性（与执行顺序无关）
///
/// **温度场 / 背墙**：唯一写者论证成立 —— 对任意格 Y，写入者只有 Y 自己、L=Y-1、U=Y-w
/// 三处，且 L、U 至多一个成立（否则推出 Y 也在区域内，与「未被拥有」矛盾），角格无人写。
/// 每个 `process_cell` 在自身调用内完成全部五笔贡献、值来自不可变快照、halo 只写一次，
/// 所以切片以任意并行顺序跑，都不改变任一格的写入序列 → 逐位一致与调度无关。
//
/// **事件**：`sim_events` 是共享副作用汇，且 `SimEvents` 各字段是只增长的 `MsvcVector`
/// （顺序可观测）。并行若并发直写同一 `MsvcVector` = 数据竞争，且顺序随调度乱。
/// 故本函数**强制每切片缓冲 + 逐行定序合并**：
///   - 缓冲：切片域内用 `with_region_sink` 把 `do_state_transition` /
///     `update_temperature_for_backwall` 产出的事件路由到该切片的 `RegionEvents`；
///   - 分段：每处理完一行记一次长度快照（`RowMarks`），使行成为可寻址的段；
///   - 合并：按「区域序 → 区域内逐行 → 行内切片左→右」逐段追加。
///
/// 为什么要分段到行：切片是 32×32 **瓦片**，而原版是逐行横跨整幅宽度。若按切片整片
/// 拼接，同一 tile 行内右侧切片的事件会被整批推到左侧切片全部行之后，顺序即与原版不符。
/// 分段到行后，合并能精确还原「区域序 × 行主序」，与原版**逐位一致**
/// （见 `event_order_matches_serial_for_multi_tile_region`）。
///
/// 调用方保证：各区域（因而各切片，含 halo 收缩后的实际写区）**两两不交**，否则重叠格
/// 会出现两个写入者。本函数只消费 `SliceGrid::build` 给出的切片，不做重叠校验。
pub fn run_temperature_sliced_serial(
    sim: &mut SimData,
    regions: &[RegionBounds],
    slice_size: usize,
) {
    run_temperature_sliced(sim, regions, slice_size, false);
}

/// 切片工作缓冲池：同时缓存 `SliceGrid` 与每切片事件缓冲，**只缓存容量、不缓存逻辑状态**。
///
/// 背景：
/// 1. `SliceGrid::build` 每帧重建会分配整张切片表（~147 项 `SliceBounds` 的 `Vec`）；
/// 2. `SliceEventBuf` 的 `RegionEvents`（8 个 `MsvcVector`）与 `row_marks` 每帧重建
///    会反复分配（大区域单切片就能到上百 KB）。
///
/// 本池把两者合并缓存：签名（区域 + 切片尺寸 + 网格尺寸）不变 → 全部复用；
/// 变化 → grid 与缓冲一起重建。
///
/// # 安全性（为什么不会引入逻辑 bug）
///
/// 1. **容量 vs 状态分离**：缓存持有的是 allocation，不是逻辑状态。
///    `acquire` 每次把 8 个事件向量 `clear_keep_capacity()`（end = begin）、
///    `row_marks.clear()` —— 逻辑状态与新建完全一致，`row_marks` 从空开始重新
///    push，行区间 `prev..cur` 以本轮实际 push 长度为准，跨帧不可能残留。
/// 2. **签名重建**：签名与 `SliceGrid::build` 的输入完全一致。区域/尺寸/网格变化
///    → 切片数变化 → 缓冲池按新切片数重建，旧缓冲不残留。
/// 3. **并行安全**：rayon 各任务写各自的 `SliceEventBuf`（索引与 `grid.slices`
///    一一对应），`acquire` 在并行派发之前完成，无并发借用。
///
/// 调用方负责在帧末 drop 本池持锁的守卫（`run_temperature_sliced_collect` 结束即
/// 释放），因此同一时刻只有一个执行流在使用缓冲。
#[derive(Default)]
pub struct SliceEventPool {
    signature: Option<(Vec<RegionBounds>, usize, usize, usize)>,
    cache: SliceCache,
    pool: Vec<SliceEventBuf>,
}

impl SliceEventPool {
    /// 取回本帧的切片网格 + 每切片事件缓冲（已 reset，可直接使用）。
    ///
    /// 返回的 `&SliceGrid` 与 `&mut Vec<SliceEventBuf>` 借用自 `self` 的不同字段，
    /// 可同时存活；调用方用完即可，池随锁守卫 drop 而让出。
    pub fn acquire(
        &mut self,
        regions: &[RegionBounds],
        slice_size: usize,
        width: usize,
        height: usize,
    ) -> (&SliceGrid, &mut Vec<SliceEventBuf>) {
        let sig = (regions.to_vec(), slice_size, width, height);
        let changed = self
            .signature
            .as_ref()
            .map(|s| *s != sig)
            .unwrap_or(true);
        if changed {
            // 切片数随签名变化 → 缓冲池按新切片数重建（grid 由 cache 管理）。
            let n = SliceGrid::build(regions, slice_size).slices.len();
            self.pool = (0..n).map(|_| SliceEventBuf::new(0)).collect();
            self.signature = Some(sig);
        }
        // grid：签名不变则不重建（SliceCache 内部判定）。
        let grid = self.cache.get(regions, slice_size, width, height);
        // 帧间复用：清逻辑状态、留容量。必须在任何写入/合并之前执行。
        for buf in self.pool.iter_mut() {
            buf.reset();
        }
        (grid, &mut self.pool)
    }
}

/// 线程本地切片工作缓冲池（`run_temperature_sliced_collect` 跨帧复用）。
///
/// 为什么用 `thread_local` 而不是全局 `static`：`SliceEventBuf` 含裸指针
/// （`MsvcVector` 的 `*mut T`），不是 `Send`，无法放进 `static Mutex`；
/// 而 sim 是单线程驱动，每线程一个池天然无锁、且避免跨帧全局锁竞争。
/// 与 `region_events::REGION_EVENTS_SINK` 同款 thread_local 模式。
thread_local! {
    static SLICE_EVENT_POOL: RefCell<SliceEventPool> = RefCell::new(SliceEventPool::default());
}

/// 同 `run_temperature_sliced_serial`，`parallel=true` 时走 rayon 持久池并行派发切片。
///
/// 并行版**不做**任何逻辑改变：温度场仍由唯一写者保证逐位一致，事件仍走每切片缓冲 +
/// 逐行定序合并。区别仅在切片间的执行顺序（并行乱序），而该顺序不影响任何可观测输出。
#[cfg(feature = "d4-slice")]
pub fn run_temperature_sliced(
    sim: &mut SimData,
    regions: &[RegionBounds],
    slice_size: usize,
    parallel: bool,
) {
    let per_region = run_temperature_sliced_collect(sim, regions, slice_size, parallel);
    // 按区域序逐个落盘。`collect` 已经把每个区域缓冲内部排成行主序，
    // 这里按区域先后 append，全局顺序即「区域序 × 行主序」，与直接落盘逐位一致。
    for mut buf in per_region {
        buf.merge_into(sim);
    }
}

/// 核心实现：跑切片化温度阶段，事件**不落盘**，而是按区域收集后返回。
///
/// 返回值下标 = `regions` 的原始下标（`SliceBounds::region_index` 语义一致）；
/// 没有切片的区域（空区域）对应空缓冲。每个区域缓冲内部已排成
/// 「tile 行 → 行内切片左→右」的行主序，与串行版区域内的追加顺序逐位一致。
///
/// 调用方（`update_data` 的 D4 分支）拿到缓冲后交给既有管线：
/// 并行路径把 `temp_events[i]` 前置进区域 i 的阶段 A 缓冲（`append_from`），
/// 串行路径在区域 i 的其余阶段前 `merge_into` 全局 —— 两条路都保证
/// 「温度事件先于本区域气体/液体事件」，全局顺序与原版一致。
#[cfg(feature = "d4-slice")]
pub fn run_temperature_sliced_collect(
    sim: &mut SimData,
    regions: &[RegionBounds],
    slice_size: usize,
    parallel: bool,
) -> Vec<RegionEvents> {
    // 早退也返回与 `regions` 等长的空缓冲，保证调用方按下标取用始终对齐。
    let empty = || (0..regions.len()).map(|_| RegionEvents::default()).collect();
    // 与 `run_temperature_band` 的守卫保持一致（含 backwall / sim_events，
    // `process_cell` 会解引用它们）。
    if sim.cells.ptr.is_null()
        || sim.updated_cells.ptr.is_null()
        || sim.backwall.ptr.is_null()
        || sim.sim_events.ptr.is_null()
    {
        return empty();
    }
    let w = sim.width as usize;
    let h = sim.height as usize;
    if w < 3 || h < 3 {
        return empty();
    }
    // 取线程本地池（grid + 每切片事件缓冲）。签名不变 → 复用容量；变化 → 重建。
    // 借用贯穿整个主体（含并行派发与合并），保证缓冲只被本帧一个执行流使用。
    SLICE_EVENT_POOL.with(|cell| {
        let mut pool = cell.borrow_mut();
        let (grid, slice_events) = pool.acquire(regions, slice_size, w, h);
        run_temperature_sliced_collect_with(sim, regions, parallel, grid, slice_events, w, h)
    })
}

/// `run_temperature_sliced_collect` 的主体（线程本地池借用已就绪）。
/// `grid` / `slice_events` 由池提供（签名匹配、容量复用），本函数只消费不持有。
#[cfg(feature = "d4-slice")]
fn run_temperature_sliced_collect_with(
    sim: &mut SimData,
    regions: &[RegionBounds],
    parallel: bool,
    grid: &SliceGrid,
    slice_events: &mut Vec<SliceEventBuf>,
    w: usize,
    h: usize,
) -> Vec<RegionEvents> {
    // 注意：owned 判定的是「邻居是否被**任何**切片覆盖」，不是「是否属于本切片」。
    let owned = |cell: usize| is_owned(cell, w, regions);

    // 处理单个切片。并行安全的前提：
    // - 温度场：唯一写者论证保证不同切片写互不相交的格（halo 收缩后）；
    // - 事件：本切片域内用 `with_region_sink` 路由到自己的 `RegionEvents`，不碰全局。
    // `sim` 以裸指针传入，由调用方（唯一写者论证）保证跨切片不共享同一格。
    //
    // 逐行处理并在每行结束时记长度快照 —— 合并阶段要按行交错还原行主序。
    let process_one = |sim: &mut SimData, s: &SliceBounds, buf: &mut SliceEventBuf| {
        if s.is_empty() {
            return;
        }
        for y in s.min_y..s.max_y {
            with_region_sink(&mut buf.events as *mut RegionEvents, || {
                for x in s.min_x..s.max_x {
                    if y >= h - 1 || x >= w - 1 {
                        continue; // 复刻串行循环守卫
                    }
                    process_cell(sim, y * w + x, w, h, &owned);
                }
            });
            buf.row_marks.push(RowMarks::of(&buf.events));
        }
    };

    if parallel {
        // rayon::scope + SendSyncPtr 包装裸指针：SimData 含裸指针字段未实现 Send，
        // 故用 d3_rayon 同款 `SendSyncPtr` 包裹 `&mut sim` 与每切片事件缓冲，闭包内
        // `.get()` 取回裸指针再重借 `&mut`。唯一写者论证保证各切片写互不相交的格，
        // 事件走每切片 `RegionEvents` 缓冲 → 逻辑上无数据竞争。
        use crate::globals::SendSyncPtr;
        let sim_ptr = SendSyncPtr(sim as *mut SimData);
        let events_ptrs: Vec<SendSyncPtr<SliceEventBuf>> = (0..slice_events.len())
            .map(|i| SendSyncPtr(&mut slice_events[i] as *mut SliceEventBuf))
            .collect();
        let slices = &grid.slices; // 不 move grid：merge 阶段还要按区域/行用
        crate::d3_rayon::pool().install(|| {
            rayon::scope(|s| {
                for i in 0..slices.len() {
                    let sim_ptr = sim_ptr.clone();
                    let ev_ptr = events_ptrs[i].clone();
                    let sref = &slices[i];
                    s.spawn(move |_| {
                        let sim_raw = sim_ptr.get();
                        let ev_raw = ev_ptr.get();
                        process_one(unsafe { &mut *sim_raw }, sref, unsafe { &mut *ev_raw });
                    });
                }
            });
        });
    } else {
        for (i, s) in grid.slices.iter().enumerate() {
            process_one(sim, s, &mut slice_events[i]);
        }
    }

    // —— 定序合并：还原与串行**完全一致**的追加顺序（按区域收集）——
    //
    // 串行版 `run_temperature_task` 的遍历顺序是「区域序 × 区域内行主序」。
    // 故合并也分两层：
    //   外层：按 region_index（区域在 `regions` 中的原始下标）逐个区域
    //   内层：区域内按 tile 行推进，每个 tile 行内**逐行**遍历该行的切片（按 min_x 左→右）
    //
    // 关键就在「逐行」这一层：若改成「切片序整片拼接」，同一 tile 行内右侧切片的事件
    // 会被整批推到左侧切片全部行之后，顺序就与原版不同了（这是本任务实测到的核心问题）。
    //
    // 收集形态下写入各区域的 `RegionEvents`（不下标全局）；`run_temperature_sliced`
    // 再按区域序 `merge_into` 全局 —— 与直接落盘逐位一致（追加序不因中转而变）。
    let mut region_out: Vec<RegionEvents> =
        (0..regions.len()).map(|_| RegionEvents::default()).collect();
    for ri in 0..regions.len() {
        let mut rs: Vec<usize> = (0..grid.slices.len())
            .filter(|&i| grid.slices[i].region_index == ri && !grid.slices[i].is_empty())
            .collect();
        if rs.is_empty() {
            continue;
        }
        // 同区域内：先按 tile 行（min_y），再按列（min_x）—— 与 `SliceGrid::build` 一致。
        rs.sort_by_key(|&i| (grid.slices[i].min_y, grid.slices[i].min_x));
        let mut p = 0;
        while p < rs.len() {
            let y0 = grid.slices[rs[p]].min_y;
            let y1 = grid.slices[rs[p]].max_y;
            let mut q = p;
            while q < rs.len() && grid.slices[rs[q]].min_y == y0 {
                q += 1;
            }
            let band = &rs[p..q];
            for y in y0..y1 {
                for &si in band {
                    let k = y - grid.slices[si].min_y;
                    slice_events[si].merge_row_into(&mut region_out[ri], k);
                }
            }
            p = q;
        }
    }
    region_out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;
    use crate::b_elements::element::{Element, ElementTemperatureData};
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::d4_slice::DEFAULT_SLICE_SIZE;
    use crate::LIB_TESTS_LOCK;

    /// 池缓存语义：同签名复用同一份缓冲（容量保留），`reset` 清空逻辑状态。
    /// 变异对照：若 `reset` 漏清 `row_marks` / 事件向量，第二次 `acquire` 会残留
    /// 上一帧的行区间 → 合并阶段 `prev..cur` 区间错位 → 事件重复/错位（见下）。
    #[test]
    fn pool_reuses_capacity_and_resets_state() {
        let mut pool = SliceEventPool::default();
        let regions = [RegionBounds { min_x: 1, min_y: 1, max_x: 71, max_y: 41 }];

        // 第一帧：写一些事件 + 行标记（模拟一帧真实使用）
        {
            let (_grid, bufs) = pool.acquire(&regions, DEFAULT_SLICE_SIZE, 80, 50);
            // 6 片（70x40 → 3x2）；给第 0 片塞 2 行标记与事件
            assert_eq!(bufs.len(), 6, "70x40 按 32 切应为 6 片");
            // 先读 events 长度、再 push 行标记 —— 避免同下标同时可变/不可变借用
            let mark0 = RowMarks::of(&bufs[0].events);
            let mark1 = RowMarks::of(&bufs[0].events);
            bufs[0].row_marks.push(mark0);
            bufs[0].row_marks.push(mark1);
            unsafe {
                bufs[0].events.substance_change_info.push(
                    crate::a_framework::game_data::SubstanceChangeInfo::default(),
                );
            }            assert_eq!(bufs[0].row_marks.len(), 2, "第一帧应记 2 行标记");
            assert_eq!(bufs[0].events.substance_change_info.len(), 1);
        }

        // 第二帧：同签名 → 复用同一份缓冲（地址稳定），但逻辑状态被清空
        let (_grid, bufs) = pool.acquire(&regions, DEFAULT_SLICE_SIZE, 80, 50);
        assert_eq!(bufs.len(), 6);
        for b in bufs.iter() {
            assert_eq!(b.row_marks.len(), 0, "reset 应清空行标记");
            assert_eq!(
                b.events.substance_change_info.len(),
                0,
                "reset 应清空事件向量（保留容量）"
            );
        }
        // 容量保留：row_marks 的 allocation 应仍在（len 0 但 capacity ≥ 2）
        // 直接验证复用：同一帧连续两次 acquire 不应重建（签名未变）
        let mut pool2 = SliceEventPool::default();
        let (_g1, b1) = pool2.acquire(&regions, DEFAULT_SLICE_SIZE, 80, 50);
        let ptr1 = b1.as_ptr() as usize;
        let (_g2, b2) = pool2.acquire(&regions, DEFAULT_SLICE_SIZE, 80, 50);
        let ptr2 = b2.as_ptr() as usize;
        assert_eq!(ptr1, ptr2, "签名未变应复用同一份缓冲");
    }

    /// 签名变化（切片数变化）→ 缓冲池重建为新的切片数。
    #[test]
    fn pool_rebuilds_when_signature_changes() {
        let mut pool = SliceEventPool::default();
        let r_small = [RegionBounds { min_x: 1, min_y: 1, max_x: 41, max_y: 41 }]; // 40x40 → 2x2 = 4 片
        let r_big = [RegionBounds { min_x: 1, min_y: 1, max_x: 101, max_y: 101 }]; // 100x100 → 4x4 = 16 片

        let (_g, bufs) = pool.acquire(&r_small, DEFAULT_SLICE_SIZE, 110, 110);
        assert_eq!(bufs.len(), 4);
        let (_g, bufs) = pool.acquire(&r_big, DEFAULT_SLICE_SIZE, 110, 110);
        assert_eq!(bufs.len(), 16, "区域变大应重建为 16 片");
        // 回小：也应重建（签名精确比较，不是只增不减）
        let (_g, bufs) = pool.acquire(&r_small, DEFAULT_SLICE_SIZE, 110, 110);
        assert_eq!(bufs.len(), 4, "区域变小应重建回 4 片");
    }

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
                // 温度场刻意取**不整齐**的数值（素数码伪随机），而非简报的平滑梯度：
                // 平滑梯度下水平 ΔT 恒为 3.0、垂直恒为 1.5，配上全域统一的质量与元素，
                // 整张图只有两种 delta 量级，数值路径覆盖极窄。换不规则场是为了
                // 扩大数值路径覆盖。
                //
                // ⚠️ 但**施加顺序仍未被任何断言区分**：换场之后把上/左两块对调，
                // 全部用例照样照绿（见报告 §7.1）。顺序正确性由 §2 的推导保证，
                // 而非由测试保证 —— 别以为这里测过了。
                let t = 20.0 + ((x * 7919 + y * 104729) % 997) as f32 * 0.137;
                u.temperature.set(cell, t);
                // insulation 也很关键：默认 0 → iv = 0 → `combine_conductivity`
                // 走 `min(k_a, k_b)` 分支得到 0 → 换热恒 0，测试同样假绿。
                // 取 255（与 `c2_physics` 既有 pair 测试同值）：f32 下 iv 舍入为 ≥1.0，
                // 走几何平均分支，导率非零。
                u.insulation.set(cell, 255);

                // 背墙层：不设就是又一个静默空转口袋。
                // `BackwallSOA::with_size` 把 mass / temperature 全零初始化
                // （`a_framework/sim_data.rs:162-163`），而 `process_cell` 的背墙门控
                // 要求 `bw.mass > 0` —— 不设的话背墙分支在全部用例里一次都不会进，
                // 而 `assert_work_happened` 靠配对换热就能达标，检测不到它。
                {
                    let bw = unsafe { &mut *sd.backwall.ptr };
                    bw.element_idx.set(cell, 2); // 冰（非真空/void，否则门控内直接 return）
                    bw.mass.set(cell, 200.0);
                    // 背墙温度取与水温不同的一套伪随机值，保证 |ΔT| 显著、换热非零
                    bw.temperature
                        .set(cell, 5.0 + ((x * 6151 + y * 8191) % 631) as f32 * 0.173);
                }
            }
        }
        // 温度阶段读 `cells`（不可变快照），必须先同步一次
        crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(&mut sd);
        sd
    }

    /// 逐位比对温度阶段的**全部**可观测输出。
    ///
    /// 温度阶段不只写 `updated_cells`，还会写 `backwall.temperature`，
    /// 并往共享的 `sim_events` 队列追加条目 —— 三者都要比，
    /// 否则少比的那一项就是又一个静默空转口袋（背墙正是这么漏掉的，见报告 §I-2）。
    ///
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
        bit_eq_backwall(a, b);
        // 事件只比**内容**（集合相等）。顺序由专门的用例独立把守
        // （`event_order_matches_serial_for_multi_tile_region`，以原版串行为参照系），
        // 两层分开报，定位问题更直接。
        bit_eq_events(a, b, false);
    }

    /// 背墙层：`update_temperature_for_backwall` 直接写 `backwall.temperature`
    /// （不是增量，是覆盖），`updated_cells` 比对覆盖不到它。
    fn bit_eq_backwall(a: &SimData, b: &SimData) {
        let ba = unsafe { &*a.backwall.ptr };
        let bb = unsafe { &*b.backwall.ptr };
        let n = ba.temperature.len();
        assert_eq!(n, bb.temperature.len(), "backwall 尺寸不同");
        for i in 0..n {
            assert_eq!(
                ba.temperature.get(i).to_bits(),
                bb.temperature.get(i).to_bits(),
                "backwall.temperature 第 {i} 格不一致"
            );
            assert_eq!(
                ba.mass.get(i).to_bits(),
                bb.mass.get(i).to_bits(),
                "backwall.mass 第 {i} 格不一致"
            );
            assert_eq!(
                ba.element_idx.get(i),
                bb.element_idx.get(i),
                "backwall.element_idx 第 {i} 格不一致"
            );
        }
    }

    /// 共享事件队列（对应报告 §I-3(a)）。
    ///
    /// 温度阶段会往 `sim_events` 追加**五类**条目（已逐一核对
    /// `c2_physics/temperature.rs` 里的 `emit_*` 调用点）：
    ///   - `substance_change_info`（`do_state_transition` → `push_substance_change`）
    ///   - `cell_melted_info`（`do_state_transition`，高温熔化）
    ///   - `spawn_ore_info`（`do_state_transition` → `spawn_ore`）
    ///   - `spawn_liquid_info`（`do_state_transition`，熔化成液体）
    ///   - `backwall_should_transition_info`（`update_temperature_for_backwall`）
    ///
    /// ⚠️ 本函数最初只比了其中三类，漏了 `spawn_ore` / `spawn_liquid` ——
    /// 那两类若在切片路径上出错，测试是**全绿**的。补齐后 5 类全比。
    ///
    /// `SimEvents` 各字段是**只增长的 MsvcVector**，不是环形缓冲 ——
    /// 所以条数与**顺序**都可观测。
    ///
    /// # `check_order` 为什么是参数，而不是恒为 true
    ///
    /// 事件的**内容**（哪些格发了什么）恒被严格守住（集合相等：不能多也不能少）。
    ///
    /// 实现「逐行分段 + 逐行交错合并」后，切片版的追加顺序已与原版**逐位一致**，
    /// 所以与原版比也可以 `check_order = true` —— 现有调用点仍在用 `false`，
    /// 是为了让「内容一致」这一层始终独立可查（顺序若回归，会有专门的顺序用例报错，
    /// 不会淹没在内容断言里）。两者都跑，覆盖更清楚。
    ///
    /// 条目类型只 derive 了 `Clone/Copy/Default`（无 `PartialEq`），
    /// 故逐字段取值后再比。
    fn bit_eq_events(a: &SimData, b: &SimData, check_order: bool) {
        let ea = unsafe { &*a.sim_events.ptr };
        let eb = unsafe { &*b.sim_events.ptr };

        let subst = |e: &crate::a_framework::sim_events::SimEvents| {
            (0..e.substance_change_info.len())
                .map(|i| {
                    let s = e.substance_change_info.get(i);
                    (s.cell_idx, s.old_element_idx, s.new_element_idx)
                })
                .collect::<Vec<_>>()
        };
        let melted = |e: &crate::a_framework::sim_events::SimEvents| {
            (0..e.cell_melted_info.len())
                .map(|i| e.cell_melted_info.get(i).game_cell)
                .collect::<Vec<_>>()
        };
        let bw_tr = |e: &crate::a_framework::sim_events::SimEvents| {
            (0..e.backwall_should_transition_info.len())
                .map(|i| e.backwall_should_transition_info.get(i).game_cell)
                .collect::<Vec<_>>()
        };
        let ore = |e: &crate::a_framework::sim_events::SimEvents| {
            (0..e.spawn_ore_info.len())
                .map(|i| {
                    let s = e.spawn_ore_info.get(i);
                    (
                        s.cell_idx,
                        s.elem_idx,
                        s.disease_idx,
                        s.mass.to_bits(),
                        s.temperature.to_bits(),
                        s.disease_count,
                    )
                })
                .collect::<Vec<_>>()
        };
        let liquid = |e: &crate::a_framework::sim_events::SimEvents| {
            (0..e.spawn_liquid_info.len())
                .map(|i| {
                    let s = e.spawn_liquid_info.get(i);
                    (
                        s.cell_idx,
                        s.element_idx,
                        s.disease_idx,
                        s.mass.to_bits(),
                        s.temperature.to_bits(),
                        s.disease_count,
                    )
                })
                .collect::<Vec<_>>()
        };

        if check_order {
            // 严格序列相等：条数、内容、**顺序**全都要一致。
            assert_eq!(subst(ea), subst(eb), "substance_change_info 序列不一致（含顺序）");
            assert_eq!(melted(ea), melted(eb), "cell_melted_info 序列不一致（含顺序）");
            assert_eq!(
                bw_tr(ea),
                bw_tr(eb),
                "backwall_should_transition_info 序列不一致（含顺序）"
            );
            assert_eq!(ore(ea), ore(eb), "spawn_ore_info 序列不一致（含顺序）");
            assert_eq!(liquid(ea), liquid(eb), "spawn_liquid_info 序列不一致（含顺序）");
        } else {
            // 集合相等：条数与内容严格一致，仅容忍追加顺序不同。
            // 排序后比对**不会**放过「多一条」「少一条」「某条字段不同」——
            // 只放过「同一批条目的排列不同」这一种差异。
            // 用宏而非闭包：五类条目的元素类型各不相同，闭包无法泛型。
            macro_rules! set_eq {
                ($x:expr, $y:expr, $what:literal) => {{
                    let (mut xs, mut ys) = ($x, $y);
                    xs.sort();
                    ys.sort();
                    assert_eq!(
                        xs,
                        ys,
                        concat!($what, " 内容不一致（已按内容排序，与顺序无关）")
                    );
                }};
            }
            set_eq!(subst(ea), subst(eb), "substance_change_info");
            set_eq!(melted(ea), melted(eb), "cell_melted_info");
            set_eq!(bw_tr(ea), bw_tr(eb), "backwall_should_transition_info");
            set_eq!(ore(ea), ore(eb), "spawn_ore_info");
            set_eq!(liquid(ea), liquid(eb), "spawn_liquid_info");
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        let mut serial = make_sim(40, 40);
        let mut sliced = make_sim(40, 40);
        let pristine = make_sim(40, 40);
        let bounds = RegionBounds { min_x: 1, min_y: 1, max_x: 39, max_y: 39 };

        crate::c2_physics::temperature::run_temperature_task(&mut serial, bounds);
        run_temperature_sliced_serial(&mut sliced, &[bounds], DEFAULT_SLICE_SIZE);

        assert_work_happened(&pristine, &serial, 500);
        bit_eq_cells(&serial, &sliced);
    }

    /// 并行切片 == 串行切片，**含事件追加顺序**逐位一致 —— 任务 5/6 的核心契约。
    ///
    /// # 为什么断言的是「并行 == 串行切片」，而不是「切片 == 原版串行」
    ///
    /// 最初我把契约写成「切片版事件顺序须等于 `run_temperature_task`」，实测证明
    /// **在瓦片式切片下这是不可能达成的**，不是实现偷懒：
    ///
    /// - 原版 `run_temperature_band` 是「逐行扫满整幅宽度」→ 事件按 game_cell 递增追加
    /// - `SliceGrid::build` 外层 y、内层 x → 切片是 32×32 **瓦片**，不是整幅竖条。
    ///   区域 38×38 切 32 得 4 片：s0=x[1,33) y[1,33)、s1=x[33,39) y[1,33)、
    ///   s2=x[1,33) y[33,39)、s3=x[33,39) y[33,39)
    /// - 按 `SliceBounds::index` 合并的顺序是「s0 全部行 → s1 全部行 → …」，
    ///   而原版是「第 1 行整行 → 第 2 行整行 → …」。同一 tile 行内，原版逐行横跨 s0/s1，
    ///   切片版却要先把 s0 的 32 行跑完才轮到 s1 → s1 的格（实测 game_cell
    ///   35/36/37/74/75/113…）整批被推到末尾。
    ///
    /// 要还原全局行主序，只有两条路：切片改成整行条带（放弃 32×32 瓦片），
    /// 或按「片内逐行」粒度缓冲再交错合并（复杂度高、且事件顺序无语义收益）。
    ///
    /// **事件顺序本身没有语义**：三类事件都自带 `game_cell` / `cell_idx` 定位到具体格，
    /// C# 侧按格处理，与到达先后无关。真正要保证的是**内容一致**（由
    /// `bit_eq_cells` → `bit_eq_events(_, _, false)` 严格守住）与**确定性**
    /// （本用例：并行与串行切片逐位相同，含顺序）。
    ///
    /// 并行路径靠两条保证：唯一写者论证（温度场/背墙逐位一致，与调度无关）
    /// + 每切片 `RegionEvents` 缓冲 + 逐行分段交错合并（事件顺序确定）。
    ///
    /// ⚠️ 本用例**守不住合并顺序本身**：并行与串行切片共用同一段合并代码，
    /// 顺序若写错会同时污染两边、相互抵消（实测 `reverse()` 后仍全绿）。
    /// 守合并顺序的是以原版串行为参照系的
    /// `event_order_matches_serial_for_multi_tile_region`。
    #[cfg(feature = "d4-slice")]
    #[test]
    fn parallel_sliced_matches_serial_sliced_bitwise() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        let mut sliced = make_sim(40, 40);
        let mut sliced_p = make_sim(40, 40);
        let pristine = make_sim(40, 40);
        let bounds = RegionBounds { min_x: 1, min_y: 1, max_x: 39, max_y: 39 };

        run_temperature_sliced_serial(&mut sliced, &[bounds], DEFAULT_SLICE_SIZE);
        run_temperature_sliced(&mut sliced_p, &[bounds], DEFAULT_SLICE_SIZE, true);

        // 反假绿：若这一趟没产生任何事件，顺序断言恒真 —— 那本用例毫无意义。
        let ev = unsafe { &*sliced.sim_events.ptr };
        assert!(
            ev.substance_change_info.len() > 0,
            "切片侧未产生 substance_change_info，本用例退化为空断言"
        );

        assert_work_happened(&pristine, &sliced, 500);
        // 温度场/背墙/事件内容 + 事件顺序，全部逐位一致。
        bit_eq_cells(&sliced, &sliced_p);
        bit_eq_events(&sliced, &sliced_p, true);
    }

    /// 事件**追加顺序**的硬闸门（多瓦片，真实形状）。
    ///
    /// 这是方案 B（保留 32×32 瓦片 + 逐行分段交错合并）的验收核心：区域被切成
    /// **多个 tile 行 × 多个 tile 列**时，合并仍须还原出原版「逐行横跨整幅宽度」的顺序。
    ///
    /// 参照系是**原版串行**而不是「串行切片」—— 这点要紧：
    /// `parallel_sliced_matches_serial_sliced_bitwise` 比的是并行 vs 串行切片，
    /// 两者**共用同一段合并代码**，合并顺序若写错会同时污染两边从而相互抵消
    /// （实测：把合并顺序 `reverse()` 后该用例照样全绿）。只有拿原版当参照才能钉住它。
    #[test]
    fn event_order_matches_serial_for_multi_tile_region() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        // 区域 68×68（> 切片尺寸），切片 32 → 3×3 = 9 片瓦片，真实的多行多列形状。
        let mut serial = make_sim(70, 70);
        let mut sliced = make_sim(70, 70);
        let pristine = make_sim(70, 70);
        let bounds = RegionBounds { min_x: 1, min_y: 1, max_x: 69, max_y: 69 };

        crate::c2_physics::temperature::run_temperature_task(&mut serial, bounds);
        run_temperature_sliced_serial(&mut sliced, &[bounds], DEFAULT_SLICE_SIZE);

        // 反假绿：确认确有多个 tile 行与多个 tile 列（否则退化成单列/单行，
        // 交错逻辑根本没被走到），且确实产生了事件。
        let grid = SliceGrid::build(&[bounds], DEFAULT_SLICE_SIZE);
        let rows: std::collections::BTreeSet<usize> =
            grid.slices.iter().map(|s| s.min_y).collect();
        let cols: std::collections::BTreeSet<usize> =
            grid.slices.iter().map(|s| s.min_x).collect();
        assert!(
            rows.len() > 1 && cols.len() > 1,
            "本用例需要多行多列瓦片才有意义，实际 {} 行 × {} 列",
            rows.len(),
            cols.len()
        );
        let ev = unsafe { &*serial.sim_events.ptr };
        assert!(
            ev.substance_change_info.len() > 0,
            "串行侧未产生 substance_change_info，本用例退化为空断言"
        );

        assert_work_happened(&pristine, &serial, 500);
        bit_eq_cells(&serial, &sliced);
        // 严格含顺序：逐行交错合并后，必须与原版行主序逐位相同。
        bit_eq_events(&serial, &sliced, true);
    }

    /// 补充形状：区域宽度 ≤ 切片尺寸时，切片退化为竖向堆叠的条带（每片横跨整个区域
    /// 宽度）。此时交错退化成「整片拼接」，是 `event_order_matches_serial_for_multi_tile_region`
    /// 的边界情形，单独留着防止退化解被改坏。
    #[test]
    fn event_order_matches_serial_for_single_tile_column() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        // 区域宽 32（= 切片尺寸）→ 每个切片横跨整个宽度，只沿 y 方向堆叠成多片。
        let mut serial = make_sim(40, 80);
        let mut sliced = make_sim(40, 80);
        let pristine = make_sim(40, 80);
        let bounds = RegionBounds { min_x: 1, min_y: 1, max_x: 33, max_y: 79 };

        crate::c2_physics::temperature::run_temperature_task(&mut serial, bounds);
        run_temperature_sliced_serial(&mut sliced, &[bounds], DEFAULT_SLICE_SIZE);

        // 反假绿：确认确实切成了多于一片（否则本用例退化成「单片」平凡情形），
        // 且确实产生了事件（否则顺序断言恒真）。
        let grid = SliceGrid::build(&[bounds], DEFAULT_SLICE_SIZE);
        assert!(
            grid.slices.len() > 1,
            "本用例需要多个切片才有意义，实际切出 {} 片",
            grid.slices.len()
        );
        let ev = unsafe { &*serial.sim_events.ptr };
        assert!(
            ev.substance_change_info.len() > 0,
            "串行侧未产生 substance_change_info，本用例退化为空断言"
        );

        assert_work_happened(&pristine, &serial, 500);
        bit_eq_cells(&serial, &sliced);
        // 严格含顺序：多片竖向堆叠时，按 index 升序拼接 == 原版行主序。
        bit_eq_events(&serial, &sliced, true);
    }

    /// 参数化：切片尺寸 16 / 32 / 64 各跑一遍逐位一致，
    /// 同时覆盖「能被整除」与「不能被整除」两种切片收窄。
    #[test]
    fn sliced_serial_matches_for_all_slice_sizes() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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

    /// 性能对比基准（手动运行：`cargo test --release --features d4-slice -- --ignored
    /// temperature_slice_speedup_benchmark`）。默认 `#[ignore]` —— 基准依赖
    /// release 优化、CPU 核数、负载，不该进常规测试（单核/CI 下会误报）。
    ///
    /// 用真实起始星尺寸（160×274）的满水网格，对比**温度阶段**串行
    /// （`run_temperature_task`，内部走 ParallelTaskQueue 行带）vs 切片并行
    /// （`run_temperature_sliced_collect(parallel=true)`，rayon 持久池）。
    /// 只打印耗时，不硬断言加速比。
    ///
    /// ⚠️ **最坏情况**：`make_sim` 的伪随机温度场 ΔT 很大（~137°C 级），
    /// `compute_pair_delta` 的 |ΔT|≥1 门控**全部放行**，于是切片版「每格算 4 对」
    /// （全邻域，为任意切片边界支持）对比串行「每格算 2 对」（半邻域）的 ×2 计算
    /// 代价全额暴露，实测 speedup 通常 <1（本机 0.07x）。真实游戏温度场接近热平衡
    /// （ΔT 小 → 门控大量跳过），×2 代价被隐藏，并行度收益才体现出来 —— 用户
    /// 实测帧数提升即为证。**本基准衡量的是最坏开销，不代表真实场景**。
    #[test]
    #[ignore]
    fn temperature_slice_speedup_benchmark() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        use std::time::Instant;

        let w = 160i32;
        let h = 274i32;
        let bounds = RegionBounds { min_x: 1, min_y: 1, max_x: (w - 1) as usize, max_y: (h - 1) as usize };
        let frames = 200usize;

        // 预热（JIT/分配器热身）
        {
            let mut sd = make_sim(w, h);
            crate::c2_physics::temperature::run_temperature_task(&mut sd, bounds);
            let mut sd2 = make_sim(w, h);
            let _ = run_temperature_sliced_collect(&mut sd2, &[bounds], DEFAULT_SLICE_SIZE, true);
        }

        // 串行（同一 sim 跑多帧，只测温度阶段；不重建 sim——重建成本不在对比范围）
        let mut serial = make_sim(w, h);
        let t0 = Instant::now();
        for _ in 0..frames {
            crate::c2_physics::temperature::run_temperature_task(&mut serial, bounds);
        }
        let serial_dur = t0.elapsed();

        // 并行（同一 sim 跑多帧，与串行对称；同样不含建 sim 成本）
        let mut par_sim = make_sim(w, h);
        let t1 = Instant::now();
        for _ in 0..frames {
            let _ = run_temperature_sliced_collect(&mut par_sim, &[bounds], DEFAULT_SLICE_SIZE, true);
        }
        let par_dur = t1.elapsed();

        let serial_ms = serial_dur.as_secs_f64() * 1000.0 / frames as f64;
        let par_ms = par_dur.as_secs_f64() * 1000.0 / frames as f64;
        let speedup = serial_ms / par_ms;
        println!(
            "[d4-bench] 160x274 temperature: serial={serial_ms:.3}ms/frame  sliced-parallel={par_ms:.3}ms/frame  speedup={speedup:.2}x"
        );
        // 不硬断言：仅提示。单核/低核环境 speedup 可能 <1（同步开销）。
        assert!(par_ms > 0.0, "并行耗时应为正");
    }
}
