# 病菌模块・阶段 2：格子病菌模拟（设计规格，2026-08-05）

## 目标与范围

**本阶段**（对照原版 simDLL，11_msvcrt_ignored.c + SimDLL_Source.c）：
1. `Disease::UpdateCells`（L26646）——相邻两格病菌**扩散**（读 `cells` 稳定缓冲，写 `updatedCells`）；
2. `Disease::PostProcess`（L26424）——逐格病菌**生长/衰减**（温度因子 + 种群因子 + 辐射杀伤 + 小数误差结转 + 清除 + tick 递增）；
3. `Disease::GetDiffusionScale`（L26329）——扩散门控（minDiffusionCount / minDiffusionInfestationTickCount / diffusionScale）；
4. `Disease::CalculateFinalDiseaseCount`（L26268）——异病菌混合时目标病菌判定（供其他模块复用；本阶段 UpdateCells 走 AddDiseaseToCell，PostProcess 不直接用它）。
5. **挂接**：`update_data` 区域循环内，按原版顺序插入（L145069 CopyFrom → L145088 扩散 → L145539 生长）。

**不做**（后续阶段）：辐射病菌贡献（L145106-145464，RadiationSickness hash 0xd49f77d6 → 阶段 4，本阶段仅预留挂接位）；病菌消息/组件（阶段 3）；管道病菌（C# ConduitDiseaseManager 全在 C# 侧，与 simDLL 无关）。

## 原版机制全解

### 1. 调用链与挂接位置（SimDLL_Source.c UpdateData 区域循环）

```text
温度任务 → CopyFrom(L144548) → 气体 → CopyFrom(L144950) → 压力 → 液体
→ CopyFrom(L145069)                          ← 本阶段新增（病菌扩散读 cells 稳定缓冲）
→ 病菌扩散 UpdateCells 循环(L145088)          ← 本阶段新增
→ 辐射病菌贡献(L145106-145464)                ← 阶段 4 预留位
→ 太空真空(L145466)                           ← 已实现（位置吻合）
→ [辐射任务 / RadiationEmitter 组件]           ← 已实现
→ PostProcessCell 循环(L145535)               ← 已实现（post_process_loop）
→ 病菌生长 Disease::PostProcess(L145539)      ← 本阶段新增
```

扩散循环（L145082-145100）：区域 [x_start..x_end)×[y_start..y_end) 内每个 cell：
- 若 `disease[cell] != 0xff || disease[cell+1] != 0xff` → `UpdateCells(dt=0.2, sim, events, cell, cell+1)`（右邻）；
- 若 `disease[cell] != 0xff || disease[cell+width] != 0xff` → `UpdateCells(..., cell, cell+width)`（下邻）。

每对只处理一次（恒为"左→右 / 上→下"方向，参数顺序固定）。区域边界与 PostProcessCell/PostProcess 相同（trimmed 内部格）。

### 2. GetDiffusionScale（L26329）

```
disease_idx = cells.disease_idx[cell]
elem = cells.element_idx[cell]
若 disease_idx 或 elem 越界 → 原版断言；本项目防御返回 0.0
若 count[cell] < elemGrowthInfo[disease].minDiffusionCount[elem] → 0.0
若 tick[cell] < elemGrowthInfo[disease].minDiffusionInfestationTickCount[elem] → 0.0
返回 elemGrowthInfo[disease].diffusionScale[elem]
```

### 3. UpdateCells（L26646-26812）——相邻两格扩散

```
读 cells：elem_a/elem_b、disease_a/disease_b、count_a/count_b
门控：postProcessData[elem_b].state&3 != postProcessData[elem_a].state&3 → return
      （异相态不扩散：液体/气体/固体/真空各自同类扩散）

分支：
1) disease_a == disease_b：
   - 均为 0xff → return
   - i8 = count_b − count_a；source = (i8>0 ? b : a)
   - transfer = GetDiffusionScale(source) × i8 × 0.125
   - AddDiseaseToCell(updated, a, disease_a, +transfer)
   - AddDiseaseToCell(updated, b, disease_b, −transfer)
2) disease_a == 0xff（a 无病，b 有病）：
   - transfer = count_b × GetDiffusionScale(b) × 0.125
   - a ← +transfer（disease_b），b ← −transfer
3) disease_b == 0xff（对称）：
   - transfer = count_a × GetDiffusionScale(a) × 0.125
   - a ← −transfer，b ← +transfer（disease_a）
4) 异真实病菌：
   - 若 count_b×strength[b] < count_a×strength[a] → source=a（强方）
     transfer = count_a × GetDiffusionScale(a) × 0.125，a 失 b 得
   - 否则 source=b（含相等）→ b 失 a 得
```

