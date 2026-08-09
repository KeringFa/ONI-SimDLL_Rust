# d3_rayon 技术参考（提炼自桌面版旧 d3 线）

> 来源：`C:\Users\KeringFa\Desktop\simDLL_RS_Kit\simDLL_RS`（旧 d3 线，T1-T5 归档 +
> 未提交 A 组审查改动）。本文只提炼**设计思路与机制**，作为 v1 重新实现 d3_rayon
> 的依据；**不照抄代码**——旧线建立在旧底层上（含已修复的崩溃根因），直接搬用会
> 带回旧问题。v1 已修复的底层项（★1-★4、worldgen、clean_up 等）是 d3 的前提。

## 1. 总体架构

d3 的并行化目标：**多星（多活动区域）时把各世界的网格物理分给不同物理核心**，
单星/单活动区域永远走原版串行路径（行为零变化）。采用"三段式 + 门控"：

```text
update_data
├─ should_parallelize(region_count)?
│    ├─ false（单区域/单星）→ 串行：for region { A → B → C }（原版逐帧行为）
│    └─ true（多区域）→ 并行：
│         realign RegionRng（每区域独立种子/方向）
│         阶段 A：网格物理并行（每区域组 1 个 rayon job）
│         合并 A 事件（按区域序）
│         阶段 B：组件串行（注册表全局共享，按区域序）
│         阶段 C：收尾并行（PostProcess + 病菌生长）
│         合并 C 事件
```

阶段划分（与原版区域串行顺序一致）：

- **阶段 A（网格物理）**：温度 → CopyFrom → 气体压力/位移 → CopyFrom → 液体流动/位移
  → CopyFrom → 病菌扩散 → 太空真空 → 辐射。
- **阶段 B（组件）**：RadiationEmitter / DiseaseEmitter（旧线只含这两者；v1 应按 ★4
  的完整注册顺序：Consumer → Emitter → RadiationEmitter → Chunk → BuildingHeatExchange
  → BuildingToBuilding → DiseaseEmitter）。组件注册表全局共享、不可并行，事件直接写全局。
- **阶段 C（收尾）**：PostProcessCell → 病菌生长。

## 2. 核心技术点

### 2.1 D1 门控 `should_parallelize(region_count)`

- `region_count > 1` 才启用并行；单星存档、或 DLC 但只发现 1 个星球 → false，走原串行
  路径，行为与原版逐帧一致（零变化）。
- 纯调度决策，无存档标志位。
- 旧线曾因排查崩溃临时 `return false` 禁用并行（二分排查）；v1 实现时应保持"门控可
  一键禁用"的调试手段，但**根因修复后必须恢复**，不能像旧线一样留在禁用态。

### 2.2 D6 线程池（rayon ThreadPool）

- 线程数 = `max(1, physical_cores − RESERVE_CORES)`，`RESERVE_CORES = 1`（预留给游戏
  主线程/系统；8C16T → 7 个 rayon 线程，8 个超线程留给系统后台）。
- `physical_cores()`：Windows `GetLogicalProcessorInformation` 按
  `RelationProcessorCore` 数**物理核**（8C16T → 8、4C8T → 4）；API 失败回退
  `available_parallelism`（逻辑核）。
- `OnceLock` 懒构建一次并缓存；单区域路径不触碰线程池。
- rayon 天然提供**工作窃取**（work-stealing）：job 数 > 线程数时排队，无超额订阅、
  无嵌套池争抢。

### 2.3 T6.3 LPT 面积装箱 `bin_pack_region_groups`

- 输入：区域边界（含 `cell_count`）；输出：区域索引分组 `Vec<Vec<usize>>`。
- 按实际面积（cell_count）**从大到小**贪心放入当前累计最轻的组（LPT）。
- 组数 = `min(区域数, 线程数)`；每组累计面积尽量接近理想值（总面积/组数），
  任一组不超 `max(理想, 最大单世界)`。
- 组内区域**串行**执行（1 个 job 处理一组）；组数 ≤ 线程数时任务自然排队。
- 目的：把小世界（含太空员舱）整合进不超过主星的组，避免每个小世界独占线程，
  也避免 11 线程跑在 8 物理核上的超线程争抢。

### 2.4 T2 区域局部 CopyFrom（`copy_updated_cells_to_cells_region`）

- 并行路径下只复制区域矩形内的格子（含 **halo** 边界 1 格，保证跨区域读安全；
  边界处 clamp 到网格边缘），不做全网格拷贝。
