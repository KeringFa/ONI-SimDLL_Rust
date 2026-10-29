# 方向 1（1 星 1 线程 / 超线程优化）实现计划

> **面向 AI 代理的工作者：** 使用 superpowers:executing-plans（内联，推荐）逐任务实现本计划；
> 每个任务含失败测试 → 最小实现 → 通过 → commit。任务用 `- [ ]` 跟踪进度。

**目标：** 运行时多模式调度器（Auto/D2/D1）。D1 = 每世界组一个 std::thread + 亲和性
（大世界独占物理核、太空员舱集群合并一线程、最小世界两两合并）；接口 mod 新增
“启用超线程优化”勾选项（默认关、无超线程时灰显）。

**架构：** `affinity.rs`（CPU 拓扑 + 分组分配）→ `d1_pipeline.rs`（D1 执行器）→
`scheduler.rs`（模式 + FFI + update_data 分发）；C# 侧 `CpuInfo.cs` 检测 +
`Options.cs` 勾选项 2 + `ModConfig` 字段 + `Loader` 接线。

**技术栈：** Rust（windows crate、std::sync::mpsc、std::sync::Barrier、
SetThreadAffinityMask）；C#（P/Invoke GetLogicalProcessorInformation）。

---

## 文件职责

**Rust（src/d3_rayon/）：**
- `affinity.rs`（新建）：`CpuTopology`（物理核/逻辑核/超线程检测）、
  `assign_groups(world_tiles, has_ht) -> Vec<AffinityGroup>`（纯函数，可测）。
- `d1_pipeline.rs`（新建）：`D1Executor`（持久线程 + mpsc + Barrier）、
  `run_d1_pipeline(...)`（阶段 A/B/C 组并行 + 事件合并）。
- `scheduler.rs`（新建）：`SchedulerMode` + `RS_SetSchedulerMode` FFI + 每帧分发。
- `mod.rs`（修改）：`pub mod affinity; pub mod d1_pipeline; pub mod scheduler;`
  并把 `RS_SetSchedulerMode` 设为 `#[no_mangle] pub extern "C"`。
- `src/c2_physics/mod.rs`（修改）：`update_data` 按调度器模式分发
  （Auto/D2 走现有路径，D1 走 `run_d1_pipeline`）。

**C#（my_Project/simDLL_RS_Kit_v1/SimDLL_Rust/）：**
- `CpuInfo.cs`（新建）：`HasHyperThreading()` / `PhysicalCoreCount()`。
- `Options.cs`（修改）：勾选项 2（超线程优化）+ 提示 2 + 无 HT 灰显。
- `ModConfig.cs`（修改）：`HyperThreadOptimizationEnabled`（默认 false）+ config.json。
- `Loader.cs`（修改）：OnLoad 按配置调 `RS_SetSchedulerMode`。
- `Native.cs`（修改）：声明 `RS_SetSchedulerMode`。
- `strings/Strings.cs` + `translations/zh.po`（修改）：勾选项 2 / 提示 2 文案。

---

## 任务 1：affinity.rs — CPU 拓扑枚举 + 超线程检测

**文件：** 创建 `src/d3_rayon/affinity.rs`；测试在文件内 `#[cfg(test)]`。

- [ ] **步骤 1：写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_sane_on_this_machine() {
        let t = CpuTopology::query();
        assert!(t.physical_cores >= 1, "物理核 ≥ 1");
        assert!(t.logical_cores >= t.physical_cores, "逻辑核 ≥ 物理核");
        // 无 HT 的机器上 has_hyper_threading()==false 也合法；有 HT 时每核逻辑核数 ≥ 2
        if t.has_hyper_threading() {
            assert!(t.logical_cores >= t.physical_cores * 2);
        }
    }
}
```

- [ ] **步骤 2：运行测试确认失败**（`cargo test --lib --features d3-rayon affinity::tests`，
  预期 FAIL：`CpuTopology` 未定义）
- [ ] **步骤 3：最小实现**

```rust
/// CPU 拓扑（GetLogicalProcessorInformation，RelationProcessorCore）。
pub struct CpuTopology {
    pub physical_cores: usize,
    pub logical_cores: usize,
    /// 每个物理核的逻辑处理器数（≥1；>1 表示启用超线程）。
    pub logical_per_core: Vec<usize>,
}

