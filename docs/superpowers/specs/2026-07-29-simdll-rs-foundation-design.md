# simDLL_RS Phase 0：基础设计与路线图

> **状态**：已批准（待用户最终审查）
> **日期**：2026-07-29
> **范围**：Phase 0 子项目规格 — 源码分段 + Rust 项目骨架 + 桩函数 + Logger/CrashDump + 总体重写路线图

---

## 目标

用 Rust 重写《缺氧》游戏原版 `simDLL.dll`，让游戏能正常加载并运行。本项目因规模过大被拆分为多个 Phase，每个 Phase 走独立的「brainstorm → plan → implement」流程。本文档定义 **Phase 0（基础）** 的设计，并给出 Phase 1-11 的总体路线图。

### 关键约束

- **ABI 兼容性**：Rust 端用核心逻辑 + C ABI 适配层导出 7 个 `SIM_*` 函数，签名与原版一致，C# 侧零修改。
- **性能预算**：每帧 sim 更新 ≤ 200ms（正常速度）/ 66.7ms（3 倍速），不影响 C# 端渲染。
- **部署边界**：用户自行部署 `SimDLL.dll`；助手不代部署。
- **代码小白用户**：所有设计需向用户解释清楚，用户只能基于游戏画面表现给出推理逻辑。
- **严格对照源码**：原理不变，允许改造结构但需说明运行时差异。

### 源码文件现状

| 文件 | 行数 | 大小 | 来源 |
|---|---|---|---|
| `OxygenNotIncluded_Source/SimDLL_Source.c` | 268,587 | 8.6 MB | Ghidra 12.1.2 反编译 + SimDLL.pdb 符号加载后导出 |

实际业务逻辑约 5 万行，其余为 MSVCRT 库函数、Windows 异常处理、locale 国际化等 Ghidra 附带导出物。

### 7 个导出 API（C# 通过 P/Invoke 调用）

| 函数 | 原型 | 作用 |
|---|---|---|
| `SIM_Initialize` | `void(GameMessageHandler*)` | 初始化 simDLL，注册 C# 回调 |
| `SIM_Shutdown` | `void()` | 清理 simDLL |
| `SIM_HandleMessage` | `void*(int id, int size, schar* data)` | 处理单条消息，返回响应指针 |
| `SIM_HandleMessages` | `void*(int id, int size, int count, schar* data)` | 批量处理消息 |
| `SIM_BeginSave` | `schar*(int* outSize, int, int)` | 开始序列化存档 |
| `SIM_EndSave` | `void()` | 结束存档 |
| `SIM_DebugCrash` | `void()` | 触发调试崩溃（测试用） |

附加导出：`SYSINFO_Acquire / SYSINFO_Release`（系统信息 JSON）。

---

## §1 源码分段算法

### 输出位置

`C:\Users\KeringFa\Desktop\simDLL_RS_Kit\OxygenNotIncluded_Source\simDLL_edit\`

### 分段算法（确定性、可重跑）

用一次性脚本扫描 `SimDLL_Source.c`，按以下规则切分：

**第 1 步：识别函数边界**
- 匹配 Ghidra 函数标记：`^\s*//\s*0x[0-9a-f]+\s+\d+\s+(\w+)`
- 匹配函数定义：`^(void|int|bool|undefined\d?|dword|qword|longlong|uint|ulong|char|float|double|ushort|byte|schar|wchar16|wchar32|struct\s+\w+|enum\s+\w+)\s+\*?\s*\w+\s*\(`
- 每个函数从其标记/定义行开始，到下一个函数标记前结束

**第 2 步：按函数名前缀归类**

