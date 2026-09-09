# D4：32×32 切片并行（第三条性能优化路径）— 设计规格

日期：2026-08-30
状态：设计已批准，待出实现计划
范围：**仅温度阶段**（第一批次）。气体/辐射/后处理/液体后续分批。

---

## 1. 背景与目标

现有两条并行路线 D1（1 星 1 线程 + 亲和分组）与 D2（rayon 工作窃取）都是**区域级**
并行 —— 工作单元是"一个已发现世界"。实测 Ceres Spaced Out 星群只有 9 个区域，
其中起始星（160×274 = 43,840 格）占全部工作量的近三分之一。结果是大区域独占
一个线程，其余线程早干完在空转，多核利用率上不去。

本方案把工作单元从"区域"细化到 **32×32 切片**：全星群 9 个区域可切出约
**147 个切片**（实测值，见 §10），配合 rayon 工作窃取实现真正的负载均衡。

目标：

1. 单星存档也能吃到并行（D1/D2 现在完全吃不到）
2. 不挑阶段 —— 哪个阶段慢都受益（见 §3.6 统一派发）
3. **结果与原版逐位一致**，不是"在容差内接近"

---

## 2. 约束（用户决策汇总）

| # | 约束 | 来源 |
|---|---|---|
| C1 | 切片基准 32×32（火箭舱内部世界 `ROCKET_INTERIOR_SIZE` 同尺寸） | 用户 |
| C2 | **不仿制**前辈 simSimDLL 的方向游程分解 | 用户 |
| C3 | 气体 6 向扩散；液体受重力影响约 3 向，**必须谨慎对待** | 用户 |
| C4 | `liquid_flow.rs` 大部分是开发期测试代码，后续考虑删除 → **本批次不动** | 用户 |
| C5 | 最低 4 物理核。**不考虑**支持双核 | 用户 |
| C6 | 切片为**实验性**功能，两级门控：编译期 feature 不进 default；运行时开关默认开，后续 C# 适配后改默认关 | 用户 |
| C7 | **用户所有优化都必须与原版可区分** —— "不带 feature 编译即 v1 稳定版"是硬约定 | 用户 |
| C8 | 切片与 D1/D2 的关系 = **统一派发** | 用户 |
| C9 | 不挑瓶颈阶段，走"能切就切"的路线 | 用户 |
| C10 | 超线程下的总线程数控制**暂不考虑**，但保留阈值常量 | 用户 |

---

## 3. 架构设计

### 3.1 两级门控（照搬 `d3-rayon` 既有模式）

| 层级 | 机制 | 默认值 | 说明 |
|---|---|---|---|
| 编译期 | 新目录 `src/d4_slice/` + Cargo feature `d4-slice` | **不进 `default`** | 满足 C7：`cargo build --release` 出来的仍是 v1 稳定版 |
| 运行时 | `RS_SetSliceEnabled(bool)` | **true**（当前） | 满足 C6：后续在接口 mod 适配后改为默认 false |

测试构建命令：

```
cargo build --release --features d3-rayon,d4-slice
```

命名沿用项目既有习惯（目录 `d3_rayon` 装的是 D2 路线，编号与目录名本就不严格对齐），
新目录取 `d4_slice`。

### 3.2 切片生成

```rust
pub struct SliceBounds {
    pub index: usize,                       // 全局序号，用于事件定序合并
    pub min_x: usize, pub min_y: usize,
    pub max_x: usize, pub max_y: usize,     // 排他上界
}
```

- 只在**区域内部**生成，不跨区域 —— 区域之间本就不相邻（间隔 ≥2 格），跨了无意义
- 边界切片自动收窄（160×274 → 最后一行切片为 160×10）
- 切片列表**缓存**，仅在区域集合或网格尺寸变化时重算，避免每帧分配

### 3.3 配对函数拆两步（核心改造）

```rust
/// 纯计算：只读 `sim.cells` 不可变快照，无副作用。
/// 返回 (delta_a, delta_b)；门控不通过返回 None。
///
/// `boundary_ok` 由**调用方**传入 —— 边界门控按方向而异（见 §3.4 表格），
/// 不能统一套用；`|ΔT| >= 1` 与 `pair_allowed(a, b)` 两道数值门控在本函数内部判定。
fn compute_pair_delta(sim: &SimData, a: usize, b: usize, boundary_ok: bool)
    -> Option<(f32, f32)>;

/// 只把 delta 累加进**一个**格子。调用方保证该格子归自己所有。
fn apply_delta(sim: &mut SimData, cell: usize, delta: f32);
```

现有 `update_temperature_pair`（temperature.rs:647）**保留不动**，串行路径继续用它，
内部改为 `compute_pair_delta` + 两次 `apply_delta`。

→ 串行路径行为零变化，现有 585 个测试全部继续有效。

### 3.4 每格固定执行次序（逐位一致的关键）

**为什么能做到逐位一致**：`compute_pair_delta` 与门控函数 `pair_allowed`
（temperature.rs:821）都只读 `sim.cells` 不可变快照 → **delta 值与执行顺序无关**。
唯一与顺序有关的是"往累加器里加"的次序（`u.temperature[cell] += delta` 是读改写）。