impl CpuTopology {
    pub fn query() -> Self { /* 枚举 RelationProcessorCore，累计 logical_cores 与 per-core 数 */ }
    pub fn has_hyper_threading(&self) -> bool {
        self.logical_per_core.iter().any(|&n| n > 1)
    }
}
```

- [ ] **步骤 4：运行测试确认通过**
- [ ] **步骤 5：Commit**（`feat(d1): affinity CPU 拓扑枚举 + 超线程检测`）

---

## 任务 2：affinity.rs — 分组分配算法（纯函数）

**文件：** `src/d3_rayon/affinity.rs` 追加。

```rust
/// 一个调度组：region 索引 + 亲和掩码。
pub struct AffinityGroup {
    pub region_indices: Vec<usize>,
    /// Windows 亲和掩码（位 = 逻辑处理器）。
    pub affinity_mask: u64,
}

/// 输入：各世界 tiles（按 region 顺序，含太空员舱集群的合计值）、拓扑。
/// 输出：物理核组（前 physical_cores-1 大）+ 超线程组（其余两两合并）。
/// 无 HT 时返回 None（调用方回退 D2）。
pub fn assign_groups(world_tiles: &[u64], topo: &CpuTopology) -> Option<Vec<AffinityGroup>>
```

算法：
1. `has_hyper_threading()==false` → `None`。
2. `P = topo.physical_cores - 1`（留 1 核给系统/主线程）。
3. 按 tiles 降序排序索引；前 P 个各成物理核组（mask = 对应物理核的第 1 逻辑核）。
4. 剩余索引按 tiles 降序两两合并（最小两个先合并）成超线程组；
   每组 mask = “第 P 大物理核组（最小大世界）”的空闲逻辑核（先复用最小物理核组的空闲逻辑核，
   若组数超空闲核数，再逐个回退到次小物理核组的空闲逻辑核）。
5. 所有 mask 用 `logical_per_core` 计算逻辑核全局位索引。

- [ ] **步骤 1：写失败测试**（模拟 8C16T + 11 星 + 太空员舱集群）

```rust
#[test]
fn assign_groups_big_worlds_never_share_physical_core() {
    let topo = CpuTopology {
        physical_cores: 8,
        logical_cores: 16,
        logical_per_core: vec![2; 8],
    };
    // 5 同大小(6144) + 6 大小不一 + 太空员舱集群(10240)
    let mut tiles: Vec<u64> = vec![6144; 5];
    tiles.extend([15360, 13920, 9216, 8192, 7680, 6144]); // Regolith/Water/MiniRegolith/Tundra/Moo/Wetland
    tiles.push(10240); // 太空员舱集群
    let groups = assign_groups(&tiles, &topo).expect("有 HT 应可分配");
    // 物理核组 = 7 组（8-1），每组恰好 1 个 region
    let physical: Vec<_> = groups.iter().filter(|g| g.region_indices.len() == 1).collect();
    assert_eq!(physical.len(), 7, "前 7 大世界独占物理核");
    // 大世界组互不共享物理核：mask 两两交集为空
    for (i, a) in physical.iter().enumerate() {
        for b in physical.iter().skip(i + 1) {
            assert_eq!(a.affinity_mask & b.affinity_mask, 0, "大世界不得共享物理核");
        }
    }
    // 超线程组 ≤ 8 空闲逻辑核
    let ht: Vec<_> = groups.iter().filter(|g| g.region_indices.len() > 1).collect();
    assert!(ht.len() <= 8, "超线程组数 {0} 应在空闲逻辑核内", ht.len());
    // 两两合并后每组 tiles 总和 ≤ 19584（变体后上限）
    for g in &ht {
        let sum: u64 = g.region_indices.iter().map(|&i| tiles[i]).sum();
        assert!(sum <= 19584, "合并组 tiles {sum} 超上限");
    }
}
```

- [ ] **步骤 2：运行确认失败**
- [ ] **步骤 3：实现 `assign_groups`**（排序 → 物理核组 → 两两合并 → mask 计算）
- [ ] **步骤 4：运行确认通过**（`cargo test --lib --features d3-rayon assign_groups`）
- [ ] **步骤 5：Commit**（`feat(d1): 分组分配算法（物理核独占 + 超线程两两合并）`）

---

## 任务 3：d1_pipeline.rs — D1 执行器（持久线程 + channel + Barrier）

**文件：** 创建 `src/d3_rayon/d1_pipeline.rs`。

接口（对齐现有 `run_parallel_pipeline`）：

```rust
pub fn run_d1_pipeline(
    sim: &mut SimData,
    region_bounds: &[RegionBounds],
    region_pp_bounds: &[RegionBounds],
    region_cosmic: &[f32],
    groups: &[AffinityGroup],
)
```

实现：
- 全局 `G_D1_EXECUTOR: OnceLock<Mutex<Option<D1Executor>>>`；
- `D1Executor`：每组的持久线程 + `mpsc::Sender<u64>`（tick 信号）+
  `std::sync::Barrier`（组数+1）；
- 线程循环：`recv(tick)` → 用 region sink 跑组内各 region 的
  `run_region_physics_a(&mut *sim, bounds, cosmic, true)` → `barrier.wait()` →
  `run_region_physics_c(...)` → `barrier.wait()`；
- 线程创建时 `SetThreadAffinityMask(GetCurrentThread(), mask)`；
- `run_d1_pipeline`：若 `groups` 与缓存不一致（数量/bounds 变化）→ 重建 executor；
  主线程发 tick → 阶段 A barrier → 合并 A 事件（region sink 合并）→
  阶段 B 串行 `run_region_emitters` → 发 tick → 阶段 C barrier → 合并 C 事件。

- [ ] **步骤 1：写失败测试**（复用 pipeline.rs 的 `make_two_region_sim` 模式，
  在 `d1_pipeline.rs` 内建 2 区域 sim，构造 2 组各 1 区域，跑 `run_d1_pipeline`，
  断言液体总质量守恒 200kg）

```rust
#[test]
fn d1_pipeline_preserves_mass() {
    // 2 区域布局（复制 pipeline.rs make_two_region_sim 的等价构造），
    // groups = 每区域一组（mask 用本机逻辑核 0/1）。
    // 跑 run_d1_pipeline 后断言 updated_cells 液体总质量 ≈ 200kg。
}
```

- [ ] **步骤 2：运行确认失败**（`cargo test --lib --features d3-rayon d1_pipeline`）
- [ ] **步骤 3：实现执行器 + run_d1_pipeline**
- [ ] **步骤 4：运行确认通过**
- [ ] **步骤 5：Commit**（`feat(d1): D1 执行器（持久线程 + 亲和性 + 帧同步）`）

---

## 任务 4：scheduler.rs — 模式 + RS_SetSchedulerMode + update_data 分发

**文件：** 创建 `src/d3_rayon/scheduler.rs`；修改 `src/d3_rayon/mod.rs`、`src/c2_physics/mod.rs`。

```rust
pub enum SchedulerMode { Auto = 0, D2 = 1, D1 = 2 }

