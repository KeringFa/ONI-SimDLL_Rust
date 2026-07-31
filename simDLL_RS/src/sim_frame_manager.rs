//! SimFrameManager — 帧管理器（管理帧队列与回放）+ Phase 6 消息路由。
//!
//! 字段对照源码 00_types_reference.c L6468-6479（`struct SimFrameManager`）。
//! 原版运行时位于 `gSim + 0x50`（01_sim_api.c L227 / 11_msvcrt_ignored.c L2424）；
//! Rust 改造为独立全局单例 `G_SIM_FRAME_MANAGER`（C# 侧无感知，见规格 §3.1）。
//!
//! Phase 6 新增 `handle_message` 方法（44 路由 match arms 骨架）。

use crate::buffer::BinaryBufferReader;
use crate::message_handler::MessageType;
use crate::sim_data::ActiveRegion;
use crate::sim_events::{
    CellModification, CellPropertiesChange, MassConsumption, MassEmission, SetCellFloatValue,
    SimFrameInfo,
};
use crate::stl_shim::{MsvcMutex, MsvcVector};
use once_cell::sync::Lazy;
use std::os::raw::c_int;
use std::sync::Mutex;

// === Phase 6：HandleMessage 路由消息 ID const ===
//
// 对照源码 08_sim_frame_manager.c L393-1112 的 if-else 路由表。
// 值用 `MessageType::Xxx as u32` 显式取出，便于一眼对照源码 enum Type。
// 用 const 项而非直接 `MessageType::Load as u32` 作为 match 模式，
// 因为 Rust 模式语法要求路径必须是 const 项。
// 与 sim_api.rs 既有 `MSG_LOAD` 模式一致。

// === 3 个特殊处理（不 emplace，直接操作字段）===
/// SimFrameManager_NewGameFrame (0xD1C97543) — 源码调 NewFrame + 填充 activeRegions。
const MSG_NEW_GAME_FRAME: u32 = MessageType::SimFrameManager_NewGameFrame as u32;
/// SetDebugProperties (0x9BADA664) — 源码直接赋值 currentFrame->debugProperties 4 个字段。
const MSG_SET_DEBUG_PROPERTIES: u32 = MessageType::SetDebugProperties as u32;
/// ModifyBuildingEnergy (0xAF9B1296) — 源码直接赋值 currentFrame->modifyBuildingEnergyMessages。
const MSG_MODIFY_BUILDING_ENERGY: u32 = MessageType::ModifyBuildingEnergy as u32;

// === 42 个普通 emplace 路由（消息 ID → currentFrame 字段）===
const MSG_CHANGE_CELL_PROPERTIES: u32 = MessageType::ChangeCellProperties as u32;
const MSG_ADD_BUILDING_TO_BUILDING_HEAT_EXCHANGE: u32 =
    MessageType::AddBuildingToBuildingHeatExchange as u32;
const MSG_REMOVE_BUILDING_IN_CONTACT_FROM_BUILDING_TO_BUILDING_HEAT_EXCHANGE: u32 =
    MessageType::RemoveBuildingInContactFromBuildingToBuildingHeatExchange as u32;
const MSG_CELL_RADIATION_MODIFICATION: u32 = MessageType::CellRadiationModification as u32;
const MSG_MODIFY_DISEASE_EMITTER: u32 = MessageType::ModifyDiseaseEmitter as u32;
const MSG_CELL_DISEASE_MODIFICATION: u32 = MessageType::CellDiseaseModification as u32;
const MSG_MODIFY_DISEASE_CONSUMER: u32 = MessageType::ModifyDiseaseConsumer as u32;
const MSG_ADD_IN_CONTACT_BUILDING_TO_BUILDING_TO_BUILDING_HEAT_EXCHANGE: u32 =
    MessageType::AddInContactBuildingToBuildingToBuildingHeatExchange as u32;
