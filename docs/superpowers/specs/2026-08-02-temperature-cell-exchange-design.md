# 方格↔方格温度换热（UpdateTemperature 任务循环）严格对照实现设计

**日期**：2026-08-02
**状态**：已获批（用户确认含背墙换热、缓冲语义严格忠实原版：每子步 CopyFrom）
**类别**：C2 格子级物理 · 方格↔方格温度换热 / 背墙换热 / 逐格相变

## 1. 背景

温度模块方案 A（建筑温度链路）已完成并实测通过；管道换热漏洞（dest_temp 句柄索引错位）已修复。
当前确认**方格之间不发生任何换热**（液体/气体/砖块两两之间）——即 C2 格子级温度演化缺失。

本设计严格对照原版 `SimDLL_Source.c` 实现：
- `SimUpdateTemperatureTask::InternalDoTask`（L144205-144310）——逐格循环（背墙换热 + 右/下邻换热 + DoStateTransition）；
- `UpdateTemperature`（L133737-134004）——两格换热；
- `UpdateTemperatureForBackwallExchange`（L134004-134079）——格子内容物 ↔ 背墙层；
- `CalculateTemperatureExchange` / `CalculateTemperatureExchange_precise`（L133079-133170）——能量守恒换热核心；
- `SimBase::UpdateData` 的缓冲同步点（L144415 开头 CopyFrom、L144548 温度后 CopyFrom）。

## 2. 架构认知

### 2.1 背墙换热是什么

- 背墙换热 = **该格内容物（气体/液体/固体元素）↔ 该格背墙层元素**（`BackwallSOA`：element_idx/mass/temperature，独立于格子与建筑两层）。
- Drywall（干板墙）、变温板（ThermalBlock）等是 `ObjectLayer.Backwall` 类建筑，热数据在背墙层，**不占普通建筑层**；背墙换热**不涉及建筑句柄、不与其他建筑直接换热**——与用户理解一致。
- 与建筑↔格子换热（BuildingHeatExchange::Update，方案 A 已实现）是**两条独立路径**：建筑换热只看格子内容（mass>0），从不看背墙层。

### 2.2 缓冲语义（用户强制要求：严格忠实原版，换热速率无差异）

原版 `UpdateData`（L144323-145434）关键同步点：
1. L144415：`CopyFrom(cells, updatedCells)` —— 每子步开头；
2. 温度任务（L144430-144548）读 `cells`、写 `updatedCells` 增量；
3. L144548：`CopyFrom(cells, updatedCells)` —— 温度之后；
4. 压力（L144560-144904）→ L144904 CopyFrom + ConsolidateEvents → 液体流（L144937）→ L145069 CopyFrom → ... → L145434 `UpdateComponents`。

本模块集成（只动温度相关，液体/压力阶段保持现有已测结构）：

```
update_data 开头：
  1. copy_updated_cells_to_cells(sim_data)          // L144415
  2. run_temperature_task(sim_data)                 // 读 cells、写 updatedCells（含背墙/邻格/相变）
  3. copy_updated_cells_to_cells(sim_data)          // L144548
  4. 现有液体 → 消费者 → 压力 → 建筑换热（顺序不变）
```

- 换热速率与原版一致：每子步温度基于"上一子步结果"，多子步/卡顿帧不会重复施加相同 delta；
- 单子步测试（fill_both 同值）中 opening CopyFrom 为恒等操作，现有液体/压力测试不受影响；
- sim_thread 帧末同步保留（与开头 CopyFrom 共同保证帧间一致，冗余但无害）。

### 2.3 门控（原版任务循环 L144205-144310）

每格（区域内部格，整图 = active region，行为等价）依次：

1. **背墙换热**：`cells.mass[cell] > 0 且 backwall.mass[cell] > 0` → `UpdateTemperatureForBackwallExchange(cell)`；
2. **右邻换热**：`x < maxX−1` 且 `|T[cell] − T[cell+1]| ≥ 1.0`，双方元素 ETD 有效、`ETD.TC > 0`、
   `(state_cell | state_right) & 0x10 == 0`（0x10 = **TemperatureInsulated**，C# `Element.State` L15-23 已确认）→ `UpdateTemperature(cell, cell+1)`；
3. **下邻换热**：`y < maxY−1` 且 `|T[cell] − T[cell+width]| ≥ 1.0`，同元素门控 → `UpdateTemperature(cell, cell+width)`；
4. **DoStateTransition(cell)**（任务 4 已实现，直接复用；作用于 updatedCells）。

## 3. 实现项

### 3.1 `CalculateTemperatureExchange_precise`（L133110-133170）

输入：`(rate=0.25, k, dt=0.2, T_a, HC_a, T_b, HC_b)`（double 精度计算，f32 输出）。

```
equilibrium = (T_a×HC_a + T_b×HC_b) / (HC_a + HC_b)      // 加权均衡
candidates:
  heat_candidate_1 = (T_b − T_a) × k × dt
  heat_candidate_2 = (T_b − T_a) × 0.25 × HC_a
  heat_candidate_3 = (T_b − T_a) × 0.25 × HC_b
heat = min(三值)                                          // 能量守恒取小
new_T_a = heat/HC_a + T_a，钳制不越过 equilibrium
new_T_b = −heat/HC_b + T_b，钳制不越过 equilibrium
返回 (new_T_b, new_T_a, heat_from_a = (T_a − new_T_a)×HC_a)
```

