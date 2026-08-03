# 温度模块补全（DoLoadTimeStateTransition + DoPartialHeatTransition + DoPartialMelt + 透气砖/网格砖验证）设计

**日期**：2026-08-03
**状态**：待用户确认
**类别**：C2 物理 · 温度域补全（04_temperature.c 剩余项）

## 1. 背景

温度模块已完成并经实测：DoStateTransition、方格↔方格换热、背墙换热、ElementChunk、建筑换热、
导热板 BuildingToBuilding、DoPressureBreak、DoSublimation、AddGas/AddLiquid/AddSolid 基础版、
CalculateCombinedTemperature。剩余 4 项（用户确认方案 A）：

| # | 项 | 现状 | 位置 |
|---|----|------|------|
| ① | DoLoadTimeStateTransition | 空桩 | 04_temperature.c L7-73 |
| ② | DoPartialHeatTransition | 留桩 | 04_temperature.c L2105-2471 |
| ③ | DoPartialMelt | 仅判定近似（无副作用） | 04_temperature.c L2471-2662 |
| ④ | 透气砖/网格砖换热 | 验证收尾（机制已查清，见 §2） | — |

## 2. 透气砖/网格砖换热机制归属（第 4 项先决结论）

**结论：下方密实方块换热是 simDLL 通用原理（ElementChunk::Update 的 groundTransferScale 机制），
不是 C# 特判。** 证据链：

1. MeshTile / GasPermeableMembrane 建筑配置（MeshTileConfig.cs / GasPermeableMembraneConfig.cs）：
   `doReplaceElement = false` + `setTransparent = true` → **建砖不替换格子元素**，格子里保持建造前内容
   （真空环境下即真空）。
2. C# 侧 `SimTemperatureTransfer` 组件（两个砖都挂）通过 **`SimMessages.AddElementChunk`** 把建筑注册为
   **ElementChunk**（碎片），传 surfaceArea=10 / thickness=0.01 / groundTransferScale=0.0625，
   **不注册 BuildingHeatExchange**。
3. sim 侧 `ElementChunk::Update`（L140092-140202）只做两处换热：
   - 本格（scale 1.0）—— `ExchangeHeatEnergyWithWorld` 对质量 ≤ 0 的格子直接跳过（真空不换热）；
   - 下方格（scale groundTransferScale）—— 仅当下方格元素 `state & 3 == 3`（密实固体）才换热。
4. 由此推出全部观察行为：真空环境上/左/右不换热（Update 从不触碰）；下方液体/气体不换热
   （state&3≠3 门控）；变温板（ThermalBlock 纯 C# 驱动格子温度）接触不到 ElementChunk 记录 → 无法均温；
   导热板中心格（BuildingToBuildingHeatExchange）只对 BuildingHeatExchange 记录换热 → 无法换热。
5. **本项目 ElementChunk 模块（2026-08-03）已逐行镜像该逻辑**（element_chunk.rs `update_element_chunks_inner`
   L458 "下方格为固体 → groundTransferScale 换热" + `exchange_heat_energy_with_world` 质量≤0 跳过）。
   → 第 4 项 **无 sim 代码改动**，验证收尾即可。

## 3. 实现项

### 3.1 ① `do_load_time_state_transition`（加载时相变修正）

位置：temperature.rs 新增 `pub(crate) fn do_load_time_state_transition(sim: &mut SimData, cell: usize)`；
save_load.rs L296 调用点改调该函数；删除 sim_data.rs 的空桩方法。

语义（读 `updated_cells`，逐行镜像 L7-73）：

1. `elem = updated.element_idx[cell]`；`etd = element_temperature_data[elem]`（无效则跳过）。
2. `temp = updated.temperature[cell]`。
3. `low = etd.low_temp`；`high = etd.high_temp`；
   `low_trans = etd.low_temp_transition_idx`；`high_trans = etd.high_temp_transition_idx`。
