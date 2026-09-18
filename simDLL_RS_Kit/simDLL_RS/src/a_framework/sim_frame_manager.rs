//! SimFrameManager — 帧管理器 + 消息路由骨架。
//!
//! 字段对照源码 00_types_reference.c L6468-6479。
//! A2 实现 handle_message 45 match arm 路由骨架（全部返回 null，不 emplace）。
//! A3 接入真实 emplace 逻辑。

use crate::a_framework::buffer::BinaryBufferReader;
use crate::a_framework::message_handler::MessageType;
use crate::a_framework::sim_data::{ActiveRegion, DebugProperties};
use crate::a_framework::sim_events::{
    CellModification, CellPropertiesChange, DigPoint, MassConsumption, MassEmission,
    SetCellFloatValue,
};
use crate::a_framework::stl_shim::{MsvcMutex, MsvcVector};
use std::ffi::c_void;

/// 从 BinaryBufferReader 当前位置读取一个结构体（不消耗 reader 的所有权）。
///
/// 对照源码 `std::vector<>::_Emplace_reallocate<>(this, ptr, param_3)`：
/// 源码直接从 param_3（消息数据指针）拷贝 sizeof(T) 字节到 vector 末尾。
/// Rust 等价：从 reader 的内部缓冲区读取 sizeof(T) 字节并 reinterpret 为 T。
///
/// 读取后推进 reader 的 offset。
unsafe fn read_struct<T: Copy + Default>(reader: &mut BinaryBufferReader) -> Option<T> {
    let size = std::mem::size_of::<T>();
    if reader.offset() + size as u64 > reader.buffer_size {
        tracing::warn!(
            "read_struct: size {} exceeds buffer (offset={}, buf_size={})",
            size, reader.offset(), reader.buffer_size
        );
        return None;
    }
    let ptr = (reader.buffer_data as *const u8).add(reader.offset() as usize) as *const T;
    let val = std::ptr::read_unaligned(ptr);
    reader.offset += size as u64;
    Some(val)
}

/// 从 reader 读取 N 字节并追加到 MsvcVector<u8>（C2 桩消息用）。
///
/// C1 阶段：某些消息（如 ModifyCellEnergy/ConsumeDisease 等）的目标 vector 是
/// `MsvcVector<u8>`（C2 阶段会替换为真实结构体类型）。此函数读取 N 字节并追加，
/// 保持 reader 对齐，使后续消息能正确解析。
///
/// 返回 true 表示成功 emplace，false 表示读取失败。
fn emplace_bytes(vec: &mut MsvcVector<u8>, reader: &mut BinaryBufferReader, n: usize) -> bool {
    match reader.read_bytes(n) {
        Ok(bytes) => {
            for b in bytes {
                vec.push(b);
            }
            true
        }
        Err(_) => {
            tracing::warn!(
                "emplace_bytes: failed to read {} bytes (offset={}, buf_size={})",
                n, reader.offset(), reader.buffer_size
            );
            false
        }
    }
}

// ===== 消息 ID 常量（用于 match 模式）=====
// Rust 的 match 模式不允许 `MessageType::X as u32` 表达式，需用 const。
const MSG_DIG: u32 = MessageType::Dig as u32;
const MSG_MODIFY_CELL: u32 = MessageType::ModifyCell as u32;
const MSG_SET_INSULATION: u32 = MessageType::SetInsulationValue as u32;
const MSG_SET_STRENGTH: u32 = MessageType::SetStrengthValue as u32;
const MSG_CHANGE_CELL_PROPS: u32 = MessageType::ChangeCellProperties as u32;
const MSG_MASS_CONSUMPTION: u32 = MessageType::MassConsumption as u32;
const MSG_MASS_EMISSION: u32 = MessageType::MassEmission as u32;
const MSG_MODIFY_CELL_WORLD_ZONE: u32 = MessageType::ModifyCellWorldZone as u32;
const MSG_MODIFY_CELL_ENERGY: u32 = MessageType::ModifyCellEnergy as u32;
const MSG_CONSUME_DISEASE: u32 = MessageType::ConsumeDisease as u32;
const MSG_CELL_DISEASE_MOD: u32 = MessageType::CellDiseaseModification as u32;
const MSG_CELL_RADIATION_MOD: u32 = MessageType::CellRadiationModification as u32;
const MSG_RADIATION_PARAMS_MOD: u32 = MessageType::RadiationParamsModification as u32;
const MSG_MODIFY_BUILDING_ENERGY: u32 = MessageType::ModifyBuildingEnergy as u32;
const MSG_ADD_BUILDING_IN_CONTACT: u32 = MessageType::AddInContactBuildingToBuildingToBuildingHeatExchange as u32;
const MSG_MOVE_ELEMENT_CHUNK: u32 = MessageType::MoveElementChunk as u32;
const MSG_MODIFY_ELEMENT_CHUNK_ENERGY: u32 = MessageType::ModifyElementChunkEnergy as u32;
const MSG_MODIFY_CHUNK_TEMP_ADJUSTER: u32 = MessageType::ModifyChunkTemperatureAdjuster as u32;
const MSG_MODIFY_BACKWALL_DATA: u32 = MessageType::ModifyBackwallData as u32;

const MSG_ADD_BUILDING_HE: u32 = MessageType::AddBuildingHeatExchange as u32;
const MSG_MODIFY_BUILDING_HE: u32 = MessageType::ModifyBuildingHeatExchange as u32;
const MSG_REMOVE_BUILDING_HE: u32 = MessageType::RemoveBuildingHeatExchange as u32;
const MSG_ADD_BUILDING_TO_BUILDING_HE: u32 = MessageType::AddBuildingToBuildingHeatExchange as u32;
const MSG_REMOVE_BUILDING_TO_BUILDING_HE: u32 = MessageType::RemoveBuildingToBuildingHeatExchange as u32;
const MSG_REMOVE_BUILDING_IN_CONTACT: u32 = MessageType::RemoveBuildingInContactFromBuildingToBuildingHeatExchange as u32;
const MSG_ADD_ELEMENT_CHUNK: u32 = MessageType::AddElementChunk as u32;
const MSG_REMOVE_ELEMENT_CHUNK: u32 = MessageType::RemoveElementChunk as u32;
const MSG_SET_ELEMENT_CHUNK_DATA: u32 = MessageType::SetElementChunkData as u32;
const MSG_ADD_ELEMENT_CONSUMER: u32 = MessageType::AddElementConsumer as u32;
const MSG_REMOVE_ELEMENT_CONSUMER: u32 = MessageType::RemoveElementConsumer as u32;
const MSG_SET_ELEMENT_CONSUMER_DATA: u32 = MessageType::SetElementConsumerData as u32;
const MSG_ADD_ELEMENT_EMITTER: u32 = MessageType::AddElementEmitter as u32;
const MSG_MODIFY_ELEMENT_EMITTER: u32 = MessageType::ModifyElementEmitter as u32;
const MSG_REMOVE_ELEMENT_EMITTER: u32 = MessageType::RemoveElementEmitter as u32;
const MSG_ADD_DISEASE_EMITTER: u32 = MessageType::AddDiseaseEmitter as u32;
const MSG_MODIFY_DISEASE_EMITTER: u32 = MessageType::ModifyDiseaseEmitter as u32;
const MSG_REMOVE_DISEASE_EMITTER: u32 = MessageType::RemoveDiseaseEmitter as u32;
const MSG_ADD_DISEASE_CONSUMER: u32 = MessageType::AddDiseaseConsumer as u32;
const MSG_MODIFY_DISEASE_CONSUMER: u32 = MessageType::ModifyDiseaseConsumer as u32;
const MSG_REMOVE_DISEASE_CONSUMER: u32 = MessageType::RemoveDiseaseConsumer as u32;
const MSG_ADD_RADIATION_EMITTER: u32 = MessageType::AddRadiationEmitter as u32;
const MSG_MODIFY_RADIATION_EMITTER: u32 = MessageType::ModifyRadiationEmitter as u32;
const MSG_REMOVE_RADIATION_EMITTER: u32 = MessageType::RemoveRadiationEmitter as u32;

