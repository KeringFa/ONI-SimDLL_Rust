//! Temperature — 温度模拟模块（Phase 7 T11-T13 实现）。
//!
//! 对照源码 04_temperature.c。
//! T2 期间提供 do_load_time_state_transition 桩以编译通过。
//! T11-T13 实现：
//! - update_temperature / update_temperature_for_backwall_exchange
//! - do_state_transition / do_load_time_state_transition
//! - do_partial_heat_transition / do_partial_melt / calculate_combined_temperatures

use crate::sim_data::SimData;

/// `DoLoadTimeStateTransition` — 加载时一次性相变检查。
///
/// 对照源码 02_save_load.c L104 + 04_temperature.c。
/// T12 替换为真实实现。当前桩：仅 log。
pub fn do_load_time_state_transition(_sim_data: &mut SimData, _cell_idx: i32) {
    // T12 替换为真实实现
}
