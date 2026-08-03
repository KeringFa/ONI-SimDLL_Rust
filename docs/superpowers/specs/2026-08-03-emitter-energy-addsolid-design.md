# ElementEmitter 本体 + ModifyCellEnergy + AddSolid 垂直位移 设计

**日期**：2026-08-03
**状态**：待用户确认
**类别**：C1/C2 接线补全（方案一 + 垂直位移）

## 1. 背景

用户实测确认三项事实，据此扩展批次范围：

1. **建筑只能生成气/液/固体碎片，不能直接生成固体方块**（肥料合成器、石油发电机等走
   ElementEmitter → 气/液入格 + 固体推回调）；火箭引擎例外（喷发 CO2/蒸汽），
   HEPEngine 喷核尘埃。
2. **火箭发射/降落把引擎热量释放到下方 3×9 物质方格**（宽 3、深 9，不含碎片）——
   范围与数值在 C#（RocketEngineCluster.DoBurn），执行在 simDLL
   （ModifyCellEnergy → ProcessCellEnergyModifications，当前是 C2 桩）。
3. **研究性反应堆融毁爆发 NuclearWasteComet**——流星是唯一能直接生成固体方块的事物，
   撞击落地统一走 `Comet.DepositTiles → SimMessages.AddRemoveSubstance(固体, 默认
   do_vertical_solid_displacement=true) → AddSolid Type 0（DoVerticalDisplacement）`。
   所有流星（铝/漂白石/铜/铁/金/硬冰/核废料/氧石/岩石/粘液/雪球/铀/大型撞击者/迷你等）
   共用同一条原理，差异仅 addTiles 数量；无子类覆写 DepositTiles。

本批次三项（用户已确认）：**A ElementEmitter 本体、B ProcessCellEnergyModifications、
C add_solid 重写（含 DoVerticalDisplacement，纠正"四邻搜索混合版"）**。

## 2. 源码对照

### A ElementEmitter
- `ElementEmitter::Emit`（04_temperature.c L1495-1609）；
- `ElementEmitter::Modify`（L1610-1678）；
- `ElementEmitter::Register`（L1679-1791）；
- `ElementEmitter::TryEmit`（L1792-1965）；
- `ElementEmitter::Update`（L1966-2104）；
- `GetReachableCells`（11_msvcrt_ignored.c L34451，Flood BFS）；
- CopySimDataToGame emittedMassInfo 拷贝+清零（11_msvcrt_ignored.c L31626-31648）；
- 消息处理顺序：ElementConsumer removes → **ElementEmitter adds → modifies → removes** →
  RadiationEmitter adds…（08_sim_frame_manager.c L1628-1665）。

### B ProcessCellEnergyModifications
- 06_process_messages.c L134-211；调用点 08_sim_frame_manager.c L1260；
- 消息 `ModifyCellEnergyMessage` 16B：cellIdx i32、kilojoules f32、maxTemperature f32、
  id i32（EnergySourceID，仅日志）。

### C AddSolid
- SimDLL_Source.c L134555-134789；调用点 ProcessCellModifications（06_process_messages.c，
  固体分支 `local_b8 & 0xff` = 消息 addSubType）；
- C# ModifyCell：`addSubType = (!do_vertical_solid_displacement) ? 1 : 0`（SimMessages.cs
  ModifyCell 尾部）→ **0 = DoVerticalDisplacement（默认），1 = OnlyIfSameElement**；
- 我们 CellModification.flags（sim_events.rs @24）已解析该字节，但 frame_processor 分派
  时未传给 add_solid（当前 add_solid 是"四邻搜索混合版"，非原版任一模式，需重写）。

## 3. 消息结构（线序，已核对 C# StructLayout Pack=4）

| 消息 | ID | 布局 |
|------|-----|------|
| AddElementEmitter | 3789496115 | maxPressure f32 / callbackIdx i32 / onBlockedCB i32 / onUnblockedCB i32（16B） |
| ModifyElementEmitter | 403589164 | handle i32 / cellIdx i32 / emitInterval f32 / emitMass f32 / emitTemperature f32 / maxPressure f32 / diseaseCount i32 / elementIdx u16 / maxDepth u8 / diseaseIdx u8（32B） |
| RemoveElementEmitter | 2770849014 | handle i32 / callbackIdx i32（8B） |
| ModifyCellEnergy | 818320644 | cellIdx i32 / kilojoules f32 / maxTemperature f32 / id i32（16B） |

## 4. 实现项

### 4.1 A ElementEmitter 本体（新文件 `c_simulation/element_emitter.rs`）

**注册表**（仿 ElementChunk/BuildingToBuilding 模式）：
- `ElementEmitterData` 44B repr：elapsedTime f32 / emitInterval f32 / emitMass f32 /
  emitTemperature f32 / maxPressure f32 / emitDiseaseCount i32 / cell [u16;2]（row,col）/
  elemIdx u16 / maxDepth u8 / offsetIdx u8 / blockedState u8 / diseaseIdx u8 /
  blockedCBIdx u32 / unblockedCBIdx u32。
- Manager：records Vec + versions Vec<u8> + items Vec<i32>（free list）+ emitted_mass_info
  Vec<EmittedMassInfo>（16B：elemIdx u16 / diseaseIdx u8 / pad u8 / mass f32 /
  temperature f32 / diseaseCount i32，按 handle index 索引）。
- 句柄 `(version<<24)|index`，版本复用（与 ElementChunk 同款）。

**消息处理**（frame_processor，顺序 adds→modifies→removes）：
- Register：分配记录（maxPressure/callbacks 来自 Add，其余 0）；emitted_mass_info[idx] =
  {elemIdx=0xFFFF, 0…}；componentStateChanged 回调推 handle（callbackIdx≠-1 时）。