const MSG_NEW_GAME_FRAME: u32 = MessageType::SimFrameManager_NewGameFrame as u32;
const MSG_SET_DEBUG_PROPERTIES: u32 = MessageType::SetDebugProperties as u32;

// ===== SimFrameInfo 结构体（完整版，0x5b8 = 1464 字节）=====
//
// 对照源码 00_types_reference.c L5406-5446（从 PDB 恢复的真实定义）。
// 字段顺序和类型严格按源码，保证偏移正确（HandleMessage 按 offset emplace）。
//
// C1 策略：
// - Dig/ModifyCell/SetInsulation/SetStrength/SetCellProperties/ClearCellProperties/
//   ModifyCellWorldZone/MassConsumption/MassEmission → 真实类型
// - 其他 C2 桩消息 → MsvcVector<u8> 占位（C1 只 clear_keep_capacity，不 push）
// - ComponentMessages → 3 × MsvcVector<u8>（C1 只清空，不处理）

/// CellWorldZoneModification — ModifyCellWorldZone 消息体（8B，MSVC 默认对齐）。
/// 对照源码 00_types_reference.c L1840-1843。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CellWorldZoneModification {
    pub game_cell: i32,   // @0x0
    pub zone_id: u8,      // @0x4
    pub _pad: [u8; 3],    // @0x5 对齐到 8B
}

/// ComponentMessages — 组件消息三元组（96B = 3 × 32B）。
/// 对照源码 00_types_reference.c L3768（ComponentMessages 模板实例）。
/// C1 阶段用 MsvcVector<u8> 占位（只清空，不处理）。
#[repr(C)]
pub struct ComponentMessages {
    pub adds: MsvcVector<u8>,
    pub modifies: MsvcVector<u8>,
    pub removes: MsvcVector<u8>,
}

impl Default for ComponentMessages {
    fn default() -> Self {
        Self {
            adds: MsvcVector::new(),
            modifies: MsvcVector::new(),
            removes: MsvcVector::new(),
        }
    }
}

/// SimFrameInfo — 单帧信息（0x5b8 = 1464 字节）。
///
/// 对照源码 00_types_reference.c L5406-5446。
/// 包含一帧内所有消息的 vector 缓冲 + 调试属性 + 帧时间。
#[repr(C)]
pub struct SimFrameInfo {
    pub elapsed_seconds: f32,                                              // @0x000
    pub _pad0: [u8; 4],                                                    // @0x004
    pub dig_points: MsvcVector<DigPoint>,                                  // @0x008
    pub cell_modifications: MsvcVector<CellModification>,                  // @0x028
    pub cell_energy_modifications: MsvcVector<u8>,                         // @0x048 (C2 桩)
    pub pipe_changes: MsvcVector<u8>,                                      // @0x068 (C2 桩)
    pub set_insulation_values: MsvcVector<SetCellFloatValue>,              // @0x088
    pub set_strength_values: MsvcVector<SetCellFloatValue>,                // @0x0a8
    pub set_cell_properties: MsvcVector<CellPropertiesChange>,             // @0x0c8
    pub clear_cell_properties: MsvcVector<CellPropertiesChange>,           // @0x0e8
    pub consume_disease: MsvcVector<u8>,                                   // @0x108 (C2 桩)
    pub cell_disease_modifications: MsvcVector<u8>,                        // @0x128 (C2 桩)
    pub cell_radiation_modifications: MsvcVector<u8>,                      // @0x148 (C2 桩)
    pub radiation_params_modifications: MsvcVector<u8>,                    // @0x168 (C2 桩)
    pub cell_world_zone_modifications: MsvcVector<CellWorldZoneModification>, // @0x188
    pub mass_consumption_messages: MsvcVector<MassConsumption>,            // @0x1a8
    pub mass_emission_messages: MsvcVector<MassEmission>,                  // @0x1c8
    pub building_heat_exchange_messages: ComponentMessages,                // @0x1e8 (C2 桩)
    pub modify_building_energy_messages: MsvcVector<u8>,                   // @0x248 (C2 桩)
    pub building_to_building_heat_exchange_messages: ComponentMessages,    // @0x268 (C2 桩)
    pub add_building_in_contact_messages: MsvcVector<u8>,                  // @0x2c8 (C2 桩)
    pub element_chunk_messages: ComponentMessages,                         // @0x2e8 (C2 桩)
    pub move_element_chunk_messages: MsvcVector<u8>,                       // @0x348 (C2 桩)
    pub modify_element_chunk_energy_messages: MsvcVector<u8>,              // @0x368 (C2 桩)
    pub modify_element_chunk_adjuster_messages: MsvcVector<u8>,            // @0x388 (C2 桩)
    pub modify_backwall_data_messages: MsvcVector<u8>,                     // @0x3a8 (C2 桩)
    pub element_consumer_messages: ComponentMessages,                      // @0x3c8 (C2 桩)
    pub element_emitter_messages: ComponentMessages,                       // @0x428 (C2 桩)
    pub disease_emitter_messages: ComponentMessages,                       // @0x488 (C2 桩)
    pub disease_consumer_messages: ComponentMessages,                      // @0x4e8 (C2 桩)
    pub radiation_emitter_messages: ComponentMessages,                     // @0x548 (C2 桩)
    pub debug_properties: DebugProperties,                                 // @0x5a8
    pub _pad1: [u8; 4],                                                    // @0x5b4
}

impl Default for SimFrameInfo {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

/// 清空 ComponentMessages 的三个 vector（保留容量）。
fn clear_component_messages(cm: &mut ComponentMessages) {
    cm.adds.clear_keep_capacity();
    cm.modifies.clear_keep_capacity();
    cm.removes.clear_keep_capacity();
}

/// NewGameFrame — 新游戏帧辅助结构（28B）。
/// 源码 00_types_reference.c L6391-6396。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NewGameFrame {
    pub elapsed_seconds: f32,
    pub active_region_min_x: i32,
    pub active_region_min_y: i32,
    pub active_region_max_x: i32,
    pub active_region_max_y: i32,
    pub current_sunlight_intensity: f32,
    pub current_cosmic_radiation_intensity: f32,
}

/// SimFrameManager — 帧管理器（约 288B）。
/// 源码 L6468-6479。
#[repr(C)]
pub struct SimFrameManager {
    pub frame_mutex: MsvcMutex,                         // offset 0, 80B
    pub current_frame: *mut SimFrameInfo,               // offset 80, 8B
    pub frame_pool: MsvcVector<*mut SimFrameInfo>,      // offset 88, 32B
    pub queued_frames: MsvcVector<*mut SimFrameInfo>,   // offset 120, 32B
    pub active_frames: MsvcVector<*mut SimFrameInfo>,   // offset 152, 32B
    pub processed_frames: MsvcVector<*mut SimFrameInfo>,// offset 184, 32B
    pub active_region: NewGameFrame,                    // offset 216, 28B
    pub elapsed_seconds: f32,                           // offset 244, 4B
    pub active_regions: MsvcVector<ActiveRegion>,       // offset 248, 32B
    pub num_frames_processed: i32,                      // offset 280, 4B
    pub _padding: i32,                                  // offset 284, 4B
}

// SAFETY: 含裸指针字段，为放入 Mutex<Box<SimFrameManager>> 补 Send/Sync。
unsafe impl Send for SimFrameManager {}
unsafe impl Sync for SimFrameManager {}

impl SimFrameManager {
    /// 构造零值初始化的 SimFrameManager，并创建初始 current_frame。
    pub fn new_zeroed() -> Self {
        let mut mgr: Self = unsafe { std::mem::zeroed() };
        mgr.new_frame();
        mgr
    }

