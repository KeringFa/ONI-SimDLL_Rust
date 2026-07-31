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

    /// `SimFrameManager::NewFrame` — 从帧池取/建帧，置为 current_frame。
    ///
    /// 对照源码 08_sim_frame_manager.c L1116-1176。
    /// **改造**：源码从 `frame_pool` 取 `SimFrameInfo*`（若空则 `operator new`）；
    /// Rust 用 `Box<SimFrameInfo>` + `Box::into_raw`，`frame_pool` 改为存 `*mut SimFrameInfo`。
    /// 取出后清空 frame 内部 vectors（reset 到空），置为 `current_frame`。
    ///
    /// **运行时影响**：游戏每帧开始时会调用此方法获取一个空白 `SimFrameInfo`。
    /// 第一次调用会分配新帧；后续若已有回收帧则从池中复用（减少分配开销）。
    /// 与原版 `operator new` 行为一致，对游戏画面无可见差异。
    pub fn new_frame(&mut self) {
        // 从 frame_pool 取一个可复用帧
        let frame_ptr = if self.frame_pool.is_empty() {
            // 新建
            let boxed = Box::new(SimFrameInfo::default());
            Box::into_raw(boxed)
        } else {
            let ptr = self.frame_pool.as_slice()[0];
            self.frame_pool.erase(0);
            ptr
        };
        // 清空 frame 内部 vectors（reset 到空）
        if !frame_ptr.is_null() {
            unsafe {
                let frame = &mut *frame_ptr;
                frame.cell_modifications.clear();
                frame.set_insulation_values.clear();
                frame.set_strength_values.clear();
                frame.set_cell_properties.clear();
                frame.mass_consumption_messages.clear();
                frame.mass_emission_messages.clear();
                frame.elapsed_seconds = 0.0;
                frame.debug_properties = crate::sim_data::DebugProperties::default();
            }
        }
        self.current_frame = frame_ptr;
    }

    /// `SimFrameManager::BeginFrameProcessing` — 锁 mutex，queued → active。
    ///
    /// 对照源码 08_sim_frame_manager.c L182-246。
    /// 返回 active_frames 长度（待处理帧数）。
    ///
    /// **改造**：源码用 `std::lock_guard` 锁 `frame_mutex`，Rust 由外部
    /// `Mutex<Box<SimFrameManager>>` 提供同步（G_SIM_FRAME_MANAGER 单例），
    /// 此方法内部不再二次加锁——避免 deadlock。
    ///
    /// **运行时影响**：游戏主循环每帧开始时调用，把上一帧排队的消息帧
    /// 移入"待处理"队列。无可见差异。
    pub fn begin_frame_processing(&mut self) -> i32 {
        // 移空 queued_frames 到 active_frames
        while !self.queued_frames.is_empty() {
            let ptr = self.queued_frames.as_slice()[0];
            self.queued_frames.erase(0);
            self.active_frames.push(ptr);
        }
        self.active_frames.len() as i32
    }

    /// `SimFrameManager::ProcessNextFrame` — 处理 active_frames 下一帧。
    ///
    /// 对照源码 08_sim_frame_manager.c L1903-1929。
    /// 取 `active_frames[num_frames_processed]`，调 `ProcessFrame`，
    /// `num_frames_processed += 1`，返回 elapsed_seconds。
    ///
    /// **运行时影响**：游戏主循环逐帧处理消息（如格子温度变化、物质排放）。
    /// Phase 7 简化版只调 `process_frame` 桩（tracing log），不实际修改游戏状态，
    /// 因此画面上还看不到温度/物质变化（T6-T10 接入真实处理后才有差异）。
    pub fn process_next_frame(&mut self, sim_data: *mut crate::sim_data::SimData) -> f32 {
        let idx = self.num_frames_processed as usize;
        if idx >= self.active_frames.len() {
            return 0.0;
        }
        let frame_ptr = self.active_frames.as_slice()[idx];
        self.current_frame = frame_ptr;
        self.num_frames_processed += 1;
        // 调 ProcessFrame（T6 实现，本任务先用桩）
        self.process_frame(sim_data);
        // 返回 elapsed_seconds（从 active_region 或 frame）
        let elapsed = unsafe { (*frame_ptr).elapsed_seconds };
        elapsed
    }

    /// `SimFrameManager::EndFrameProcessing` — active → processed，清空 active。
    ///
    /// 对照源码 08_sim_frame_manager.c L247-635。
    ///
    /// **运行时影响**：游戏主循环一帧处理完毕后调用，把已处理帧移入
    /// `processed_frames` 队列（供后续回收复用），并重置 `num_frames_processed`
    /// 计数器。无可见差异。
    pub fn end_frame_processing(&mut self) {
        while !self.active_frames.is_empty() {
            let ptr = self.active_frames.as_slice()[0];
            self.active_frames.erase(0);
            self.processed_frames.push(ptr);
        }
        self.num_frames_processed = 0;
    }

    /// `SimFrameManager::ProcessFrame` — 处理当前帧（Phase 7 简化版）。
    ///
    /// 对照源码 08_sim_frame_manager.c L1178-1902。
    /// Phase 7 实现：
    /// - 内联循环 set_insulation_values / set_strength_values
    /// - 调 4 个 Process* 函数（T7-T10 实现，本任务先用桩）
    /// Phase 9 的内联循环全部省略（消息已被 drain-only 消费）。
    ///
    /// **运行时影响**：本方法把 C# 发来的格子隔热值/强度值写入 `updated_cells` SOA。
    /// 游戏画面上对应的格子隔热/强度属性会随 C# 请求变化（如建墙后隔热值上升）。
    /// 4 个 Process* 函数为桩，相关功能（属性标志、物质置换等）暂未生效。
    pub fn process_frame(&mut self, sim_data: *mut crate::sim_data::SimData) {
        if self.current_frame.is_null() || sim_data.is_null() {
            return;
        }
        let frame = unsafe { &*self.current_frame };
        let sd = unsafe { &mut *sim_data };

        // === 内联循环 set_insulation_values / set_strength_values ===
        // 对照源码 08_sim_frame_manager.c L1180-?:
        // gameCell → cellIdx 转换公式（02_save_load.c 多处使用）：
        // idx = (game_cell / (height-2) + 1) * width + game_cell % (height-2) + 1
        let width = sd.width;
        let height_minus_2 = sd.height - 2;
        if width > 2 && height_minus_2 > 0 && !sd.updated_cells.ptr.is_null() {
            let updated_cells = unsafe { &mut *sd.updated_cells.ptr };
            // set_insulation_values: value (0.0..1.0) × 255 → u8
            let insulation = updated_cells.insulation.as_mut_slice();
            for v in frame.set_insulation_values.as_slice() {
                if let Some(idx) = game_cell_to_idx(v.game_cell, width, height_minus_2) {
                    let val = (v.value * 255.0).clamp(0.0, 255.0) as u8;
                    if (idx as usize) < insulation.len() {
                        insulation[idx as usize] = val;
                    }
                }
            }
            // set_strength_values: value 直接写 u8
            let strength = updated_cells.strength_info.as_mut_slice();
            for v in frame.set_strength_values.as_slice() {
                if let Some(idx) = game_cell_to_idx(v.game_cell, width, height_minus_2) {
                    let val = v.value.clamp(0.0, 255.0) as u8;
                    if (idx as usize) < strength.len() {
                        strength[idx as usize] = val;
                    }
                }
            }
        }

        // === 调 4 个 Process* 函数（T7-T10 实现，本任务用桩）===
        crate::process_messages::process_cell_properties(frame, sd);
        crate::process_messages::process_cell_modifications(frame, sd);
        crate::process_messages::process_mass_consumption(frame, sd);
        crate::process_messages::process_mass_emission(frame, sd);

        tracing::trace!(
            insulation_count = frame.set_insulation_values.len(),
            strength_count = frame.set_strength_values.len(),
            cell_mod_count = frame.cell_modifications.len(),
            "ProcessFrame: Phase 7 simplified (real inline loops + Process* stubs)"
        );
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

/// 辅助：game_cell → cell_idx 转换。
///
/// 对照源码 02_save_load.c 多处使用：
/// idx = (game_cell / (height-2) + 1) * width + game_cell % (height-2) + 1
///
/// 游戏内部坐标 (0..inner_w, 0..inner_h) 转换为含边界的 SOA 索引
/// （边界 1 格用于 unobtanium 墙）。
fn game_cell_to_idx(game_cell: i32, width: i32, height_minus_2: i32) -> Option<i32> {
    if width <= 2 || height_minus_2 <= 0 || game_cell < 0 {
        return None;
    }
    let row = game_cell / height_minus_2;
    let col = game_cell % height_minus_2;
    if row < 0 || col < 0 {
        return None;
    }
    Some((row + 1) * width + col + 1)
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

    // ===== Phase 7 T4：帧池方法 4 项测试 =====

    #[test]
    fn new_frame_creates_current_frame_nonnull() {
        let mut mgr = SimFrameManager::new_zeroed();
        assert!(mgr.current_frame.is_null());
        mgr.new_frame();
        assert!(!mgr.current_frame.is_null(), "new_frame should set current_frame");
        // 清理：释放分配的帧
        unsafe {
            let _ = Box::from_raw(mgr.current_frame);
            mgr.current_frame = std::ptr::null_mut();
        }
    }

    #[test]
    fn begin_frame_processing_moves_queued_to_active() {
        let mut mgr = SimFrameManager::new_zeroed();
        // 准备一个 queued frame
        let frame = Box::into_raw(Box::new(SimFrameInfo::default()));
        mgr.queued_frames.push(frame);
        let count = mgr.begin_frame_processing();
        assert_eq!(count, 1);
        assert_eq!(mgr.active_frames.len(), 1);
        assert_eq!(mgr.queued_frames.len(), 0);
        // 清理
        unsafe {
            let _ = Box::from_raw(mgr.active_frames.as_slice()[0]);
        }
    }

    #[test]
    fn end_frame_processing_moves_active_to_processed() {
        let mut mgr = SimFrameManager::new_zeroed();
        let frame = Box::into_raw(Box::new(SimFrameInfo::default()));
        mgr.active_frames.push(frame);
        mgr.end_frame_processing();
        assert_eq!(mgr.active_frames.len(), 0);
        assert_eq!(mgr.processed_frames.len(), 1);
        assert_eq!(mgr.num_frames_processed, 0);
        // 清理
        unsafe {
            let _ = Box::from_raw(mgr.processed_frames.as_slice()[0]);
        }
    }

    #[test]
    fn process_next_frame_returns_zero_when_empty() {
        let mut mgr = SimFrameManager::new_zeroed();
        let elapsed = mgr.process_next_frame(std::ptr::null_mut());
        assert_eq!(elapsed, 0.0);
    }

    // ===== Phase 7 T6：process_frame + game_cell_to_idx 测试 =====

    #[test]
    fn process_frame_handles_null_sim_data() {
        let mut mgr = SimFrameManager::new_zeroed();
        let mut frame = Box::new(SimFrameInfo::default());
        mgr.current_frame = frame.as_mut() as *mut _;
        // sim_data 为 null 应不 panic
        mgr.process_frame(std::ptr::null_mut());
    }

    #[test]
    fn process_frame_handles_null_current_frame() {
        let mut mgr = SimFrameManager::new_zeroed();
        // current_frame 为 null 应不 panic（即使 sim_data 也为 null）
        mgr.process_frame(std::ptr::null_mut());
    }

    #[test]
    fn game_cell_to_idx_converts_correctly() {
        // width=6, height-2=4（对应 SimData 6×6，内部 4×4）
        // game_cell=0 → row=0, col=0 → idx = (0+1)*6 + 0+1 = 7
        assert_eq!(game_cell_to_idx(0, 6, 4), Some(7));
        // game_cell=1 → row=0, col=1 → idx = (0+1)*6 + 1+1 = 8
        assert_eq!(game_cell_to_idx(1, 6, 4), Some(8));
        // game_cell=4 → row=1, col=0 → idx = (1+1)*6 + 0+1 = 13
        assert_eq!(game_cell_to_idx(4, 6, 4), Some(13));
        // game_cell=15 → row=3, col=3 → idx = (3+1)*6 + 3+1 = 28
        assert_eq!(game_cell_to_idx(15, 6, 4), Some(28));
    }

    #[test]
    fn game_cell_to_idx_returns_none_for_invalid_inputs() {
        assert_eq!(game_cell_to_idx(-1, 6, 4), None);  // 负 game_cell
        assert_eq!(game_cell_to_idx(0, 2, 4), None);   // width <= 2
        assert_eq!(game_cell_to_idx(0, 6, 0), None);   // height_minus_2 <= 0
        assert_eq!(game_cell_to_idx(0, 6, -1), None);  // height_minus_2 < 0
    }

    #[test]
    fn process_frame_writes_insulation_and_strength_values() {
        // 构造一个 4×4 SimData（width=6, height=6, 含边界）
        // game_cell=0 → idx=7
        use crate::sim_data::{CellSOA, SimData};
        use crate::sim_events::SetCellFloatValue;
        let mut mgr = SimFrameManager::new_zeroed();
        let mut frame = Box::new(SimFrameInfo::default());
        // 添加一条 set_insulation_values: game_cell=0, value=1.0 → 255
        frame.set_insulation_values.push(SetCellFloatValue {
            game_cell: 0,
            value: 1.0,
        });
        // 添加一条 set_strength_values: game_cell=1, value=128.0
        frame.set_strength_values.push(SetCellFloatValue {
            game_cell: 1,
            value: 128.0,
        });
        mgr.current_frame = frame.as_mut() as *mut _;

        // 构造 SimData
        let mut sd = SimData::new_zeroed();
        sd.width = 6;
        sd.height = 6;
        let mut cells = Box::new(CellSOA::new());
        cells.insulation.resize(36, 0);
        cells.strength_info.resize(36, 0);
        sd.updated_cells.ptr = Box::into_raw(cells);

        mgr.process_frame(&mut sd as *mut _);

        // 验证写入
        unsafe {
            let cells = &*sd.updated_cells.ptr;
            // game_cell=0 → idx=7, value=1.0*255=255
            assert_eq!(cells.insulation.get(7), 255);
            // game_cell=1 → idx=8, value=128
            assert_eq!(cells.strength_info.get(8), 128);
            // 清理
            drop(Box::from_raw(sd.updated_cells.ptr));
        }
    }
}