- Modify：版本校验；写全部发射参数；gameCell→sim cell（row/col 存储）；`emitMass<=0` →
  blockedState=0xFF（停用标记）。
- Remove：版本校验；释放记录；componentStateChanged 回调推 -1。

**Update**（挂 update_data 组件阶段，与 element_chunk/building_to_building 同批；
`update_element_emitters(sim, dt, width)`；区域过滤跳过，全量处理）：
- 每发射器：`elapsedTime += dt`；`elapsedTime >= emitInterval` 时：
  1. 发射器格元素 ∈ {0xFFFF, vacuum, elemIdx} → GetReachableCells(row, col, maxDepth)
     BFS（复用 ElementConsumer 的 Flood 实现）收集可达格；
  2. 任一可达格 `mass < maxPressure` → 有空间；否则堵住。堵/通状态变化时推 blocked/
     unblocked 回调（blockedCBIdx/unblockedCBIdx），blockedState 置 1/0；
  3. `TryEmit`：从 offsetIdx 起轮转可达格（offsetIdx++）；每格满足
     `mass < maxPressure && 发射元素 state&3 != 0`：
     - state 1（气体）：异元素 → DisplaceGas；state 2（液体）：异元素 → DisplaceLiquid
       （失败 → DisplaceGas）；state 3（固体）：推 MassConsumedCallback（20B，
       callbackIdx=gameCell、elemIdx、mass、temperature、disease 0；可见性检查）后
       goto 下一候选；
     - 然后 `Emit`：目标格 = 发射元素；质量合并（CalculateCombinedTemperature）+ 病菌
       （AddDiseaseToCell 简化：走 add_mass_and_update_temperature）+ 真空入格推
       ChangeSubstance 事件 + timers |= 0x1F；返回 EmittedMassInfo；
  4. 累积到 `emitted_mass_info[handle_index]`（elemIdx/mass/temperature 加权合并）；
  5. `elapsedTime -= emitInterval`。

**输出**：sim_data.element_emitter.emitted_mass_info 每帧 resize 到记录数并填充；
CopySimDataToGame 现有 swap（等价原版 copy+clear）无需改动。

### 4.2 B ProcessCellEnergyModifications（frame_processor.rs L563 桩替换）

对 cell_energy_modifications 每条 16B：
1. gameCell→simCell；元素有效且 `state&3 != 0`；`mass > 0.001`；`kilojoules > 0`；
2. `new_temp = temp + kilojoules / (mass × SHC)`；`cap = max(temp, maxTemperature)`；
   `new_temp = min(new_temp, cap)`；钳制 [1, 10000]（越界 fprintf 等价 warn）；
3. 写温度；`new_temp > 0` → DoStateTransition。

### 4.3 C add_solid 重写（liquid_flow.rs，含 Type 参数）

签名改为 `add_solid(sim, cell, elem, mass, temp, disease_idx, disease_count, add_sub_type)`，
frame_processor mode 0 固体分支传 `entry.flags`：
- **Type 0 = DoVerticalDisplacement**：
  1. 同列上方 4 格（row−1/0/+1/+2）若元素==elem：按 ppd.maxMass 补满
     （f16 = maxMass − mass[cell]；f16 = min(f16, 剩余质量)；AddMassAndUpdateTemperature
      + 病菌按 f16/剩余 比例）；剩余质量 ≤ 0 → return；
  2. 从目标格向下走（+width 逐格）：固体 → 继续；unobtanium → return；气体 →
     DisplaceGas；液体 → DisplaceLiquid；然后放置（elem/mass/temp/病菌 + ChangeSubstance
     + SetDisease + 温度断言）；到底 → return。
- **Type 1 = OnlyIfSameElement**：目标固体且元素≠elem → return（no-op）；气体 →
  DisplaceGas；液体 → DisplaceLiquid；AddMassAndUpdateTemperature 合并 + ChangeSubstance。
- **其他 Type → no-op**（原版直接 return）。
- 删除现有"四邻搜索混合版"逻辑。

## 5. 结构决策

- `c_simulation/element_emitter.rs` 新文件（Manager + Update + 消息）；mod.rs 挂载。
- `add_solid` 就地重写（liquid_flow.rs），frame_processor 传 flags。
- `process_cell_energy_modifications` 在 frame_processor.rs 实现。
- 复用：displace_gas/displace_liquid/add_mass_and_update_temperature/
  calculate_temperature_exchange_precise/push_substance_change/clear_disease/
  GetReachableCells Flood（ElementConsumer 同款）/组件回调链（componentStateChanged）。

## 6. 非目标

- disease_emitter / radiation_emitter 消息路由（后续独立项）；
- ElementEmitter 原版 Register 附带 BuildingHeatExchangeData 内部记录（无 C# 消费者，跳过）；
- GetReachableCells 区域过滤（全量处理，与现有组件一致）；
- 固体发射的 C# 侧碎片生成（MassConsumedCallback 已推，C# 侧自会 SpawnResource）。

## 7. 测试与验收

- 每项 TDD：消息尺寸/注册回调/句柄版本/Modify 停用标记/Update 计时发射/压力门与堵通回调/
  TryEmit 三态/EmittedMassInfo 累积（A）；ΔT 公式/钳制/DoStateTransition（B）；
  垂直位移两阶段/仅同元素/其他 no-op/帧级接线（C）。
- 全量回归 + Release 构建。
- **用户实测**：① 化肥机持续出天然气 + "Emitting" 状态 + 压力满 blocked 切换；
  ② 火箭发射/降落地面 3×9 区域升温（ModifyEnergy 生效）；③ 流星撞击生成固体方块
  （DoVerticalDisplacement：向上补同元素格、向下落格）；④ 反应堆融毁核废料流星 + 3000°C
  核废液 AddLiquid。
