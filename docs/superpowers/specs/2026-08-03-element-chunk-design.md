# ElementChunk（物质碎片）温度组件严格对照实现设计

**日期**：2026-08-03
**状态**：已获批（用户确认范围：完整组件；透气砖/网格砖特殊机制暂不研究）
**类别**：C1 消息/组件/句柄层 · ElementChunk（物质碎片温度）

## 1. 背景

温度模块进展：建筑温度链路、管道换热、方格↔方格换热、建筑↔格子换热均已实现并实测通过。
当前缺口：**物质碎片（ElementChunk）** 不与世界换热。原版中碎片会与其所在格内容物换热
（全比例），并在真空环境下与其**下方实体方格**换热（groundTransferScale）——用户已实测确认该原版行为。

本设计严格对照原版 `SimDLL_Source.c`：
- `ElementChunk::Register`（L139972-140075）——Add 派生；
- `ElementChunk::Modify`（L139894）、`ModifyAdjuster`（L139926）、`ModifyEnergy`（L139947）——消息；
- `ElementChunk::Update`（L140092-140202）——逐碎片换热 + 相变 + 输出；
- `ExchangeHeatEnergyWithWorld`（L139789-139890）——碎片↔格子换热核心；
- `ElementChunk::UpdateDataListOnly`（L140202）——输出刷新。

## 2. 架构认知

### 2.1 数据（ElementChunkData，44B = 0x2c，L10107）

| 偏移 | 字段 | 说明 |
|------|------|------|
| 0x00 | cell | sim 格索引 |
| 0x04 | temperature | 碎片温度 |
| 0x08 | heatCapacity | 碎片热容 = SHC×mass |
| 0x0c | thermalConductivity | 碎片导率 = 元素 TC |
| 0x10 | maxEnergyTransferScaleFactor | = surfaceArea / thickness |
| 0x14 | groundTransferScale | 与下方固体格换热比例 |
| 0x18 | highTemp | 元素 highTemp |
| 0x1c | lowTemp | 元素 lowTemp |
| 0x20 | adjuster | {handle, temperature, heatCapacity, thermalConductivity}（ExternalAdjuster） |

注册表模式同 BuildingTemperatureManager：版本句柄 `handle = (version<<24)|index`、free list 复用、
swap-remove 数据槽。`cell`/`adjuster.handle` 为 int 字段，其余为 float。

### 2.2 消息（对照帧缓冲）

| 消息 | 尺寸 | 内容 |
|------|------|------|
| Add（element_chunk_messages.adds） | 32B | {gameCell, callbackIdx, mass, temperature, surfaceArea, thickness, groundTransferScale, elementIdx u16, pad} |
| Remove（.removes） | 8B | {handle, callbackIdx} |
| Move（move_element_chunk_messages） | 8B | {handle, gameCell}（L12885） |
| Modify（.modifies） | 12B | {handle, temperature, heatCapacity} |
| ModifyEnergy（modify_element_chunk_energy_messages） | 8B | {handle, deltaKJ} |
| ModifyAdjuster（modify_element_chunk_adjuster_messages） | 16B | {handle, temperature, heatCapacity, thermalConductivity} |

Add 处理顺序（原版 ProcessFrame）：adds（Register）→ modifies（Modify）→ modifyEnergy → modifyAdjuster →
Move → removes（Unregister）——实现时按原版 ProcessFrame 段核对（L116653-116682 附近）。

### 2.3 Register 派生（L139972-140075）

```
elem = gETD[elementIdx]
cell        = (gameCell / (width-2) + 1) × width + gameCell % (width-2) + 1   // game→sim
temperature = msg.temperature
heatCapacity = elem.SHC × msg.mass
thermalConductivity = elem.TC
maxEnergyTransferScaleFactor = msg.surfaceArea / msg.thickness
groundTransferScale = msg.groundTransferScale
highTemp = elem.highTemp；lowTemp = elem.lowTemp
adjuster = {handle:-1, temperature:0, heatCapacity:0, thermalConductivity:0}
```

Modify（L139894）：`temperature = msg.temperature; heatCapacity = msg.heatCapacity`（temp≤0 且 hc≠0 → 断言转日志）。
ModifyEnergy（L139947）：`if (heatCapacity > 0) temperature += deltaKJ/heatCapacity`，钳制 ≥0。
ModifyAdjuster（L139926）：adjuster.temperature/heatCapacity/thermalConductivity = 消息三值。
Move：`cell = game→sim(msg.gameCell)`。

## 3. 实现项

### 3.1 `ExchangeHeatEnergyWithWorld`（L139789-139890）