| 文件 | 匹配规则 | 预估行号范围 |
|---|---|---|
| `00_types_reference.c` | 函数标记之前的所有内容（typedef/struct） | 1 ~ 96181 |
| `01_sim_api.c` | `SIM_*`、`SYSINFO_*` 函数 + 内部依赖 | 98220 ~ 98620 |
| `02_save_load.c` | `Load`、`Start`、`AllocateCells`、`DefineWorldOffsets`、`PrepareGameData`、`ClearUnoccupiedCells`、`CreateElementInteractionsLocked`、`SetSavedOptions`、`ResizeAndInitializeVacuumCells`、`CreateDiseaseTable`、`CellRead`、`CleanUp`、`GameSyncFunction`、`PrepareGameDataUpdate` | 96334 ~ 98220 |
| `03_elements.c` | `CreateElementsTable`、`DestroyElementsTable`、`GetElementIndex`、`CreateElementInteractions` | 119291 ~ 126337 |
| `04_temperature.c` | `UpdateTemperature*`、`CalculateCombinedTemperature`、`DoPartialHeatTransition`、`DoPartialMelt`、`DoLoadTimeStateTransition` | 133190 ~ 148813 |
| `05_liquid_flow.c` | `UpdateLiquid*`、`UpdateFlowTexture`、`UpdateLiquidPropertyTexture`、`UpdateExposedToSunPropertyTexture`、`IsLiquidPermeable`、`IsSolid`、`Evaporate`、`PostProcessCell`、`DoUnstableCheck*` | 145676 ~ 149292 |
| `06_process_messages.c` | 所有 `Process*` 函数 + `CellRead` | 115412 ~ 117172 |
| `07_conduit_temperature.c` | `ConduitTemperatureManager_*` 7 个 | 96182 ~ 96340 |
| `08_sim_frame_manager.c` | `Sim::`、`SimFrameManager::` 方法 + `gSimMessageHandlers` 表 | 散落，脚本聚合 |
| `09_crashdump_logger.c` | `InitializeCrashDumpHandler`、`SimDLLUnhandledExceptionFilter`、`KCrashReporterReportMessage`、`Logger::*`、`Timer::*`、`FrameSync::*` | 98741 ~ 98956 + 120320 |
| `10_kprofiler.c` | `kprofiler_*`、`kprofile_*` 16 个 | 168664 ~ 182790 |
| `11_msvcrt_ignored.c` | 剩余所有未归类函数（locale/exception/CRT） | 150000 ~ 268587 |

**第 3 步：处理跨段依赖**
- 若某函数被多段引用（如 `GetElementIndex` 同时被存档和元素系统用），归到主语义段（`GetElementIndex` → `03_elements.c`），其他段在头部注释标注"依赖 03_elements.c:GetElementIndex"
- 共享辅助函数（如 `operator_new`、`operator_delete`）若数量多，归到 `09_helpers.c`

**第 4 步：生成 `INDEX.md`**

```markdown
# simDLL_source 分段索引

## 文件清单
| 文件 | 行号范围 | 函数数 | 状态 | 对应 Rust 模块 |
|---|---|---|---|---|
| 00_types_reference.c | 1-96181 | - | 参考用，不重写 | (无) |
| 01_sim_api.c | 98220-98620 | 9 | 待重写 | sim_api.rs |
| ... | ... | ... | ... | ... |

## 函数清单（按文件分组）
### 01_sim_api.c
- L98220  SIM_BeginSave(int*, int, int) -> schar*       // 0x74e0
- L98388  SIM_DebugCrash(void) -> void                  // 0x7a10
- ...
```

### 完整性校验

脚本执行后输出统计：
- 每个文件行数、函数数
- **完整性校验**：所有文件拼接后行数 = 268587（原文件总行数）
- 如果出现函数未归类，列入 `12_unclassified.c` 并警告

### 关键决策

1. 类型定义不切散：`00_types_reference.c` 保持整体作为参考。
2. MSVCRT 不重写：`11_msvcrt_ignored.c` 整体保留，明确标注"参考用"。Rust 标准库 + `windows` crate 替代。
3. `INDEX.md` 是用户与助手之间的翻译桥梁。

---

## §2 Rust 项目骨架

### 项目位置

`C:\Users\KeringFa\Desktop\simDLL_RS_Kit\simDLL_RS\`

### Cargo 配置

```toml
[package]
name = "simdll_rs"
version = "0.1.0"
edition = "2021"
build = "build.rs"

[lib]
name = "SimDLL"                  # 输出 SimDLL.dll（与原版同名）
crate-type = ["cdylib", "rlib"]  # cdylib 给游戏用，rlib 给测试链接

