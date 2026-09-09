# FastTrack A/B 双跑对比解读模板（PerfProbe v4）

日期：2026-09-06　状态：待填数据
配套：PerfProbe v4（union 口径 + GC 归因 + ft= 检测，commit db97f1e）

## 数据来源约定

| 跑 | FastTrack | 探针 | 状态 |
|----|-----------|------|------|
| A | 关 | v4 | 待确认（若沿用 v3 旧数据见"口径警告"） |
| B | 开（行为保持选项组） | v4 | 待采 |

- 日志位置：`Documents\Klei\OxygenNotIncluded\mods\Local\SimDLL_PerfProbe\perf.log`
- 环境自证：窗口首行 `ft=0/1` 自动区分两跑；确认 A 行 `ft=0`、B 行 `ft=1`
- 场景要求：同一存档同一区域、vsync 关、60s；simdll_perf.log（Rust stages）同步留档作对照

## 口径警告（填数前必读）

1. **v3→v4 口径不可直接对比**：v3 的 RENDERGAP 是嵌套重复计入的低估下界；
   v4 是 union 口径。若 A 用的是 v3 旧数据，与 v4-B 对比会高估 FastTrack 收益。
   正确做法：A、B 都用 v4 探针各跑一次。
2. **FT join 等待**：B 跑的 RENDERGAP 里可能混有 FastTrack 主线程 join 等待
   （它把工作移后台后主线程 semaphore.WaitOne），不是渲染也不是 GC。
3. **Rust sim 不变量**：stages 各阶段值在 A/B 间应基本一致（FT 不碰 sim DLL）。
   若出现偏差 → 说明有意外交互，先停下排查。

## 对比表（已填，2026-09-06 14:12-14:18 实测，多星场景）

说明：取两跑稳定 5s 窗口（剔除开档/传送大窗口）；A=19 窗、B=23 窗，ft=0/1 自证 ✓

| 指标（5s 窗口） | A（ft=0） | B（ft=1） | Δ |
|----------------|-----------|-----------|---|
| FRAME avg (ms) | 46~97，中位 ≈52（≈20fps） | 43~69，中位 ≈50 | ≈0 |
| RENDERGAP avg (ms/帧) | 16~37（≈帧的 30~33%） | 17~39（≈33%） | ≈0 |
| SM.AdvanceOneSimSubTick (ms/5s) | 1009~2148（≈28% 墙钟） | 874~1528（≈28%） | 均值≈0；**1s 尖刺消失**（max 1002/1017 → 148） |
| Game.StepTheSim (ms/5s) | 365~547（≈9%） | 314~582（≈9%） | ≈0 |
| LogicCircuit.RenderEveryTick (ms/5s) | 10~18 | 10~20 | ≈0 |
| SolidConduitFlow.RenderEveryTick (ms/5s) | 17~48 | 28~57 | ≈0 |
| AsyncPathProber.TickFrame (ms/5s) | 4~9 | 5~11 | ≈0 |
| PropertyTextures.LateUpdate (ms/5s) | 71~132 | 6~20 | **−85~95%** |
| SM.RenderEveryTick (ms/5s) | 176~288 | 72~134 | **−55~70%** |
| KAnim.Render (ms/5s) | 39~219 | 22~104 | **−50~70%** |
| Game.LateUpdate (ms/5s) | 231~429 | 142~242 | **−40%** |
| KBatchedAnimUpdater.LateUpdate (ms/5s) | 140~247 | 156~231 | ≈0 |
| gc0+/gc1+/gc2+（窗口 delta） | 稳态全 0；尖刺帧 gc0+1 gc1+1 gc2+1 | 同 A | **1s 完整 GC 尖刺两跑都在** |
| 尖刺清单 | GC ~1s ×3~4 + SM 决策 ~1s ×2 | GC ~1s ×1~2 + **World.LateUpdate ~1s ×2（FT join 嫌疑）** | SM 尖刺消失，新增 FT stall |

Rust sim 不变量 ✓：stages 各阶段 A/B 几乎一致（copy≈2.0 / temperature≈5.1 / gas_pressure≈8.1 / postprocess≈15.0 / sim_to_game≈6.9 ms·CPU/帧），StepTheSim avg ≈7ms，FT 未触碰 sim。

环境口径：两跑均 `fpsCap=-1 vsync=1`（**vsync 实际是开的**，与此前认知相反；但 FRAME≈50ms ≫ 16.7ms，vsync 非瓶颈）。

## 判定矩阵逐行结论（已填）

| 桶 | A→B 变化 | mod 可动性结论 |
|----|---------|---------------|
| SM 状态机残余 (~28%) | 均值不变；1s 决策尖刺被 FT 拆散 | 均值=原版决策本体（不可动）；尖刺已被 FT 解决 |
| RENDERGAP 残余 (~33%) | 不变 | 主成分=Unity native 渲染+GPU（不可动）；B 中混入 FT join stall |
| GC（1s 完整 GC，两跑都有） | FT 未消除 | ✅ **sim/mod 侧分配嫌疑（sim_to_game 拷贝路径）**——唯一真正可动手大项 |
| 渲染局部桶 | −40%~−95% | 已被 FT 吃掉，无需再动 |

## 结论区（已填）

- **FastTrack 当前选项组的 FRAME 均值收益≈0**（中位 52→50ms）：帧时间大头（SM.Adv 28% + RENDERGAP 33% + StepTheSim 9%）不在其已开选项打击面上；但局部渲染桶减半、SM 1s 决策尖刺消失（AsyncBrain 生效）。
- **头号新发现：~1s 完整 GC 暂停（gc0/1/2 各+1，renderGap≈0.9~1.0s）每 30~60s 一次，A/B 均有** → 与 FT 无关，指向 sim→game 互操作/拷贝路径的托管分配（sim_to_game 6.9ms/帧 SOA 拷贝若逐帧分配托管数组即 ~300 次/s）。
- 帧上限格局更新：FT 后 C# 侧仍 ≈ 61%（SM 28% + GAP 33%）+ sim 9% —— 均值天花板未动，**尖刺治理**（GC）成为下一主战场。
- 下一动作候选：
  1. **sim_to_game 分配审计**（Rust→C# 拷贝是否逐帧分配托管数组/字符串）
  2. vsync 关对照跑一次（排除 Pacer 等待干扰，修正环境认知）
  3. FT 补开 AsyncPathProbe/CachePaths（行为保持类）看 SM.Adv 均值是否下降
  4. B 跑新增的 World.LateUpdate ~1s stall：观察项，疑似 FT 后台 join，持续跟踪
