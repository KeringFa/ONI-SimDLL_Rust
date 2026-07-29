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
