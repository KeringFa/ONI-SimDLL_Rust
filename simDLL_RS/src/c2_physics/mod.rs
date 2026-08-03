//! C2 阶段物理模拟桩实现。
//!
//! C1 阶段：所有函数为空桩，不执行任何物理计算。
//! C2 阶段：逐步替换为真实物理计算（温度/液体/气体/辐射/疾病）。

#![allow(dead_code)]

use crate::a_framework::sim_data::SimData;

pub mod liquid_flow;
pub mod property_texture;
pub mod temperature;

/// SimBase::UpdateData 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c L41811。
/// 原版调用 UpdateTemperature/UpdatePressure/UpdateLiquid 等物理更新。
/// C1 桩：不执行任何物理计算，cell 状态保持不变。
pub fn update_data(sim_data: &mut SimData) {
    // 原版 UpdateData L144415：每子步开头 CopyFrom(cells, updatedCells)
    crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);
    // 温度任务（原版 L144430-144548：读 cells 稳定缓冲、写 updatedCells 增量；
    // 含背墙换热、右/下邻换热、逐格 DoStateTransition）
    crate::c2_physics::temperature::run_temperature_task(sim_data);
    // 原版 L144548：温度后 CopyFrom，供后续压力/液体阶段读取新温度
    crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);

    // C2 阶段实现：液体流动
    crate::c2_physics::liquid_flow::update_liquids(sim_data);
    // ElementConsumer 逐帧消费（2026-08-02：水泵/涡轮取水；原版 UpdateData 调组件 Update）
    crate::c_simulation::frame_processor::process_element_consumers(sim_data);
    // UpdateData 压力循环（对照源码 11_msvcrt_ignored.c L42108-42192 的 3 处 UpdatePressure 调用）：
    // 遍历内部格，对液体格向 4 方向调 update_pressure，用返回值累加 flow 分量。
    // 原版语义（L42121-42126 水平 / L42157-42160 垂直）：
    //   flow[本格].y += dir×f39、flow[邻格].x −= dir×f39（水平，dir=±1）
    //   flow[本格].z += f39、flow[邻格].w −= f39（垂直）
    let sim_w = sim_data.width as usize;
    let sim_h = sim_data.height as usize;
    if sim_w < 3 || sim_h < 3 || sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let flow_ptr = sim_data.flow.ptr;
    if flow_ptr.is_null() {
        return; // flow 未分配（测试/异常场景）：无 flow 分量可累加
    }
    let total = sim_w * sim_h;
    let flow = unsafe { std::slice::from_raw_parts_mut(flow_ptr, total) };
    for row in 1..sim_h - 1 {
        for col in 1..sim_w - 1 {
            let cell = row * sim_w + col;
            // 仅液体格（state & 3 == 2 且 mass > 0）——与 update_liquids 同判定
            let is_liquid = {
                let updated = unsafe { &*sim_data.updated_cells.ptr };
                crate::b_elements::elements_table::get_element_liquid_data(updated.element_idx.get(cell))
                    .map(|d| (d.state & 3) == 2)
                    .unwrap_or(false)
                    && updated.mass.get(cell) > 0.0
            };
            if !is_liquid {
                continue;
            }
            // 邻格状态（从 cells 读——update_pressure 读取的缓冲区）：
            // 邻格为固体（state & 3 == 3）时**跳过**压力转移（原版门控 L42106，防固体侵蚀）。
            let [below, above, right, left] = [cell + sim_w, cell - sim_w, cell + 1, cell - 1];
            let neighbor_states = {
                let cells = unsafe { &*sim_data.cells.ptr };
                [below, above, right, left].map(|n| {
                    crate::b_elements::elements_table::get_element_state_by_idx(cells.element_idx.get(n))
                })
            };
            // 下（+w）、上（-w）垂直：flow[cell].z += f39、flow[邻格].w −= f39
            if neighbor_states[0] != Some(3) {
                let f39 = crate::c2_physics::liquid_flow::update_pressure(sim_data, cell, below);
                flow[cell].z += f39;
                flow[below].w -= f39;
            }
            if neighbor_states[1] != Some(3) {
                let f39 = crate::c2_physics::liquid_flow::update_pressure(sim_data, cell, above);
                flow[cell].z += f39;
                flow[above].w -= f39;
            }
            // 右（+1，dir=+1）、左（-1，dir=-1）水平：
            // flow[cell].y += dir×f39、flow[邻格].x −= dir×f39
            if neighbor_states[2] != Some(3) {
                let f39 = crate::c2_physics::liquid_flow::update_pressure(sim_data, cell, right);
                flow[cell].y += f39;
                flow[right].x -= f39;
            }
            if neighbor_states[3] != Some(3) {
                let f39 = crate::c2_physics::liquid_flow::update_pressure(sim_data, cell, left);
                flow[cell].y -= f39;
                flow[left].x += f39;
            }
        }
    }

    // BuildingHeatExchange::Update（原版 L145434 组件阶段：建筑↔格子换热，dt=0.2）
    crate::c_simulation::building_temperature::update_building_heat_exchange(sim_data);
    // ElementChunk::Update（原版 L145434 同一组件阶段：物质碎片换热）
    crate::c_simulation::element_chunk::update_element_chunks(sim_data, 0.2, sim_data.width);
    // ElementEmitter::Update（原版 L145434 同一组件阶段：发射器逐帧发射）
    crate::c_simulation::element_emitter::update_element_emitters(sim_data, 0.2, sim_data.width);
    // BuildingToBuildingHeatExchange::Update（原版 L145434 同一组件阶段：导热板建筑↔建筑换热）
    crate::c_simulation::building_to_building::update_building_to_building(sim_data, 0.2);
}

