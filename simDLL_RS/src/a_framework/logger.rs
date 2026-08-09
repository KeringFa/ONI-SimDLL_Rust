//! 日志系统：tracing 双输出单例
//!
//! 文件按天滚动（simdll.log.YYYY-MM-DD）+ stderr ANSI。
//! 对照源码 09_crashdump_logger.c 中的 Logger 实现。

use once_cell::sync::OnceCell;
use std::ffi::{c_char, CStr};
use std::path::PathBuf;
use std::sync::Mutex;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::reload;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

struct LoggerState {
    _file_guard: WorkerGuard,
}

static LOGGER: OnceCell<Mutex<LoggerState>> = OnceCell::new();
/// 运行时日志级别控制句柄（logger 初始化后可用；RS_SetLogLevel 动态更新）。
static RELOAD_HANDLE: OnceCell<reload::Handle<EnvFilter, Registry>> = OnceCell::new();

/// 把字符串级别解析为 EnvFilter（off/error/warn/info/debug/trace）。
fn parse_level(level: &str) -> Option<EnvFilter> {
    let normalized = level.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "off" | "error" | "warn" | "info" | "debug" | "trace" => {
            Some(EnvFilter::new(normalized))
        }
        _ => None,
    }
}

/// 动态设置日志级别。返回 false 表示级别非法或 logger 未初始化。
pub fn set_level(level: &str) -> bool {
    let Some(filter) = parse_level(level) else {
        return false;
    };
    match RELOAD_HANDLE.get() {
        Some(handle) => handle.reload(filter).is_ok(),
        None => false,
    }
}

/// FFI：RS_SetLogLevel —— 由 SimDLL_Rust mod 配置调用（UTF-8 NUL 结尾字符串）。
#[no_mangle]
pub extern "C" fn RS_SetLogLevel(level: *const c_char) -> i32 {
    if level.is_null() {
        return 0;
    }
    let level = unsafe { CStr::from_ptr(level) }.to_string_lossy().into_owned();
    if set_level(&level) {
        1
    } else {
        0
    }
}

/// 解析日志目录：SIMDLL_LOG_DIR 环境变量 > 游戏 logs 目录（可创建/可写才用）> 系统临时目录兜底。
fn resolve_log_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SIMDLL_LOG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let game_logs =
        PathBuf::from("D:\\Games\\steam\\steamapps\\common\\OxygenNotIncluded\\logs");
    if std::fs::create_dir_all(&game_logs).is_ok() {
        // 目录可创建不代表可写：探针文件实测，失败即降级临时目录
        let probe = game_logs.join(".simdll_write_probe");
        if std::fs::write(&probe, b"probe").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return game_logs;
        }
    }
    let fallback = std::env::temp_dir().join("simdll_rs_logs");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// 初始化日志系统。
///
/// 返回 true 表示成功初始化，false 表示已初始化过或失败。
/// 日志文件写入游戏 logs 目录：simdll.log.YYYY-MM-DD
pub fn init() -> bool {
    if LOGGER.get().is_some() {
        return false;
    }

    let log_dir = resolve_log_dir();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "simdll.log");
    let (non_blocking_file, file_guard) = tracing_appender::non_blocking(file_appender);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking_file)
        .with_ansi(false);

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true);

    let filter = EnvFilter::new("info");
    let (filter, reload_handle) = reload::Layer::new(filter);

    let result = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init();

    if result.is_err() {
        // subscriber 已设置或失败，用 eprintln 兜底
        eprintln!("simDLL_RS: logger init failed, falling back to eprintln");
        return false;
    }

    let _ = RELOAD_HANDLE.set(reload_handle);
    let _ = LOGGER.set(Mutex::new(LoggerState { _file_guard: file_guard }));
    tracing::info!("simDLL_RS logger initialized");
    true
}

/// 刷新日志。
pub fn flush() {
    if let Some(state) = LOGGER.get() {
        let _guard = state.lock().unwrap();
        // WorkerGuard drop 时会 flush，这里持锁确保顺序
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_can_be_called_without_panic() {
        // init 可能因全局 subscriber 已设置而返回 false，不应 panic
        let _ = init();
    }

    #[test]
    fn flush_does_not_panic() {
        // flush 在未 init 时也不应 panic
        flush();
    }
}