4. 分支：
   - 若 `temp >= low − 3.0` **或** `low_trans == 0xFFFF`：
     - 若 `high + 3.0 < temp` 且 `high_trans != 0xFFFF`：`temp −= 1.5`，`element = high_trans`。
   - 否则（低于窗口且有低温转换目标）：`temp += 1.5`，`element = low_trans`。
5. 终温 > 10000 → 警告（原版 DebugBreak 断言；项目惯例 warn 后继续）。

测试：低于窗口→低温转换（温度 +1.5/换元素）；高于窗口→高温转换；无转换目标→不动；
恰好边界（low−3 / high+3）→ 不触发。

### 3.2 ② `do_partial_heat_transition`（部分沸腾/分批相变）

位置：temperature.rs 新增 `pub(crate) fn do_partial_heat_transition(sim: &mut SimData, cell: usize, neighbor: usize) -> bool`；
PostProcessCell 液体分支末尾四方向调用点（liquid_flow.rs L2309 留桩位置）改调，顺序
`[上, 左, 右, 下]`（原版 L2705-2721），任一返回 true → 提前 return。

流程（逐行镜像 L2105-2471）：

1. 前置（任一不满足 → false）：
   - `mass[cell] >= 5.0`（`< 5.0` 直接 false，`== 5.0` 放行）；
   - cell / neighbor 元素有效；`cell_elem.high_temp_transition_idx != 0xFFFF`；
   - `neighbor_temp > cell_elem.high_temp + 3`；
   - `neighbor_temp >= neighbor_elem.low_temp + 13`；
   - `cell_temp < cell_elem.high_temp − 3`。
2. `heat = (cell_high + 3 − cell_temp) × cell_elem.tc × 5.0`（所需热量）。
3. `f21 = mass[neighbor] × neighbor_elem.tc`；`f21 × 10 < heat` → false（邻格供不起）。
4. `neighbor_new_temp = (neighbor_temp × f21 − heat) / f21`。
5. `i16 = (int)(disease[cell] × (5.0 / mass[cell]))`。
6. 目标元素 = `cell_elem.high_temp_transition_idx`，按目标 `state & 3` 分支：
   - **气体（1）**：`mass[cell] −= 5.0`、`disease[cell] −= i16`（<1 清病菌）、`temp[neighbor] = neighbor_new_temp`；
     下方格 `below = cell + width`：
     - below 是气体 → `DisplaceGas(below)` 成功 → `below: mass=5.0, temp=cell_high+3, elem=目标` +
       ChangeSubstance(below) + return true；失败 → 恢复（`mass[cell] += 5.0`、`disease += i16`、
       `temp[neighbor] = 原值`）return false（原版此处进崩溃标签，安全镜像为恢复）；
     - below 非气体 → `DisplaceLiquid(cell, cell_elem)` 成功 → `cell: mass=5.0, temp=cell_high+3,
       elem=目标` + ChangeSubstance(cell) + return true；失败 → 恢复 return false。
   - **液体（2）**：`mass[cell] −= 5.0`、`disease −= i16`、`temp[neighbor] = neighbor_new_temp`；
     `DisplaceLiquid(cell, cell_elem)`：
     - 成功 → `SpawnFallingLiquid(cell, 目标, 5.0, cell_high+3, disease_idx[cell], i16, is_debug)`
       成功 return true；失败 → `cell: mass=5.0, temp=cell_high+3, elem=目标` + ChangeSubstance + return true；
     - 失败 → 恢复 return false。
   - 其他 state → 警告 return false。

测试：各前置失败分支；气体目标排到下方格；液体目标液滴生成；液滴失败就地转化；失败恢复
（质量/病菌/邻格温度全部还原）；四方向短路。

### 3.3 ③ `do_partial_melt`（部分熔化，完整版）

位置：从 liquid_flow.rs 移到 temperature.rs 并实现完整版；PostProcessCell 气体分支门控
（liquid_flow.rs L2158-2161）改调 `temperature::do_partial_melt`。

