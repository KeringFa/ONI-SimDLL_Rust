//! Crash dump handler：`SetUnhandledExceptionFilter` + `MiniDumpWriteDump`。
//!
//! 严格对照源码 L98741-L98952。
//! - `InitializeCrashDumpHandler` 调用 `SetUnhandledExceptionFilter`
//! - `SimDLLUnhandledExceptionFilter` 流程见模块底部注释
//!
//! Rust 改造说明：
//!   1. 用 `RtlCaptureStackBackTrace` 替代 `StackWalker::ShowCallstack`（不带符号名）。
//!   2. 不调用 `AppVersion::GetRevision/GetBranch`（桩）。
//!   3. 用 `format!` 替代 `StringCchPrintfA`。

#![allow(dead_code)]

use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::System::Diagnostics::Debug::{
    MiniDumpWriteDump, MINIDUMP_TYPE, EXCEPTION_POINTERS, EXCEPTION_CONTINUE_SEARCH,
    SetUnhandledExceptionFilter,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId};
use windows::Win32::Storage::FileSystem::{
    CreateFileA, FILE_SHARE_READ, FILE_SHARE_WRITE, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL,
    FILE_GENERIC_WRITE, FILE_GENERIC_READ,
};
use windows::core::PCSTR;

use crate::abi::{CrashNotifyPayload, GameMessageHandler};
use crate::globals::G_GAME_MESSAGE_HANDLER;

/// 防递归标志。原版用 LOCK/UNLOCK，Rust 用 AtomicBool::compare_exchange。
static IN_HANDLER: AtomicBool = AtomicBool::new(false);

/// 最大捕获栈帧数（原版 StackWalker 构造时传 0x3f = 63）。
const MAX_FRAMES: usize = 64;

/// 初始化崩溃处理器。对应源码 L98741 `InitializeCrashDumpHandler`。
///
/// # Safety
/// 调用 Win32 API `SetUnhandledExceptionFilter`，注册全局异常过滤器。
/// 一旦注册，进程内任何未处理异常都会路由到 `sim_dll_unhandled_exception_filter`。
pub fn initialize() {
    unsafe {
        // SAFETY: SetUnhandledExceptionFilter 注册回调，副作用是修改进程级状态。
        // 回调本身是 unsafe extern "system" fn，签名匹配 EXCEPTION_FILTER_HANDLER。
        SetUnhandledExceptionFilter(Some(sim_dll_unhandled_exception_filter));
    }
}

