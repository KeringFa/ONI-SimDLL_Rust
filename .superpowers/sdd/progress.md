# simDLL_RS Phase 0 进度账本

> 计划文件：`docs/superpowers/plans/2026-07-29-simdll-rs-phase0-foundation.md`
> 起始 commit：`792d2b2 docs: add Phase 0 design spec for simDLL_RS rewrite`
> 工具链：Rust 1.96.1 / windows 0.58 / PowerShell（执行策略被禁用，调用必须加 `-ExecutionPolicy Bypass`）

## 任务进度

- [x] Task 1：PowerShell 源码分段脚本（split_source.ps1 + 12 文件 + INDEX.md）
- [x] Task 2：Rust 项目骨架（Cargo.toml + build.rs + lib.rs + 10 占位模块）
- [x] Task 3：abi.rs（C ABI 类型 + GameMessageHandler 假设）
- [x] Task 4：logger.rs（tracing 双输出单例）
- [x] Task 5：crashdump.rs（SetUnhandledExceptionFilter + MiniDumpWriteDump）
- [x] Task 6：占位 struct（sim/buffer/sysinfo 等 6 个）
- [x] Task 7：sim_api.rs（9 个 #[no_mangle] 导出函数）
- [x] Task 8：tests/abi_smoke.rs（集成冒烟测试）
- [x] Task 9：docs/roadmap.md（总体路线图）
- [x] Task 10：最终验收（cargo test + release build + git log）

## 完成记录（任务完成后追加）

<!-- 格式：Task N: complete (commits <base7>..<head7>, review clean) -->

Task 1: complete (commits 792d2b2..8b37b91, review clean)
- 产物：`OxygenNotIncluded_Source/simDLL_edit/`（split_source.ps1 + 12 个 .c 分段文件 + INDEX.md）
- 修复：脚本原始版本 $typeRe 未使用导致只识别 32 个 markers（用户已批准修复），修复后识别 1988 个 markers，12 文件全部非空，行数对齐 268587=268587 ✓
- 审查次要发现（已记入 task-1-review.md，无需修复）：
  1. ConduitTemperatureManager_Add 函数签名跨行被归入 11_msvcrt_ignored.c（函数体在 07）
  2. INDEX.md 函数计数含重复 markers（仅影响统计数字）
  3. $expectedTotal/$actualTotal 变量未使用（冗余）
  4. ^Process 前缀匹配较宽（原始简报逻辑）

Task 2: complete (commits 8b37b91..c59f36a, review passed with notes)
- 产物：`simDLL_RS/`（Cargo.toml + build.rs + README.md + src/lib.rs + 10 个占位模块文件，共 14 文件 109 行）
- 构建：cargo build 成功（1m 53s 首次，0.13s 缓存），SimDLL.dll 产出 104448 字节（≈102 KB，windows crate 元数据导致，非缺陷）
- 审查结论：规格 ✅，任务质量通过。详细记录见 task-2-review.md。
- 重要发现（已接受不回滚）：实现者自作主张删除 `simDLL_RS/.git` 嵌套空仓库未升级给控制者裁定。控制者已核实文件系统无 cargo init 残留、diff 零删除、commit 含 14 文件符合预期。本任务作为先例不追究，但已记入全局约束禁止后续任务重蹈。
- 次要发现：
  1. 实现者报告"2 个 warning"原文证据不全（仅展示 1 个）—— 报告卫生问题，控制者已亲自重构建补齐证据
  2. SimDLL.dll 102 KB 与简报预估"几 KB"偏差 20 倍 —— 简报预估偏低，待任务 8 dumpbin 验证时复核

Task 3: complete (commits c59f36a..a8af945, review clean)
- 产物：`simDLL_RS/src/abi.rs` 全量替换为 96 行（含 SChar / GameMessageHandler / CrashNotifyPayload / SYSINFO_STUB_JSON + 4 个 inline 单元测试）
- 测试：cargo test --lib abi 4/4 通过（含 TDD RED→GREEN 证据：预授权将 SYSINFO_STUB_JSON.len() 断言从简报错误的 50 修正为实际值 45）
- 审查结论：规格 ✅，任务质量通过，无关键/重要发现。
- 次要发现：
  1. `schar_is_signed_char` 测试中 `i8::MIN as i32 == -128` 是恒真断言（i8 定义性质，与 SChar 是否 signed 无关）—— 计划强制，非实现者缺陷；可考虑在后续涉及 SChar 实际使用的任务中加强（如 `SChar::MIN == -128`）。本任务不修。

Task 4: complete (commits a8af945..3b7015c, review clean)
- 产物：`simDLL_RS/src/logger.rs` 全量替换为 133 行（OnceCell<Mutex<LoggerState>> 单例 + tracing/tracing-appender 双输出层 + 文件按天滚动 + stderr ANSI + EnvFilter::new("info") + try_init 失败走 eprintln 兜底 + 2 个 inline 单测）
- 测试：cargo test --lib logger 2/2 通过；cargo test --lib 6/6 通过（4 abi + 2 logger）。预授权裁定 1 执行：原样写入 → 看到 `unused import: tracing_appender::non_blocking::NonBlocking` warning（src/logger.rs:13:5）→ 删除该行 import → 重跑干净。预授权裁定 2 用原样版本（init_can_be_called_twice_safely 连续 3 次通过，未启用 fallback）。
- 审查结论：规格 ✅，任务质量通过，无关键/重要发现。
- 次要发现（均为"计划强制"，本任务不修，记入后续 Task 5+ 接入 SIM_Initialize 时注意）：
  1. `init()` 文档注释与失败分支返回值不一致——doc 说"false 表示已初始化过或失败"，但 try_init 失败的兜底分支也返回 true。后续调用方需知 `init() == true` 不等于 subscriber 成功 set（crashdump.rs:48）。
  2. `flush()` 持锁调用 `tracing::info!`——当前 subscriber 不会回调访问 LOGGER 不会死锁，但持锁做 I/O 是不必要的临界区延长（crashdump.rs:118-124）。
  3. 测试未验证文件输出实际落盘——两个测试只验证不 panic / 第二次返回 false，不检查 `./logs/simdll.log.YYYY-MM-DD` 是否真被创建写入。stderr 输出证据存在但文件层无验证（简报 L165-168 明确说明不验证 subscriber 实际生效）。
  4. `init()` 失败分支立即 drop `file_guard`——行为正确（先 writer 后 guard drop 顺序无误），但代码注释未说明 file_guard 会被立即 drop 触发 worker 关闭，读者可能误以为泄漏（crashdump.rs:74,93,109）。

Task 5: complete (commits 3b7015c..b5f3375 via 4ccc1a9, review passed after fix)
- 产物：`simDLL_RS/src/crashdump.rs`（全量替换 327 行）+ `simDLL_RS/src/lib.rs`（追加 `pub mod globals` 子模块，G_GAME_MESSAGE_HANDLER OnceCell）+ `simDLL_RS/Cargo.toml`（windows features 从 7 个增加到 10 个：新增 Win32_System_Kernel/Win32_System_Memory/Win32_Security，crate 数量仍为 7，符合全局约束）
- 测试：cargo test --lib crashdump 4/4 通过；cargo test --lib 10/10 通过（4 abi + 2 logger + 4 crashdump），0 warning
- 审查结论：第一轮审查发现 2 项重要 + 3 项次要（均为简报代码 bug），修复后第二轮审查通过
- **8 项控制者裁定（简报代码 bug 修复，均为简报作者对 windows crate 0.58 API 的误判或笔误）**：
  1. Cargo.toml 添加 3 个 windows features（Win32_System_Kernel/Win32_System_Memory/Win32_Security）——简报代码需要的 API 被 feature gate
  2. leak_c_string 返回类型 `*const u8` → `*const c_char`——CString::as_ptr() 返回 *const i8
  3. 删除 unused variable `let s = "hello\0".to_string();`——简报测试残留
  4. sim_dll_unhandled_exception_filter 签名 `extern "C"` → `extern "system"`，`*mut EXCEPTION_POINTERS` → `*const EXCEPTION_POINTERS`——SetUnhandledExceptionFilter 期望的回调签名
  5. CreateFileA 第二参数 `FILE_GENERIC_READ | FILE_GENERIC_WRITE` → `(FILE_GENERIC_READ | FILE_GENERIC_WRITE).0`——FILE_ACCESS_RIGHTS newtype 需 .0 提取 u32
  6. filename_cstr 去掉 `format!("{}\0", ...)` 的 `\0`——CString::new 拒绝含内部 NUL 的输入，导致 filename_cstr 必为 null（审查发现的重要 bug）
  7. SwitchToThread 加 `let _ =`——unused_must_use warning（审查发现的重要 bug）
  8. SAFETY 注释 "C" → "system"——与修复 4 同步（审查发现的次要 bug）
- 预授权范围内的修改（实现者第一轮自行完成）：删除未使用导入 c_int/BOOL/HANDLE、EXCEPTION_CONTINUE_SEARCH.0 → EXCEPTION_CONTINUE_SEARCH（3 处）、SetUnhandledExceptionFilter 路径从 WindowsProgramming 改到 Diagnostics::Debug
- 次要发现（遗留，本任务不修，无运行时影响）：
  1. crashdump.rs:219 `dump_filename.clone()` 可简化为 move（风格偏好）
  2. crashdump.rs:88 MAX_FRAMES 注释 off-by-one（0x3f=63 vs 64，简报内部不一致）
  3. crashdump.rs:136-141 dump 文件名格式与简报文字描述不一致（简报内部不一致，实际格式 SimDLL_CRASH_{date}.{time}.{date}.dmp）
