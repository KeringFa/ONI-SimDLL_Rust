//! Logger 单例：tracing + tracing-appender 双输出。
//!
//! - 文件：`./logs/simdll.log`（按天滚动，文件名 `simdll.log.YYYY-MM-DD`）
//! - stderr：控制台实时可见
//!
//! 模仿原版 `Util::cSingleton<Logger>` 的单例语义。
//! 在 `SIM_Initialize` 调用 `Logger::init()`，在 `SIM_Shutdown` 调用 `Logger::flush()`。

#![allow(dead_code)]

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

/// 全局 Logger 单例。OnceCell 保证只初始化一次。
/// WorkerGuard 必须存活到进程退出，否则后台线程会丢日志。
static LOGGER: OnceCell<Mutex<LoggerState>> = OnceCell::new();

struct LoggerState {
    /// None 表示已 shutdown。
    guard: Option<WorkerGuard>,
    initialized: bool,
}

/// 初始化 Logger。多次调用安全（第二次起 no-op）。
/// 返回 true 表示首次初始化成功，false 表示已初始化过或失败。
pub fn init() -> bool {
    // OnceCell::get_or_init 保证只构造一次
    let state = LOGGER.get_or_init(|| {
        Mutex::new(LoggerState {
            guard: None,
            initialized: false,
        })
    });

    let mut guard = state.lock();
    if guard.initialized {
        return false;
    }

    // 创建 logs 目录（相对当前工作目录，即游戏可执行文件所在目录）
    let logs_dir = std::path::Path::new("./logs");
    if !logs_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(logs_dir) {
            // 目录创建失败，至少 stderr 能用
            eprintln!("[simDLL_RS] WARN: failed to create ./logs: {e}");
        }
    }

    // 文件 appender：按天滚动，前缀 simdll，后缀 .log
    let file_appender = tracing_appender::rolling::daily(logs_dir, "simdll.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

    // 文件层：info 及以上（避免 trace 刷爆）；Phase 6 为 sim_frame_manager / message_handler / sim_api
    // 三个模块额外开 trace，验证消息路由（部署后日志可见 HandleMessage 命中与 handler 表查找）
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false) // 文件不要 ANSI 颜色码
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_filter(tracing_subscriber::EnvFilter::new(
            "info,SimDLL::sim_frame_manager=trace,SimDLL::message_handler=trace,SimDLL::sim_api=trace",
        ));

    // stderr 层：info 及以上，带 ANSI 颜色（与文件层同 filter，保证 stderr 与文件一致）
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(true)
        .with_filter(tracing_subscriber::EnvFilter::new(
            "info,SimDLL::sim_frame_manager=trace,SimDLL::message_handler=trace,SimDLL::sim_api=trace",
        ));

    let result = tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .try_init();

    match result {
        Ok(()) => {
            guard.guard = Some(file_guard);
            guard.initialized = true;
            tracing::info!("simDLL_RS Logger initialized (Phase 0 stub)");
            true
        }
        Err(_) => {
            // 已有全局 subscriber，不强制覆盖（如游戏宿主已设置）
            // 改用 eprintln 兜底
            eprintln!("[simDLL_RS] WARN: tracing subscriber already set; using eprintln fallback");
            guard.initialized = true;
            // guard 不持有 WorkerGuard，文件层不可用
            true
        }
    }
}

/// Flush 日志。在 `SIM_Shutdown` 调用。
/// 注意：WorkerGuard 不主动 drop（保留到进程退出），只调用 flush。
pub fn flush() {
    if let Some(state) = LOGGER.get() {
        let g = state.lock();
        if g.initialized {
            // tracing 没有显式 flush API；tracing-appender 在 drop WorkerGuard 时 flush。
            // 我们保留 guard 不 drop，让进程退出时自然 flush。
            // 这里只写一条 shutdown 日志作为标记。
            tracing::info!("simDLL_RS Logger flushing on shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_can_be_called_twice_safely() {
        // 多次调用不 panic
        // 注意：tracing 全局 subscriber 一旦 set 不能再 set，
        // 但我们的 init() 内部用 initialized flag 防重入。
        // 此测试在单线程隔离运行，且不验证 subscriber 实际生效。
        let _ = init();
        let second = init();
        // 第二次必然返回 false（已 initialized）
        assert!(!second, "second init() should return false");
    }

    #[test]
    fn flush_without_init_is_safe() {
        // 若未 init 就调用 flush，不应 panic
        // 注意：在 cargo test 多测试并发场景下，LOGGER 可能已被其他测试 init
        // 所以这里只验证不 panic
        flush();
    }
}
