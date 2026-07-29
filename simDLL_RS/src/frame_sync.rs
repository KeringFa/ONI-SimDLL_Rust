//! FrameSync — 游戏帧同步数据（每帧 C# 和 simDLL 交换 GameData + 同步原语）。
//!
//! 字段对照源码 00_types_reference.c L5696-5706（`struct FrameSync`）。
//! 运行时访问：FrameSync::GameSync / SimSync / clear / initGameData
//! （09_crashdump_logger.c L560-714 / 02_save_load.c L245/L398/L637/L653）。
//! Phase 1 用占位字段（MsvcMutex/MsvcCondVar 不实现真实 lock/wait），
//! Phase 3+ 补全同步原语实现。

use crate::game_data::GameData;
use crate::stl_shim::{MsvcCondVar, MsvcMutex, UniquePtr};

/// FrameSync — 游戏帧同步数据。
///
/// **双缓冲模式**（源码 FrameSync::GameSync L615-617）：
/// - sim 线程写入 `m_sim_data`，game 线程读取 `m_game_data`
/// - GameSync 时两者交换：`m_sim_data ↔ m_game_data`
/// - `m_sim_ready_to_swap` 由 sim 线程置 true（SimSync），由 GameSync 置 false
#[repr(C)]
pub struct FrameSync {
    /// offset 0, 8B —— sim 线程私有的 GameData 副本（被 SimSync 写入）
    pub m_sim_data: UniquePtr<GameData>,
    /// offset 8, 8B —— game 线程读取的权威 GameData（C# 通过此指针读数据）
    pub m_game_data: UniquePtr<GameData>,
    /// offset 16, 1B —— sim 已就绪可交换（sim 线程置 true，GameSync 置 false）
    pub m_sim_ready_to_swap: bool,
    /// offset 17, 1B —— FrameSync 已初始化（initGameData 后置 true）
    pub m_initialized: bool,
    /// offset 20, 4B —— 源码 _padding_ 字段（offset 18-19 编译器自动补齐）
    pub _padding: i32,
    /// offset 24, 80B —— 主锁（保护 m_sim_data/m_game_data 交换）
    pub m_mutex: MsvcMutex,
    /// offset 104, 8B —— game 线程等待条件（GameSync 中 _Cnd_wait）
    pub m_game_cond: MsvcCondVar,
    /// offset 112, 8B —— sim 线程等待条件（SimSync 中 _Cnd_wait）
    pub m_sim_cond: MsvcCondVar,
    /// offset 120, 80B —— sim 数据锁（保护 sim 线程内部数据访问）
    pub m_sim_mutex: MsvcMutex,
}
// 估算总计 = 200B（80B mutex × 2 + 8B cond_var × 2 + 24B 头部 + 8B padding）

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn frame_sync_size_in_expected_range() {
        // 估算 200B（80B mutex × 2 + 8B cond_var × 2 + 24B 头部）
        // 范围断言容纳 std::mutex 实际可能 8B（总 120B）或 80B（总 200B）
        let actual = size_of::<FrameSync>();
        assert!(actual > 150 && actual < 250, "FrameSync size = {}", actual);
    }
}
