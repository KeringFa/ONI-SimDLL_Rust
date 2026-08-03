# 升华/offgas（DoSublimation + 液体 offgas）严格对照实现设计

**日期**：2026-08-03
**状态**：待用户确认
**类别**：C2 物理 · PostProcessCell 后处理（自然格升华 + 液体 offgas 补全）

## 1. 背景

液体物理（扩散/坠落/合并/过压/拆除排水）与温度模块（换热/相变简化版）已完善；PostProcessCell 的升华路径此前只有空桩
`do_sublimation_stub`（真空/气体分支调用）+ 液体分支只累积 `accumulated_flow`（L2831 部分），真正的产气未实现。

用户确认范围（方案 A）：**自然格升华 `DoSublimation` + 液体 offgas 补全**。温度相变
`DoPartialHeatTransition`（5kg 分批/沸腾/凝固）不在本步；碎片挥发（C# `Sublimates`，含"过冷"）为 C# 侧行为，已正常，不实现。

机制由**元素属性驱动**（已对照游戏元素表 `StreamingAssets/elements/*.yaml` 核实）：

| 元素 | 升华产物 | sublimateRate | sublimateEfficiency | sublimateProbability | offGasPercentage |
|------|----------|---------------|---------------------|----------------------|------------------|
| 氧石 OxyRock（固体） | 氧气 | 0.4 | 0.5 | 1.0 | - |
| 核废料 Corium（固体） | 核废料气体 | 0.4 | 0.5 | 1.0 | - |
| 毒沙 ToxicSand（固体） | 污染氧 | 0.1 | 0.2 | 0.05 | - |
| 毒泥 ToxicMud（固体） | 污染氧 | 0.1 | 0.2 | 0.05 | - |
| 污染水 DirtyWater（液体） | 污染氧 | - | 1.0 | 0.01 | 0.001 |

漂白石 BleachStone：元素表**无** `sublimateId`（仅 `highTempTransitionTarget: Chlorine`）→ sim 侧
`sublimateIndex = 0xFFFF`，自然方块永不升华；碎片升华完全由 C# `Sublimates` 组件负责（用户实测确认，无需处理）。

实现方案（用户已确认）：**方案一，严格镜像原版，最小侵入**——新写 `do_sublimation` 替换桩，液体分支补全产气，
完全复用现有 `add_mass_and_update_temperature` / `displace_gas` / `push_substance_change` / `clear_cell` / `next_random`，
仅新增 `push_spawn_fx`。

## 2. 源码对照

- `DoSublimation`（SimDLL_Source.c L148591-148880）；
- `PostProcessCell` 真空/气体分支（L149292，真空/气体共享 L149351 LAB_18004ea48 出口调 DoSublimation）；
- 液体 offgas 内联路径（L149677-149834，含 SpawnFX 出口 LAB_18004e400）；
- `SimEvents::SpawnFX`（L149835+，事件结构 SpawnFXInfo 12B，L7251-7253）；
- `AddMassAndUpdateTemperature`（L135754-135850，项目已有等价实现）。

调用链：`update_data`（每 0.2s 子步，sim_thread 按累计时间计算子步数）→ `update_liquids`
→ 每内部格 `post_process_cell`。当前项目挂载粒度与原版一致，无需调整。

## 3. 实现项

### 3.1 `do_sublimation`（替换 `do_sublimation_stub`）

调用点：真空分支、气体分支**末尾**（与原版一致）。**删除**液体分支末尾现有的 stub 调用（原版液体分支不调 DoSublimation，
液体 offgas 是内联路径）。

流程（逐行镜像 L148591-148880）：

1. **入口**：调用格质量必须 `< 1.8`（`<= 1.8 && != 1.8`），否则整函数跳过。
2. **四邻循环**，顺序 `[cell+width, cell+1, cell-1, cell-width]`（上/右/左/下）。对每邻格：
   - 邻格元素 `state & 3 == 3`（固体）**且** `sublimateIndex != 0xFFFF` 时，**才消耗一次 randomSeed**
     （LCG 0x343fd/0x269ec3，复用 `next_random`）；`random <= sublimateProbability` 通过才继续。
   - `f18 = sublimateRate × 0.2`；`f21 = min(f18, 砖质量)`。
3. **部分转移分支**（`f18 <= 砖质量 − f21`，即砖质量 ≥ 2×f18）：
   - 病菌 `i15 = (f21 / 砖质量) × 砖病菌数`（整数截断）。
   - **入口元素快照**：调用格元素在函数入口读一次（sVar4），后续所有邻格比较都用快照，不随前面邻格换元素更新。
   - 快照 == sublimateIndex（同种气体目标）：`f20 = f21 × sublimateEfficiency`；`room = 1.8 − 目标质量`；
     `room < f20` 时按比例缩放 `f21`（重算 `i15`，`f20 = room`）；`add_mass_and_update_temperature(目标, f20, 砖温度, 砖病菌idx, i15)`。
   - 快照 != sublimateIndex（异种/真空目标）：
     - 调用格入口状态为真空（`state&3==0`）→ 直接走换元素段；
     - 否则 `displace_gas(目标)`；失败且本调用尚无先前成功邻格（bVar16==false）→ 跳过该邻格；
     - 换元素段：目标元素 = sublimateIndex、质量 += `f21 × sublimateEfficiency`、温度 = 砖温度、
       病菌idx = 砖病菌idx、病菌数 = i15、`ChangeSubstance(目标)` 事件。
   - 两路径共同落到扣减段：砖质量 −= f21；砖病菌 −= i15（<1 → ClearDisease）；`accumulated_flow[砖] += f21`；
     `SpawnFX(砖, sublimateFX, 方向角)`；砖质量 ≤ 1e-9 → `Evaporate(砖)`。
   - **原版怪癖（逐行镜像）**：换元素段 DisplaceGas 失败但 bVar16==true（本调用先前邻格已成功）时，
     仍落到扣减段——砖扣质量、播放 FX、累积 flow，但目标格未产气（质量凭空减少）。此怪癖属 sim 侧元素方格升华机制，
     与碎片（C#）无关；仅在两个不同升华产物的固体砖邻接同一气体格且挤气失败时触发，原版自然地图极少见。