- 动机：网格拷贝是 memory-bound，并行无收益且引入调度抖动；串行路径仍用全网格
  `copy_updated_cells_to_cells`（与原版一致）。
- 注意：区域之间用真空/边界 buffer 隔离，halo 落在区域内真空 buffer 中，跨区域
  读安全——这是"世界之间互不干扰"的网格层前提。

### 2.5 T3 RegionEvents 区域事件收集器

- 并行路径下每个区域任务写**自己的**事件缓冲（线程局部 sink），scope 结束后按
  区域序合并进全局 `sim.sim_events`——否则并行任务竞争全局事件且跨区域顺序不确定。
- 机制：线程局部 `REGION_EVENTS_SINK: Cell<Option<*mut RegionEvents>>`；区域任务在
  `with_region_sink(&mut region_events[i], || …)` 内运行；物理 emit helper 检查 sink：
  有则写区域缓冲，无则写全局（串行路径/组件路径行为不变）。
- 覆盖 8 类事件：substance_change / spawn_liquid / spawn_ore / unstable_cell /
  cell_melted / spawn_fx / world_damage / backwall_should_transition。
- 组件事件（阶段 B 串行）不经过收集器，直接写全局。
- 合并顺序 = 字段声明顺序，与原版区域串行 push 顺序逐字节一致；合并后清空本区域
  缓冲（阶段 A/C 各合并一次，防重复追加）。

### 2.6 T4 RegionRng 每区域独立随机（方案 A）

- 原版区域串行：全局 LCG（`sim.random_seed`）按区域 0→1→… 顺序消耗，
  `sim.displacement_direction` 每格交替反转。d3 并行路径下跨区域消耗顺序天然不确定，
  因此每区域独立**持久**种子/方向：
  - 随机流自洽、逐帧确定，世界间互不干扰（统计等价，与原版全局序列不同）。
  - 串行路径（无区域 RNG）回退 `sim.random_seed` / `sim.displacement_direction`，行为零变化。
- 机制：线程局部 `REGION_RNG: Cell<Option<*mut RegionRng>>`；`next_random` /
  `next_displacement_direction` 检查线程局部指针：有则用区域状态，无则用全局。
- 持久化：`REGION_RNG_STATE`（静态，sim 线程独占写）按 active_regions 对齐：
  - 区域 key（min/max 偏移矩形）不变 → 保留原种子/方向；
  - 新增/变化区域 → `base_seed + i × 0x9E3779B9` 派生，方向初始 -1（与原版一致）。
  - **并行 scope 期间不得 realign**（区域任务持有指向静态 Vec 缓冲的裸指针）。

### 2.7 T5 三段式并行的具体结构

```text
pool.scope(|s| { for group in groups { s.spawn(组内逐区域串行) } })   // 阶段 A
for ev in region_events { ev.merge_into(sim) }                       // 合并 A 事件
for i in regions { run_region_emitters(sim, bounds[i]) }             // 阶段 B（串行）
pool.scope(|s| { for group in groups { s.spawn(组内逐区域串行) } })   // 阶段 C
for ev in region_events { ev.merge_into(sim) }                       // 合并 C 事件
```

- 每个区域 job 用 `with_region_rng(rng_ptr, || with_region_sink(events_ptr, || …))`
  包裹，保证随机/事件路由正确。
- 温度任务在并行路径**内联整区**执行（不经全局 ParallelTaskQueue），避免嵌套并行争抢。
- 阶段间 CopyFrom 用区域局部版本（T2）。

## 3. 线程方向（用户决策）

### 3.1 方向 1：每世界一条独立线程（实验性，暂不做）

- 无论多少世界（太空员舱也不例外），每个世界单独开一条线程。
- 优点：低延迟（无调度等待、无装箱粒度损失）。
- 缺点：对核心少的玩家不利（世界数 > 物理核时线程切换/超线程争抢）。
- 定位：**实验性功能，玩家可选开启**。v1 现阶段不实现，留作后续可插拔开关
  （可与方向 2 共存：方向 2 是默认，方向 1 是可选覆盖）。
- **接入预留（2026-08-09 已落地）**：调度层抽象在 `d3_rayon::pipeline`——
  方向 1 实现时新增第二个 Cargo feature（如 `d3-single-thread`）或运行时开关，
  把"区域组 → job"的映射从当前 LPT 装箱（`bin_pack_region_groups`）替换为
  "每区域一个 job（含太空员舱）"；`run_parallel_pipeline` 的阶段 A/B/C 结构与
  事件/随机路由（RegionEvents/RegionRng）完全复用，仅分组策略不同。
  两个 feature 互斥（`d3-rayon` 与 `d3-single-thread` 二选一）。