/// SimData::UpdateComponentsDataListOnly 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c。
/// 原版更新组件数据列表（ElementConsumer/Emitter 等）。
/// C1 桩：不执行任何操作。
pub fn update_components_data_list_only(_sim_data: &mut SimData) {
    // ElementChunk::UpdateDataListOnly（原版 L140202）：无时间流逝帧刷新输出，
    // 保证 GDU elementChunkInfos 非空（否则 C# OnSimRegistered 读 elementChunks 时 NRE）
    crate::c_simulation::element_chunk::update_element_chunks_data_list_only();
}

/// ConsolidateEvents 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c。
/// 原版合并 sim_events 中的重复事件。
/// C1 桩：不执行任何操作。
pub fn consolidate_events(_sim_data: &mut SimData) {
    // C2 阶段实现
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::{CellSOA, SimData};
    use crate::b_elements::element::{
        Element, ElementLiquidData, ElementPostProcessData, ElementPressureData, ElementStateData,
        ElementTemperatureData, INVALID_ELEMENT_INDEX,
    };
    use crate::b_elements::elements_table::G_ELEMENTS_TABLE;
    use crate::LIB_TESTS_LOCK;

    /// 建 3 元素最小表：0=真空(state=0, flow=0)、1=液体(state=2, flow=50, viscosity=50)、2=固体(state=3)。
    /// 调用方需持 LIB_TESTS_LOCK。
    fn init_pressure_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.element_names.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        for (id, state, flow) in [(0i32, 0u8, 0.0f32), (1i32, 2u8, 50.0f32), (2i32, 3u8, 0.0f32)] {
            let mut elem = Element::default();
            elem.id = id;
            elem.state = state;
            elem.flow = flow;
            table.elements.push(elem);
            table.state_data.push(ElementStateData { state });
            // viscosity=50：UpdateLiquid 流率上限读 viscosity（原版 +8），
            // 生产数据 flow=0、viscosity=speed（2026-08-02 根因修复后测试表须给 viscosity）。
            // max_mass=1000：上推分支阈值 max(up×1.01, max_mass) 需要（2026-08-02 追加）。
            table.liquid_data.push(ElementLiquidData {
                state,
                flow,
                viscosity: 50.0,
                max_mass: 1000.0,
                ..Default::default()
            });
            table.pressure_data.push(ElementPressureData { state, flow });
            // post_process 字段默认：sublimate_index=0xffff → 升华路径不触发
            table.post_process_data.push(ElementPostProcessData { state, ..Default::default() });
        }
    }

    /// 同 init_pressure_table，另补安全 temperature_data（elem1 水、elem2 砖，相变惰性）。
    /// 调用方需持 LIB_TESTS_LOCK。
    fn init_temp_task_table() {
        init_pressure_table();
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        let mut water = ElementTemperatureData::default();
        water.state = 2;
        water.specific_heat_capacity = 4.179;
        water.thermal_conductivity = 0.609;
        water.gas_surface_area_multiplier = 1.0;
        water.liquid_surface_area_multiplier = 1.0;
        water.solid_surface_area_multiplier = 1.0;
        water.low_temp = 0.0;
        water.high_temp = 1000.0;
        water.low_temp_transition_idx = INVALID_ELEMENT_INDEX;
        water.high_temp_transition_idx = INVALID_ELEMENT_INDEX;
        let mut rock = ElementTemperatureData::default();
        rock.state = 3;
        rock.specific_heat_capacity = 0.8;
        rock.thermal_conductivity = 2.0;
        rock.gas_surface_area_multiplier = 1.0;
        rock.liquid_surface_area_multiplier = 1.0;
        rock.solid_surface_area_multiplier = 1.0;
        rock.low_temp = 0.0;
        rock.high_temp = 1000.0;
        rock.low_temp_transition_idx = INVALID_ELEMENT_INDEX;
        rock.high_temp_transition_idx = INVALID_ELEMENT_INDEX;
        table.temperature_data.clear();
        table.temperature_data.push(ElementTemperatureData::default()); // 0
        table.temperature_data.push(water); // 1
        table.temperature_data.push(rock); // 2
    }

    /// 6×6（内部 4×4）SimData，cells/updated_cells/flow/accumulated_flow/backwall/sim_events 已分配。
    fn make_sim_data() -> SimData {
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, true);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF; // 真空元素 0 不是 void → 走元素变化分支
        sd
    }

    /// 填充 cells 与 updated_cells 双缓冲的辅助。
    fn fill_both(sd: &mut SimData, f: impl Fn(&mut CellSOA)) {
        unsafe {
            f(&mut *sd.cells.ptr);
            f(&mut *sd.updated_cells.ptr);
        }
    }

    /// K1 生产路径测试（RED 依据）：液体格对真空邻格产生**水平**压力转移。
    /// 场景（6×6，内部格）：
    /// - cell14 (row2,col2) 液体 mass=100；下方格 cell8 (row1,col2) **固体** → 液体不下落，
    ///   且压力循环跳过固体邻格（K1 门控）；右/左/上方格（15/13/20）真空。
    /// update_data 帧末期望（任务 2 修正：右分支已实现，右扩散叠加到真空右邻）：
    /// - update_liquid 左分支（原版 L849-945）：下方固体→跳过下落，向左扩散
    ///   min(min(100,50),100×0.25)=25 到 cell13（flow[14].x += 25）；
    /// - update_liquid 右分支（任务 2 新增，原版 L946-1042）：左分支递减后剩余 75，
    ///   向右扩散 min(min(75,50),75×0.25)=18.75 到 cell15（flow[14].y += 18.75）；
    /// - update_pressure 从 cells 帧初快照读质量（源格 100），4 方向各 clamp 12.5%，
    ///   跳过上方固体（cell8）→ 15/20/13 各得 12.5（cell13 叠加左扩散得 25+12.5=37.5）；
    /// - cell15 = 右扩散 18.75 + 压力 12.5 = 31.25；
    /// - 源格最终 = 100 − 25（左扩散）− 18.75（右扩散）− 3×12.5（压力）= 18.75。
    #[test]
    fn update_data_spreads_liquid_to_vacuum_neighbor() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pressure_table();
        let mut sd = make_sim_data();
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 1); // 液体
            b.mass.set(14, 100.0);
            b.temperature.set(14, 300.0);
            b.element_idx.set(8, 2); // 下方固体
            b.mass.set(8, 500.0);
            b.temperature.set(8, 300.0);
            // 13/15/20 为真空（质量 0，元素 0）
        });
        update_data(&mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // 水平蔓延：右邻真空格被液体填充（右扩散 18.75 + 压力 12.5 = 31.25）
            assert_eq!(updated.element_idx.get(15), 1, "真空右邻格应变为液体元素");
            assert!(
                (updated.mass.get(15) - 31.25).abs() < 1e-3,
                "cell15 应得右扩散 18.75 + 压力 12.5 = 31.25，got {}",
                updated.mass.get(15)
            );
            assert!(
                (updated.temperature.get(15) - 300.0).abs() < 1e-3,
                "温度应整体转移为 300"
            );
            // 上方真空邻格得 12.5（update_pressure 从 cells 帧初快照读质量）
            assert!((updated.mass.get(20) - 12.5).abs() < 1e-3);
            // 左邻：左扩散 25 + 压力 12.5 = 37.5
            assert!(
                (updated.mass.get(13) - 37.5).abs() < 1e-3,
                "cell13 应得左扩散 25 + 压力 12.5 = 37.5，got {}",
                updated.mass.get(13)
            );
            assert!(
                (updated.mass.get(14) - 18.75).abs() < 1e-3,
                "源格应 100−25（左）−18.75（右）−3×12.5（压力）= 18.75，got {}",
                updated.mass.get(14)
            );
            // 固体邻格（cell8）不受压力转移影响（K1 门控）
            assert_eq!(updated.element_idx.get(8), 2, "固体邻格不应被侵蚀");
            assert!((updated.mass.get(8) - 500.0).abs() < 1e-3, "固体质量不变");
            // flow 分量累加：flow[15].x -= f39；flow[14].x 含左扩散 +25
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 36);
            assert!(
                (flow[15].x - (-12.5)).abs() < 1e-3,
                "flow[15].x 应 -12.5，got {}",
                flow[15].x
            );
            assert!(
                (flow[14].x - 25.0).abs() < 1e-3,
                "flow[14].x 应含左扩散 +25，got {}",
                flow[14].x
            );
            // 真空格被填充 → substance 事件（sim15 → game cell 6）
            let events = &*sd.sim_events.ptr;
            let has_cell6 = events.substance_change_info.as_slice().iter().any(|e| e.cell_idx == 6);
            assert!(has_cell6, "应产生 sim15（game6）的 substance 事件");
        }
    }

    /// K1 门控补充：液体格 4 方向全是固体时，压力循环不应转移任何质量/不产生事件。
    #[test]
    fn update_data_skips_pressure_when_all_neighbors_solid() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_pressure_table();
        let mut sd = make_sim_data();
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 1);
            b.mass.set(14, 100.0);
            b.temperature.set(14, 300.0);
            // 4 方向（8 下 / 20 上 / 15 右 / 13 左）全固体
            for c in [8usize, 20, 15, 13] {
                b.element_idx.set(c, 2);
                b.mass.set(c, 500.0);
            }
        });
        update_data(&mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            for c in [8usize, 20, 15, 13] {
                assert_eq!(updated.element_idx.get(c), 2, "固体邻格 {} 不应被侵蚀", c);
                assert!((updated.mass.get(c) - 500.0).abs() < 1e-3);
            }
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 0, "全固体邻格不应产生转移事件");
        }
    }

    #[test]
    fn update_data_syncs_cells_before_temperature_each_substep() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_temp_task_table();
        let mut sd = make_sim_data();
        // cell14 水 300、cell15 砖 400（相邻换热）；两者四周全是同温固体 → 液体流/压力惰性，
        // 隔离验证"温度任务基于上一子步结果"（原版 L144415 每子步开头 CopyFrom）。
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 1);
            b.mass.set(14, 100.0);
            b.temperature.set(14, 300.0);
            b.insulation.set(14, 255);
            b.element_idx.set(15, 2);
            b.mass.set(15, 200.0);
            b.temperature.set(15, 400.0);
            b.insulation.set(15, 255);
            for (cell, temp) in [(8usize, 300.0f32), (20usize, 300.0f32), (13usize, 300.0f32)] {
                b.element_idx.set(cell, 2);
                b.mass.set(cell, 1000.0);
                b.temperature.set(cell, temp);
                b.insulation.set(cell, 255);
            }
            for (cell, temp) in [(9usize, 400.0f32), (21usize, 400.0f32), (16usize, 400.0f32)] {
                b.element_idx.set(cell, 2);
                b.mass.set(cell, 1000.0);
                b.temperature.set(cell, temp);
                b.insulation.set(cell, 255);
            }
        });
        update_data(&mut sd);
        let after_1 = unsafe { (*sd.updated_cells.ptr).temperature.get(14) };
        update_data(&mut sd); // 第二个子步
        let after_2 = unsafe { (*sd.updated_cells.ptr).temperature.get(14) };
        // opening CopyFrom 使第二子步基于第一子步结果：温度继续收敛（而非重复相同 delta 过冲）
        assert!(
            after_2 > after_1,
            "第二子步应继续升温收敛：{after_1} → {after_2}"
        );
        assert!(after_2 < 400.0, "不得越过热侧 400：{after_2}");
    }
}
