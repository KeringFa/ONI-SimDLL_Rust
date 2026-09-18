//! A类：底层框架
//!
//! 决定游戏与 C# 端的基本被调关系。

pub mod abi;
pub mod buffer;
// crashdump 依赖 Windows SEH（SetUnhandledExceptionFilter + MiniDumpWriteDump），
// Linux 下以 no-op 存根替代（崩溃转储为 Windows 专属诊断能力）。
#[cfg(windows)]
pub mod crashdump;
#[cfg(not(windows))]
pub mod crashdump {
    /// Linux stub：无 SEH/MiniDump 等价实现，注册为空操作。
    pub fn init() {}
}
pub mod frame_sync;
pub mod stage_profiler;
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
