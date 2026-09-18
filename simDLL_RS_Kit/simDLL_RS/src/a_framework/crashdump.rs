//! 崩溃转储处理
//!
//! 对照源码 09_crashdump_logger.c 中的 InitializeCrashDumpHandler 和
//! SimDLLUnhandledExceptionFilter。
//!
//! 流程：SetUnhandledExceptionFilter → MiniDumpWriteDump →
//! RtlCaptureStackBackTrace → 通知 GameMessageHandler → SwitchToThread 死循环。

use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::System::Diagnostics::Debug::{
    MiniDumpWriteDump, MINIDUMP_TYPE, EXCEPTION_POINTERS, EXCEPTION_CONTINUE_SEARCH,
    SetUnhandledExceptionFilter, IsDebuggerPresent,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, SwitchToThread};
use windows::Win32::Storage::FileSystem::{
    CreateFileA, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL,
};
use windows::Win32::Foundation::{CloseHandle, BOOL};
use windows::core::PCSTR;

use crate::globals::G_GAME_MESSAGE_HANDLER;
use super::abi::CrashNotifyPayload;

/// 防递归标志。
static IN_CRASH_HANDLER: AtomicBool = AtomicBool::new(false);

/// 最大捕获栈帧数。
const MAX_FRAMES: usize = 64;

/// 初始化崩溃处理。
///
/// 对照源码 InitializeCrashDumpHandler，注册 SimDLLUnhandledExceptionFilter
/// 为全局未处理异常过滤器。
pub fn init() {
    unsafe {
        let _ = SetUnhandledExceptionFilter(Some(sim_dll_unhandled_exception_filter));
    }
    tracing::info!("Crash dump handler initialized");
}

/// 崩溃异常过滤器。
///
/// 对照源码 SimDLLUnhandledExceptionFilter。
/// # Safety
/// 这是 Windows 异常过滤器回调，由操作系统调用。
unsafe extern "system" fn sim_dll_unhandled_exception_filter(
    exception_info: *const EXCEPTION_POINTERS,
) -> i32 {
    // 步骤 1：防递归（CAS）
    if IN_CRASH_HANDLER
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    // 步骤 2：挂调试器时让出
    if unsafe { IsDebuggerPresent() }.as_bool() {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    // 步骤 3：生成 dump 文件名
    let now = chrono::Local::now();
    let dump_filename = format!("SimDLL_CRASH_{}.dmp", now.format("%Y%m%d_%H%M%S"));
    let dump_path = format!("./logs/{}\0", dump_filename);

    tracing::error!(
        dump_path = %dump_path,
        exception_info = ?exception_info,
        "Unhandled exception caught, writing minidump"
    );

    // 步骤 4：CreateFileA 创建 dump 文件
    let file_handle = unsafe {
        CreateFileA(
            PCSTR(dump_path.as_ptr()),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };

    if file_handle.is_ok() {
        let handle = file_handle.unwrap();
        // 步骤 5：MiniDumpWriteDump（原版传 0，最小 dump，不传 exception info）
        let proc = unsafe { GetCurrentProcess() };
        let pid = unsafe { GetCurrentProcessId() };
        let result = unsafe {
            MiniDumpWriteDump(
                proc,
                pid,
                handle,
                MINIDUMP_TYPE(0),
                None,
                None,
                None,
            )
        };
        let _ = unsafe { CloseHandle(handle) };
        tracing::info!(dump_written = result.is_ok(), "minidump write result");
    }

    // 步骤 6：RtlCaptureStackBackTrace 抓栈（动态加载，windows 0.58 无直接绑定）
    let mut frames: [usize; MAX_FRAMES] = [0; MAX_FRAMES];
    let frame_count = capture_backtrace(&mut frames);

    // 步骤 7：通知 GameMessageHandler（如果已注册）
    if let Some(handler) = *G_GAME_MESSAGE_HANDLER.lock() {
        let stacktrace_str = format!(
            "simDLL_RS crash at {}\n{} frames captured:\n",
            now.format("%Y-%m-%d %H:%M:%S"),
            frame_count
        );
        let mut trace = stacktrace_str;
        for (i, frame) in frames.iter().take(frame_count).enumerate() {
            trace.push_str(&format!("  #{}: 0x{:016x}\n", i, frame));
        }
        let trace_cstr = leak_c_string(trace);
        let filename_cstr = leak_c_string(dump_filename);

        // 原版 09_crashdump_logger.c L186-210：message_id=0 的 payload 为
        // {callstack=堆栈, message=转储文件名, file=null, line=0}（C# 按
        // DLLReportMessageMessage 布局读取：callstack@0/message@8/file@16/line@24）。
        let payload = CrashNotifyPayload {
            callstack: trace_cstr as *const c_char,
            message: filename_cstr as *const c_char,
            file: std::ptr::null(),
            line: 0,
        };
        let payload_ptr: *mut CrashNotifyPayload = &payload as *const _ as *mut _;
        let payload_double_ptr: *mut *mut c_void = payload_ptr as *mut *mut c_void;

        tracing::info!("notifying C# game message handler");
        unsafe {
            handler(0, payload_double_ptr);
        }

        // 步骤 8：原版无限 SwitchToThread() 死循环
        tracing::error!("entering infinite SwitchToThread loop (preserving original behavior)");
        loop {
            unsafe {
                let _ = SwitchToThread();
            }
            std::hint::spin_loop();
        }
    }

    EXCEPTION_CONTINUE_SEARCH
}

/// 用 RtlCaptureStackBackTrace 抓栈（动态加载）。
fn capture_backtrace(frames: &mut [usize]) -> usize {
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
            0,
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

/// 把 String leak 成 NUL 终止的 C 字符串。
fn leak_c_string(s: String) -> *const c_char {
    let cstr = match std::ffi::CString::new(s) {
        Ok(c) => c,
        Err(_) => return std::ptr::null(),
    };
    let ptr = cstr.as_ptr();
    std::mem::forget(cstr);
    ptr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_crash_handler_initially_false() {
        assert!(!IN_CRASH_HANDLER.load(Ordering::SeqCst));
    }

    #[test]
    fn init_can_be_called_without_panic() {
        init();
    }

    #[test]
    fn atomic_bool_swap_works() {
        let flag = AtomicBool::new(false);
        assert!(!flag.swap(true, Ordering::SeqCst));
        assert!(flag.swap(false, Ordering::SeqCst));
    }

    #[test]
    fn crash_notify_payload_can_be_constructed() {
        let payload = CrashNotifyPayload {
            callstack: std::ptr::null(),
            message: std::ptr::null(),
            file: std::ptr::null(),
            line: 0,
        };
        let _ = payload;
    }

    #[test]
    fn capture_backtrace_does_not_panic() {
        let mut frames = [0usize; 8];
        let _ = capture_backtrace(&mut frames);
    }

    #[test]
    fn leak_c_string_returns_valid_pointer() {
        let ptr = leak_c_string("hello world".to_string());
        assert!(!ptr.is_null());
        unsafe {
            let bytes = std::slice::from_raw_parts(ptr as *const u8, 11);
            assert_eq!(bytes, b"hello world".as_ref());
        }
    }

    #[test]
    fn bool_type_compatibility() {
        // 验证 BOOL 类型可用于结构体字段
        let _b = BOOL(0);
        let _b2 = BOOL(1);
    }
}
