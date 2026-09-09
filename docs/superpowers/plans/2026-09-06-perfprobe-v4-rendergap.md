# PerfProbe v4 — RENDERGAP 黑盒拆解 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [x]`）语法来跟踪进度。

**目标：** 把 PerfProbe 的 RENDERGAP（帧内未归因时间）从"含嵌套重复计入的低估下界"修正为 union 口径的真实值，补齐 3 个裸奔方法的探针，并为尖刺加上 GC 归因与环境口径。

**架构：** 全部改动集中在 `SimDLL_PerfProbe/PerfProbe.cs` 单文件。深度计数器（Begin/Record 自动维护）实现"最外层方法独占归因"的 union 语义；账本乒乓交换实现稳态零分配；GC 用 `GC.CollectionCount` delta 归因。纯测量，零行为修改。

**技术栈：** C# 9 / net48 / HarmonyX（0Harmony）/ Unity net48 引用。构建 `dotnet build`；无单测基础设施（Unity 运行时 mod），验证 = 编译通过 + 游戏内验收标准（见规格 §6）。

**规格：** `docs/superpowers/specs/2026-09-06-rendergap-probe-v4-design.md`

---

### 任务 1：Probe 核心 — 深度计数 + union 归因 + 零分配账本

**文件：**
- 修改：`SimDLL_PerfProbe/PerfProbe.cs`（Probe 类与 P_GameUpdate，约 L60-L330）

- [x] **步骤 1.1：Probe 类新增字段与深度语义**

在 `Probe` 类静态字段区（L69-80 附近）新增：

```csharp
[ThreadStatic] 不需要——ONI 主线程单线程调用这些入口
private static int depth; // 已探针方法嵌套深度；==1 表示当前 Record 是最外层方法
private static long lastGc0, lastGc1, lastGc2; // 上一帧/上一窗口的 GC 计数快照
private static long winGc0, winGc1, winGc2; // 窗口起始快照
```

修改 `Begin` 与 `Record`（L89-144）：

```csharp
internal static long Begin()
{
    depth++;
    return Stopwatch.GetTimestamp();
}

internal static void Record(string key, long us)
{
    bool isOutermost = depth == 1;
    depth--; // 与 Begin 配对，先判定后递减
    string k = (IsCameraMoving ? "M." : "S.") + key;
    if (!samples.TryGetValue(k, out var s)) s = new Sample();
    s.totalUs += us;
    s.count++;
    if (us > s.maxUs) s.maxUs = s.maxUs > us ? s.maxUs : us;
    samples[k] = s;

    // union 归因：只有最外层探针方法进帧归因账本（嵌套者的耗时已含在外层里）
    if (isOutermost)
    {
        curFrameMethods.TryGetValue(k, out var prevUs);
        curFrameMethods[k] = prevUs + us;
    }
    // ……尖刺逻辑不变
}
```

注意：`max` 更新写成普通 `if (us > s.maxUs) s.maxUs = us;`（保持原样即可，上面示例里的三元是笔误防护，实施用原式）。

- [x] **步骤 1.2：RecordFrame 零分配账本 + GC 归因**

修改 `RecordFrame`（L165-229）：

1. 尖刺日志（`us > FRAME_SPIKE_THRESHOLD_US` 分支）追加 GC 标记：`" gc0+" + frameGc0 + " gc1+" + frameGc1 + " gc2+" + frameGc2`（仅 delta>0 的段才追加，全零则追加 `gc0=0` 明示"非 GC 尖刺"）。
2. 末尾账本交换（替换 `prevFrameMethods = curFrameMethods; curFrameMethods = new Dictionary<string, long>();`）：

```csharp
var tmp = prevFrameMethods;
prevFrameMethods = curFrameMethods;
curFrameMethods = tmp;
curFrameMethods.Clear(); // 稳态零分配
```

新增字段承接帧级 GC delta（由 P_GameUpdate.Prefix 在调用 RecordFrame 前写入）：

```csharp
internal static long frameGc0, frameGc1, frameGc2; // 本帧间隔内 collection 次数 delta
```

- [x] **步骤 1.3：P_GameUpdate.Prefix — 帧边界职责**

修改 `P_GameUpdate.Prefix`（L294-317）：在 `RecordFrame` 调用前加两件事：

```csharp
depth = 0; // 帧边界深度泄漏防护（异常路径的 Postfix 缺失不会永久污染归因）
long g0 = GC.CollectionCount(0), g1 = GC.CollectionCount(1), g2 = GC.CollectionCount(2);
Probe.frameGc0 = g0 - Probe.lastGc0; Probe.frameGc1 = g1 - Probe.lastGc1; Probe.frameGc2 = g2 - Probe.lastGc2;
Probe.lastGc0 = g0; Probe.lastGc1 = g1; Probe.lastGc2 = g2;
```

- [x] **步骤 1.4：Dump — v4 标记 + 环境口径 + 窗口 GC**

修改 `Dump`（L231-287）窗口行首部：

```csharp
sb.Append("[PerfProbe v4] window=").Append(winMs).Append("ms")
  .Append(" fpsCap=").Append(Application.targetFrameRate)
  .Append(" vsync=").Append(QualitySettings.vSyncCount)
  .Append(" gc0+").Append(GC.CollectionCount(0) - winGc0)
  .Append(" gc1+").Append(GC.CollectionCount(1) - winGc1)
  .Append(" gc2+").Append(GC.CollectionCount(2) - winGc2)
  .Append(" envThreads=").Append(Environment.ProcessorCount)
  .Append(" total=").Append(totalUs / 1000).Append("ms | ");
```

Dump 末尾（`window.Restart()` 之后）刷新窗口 GC 快照：