[dependencies]
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_System_Diagnostics_Debug",   # MiniDumpWriteDump
    "Win32_System_Threading",            # GetCurrentProcess
    "Win32_System_SystemInformation",    # SYSINFO
    "Win32_System_LibraryLoader",        # GetModuleHandle
    "Win32_System_WindowsProgramming",   # EXCEPTION_CONTINUE_SEARCH
    "Win32_Storage_FileSystem",          # CreateFileA
] }
tracing = "0.1"
tracing-appender = "0.2"
tracing-subscriber = "0.3"
parking_lot = "0.12"
once_cell = "1.19"
chrono = "0.4"

[profile.release]
panic = "abort"                        # 防止 panic 跨 C ABI unwind
opt-level = 3
lto = true
codegen-units = 1
strip = "debuginfo"

[profile.test]                         # 测试时保留 unwind
panic = "unwind"
```

### 模块结构

```
simDLL_RS/
├── Cargo.toml
├── build.rs                 # 验证目标平台为 windows-msvc
├── README.md                # 项目总览（一段说明）
├── src/
│   ├── lib.rs               # 入口：模块声明 + 全局静态变量
│   ├── sim_api.rs           # ← 01_sim_api.c：7 个 SIM_* + SYSINFO_* 导出
│   ├── abi.rs               # C ABI 类型：#[repr(C)] struct + 类型别名
│   ├── buffer.rs            # BinaryBufferReader/Writer 桩
│   ├── logger.rs            # Logger singleton
│   ├── crashdump.rs         # InitializeCrashDumpHandler + UnhandledExceptionFilter
│   ├── sim.rs               # Sim struct 桩
│   ├── sim_frame_manager.rs # SimFrameManager 桩
│   ├── frame_sync.rs        # FrameSync 桩
│   ├── message_handler.rs   # MessageHandlerEntry + 分派表
│   └── sysinfo.rs           # SYSINFO + GetJSONSysInfo 桩
├── tests/
│   └── abi_smoke.rs         # 冒烟测试：每个 #[no_mangle] 函数不崩
└── docs/
    └── roadmap.md           # 总体重写路线图
```

### C ABI 约定

- 所有导出函数：`#[no_mangle] pub extern "C" fn SIM_Initialize(...)`
- 所有跨 ABI 结构体：`#[repr(C)]`
- 使用 `*mut T` / `*const T` 处理 raw pointer
- 使用 `std::os::raw::c_*` 类型别名
- 避免在 ABI 边界使用 Rust 专有类型（String/Vec/Box）

### 全局静态变量

```rust
use once_cell::sync::OnceCell;
use parking_lot::Mutex;

pub static G_SIM: OnceCell<Mutex<Option<Sim>>> = OnceCell::new();
pub static G_SIM_DATA: OnceCell<Mutex<Option<SimData>>> = OnceCell::new();
pub static G_ELEMENTS: OnceCell<Mutex<Option<ElementsTable>>> = OnceCell::new();
pub static G_DISEASE: OnceCell<Mutex<Option<DiseaseTable>>> = OnceCell::new();
pub static SAVE_BUFFER: OnceCell<Mutex<Option<Buffer>>> = OnceCell::new();
pub static G_FRAME_SYNC: OnceCell<Mutex<FrameSync>> = OnceCell::new();
pub static G_GAME_MESSAGE_HANDLER: OnceCell<GameMessageHandler> = OnceCell::new();
pub static G_SIM_MESSAGE_HANDLERS: &[MessageHandlerEntry] = &[];  // 桩：空表
```

### build.rs

```rust
fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows-msvc") {
        panic!("simDLL_RS 仅支持 x86_64-pc-windows-msvc 目标，当前目标: {target}");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
```

### 关键决策

1. `crate-type = ["cdylib", "rlib"]`：双输出，cdylib 给游戏用，rlib 给测试用。
2. `panic = "abort"` 在 release，`panic = "unwind"` 在 test：测试需要 panic 可被 catch_unwind 捕获。
3. 依赖最小化：只用 7 个 crate（windows, tracing, tracing-appender, tracing-subscriber, parking_lot, once_cell, chrono），不引入 serde/tokio/anyhow 等大件。
4. 模块名对应分段文件名，便于用户对照阅读。