    /// 从帧池取/建帧，置为 current_frame。
    /// 对照源码 08_sim_frame_manager.c L1116-1176。
    ///
    /// **线程安全（2026-08-01 修复）**：原版 NewFrame 全程持 frame_mutex
    /// （08_sim_frame_manager.c L1128 `_Mtx_lock(this)`）；本函数操作
    /// frame_pool/queued_frames/current_frame，与 sim 线程的
    /// begin_frame_processing/end_frame_processing（已持 frame_mutex）
    /// 并发访问同一批数据结构——此前缺锁属数据竞争隐患（vector push/erase
    /// 与遍历并发 = UB）。加锁后与 begin/end 互斥。
    /// 锁序：sim 线程 m_sim_mutex → frame_mutex；本函数仅 frame_mutex，
    /// 单向无嵌套，无死锁。
    pub fn new_frame(&mut self) {
        unsafe { self.frame_mutex.lock_raw(); }
        self.new_frame_locked();
        unsafe { self.frame_mutex.unlock_raw(); }
    }

    /// new_frame 的锁内核心（调用方须已持 frame_mutex）。
    fn new_frame_locked(&mut self) {
        let frame_ptr = if self.frame_pool.is_empty() {
            let mut boxed = Box::new(SimFrameInfo::default());
            // 原版 SimFrameInfo 构造默认 debugProperties=0.001（L113777-113778）
            unsafe {
                (*boxed).debug_properties = DebugProperties {
                    building_temperature_scale: 0.001,
                    building_to_building_temperature_scale: 0.001,
                    ..DebugProperties::default()
                };
            }
            Box::into_raw(boxed)
        } else {
            // 原版 NewFrame（08_sim_frame_manager.c L1162-1164）：framePool 非空时
            // **pop_back**——取末尾元素 `*(begin + (size-1)*8)`，随后 end -= 8。
            // 此前误用取头部 erase(0)，虽不改变"取空闲帧"语义，但不符合源码顺序。
            let len = self.frame_pool.len();
            let ptr = self.frame_pool.as_slice()[len - 1];
            unsafe {
                self.frame_pool.end = self.frame_pool.end.sub(1);
            }
            ptr
        };
        if !frame_ptr.is_null() {
            unsafe {
                let frame = &mut *frame_ptr;
                // 清空所有 vector（对照源码 NewFrame L225-244 的 clear_keep_capacity 行为）
                frame.dig_points.clear_keep_capacity();
                frame.cell_modifications.clear_keep_capacity();
                frame.cell_energy_modifications.clear_keep_capacity();
                frame.pipe_changes.clear_keep_capacity();
                frame.set_insulation_values.clear_keep_capacity();
                frame.set_strength_values.clear_keep_capacity();
                frame.set_cell_properties.clear_keep_capacity();
                frame.clear_cell_properties.clear_keep_capacity();
                frame.consume_disease.clear_keep_capacity();
                frame.cell_disease_modifications.clear_keep_capacity();
                frame.cell_radiation_modifications.clear_keep_capacity();
                frame.radiation_params_modifications.clear_keep_capacity();
                frame.cell_world_zone_modifications.clear_keep_capacity();
                frame.mass_consumption_messages.clear_keep_capacity();
                frame.mass_emission_messages.clear_keep_capacity();
                clear_component_messages(&mut frame.building_heat_exchange_messages);
                frame.modify_building_energy_messages.clear_keep_capacity();
                clear_component_messages(&mut frame.building_to_building_heat_exchange_messages);
                frame.add_building_in_contact_messages.clear_keep_capacity();
                clear_component_messages(&mut frame.element_chunk_messages);
                frame.move_element_chunk_messages.clear_keep_capacity();
                frame.modify_element_chunk_energy_messages.clear_keep_capacity();
                frame.modify_element_chunk_adjuster_messages.clear_keep_capacity();
                frame.modify_backwall_data_messages.clear_keep_capacity();
                clear_component_messages(&mut frame.element_consumer_messages);
                clear_component_messages(&mut frame.element_emitter_messages);
                clear_component_messages(&mut frame.disease_emitter_messages);
                clear_component_messages(&mut frame.disease_consumer_messages);
                clear_component_messages(&mut frame.radiation_emitter_messages);
                // 重置帧时间和调试属性
                frame.elapsed_seconds = 0.0;
                // 原版 NewFrame 复用帧**不重置** debugProperties（★36）；
                // 仅在新建帧时（上方 Box::new 路径）置构造默认值。
            }
        }
        self.current_frame = frame_ptr;
    }

