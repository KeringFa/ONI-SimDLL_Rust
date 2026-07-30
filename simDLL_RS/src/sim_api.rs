//! C ABI 导出层：9 个 `SIM_*/SYSINFO_*` 函数。
//!
//! 严格对照源码 L98220-L98625。
//! 每个函数包 `catch_unwind`，panic 不跨 C ABI 边界（release 模式下 panic=abort，
//! 但 test 模式 unwind，catch_unwind 双保险）。

#![allow(dead_code)]

use std::os::raw::{c_char, c_int, c_schar, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::abi::GameMessageHandler;
use crate::buffer::{BinaryBufferReader, BufferError};
use crate::crashdump;
use crate::globals::G_GAME_MESSAGE_HANDLER;
use crate::logger;
use crate::message_handler::MessageType;
use crate::save_load;
use crate::sysinfo;

/// 9 个 save_load 消息 ID（对照源码 00_types_reference.c L9157-9220 enum Type 的 save_load 子集）。
///
/// 用 `const` 项而非直接 `MessageType::Load as u32` 作为 match 模式，
/// 因为 Rust 模式语法要求路径必须是 const 项，不能是任意 const expr。
/// 值通过 `MessageType::Xxx as u32` 显式取出，便于一眼对照源码 enum Type。
const MSG_LOAD: u32 = MessageType::Load as u32;                                   // 3622429126
const MSG_START: u32 = MessageType::Start as u32;                                 // 3363520610
const MSG_ALLOCATE_CELLS: u32 = MessageType::AllocateCells as u32;                // 1092408308
const MSG_DEFINE_WORLD_OFFSETS: u32 = MessageType::DefineWorldOffsets as u32;    // 3399120745
const MSG_PREPARE_GAME_DATA: u32 = MessageType::PrepareGameData as u32;           // 1078620451
const MSG_CLEAR_UNOCCUPIED_CELLS: u32 = MessageType::ClearUnoccupiedCells as u32; // 2458763021
const MSG_SET_SAVED_OPTIONS: u32 = MessageType::SetSavedOptions as u32;          // 1154135737
const MSG_DISEASE_CREATE_TABLE: u32 = MessageType::Disease_CreateTable as u32;    // 825301935
const MSG_RESIZE_AND_INIT_VACUUM_CELLS: u32 =
    MessageType::SimData_ResizeAndInitializeVacuumCells as u32;                   // 3542291143

/// 从 `(data, size)` 构造字节切片。`data` 为 null 或 `size <= 0` 时返回空切片。
///
/// 对照原版 `BinaryBufferReader::BinaryBufferReader(&local_30, (longlong)param_2, param_3)`
/// (01_sim_api.c L225)。原版不检查 null/负 size（直接构造 reader），Rust 版防御性
/// 处理避免 UB（C# 侧若误传 null 不会触发 undefined behavior）。
///
/// # Safety
/// 调用者需保证 `data` 在 `size` 字节范围内合法可读（C# FFI 契约默认满足）。
fn make_slice<'a>(data: *mut c_schar, size: c_int) -> &'a [u8] {
    if data.is_null() || size <= 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) }
    }
}

