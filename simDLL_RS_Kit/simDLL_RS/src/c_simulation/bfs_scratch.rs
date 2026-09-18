//! BFS visited 缓冲复用池（性能内务，行为保持）。
//!
//! 背景：泵选格/质量移除/病菌发射/火山喷发四处 BFS 每次调用都
//! `vec![false; width*height]` 分配并清零一张 visited 表（736×304 网格
//! ≈218KB/次），实际 BFS 只访问几格到几十格。中后期存档每帧 15~80 次
//! 调用 → 每帧 3~16MB 的分配+清零纯属浪费。
//!
//! 方案：thread_local 复用一张 `Vec<u32>` 表 + **世代戳**——
//! 每次 BFS 调用 generation 自增，`visited[i] == generation` 视为已访问。
//! 等价于 bool 表（true=已访问），但**无需每次清零**：旧世代号天然 ≠ 当前世代。
//!
//! 并发安全：全部调用方在阶段 B（组件，串行）与帧消息处理（主线程串行），
//! 无并行（d3 pipeline.rs:105 注释确认）；thread_local 天然线程隔离。
//! ⚠️ 嵌套调用会触发 RefCell already-borrowed panic（fail loud）——当前
//! 调用链无嵌套（find→flood 为同步顺序，remove_mass_from_cell 不调 BFS）；
//! 若未来引入嵌套，此处会立即暴露而不是静默出错。
//!
//! 溢出：u32 世代每次调用 +1，回绕（约 2^32 次）时清零重来（O(N) 一次）。
//! 尺寸变化（切存档/世界）：resize + 清零 + 世代归零（一次性 O(N)）。
//! 队列（VecDeque）不在此池复用——懒分配、实际很小，不值得增加耦合。

use std::cell::RefCell;

struct Scratch {
    /// 世代戳表：`visited[i] == generation` ⟺ 格 i 在当前 BFS 调用中已访问。
    visited: Vec<u32>,
    generation: u32,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch {
        visited: Vec::new(),
        generation: 0,
    });
}

/// 在 `f` 执行期间借出本线程的 visited 表（已推进到新世代）。
///
/// `total`：网格总格数（width×height）。与缓存的容量不符（首帧/切存档）时
/// resize + 清零 + 世代归零。
///
/// 返回 `(visited 切片, 当前世代)`；BFS 内以 `visited[i] == generation`
/// 判定"已访问"，写入用 `visited[i] = generation`。
pub(crate) fn with_visited_scratch<R>(
    total: usize,
    f: impl FnOnce(&mut [u32], u32) -> R,
) -> R {
    SCRATCH.with(|cell| {
        let mut s = cell.borrow_mut();
        if s.visited.len() != total {
            s.visited.clear();
            s.visited.resize(total, 0);
            s.generation = 0;
        }
        s.generation = s.generation.wrapping_add(1);
        if s.generation == 0 {
            // u32 回绕（约 2^32 次调用一次）：清零重来，避免撞上残留旧值。
            s.visited.fill(0);
            s.generation = 1;
        }
        let generation = s.generation;
        f(&mut s.visited, generation)
    })
}