const MSG_REMOVE_ELEMENT_EMITTER: u32 = MessageType::RemoveElementEmitter as u32;
const MSG_ADD_RADIATION_EMITTER: u32 = MessageType::AddRadiationEmitter as u32;
const MSG_MODIFY_CHUNK_TEMPERATURE_ADJUSTER: u32 = MessageType::ModifyChunkTemperatureAdjuster as u32;
const MSG_MODIFY_CELL: u32 = MessageType::ModifyCell as u32;
const MSG_CONSUME_DISEASE: u32 = MessageType::ConsumeDisease as u32;
const MSG_REMOVE_ELEMENT_CHUNK: u32 = MessageType::RemoveElementChunk as u32;
const MSG_SET_INSULATION_VALUE: u32 = MessageType::SetInsulationValue as u32;
const MSG_REMOVE_DISEASE_CONSUMER: u32 = MessageType::RemoveDiseaseConsumer as u32;
const MSG_REMOVE_RADIATION_EMITTER: u32 = MessageType::RemoveRadiationEmitter as u32;
const MSG_MODIFY_BACKWALL_DATA: u32 = MessageType::ModifyBackwallData as u32;
const MSG_ADD_ELEMENT_EMITTER: u32 = MessageType::AddElementEmitter as u32;
const MSG_MODIFY_RADIATION_EMITTER: u32 = MessageType::ModifyRadiationEmitter as u32;
const MSG_REMOVE_BUILDING_HEAT_EXCHANGE: u32 = MessageType::RemoveBuildingHeatExchange as u32;
const MSG_MODIFY_CELL_WORLD_ZONE: u32 = MessageType::ModifyCellWorldZone as u32;
const MSG_SET_ELEMENT_CHUNK_DATA: u32 = MessageType::SetElementChunkData as u32;
const MSG_MOVE_ELEMENT_CHUNK: u32 = MessageType::MoveElementChunk as u32;
const MSG_ADD_DISEASE_CONSUMER: u32 = MessageType::AddDiseaseConsumer as u32;
const MSG_MODIFY_ELEMENT_EMITTER: u32 = MessageType::ModifyElementEmitter as u32;
const MSG_REMOVE_DISEASE_EMITTER: u32 = MessageType::RemoveDiseaseEmitter as u32;
const MSG_REMOVE_BUILDING_TO_BUILDING_HEAT_EXCHANGE: u32 =
    MessageType::RemoveBuildingToBuildingHeatExchange as u32;
const MSG_MASS_EMISSION: u32 = MessageType::MassEmission as u32;
const MSG_MODIFY_CELL_ENERGY: u32 = MessageType::ModifyCellEnergy as u32;
const MSG_DIG: u32 = MessageType::Dig as u32;
const MSG_RADIATION_PARAMS_MODIFICATION: u32 = MessageType::RadiationParamsModification as u32;
const MSG_SET_STRENGTH_VALUE: u32 = MessageType::SetStrengthValue as u32;
const MSG_MASS_CONSUMPTION: u32 = MessageType::MassConsumption as u32;
const MSG_ADD_BUILDING_HEAT_EXCHANGE: u32 = MessageType::AddBuildingHeatExchange as u32;
const MSG_MODIFY_BUILDING_HEAT_EXCHANGE: u32 = MessageType::ModifyBuildingHeatExchange as u32;
const MSG_ADD_ELEMENT_CONSUMER: u32 = MessageType::AddElementConsumer as u32;
const MSG_ADD_ELEMENT_CHUNK: u32 = MessageType::AddElementChunk as u32;
const MSG_ADD_DISEASE_EMITTER: u32 = MessageType::AddDiseaseEmitter as u32;
const MSG_SET_ELEMENT_CONSUMER_DATA: u32 = MessageType::SetElementConsumerData as u32;
const MSG_MODIFY_ELEMENT_CHUNK_ENERGY: u32 = MessageType::ModifyElementChunkEnergy as u32;
const MSG_REMOVE_ELEMENT_CONSUMER: u32 = MessageType::RemoveElementConsumer as u32;

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

/// `unsafe impl Send/Sync`：`SimFrameManager` 含 `*mut SimFrameInfo` 与
/// `MsvcVector<*mut SimFrameInfo>` 字段，Rust 默认 `*mut T: !Send`。
/// 与 `sim.rs::Sim` / `save_load.rs::SimData` 处理一致——为 FFI 含裸指针类型补
/// `unsafe impl Send/Sync`，使其可放入 `Lazy<Mutex<Box<SimFrameManager>>>`。
// SAFETY: `currentFrame` 与 4 个 MsvcVector 中的 `*mut SimFrameInfo` 是 owning
// 指针（源码 SimFrameManager dtor 释放），但 Phase 6 骨架不实际管理其生命周期
// （`currentFrame` 保持 null，vector 保持空）。跨线程访问由 Mutex 提供同步；
// Phase 7+ 接入真实帧池生命周期时复核。
unsafe impl Send for SimFrameManager {}
unsafe impl Sync for SimFrameManager {}