/// 分发单条消息到 save_load handler。
///
/// 对照原版 `gSimMessageHandlers` 表查找 (01_sim_api.c L230-238)。
/// 原版表有 14 个条目（Ghidra 未反编译出静态初始化代码），其中 9 个是 save_load handler（已知），
/// 另外 5 个是非 save_load handler（Phase 6+ 实现）。
///
/// # 返回值
/// 始终返回 null。原版 save_load handler 签名是 `void * (*handler)(BinaryBufferReader *)`，
/// 但 Phase 3 的 Rust 实现返回 `Result<(), BufferError>`（无指针 payload）——因为 save_load 是
/// "command"消息（修改全局状态，无返回数据给 C#）。"Query" 消息由 SimFrameManager::HandleMessage
/// 处理（Phase 6+ 实现，当前跳过）。
///
/// # 错误处理
/// save_load handler 返回 `Err(BufferError)` 时记录 error 日志并返回 null（不 panic，不中断调用方）。
fn dispatch_save_load_message(id: c_int, reader: &mut BinaryBufferReader) -> *mut c_void {
    let id_u32 = id as u32;
    let result: Result<(), BufferError> = match id_u32 {
        MSG_LOAD => save_load::load(reader),
        MSG_START => save_load::start(reader),
        MSG_ALLOCATE_CELLS => save_load::allocate_cells(reader),
        MSG_DEFINE_WORLD_OFFSETS => save_load::define_world_offsets(reader),
        MSG_PREPARE_GAME_DATA => save_load::prepare_game_data(reader),
        MSG_CLEAR_UNOCCUPIED_CELLS => save_load::clear_unoccupied_cells(reader),
        MSG_SET_SAVED_OPTIONS => save_load::set_saved_options(reader),
        MSG_DISEASE_CREATE_TABLE => save_load::create_disease_table(reader),
        MSG_RESIZE_AND_INIT_VACUUM_CELLS => save_load::resize_and_initialize_vacuum_cells(reader),
        _ => {
            // 非 save_load 消息（SimFrameManager::HandleMessage 未实现，Phase 6+）
            // 或未知消息 ID。返回 null（与原版未在表中找到 handler 时的行为一致——
            // local_res20 保持初始 null 值）。
            tracing::trace!(id, "SIM_HandleMessage: no save_load handler for id");
            return std::ptr::null_mut();
        }
    };
    match result {
        Ok(()) => std::ptr::null_mut(),
        Err(e) => {
            tracing::error!(id, error = ?e, "save_load handler returned error");
            std::ptr::null_mut()
        }
    }
}

/// 对应原版 `SIM_Initialize(_func___cdecl_void_ptr_int_void_ptr *param_1)` (L98515)。
///
/// 原版流程：
///   1. Timer::Initialize() — 桩不实现
///   2. operator_new(0x1070) + Logger::Logger — Rust 用 Logger::init()
///   3. SymInitialize(...) — 桩不实现（Phase 10 补）
///   4. InitializeCrashDumpHandler() — 实现
///   5. CleanUp() — 桩不实现
///   6. gGameMessageHandler = param_1 — 实现
///   7. FrameSync::clear(&gFrameSync) — 实现
///   8. operator_new(0x228) + Sim::Sim(this, &gFrameSync) — 桩简化为不分配
#[no_mangle]
pub extern "C" fn SIM_Initialize(handler: Option<GameMessageHandler>) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // 1. Logger 初始化（对应原版 Logger::Logger + Util::cSingleton<Logger>::mInstance = ...）
        let _ = logger::init();

        tracing::info!("SIM_Initialize (Phase 0 stub) — start");

        // 2. SymInitialize 桩跳过（Phase 10）

        // 3. InitializeCrashDumpHandler
        crashdump::initialize();
        tracing::info!("CrashDumpHandler registered");

        // 4. CleanUp 桩跳过

        // 5. gGameMessageHandler = param_1
        if let Some(h) = handler {
            let _ = G_GAME_MESSAGE_HANDLER.set(h);
            tracing::info!("GameMessageHandler registered");
        } else {
            tracing::warn!("SIM_Initialize called with null handler");
        }

        // 6. FrameSync::clear — 桩，no-op
        // （FrameSync 全局静态在 Phase 1 添加，Phase 0 跳过）

        // 7. Sim::Sim 桩跳过（不分配 0x228 字节，避免误用未实现字段）

        tracing::info!("SIM_Initialize (Phase 0 stub) — done");
    }));
}

/// 对应原版 `SIM_Shutdown(void)` (L98562)。
///
/// 原版流程：
///   1. CleanUp() — 桩跳过
///   2. 析构 Logger 单例 — Rust 用 Logger::flush()
///   3. SymCleanup(GetCurrentProcess) — 桩跳过
#[no_mangle]
pub extern "C" fn SIM_Shutdown() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        tracing::info!("SIM_Shutdown (Phase 0 stub) — start");
        // CleanUp 桩跳过
        logger::flush();
        // SymCleanup 桩跳过
        tracing::info!("SIM_Shutdown (Phase 0 stub) — done");
    }));
}

