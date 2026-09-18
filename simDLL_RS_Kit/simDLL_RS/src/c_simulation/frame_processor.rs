//! ProcessFrame 框架 + Process* 子函数。
//!
//! 对照源码 08_sim_frame_manager.c L1178-1899（ProcessFrame）和
//! 06_process_messages.c（各 Process* 子函数）。
//!
//! 消息处理现状（2026-08-10 更新）：
//! - Dig/ModifyCell/SetInsulation/CellEnergy/MassConsumption/MassEmission 等
//!   均已真实实现并带回归测试（早期 C1 阶段曾为"只清空 vector"桩）。
//! - 残留桩：consolidate_events（单 worker 下行为等价，见 c2_physics/mod.rs）。

use crate::a_framework::sim_data::SimData;
use crate::a_framework::sim_frame_manager::SimFrameInfo;
use crate::b_elements::elements_table;

/// ProcessFrame：处理一帧的所有消息。
///
/// 对照源码 08_sim_frame_manager.c L1178-1899。
/// 依次调用处理步骤（非物理子函数真实实现，物理子函数桩）。
pub fn process_frame(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    // 步骤 1: 内联处理 set_insulation_values
    process_set_insulation_values(frame, sim_data);

    // 步骤 2: 内联处理 set_strength_values
    process_set_strength_values(frame, sim_data);

    // 步骤 3: ProcessCellEnergyModifications（C2 桩）
    process_cell_energy_modifications(frame, sim_data);

    // 步骤 4: ProcessCellProperties
    process_cell_properties(frame, sim_data);

    // 步骤 5: ProcessMassConsumption（C2 桩）
    process_mass_consumption(frame, sim_data);

    // 步骤 6: ProcessMassEmission（C2 桩）
    process_mass_emission(frame, sim_data);

    // 步骤 7: ProcessConsumeDisease（C2 桩）
    process_consume_disease(frame, sim_data);

    // 步骤 8: 内联处理 cell_disease_modifications（C2 桩）
    process_cell_disease_modifications(frame, sim_data);

    // 步骤 9: ProcessRadiationChanges（原版 06_process_messages.c L370-414）
    process_frame_radiation(frame, sim_data);

    // 步骤 10: ProcessDigPoints（C1 真实实现）
    process_dig_points(frame, sim_data);

    // 步骤 11: ProcessCellModifications
    process_cell_modifications(frame, sim_data);

    // 步骤 12: 内联处理 cell_world_zone_modifications
    process_cell_world_zone_modifications(frame, sim_data);

    // 步骤 13: ElementConsumer 消息（Register/Modify/Unregister，2026-08-02 实现）
    process_element_consumer_messages(frame, sim_data);
    process_building_heat_exchange_messages(frame, sim_data);
    // 原版 ProcessFrame 中 BuildingToBuilding 段位于 buildingHeatExchange 与 elementChunk 之间（L116540-116650）
    process_building_to_building_messages(frame, sim_data);
    process_element_chunk_messages(frame, sim_data);
    // 原版 ProcessFrame：ElementConsumer removes → ElementEmitter adds→modifies→removes（L1628-1665）
    process_element_emitter_messages(frame, sim_data);

    // 步骤 14+: 其他 Component 消息处理（C2 桩）
    process_component_messages_stubs(frame, sim_data);

    // RadiationEmitter 消息（阶段 B：adds→modifies→removes）
    crate::c_simulation::radiation_emitter::process_radiation_emitter_messages(frame, sim_data);
    frame.radiation_emitter_messages.adds.clear_keep_capacity();
    frame.radiation_emitter_messages.modifies.clear_keep_capacity();
    frame.radiation_emitter_messages.removes.clear_keep_capacity();

    // DiseaseEmitter 消息（阶段 3：adds 4B → modifies 20B → removes 8B；
    // 原版 ProcessFrame 组件消息段紧跟 radiationEmitter）
    crate::c_simulation::disease_component::process_disease_emitter_messages(frame, sim_data);
    frame.disease_emitter_messages.adds.clear_keep_capacity();
    frame.disease_emitter_messages.modifies.clear_keep_capacity();
    frame.disease_emitter_messages.removes.clear_keep_capacity();

    // DiseaseConsumer 消息（阶段 3：防御链路；C# 无发送方，注册表恒空）
    crate::c_simulation::disease_component::process_disease_consumer_messages(frame, sim_data);
    frame.disease_consumer_messages.adds.clear_keep_capacity();
    frame.disease_consumer_messages.modifies.clear_keep_capacity();
    frame.disease_consumer_messages.removes.clear_keep_capacity();

    // 原版 ProcessFrame 末尾（L117034-117046）：frame.debugProperties → simData.debugProperties。
    // 缺失导致 building_temperature_scale 恒 0 → 建筑↔格子换热 k=0 失效，is_debug_editing 恒 false。
    sim_data.debug_properties = frame.debug_properties;

}

/// ProcessDigPoints：处理挖掘点。
///
/// 对照源码 06_process_messages.c L524-744。
///
/// 关键逻辑：
/// 1. 遍历 dig_points（12 字节步长）
/// 2. 坐标转换 gameCell → simCell
/// 3. 读取元素索引（backwall 决定从 updatedCells 还是 backwall 读取）
/// 4. 如果 callback_idx != -1，push 到 callback_info
/// 5. 检查 element.state & 3 == 3（固体）
/// 6. 如果固体且 !skip_event：读取 mass/temp/diseaseIdx/diseaseCount
///    - 如果 mass > 0：push MassConsumedCallback 到 dig_info（复用 SpawnOreInfo）
/// 7. 清除单元格（ClearCell 或 ClearBackwall）
/// 8. 如果 backwall：push BackwallElementChangedInfo
/// 9. push SubstanceChangeInfo（oldElementIdx=0xFFFF, newElementIdx=0xFFFF）
/// 10. 设置 timers[simCell] |= 0x1F
/// 11. 清空 dig_points
pub fn process_dig_points(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.dig_points.clear_keep_capacity();
        return;
    }

    if sim_data.updated_cells.ptr.is_null() || sim_data.backwall.ptr.is_null() || sim_data.sim_events.ptr.is_null() {
        tracing::warn!("process_dig_points: updated_cells/backwall/sim_events ptr is null");
        frame.dig_points.clear_keep_capacity();
        return;
    }

    let dig_points_slice = frame.dig_points.as_slice();
    let count = dig_points_slice.len();

    for i in 0..count {
        let dig_point = dig_points_slice[i];

        // 1. 坐标转换：gameCell → simCell
        // 公式：simCell = (gameCell % (width-2) + 1) + (gameCell / (width-2) + 1) * width
        let internal_cell = ((dig_point.game_cell % game_w + 1) as i32)
            + ((dig_point.game_cell / game_w + 1) as i32) * sim_data.width;
        let cell_idx = internal_cell as usize;

        // 2. 读取元素索引
        let elem_idx = if dig_point.backwall == 0 {
            let updated_cells = unsafe { &*sim_data.updated_cells.ptr };
            updated_cells.element_idx.get(cell_idx)
        } else {
            let backwall = unsafe { &*sim_data.backwall.ptr };
            backwall.element_idx.get(cell_idx)
        };

        // 3. 如果 callback_idx != -1，push 到 callback_info
        if dig_point.callback_idx != -1 {
            let sim_events = unsafe { &mut *sim_data.sim_events.ptr };
            sim_events.callback_info.push(crate::a_framework::game_data::CallbackInfo {
                callback_idx: dig_point.callback_idx,
            });
        }

        // 4. 检查元素是否为固体（state & 3 == 3）
        let is_solid = match elements_table::get_element_state_by_idx(elem_idx) {
            Some(state) => state == 3,
            None => false,
        };

        if is_solid {
            // 5. 如果 !skip_event，生成 MassConsumedCallback（push 到 dig_info）
            if dig_point.skip_event == 0 {
                let (mass, temp, disease_idx, disease_count) = if dig_point.backwall == 0 {
                    let updated_cells = unsafe { &*sim_data.updated_cells.ptr };
                    let mass = updated_cells.mass.get(cell_idx);
                    let temp = updated_cells.temperature.get(cell_idx);
                    let disease_idx = updated_cells.disease_idx.get(cell_idx);
                    let disease_count = updated_cells.disease_count.get(cell_idx);
                    (mass, temp, disease_idx, disease_count)
                } else {
                    let backwall = unsafe { &*sim_data.backwall.ptr };
                    let mass = backwall.mass.get(cell_idx);
                    let temp = backwall.temperature.get(cell_idx);
                    (mass, temp, 0xFFu8, 0i32)
                };

                if mass > 0.0 {
                    // 反向坐标转换：simCell → gameCell
                    let game_cell_out = ((internal_cell % sim_data.width) - 1)
                        + ((internal_cell / sim_data.width) - 1) * game_w;

                    if game_cell_out >= 0 && game_cell_out < sim_data.num_game_cells {
                        // push MassConsumedCallback 到 dig_info（复用 SpawnOreInfo）
                        // 对照源码 L667-688：callbackIdx 字段复用存 gameCell
                        let sim_events = unsafe { &mut *sim_data.sim_events.ptr };
                        sim_events.dig_info.push(crate::a_framework::game_data::SpawnOreInfo {
                            cell_idx: game_cell_out,  // 对应源码 callbackIdx = gameCell
                            elem_idx: elem_idx,
                            disease_idx: disease_idx,
                            pad: 0,
                            mass: mass,
                            temperature: temp,
                            disease_count: disease_count,
                        });
                    }
                }
            }

            // 6. 移除单元格元素
            if dig_point.backwall == 0 {
                crate::c_simulation::sim_data_ops::clear_cell(sim_data, internal_cell);
            } else {
                crate::c_simulation::sim_data_ops::clear_backwall(sim_data, internal_cell);

                // 生成 BackwallElementChangedInfo
                let game_cell_out = ((internal_cell % sim_data.width) - 1)
                    + ((internal_cell / sim_data.width) - 1) * game_w;
                if game_cell_out >= 0 && game_cell_out < sim_data.num_game_cells {
                    let sim_events = unsafe { &mut *sim_data.sim_events.ptr };
                    sim_events.backwall_element_changed_info.push(
                        crate::a_framework::game_data::BackwallElementChangedInfo {
                            game_cell: game_cell_out as u32,
                        }
                    );
                }
            }

            // 7. 生成 SubstanceChangeInfo
            // 对照源码 06_process_messages.c L714-733。
            //
            // 原版此处 old/new 均填 0xFFFF 占位——真实值由
            // CopySimDataToGame 的 SubstanceChangeInfo 后处理
            // （sim_data_ops.rs，对照源码 L132321-132337）用
            // mGameData(旧)/mSimData(新) 的 cells 回填，并派生
            // solidInfo/solidSubstanceChangeInfo/liquidChangeInfo。
            let game_cell_out = ((internal_cell % sim_data.width) - 1)
                + ((internal_cell / sim_data.width) - 1) * game_w;
            if game_cell_out >= 0 && game_cell_out < sim_data.num_game_cells {
                let sim_events = unsafe { &mut *sim_data.sim_events.ptr };
                sim_events.substance_change_info.push(
                    crate::a_framework::game_data::SubstanceChangeInfo {
                        cell_idx: game_cell_out,
                        old_element_idx: 0xFFFF,
                        new_element_idx: 0xFFFF,
                    }
                );
            }

            // 8. 设置定时器位标志（timers[cell] |= 0x1F）
            // 对照源码 L734-735
            if !sim_data.timers.ptr.is_null() {
                unsafe {
                    let timers_ptr = sim_data.timers.ptr as *mut u8;
                    let old_val = std::ptr::read(timers_ptr.add(cell_idx));
                    std::ptr::write(timers_ptr.add(cell_idx), old_val | 0x1F);
                }
            }
        }
    }

    // 9. 清空 dig_points
    frame.dig_points.clear_keep_capacity();
}

/// ProcessSetInsulationValues：处理绝缘值设置。
/// 对照源码 ProcessFrame L1218-1234。
fn process_set_insulation_values(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.set_insulation_values.clear_keep_capacity();
        return;
    }
    if sim_data.updated_cells.ptr.is_null() {
        frame.set_insulation_values.clear_keep_capacity();
        return;
    }
    let slice = frame.set_insulation_values.as_slice();
    let count = slice.len();
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = ((entry.game_cell % game_w + 1) as i32)
            + ((entry.game_cell / game_w + 1) as i32) * sim_data.width;
        updated_cells
            .insulation
            .set(internal_cell as usize, insulation_u8(entry.value));
    }
    if count > 0 {
        let sample = slice[0];
        tracing::info!(
            count,
            first_value = sample.value,
            first_byte = insulation_u8(sample.value),
            "SetInsulationValue processed"
        );
    }
    frame.set_insulation_values.clear_keep_capacity();
}

/// 原版 L116370：`insulation = (u8)(value × 255.0)`——C# 发送 0-1 导热系数（1.0=满导热）。
/// 此前直接 `as u8` 截断：1.0f → 1（≈0 导热），导致格子间/背墙换热全部失效（k = ins²×1.53787e-05×TC ≈ 0）。
fn insulation_u8(value: f32) -> u8 {
    (value * 255.0) as u8
}

/// ProcessSetStrengthValues：处理强度值设置。
/// 对照源码 ProcessFrame L1235-1259。
fn process_set_strength_values(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.set_strength_values.clear_keep_capacity();
        return;
    }
    if sim_data.updated_cells.ptr.is_null() {
        frame.set_strength_values.clear_keep_capacity();
        return;
    }
    let slice = frame.set_strength_values.as_slice();
    let count = slice.len();
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = ((entry.game_cell % game_w + 1) as i32)
            + ((entry.game_cell / game_w + 1) as i32) * sim_data.width;
        updated_cells.strength_info.set(internal_cell as usize, entry.value as u8);
    }
    frame.set_strength_values.clear_keep_capacity();
}

/// ProcessCellProperties：处理单元格属性设置/清除。
/// 对照源码 06_process_messages.c L262-269。
fn process_cell_properties(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.set_cell_properties.clear_keep_capacity();
        return;
    }
    if sim_data.updated_cells.ptr.is_null() {
        frame.set_cell_properties.clear_keep_capacity();
        return;
    }
    let slice = frame.set_cell_properties.as_slice();
    let count = slice.len();
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = ((entry.game_cell % game_w + 1) as i32)
            + ((entry.game_cell / game_w + 1) as i32) * sim_data.width;
        let idx = internal_cell as usize;
        // entry.property_flags = mask, entry.set_or_clear = set(1)/clear(0) flag
        let mask = entry.property_flags;
        let set_flag = entry.set_or_clear;
        let (props, mass, elem) = {
            let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
            let current = updated_cells.properties.get(idx);
            let new_val = if set_flag == 0 {
                current & !mask
            } else {
                current | mask
            };
            updated_cells.properties.set(idx, new_val);
            (
                new_val,
                updated_cells.mass.get(idx),
                updated_cells.element_idx.get(idx),
            )
        };
        // 原版 06_process_messages.c L270-297：属性变更后，若格有质量且新属性命中位移标志，
        // 触发 DisplaceLiquid / DisplaceGas 挤压（bit0==0 && bit1!=0 → 液体；bit0!=0 → 气体）。
        // 2026-08-07 补齐：此前只写 properties 字节，建筑放置/属性驱动位移链路缺失。
        if props != 0 && mass > 0.0 {
            if props & 1 == 0 {
                if props & 2 != 0 {
                    crate::c2_physics::liquid_flow::displace_liquid(sim_data, idx, elem);
                }
            } else {
                crate::c2_physics::liquid_flow::displace_gas(sim_data, idx, elem);
            }
        }
        // 原版 L298-311：callback_idx != -1 → 推 callback_info（C# 协程等待该回调）
        if entry.callback_idx != -1 {
            let events = unsafe { &mut *sim_data.sim_events.ptr };
            events.callback_info.push(crate::a_framework::game_data::CallbackInfo {
                callback_idx: entry.callback_idx,
            });
        }
    }
    frame.set_cell_properties.clear_keep_capacity();
}

