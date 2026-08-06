# 塌方/不稳定方块下落模块设计（2026-08-06）

## 背景

`DoUnstableCheck`（原版 05_liquid_flow.c L1841-2210+）此前是空桩
（`do_unstable_check_stub`）。这是沙子/碎石/污染土等"不稳定"元素被挖空下方后
塌方的核心机制：sim 把方块格清空并推 `UnstableCellInfo`（20B）事件，C# 侧
`Game.cs L1171` 读到 `fallingInfo==0` 后由 `UnstableGroundManager.Spawn` 生成
下落实体，落地时经 `AddRemoveSubstance(do_vertical_solid_displacement: true)`
回注 sim（走 add_solid 垂直下落路径）。

## 原版语义（对照 05_liquid_flow.c + SimDLL_Source.c）

### 调用门（PostProcessCell L2533-2545）

固体元素且 `ppd.state & 0xb == 0xb`（state 低 2 位=3 且 bit3 置位 = "不稳定"
标记）时调用：

```text
savedOptions & 1 != 0 → DoUnstableCheckWithDiagonals（C# 当前从不设置，
休眠选项 ENABLE_DIAGONAL_FALLING_SAND）
否则              → DoUnstableCheckBasic
```

### DoUnstableCheckBasic（L1841-1937）

读 `updatedCells`：
1. `below = cell − width`（下方）；越界 → return（原版断言）。
2. below 元素 `state&3 < 3`（气体/液体/真空）且元素 ≠ void 且
   `properties[below]&4 == 0`（非 SolidImpermeable）：
   - `GetStableTicksRemaining(cell) == 0`（稳定 tick 耗尽）→ 塌方：
     - 非 headless：推 `UnstableCellInfo{elemIdx[cell], cellIdx=game(cell),
       fallingInfo=0, diseaseIdx[cell], mass[cell], temperature[cell],
       diseaseCount[cell]}`（20B）→ `Evaporate(cell)`（ClearCell + ChangeSubstance）。
     - headless：`HeadlessUnstableFallSimpleSwap`（逐格向下交换，世界生成用）。
   - return。
3. 否则（下方不满足条件）：`timers[cell] |= 0x1F`（标记"刚变化"）。

### GetStableTicksRemaining（SimDLL_Source.c L123214）

`timers[cell]` 低 5 位：
- `== 0x1F`（刚变化标记）：LCG 随机重掷 `3 + (seed>>16 & 0x7fff)×9.155553e-05`
  ∈ [3,6) → 写入低 5 位，返回新值（3-5 tick 稳定期）。
- `!= 0`：减 1 写回，返回递减后值。
- `== 0`：返回 0（可塌方）。

### DoUnstableCheckWithDiagonals（L1937-2208，休眠）

三方向扫描（方向表 {offset, 权重, 阈值系数, 正交标志}）：
- 正下：offset=−width、权重 1.0、阈值 0、正交标志 false；
- 左下：offset=−width−1、权重 0.5、阈值 1.0×minHorizontalFlow、正交 true；
- 右下：offset=1−width、权重 0.5、阈值 1.0×minHorizontalFlow、正交 true。

门控 `(mass[cell] − 阈值系数×ppd.minHorizontalFlow) × 权重 > 0`；候选格
（cell+offset）须 gas/liquid/vacuum、非 void、props&4==0；正交标志时额外要求
候选格下方格（candidate+width）非固体且 props&4==0。稳定 tick 耗尽后：
- 非 headless：推 `UnstableCellInfo`（**cellIdx=候选格 game 坐标**，mass/disease
  ×权重），源格 `modifyMass(−mass×权重)`、`ModifyDiseaseCount(−count×权重)`、
  ChangeSubstance；`mass<=0` → Evaporate。
- headless：候选格 DisplaceGas/DisplaceLiquid 后元素/质量/温度/病菌搬入候选格，
  源格按权重扣减，再 `HeadlessUnstableFallSimpleSwap(candidate)`。

### HeadlessUnstableFallSimpleSwap（L2209-2260）

从 `cell−width` 起向下走：逐格交换 7 字段（element/mass/temperature/disease×4）；
遇固体 → `ChangeSubstance(下方格)` 返回；越界返回。

### SetSavedOptions（02_save_load.c L466-484 + C# SimMessages L366-374）

2B 消息 `{clearBits u8, setBits u8}` → `saved_options = (saved_options & ~clearBits) |
setBits`。C# 当前无调用方（`SetSavedOptionValue` 未在任何实体使用），saved_options
实际恒 0 → Basic 变体；实现仍忠实，供存档/未来使用。

## 实现要点

- `properties` bit 含义（C# Sim.Cell.Properties）：1=GasImpermeable、2=
  LiquidImpermeable（sim 兼作固体标志）、4=SolidImpermeable、8=Unbreakable、
  0x40=NotifyOnMelt。
- UnstableCellInfo 20B 布局：elemIdx u16 + cellIdx i32 + fallingInfo u8 +
  diseaseIdx u8 + mass f32 + temperature f32 + diseaseCount i32（game_data.rs 已有）。
- game 坐标换算：`game = (cell % width − 1) + (width − 2) × (cell / width − 1)`。
- `Evaporate` = `do_evaporate`（clear_cell + push_substance_change，已有）。
- 随机数复用 `next_random`（LCG，`(seed>>16 & 0x7fff)`）。

## 测试计划

- SetSavedOptions：clear/set 位掩码（含重叠位）。
- GetStableTicksRemaining：0x1F 重掷 3-5、递减至 0、0 保持。
- DoUnstableCheckBasic：气体下方 → 事件+清格；固体下方 → 无事件+timers|0x1F；
  void 下方 → 不塌；稳定 tick>0 → 不塌；headless → 交换。
- DoUnstableCheckWithDiagonals：正下/对角分支（休眠路径，结构断言）。
- update_data 集成：固体不稳定格塌方事件进入 sim_events → copy_sim_data_to_game。