/// 对应原版 `SIM_HandleMessage(int id, int size, schar *data)` (L98420 / 01_sim_api.c L208-244)。
///
/// 原版流程：
///   1. 检查 gSim != null（若 null 直接返回 null）
///   2. 构造 BinaryBufferReader（从 param_2=size, param_3=data 指针）
///   3. 调用 SimFrameManager::HandleMessage — 返回 bool
///   4. 若返回 false（未处理），遍历 gSimMessageHandlers[14] 表查 handler
///   5. 析构 BinaryBufferReader，返回 result
///
/// Phase 3 实现：
///   - gSim 检查跳过（G_SIM 是 Lazy<Mutex<Box<Sim>>> 永远非 null）
///   - SimFrameManager::HandleMessage 未实现（Phase 6+），直接查表
///   - gSimMessageHandlers 表的 9 个 save_load 条目已实现（dispatch_save_load_message）
///   - 其余 5 个非 save_load 条目未实现，非匹配返回 null
#[no_mangle]
pub extern "C" fn SIM_HandleMessage(
    id: c_int,
    size: c_int,
    data: *mut c_schar,
) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        // 对照原版 L225: BinaryBufferReader::BinaryBufferReader(&local_30, (longlong)param_2, param_3)
        let slice = make_slice(data, size);
        let mut reader = BinaryBufferReader::new(slice);
        // 对照原版 L226-238: SimFrameManager::HandleMessage + gSimMessageHandlers 表查找
        dispatch_save_load_message(id, &mut reader)
    }))
    .unwrap_or_else(|_| {
        tracing::error!(id, size, "SIM_HandleMessage panicked, returning null");
        std::ptr::null_mut()
    })
}

/// 对应原版 `SIM_HandleMessages(int id, int size, int count, schar *data)` (L98460 / 01_sim_api.c L248-298)。
///
/// 原版流程：循环 count 次调用单条 HandleMessage，每次推进 data 指针 size 字节，
/// 返回最后一次调用的结果。
///
/// Phase 3 实现：与 SIM_HandleMessage 一致，循环 count 次调用 dispatch_save_load_message。
/// count <= 0 时直接返回 null（与原版 `if (0 < param_3)` 等价）。
#[no_mangle]
pub extern "C" fn SIM_HandleMessages(
    id: c_int,
    size: c_int,
    count: c_int,
    data: *mut c_schar,
) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        // 对照原版 L266: if (0 < param_3)
        if count <= 0 {
            return std::ptr::null_mut();
        }
        let mut current_data = data;
        let mut last_result: *mut c_void = std::ptr::null_mut();
        for _ in 0..count {
            // 对照原版 L272: 每次循环重新构造 BinaryBufferReader
            let slice = make_slice(current_data, size);
            let mut reader = BinaryBufferReader::new(slice);
            // 对照原版 L273-289: SimFrameManager::HandleMessage + gSimMessageHandlers 表查找
            last_result = dispatch_save_load_message(id, &mut reader);
            // 对照原版 L293: param_4 = param_4 + param_2（推进到下一条同 size 消息）
            // 防御：data 初始为 null 时不推进（避免 null.add(n) UB）
            if size > 0 && !current_data.is_null() {
                current_data = unsafe { current_data.add(size as usize) };
            }
        }
        last_result
    }))
    .unwrap_or_else(|_| {
        tracing::error!(id, size, count, "SIM_HandleMessages panicked, returning null");
        std::ptr::null_mut()
    })
}