/// ProcessCellModifications：处理单元格修改。
/// 对照源码 06_process_messages.c ProcessCellModifications。
/// ProcessCellModifications：处理单元格修改（ModifyCell 消息）。
/// 对照源码 cellmodifications.cpp（SimDLL_Source.c L134789-135039）。
///
/// 按 replace_mode 分派（源码 L134880-135018）：
/// - 1 (Replace)：replace_element，完整对照源码 L135141-135225
/// - 0 (Add/按状态累加)：简化实现——写字段 + change_substance 通知
///   （原版 AddGas/AddSolid/AddLiquid 的合并语义属 C2 物理，TODO C2）
/// - 2 (ReplaceAndDisplace)：目标格液体 → displace_liquid；气体 → displace_gas；
///   然后写字段 + change_substance 通知（K2 修复，原版 DisplaceGas/DisplaceLiquid 置换语义）。
/// 每条消息末尾：callback_idx != -1 → push callback_info（源码 L135019-135031）。
fn process_cell_modifications(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.cell_modifications.clear_keep_capacity();
        return;
    }
    if sim_data.updated_cells.ptr.is_null() || sim_data.sim_events.ptr.is_null() {
        frame.cell_modifications.clear_keep_capacity();
        return;
    }
    let slice = frame.cell_modifications.as_slice();
    let count = slice.len();
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = ((entry.game_cell % game_w + 1) as i32)
            + ((entry.game_cell / game_w + 1) as i32) * sim_data.width;
        let idx = internal_cell as usize;

        match entry.replace_mode {
            1 => replace_element(sim_data, idx, &entry),
            // 2 (ReplaceAndDisplace)：先把目标格的液体/气体挤压到邻格，再写新字段
            // （原版 06_process_messages.c L250-297：液体 → DisplaceLiquid；气体 → DisplaceGas）。
            // K2 修复：此前为简化写入（TODO C2），挤压位移链路在生产中永不执行。
            2 => {
                let cur_elem = {
                    let updated_cells = unsafe { &*sim_data.updated_cells.ptr };
                    updated_cells.element_idx.get(idx)
                };
                let cur_state = elements_table::get_element_post_process_data(cur_elem)
                    .map(|p| p.state & 3)
                    .unwrap_or(0);
                if cur_state == 2 {
                    crate::c2_physics::liquid_flow::displace_liquid(sim_data, idx, cur_elem);
                } else if cur_state == 1 {
                    crate::c2_physics::liquid_flow::displace_gas(sim_data, idx, cur_elem);
                }
                let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
                updated_cells.element_idx.set(idx, entry.element_idx);
                updated_cells.temperature.set(idx, entry.temperature);
                updated_cells.mass.set(idx, entry.mass);
                updated_cells.disease_count.set(idx, entry.disease_count);
                updated_cells.disease_idx.set(idx, entry.disease_idx);
                change_substance(sim_data, idx);
            }
            // 0 (Add)：严格对照原版 cellmodifications.cpp 分派（SimDLL_Source.c L134880-135018）——
            // 按**被添加元素**形态分派：液体→add_liquid（合并吸收为一格）、气体→add_gas、固体→add_solid；
            // mass<0 → 目标同形态减质量（<=0 清格 + 事件）。
            // 2026-08-02 修复：此前直接覆盖质量（吞掉落点原有液体），液滴落地回注语义错误。
            _ => {
                if entry.replace_mode != 0 {
                    tracing::warn!(
                        "process_cell_modifications: invalid replace_mode {} (game_cell={})",
                        entry.replace_mode, entry.game_cell
                    );
                }
                let mass = entry.mass;
                let add_state = crate::b_elements::elements_table::get_element_post_process_data(
                    entry.element_idx,
                )
                .map(|p| p.state & 3)
                .unwrap_or(0xFF);
                if mass < 0.0 {
                    // 原版 L134892-134955 负质量：目标同形态 → 减质量，<=0 清格 + 事件
                    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
                    let cur_state =
                        crate::b_elements::elements_table::get_element_post_process_data(
                            updated_cells.element_idx.get(idx),
                        )
                        .map(|p| p.state & 3)
                        .unwrap_or(0xFF);
                    if cur_state == add_state {
                        let new_mass = (updated_cells.mass.get(idx) + mass).max(0.0);
                        updated_cells.mass.set(idx, new_mass);
                        if new_mass <= 1.1754944e-38 {
                            updated_cells.element_idx.set(idx, sim_data.vacuum_element_idx);
                            updated_cells.mass.set(idx, 0.0);
                            updated_cells.temperature.set(idx, 0.0);
                            updated_cells.disease_idx.set(idx, 0xFF);
                            updated_cells.disease_count.set(idx, 0);
                            updated_cells.disease_infestation_tick_count.set(idx, 0);
                            updated_cells.disease_growth_accumulated_error.set(idx, 0.0);
                            change_substance(sim_data, idx);
                        }
                    }
                } else if mass > 0.0 {
                    let (elem, temp, didx, dcount) = (
                        entry.element_idx,
                        entry.temperature,
                        entry.disease_idx,
                        entry.disease_count,
                    );
                    match add_state {
                        1 => crate::c2_physics::liquid_flow::add_gas(
                            sim_data, idx, elem, mass, temp, didx, dcount,
                        ),
                        2 => crate::c2_physics::liquid_flow::add_liquid(
                            sim_data, idx, elem, mass, temp, didx, dcount,
                        ),
                        3 => crate::c2_physics::liquid_flow::add_solid(
                            sim_data, idx, elem, mass, temp, didx, dcount, entry.flags,
                        ),
                        _ => {
                            // 真空元素 Add → 原版 "Invalid replacement type" 崩溃；安全跳过
                            tracing::warn!(
                                "process_cell_modifications: add vacuum element ignored (game_cell={})",
                                entry.game_cell
                            );
                        }
                    }
                }
            }
        }

        // callback_idx != -1 → push callback_info（源码 L135019-135031）
        if entry.callback_idx != -1 {
            let sim_events = unsafe { &mut *sim_data.sim_events.ptr };
            sim_events.callback_info.push(crate::a_framework::game_data::CallbackInfo {
                callback_idx: entry.callback_idx,
            });
        }
    }
    frame.cell_modifications.clear_keep_capacity();
}

/// ReplaceElement（对照源码 L135141-135225）。
///
/// 1. 写入 element_idx
/// 2. 新元素 state==0（Vacuum/Void）→ 温度钳 0、质量钳 0
/// 3. 写入 temperature / mass
/// 4. ChangeSubstance 通知（substance_change_info + timers 标脏）
/// 5. 写入 disease 三字段并清零侵扰计数/增长误差
fn replace_element(
    sim_data: &mut SimData,
    sim_cell_idx: usize,
    entry: &crate::a_framework::sim_events::CellModification,
) {
    let idx = sim_cell_idx;
    let state = elements_table::get_element_state_by_idx(entry.element_idx).unwrap_or(0);
    let mut temperature = entry.temperature;
    let mut mass = entry.mass;
    {
        let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
        updated_cells.element_idx.set(idx, entry.element_idx);
        if state == 0 {
            temperature = 0.0;
        }
        updated_cells.temperature.set(idx, temperature);
        if state == 0 {
            mass = 0.0;
        }
        updated_cells.mass.set(idx, mass);
    }

    // ChangeSubstance（源码 L135201）
    change_substance(sim_data, idx);

    // disease 字段（源码 L135202-135221：diseaseIdx/diseaseCount/侵扰计数/增长误差）
    let updated_cells = unsafe { &mut *sim_data.updated_cells.ptr };
    updated_cells.disease_idx.set(idx, entry.disease_idx);
    updated_cells.disease_count.set(idx, entry.disease_count);
    updated_cells.disease_infestation_tick_count.set(idx, 0u8);
    updated_cells.disease_growth_accumulated_error.set(idx, 0.0f32);
}

/// SimEvents::ChangeSubstance（对照源码 L133152-133186）。
///
/// simCell → gameCell 坐标转换，push substance_change_info
/// （old/new 填 0xFFFF 占位，由 CopySimDataToGame 后处理回填并派生
/// solidInfo/solidSubstanceChangeInfo/liquidChangeInfo），
/// 并标记 timers[simCell] |= 0x1F。
fn change_substance(sim_data: &mut SimData, sim_cell_idx: usize) {
    let width = sim_data.width;
    let game_w = width - 2;
    let internal = sim_cell_idx as i32;
    let game_cell = (internal % width - 1) + (internal / width - 1) * game_w;
    if game_cell >= 0 && game_cell < sim_data.num_game_cells {
        let sim_events = unsafe { &mut *sim_data.sim_events.ptr };
        sim_events.substance_change_info.push(
            crate::a_framework::game_data::SubstanceChangeInfo {
                cell_idx: game_cell,
                old_element_idx: 0xFFFF,
                new_element_idx: 0xFFFF,
            }
        );
    }
    if !sim_data.timers.ptr.is_null() {
        unsafe {
            let timers_ptr = sim_data.timers.ptr as *mut u8;
            let old_val = std::ptr::read(timers_ptr.add(sim_cell_idx));
            std::ptr::write(timers_ptr.add(sim_cell_idx), old_val | 0x1F);
        }
    }
}

/// ProcessCellWorldZoneModifications：处理世界区域修改。
/// 对照源码 ProcessFrame L1331-1343。
fn process_cell_world_zone_modifications(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.cell_world_zone_modifications.clear_keep_capacity();
        return;
    }
    if sim_data.world_zones.ptr.is_null() {
        frame.cell_world_zone_modifications.clear_keep_capacity();
        return;
    }
    let slice = frame.cell_world_zone_modifications.as_slice();
    let count = slice.len();
    let world_zones = sim_data.world_zones.ptr as *mut u8;
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = ((entry.game_cell % game_w + 1) as i32)
            + ((entry.game_cell / game_w + 1) as i32) * sim_data.width;
        unsafe {
            std::ptr::write(world_zones.add(internal_cell as usize), entry.zone_id);
        }
    }
    frame.cell_world_zone_modifications.clear_keep_capacity();
}

// ===== C2 桩子函数 =====

/// ProcessCellEnergyModifications（原版 06_process_messages.c L134-211）。
/// 每条 16B：cellIdx i32 / kilojoules f32 / maxTemperature f32 / id i32（id 仅日志）。
/// ΔT = kilojoules/(mass×SHC)；new = min(旧+ΔT, max(旧, maxT))；钳制 [1,10000]；写后 DoStateTransition。
fn process_cell_energy_modifications(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.cell_energy_modifications.clear_keep_capacity();
        return;
    }
    if sim_data.updated_cells.ptr.is_null() || sim_data.sim_events.ptr.is_null() {
        frame.cell_energy_modifications.clear_keep_capacity();
        return;
    }
    let slice = frame.cell_energy_modifications.as_slice();
    let n = slice.len() / 16;
    for i in 0..n {
        let base = i * 16;
        let cell_idx =
            i32::from_le_bytes([slice[base], slice[base + 1], slice[base + 2], slice[base + 3]]);
        let kj = f32::from_le_bytes([
            slice[base + 4],
            slice[base + 5],
            slice[base + 6],
            slice[base + 7],
        ]);
        let max_temp = f32::from_le_bytes([
            slice[base + 8],
            slice[base + 9],
            slice[base + 10],
            slice[base + 11],
        ]);
        let internal = (cell_idx % game_w + 1) + (cell_idx / game_w + 1) * sim_data.width;
        let cell = internal as usize;
        let updated = unsafe { &*sim_data.updated_cells.ptr };
        if cell >= updated.mass.len() || cell >= updated.temperature.len() {
            continue;
        }
        let elem = updated.element_idx.get(cell);
        let Some(etd) = crate::b_elements::elements_table::get_element_temperature_data(elem) else {
            continue;
        };
        if (etd.state & 3) == 0 {
            continue; // 真空
        }
        let mass = updated.mass.get(cell);
        // 原版 06_process_messages.c L172：守卫是 mass > 0.001 && maxTemp > 0，
        // 完全不检查 kJ 符号——负 kJ（降温，如 DebugCool/HeatBulb）必须照常处理。
        // 2026-08-10 修复：此前误加 kj <= 0.0 判断，把全部降温消息整条丢弃。
        if mass <= 0.001 || max_temp <= 0.0 {
            continue;
        }
        let temp = updated.temperature.get(cell);
        let cap = temp.max(max_temp);
        let mut new_temp = temp + kj / (mass * etd.specific_heat_capacity);
        new_temp = new_temp.min(cap);
        if new_temp <= 0.0 || new_temp > 10000.0 {
            tracing::warn!(
                cell,
                kj,
                max_temp,
                mass,
                temp,
                new_temp,
                "ProcessCellEnergyModifications: invalid temperature"
            );
            new_temp = new_temp.clamp(1.0, 10000.0);
        }
        if new_temp > 0.0 {
            let updated = unsafe { &mut *sim_data.updated_cells.ptr };
            updated.temperature.set(cell, new_temp);
            crate::c2_physics::temperature::do_state_transition(sim_data, cell, &etd);
        }
    }
    frame.cell_energy_modifications.clear_keep_capacity();
}

/// 被移除质量汇总（原版 ConsumedMassInfo 语义）。
#[derive(Default, Clone, Copy)]
struct RemovedMass {
    disease_idx: u8,
    mass: f32,
    temperature: f32,
    disease_count: i32,
}

/// 从单格移除质量（原版 do_remove，SimDLL_Source.c L136509）。
/// 只移除 element == elem 的格；扣 min(mass_to_remove, cell_mass)，
/// 质量加权温度混合进 removed、病菌按比例；格质量 <= 0 → 清格 + substance 事件。
fn remove_mass_from_cell(
    sim_data: &mut SimData,
    cell: usize,
    elem: u16,
    mass_to_remove: &mut f32,
    removed: &mut RemovedMass,
) {
    if sim_data.updated_cells.ptr.is_null() {
        return;
    }
    let updated = unsafe { &mut *sim_data.updated_cells.ptr };
    if cell >= updated.element_idx.len() || updated.element_idx.get(cell) != elem {
        return;
    }
    let cell_mass = updated.mass.get(cell);
    let remove = if *mass_to_remove < cell_mass { *mass_to_remove } else { cell_mass };
    if remove <= 0.0 {
        return;
    }
    let cell_disease = updated.disease_count.get(cell);
    let disease_removed = ((cell_disease as f32) * (remove / cell_mass)) as i32;
    let cell_temp = updated.temperature.get(cell);
    // 质量加权温度混合（原版 L136345-136370，clamp 到两温度之间）
    let new_mass = removed.mass + remove;
    let mixed = if new_mass > 0.0 {
        (removed.mass * removed.temperature + cell_temp * remove) / new_mass
    } else {
        cell_temp
    };
    let lo = removed.temperature.min(cell_temp);
    let hi = removed.temperature.max(cell_temp);
    removed.temperature = mixed.clamp(lo, hi);
    removed.mass = new_mass;
    // 病菌汇总（简化：同病菌累加、异病菌替换，与 add_disease_to_cell 一致）
    let cell_d_idx = updated.disease_idx.get(cell);
    if cell_d_idx != 0xFF {
        if removed.disease_idx == 0xFF || removed.disease_idx == cell_d_idx {
            removed.disease_idx = cell_d_idx;
            removed.disease_count += disease_removed;
        } else {
            removed.disease_idx = cell_d_idx;
            removed.disease_count = disease_removed;
        }
    }
    // 源格扣减
    let remaining_mass = cell_mass - remove;
    let remaining_disease = cell_disease - disease_removed;
    updated.mass.set(cell, remaining_mass);
    updated.disease_count.set(cell, remaining_disease);
    if remaining_disease <= 0 {
        updated.disease_idx.set(cell, 0xFF);
        updated.disease_infestation_tick_count.set(cell, 0);
        updated.disease_growth_accumulated_error.set(cell, 0.0);
    }
    *mass_to_remove -= remove;
    if remaining_mass <= 1.1754944e-38 {
        // ClearCell + ChangeSubstance（原版 L136370-136377）
        updated.element_idx.set(cell, sim_data.vacuum_element_idx);
        updated.mass.set(cell, 0.0);
        updated.temperature.set(cell, 0.0);
        updated.disease_idx.set(cell, 0xFF);
        updated.disease_count.set(cell, 0);
        updated.disease_infestation_tick_count.set(cell, 0);
        updated.disease_growth_accumulated_error.set(cell, 0.0);
        change_substance(sim_data, cell);
    }
}

