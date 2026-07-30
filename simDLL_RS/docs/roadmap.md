# simDLL_RS 总体重写路线图

> 最后更新：2026-07-29
> 当前 Phase：0（基础骨架）

## 总览

| Phase | 名称 | 里程碑 | 状态 |
|---|---|---|---|
| 0 | 基础骨架 | 游戏能进主菜单，日志/dump 可生成 | 🔄 进行中 |
| 1 | 数据层 + 序列化 | 序列化 round-trip 单测通过 | ⏳ 待启动 |
| 2 | 元素系统 | 元素表加载与原版 ID 一致 | ⏳ |
| 3 | 存档加载 ⭐ | 用户能加载存档进入游戏（世界静止） | ⏳ |
| 4 | 存档保存 | 新存档可被原版游戏加载 | ⏳ |
| 5 | 测试基础设施 | 性能基准框架 + 互通性回归测试 | ⏳ |
| 6 | 消息分派骨架 | 日志能看到消息 ID 被正确路由 | ⏳ |
| 7 | 基础模拟 ⭐ | 用户看到温度变化、质量转移 | ⏳ |
| 8 | 流体模拟 | 用户看到水流、气体扩散 | ⏳ |
| 9 | 高级模拟 ⭐ | 辐射/病菌/管道温度工作 | ⏳ |
| 10 | 系统信息 + 性能分析器 | SYSINFO 显示真实信息 | ⏳ 可选 |
| 11 | 优化与验收 ⭐ | 200ms/帧预算达标，24h 稳定 | ⏳ |

## 关键约束

- **ABI 兼容性**：7 个 `SIM_*` + 2 个 `SYSINFO_*` 函数签名与原版一致，C# 侧零修改
- **性能预算**：每帧 sim 更新 ≤ 200ms（正常）/ 66.7ms（3 倍速）
- **部署边界**：用户自行部署 simDLL.dll
- **代码小白用户**：所有设计需向用户解释清楚
- **严格对照源码**：原理不变，允许改造结构但需说明运行时差异

## Phase 详述

### Phase 0 — 基础骨架（当前）
- 源码分段（PowerShell 脚本 + 12 文件 + INDEX.md）
- Rust 项目骨架（Cargo + 11 模块）
- 9 个 `SIM_*/SYSINFO_*` 桩函数（catch_unwind 包裹）
- Logger 双输出（tracing + tracing-appender，文件 + stderr）
- CrashDumpHandler（SetUnhandledExceptionFilter + MiniDumpWriteDump + RtlCaptureStackBackTrace + SwitchToThread 死循环）
- 冒烟测试（10 个集成测试 + 单元测试）

**验收**：
- ✅ `cargo build --release` 产出 SimDLL.dll
- ✅ `cargo test` 全部通过
- ✅ DLL 导出 9 个函数符号
- ✅ 用户部署后游戏能进主菜单
- ✅ `./logs/simdll.log` 能看到初始化日志

### Phase 1 — 数据层 + 序列化
- 完整 `BinaryBufferReader` / `BinaryBufferWriter`（读/写 int/float/bytes/字符串）
- `SimData` / `GameData` / `CellSOA` / `Element` / `Disease` / `SimEvents` / `SimFrameInfo` / `FrameSync` 完整字段
- `Sim` / `SimFrameManager` 类骨架（仅字段，无方法）
- `MessageHandlerEntry` 表结构

**验收**：
- ✅ BinaryBufferReader round-trip 单测通过
- ✅ 所有 struct 大小与原版一致（用 `assert_eq!(size_of::<X>(), 原版字节数)`）

### Phase 2 — 元素系统
- `CreateElementsTable` / `DestroyElementsTable` / `GetElementIndex`
- `CreateElementInteractions` / `CreateElementInteractionsLocked`
- 从 `BinaryBufferReader` 反序列化元素表（C# 侧读 yaml 后序列化成 buffer 传给 simDLL，simDLL 不直接读 yaml）

**验收**：
- ✅ 元素表加载后元素 ID 与原版一致
- ✅ `GetElementIndex(hash)` 返回正确索引
- ✅ 5 个导出函数可被集成测试调用不 panic

### Phase 3 — 存档加载 ⭐（关键里程碑）
- `Load` / `Start` / `AllocateCells` / `DefineWorldOffsets` / `PrepareGameData`
- `ClearUnoccupiedCells` / `CreateElementInteractionsLocked` / `SetSavedOptions`
- `ResizeAndInitializeVacuumCells` / `CreateDiseaseTable` / `CellRead`
- `CleanUp` / `GameSyncFunction` / `PrepareGameDataUpdate`

**验收**：
- ✅ 用户能用原版存档加载到 Rust simDLL
- ✅ 世界静止（无模拟更新），但 cell/element 数据正确显示
- ✅ 进入游戏后画面与原版一致（地形、生物、建筑都在）

### Phase 4 — 存档保存
- `SIM_BeginSave` / `SIM_EndSave` 完整实现
- 与原版 save 文件二进制兼容

**验收**：
- ✅ Rust simDLL 保存的存档能被原版 simDLL 加载
- ✅ 存档字节序、字段顺序与原版完全一致