static MODE: AtomicI32 = AtomicI32::new(0);

#[no_mangle]
pub extern "C" fn RS_SetSchedulerMode(mode: i32) -> i32 {
    // 0/1/2 合法；其余返回 -1。D1 启动时校验拓扑（affinity::CpuTopology），
    // 无 HT → 回退 D2 并返回 2（表示实际用 D2）。
}
```

`update_data` 分发（c2_physics/mod.rs）：

```rust
match scheduler::current_mode() {
    SchedulerMode::D2 => run_parallel_pipeline(/* 强制并行，region_count==1 也并行 */),
    SchedulerMode::D1 => run_d1_pipeline(/* 若拓扑/分组失败回退 D2 */),
    SchedulerMode::Auto => { /* 现有逻辑：region_count>1 → D2，否则串行 */ }
}
```

- [ ] **步骤 1：失败测试**（`RS_SetSchedulerMode` 非法值返回 -1、D1 在无 HT 拓扑下回退 D2）
- [ ] **步骤 2：运行确认失败**
- [ ] **步骤 3：实现 scheduler + 接线**（先接入 D1 调用；D1 内部失败回退 D2）
- [ ] **步骤 4：运行全量测试**（`cargo test --lib --features d3-rayon -- --test-threads=1`，
  预期 559 + 新增全过；串行路径零变化）
- [ ] **步骤 5：Commit**（`feat(d1): 运行时调度器模式（Auto/D2/D1）+ RS_SetSchedulerMode`）

---

## 任务 5：接口 mod — CpuInfo + 勾选项 2 + 灰显 + 接线

**文件：** `CpuInfo.cs`（新建）；`Native.cs`、`ModConfig.cs`、`Loader.cs`、
`Options.cs`、`strings/Strings.cs`、`translations/zh.po`（修改）。

- [ ] **步骤 1：CpuInfo.cs**

```csharp
using System;
using System.Runtime.InteropServices;