流程（逐行镜像 L2471-2662）：

1. 前置（任一不满足 → false）：
   - `cell`（气体格）`state & 3 <= 1`；`neighbor` `state & 3 == 3`（固体）；
   - `mass[neighbor] > 5.0`；`props[neighbor] & 0x48 == 0`；
   - `cell_temp > neighbor_elem.high_temp + 3`；`neighbor_temp < neighbor_elem.high_temp − 3`；
   - `neighbor_elem.high_temp_transition_idx != 0xFFFF` 且目标元素有效、`state & 3 == 2`（液体）。
2. `heat = (neighbor_high + 3 − neighbor_temp) × 目标.tc × 5.0`。
3. `f17 = mass[cell] × cell_elem.tc`；`(cell_temp − (cell_elem.low_temp + 6.0)) × f17 < heat` → false。
4. `cell_new_temp = (f17 × cell_temp − heat) / f17`。
5. `i14 = (int)((5.0 / mass[neighbor]) × disease[neighbor])`。
6. `SpawnFallingLiquid(cell, 目标, 5.0, neighbor_high+3, disease_idx[neighbor], i14, is_debug)`：
   - 成功：`temp[cell] = cell_new_temp`；`mass[neighbor] −= 5.0`、`disease[neighbor] −= i14`；
     `mass[neighbor] ≤ 1.1754944e-38` → `Evaporate(neighbor)`；return true。
   - 失败：`DisplaceGas(cell, cell_elem)` 失败 → return false；成功 → `cell: elem=目标, mass=5.0,
     temp=neighbor_high+3` + ChangeSubstance(cell)；`mass[neighbor] −= 5.0`、`disease −= i14`、
     ≤1.17e-38 → Evaporate；return true。

门控变化：气体分支 `melt_blocked = a || b || c || d` 现在**执行真实熔化副作用**；
短路语义与原版一致（任一方向熔化成功即停止其余方向，熔化结果阻断气体置换门）。

测试：各前置失败分支；熔化成功（液滴 + 双方温度/质量/病菌变化）；液滴失败就地转化；
neighbor 质量扣尽 → Evaporate；气体分支门控集成。

### 3.4 ④ 透气砖/网格砖换热——验证收尾

无 sim 代码改动。产出：
- 机制归属文档（本规格 §2）。
- 用户游戏内实测清单：真空环境透气砖/网格砖与下方密实方块缓慢换热（0.0625 倍速）；
  上/左/右真空不换热；下方液体/气体不换热；变温板无法均温；导热板中心格无法换热；
  非真空场景（本格有内容物）正常换热对照。

## 4. 结构决策

- `do_load_time_state_transition` / `do_partial_heat_transition` / `do_partial_melt` 全部放
  temperature.rs（镜像 04_temperature.c 归属）；liquid_flow.rs 的 `spawn_falling_liquid` /
  `do_evaporate` 提升 `pub(crate)` 供 temperature.rs 使用。
- save_load.rs 调用点改 `temperature::do_load_time_state_transition`；sim_data.rs 空桩删除。
- PostProcessCell 的 4 方向 partial heat 调用 + 气体分支 melt 门控改调 temperature.rs。

## 5. 非目标

- add_solid 垂直位移（DoVerticalDisplacement，用户裁决暂不做，另案）；
- ElementEmitter 本体（C1 组件，单独立项）；
- 已知偏差照旧登记不处理（病菌表 gDisease 全量、powf alpha、压力循环门控、post_process NaN 防御等）。

## 6. 测试与验收

- 每函数边界/主路径/失败恢复测试（见各节），全量回归 398+ 串行全过；release 构建成功。
- 用户实测：加载存档温度越界格被修正（①）；大质量液体遇极热邻格分批沸腾 5kg（②）；
  极热气体熔化相邻固体 5kg 生成液滴（③）；§3.4 实测清单（④）。