    /// handle_message — 消息路由（45 个分支）。
    ///
    /// 对照源码 08_sim_frame_manager.c L393-1112。
    ///
    /// **签名**：返回 `bool`（是否已处理）+ `out_result` 输出参数。
    /// 对照源码 01_sim_api.c L226-228：HandleMessage 返回 bool，
    /// 通过 `void** local_res20` 参数输出结果。
    /// - 返回 `true`：已处理，不再查 handler 表
    /// - 返回 `false`：未处理，查 14 项 handler 表
    ///
    /// C1 实现：
    /// - 非物理消息（Dig/ModifyCell/SetInsulation 等）：真实 emplace 到 SimFrameInfo vector
    /// - NewGameFrame：设置 elapsedSeconds + 解析 active_regions + new_frame
    /// - SetDebugProperties：直接写 debugProperties 字段
    /// - ComponentMessages 类（building/emitter/consumer 等）：返回 true，C2 阶段实现 emplace
    /// - 其他未知消息：返回 false
    pub fn handle_message(
        &mut self,
        id: u32,
        reader: &mut BinaryBufferReader,
        out_result: &mut *mut c_void,
    ) -> bool {
        *out_result = std::ptr::null_mut();

        if self.current_frame.is_null() {
            tracing::error!("handle_message: current_frame is null (id={:x})", id);
            return false;
        }
        let frame = unsafe { &mut *self.current_frame };

        match id {
            // ===== 简单 emplace 类：读取结构体并 push 到对应 vector =====

            // Dig (0x31a728a2) → dig_points
            MSG_DIG => {
                if let Some(val) = unsafe { read_struct::<DigPoint>(reader) } {
                    frame.dig_points.push(val);
                    true
                } else { false }
            }

            // ModifyCell (0xB558693C) → cell_modifications
            MSG_MODIFY_CELL => {
                if let Some(val) = unsafe { read_struct::<CellModification>(reader) } {
                    frame.cell_modifications.push(val);
                    true
                } else { false }
            }

            // SetInsulationValue (0xCAB2C9DF) → set_insulation_values
            MSG_SET_INSULATION => {
                if let Some(val) = unsafe { read_struct::<SetCellFloatValue>(reader) } {
                    frame.set_insulation_values.push(val);
                    true
                } else { false }
            }

            // SetStrengthValue (0x5F078F8E) → set_strength_values
            MSG_SET_STRENGTH => {
                if let Some(val) = unsafe { read_struct::<SetCellFloatValue>(reader) } {
                    frame.set_strength_values.push(val);
                    true
                } else { false }
            }

            // ChangeCellProperties (0xE40D6F65) → set_cell_properties
            MSG_CHANGE_CELL_PROPS => {
                if let Some(val) = unsafe { read_struct::<CellPropertiesChange>(reader) } {
                    frame.set_cell_properties.push(val);
                    true
                } else { false }
            }

            // MassConsumption (0x671B6E97) → mass_consumption_messages
            MSG_MASS_CONSUMPTION => {
                if let Some(val) = unsafe { read_struct::<MassConsumption>(reader) } {
                    frame.mass_consumption_messages.push(val);
                    true
                } else { false }
            }

            // MassEmission (0x2F8A9D6B) → mass_emission_messages
            MSG_MASS_EMISSION => {
                if let Some(val) = unsafe { read_struct::<MassEmission>(reader) } {
                    frame.mass_emission_messages.push(val);
                    true
                } else { false }
            }

            // ModifyCellWorldZone (0xE5C7B282) → cell_world_zone_modifications
            MSG_MODIFY_CELL_WORLD_ZONE => {
                if let Some(val) = unsafe { read_struct::<CellWorldZoneModification>(reader) } {
                    frame.cell_world_zone_modifications.push(val);
                    true
                } else { false }
            }

            // ===== C2 桩 emplace 类：emplace 到 u8 vector（按消息大小追加字节）=====
            // 这些消息在 C1 阶段不真正处理，但需要 emplace 到 vector 以保持 reader 对齐。
            // C2 阶段会替换为真实结构体类型。

            // ModifyCellEnergy (0x30C69504) → cell_energy_modifications（C2 桩，16B）
            MSG_MODIFY_CELL_ENERGY => {
                emplace_bytes(&mut frame.cell_energy_modifications, reader, 16)
            }

            // ConsumeDisease (0xC3217E40) → consume_disease（16B：gameCell + callbackIdx +
            // percentToConsume(f32) + maxToConsume；C# SimMessages.ConsumeDiseaseMessage；
            // 原版 ProcessConsumeDisease stride +4。2026-08-05 修正 12B→16B）
            MSG_CONSUME_DISEASE => {
                emplace_bytes(&mut frame.consume_disease, reader, 16)
            }

            // CellDiseaseModification (0x9174F696) → cell_disease_modifications（C2 桩，12B）
            MSG_CELL_DISEASE_MOD => {
                emplace_bytes(&mut frame.cell_disease_modifications, reader, 12)
            }

            // CellRadiationModification (0x8DE9DD2B) → cell_radiation_modifications（C2 桩，12B）
            MSG_CELL_RADIATION_MOD => {
                emplace_bytes(&mut frame.cell_radiation_modifications, reader, 12)
            }

            // RadiationParamsModification (0x167A4883) → radiation_params_modifications（8B：
            // {int type, float value}，原版/C# 结构一致；2026-08-04 修正 12B→8B）
            MSG_RADIATION_PARAMS_MOD => {
                emplace_bytes(&mut frame.radiation_params_modifications, reader, 8)
            }

            // ===== ElementConsumer 消息（2026-08-02 实现，原版 ProcessFrame L116710-116790）=====
            // 消息体：Add 12B / SetData 12B / Remove 8B（与 C# SimMessages 一致）。
            // 逐帧由 process_element_consumer_messages 处理（Register/Modify/Unregister）。
            MSG_ADD_ELEMENT_CONSUMER => {
                emplace_bytes(&mut frame.element_consumer_messages.adds, reader, 12)
            }
            MSG_SET_ELEMENT_CONSUMER_DATA => {
                emplace_bytes(&mut frame.element_consumer_messages.modifies, reader, 12)
            }
            MSG_REMOVE_ELEMENT_CONSUMER => {
                emplace_bytes(&mut frame.element_consumer_messages.removes, reader, 8)
            }

            // ModifyBuildingEnergy (0xAF9C5B96) → modify_building_energy_messages（C2 桩，16B）
            MSG_MODIFY_BUILDING_ENERGY => {
                emplace_bytes(&mut frame.modify_building_energy_messages, reader, 16)
            }

            // ===== BuildingToBuildingHeatExchange 消息（2026-08-03 实现，原版 ProcessFrame L116540-116650）=====
            // RegisterBuildingToBuildingHeatExchange → adds 8B {callbackIdx, heatExchange_handle}
            MSG_ADD_BUILDING_TO_BUILDING_HE => {
                emplace_bytes(&mut frame.building_to_building_heat_exchange_messages.adds, reader, 8)
            }
            // RemoveBuildingInContactFromBuildingToBuildingHeatExchange → modifies 8B {self_handle, buildingInContact}
            MSG_REMOVE_BUILDING_IN_CONTACT => {
                emplace_bytes(&mut frame.building_to_building_heat_exchange_messages.modifies, reader, 8)
            }
            // AddInContactBuildingToBuildingToBuildingHeatExchange → add_building_in_contact_messages 12B
            // {self_handle, buildingInContact, cellsInContact}（原 8B 修正为 12B）
            MSG_ADD_BUILDING_IN_CONTACT => {
                emplace_bytes(&mut frame.add_building_in_contact_messages, reader, 12)
            }
            // RemoveBuildingToBuildingHeatExchange → removes 8B {callbackIdx, handle}
            MSG_REMOVE_BUILDING_TO_BUILDING_HE => {
                emplace_bytes(&mut frame.building_to_building_heat_exchange_messages.removes, reader, 8)
            }

            // MoveElementChunk → move_element_chunk_messages（原版 ProcessFrame stride +2 = 8B）
            MSG_MOVE_ELEMENT_CHUNK => {
                emplace_bytes(&mut frame.move_element_chunk_messages, reader, 8)
            }

            // ModifyElementChunkEnergy → modify_element_chunk_energy_messages（原版 stride +2 = 8B）
            MSG_MODIFY_ELEMENT_CHUNK_ENERGY => {
                emplace_bytes(&mut frame.modify_element_chunk_energy_messages, reader, 8)
            }

            // ModifyChunkTemperatureAdjuster → modify_element_chunk_adjuster_messages（C2 桩，16B）
            MSG_MODIFY_CHUNK_TEMP_ADJUSTER => {
                emplace_bytes(&mut frame.modify_element_chunk_adjuster_messages, reader, 16)
            }

            // AddElementChunk → adds（原版 stride +0x20 = 32B）
            MSG_ADD_ELEMENT_CHUNK => {
                emplace_bytes(&mut frame.element_chunk_messages.adds, reader, 32)
            }

            // SetElementChunkData（= Modify）→ modifies（原版 stride +3 = 12B）
            MSG_SET_ELEMENT_CHUNK_DATA => {
                emplace_bytes(&mut frame.element_chunk_messages.modifies, reader, 12)
            }

            // RemoveElementChunk → removes（原版 stride +2 = 8B）
            MSG_REMOVE_ELEMENT_CHUNK => {
                emplace_bytes(&mut frame.element_chunk_messages.removes, reader, 8)
            }

            // ModifyBackwallData → modify_backwall_data_messages（16B：
            // gameCell(i32) + elemIdx(u16)+pad(u16) + mass(f32) + temperature(f32)）
            // 2026-08-04 修正：此前 12B 尺寸错误（原版步进 4×i32，06_process_messages.c L871-914）。
            MSG_MODIFY_BACKWALL_DATA => {
                emplace_bytes(&mut frame.modify_backwall_data_messages, reader, 16)
            }

            // AddBuildingHeatExchange (0x67A75D28) -> adds, 44B
            MSG_ADD_BUILDING_HE => {
                emplace_bytes(&mut frame.building_heat_exchange_messages.adds, reader, 44)
            }

            // ModifyBuildingHeatExchange (0x6C5C80A1) -> modifies, 44B
            MSG_MODIFY_BUILDING_HE => {
                emplace_bytes(&mut frame.building_heat_exchange_messages.modifies, reader, 44)
            }

            // RemoveBuildingHeatExchange (0xE4D1E06B) -> removes, 8B
            MSG_REMOVE_BUILDING_HE => {
                emplace_bytes(&mut frame.building_heat_exchange_messages.removes, reader, 8)
            }

            // ===== ElementEmitter 消息（adds 16B / modifies 32B / removes 8B）=====
            MSG_ADD_ELEMENT_EMITTER => {
                emplace_bytes(&mut frame.element_emitter_messages.adds, reader, 16)
            }
            MSG_MODIFY_ELEMENT_EMITTER => {
                emplace_bytes(&mut frame.element_emitter_messages.modifies, reader, 32)
            }
            MSG_REMOVE_ELEMENT_EMITTER => {
                emplace_bytes(&mut frame.element_emitter_messages.removes, reader, 8)
            }

            // ===== DiseaseEmitter 消息（阶段 3：adds 4B / modifies 20B / removes 8B）=====
            MSG_ADD_DISEASE_EMITTER => {
                emplace_bytes(&mut frame.disease_emitter_messages.adds, reader, 4)
            }
            MSG_MODIFY_DISEASE_EMITTER => {
                emplace_bytes(&mut frame.disease_emitter_messages.modifies, reader, 20)
            }
            MSG_REMOVE_DISEASE_EMITTER => {
                emplace_bytes(&mut frame.disease_emitter_messages.removes, reader, 8)
            }

            // ===== DiseaseConsumer 消息（阶段 3：adds 12B / modifies 20B / removes 8B）=====
            // C# 无发送方（仅 SimMessageHashes 枚举），尺寸按原版 frame manager
            // stride 确定（adds +3 int、modifies +0x14、removes +2 int）。
            MSG_ADD_DISEASE_CONSUMER => {
                emplace_bytes(&mut frame.disease_consumer_messages.adds, reader, 12)
            }
            MSG_MODIFY_DISEASE_CONSUMER => {
                emplace_bytes(&mut frame.disease_consumer_messages.modifies, reader, 20)
            }
            MSG_REMOVE_DISEASE_CONSUMER => {
                emplace_bytes(&mut frame.disease_consumer_messages.removes, reader, 8)
            }

              // ===== RadiationEmitter 消息（2026-08-04 阶段 B：adds 36B / modifies 40B / removes 8B）=====
              // AddRadiationEmitter (0xA649D34E?) → adds 36B {callbackIdx, cell, radiusX(s16),
              //   radiusY(s16), emitRads, emitRate, emitSpeed, emitDirection, emitAngle, emitType}
              MSG_ADD_RADIATION_EMITTER => {
                  emplace_bytes(&mut frame.radiation_emitter_messages.adds, reader, 36)
              }
              // ModifyRadiationEmitter → modifies 40B {handle, cell, callbackIdx, ...同 Add 尾部}
              MSG_MODIFY_RADIATION_EMITTER => {
                  emplace_bytes(&mut frame.radiation_emitter_messages.modifies, reader, 40)
              }
              // RemoveRadiationEmitter → removes 8B {handle, callbackIdx}
              MSG_REMOVE_RADIATION_EMITTER => {
                  emplace_bytes(&mut frame.radiation_emitter_messages.removes, reader, 8)
              }

            // ===== 特殊处理类：NewGameFrame =====
            // 对照源码 08_sim_frame_manager.c L594-636。
            MSG_NEW_GAME_FRAME => {
                self.handle_new_game_frame(reader);
                true
            }

            // ===== 特殊处理类：SetDebugProperties =====
            // 对照源码 08_sim_frame_manager.c L417-428。
            MSG_SET_DEBUG_PROPERTIES => {
                self.handle_set_debug_properties(reader);
                true
            }

            // ===== 其他消息：未处理，交由 handler 表 =====
            _ => {
                tracing::trace!("handle_message: id={:x} not handled, fall through to handler table", id);
                false
            }
        }
    }