/// 异常过滤器。对应源码 L98854 `SimDLLUnhandledExceptionFilter`。
///
/// # Safety
/// - 读取 `EXCEPTION_POINTERS` 原始指针
/// - 调用 Win32 API（CreateFileA/MiniDumpWriteDump/CloseHandle/RtlCaptureStackBackTrace）
/// - 调用 C# 注册的 `GameMessageHandler` 回调
unsafe extern "system" fn sim_dll_unhandled_exception_filter(
    exception_pointers: *const EXCEPTION_POINTERS,
) -> i32 {
    // 步骤 1：加锁防递归
    // 原版：LOCK(); bVar3 = !bVar2; if (bVar3) { bVar2 = true; } UNLOCK(); while (!bVar3);
    // Rust 等价：CAS，若已是 true 则表示重入，让出（返回 EXCEPTION_CONTINUE_SEARCH）
    if IN_HANDLER
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        // 重入：让调试器/系统处理
        return EXCEPTION_CONTINUE_SEARCH;
    }

    // 步骤 2：挂调试器时让出（原版 IsDebuggerPresent != 0 时跳过 dump）
    let debugger_present = unsafe { windows::Win32::System::Diagnostics::Debug::IsDebuggerPresent() };
    if debugger_present.as_bool() {
        // 不重置 IN_HANDLER，让后续调试时也只走一次（与原版语义一致）
        return EXCEPTION_CONTINUE_SEARCH;
    }

    // 步骤 3：生成 dump 文件名
    // 原版：StringCchPrintfA(szFileName, 0x104, "%s_%s_%d_%04d%02d%02d-%02d.%02d.%02d.dmp",
    //                       "SimDLL_CRASH", branch, revision, year, month, day, h, m, s)
    // 桩：只用时间戳（branch/revision 不实现）
    let now = chrono::Local::now();
    let dump_filename = format!(
        "SimDLL_CRASH_{}.{}.{}.dmp",
        now.format("%Y%m%d"),
        now.format("%H%M%S"),
        now.format("%Y%m%d") // 占位，文件名有日期就够
    );
    // 完整路径：./logs/<filename>
    let dump_path = format!("./logs/{}", dump_filename);
    tracing::error!(
        dump_path = %dump_path,
        exception_pointers = ?exception_pointers,
        "Unhandled exception caught, writing minidump"
    );

    // 步骤 4：CreateFileA 创建 dump 文件
    // 原版：CreateFileA(szFileName, 0xC0000000, 3, NULL, 2, 0, NULL)
    //   0xC0000000 = GENERIC_READ|GENERIC_WRITE
    //   3 = FILE_SHARE_READ|FILE_SHARE_WRITE
    //   2 = CREATE_ALWAYS
    let path_bytes = format!("{}\0", dump_path); // NUL 终止
    let file_handle = unsafe {
        CreateFileA(
            PCSTR(path_bytes.as_ptr()),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };

    let dump_written = if file_handle.is_ok() {
        let handle = file_handle.unwrap();
        // 步骤 5：MiniDumpWriteDump(GetCurrentProcess, pid, file_handle, 0)
        // 原版：pvVar8 = GetCurrentProcess(); iVar6 = MiniDumpWriteDump(pvVar8, DVar5, pvVar7, 0);
        let proc = unsafe { GetCurrentProcess() };
        let pid = unsafe { GetCurrentProcessId() };
        let result = unsafe {
            MiniDumpWriteDump(
                proc,
                pid,
                handle,
                MINIDUMP_TYPE(0), // 原版传 0，最小 dump
                None,
                None,
                None,
            )
        };
        // 步骤 6：CloseHandle
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
        result.is_ok()
    } else {
        tracing::error!(
            err = ?file_handle.unwrap_err(),
            "CreateFileA failed for dump file"
        );
        false
    };

    tracing::info!(dump_written, "minidump write result");

    // 步骤 7：RtlCaptureStackBackTrace 抓栈（替代原版 StackWalker::ShowCallstack）
    let mut frames: [usize; MAX_FRAMES] = [0; MAX_FRAMES];
    // RtlCaptureStackBackTrace 在 windows crate 中位于 Win32_System_Diagnostics::Debug
    // 但 0.58 版本可能未直接导出，用 GetProcAddress 动态加载
    let frame_count = capture_backtrace(&mut frames);

    // 步骤 8：通知 C#（原版 L98944 (*gGameMessageHandler)(0, &local_e0)）
    // local_e0 指向 [stacktrace_ptr, filename_ptr]
    if let Some(handler) = get_game_message_handler() {
        let stacktrace_str = format!(
            "simDLL_RS crash at {}\n{} frames captured (no symbol resolution yet):\n",
            now.format("%Y-%m-%d %H:%M:%S"),
            frame_count
        );
        // 把帧地址拼成可读字符串
        let mut trace = stacktrace_str;
        for (i, frame) in frames.iter().take(frame_count).enumerate() {
            trace.push_str(&format!("  #{}: 0x{:016x}\n", i, frame));
        }
        // 把字符串 leak 成 C 字符串
        let trace_cstr = leak_c_string(trace);
        let filename_cstr = leak_c_string(dump_filename.clone());

        let payload = CrashNotifyPayload {
            stacktrace: trace_cstr as *const c_char,
            filename: filename_cstr as *const c_char,
        };
        let payload_ptr: *mut CrashNotifyPayload = &payload as *const _ as *mut _;
        let payload_double_ptr: *mut *mut c_void = payload_ptr as *mut *mut c_void;

        tracing::info!("notifying C# game message handler");
        unsafe {
            handler(0, payload_double_ptr);
        }

        // 步骤 9：原版无限 SwitchToThread() 死循环
        // Rust 等价：用 loop + SwitchToThread 让出 CPU，避免 busy-wait
        tracing::error!("entering infinite SwitchToThread loop (preserving original behavior)");
        loop {
            // SwitchToThread 在 windows crate 中需要 Win32_System_Threading
            unsafe {
                let _ = windows::Win32::System::Threading::SwitchToThread();
            }
            // 避免 Rust 编译器优化掉空循环
            std::hint::spin_loop();
        }
    }

    // 若没有 gGameMessageHandler，返回 EXCEPTION_CONTINUE_SEARCH 让系统弹错误对话框
    EXCEPTION_CONTINUE_SEARCH
}

