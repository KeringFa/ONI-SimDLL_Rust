//! simDLL_RS - Rust port of Oxygen Not Included's simDLL.dll
//!
//! Phase 0: 仅骨架，模块声明全部占位。

#![allow(dead_code)]

// 模块声明（后续任务逐步填充实现）
pub mod abi;
pub mod buffer;
pub mod crashdump;
pub mod frame_sync;
pub mod logger;
pub mod message_handler;
pub mod sim;
pub mod sim_api;
pub mod sim_frame_manager;
pub mod sysinfo;
pub mod stl_shim;
pub mod vector_math;

pub mod globals {
    //! 全局静态变量，对应原版 simDLL 的全局状态。
    //! Phase 0：仅 GameMessageHandler，其他状态在 Phase 1+ 添加。

    use crate::abi::GameMessageHandler;
    use once_cell::sync::OnceCell;

    /// C# 注册的消息回调。对应原版 `gGameMessageHandler`。
    pub static G_GAME_MESSAGE_HANDLER: OnceCell<GameMessageHandler> = OnceCell::new();
}