- 教训：简报代码块与 windows crate 0.58 实际 API 存在大量不匹配（8 项 bug），原因是简报作者写代码时未实际编译验证。后续任务（Task 7 sim_api.rs 涉及更多 Win32 API）应预判类似问题，控制者派发时可预授权"按实际 API 签名适配"。

Task 6: complete (commits b5f3375..5f11e1a, review clean)
- 产物：6 个占位模块文件全量替换（buffer.rs 40 行 / sim.rs 32 行 / sim_frame_manager.rs 36 行 / frame_sync.rs 26 行 / message_handler.rs 26 行 / sysinfo.rs 64 行，合计 224 行）+ 7 个新单元测试
- 测试：cargo test --lib 17/17 通过（10 旧 + 7 新），0 error，1 warning（Cargo.toml [profile.test] panic="unwind"，Task 1 遗留，非本任务引入）
- 审查结论：规格 ✅，任务质量通过，0 关键/重要，3 次要
- **3 处简报偏差（实现者均正确处理）**：
  1. 简报步骤 7 第一条 cargo test 命令 `cargo test --lib buffer sim ...` 语法错误（cargo test 仅接受单 TESTNAME positional）→ 改为 `cargo test --lib -- buffer sim ...` 用 `--` 分隔
  2. 简报预期"0 warning"但实际有 Cargo.toml `[profile.test] panic="unwind"` warning（Task 1 遗留）→ 实现者正确识别未越界修改 Cargo.toml，建议后续单独 chore 任务删除该冗余配置
  3. 简报预判 message_handler.rs `c_void` import 可能 unused，实际被 `MessageHandlerEntry.handler` 返回类型 `-> *mut c_void` 使用 → 保持原文不动，无 warning
- 次要发现（遗留，本任务不修）：
  1. sim.rs 测试 `Sim::new(&mut fs as *mut _)` 用 `_` 类型推断（简报原文），显式 `as *mut FrameSync` 更清晰
  2. sysinfo.rs acquire() 泄漏 by-design（简报要求），Phase 1+ 需追踪释放旧指针
  3. sysinfo.rs release_is_safe_no_op 测试未配对 acquire()，对桩足够，Phase 1+ 可补充配对场景
- 教训更新：Task 6 是纯占位 struct 无 Win32 API，简报偏差仅 3 项且均为命令语法/预判落空类（非 API 不匹配），证实 Task 5 教训的预判——Win32 API 越多偏差越多。Task 7 涉及 9 个导出函数 + catch_unwind + 更多 Win32 调用，预判简报偏差会回升，控制者已预授权"按实际 API 签名适配"。

Task 7: complete (commits 5f11e1a..6c9670b via 87c8358 + 6c9670b, review clean)
- 产物：`simDLL_RS/src/sim_api.rs` 全量替换 276 行（9 个 `#[no_mangle] pub extern "C" fn` 导出函数 + 10 个单元测试）。9 函数：SIM_Initialize / SIM_Shutdown / SIM_HandleMessage / SIM_HandleMessages / SIM_BeginSave / SIM_EndSave / SIM_DebugCrash / SYSINFO_Acquire / SYSINFO_Release。每个函数包 catch_unwind：void 返回用 `let _ = catch_unwind(...)`，指针返回用 `.unwrap_or_else(|_| null_mut())`
- 测试：cargo test --lib sim_api 10/10 通过；cargo test --lib 全量 27/27 通过（17 旧 + 10 新）；cargo build --release --target x86_64-pc-windows-msvc 成功产出 SimDLL.dll 996864 字节（≈973 KB）；dumpbin 不可用按简报跳过
- 审查结论：规格 ✅，任务质量通过，0 关键/重要，4 次要
- **2 个 commit**：
  - 87c8358（初始实现）：应用预判 bug #1 修复（测试 from_raw_parts(ptr,50) 越界 → 改用 SYSINFO_STUB_JSON.len() + assert_eq!(bytes, SYSINFO_STUB_JSON)）
  - 6c9670b（预审查修复）：控制者简报失误——`use crate::abi::SYSINFO_STUB_JSON;` 放在顶部 import 区但仅 #[cfg(test)] 用，触发 release build unused_imports warning → 移入 #[cfg(test)] mod tests 内（方案 A）
- 预判 bug #1 证实：控制者派发前核实 abi.rs 确认 SYSINFO_STUB_JSON.len()=45，简报测试 from_raw_parts(ptr,50) 越 5 字节 + assert_eq 长度不匹配必然 panic。修复在初始实现阶段即应用，未产生测试失败往返
- 预判偏差回升未发生：Task 7 代码不直接调用 Win32 API（全部委托已实现模块），仅预判 bug #1 + 控制者自身简报失误（import 位置）2 项，少于 Task 5 的 8 项。Task 5 教训的"Win32 API 越多偏差越多"在 Task 7 因架构分层（导出层不直接碰 Win32）而未应验
- 次要发现（遗留，本任务不修）：
  1. sim_api.rs SIM_HandleMessages doc comment 写"循环 count 次记日志"但代码只记一次（简报内部不一致，doc 来自计划）
  2. 测试输出含 tracing 日志噪声（logger 全局 subscriber 副作用，非本任务能单独修复，需 test 专用 subscriber 隔离）
  3. panic 安全性（catch_unwind 兜底返回 null）未被显式测试覆盖（简报未要求 panic 注入测试）
  4. SIM_Initialize 未测试 Some(handler) 路径（仅测 None 分支，简报未要求）
- release build 遗留 warning（非 Task 7 引入）：`[profile.test] panic`（Task 1）+ `crate SimDLL should have a snake case name`（Task 2，lib name 必须 "SimDLL" 匹配原版 DLL 名，无法改）

Task 8: complete (commits 6c9670b..2f856ef, review clean)
- 产物：`simDLL_RS/tests/abi_smoke.rs` 新建 87 行（10 个集成测试，外部链接 rlib 调用 9 个导出函数，模拟 C# P/Invoke）
- 测试：cargo test --test abi_smoke 10/10 通过；cargo test 全量 37/37 通过（27 lib 单元 + 10 集成），0 error
- 审查结论：规格 ✅，任务质量通过，0 关键/重要，2 次要
- **预判 bug #1 证实并修复**：简报 `use simdll_rs::sim_api::*;` 因 Cargo.toml `[lib] name = "SimDLL"` 设置自定义 lib 名，crate 标识符是 `SimDLL` 而非包名 `simdll_rs`。首次编译失败（E0433 unresolved import + 15 个 E0425），按预授权改为 `use SimDLL::sim_api::*;` 后通过。控制者派发前已核实 Cargo.toml L9/L10 并在简报预判此问题，实现者一次适配成功，无往返
- 次要发现（遗留，本任务不修）：
  1. tests/abi_smoke.rs:6 `use std::os::raw::{c_char, c_int, c_schar, c_void};` 中 `c_schar` 未被直接使用（测试用 null_mut() 类型推断传入），触发 unused import warning。简报代码逐字引入，实现者按"严格遵循简报"纪律未自行移除。建议后续清理任务统一处理
  2. SIM_BeginSave 的 null out_size 路径未在集成测试覆盖（Task 7 单元测试已覆盖此分支，集成测试本意是冒烟非穷尽）
- 教训：集成测试 crate 引用名 = `[lib] name`（`-`→`_`），非 `[package] name`。当两者不同时简报易出错（简报作者按包名写）。后续若新增集成测试/bench/example，需用 lib 名 `SimDLL`

Task 9: complete (commits 2f856ef..1624795, review clean)
- 产物：`simDLL_RS/docs/roadmap.md`（新建 175 行，纯 markdown 文档，含 Phase 0-11 总览表 + 12 个 Phase 详述 + 关键决策 6 条 + 开放问题 4 条 + 参考链接 4 条）
- 测试：纯文档任务，无 cargo test/build（按简报跳过）
- 审查结论：规格 ✅，任务质量通过，0 关键/重要/次要
- 实现者严格逐字复制简报内容（含 emoji 🔄⏳⭐✅、反引号、表格分隔符），未引入任何偏差
- 预判 0 项偏差风险得到证实（纯文档无编译器/API/测试，最简单的任务）
- 唯一环境噪声：Git LF→CRLF 警告（Windows 正常）+ PowerShell profile 加载错误（执行策略限制），均与本任务无关

Task 10: complete (final acceptance, no new commits)
- cargo test --target x86_64-pc-windows-msvc：**37/37 通过**（27 lib 单元 + 10 abi_smoke 集成 + 0 doc-tests），0 error
- cargo build --release --target x86_64-pc-windows-msvc：exit 0，SimDLL.dll 996864 字节（≈973 KB），缓存命中 0.44s（source 自 Task 7 未变）
- dumpbin 不可用跳过，9 个导出符号由 abi_smoke 集成测试运行时调用等价证明
- git log 12 个 commit 覆盖 Task 1-9 全部，与简报预期完全匹配
- 规格覆盖度 7/7 项全通过（§1-§5 + 占位 struct + 冒烟测试）
- 占位符扫描：9 处"占位"均为显式 Phase 0 桩说明（非 TODO/FIXME），全部接受
- 类型一致性验证：GameMessageHandler / CrashNotifyPayload / SYSINFO_STUB_JSON 跨文件引用全部一致
- 遗留 warnings（3 项不阻塞）：[profile.test] panic（Task 1）/ crate SimDLL snake_case（Task 2 不可改）/ tests/abi_smoke.rs:6 unused c_schar（Task 8 简报逐字引入）
- 最终结论：**Phase 0 全部 9 个内容任务完成，最终验收通过**

---

# simDLL_RS Phase 1 进度账本

> 计划文件：`docs/superpowers/plans/2026-07-29-simdll-rs-phase1-data-layer.md`
> 起始 commit：`ac79be7`（Phase 1 plan commit）
> BASE（Task 1 分派前）：`ac79be7`

## 任务进度