---

## §3 C ABI 适配层 + 7 个 SIM_* 桩函数

### 设计原则

1. **签名一致**：与原版 `__cdecl` 签名完全一致（C# P/Invoke 不变）
2. **不崩游戏**：游戏加载 DLL、进入主菜单、尝试进入存档都不崩溃
3. **可观测**：每个桩被调用时写日志，方便事后定位

### 桩实现要点

```rust
#[no_mangle]
pub extern "C" fn SIM_Initialize(handler: Option<&GameMessageHandler>) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        Logger::init();
        tracing::info!("SIM_Initialize (stub) — start");
        initialize_crash_dump_handler();
        if let Some(h) = handler {
            let _ = G_GAME_MESSAGE_HANDLER.set(*h);
        }
        let _ = G_FRAME_SYNC.set(Mutex::new(FrameSync::default()));
        tracing::info!("SIM_Initialize (stub) — done");
    }));
}

#[no_mangle]
pub extern "C" fn SIM_HandleMessage(id: i32, size: i32, data: *mut c_schar) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        tracing::debug!(id, size, "SIM_HandleMessage (stub) — returning null");
        std::ptr::null_mut()
    })).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn SIM_BeginSave(out_size: *mut i32, _p2: i32, _p3: i32) -> *mut c_schar {
    catch_unwind(AssertUnwindSafe(|| {
        tracing::warn!("SIM_BeginSave (stub) — save not implemented");
        if !out_size.is_null() { unsafe { *out_size = 0; } }
        std::ptr::null_mut()
    })).unwrap_or(std::ptr::null_mut())
}
```

其余 6 个导出函数（`SIM_Shutdown`、`SIM_HandleMessages`、`SIM_EndSave`、`SIM_DebugCrash`、`SYSINFO_Acquire`、`SYSINFO_Release`）遵循相同模板：

| 函数 | 行为 | 返回值 |
|---|---|---|
| `SIM_Shutdown()` | 写日志，flush Logger | `void` |
| `SIM_HandleMessages(id, size, count, data)` | 循环 count 次记日志，不做实际处理 | `null` |
| `SIM_EndSave()` | 写日志，no-op（SaveBuffer 未分配） | `void` |
| `SIM_DebugCrash()` | 写 warn 日志，**不真正崩溃**（避免误触） | `void` |
| `SYSINFO_Acquire()` | 返回最小 JSON `{"stub":true,"os":"windows","arch":"x86_64"}\0`，用 `leak()` 故意泄漏 | `*mut c_char` |
| `SYSINFO_Release()` | no-op（泄漏内存由进程退出回收） | `void` |

### 冒烟测试

`tests/abi_smoke.rs`：
- `SIM_Initialize(null)` 不崩
- `SIM_HandleMessage(0, 0, null)` 返回 null
- `SIM_BeginSave(&mut size, 0, 0)` 返回 null 且 `size == 0`
- `SIM_Shutdown()` 不崩

### 运行时表现（用户部署后预期）

- ✅ 游戏能启动到主菜单
- ✅ 主菜单 SYSINFO 能显示（简化版）
- ⚠️ "继续游戏"/"加载存档"会失败（sim 未初始化）
- ⚠️ "保存"不可用
- ⚠️ 进入游戏后世界静止（无模拟更新）
- ✅ 一旦出问题，日志文件能告诉助手哪个桩被调用了几次

### 关键决策

1. 每个导出函数都包 `catch_unwind`：双保险，panic 不跨 C ABI 边界。
2. `SYSINFO_Acquire` 用最小 JSON：返回 `{"stub":true,...}`，内存用 `leak()` 故意泄漏（每次约 60 字节，游戏会话调用次数有限）。
3. `SIM_DebugCrash` 桩不真正崩溃：避免误触发。
4. `SIM_BeginSave` 桩返回空：保存功能不可用，但加载主菜单不崩。

---

