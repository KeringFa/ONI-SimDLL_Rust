# 建筑温度链路（方案 A）严格对照实现设计

**日期**：2026-08-02
**状态**：已获批（用户确认方案 A 与内部子模块顺序）
**类别**：C1 建筑温度 · ModifyBuildingEnergy / BuildingHeatExchange::Update / 过热事件（含 DoStateTransition 依赖）

## 1. 背景

管道子系统（水珠/气珠换热）已全量恢复，但 `BuildingTemperatureManager` 只打通了"注册 → 回调 → 入网"链路：

- `modify_building_energy_messages`（16B）已收集、无人消费 → 建筑产热（水泵 SelfHeat 2kW 等）不升温 → 管道换热"空转"（有数学、无真实温差来源）。
- `BuildingHeatExchange::Update`（建筑↔格子换热主体）未实现 → 建筑温度不随环境变化。
- 过热/熔化事件容器已建、GDU 回传已接，但无人推数据。

本设计严格对照原版 `SimDLL_Source.c` 实现建筑温度链路；**真空格绝对隔热**（mass=0/SHC=0/TC=0，两种换热天然跳过）已由源码核实。

## 2. 架构认知

### 2.1 真空格（源码核实）

- 建筑↔格子换热 `BuildingHeatExchange::Update`（L137362）：每格显式闸门 `if (0.0 < mass)`，真空格 mass=0 直接跳过。
- 方格↔方格 `UpdateTemperature`（L133737）：热容 = `mass×SHC`，导率 = `insulation²×1.53787e-05×TC`（insulation 为每格绝缘字节，`cells+0x88`），再乘状态表面倍率（ETD +0x1c/+0x20/+0x24，按对方元素 state&3 选择）；真空 TC=0 → 换热恒 0。函数开头断言"Invalid temperature T=%f, M=%f"：温度≤0 的格必须 mass==0。
- 结论：真空是元素（mass=0、SHC=0、TC=0），绝对隔热，无特殊分支。

### 2.2 相关建筑类型归属（本轮不实现，仅记录）

- 导热板（ContactConductivePipeBridge）：走独立组件 `BuildingToBuildingHeatExchange::Update`（L138358），接触列表由 C# 注册控制。属后续子模块。
- 变温板（ThermalBlock）：**无 sim 特殊逻辑**。C# `ThermalBlockConfig` 用 `OverrideExtents(对角四格)` 把 1×1 扩为 3×3，sim 侧仍走标准 AddBuildingHeatExchange（min/max 遍历）。真空格依旧不换热，中间真空不影响对角格。

### 2.3 调度时机（源码核实）

- `ProcessFrame`（L116490-116558）：`adds → ProcessBuildingHeatExchangeMessages（modifies → modifyBuildingEnergyMessages）→ removes`。
- `SimBase::UpdateData` 每子步（dt=0.2s）末尾 L145434：`SimData::UpdateComponents(0.2, region)` → `BuildingHeatExchange::Update`。当前代码等价挂载点：`c2_physics::update_data` 末尾（无 Region 概念，遍历全部记录 ≈ 全图 active region，行为等价）。

## 3. 实现项

### 3.1 记录扩展：BuildingTemperatureRecord 派生字段

对照原版 `BuildingHeatExchangeData`（44B，L7539 附近）与 `InitializeFromMessageData`（L137088-137186）。`BuildingTemperatureRecord`（`building_temperature.rs`）在 Add/Modify 时按消息全量重算派生字段：

| 字段 | 计算 | 原版位置 |
|------|------|---------|
| `total_heat_capacity` | `SHC(elem)×mass` | L137130（`gETD[+0xc]×mass`）|
| `per_cell_heat_capacity` | `total / ((maxX-minX)×(maxY-minY))` | L137132/137143 |
| `thermal_conductivity`（记录值） | `元素TC×消息TC` | L137133/137145 |
| `high_temp` | `gETD[elem].+0x2c` | L137134/137146 |
| `sim_min_x/y` | `minX+1, minY+1` | L137138-137139 |
| `sim_max_x/y` | `maxX+1, maxY+1` | L137140-137141 |