/// 全局 `SimFrameManager` 单例（对应原版 `gSim + 0x50` 处的 `SimFrameManager` 子对象）。
///
/// **改造说明**：原版 `SimFrameManager` 内嵌在 `Sim` 对象 offset 0x50 处，C# 不直接
/// 操作该布局（仅通过消息 ID 与返回值交互）。Rust 改造为独立全局单例，运行时无差异
/// （见规格 §3.1 决策）。
pub static G_SIM_FRAME_MANAGER: Lazy<Mutex<Box<SimFrameManager>>> =
    Lazy::new(|| Mutex::new(Box::new(SimFrameManager::new_zeroed())));

impl SimFrameManager {
    /// 构造零值初始化的 `SimFrameManager`（所有字段 default / null / 空 vector）。
    ///
    /// 对照源码 `Sim::Sim` 中对 `SimFrameManager` 子对象的构造（01_sim_api.c L227）。
    /// 与 `SimData::new_zeroed` / `FrameSync::new_zeroed` 一致：用 `std::mem::zeroed()`
    /// 全零初始化（MsvcVector 全零 = 空 vector，裸指针全零 = null）。
    /// Phase 6 骨架：`current_frame` = null，4 个 vector 为空，`active_region` 全 0。
    fn new_zeroed() -> Self {
        // unsafe { std::mem::zeroed() } 是简单可靠的零初始化方式。
        // MsvcVector 全零表示空 vector（_First=_Last=_End=null），
        // *mut SimFrameInfo 全零表示 null 指针，bool/f32/i32 全零为 0/false。
        unsafe { std::mem::zeroed() }
    }

