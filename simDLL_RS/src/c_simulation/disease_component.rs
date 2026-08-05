//! 病菌消息/组件（阶段 3）：ConsumeDisease、CellDiseaseModification、
//! DiseaseEmitter/DiseaseConsumer 组件。
//!
//! 对照原版 06_process_messages.c（ProcessConsumeDisease L422）、SimDLL_Source.c
//! （CellDiseaseModification L116412-116474）、11_msvcrt_ignored.c
//! （DiseaseEmitter L37492-37790、DiseaseConsumer L37036-37115、GetReachableCells L34451、
//! Flood L33466）。
use crate::a_framework::game_data::DiseaseConsumedCallback;
use crate::a_framework::sim_data::SimData;
use crate::a_framework::sim_frame_manager::SimFrameInfo;

/// GetReachableCells — 4 向 BFS 泛洪（原版 Flood L33466 + GetReachableCells L34451）。
///
/// 从 (x,y) 起，深度 < range 的格全部可达（三个 state 过滤 flag 全 false →
/// 不过滤元素内容）；跳过 x==0/y==0/越界格（原版 L33506-33513 边界检查）。
/// 返回可达 sim 格索引（BFS 顺序：上、左、右、下）。
pub(crate) fn get_reachable_cells(sim: &SimData, x: usize, y: usize, range: usize) -> Vec<usize> {
    let width = sim.width as usize;
    let height = sim.height as usize;
    let mut reachable = Vec::new();
    if range == 0 || x >= width || y >= height || width == 0 || height == 0 {
        return reachable;
    }
    let mut visited = vec![false; width * height];
    let mut queue = std::collections::VecDeque::new();
    visited[y * width + x] = true;
    queue.push_back((x, y, 0usize));
    while let Some((cx, cy, depth)) = queue.pop_front() {
        if depth >= range {
            continue;
        }
        if cx == 0 || cy == 0 || cx >= width || cy >= height {
            continue;
        }
        reachable.push(cy * width + cx);
        let neighbors = [
            (cx, cy.wrapping_sub(1)), // 上
            (cx.wrapping_sub(1), cy), // 左
            (cx + 1, cy),             // 右
            (cx, cy + 1),             // 下
        ];
        for (nx, ny) in neighbors {
            if nx < width && ny < height {
                let nc = ny * width + nx;
                if !visited[nc] {
                    visited[nc] = true;
                    queue.push_back((nx, ny, depth + 1));
                }
            }
        }
    }
    reachable
}

/// ConsumeDisease 处理（原版 06_process_messages.c L422-522）。
///
/// 16B/条：{gameCell i32, callbackIdx i32, percentToConsume f32, maxToConsume i32}。
/// consumed = (int)(count × percent + 0.5)（四舍五入）→ 钳制 maxToConsume →
/// count 扣减 → count<1 清四字段 → callbackIdx≠-1 时 push DiseaseConsumedCallback{12B}。
pub(crate) fn process_consume_disease(frame: &mut SimFrameInfo, sim: &mut SimData) {
    let width = sim.width as i32;
    let game_w = width - 2;
    let bytes = frame.consume_disease.as_slice();
    let mut off = 0usize;
    while off + 16 <= bytes.len() {
        let game_cell = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let callback_idx = i32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
        let percent = f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
        let max_to_consume = i32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap());
        off += 16;
        let sim_cell = (game_cell / game_w + 1) * width + game_cell % game_w + 1;
        if sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
            continue;
        }
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        if (sim_cell as usize) >= updated.disease_idx.len()
            || (sim_cell as usize) >= updated.disease_count.len()
        {
            continue;
        }
        let mut disease_idx = updated.disease_idx.get(sim_cell as usize);
        let mut consumed = 0i32;
        if disease_idx != 0xff {
            let count = updated.disease_count.get(sim_cell as usize);
            consumed = (count as f32 * percent + 0.5) as i32;
            if max_to_consume <= consumed {
                consumed = max_to_consume;
            }
            updated
                .disease_count
                .set(sim_cell as usize, count - consumed);
            if updated.disease_count.get(sim_cell as usize) < 1 {
                crate::c2_physics::liquid_flow::clear_disease(updated, sim_cell as usize);
            }
            disease_idx = updated.disease_idx.get(sim_cell as usize);
        }
        if callback_idx != -1 && !sim.sim_events.ptr.is_null() {
            let events = unsafe { &mut *sim.sim_events.ptr };
            events.disease_consumed_callbacks.push(DiseaseConsumedCallback {
                callback_idx,
                disease_idx,
                pad: [0; 3],
                disease_count: consumed,
            });
        }
    }
    frame.consume_disease.clear_keep_capacity();
}