    /// 处理 NewGameFrame 消息。
    ///
    /// 对照源码 08_sim_frame_manager.c L594-636。
    /// 1. currentFrame.elapsedSeconds = *(float*)param_3
    /// 2. 重置 activeRegions.end = activeRegions.begin
    /// 3. 遍历 N 个 NewGameFrame 结构体（每个 28B），提取 ActiveRegion 并 push
    ///    - 坐标 +1 转换为内部坐标（添加边界偏移）
    /// 4. 调用 new_frame() 准备下一帧
    fn handle_new_game_frame(&mut self, reader: &mut BinaryBufferReader) {
        if self.current_frame.is_null() {
            return;
        }

        // C# 发送的是 N 个 Sim.NewGameFrame（每个 28B）。
        // 源码取第一个的 elapsedSeconds 作为帧时间。
        // 每个结构体：elapsedSeconds(4) + minX(4) + minY(4) + maxX(4) + maxY(4) + sunlight(4) + cosmicRad(4) = 28B

        // 2026-08-07 修正：用剩余字节数而非 buffer_size（批量/多消息场景更稳妥；
        // 单消息时二者等价）。
        let frame_count = (reader.remaining() as usize) / 28;
        if frame_count == 0 {
            tracing::warn!(
                "NewGameFrame: buffer too small (remaining={})",
                reader.remaining()
            );
            return;
        }

        // 读取所有 NewGameFrame 结构体
        let mut regions: Vec<ActiveRegion> = Vec::with_capacity(frame_count);
        let mut elapsed_seconds = 0.0f32;

        for i in 0..frame_count {
            // 读取 28 字节作为 NewGameFrame
            let bytes = match reader.read_bytes(28) {
                Ok(b) => b,
                Err(_) => {
                    tracing::warn!("NewGameFrame: failed to read frame {}", i);
                    return;
                }
            };
            // 解析字段
            let elapsed = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let min_x = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            let min_y = i32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
            let max_x = i32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
            let max_y = i32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
            let sunlight = f32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            let cosmic_rad = f32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);

            if i == 0 {
                elapsed_seconds = elapsed;
            }

            // 坐标 +1 转换为内部坐标（对照源码 L607-610）
            regions.push(ActiveRegion {
                min_x: min_x + 1,
                min_y: min_y + 1,
                max_x: max_x + 1,
                max_y: max_y + 1,
                current_sunlight_intensity: sunlight,
                current_cosmic_radiation_intensity: cosmic_rad,
            });
        }

        // 写入 currentFrame.elapsedSeconds
        unsafe { (*self.current_frame).elapsed_seconds = elapsed_seconds; }

        // 重置 active_regions 并 push 所有 region
        self.active_regions.clear_keep_capacity();
        for region in regions {
            self.active_regions.push(region);
        }

        // 保存 elapsed_seconds 到 SimFrameManager.active_region（用于后续帧处理）
        self.active_region.elapsed_seconds = elapsed_seconds;