/// RectangularRemoved（原版 L136290-136430）：矩形区域 [x, x+width) × [y, y+height)
/// 逐格移除 elem，直到 mass_to_remove 耗尽。
fn rectangular_removed(
    sim_data: &mut SimData,
    mass_to_remove: &mut f32,
    elem: u16,
    removed: &mut RemovedMass,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) {
    let w = sim_data.width as usize;
    for row in y..y + height {
        for col in x..x + width {
            if *mass_to_remove <= 0.0 {
                return;
            }
            remove_mass_from_cell(sim_data, row * w + col, elem, mass_to_remove, removed);
        }
    }
}

/// FloodRemoved（原版 L136290 + Flood<> L135332）：从 (x,y) 起 BFS，深度 < radius。
/// 过滤器按被移除元素形态：移除固体 → 只穿越固体；移除液体/气体 → 跳过固体。
fn flood_removed(
    sim_data: &mut SimData,
    mass_to_remove: &mut f32,
    elem: u16,
    removed: &mut RemovedMass,
    x: usize,
    y: usize,
    radius: usize,
) {
    let elem_is_solid = match elements_table::get_element_post_process_data(elem) {
        Some(p) => (p.state & 3) == 3,
        None => return,
    };
    let skip_solid = !elem_is_solid;
    let skip_liquid = elem_is_solid;
    let skip_gas = elem_is_solid;
    let w = sim_data.width as usize;
    let h = sim_data.height as usize;
    let total = w * h;
    // visited 用线程本地世代戳池复用（省每次 218KB 分配+清零，行为等价）。
    crate::c_simulation::bfs_scratch::with_visited_scratch(total, |visited, generation| {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((x, y, 0usize));
        if x < w && y < h {
            visited[y * w + x] = generation;
        }
        while let Some((cx, cy, depth)) = queue.pop_front() {
            if *mass_to_remove <= 0.0 {
                break;
            }
            if depth >= radius {
                continue;
            }
            // 原版 Flood（L135332）出队门控：跳过 x==0 / y==0 边界格才 do_remove。
            // 2026-09-05 补齐（与 find_reachable_state 同一处原版语义）。
            if cx == 0 || cy == 0 || cx >= w || cy >= h {
                continue;
            }
            let cell = cy * w + cx;
            let state = {
                let updated = unsafe { &*sim_data.updated_cells.ptr };
                if cell >= updated.element_idx.len() {
                    continue;
                }
                elements_table::get_element_post_process_data(updated.element_idx.get(cell))
                    .map(|p| p.state & 3)
                    .unwrap_or(0xFF)
            };
            if (skip_solid && state == 3) || (skip_liquid && state == 2) || (skip_gas && state == 1)
            {
                continue;
            }
            remove_mass_from_cell(sim_data, cell, elem, mass_to_remove, removed);
            // BFS 邻居顺序 = 原版 Flood<>（L135332）入队序：上 → 左 → 右 → 下。
            // 2026-09-05 修复：此前误用 [右,左,下,上]（与原版不一致）。
            for (nx, ny) in [
                (cx, cy.wrapping_sub(1)), // 上
                (cx.wrapping_sub(1), cy), // 左
                (cx + 1, cy),             // 右
                (cx, cy + 1),             // 下
            ] {
                if nx < w && ny < h {
                    let n = ny * w + nx;
                    if visited[n] != generation {
                        visited[n] = generation;
                        queue.push_back((nx, ny, depth + 1));
                    }
                }
            }
        }
    });
}

/// ProcessMassConsumption（原版 SimDLL_Source.c L117050-117185）。
/// MassConsumption 消息：radius 模式 → FloodRemoved；height>0 → RectangularRemoved；
/// callback_idx != -1 → 推送 MassConsumedCallback。
/// 2026-08-02 实现：此前为桩（且消息结构体 12B 错位），大块液体移除/建筑拆除排水缺失。
fn process_mass_consumption(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    if sim_data.updated_cells.ptr.is_null() {
        frame.mass_consumption_messages.clear_keep_capacity();
        return;
    }
    let game_w = sim_data.width - 2;
    let width = sim_data.width as usize;
    let messages = frame.mass_consumption_messages.as_slice();
    let count = messages.len();
    for i in 0..count {
        let msg = messages[i];
        let internal = ((msg.game_cell % game_w + 1) as i32)
            + ((msg.game_cell / game_w + 1) as i32) * sim_data.width;
        if internal < 0 {
            continue;
        }
        let internal = internal as usize;
        let x = internal % width;
        let y = internal / width;
        let mut mass_to_remove = msg.mass;
        let mut removed = RemovedMass {
            disease_idx: 0xFF,
            mass: 0.0,
            temperature: 0.0,
            disease_count: 0,
        };
        if msg.height == 0 {
            flood_removed(sim_data, &mut mass_to_remove, msg.element_idx, &mut removed, x, y, msg.radius as usize);
        } else {
            rectangular_removed(
                sim_data,
                &mut mass_to_remove,
                msg.element_idx,
                &mut removed,
                x,
                y,
                msg.radius as usize,
                msg.height as usize,
            );
        }
        if msg.callback_idx != -1 {
            let events = unsafe { &mut *sim_data.sim_events.ptr };
            events.mass_consumed_callbacks.push(
                crate::a_framework::game_data::MassConsumedCallback {
                    callback_idx: msg.callback_idx,
                    elem_idx: msg.element_idx,
                    disease_idx: removed.disease_idx,
                    pad: 0,
                    mass: removed.mass,
                    temperature: removed.temperature,
                    disease_count: removed.disease_count,
                },
            );
        }
    }
    frame.mass_consumption_messages.clear_keep_capacity();
}

/// ElementConsumer Register（原版 L140670）：把消费者加入注册表，返回句柄（槽位索引）。
/// cell_x=0xFFFF 为墓碑槽，可复用。
fn element_consumer_register(
    sim: &mut SimData,
    msg: &crate::a_framework::sim_events::AddElementConsumerMsg,
) -> i32 {
    let game_w = sim.width - 2;
    if game_w <= 0 {
        return -1;
    }
    let width = sim.width as usize;
    let internal = ((msg.cell_idx % game_w + 1) as i32)
        + ((msg.cell_idx / game_w + 1) as i32) * sim.width;
    if internal < 0 {
        return -1;
    }
    let internal = internal as usize;
    let entry = crate::a_framework::sim_data::ElementConsumerEntry {
        consumption_rate: 0.0, // SetElementConsumerData 会设置真实速率
        max_depth: msg.radius,
        configuration: msg.configuration,
        element_idx: msg.element_idx,
        offset_idx: 0,
        field_0x9: 0,
        cell_y: (internal / width) as u16,
        cell_x: (internal % width) as u16,
        field_0xe: 0,
    };
    let registry = &mut sim.element_consumer.registry;
    let mut handle = -1i32;
    let n = registry.len();
    for i in 0..n {
        let e = registry.get(i);
        if e.cell_x == 0xFFFF {
            registry.set(i, entry);
            handle = i as i32;
            break;
        }
    }
    if handle < 0 {
        registry.push(entry);
        handle = (registry.len() - 1) as i32;
    }
    handle
}

/// ElementConsumer Unregister（原版 L140760）：标记墓碑。
fn element_consumer_unregister(sim: &mut SimData, handle: i32) {
    if handle < 0 {
        return;
    }
    let registry = &mut sim.element_consumer.registry;
    let h = handle as usize;
    if h < registry.len() {
        let mut e = registry.get(h);
        if e.cell_x != 0xFFFF {
            e.cell_x = 0xFFFF;
            registry.set(h, e);
        }
    }
}

/// ElementConsumer Modify（原版 L140620）：更新速率与格子。
fn element_consumer_modify(sim: &mut SimData, msg: &crate::a_framework::sim_events::SetElementConsumerDataMsg) {
    if msg.handle < 0 {
        return;
    }
    let game_w = sim.width - 2;
    if game_w <= 0 {
        return;
    }
    let width = sim.width as usize;
    let internal = ((msg.cell % game_w + 1) as i32) + ((msg.cell / game_w + 1) as i32) * sim.width;
    if internal < 0 {
        return;
    }
    let internal = internal as usize;
    let registry = &mut sim.element_consumer.registry;
    let h = msg.handle as usize;
    if h < registry.len() {
        let mut e = registry.get(h);
        if e.cell_x != 0xFFFF {
            e.consumption_rate = msg.consumption_rate;
            e.cell_x = (internal % width) as u16;
            e.cell_y = (internal / width) as u16;
            registry.set(h, e);
        }
    }
}

/// 帧处理：ElementConsumer 消息（原版 ProcessFrame L116710-116790）。
/// Register/Modify/Unregister + component_state_changed 回传句柄给 C#。
fn process_element_consumer_messages(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    if sim_data.sim_events.ptr.is_null() {
        frame.element_consumer_messages.adds.clear_keep_capacity();
        frame.element_consumer_messages.modifies.clear_keep_capacity();
        frame.element_consumer_messages.removes.clear_keep_capacity();
        return;
    }
    // adds（12B）
    {
        let adds = frame.element_consumer_messages.adds.as_slice();
        let n = adds.len() / 12;
        for i in 0..n {
            let msg = unsafe {
                std::ptr::read_unaligned(
                    adds.as_ptr().add(i * 12)
                        as *const crate::a_framework::sim_events::AddElementConsumerMsg,
                )
            };
            let handle = element_consumer_register(sim_data, &msg);
            tracing::info!(
                cell = msg.cell_idx,
                callback = msg.callback_idx,
                config = msg.configuration,
                elem = msg.element_idx,
                radius = msg.radius,
                handle,
                "element_consumer: registered"
            );
            if msg.callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events
                    .component_state_changed_messages
                    .push(crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx: msg.callback_idx,
                        sim_handle: handle,
                    });
                tracing::info!(
                    callback = msg.callback_idx,
                    handle,
                    "element_consumer: handle callback pushed"
                );
            }
        }
        frame.element_consumer_messages.adds.clear_keep_capacity();
    }
    // modifies（12B）
    {
        let mods = frame.element_consumer_messages.modifies.as_slice();
        let n = mods.len() / 12;
        for i in 0..n {
            let msg = unsafe {
                std::ptr::read_unaligned(
                    mods.as_ptr().add(i * 12)
                        as *const crate::a_framework::sim_events::SetElementConsumerDataMsg,
                )
            };
            element_consumer_modify(sim_data, &msg);
            tracing::info!(
                handle = msg.handle,
                cell = msg.cell,
                rate = msg.consumption_rate,
                "element_consumer: modified"
            );
        }
        frame.element_consumer_messages.modifies.clear_keep_capacity();
    }
    // removes（8B）
    {
        let removes = frame.element_consumer_messages.removes.as_slice();
        let n = removes.len() / 8;
        for i in 0..n {
            let msg = unsafe {
                std::ptr::read_unaligned(
                    removes.as_ptr().add(i * 8)
                        as *const crate::a_framework::sim_events::RemoveElementConsumerMsg,
                )
            };
            element_consumer_unregister(sim_data, msg.handle);
            if msg.callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events
                    .component_state_changed_messages
                    .push(crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx: msg.callback_idx,
                        sim_handle: -1,
                    });
            }
        }
        frame.element_consumer_messages.removes.clear_keep_capacity();
    }
}

/// Frame handling for BuildingHeatExchange messages (adds 44B / modifies 44B / removes 8B).
/// - adds: allocate building-temperature handle, push component_state_changed callback (C# pipe join)
/// - modifies: msg.callback_idx carries the sim handle (C# SimMessages.ModifyBuildingHeatExchange)
/// - removes: free handle
fn process_building_heat_exchange_messages(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    if sim_data.sim_events.ptr.is_null() {
        frame.building_heat_exchange_messages.adds.clear_keep_capacity();
        frame
            .building_heat_exchange_messages
            .modifies
            .clear_keep_capacity();
        frame
            .building_heat_exchange_messages
            .removes
            .clear_keep_capacity();
        return;
    }

    {
        let adds = frame.building_heat_exchange_messages.adds.as_slice();
        let n = adds.len() / 44;
        for i in 0..n {
            let msg = unsafe {
                std::ptr::read_unaligned(
                    adds.as_ptr().add(i * 44)
                        as *const crate::a_framework::sim_events::AddBuildingHeatExchangeMsg,
                )
            };
            let handle = crate::c_simulation::building_temperature::add_building(&msg);
            tracing::info!(
                callback = msg.callback_idx,
                elem = msg.elem_idx,
                mass = msg.mass,
                temp = msg.temperature,
                handle,
                "building_temperature: added"
            );
            if msg.callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events
                    .component_state_changed_messages
                    .push(crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx: msg.callback_idx,
                        sim_handle: handle,
                    });
                tracing::info!(
                    callback = msg.callback_idx,
                    handle,
                    "building_temperature: handle callback pushed"
                );
            }
        }
        frame.building_heat_exchange_messages.adds.clear_keep_capacity();
    }

    {
        let mods = frame.building_heat_exchange_messages.modifies.as_slice();
        let n = mods.len() / 44;
        for i in 0..n {
            let msg = unsafe {
                std::ptr::read_unaligned(
                    mods.as_ptr().add(i * 44)
                        as *const crate::a_framework::sim_events::AddBuildingHeatExchangeMsg,
                )
            };
            let ok = if sim_data.sim_events.ptr.is_null() {
                crate::c_simulation::building_temperature::modify_building(
                    msg.callback_idx,
                    &msg,
                )
            } else {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                crate::c_simulation::building_temperature::modify_building_with_events(
                    msg.callback_idx,
                    &msg,
                    events,
                )
            };
            tracing::trace!(
                handle = msg.callback_idx,
                ok,
                temp = msg.temperature,
                "building_temperature: modified"
            );
        }
        frame
            .building_heat_exchange_messages
            .modifies
            .clear_keep_capacity();
    }

    {
        // ModifyBuildingEnergy 消费（原版 L115484-115540：modifies 之后、removes 之前）。
        // 16B 消息 {handle i32, deltaKJ f32, minTemperature f32, maxTemperature f32}。
        let bytes = frame.modify_building_energy_messages.as_slice();
        let n = bytes.len() / 16;
        for i in 0..n {
            let off = i * 16;
            let handle = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            let delta_kj = f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            let min_temp = f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
            let max_temp = f32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap());
            let ok = crate::c_simulation::building_temperature::modify_building_energy(
                handle, delta_kj, min_temp, max_temp,
            );
            tracing::debug!(handle, ok, "modify_building_energy consumed");
        }
        frame.modify_building_energy_messages.clear_keep_capacity();
    }

    {
        let removes = frame.building_heat_exchange_messages.removes.as_slice();
        let n = removes.len() / 8;
        for i in 0..n {
            let msg = unsafe {
                std::ptr::read_unaligned(
                    removes.as_ptr().add(i * 8)
                        as *const crate::a_framework::sim_events::RemoveBuildingHeatExchangeMsg,
                )
            };
            let ok = crate::c_simulation::building_temperature::remove_building(msg.handle);
            tracing::info!(handle = msg.handle, ok, "building_temperature: removed");
            if msg.callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events
                    .component_state_changed_messages
                    .push(crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx: msg.callback_idx,
                        // 原版 remove 回调 sim_handle = -1（SimDLL_Source.c L116529-116537）
                        sim_handle: -1,
                    });
            }
        }
        frame
            .building_heat_exchange_messages
            .removes
            .clear_keep_capacity();
    }
}