## §4 Logger + CrashDumpHandler

### 原版行为（来自源码 L98741-L98952）

**SIM_Initialize 中的初始化顺序：**
1. `Timer::Initialize()`
2. 分配并构造 `Logger` 单例
3. `SymInitialize(GetCurrentProcess(), ".;./OxygenNotIncluded_Data/Plugins;./Plugins;./Assets/Plugins/x86;./Assets/Plugins/x86_64", 0)`
4. `InitializeCrashDumpHandler()` — 调用 `SetUnhandledExceptionFilter(SimDLLUnhandledExceptionFilter)`

**`SimDLLUnhandledExceptionFilter` 流程：**
1. 加锁防递归
2. 如果未挂调试器：
   - 生成文件名 `SimDLL_CRASH_YYYYMMDD-HH.MM.SS.dmp`
   - `CreateFileA` 创建文件
   - `MiniDumpWriteDump(GetCurrentProcess(), pid, file_handle, 0)`
   - `CloseHandle`
   - 如果 `gGameMessageHandler` 已注册：
     - `StackWalker::ShowCallstack` 捕获调用栈
     - `CallstackCapturer::OnCallstackEntry` 格式化每帧为 `filename:lineNumber`
     - 调用 `gGameMessageHandler(0, &stacktrace_string)` 发给 C#
     - 无限 `SwitchToThread()` 死循环

### Rust 实现

**`src/logger.rs`：**
- 用 `tracing` + `tracing-appender` 双输出（文件 + stderr）
- 日志文件位置：`./logs/simdll.log`（按天滚动）
- 单例 via `OnceCell<(NonBlocking, WorkerGuard)>`
- `Logger::init()` 在 `SIM_Initialize` 调用
- `Logger::flush()` 在 `SIM_Shutdown` 调用

**`src/crashdump.rs`：**
- `initialize_crash_dump_handler()`：调用 `SetUnhandledExceptionFilter(Some(sim_dll_unhandled_exception_filter))`
- `sim_dll_unhandled_exception_filter`：
  1. `IN_HANDLER.swap(true)` 防递归
  2. `IsDebuggerPresent()` 检查，挂调试器时让出
  3. `build_dump_filename()` 生成时间戳文件名
  4. `write_minidump()` 用 `MiniDumpWriteDump` 写入 `./logs/SimDLL_CRASH_*.dmp`
  5. `capture_stack_trace()` 用 `RtlCaptureStackBackTrace` 抓最近 64 帧
  6. 调用 `gGameMessageHandler(0, &stacktrace)` 通知 C#（结构假设为 `[stacktrace_ptr, filename_ptr]` 二指针数组，基于源码 `local_e0 = &local_70; local_d8 = szFileName; (*handler)(0, &local_e0)` 反推；待实施时对照 C# Assembly-CSharp 端的 Marshal 定义验证）
  7. `tracing::error!` 写崩溃日志
  8. 无限 `SwitchToThread()` 死循环（保留原版行为）

### 关键决策

1. **日志双输出**：文件（按天滚动）+ stderr（控制台实时可见）。
2. **崩溃时双写**：minidump 文件 + simdll.log 里的 stack trace。
3. **死循环保留**：原版崩溃后 `SwitchToThread()` 死循环，避免游戏继续执行造成更严重损坏。
4. **`gGameMessageHandler(0, ...)` 通知 C#**：桩传 `[stacktrace_ptr, filename_ptr]` 二指针数组。**开放问题**：源码 `(*handler)(0, &local_e0)` 反推出传 `void**`，但 C# 侧 `GameMessageHandler` 的实际 Marshal 结构需要在 Phase 0 实施时通过反编译 Assembly-CSharp.dll 验证。即使 C# 解析失败也只是弹窗乱码，不会让游戏更崩。
5. **`VectoredExceptionHandler` 不加**：原版只用 `SetUnhandledExceptionFilter`，桩也只用这个，保持一致。
6. **`StackWalk64` vs `RtlCaptureStackBackTrace`**：桩先用 `RtlCaptureStackBackTrace`（API 简单，最多 64 帧）。后续 Phase 用 `StackWalk64` 实现完整符号解析。