输入：`dt`、`sim`、`chunk`（&mut ElementChunkData）、`cell`、`scale`。返回热流（f32）。

```
门控：cell mass > 0；chunk.temperature ∈ [0,10000]；cell 元素 ETD 有效；cell ETD.state 无 0x10（TemperatureInsulated）
cellHC = cell mass × cell SHC
k_cell = ins² × 1.53787e-05 × cellTC
k = min(chunk.thermalConductivity, k_cell) × scale × chunk.maxEnergyTransferScaleFactor × 0.001
CalculateTemperatureExchange(1.0, k, dt, cellT, cellHC, chunkT, chunkHC)
  —— wrapper 自动按温度排序（冷侧在前），返回 (new_热侧, new_冷侧, heat)
cell（updatedCells）.temperature = 新 cell 温度（直接写）
chunk.temperature = 新 chunk 温度
返回 heat
```

### 3.2 `ElementChunk::Update`（L140092-140202）

逐碎片（区域 = 整图内部格，行为等价）：

1. 门控：chunk 所在格在区域范围内且 `heatCapacity > 0`；
2. **adjuster 分支**：`adjuster.heatCapacity > 0` →
   `CalculateTemperatureExchange(1.0, adjuster.thermalConductivity, dt, adjusterT, adjusterHC, chunkT, chunkHC)`，
   `chunk.temperature = 新值`（不写格子）；
3. **世界换热分支**：
   a. `ExchangeHeatEnergyWithWorld(dt, sim, chunk, 本格, scale=1.0)`，累加热流；
   b. 若 `chunk.cell − width`（下方格）为**固体**（state&3==3）→
      `ExchangeHeatEnergyWithWorld(dt, sim, chunk, 下方格, scale=groundTransferScale)`，累加热流
      ——即"碎片在真空与下方实体方格换热"机制（L140165）；
4. 输出：`elementChunkInfo[handleIndex] = {temperature, 累计热流}`（追加，原版按 handle index 写）；
5. 相变：`chunk.temperature ≥ highTemp+3 或 < lowTemp−3` →
   `elementChunkMeltedInfo.push(MeltedInfo{handle})`（L140186-140196，SimEvents +0x80）。

### 3.3 输出与集成

- 新模块 `c_simulation/element_chunk.rs`（或并入 temperature 域）：`ElementChunkManager` 全局单例
  （模式同 BuildingTemperatureManager），持 records/items/versions/free_handles + 每帧输出 Vec<ElementChunkInfo>。
- `copy_sim_data_to_game`：新增 `game_data.element_chunk_info ↔ manager 输出` swap（对照原版
  CopySimDataToGame L97889 附近）；`element_chunk_melted_info` 已在 20 事件 swap 列表 ✓。
- `update_data` 组件阶段：`update_element_chunks(sim_data)`（与 `update_building_heat_exchange` 并列，
  原版 UpdateComponents L145434 同一循环）。
- frame_processor：`process_element_chunk_messages(frame, sim_data)`（adds→modifies→energy→adjuster→
  move→removes，对照原版 L116653-116682 段）。

## 4. 测试

单测（LIB_TESTS_LOCK 串行）：

- Register 派生：cell 坐标换算、heatCapacity=SHC×mass、TC=元素TC、maxEnergyScale=area/thickness、
  groundTransferScale、low/highTemp、adjuster 清零；
- Modify / ModifyEnergy（ΔT=ΔQ/HC 钳制≥0）/ ModifyAdjuster / Move；
- Update adjuster 分支：adjuster 换热不写格子；
- Update 世界分支：本格换热（mass>0）、下方固体格 groundTransferScale 换热（真空也可）、
  下方非固体不换热、TC=0 / 0x10 / mass 0 门控跳过；
- 相变事件 elementChunkMeltedInfo；
- 输出：elementChunkInfo {temperature, delta_kj} 填充与 swap；
- 回归：全量 `cargo test -- --test-threads=1` + Release。

## 5. 验收

- 全量测试通过 + Release 构建成功。
- 用户实测：碎片在气体/液体/真空环境与所在格换热；真空下与下方实体方格换热（groundTransferScale）；
  碎片被加热/冷却至相变温度 → 熔化事件。

## 6. 范围外（记录）

- 透气砖（Airflow）/ 网格砖（Mesh）特殊换热机制（用户暂不研究）；
- 碎片掉落动画与移动物理（C# ConduitFlow 类机制；Move 消息仅更新 sim 侧位置）；
- 碎片 disease 转移、与世界格辐射换热（后续）。
