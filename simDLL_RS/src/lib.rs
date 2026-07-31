//! simDLL_RS - Rust port of Oxygen Not Included's simDLL.dll
//!
//! Phase 0: 仅骨架，模块声明全部占位。

#![allow(dead_code)]

// 模块声明（后续任务逐步填充实现）
pub mod abi;
pub mod buffer;
pub mod crashdump;
pub mod disease;
pub mod element;
pub mod elements_table;
pub mod frame_sync;
pub mod game_data;
pub mod game_data_update;
pub mod logger;
pub mod message_handler;
pub mod process_messages;
pub mod save_load;
pub mod sim;
pub mod sim_api;
pub mod sim_data;
pub mod sim_events;
pub mod sim_frame_manager;
pub mod sysinfo;
pub mod stl_shim;
pub mod temperature;
pub mod vector_math;

pub mod globals {
    //! 全局静态变量，对应原版 simDLL 的全局状态。
    //! Phase 0：仅 GameMessageHandler，其他状态在 Phase 1+ 添加。

    use crate::abi::GameMessageHandler;
    use once_cell::sync::OnceCell;

    /// C# 注册的消息回调。对应原版 `gGameMessageHandler`。
    pub static G_GAME_MESSAGE_HANDLER: OnceCell<GameMessageHandler> = OnceCell::new();
}

/// 跨模块共享的测试序列化锁：串行化所有操作全局状态
/// （G_SIM_DATA / G_SAVE_BUFFER / G_ELEMENTS_TABLE / G_DISEASE 等）
/// 的 lib 单元测试，避免并行执行导致 setup 互相覆盖。
/// elements_table::tests / save_load::tests / sim_api::tests 均获取此锁。
#[cfg(test)]
pub(crate) static LIB_TESTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
