//! SimEvents / SimFrameInfo — 模拟事件集合 + 单帧信息。
//!
//! 字段对照源码：
//! - SimFrameInfo: 00_types_reference.c L5406-?（30+ 字段含 vector/ComponentMessages）
//! - SimEvents: 00_types_reference.c L9363-9384（20 个 vector 字段，合计 480B）
//!
//! Phase 1 暂用空 struct 占位（size_of=0）。
//! 字段待 Phase 3 从源码完整定位（需先定义所有引用的子 struct，
//! 如 DigPoint/CellModification/SubstanceChangeInfo/SpawnOreInfo 等）。

/// SimFrameInfo — 单帧信息。
///
/// 源码 L5406 起定义，含 elapsedSeconds + 4B undefined + 30+ vector/ComponentMessages 字段。
/// Phase 1 占位：空 struct（size_of=0），Phase 3 补全字段后此断言会更新。
#[repr(C)]
pub struct SimFrameInfo {
    // Phase 1 占位：字段待 Phase 3 补全
}

/// SimEvents — 模拟事件集合。
///
/// 源码 L9363-9384 定义，含 20 个 vector<X> 字段（substanceChangeInfo/spawnLiquidInfo/.../backwallShouldTransitionInfo）。
/// Phase 1 占位：空 struct（size_of=0），Phase 3 补全字段后此断言会更新。
#[repr(C)]
pub struct SimEvents {
    // Phase 1 占位：字段待 Phase 3 补全
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn sim_frame_info_size() {
        // 占位空 struct = 0 字节（Phase 3 补全字段后此断言会更新）
        assert_eq!(size_of::<SimFrameInfo>(), 0);
    }

    #[test]
    fn sim_events_size() {
        // 占位空 struct = 0 字节（Phase 3 补全字段后此断言会更新）
        assert_eq!(size_of::<SimEvents>(), 0);
    }
}