/// Frame handling for BuildingToBuildingHeatExchange messages（原版 ProcessFrame L116540-116650）：
/// adds(Register 8B) → modifies(RemoveInContact 8B) → addInContact(Add 12B) → removes(Unregister 8B)。
/// Register/Remove 均带 callbackIdx：≠-1 时推 componentStateChanged（注册失败推 sim_handle=-1，
/// remove 恒推 -1，与原版一致）。
fn process_building_to_building_messages(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    if sim_data.sim_events.ptr.is_null() {
        frame
            .building_to_building_heat_exchange_messages
            .adds
            .clear_keep_capacity();
        frame
            .building_to_building_heat_exchange_messages
            .modifies
            .clear_keep_capacity();
        frame
            .building_to_building_heat_exchange_messages
            .removes
            .clear_keep_capacity();
        frame.add_building_in_contact_messages.clear_keep_capacity();
        return;
    }
    // adds：8B {callbackIdx, heatExchange_handle} → Register
    {
        let adds = frame
            .building_to_building_heat_exchange_messages
            .adds
            .as_slice();
        let n = adds.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let callback_idx = i32::from_le_bytes(adds[off..off + 4].try_into().unwrap());
            let heat_exchange_handle =
                i32::from_le_bytes(adds[off + 4..off + 8].try_into().unwrap());
            let handle =
                crate::c_simulation::building_to_building::register_building_to_building(
                    callback_idx,
                    heat_exchange_handle,
                );
            tracing::info!(
                callback = callback_idx,
                heat_exchange_handle,
                handle,
                "building_to_building: registered"
            );
            if callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events
                    .component_state_changed_messages
                    .push(crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx,
                        sim_handle: handle, // 注册失败时 handle=-1，与原版 Register 一致
                    });
            }
        }
        frame
            .building_to_building_heat_exchange_messages
            .adds
            .clear_keep_capacity();
    }
    // modifies：8B {self_handle, buildingInContact} → RemoveInContact
    {
        let mods = frame
            .building_to_building_heat_exchange_messages
            .modifies
            .as_slice();
        let n = mods.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let self_handle = i32::from_le_bytes(mods[off..off + 4].try_into().unwrap());
            let building_in_contact =
                i32::from_le_bytes(mods[off + 4..off + 8].try_into().unwrap());
            crate::c_simulation::building_to_building::remove_building_in_contact(
                self_handle,
                building_in_contact,
            );
        }
        frame
            .building_to_building_heat_exchange_messages
            .modifies
            .clear_keep_capacity();
    }
    // addInContact：12B {self_handle, buildingInContact, cellsInContact} → Add
    {
        let bytes = frame.add_building_in_contact_messages.as_slice();
        let n = bytes.len() / 12;
        for i in 0..n {
            let off = i * 12;
            let self_handle = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            let building_in_contact =
                i32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            let cells_in_contact =
                i32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
            crate::c_simulation::building_to_building::add_building_to_building_contact(
                self_handle,
                building_in_contact,
                cells_in_contact,
            );
        }
        frame.add_building_in_contact_messages.clear_keep_capacity();
    }
    // removes：8B {callbackIdx, handle} → Unregister
    {
        let removes = frame
            .building_to_building_heat_exchange_messages
            .removes
            .as_slice();
        let n = removes.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let callback_idx = i32::from_le_bytes(removes[off..off + 4].try_into().unwrap());
            let handle = i32::from_le_bytes(removes[off + 4..off + 8].try_into().unwrap());
            crate::c_simulation::building_to_building::remove_building_to_building(handle);
            if callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events
                    .component_state_changed_messages
                    .push(crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx,
                        sim_handle: -1, // 原版 remove 回调 sim_handle=-1
                    });
            }
        }
        frame
            .building_to_building_heat_exchange_messages
            .removes
            .clear_keep_capacity();
    }
}

/// BFS 构建 radius 内可达格列表，从 `offset`（offset_idx 轮转）起循环扫描，
/// 返回首个 state==target 的格元素（原版 GetReachableCells + AnyInputCellHasState）。
///
/// 原版 AnyInputCellHasState（11_msvcrt_ignored.c L38623-38648）：
/// `uVar3 = (offset + i) % 列表长度` —— 消费者（水泵等）以 offset_idx 为起点
/// 轮转遍历可达格，"轮流吸取"不同元素/格子；此前 Rust 恒从 BFS 首个匹配格
/// 开始（等效 offset=0），轮换消费逻辑未生效（offset_idx 递增但未被使用）。
fn find_reachable_state(
    sim: &SimData,
    x: usize,
    y: usize,
    radius: usize,
    target: u8,
    offset: usize,
) -> Option<u16> {
    if sim.updated_cells.ptr.is_null() {
        return None;
    }
    let w = sim.width as usize;
    let h = sim.height as usize;
    let total = w * h;
    let mut reachable: Vec<usize> = Vec::new();
    // visited 用线程本地世代戳池复用（省每次 218KB 分配+清零，行为等价）。
    crate::c_simulation::bfs_scratch::with_visited_scratch(total, |visited, generation| {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((x, y, 0usize));
        if x < w && y < h {
            visited[y * w + x] = generation;
        }
        while let Some((cx, cy, depth)) = queue.pop_front() {
            if depth >= radius {
                continue;
            }
            // 原版 Flood 出队门控：跳过 x==0 / y==0 边界格（SimDLL_Source.c L135568-135571：
            // `x != 0 && x < width && y != 0 && y < height` 才处理）。2026-09-05 补齐
            // （此前 Rust 误把 0 边界格纳入可达候选，顺序与原版不一致）。
            if cx == 0 || cy == 0 || cx >= w || cy >= h {
                continue;
            }
            let cell = cy * w + cx;
            // 原版 GetReachableCells → Flood(skip_solid=true)（ElementConsumer::Update
            // L140819 传 param_8=true）：元素 state&3==3（固体）的格**不入候选、不穿越**
            // （SimDLL_Source.c L135436-135442 `!param_5 || (state&3)!=3` 才进入收集块，
            // 邻居 enqueue 也在块内 → 固体挡 BFS 扩展）。2026-09-05 补齐。
            // 只判元素相态（ElementPostProcessData.state），不读格子的 properties 位
            // （砖格的可渗透/容纳由液体/气体模块按 properties 判定，与选格无关）。
            let cell_elem = unsafe { (*sim.updated_cells.ptr).element_idx.get(cell) };
            let cell_state = elements_table::get_element_post_process_data(cell_elem)
                .map(|p| p.state & 3)
                .unwrap_or(0xFF);
            if cell_state == 3 {
                continue; // 固体：不入候选，也不扩展（挡路）
            }
            reachable.push(cell);
            // BFS 邻居顺序 = 原版 Flood<> 的入队序：上 → 左 → 右 → 下
            // （SimDLL_Source.c L135666-135678 / L135493-135501）。
            // 2026-09-05 修复：此前误用 [右,左,下,上]，导致泵抽吸顺序与原版不一致
            // （玩家实测"右侧优先于下侧"——原版应 上/左 优先）。
            for (nx, ny) in [
                (cx, cy.wrapping_sub(1)), // 上
                (cx.wrapping_sub(1), cy), // 左
                (cx + 1, cy),             // 右
                (cx, cy + 1),             // 下
            ] {
                if nx < w && ny < h {
                    let n = ny * w + nx;
                    if visited[n] != generation {
                        visited[n] = generation;
                        queue.push_back((nx, ny, depth + 1));
                    }
                }
            }
        }
    });
    let n = reachable.len();
    if n == 0 {
        return None;
    }
    let updated = unsafe { &*sim.updated_cells.ptr };
    let start = offset % n;
    for i in 0..n {
        let cell = reachable[(start + i) % n];
        if cell < updated.element_idx.len() {
            let elem = updated.element_idx.get(cell);
            let state = elements_table::get_element_post_process_data(elem)
                .map(|p| p.state & 3)
                .unwrap_or(0xFF);
            if state == target {
                return Some(elem);
            }
        }
    }
    None
}

/// 原版 ProcessFrame ElementChunk 段（L116653-116682）：adds → modifies → modifyEnergy →
/// modifyAdjuster → move → removes。
fn process_element_chunk_messages(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    use crate::c_simulation::element_chunk::*;
    let width = sim_data.width;
    // adds（32B）{gameCell, callbackIdx, mass, temperature, surfaceArea, thickness, groundTransferScale, elementIdx, pad}
    {
        let adds = frame.element_chunk_messages.adds.as_slice();
        let n = adds.len() / 32;
        for i in 0..n {
            let off = i * 32;
            let msg = AddElementChunkMsg {
                game_cell: i32::from_le_bytes(adds[off..off + 4].try_into().unwrap()),
                callback_idx: i32::from_le_bytes(adds[off + 4..off + 8].try_into().unwrap()),
                mass: f32::from_le_bytes(adds[off + 8..off + 12].try_into().unwrap()),
                temperature: f32::from_le_bytes(adds[off + 12..off + 16].try_into().unwrap()),
                surface_area: f32::from_le_bytes(adds[off + 16..off + 20].try_into().unwrap()),
                thickness: f32::from_le_bytes(adds[off + 20..off + 24].try_into().unwrap()),
                ground_transfer_scale: f32::from_le_bytes(adds[off + 24..off + 28].try_into().unwrap()),
                element_idx: u16::from_le_bytes(adds[off + 28..off + 30].try_into().unwrap()),
                pad: [adds[off + 30], adds[off + 31]],
            };
            let h = add_element_chunk(width, &msg);
            if msg.callback_idx != -1 && !sim_data.sim_events.ptr.is_null() {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events.component_state_changed_messages.push(
                    crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx: msg.callback_idx,
                        sim_handle: h,
                    },
                );
            }
        }
        frame.element_chunk_messages.adds.clear_keep_capacity();
    }
    // move（8B）{handle, gameCell}——原版 ProcessElementChunkMessages 顺序：move 在 modifies 之前（L116132-116223）
    {
        let bytes = frame.move_element_chunk_messages.as_slice();
        let n = bytes.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let handle = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            let gc = i32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            move_element_chunk(handle, gc, width);
        }
        frame.move_element_chunk_messages.clear_keep_capacity();
    }
    // modifies（12B）{handle, temperature, heatCapacity}
    {
        let mods = frame.element_chunk_messages.modifies.as_slice();
        let n = mods.len() / 12;
        for i in 0..n {
            let off = i * 12;
            let handle = i32::from_le_bytes(mods[off..off + 4].try_into().unwrap());
            let t = f32::from_le_bytes(mods[off + 4..off + 8].try_into().unwrap());
            let hc = f32::from_le_bytes(mods[off + 8..off + 12].try_into().unwrap());
            modify_element_chunk(handle, t, hc);
        }
        frame.element_chunk_messages.modifies.clear_keep_capacity();
    }
    // modifyEnergy（8B）{handle, deltaKJ}
    {
        let bytes = frame.modify_element_chunk_energy_messages.as_slice();
        let n = bytes.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let handle = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            let dk = f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            modify_element_chunk_energy(handle, dk);
        }
        frame.modify_element_chunk_energy_messages.clear_keep_capacity();
    }
    // modifyAdjuster（16B）{handle, temperature, heatCapacity, thermalConductivity}
    {
        let bytes = frame.modify_element_chunk_adjuster_messages.as_slice();
        let n = bytes.len() / 16;
        for i in 0..n {
            let off = i * 16;
            let handle = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            let t = f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            let hc = f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
            let tc = f32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap());
            modify_element_chunk_adjuster(handle, t, hc, tc);
        }
        frame.modify_element_chunk_adjuster_messages.clear_keep_capacity();
    }
    // removes（8B）{handle, callbackIdx}
    {
        let removes = frame.element_chunk_messages.removes.as_slice();
        let n = removes.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let handle = i32::from_le_bytes(removes[off..off + 4].try_into().unwrap());
            let cb = i32::from_le_bytes(removes[off + 4..off + 8].try_into().unwrap());
            remove_element_chunk(handle);
            if cb != -1 && !sim_data.sim_events.ptr.is_null() {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events.component_state_changed_messages.push(
                    crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx: cb,
                        sim_handle: -1,
                    },
                );
            }
        }
        frame.element_chunk_messages.removes.clear_keep_capacity();
    }
    // modifyBackwallData（16B）：{gameCell(i32), elemIdx(u16)+pad(u16), mass(f32), temperature(f32)}
    // 原版 ProcessElementChunkMessages 尾部 L871-914：
    //   uVar20 = (gc/(width-2)+1)*width + gc%(width-2) + 1（game→sim 内部格）
    //   写 backwall.elementIdx/mass/temperature；元素变化推 BackwallElementChangedInfo(gameCell)。
    // 2026-08-04 实现：此前 C2 桩只清空 → C# 设置的背墙（含太空格真空背墙）从未生效，
    // 太空删除判定 backwall.elementIdx==vacuum 恒失败。
    {
        let bytes = frame.modify_backwall_data_messages.as_slice();
        let n = bytes.len() / 16;
        let game_w = sim_data.width - 2;
        if n > 0
            && game_w > 0
            && !sim_data.backwall.ptr.is_null()
            && !sim_data.sim_events.ptr.is_null()
        {
            let width = sim_data.width as usize;
            let backwall = unsafe { &mut *sim_data.backwall.ptr };
            let events = unsafe { &mut *sim_data.sim_events.ptr };
            for i in 0..n {
                let off = i * 16;
                let game_cell = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
                let elem_idx = u16::from_le_bytes(bytes[off + 4..off + 6].try_into().unwrap());
                let mass = f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
                let temp = f32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap());
                // 原版 L879：sim_cell = (gc/gw+1)*width + gc%gw + 1
                let sim_cell =
                    ((game_cell / game_w + 1) * (width as i32) + game_cell % game_w + 1) as usize;
                let old = backwall.element_idx.get(sim_cell);
                backwall.element_idx.set(sim_cell, elem_idx);
                backwall.mass.set(sim_cell, mass);
                backwall.temperature.set(sim_cell, temp);
                if old != elem_idx {
                    events.backwall_element_changed_info.push(
                        crate::a_framework::game_data::BackwallElementChangedInfo {
                            game_cell: game_cell as u32,
                        },
                    );
                }
            }
        }
        frame.modify_backwall_data_messages.clear_keep_capacity();
    }
}

