//! ProcessMessages — Phase 7 消息处理（应用 C# 格子改造请求）。
//!
//! 对照源码 06_process_messages.c。
//! T7-T10 实现 4 个 Process* 函数；T6 期间为桩。

use crate::sim_data::SimData;
use crate::sim_events::SimFrameInfo;

/// `ProcessCellProperties` — 设置/清除格子属性标志。
///
/// 对照源码 06_process_messages.c L221-322。
/// 遍历 frame.set_cell_properties（每条 12B），game_cell → cell_idx，
/// set_or_clear == 0 → properties[idx] &= !flags，set_or_clear != 0 → properties[idx] |= flags。
/// 若格子非空 → 检查置换需求（DisplaceGas/DisplaceLiquid 桩返回 false，不处理）。
///
/// **运行时影响**：C# 设置格子属性（如 solid/insulated 等）时，对应标志位会真实写入
/// `updated_cells.properties`。游戏画面上对应格子的视觉/物理行为会随属性生效。
pub fn process_cell_properties(frame: &SimFrameInfo, sim_data: &mut SimData) {
    let width = sim_data.width;
    let height_minus_2 = sim_data.height - 2;
    if width <= 2 || height_minus_2 <= 0 {
        return;
    }
    // UniquePtr<CellSOA> 只有 ptr 字段，需 unsafe 解引用
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    let properties = updated_cells.properties.as_mut_slice();
    let mass = updated_cells.mass.as_slice();
    for cp in frame.set_cell_properties.as_slice() {
        if let Some(idx) = crate::sim_frame_manager::game_cell_to_idx(
            cp.game_cell,
            width,
            height_minus_2,
        ) {
            let i = idx as usize;
            if i < properties.len() {
                if cp.set_or_clear == 0 {
                    properties[i] &= !cp.property_flags;
                } else {
                    properties[i] |= cp.property_flags;
                }
                // 若格子非空（mass > 0）→ 检查 DisplaceGas/DisplaceLiquid（桩，跳过）
                if i < mass.len() && mass[i] > 0.0 {
                    // Phase 7：DisplaceGas/DisplaceLiquid 桩返回 false，不处理
                }
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_data::{CellSOA, SimData};
    use crate::sim_events::{CellPropertiesChange, SimFrameInfo};

    /// 辅助：构造测试用 SimData（width=6, height=6 → 内部 4×4，含边界 36 cells）
    /// 并分配 updated_cells（properties/mass 全 0，长度 36）
    fn make_test_sim_data() -> SimData {
        let mut sd = SimData::new_zeroed();
        sd.width = 6;
        sd.height = 6;
        sd.num_game_cells = 16;  // 4×4 内部格子
        let mut cells = Box::new(CellSOA::new());
        cells.properties.resize(36, 0);
        cells.mass.resize(36, 0.0);
        sd.updated_cells.ptr = Box::into_raw(cells);
        sd
    }

    /// 辅助：释放 SimData 中通过 Box::into_raw 分配的 CellSOA
    fn free_test_sim_data(sd: &mut SimData) {
        if !sd.updated_cells.ptr.is_null() {
            unsafe {
                drop(Box::from_raw(sd.updated_cells.ptr));
            }
            sd.updated_cells.ptr = std::ptr::null_mut();
        }
    }

    #[test]
    fn process_cell_properties_sets_flag() {
        let mut frame = SimFrameInfo::default();
        // game_cell=0 → idx=7（width=6, h-2=4）
        frame.set_cell_properties.push(CellPropertiesChange {
            game_cell: 0,
            callback_idx: 0,
            property_flags: 0x01,
            set_or_clear: 1,
            _pad: [0; 2],
        });
        let mut sd = make_test_sim_data();
        process_cell_properties(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert_eq!(cells.properties.get(7), 0x01, "set 应把 flag 位 |= 0x01");
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_cell_properties_clears_flag() {
        let mut frame = SimFrameInfo::default();
        // 预置 properties[7] = 0xff，再用 set_or_clear=0 清除 0xff
        frame.set_cell_properties.push(CellPropertiesChange {
            game_cell: 0,
            callback_idx: 0,
            property_flags: 0xff,
            set_or_clear: 0,
            _pad: [0; 2],
        });
        let mut sd = make_test_sim_data();
        unsafe {
            (*sd.updated_cells.ptr).properties.set(7, 0xff);
        }
        process_cell_properties(&frame, &mut sd);
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            assert_eq!(cells.properties.get(7), 0x00, "clear 应把 flag 位 &= !0xff → 0");
        }
        free_test_sim_data(&mut sd);
    }

    #[test]
    fn process_cell_properties_skips_invalid_width() {
        let mut sd = SimData::new_zeroed();
        sd.width = 0;  // 无效 width
        let frame = SimFrameInfo::default();
        process_cell_properties(&frame, &mut sd);
        // 不 panic 即可
    }

    #[test]
    fn process_cell_properties_skips_null_updated_cells() {
        // updated_cells.ptr 为 null 应不 panic
        let mut sd = SimData::new_zeroed();
        sd.width = 6;
        sd.height = 6;
        // sd.updated_cells.ptr 保持 null
        let frame = SimFrameInfo::default();
        process_cell_properties(&frame, &mut sd);
        // 不 panic 即可
    }

    #[test]
    fn process_cell_properties_skips_out_of_range_game_cell() {
        // game_cell=9999 远超 num_game_cells，越界保护应跳过
        let mut frame = SimFrameInfo::default();
        frame.set_cell_properties.push(CellPropertiesChange {
            game_cell: 9999,
            callback_idx: 0,
            property_flags: 0x01,
            set_or_clear: 1,
            _pad: [0; 2],
        });
        let mut sd = make_test_sim_data();
        process_cell_properties(&frame, &mut sd);
        // 不 panic 且 properties[7..35] 仍为 0
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            for i in 0..36 {
                assert_eq!(cells.properties.get(i), 0, "idx {} 不应被修改", i);
            }
        }
        free_test_sim_data(&mut sd);
    }
}