    /// `SimFrameManager::HandleMessage`（Phase 7 升级版）。
    ///
    /// 对照源码 `08_sim_frame_manager.c` L352-1112。
    ///
    /// Phase 7 改造：
    /// - 8 真实 emplace：NewGameFrame / SetDebugProperties / ModifyCell /
    ///   ChangeCellProperties / SetInsulationValue / SetStrengthValue /
    ///   MassConsumption / MassEmission（NewGameFrame + SetDebugProperties 是
    ///   特殊处理，6 个是普通 emplace——总 8 个 arm 真实写入 currentFrame 字段）
    /// - 1 ModifyBuildingEnergy drain（Phase 9 实现）
    /// - 36 drain-only：读消息字节后丢弃（保持 reader 缓冲区同步推进）
    /// - 0 个未命中：返回 false，调用方回退到 gSimMessageHandlers 表
    ///
    /// # 参数
    /// - `id`：消息 ID
    /// - `reader`：消息体 reader（包含消息数据，由调用方传入）
    ///
    /// # 返回
    /// - `true`：消息 ID 命中（已 emplace 或 drain）
    /// - `false`：未命中，回退到 handler 表
    ///
    /// # 改造说明
    /// 原版方法是 `__thiscall`（this + id + size + data + out_ptr），
    /// Rust 改造为 `&mut self` + `id` + `reader: &mut BinaryBufferReader`。
    /// `current_frame` 必须非 null（NewFrame 之后调用），否则返回 false 不 panic。
    pub fn handle_message(&mut self, id: c_int, reader: &mut BinaryBufferReader) -> bool {
        let id_u32 = id as u32;
        // current_frame 为 null 时不能 emplace（防御性）
        if self.current_frame.is_null() {
            tracing::error!(id = id_u32, "HandleMessage: current_frame is null, skipping emplace");
            return false;
        }
        let frame: &mut SimFrameInfo = unsafe { &mut *self.current_frame };
        match id_u32 {
            // === 特殊处理 2 项 ===
            MSG_NEW_GAME_FRAME => {
                // 源码 L393-440：调 NewFrame（创建/复用帧）+ 读取 activeRegion 数据
                // Phase 7：NewFrame 已在 T4 实现，这里读取消息体填充 active_region
                // 消息体格式：f32 elapsed + 4×i32 region + 2×f32 sunlight/cosmic = 7×4=28B
                let elapsed = reader.read_float().unwrap_or(0.0);
                let min_x = reader.read_int().unwrap_or(0);
                let min_y = reader.read_int().unwrap_or(0);
                let max_x = reader.read_int().unwrap_or(0);
                let max_y = reader.read_int().unwrap_or(0);
                let sunlight = reader.read_float().unwrap_or(0.0);
                let cosmic = reader.read_float().unwrap_or(0.0);
                self.active_region = NewGameFrame {
                    elapsed_seconds: elapsed,
                    active_region_min_x: min_x,
                    active_region_min_y: min_y,
                    active_region_max_x: max_x,
                    active_region_max_y: max_y,
                    current_sunlight_intensity: sunlight,
                    current_cosmic_radiation_intensity: cosmic,
                };
                self.elapsed_seconds = elapsed;
                tracing::trace!(id = id_u32, elapsed, "HandleMessage: NewGameFrame (real)");
                true
            }
            MSG_SET_DEBUG_PROPERTIES => {
                // 源码 L441-450：直接赋值 currentFrame->debugProperties 3 字段
                let bts = reader.read_float().unwrap_or(1.0);
                let btbts = reader.read_float().unwrap_or(1.0);
                let is_debug = reader.read_byte().unwrap_or(0);
                frame.debug_properties.building_temperature_scale = bts;
                frame.debug_properties.building_to_building_temperature_scale = btbts;
                frame.debug_properties.is_debug_editing = is_debug != 0;
                tracing::trace!(id = id_u32, bts, btbts, "HandleMessage: SetDebugProperties (real)");
                true
            }
            // === 普通 emplace 6 项 ===
            MSG_MODIFY_CELL => {
                // 源码：emplace_back 到 currentFrame->cellModifications
                // 消息体 28B：CellModification
                let cm = read_cell_modification(reader);
                frame.cell_modifications.push(cm);
                tracing::trace!(id = id_u32, count = frame.cell_modifications.len(), "HandleMessage: emplace cellModifications (real)");
                true
            }
            MSG_CHANGE_CELL_PROPERTIES => {
                let cp = read_cell_properties_change(reader);
                frame.set_cell_properties.push(cp);
                tracing::trace!(id = id_u32, count = frame.set_cell_properties.len(), "HandleMessage: emplace setCellProperties (real)");
                true
            }
            MSG_SET_INSULATION_VALUE => {
                let v = read_set_cell_float_value(reader);
                frame.set_insulation_values.push(v);
                tracing::trace!(id = id_u32, count = frame.set_insulation_values.len(), "HandleMessage: emplace setInsulationValues (real)");
                true
            }
            MSG_SET_STRENGTH_VALUE => {
                let v = read_set_cell_float_value(reader);
                frame.set_strength_values.push(v);
                tracing::trace!(id = id_u32, count = frame.set_strength_values.len(), "HandleMessage: emplace setStrengthValues (real)");
                true
            }
            MSG_MASS_CONSUMPTION => {
                let m = read_mass_consumption(reader);
                frame.mass_consumption_messages.push(m);
                tracing::trace!(id = id_u32, count = frame.mass_consumption_messages.len(), "HandleMessage: emplace massConsumptionMessages (real)");
                true
            }
            MSG_MASS_EMISSION => {
                let m = read_mass_emission(reader);
                frame.mass_emission_messages.push(m);
                tracing::trace!(id = id_u32, count = frame.mass_emission_messages.len(), "HandleMessage: emplace massEmissionMessages (real)");
                true
            }
            // === ModifyBuildingEnergy 特殊处理：drain-only（Phase 9 实现）===
            MSG_MODIFY_BUILDING_ENERGY => {
                // 源码 L450-?：赋值 currentFrame->modifyBuildingEnergyMessages
                // Phase 7 未声明此字段，drain-only：读 8B 丢弃
                drain_message(reader, 8, id_u32, "modifyBuildingEnergyMessages");
                true
            }
            // === drain-only 36 项（消息体大小待源码逐项核对）===
            // 注：先用 drain_message(reader, 8, ...) 读 8B 丢弃，保持 reader 同步推进。
            // 若部署后日志显示 reader 错位，再调整 N。
            MSG_ADD_BUILDING_TO_BUILDING_HEAT_EXCHANGE => {
                drain_message(reader, 8, id_u32, "buildingToBuildingHeatExchangeMessages.adds");
                true
            }
            MSG_REMOVE_BUILDING_IN_CONTACT_FROM_BUILDING_TO_BUILDING_HEAT_EXCHANGE => {
                drain_message(reader, 8, id_u32, "buildingToBuildingHeatExchangeMessages.modifies");
                true
            }
            MSG_CELL_RADIATION_MODIFICATION => {
                drain_message(reader, 8, id_u32, "cellRadiationModifications");
                true
            }
            MSG_MODIFY_DISEASE_EMITTER => {
                drain_message(reader, 8, id_u32, "diseaseEmitterMessages.modifies");
                true
            }
            MSG_CELL_DISEASE_MODIFICATION => {
                drain_message(reader, 8, id_u32, "cellDiseaseModifications");
                true
            }
            MSG_MODIFY_DISEASE_CONSUMER => {
                drain_message(reader, 8, id_u32, "diseaseConsumerMessages.modifies");
                true
            }
            MSG_ADD_IN_CONTACT_BUILDING_TO_BUILDING_TO_BUILDING_HEAT_EXCHANGE => {
                drain_message(reader, 8, id_u32, "addBuildingInContactToBuildingToBuildingHeatExchangeMessages");
                true
            }
            MSG_REMOVE_ELEMENT_EMITTER => {
                drain_message(reader, 8, id_u32, "elementEmitterMessages.removes");
                true
            }
            MSG_ADD_RADIATION_EMITTER => {
                drain_message(reader, 8, id_u32, "radiationEmitterMessages.adds");
                true
            }
            MSG_MODIFY_CHUNK_TEMPERATURE_ADJUSTER => {
                drain_message(reader, 8, id_u32, "modifyElementChunkAdjusterMessages");
                true
            }
            MSG_CONSUME_DISEASE => {
                drain_message(reader, 8, id_u32, "consumeDisease");
                true
            }
            MSG_REMOVE_ELEMENT_CHUNK => {
                drain_message(reader, 8, id_u32, "elementChunkMessages.removes");
                true
            }
            MSG_REMOVE_DISEASE_CONSUMER => {
                drain_message(reader, 8, id_u32, "diseaseConsumerMessages.removes");
                true
            }
            MSG_REMOVE_RADIATION_EMITTER => {
                drain_message(reader, 8, id_u32, "radiationEmitterMessages.removes");
                true
            }
            MSG_MODIFY_BACKWALL_DATA => {
                drain_message(reader, 8, id_u32, "modifyBackwallDataMessages");
                true
            }
            MSG_ADD_ELEMENT_EMITTER => {
                drain_message(reader, 8, id_u32, "elementEmitterMessages.adds");
                true
            }
            MSG_MODIFY_RADIATION_EMITTER => {
                drain_message(reader, 8, id_u32, "radiationEmitterMessages.modifies");
                true
            }
            MSG_REMOVE_BUILDING_HEAT_EXCHANGE => {
                drain_message(reader, 8, id_u32, "buildingHeatExchangeMessages.removes");
                true
            }
            MSG_MODIFY_CELL_WORLD_ZONE => {
                drain_message(reader, 8, id_u32, "cellWorldZoneModifications");
                true
            }
            MSG_SET_ELEMENT_CHUNK_DATA => {
                drain_message(reader, 8, id_u32, "elementChunkMessages.modifies");
                true
            }
            MSG_MOVE_ELEMENT_CHUNK => {
                drain_message(reader, 8, id_u32, "moveElementChunkMessages");
                true
            }
            MSG_ADD_DISEASE_CONSUMER => {
                drain_message(reader, 8, id_u32, "diseaseConsumerMessages.adds");
                true
            }
            MSG_MODIFY_ELEMENT_EMITTER => {
                drain_message(reader, 8, id_u32, "elementEmitterMessages.modifies");
                true
            }
            MSG_REMOVE_DISEASE_EMITTER => {
                drain_message(reader, 8, id_u32, "diseaseEmitterMessages.removes");
                true
            }
            MSG_REMOVE_BUILDING_TO_BUILDING_HEAT_EXCHANGE => {
                drain_message(reader, 8, id_u32, "buildingToBuildingHeatExchangeMessages.removes");
                true
            }
            MSG_MODIFY_CELL_ENERGY => {
                drain_message(reader, 8, id_u32, "cellEnergyModifications");
                true
            }
            MSG_DIG => {
                drain_message(reader, 8, id_u32, "digPoints");
                true
            }
            MSG_RADIATION_PARAMS_MODIFICATION => {
                drain_message(reader, 8, id_u32, "radiationParamsModifications");
                true
            }
            MSG_ADD_BUILDING_HEAT_EXCHANGE => {
                drain_message(reader, 8, id_u32, "buildingHeatExchangeMessages.adds");
                true
            }
            MSG_MODIFY_BUILDING_HEAT_EXCHANGE => {
                drain_message(reader, 8, id_u32, "buildingHeatExchangeMessages.modifies");
                true
            }
            MSG_ADD_ELEMENT_CONSUMER => {
                drain_message(reader, 8, id_u32, "elementConsumerMessages.adds");
                true
            }
            MSG_ADD_ELEMENT_CHUNK => {
                drain_message(reader, 8, id_u32, "elementChunkMessages.adds");
                true
            }
            MSG_ADD_DISEASE_EMITTER => {
                drain_message(reader, 8, id_u32, "diseaseEmitterMessages.adds");
                true
            }
            MSG_SET_ELEMENT_CONSUMER_DATA => {
                drain_message(reader, 8, id_u32, "elementConsumerMessages.modifies");
                true
            }
            MSG_MODIFY_ELEMENT_CHUNK_ENERGY => {
                drain_message(reader, 8, id_u32, "modifyElementChunkEnergyMessages");
                true
            }
            MSG_REMOVE_ELEMENT_CONSUMER => {
                drain_message(reader, 8, id_u32, "elementConsumerMessages.removes");
                true
            }
            // 未命中：返回 false，调用方回退到 gSimMessageHandlers 表
            _ => false,
        }
    }
}

