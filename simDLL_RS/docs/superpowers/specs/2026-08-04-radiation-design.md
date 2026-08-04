# 辐射模块·阶段 A（格子级辐射核心）设计规格

日期：2026-08-04
范围：C2 辐射模块第一阶段——格子级辐射核心（元素辐射 + 宇宙辐射 + 参数 + 衰减 + CellRadiationModifications）。
后续阶段：B = RadiationEmitter 建造辐射；C = 生物辐射 / RadiationConsumer 辐射消费。

## 背景

- 辐射是《眼冒金星》DLC 专属功能；单星（基础版）存档不包含任何辐射内容。
- 原版通过 `SimData.radiationEnabled`（AllocateCells 第 4 参，我们日志中的 flag1）门控全部辐射模拟
  （11_msvcrt_ignored.c L42594 宇宙辐射段、L39721 RadiationEmitter::Update）。
- 我们当前：`cells.radiation` 字段、`ElementRadiationData`（factor + radsPer1000）、
  `currentCosmicRadiationIntensity`（NewGameFrame 已解析）、`cosmicRadiationOcclusion` 缓冲、
  全局辐射参数、三条消息解析（CellRadiationModification / RadiationParamsModification /
  RadiationEmitter Add/Modify/Remove）均已就绪但全部为桩（frame_processor 只清空不处理）。

## 设计原则

1. **严格 DLC 门控**：全部辐射逻辑包在 `sim.radiation_enabled == true` 内；单星（flag1=false）
   零辐射执行，`cells.radiation` 恒 0，与原版一致。
2. **严格对照原版**（11_msvcrt_ignored.c L42594-42914 宇宙辐射段；06_process_messages.c
   L370-414 消息处理；08_sim_frame_manager.c 消息解析）。
3. 病菌贡献依赖病菌表（CreateDiseaseTable 空桩）→ 阶段 A 跳过，记录依赖。

## 1. 消息落地（frame_processor）

### 1.1 RadiationParamsModification（**尺寸修正 8B**）

- 原版/C# 结构：`{ int type; float value; }`（8B，C# SimMessages.cs、00_types_reference.c L7459-7462）。
- ⚠️ 我们 frame_manager 现按 12B 解析（radiation_params_modifications @0x168）——**须修正为 8B**。
- type 映射（06_process_messages.c L392-410）：
  - 0 → RADIATION_LINGER_RATE
  - 2 → RADIATION_BASE_WEIGHT
  - 3 → RADIATION_DENSITY_WEIGHT
  - 4 → RADIATION_CONSTRUCTED_FACTOR
  - 5 → RADIATION_MAX_MASS
  - （type 1 原版无 case，跳过；其余忽略）
- 处理位置：ProcessFrame 消息阶段（对应 06_process_messages.c 该函数）。

### 1.2 CellRadiationModification（12B）

- 结构：`{ int gameCell; float radiationDelta; int callbackIdx; }`（12B，L14410-14414）。
- 处理（06_process_messages.c L355-382）：
  - gameCell → sim cell（`(gc % gameW + 1) + (gc / gameW + 1) × simW`）；
  - `updatedCells.radiation[cell] += delta`；结果 ≤ 0 → 置 0；
  - `callbackIdx != -1` → 推 `RadiationConsumedCallback { gameCell, callbackIdx, radiation }`
    （12B）到 `simEvents.radiationConsumedCallbacks`（sim_events 已有该 vector）。

## 2. 每子步辐射模拟（update_data）

位置：原版 UpdateData L42594（温度/液体等阶段之后、`SimData::UpdateComponents` 之前），
按 active region 逐格执行，整体包在 `radiation_enabled` 门控内。

### 2.1 宇宙辐射 occlusion（自顶向下）

- 每帧先 `cosmicRadiationOcclusion` 区域填 1.0（L42688 resize 初值 1.0）。
- 逐格（top→bottom，读上方格 cell+width 的 occlusion）：
  - block：`updatedCells.properties[cell] & 0x80`（ConstructedTile）→ `factor × constructedFactor`；
    否则 → `(mass / maxMass) × factor × densityWeight + factor × baseWeight`；
    factor = `gElementRadiationData[cell].factor`（radiationAbsorptionFactor @0）；
  - block clamp [0,1]；`occlusion[cell] = (1 - block) × occlusion[上方格]`。