串行版（`run_temperature_band`，行优先扫描）对格 X=(r,c) 的写入序列恒为：

| 序 | 来源 | 说明 |
|---|---|---|
| 1 | 格 (r-1,c) 的下邻配对 | X 作为 b |
| 2 | 格 (r,c-1) 的右邻配对 | X 作为 b |
| 3 | 格 (r,c) 的背墙换热 | 只写本格 |
| 4 | 格 (r,c) 的右邻配对 | X 作为 a |
| 5 | 格 (r,c) 的下邻配对 | X 作为 a |
| 6 | 格 (r,c) 的状态转换 | 只读写本格 |

切片版对每个**自己拥有的格**严格执行同样次序：

```
// 计算（顺序无关，只读快照；边界邻居照样算，即使邻居不属于本片）
let d_up    = compute_pair_delta(sim, cell - w, cell, r     < h - 1).map(|(_, b)| b);
let d_left  = compute_pair_delta(sim, cell - 1, cell, c     < w - 1).map(|(_, b)| b);
let d_right = compute_pair_delta(sim, cell, cell + 1, c + 1 < w - 1).map(|(a, _)| a);
let d_down  = compute_pair_delta(sim, cell, cell + w, r + 1 < h - 1).map(|(a, _)| a);

// 应用（顺序严格，复刻串行）
apply(d_up);
apply(d_left);
update_temperature_for_backwall(cell);
apply(d_right);
apply(d_down);
do_state_transition(cell);
```

（上表的 `r`/`c` 为本格行列；源格自身的循环守卫见下方门控表。）

#### ⚠️ 门控条件按方向不同，必须逐个精确复刻

串行代码（`run_temperature_band`）里四个方向的**边界门控写法不一样**，
统一套用会漏掉边缘格。设 X = (r, c)，`w`/`h` 为网格宽高：

| delta | 源格 | 边界门控（必须原样复刻） | 数值门控 |
|---|---|---|---|
| `d_up` | (r-1, c) 的下邻配对 | **`(r-1) + 1 < h-1`** 即 `r < h-1`；且源格过循环守卫 `r-1 < h-1 && c < w-1` | `\|t(X-w) - t(X)\| >= 1` 且 `pair_allowed(X-w, X)` |
| `d_left` | (r, c-1) 的右邻配对 | **`(c-1) + 1 < w-1`** 即 `c < w-1`；且源格过循环守卫 `r < h-1 && c-1 < w-1` | 同上 |
| `d_right` | (r, c) 自己的右邻配对 | **`c + 1 < w-1`**；且本格过循环守卫 `r < h-1 && c < w-1` | 同上 |
| `d_down` | (r, c) 自己的下邻配对 | **`r + 1 < h-1`**；且本格过循环守卫 `r < h-1 && c < w-1` | 同上 |

另外两个函数的门控同样要原样保留：

- `update_temperature_for_backwall`：`cell < cells.mass.len() && cell < backwall.mass.len() && cells.mass[cell] > 0.0 && backwall.mass[cell] > 0.0`
- `do_state_transition`：`cell < updated.mass.len() && updated.mass[cell] > 0.0`，且 `get_element_temperature_data(elem)` 返回 `Some`

**这些门控任何一处不精确，逐位一致测试就会失败** —— 这正是 M3 阶段的价值：
先跑串行切片版，把"逻辑复刻"和"并行安全"分开验证。

边界配对（如 c5–c6）被两侧各计算一次，各应用自己那一半。
**代价：配对计算量 ×2**，但 `|ΔT| ≥ 1` 门控会挡掉绝大多数配对。

已核实为"只读写本格、无跨格数据依赖"的函数（可直接切片）：
- `update_temperature_for_backwall`（temperature.rs:709）→ 只写 `u.temperature[cell]`、`bw.temperature[cell]`
- `do_state_transition`（temperature.rs:416）→ 只读写 `updated.*[cell]`，仅向外发事件

**唯一跨格函数**就是 `update_temperature_pair`，已由本节方案解决。

### 3.5 事件缓冲与定序合并

`do_state_transition` / `update_temperature_for_backwall` 会发事件。

每切片一份 `SimEvents`，按 `(region_index, slice_index)` 定序合并 —— **直接复用**
`c2_physics/region_events.rs` 现有机制，不新造轮子。

### 3.6 与 D1/D2 的组合：统一派发（C8）

所有区域的切片全部打散进同一个池子。D1/D2 退化为**线程分配策略**：

| 模式 | 语义 |
|---|---|
| D1 | 常驻线程 + 亲和绑定，消费切片队列 |
| D2 | rayon 工作窃取，消费切片队列 |

带来的副作用（用户已确认接受）：D1 原本"每线程包干一个星球"的语义不再成立，
太空员舱集群不再需要单独留一个线程。

---

## 4. 数据流

