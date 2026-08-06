# 塌方/不稳定方块下落实现计划（2026-08-06）

规格：docs/superpowers/specs/2026-08-06-unstable-fall-design.md

## 任务分解

1. **SetSavedOptions handler**（save_load.rs）：2B {clearBits, setBits} →
   `saved_options = (saved_options & ~clearBits) | setBits`。TDD。
2. **GetStableTicksRemaining**（liquid_flow.rs）：timers 低 5 位语义
   （0x1F 重掷 3-5 / 递减 / 0 保持）。TDD。
3. **DoUnstableCheckBasic**：下方 gas/liquid/vacuum 且非 void 且
   props&4==0 且稳定 tick 耗尽 → 推 UnstableCellInfo + do_evaporate；
   否则 timers|0x1F；headless → HeadlessUnstableFallSimpleSwap。TDD。
4. **HeadlessUnstableFallSimpleSwap**：逐格向下交换 7 字段至固体/越界。TDD。
5. **DoUnstableCheckWithDiagonals**（休眠变体，savedOptions&1 门控）：三方向
   扫描 + 权重扣减 + headless 分支。结构级实现，注释标注休眠与方向表不确定性。
6. **post_process_loop 接线**：替换 do_unstable_check_stub → 按 savedOptions&1
   分派。
7. 全量测试 + release + 代办.txt 更新 + commit。

## 验收标准

- 沙子/碎石被挖空下方后：sim 清格 + unstableCellInfo 事件（fallingInfo=0）；
- 稳定 tick 延迟（3-5 tick）生效；timers 置位语义与原版一致；
- 固体下方/void 下方不塌；
- headless 路径逐格交换；
- 全量测试串行全过；release 构建成功；用户实测塌方行为与原版一致。