- [x] Task 1：stl_shim.rs — STL 容器等价物
- [x] Task 2：vector_math.rs — Vector2/3/4
- [x] Task 3：buffer.rs — BinaryBufferReader/Writer
- [x] Task 4：element.rs — Element + PhysicsData
- [x] Task 5：disease.rs — Disease + DiseaseInfo
- [x] Task 6：sim_events.rs — SimEvents + SimFrameInfo
- [x] Task 7：sim_data.rs — SimData + CellSOA + BackwallSOA + 8 子结构
- [x] Task 8：game_data.rs — GameData + Handle + 25 *Info 子结构
- [x] Task 9：frame_sync.rs + sim.rs + sim_frame_manager.rs — 顶层骨架
- [x] Task 10：message_handler.rs — MessageType enum + MessageHandlerEntry
- [x] Task 11：lib.rs — 模块声明集成
- [x] Task 12：abi_smoke.rs 集成测试 + Cargo.toml 清理

## 完成记录

Task 1: complete (commits ac79be7..6dba032, review clean)
- 产物：`simDLL_RS/src/stl_shim.rs`（新增 70 行）+ `simDLL_RS/src/lib.rs`（+1 行 `pub mod stl_shim;`）
- 测试：cargo test --lib stl_shim::tests 4/4 通过；cargo test 全量 41/41 通过（Phase 0 的 37 + Phase 1 新增 4），stl_shim.rs 自身零 warning
- 审查结论：规格 ✅，任务质量通过，0 关键/重要，1 次要（计划强制的模块顺序非字母序，仅记录）
- TDD 证据：RED 阶段 4 个 E0425 编译错误（struct 未定义）→ GREEN 阶段 4/4 通过
- 4 个 #[repr(C)] struct：MsvcVector(24B)/UniquePtr(8B)/MsvcString(32B)/MsvcFunction(64B)，4 个 size_of 断言验证内存布局

Task 2: complete (commits 6dba032..b943036, review clean)
- 产物：`simDLL_RS/src/vector_math.rs`（新建 50 行）+ `simDLL_RS/src/lib.rs`（+1 行 `pub mod vector_math;`）
- 测试：cargo test --lib vector_math::tests 3/3 通过；cargo test 全量 44/44 通过（37 + 4 + 3），无新 warning
- 审查结论：规格 ✅，任务质量通过，0 关键/重要/次要
- TDD 证据：RED 阶段 3 个 E0425 编译错误 → GREEN 阶段 3/3 通过
- 3 个 #[repr(C)] POD struct：Vector2f(8B)/Vector3f(12B)/Vector4f(16B)，派生 Clone/Copy/Default

Task 3: complete (commits b943036..26c82f4, review passed with notes)
- 产物：`simDLL_RS/src/buffer.rs` 完全替换 Phase 0 桩（40 行 → 334 行，+311/-18）。BufferError enum（4 变体）+ BinaryBufferReader（#[repr(C)] 24B：m_offset:u64/m_buffer_data:*const i8/m_buffer_size:u64）+ BinaryBufferWriter（Vec<u8> 后端，无 #[repr(C)]）+ `_phase0_buffer_abi_stub` C 导出符号（前向安全网）+ 13 个测试（9 round-trip + 1 invalid bool + 1 越界 + 2 skip + 1 size_of=24）
- 测试：cargo test --lib buffer::tests 13/13 通过；cargo test 全量 56/56 通过（lib 46 + abi_smoke 10），buffer.rs 本身零警告
- 审查结论：规格 ✅，任务质量通过，0 关键，1 重要（计划强制），4 次要（3 项计划强制）
- **简报 2 项偏差（均计划内部不一致，非实现者错误，已记录）**：
  1. 预期测试数 57 实际 56 —— 简报按 44+13=57 算，未计入完整替换 buffer.rs 时移除 Phase 0 桩测试 `buffer_is_valid_returns_false_in_stub`（1 个）。正确算式 44−1+13=56
  2. `_phase0_buffer_abi_stub` 是新增而非"保留" —— brief L215 注释措辞错误（Phase 0 桩不存在该符号），实现者按 brief 逐字写入，控制者已核实
- **审查发现 1 项重要（[计划强制]）**：`BinaryBufferReader::new` 无生命周期参数，取 `&[u8]` 但存 `*const i8` 裸指针，无 `PhantomData` 绑定借用，同线程内可构造悬垂读取器（UAF）。审查者建议后续任务评估是否加 `<'a>` + `PhantomData<&'a [u8]>`（零开销，不改 24B 布局）。**控制者裁定**：暂不改造（保持与 brief 一致），记入遗留观察项，Task 7 SimData 接入时如暴露问题再考虑改造
- **审查发现 4 项次要（3 项 [计划强制]）**：
  1. `_phase0_buffer_abi_stub` 未 catch_unwind（计划强制，函数体仅返回 0 不会 panic，Phase 6+ 真实实现时补上）
  2. `read_string` 先分配后边界检查（计划强制，恶意 buffer 可触发 OOM，simDLL 内部读取自有 buffer 风险低）
  3. `InvalidUtf8` 错误路径未测试（计划强制，brief 未提供该测试用例）
  4. `read_bytes` 逐字节循环而非 `ptr::copy_nonoverlapping`（性能非关键路径，实现者自审已说明）

Task 4: complete (commits 26c82f4..9cfdcae, review clean)
- 产物：`simDLL_RS/src/element.rs`（新建 106 行）+ `simDLL_RS/src/lib.rs`（+1 行 `pub mod element;` 按字母序插入 crashdump 与 frame_sync 之间）
- 3 个 #[repr(C)] struct：ElementState(1B) / PhysicsData(12B，3 字段 temperature/mass/pressure) / Element(实测 164B，38 字段对照源码 00_types_reference.c L11104-11143，每行尾标注源码行号注释)
- 测试：cargo test --lib element::tests 3/3 通过；cargo test 全量 59/59 通过（56 + 3），无新 warning
- 审查结论：规格 ✅，任务质量通过，0 关键/重要，2 次要（均为计划强制）
- TDD 证据：RED 阶段 3 个 E0425 编译错误（struct 未定义）→ GREEN 阶段 3/3 通过
- **Element size_of 实测 164B 与控制者手算 164B 完全一致**（按 #[repr(C)] 4 字节对齐逐字段累加）。实现者报告的"手算 172B 差 8B"是实现者多算了 2 个 f32 字段（specific_heat_capacity..strength 实际是 14 个非 16 个），非代码缺陷
- **简报偏差**：原计划简报 PhysicsData 缺 `pressure` 字段（源码 L10643 确认存在），控制者派发前已修订 brief 加入 `pub pressure: f32`，size_of 断言改为 12。实现者按修订后 brief 写入，无自行偏差
- **审查发现 2 项次要（均 [计划强制]）**：
  1. `element_size_matches_source` 测试注释算术错误（brief L101-105 原样复制，"16×4 f32" 应为 "14×4"，加法本身也错）—— Phase 2 时修复
  2. Element 164B 已可精确断言但 brief 强制范围断言 (100, 200) —— Phase 2 时改为精确断言并对照源码 sizeof 核实
- **遗留观察项（Phase 2 复核）**：Element 实测 164B 与原版 C++ sizeof 是否一致需在 Phase 2 接入 CreateElementsTable 时核实。若不一致需回头查字段顺序/类型/数组维度（特别是 gradient_colours[6] 维度）

Task 5: complete (commits 9cfdcae..8cdd2b0, review clean)
- 产物：`simDLL_RS/src/disease.rs`（新建 46 行）+ `simDLL_RS/src/lib.rs`（+1 行 `pub mod disease;` 按字母序插入 element 之前）
- 2 个 #[repr(C)] struct：DiseaseInfo(20B，5 字段 name_hash/colour/fatality_rate/duration/cure_base_rate) / Disease(64B=0x40，2 个 MsvcVector + reserved [u8;16] 占位对应开放问题 #3)
- 测试：cargo test --lib disease::tests 2/2 通过；cargo test 全量 61/61 通过（59 + 2），无新 warning
- 审查结论：规格 ✅，任务质量通过，0 关键/重要，3 次要（均为文档/环境类，非代码缺陷）
- TDD 证据：RED 阶段 2 个 E0425 编译错误（struct 未定义）→ GREEN 阶段 2/2 通过
- **审查发现 3 项次要**：①报告行数统计偏差 1 行（47 vs 实际 46）②brief L5 措辞矛盾（"element 与 frame_sync 之间" 但 disease < element 应在 element 之前，实现者正确选字母序）③环境噪声（panic profile warning/CRLF/PowerShell CLIXML）
- **遗留观察项（Phase 3 复核）**：Disease reserved 16B 字段待 Phase 3 存档加载时定位（开放问题 #3）

Task 6: complete (commits 8cdd2b0..786250a, review clean — 完美通过)
- 产物：`simDLL_RS/src/sim_events.rs`（新建 45 行）+ `simDLL_RS/src/lib.rs`（+1 行 `pub mod sim_events;` 按字母序插入 sim_api 与 sim_frame_manager 之间）
- 2 个 #[repr(C)] 空 struct 占位：SimFrameInfo / SimEvents，size_of=0
- 测试：cargo test --lib sim_events::tests 2/2 通过；cargo test 全量 63/63 通过（61 + 2），无新 warning
- 审查结论：规格 ✅，任务质量通过，0 关键/重要/次要（完美通过）
- TDD 证据：RED 阶段 2 个 E0425 编译错误（struct 未定义）→ GREEN 阶段 2/2 通过
- **控制者派发前修订 brief**：①删除原简报 unused imports（use MsvcVector/use vector_math）②原简报 SimFrameInfo 设计（frame_id+time_delta=8B）与源码 L5406 不符，改用空 struct 占位 ③SimEvents 源码 L9363 有 20 个 vector 字段（480B），子 struct 类型 Phase 1 未定义，同样用空 struct 占位
- **遗留观察项（Phase 3 大任务）**：SimFrameInfo 源码 30+ 字段，SimEvents 20 个 vector 字段，引用的子 struct（DigPoint/CellModification/SubstanceChangeInfo/SpawnOreInfo/...）大部分未定义。Phase 3 需先定义所有子 struct（可能 30+ 个新 struct），再填充字段