### 3.2 方向 2：按物理核开线程 + 工作窃取（主方向）

- 线程数 = `physical_cores − reserve(1)`，rayon scope 提交 job 数 = 区域组数
  （LPT 装箱），rayon 自行工作窃取。
- 延迟比方向 1 高一点点，几乎感知不到；对核心少的玩家更公平（任务排队、无超额订阅）。
- 桌面版已实现（D6 线程池 + LPT 装箱 + 三段式），但被临时禁用；v1 按本文重新实现。

## 4. v1 融合点（与旧线差异，实现时必须处理）

1. **目录命名**：v1 是 `c_simulation`（旧线是 `c1_simulation`）。
2. **组件阶段**：旧线阶段 B 只有 RadiationEmitter + DiseaseEmitter；v1 按 ★4 的完整
   注册顺序（Consumer → Emitter → RadiationEmitter → Chunk → BuildingHeatExchange →
   BuildingToBuilding → DiseaseEmitter）在阶段 B 串行执行，且组件已接收 bounds 过滤。
3. **串行路径**：v1 已是"单循环内联组件"结构（★4）；d3 应保持串行路径不变，只新增
   并行分支（旧线用 run_region_physics_a/emitters/c 三函数拆分，v1 可复用此拆分思路）。
4. **区域 CopyFrom 位置**：v1 串行路径的 CopyFrom 是全局的；并行路径用区域局部版本。
5. **辐射遮挡修复**：旧线 T1-T5 附带修过辐射遮挡，v1 需核对是否已含（radiation.rs）。

## 5. 模块化设计（Cargo feature + cfg，文件夹级开关）

目标：**把 d3 相关文件夹移走（或关闭编译开关），编译出的就是当前 v1 稳定版**。
专业术语：**Cargo features（特性开关）+ 条件编译（`#[cfg]`）**。

做法：

```toml
[dependencies]
rayon = { version = "1", optional = true }

[features]
default = []
d3-rayon = ["dep:rayon"]
```

```rust
// lib.rs
#[cfg(feature = "d3-rayon")]
pub mod d3_rayon;          // 整个 src/d3_rayon/ 目录（含 region_events/region_rng 子模块）
```

效果：

- 默认 `cargo build --release` → 稳定版（不编译 d3 代码，无 rayon 依赖）。
- `cargo build --release --features d3-rayon` → d3 版。
- `#[cfg]` 关闭时不加载 `src/d3_rayon/` 文件，物理移走该文件夹也能编译通过。
- 物理代码里的 d3 接线（mod.rs 并行分支、物理 emit 的 sink 路由等）必须用
  `#[cfg(feature = "d3-rayon")]` 包裹，或把全部 d3 专属逻辑收进 d3_rayon 目录，
  通过接口（trait/回调）与主代码解耦——**目标是主代码零 d3 残留**。

方向 1 后续可加第二个 feature（如 `d3-single-thread`）或运行时开关，二者互斥/可选。

## 6. 旧线教训与风险

- 旧线失败不是 rayon 本身的问题，而是底层框架同步/组件分派/区域边界等未修好
  （世界生成错位、揭雾崩溃）。v1 已修完这些，是 d3 的正确基础。
- 实现顺序：先确保串行路径全绿（v1 现状），再新增并行分支；并行分支默认关闭
  （feature 门控），逐步打开验证。
- 并行正确性三要素：每区域独立随机（T4）、独立事件缓冲（T3）、区域局部快照
  （T2）+ 世界间网格隔离（halo/真空 buffer）。
- 测试：区域分组、门控、线程池、RNG realign、事件合并都要有单测；多星集成行为
  需用户实测（并行 vs 串行逐帧一致性）。

## 7. 实现计划轮廓（供 writing-plans 细化）

1. Cargo feature + cfg 模块化骨架（d3_rayon 目录、rayon optional、lib.rs 门控）。
2. D1 门控 + D6 线程池（物理核探测、reserve）。
3. T6.3 LPT 装箱（移植思路，v1 测试）。
4. T2 区域局部 CopyFrom（halo）。
5. T3 RegionEvents + 物理 emit 路由改造（8 类事件）。
6. T4 RegionRng（key 对齐、派生、realign 时机）。
7. T5 三段式接线（阶段 A/B/C，组件按 ★4 顺序），并行分支 feature 门控。
8. 单测 + 多星实测（并行 vs 串行逐帧一致性、帧率/卡顿数据）。
9. 方向 1（实验性）留接口，暂不实现。
