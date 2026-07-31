//! ProcessMessages — Phase 7 消息处理（应用 C# 格子改造请求）。
//!
//! 对照源码 06_process_messages.c。
//! T7-T10 实现 4 个 Process* 函数；T6 期间为桩。

use crate::sim_data::SimData;
use crate::sim_events::SimFrameInfo;

/// `ProcessCellProperties` — 设置/清除格子属性标志。
///
/// 对照源码 06_process_messages.c L221-322。
/// T7 实现，当前桩：仅 log。
pub fn process_cell_properties(_frame: &SimFrameInfo, _sim_data: &mut SimData) {
    // T7 实现
}

/// `ProcessCellModifications` — 添加/替换/置换格子元素。
///
/// 对照源码 06_process_messages.c L1252-1502。
/// T8 实现，当前桩。
pub fn process_cell_modifications(_frame: &SimFrameInfo, _sim_data: &mut SimData) {
    // T8 实现
}

/// `ProcessMassConsumption` — 消耗质量（挖掘等）。
///
/// 对照源码 06_process_messages.c L920-1038。
/// T9 实现，当前桩。
pub fn process_mass_consumption(_frame: &SimFrameInfo, _sim_data: &mut SimData) {
    // T9 实现
}

/// `ProcessMassEmission` — 排放质量（间歇泉等）。
///
/// 对照源码 06_process_messages.c L1042-1248。
/// T10 实现，当前桩。
pub fn process_mass_emission(_frame: &SimFrameInfo, _sim_data: &mut SimData) {
    // T10 实现
}