/// 对应原版 `SIM_BeginSave(int *out_size, int p2, int p3)` (L98220 / 01_sim_api.c L7-171)。
///
/// 原版流程：分配 SaveBuffer，写入 "SIMSAVE" 头 + 元数据 + 所有 cell。
/// Phase 4 实现：调用 save_load::begin_save，成功时 *out_size = buffer.len() 返回指针，
/// 失败时 *out_size = 0 返回 null。
///
/// **运行时影响**：游戏"保存"按钮点击后 C# 收到非空指针 + size，
/// 拷贝字节写入存档文件 → 保存成功（Phase 0 桩时返回 null 导致保存失败）。
#[no_mangle]
pub extern "C" fn SIM_BeginSave(
    out_size: *mut c_int,
    p2: c_int,
    p3: c_int,
) -> *mut c_schar {
    catch_unwind(AssertUnwindSafe(|| {
        match save_load::begin_save(p2, p3) {
            Ok(ptr) => {
                // 写入 out_size（buffer 长度从 G_SAVE_BUFFER 读取）
                let size = {
                    let g = save_load::G_SAVE_BUFFER.lock().unwrap();
                    g.as_ref().map(|b| b.len() as c_int).unwrap_or(0)
                };
                if !out_size.is_null() {
                    unsafe { *out_size = size };
                }
                tracing::info!(size, "SIM_BeginSave: save buffer ready");
                ptr as *mut c_schar
            }
            Err(e) => {
                tracing::error!(error = ?e, "SIM_BeginSave failed, returning null");
                if !out_size.is_null() {
                    unsafe { *out_size = 0 };
                }
                std::ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        tracing::error!("SIM_BeginSave panicked, returning null");
        if !out_size.is_null() {
            unsafe { *out_size = 0 };
        }
        std::ptr::null_mut()
    })
}

/// 对应原版 `SIM_EndSave(void)` (L98408 / 01_sim_api.c L195-204)。
///
/// 原版流程：若 SaveBuffer != null，调用其析构并置 null。
/// Phase 4 实现：调用 save_load::end_save 释放全局 SaveBuffer。
#[no_mangle]
pub extern "C" fn SIM_EndSave() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        save_load::end_save();
    }));
}

/// 对应原版 `SIM_DebugCrash(void)` (L98388)。
///
/// 原版流程：若 !gAssertsDisabled && IsDebuggerPresent()，DebugBreak()；
/// 然后调用 KCrashReporterReportMessage("Assert failed: false")。
///
/// **Phase 0 决策：不真正崩溃**，避免误触发。只记 warn 日志。
/// 运行时影响：原版调用此函数会让游戏崩，Rust 桩只记日志。这避免了
/// 测试时误触导致游戏退出。后续 Phase 若需要真实测试崩溃，可加 feature flag。
#[no_mangle]
pub extern "C" fn SIM_DebugCrash() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        tracing::warn!("SIM_DebugCrash (Phase 0 stub) — not crashing (stub mode)");
    }));
}

/// 对应原版 `SYSINFO_Acquire(void)` (L98582)。
///
/// 原版流程：调用 GetJSONSysInfo() 返回完整系统信息字符串。
/// Phase 0 桩：返回最小 JSON `{"stub":true,"os":"windows","arch":"x86_64"}`。
/// 内存用 leak，由进程退出回收。
#[no_mangle]
pub extern "C" fn SYSINFO_Acquire() -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        tracing::info!("SYSINFO_Acquire (Phase 0 stub) — returning stub JSON");
        unsafe { sysinfo::acquire() as *mut c_char }
    }))
    .unwrap_or_else(|_| {
        tracing::error!("SYSINFO_Acquire panicked, returning null");
        std::ptr::null_mut()
    })
}

