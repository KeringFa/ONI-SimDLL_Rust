# 气体流动模块全量对照审查（2026-08-04）

**范围**：c2_physics/gas_flow.rs（run_gas_pressure_task / run_gas_displacement_task /
do_gas_pressure_displacement）+ liquid_flow.rs 气体相关段（post_process_cell 气体分支 /
do_density_displacement / displace_gas / update_neighbour_liquid_mass / update_pressure）
对照 SimDLL_Source.c L144590-144950（UpdateData 气体阶段）、L147277-147330
（UpdateNeighbourLiquidMass）、L147342-147571（DoGasPressureDisplacement）、
L147819-147900（DoDensityDisplacement）、L149292-149550（PostProcessCell 气体分支）、
L135901-136060（DisplaceGas）、L11008-11020（CellSOA 布局）。

## 结论：3 项修正（均已在审查中修复）+ 6 项核对通过；无 Critical / Important 遗留

## 修正项（审查发现，已修复）

1. **DoDensityDisplacement 分支 B 门控读反（🔴 修复）**
   - 证据：CellSOA 布局 L11008-11020 = elementIdx/temperature(0x28)/mass(0x48)/
     properties(0x68)...；分支 B（L147862-147885）读 0x28 数组做 `<=` 门控 → 那是
     **温度**数组；0x48 数组仅作 FastCalculateCombinedTemperature 的质量参数。
   - 原版语义：同元素分支唯一门控 = `temp[below] > temp[cell]`（下方更暖才处理）；
     **无 mass 门控**。气体 → 交换（暖上升）；液体 → 质量加权合并温度写回两格。
   - 修正：恢复温度检查、移除多余质量检查（此前一轮误删温度、保留质量——方向反了，
     本轮审查中纠正）。

2. **随机交换候选选择误用 mass（🔴 修复）**
   - 证据：原版 L149467 `fVar24 = *(float *)(updatedCells + 0x28 + cand*4)` → 0x28 =
     temperature → 候选选择"**最低温度**"气体格（fVar24 <= fVar23，FLT_MAX 起）。
   - 修正：`updated.mass.get(cand)` → `updated.temperature.get(cand)`（old 版亦取温度）。

3. **run_gas_displacement_task 调用方"beyond 必须气体"预检（🔴 修复）**
   - 证据：原版 DoGasPressureDisplacement L147502-147512：beyond 元素 == dst 元素
     **或** vacuum（且可渗透）才放行；调用方（L144779-144791）只预检 dst
     （异元素 + 可渗透 + 气体），不预检 beyond。
   - 修正：删除调用方 beyond-state==1 预检，交由内部检查（本就与原版一致）。
   - 备注：该预检源自 old 版修 Bug1-4 的额外限制；原版允许气体被推入真空格。
     测试 `different_gas_displacement_beyond_vacuum_blocked` 改为
     `..._allowed`（dst 变气体A、气体B 全量顶入真空格）。

## 核对通过（无差异）

- **run_gas_pressure_task**（L144590-144760）：源格 state<2 + 可渗透 + 快照一致；
  dst 四条件（可渗透/快照一致/state<2/非真空-真空/非异元素同状态）；三方向
  （水平 ±iterateDir、垂直 +width、对角 +width+iterateDir）；对角角落检查 =
  `(state[水平邻] | state[垂直邻]) & 3 != 3`（原版 L144688-144689 用
  src+iterateDir 与 src+width 两格）；flow 记账 y/x/z/w 与原版一致。
- **do_gas_pressure_displacement**（L147342-147571）：源格质量 > 邻格（cells）、
  updated 一致性、邻格质量 > 0、beyond==dst元素或真空+可渗透、DoDisplacement、
  flow=min(12.5%×原质量, 当前质量)、温度直接覆盖（cells→updated）、元素+事件、
  病菌固定 0.125。
- **displace_gas**（L135901-136060）：候选 [+width, -1, +1, -width] 按 tickCount
  旋转；循环1 同气体或真空+可渗透；循环2 对角 +width±1（上方左右）同气体+
  同侧平格非固体+可渗透。
- **update_neighbour_liquid_mass**（L147277-147330）：同元素合并 / void 清质量 /
  异元素液体拒绝 / 气体 → DisplaceGas 挤走再占据 / 真空 → 直接占据。
- **post_process_cell 气体分支**（L149292-149550）：挤压邻居检查顺序
  [cell±dir, 下, 上] 质量≥1.0 气体；质量 <1e-9 蒸发、[1e-9,0.001) 被挤压才蒸发；
  DoDensityDisplacement(0.99)；随机交换 10% 门槛 + 0.5 候选门槛 + 三候选
  [cell-旧dir, cell+旧dir, 下] + 摩尔约束仅下方（标志位 [false,false,true]）+ 最低温度；
  末尾 DoSublimation。
- **update_pressure 流率系数**（L1475-1477）：K1 方案 A 的角色对调语义已记录，
  气体/真空驱动与液体邻格实测验证一致。

## 备注（低置信度 / 已实测覆盖）

- **run_gas_pressure_task 真空源格 re-check**：水平/垂直后对真空源格重读
  updated[src] 元素（变了就跳过后续方向）——反编译寄存器打包噪声无法逐行确认，
  但整体扩散行为用户实测与原版一致（圆形振荡），判为等价近似，记录在案。
- **异元素气体置换 flow 记账**：方向 1 的 flow[dst].z += f 表达式在反编译中
  混有属性指针噪声，按意图核对为 flow[src].w -= f、flow[dst].z += f（与其它方向一致）。

## 验证

- 全量 450 测试串行全过（398 lib + 52 集成；新增/改写 2 条：beyond 真空允许、
  气液水平置换、分层回归 3 条为 gas_flow 侧）；release 构建成功。
- 分层回归：12×8 房间 CO2 100kg 上 / O2 200kg 下 → 1000 帧 ~92% 沉底、
  45000 帧上半区归零（温度选择语义下仍收敛）。

## 待实测（部署后）

- ① 气体圆形振荡/真空扩散不回归（beyond 真空放开后重点回归）；
- ② 分层速度/效果不回归；③ 液体水平推挤气体不回归。