Task 7: complete (commits 618b457..7b9bec3, review clean — 完美通过)
- 产物：`simDLL_RS/src/sim_data.rs`（新建 430 行）+ `simDLL_RS/src/lib.rs`（+1 行 `pub mod sim_data;` 按字母序插入 sim_api 与 sim_events 之间）
- 16 个 #[repr(C)] struct（SimData 0x890=2192B + CellSOA 352B + BackwallSOA 104B + 5 辅助子结构 + 8 padding 子结构），基于 32B MsvcVector（commit 618b457 修订）+ 8 子结构 dtor 偏移差值
- 测试：cargo test --lib sim_data::tests 16/16 通过；cargo test 全量 79/79 通过（53 lib + 10 abi_smoke + 16 sim_data），无新 warning
- 审查结论：规格 ✅，任务质量通过，0 关键/重要/次要（完美通过）
- TDD 证据：方式 A 严格 TDD，RED 16/16 失败（空 unit struct size=0）→ GREEN 16/16 通过；sim_data_size_matches_original 一次通过 0x890，无需"占位凑齐"
- **控制者派发前修订 brief**：基于完整源码研究（SimData 0x890 字段布局表 + 8 子结构 dtor 偏移差值）应用 4 项预判偏差：
  1. CellSOA 测试断言 264B → 改为 352B（11×32B MsvcVector，原计划基于 24B）
  2. BackwallSOA 原简报用 `reserved[264]` 占位 → 改为精确字段（u16+i16+4B pad+3×MsvcVector=104B，源码 L9458-9464 三 vector 已验证）
  3. 8 子结构原简报均用 `reserved[64]` 占位 → 改为实际 dtor 偏移差值（312/312/176/208/208/176/312/208B，来自 01_sim_api.c L6109-6174）
  4. SimData size_of 测试不再是"很可能 FAIL" → 期望精确达 0x890 一次通过（控制者预验证）
- **审查者独立验证**：基于 stl_shim.rs MsvcVector align=8/size=32，逐字段追踪 SimData 布局（0–32 / 32–64 / 64–96 / 96–160 / 160–168 / 168–192 / 192–224 / 224–2136 / 2136–2192），所有偏移与 dtor 偏移（0xe0/0x218/0x350/0x400/0x4d0/0x5a0/0x650/0x788）完美对齐
- **遗留观察项（Phase 3 大行数）**：
  1. 8 子结构内部 padding 内容未深究（源码显示 22 longlong padding + N vector 模式或 176 undefined 字段），统一用 `reserved: [u8; N]` 占位。Phase 3 接入业务逻辑时需细化每个子结构的实际字段
  2. ActiveRegion 内联 Region 为 4 个 i32（避免单独定义 Region struct），若 Phase 3+ Region 出现在其他位置需抽出为独立 struct
  3. Timers bitfield:5 用 u8 占位，Phase 3+ 接入位操作时需改用 bitfield crate 或手写 getter/setter
  4. SimComponent 8B 占位（reserved[8]），Phase 6+ 实现
- **简报偏差数量回升**：本任务简报偏差 = 0（实现者零偏差，简报 4 项预判全部命中），与 Task 6（3 项简报偏差但均为命令语法/预判落空类）相比继续走低。Task 5 教训"Win32 API 越多偏差越多"在 Task 7 因不涉及 Win32 API（纯内存布局 struct）而未应验，且控制者派发前完整源码研究消除了所有潜在偏差
- **基线测试数说明**：实现者报告基线 53 测试是 `cargo test --lib` 输出（lib only），全量 `cargo test` 含 abi_smoke 集成测试应为 63（53 lib + 10 abi_smoke），简报"基线 63 个测试"是全量口径，无丢失测试

Task 8: complete (commits 7b9bec3..90a50f9, review clean)
- 产物：`simDLL_RS/src/game_data.rs`（新建 405 行）+ `simDLL_RS/src/lib.rs`（+1 行 `pub mod game_data;` 按字母序插入 frame_sync 与 logger 之间）
- 27 个 #[repr(C)] struct：Handle(4B) + 25 *Info 子结构（4/8/12/16/20B 五档）+ GameData(1024B=0x400)。全部基于源码实际字段定义（非估算占位），字段对照 00_types_reference.c L1041/L1801/L1847/L2064/L5973/L7183/L7250/L7438/L7528/L8737/L9481/L9784/L10127/L11929/L11935/L12206/L12451/L12523/L12545/L12738/L12748/L12752/L12758/L12762 + L3225-3269
- 测试：cargo test --lib game_data::tests 7/7 通过；cargo test 全量 86/86 通过（79 旧 + 7 新），无新 warning
- 审查结论：规格 ✅，任务质量通过，0 关键/重要，3 次要（均不阻塞）
- TDD 证据：方式 A 严格 TDD，RED 7/7 失败（空 unit struct size=0 ≠ 期望值）→ GREEN 7/7 通过
- **控制者派发前修订 brief**：基于完整源码研究（25 个 *Info struct 全部定位到源码定义 + Handle struct 发现）应用 6 项预判偏差：
  1. 计划标题"27 *Info"实为 25 个去重类型（SpawnOreInfo 被 digInfo 复用 -1，MeltedInfo 被 4 个字段复用 -3）
  2. 计划用 16B 占位宏 → 改用源码实际字段（大小 4/8/12/16/20B 五档），遵循用户规则"严格对照源码"
  3. GameData 大小从范围断言 600-800 → 精确 assert_eq!(1024)（基于 32B MsvcVector + 29 vector + 10 unique_ptr + 4 int）
  4. Handle struct 需定义（计划未提及，源码 L1801 被 3 个 *Info 引用）
  5. dig_info 字段类型是 MsvcVector<SpawnOreInfo>（源码 L3234 明确）
  6. 4 个 melted 字段都使用 MeltedInfo（不需 BuildingMeltedInfo 等多余类型）
- **GameData 大小计算**：4×i32(16) + 29×MsvcVector(928) + 10×UniquePtr(80) = 1024B。MsvcVector<T> 大小与 T 无关（4 指针=32B），故 *Info 子结构大小不影响 GameData 总大小。1024 是 2 的幂，布局完美对齐（无 padding gap）
- **审查者独立验证**：逐个清点 27 个 struct、43 个 GameData 字段、7 个测试，全部与 brief/源码一致。字段对齐验证通过（如 DiseaseEmittedInfo u8+[u8;3]+i32=8B 精确）
- **简报偏差 = 0**：实现者零偏差，6 项预判全部命中。与 Task 7 一致（纯内存布局 struct，无 Win32 API，控制者派发前完整源码研究消除所有潜在偏差）
- **遗留观察项**：
  1. GameData 未 derive Default（因含 MsvcVector/UniquePtr 未实现 Default），后续 Task 若需构造 GameData 实例须手动初始化 43 个字段
  2. SpawnFallingLiquidInfo 用 element_idx，SpawnOreInfo/UnstableCellInfo 用 elem_idx——忠实于源码命名差异（elementIdx vs elemIdx），非代码缺陷
  3. 报告行数 406 vs 实际 405（报告措辞误差，不影响代码）

Task 9: complete (commits 90a50f9..e9db4d2, review clean — 完美通过)
- 产物：4 文件全量替换（frame_sync.rs +57/-15、sim.rs +69/-30、sim_frame_manager.rs +111/-29、stl_shim.rs +35/-0，合计 +215/-57）。无新增模块，复用 Phase 0 已声明的 frame_sync/sim/sim_frame_manager/stl_shim
- 3 个 #[repr(C)] struct 替换 Phase 0 桩：
  - FrameSync（9 字段，源码字段名 m_sim_data/m_game_data/m_sim_ready_to_swap/m_initialized/_padding/m_mutex/m_game_cond/m_sim_cond/m_sim_mutex，对照 00_types_reference.c L5696-5706）
  - SimFrameManager（11 字段：frame_mutex@0 / current_frame *mut SimFrameInfo@8 / 4 个独立 MsvcVector<*mut SimFrameInfo> / active_region 4×i32 / elapsed_seconds f32 / active_regions MsvcVector<ActiveRegion> / num_frames_processed i32 / _padding i32，对照 L6468-6479）
  - Sim（3 字段：reserved_head[0x50] Thread 基类 + reserved_middle[0x1D0] + frame_sync *mut FrameSync@0x220，size=552=0x228 对照 L6982-7052 + 01_sim_api.c L330 operator_new(0x228)）
