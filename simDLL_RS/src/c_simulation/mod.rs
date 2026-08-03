//! C 类物理模拟模块。
//!
//! C1 阶段：Sim 线程基础设施 + ProcessFrame 框架。
//! C2 阶段：真实物理模拟（温度/液体/气体/辐射/疾病）。

pub mod building_temperature;
pub mod conduit_temperature;
pub mod element_chunk;
pub mod frame_processor;
pub mod sim_data_ops;