### 2.2 衰减（L42785-42810）

- `fVar22 = radiation[cell] / lingerRate`；
- `fVar22 == 0.0`（radiation < lingerRate）→ `radiation -= 1.0`，clamp ≥0；
- 否则 `radiation -= fVar22`。

### 2.3 元素辐射扩散（25 格权重核，L42810-42850）

- 条件：`gElementRadiationData[cell].radsPer1000（@4）> 0`。
- 25 格核（±2 范围，中心权重 1.0，距离权重 0.1/0.15/0.25/0.5/0.75，对角同权）：
  `radiation[target] += mass[cell] × 0.001 × radsPer1000 × weight`；
- target 必须落在区域边界内（原版 L42830-42837 边界检查）。

### 2.4 病菌贡献（依赖病菌表，阶段 A 跳过）

- 原版 L42860：`diseaseIdx[cell] == RadiationSickness(0xd49f77d6)` →
  `radiation += diseaseCount × 0.001`。
- 依赖 CreateDiseaseTable（空桩，disease index 映射未建）→ 阶段 A 不实现，记录为依赖。

### 2.5 宇宙辐射应用（L42875-42884）

- `if (occlusion[cell] > 0)`：`radiation[cell] += (regionCosmicIntensity / lingerRate) × occlusion[cell]`；
- regionCosmicIntensity = 当前 active region 的 `currentCosmicRadiationIntensity`
  （L41914 `fStack_ec = local_160[5]`）。

### 2.6 钳制（L42885-42910）

- `radiation = clamp(radiation, 0, 9e6)`（SIM_MIN_RADIATION / SIM_MAX_RADIATION）；
- `radiation ≤ 0.01 → 0`。

## 3. 接线位置

- frame_processor：新增 `process_radiation_params` / `process_cell_radiation_changes`
  真实实现（替换桩）；回调推 simEvents.radiationConsumedCallbacks。
- update_data：在液体/太空真空/PostProcess 之后、组件阶段（BuildingHeatExchange 等）之前
  插入 `run_radiation_task(sim_data, bounds)`（包在 radiation_enabled 内）。
- copy_sim_data_to_game：无需改（cells.radiation 已随 CellSOA 拷贝；GDU.radiation 已接）。

## 4. 测试计划

- 单星门控：`radiation_enabled=false` → run_radiation_task 零执行、cells.radiation 不变；
- params 8B：type 0/2/3/4/5 映射、type 1/6 跳过；
- cell radiation：delta 累加、≤0 清零、callback 推入（12B 内容）；
- occlusion：顶行 1.0、元素吸收数值、构造格 constructedFactor 分支；
- 衰减：radiation/lingerRate、零值 −1 清理；
- 元素扩散：25 格核数值（中心 1.0、距离权重）；
- 宇宙应用：occlusion>0 累加 cosmic/linger × occlusion；
- 钳制：9e6 上限、≤0.01 清零。
- 元素表需含辐射字段（factor/radsPer1000），真空/固体/放射性元素样例。

## 5. 依赖与边界

- 病菌贡献 → 病菌模块（CreateDiseaseTable）实现后补；
- RadiationEmitter（建造辐射）→ 阶段 B；
- 生物辐射 / RadiationConsumer → 阶段 C（CellRadiationModification 的 callback 已就绪）；
- RadiationEmitter 消息（Add/Modify/Remove）保持桩（阶段 B）。

## 6. 原版参考位置

- 宇宙辐射段：11_msvcrt_ignored.c L42594-42914；
- 消息处理：06_process_messages.c L355-414；
- 消息解析：08_sim_frame_manager.c L483（cellRadiationModifications）、L797（radiationParamsModifications）；
- 结构：00_types_reference.c L7459-7462（params）、L14410-14414（cell）、L8583-8600（RadiationEmitterData）；
- 元素辐射数据：03_elements.c L369-377（gElementRadiationData 构建）。
