# PerfProbe v4 — RENDERGAP 黑盒拆解设计

日期：2026-09-06
状态：已批准（用户口头批准 4 节设计）
范围：simDLL_RS_Kit_v1/SimDLL_PerfProbe/PerfProbe.cs（仅测量，零行为修改）

## 1. 背景与目标

PerfProbe v3 实测（2026-09-06，多星场景）：C# 帧预算中 RENDERGAP（未探针方法合计）占 22%。
源码核实结论：

- **口径偏差**：`RecordFrame` 用 `gap = FRAME − Σ(prevFrameMethods)`，嵌套链
  Game.Update ⊃ SimEveryTick ⊃ UnsafeSim200ms / SM.AdvanceOneSimSubTick ⊃ StepTheSim
  各自独立 Record，同一段 sim 时间被计 2~4 次 → Σ 高估 → RENDERGAP 是**低估的下界**。
- **memory 中"A* 查询在 RENDERGAP"的猜测部分错误**：`Pathfinding.RenderEveryTick` 实为
  DebugUpdate（可忽略）；同步 A* 大头在 SM.AdvanceOneSimSubTick 内（已计入 37% 桶）。
- **真实裸奔点**：Game.Update 内每帧调用但未探针的方法：`LogicCircuitManager.RenderEveryTick`、
  `SolidConduitFlow.RenderEveryTick`、`AsyncPathProber.TickFrame`。
- **等待假设已排除**：用户确认测试时 vsync 关、高刷未锁帧 → 22% 大概率是真实成本。

目标：诊断后**可能动手**。拆解后的每一桶必须能回答"mod 动得了吗"（判定矩阵，见 §5）。

## 2. 口径修复（union 语义）

- 新增静态标志 `inGameUpdate`：`P_GameUpdate.Prefix` 置位，`Postfix` 复位。
- `Probe.Record`：处于 Game.Update 内部且 key ≠ "Game.Update" 的样本，照常进
  分桶统计（M./S. 桶、spikes 不受影响），但**不计入 `curFrameMethods`** 帧归因账本
  （其耗时已含在 Game.Update 内）。
- 效果：Σ 从"含嵌套的 sum"变为"顶层 union"，RENDERGAP = FRAME − union = 真实未覆盖时间。
- 注意：Game.LateUpdate、World.LateUpdate、PropertyTextures.LateUpdate、
  KAnimBatchManager.*、KBatchedAnimUpdater.* 等不在 Game.Update 内的顶层方法照常计入 union。
  KAnim.UpdateActiveArea/UpdateDirty 是否嵌套于 LateUpdate 链，实施时用深度计数器统一处理
  （深度 >0 的样本不进帧归因账本；深度以任一已探针顶层方法为根）。

### 探针自身 GC 污染修复

- `RecordFrame` 每帧 `new Dictionary<string,long>`（~300 次/秒分配）→ 改为两本账本
  `Swap(ref prev, ref cur) + cur.Clear()`，稳态零分配。
- 这是 v4 自身合规的前提：探针若自己制造 GC 压力，GC 归因（§4）结论会被污染。

## 3. 补充 patch + 环境口径

编译期属性 patch（反编译源已核实签名）：

| 方法 | 键名 |
|------|------|
| `LogicCircuitManager.RenderEveryTick(float)` | `LogicCircuit.RenderEveryTick` |
| `SolidConduitFlow.RenderEveryTick(float)` | `SolidConduitFlow.RenderEveryTick` |
| `AsyncPathProber.TickFrame()` | `AsyncPathProber.TickFrame` |

环境口径：Dump 窗口首行追加 `fpsCap=targetFrameRate vsync=vSyncCount`（UnityEngine.Application/
QualitySettings 读取），每份 log 自带"等待 vs 真实成本"判定依据。

不做：GameInput patch（`GameInputManager` 类型不存在，UI/输入归残余桶）；
逐 MonoBehaviour patch（patch 风暴污染测量）。

## 4. GC 归因

- 每窗口（5s）Dump 记录：`GC.CollectionCount(0/1/2)` 相对上一窗口的 delta、
  `GC.GetTotalMemory(false)`。
- `RecordFrame`：>10ms 尖刺帧记录该帧内是否发生 collection（进入 RecordFrame 时读一次
  `GC.CollectionCount(0)`，与帧首快照比较），尖刺日志追加 `gc0+`/`gc1+`/`gc2+` 标记。
- 用途：尖刺分为"GC 尖刺"与"逻辑尖刺"。若 GC 尖刺占比显著 → 下一嫌疑是 mod 侧分配
  （Rust interop / 探针 / sim_to_game），那是 mod 唯一现实的可动手项。

## 5. 判定矩阵（拆解后的处置）

| 桶 | 性质 | mod 可动性 |
|----|------|-----------|
| LogicCircuit / SolidConduitFlow / AsyncPathProber 新探针命中 | 原版 C# 逻辑 | ❌ 量化进报告 |
| Unity native 渲染 + GPU 提交 | 引擎 | ❌ |
| GC 频繁回收 / GC 尖刺 | 可能含 mod 分配 | ✅ 查 mod 侧分配热点 |
| 剩余无法归因残余 | 混杂 | >10% 则启动 PlayerLoop 注入备选，否则记录收官 |

## 6. 验收标准

1. 构建通过，进游戏无异常日志（纯测量，游戏行为不变）。
2. 同一场景重跑：RENDERGAP 数值 ≥ v3 口径下的 22%（union 去重后只会变大），
   且各桶之和 + RENDERGAP ≈ FRAME 总量（±探针开销）。
3. 尖刺日志带 GC 标记；窗口行带环境口径。
4. 输出一份对照解读：RENDERGAP 拆解后的各桶占比 + 判定矩阵逐行结论。

## 7. 风险

- Harmony patch `SolidConduitFlow.RenderEveryTick` 等为实例方法，无重载歧义（签名已核实）。
- 深度计数器实现需覆盖"探针方法抛异常导致深度泄漏"——Prefix/Postfix 成对由 Harmony 保证，
  但 Record 内 try 保护深度回退即可（防御性）。
- v3→v4 口径变化：新旧 log 数值不可直接对比，Dump 行加版本号 `v4` 标记。
