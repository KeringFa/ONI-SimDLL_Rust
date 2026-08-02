# 液滴轨道（轨道 B）严格对照实现设计

**日期**：2026-08-02
**状态**：已批准（用户实测原版"悬崖边缘 = 液滴事件、无实体方格滑落"后确认方向 A）
**类别**：C2 液体物理 · 液滴/虚液轨道

## 1. 背景

用户实测原版：液体水平扩散至悬崖时，边缘产生**液滴事件**（spawnLiquidInfo），不存在实体液体方格滑落；液滴落地才合并回格子（"吸收为一格"）。

我们的 sim 因 `visible_grid` 未分配、`spawn_falling_liquid` 恒 false，液滴轨道从未激活 → 悬崖处质量走格子转移 → 空中继续横漂、落地叠叠高。

## 2. 架构认知：双轨制

- **轨道 A（格子）**：UpdateLiquid/UpdatePressure/PostProcessCell 全在 sim 内，质量始终是格子实体（已实现）。
- **轨道 B（液滴）**：液体流到壁边（下方固体 + 斜下可渗透）时，sim 从源格**扣下一团质量并发 spawnLiquidInfo 事件**，质量暂时离开 sim；C#（原版游戏，不可改）渲染成下落粒子，落地时经 `AddToSim → AddRemoveSubstance → ModifyCell(ReplaceType.None)` 把质量**加回**落点格。

本设计实现轨道 B 的 sim 侧前半段，并修复回注通道的 Add 语义。

## 3. 实现项

### 3.1 分配 visible_grid（前置）
- `SimData.visible_grid`（L8409）：`num_game_cells` 字节，初始全 0xFF（可见兜底，先保证液滴能发；屏幕外多发事件的偏差记录在案）。
- `GameData.visible_grid`（L3263）：同样分配。
- `copy_sim_data_to_game` 增加 visibleGrid 指针交换（对齐原版 `GameData::swapVisibleGrid`，SimDLL_Source.c L127019/L132390）。
- `SetVisibleCells` 消息（3731910273）：C# 反编译未见发送方，暂不实现 handler；如后续定位到格式再补。

### 3.2 update_liquid 左右分支：严格对照 SpawnFallingLiquid 路径
- 门限通过后（原版 L887-909 左 / L987-1007 右）：
  - `IsSolid(cells, below)` 且 `IsLiquidPermeable(updated, 斜下)` → 先调 `spawn_falling_liquid(边缘格, src_elem, f23, src_temp, src_disease_idx, disease_to_transfer)`；
  - 成功 → 跳过 UpdateNeighbourLiquidMass，走成功路径（源格病菌/质量递减、flow 累加）——质量离开格子成为液滴；
  - 失败 → 回退 `update_neighbour_liquid_mass`（现状路径）。
- `spawn_falling_liquid`（11_msvcrt L43490）：headless→false；元素表/温度范围检查；`visibleGrid[gameIdx]==0 且非 debugEditing`→false；事件 cellIdx=边缘格 game 坐标（现有公式正确）。

### 3.3 修复 process_cell_modifications Add（mode 0）语义
- 分派按**被添加元素形态**（原版 L134880-135020）：液体→`AddLiquid`、气体→`AddGas`、固体→`AddSolid`；mass<=0 → 同形态目标减质量/清格。
- `AddLiquid`（L134362-134555）：同元素→合并质量温度（吸收为一格）；异液体→`DisplaceLiquid` 后放置；气体→`DisplaceGas` 失败则与下方液体交换后放置；真空→直接放置；目标固体→四邻找非固体格放置；四邻有同元素液体格→优先合并。
- `AddGas`（L134150-134360）：同元素→合并；异形态→Displace 后放置；失败→四邻找真空/同元素格，否则原位替换（减质量）。
- `AddSolid`（L134555-134789）：同元素→合并；异形态→非固体邻格放置；**垂直位移（DoVerticalDisplacement）推迟**（属排液口/网格砖机制，用户裁决暂不做，记录偏差）。

### 3.4 事件协议
- `SpawnFallingLiquidInfo` 20B 布局已核对一致；`copy_sim_data_to_game` 已 swap `spawn_liquid_info` → C# 消费（Game.cs L1132-1136 → FallingWater）。

## 4. 测试
- 可见门控：全可见/不可见（非 debug）/headless 三态。
- 悬崖场景：下方固体 + 斜下真空 → 发事件、源格扣质量、邻格不变（不再转移）。
- AddLiquid：同元素合并、异液体挤压、真空放置、固体找邻、同元素邻格优先。
- AddGas：同元素合并、真空放置、失败回退。
- 回归：既有 31 个液体测试 + 全量。

## 5. 验收
- `cargo test -- --test-threads=1` 全过；release 编译成功。
- 实测：悬崖倒水 → 边缘出液滴、空中无横漂、落地合并一格、凹坑找平。

## 6. 范围外（记录）
- DoUnstableCheck 系列（仅固体不稳定物，液体不经过，已确认）。
- 排液口/网格砖"液滴转实体方块"（用户裁决暂不管；含 AddSolid 垂直位移）。
- Flood/大块液体移除、SetVisibleCells 消息格式（C# 发送方未定位）。