注意：strength 来自 gDisease 表；transfer 是 `(int)` 截断；AddDiseaseToCell 走阶段 1 的强度混合（updatedCells）。

### 4. PostProcess（L26424-26640）——逐格生长

第一遍（区域每格，跳过 disease==0xff）：

```
temp = updatedCells.temperature[cell]

// A. 温度因子 fVar24（对照 C# Disease.CalculateRangeHalfLife + HalfLifeToGrowthRate）
找到第一个 i ∈ 0..3 使 temp <= range[i]（range = minViable/minGrowth/maxGrowth/maxViable）
  lower = max(i−1, 0)，upper = i；无匹配（temp > maxViable）→ lower=upper=3
若 (lower==1 && i==2) || half_lives[lower]==INF || half_lives[upper]==INF → fVar24 = 1.0
否则 t = (temp − range[lower]) / (range[upper] − range[lower])（分母≤0 → t=0）
  lerped = (1−t)×half_lives[lower] + t×half_lives[upper]
  fVar24 = HalfLifeToGrowthRate(lerped, dt=0.2)   // lerped==0→0，lerped==INF→1

// B. 种群因子 growth（对照 C# ElemGrowthInfo.CalculateDiseaseCountDelta）
count = disease_count[cell]；mass = updatedCells.mass[cell]；elem = element_idx[cell]
min = mass × minCountPerKG[elem]；max = mass × maxCountPerKG[elem]
若 count < min：
  growth = −underPopulationDeathRate[elem] × 0.2
否则若 count > max：
  growth = (HalfLifeToGrowthRate(overPopulationHalfLife[elem], 0.2) − 1) × count
否则：
  growth = (HalfLifeToGrowthRate(populationHalfLife[elem], 0.2) − 1) × count

// C. 汇总
f25 = (count × fVar24 + acc_err[cell] − count) + growth
若 radiationEnabled：f25 −= updatedCells.radiation[cell] × radiationKillRate[disease]
acc_err[cell] = f25 − trunc(f25)          // 小数结转
count[cell] += trunc(f25)
```

第二遍（区域每格，无条件）：

```
若 count[cell] < 1：
  disease_idx=0xff、count=0、tick=0、acc_err=0
tick[cell] = min(tick[cell] + 1, 0xfe)     // 原版怪癖：刚清除的格 tick 置 0 后再 +1 → 1
```

### 5. 辅助公式（C# Klei.AI.Disease 对照）

```csharp
// C# L314-322：HalfLifeToGrowthRate(half_life, dt)
// half_life == 0 → 0；half_life == +INF → 1；否则 powf(2, −dt/half_life)
// C# L348-381：CalculateRangeHalfLife = 上述 A 步的带间插值（含最优带 [minGrowth,maxGrowth] → +INF）
```

simDLL 侧 PostProcess/UpdateCells 的 dt 固定 0.2（UpdateCells 显式传 0.2；PostProcess 内 `× −0.2` 与 powf 指数隐式 0.2）。

## 实现设计

### 模块结构（新建 `simDLL_RS/src/c2_physics/disease.rs`）