/// 从 G_GAME_MESSAGE_HANDLER 取已注册的回调。
fn get_game_message_handler() -> Option<GameMessageHandler> {
    G_GAME_MESSAGE_HANDLER.get().copied()
}

/// 用 `RtlCaptureStackBackTrace` 抓栈。
/// 返回实际捕获的帧数。
///
/// # Safety
/// 调用 `RtlCaptureStackBackTrace`，它是 kernel32.dll 的导出函数。
fn capture_backtrace(frames: &mut [usize]) -> usize {
    // windows crate 0.58 没有直接绑定 RtlCaptureStackBackTrace
    // 通过 GetModuleHandle + GetProcAddress 动态获取
    use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

    let module_name = b"kernel32.dll\0";
    let proc_name = b"RtlCaptureStackBackTrace\0";

    unsafe {
        let h = match GetModuleHandleA(PCSTR(module_name.as_ptr())) {
            Ok(h) => h,
            Err(_) => return 0,
        };
        let addr = match GetProcAddress(h, PCSTR(proc_name.as_ptr())) {
            Some(a) => a,
            None => return 0,
        };
        let func: unsafe extern "C" fn(u32, u32, *mut *mut c_void, *mut u32) -> u16 =
            std::mem::transmute(addr);
        let mut frames_ptr: [*mut c_void; MAX_FRAMES] = [std::ptr::null_mut(); MAX_FRAMES];
        let captured = func(
            0, // FramesToSkip
            frames.len() as u32,
            frames_ptr.as_mut_ptr(),
            std::ptr::null_mut(),
        );
        for (i, ptr) in frames_ptr.iter().enumerate().take(captured as usize) {
            frames[i] = *ptr as usize;
        }
        captured as usize
    }
}

/// 把 String leak 成 NUL 终止的 C 字符串（`*const c_char`）。
/// 内存不释放，由进程退出回收。原版 C++ 也走类似路径（堆分配后不释放）。
fn leak_c_string(s: String) -> *const c_char {
    let cstr = match std::ffi::CString::new(s) {
        Ok(c) => c,
        Err(_) => return std::ptr::null(),
    };
    let ptr = cstr.as_ptr();
    std::mem::forget(cstr); // 故意泄漏
    ptr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_handler_starts_false() {
        // 验证初始状态
        assert!(!IN_HANDLER.load(Ordering::SeqCst));
    }

    #[test]
    fn leak_c_string_returns_valid_pointer() {
        // 注意：CString::new 会拒绝内部 NUL，但我们传入的是无内部 NUL 的字符串
        let ptr = leak_c_string("hello world".to_string());
        assert!(!ptr.is_null());
        // 验证内容
        unsafe {
            let bytes = std::slice::from_raw_parts(ptr as *const u8, 11);
            assert_eq!(bytes, b"hello world".as_ref());
        }
    }

    #[test]
    fn capture_backtrace_does_not_panic() {
        // 此测试只验证 RtlCaptureStackBackTrace 能加载并调用，不验证帧数
        let mut frames = [0usize; 8];
        let _ = capture_backtrace(&mut frames);
        // 不 panic 即通过
    }

    #[test]
    fn initialize_can_be_called_safely() {
        // SetUnhandledExceptionFilter 注册回调，副作用是进程级
        // 但多次调用安全（覆盖前一个 filter）
        initialize();
        initialize();
    }
}
