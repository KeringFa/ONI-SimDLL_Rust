//! SimFrameManager — 帧管理器（管理帧队列与回放）+ Phase 6 消息路由。
//!
//! 字段对照源码 00_types_reference.c L6468-6479（`struct SimFrameManager`）。
//! 原版运行时位于 `gSim + 0x50`（01_sim_api.c L227 / 11_msvcrt_ignored.c L2424）；
//! Rust 改造为独立全局单例 `G_SIM_FRAME_MANAGER`（C# 侧无感知，见规格 §3.1）。
//!
//! Phase 6 新增 `handle_message` 方法（44 路由 match arms 骨架）。

use crate::message_handler::MessageType;
use crate::sim_data::ActiveRegion;
use crate::sim_events::SimFrameInfo;
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

    /// `SimFrameManager::HandleMessage` 路由骨架（Phase 6）。
    ///
    /// 对照源码 `08_sim_frame_manager.c` L352-1112。
    ///
    /// 命中 45 个消息 ID 之一时记录 `tracing::trace!` 日志并返回 `true`（表示已处理）。
    /// 不实际反序列化填充 `currentFrame` 字段（`currentFrame` 为 null，`SimFrameInfo`
    /// 仍是空占位，Phase 7+ 补全字段后替换为真实 emplace 逻辑）。
    ///
    /// # 参数
    /// - `id`：消息 ID（原版 `param_1`，C# `int` 类型，bit-pattern 保留转为 u32 匹配）
    ///
    /// # 返回
    /// - `true`：消息 ID 命中路由表之一（已记录 log，调用方跳过 `gSimMessageHandlers` 表）
    /// - `false`：消息 ID 未命中，调用方应回退到 `gSimMessageHandlers` 表查找
    ///
    /// # 改造说明
    /// 原版方法是 `__thiscall`（this 指针 + 4 个参数：id / size / data / out_ptr），
    /// Rust 改造为 `&mut self` + 仅 `id` 参数。`size` / `data` / `out_ptr` 在 Phase 6 骨架
    /// 不使用（不反序列化）。Phase 7+ 接入真实 emplace 时补回 `data: *mut c_void` 参数
    /// 读取消息体。**运行时无差异**：调用方 `SIM_HandleMessage` 已经持有 reader，
    /// 仅在 `handle_message` 返回 false 时才用 reader 走 handler 表，命中时不读 reader。
    pub fn handle_message(&mut self, id: c_int) -> bool {
        let id_u32 = id as u32;
        match id_u32 {
            // === 特殊处理 3 项（源码非 emplace 模式）===
            MSG_NEW_GAME_FRAME => {
                tracing::trace!(
                    id = id_u32,
                    "HandleMessage: SimFrameManager_NewGameFrame (special: NewFrame + fill activeRegions) — skeleton log only"
                );
                true
            }
            MSG_SET_DEBUG_PROPERTIES => {
                tracing::trace!(
                    id = id_u32,
                    "HandleMessage: SetDebugProperties (special: direct assign debugProperties 4 fields) — skeleton log only"
                );
                true
            }
            MSG_MODIFY_BUILDING_ENERGY => {
                tracing::trace!(
                    id = id_u32,
                    "HandleMessage: ModifyBuildingEnergy (special: direct assign modifyBuildingEnergyMessages) — skeleton log only"
                );
                true
            }
            // === 普通 emplace 路由 42 项（源码模式：currentFrame->X.emplace_back(reader->read<T>())）===
            MSG_CHANGE_CELL_PROPERTIES => {
                tracing::trace!(id = id_u32, field = "setCellProperties", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_ADD_BUILDING_TO_BUILDING_HEAT_EXCHANGE => {
                tracing::trace!(id = id_u32, field = "buildingToBuildingHeatExchangeMessages.adds", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_REMOVE_BUILDING_IN_CONTACT_FROM_BUILDING_TO_BUILDING_HEAT_EXCHANGE => {
                tracing::trace!(id = id_u32, field = "buildingToBuildingHeatExchangeMessages.modifies", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_CELL_RADIATION_MODIFICATION => {
                tracing::trace!(id = id_u32, field = "cellRadiationModifications", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_DISEASE_EMITTER => {
                tracing::trace!(id = id_u32, field = "diseaseEmitterMessages.modifies", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_CELL_DISEASE_MODIFICATION => {
                tracing::trace!(id = id_u32, field = "cellDiseaseModifications", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_DISEASE_CONSUMER => {
                tracing::trace!(id = id_u32, field = "diseaseConsumerMessages.modifies", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_ADD_IN_CONTACT_BUILDING_TO_BUILDING_TO_BUILDING_HEAT_EXCHANGE => {
                tracing::trace!(id = id_u32, field = "addBuildingInContactToBuildingToBuildingHeatExchangeMessages", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_REMOVE_ELEMENT_EMITTER => {
                tracing::trace!(id = id_u32, field = "elementEmitterMessages.removes", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_ADD_RADIATION_EMITTER => {
                tracing::trace!(id = id_u32, field = "radiationEmitterMessages.adds", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_CHUNK_TEMPERATURE_ADJUSTER => {
                tracing::trace!(id = id_u32, field = "modifyElementChunkAdjusterMessages", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_CELL => {
                tracing::trace!(id = id_u32, field = "cellModifications", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_CONSUME_DISEASE => {
                tracing::trace!(id = id_u32, field = "consumeDisease", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_REMOVE_ELEMENT_CHUNK => {
                tracing::trace!(id = id_u32, field = "elementChunkMessages.removes", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_SET_INSULATION_VALUE => {
                tracing::trace!(id = id_u32, field = "setInsulationValues", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_REMOVE_DISEASE_CONSUMER => {
                tracing::trace!(id = id_u32, field = "diseaseConsumerMessages.removes", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_REMOVE_RADIATION_EMITTER => {
                tracing::trace!(id = id_u32, field = "radiationEmitterMessages.removes", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_BACKWALL_DATA => {
                tracing::trace!(id = id_u32, field = "modifyBackwallDataMessages", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_ADD_ELEMENT_EMITTER => {
                tracing::trace!(id = id_u32, field = "elementEmitterMessages.adds", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_RADIATION_EMITTER => {
                tracing::trace!(id = id_u32, field = "radiationEmitterMessages.modifies", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_REMOVE_BUILDING_HEAT_EXCHANGE => {
                tracing::trace!(id = id_u32, field = "buildingHeatExchangeMessages.removes", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_CELL_WORLD_ZONE => {
                tracing::trace!(id = id_u32, field = "cellWorldZoneModifications", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_SET_ELEMENT_CHUNK_DATA => {
                tracing::trace!(id = id_u32, field = "elementChunkMessages.modifies", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MOVE_ELEMENT_CHUNK => {
                tracing::trace!(id = id_u32, field = "moveElementChunkMessages", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_ADD_DISEASE_CONSUMER => {
                tracing::trace!(id = id_u32, field = "diseaseConsumerMessages.adds", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_ELEMENT_EMITTER => {
                tracing::trace!(id = id_u32, field = "elementEmitterMessages.modifies", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_REMOVE_DISEASE_EMITTER => {
                tracing::trace!(id = id_u32, field = "diseaseEmitterMessages.removes", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_REMOVE_BUILDING_TO_BUILDING_HEAT_EXCHANGE => {
                tracing::trace!(id = id_u32, field = "buildingToBuildingHeatExchangeMessages.removes", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MASS_EMISSION => {
                tracing::trace!(id = id_u32, field = "massEmissionMessages", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_CELL_ENERGY => {
                tracing::trace!(id = id_u32, field = "cellEnergyModifications", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_DIG => {
                tracing::trace!(id = id_u32, field = "digPoints", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_RADIATION_PARAMS_MODIFICATION => {
                tracing::trace!(id = id_u32, field = "radiationParamsModifications", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_SET_STRENGTH_VALUE => {
                tracing::trace!(id = id_u32, field = "setStrengthValues", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MASS_CONSUMPTION => {
                tracing::trace!(id = id_u32, field = "massConsumptionMessages", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_ADD_BUILDING_HEAT_EXCHANGE => {
                tracing::trace!(id = id_u32, field = "buildingHeatExchangeMessages.adds", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_BUILDING_HEAT_EXCHANGE => {
                tracing::trace!(id = id_u32, field = "buildingHeatExchangeMessages.modifies", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_ADD_ELEMENT_CONSUMER => {
                tracing::trace!(id = id_u32, field = "elementConsumerMessages.adds", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_ADD_ELEMENT_CHUNK => {
                tracing::trace!(id = id_u32, field = "elementChunkMessages.adds", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_ADD_DISEASE_EMITTER => {
                tracing::trace!(id = id_u32, field = "diseaseEmitterMessages.adds", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_SET_ELEMENT_CONSUMER_DATA => {
                tracing::trace!(id = id_u32, field = "elementConsumerMessages.modifies", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_MODIFY_ELEMENT_CHUNK_ENERGY => {
                tracing::trace!(id = id_u32, field = "modifyElementChunkEnergyMessages", "HandleMessage: emplace (skeleton log only)");
                true
            }
            MSG_REMOVE_ELEMENT_CONSUMER => {
                tracing::trace!(id = id_u32, field = "elementConsumerMessages.removes", "HandleMessage: emplace (skeleton log only)");
                true
            }
            // 未命中：返回 false，调用方回退到 gSimMessageHandlers 表
            _ => false,
        }
    }
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

    #[test]
    fn handle_message_returns_true_for_new_game_frame() {
        let mut mgr = SimFrameManager::new_zeroed();
        assert!(mgr.handle_message(msg_id(MessageType::SimFrameManager_NewGameFrame)));
    }

    #[test]
    fn handle_message_returns_true_for_set_debug_properties() {
        let mut mgr = SimFrameManager::new_zeroed();
        assert!(mgr.handle_message(msg_id(MessageType::SetDebugProperties)));
    }

    #[test]
    fn handle_message_returns_true_for_modify_building_energy() {
        let mut mgr = SimFrameManager::new_zeroed();
        assert!(mgr.handle_message(msg_id(MessageType::ModifyBuildingEnergy)));
    }

    #[test]
    fn handle_message_returns_true_for_all_emplace_routes() {
        // 验证全部普通 emplace 路由命中（返回 true）。
        // 数量 = 42（原研究阶段概括为 40/41，逐项核对源码 if-else 树后确认 42 项，
        // 含原计划遗漏的 ModifyElementChunkEnergy；详见 spec §6.2 注释）。
        let mut mgr = SimFrameManager::new_zeroed();
        let routes = [
            MessageType::ChangeCellProperties,
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
            MessageType::ModifyCell,
            MessageType::ConsumeDisease,
            MessageType::RemoveElementChunk,
            MessageType::SetInsulationValue,
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
            MessageType::MassEmission,
            MessageType::ModifyCellEnergy,
            MessageType::Dig,
            MessageType::RadiationParamsModification,
            MessageType::SetStrengthValue,
            MessageType::MassConsumption,
            MessageType::AddBuildingHeatExchange,
            MessageType::ModifyBuildingHeatExchange,
            MessageType::AddElementConsumer,
            MessageType::AddElementChunk,
            MessageType::AddDiseaseEmitter,
            MessageType::SetElementConsumerData,
            MessageType::ModifyElementChunkEnergy,
            MessageType::RemoveElementConsumer,
        ];
        assert_eq!(
            routes.len(),
            42,
            "普通 emplace 实际 42 项（原计划 40，逐项核对源码后补全 ModifyElementChunkEnergy + CellRadiationModification 等，详见 spec §6.2）"
        );
        for mt in routes {
            assert!(
                mgr.handle_message(msg_id(mt)),
                "HandleMessage should route id={}",
                mt as u32
            );
        }
    }

    #[test]
    fn handle_message_returns_false_for_save_load_ids() {
        // 9 个 save_load ID 不在 HandleMessage 路由表，应返回 false
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
            assert!(
                !mgr.handle_message(msg_id(mt)),
                "HandleMessage should NOT route save_load id={}",
                mt as u32
            );
        }
    }

    #[test]
    fn handle_message_returns_false_for_unknown_id() {
        let mut mgr = SimFrameManager::new_zeroed();
        // id=0 不在任何路由表
        assert!(!mgr.handle_message(0));
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
        assert!(!mgr.handle_message(0), "unknown id should return false");
    }
}