- `half_life_to_growth_rate(half_life: f32, dt: f32) -> f32`（0→0、INF→1、powf(2, −dt/hl)）；
- `calculate_range_half_life(temp, range: &RangeInfo, half_lives: &RangeInfo) -> f32`（带间插值 + 最优带 + INF 传播）；
- `get_diffusion_scale(disease: &Disease, sim: &SimData, cell) -> f32`（防御越界返回 0.0）；
- `update_cells(disease: &Disease, sim: &mut SimData, cell_a, cell_b)`（四分支 + 元素相态门控 + AddDiseaseToCell）；
- `run_disease_diffusion_task(sim: &mut SimData, bounds: RegionBounds)`（循环右/下邻 + 门控调用）；
- `run_disease_growth_task(sim: &mut SimData, bounds: RegionBounds)`（PostProcess 两遍）。

数据依赖：gDisease（`globals::G_DISEASE`，锁一次取引用）；elements_table::get_element_post_process_data（相态门控）；CellSOA 字段（element_idx/temperature/mass/disease_idx/disease_count/disease_infestation_tick_count/disease_growth_accumulated_error/radiation）。

### 挂接点（c2_physics/mod.rs update_data 区域循环）

```rust
// 现有：update_liquid_loop
crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data); // 原版 L145069
crate::c2_physics::disease::run_disease_diffusion_task(sim_data, bounds); // 原版 L145088
// 原版 L145106 辐射病菌贡献（RadiationSickness）→ 阶段 4 预留位（注释占位）
// 现有：run_space_vacuum_task / radiation / emitters / post_process_loop
crate::c2_physics::disease::run_disease_growth_task(sim_data, bounds);    // 原版 L145539
```

### 防御语义（原版越界断言 → 本项目跳过/返回 0）

- GetDiffusionScale：disease_idx/elem 越界 → 0.0（不扩散）；minDiffusionCount/tick 向量越界 → 0.0；
- UpdateCells：向量越界/表缺失 → 返回（不 panic）；
- PostProcess：disease_idx 越界 → 跳过该格；ElemGrowthInfo 向量越界 → 跳过该格；
- `fVar24` 插值分母 ≤ 0 → t=0（原版同款）；powf 结果非有限 → 0 防御（原版依赖数据不变量）。

## 测试计划（TDD，全部对照原版公式手算预期值）

1. **HalfLifeToGrowthRate**：0→0、INF→1、1.0@dt=0.2 → 2^(−0.2)=0.8705…、10.0 → 2^(−0.02)；
2. **calculate_range_half_life**：最优带 [minGrowth,maxGrowth] → INF；带间插值；越界 clamp；
3. **GetDiffusionScale**：minDiffusionCount 不足 → 0；tick 不足 → 0；正常 → diffusionScale；
4. **UpdateCells**：同病菌均衡（count 差×0.125×scale 从高到低流）；a 空单向流入；b 空对称；异病菌强方流向弱方（含相等→b 为源）；元素相态门控（液↔气不扩散）；写入 updatedCells 且走 AddDiseaseToCell；
5. **PostProcess**：温度因子四带（低于 minViable 衰减 / 最优带不变 / 高于 maxViable 衰减 / 插值）；种群三分支（min/max per kg 边界）；acc_err 结转（小数）；radiation 杀伤（radiationEnabled 开关）；count<1 清除四字段；tick 递增（含"清除后 tick=1"怪癖）；
6. **update_data 集成**：两格病菌经 update_data 后扩散+生长正确（含 CopyFrom 后 cells 同步）；
7. 回归：全量串行 + release。

## 回归与安全

- 新增 `liquid 后 CopyFrom`：只把 updatedCells 提交到 cells，不改变 updatedCells；现有温度/气体阶段已有同款 CopyFrom，语义一致；
- 扩散只作用于有病菌的邻对，无病菌存档零开销（门控先行）；
- 单星（无病菌表）防御：gDisease 缺失 → 扩散/生长跳过；
- 全量测试串行 + release 构建。

## 自检

1. **占位符**：无"待定/TODO"；阶段 4 预留位明确标注为注释占位。
2. **内部一致**：四函数语义与反编译/C# 逐行对照；挂接点与 L145069/145088/145539 一一对应。
3. **范围**：单实现计划可覆盖（新建 1 模块 + 2 处挂接 + 测试）。
4. **歧义**：dt=0.2 已从 `× −0.2` 与 C# HalfLifeToGrowthRate 交叉确认；扩散方向与区域边界已确认。