/// 对应原版 `SYSINFO_Release(void)` (L98611)。
///
/// 原版流程：释放 QUERIED_SYSTEM_INFO 并置 null。
/// Phase 0 桩：no-op（leak 内存由进程退出回收）。
#[no_mangle]
pub extern "C" fn SYSINFO_Release() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        tracing::info!("SYSINFO_Release (Phase 0 stub) — no-op");
        unsafe { sysinfo::release() }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::SYSINFO_STUB_JSON;

    #[test]
    fn sim_initialize_with_null_handler_does_not_panic() {
        // 多次调用安全（Logger/CrashDump 都是幂等）
        SIM_Initialize(None);
        SIM_Initialize(None);
    }

    #[test]
    fn sim_handle_message_returns_null() {
        // id=0 不在 save_load 集合中，dispatch_save_load_message 走 `_` 分支返回 null
        let result = SIM_HandleMessage(0, 0, std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn sim_handle_messages_returns_null() {
        // count=0，循环不执行，直接返回 null
        let result = SIM_HandleMessages(0, 0, 0, std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn sim_handle_message_dispatches_load_with_empty_buffer() {
        // 验证 Load 消息能正确路由到 save_load::load
        // id=Load=3622429126 超过 i32::MAX，需用 `MessageType::Load as u32 as c_int`
        // 保位转换（u32 → i32 bit-pattern preserving）
        // Load with empty buffer → save_load::load 返回 Err(InvalidMagic)
        // SIM_HandleMessage 应记录 error 并返回 null（不 panic）
        // 不修改任何全局状态（魔数检查在 mutex lock 之前失败），并行安全
        let id: c_int = MessageType::Load as u32 as c_int;
        let result = SIM_HandleMessage(id, 0, std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn sim_handle_message_dispatches_load_with_valid_magic() {
        // 验证 Load 消息通过魔数检查后的完整 dispatch 路径
        // 构造 magic + version=0x10（高版本跳过 header 读取，start_offset=0 跳过所有循环）
        // 此路径仅读取 G_SIM_DATA（read-only，width/updated_cells_ptr），不修改任何全局状态
        // 与 save_load::tests::load_with_valid_magic_and_high_version_does_not_panic 并行安全
        let mut buf: Vec<u8> = vec![0x53, 0x49, 0x4d, 0x53, 0x41, 0x56, 0x45, 0x00]; // "SIMSAVE\0"
        buf.extend_from_slice(&0x10u32.to_le_bytes()); // version = 0x10
        let id: c_int = MessageType::Load as u32 as c_int;
        let result = SIM_HandleMessage(id, buf.len() as c_int, buf.as_ptr() as *mut c_schar);
        assert!(result.is_null());
    }

    #[test]
    fn sim_handle_messages_loops_count_times() {
        // count=2, 每次调用 Load with empty buffer（不修改全局状态，并行安全）
        // 验证循环不 panic，返回 null（最后一次调用的结果）
        let id: c_int = MessageType::Load as u32 as c_int;
        let result = SIM_HandleMessages(id, 0, 2, std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn sim_begin_save_returns_non_null_and_correct_size() {
        // empty SimData (width=0/height=0) → 29 字节头部
        // 注意：此测试依赖 G_SIM_DATA 的 width/height=0（由其他测试并行设置，
        // Mutex 提供同步；即使其他测试设置非零 width/height，begin_save 仍应
        // 返回非空指针，只是 size 不同。这里只断言非空 + size>0）
        let mut size: c_int = -1;
        let result = SIM_BeginSave(&mut size as *mut _, 0, 0);
        assert!(!result.is_null(), "SIM_BeginSave should return non-null pointer");
        assert!(size > 0, "size should be positive, got {}", size);
        // 清理 save buffer（避免泄漏影响后续测试）
        SIM_EndSave();
    }

    #[test]
    fn sim_begin_save_with_null_out_size_does_not_panic() {
        // null out_size 仍应返回非空指针（begin_save 不依赖 out_size）
        let result = SIM_BeginSave(std::ptr::null_mut(), 0, 0);
        assert!(!result.is_null(), "SIM_BeginSave should return non-null even with null out_size");
        SIM_EndSave();
    }

    #[test]
    fn sim_end_save_does_not_panic() {
        SIM_EndSave();
    }

    #[test]
    fn sim_shutdown_does_not_panic() {
        SIM_Shutdown();
    }

    #[test]
    fn sim_debug_crash_does_not_actually_crash() {
        // 桩模式：不应 panic，不应触发 abort
        SIM_DebugCrash();
    }

    #[test]
    fn sysinfo_acquire_returns_non_null_terminated_string() {
        unsafe {
            let ptr = SYSINFO_Acquire();
            assert!(!ptr.is_null());
            // 验证内容（长度来自 abi::SYSINFO_STUB_JSON，单一真源，避免硬编码 50 导致越界）
            let bytes = std::slice::from_raw_parts(ptr as *const u8, SYSINFO_STUB_JSON.len());
            assert_eq!(bytes, SYSINFO_STUB_JSON);
            // 末字节是 NUL
            assert_eq!(*bytes.last().unwrap(), 0u8);
        }
    }

    #[test]
    fn sysinfo_release_does_not_panic() {
        SYSINFO_Release();
    }
}