错误处理（原版断言，Rust 侧按项目惯例 logging + 跳过/继续）：
- `(maxX-minX)×(maxY-minY) < 1` → "Size zero building passed into BuildingHeatExchange"（L137121-137126）。
- 消息温度 ∉ [0,10000] → "Invalid temperature passed into BuildingHeatExchange"（L137127-137134）。
- `elem_idx` 越界 → invalid_parameter（L137113-137115）。

### 3.2 ModifyBuildingEnergy 消费

消息：`{handle i32, deltaKJ f32, minTemperature f32, maxTemperature f32}` = 16B（L5941-5946）。

位置：`frame_processor::process_building_heat_exchange_messages` 的 modifies 之后、removes 之前（原版 L116518 调用 `ProcessBuildingHeatExchangeMessages`，内部 L115484-115540 消费）。

算法（严格对照 L115484-115540）：

```
handle 版本校验（index<0x100000、version 匹配、记录有效），无效跳过
f = record.temperature
clamp_low  = min(f, msg.minTemperature)
clamp_high = max(f, msg.maxTemperature)
if clamp_low >= 0.0 && clamp_high <= 10000.0:
    f = f + msg.deltaKJ / ((sim_max_x - sim_min_x) × (sim_max_y - sim_min_y) × per_cell_heat_capacity)
    f = clamp(f, clamp_low, clamp_high)
    if f <= 0.0 || f >= 10000.0:
        warning 日志 "Invalid final temperature in ProcessBuildingHeatExchangeMessages"
        （原版 4 条日志：Invalid final temperature / delta/hc/ft / msg dkj / building %i…）
        不写回
    else:
        record.temperature = f
```

**注意**：分母是 `per_cell_heat_capacity`（记录 @0x10），不是消息 mass。物理上 `(simMax.x-simMin.x)×(simMax.y-simMin.y)×perCellHC = 面积×(SHC×mass/面积) = SHC×mass`，即 ΔT = ΔQ/(SHC×mass)。此前管道审查报告写 mass 有误，以源码为准。

### 3.3 BuildingHeatExchange::Update（建筑↔格子换热）

位置：`c2_physics::update_data` 末尾（等价 L145434 组件阶段；每子步 dt=0.2）。

算法（严格对照 L137362-137633）：

1. 预热 temperatureInfo 输出（长度 = items.len()，空槽 -1.0）。
2. 逐条有效记录：
   - 闸门：`per_cell_heat_capacity > 0.0 && temperature >= 0.0`（L137399-137400）。
   - `cell_count = (sim_max_y - sim_min_y) × (sim_max_x - sim_min_x)`（L137401）。
   - `min_seen = max_seen = record.temperature`。
   - 遍历 `sim_min..sim_max` 每格（**读 updated_cells**）：
     - `mass[cell] <= 0` → 跳过（真空/空格，L137413）。
     - `etd = gETD[elem_idx[cell]]`（SHC@0xc、TC@0x10）；`state = updated.state[cell]`。
     - `cell_hc = mass × SHC`。
     - 换热系数 `f24 = (cellT < bldgT) ? perCellHC : cellHC`（L137435-137437）。
     - `k = f24 × (cellT − bldgT) × elemTC × recordTC × insulation² × 7.68935e-08 × 0.2 × buildingTemperatureScale`（L137438-137440；`fVar22 = updatedCells.insulation[cell]`，`7.68935e-08 = 1.53787e-05×0.005`）。
     - `low = min(bldgT, cellT)`，`high = max(bldgT, cellT)`。
     - `newCellT = cellT − k/cellHC`；`newBldgT = k/perCellHC + bldgT`；均钳制 [low, high]（L137441-137450）。
     - 若 `(newBldgT − newCellT)×(bldgT − cellT) < 0`（交叉）→ 能量守恒均衡（L137451-137458）：`k = 1/(perCellHC+cellHC)`；`balanced = (perCellHC×bldgT + cellHC×cellT)×k`，钳制 [low, high]；`k = (balanced − bldgT)×perCellHC`。
     - 校验 `newCellT ∈ [low, high]` 与 `[0, 10000]`，否则 crash 报告（L137459-137473，Rust 侧 tracing::error + 继续）。
     - `updated.temperature[cell] = newCellT`（L137474-137477）。
     - `DoStateTransition(sim, events, cell, etd)`（L137478）。
     - `Σk += k`；更新 min_seen/max_seen。
   - 建筑终温（L137486-137495）：`T = Σk/(cell_count×perCellHC) + bldgT`，钳制 [min_seen, max_seen]，再 `+ 0.2 × operating_kilowatts/(cell_count×perCellHC)`。
   - 校验 `T ∈ [0, 10000]`，否则 crash 报告（L137496-137501）。
   - 事件（L137502-137537）：
     - `overheat_temperature <= T` → `buildingOverheatInfo.push(MeltedInfo{handle})`（+0xc0）。
     - `overheat_temperature <= oldT && T < overheat_temperature` → `buildingNoLongerOverheatedInfo.push(handle)`（+0xe0）。
     - `high_temp <= T` → `buildingMeltedInfo.push(handle)`（+0xa0）。
   - `record.temperature = T`；`temperatureInfo[handle_index] = {handle, T}`（L137538-137545）。

