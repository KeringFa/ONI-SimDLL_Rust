# 病菌模块・阶段 3：病菌消息/组件（设计规格，2026-08-05）

## 目标与范围

**本阶段**（对照原版 06_process_messages.c + 08_sim_frame_manager.c + 11_msvcrt_ignored.c + C# SimMessages）：
1. **DiseaseEmitter 组件**（C# 活跃使用，如 CrabFreshWaterConfig / DiseaseEmitter.cs）：Add（4B）/ Modify（20B）/ Remove（8B）消息 + 版本句柄注册表 + 组件 Update（BFS 泛洪发射）；
2. **ConsumeDisease**（16B，C# 发送）：百分比消费 + DiseaseConsumedCallback（12B）；
3. **CellDiseaseModification**（12B，C# 发送）：gameCell→simCell + 直加/AddDiseaseToCell；
4. **DiseaseConsumer 组件**（消息仅存在于 C# SimMessageHashes，**无发送方**）：Add（12B）/ Modify（20B）/ Remove（8B）解析 + 注册表（防御性，Update 预留）。

**修正代办记录**：ConsumeDisease 消息为 **16B**（C# struct：gameCell i32 + callbackIdx i32 + percentToConsume f32 + maxToConsume i32；simDLL stride +4），此前桩标 12B 有误。

**不做**（后续阶段）：阶段 4 辐射病菌贡献（挂接位已留）；病菌表/格子模拟（阶段 1/2 已完）。

## 原版机制全解

### 1. 消息结构与路由（C# SimMessages + SimFrameManager::HandleMessage）

| 消息 | ID | 结构 | 尺寸 |
|------|----|------|------|
| AddDiseaseEmitter | 1486783027 | `{int callbackIdx}` | 4B |
| ModifyDiseaseEmitter | 2395843372 | `{int handle, int gameCell, byte diseaseIdx, byte maxDepth, pad0, pad1, float emitInterval, int emitCount}` | 20B |
| RemoveDiseaseEmitter | 468135926 | `{int handle, int callbackIdx}` | 8B |
| AddDiseaseConsumer | 348345681 | `{int gameCell, int callbackIdx, ...4B}`（无 C# struct；Register 不读字段） | 12B |
| ModifyDiseaseConsumer | 2471979672 | 与 ModifyDiseaseEmitter 同布局（frame manager 0x14 stride） | 20B |
| RemoveDiseaseConsumer | 3513325646 | `{int handle, int callbackIdx}` | 8B |
| ConsumeDisease | 3275125760 | `{int gameCell, int callbackIdx, float percentToConsume, int maxToConsume}` | 16B |
| CellDiseaseModification | 2441296022 | `{int cellIdx(gameCell), byte diseaseIdx, pad×3, int diseaseCount}` | 12B |

Frame manager 路由：各消息 emplace 到 `diseaseEmitterMessages.adds/modifies/removes`、`diseaseConsumerMessages.adds/modifies/removes`、`consumeDisease`、`cellDiseaseModifications`。

### 2. ProcessConsumeDisease（06_process_messages.c L422-522）

```
每条消息：
  simCell = (gameCell/(width-2)+1)×width + gameCell%(width-2) + 1
  uVar10 = 0xff；consumed = 0
  若 disease_idx[simCell] != 0xff：
    count = disease_count[simCell]
    consumed = (int)(count × percentToConsume + 0.5)      // 四舍五入
    若 maxToConsume <= consumed → consumed = maxToConsume
    count[simCell] -= consumed
    若 count[simCell] < 1 → 清四字段（idx=0xff,count=0,tick=0,err=0）
    uVar10 = disease_idx[simCell]（清除后为 0xff）
  若 callbackIdx != -1：
    push DiseaseConsumedCallback{callbackIdx, diseaseIdx=uVar10, diseaseCount=consumed}（12B）
```

### 3. CellDiseaseModification 处理（SimDLL_Source.c L116412-116474，位于 ProcessConsumeDisease 之后）