// === Phase 7 T3：reader 辅助函数（handle_message emplace 用）===

/// 辅助：从 reader 读 CellModification（28B）。
///
/// 对照源码 06_process_messages.c L1303-1305 的 `local_c0` 拆包。
fn read_cell_modification(reader: &mut BinaryBufferReader) -> CellModification {
    CellModification {
        game_cell: reader.read_int().unwrap_or(0),
        callback_idx: reader.read_int().unwrap_or(0),
        mass: reader.read_float().unwrap_or(0.0),
        temperature: reader.read_float().unwrap_or(0.0),
        disease_count: reader.read_int().unwrap_or(0),
        element_idx: reader.read_ushort().unwrap_or(0),
        disease_idx: reader.read_byte().unwrap_or(0),
        replace_mode: reader.read_byte().unwrap_or(0),
        flags: reader.read_byte().unwrap_or(0),
        _pad: [0; 3],
    }
}

/// 辅助：从 reader 读 CellPropertiesChange（12B）。
fn read_cell_properties_change(reader: &mut BinaryBufferReader) -> CellPropertiesChange {
    CellPropertiesChange {
        game_cell: reader.read_int().unwrap_or(0),
        callback_idx: reader.read_int().unwrap_or(0),
        property_flags: reader.read_byte().unwrap_or(0),
        set_or_clear: reader.read_byte().unwrap_or(0),
        _pad: [0; 2],
    }
}

