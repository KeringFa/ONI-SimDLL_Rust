//! C ABI 导出层：9 个 `SIM_*/SYSINFO_*` 函数。
//!
//! 严格对照源码 L98220-L98625。
//! 每个函数包 `catch_unwind`，panic 不跨 C ABI 边界（release 模式下 panic=abort，
//! 但 test 模式 unwind，catch_unwind 双保险）。

#![allow(dead_code)]

use std::os::raw::{c_char, c_int, c_schar, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::abi::GameMessageHandler;
use crate::abi::SYSINFO_STUB_JSON;
use crate::crashdump;
use crate::globals::G_GAME_MESSAGE_HANDLER;
use crate::logger;
use crate::sysinfo;

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

/// 对应原版 `SIM_HandleMessage(int id, int size, schar *data)` (L98420)。
///
/// 原版流程：若 gSim != null，调用 SimFrameManager::HandleMessage；未处理则查
/// gSimMessageHandlers 表。
/// Phase 0 桩：gSim 不分配，永远返回 null。
#[no_mangle]
pub extern "C" fn SIM_HandleMessage(
    id: c_int,
    size: c_int,
    data: *mut c_schar,
) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        tracing::debug!(id, size, data_ptr = ?data, "SIM_HandleMessage (Phase 0 stub) — returning null");
        std::ptr::null_mut()
    }))
    .unwrap_or_else(|_| {
        tracing::error!(id, size, "SIM_HandleMessage panicked, returning null");
        std::ptr::null_mut()
    })
}

/// 对应原版 `SIM_HandleMessages(int id, int size, int count, schar *data)` (L98460)。
///
/// 原版流程：循环 count 次调用单条 HandleMessage。
/// Phase 0 桩：循环 count 次记日志，不实际处理，返回 null。
#[no_mangle]
pub extern "C" fn SIM_HandleMessages(
    id: c_int,
    size: c_int,
    count: c_int,
    data: *mut c_schar,
) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        tracing::debug!(id, size, count, data_ptr = ?data, "SIM_HandleMessages (Phase 0 stub) — returning null");
        // 不实际循环处理，只记一次日志（性能优先）
        std::ptr::null_mut()
    }))
    .unwrap_or_else(|_| {
        tracing::error!(id, size, count, "SIM_HandleMessages panicked, returning null");
        std::ptr::null_mut()
    })
}

/// 对应原版 `SIM_BeginSave(int *out_size, int p2, int p3)` (L98220)。
///
/// 原版流程：分配 SaveBuffer，写入 "SIMSAVE" 头 + 元数据 + 所有 cell。
/// Phase 0 桩：返回 null，*out_size = 0。
/// 运行时影响：游戏"保存"按钮点击后 C# 收到 null，应弹"无法保存"提示或静默忽略。
#[no_mangle]
pub extern "C" fn SIM_BeginSave(
    out_size: *mut c_int,
    _p2: c_int,
    _p3: c_int,
) -> *mut c_schar {
    catch_unwind(AssertUnwindSafe(|| {
        tracing::warn!("SIM_BeginSave (Phase 0 stub) — save not implemented, returning null");
        if !out_size.is_null() {
            unsafe { *out_size = 0 };
        }
        std::ptr::null_mut()
    }))
    .unwrap_or_else(|_| {
        tracing::error!("SIM_BeginSave panicked, returning null");
        if !out_size.is_null() {
            unsafe { *out_size = 0 };
        }
        std::ptr::null_mut()
    })
}

/// 对应原版 `SIM_EndSave(void)` (L98408)。
///
/// 原版流程：若 SaveBuffer != null，调用其析构并置 null。
/// Phase 0 桩：SaveBuffer 永远 null，no-op。
#[no_mangle]
pub extern "C" fn SIM_EndSave() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        tracing::info!("SIM_EndSave (Phase 0 stub) — no-op (SaveBuffer not allocated)");
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

    #[test]
    fn sim_initialize_with_null_handler_does_not_panic() {
        // 多次调用安全（Logger/CrashDump 都是幂等）
        SIM_Initialize(None);
        SIM_Initialize(None);
    }

    #[test]
    fn sim_handle_message_returns_null() {
        let result = SIM_HandleMessage(0, 0, std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn sim_handle_messages_returns_null() {
        let result = SIM_HandleMessages(0, 0, 0, std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn sim_begin_save_returns_null_and_zero_size() {
        let mut size: c_int = -1;
        let result = SIM_BeginSave(&mut size as *mut _, 0, 0);
        assert!(result.is_null());
        assert_eq!(size, 0);
    }

    #[test]
    fn sim_begin_save_with_null_out_size_does_not_panic() {
        let result = SIM_BeginSave(std::ptr::null_mut(), 0, 0);
        assert!(result.is_null());
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