事件容器偏移核实：SimEvents L9362-9368 = `elementChunkMeltedInfo(+0x80) / buildingMeltedInfo(+0xa0) / buildingOverheatInfo(+0xc0) / buildingNoLongerOverheatedInfo(+0xe0) / cellMeltedInfo(+0x100)`；GameData/GDU 镜像已接好（copy_sim_data_to_game swap + save_load GDU 填充），只差推数据。

### 3.4 DoStateTransition（方案 A 的必要依赖）

位置：每格换热后调用（L137478），作用于 updated_cells。语义（严格对照 L133261-133737）：

```
fn do_state_transition(sim, events, cell, etd) -> bool:
    if mass[cell] <= 0: return false
    T = updated.temperature[cell]

    if T >= etd.low_temp - 3.0 || etd.low_temp_transition_idx == 0xFFFF:
        # 高温侧（沸腾/熔化）
        if T <= etd.high_temp + 3.0 || etd.high_temp_transition_idx == 0xFFFF: return false
        temperature = max(0.0, T - 1.5)
        element_idx[cell] = etd.high_temp_transition_idx
        if etd.high_temp_transition_ore_idx != 0xFFFF:
            ore_mass = etd.high_temp_transition_ore_mass_conversion × mass
            if ore_mass > 0.001 && SpawnOre(sim, events, cell, ore_idx, ore_mass, T, disease_idx,
                                            (int)(disease_count × conversion), true):
                mass -= ore_mass; ModifyDiseaseCount(-(int)(disease_count × conversion))
        if updated_cells.properties[cell] & 0x40:   # 原版读 updatedCells+0x68（properties）位 0x40
            cell_melted_info.push(CellMeltedInfo{game_cell}); timers |= 0x1F
        ChangeSubstance(cell); return true

    # 低温侧（冻结/冷凝），条件 T < etd.low_temp - 3.0 且 low_idx != 0xFFFF
    temperature = max(0.0, T + 1.5)
    if etd.low_temp_transition_ore_idx != 0xFFFF:
        ore_mass = etd.low_temp_transition_ore_mass_conversion × mass
        if ore_mass > 0.001 && SpawnOre(...): mass -= ore_mass; ModifyDiseaseCount(-换算)
    transition_etd = gETD[etd.low_temp_transition_idx]
    if transition_etd.state & 3 == 3:   # 目标为固态
        if mass / transition_etd.max_mass <= 0.8:
            if SpawnOre(sim, events, cell, transition_idx, mass, T, disease_idx, disease_count, debug_editing):
                ClearCell(cell); ChangeSubstance(cell); return true
        element_idx[cell] = transition_idx; ChangeSubstance(cell); return true
    else:   # 目标非固态（如气→液）
        if !headless:
            above = cell - width
            if cells.properties[above] & 2 == 0:   # 原版读稳定缓冲 cells+0x68（properties）位 2 = 非固态
                game_cell_above = (width-2)×(row-1) + col - 1
                if visible_grid[game_cell_above] != 0 || debug_editing:
                    elem = gElements[transition_idx]
                    if elem.low_temp - 3.0 <= T <= elem.high_temp + 3.0:
                        spawn_liquid_info.push(SpawnFallingLiquidInfo{game_cell_above, transition_idx,
                                            disease_idx, mass, T, disease_count})
                        ClearCell(cell); ChangeSubstance(cell); return true
        element_idx[cell] = transition_idx; ChangeSubstance(cell); return true
```