```csharp
winGc0 = GC.CollectionCount(0); winGc1 = GC.CollectionCount(1); winGc2 = GC.CollectionCount(2);
```

- [x] **步骤 1.5：编译验证**

运行：`dotnet build "C:/Users/KeringFa/Desktop/my_Project/simDLL_RS_Kit_v1/SimDLL_PerfProbe/SimDLL_PerfProbe.csproj" -c Release`
预期：Build succeeded，0 Error（warning 允许）。

- [x] **步骤 1.6：Commit**

```bash
git -C "C:/Users/KeringFa/Desktop/my_Project/simDLL_RS_Kit_v1" add SimDLL_PerfProbe/PerfProbe.cs
git -C "C:/Users/KeringFa/Desktop/my_Project/simDLL_RS_Kit_v1" commit -m "feat(perfprobe): union frame attribution via depth counter, zero-alloc ledger, GC deltas, env line, v4 tag"
```

### 任务 2：补 3 个 patch（LogicCircuit / SolidConduitFlow / AsyncPathProber.Manager）

**文件：**
- 修改：`SimDLL_PerfProbe/PerfProbe.cs`（新增 3 个 HarmonyPatch 类，放在 L451"管道/电路网络"区块附近）

- [x] **步骤 2.1：新增 patch 类**（签名已对反编译源核实：`LogicCircuitManager.RenderEveryTick(float)` L47、`SolidConduitFlow.RenderEveryTick(float)` L1000、`AsyncPathProber.Manager.TickFrame()` L261）

```csharp
[HarmonyPatch(typeof(LogicCircuitManager), "RenderEveryTick", new Type[] { typeof(float) })]
internal static class P_LogicCircuitRenderEveryTick
{
    internal static void Prefix(out long __state) { __state = Probe.Begin(); }
    internal static void Postfix(long __state) { Probe.Record("LogicCircuit.RenderEveryTick", Probe.EndUs(__state)); }
}

[HarmonyPatch(typeof(SolidConduitFlow), "RenderEveryTick", new Type[] { typeof(float) })]
internal static class P_SolidConduitFlowRenderEveryTick
{
    internal static void Prefix(out long __state) { __state = Probe.Begin(); }
    internal static void Postfix(long __state) { Probe.Record("SolidConduitFlow.RenderEveryTick", Probe.EndUs(__state)); }
}

[HarmonyPatch(typeof(AsyncPathProber.Manager), "TickFrame")]
internal static class P_AsyncPathProberTickFrame
{
    internal static void Prefix(out long __state) { __state = Probe.Begin(); }
    internal static void Postfix(long __state) { Probe.Record("AsyncPathProber.TickFrame", Probe.EndUs(__state)); }
}
```

注意：这三个方法都在 Game.Update 调用链内 → 依 union 语义会进 `M./S.` 分桶与尖刺日志，但不进帧归因账本（已含于 Game.Update）。这是预期行为——它们的价值在分桶占比与尖刺归因。

- [x] **步骤 2.2：编译验证**（同任务 1 步骤 1.5 命令）
- [x] **步骤 2.3：Commit**

```bash
git -C "C:/Users/KeringFa/Desktop/my_Project/simDLL_RS_Kit_v1" add SimDLL_PerfProbe/PerfProbe.cs
git -C "C:/Users/KeringFa/Desktop/my_Project/simDLL_RS_Kit_v1" commit -m "feat(perfprobe): probe LogicCircuit/SolidConduitFlow RenderEveryTick and AsyncPathProber.Manager.TickFrame"
```

### 任务 3：部署 Local + 验收

**文件：**
- 复制：`SimDLL_PerfProbe/bin/SimDLL_PerfProbe.dll` → `Local/SimDLL_PerfProbe/SimDLL_PerfProbe.dll`

- [x] **步骤 3.1：部署**

```bash
cp "C:/Users/KeringFa/Desktop/my_Project/simDLL_RS_Kit_v1/SimDLL_PerfProbe/bin/SimDLL_PerfProbe.dll" "C:/Users/KeringFa/Desktop/my_Project/simDLL_RS_Kit_v1/Local/SimDLL_PerfProbe/SimDLL_PerfProbe.dll"
```

- [ ] **步骤 3.2：游戏内验收（用户执行，按规格 §6）**

进游戏（vsync 关）跑 60s（建议同一多星场景）。检查 perf.log：
1. 首行带 `v4` + `fpsCap=-1`（或实际值）+ `vsync=0` + `gc0+/gc1+/gc2+`。
2. 各桶之和 + RENDERGAP ≈ FRAME 总量（±探针开销）；RENDERGAP ≥ v3 同场景 22%。
3. 新键 `LogicCircuit.RenderEveryTick` / `SolidConduitFlow.RenderEveryTick` / `AsyncPathProber.TickFrame` 出现且有数值。
4. 尖刺日志带 gc 标记。
5. 把 perf.log 发回 → 按判定矩阵出对照解读。

- [x] **步骤 3.3：更新 memory 日志**（实施完成后追加本节到 `.workbuddy/memory/2026-09-06.md`）

---

## 自检记录

- 规格覆盖度：§2 union 口径（任务1.1-1.3）、零分配（任务1.2）、§3 补 patch + 环境口径（任务1.4/2.1）、§4 GC 归因（任务1.2-1.4）、§5 判定矩阵（任务3.2 解读环节）、§6 验收（任务3.2）——全部有对应任务。
- 占位符扫描：无 TODO/待定；所有代码步骤含完整代码。
- 类型一致性：`AsyncPathProber.Manager` 已核实（源 L120/L261）；`depth` 判定后递减的次序已在代码中固定；`frameGc*` 字段命名前后一致。