        // 调用 new_frame() 准备下一帧（对照源码 L635）
        // 注意：源码在 NewGameFrame 末尾调用 NewFrame，将 currentFrame 入队 queuedFrames
        self.new_frame_and_queue();
    }

    /// 处理 SetDebugProperties 消息。
    ///
    /// 对照源码 08_sim_frame_manager.c L417-428。
    /// 消息体 12 字节：
    /// - 0-3: buildingTemperatureScale (int32 → float)
    /// - 4-7: buildingToBuildingTemperatureScale (int32 → float)
    /// - 8-11: isDebugEditing(1B) + pad[3](3B)
    fn handle_set_debug_properties(&mut self, reader: &mut BinaryBufferReader) {
        if self.current_frame.is_null() {
            return;
        }

        // 读取 12 字节
        let bytes = match reader.read_bytes(12) {
            Ok(b) => b,
            Err(_) => {
                tracing::warn!("SetDebugProperties: failed to read 12 bytes");
                return;
            }
        };

        // 源码：buildingTemperatureScale = (float)(int)uVar3
        // 即把 int32 位模式直接 reinterpret 为 float
        let bts_bits = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let btbts_bits = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let is_debug_editing = bytes[8] != 0;
        let pad = [bytes[9] != 0, bytes[10] != 0, bytes[11] != 0];

        unsafe {
            (*self.current_frame).debug_properties = DebugProperties {
                building_temperature_scale: f32::from_bits(bts_bits as u32),
                building_to_building_temperature_scale: f32::from_bits(btbts_bits as u32),
                is_debug_editing,
                pad,
            };
        }
    }

    /// 创建新帧并入队（对照源码 NewGameFrame L635 的 NewFrame 调用）。
    ///
    /// 源码 NewFrame 会重置 currentFrame 的所有 vector，
    /// 但 HandleMessage 在 NewGameFrame 末尾调用 NewFrame 后，
    /// currentFrame 指向新帧，旧帧需要入队 queuedFrames。
    ///
    /// **注意**：源码的 NewFrame 只是重置 currentFrame，不自动入队。
    /// 入队逻辑在 SimFrameManager 的其他方法中（如 Sim 线程主循环）。
    /// 此处只调用 new_frame()，入队由 Sim 线程主循环处理。
    fn new_frame_and_queue(&mut self) {
        // 对照原版 NewFrame（08_sim_frame_manager.c L1128-1169）：
        // 入队 + 取新帧在**同一把 frame_mutex 内原子完成**。
        // 此前入队在锁外 push_unchecked → 与 sim 线程 begin_frame_processing
        // （持锁遍历/清空 queued_frames）数据竞争 → 帧被撕裂/重复入队 →
        // 消息被处理两次 → 重复回显（3× 突发下 667 条重复交付）。
        // 另补原版 30 帧队列上限背压：队列满时旧帧丢回帧池（主动丢帧）。
        unsafe { self.frame_mutex.lock_raw(); }
        let old_frame = self.current_frame;
        if !old_frame.is_null() {
            if self.queued_frames.len() < 30 {
                self.queued_frames.push_unchecked(old_frame);
            } else {
                self.frame_pool.push_unchecked(old_frame);
            }
        }
        self.new_frame_locked();
        unsafe { self.frame_mutex.unlock_raw(); }
    }

    /// BeginFrameProcessing：开始帧处理。
    ///
    /// 对照源码 08_sim_frame_manager.c L182-243。
    /// 1. 锁 frame_mutex
    /// 2. processedFrames → framePool（回收已处理帧），清空 processedFrames
    /// 3. queuedFrames → activeFrames（待处理帧），清空 queuedFrames
    /// 4. numFramesProcessed = 0
    /// 5. 返回 activeFrames 大小
    pub fn begin_frame_processing(&mut self) -> i32 {
        let _guard = self.frame_mutex.lock();

        // 2. processedFrames → framePool（回收）
        let processed_count = self.processed_frames.len();
        for i in 0..processed_count {
            let frame_ptr = self.processed_frames.as_slice()[i];
            self.frame_pool.push_unchecked(frame_ptr);
        }
        self.processed_frames.clear_keep_capacity();

        // 3. queuedFrames → activeFrames
        let queued_count = self.queued_frames.len();
        for i in 0..queued_count {
            let frame_ptr = self.queued_frames.as_slice()[i];
            self.active_frames.push_unchecked(frame_ptr);
        }
        self.queued_frames.clear_keep_capacity();

        // 4. numFramesProcessed = 0
        self.num_frames_processed = 0;

        // 5. 返回 activeFrames 大小
        self.active_frames.len() as i32
    }

    /// EndFrameProcessing：结束帧处理。
    ///
    /// 对照源码 08_sim_frame_manager.c L247-348。
    /// 1. 锁 frame_mutex
    /// 2. activeFrames[0..numFramesProcessed] → processedFrames
    /// 3. activeFrames[numFramesProcessed..] 前移到 activeFrames[0..]
    /// 4. 解锁
    pub fn end_frame_processing(&mut self) {
        let _guard = self.frame_mutex.lock();

        let processed = self.num_frames_processed as usize;
        let active_len = self.active_frames.len();

        // 2. activeFrames[0..processed] → processedFrames
        for i in 0..processed {
            if i < active_len {
                let frame_ptr = self.active_frames.as_slice()[i];
                self.processed_frames.push_unchecked(frame_ptr);
            }
        }

        // 3. activeFrames[processed..] 前移
        // 将剩余未处理帧从 active_frames[processed..] 移到 active_frames[0..]
        if processed > 0 && processed < active_len {
            let remaining = active_len - processed;
            for i in 0..remaining {
                let frame_ptr = self.active_frames.as_slice()[processed + i];
                // 直接覆盖前 processed 个位置
                unsafe {
                    std::ptr::write(self.active_frames.begin.add(i), frame_ptr);
                }
            }
            // 更新 end 指针
            self.active_frames.end = unsafe { self.active_frames.begin.add(remaining) };
        } else if processed >= active_len {
            // 所有帧都已处理，清空 active_frames
            self.active_frames.clear_keep_capacity();
        }
    }

    /// ProcessNextFrame：处理下一帧。
    ///
    /// 对照源码 08_sim_frame_manager.c L1903-1929。
    /// 1. 检查 num_frames_processed < active_frames.len()
    /// 2. 取 active_frames[num_frames_processed]
    /// 3. 调用 ProcessFrame（C1：通过 frame_processor 模块）
    /// 4. num_frames_processed++
    /// 5. 返回该帧的 elapsed_seconds
    pub fn process_next_frame(
        &mut self,
        sim_data: &mut crate::a_framework::sim_data::SimData,
    ) -> f32 {
        let idx = self.num_frames_processed as usize;
        if idx >= self.active_frames.len() {
            tracing::warn!("ProcessNextFrame: no active frames to process");
            return 0.0;
        }

        let frame_ptr = self.active_frames.as_slice()[idx];
        if frame_ptr.is_null() {
            tracing::error!("ProcessNextFrame: frame pointer is null at index {}", idx);
            self.num_frames_processed += 1;
            return 0.0;
        }

        let frame = unsafe { &mut *frame_ptr };

        // 调用 ProcessFrame（C1 阶段通过 frame_processor 模块实现）
        crate::c_simulation::frame_processor::process_frame(frame, sim_data);

        // 原版 ProcessFrame 末尾（L117396-117400）：SimFrameManager::activeRegions
        // → SimData::activeRegions（_Assign_range）。供 update_data 按活动区域裁剪。
        // 2026-08-04 D1：此前未同步 → 物理层读不到区域数据。
        sim_data.active_regions.clear_keep_capacity();
        for r in self.active_regions.as_slice() {
            sim_data.active_regions.push(*r);
        }

        self.num_frames_processed += 1;
        frame.elapsed_seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn new_game_frame_size_is_28() {
        assert_eq!(size_of::<NewGameFrame>(), 28);
    }

    #[test]
    fn sim_frame_manager_size_in_expected_range() {
        let actual = size_of::<SimFrameManager>();
        assert!(actual >= 280 && actual <= 296, "SimFrameManager size = {}", actual);
    }

    #[test]
    fn new_zeroed_creates_current_frame() {
        let mgr = SimFrameManager::new_zeroed();
        assert!(!mgr.current_frame.is_null(), "current_frame should be non-null after new_zeroed");
        // 清理
        unsafe { let _ = Box::from_raw(mgr.current_frame); }
    }

    #[test]
    fn handle_message_returns_false_for_unknown_id() {
        let mut mgr = SimFrameManager::new_zeroed();
        let data = [0u8; 4];
        let mut reader = BinaryBufferReader::new(&data);
        let mut result: *mut c_void = std::ptr::null_mut();

        // 未知 ID 返回 false（交由 handler 表）
        assert!(!mgr.handle_message(0xDEADBEEF, &mut reader, &mut result));
        assert!(result.is_null());

        // 清理（new_zeroed 创建了一个 current_frame，但 handle_message 可能创建新的）
        unsafe {
            if !mgr.current_frame.is_null() {
                let _ = Box::from_raw(mgr.current_frame);
            }
            // 清理 queued_frames
            let qf = mgr.queued_frames.as_slice();
            for &p in qf {
                if !p.is_null() {
                    let _ = Box::from_raw(p);
                }
            }
        }
    }

    #[test]
    fn handle_message_dig_emplaces_to_dig_points() {
        let mut mgr = SimFrameManager::new_zeroed();
        // DigPoint: game_cell(4) + callback_idx(4) + skip_event(1) + backwall(1) + pad(2) = 12B
        let data = [0x01, 0x00, 0x00, 0x00, // game_cell = 1
                    0xFF, 0xFF, 0xFF, 0xFF, // callback_idx = -1
                    0x00, 0x00, 0x00, 0x00]; // skip_event=0, backwall=0, pad
        let mut reader = BinaryBufferReader::new(&data);
        let mut result: *mut c_void = std::ptr::null_mut();

        assert!(mgr.handle_message(MessageType::Dig as u32, &mut reader, &mut result));
        assert!(result.is_null());

        let frame = unsafe { &*mgr.current_frame };
        assert_eq!(frame.dig_points.len(), 1);
        assert_eq!(frame.dig_points.get(0).game_cell, 1);
        assert_eq!(frame.dig_points.get(0).callback_idx, -1);

        // 清理
        unsafe { let _ = Box::from_raw(mgr.current_frame); }
    }

    #[test]
    fn handle_message_modify_cell_emplaces() {
        let mut mgr = SimFrameManager::new_zeroed();
        // CellModification: 28B
        let mut data = vec![0u8; 28];
        data[0..4].copy_from_slice(&100i32.to_le_bytes()); // game_cell = 100
        data[4..8].copy_from_slice(&5i32.to_le_bytes()); // callback_idx = 5
        let mut reader = BinaryBufferReader::new(&data);
        let mut result: *mut c_void = std::ptr::null_mut();

        assert!(mgr.handle_message(MessageType::ModifyCell as u32, &mut reader, &mut result));

        let frame = unsafe { &*mgr.current_frame };
        assert_eq!(frame.cell_modifications.len(), 1);
        assert_eq!(frame.cell_modifications.get(0).game_cell, 100);

        unsafe { let _ = Box::from_raw(mgr.current_frame); }
    }

    #[test]
    fn handle_message_component_messages_returns_true() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut result: *mut c_void = std::ptr::null_mut();

        // AddBuildingHeatExchange 44B / Modify 44B / Remove 8B 已真实 emplace
        let data44 = [0u8; 44];
        let mut r44 = BinaryBufferReader::new(&data44);
        assert!(mgr.handle_message(MessageType::AddBuildingHeatExchange as u32, &mut r44, &mut result));
        let data44b = [0u8; 44];
        let mut r44b = BinaryBufferReader::new(&data44b);
        assert!(mgr.handle_message(MessageType::ModifyBuildingHeatExchange as u32, &mut r44b, &mut result));
        let data8 = [0u8; 8];
        let mut r8 = BinaryBufferReader::new(&data8);
        assert!(mgr.handle_message(MessageType::RemoveBuildingHeatExchange as u32, &mut r8, &mut result));

        // ElementChunk 已真实 emplace（Add 32B）；其余 ComponentMessages 仍是 C2 桩
        let data32 = [0u8; 32];
        let mut r32 = BinaryBufferReader::new(&data32);
        assert!(mgr.handle_message(MessageType::AddElementChunk as u32, &mut r32, &mut result));
        let data = [0u8; 16];
        let mut reader = BinaryBufferReader::new(&data);
        assert!(mgr.handle_message(MessageType::AddDiseaseEmitter as u32, &mut reader, &mut result));

        unsafe { let _ = Box::from_raw(mgr.current_frame); }
    }

    #[test]
    fn handle_message_consume_disease_emplaces_16b() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut result: *mut c_void = std::ptr::null_mut();
        // 16B：gameCell=5, callbackIdx=7, percent=0.5, maxToConsume=100
        let mut data = vec![0u8; 16];
        data[0..4].copy_from_slice(&5i32.to_le_bytes());
        data[4..8].copy_from_slice(&7i32.to_le_bytes());
        data[8..12].copy_from_slice(&0.5f32.to_le_bytes());
        data[12..16].copy_from_slice(&100i32.to_le_bytes());
        let mut reader = BinaryBufferReader::new(&data);
        assert!(mgr.handle_message(MessageType::ConsumeDisease as u32, &mut reader, &mut result));
        unsafe {
            assert_eq!(
                (*mgr.current_frame).consume_disease.len(),
                16,
                "ConsumeDisease 消息 16B（gameCell+callbackIdx+percent+max）"
            );
            let _ = Box::from_raw(mgr.current_frame);
        }
    }

    #[test]
    fn handle_message_disease_emitter_messages_emplace_sizes() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut result: *mut c_void = std::ptr::null_mut();
        for (id, n) in [
            (MessageType::AddDiseaseEmitter as u32, 4usize),
            (MessageType::ModifyDiseaseEmitter as u32, 20usize),
            (MessageType::RemoveDiseaseEmitter as u32, 8usize),
        ] {
            let data = vec![0u8; n];
            let mut r = BinaryBufferReader::new(&data);
            assert!(mgr.handle_message(id, &mut r, &mut result));
        }
        unsafe {
            let f = &*mgr.current_frame;
            assert_eq!(f.disease_emitter_messages.adds.len(), 4, "AddDiseaseEmitter 4B");
            assert_eq!(f.disease_emitter_messages.modifies.len(), 20, "ModifyDiseaseEmitter 20B");
            assert_eq!(f.disease_emitter_messages.removes.len(), 8, "RemoveDiseaseEmitter 8B");
            let _ = Box::from_raw(mgr.current_frame);
        }
    }

    #[test]
    fn handle_message_disease_consumer_messages_emplace_sizes() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut result: *mut c_void = std::ptr::null_mut();
        for (id, n) in [
            (MessageType::AddDiseaseConsumer as u32, 12usize),
            (MessageType::ModifyDiseaseConsumer as u32, 20usize),
            (MessageType::RemoveDiseaseConsumer as u32, 8usize),
        ] {
            let data = vec![0u8; n];
            let mut r = BinaryBufferReader::new(&data);
            assert!(mgr.handle_message(id, &mut r, &mut result));
        }
        unsafe {
            let f = &*mgr.current_frame;
            assert_eq!(f.disease_consumer_messages.adds.len(), 12, "AddDiseaseConsumer 12B");
            assert_eq!(f.disease_consumer_messages.modifies.len(), 20, "ModifyDiseaseConsumer 20B");
            assert_eq!(f.disease_consumer_messages.removes.len(), 8, "RemoveDiseaseConsumer 8B");
            let _ = Box::from_raw(mgr.current_frame);
        }
    }

    #[test]
    fn handle_message_new_game_frame_parses_regions() {
        let mut mgr = SimFrameManager::new_zeroed();
        // 构造 2 个 NewGameFrame（每个 28B）
        let mut data = vec![0u8; 56];
        // 第一个 frame: elapsed=0.5, min=(0,0), max=(10,10), sunlight=1.0, cosmic=0.5
        data[0..4].copy_from_slice(&0.5f32.to_le_bytes());
        data[4..8].copy_from_slice(&0i32.to_le_bytes());
        data[8..12].copy_from_slice(&0i32.to_le_bytes());
        data[12..16].copy_from_slice(&10i32.to_le_bytes());
        data[16..20].copy_from_slice(&10i32.to_le_bytes());
        data[20..24].copy_from_slice(&1.0f32.to_le_bytes());
        data[24..28].copy_from_slice(&0.5f32.to_le_bytes());
        // 第二个 frame: elapsed=0.5, min=(20,20), max=(30,30)
        data[28..32].copy_from_slice(&0.5f32.to_le_bytes());
        data[32..36].copy_from_slice(&20i32.to_le_bytes());
        data[36..40].copy_from_slice(&20i32.to_le_bytes());
        data[40..44].copy_from_slice(&30i32.to_le_bytes());
        data[44..48].copy_from_slice(&30i32.to_le_bytes());

        let mut reader = BinaryBufferReader::new(&data);
        let mut result: *mut c_void = std::ptr::null_mut();

        assert!(mgr.handle_message(MessageType::SimFrameManager_NewGameFrame as u32, &mut reader, &mut result));

        // 验证 active_regions 有 2 个（坐标 +1）
        assert_eq!(mgr.active_regions.len(), 2);
        let r0 = mgr.active_regions.get(0);
        assert_eq!(r0.min_x, 1); // 0 + 1
        assert_eq!(r0.max_x, 11); // 10 + 1
        let r1 = mgr.active_regions.get(1);
        assert_eq!(r1.min_x, 21); // 20 + 1

        // 验证 queued_frames 有 1 个（旧 current_frame 入队）
        assert_eq!(mgr.queued_frames.len(), 1);

        // 清理：queued_frames 中的旧帧 + 新的 current_frame
        unsafe {
            let old = mgr.queued_frames.as_slice()[0];
            if !old.is_null() {
                let _ = Box::from_raw(old);
            }
            if !mgr.current_frame.is_null() {
                let _ = Box::from_raw(mgr.current_frame);
            }
        }
    }

    #[test]
    fn handle_message_set_debug_properties_writes_fields() {
        let mut mgr = SimFrameManager::new_zeroed();
        // 12 字节: bts(int32) + btbts(int32) + is_debug(1) + pad(3)
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(&1065353216i32.to_le_bytes()); // 1.0f 的 bits
        data[4..8].copy_from_slice(&0i32.to_le_bytes());
        data[8] = 1; // is_debug_editing = true
        let mut reader = BinaryBufferReader::new(&data);
        let mut result: *mut c_void = std::ptr::null_mut();

        assert!(mgr.handle_message(MessageType::SetDebugProperties as u32, &mut reader, &mut result));

        let frame = unsafe { &*mgr.current_frame };
        assert_eq!(frame.debug_properties.building_temperature_scale, 1.0);
        assert!(frame.debug_properties.is_debug_editing);

        unsafe { let _ = Box::from_raw(mgr.current_frame); }
    }

    #[test]
    fn handle_message_element_chunk_messages_route_to_buffers() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut result: *mut c_void = std::ptr::null_mut();
        // AddElementChunk（32B）
        let mut add = vec![0u8; 32];
        add[0..4].copy_from_slice(&5i32.to_le_bytes());
        add[4..8].copy_from_slice(&7i32.to_le_bytes());
        add[8..12].copy_from_slice(&100f32.to_le_bytes());
        add[12..16].copy_from_slice(&300f32.to_le_bytes());
        add[16..20].copy_from_slice(&2f32.to_le_bytes());
        add[20..24].copy_from_slice(&0.5f32.to_le_bytes());
        add[24..28].copy_from_slice(&0.5f32.to_le_bytes());
        add[28..30].copy_from_slice(&5u16.to_le_bytes());
        let mut reader = BinaryBufferReader::new(&add);
        assert!(mgr.handle_message(MessageType::AddElementChunk as u32, &mut reader, &mut result));
        let frame = unsafe { &*mgr.current_frame };
        assert_eq!(frame.element_chunk_messages.adds.len(), 32);

        // MoveElementChunk（8B，不是 16B）
        let mut mv = vec![0u8; 8];
        mv[0..4].copy_from_slice(&1i32.to_le_bytes());
        mv[4..8].copy_from_slice(&9i32.to_le_bytes());
        let mut reader = BinaryBufferReader::new(&mv);
        assert!(mgr.handle_message(MessageType::MoveElementChunk as u32, &mut reader, &mut result));
        assert_eq!(frame.move_element_chunk_messages.len(), 8);

        // ModifyElementChunkEnergy（8B，不是 16B）
        let mut en = vec![0u8; 8];
        en[0..4].copy_from_slice(&1i32.to_le_bytes());
        en[4..8].copy_from_slice(&200f32.to_le_bytes());
        let mut reader = BinaryBufferReader::new(&en);
        assert!(mgr.handle_message(
            MessageType::ModifyElementChunkEnergy as u32,
            &mut reader,
            &mut result
        ));
        assert_eq!(frame.modify_element_chunk_energy_messages.len(), 8);

        // SetElementChunkData = Modify（12B）→ modifies
        let mut sd = vec![0u8; 12];
        sd[0..4].copy_from_slice(&1i32.to_le_bytes());
        sd[4..8].copy_from_slice(&400f32.to_le_bytes());
        sd[8..12].copy_from_slice(&500f32.to_le_bytes());
        let mut reader = BinaryBufferReader::new(&sd);
        assert!(mgr.handle_message(
            MessageType::SetElementChunkData as u32,
            &mut reader,
            &mut result
        ));
        assert_eq!(frame.element_chunk_messages.modifies.len(), 12);

        // RemoveElementChunk（8B）→ removes
        let mut rm = vec![0u8; 8];
        rm[0..4].copy_from_slice(&1i32.to_le_bytes());
        rm[4..8].copy_from_slice(&7i32.to_le_bytes());
        let mut reader = BinaryBufferReader::new(&rm);
        assert!(mgr.handle_message(
            MessageType::RemoveElementChunk as u32,
            &mut reader,
            &mut result
        ));
        assert_eq!(frame.element_chunk_messages.removes.len(), 8);

        unsafe { let _ = Box::from_raw(mgr.current_frame); }
    }

    #[test]
    fn handle_message_building_to_building_routes() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut result: *mut c_void = std::ptr::null_mut();
        // Register 8B {callbackIdx, heatExchange_handle} → adds
        let mut reg = vec![0u8; 8];
        reg[0..4].copy_from_slice(&(-1i32).to_le_bytes());
        reg[4..8].copy_from_slice(&7i32.to_le_bytes());
        let mut r = BinaryBufferReader::new(&reg);
        assert!(mgr.handle_message(
            MessageType::AddBuildingToBuildingHeatExchange as u32,
            &mut r,
            &mut result
        ));
        assert_eq!(
            unsafe { &*mgr.current_frame }
                .building_to_building_heat_exchange_messages
                .adds
                .len(),
            8
        );
        // AddInContact 12B（原 8B 错误）
        let mut ac = vec![0u8; 12];
        let mut r = BinaryBufferReader::new(&ac);
        assert!(mgr.handle_message(
            MessageType::AddInContactBuildingToBuildingToBuildingHeatExchange as u32,
            &mut r,
            &mut result
        ));
        assert_eq!(
            unsafe { &*mgr.current_frame }.add_building_in_contact_messages.len(),
            12
        );
        // RemoveInContact 8B → modifies
        let mut rc = vec![0u8; 8];
        let mut r = BinaryBufferReader::new(&rc);
        assert!(mgr.handle_message(
            MessageType::RemoveBuildingInContactFromBuildingToBuildingHeatExchange as u32,
            &mut r,
            &mut result
        ));
        assert_eq!(
            unsafe { &*mgr.current_frame }
                .building_to_building_heat_exchange_messages
                .modifies
                .len(),
            8
        );
        // Remove 8B {callbackIdx, handle} → removes
        let mut rm = vec![0u8; 8];
        let mut r = BinaryBufferReader::new(&rm);
        assert!(mgr.handle_message(
            MessageType::RemoveBuildingToBuildingHeatExchange as u32,
            &mut r,
            &mut result
        ));
        assert_eq!(
            unsafe { &*mgr.current_frame }
                .building_to_building_heat_exchange_messages
                .removes
                .len(),
            8
        );
        unsafe { let _ = Box::from_raw(mgr.current_frame); }
    }

    /// SimFrameInfo 大小必须为 0x5b8 = 1464 字节（对照源码 operator_new(0x5b8)）。
    #[test]
    fn sim_frame_info_size_is_0x5b8() {
        assert_eq!(size_of::<SimFrameInfo>(), 0x5b8);
    }

    /// CellWorldZoneModification 大小为 8B（MSVC 默认对齐）。
    #[test]
    fn cell_world_zone_modification_size_is_8() {
        assert_eq!(size_of::<CellWorldZoneModification>(), 8);
    }

    /// ComponentMessages 大小为 96B（3 × 32B）。
    #[test]
    fn component_messages_size_is_96() {
        assert_eq!(size_of::<ComponentMessages>(), 96);
    }

    /// SimFrameInfo::default() 所有 vector 应为空。
    #[test]
    fn sim_frame_info_default_all_empty() {
        let f = SimFrameInfo::default();
        assert_eq!(f.elapsed_seconds, 0.0);
        assert!(f.dig_points.is_empty());
        assert!(f.cell_modifications.is_empty());
        assert!(f.set_insulation_values.is_empty());
        assert!(f.cell_world_zone_modifications.is_empty());
    }
}