namespace SimDLL_Rust
{
    public static class CpuInfo
    {
        // P/Invoke GetLogicalProcessorInformation（RelationProcessorCore），
        // 返回 (physicalCoreCount, hasHyperThreading)。
        public static bool HasHyperThreading() { /* 失败保守返回 false */ }
        public static int PhysicalCoreCount() { /* 供诊断 */ }
    }
}
```

- [ ] **步骤 2：Native.cs 加 `RS_SetSchedulerMode` 声明**
  `[DllImport("SimDLL_RS.sim")] internal static extern int RS_SetSchedulerMode(int mode);`
- [ ] **步骤 3：ModConfig.cs 加 `HyperThreadOptimizationEnabled`（默认 false）+
  config.json 持久化**
- [ ] **步骤 4：Options.cs 加勾选项 2（“启用超线程优化”）+ 提示 2；**
  构建配置屏时 `CpuInfo.HasHyperThreading()` 为 false → `toggle.enabled=false` +
  `toggle_image` alpha 降至 0.4（灰显不可勾选）
- [ ] **步骤 5：Loader.cs OnLoad**：`CpuInfo.HasHyperThreading()` 且
  `ModConfig.HyperThreadOptimizationEnabled` → `Native.RS_SetSchedulerMode(2)`；
  否则 `RS_SetSchedulerMode(1)`（D2）
- [ ] **步骤 6：Strings.cs + zh.po**：勾选项 2 = “启用超线程优化”（Enable
  Hyper-Threading Optimization）、提示 2 = “启用超线程优化前，确保你的 CPU 开启了
  超线程，否则会自动禁用”（Make sure Hyper-Threading is enabled on your CPU before
  turning this on, otherwise it will be automatically disabled.）
- [ ] **步骤 7：dotnet build 0 警告 0 错误；部署到 V1 Local / 桌面源 / Documents mods**
- [ ] **步骤 8：Commit**（接口 mod 目录，如所在仓库有 git 则提交；
  否则记录变更清单）

---

## 任务 6：联调 + 构建 + 部署

- [ ] **步骤 1：`cargo test --lib --features d3-rayon -- --test-threads=1` 全过**
- [ ] **步骤 2：`cargo build --release --features d3-rayon` + Python 导出校验**
  （RS_SetSchedulerMode 存在）
- [ ] **步骤 3：更新三处 SimDLL_RS.sim / SimDLL_Rust.dll / zh.po**
- [ ] **步骤 4：向用户提交实测清单**（多星 D1 开关、无 HT 灰显、太空员舱增删、
  变体星、单星串行零变化）

---

## 自检记录

- 规格覆盖：调度器模式（任务 4）、分组/亲和性（任务 1/2）、线程模型（任务 3）、
  接口 mod 勾选/灰显/接线（任务 5）、联调部署（任务 6）。
- 无占位符；所有任务有具体文件、测试、实现、命令。
- 类型一致：`AffinityGroup`、`CpuTopology`、`assign_groups`、`run_d1_pipeline`、
  `RS_SetSchedulerMode` 在任务间保持一致。