### Phase 5 — 测试基础设施
- 每帧耗时基准测试（cargo bench + criterion）
- 内存占用监测
- save 互通性回归测试框架
- 24h 长跑稳定性测试框架

**验收**：
- ✅ `cargo bench` 输出每帧 sim 耗时基线
- ✅ save round-trip 回归测试可在 CI 跑

### Phase 6 — 消息分派骨架
- `SimFrameManager::HandleMessage` 实现
- 14 个全局 `gSimMessageHandlers` 注册（指向桩 handler）
- `SIM_HandleMessage` / `SIM_HandleMessages` 调用真实分派

**验收**：
- ✅ 日志能看到每个消息 ID 被正确路由到 handler

### Phase 7 — 基础模拟（温度/质量）⭐（关键里程碑）
- `ProcessCellModifications` / `ProcessCellProperties` / `ProcessMassConsumption` / `ProcessMassEmission`
- `UpdateTemperature` / `CalculateCombinedTemperature`
- `DoPartialHeatTransition` / `DoPartialMelt` / `DoLoadTimeStateTransition`

**验收**：
- ✅ 用户看到温度变化（数字动起来）
- ✅ 质量转移工作（气体扩散、液体流动基础）
- ✅ 每帧 sim 耗时 < 200ms（小地图）

### Phase 8 — 流体模拟
- `UpdateLiquid` / `UpdateFlowTexture` / `IsLiquidPermeable` / `IsSolid`
- `Evaporate` / `PostProcessCell`
- `DoUnstableCheckBasic` / `DoUnstableCheckWithDiagonals`
- `UpdateExposedToSunPropertyTexture` / `UpdateLiquidPropertyTexture`

**验收**：
- ✅ 用户看到水流、气体扩散
- ✅ 蒸发/相变工作

### Phase 9 — 高级模拟 ⭐（关键里程碑）
- `ProcessCellRadiationChanges` / `ProcessConsumeDisease` / `ProcessDigPoints`
- `ProcessBuildingHeatExchangeMessages` / `ProcessCellEnergyModifications`
- `ProcessElementChunkMessages`
- `ConduitTemperatureManager_*` 7 个函数

**验收**：
- ✅ 辐射、病菌、管道温度工作
- ✅ 游戏可玩（建家、挖矿、自动化）

### Phase 10 — 系统信息 + 性能分析器（可选）
- `GetJSONSysInfo` / `Begin` / 各 `ToJSON` 重载
- `KCrashReporterReportMessage` 完整实现
- `kprofiler_*` 16 个函数（按需）

**验收**：
- ✅ SYSINFO 显示真实 CPU/RAM/GPU 信息
- ✅ Crash dump 带完整符号解析

### Phase 11 — 优化与验收 ⭐（最终里程碑）
- 性能剖析：每帧 sim 耗时 < 200ms（正常）/ 66.7ms（3 倍速）
- 内存占用与原版对比
- 长时间运行稳定性（24h 不漏不崩）
- 与原版 save 互通性测试

**验收**：
- ✅ 性能预算达标
- ✅ 24h 长跑无崩溃
- ✅ save 互通性 100%

## 关键决策

1. **每个 Phase 是独立子项目**：各自走 brainstorm → plan → implement。
2. **Phase 1-2 顺序固定**：必须先有数据层才能加载元素表。
3. **Phase 3 是关键里程碑**：第一次有"可见进展"。
4. **Phase 5 测试基础设施独立 Phase**：在 sim 有真实数据可测时建立。
5. **Phase 10 标记为可选**：profiler 不是必需，YAGNI。
6. **每个 Phase 验收包含"用户部署测试"**：用户部署后基于画面表现反馈。

## 开放问题（实施时需验证）

1. **`gGameMessageHandler` 的 C# Marshal 结构**：源码 `(*handler)(0, &local_e0)` 反推为传 `void**`，桩假设为 `[stacktrace_ptr, filename_ptr]` 二指针数组。Phase 0 实施时需反编译 `Assembly-CSharp.dll` 找到 `GameMessageHandler` 实际委托签名验证。
2. **`AppVersion::GetRevision` / `AppVersion::GetBranch`**：原版 `SimDLLUnhandledExceptionFilter` 调用这两个函数拼到 dump 文件名。Phase 0 桩不实现，dump 文件名只用时间戳。Phase 10 可补。
3. **`StackWalk64` vs `RtlCaptureStackBackTrace`**：Phase 0 桩用后者（API 简单，最多 64 帧）。Phase 10 用前者实现完整符号解析。
4. **`SimDLLUnhandledExceptionFilter` 中的 `SymInitialize` 路径**：原版初始化时传 `"./OxygenNotIncluded_Data/Plugins;./Plugins;..."` 多路径搜索符号。Phase 0 桩不调用 SymInitialize，dump 里只有地址。Phase 10 可补。

## 参考链接

- 设计规格：`docs/superpowers/specs/2026-07-29-simdll-rs-foundation-design.md`
- Phase 0 实现计划：`docs/superpowers/plans/2026-07-29-simdll-rs-phase0-foundation.md`
- 源码分段索引：`OxygenNotIncluded_Source/simDLL_edit/INDEX.md`
- 源码：`OxygenNotIncluded_Source/SimDLL_Source.c`（268,587 行）