```
simCell = (gameCell/(width-2)+1)×width + gameCell%(width-2) + 1
若 diseaseIdx == 0xff：
  count[simCell] += diseaseCount
  若 count[simCell] < 1 → 清四字段
否则：
  Disease::AddDiseaseToCell(updated, simCell, diseaseIdx, diseaseCount)   // 走阶段 1 强度混合
```

### 4. DiseaseEmitter 组件（11_msvcrt_ignored.c）

**DiseaseEmitterData（20B）**：`int cell@0`（sim 格）、`uchar diseaseIdx@4`、`uchar range@5`、`int emitCount@8`、`float emitInterval@0xc`、`float elapsedTime@0x10`。

**Register（L37549）**：分配槽（版本句柄，CompactedVector 模式），默认 `cell=-1, diseaseIdx=0xff, range=0, emitCount=0, emitInterval=0, elapsedTime=0`；返回 handle；callbackIdx≠-1 → `componentStateChanged{handle, callbackIdx}`（8B）。

**Modify（L37492）**：版本校验后写 `cell=(gameCell/(width-2)+1)×width+gameCell%(width-2)+1`、diseaseIdx、range(maxDepth)、emitCount、emitInterval；**不重置 elapsedTime**。

**Unregister（L37635）**：释放槽（版本+1）；callbackIdx≠-1 → `componentStateChanged{-1, callbackIdx}`。

**Update（L37651，dt=0.2, region）**：
```
每发射器 i：
  若 diseaseIdx[i] == 0xff → 跳过
  若 cell[i] 不在 region 内 → 跳过
  elapsedTime[i] += dt
  若 emitInterval[i] <= elapsedTime[i]：
    emittedInfo[i] = {diseaseIdx[i], emitCount[i]}          // 8B 输出（推断自 bounds check + C# 读取）
    reachable = GetReachableCells(cell.x, cell.y, range[i]) // 4 向 BFS，depth < range
    若 emitCount[i] < 0：
      仅对 disease_idx[cell]==diseaseIdx[i] 的可达格：AddDiseaseToCell(updated, cell, diseaseIdx, emitCount)
    否则：
      所有可达格：AddDiseaseToCell(updated, cell, diseaseIdx, emitCount)
    elapsedTime[i] -= emitInterval[i]
```

**UpdateDataListOnly（L37757）**：无时间流逝帧清 per-emitter 状态（emittedInfo/visited 重置）。

### 5. GetReachableCells / Flood（L34451 / L33466）

```
4 向 BFS（上下左右），起始 (x,y,depth=0)；
每个弹出的格：depth < range 且 x∈[1,width-1]、y∈[1,height-1] 且未访问：
  → 加入 reachable；压入 4 邻格 depth+1
不筛选内容（GetReachableCells 三个 state 过滤 flag 全 false）——发射可达所有元素格。
```

### 6. DiseaseConsumer 组件（无 C# 发送方）

- DiseaseConsumerData（12B）：`float consumptionRate@0`、`uchar maxDepth@4`、`Vector2<ushort> cell@5`、`ushort propertyMask@9`；
- Register（L37036）：分配槽（消息字段未读，注册零值记录）；Unregister（L37115）：释放槽；
- consumedInfo（vector<DiseaseConsumedInfo>，8B/条）经 copy_sim_data_to_game swap 到 GameData（save_load.rs L800-806 已接）；
- **Update 预留**：原版 vtable 未解析、C# 无发送方 → 注册表恒空；本阶段实现消息解析 + 注册表 + consumedInfo 输出清空，Update 留注释占位（日后有发送方或更完整反编译再补）。

### 7. 挂接位置

- **frame_processor.process_frame**：步骤 7 ProcessConsumeDisease（现桩）→ 步骤 8 CellDiseaseModification（现桩）→ … → 组件消息段（diseaseEmitter/diseaseConsumer adds→modifies→removes，紧跟 radiationEmitter 消息，原版 ProcessFrame 同区）。
- **update_data 区域循环**：`update_disease_emitters(sim, 0.2, bounds)` 紧跟 `update_radiation_emitters_global`（原版 L145434 UpdateComponents 位于辐射任务后、太空真空前）。