能量守恒：同取 min(三值)，a 失去 = b 得到。

### 3.2 `UpdateTemperature(cell_a, cell_b)`（L133737-134004）

读 `cells`（稳定缓冲），写 `updatedCells.temperature` 增量 + [1,10000] 钳制：

1. 断言（温度≤0 需 mass==0、双方温度 ∈ [0,10000]），按项目惯例转日志 + 继续；
2. `ETD_a / ETD_b`；`state_a = ETD_a.state & 3`、`state_b = ETD_b.state & 3`；
3. `k_a = insulation_a² × 1.53787e-05 × ETD_a.TC`；`k_b` 同理；
4. 合并：`insulation_a²×1.53787e-05 < 1.0 || insulation_b²×1.53787e-05 < 1.0` → `k = min(k_a, k_b)`；
   否则 `k = sqrt(k_a × k_b)`（原版 logf+logf+expf = 几何平均；实现时核对量级）；
5. 表面倍率：`mult_a = ETD_a 按 state_b 取 (gas/liquid/solid)SurfaceAreaMultiplier`（state 0 落 liquid 倍率，
   真空已被 TC>0 门控排除）；`mult_b` 同理；`k_total = k × mult_a × mult_b`；
6. `HC_a = cells.mass[cell_a] × ETD_a.SHC`；`HC_b` 同理（HC≤0 防御性跳过 + 日志）；
7. 顺序：`T_cell ≤ T_neighbor` → `precise(0.25, k_total, 0.2, T_cell, HC_cell, T_neighbor, HC_neighbor)`；
   否则 swapped 调用后交换返回的新温度（返回元组 = (new_T_第二参, new_T_第一参)）；
8. 写回：`updated[cell_a] += new_T_cell − cells[cell_a]`；`updated[cell_b] += new_T_neighbor − cells[cell_b]`；
   两格各自钳制 [1, 10000]。

### 3.3 `UpdateTemperatureForBackwallExchange(cell)`（L134004-134079）

1. 门控：`cells.element[cell]` 与 `backwall.element[cell]` 均非真空/void（`sim.vacuum_element_idx` / `sim.void_element_idx`）；
2. `k_cell = insulation²×1.53787e-05 × ETD_cell.TC`；与 `ETD_backwall.TC` 按 3.2 规则合并（min / 几何平均）；
3. 表面倍率同 3.2；
4. `CalculateTemperatureExchange(0.01, k×mult_cell×mult_back, 0.2, cell_T, cell_HC, back_T, back_HC)`
   （**因子 0.01**，比邻格换热弱；用 wrapper 自动按温度排序）；
5. 写回：`updatedCells.temperature[cell] += new_cell_T − cells.temperature[cell]`；
   `backwall.temperature[cell] = new_back_T`（背墙单缓冲**直接写**）；
6. 相变：`new_back_T` 越 `ETD_back.lowTemp − 3` 或 `highTemp + 3` → `backwallShouldTransitionInfo.push(BackwallElementChangedInfo{gameCell})`。

### 3.4 温度任务循环 `run_temperature_task`（单线程等价 L144205-144310）

遍历内部格（rows 1..h−1、cols 1..w−1），按 2.3 门控执行背墙换热、右/下邻换热、DoStateTransition。
放置于 `c2_physics/temperature.rs`；由 `update_data` 按 2.2 顺序调用。

## 4. 测试

单测（沿用项目 lib test 惯例，LIB_TESTS_LOCK 串行）：

- 门控：|ΔT|<1 不换热；真空（TC=0）跳过；TemperatureInsulated(0x10) 跳过；
- 合并：insulation 值 <1 → min；≥1 → 几何平均；
- 表面倍率：gas/liquid/solid 倍率选择（双方元素 state 组合）；
- 能量守恒：两格向均衡收敛，总能量不变；0.25 单步不越过加权均衡；
- 钳制：[1,10000]；
- 背墙换热：因子 0.01、背墙温度直接写、相变事件（backwallShouldTransitionInfo）、真空/void 跳过；
- 缓冲语义：多子步序列（调 update_data 两次）温度基于上一子步结果（opening CopyFrom 生效）；
- DoStateTransition 联动：换热致跨相变 → 元素切换；
- 回归：全量 `cargo test -- --test-threads=1` + Release 构建。

## 5. 验收

- 全量测试通过 + Release 构建成功。
- 用户实测：相邻液体/气体/液体↔砖块温度逐渐均衡（ΔT≥1 才换热，温差大时明显）；真空格仍不换热；
  背墙（Drywall）与该格内容物换热（0.01 因子，慢于邻格）；建筑↔格子与背墙路径互不干扰。

## 6. 范围外（记录）

- UpdateData 多线程温度任务分发（我们单线程整图遍历，行为等价）；
- iterateDirection 翻转（原版 L144325，作用于压力/液体方向，温度任务不受影响）；
- 升华/持续蒸发、ElementEmitter 本体；
- 已知偏差沿用：无 Region 概念（整图 = active region）；断言按项目惯例转 tracing 日志 + 继续。