4. **整格消耗分支**（不满足部分转移条件）：砖元素 = sublimateIndex、砖质量 = `砖质量 × sublimateEfficiency`、
   `ChangeSubstance(砖)` + `SpawnFX(砖, sublimateFX, 0.0)`；不改调用格、不处理温度/病菌（照抄原版）。

方向角常量（镜像，不做换算）：邻格偏移 `+width → 180.0`、`+1 → 90.0`、`−1 → −90.0`、`−width → 0.0`。

### 3.2 液体 offgas 补全（L149677-149834）

保留现有已实现部分不动：概率门（random 在 `props&1` 检查**前**消耗）、目标（cell+width）质量 <1.8、
目标真空/气体、`f24 = min(offGasPercentage × 质量, 1.0)`、cell−width 同元素且更重时用其质量重算、
剩余 ≥0.01 判定、异种气体先 `displace_gas`、`room = 1.8 − 目标质量` 缩放、`accumulated_flow[cell] += f24`。

补三件事：

1. **主路径产气**（剩余 ≥0.01）：液体格扣 `f24` 质量（病菌按 `f24 / 原质量` 比例）；目标格（cell+width）：
   - 真空 → 换元素 = sublimateIndex + `ChangeSubstance` 事件；
   - 异种气体 → 已在前置 `displace_gas`（失败 return，保留）；
   - 同种气体 → 质量合并（`add_mass_and_update_temperature`，温度加权）；
   - 病菌合并判定（bVar9，在产气前由目标格与液体格病菌重新计算）：目标无菌（idx==0xFF）或同菌种 → 合并病菌，
     否则不转移病菌。
2. **耗尽路径**（剩余 <0.01）：液体格就地转气体（剩余质量全部转化，含病菌按比例）；若 cell−width 同元素更重
   （bVar9）且排放量 `f24 > 剩余质量`，从 cell−width 液体格补足差额（该格扣质量、本格得气体）。
3. **`SpawnFX(液体格, sublimateFX, 0.0)`**——两条路径共用一个出口。

注：反编译中产气调用为匿名 lambda（`<>::operator()`，L149810/149830），内联体不可见；按可观测行为
（源格扣质量/病菌、目标格得气体/温度加权/病菌合并）结合既有 `add_mass_and_update_temperature` 模式实现，
与游戏实测对齐。

### 3.3 `push_spawn_fx` 事件辅助

新增 `push_spawn_fx(sim, sim_cell, fx_id, rotation)`：sim→game 坐标转换（同 `push_substance_change`），
推 `SpawnFXInfo { cell_idx: i32, fx_id: i32, rotation: f32 }`（12B，L7251-7253，C# 直接读）。
`fx_id = sublimateFX`（元素表字段），`rotation` = 上述方向角常量。

## 4. 测试

单测（LIB_TESTS_LOCK 串行，`cargo test -p simdll_rs -- --test-threads=1`）：

- 同种气体合并 + 1.8 上限缩放（含缩放后病菌重算）；
- 异种气体 DisplaceGas 成功换元素 + ChangeSubstance 事件；
- DisplaceGas 失败：bVar16==false 跳过（砖无损）；bVar16==true 镜像怪癖（扣质量+FX、目标不产气）；
- 真空目标换元素（质量 += f21×eff、温度/病菌拷贝）；
- 病菌按比例转移与 <1 清空；
- 整格消耗分支：砖就地转元素、质量×eff、不改调用格；
- 四邻顺序（上→右→左→下）与入口元素快照语义；
- 概率门随机消耗时机（仅固体+sublimateIndex 有效时消耗）；
- 液体主路径：扣质量+产气+病菌合并（同菌/无菌/异菌）；
- 液体耗尽路径：就地转气体 + cell−width 补足差额；
- SpawnFX 事件字段（cell_idx/fx_id/rotation）；
- 回归：全量测试 + Release 构建成功。

## 5. 验收

- 全量测试通过 + Release 构建成功。
- 用户实测：氧石自然格持续出氧（自身质量减少、目标格 1.8kg 封顶、满后暂停、异种气体被挤走）；
  污染水缓慢冒污染氧（1% 概率/子步、0.1% 质量/次）；漂白石方块不升华；升华动画（FX）正常出现。

## 6. 范围外（记录）

- `DoPartialHeatTransition`（温度相变：沸腾/凝固/凝结，5kg 分批）——下一步单独做；
- 碎片升华（C# `Sublimates`，含"过冷"）——C# 已正常，不实现；
- 漂白石方块升华——元素表无 `sublimateId`，天然不触发；
- 神秘版本/早期稳定版——不再参考，当前项目进度领先。