/// CellDiseaseModification 处理（SimDLL_Source.c L116412-116474，位于 ConsumeDisease 之后）。
///
/// 12B/条：{cellIdx(gameCell) i32, diseaseIdx u8, pad×3, diseaseCount i32}。
/// diseaseIdx==0xff → 直加 count（不改 idx），count<1 清四字段；
/// 否则 Disease::AddDiseaseToCell（阶段 1 强度混合）。
pub(crate) fn process_cell_disease_modifications(frame: &mut SimFrameInfo, sim: &mut SimData) {
    let width = sim.width as i32;
    let game_w = width - 2;
    let bytes = frame.cell_disease_modifications.as_slice();
    let mut off = 0usize;
    while off + 12 <= bytes.len() {
        let cell_idx = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let disease_idx = bytes[off + 4];
        let disease_count = i32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
        off += 12;
        let sim_cell = (cell_idx / game_w + 1) * width + cell_idx % game_w + 1;
        if sim.updated_cells.ptr.is_null() {
            continue;
        }
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        if (sim_cell as usize) >= updated.disease_count.len() {
            continue;
        }
        if disease_idx == 0xff {
            updated
                .disease_count
                .set(sim_cell as usize, updated.disease_count.get(sim_cell as usize) + disease_count);
            if updated.disease_count.get(sim_cell as usize) < 1 {
                crate::c2_physics::liquid_flow::clear_disease(updated, sim_cell as usize);
            }
        } else {
            crate::c2_physics::liquid_flow::add_disease_to_cell(
                updated,
                sim_cell as usize,
                disease_idx,
                disease_count,
            );
        }
    }
    frame.cell_disease_modifications.clear_keep_capacity();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
    use crate::a_framework::sim_data::SimData;
    use crate::a_framework::sim_frame_manager::SimFrameInfo;
    use crate::b_elements::disease::{Disease, DiseaseInfo};
    use crate::b_elements::element::Element;
    use crate::b_elements::elements_table::{CreateElementsTable, DestroyElementsTable};
    use crate::LIB_TESTS_LOCK;

    /// 1 固体元素（state=3），温度范围覆盖 300K（避免 DoStateTransition）。
    fn create_solid_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(1);
        let mut elem = Element::default();
        elem.id = 0;
        elem.state = 3;
        elem.low_temp = -273.0;
        elem.high_temp = 1000.0;
        elem.number_of_gradient_colors = 1;
        let elem_bytes = unsafe {
            std::slice::from_raw_parts(&elem as *const Element as *const u8, 164).to_vec()
        };
        w.write_bytes(&elem_bytes);
        w.write_int(0);
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 1 病菌表（idx0 strength=1.0）挂 G_DISEASE（add_disease_to_cell 混合需要）。
    fn install_disease_table() {
        let mut t = Disease::new();
        t.diseases.push_unchecked(DiseaseInfo {
            hash_id: 0xD3,
            strength: 1.0,
            ..Default::default()
        });
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
        }
        g.0 = Box::into_raw(Box::new(t));
    }

    fn uninstall_disease_table() {
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
            g.0 = std::ptr::null_mut();
        }
    }

    fn make_sim() -> SimData {
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let c = &mut *buf_ptr;
                for i in 0..30usize {
                    c.element_idx.set(i, 0);
                    c.temperature.set(i, 300.0);
                    c.mass.set(i, 1000.0);
                    c.disease_idx.set(i, 0xff);
                    c.disease_count.set(i, 0);
                    c.disease_infestation_tick_count.set(i, 0);
                }
            }
        }
        sd
    }

    fn push_u8(frame: &mut SimFrameInfo, v: u8) {
        frame.consume_disease.push_unchecked(v);
    }

    #[test]
    fn get_reachable_cells_depth_gate_and_bounds() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        let sd = make_sim();
        // 起点 (2,2)=cell14；range=2 → 14 + 上8 + 左13 + 右15 + 下20
        let r = get_reachable_cells(&sd, 2, 2, 2);
        assert_eq!(r, vec![14usize, 8, 13, 15, 20]);
        // range=1 → 仅中心
        let r1 = get_reachable_cells(&sd, 2, 2, 1);
        assert_eq!(r1, vec![14usize]);
        // 边界：起点 (1,1)=cell7，range=3 → x==0/y==0 格被排除
        let r3 = get_reachable_cells(&sd, 1, 1, 3);
        assert!(!r3.contains(&1usize), "x==0 排除");
        assert!(!r3.contains(&6usize), "y==0 排除");
    }

    #[test]
    fn consume_disease_rounds_and_clamps() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        // gameCell=0 → simCell=7（width=6, game_w=4）
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.disease_idx.set(7, 0);
            u.disease_count.set(7, 100);
        }
        let mut frame = SimFrameInfo::default();
        // 消息 1：percent=0.25 → consumed=(int)(100×0.25+0.5)=25 → count75 + callback{7,0,25}
        for b in 0i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 7i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 0.25f32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 1000i32.to_le_bytes() { push_u8(&mut frame, b); }
        // 消息 2：percent=0.5 → 50，max=30 → consumed=30 → count45
        for b in 0i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 8i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 0.5f32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 30i32.to_le_bytes() { push_u8(&mut frame, b); }
        process_consume_disease(&mut frame, &mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 45, "25+30=55 消费 → 45");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.disease_consumed_callbacks.len(), 2);
            let a = events.disease_consumed_callbacks.as_slice();
            assert_eq!(a[0].callback_idx, 7);
            assert_eq!(a[0].disease_idx, 0);
            assert_eq!(a[0].disease_count, 25);
            assert_eq!(a[1].callback_idx, 8);
            assert_eq!(a[1].disease_count, 30);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn consume_disease_clears_and_reports_zero() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.disease_idx.set(7, 0);
            u.disease_count.set(7, 5);
            u.disease_infestation_tick_count.set(7, 9);
            u.disease_growth_accumulated_error.set(7, 1.5);
        }
        let mut frame = SimFrameInfo::default();
        for b in 0i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 42i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 1.0f32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 1000i32.to_le_bytes() { push_u8(&mut frame, b); }
        process_consume_disease(&mut frame, &mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 0);
            assert_eq!(u.disease_idx.get(7), 0xff, "count<1 → 清病菌");
            assert_eq!(u.disease_infestation_tick_count.get(7), 0);
            assert_eq!(u.disease_growth_accumulated_error.get(7), 0.0);
            let events = &*sd.sim_events.ptr;
            let a = events.disease_consumed_callbacks.as_slice();
            assert_eq!(a[0].callback_idx, 42);
            assert_eq!(a[0].disease_idx, 0xff, "清除后回调 diseaseIdx=0xff");
            assert_eq!(a[0].disease_count, 5);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn cell_disease_modification_direct_and_mixed() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        let mut frame = SimFrameInfo::default();
        // {cellIdx=0, diseaseIdx=0xff, count=+10} → cell7 count=10（idx 保持 0xff，原版只加 count）
        for b in 0i32.to_le_bytes() { frame.cell_disease_modifications.push_unchecked(b); }
        frame.cell_disease_modifications.push_unchecked(0xff);
        frame.cell_disease_modifications.push_unchecked(0);
        frame.cell_disease_modifications.push_unchecked(0);
        frame.cell_disease_modifications.push_unchecked(0);
        for b in 10i32.to_le_bytes() { frame.cell_disease_modifications.push_unchecked(b); }
        // {cellIdx=0, diseaseIdx=3, count=5} → AddDiseaseToCell：cur=0xff → 替换 → idx3 count5
        for b in 0i32.to_le_bytes() { frame.cell_disease_modifications.push_unchecked(b); }
        frame.cell_disease_modifications.push_unchecked(3);
        frame.cell_disease_modifications.push_unchecked(0);
        frame.cell_disease_modifications.push_unchecked(0);
        frame.cell_disease_modifications.push_unchecked(0);
        for b in 5i32.to_le_bytes() { frame.cell_disease_modifications.push_unchecked(b); }
        process_cell_disease_modifications(&mut frame, &mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_idx.get(7), 3, "非 0xff → AddDiseaseToCell");
            assert_eq!(u.disease_count.get(7), 5, "替换（cur=0xff）");
        }
        // 消息 1 的直加语义：先加 count=10（idx 保持 0xff），再被消息 2 替换 → 覆盖验证
        // （用第二个独立用例直接验证直加，见下）
        let mut sd2 = make_sim();
        let mut frame2 = SimFrameInfo::default();
        for b in 0i32.to_le_bytes() { frame2.cell_disease_modifications.push_unchecked(b); }
        frame2.cell_disease_modifications.push_unchecked(0xff);
        frame2.cell_disease_modifications.push_unchecked(0);
        frame2.cell_disease_modifications.push_unchecked(0);
        frame2.cell_disease_modifications.push_unchecked(0);
        for b in 10i32.to_le_bytes() { frame2.cell_disease_modifications.push_unchecked(b); }
        process_cell_disease_modifications(&mut frame2, &mut sd2);
        unsafe {
            let u = &*sd2.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 10, "idx=0xff 直加 count");
            assert_eq!(u.disease_idx.get(7), 0xff, "直加路径不改 idx（原版语义）");
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }
}