## 实现设计

### 模块结构（新建 `simDLL_RS/src/c_simulation/disease_component.rs`）

- `DiseaseEmitterRegistry`：版本句柄 + free list（仿 radiation_emitter.rs）+ `emitted_info: Vec<DiseaseEmittedInfo>`；
- `register/modify/remove`（含 componentStateChanged 回调）；
- `process_disease_emitter_messages(frame, sim)`：adds→modifies→removes + 清空；
- `update_disease_emitters(sim, dt, bounds)`：逐发射器按原版 Update 发射（BFS 复用新写 `get_reachable_cells`）；
- `process_disease_consumer_messages(frame, sim)`：adds→modifies(预留)→removes + 清空；
- `process_consume_disease(frame, sim)` / `process_cell_disease_modifications(frame, sim)`（替换 frame_processor 桩，复用 liquid_flow::modify_disease_count 语义）。

### 消息路由修改（sim_frame_manager.rs）

- `MSG_CONSUME_DISEASE`：12B → **16B**；
- `MSG_ADD/MODIFY/REMOVE_DISEASE_EMITTER`：从 "C2 stub no emplace" 改为 emplace 4/20/8B；
- `MSG_ADD/MODIFY/REMOVE_DISEASE_CONSUMER`：emplace 12/20/8B；
- frame_processor 的 process_component_messages_stubs 中移除 disease 向量清空（改由真实处理函数清空）。

### 防御语义（原版断言 → 跳过）

- Register/Modify：handle 版本不符 → 忽略；gameCell 越界 → 跳过；
- Update：gDisease 表缺失/diseaseIdx 越界 → 跳过该发射器；emittedInfo 越界 → 跳过输出；
- ConsumeDisease/CellDiseaseModification：向量越界 → 跳过（不 panic）；
- GetReachableCells：越界格跳过（原版部分越界断言）。

## 测试计划（TDD）

1. **消息路由尺寸**：ConsumeDisease 16B / CellDiseaseModification 12B / emitter 4-20-8B / consumer 12-20-8B 的 emplace 尺寸；
2. **ConsumeDisease**：百分比四舍五入、maxToConsume 钳制、count<1 清四字段、callback 推送（含清除后 diseaseIdx=0xff）；
3. **CellDiseaseModification**：diseaseIdx==0xff 直加 + 清除；非 0xff 走 AddDiseaseToCell（强度混合）；
4. **DiseaseEmitter**：Register 默认值 + 句柄版本；Modify 写字段 + game→sim 坐标；Remove 释放 + 回调；Update：elapsedTime 累加、interval 触发射、emitCount 正负两分支、emittedInfo 输出、region 门控；
5. **GetReachableCells**：4 向 BFS 深度门控（depth<range）、边界排除、全元素可达；
6. **DiseaseConsumer**：注册/移除 + 消息解析（防御）；
7. **update_data 集成**：发射器经 update_data 向邻格发射病菌；
8. 全量回归 + release。

## 回归与安全

- frame 消息路由改动只影响病菌六消息 + ConsumeDisease（此前桩仅清空，无行为）；
- 发射器组件只在注册后运行（无 C# 发射器 → 零开销）；
- 全量测试串行 + release 构建。

## 自检

1. **占位符**：无"待定/TODO"；DiseaseConsumer::Update 明确标注"预留（无 C# 发送方 + vtable 未解析）"。
2. **内部一致**：消息结构/尺寸逐字段对照 C# struct + frame manager stride；ConsumeDisease 16B 修正已在规格与代办记录。
3. **范围**：单实现计划可覆盖（新建 1 模块 + 路由/桩修改 + 测试）。
4. **歧义**：emittedInfo 写语义为推断（bounds check + C# 读取 + swap），已在规格标注；DiseaseConsumer::Update 无证据实现，预留。