/// ElementEmitter 消息：adds(16B)→modifies(32B)→removes(8B) + componentStateChanged 回调
/// （原版 08_sim_frame_manager.c L1628-1665）。
fn process_element_emitter_messages(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    // adds（16B）{maxPressure, callbackIdx, onBlockedCB, onUnblockedCB}
    {
        let adds = frame.element_emitter_messages.adds.as_slice();
        let n = adds.len() / 16;
        for i in 0..n {
            let off = i * 16;
            let msg = crate::c_simulation::element_emitter::AddElementEmitterMsg {
                max_pressure: f32::from_le_bytes(adds[off..off + 4].try_into().unwrap()),
                callback_idx: i32::from_le_bytes(adds[off + 4..off + 8].try_into().unwrap()),
                on_blocked_cb: i32::from_le_bytes(adds[off + 8..off + 12].try_into().unwrap()),
                on_unblocked_cb: i32::from_le_bytes(adds[off + 12..off + 16].try_into().unwrap()),
            };
            let h = crate::c_simulation::element_emitter::add_element_emitter(sim_data, &msg);
            if msg.callback_idx != -1 && !sim_data.sim_events.ptr.is_null() {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events.component_state_changed_messages.push(
                    crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx: msg.callback_idx,
                        sim_handle: h,
                    },
                );
            }
        }
        frame.element_emitter_messages.adds.clear_keep_capacity();
    }
    // modifies（32B）{handle, cellIdx, emitInterval, emitMass, emitTemperature, maxPressure,
    //                diseaseCount, elementIdx, maxDepth, diseaseIdx}
    {
        let mods = frame.element_emitter_messages.modifies.as_slice();
        let n = mods.len() / 32;
        for i in 0..n {
            let off = i * 32;
            let msg = crate::c_simulation::element_emitter::ModifyElementEmitterMsg {
                handle: i32::from_le_bytes(mods[off..off + 4].try_into().unwrap()),
                cell_idx: i32::from_le_bytes(mods[off + 4..off + 8].try_into().unwrap()),
                emit_interval: f32::from_le_bytes(mods[off + 8..off + 12].try_into().unwrap()),
                emit_mass: f32::from_le_bytes(mods[off + 12..off + 16].try_into().unwrap()),
                emit_temperature: f32::from_le_bytes(mods[off + 16..off + 20].try_into().unwrap()),
                max_pressure: f32::from_le_bytes(mods[off + 20..off + 24].try_into().unwrap()),
                disease_count: i32::from_le_bytes(mods[off + 24..off + 28].try_into().unwrap()),
                element_idx: u16::from_le_bytes(mods[off + 28..off + 30].try_into().unwrap()),
                max_depth: mods[off + 30],
                disease_idx: mods[off + 31],
            };
            crate::c_simulation::element_emitter::modify_element_emitter(sim_data, &msg);
        }
        frame.element_emitter_messages.modifies.clear_keep_capacity();
    }
    // removes（8B）{handle, callbackIdx}
    {
        let removes = frame.element_emitter_messages.removes.as_slice();
        let n = removes.len() / 8;
        for i in 0..n {
            let off = i * 8;
            let handle = i32::from_le_bytes(removes[off..off + 4].try_into().unwrap());
            let cb = i32::from_le_bytes(removes[off + 4..off + 8].try_into().unwrap());
            crate::c_simulation::element_emitter::remove_element_emitter(handle);
            if cb != -1 && !sim_data.sim_events.ptr.is_null() {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events.component_state_changed_messages.push(
                    crate::a_framework::game_data::ComponentStateChangedMessage {
                        callback_idx: cb,
                        sim_handle: -1,
                    },
                );
            }
        }
        frame.element_emitter_messages.removes.clear_keep_capacity();
    }
}

/// ElementConsumer::Update（原版 L140776）：逐帧对每个注册消费者 BFS 消费，
/// 结果填入 elementConsumer.consumedMassInfo（供 C# 泵/涡轮取用）。
pub(crate) fn process_element_consumers(
    sim: &mut SimData,
    bounds: crate::d1_activity::RegionBounds,
) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let registry_len = sim.element_consumer.registry.len();
    if registry_len == 0 {
        return;
    }
    let dt = 0.2f32; // 子步时长（与 sim_thread 200ms 子步一致）
    let snapshot: Vec<crate::a_framework::sim_data::ElementConsumerEntry> =
        sim.element_consumer.registry.as_slice().to_vec();
    for (i, e) in snapshot.iter().enumerate() {
        if e.cell_x == 0xFFFF {
            continue;
        }
        let x = e.cell_x as usize;
        let y = e.cell_y as usize;
        // 原版 ElementConsumer::Update L140806-140808：仅处理落在 region 矩形内的
        // 消费者（★4）。max 为**包含**（`<= region.max`，严格复刻组件字面语义）。
        if e.cell_x < bounds.min_x as u16
            || e.cell_y < bounds.min_y as u16
            || e.cell_x > bounds.max_x as u16
            || e.cell_y > bounds.max_y as u16
        {
            continue;
        }
        let radius = e.max_depth as usize;
        let elem = match e.configuration {
            0 => Some(e.element_idx),
            // offset_idx 轮转：原版 AnyInputCellHasState 从 offset 起循环扫描可达格
            // （L38627 `(offset + i) % 列表长度`），实现水泵"轮流吸取"。
            1 => find_reachable_state(sim, x, y, radius, 2, e.offset_idx as usize),
            2 => find_reachable_state(sim, x, y, radius, 1, e.offset_idx as usize),
            _ => None,
        };
        if let Some(elem) = elem {
            let mut amount = dt * e.consumption_rate;
            let mut removed = RemovedMass {
                disease_idx: 0xFF,
                ..Default::default()
            };
            flood_removed(sim, &mut amount, elem, &mut removed, x, y, radius);
            if removed.mass > 0.0 {
                sim.element_consumer
                    .consumed_mass_info
                    .push(crate::a_framework::game_data::ConsumedMassInfo {
                        sim_handle: crate::a_framework::game_data::Handle {
                            value: i as i32,
                        },
                        removed_elem_idx: elem,
                        disease_idx: removed.disease_idx,
                        pad: 0,
                        mass: removed.mass,
                        temperature: removed.temperature,
                        disease_count: removed.disease_count,
                    });
                tracing::info!(
                    handle = i,
                    elem,
                    mass = removed.mass,
                    "element_consumer: consumed"
                );
            }
            // offset_idx 轮转（原版 L140922）
            let registry = &mut sim.element_consumer.registry;
            if i < registry.len() {
                let mut ent = registry.get(i);
                ent.offset_idx = ent.offset_idx.wrapping_add(1);
                registry.set(i, ent);
            }
        }
    }
}

fn process_mass_emission(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    if sim_data.updated_cells.ptr.is_null() {
        frame.mass_emission_messages.clear_keep_capacity();
        return;
    }
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.mass_emission_messages.clear_keep_capacity();
        return;
    }
    let messages = frame.mass_emission_messages.as_slice();
    let count = messages.len();
    for i in 0..count {
        let msg = messages[i];
        let internal = ((msg.game_cell % game_w + 1) as i32)
            + ((msg.game_cell / game_w + 1) as i32) * sim_data.width;
        if internal < 0 {
            continue;
        }
        let cell = internal as usize;
        if cell >= unsafe { (*sim_data.updated_cells.ptr).element_idx.len() } {
            continue;
        }
        let callback_idx = msg.callback_idx_or_temp.to_bits() as i32;
        let cur_elem = unsafe { (*sim_data.updated_cells.ptr).element_idx.get(cell) };
        let same_or_vacuum =
            cur_elem == msg.element_idx || cur_elem == sim_data.vacuum_element_idx;
        if !same_or_vacuum {
            // 原版 SimDLL_Source.c L117328-117342：目标格为异元素 → 按 msg 元素形态
            // DisplaceGas / DisplaceLiquid 挤压腾位；失败 → 不加质量。
            // 2026-08-07 补齐：此前无条件加质量并直接改元素（重复计入 + 丢失原元素）。
            let state = crate::b_elements::elements_table::get_element_post_process_data(
                msg.element_idx,
            )
            .map(|p| p.state & 3)
            .unwrap_or(0xFF);
            let displaced = if state == 1 {
                crate::c2_physics::liquid_flow::displace_gas(sim_data, cell, cur_elem)
            } else if state == 2 {
                crate::c2_physics::liquid_flow::displace_liquid(sim_data, cell, cur_elem)
            } else {
                false
            };
            if !displaced {
                // 原版 L117343-117370：位移失败 → 失败回调（emitted=0、全零、elem 0xffff）
                if callback_idx != -1 {
                    let events = unsafe { &mut *sim_data.sim_events.ptr };
                    events
                        .mass_emitted_callbacks
                        .push(crate::a_framework::game_data::MassEmittedCallback {
                            callback_idx,
                            elem_idx: 0xFFFF,
                            emitted: 0,
                            disease_idx: 0xFF,
                            mass: 0.0,
                            temperature: 0.0,
                            disease_count: 0,
                        });
                }
                continue;
            }
        }
        // 加质量路径（原版 LAB_180020a38，L117223-117298）
        let updated = unsafe { &mut *sim_data.updated_cells.ptr };
        let cur_mass = updated.mass.get(cell);
        let new_mass = cur_mass + msg.mass;
        if new_mass > 0.0 {
            let cur_temp = updated.temperature.get(cell);
            let avg = (cur_mass * cur_temp + msg.temperature * msg.mass) / new_mass;
            let lo = cur_temp.min(msg.temperature);
            let hi = cur_temp.max(msg.temperature);
            updated.temperature.set(cell, avg.clamp(lo, hi));
            updated.mass.set(cell, new_mass);
        }
        if msg.disease_idx != 0xFF {
            // AddDiseaseToCell 简化（同病菌累加/异病菌替换，与原库病菌简化一致）
            let cur_d_idx = updated.disease_idx.get(cell);
            if cur_d_idx == msg.disease_idx {
                updated.disease_count.set(
                    cell,
                    updated.disease_count.get(cell) + msg.disease_count,
                );
            } else {
                updated.disease_idx.set(cell, msg.disease_idx);
                updated.disease_count.set(cell, msg.disease_count);
            }
        }
        if cur_elem != msg.element_idx {
            updated.element_idx.set(cell, msg.element_idx);
            change_substance(sim_data, cell);
        }
        if callback_idx != -1 {
            let events = unsafe { &mut *sim_data.sim_events.ptr };
            events
                .mass_emitted_callbacks
                .push(crate::a_framework::game_data::MassEmittedCallback {
                    callback_idx,
                    elem_idx: msg.element_idx,
                    emitted: 1,
                    disease_idx: msg.disease_idx,
                    mass: msg.mass,
                    temperature: msg.temperature,
                    disease_count: msg.disease_count,
                });
        }
    }
    frame.mass_emission_messages.clear_keep_capacity();
}

fn process_consume_disease(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    // 阶段 3（2026-08-05）：ConsumeDisease 16B/条 → 百分比消费 + DiseaseConsumedCallback
    crate::c_simulation::disease_component::process_consume_disease(frame, _sim_data);
}

fn process_cell_disease_modifications(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    // 阶段 3（2026-08-05）：CellDiseaseModification 12B/条 → 直加/AddDiseaseToCell
    crate::c_simulation::disease_component::process_cell_disease_modifications(frame, _sim_data);
}

/// 步骤 9：ProcessRadiationChanges（原版 06_process_messages.c L370-414）。
/// 1) radiation_params_modifications（8B/条：type, value）→ 参数映射；
/// 2) cell_radiation_modifications（12B/条：gameCell, radiationDelta, callbackIdx）→ 格子辐射 + 消费回调。
fn process_frame_radiation(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    process_radiation_params(sim_data, frame.radiation_params_modifications.as_slice());
    frame.radiation_params_modifications.clear_keep_capacity();
    process_cell_radiation_changes(sim_data, frame.cell_radiation_modifications.as_slice());
    frame.cell_radiation_modifications.clear_keep_capacity();
}

/// RadiationParamsModification（8B：int type + float value）→ SimData 辐射参数。
/// type 映射（原版 06_process_messages.c L392-410）：0→linger、2→base、3→density、
/// 4→constructed、5→maxMass；type 1 与其余无映射，跳过。
pub fn process_radiation_params(sim: &mut SimData, entries: &[u8]) {
    let mut off = 0usize;
    while off + 8 <= entries.len() {
        let typ = i32::from_le_bytes([entries[off], entries[off + 1], entries[off + 2], entries[off + 3]]);
        let value = f32::from_le_bytes([entries[off + 4], entries[off + 5], entries[off + 6], entries[off + 7]]);
        match typ {
            0 => sim.radiation_linger_rate = value,
            2 => sim.radiation_base_weight = value,
            3 => sim.radiation_density_weight = value,
            4 => sim.radiation_constructed_factor = value,
            5 => sim.radiation_max_mass = value,
            _ => {} // 原版无 case，跳过
        }
        off += 8;
    }
}

/// CellRadiationModification（12B：int gameCell + float delta + int callbackIdx）。
/// gameCell → sim cell（+1 边界）；radiation += delta，≤0 清零；
/// callbackIdx != -1 → 推 RadiationConsumedCallback（12B）。
pub fn process_cell_radiation_changes(sim: &mut SimData, entries: &[u8]) {
    if sim.updated_cells.ptr.is_null() || sim.sim_events.ptr.is_null() {
        return;
    }
    let game_w = (sim.width - 2).max(1) as i32;
    let mut off = 0usize;
    while off + 12 <= entries.len() {
        let game_cell = i32::from_le_bytes([entries[off], entries[off + 1], entries[off + 2], entries[off + 3]]);
        let delta = f32::from_le_bytes([entries[off + 4], entries[off + 5], entries[off + 6], entries[off + 7]]);
        let callback_idx = i32::from_le_bytes([entries[off + 8], entries[off + 9], entries[off + 10], entries[off + 11]]);
        let sim_cell = ((game_cell % game_w + 1) + (game_cell / game_w + 1) * sim.width) as usize;
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        if sim_cell >= updated.radiation.len() {
            off += 12;
            continue;
        }
        // 原版 06_process_messages.c L341-372：fVar1=原值 → +=delta →
        // 结果 ≤0 时清零且回调 radiation 改记**原值 fVar1**（非 delta）。
        let original = updated.radiation.get(sim_cell);
        let mut r = original + delta;
        let mut cb_rad = delta;
        if r <= 0.0 {
            r = 0.0;
            cb_rad = original;
        }
        updated.radiation.set(sim_cell, r);
        if callback_idx != -1 {
            let events = unsafe { &mut *sim.sim_events.ptr };
            events.radiation_consumed_callbacks.push(crate::a_framework::game_data::RadiationConsumedCallback {
                game_cell,
                callback_idx,
                radiation: cb_rad,
            });
        }
        off += 12;
    }
}