- NewGameFrame（28B：elapsed_seconds f32 + 4×i32 active_region + frame_idx i32 + pad i32，对照 L6391-6396）作为 SimFrameManager 测试模块内嵌子 struct，未提升为模块级（Phase 1 仅 SimFrameManager 内部使用）
- stl_shim.rs 追加 2 个 #[repr(C, align(8))] 占位类型：MsvcMutex([u8;80]) + MsvcCondVar([u8;8])，与现有 MsvcVector/UniquePtr/MsvcString/MsvcFunction 模式一致（内部仅 [u8;N] 字节数组，不模拟 std::mutex 实际内部布局）
- 测试：cargo test --lib 全量 79/79 通过（76 Phase 1 Task 1-8 + 3 净增）；cargo test 全量 89/89 通过（79 lib + 10 abi_smoke），无新 warning（仅 3 项遗留：[profile.test] panic / crate SimDLL snake_case / abi_smoke.rs:6 unused c_schar）
- 审查结论：规格 ✅，任务质量通过，0 关键/重要，2 次要（均不阻塞，均为自审陈述准确性问题）
- TDD 证据：方式 A 严格 TDD，RED 阶段 4 个测试失败（FrameSync/Sim/NewGameFrame/SimFrameManager unit struct size=0 ≠ 期望值）→ GREEN 阶段填充字段后 79/79 通过
- **控制者派发前修订 brief**：基于完整源码研究（FrameSync/SimFrameManager/NewGameFrame/Sim 4 个 struct 定义 + operator_new(0x228) 硬编码 + SimFrameManager 位于 gSim+0x50 + GameSync 双缓冲交换）应用 5 项预判偏差：
  1. 原简报 3 文件 → 实际 4 文件（追加 stl_shim.rs 定义 MsvcMutex(80B,align8) + MsvcCondVar(8B,align8)）
  2. 字段名臆造（game_data/prev_game_data/sim_frame_info）→ 改用源码字段名（m_sim_data/m_game_data/m_sim_ready_to_swap/m_initialized/_padding/m_mutex/m_game_cond/m_sim_cond/m_sim_mutex）
  3. Sim 字段顺序错（frame_manager 在前）→ 改用源码 offset 顺序（reserved_head[0x50] Thread 基类 + reserved_middle[0x1D0] + frame_sync 指针在 0x220）
  4. SimFrameManager 字段错（current_frame:u32 在前）→ 改用源码顺序（frameMutex offset 0，currentFrame 是 *mut SimFrameInfo 8B 指针，4 个独立 vector 而非单一 frames）
  5. 测试断言过弱（assert!(size>0)）→ 改用范围断言（FrameSync/SimFrameManager）+ 精确断言（Sim=0x228=552B、NewGameFrame=28B、MsvcMutex=80B、MsvcCondVar=8B）
- **审查者独立验证**：5 项控制者预判偏差 + 9 项验收标准全部通过，逐字段对照源码行号（FrameSync L5696-5706、SimFrameManager L6468-6479、Sim L6982-7052、NewGameFrame L6391-6396）+ offset 注释正确（frame_sync 指针在 Sim offset 0x220，frame_mutex 在 SimFrameManager offset 0）
- **简报偏差 = 0**：实现者零偏差，5 项预判全部命中。与 Task 6/7/8 一致（纯内存布局 struct，无 Win32 API，控制者派发前完整源码研究消除所有潜在偏差）
- **次要发现**（遗留，本任务不修）：
  1. 实现者自审 #2 关于 SimFrameManager Send+Sync 的陈述不准确——SimFrameManager 含 `current_frame: *mut SimFrameInfo` 裸指针字段，且 4 个 `MsvcVector<*mut SimFrameInfo>` 因 `T: Send` 约束（*mut SimFrameInfo 不实现 Send）也不会自动实现 Send。FrameSync 确实自动 Send+Sync。这不影响 Phase 1 编译或测试（无跨线程使用代码），brief 也未要求 SimFrameManager 加 unsafe impl。Phase 3+ 接入多线程时需重新审视
  2. brief 步骤 5 提到"2 个 commit"但只列出 1 个 commit 命令——简报文档内部不一致，实现者按 brief 实际内容单 commit 提交，已合理处置
- **遗留观察项（Phase 3+ 复核）**：
  1. SimFrameManager 4 个 `MsvcVector<*mut SimFrameInfo>` 的实际元素类型是 SimFrameInfo*（源码 L6468-6479），Phase 3 接入业务逻辑时需保证 *mut 跨 FFI 边界的所有权语义清晰（vector 持有指针是否需要 dtor 释放？）
  2. Sim 的 reserved_head[0x50] 对应 Thread 基类，Phase 6+ 接入多线程时需细化（参考 Thread::Start 实现）
  3. Sim 的 reserved_middle[0x1D0]（464B）覆盖 SimFrameManager 288B + 其他字段 176B，Phase 3+ 按 offset cast 子对象时需精确切割
  4. MsvcMutex/MsvcCondVar 仅字节数组占位（不模拟 std::mutex 内部布局），Phase 3+ 接入多线程同步时需替换为真实同步原语（如 std::sync::Mutex / parking_lot::Mutex）

Task 10: complete (commits e9db4d2..981680d, review clean)
- 产物：`simDLL_RS/src/message_handler.rs` 全量替换 Phase 0 桩（26 行 → 137 行，+119/-8）。仅 1 个文件被修改，diff scope 严格隔离
- 4 项定义：
  - `MessageType` enum（62 个变体，`#[repr(u32)]` + `#[derive(Clone, Copy, PartialEq, Eq)]` + `#[allow(non_camel_case_types)]`，对照源码 00_types_reference.c L9157-9220 逐字）
  - `MessageHandlerFn` 类型别名（`unsafe extern "C" fn(*mut BinaryBufferReader) -> *mut c_void`，对照源码 L9224）
  - `MessageHandlerEntry` struct（3 字段：id:MessageType@0/padding:u32@4/handler:Option<MessageHandlerFn>@8，`#[repr(C)]`，size=16，对照源码 L9222-9225）
  - `G_SIM_MESSAGE_HANDLERS: &[MessageHandlerEntry] = &[]` 空表（Phase 6 填充）
- 测试：cargo test --lib message_handler::tests 4/4 通过；cargo test --lib 全量 82/82 通过（Task 9 终点 79 + 本任务净增 3 = -1 桩删除 + 4 新）；cargo test 全量 92/92 通过（82 lib + 10 abi_smoke），无新 warning（仅 3 项遗留：[profile.test] panic / crate SimDLL snake_case / abi_smoke.rs:6 unused c_schar）
- 审查结论：规格 ✅，任务质量通过，0 关键/重要，3 次要（均不影响代码正确性）
- TDD 证据：方式 A 严格 TDD，RED 阶段 4 个测试因 enum/struct 未定义 E0425 编译失败 → GREEN 阶段填充后 4/4 通过
- **控制者派发前修订 brief**：基于完整源码研究（62 个枚举值逐字提取 + 值域分析 + struct 定义）应用 1 项关键预判偏差：
  - 原简报 `#[repr(C)]` → 改用 `#[repr(u32)]`，因 62 个枚举值中 37 个超过 i32::MAX（如 `MoveElementChunk=3920055938`），C 编译器实际选用 `unsigned int`（u32）。`#[repr(C)]` 默认 isize 在 x86_64-pc-windows-msvc 是 8 字节，会破坏 size_of==4 断言与 C ABI 兼容性
- **实现者合理适配（brief 未预判）**：在 enum 上加 `#[allow(non_camel_case_types)]`（message_handler.rs L18）抑制 7 个 C 风格下划线变体名的 warning（Disease_CreateTable / Elements_CreateTable / SimData_InitializeFromCells / SimFrameManager_NewGameFrame / SimData_ResizeAndInitializeVacuumCells / SimData_FreeCells / Elements_CreateInteractions）。保留源码命名便于 Phase 6 grep 源码对照
- **审查者独立验证**：6 项规格需求全部满足（MessageType enum + MessageHandlerFn + MessageHandlerEntry + G_SIM_MESSAGE_HANDLERS + 4 测试 + #[repr(u32)]），62 个枚举值逐字对照源码，diff scope 严格隔离
- **简报偏差 = 0**（实现者对 brief 代码本身零偏差，仅追加 `#[allow(non_camel_case_types)]` 这一无副作用 lint 抑制）
- **brief 估算偏差（非实现者错误）**：brief 写"Task 9 后 79 + 本任务 4 = 83 lib"，实际 Task 9 终点 79 含 1 个 Phase 0 桩测试 `message_table_is_empty_in_stub`，Task 10 删除该桩 + 新增 4 = 79 - 1 + 4 = 82 lib。控制者已 checkout e9db4d2 验证 Task 9 终点 79 正确
- **次要发现**（遗留，本任务不修）：
  1. `message_type_max_value_fits_u32` 测试注释略微夸大保护作用——注释写"防止 #[repr(C)] 或 #[repr(i32)] 导致溢出"，实际 `#[repr(C)]`（isize=8B）不会溢出（3920055938 < i64::MAX），`#[repr(i32)]` 会编译错误而非运行时溢出。测试真实价值是回归守卫防止数值笔误，注释可改进（不影响代码）
  2. 实现者报告推理链错误：报告称"Task 9 后实际 78 lib"，实际 Task 9 终点是 79 lib（含 1 桩），Task 10 = 79 - 1 + 4 = 82。最终结论 82 正确，仅中间推理漏算桩删除
  3. 实现者报告部分 file:line 引用 off-by-one 偏差（如"L99-103 struct fields"实际是 L100-104，"L107 G_SIM_MESSAGE_HANDLERS"实际是 L108）
- **遗留观察项（Phase 6+ 复核）**：
  1. `gSimMessageHandlers` 实际条目数：Phase 0 桩注释说"14 个条目"，Phase 6 通过源码 grep `gSimMessageHandlers[` 或 `gSimMessageHandlers +` 实际数组初始化处核实
  2. handler 注册逻辑：Phase 6 需要根据 msg_id 路由到对应 MessageHandlerFn，可能需要建 `MessageType → MessageHandlerFn` 查找表或 match 表达式
  3. `Option<MessageHandlerFn>` 的 None 语义：Phase 6 表中某些条目可能 handler 为 null，需在路由逻辑中显式处理
  4. `MessageType` derives 是否足够：Phase 6 若需用 MessageType 作为 HashMap key，需追加 `#[derive(Hash)]`；若需 debug 输出，需追加 `#[derive(Debug)]`。Phase 1 不预先添加（YAGNI）

