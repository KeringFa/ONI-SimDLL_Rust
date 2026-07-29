//! SimFrameManager — 帧管理器（管理帧队列与回放）。
//!
//! 字段对照源码 00_types_reference.c L6468-6479（`struct SimFrameManager`）。
//! 运行时位于 `gSim + 0x50`（01_sim_api.c L227 / 11_msvcrt_ignored.c L2424）。
//! Phase 1 用占位字段，Phase 3+ 补全帧入队/出队/处理逻辑。

use crate::sim_data::ActiveRegion;
use crate::sim_events::SimFrameInfo;
use crate::stl_shim::{MsvcMutex, MsvcVector};

/// NewGameFrame — 新游戏帧辅助结构。
///
/// 源码 00_types_reference.c L6391-6396：
/// - elapsed_seconds: f32
/// - active_region: Region（16B，内联为 4 × i32，与 Task 7 ActiveRegion 同策略）
/// - current_sunlight_intensity: f32
/// - current_cosmic_radiation_intensity: f32
///
/// 大小 = 4 + 16 + 4 + 4 = 28B。
///
/// **注意**：与 `sim_data::ActiveRegion`（24B）是**不同 struct**：
/// - ActiveRegion = Region + 2 floats（24B），用于 `vector<ActiveRegion>`
/// - NewGameFrame = float + Region + 2 floats（28B），用于 SimFrameManager.activeRegion
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NewGameFrame {
    /// offset 0, 4B —— 帧流逝秒数
    pub elapsed_seconds: f32,
    /// offset 4, 4B —— 活跃区域 minimum.x（源码 L6393 region.minimum.x）
    pub active_region_min_x: i32,
    /// offset 8, 4B —— 活跃区域 minimum.y（源码 L6393 region.minimum.y）
    pub active_region_min_y: i32,
    /// offset 12, 4B —— 活跃区域 maximum.x（源码 L6393 region.maximum.x）
    pub active_region_max_x: i32,
    /// offset 16, 4B —— 活跃区域 maximum.y（源码 L6393 region.maximum.y）
    pub active_region_max_y: i32,
    /// offset 20, 4B —— 当前阳光强度
    pub current_sunlight_intensity: f32,
    /// offset 24, 4B —— 当前宇宙辐射强度
    pub current_cosmic_radiation_intensity: f32,
}

/// SimFrameManager — 帧管理器（管理帧队列与回放）。
///
/// 源码 L6468-6479 字段顺序（运行时访问见 08_sim_frame_manager.c L15-172）：
/// 1. frameMutex（std::mutex）
/// 2. currentFrame（SimFrameInfo*）
/// 3. framePool / queuedFrames / activeFrames / processedFrames（4 × vector<SimFrameInfo*>）
/// 4. activeRegion（NewGameFrame）
/// 5. elapsedSeconds（float）
/// 6. activeRegions（vector<ActiveRegion>）
/// 7. numFramesProcessed（int）
#[repr(C)]
pub struct SimFrameManager {
    /// offset 0, 80B —— 帧管理器锁（保护帧队列操作）
    pub frame_mutex: MsvcMutex,
    /// offset 80, 8B —— 当前正在处理的帧指针
    pub current_frame: *mut SimFrameInfo,
    /// offset 88, 32B —— 帧池（可复用的空闲帧）
    pub frame_pool: MsvcVector<*mut SimFrameInfo>,
    /// offset 120, 32B —— 排队等待处理的帧
    pub queued_frames: MsvcVector<*mut SimFrameInfo>,
    /// offset 152, 32B —— 正在处理的帧
    pub active_frames: MsvcVector<*mut SimFrameInfo>,
    /// offset 184, 32B —— 已处理的帧
    pub processed_frames: MsvcVector<*mut SimFrameInfo>,
    /// offset 216, 28B —— 当前活跃区域
    pub active_region: NewGameFrame,
    /// offset 244, 4B —— 流逝秒数
    pub elapsed_seconds: f32,
    /// offset 248, 32B —— 活跃区域列表
    pub active_regions: MsvcVector<ActiveRegion>,
    /// offset 280, 4B —— 已处理帧数
    pub num_frames_processed: i32,
    /// offset 284, 4B —— 编译器补齐到 8 字节边界
    pub _padding: i32,
}
// 估算总计 = 288B（multiple of 8）

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn new_game_frame_size() {
        // 4 + 16 + 4 + 4 = 28B
        assert_eq!(size_of::<NewGameFrame>(), 28);
    }

    #[test]
    fn sim_frame_manager_size_in_expected_range() {
        // 估算 288B（80B mutex + 8B ptr + 4×32B vector + 28B NewGameFrame
        // + 4B f32 + 32B vector + 4B i32 + 4B padding）
        // 范围断言容纳 std::mutex 8B（总 216B）或 80B（总 288B）
        let actual = size_of::<SimFrameManager>();
        assert!(actual > 200 && actual < 350, "SimFrameManager size = {}", actual);
    }
}