### 路线图后续工作

- 实现 `format_symbol_line`：用 `SymFromAddrW` + `SymGetLineFromAddrW64` 解析函数名/文件名/行号
- 加 `VectoredExceptionHandler` 兜底（可选）
- 兼容 `gGameMessageHandler` 的实际字符串结构（需要看 C# 侧 Marshal 定义）

---

## §5 总体重写路线图

### Phase 列表

| Phase | 名称 | 里程碑 | 验收标准 |
|---|---|---|---|
| 0 | 基础（本次） | - | 游戏能进主菜单，日志/dump 可生成 |
| 1 | 数据层 + 序列化 | - | 序列化 round-trip 单测通过 |
| 2 | 元素系统 | - | 元素表加载与原版 ID 一致 |
| 3 | 存档加载 | ⭐ 能加载原版存档 | 用户能加载存档进入游戏（世界静止） |
| 4 | 存档保存 | - | 新存档可被原版游戏加载 |
| 5 | 测试基础设施 | - | 性能基准框架 + 互通性回归测试 |
| 6 | 消息分派骨架 | - | 日志能看到消息 ID 被正确路由 |
| 7 | 基础模拟 | ⭐ 世界开始动 | 用户看到温度变化、质量转移 |
| 8 | 流体模拟 | - | 用户看到水流、气体扩散 |
| 9 | 高级模拟 | ⭐ 游戏可玩 | 辐射/病菌/管道温度工作 |
| 10 | 系统信息 + 性能分析器 | - | SYSINFO 显示真实信息 |
| 11 | 优化与验收 | ⭐ 可发布 | 200ms/帧预算达标，24h 稳定 |

### Phase 详述

#### Phase 0 — 基础（本次）
- 源码分段（10+2 文件 + INDEX.md：10 业务文件 01-10 + 2 参考文件 00 和 11）
- Rust 项目骨架
- 7 SIM_* + 2 SYSINFO_* 桩函数
- Logger + CrashDumpHandler Rust 实现
- 路线图文档

#### Phase 1 — 数据层 + 序列化
- 完整 `BinaryBufferReader` / `BinaryBufferWriter`
- `SimData` / `GameData` / `CellSOA` / `Element` / `Disease` / `SimEvents` / `SimFrameInfo` / `FrameSync` 完整字段
- `Sim` / `SimFrameManager` 类骨架（仅字段）
- `MessageHandlerEntry` 表结构

#### Phase 2 — 元素系统
- `CreateElementsTable` / `DestroyElementsTable` / `GetElementIndex`
- `CreateElementInteractions`
- 加载 `OxygenNotIncluded_Data/StreamingAssets/elements/gas.yaml`

#### Phase 3 — 存档加载 ⭐
- `Load` / `Start` / `AllocateCells` / `DefineWorldOffsets` / `PrepareGameData`
- `ClearUnoccupiedCells` / `CreateElementInteractionsLocked` / `SetSavedOptions`
- `ResizeAndInitializeVacuumCells` / `CreateDiseaseTable` / `CellRead`
- `CleanUp` / `GameSyncFunction` / `PrepareGameDataUpdate`

#### Phase 4 — 存档保存
- `SIM_BeginSave` / `SIM_EndSave` 完整实现
- 与原版 save 文件二进制兼容

#### Phase 5 — 测试基础设施
- 每帧耗时基准测试（cargo bench + criterion）
- 内存占用监测
- save 互通性回归测试框架
- 24h 长跑稳定性测试框架

#### Phase 6 — 消息分派骨架
- `SimFrameManager::HandleMessage` 实现
- 14 个全局 `gSimMessageHandlers` 注册（指向桩 handler）
- `SIM_HandleMessage` / `SIM_HandleMessages` 调用真实分派

#### Phase 7 — 基础模拟（温度/质量）
- `ProcessCellModifications` / `ProcessCellProperties` / `ProcessMassConsumption` / `ProcessMassEmission`
- `UpdateTemperature` / `CalculateCombinedTemperature`
- `DoPartialHeatTransition` / `DoPartialMelt` / `DoLoadTimeStateTransition`