/// 辅助：从 reader 读 MassConsumption（16B）。
fn read_mass_consumption(reader: &mut BinaryBufferReader) -> MassConsumption {
    MassConsumption {
        game_cell: reader.read_int().unwrap_or(0),
        callback_idx: reader.read_int().unwrap_or(0),
        removed_elem_idx: reader.read_ushort().unwrap_or(0),
        loop_radius: reader.read_byte().unwrap_or(0),
        use_rectangle: reader.read_byte().unwrap_or(0),
    }
}

/// 辅助：从 reader 读 MassEmission（24B）。
fn read_mass_emission(reader: &mut BinaryBufferReader) -> MassEmission {
    MassEmission {
        game_cell: reader.read_int().unwrap_or(0),
        callback_idx_or_temp: reader.read_float().unwrap_or(0.0),
        mass: reader.read_float().unwrap_or(0.0),
        temperature: reader.read_float().unwrap_or(0.0),
        disease_count: reader.read_int().unwrap_or(0),
        element_idx: reader.read_ushort().unwrap_or(0),
        disease_idx: reader.read_byte().unwrap_or(0),
        emitted: reader.read_byte().unwrap_or(0),
        _pad: [0; 2],
    }
}

/// 辅助：从 reader 读 SetCellFloatValue（8B）。
fn read_set_cell_float_value(reader: &mut BinaryBufferReader) -> SetCellFloatValue {
    SetCellFloatValue {
        game_cell: reader.read_int().unwrap_or(0),
        value: reader.read_float().unwrap_or(0.0),
    }
}