```
sim_data.active_regions
        │
        ▼
  切片缓存（区域集合/尺寸变化时重建）
        │  Vec<SliceBounds>，约 147 项
        ▼
  rayon / 常驻线程 统一派发
        │
        ├─ 每片：遍历自己的格子 → 计算 4 向 delta → 按固定次序应用
        ├─ 每片：自己的 SimEvents 缓冲
        │
        ▼
  屏障
        │
        ▼
  按 (region, slice) 定序合并事件 → 全局 sim_events
```

---

## 5. 错误处理与降级

| 情形 | 处理 |
|---|---|
| 未编译 `d4-slice` feature | 代码不存在，无影响（C7） |
| `RS_SetSliceEnabled(false)` | 完全走现有串行 / D1 / D2 路径 |
| 物理核 `< MIN_SLICE_CORES`（常量，初始 4） | 自动不走切片，回落现有路径 |
| 切片列表为空（区域未就绪） | 走现有路径 |
| 区域数或切片数异常（> 上限） | 走现有路径，不 panic |

原则：**任何异常都退回现有路径，绝不改变原版行为**。

---

## 6. 测试（最贵的部分，不可省）

### 6.1 逐位一致性测试（核心）

同一份 `SimData` 跑两遍，断言 `updated_cells` 全部字段**逐位相等**（不是容差）：

```rust
run_temperature_task(&mut sd_serial, bounds);
run_temperature_sliced(&mut sd_sliced, bounds, 32);
assert_bit_eq!(sd_serial.updated_cells, sd_sliced.updated_cells);
```

必须覆盖：

- 网格能被 32 整除 / 不能整除（尾部切片收窄）
- 单区域 / 多区域
- 切片内部边界格（邻居在同片）
- 切片之间边界格（邻居在邻片）
- 区域边缘格（邻居在区域外，应被钳制跳过）
- 事件顺序：切片版与串行版产生的事件序列应完全一致

### 6.2 参数化

切片尺寸做成参数（32 / 16 / 64），测试跑多档，验证一致性与切片尺寸无关。

### 6.3 现有 585 个测试

必须全部保持通过 —— 串行路径未改动，这是回归基线。

---

## 7. 里程碑

| # | 内容 | 验收 |
|---|---|---|
| M1 | `compute_pair_delta` / `apply_delta` 拆分；`update_temperature_pair` 改为复用二者 | 585 个现有测试全绿，行为零变化 |
| M2 | `SliceBounds` 生成 + 缓存 | 单元测试覆盖整除/不整除/多区域 |
| M3 | `run_temperature_sliced` 串行执行版（先不并行，只跑通切片逻辑） | §6.1 逐位一致测试通过 |
| M4 | 接 rayon 并行派发 | §6.1 在多线程下仍逐位一致 |
| M5 | 事件缓冲 + 定序合并 | 事件序列一致 |
| M6 | 运行时开关 `RS_SetSliceEnabled` + `MIN_SLICE_CORES` 回退 | 开关关闭时行为与现状一致 |
| M7 | 部署到 Local，进游戏实测 | 流动/温度表现与旧版一致 |

**M3 是关键关卡**：先做串行执行版，把"逻辑正确"和"并行安全"两件事分开验证。
并行一上，出错就分不清是切片逻辑错还是调度错。

---

## 8. 明确不做（YAGNI）

- 不做气体 / 辐射 / 后处理 / 液体切片（后续批次）
- 不做死区剔除 —— 已证伪，原版本就按活动区域跳过（`Game.cs:1424`）
- 不做自适应切片尺寸（先固定 32，参数化仅用于测试）
- 不做超线程下的总线程数控制（C10，仅保留常量）
- 不重构 `liquid_flow.rs`（C4）
- 不加分阶段耗时埋点进本批次（另立任务）

---

## 9. 风险

| 风险 | 说明 | 缓解 |
|---|---|---|
| 浮点次序未严格复刻 | 导致逐位不一致 | §3.4 已给出严格次序；M3 的逐位测试直接暴露 |
| 低核数机器负收益 | 配对计算 ×2 可能盖过并行收益 | `MIN_SLICE_CORES = 4` 回退 |
| 阶段 B 串行形成天花板 | 组件阶段串行，加速上限受 Amdahl 限制 | 已知，后续另议 |
| 事件顺序漂移 | 影响游戏侧表现 | §3.5 定序合并 + §6.1 事件序列断言 |

---

## 10. 参考数据

切片数实测（32×32，Ceres Spaced Out 星群 9 个已发现世界）：

| 世界 | 尺寸 | 切片数 |
|---|---|---|
| CeresSpacedOutAsteroid（起始星） | 160×274 | 45 |
| WaterMoonlet | 80×174 | 18 |
| SwampyLandingSite | 128×153 | 20 |
| OilRichWarpTarget | 128×153 | 20 |
| RegolithMoonlet | 160×96 | 15 |
| MooMoonlet | 96×80 | 9 |
| TundraMoonlet | 64×128 | 8 |
| MarshyMoonlet | 64×96 | 6 |
| NiobiumMoonlet | 64×96 | 6 |
| **合计** | | **147** |

（16×16 则为 565 个，作为对比参考。）