#### Phase 8 — 流体模拟
- `UpdateLiquid` / `UpdateFlowTexture` / `IsLiquidPermeable` / `IsSolid`
- `Evaporate` / `PostProcessCell`
- `DoUnstableCheckBasic` / `DoUnstableCheckWithDiagonals`
- `UpdateExposedToSunPropertyTexture` / `UpdateLiquidPropertyTexture`

#### Phase 9 — 高级模拟
- `ProcessCellRadiationChanges` / `ProcessConsumeDisease` / `ProcessDigPoints`
- `ProcessBuildingHeatExchangeMessages` / `ProcessCellEnergyModifications`
- `ProcessElementChunkMessages`
- `ConduitTemperatureManager_*` 7 个函数

#### Phase 10 — 系统信息 + 性能分析器（可选）
- `GetJSONSysInfo` / `Begin` / 各 `ToJSON` 重载
- `KCrashReporterReportMessage` 完整实现
- `kprofiler_*` 16 个函数（按需）

#### Phase 11 — 优化与验收
- 性能剖析：每帧 sim 耗时 < 200ms（正常）/ 66.7ms（3 倍速）
- 内存占用与原版对比
- 长时间运行稳定性（24h 不漏不崩）
- 与原版 save 互通性测试

### 关键决策

1. **每个 Phase 是独立子项目**：各自走 brainstorm → plan → implement。
2. **Phase 1-2 顺序固定**：必须先有数据层才能加载元素表。
3. **Phase 3 是关键里程碑**：第一次有"可见进展"。
4. **Phase 5 测试基础设施独立 Phase**：在 sim 有真实数据可测时建立，让后续模拟逻辑开发有性能护栏。
5. **Phase 10 标记为可选**：profiler 不是必需，YAGNI。
6. **每个 Phase 验收包含"用户部署测试"**：用户部署后基于画面表现反馈。

---

## 不在 Phase 0 范围内

- 任何 `Process*` / `Update*` 业务逻辑实现
- `Sim` 类的实例化与运行
- `SimFrameManager::HandleMessage` 的真实分派
- `gSimMessageHandlers` 表的真实 handler 注册
- 元素表加载、存档读写
- `StackWalk64` 完整符号解析（桩用 `RtlCaptureStackBackTrace` 替代）
- `GetJSONSysInfo` 真实 JSON 内容（桩返回最小 JSON）
- `kprofiler_*` 16 个函数

---

## 待实施细节（在 writing-plans 阶段落实）

- 源码分段脚本语言选择（Python 或 PowerShell）
- git 仓库初始化与首次提交策略
- Rust 工具链版本要求（建议 stable 1.80+）
- 构建命令格式（`cargo build --release --target x86_64-pc-windows-msvc`）
- 单元测试运行方式（`cargo test`）
- 日志文件路径与权限
- 桩函数返回的 minidump 文件大小限制

## 开放问题（实施时需验证）

- **`gGameMessageHandler` 在崩溃通知时的 C# Marshal 结构**：源码 `(*handler)(0, &local_e0)` 反推为传 `void**`，桩假设为 `[stacktrace_ptr, filename_ptr]` 二指针数组。实施时需反编译 `Assembly-CSharp.dll` 找到 `GameMessageHandler` 的实际委托签名验证。
- **`AppVersion::GetRevision` / `AppVersion::GetBranch`**：原版 `SimDLLUnhandledExceptionFilter` 调用这两个函数拼接到 dump 文件名。桩暂不实现，dump 文件名只用时间戳。后续可补。

---

## 参考

- 项目说明：`simDLL_RS项目说明.txt`
- 源码：`OxygenNotIncluded_Source/SimDLL_Source.c`（268,587 行）
- 原版 DLL：`OxygenNotIncluded_Data/Plugins/x86_64/SimDLL.dll`
- 游戏数据：`OxygenNotIncluded_Data/StreamingAssets/`
- 用户规则：允许改造代码结构（前提原理不变），需详细说明改动与游戏运行时体验差异；用户是代码小白；助手不帮用户部署