依赖（需在实现时一并补齐）：
- `SimEvents::SpawnOre`（L133682-133737）：mass≤0 → false；gameCell = `(width-2)×(row-1) + (col-1)`；非 debugEditing 且 `visibleGrid[gameCell]==0` → false；推 `spawn_ore_info.push(SpawnOreInfo{callbackIdx=gameCell, elemIdx, diseaseIdx, mass, temperature, diseaseCount})`。**不扣质量/疾病**，调用方扣。
- `ChangeSubstance`：复用 `liquid_flow::push_substance_change`（改 pub(crate)）。
- `ClearCell`：复用 `liquid_flow::clear_cell`（改 pub(crate)，作用于 updated_cells）或 `sim_data_ops::clear_cell`（按原版 `SimData::ClearCell` 语义，两者对 7 字段清零一致，实现时选定其一并统一）。

### 3.5 Modify 过热事件（原版 L137187-137203）

`BuildingTemperatureManager::modify` 增加：
- 先用 3.1 的派生字段计算重建 `local_40`（不写回）。
- 若 `old_temperature >= overheat_temperature && new_temperature < overheat_temperature` → `buildingNoLongerOverheatedInfo.push(MeltedInfo{handle})`。
  - **注意**：条件是 ≥（不是 ≤），事件是 no-longer-overheated（不是 overheat）；此前管道审查建议方向写反，以源码为准。
- 句柄校验后整体写回记录（含派生字段）。

## 4. 测试

单测（沿用项目 lib test 惯例，LIB_TESTS_LOCK 串行）：

- InitializeFromMessageData：派生字段（perCellHC = SHC×mass/格数、TC = 元素TC×消息TC、simMin/Max+1、highTemp）、面积<1 断言、温度越界断言。
- ModifyBuildingEnergy：正常升温、钳制 [minTemp,maxTemp]、钳制 [0,10000]、分母为 perCellHC（非 mass）、非法温度日志不写回、坏句柄跳过。
- BuildingHeatExchange::Update：真空格跳过（建筑邻真空格不换热）、单格能量守恒收敛、交叉钳制、operatingKW 产热升温、过热度三事件边界（=、<、>）、melted 事件、temperatureInfo 输出（含空槽 -1）。
- DoStateTransition：高温转变（元素切换 + 矿渣 + ChangeSubstance + 背墙事件）、低温固态转变（≤0.8 清格出矿渣 / >0.8 保格）、低温非固态转变（可见 → 液滴事件 / 不可见 → 保格）、±3 闸门与 0xFFFF 闸门。
- 回归：全量 `cargo test -- --test-threads=1` 通过 + Release 构建成功。

## 5. 验收

- `cargo test -- --test-threads=1` 全过，`cargo build --release` 成功。
- 用户实测：
  1. 发热建筑（水泵等）运行 → 建筑温度上升（ModifyBuildingEnergy 消费生效）。
  2. 建筑周围格子升温；真空房内建筑温度只升不散（真空不换热）。
  3. 过热度触发 → 游戏内过热提示（C# 读 buildingOverheatInfo）。
  4. 管道换热有真实温差：管道水珠温度随建筑温度变化。

## 6. 范围外（记录）

- 方格↔方格 `UpdateTemperature` / 背墙换热 `UpdateTemperatureForBackwallExchange`（C2 下一轮）。
- 导热板 `BuildingToBuildingHeatExchange`（L138358，接触列表消息链）。
- 变温板 `OverrideExtents`（纯 C#，sim 无需改动）。
- 升华/持续蒸发、ElementEmitter 本体。
- 已知偏差：无 Region 概念（整图遍历，行为等价）；records 非压缩（沿用现有，长会话内存线性增长已记录）；调试断言按项目惯例 tracing 日志 + 继续（原版 DebugBreak/崩溃转为告警）；`visible_grid` 全可见兜底（沿用现有分配，屏外多事件偏差已记录）。
