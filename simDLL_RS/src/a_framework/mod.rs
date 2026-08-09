//! A类：底层框架
//!
//! 决定游戏与 C# 端的基本被调关系。

pub mod abi;
pub mod buffer;
pub mod crashdump;
pub mod frame_sync;
pub mod game_data;
pub mod game_data_update;
pub mod loader_hook;
pub mod logger;
pub mod message_handler;
pub mod save_load;
pub mod sim;
pub mod sim_api;
pub mod sim_data;
pub mod sim_events;
pub mod sim_frame_manager;
pub mod sim_thread;
pub mod stl_shim;
pub mod sysinfo;
pub mod vector_math;