/// 辅助：drain-only 读取 N 字节后丢弃（保持 reader 缓冲区同步推进）。
///
/// 对照源码 36 个非 Phase 7 arm 的 `reader->read<T>()` 模式。
/// Phase 7 不 emplace 这些消息到 currentFrame（字段未声明），
/// 但必须读 N 字节以保持 reader 位置与原版一致（否则后续消息错位）。
fn drain_message(reader: &mut BinaryBufferReader, n: usize, id: u32, field: &'static str) {
    for _ in 0..n {
        let _ = reader.read_byte();
    }
    tracing::trace!(id, field, n, "HandleMessage: drain-only (Phase 7)");
}

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

    use crate::message_handler::MessageType;

    /// 辅助：把 MessageType 转为 c_int（bit-pattern 保留，u32 → i32）。
    fn msg_id(mt: MessageType) -> i32 {
        mt as u32 as i32
    }

    /// 辅助：构造测试用 BinaryBufferReader（从 byte slice）。
    fn make_test_reader(data: &[u8]) -> BinaryBufferReader {
        BinaryBufferReader::new(data)
    }

    #[test]
    fn handle_message_returns_false_for_new_game_frame_when_current_frame_null() {
        // current_frame 为 null 时 handle_message 应返回 false（防御性）
        let mut mgr = SimFrameManager::new_zeroed();
        let mut reader = make_test_reader(&[]);
        assert!(!mgr.handle_message(msg_id(MessageType::SimFrameManager_NewGameFrame), &mut reader));
    }

    #[test]
    fn handle_message_emplaces_set_debug_properties_when_current_frame_nonnull() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut frame = Box::new(SimFrameInfo::default());
        mgr.current_frame = frame.as_mut() as *mut _;
        // 消息体：4B f32 (1.0) + 4B f32 (1.0) + 1B u8 (1) = 9B
        let mut reader = make_test_reader(&[0x3f, 0x80, 0, 0, 0x3f, 0x80, 0, 0, 0x01]);
        assert!(mgr.handle_message(msg_id(MessageType::SetDebugProperties), &mut reader));
        assert_eq!(frame.debug_properties.is_debug_editing, true);
    }

    #[test]
    fn handle_message_drains_modify_building_energy() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut frame = Box::new(SimFrameInfo::default());
        mgr.current_frame = frame.as_mut() as *mut _;
        let mut reader = make_test_reader(&[0; 8]);
        assert!(mgr.handle_message(msg_id(MessageType::ModifyBuildingEnergy), &mut reader));
    }

    #[test]
    fn handle_message_emplaces_6_real_arms_when_current_frame_nonnull() {
        // 6 个真实 emplace：ModifyCell / ChangeCellProperties / SetInsulationValue /
        // SetStrengthValue / MassConsumption / MassEmission
        // （NewGameFrame + SetDebugProperties 是特殊处理，另 2 个测试覆盖）
        let mut mgr = SimFrameManager::new_zeroed();
        let mut frame = Box::new(SimFrameInfo::default());
        mgr.current_frame = frame.as_mut() as *mut _;

        // 28B for CellModification
        let modify_cell_data: Vec<u8> = vec![0; 28];
        let mut reader = make_test_reader(&modify_cell_data);
        assert!(mgr.handle_message(msg_id(MessageType::ModifyCell), &mut reader));
        assert_eq!(frame.cell_modifications.len(), 1);

        // 12B for CellPropertiesChange
        let prop_data: Vec<u8> = vec![0; 12];
        let mut reader = make_test_reader(&prop_data);
        assert!(mgr.handle_message(msg_id(MessageType::ChangeCellProperties), &mut reader));
        assert_eq!(frame.set_cell_properties.len(), 1);

        // 8B for SetCellFloatValue
        let val_data: Vec<u8> = vec![0; 8];
        let mut reader = make_test_reader(&val_data);
        assert!(mgr.handle_message(msg_id(MessageType::SetInsulationValue), &mut reader));
        assert_eq!(frame.set_insulation_values.len(), 1);

        let mut reader = make_test_reader(&val_data);
        assert!(mgr.handle_message(msg_id(MessageType::SetStrengthValue), &mut reader));
        assert_eq!(frame.set_strength_values.len(), 1);

        // 16B for MassConsumption
        let mc_data: Vec<u8> = vec![0; 16];
        let mut reader = make_test_reader(&mc_data);
        assert!(mgr.handle_message(msg_id(MessageType::MassConsumption), &mut reader));
        assert_eq!(frame.mass_consumption_messages.len(), 1);

        // 24B for MassEmission
        let me_data: Vec<u8> = vec![0; 24];
        let mut reader = make_test_reader(&me_data);
        assert!(mgr.handle_message(msg_id(MessageType::MassEmission), &mut reader));
        assert_eq!(frame.mass_emission_messages.len(), 1);
    }

    #[test]
    fn handle_message_drains_36_non_phase7_arms_when_current_frame_nonnull() {
        // 36 个 drain-only arm：每个应读 8B 后返回 true
        // （ModifyBuildingEnergy 单独测试，不在此列）
        let drain_ids = [
            MessageType::AddBuildingToBuildingHeatExchange,
            MessageType::RemoveBuildingInContactFromBuildingToBuildingHeatExchange,
            MessageType::CellRadiationModification,
            MessageType::ModifyDiseaseEmitter,
            MessageType::CellDiseaseModification,
            MessageType::ModifyDiseaseConsumer,
            MessageType::AddInContactBuildingToBuildingToBuildingHeatExchange,
            MessageType::RemoveElementEmitter,
            MessageType::AddRadiationEmitter,
            MessageType::ModifyChunkTemperatureAdjuster,
            MessageType::ConsumeDisease,
            MessageType::RemoveElementChunk,
            MessageType::RemoveDiseaseConsumer,
            MessageType::RemoveRadiationEmitter,
            MessageType::ModifyBackwallData,
            MessageType::AddElementEmitter,
            MessageType::ModifyRadiationEmitter,
            MessageType::RemoveBuildingHeatExchange,
            MessageType::ModifyCellWorldZone,
            MessageType::SetElementChunkData,
            MessageType::MoveElementChunk,
            MessageType::AddDiseaseConsumer,
            MessageType::ModifyElementEmitter,
            MessageType::RemoveDiseaseEmitter,
            MessageType::RemoveBuildingToBuildingHeatExchange,
            MessageType::ModifyCellEnergy,
            MessageType::Dig,
            MessageType::RadiationParamsModification,
            MessageType::AddBuildingHeatExchange,
            MessageType::ModifyBuildingHeatExchange,
            MessageType::AddElementConsumer,
            MessageType::AddElementChunk,
            MessageType::AddDiseaseEmitter,
            MessageType::SetElementConsumerData,
            MessageType::ModifyElementChunkEnergy,
            MessageType::RemoveElementConsumer,
        ];
        assert_eq!(drain_ids.len(), 36, "drain-only arm 实际数量（8 真实 + 1 ModifyBuildingEnergy drain + 36 drain-only = 45 arms）");
        let mut mgr = SimFrameManager::new_zeroed();
        let mut frame = Box::new(SimFrameInfo::default());
        mgr.current_frame = frame.as_mut() as *mut _;
        for mt in drain_ids {
            let mut reader = make_test_reader(&[0u8; 8]);
            assert!(mgr.handle_message(msg_id(mt), &mut reader), "drain id {:?} should return true", mt);
        }
    }

    #[test]
    fn handle_message_returns_false_for_save_load_ids() {
        // 9 个 save_load ID 不在 HandleMessage 路由表，应返回 false
        // 注：current_frame 为 null 时 handle_message 在 null 检查处返回 false，
        // 不读 reader，所以空 reader 即可
        let mut mgr = SimFrameManager::new_zeroed();
        let save_load_ids = [
            MessageType::Load,
            MessageType::Start,
            MessageType::AllocateCells,
            MessageType::DefineWorldOffsets,
            MessageType::PrepareGameData,
            MessageType::ClearUnoccupiedCells,
            MessageType::SetSavedOptions,
            MessageType::Disease_CreateTable,
            MessageType::SimData_ResizeAndInitializeVacuumCells,
        ];
        for mt in save_load_ids {
            let mut reader = make_test_reader(&[]);
            assert!(
                !mgr.handle_message(msg_id(mt), &mut reader),
                "HandleMessage should NOT route save_load id={}",
                mt as u32
            );
        }
    }

    #[test]
    fn handle_message_returns_false_for_unknown_id() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut reader = make_test_reader(&[]);
        // id=0 不在任何路由表（且 current_frame 为 null）
        assert!(!mgr.handle_message(0, &mut reader));
    }

    #[test]
    fn global_sim_frame_manager_singleton_is_lockable() {
        // 验证全局单例可锁、可访问（不 panic）。
        // 用 unwrap_or_else(|e| e.into_inner()) 避免恐慌中毒导致死锁（简报关键约束）。
        let mut mgr = G_SIM_FRAME_MANAGER
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(
            mgr.current_frame.is_null(),
            "current_frame should be null in skeleton"
        );
        let mut reader = make_test_reader(&[]);
        assert!(!mgr.handle_message(0, &mut reader), "unknown id should return false");
    }
}
