//! 日志系统：tracing 双输出单例
//!
//! 文件按天滚动（simdll.log.YYYY-MM-DD）+ stderr ANSI。
//! 对照源码 09_crashdump_logger.c 中的 Logger 实现。

use once_cell::sync::OnceCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

struct LoggerState {
    _file_guard: WorkerGuard,
}

static LOGGER: OnceCell<Mutex<LoggerState>> = OnceCell::new();
/// 日志启用标志：由 RS_SetLogEnabled 设置（发布版只有开/关，无等级）。
/// SIM_Initialize 等入口据此决定是否创建日志文件——默认关闭时完全不生成日志。
static LOG_ENABLED: AtomicBool = AtomicBool::new(false);

/// 日志开关查询（供 perf_probe 等附属输出判断是否应写入——发布版默认关，
/// 玩家开启日志后 simdll_perf.log 才生成）。
pub fn is_log_enabled() -> bool {
    LOG_ENABLED.load(Ordering::Relaxed)
}

/// FFI：RS_SetLogEnabled —— 由 SimDLL_Rust mod 配置调用（纯开关，发布版无等级）。
/// enabled=true 且未初始化 → 惰性初始化（创建日志文件）；false → 不创建。
#[no_mangle]
pub extern "C" fn RS_SetLogEnabled(enabled: bool) -> i32 {
    LOG_ENABLED.store(enabled, Ordering::Relaxed);
    // 关闭且从未初始化：不创建日志文件（默认不启用日志），视为成功。
    if !enabled && LOGGER.get().is_none() {
        return 1;
    }
    // 启用且 logger 未初始化 → 惰性初始化（此时才创建日志文件）。
    if enabled && LOGGER.get().is_none() {
        let _ = init();
    }
    1
}

/// 按启用标志初始化日志：仅当日志已启用（RS_SetLogEnabled(true)）且
/// 尚未初始化时才创建日志文件；默认关闭 → 不生成任何日志文件。
pub fn ensure_init_if_enabled() {
    if LOG_ENABLED.load(Ordering::Relaxed) && LOGGER.get().is_none() {
        let _ = init();
    }
}

/// 解析日志目录：SIMDLL_LOG_DIR 环境变量 > 游戏 logs 目录（可创建/可写才用）> 系统临时目录兜底。
pub(crate) fn resolve_log_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SIMDLL_LOG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    // 动态推导：宿主 exe（OxygenNotIncluded.exe）所在目录下的 logs——
    // 不能写死路径（玩家 Steam 库位置不同）。current_exe 在 DLL 内返回宿主 exe。
    let game_logs = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("logs")))
        .unwrap_or_else(|| PathBuf::from("logs"));
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

    // 发布版固定 info 级（只输出"有价值"日志；每帧 entry/done 等噪音已降为
    // debug/trace，玩家配置无法开启）。内部调试：SIMDLL_LOG_VERBOSE=1 → debug。
    let filter = if std::env::var("SIMDLL_LOG_VERBOSE").map(|v| v == "1").unwrap_or(false) {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

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