/// 清空所有 ComponentMessages 的 adds/modifies/removes（C2 桩：不处理，只清空）。
fn process_component_messages_stubs(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    // building_heat_exchange_messages
    frame.building_heat_exchange_messages.adds.clear_keep_capacity();
    frame.building_heat_exchange_messages.modifies.clear_keep_capacity();
    frame.building_heat_exchange_messages.removes.clear_keep_capacity();

    // modify_building_energy_messages
    frame.modify_building_energy_messages.clear_keep_capacity();

    // element_chunk_messages
    frame.element_chunk_messages.adds.clear_keep_capacity();
    frame.element_chunk_messages.modifies.clear_keep_capacity();
    frame.element_chunk_messages.removes.clear_keep_capacity();

    // move_element_chunk_messages
    frame.move_element_chunk_messages.clear_keep_capacity();

    // modify_element_chunk_energy_messages
    frame.modify_element_chunk_energy_messages.clear_keep_capacity();

    // modify_element_chunk_adjuster_messages
    frame.modify_element_chunk_adjuster_messages.clear_keep_capacity();

    // modify_backwall_data_messages
    frame.modify_backwall_data_messages.clear_keep_capacity();

    // element_emitter_messages
    frame.element_emitter_messages.adds.clear_keep_capacity();
    frame.element_emitter_messages.modifies.clear_keep_capacity();
    frame.element_emitter_messages.removes.clear_keep_capacity();


    // radiation_params_modifications
    frame.radiation_params_modifications.clear_keep_capacity();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::{CellSOA, BackwallSOA, Timers};
    use crate::a_framework::sim_events::SimEvents;
    use crate::a_framework::sim_frame_manager::SimFrameInfo;
    use crate::a_framework::stl_shim::UniquePtr;
    use crate::a_framework::buffer::BinaryBufferWriter;
    use crate::b_elements::element::{Element, ElementLiquidData, ElementPostProcessData, ElementPressureData, ElementStateData};
    use crate::b_elements::elements_table::{G_ELEMENTS_TABLE, DestroyElementsTable};

    #[test]
    fn insulation_value_scale_is_255() {
        // 原版 L116370：insulation = (u8)(value × 255.0)
        assert_eq!(insulation_u8(1.0), 255, "C# 发 1.0 = 满导热 255");
        assert_eq!(insulation_u8(0.0), 0);
        assert_eq!(insulation_u8(0.01), 2); // 0.01×255 = 2.55 → 2（隔热砖）
        assert_eq!(insulation_u8(0.5), 127); // 127.5 → 127
        assert_eq!(insulation_u8(-1.0), 0); // 负值截断为 0
    }

    /// 任务 1：RadiationParamsModification（8B：type + value）→ 参数映射；
    /// type 0→linger、2→base、3→density、4→constructed、5→maxMass，其余（1 等）跳过。
    #[test]
    fn radiation_params_applies_mapping_and_skips_unknown() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        assert!((sd.radiation_linger_rate - 1.1).abs() < 1e-6);
        assert!((sd.radiation_base_weight - 0.3).abs() < 1e-6);
        assert!((sd.radiation_density_weight - 0.7).abs() < 1e-6);
        assert!((sd.radiation_constructed_factor - 0.8).abs() < 1e-6);
        assert!((sd.radiation_max_mass - 2000.0).abs() < 1e-6);

        let mut w = BinaryBufferWriter::new();
        // 8B/条：type, value
        w.write_int(0);  w.write_float(2.0);    // linger_rate
        w.write_int(2);  w.write_float(0.9);    // base_weight
        w.write_int(3);  w.write_float(0.15);   // density_weight
        w.write_int(4);  w.write_float(1.25);   // constructed_factor
        w.write_int(5);  w.write_float(3000.0); // max_mass
        w.write_int(1);  w.write_float(123.0);  // 原版无映射 → 跳过
        w.write_int(99); w.write_float(456.0);  // 越界 → 跳过
        let data = w.into_bytes();

        process_radiation_params(&mut sd, &data);
        assert!((sd.radiation_linger_rate - 2.0).abs() < 1e-6);
        assert!((sd.radiation_base_weight - 0.9).abs() < 1e-6);
        assert!((sd.radiation_density_weight - 0.15).abs() < 1e-6);
        assert!((sd.radiation_constructed_factor - 1.25).abs() < 1e-6);
        assert!((sd.radiation_max_mass - 3000.0).abs() < 1e-6);
    }

    /// 任务 2：CellRadiationModification（12B：gameCell + delta + callbackIdx）。
    /// gameCell=4 → sim cell = (4/4+1)*6 + (4%4+1) = 2*6+1 = 13；
    /// delta 累加、≤0 清零、callbackIdx≠-1 → 消费回调。
    #[test]
    fn cell_radiation_modification_applies_delta_and_callback() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            (*sd.updated_cells.ptr).radiation.set(13, 5.0);
        }
        let mut w = BinaryBufferWriter::new();
        w.write_int(4);   // gameCell
        w.write_float(2.5); // delta
        w.write_int(7);   // callbackIdx
        w.write_int(8);   // 第二条：gameCell=8 → sim=(8/4+1)*6+(8%4+1)=3*6+1=19
        w.write_float(-100.0); // delta 使结果 ≤0 → 清零
        w.write_int(-1);  // 无回调
        let data = w.into_bytes();

        process_cell_radiation_changes(&mut sd, &data);
        unsafe {
            let r13 = (*sd.updated_cells.ptr).radiation.get(13);
            assert!((r13 - 7.5).abs() < 1e-6, "delta 累加：5+2.5=7.5，got {r13}");
            let r19 = (*sd.updated_cells.ptr).radiation.get(19);
            assert_eq!(r19, 0.0, "delta 使辐射 ≤0 → 清零，got {r19}");
        }
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.radiation_consumed_callbacks.len(), 1);
        let cb = events.radiation_consumed_callbacks.as_slice();
        assert_eq!(cb[0].game_cell, 4);
        assert_eq!(cb[0].callback_idx, 7);
        assert!((cb[0].radiation - 2.5).abs() < 1e-6, "回调记录应用前的 delta");
    }

    /// 任务 2b：CellRadiationModification 结果 ≤0 清零时，回调 radiation 记
    /// **原值**（原版 06_process_messages.c：fVar11 = fVar1），而非 delta。
    #[test]
    fn cell_radiation_clamped_callback_reports_original_value() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            (*sd.updated_cells.ptr).radiation.set(13, 5.0);
        }
        let mut w = BinaryBufferWriter::new();
        w.write_int(4);      // gameCell → sim 13
        w.write_float(-10.0); // delta 使结果 -5 ≤0 → 清零
        w.write_int(7);      // callbackIdx
        let data = w.into_bytes();

        process_cell_radiation_changes(&mut sd, &data);
        unsafe {
            assert_eq!(
                (*sd.updated_cells.ptr).radiation.get(13),
                0.0,
                "结果 ≤0 → 清零"
            );
        }
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.radiation_consumed_callbacks.len(), 1);
        let cb = events.radiation_consumed_callbacks.as_slice();
        assert_eq!(cb[0].game_cell, 4);
        assert_eq!(cb[0].callback_idx, 7);
        assert!(
            (cb[0].radiation - 5.0).abs() < 1e-6,
            "钳零时回调 radiation = 原值 5.0（非 delta -10），got {}",
            cb[0].radiation
        );
    }

    #[test]
    fn cell_energy_modification_heats_cell_and_caps() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.resize(4, Default::default());
            table.temperature_data[1].state = 2; // 液体
            table.temperature_data[1].specific_heat_capacity = 4.179;
            table.temperature_data[1].low_temp = 0.0;
            table.temperature_data[1].high_temp = 1000.0; // 避免 do_state_transition 立即转换
        }
        // gameCell 0 → sim cell 7；50kg 液体 300K
        let mut sd = make_test_sim_data();
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(7, 1);
            u.mass.set(7, 50.0);
            u.temperature.set(7, 300.0);
        }
        let mut frame = SimFrameInfo::default();
        let mut buf = Vec::new();
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&500.0f32.to_le_bytes());
        buf.extend_from_slice(&3200.0f32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        for b in buf {
            frame.cell_energy_modifications.push(b);
        }
        process_cell_energy_modifications(&mut frame, &mut sd);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!(
            (u.temperature.get(7) - 302.39).abs() < 0.05,
            "ΔT=500/(50×4.179)≈2.39 → 302.39，got {}",
            u.temperature.get(7)
        );
        // cap 生效：maxT=301 → 302.39 被压到 301
        let mut frame = SimFrameInfo::default();
        let mut buf = Vec::new();
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&500.0f32.to_le_bytes());
        buf.extend_from_slice(&301.0f32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        for b in buf {
            frame.cell_energy_modifications.push(b);
        }
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.temperature.set(7, 300.0);
        }
        process_cell_energy_modifications(&mut frame, &mut sd);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!(
            (u.temperature.get(7) - 301.0).abs() < 1e-4,
            "cap 生效，got {}",
            u.temperature.get(7)
        );
    }

    #[test]
    fn cell_energy_modification_skips_vacuum_and_tiny_mass() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.resize(4, Default::default());
            table.temperature_data[1].state = 2;
            table.temperature_data[1].specific_heat_capacity = 4.179;
            table.temperature_data[1].low_temp = 0.0;
            table.temperature_data[1].high_temp = 1000.0;
        }
        let mut sd = make_test_sim_data();
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(7, 0); // 真空
            u.mass.set(7, 0.0);
            u.temperature.set(7, 300.0);
        }
        let mut frame = SimFrameInfo::default();
        let mut buf = Vec::new();
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&500.0f32.to_le_bytes());
        buf.extend_from_slice(&3200.0f32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        for b in buf {
            frame.cell_energy_modifications.push(b);
        }
        process_cell_energy_modifications(&mut frame, &mut sd);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.temperature.get(7), 300.0, "真空不应加热");
    }

    /// 2026-08-10 回归：负 kJ（降温）消息必须生效——原版 06_process_messages.c L172
    /// 只检查 mass > 0.001 && maxTemp > 0，不看 kJ 符号。此前误加 kj <= 0.0 判断
    /// 导致 DebugCool / HeatBulb 等全部降温路径失效（对应漏洞.txt #1）。
    #[test]
    fn cell_energy_modification_negative_kj_cools_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.resize(4, Default::default());
            table.temperature_data[1].state = 2; // 液体
            table.temperature_data[1].specific_heat_capacity = 4.179;
            table.temperature_data[1].low_temp = 0.0;
            table.temperature_data[1].high_temp = 1000.0;
        }
        // gameCell 0 → sim cell 7；50kg 液体 300K
        let mut sd = make_test_sim_data();
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.element_idx.set(7, 1);
            u.mass.set(7, 50.0);
            u.temperature.set(7, 300.0);
        }
        // 负 kJ：ΔT = -500/(50×4.179) ≈ -2.39 → 297.61
        let mut frame = SimFrameInfo::default();
        let mut buf = Vec::new();
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&(-500.0f32).to_le_bytes());
        buf.extend_from_slice(&3200.0f32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        for b in buf {
            frame.cell_energy_modifications.push(b);
        }
        process_cell_energy_modifications(&mut frame, &mut sd);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!(
            (u.temperature.get(7) - 297.61).abs() < 0.05,
            "负 kJ 应降温 → 297.61，got {}",
            u.temperature.get(7)
        );
    }

    #[test]
    fn process_frame_copies_debug_properties_to_sim_data() {
        let _lock = LIB_TESTS_LOCK.lock();
        use crate::a_framework::sim_data::DebugProperties;
        let mut frame = SimFrameInfo::default();
        frame.debug_properties = DebugProperties {
            building_temperature_scale: 100.0,
            building_to_building_temperature_scale: 100.0,
            is_debug_editing: true,
            pad: [false; 3],
        };
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, true);
        process_frame(&mut frame, &mut sd);
        // 原版 ProcessFrame 末尾（L117034-117046）复制到 SimData
        assert_eq!(sd.debug_properties.building_temperature_scale, 100.0);
        assert_eq!(sd.debug_properties.building_to_building_temperature_scale, 100.0);
        assert!(sd.debug_properties.is_debug_editing);
    }

    /// 2026-08-04 实现：ModifyBackwallData（16B）从 C2 桩改为真实处理
    /// （原版 ProcessElementChunkMessages 尾部 L871-914）——C# 用它设置背墙
    /// （含太空格真空背墙），此前丢弃 → 太空删除判定恒失败。
    #[test]
    fn process_element_chunk_processes_modify_backwall_data() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();
        // 16B：gameCell=0, elemIdx=5, pad=0, mass=1.5, temperature=300
        let mut msg = Vec::new();
        msg.extend_from_slice(&0i32.to_le_bytes());
        msg.extend_from_slice(&5u16.to_le_bytes());
        msg.extend_from_slice(&0u16.to_le_bytes());
        msg.extend_from_slice(&1.5f32.to_le_bytes());
        msg.extend_from_slice(&300.0f32.to_le_bytes());
        for b in msg {
            frame.modify_backwall_data_messages.push(b);
        }

        process_element_chunk_messages(&mut frame, &mut sd);

        unsafe {
            let bw = &*sd.backwall.ptr;
            // game_cell=0 → sim 内部格 (0/(6-2)+1)*6 + 0%4 + 1 = 7
            assert_eq!(bw.element_idx.get(7), 5, "backwall element_idx 应更新");
            assert!((bw.mass.get(7) - 1.5).abs() < 1e-6, "backwall mass 应更新");
            assert!((bw.temperature.get(7) - 300.0).abs() < 1e-6, "backwall temp 应更新");
            let events = &*sd.sim_events.ptr;
            assert_eq!(
                events.backwall_element_changed_info.len(),
                1,
                "元素变化应推 BackwallElementChangedInfo"
            );
            assert_eq!(events.backwall_element_changed_info.as_slice()[0].game_cell, 0);
        }
    }

    #[test]
    fn process_element_chunk_add_message_registers_chunk() {
        let _lock = LIB_TESTS_LOCK.lock();
        crate::c_simulation::element_chunk::reset_for_test();
        // 元素表：elem5 水（SHC=2、TC=4）
        {
            use crate::b_elements::element::ElementTemperatureData;
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
            table.temperature_data.resize(6, ElementTemperatureData::default());
            let mut water = ElementTemperatureData::default();
            water.state = 2;
            water.specific_heat_capacity = 2.0;
            water.thermal_conductivity = 4.0;
            table.temperature_data[5] = water;
        }
        // 构造 32B Add 消息字节（gameCell=5, callbackIdx=7, mass=100, temp=300, area=2, thickness=0.5, ground=0.5, elem=5, pad）
        let mut bytes = Vec::new();
        for v in [5i32, 7, 100, 300] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for v in [2.0f32, 0.5, 0.5] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&5u16.to_le_bytes());
        bytes.extend_from_slice(&[0u8, 0]);
        assert_eq!(bytes.len(), 32);

        let mut frame = SimFrameInfo::default();
        for b in &bytes {
            frame.element_chunk_messages.adds.push(*b);
        }
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, true);
        process_element_chunk_messages(&mut frame, &mut sd);
        assert_eq!(
            crate::c_simulation::element_chunk::element_chunk_count(),
            1,
            "Add 消息应注册一个碎片"
        );
        // 回调发出（callbackIdx=7 → component_state_changed）
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.component_state_changed_messages.len(), 1);
        assert_eq!(
            events.component_state_changed_messages.get(0).callback_idx,
            7
        );
    }

    #[test]
    fn process_building_to_building_messages_registers_contacts_and_callbacks() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 元素表：elem5 供热数据
        {
            use crate::b_elements::element::ElementTemperatureData;
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.clear();
            table.temperature_data.clear();
            table.temperature_data.resize(6, ElementTemperatureData::default());
            let mut water = ElementTemperatureData::default();
            water.state = 2;
            water.specific_heat_capacity = 2.0;
            water.thermal_conductivity = 4.0;
            table.temperature_data[5] = water;
        }
        crate::c_simulation::building_temperature::clear_building_temperature();
        crate::c_simulation::building_to_building::clear_building_to_building();
        // 注册 8 个建筑（句柄 0..7），供热记录由 elem5 推导
        for i in 0..8 {
            let msg = crate::a_framework::sim_events::AddBuildingHeatExchangeMsg {
                callback_idx: -1,
                elem_idx: 5,
                pad0: 0,
                pad1: 0,
                mass: 500.0,
                temperature: 300.0 + i as f32,
                thermal_conductivity: 1.0,
                overheat_temperature: 2000.0,
                operating_kilowatts: 0.0,
                min_x: 1,
                min_y: 2,
                max_x: 2,
                max_y: 3,
            };
            crate::c_simulation::building_temperature::add_building(&msg);
        }
        let mut frame = SimFrameInfo::default();
        // Register 8B {callbackIdx=7, heatExchange_handle=7} → adds
        let mut reg = Vec::new();
        for v in [7i32, 7] {
            reg.extend_from_slice(&v.to_le_bytes());
        }
        for b in &reg {
            frame.building_to_building_heat_exchange_messages.adds.push(*b);
        }
        // AddInContact 12B {self=0, buildingInContact=8, cellsInContact=3}
        let mut ac = Vec::new();
        for v in [0i32, 8, 3] {
            ac.extend_from_slice(&v.to_le_bytes());
        }
        for b in &ac {
            frame.add_building_in_contact_messages.push(*b);
        }
        // Remove 8B {callbackIdx=8, handle=0} → removes
        let mut rm = Vec::new();
        for v in [8i32, 0] {
            rm.extend_from_slice(&v.to_le_bytes());
        }
        for b in &rm {
            frame.building_to_building_heat_exchange_messages.removes.push(*b);
        }
        let mut sd = SimData::new_for_allocate(6, 6, 1, false, true);
        process_building_to_building_messages(&mut frame, &mut sd);
        // Register 回调 {callbackIdx=7, sim_handle=0}；Remove 回调 {8, -1}
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.component_state_changed_messages.len(), 2);
        assert_eq!(
            events.component_state_changed_messages.get(0).callback_idx,
            7
        );
        assert_eq!(
            events.component_state_changed_messages.get(0).sim_handle & 0xffffff,
            0
        );
        assert_eq!(
            events.component_state_changed_messages.get(1).callback_idx,
            8
        );
        assert_eq!(events.component_state_changed_messages.get(1).sim_handle, -1);
    }
    use crate::LIB_TESTS_LOCK;

    /// 初始化 elements_table：添加 2 个元素（索引 0=固体, 1=真空/非固体）
    fn init_elements_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.state_data.clear();
        // 元素 0：固体（state & 3 == 3）
        table.state_data.push(ElementStateData { state: 3 });
        // 元素 1：真空（state & 3 == 0）
        table.state_data.push(ElementStateData { state: 0 });
    }

    /// K2-B 专用：建 4 元素表 0=真空(state=0)、1=液体(state=2)、2=固体(state=3)、3=气体(state=1)。
    /// 挤压链路 displace_liquid/displace_gas 从 post_process_data 读状态，必须填充。
    fn init_displace_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.element_names.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        for (id, state) in [(0i32, 0u8), (1i32, 2u8), (2i32, 3u8), (3i32, 1u8)] {
            let mut elem = Element::default();
            elem.id = id;
            elem.state = state;
            table.elements.push(elem);
            table.state_data.push(ElementStateData { state });
            table.liquid_data.push(ElementLiquidData { state, flow: 0.0, ..Default::default() });
            table.pressure_data.push(ElementPressureData { state, flow: 0.0 });
            table.post_process_data.push(ElementPostProcessData { state, ..Default::default() });
        }
    }

    /// 构造测试用 SimData（4×4 含边界 = 6×6 = 36 个 cell）
    fn make_test_sim_data() -> SimData {
        let mut sd = SimData::new_zeroed();
        sd.width = 6;
        sd.height = 6;
        sd.num_game_cells = 4 * 4;
        sd.vacuum_element_idx = 1;

        let total_cells = (sd.width as usize) * (sd.height as usize);

        let cells = Box::new(CellSOA::with_size(total_cells));
        sd.cells = UniquePtr { ptr: Box::into_raw(cells) };

        // updated_cells 填充测试数据：cell_idx=5 处放固体元素
        let mut updated_cells = CellSOA::with_size(total_cells);
        updated_cells.element_idx.set(5, 0u16); // 元素索引 0
        updated_cells.mass.set(5, 100.0f32);
        updated_cells.temperature.set(5, 300.0f32);
        updated_cells.disease_idx.set(5, 0u8);
        updated_cells.disease_count.set(5, 0i32);
        sd.updated_cells = UniquePtr { ptr: Box::into_raw(Box::new(updated_cells)) };

        let backwall = Box::new(BackwallSOA::with_size(total_cells, sd.vacuum_element_idx));
        sd.backwall = UniquePtr { ptr: Box::into_raw(backwall) };

        let sim_events = Box::new(SimEvents::default());
        sd.sim_events = UniquePtr { ptr: Box::into_raw(sim_events) };

        // 分配 timers
        let timers_vec: Vec<Timers> = vec![Timers::default(); total_cells];
        let timers_box = timers_vec.into_boxed_slice();
        sd.timers = UniquePtr { ptr: Box::into_raw(timers_box) as *mut Timers };

        sd
    }

    #[test]
    fn process_dig_points_clears_solid_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_elements_table();
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();

        // gameCell=0, game_w=4 → simCell = (0%4+1) + (0/4+1)*6 = 1 + 6 = 7
        // 把测试数据放到 cell_idx=7
        let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
        updated_cells.element_idx.set(7, 0u16);  // 元素 0 = 固体
        updated_cells.mass.set(7, 100.0f32);
        updated_cells.temperature.set(7, 300.0f32);

        frame.dig_points.push(crate::a_framework::sim_events::DigPoint {
            game_cell: 0,
            callback_idx: -1,
            skip_event: 0,
            backwall: 0,
            _pad: [0, 0],
        });

        // 执行 process_dig_points
        process_dig_points(&mut frame, &mut sd);

        // 验证 cell_idx=7 已被清除
        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.element_idx.get(7), sd.vacuum_element_idx, "element should be vacuum");
        assert_eq!(updated_cells.mass.get(7), 0.0f32, "mass should be 0");

        // 验证 dig_points 已清空
        assert_eq!(frame.dig_points.len(), 0);

        // 验证 dig_info 有 1 个条目（mass > 0）
        let sim_events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(sim_events.dig_info.len(), 1, "dig_info should have 1 entry");

        // 验证 substance_change_info 有 1 个条目
        assert_eq!(sim_events.substance_change_info.len(), 1, "substance_change_info should have 1 entry");

        // 验证 old/new 均为 0xFFFF 占位（与原版一致）——真实值由
        // CopySimDataToGame 的 SubstanceChangeInfo 后处理回填
        // （对照源码 L132321-132337）。
        let sci = sim_events.substance_change_info.get(0);
        assert_eq!(sci.old_element_idx, 0xFFFF, "old_element_idx 应为 0xFFFF 占位");
        assert_eq!(sci.new_element_idx, 0xFFFF, "new_element_idx 应为 0xFFFF 占位（后处理回填）");
    }

    #[test]
    fn process_dig_points_skips_non_solid() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_elements_table();
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();

        // cell_idx=7 放 vacuum_element_idx（元素 1，非固体）
        let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
        updated_cells.element_idx.set(7, sd.vacuum_element_idx);
        updated_cells.mass.set(7, 0.0f32);

        frame.dig_points.push(crate::a_framework::sim_events::DigPoint {
            game_cell: 0,
            callback_idx: -1,
            skip_event: 0,
            backwall: 0,
            _pad: [0, 0],
        });

        process_dig_points(&mut frame, &mut sd);

        // 验证 cell 未被清除（元素仍为 vacuum，非固体不触发清除）
        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.element_idx.get(7), sd.vacuum_element_idx);

        // 验证 dig_info 为空（非固体不触发 dig_info）
        let sim_events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(sim_events.dig_info.len(), 0, "dig_info should be empty");
    }

    #[test]
    fn process_dig_points_with_callback_pushes_callback_info() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_elements_table();
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();

        // cell_idx=7 放固体元素
        let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
        updated_cells.element_idx.set(7, 0u16);
        updated_cells.mass.set(7, 50.0f32);

        frame.dig_points.push(crate::a_framework::sim_events::DigPoint {
            game_cell: 0,
            callback_idx: 42,  // 有效 callback
            skip_event: 1,     // 跳过事件（不生成 MassConsumedCallback）
            backwall: 0,
            _pad: [0, 0],
        });

        process_dig_points(&mut frame, &mut sd);

        // 验证 callback_info 有 1 个条目
        let sim_events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(sim_events.callback_info.len(), 1, "callback_info should have 1 entry");
        assert_eq!(sim_events.callback_info.get(0).callback_idx, 42);

        // 验证 dig_info 为空（skip_event=1）
        assert_eq!(sim_events.dig_info.len(), 0, "dig_info should be empty (skip_event=1)");

        // 验证 cell 已被清除
        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.element_idx.get(7), sd.vacuum_element_idx);
    }

    /// Add（mode 0）语义（2026-08-02 液滴回注修复）：向真空格添加液体 → 直接放置
    /// （add_liquid 真空分支，原版 L134426-134431），并产生 substance 事件。
    #[test]
    fn process_cell_modifications_add_liquid_to_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();

        // gameCell=0 → simCell=7（默认真空）；添加液体元素 1
        // 真空格无病菌：disease_idx=0xFF（真实游戏经 Load/ClearCell 初始化；
        // CellSOA::with_size 零初始化是原版构造器语义，此处需显式模拟真空态）。
        let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
        updated_cells.disease_idx.set(7, 0xFF);
        updated_cells.disease_count.set(7, 0);
        frame.cell_modifications.push(crate::a_framework::sim_events::CellModification {
            game_cell: 0,
            callback_idx: -1,
            mass: 75.0,
            temperature: 280.0,
            disease_count: 5,
            element_idx: 1,
            disease_idx: 2,
            replace_mode: 0,
            flags: 0,
            _pad: [0, 0, 0],
        });

        process_cell_modifications(&mut frame, &mut sd);

        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.element_idx.get(7), 1);
        assert_eq!(updated_cells.mass.get(7), 75.0);
        assert_eq!(updated_cells.temperature.get(7), 280.0);
        assert_eq!(updated_cells.disease_count.get(7), 5);
        assert_eq!(updated_cells.disease_idx.get(7), 2);
        let sim_events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(sim_events.substance_change_info.len(), 1, "Add 到真空应产生 substance 事件");
    }

    /// ProcessMassConsumption 矩形模式（原版 RectangularRemoved）：
    /// 4 格各 100kg，ConsumeMass 250kg（宽4×高1）→ 移除 100+100+50，
    /// 前两格清空、第三格剩 50、第四格不动；callback 推送 removed 质量 250。
    #[test]
    fn process_mass_consumption_rectangular_removes_region() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        let mut sd = make_test_sim_data();
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            for c in [7usize, 8, 9, 10] {
                updated.element_idx.set(c, 1); // 液体
                updated.mass.set(c, 100.0);
                updated.temperature.set(c, 300.0);
            }
        }
        let mut frame = SimFrameInfo::default();
        frame.mass_consumption_messages.push(crate::a_framework::sim_events::MassConsumption {
            game_cell: 0, // → 内部 (row1,col1)
            callback_idx: 42,
            mass: 250.0,
            element_idx: 1,
            radius: 4,
            height: 1,
        });
        process_mass_consumption(&mut frame, &mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(7), 0, "cell7 应被清空（元素→真空）");
            assert_eq!(updated.mass.get(7), 0.0);
            assert_eq!(updated.element_idx.get(8), 0, "cell8 应被清空");
            assert_eq!(updated.element_idx.get(9), 1, "cell9 应保留液体");
            assert!(
                (updated.mass.get(9) - 50.0).abs() < 1e-3,
                "cell9 应剩 50，got {}",
                updated.mass.get(9)
            );
            assert!((updated.mass.get(10) - 100.0).abs() < 1e-3, "cell10 不应被触及");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.mass_consumed_callbacks.len(), 1, "应推送 1 条回调");
            let cb = events.mass_consumed_callbacks.get(0);
            assert_eq!(cb.callback_idx, 42);
            assert_eq!(cb.elem_idx, 1);
            assert!(
                (cb.mass - 250.0).abs() < 1e-3,
                "回调质量应 250，got {}",
                cb.mass
            );
        }
        DestroyElementsTable();
    }

    /// ProcessMassConsumption 洪水半径模式（原版 FloodRemoved + Flood<> BFS）：
    /// 从 (row1,col1) 起半径 2（处理 0/1 跳），3 格液体 100kg，ConsumeMass 250kg
    /// → 移除 100+100+50，前两格清空、第三格剩 50；callback=-1 → 无回调。
    #[test]
    fn process_mass_consumption_flood_radius_removes_connected() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        let mut sd = make_test_sim_data();
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            for c in [7usize, 8, 13] {
                updated.element_idx.set(c, 1); // 液体（7=(1,1) 8=(1,2) 13=(2,1)）
                updated.mass.set(c, 100.0);
                updated.temperature.set(c, 300.0);
            }
        }
        let mut frame = SimFrameInfo::default();
        frame.mass_consumption_messages.push(crate::a_framework::sim_events::MassConsumption {
            game_cell: 0,
            callback_idx: -1,
            mass: 250.0,
            element_idx: 1,
            radius: 2,
            height: 0, // 洪水模式
        });
        process_mass_consumption(&mut frame, &mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(7), 0, "起点格应被清空");
            assert_eq!(updated.element_idx.get(8), 0, "右邻应被清空");
            assert_eq!(updated.element_idx.get(13), 1, "下邻应保留液体");
            assert!(
                (updated.mass.get(13) - 50.0).abs() < 1e-3,
                "下邻应剩 50，got {}",
                updated.mass.get(13)
            );
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.mass_consumed_callbacks.len(), 0, "callback=-1 不应推送");
        }
        DestroyElementsTable();
    }

    /// ElementConsumer 链路（2026-08-02 水泵修复）：注册 → 设速率 → 逐帧消费 →
    /// consumedMassInfo 携带正确 sim_handle（C# 泵靠它把水送进水管）。
    #[test]
    fn element_consumer_consumes_and_reports_mass() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        let mut sd = make_test_sim_data();
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(7, 1); // (row1,col1) 液体
            updated.mass.set(7, 100.0);
            updated.temperature.set(7, 300.0);
        }
        let mut frame = SimFrameInfo::default();
        // 注册（12B 消息体）
        let add = crate::a_framework::sim_events::AddElementConsumerMsg {
            cell_idx: 0,
            callback_idx: 42,
            radius: 2,
            configuration: 1, // 任意液体
            element_idx: 0,
        };
        let bytes = unsafe { std::slice::from_raw_parts(&add as *const _ as *const u8, 12) };
        for b in bytes {
            frame.element_consumer_messages.adds.push(*b);
        }
        process_element_consumer_messages(&mut frame, &mut sd);
        {
            let events = unsafe { &*sd.sim_events.ptr };
            assert_eq!(events.component_state_changed_messages.len(), 1);
            assert_eq!(events.component_state_changed_messages.get(0).callback_idx, 42);
            assert_eq!(events.component_state_changed_messages.get(0).sim_handle, 0);
        }
        // 设速率 10kg/s（12B 消息体）
        let set = crate::a_framework::sim_events::SetElementConsumerDataMsg {
            handle: 0,
            cell: 0,
            consumption_rate: 10.0,
        };
        let bytes = unsafe { std::slice::from_raw_parts(&set as *const _ as *const u8, 12) };
        for b in bytes {
            frame.element_consumer_messages.modifies.push(*b);
        }
        process_element_consumer_messages(&mut frame, &mut sd);
        // 逐帧消费：10 × 0.2s = 2kg
        let b = crate::d1_activity::full_grid_bounds(&sd);
        process_element_consumers(&mut sd, b);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!(
                (updated.mass.get(7) - 98.0).abs() < 1e-3,
                "应消耗 2kg，got {}",
                updated.mass.get(7)
            );
            assert_eq!(updated.element_idx.get(7), 1);
        }
        let consumed = &sd.element_consumer.consumed_mass_info;
        assert_eq!(consumed.len(), 1, "应推送 1 条 consumedMassInfo");
        let c = consumed.get(0);
        assert_eq!(c.sim_handle.value, 0);
        assert_eq!(c.removed_elem_idx, 1);
        assert!((c.mass - 2.0).abs() < 1e-3, "consumed mass 应 2，got {}", c.mass);
        // 2026-08-06 水泵回归：consumedMassInfo 温度必须为源格温度（300K），
        // 否则 C# 泵把 T=0 的水送进 storage → "Invalid cell modification (zero temp)"
        // 警告 + 水泵格水被判定为冰（用户实测崩溃前症状）。
        assert!(
            (c.temperature - 300.0).abs() < 1e-3,
            "consumed temperature 应为源格 300K，got {}",
            c.temperature
        );
        DestroyElementsTable();
    }

    /// find_reachable_state offset 轮转（原版 AnyInputCellHasState L38627）：
    /// 消费者以 offset_idx 为起点循环扫描可达格，实现水泵"轮流吸取"。
    /// 可达列表 BFS 序：[7, 8, 6, 13, 1, ...]（消费者内部格 7=(row1,col1)，radius 2）。
    /// cell8=液体1（列表 idx1）、cell6=液体2（idx2）：
    /// offset=0 → 从 idx0 扫 → cell8 命中液体1；offset=2 → 从 idx2 扫 → cell6 命中液体2。
    #[test]
    fn find_reachable_state_rotates_by_offset() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 自定义 4 元素表：0=真空(state0)、1=液体(state2)、2=液体(state2)、3=固体(state3)
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.clear();
            table.element_names.clear();
            table.state_data.clear();
            table.liquid_data.clear();
            table.pressure_data.clear();
            table.post_process_data.clear();
            table.temperature_data.clear();
            for (id, state) in [(0i32, 0u8), (1i32, 2u8), (2i32, 2u8), (3i32, 3u8)] {
                let mut elem = Element::default();
                elem.id = id;
                elem.state = state;
                table.elements.push(elem);
                table.state_data.push(ElementStateData { state });
                table
                    .liquid_data
                    .push(ElementLiquidData { state, flow: 0.0, ..Default::default() });
                table
                    .pressure_data
                    .push(ElementPressureData { state, flow: 0.0 });
                table
                    .post_process_data
                    .push(ElementPostProcessData { state, ..Default::default() });
            }
        }
        let mut sd = make_test_sim_data();
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(8, 1); // 液体1 @ 右邻 (x=2,y=1)
            updated.mass.set(8, 100.0);
            updated.temperature.set(8, 300.0);
            updated.element_idx.set(13, 2); // 液体2 @ 下邻 (x=1,y=2)
            updated.mass.set(13, 100.0);
            updated.temperature.set(13, 300.0);
        }
        // 消费者内部格 7 = (x=1,y=1)，radius 2，target=液体(state2)。
        // 原版 Flood 跳过 x==0/y==0 边界格 → 可达候选仅 {7, 8(右), 13(下)}，
        // BFS 邻居序上→左→右→下中仅 右(8)、下(13) 未被边界挡 → 候选序 [7, 8, 13]。
        assert_eq!(
            find_reachable_state(&sd, 1, 1, 2, 2, 0),
            Some(1),
            "offset=0 → 从 idx0 扫 → 液体1"
        );
        assert_eq!(
            find_reachable_state(&sd, 1, 1, 2, 2, 1),
            Some(1),
            "offset=1 → 从 idx1 扫 → 液体1"
        );
        assert_eq!(
            find_reachable_state(&sd, 1, 1, 2, 2, 2),
            Some(2),
            "offset=2 → 从 idx2 扫 → 液体2"
        );
        DestroyElementsTable();
    }

    /// 固体格挡 BFS 扩展（原版 GetReachableCells → Flood skip_solid=true）：
    /// 泵格 (1,1) 下方 (1,2)=cell13 放固体 → 墙后 (1,3)=cell19 的液体**选不到**。
    /// 若 BFS 不挡固体（改动前），cell19 会进候选并被 offset 扫到。
    #[test]
    fn find_reachable_state_solid_blocks_bfs_extension() {
        let _lock = LIB_TESTS_LOCK.lock();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.clear();
            table.element_names.clear();
            table.state_data.clear();
            table.liquid_data.clear();
            table.pressure_data.clear();
            table.post_process_data.clear();
            table.temperature_data.clear();
            for (id, state) in [(0i32, 0u8), (1i32, 2u8), (2i32, 3u8)] {
                let mut elem = Element::default();
                elem.id = id;
                elem.state = state;
                table.elements.push(elem);
                table.state_data.push(ElementStateData { state });
                table
                    .liquid_data
                    .push(ElementLiquidData { state, flow: 0.0, ..Default::default() });
                table
                    .pressure_data
                    .push(ElementPressureData { state, flow: 0.0 });
                table
                    .post_process_data
                    .push(ElementPostProcessData { state, ..Default::default() });
            }
        }
        let mut sd = make_test_sim_data();
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(13, 2); // 固体 @ (1,2) = 泵正下方，挡路
            updated.mass.set(13, 100.0);
            updated.element_idx.set(19, 1); // 液体 @ (1,3) = 墙后
            updated.mass.set(19, 100.0);
            updated.temperature.set(19, 300.0);
        }
        // radius 3 足够越过 (1,3)；若固体不挡路应能选到液体1
        assert_eq!(
            find_reachable_state(&sd, 1, 1, 3, 2, 0),
            None,
            "固体格 (1,2) 应挡 BFS，(1,3) 液体不可达"
        );
        DestroyElementsTable();
    }

    /// ProcessMassEmission（2026-08-02 喷口输出）：向真空格发射 5kg 液体 →
    /// 元素/质量/温度写入 + MassEmittedCallback 推送（emitted=1）。
    #[test]
    fn mass_emission_adds_mass_and_callback() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        let mut sd = make_test_sim_data();
        sd.vacuum_element_idx = 0;
        let mut frame = SimFrameInfo::default();
        frame.mass_emission_messages.push(crate::a_framework::sim_events::MassEmission {
            game_cell: 0, // → 内部 (row1,col1)
            callback_idx_or_temp: f32::from_bits(42), // callbackIdx=42 的位模式
            mass: 5.0,
            temperature: 300.0,
            disease_count: 0,
            element_idx: 1,
            disease_idx: 0xFF,
            emitted: 0,
        });
        process_mass_emission(&mut frame, &mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(7), 1);
            assert!((updated.mass.get(7) - 5.0).abs() < 1e-3);
            assert!((updated.temperature.get(7) - 300.0).abs() < 1e-3);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.mass_emitted_callbacks.len(), 1);
            let cb = events.mass_emitted_callbacks.get(0);
            assert_eq!(cb.callback_idx, 42);
            assert_eq!(cb.elem_idx, 1);
            assert_eq!(cb.emitted, 1, "真空目标 → emitted=1");
            assert!((cb.mass - 5.0).abs() < 1e-3);
        }
        DestroyElementsTable();
    }

    /// 2026-08-07 MassEmission 位移/失败回调回归：目标格为异元素（固体）且位移失败时，
    /// 原版不加质量、推失败回调（emitted=0、elem=0xffff、全零）——此前无条件加质量。
    #[test]
    fn mass_emission_failure_pushes_failure_callback_without_adding() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        let mut sd = make_test_sim_data();
        sd.vacuum_element_idx = 0;
        // 格 7 放固体（elem 2）
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(7, 2);
            updated.mass.set(7, 100.0);
            updated.temperature.set(7, 300.0);
        }
        let mut frame = SimFrameInfo::default();
        frame.mass_emission_messages.push(crate::a_framework::sim_events::MassEmission {
            game_cell: 0, // → 内部 (row1,col1)=7
            callback_idx_or_temp: f32::from_bits(42),
            mass: 5.0,
            temperature: 300.0,
            disease_count: 0,
            element_idx: 1, // 液体 → 目标固体位移失败
            disease_idx: 0xFF,
            emitted: 0,
        });
        process_mass_emission(&mut frame, &mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(7), 2, "原格保持固体");
            assert!(
                (updated.mass.get(7) - 100.0).abs() < 1e-3,
                "位移失败不应加质量，got {}",
                updated.mass.get(7)
            );
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.mass_emitted_callbacks.len(), 1, "应推失败回调");
            let cb = events.mass_emitted_callbacks.get(0);
            assert_eq!(cb.callback_idx, 42);
            assert_eq!(cb.elem_idx, 0xFFFF);
            assert_eq!(cb.emitted, 0);
            assert_eq!(cb.mass, 0.0);
            assert_eq!(cb.disease_idx, 0xFF);
        }
        DestroyElementsTable();
    }

    #[test]
    fn process_cell_modifications_replace_vacuum_destroys_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_elements_table();
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();

        // 先在 cell_idx=7 放固体（元素 0）
        let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
        updated_cells.element_idx.set(7, 0u16);
        updated_cells.mass.set(7, 500.0f32);
        updated_cells.temperature.set(7, 300.0f32);

        // Destroy：Replace 为真空（元素 1），质量/温度任意（会被钳 0）
        frame.cell_modifications.push(crate::a_framework::sim_events::CellModification {
            game_cell: 0,
            callback_idx: 7,
            temperature: 0.0,
            mass: 0.0,
            disease_count: 0,
            element_idx: 1, // 真空
            replace_mode: 1,
            disease_idx: 0xFF,
            flags: 0,
            _pad: [0, 0, 0],
        });

        process_cell_modifications(&mut frame, &mut sd);

        // 格子已摧毁：元素=真空，质量/温度=0
        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.element_idx.get(7), 1, "element should be vacuum");
        assert_eq!(updated_cells.mass.get(7), 0.0f32, "vacuum mass clamped to 0");
        assert_eq!(updated_cells.temperature.get(7), 0.0f32, "vacuum temperature clamped to 0");

        // 通知已发出：substance_change_info + callback_info
        let sim_events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(sim_events.substance_change_info.len(), 1, "ChangeSubstance should fire");
        assert_eq!(sim_events.substance_change_info.get(0).cell_idx, 0);
        assert_eq!(sim_events.callback_info.len(), 1, "callback should fire");
        assert_eq!(sim_events.callback_info.get(0).callback_idx, 7);
    }

    #[test]
    fn process_cell_modifications_replace_solid_spawns_cell() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_elements_table();
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();

        // cell_idx=7 初始为真空
        let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
        updated_cells.element_idx.set(7, 1u16);
        updated_cells.mass.set(7, 0.0f32);

        // 生成固体（元素 0），500kg @ 350K
        frame.cell_modifications.push(crate::a_framework::sim_events::CellModification {
            game_cell: 0,
            callback_idx: -1,
            temperature: 350.0,
            mass: 500.0,
            disease_count: 0,
            element_idx: 0, // 固体
            replace_mode: 1,
            disease_idx: 0xFF,
            flags: 0,
            _pad: [0, 0, 0],
        });

        process_cell_modifications(&mut frame, &mut sd);

        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.element_idx.get(7), 0, "element should be solid");
        assert_eq!(updated_cells.mass.get(7), 500.0f32, "mass preserved");
        assert_eq!(updated_cells.temperature.get(7), 350.0f32, "temperature preserved");

        let sim_events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(sim_events.substance_change_info.len(), 1, "ChangeSubstance should fire");
    }

    /// K2-B 修复测试：replace_mode 2（ReplaceAndDisplace）把目标格液体挤压到邻格后再写新字段。
    /// 目标格 gameCell=5 → simCell=14（row2,col2 中心，4 邻格全内部）。
    /// displace_liquid（原版 DisplaceLiquid L34136）：4 真空邻格各得 100/4=25，
    /// 源格 ClearCell；随后新字段（固体元素 2）写入目标格。
    #[test]
    fn process_cell_modifications_replace_and_displace_squeezes_liquid() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_displace_table();
        let mut sd = make_test_sim_data();
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1); // 液体
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            for c in [13usize, 15, 20, 8] {
                updated.element_idx.set(c, 0); // 真空
                updated.mass.set(c, 0.0);
                updated.temperature.set(c, 0.0);
            }
        }
        let mut frame = SimFrameInfo::default();
        frame.cell_modifications.push(crate::a_framework::sim_events::CellModification {
            game_cell: 5, // → simCell 14
            callback_idx: -1,
            temperature: 350.0,
            mass: 500.0,
            disease_count: 7,
            element_idx: 2, // 固体（新元素）
            replace_mode: 2,
            disease_idx: 0xAA,
            flags: 0,
            _pad: [0, 0, 0],
        });
        process_cell_modifications(&mut frame, &mut sd);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // 新字段已写入目标格
            assert_eq!(updated.element_idx.get(14), 2, "目标格应写入新固体元素");
            assert_eq!(updated.mass.get(14), 500.0, "目标格质量写入");
            assert_eq!(updated.temperature.get(14), 350.0, "目标格温度写入");
            assert_eq!(updated.disease_count.get(14), 7, "目标格病菌数写入");
            assert_eq!(updated.disease_idx.get(14), 0xAA, "目标格病菌 idx 写入");
            // 原液体被挤压均分到 4 邻格（displace_liquid 链路必须生产执行）
            for c in [13usize, 15, 20, 8] {
                assert_eq!(updated.element_idx.get(c), 1, "邻格 {} 应接收液体元素", c);
                assert!(
                    (updated.mass.get(c) - 25.0).abs() < 1e-3,
                    "邻格 {} 应得 100/4=25，got {}",
                    c,
                    updated.mass.get(c)
                );
            }
        }
        DestroyElementsTable();
    }

    #[test]
    fn process_cell_properties_sets_and_clears() {
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();

        // 先设置初始值
        let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
        updated_cells.properties.set(7, 0x00);

        // 设置属性 0x0F
        frame.set_cell_properties.push(crate::a_framework::sim_events::CellPropertiesChange {
            game_cell: 0,         // → simCell=7
            callback_idx: -1,
            property_flags: 0x0F, // mask
            set_or_clear: 1,      // set
            _pad: [0, 0],
        });

        process_cell_properties(&mut frame, &mut sd);

        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.properties.get(7), 0x0F);

        // 清除属性 0x0F
        let mut frame2 = SimFrameInfo::default();
        frame2.set_cell_properties.push(crate::a_framework::sim_events::CellPropertiesChange {
            game_cell: 0,
            callback_idx: -1,
            property_flags: 0x0F, // mask
            set_or_clear: 0,      // clear
            _pad: [0, 0],
        });

        process_cell_properties(&mut frame2, &mut sd);

        let updated_cells = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(updated_cells.properties.get(7), 0x00);
    }
}