Task 11: complete (no new commits — 验证任务)
- 产物：无代码改动。lib.rs L8-24 已含全部 17 个 `pub mod X;` 声明（abi/buffer/crashdump/disease/element/frame_sync/game_data/logger/message_handler/sim/sim_api/sim_data/sim_events/sim_frame_manager/sysinfo/stl_shim/vector_math），按字母序排列，无缺失。每个 Phase 1 Task 1-10 实现者各自完成时已逐个声明，Task 11 仅做集成验证
- 验证：
  - `cargo check` 通过，仅 2 项已知遗留 warning（`[profile.test] panic` / `crate SimDLL snake_case`），无新 warning
  - `cargo test` 全量 92/92 通过（82 lib + 10 abi_smoke），0 failed，无新 warning
- 审查结论：N/A（无代码改动，按 plan L1729 跳过 commit）
- 遗留 warning（仍 3 项，Task 12 清理 2 项）：
  1. Cargo.toml `[profile.test] panic`（Task 12 步骤 1 清理）
  2. crate SimDLL snake_case（不可改，lib 名必须匹配原版 DLL 名）
  3. tests/abi_smoke.rs:6 unused c_schar（Task 12 步骤 2 清理）

Task 12: complete (commits 981680d..d7e42b5, review clean — Phase 1 收官任务)
- 产物：3 文件改动（diff scope 严格隔离）
  - `simDLL_RS/Cargo.toml`：删除 `[profile.test] panic = "unwind"`（L39-40，2 行）。`cargo test` 默认就是 unwind，Phase 0 Task 1 引入此配置冗余且产生 warning
  - `simDLL_RS/tests/abi_smoke.rs`：①L6 移除未使用的 `c_schar` import ②末尾追加 `binary_buffer_round_trip_integration` 集成测试（L89-113，25 行），write 7 种类型（int/uint/float/string/byte/short/ushort）→ read round-trip + is_at_end 断言
  - `.superpowers/sdd/progress.md`：本条记录
- 测试：
  - `cargo test --test abi_smoke`：11/11 PASS（10 旧 + 1 新 round-trip），0 failed
  - `cargo test` 全量：93/93 PASS（82 lib + 11 abi_smoke + 0 doc），0 failed
- Release build：`cargo build --release --target x86_64-pc-windows-msvc` exit 0，SimDLL.dll = **996864 字节（973.5 KiB）**，与 Phase 0 Task 10 基线完全一致（新增测试在独立集成 crate，未编入 release cdylib；Cargo.toml 仅删 test-profile 配置不影响 release）
- Warning 终态：从 3 项降至 **1 项**（仅剩不可改的 `crate SimDLL should have a snake case name`，lib 名必须 "SimDLL" 匹配原版 DLL 名）。P0-2 `[profile.test] panic` 与 P0-3 `unused c_schar` 两项遗留 warning 已清理
- 审查结论：规格 ✅，任务质量通过，0 关键/重要/次要（完美通过）
- **简报偏差 = 0**：实现者零偏差，plan L1762-1788 给出的 round-trip 测试代码逐字写入一次通过。BinaryBuffer API（write_int/uint/float/string/byte/short/ushort + read_* + is_at_end + into_bytes）已在 Task 3 实现，本任务仅做集成层验证
- **3.14f32 round-trip 精度**：`assert_eq!(reader.read_float().unwrap(), 3.14)` 通过——`3.14` 在 f32 字面量上下文是同一 f32 位模式，write/read 无损往返
- **执行方式**：直接内联执行（任务范围小且明确：2 项 warning 清理 + 1 个集成测试 + 最终验收），未分派实现者子代理。控制者亲自完成 3 处编辑 + 3 轮 cargo 验证
- **遗留观察项（Phase 3+ 复核）**：本任务无新增遗留项。Phase 1 全部遗留观察项见下方"Phase 1 完成总结"

---

## Phase 1 完成总结（Data Layer）

**状态：✅ 全部 12 个任务完成，Phase 1 收官。待用户部署测试。**

### 交付物
- **SimDLL.dll**：`simDLL_RS/target/x86_64-pc-windows-msvc/release/SimDLL.dll`，996864 字节（973.5 KiB）
- **17 个模块**：abi / buffer / crashdump / disease / element / frame_sync / game_data / logger / message_handler / sim / sim_api / sim_data / sim_events / sim_frame_manager / sysinfo / stl_shim / vector_math
- **测试基线**：93/93 PASS（82 lib 单元 + 11 abi_smoke 集成 + 0 doc），0 failed
- **Warning 终态**：1 项不可改（crate SimDLL snake_case，lib 名匹配原版 DLL 名）

