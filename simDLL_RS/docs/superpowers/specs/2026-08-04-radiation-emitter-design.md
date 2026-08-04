# 辐射模块·阶段 B（RadiationEmitter 组件）设计规格

日期：2026-08-04
范围：RadiationEmitter 组件——**Constant + Pulsing + PulsingAveraged** 三种发射类型
（覆盖发光虫/冰息萝卜/蜜蜂/蜂巢/核反应堆）。SimplePulse/RadialBeams/Attractor 留待
反应堆实测时补（同一组件，扩展点预留）。

## 背景

- 用户实测：02 多星中**生物辐射（发光虫/冰息萝卜）失败、核反应堆不发射辐射**；
  铀元素建筑辐射正常（阶段 A 元素辐射）。三者走完全不同的路径：
  - 生物/建筑辐射 = C# RadiationEmitter 组件 → AddRadiationEmitter 消息 → sim 组件
    （本阶段实现）；
  - 铀元素辐射 = 元素辐射扩散（阶段 A 的 25 格核）。
- 核反应堆（NuclearReactorConfig）注册 RadiationEmitter **Constant**，radius 25×25；
  发光虫 Constant radius 6；冰息萝卜 Constant；蜜蜂 Constant radius 2-3；蜂巢
  Pulsing radius 7×6。
- 辐射粒子系统（HEP/Radbolt）纯 C#（HighEnergyParticle GameObject），不属 simDLL。

## 设计原则

1. 严格对照原版 11_msvcrt_ignored.c：RadiationEmitter::Register（L39374）/
   Modify（L39168）/ Update（L39693）/ tickConstant（L40172）/ tickPulsing（L40263）/
   inRadialRange / RadiationAbsorptionAlongLine / SetCircleAA（L39482）。
2. DLC 门控：Update 整体包在 `radiationEnabled`（L39721）。
3. 复用组件模式（版本句柄 + 延迟释放），与 ElementEmitter/BuildingHeatExchange 一致。

## 1. 消息与注册表

### 1.1 消息尺寸（frame_manager 需从"接受不 emplace"改为真实 emplace）

- **Add** 36B：`{callbackIdx, cell, emitRadiusX(s16), emitRadiusY(s16), emitRads,
  emitRate, emitSpeed, emitDirection, emitAngle, emitType}`；
- **Modify** 40B：`{handle, cell, callbackIdx, emitRadiusX(s16), emitRadiusY(s16),
  emitRads, emitRate, emitSpeed, emitDirection, emitAngle, emitType}`；
- **Remove** 8B：`{handle, callbackIdx}`；
- 存入 `frame.radiation_emitter_messages`（ComponentMessages：adds/modifies/removes）。

### 1.2 注册表（44B RadiationEmitterData）

- 字段：cell(i32) / radiusX(u16) / radiusY(u16) / emitRads(f32) / emitRate(f32) /
  emitSpeed(f32) / emitDirection(f32) / emitAngle(f32) / emitTimer(f32) /
  emitStepTimer(f32) / emitType(i32) / emitStep(i32)；
- 版本句柄（8bit 版本 + 24bit 索引）+ free list + 延迟释放双缓冲
  （Remove 入缓冲，两轮后 swap-remove 回收——与管道温度管理器模式一致）；
- Register：分配记录 + 回填字段 + `callbackIdx != -1` → component_state_changed 回调推
  sim_handle；失败推 -1；
- Modify：按 handle 更新字段；
- Remove：按 handle 释放 + 回调推 -1。

## 2. Update 时序（L39693-39830）

- 门控 `radiationEnabled`；
- 遍历区域内的发射器（cell 在区域边界内）：
  - `emitRads > 0` 且 `(radiusX != 0 || radiusY != 0)` 才活跃；
  - `emitTimer += dt`；
  - `emitRate == 0` → 恒发；否则 `emitTimer >= emitRate` 才发（触发后重置定时）；
  - `emitSpeed` 步进：`emitTimer <= emitSpeed` → 持续；超时 → `emitTimer = 0,
    emitStep = 0`；
  - 每 tick 步数 `n = dt / (emitSpeed / max(radiusX, radiusY))`（clamp ≥1）；
  - 按类型分发：Constant → tickConstant；Pulsing/PulsingAveraged → tickPulsing。

## 3. tickConstant（L40172）

遍历椭圆（radiusX×radiusY）内每格：
- 边界检查（区域边界内）+ `inRadialRange`（emitAngle/emitDirection 扇形）过滤；
- 椭圆内（`dx²/rx² + dy²/ry² ≤ 1`）：
  - `falloff = 1 − |dx|/rx − (|dy| − |dx·dy|/rx)/ry`；
  - `radiation[cell] += RadiationAbsorptionAlongLine(emitter→cell) ×
    (falloff × emitRads / lingerRate)`；
  - `falloff < 0.25`：随机抖动 `±12.5%`（randomSeed）。

## 4. tickPulsing（L40263）

- 当前环半径 = `radius × (emitStep+1) / max(radiusX, radiusY)`（随 emitStep 扩散）；
- PulsingAveraged：`emitRads / ((w−1)×(h−1))` 均摊；
- `emitStep == 0` → 中心格直接加（均摊后）emitRads；
- 否则 `SetCircleAA` 当前环 + setPulsing 回调（每格按
  `RadiationAbsorptionAlongLine × emitRads / lingerRate` 累加）。

## 5. 辅助函数

- `inRadialRange`：目标相对发射器的方向/角度 ∈ [emitDirection−emitAngle/2,
  emitDirection+emitAngle/2]；
- `RadiationAbsorptionAlongLine`：发射器→目标直线逐格，按元素
  `radiationAbsorptionFactor × (mass/maxMass × densityWeight + baseWeight)`
  累积遮挡（构造格用 constructedFactor），返回透射率 [0,1]；
- `SetCircleAA`：椭圆迭代回调框架（SimplePulse/Attractor 预留复用）。

## 6. 接线

- update_data：`run_radiation_task` 之后、组件阶段（BuildingHeatExchange 等）位置，
  调用 `update_radiation_emitters(sim, dt=0.2, bounds)`（内部门控）；
- frame_processor：`process_component_messages_stubs` 拆出
  `process_radiation_emitter_messages`（真实实现），其余组件保持桩。

## 7. 测试

- 消息：Add 36B 注册+回调、Modify 40B 更新、Remove 8B 释放+回调；
- 句柄：版本复用、延迟释放两轮；
- Constant：椭圆 falloff 数值、inRadialRange 扇形、吸收沿直线；
- Pulsing：环扩散、emitStep 递增、PulsingAveraged 均摊；
- 门控：radiationEnabled=false 不更新；单星零发射器执行；
- 全量回归 + release。

## 8. 原版参考

- Register/Modify/Update：11_msvcrt_ignored.c L39374 / L39168 / L39693；
- tickConstant：L40172；tickPulsing：L40263；setPulsing：L40143 附近；
- inRadialRange / RadiationAbsorptionAlongLine：L40100-40172 附近；
- SetCircleAA：L39482；
- RadiationEmitterData：00_types_reference.c L8583-8600——cell(4)+radiusX(2)+radiusY(2)+
  emitRads(4)+emitRate(4)+emitSpeed(4)+emitDirection(4)+emitAngle(4)+emitTimer(4)+
  emitStepTimer(4)+emitType(4)+emitStep(4) = **44B**。