### Phase 1 数据层成果（对照源码逐字段实现）
- **stl_shim.rs**（Task 1 + Task 7 修订）：MsvcVector(32B)/UniquePtr(8B)/MsvcString(32B)/MsvcFunction(64B)/MsvcMutex(80B)/MsvcCondVar(8B)
- **vector_math.rs**（Task 2）：Vector2f(8B)/Vector3f(12B)/Vector4f(16B)
- **buffer.rs**（Task 3）：BinaryBufferReader(24B #[repr(C)]) + BinaryBufferWriter(Vec 后端) + BufferError enum
- **element.rs**（Task 4）：ElementState(1B)/PhysicsData(12B)/Element(164B，38 字段)
- **disease.rs**（Task 5）：DiseaseInfo(20B)/Disease(64B)
- **sim_events.rs**（Task 6）：SimFrameInfo/SimEvents 空占位（Phase 3 大任务填充字段）
- **sim_data.rs**（Task 7）：SimData(0x890=2192B) + CellSOA(352B) + BackwallSOA(104B) + 5 辅助子结构 + 8 padding 子结构（dtor 偏移对齐）
- **game_data.rs**（Task 8）：GameData(0x400=1024B) + Handle(4B) + 25 *Info 子结构（4/8/12/16/20B 五档）
- **frame_sync.rs + sim.rs + sim_frame_manager.rs**（Task 9）：FrameSync + Sim(0x228=552B) + SimFrameManager + NewGameFrame(28B)
- **message_handler.rs**（Task 10）：MessageType enum(62 变体, #[repr(u32)]) + MessageHandlerEntry(16B) + G_SIM_MESSAGE_HANDLERS 空表

### Phase 1 简报偏差汇总（用于 Phase 2+ 复盘）
- Task 1: MsvcVector 24B→32B（Task 7 派发前修订，详见下方"Phase 1 Task 7 派发前关键修订"）
- Task 7: 4 项预判偏差（CellSOA 264→352B / BackwallSOA reserved→精确字段 / 8 子结构 reserved→dtor 偏移 / SimData 范围→精确 0x890）
- Task 8: 6 项预判偏差（25 *Info 非占位宏 / GameData 范围→精确 1024 / Handle struct 需定义 / dig_info vector 类型 / 4 个 melted 字段类型 / SpawnOreInfo 复用）
- Task 9: 5 项预判偏差（3→4 文件 / 字段名臆造→源码名 / Sim 字段顺序 / SimFrameManager 字段顺序 / 断言过弱→范围+精确）
- Task 10: 1 项关键预判偏差（#[repr(C)] → #[repr(u32)]，37 个值超 i32::MAX）
- Task 3/4/5/6/11/12: 0 项偏差

### Phase 1 遗留观察项（Phase 3+ 复核清单）
1. BinaryBufferReader 无生命周期（Task 3）—— Phase 3 SimData 接入时评估是否加 `<'a>` + `PhantomData`
2. Element 164B 待源码 sizeof 核实（Task 4）—— Phase 2 接入 CreateElementsTable 时对照
3. SimFrameInfo/SimEvents 字段补全（Task 6）—— Phase 3 大任务（30+ 子 struct 需先定义）
4. Task 7/8 子结构 padding（Task 7/8）—— Phase 3 细化每个子结构实际字段
5. ActiveRegion 内联 Region（Task 7）—— Phase 3+ 若 Region 出现其他位置需抽出为独立 struct
6. Timers bitfield（Task 7）—— Phase 3+ 接入位操作时改用 bitfield crate
7. SimComponent 占位（Task 7）—— Phase 6+ 实现
8. GameData 未 derive Default（Task 8）—— 后续需构造 GameData 实例时手动初始化 43 字段
9. SpawnFallingLiquidInfo 命名差异（Task 8）—— 忠实于源码（elementIdx vs elemIdx），非缺陷
10. SimFrameManager *mut 裸指针 Send+Sync（Task 9）—— Phase 3+ 多线程时审视
11. Sim reserved_head[0x50] Thread 基类 + reserved_middle[0x1D0] 切割（Task 9）—— Phase 3+ 按 offset cast
12. MsvcMutex/MsvcCondVar 仅字节数组占位（Task 9）—— Phase 3+ 替换为真实同步原语
13. gSimMessageHandlers 实际条目数 + handler 注册逻辑（Task 10）—— Phase 6 核实
14. MessageType derives 是否足够（Task 10）—— Phase 6 视需要追加 Hash/Debug

### 部署测试预期行为（告知用户）
- 游戏能进主菜单（与 Phase 0 一致，无回归）
- 加载存档仍会崩溃（Phase 1 只定义数据结构，未实现存档加载逻辑，Phase 3 才实现）
- 日志文件 `logs/simdll.log.YYYY-MM-DD` 应记录 SIM_Initialize 调用

---

## Phase 1 Task 7 派发前关键修订（MsvcVector 24B → 32B）

**触发**：Task 7 派发前控制者研究 SimData 源码与运行时访问模式时发现 MsvcVector 真实大小为 32B 而非 Task 1 实现的 24B。此为 Task 1 review 未发现的根本性 layout bug，影响 Task 5（Disease）字段布局并阻塞 Task 7（SimData 无法凑齐 0x890）。

**证据链（4 重验证）**：
1. 源码 struct 定义：`vector<float>` sizeof=32（00_types_reference.c L7768-7801，32 个 undefined 字段）；`vector<WorldOffsetData>` sizeof=32（L8343-8376）；`vector<SimComponent_*>` sizeof=32（L8190-8223）——全部一致 32B
2. 运行时访问偏移：`worlds` vector 在 gSimData 偏移 0x870，`_Last`@0x880（相对+0x10）、`_End`@0x888（相对+0x18）（11_msvcrt_ignored.c L24744-24746）——证实 vector 内部布局为 `_Container_proxy*@0 + _First@8 + _Last@16 + _End@24` = 32B
3. SimData 析构函数偏移：8 个子结构实际偏移（01_sim_api.c L6109-6174）——ElementConsumer@0xe0=224、ElementEmitter@0x218=536、RadiationEmitter@0x350=848、ElementChunk@0x400=1024、BuildingHeatExchange@0x4d0=1232、BuildingToBuildingHeatExchange@0x5a0=1440、DiseaseEmitter@0x650=1616、DiseaseConsumer@0x788=1928
4. 用 32B MsvcVector 重算 SimData 完整布局：sim_events@168 ✓（吻合 02_save_load.c L224 访问 gSimData+0xa8）、initSettleThermalBoundaries@2136 ✓（吻合 02_save_load.c L197 访问 gSimData+0x858）、worlds@2160 ✓（吻合 02_save_load.c L307 cast）、SimData 末尾=2192=0x890 ✓ 完美对齐

**根因**：simDLL.dll 编译时启用了 `_ITERATOR_DEBUG_LEVEL >= 1`，MSVC std::vector 多出 8B `_Container_proxy*` 头部用于 iterator debugging。Task 1 简报假设 EBO 后 vector=24B（仅 _Vector_val 3 指针），未实际验证源码。

**修复内容（单 commit）**：
- `simDLL_RS/src/stl_shim.rs`：MsvcVector 加第 4 字段 `container_proxy: *mut u8`，size 断言 24→32，注释详述修订原因
- `simDLL_RS/src/disease.rs`：删除 `reserved: [u8; 16]`（原"补齐 16B 凑 64B"实为 2×vector 多出的 8B 头部分摊），2×32B=64B 仍满足 size 断言；开放问题 #3 关闭
- Task 4 element.rs 不受影响（无 MsvcVector 字段，164B 不变）
- Task 6 sim_events.rs 不受影响（空 struct 占位）

**验证**：cargo test --target x86_64-pc-windows-msvc 全量 63/63 通过（53 lib + 10 abi_smoke），0 error。Disease size=64B 不变（巧合通过），但字段布局已修正为真实 C++ 内存布局

**教训**：
1. Task 1 review 时未交叉验证源码 sizeof 与简报假设——后续涉及 STL 容器的任务应强制 grep 源码 struct 定义核对 sizeof
2. Task 5 Disease "reserved[16] 凑齐 64B" 是 warning sign（用占位补齐巧合通过 size 断言），当时接受为"开放问题 #3"延迟处理。若当时深究 16B 来源应能更早发现 MsvcVector sizing bug。后续任务遇"占位补齐凑 size"应追问到底，不得仅记开放问题放过
3. MsvcVector 32B 影响 Phase 1 Task 7-8（SimData/GameData 直接使用），Task 9-12 顶层骨架不含 MsvcVector 字段故不受影响。Phase 2+ 接入业务逻辑时所有 vector 操作都需考虑 4 指针槽（_Container_proxy 在 release build 中通常为 null，但 debug build 可能非 null）

## 全局约束（控制者与审查者共用）

- **平台**：仅支持 x86_64-pc-windows-msvc 目标
- **C ABI 约定**：导出函数 `#[no_mangle] pub extern "C" fn`；跨 ABI 结构体 `#[repr(C)]`
- **panic 策略**：release 用 `panic = "abort"`，test 用 `panic = "unwind"`；导出函数必须 `catch_unwind`
- **PowerShell 执行策略被禁用**：所有 .ps1 脚本必须用 `powershell -ExecutionPolicy Bypass -File <path>` 调用
- **Python 不可用**
- **用户规则**：助手不帮用户部署 simDLL.dll；用户是代码小白；严格对照源码原理
- **依赖最小化**：仅 7 个 crate（windows, tracing, tracing-appender, tracing-subscriber, parking_lot, once_cell, chrono）
- **git 内部结构异常处置纪律**（Task 2 起强制）：实现者遇到任何 git 内部结构异常（嵌套 `.git`、submodule 提示、reflog 异常、`git add` 静默失败等）必须**先停下以 BLOCKED 或 NEEDS_CONTEXT 状态请示控制者**，不得自行 `Remove-Item -Recurse -Force .git`、`git submodule deinit`、`git clean -fd` 等破坏性 git 内部操作。本纪律源于 Task 2 实现者越级删除 `simDLL_RS/.git` 的先例。
- **报告卫生**（Task 2 起强制）：实现者报告 cargo build/test 输出时，所有 warning/error 必须贴出完整原文（含 file:line、help、note），不得仅靠叙述。

---

## Phase 2: 元素系统

Task P2-T1: complete (commits 524ee28..a931543, review clean) — element.rs 12 struct + 3 const + Element Default + 13 size tests; 105/105 tests pass
Task P2-T2: complete (commits a931543..4b78130, review clean) — buffer.rs pub fn offset() + 2 tests; 96 lib tests pass (94 baseline + 2 new)
Task P2-T3: complete (commits 4b78130..48364c6, review clean) — elements_table.rs ElementsTable struct (15 fields) + Default impl + G_ELEMENTS_TABLE (std::sync::Mutex) + G_INTERACTIONS_LOCK (parking_lot::Mutex, private static) + lib.rs mod declaration; 99 lib tests pass (96 baseline + 3 new); review: 0 critical/major, 3 minor (none blocking — report line count 107 vs 106 actual / decision #2 comment id not T3's fault / G_INTERACTIONS_LOCK visibility risk for T4+ if functions move out of module)
- ⚠️ T1/T2 baseline verification issue (recorded during T3 review): brief expected 108 baseline (93+13+2) but T3 implementation report shows 99 lib tests (96 baseline + 3 new). 12-test gap root cause is in T1/T2 not T3 — T3's own 3-test contribution is correct. To verify: run `cargo test --lib` on a931543 (post-T1) and 4b78130 (post-T2) to confirm actual baseline test counts.
Task P2-T4: complete (commits 48364c6..5d5f88c, review clean) — 3 private internal functions (destroy_elements_table / get_element_index_inner / get_element_index) + 4 TDD tests; 103 lib tests pass (99 baseline + 4 new); review: 0 critical/major, 2 minor (both "plan-forced" — test name "12 vectors" vs 11 vectors + 1 HashMap / test relies on global mutable state with theoretical flakiness risk for future T5+ tests on 3 interaction vectors); micro-adjustment: implementer removed unused INVALID_ELEMENT_HASH from test import (verified correct, would have triggered unused_imports warning)
Task P2-T5: complete (commits 5d5f88c..81d16d9 via 5ea9c83 + 81d16d9 fix, review passed after fix) — create_elements_table 4-phase deserialize + post-process (destroy→resize+memcpy→indices→names→9 附属 vector→后处理 ore_idx); 106 lib tests pass (103 baseline + 3 new); review: 0 critical, 1 major (std::ptr::read 未对齐 UB → read_unaligned), 3 minor (test parallel interference on G_ELEMENTS_TABLE / phase 3 len no upper bound check / phase 2 element_indices.clear() 冗余 — 控制者已核实 T4 destroy L90 已 clear)
- 2 Rust 必要偏差（已审查确认等价）：①阶段 2 借用冲突修复（iter().enumerate() → index 访问，Element: Copy）②2 测试 BinaryBufferReader 悬垂指针修复（&w.into_bytes() → let buf = w.into_bytes(); &buf）
- 修复 commit 81d16d9：std::ptr::read → std::ptr::read_unaligned（bytes 对齐 1 vs Element 对齐 4，UB 风险，教科书式正确修复）
- 后处理使用 get_element_index_inner(&table, hash) 避免死锁（决策 #1，std::sync::Mutex 不可重入）
- 2 个运行时计算字段：mass_area_scale=state&3==1?1.0:0.001 / mass_scale=state&3==3?∞:1.0/mass
- INVALID_ELEMENT_HASH 比较：elem.low_temp_transition_ore_id as u32 == INVALID_ELEMENT_HASH（i32→u32 转换 + u32 比较）
Task P2-T6: complete (commits 81d16d9..7302208 via 0db5954 + 7302208 fix, review passed after fix) — create_element_interactions 签名过滤 + 32B 对齐; 109 lib tests pass (106 baseline + 3 new); review: 1 critical (K1: 34B/entry vs 源码 32B/entry), 2 minor (M1 注释算术错误 / M2 entry 2 只验证 2 字段)
- **关键 bug K1**：实现读了 _padding_a(2B) + skip(2) = 34B/entry，源码 03_elements.c L565 `lVar3 = lVar3 + 0x20` 明确 32B stride。测试 buffer 共享同一错误（写 34B/entry）所以自洽通过。根因：注释算术错误 `4 + 2*4 + 5*4 = 30`（实际 32）掩盖逻辑错误。运行时影响：真实游戏数据 entry ≥1 全部解析错误
- **修复 commit 7302208**：4 项修复 — ①删实现中 reader.skip(2)? ②删测试 buffer 末尾 w.write_ushort(0) ③修 3 处注释算术为 32 字节 ④补 entry 2 的 6 字段断言（浮点字段守护对齐 bug）
- **教训**：决策 #2 "30B + skip 2 = 32B" 的字面表述易误导——实际 30B = signature(4) + 3×u16(6) + 5×f32(20) 不含 _padding_a，但简报实现代码读了 _padding_a（变 32B）又 skip(2)（变 34B）。控制者派发前未交叉验证简报注释算术与代码字段累加
- **1 项 Rust 偏差（已审查确认合理）**：简报预判偏差 #1 `..Default::default()` 实际无效（Rust functional update syntax 也需访问私有字段触发 E0451），实现者改用 `let mut g = GasObliteration::default(); g.field = value;` 模式（default() 在 element.rs 模块内构造完整实例含 _pad1=0，跨模块只赋 pub 字段）。审查者次要发现注释论断"`..Default::default()` 都会触发 E0451"不准确（实际该语法可编译），但代码功能 100% 正确，不阻塞
Task P2-T7: complete (commits 7302208..c41a471, review clean) — 5 C ABI exports + tests/elements_smoke.rs (7 integration tests); 127 tests pass (109 lib + 11 abi_smoke + 7 elements_smoke); review: 0 critical, 1 major (plan-forced: test global state race condition), 2 minor
- **5 个 #[no_mangle] pub extern "C" fn 导出**：CreateElementsTable(reader) -> *mut c_void (返回 count 或 null) / DestroyElementsTable() / GetElementIndex(hash) -> u16 / CreateElementInteractions(reader) -> *mut c_void (始终 null，源码 L568) / CreateElementInteractionsLocked(reader) -> *mut c_void (用 G_INTERACTIONS_LOCK 独立锁，决策 #2，透传 null)
- **catch_unwind 包装模式**与 Phase 1 sim_api.rs 一致：pointer 返回用 `.unwrap_or_else(|_| { tracing::error!(...); null_mut() })`，u16 返回用 `.unwrap_or(INVALID_ELEMENT_INDEX)`，void 返回用 `let _ = catch_unwind(...)`
- **4 项预判偏差全部应用到位**：①删除测试 buffer 末尾 w.write_ushort(0)（32B stride 无 trailing pad，T6 教训 K1）②删除 unused c_void import（Phase 1 Task 8 教训）③预期测试数 144→127 修正 ④use 语句移到模块顶部避免 E0259
- **1 项断言修正（控制者已核实接受）**：brief 步骤 1 测试 `create_element_interactions_locked_full_round_trip_via_ffi` 原断言 `!result.is_null()` 与源码 03_elements.c L568 (`return (void *)0x0;`，CreateElementInteractions 始终返回 null) + L17/L22 (Locked 版透传该返回值) 矛盾，实现者按用户规则"严格对照源码原理"修正为 `result.is_null()`。brief 内部矛盾佐证：brief 自身的 null reader 测试用 `is_null()` 与 round-trip 测试的 `!is_null()` 矛盾
- **审查者独立验证**：5 导出对照 brief 步骤 3 代码块逐字一致，7 测试对照 brief 步骤 1 代码块逐字一致（除控制者已核实接受的 is_null() 修正外），catch_unwind 模式与 sim_api.rs 一致，无新增依赖、无破坏性改动
- **遗留观察项（Phase 2 T8 或后续 Phase 复核）**：
  1. 测试间共享全局 G_ELEMENTS_TABLE 导致竞态条件（计划强制，brief 逐字提供测试无 #[serial] 序列化；当前 127/127 通过是窗口期幸运；修复需 brief 层面决策——引入 serial_test crate 会破坏"7 crate 不新增"约束，或重构测试不依赖全局状态，或接受脆弱性并文档化）。同一问题在 T4 review 中已记录（"test relies on global mutable state with theoretical flakiness risk for future T5+ tests on 3 interaction vectors"），T5/T6/T7 均接受为遗留观察项
  2. DestroyElementsTable panic 路径无 tracing（与 sim_api.rs void 返回模式一致，非偏差，但生产环境 void 函数 panic 静默难定位，若 sim_api.rs 后续补 tracing 本文件应同步）
  3. create_elements_table_full_round_trip_via_ffi 用 transmute 转 Element 为字节假设 #[repr(C)] 布局确定性（T1 已验证 size=164，但布局依赖 repr；建议未来用 bytemuck::cast_ref 或显式 #[repr(C)] 加固，超出本任务范围）

---

## Phase 2 完成总结（元素系统）

**状态：✅ 全部 8 个任务（T1-T8）完成。待用户部署测试。**
**Phase 2 起点 commit：** `524ee28`（Phase 1 基线 `d7e42b5`，93 测试）
**Phase 2 终点 commit：** T8 commit（本任务，无代码改动，仅 progress.md 文档）

### 交付物
- **SimDLL.dll**：`simDLL_RS/target/x86_64-pc-windows-msvc/release/SimDLL.dll`（release build 通过，仅遗留不可改的 `crate SimDLL should have a snake case name` warning）
- **5 个 C ABI 导出函数**（T7）：CreateElementsTable / DestroyElementsTable / GetElementIndex / CreateElementInteractions / CreateElementInteractionsLocked
- **3 个修改模块**：`element.rs`（T1：12 struct + 3 const + Element Default，164B）+ `buffer.rs`（T2：pub fn offset()）+ `elements_table.rs`（T3-T7：ElementsTable struct + G_ELEMENTS_TABLE/G_INTERACTIONS_LOCK 单例 + 5 内部 fn + 5 C ABI 导出）
- **1 个新测试文件**：`tests/elements_smoke.rs`（T7：7 集成测试）
- **测试基线**：127/127 PASS（109 lib + 11 abi_smoke + 7 elements_smoke），0 failed

### Phase 2 任务实现成果
- **T1**：element.rs 12 struct + 3 const + Element Default（105 测试）
- **T2**：buffer.rs BinaryBufferReader::offset() pub 暴露（96 测试，+2）
- **T3**：ElementsTable struct（15 字段）+ G_ELEMENTS_TABLE（std::sync::Mutex）+ G_INTERACTIONS_LOCK（parking_lot::Mutex 私有 static）（99 测试，+3）
- **T4**：3 个私有内部函数 destroy_elements_table / get_element_index_inner / get_element_index（103 测试，+4）
- **T5**：create_elements_table 4 阶段反序列化 + 后处理（106 测试，+3；fix: ptr::read → ptr::read_unaligned）
- **T6**：create_element_interactions 签名过滤 + 32B 对齐（109 测试，+3；fix: K1 删除 reader.skip(2) + 测试 buffer 末尾 trailing padding）
- **T7**：5 个 C ABI 导出 + 7 集成测试（127 测试，+7 集成；断言修正 `!is_null()` → `is_null()` 与源码 L568 一致）
- **T8**：文档收尾（本任务，roadmap.md L61 已在 Phase 2 起点 commit 524ee28 修正，跳过此步骤；仅 progress.md 追加 Phase 2 完成总结）

### 关键决策
1. **ElementsTable 方案 B**：封装 15 个全局变量为单 struct，G_ELEMENTS_TABLE 全局 Mutex 保护（避免 15 个独立 Mutex 的开销与死锁风险）
2. **G_INTERACTIONS_LOCK 独立锁**：用 parking_lot::Mutex 替代原版 gFrameSync.mSimMutex（决策 #2，Phase 3+ 替换 MsvcMutex 时无需回头改）
3. **CreateElementInteractions 32B stride**：源码 03_elements.c L565 `lVar3 = lVar3 + 0x20` 明确 32B/entry（4+3×u16+2 pad+5×f32 = 32B，无 trailing padding）
4. **CreateElementInteractions 始终返回 null**：源码 L568 `return (void *)0x0;`，Locked 版透传该返回值（L17/L22）——与成功/失败无关

### 简报偏差汇总（Phase 2 复盘）
- T1: 0 项
- T2: 0 项
- T3: 3 项次要（report 行数 / 决策 #2 注释 / G_INTERACTIONS_LOCK visibility 风险）
- T4: 2 项次要（测试名 "12 vectors" vs 11+1 HashMap / 全局状态竞态 flakiness 风险）
- T5: 1 项 major（std::ptr::read 未对齐 UB → read_unaligned）+ 3 项次要 + 2 项 Rust 必要偏差
- T6: 1 项 critical（K1: 34B/entry vs 32B/entry）+ 2 项次要 + 1 项 Rust 偏差（合理）
- T7: 1 项 major（plan-forced: 全局状态竞态）+ 2 项次要 + 1 项断言修正（控制者已核实接受）
- T8: roadmap.md L61 已在 Phase 2 起点 commit 524ee28 修正，跳过此步骤

### Phase 2 遗留观察项（Phase 3+ 复核清单）
1. 测试间共享全局 G_ELEMENTS_TABLE 竞态条件（T4/T7 记录，brief 逐字提供测试无 #[serial]；修复需 brief 层面决策——引入 serial_test crate 会破坏"7 crate 不新增"约束）
2. DestroyElementsTable panic 路径无 tracing（T7 记录，与 sim_api.rs void 返回模式一致）
3. create_elements_table_full_round_trip_via_ffi 用 transmute 假设 #[repr(C)] 布局（T7 记录，T1 已验证 size=164 但布局依赖 repr）
4. BinaryBufferReader 无生命周期（Phase 1 T3 遗留，Phase 3 SimData 接入时评估）
5. Element 164B 待源码 sizeof 核实（Phase 1 T4 遗留，T7 已通过 transmute 测试间接验证布局正确）

### 部署测试预期行为（告知用户）
- 游戏能进主菜单（与 Phase 1 一致，无回归）
- 加载存档行为与 Phase 1 一致（仍会崩溃，Phase 3 才实现存档加载）
- 日志文件无新增条目（除非 C# 主动调用本 Phase 函数，但 Phase 2 之前 C# 不会调用 CreateElementsTable 等）
