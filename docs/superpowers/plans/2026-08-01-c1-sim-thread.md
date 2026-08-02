# C1 阶段实现计划：Sim 线程基础设施 + ProcessFrame 框架

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 实现 C1 阶段——让 sim 线程跑起来，ProcessDigPoints 真实实现，使 Dig/Destroy 功能可用。

**架构：** 新建 `c_simulation/sim_thread.rs`（Sim::Main 主循环 + Thread::Start）、`c_simulation/frame_processor.rs`（ProcessFrame 框架 + ProcessDigPoints 等非物理子函数）、`c_simulation/sim_data_ops.rs`（ClearCell/ClearBackwall/CopySimDataToGame）、`c2_physics/mod.rs`（UpdateData 桩）。改造 `a_framework/sim_frame_manager.rs`（HandleMessage 真实实现）、`a_framework/frame_sync.rs`（SimSync 真实实现）、`a_framework/save_load.rs`（handle_start 调用 Thread::Start）、`a_framework/sim_api.rs`（SIM_Shutdown join sim 线程）。

**技术栈：** Rust 1.96.1 / parking_lot 0.12 / once_cell 1.19 / std::thread

**参考源码：**
- `OxygenNotIncluded_Source/simDLL_edit/08_sim_frame_manager.c`（Sim::Main, HandleMessage, NewFrame, ProcessFrame, BeginFrameProcessing, EndFrameProcessing, ProcessNextFrame）
- `OxygenNotIncluded_Source/simDLL_edit/06_process_messages.c`（ProcessDigPoints, ProcessCellModifications, ProcessCellProperties）
- `OxygenNotIncluded_Source/simDLL_edit/11_msvcrt_ignored.c`（ClearCell, ClearBackwall, CopySimDataToGame, UpdateData, Thread::Start）
- `OxygenNotIncluded_Source/simDLL_edit/09_crashdump_logger.c`（FrameSync::SimSync, FrameSync::clear）
- `OxygenNotIncluded_Source/simDLL_edit/02_save_load.c`（PrepareGameDataUpdate, handle_start）

---

## 文件结构

### 新建文件

| 文件 | 职责 |
|------|------|
| `src/c_simulation/sim_thread.rs` | Sim::Main 主循环 + Thread::Start/SimThreadState |
| `src/c_simulation/frame_processor.rs` | ProcessFrame 框架 + ProcessDigPoints/ProcessCellModifications/ProcessCellProperties 真实实现 + 其他 Process* 桩 |
| `src/c_simulation/sim_data_ops.rs` | ClearCell/ClearBackwall/CopySimDataToGame/CopyUpdatedCellsToCells |
| `src/c2_physics/mod.rs` | update_data 桩 + update_components_data_list_only 桩 |
| `tests/c1_sim_thread_smoke.rs` | C1 集成测试 |

### 修改文件

| 文件 | 修改内容 |
|------|---------|
| `src/c_simulation/mod.rs` | 添加 pub mod sim_thread/frame_processor/sim_data_ops |
| `src/a_framework/sim_frame_manager.rs` | HandleMessage 真实实现 + NewFrame/BeginFrameProcessing/EndFrameProcessing/ProcessNextFrame/ProcessFrame 真实实现 + SimFrameInfo 结构体定义 |
| `src/a_framework/frame_sync.rs` | SimSync 真实实现 + 条件变量字段 |
| `src/a_framework/save_load.rs` | handle_start 末尾调用 Thread::Start |
| `src/a_framework/sim_api.rs` | SIM_Shutdown 中设置 exit_requested + join sim 线程 |
| `src/a_framework/sim_data.rs` | 添加 vacuum_element_idx 字段（如尚未存在）+ active_regions 字段 |
| `src/a_framework/sim_events.rs` | 确认 dig_info/mass_consumed_callbacks 等字段存在 |
| `src/a_framework/sim.rs` | 添加 exit_requested/residual_time 字段 + thread_handle 字段 |
| `src/lib.rs` | 添加 G_SIM_THREAD 全局 + 注册 c2_physics 模块 |

---

## 任务列表

### 任务 1：c2_physics 桩模块

**文件：**
- 创建：`src/c2_physics/mod.rs`
- 修改：`src/lib.rs`

- [ ] **步骤 1.1：创建 c2_physics 桩模块**

创建 `src/c2_physics/mod.rs`：

```rust
//! C2 阶段物理模拟桩实现。
//!
//! C1 阶段：所有函数为空桩，不执行任何物理计算。
//! C2 阶段：逐步替换为真实物理计算（温度/液体/气体/辐射/疾病）。

#![allow(dead_code)]

use crate::a_framework::sim_data::SimData;

/// SimBase::UpdateData 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c L41811。
/// 原版调用 UpdateTemperature/UpdatePressure/UpdateLiquid 等物理更新。
/// C1 桩：不执行任何物理计算，cell 状态保持不变。
pub fn update_data(_sim_data: &mut SimData) {
    // C2 阶段实现
}

/// SimData::UpdateComponentsDataListOnly 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c。
/// 原版更新组件数据列表（ElementConsumer/Emitter 等）。
/// C1 桩：不执行任何操作。
pub fn update_components_data_list_only(_sim_data: &mut SimData) {
    // C2 阶段实现
}

/// ConsolidateEvents 的桩实现。
///
/// 对照源码 11_msvcrt_ignored.c。
/// 原版合并 sim_events 中的重复事件。
/// C1 桩：不执行任何操作。
pub fn consolidate_events(_sim_data: &mut SimData) {
    // C2 阶段实现
}
```

- [ ] **步骤 1.2：在 lib.rs 注册 c2_physics 模块**

在 `src/lib.rs` 中添加：

```rust
pub mod c2_physics;
```

- [ ] **步骤 1.3：验证编译**

运行：`cargo build --release`
预期：编译成功，可能有 unused 警告

---

### 任务 2：SimFrameInfo 结构体定义

**文件：**
- 修改：`src/a_framework/sim_frame_manager.rs`

- [ ] **步骤 2.1：定义 SimFrameInfo 结构体**

在 `src/a_framework/sim_frame_manager.rs` 中添加 SimFrameInfo 结构体。对照源码 `08_sim_frame_manager.c` L1152（`operator_new(0x5b8)` = 1464 字节）和 HandleMessage 中引用的字段：

```rust
/// 帧信息结构体（对照源码 SimFrameInfo，大小 0x5b8 = 1464 字节）。
///
/// 包含一帧内所有消息的 vector 缓冲。
#[repr(C)]
pub struct SimFrameInfo {
    // 非物理消息 vector（C1 真实实现）
    pub dig_points: MsvcVector<DigPoint>,                  // offset 0x000
    pub cell_modifications: MsvcVector<CellModification>,  // offset 0x020
    pub cell_energy_modifications: MsvcVector<CellEnergyModification>, // offset 0x040
    pub cell_disease_modifications: MsvcVector<DigPoint>,  // offset 0x060
    pub cell_radiation_modifications: MsvcVector<DigPoint>, // offset 0x080
    pub cell_world_zone_modifications: MsvcVector<CellWorldZoneModification>, // offset 0x0A0
    pub consume_disease: MsvcVector<ConsumeDiseaseMessage>, // offset 0x0C0
    pub set_insulation_values: MsvcVector<SetValueEntry>,  // offset 0x0E0
    pub set_strength_values: MsvcVector<SetValueEntry>,    // offset 0x100
    pub set_cell_properties: MsvcVector<DigPoint>,         // offset 0x120
    pub mass_consumption_messages: MsvcVector<MassConsumptionMessage>, // offset 0x140
    pub mass_emission_messages: MsvcVector<MassEmissionMessage>, // offset 0x160
    pub modify_backwall_data_messages: MsvcVector<BackwallDataModification>, // offset 0x180
    pub modify_building_energy_messages: MsvcVector<BuildingEnergyModification>, // offset 0x1A0
    pub modify_element_chunk_adjuster_messages: MsvcVector<ElementChunkAdjusterModification>, // offset 0x1C0
    pub modify_element_chunk_energy_messages: MsvcVector<ElementChunkEnergyModification>, // offset 0x1E0
    pub move_element_chunk_messages: MsvcVector<MoveElementChunkMessage>, // offset 0x200
    pub radiation_params_modifications: MsvcVector<RadiationParamsModification>, // offset 0x220
    
    // ComponentMessages（含 adds/modifies/removes 三个子向量，每个 0x60 = 3*0x20 字节）
    pub building_heat_exchange_messages: ComponentMessages, // offset 0x240
    pub building_to_building_heat_exchange_messages: ComponentMessages, // offset 0x2A0
    pub add_building_in_contact_messages: MsvcVector<u64>, // offset 0x300
    pub element_chunk_messages: ComponentMessages,         // offset 0x320
    pub element_consumer_messages: ComponentMessages,      // offset 0x380
    pub element_emitter_messages: ComponentMessages,      // offset 0x3E0
    pub radiation_emitter_messages: ComponentMessages,    // offset 0x440
    pub disease_emitter_messages: ComponentMessages,      // offset 0x4A0
    pub disease_consumer_messages: ComponentMessages,     // offset 0x500
    
    // 调试属性
    pub debug_properties: DebugProperties,                 // offset 0x560
    
    // 帧时间
    pub elapsed_seconds: f32,                              // offset 0x570
    
    // padding 到 0x5B8
    _padding: [u8; 0x5B8 - 0x574],
}

#[repr(C)]
pub struct ComponentMessages {
    pub adds: MsvcVector<u64>,
    pub modifies: MsvcVector<u64>,
    pub removes: MsvcVector<u64>,
}

#[repr(C)]
pub struct DebugProperties {
    pub building_temperature_scale: f32,
    pub building_to_building_temperature_scale: f32,
    pub is_debug_editing: bool,
    pub pad: [u8; 3],
}

impl SimFrameInfo {
    /// 创建零初始化的 SimFrameInfo。
    pub fn new_zeroed() -> Self {
        // 使用 unsafe 零初始化，然后构造各 vector
        unsafe { std::mem::zeroed() }
    }
}

/// DigPoint 结构体（12 字节，对照源码 LAB_18001d449）。
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct DigPoint {
    pub game_cell: i32,      // offset 0x0
    pub callback_idx: i32,   // offset 0x4
    pub skip_event: u8,      // offset 0x8（在 SetCellProperties 上下文中是属性 mask）
    pub backwall: u8,        // offset 0x9（在 SetCellProperties 上下文中是 set/clear 标志）
    pub _pad: [u8; 2],       // offset 0xA
}

/// CellModification 结构体（28 字节，对照源码 L638-660）。
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct CellModification {
    pub game_cell: i32,
    pub callback_idx: i32,
    pub temperature: f32,
    pub mass: f32,
    pub disease_count: i32,
    pub element_idx: u16,
    pub replace_type: u8,
    pub disease_idx: u8,
    pub add_sub_type: u32,
}

/// CellEnergyModification 结构体（16 字节）。
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct CellEnergyModification {
    pub game_cell: i32,
    pub element_idx: u16,
    pub temperature: f32,
    pub strength: u32,
}

/// CellWorldZoneModification 结构体（8 字节）。
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct CellWorldZoneModification {
    pub game_cell: i32,
    pub zone_id: u8,
    pub _pad: [u8; 3],
}

/// SetValueEntry（8 字节，用于 SetInsulation/SetStrength）。
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct SetValueEntry {
    pub game_cell: i32,
    pub value: u32,  // 实际为 float 的位表示
}

/// BackwallDataModification 结构体（16 字节）。
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BackwallDataModification {
    pub game_cell: i32,
    pub element_idx: u16,
    pub temperature: f32,
    pub strength: u32,
}

// 其他消息结构体（C1 桩用，定义最小化）
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ConsumeDiseaseMessage { pub data: [u8; 16] } }

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MassConsumptionMessage { pub data: [u8; 16] } }

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MassEmissionMessage { pub data: [u8; 24] } }

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BuildingEnergyModification { pub data: [u8; 16] } }

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ElementChunkAdjusterModification { pub data: [u8; 16] } }

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ElementChunkEnergyModification { pub data: [u8; 8] } }

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MoveElementChunkMessage { pub data: [u8; 8] } }

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RadiationParamsModification { pub data: [u8; 8] } }
```

- [ ] **步骤 2.2：验证编译**

运行：`cargo build --release`
预期：编译成功

- [ ] **步骤 2.3：验证 SimFrameInfo 大小**

在 `src/a_framework/sim_frame_manager.rs` 的 `#[cfg(test)] mod tests` 中添加测试：

```rust
#[test]
fn sim_frame_info_size_is_0x5b8() {
    assert_eq!(std::mem::size_of::<SimFrameInfo>(), 0x5b8);
}

#[test]
fn dig_point_size_is_12() {
    assert_eq!(std::mem::size_of::<DigPoint>(), 12);
}

#[test]
fn cell_modification_size_is_28() {
    assert_eq!(std::mem::size_of::<CellModification>(), 28);
}
```

运行：`cargo test --release sim_frame_info_size -- --nocapture`
预期：PASS

---

### 任务 3：SimFrameManager 真实实现（NewFrame/BeginFrameProcessing/EndFrameProcessing/ProcessNextFrame）

**文件：**
- 修改：`src/a_framework/sim_frame_manager.rs`

- [ ] **步骤 3.1：实现 NewFrame**

对照源码 `08_sim_frame_manager.c` L1116-1174：

```rust
impl SimFrameManager {
    /// NewFrame：切换帧缓冲。
    ///
    /// 对照源码 08_sim_frame_manager.c L1116-1174。
    /// 1. 若 currentFrame 非空，放入 queuedFrames（如大小 < 30）或 framePool
    /// 2. 从 framePool 取一个作为新 currentFrame；若 framePool 为空则新建
    pub fn new_frame(&mut self) {
        let _guard = self.m_game_mutex.lock();
        
        // 若 currentFrame 非空，回收
        if !self.current_frame.is_null() {
            let queued_size = self.queued_frames.len();
            if queued_size < 30 {
                // 放入 queued_frames
                unsafe { self.queued_frames.push(self.current_frame) };
            } else {
                // 放入 frame_pool
                unsafe { self.frame_pool.push(self.current_frame) };
            }
        }
        
        // 从 frame_pool 取一个，或新建
        if self.frame_pool.is_empty() {
            // 新建 SimFrameInfo
            let frame = Box::into_raw(Box::new(SimFrameInfo::new_zeroed()));
            self.current_frame = frame;
        } else {
            // 从 frame_pool 末尾取一个
            self.current_frame = unsafe { self.frame_pool.pop().unwrap() };
        }
    }
}
```

- [ ] **步骤 3.2：实现 BeginFrameProcessing**

对照源码 L182-243：

```rust
impl SimFrameManager {
    /// BeginFrameProcessing：开始帧处理。
    ///
    /// 对照源码 08_sim_frame_manager.c L182-243。
    /// 1. 把 processed_frames 移入 frame_pool（回收）
    /// 2. 清空 processed_frames
    /// 3. 把 queued_frames 移入 active_frames（待处理）
    /// 4. 清空 queued_frames
    /// 5. num_frames_processed = 0
    /// 6. 返回 active_frames 大小
    pub fn begin_frame_processing(&mut self) -> i32 {
        let _guard = self.m_game_mutex.lock();
        
        // 1. processed_frames → frame_pool
        while let Some(frame) = self.processed_frames.pop() {
            self.frame_pool.push(frame);
        }
        // processed_frames 已清空
        
        // 3. queued_frames → active_frames
        while let Some(frame) = self.queued_frames.pop() {
            self.active_frames.push(frame);
        }
        // queued_frames 已清空
        
        // 5. num_frames_processed = 0
        self.num_frames_processed = 0;
        
        // 6. 返回 active_frames 大小
        self.active_frames.len() as i32
    }
}
```

- [ ] **步骤 3.3：实现 EndFrameProcessing**

对照源码 L247-348：

```rust
impl SimFrameManager {
    /// EndFrameProcessing：结束帧处理。
    ///
    /// 对照源码 08_sim_frame_manager.c L247-348。
    /// 1. 把 active_frames 前 num_frames_processed 个移入 processed_frames
    /// 2. 把 active_frames 中剩余未处理帧前移
    pub fn end_frame_processing(&mut self) {
        let _guard = self.m_game_mutex.lock();
        
        let processed = self.num_frames_processed as usize;
        
        // 1. active_frames[0..processed] → processed_frames
        for i in 0..processed {
            if i < self.active_frames.len() {
                let frame = self.active_frames[i];
                self.processed_frames.push(frame);
            }
        }
        
        // 2. active_frames[processed..] 前移到 active_frames[0..]
        self.active_frames.drain(0..processed);
    }
}
```

- [ ] **步骤 3.4：实现 ProcessNextFrame**

对照源码 L1903-1929：

```rust
impl SimFrameManager {
    /// ProcessNextFrame：处理下一帧。
    ///
    /// 对照源码 08_sim_frame_manager.c L1903-1929。
    /// 1. 检查 num_frames_processed < active_frames.size()
    /// 2. 取 active_frames[num_frames_processed]
    /// 3. 调用 ProcessFrame
    /// 4. 拷贝 active_regions 到 sim_data
    /// 5. num_frames_processed++
    /// 6. 返回该帧的 elapsed_seconds
    pub fn process_next_frame(&mut self, sim_data: &mut SimData) -> f32 {
        let idx = self.num_frames_processed as usize;
        if idx >= self.active_frames.len() {
            panic!("ProcessNextFrame called with no active frames");
        }
        
        let frame_ptr = self.active_frames[idx];
        let frame = unsafe { &mut *frame_ptr };
        
        // 调用 ProcessFrame
        crate::c_simulation::frame_processor::process_frame(frame, sim_data);
        
        // 拷贝 active_regions 到 sim_data
        // C1 桩：暂时跳过 active_regions 拷贝（需要 SimData 有 active_regions 字段）
        
        self.num_frames_processed += 1;
        frame.elapsed_seconds
    }
}
```

- [ ] **步骤 3.5：验证编译**

运行：`cargo build --release`
预期：编译成功

---

### 任务 4：SimDataOps（ClearCell/ClearBackwall/CopySimDataToGame）

**文件：**
- 创建：`src/c_simulation/sim_data_ops.rs`
- 修改：`src/c_simulation/mod.rs`

- [ ] **步骤 4.1：创建 sim_data_ops.rs**

创建 `src/c_simulation/sim_data_ops.rs`：

```rust
//! SimData 操作函数。
//!
//! 对照源码 11_msvcrt_ignored.c 中的 ClearCell/ClearBackwall/CopySimDataToGame。

use crate::a_framework::sim_data::SimData;
use crate::a_framework::game_data::GameData;

/// SimData::ClearCell：清除单元格元素。
///
/// 对照源码 11_msvcrt_ignored.c L23677-23723。
/// 清零 7 个字段：elementIdx/mass/temperature/diseaseIdx/diseaseCount/
/// diseaseInfestationTickCount/diseaseGrowthAccumulatedError。
pub fn clear_cell(sim_data: &mut SimData, cell_idx: i32) {
    let idx = cell_idx as usize;
    
    // elementIdx → vacuumElementIdx
    sim_data.updated_cells.element_idx.set(idx, sim_data.vacuum_element_idx);
    
    // mass → 0
    sim_data.updated_cells.mass.set(idx, 0.0);
    
    // temperature → 0
    sim_data.updated_cells.temperature.set(idx, 0.0);
    
    // diseaseIdx → 0xFF (255，表示"无疾病")
    sim_data.updated_cells.disease_idx.set(idx, 0xFF);
    
    // diseaseCount → 0
    sim_data.updated_cells.disease_count.set(idx, 0);
    
    // diseaseInfestationTickCount → 0
    sim_data.updated_cells.disease_infestation_tick_count.set(idx, 0);
    
    // diseaseGrowthAccumulatedError → 0
    sim_data.updated_cells.disease_growth_accumulated_error.set(idx, 0.0);
}

/// SimData::ClearBackwall：清除后墙元素。
///
/// 对照源码 11_msvcrt_ignored.c L23648-23673。
/// 清零 3 个字段：elementIdx/mass/temperature。
pub fn clear_backwall(sim_data: &mut SimData, cell_idx: i32) {
    let idx = cell_idx as usize;
    
    // elementIdx → backwall 默认值（vacuum）
    sim_data.backwall.element_idx.set(idx, sim_data.vacuum_element_idx);
    
    // mass → 0
    sim_data.backwall.mass.set(idx, 0.0);
    
    // temperature → 0
    sim_data.backwall.temperature.set(idx, 0.0);
}

/// SimBase::CopyUpdatedCellsToCells：将 updated_cells 拷贝到 cells。
///
/// 对照源码 11_msvcrt_ignored.c。
pub fn copy_updated_cells_to_cells(sim_data: &mut SimData) {
    sim_data.cells.copy_from(&sim_data.updated_cells);
}

/// SimBase::CopySimDataToGame：将 SimData 拷贝到 GameData。
///
/// 对照源码 11_msvcrt_ignored.c L31168-32200。
/// 1. 拷贝 cells SOA（updated_cells → game_data.cells）
/// 2. 拷贝 backwalls SOA（backwall → game_data.backwalls）
/// 3. swap sim_events 的 29 个事件 vector
/// 4. 更新 num_frames_processed
pub fn copy_sim_data_to_game(sim_data: &mut SimData, game_data: &mut GameData, frame_count: i32) {
    // 1. 拷贝 cells SOA
    game_data.cells.copy_from(&sim_data.updated_cells);
    
    // 2. 拷贝 backwalls SOA
    game_data.backwalls.copy_from(&sim_data.backwall);
    
    // 3. swap sim_events 的 29 个事件 vector
    // 对照源码 L31791-L32174
    let sim_events = unsafe { &mut *sim_data.sim_events.ptr };
    
    std::mem::swap(&mut game_data.substance_change_info, &mut sim_events.substance_change_info);
    std::mem::swap(&mut game_data.callback_info, &mut sim_events.callback_info);
    std::mem::swap(&mut game_data.spawn_falling_liquid_info, &mut sim_events.spawn_liquid_info);
    std::mem::swap(&mut game_data.unstable_cell_info, &mut sim_events.unstable_cell_info);
    std::mem::swap(&mut game_data.element_chunk_melted_info, &mut sim_events.element_chunk_melted_info);
    std::mem::swap(&mut game_data.building_melted_info, &mut sim_events.building_melted_info);
    std::mem::swap(&mut game_data.building_overheat_info, &mut sim_events.building_overheat_info);
    std::mem::swap(&mut game_data.building_no_longer_overheated_info, &mut sim_events.building_no_longer_overheated_info);
    std::mem::swap(&mut game_data.cell_melted_info, &mut sim_events.cell_melted_info);
    std::mem::swap(&mut game_data.world_damage_info, &mut sim_events.world_damage_info);
    std::mem::swap(&mut game_data.mass_consumed_callbacks, &mut sim_events.mass_consumed_callbacks);
    std::mem::swap(&mut game_data.radiation_consumed_callbacks, &mut sim_events.radiation_consumed_callbacks);
    std::mem::swap(&mut game_data.mass_emitted_callbacks, &mut sim_events.mass_emitted_callbacks);
    std::mem::swap(&mut game_data.disease_consumed_callbacks, &mut sim_events.disease_consumed_callbacks);
    std::mem::swap(&mut game_data.spawn_fx_info, &mut sim_events.spawn_fx_info);
    std::mem::swap(&mut game_data.component_state_changed_messages, &mut sim_events.component_state_changed_messages);
    std::mem::swap(&mut game_data.dig_info, &mut sim_events.dig_info);
    std::mem::swap(&mut game_data.backwall_element_changed_info, &mut sim_events.backwall_element_changed_info);
    std::mem::swap(&mut game_data.backwall_should_transition_info, &mut sim_events.backwall_should_transition_info);
    
    // spawn_ore_info 需要排序后处理（C1 桩：直接 swap）
    std::mem::swap(&mut game_data.spawn_ore_info, &mut sim_events.spawn_ore_info);
    
    // 4. 更新 num_frames_processed
    game_data.num_frames_processed = frame_count;
}
```

- [ ] **步骤 4.2：在 c_simulation/mod.rs 注册模块**

在 `src/c_simulation/mod.rs` 中添加：

```rust
pub mod sim_data_ops;
```

- [ ] **步骤 4.3：验证编译**

运行：`cargo build --release`
预期：编译可能失败，因为 SimData/CellSOA/BackwallSOA/SimEvents 的字段名需要核对

- [ ] **步骤 4.4：核对并修正字段名**

根据编译错误，核对 `src/a_framework/sim_data.rs`、`src/a_framework/sim_events.rs`、`src/a_framework/game_data.rs` 中的字段名，确保 `sim_data_ops.rs` 中引用的字段名正确。

运行：`cargo build --release`
预期：编译成功

---

### 任务 5：FrameProcessor（ProcessFrame + ProcessDigPoints 等子函数）

**文件：**
- 创建：`src/c_simulation/frame_processor.rs`
- 修改：`src/c_simulation/mod.rs`

- [ ] **步骤 5.1：创建 frame_processor.rs 框架**

创建 `src/c_simulation/frame_processor.rs`：

```rust
//! ProcessFrame 框架 + Process* 子函数。
//!
//! 对照源码 08_sim_frame_manager.c L1178-1899（ProcessFrame）和
//! 06_process_messages.c（各 Process* 子函数）。

use crate::a_framework::sim_data::SimData;
use crate::a_framework::sim_frame_manager::SimFrameInfo;
use crate::b_elements::elements_table;

/// ProcessFrame：处理一帧的所有消息。
///
/// 对照源码 08_sim_frame_manager.c L1178-1899。
/// 依次调用 38 个处理步骤（非物理子函数真实实现，物理子函数桩）。
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
    
    // 步骤 9: ProcessCellRadiationChanges（C2 桩）
    process_cell_radiation_changes(frame, sim_data);
    
    // 步骤 10: ProcessDigPoints（C1 真实实现）
    process_dig_points(frame, sim_data);
    
    // 步骤 11: ProcessCellModifications
    process_cell_modifications(frame, sim_data);
    
    // 步骤 12: 内联处理 cell_world_zone_modifications
    process_cell_world_zone_modifications(frame, sim_data);
    
    // 步骤 13-38: 其他 Component 消息处理（C2 桩）
    process_component_messages_stubs(frame, sim_data);
}

/// ProcessDigPoints：处理挖掘点。
///
/// 对照源码 06_process_messages.c L524-744。
pub fn process_dig_points(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    let sim_events = unsafe { &mut *sim_data.sim_events.ptr };
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        // 清空 dig_points
        frame.dig_points.clear_keep_capacity();
        return;
    }
    
    let dig_points_slice = frame.dig_points.as_slice();
    let count = dig_points_slice.len();
    
    for i in 0..count {
        let dig_point = dig_points_slice[i];
        
        // 1. 坐标转换：gameCell → simCell
        // 公式：simCell = (gameCell % (width-2) + 1) + (gameCell / (width-2) + 1) * width
        let internal_cell = (dig_point.game_cell % game_w + 1) 
                          + (dig_point.game_cell / game_w + 1) * sim_data.width;
        let cell_idx = internal_cell as usize;
        
        // 2. 读取元素索引
        let elem_idx = if dig_point.backwall == 0 {
            sim_data.updated_cells.element_idx.get(cell_idx)
        } else {
            sim_data.backwall.element_idx.get(cell_idx)
        };
        
        // 3. 查找元素数据
        let element = match elements_table::get_element(elem_idx as u32) {
            Some(e) => e,
            None => continue,
        };
        
        // 4. 如果 callback_idx != -1，添加回调到 simEvents[0x120]
        if dig_point.callback_idx != -1 {
            // push 到 callback_info（步长 4 字节，仅 callbackIdx）
            // C1 桩：暂时跳过（需要确认 SimEvents.callback_info 的确切类型）
        }
        
        // 5. 检查元素是否为固体（state & 3 == 3）
        if (element.state & 3) == 3 {
            // 6. 如果 !skip_event，生成 MassConsumedCallback
            if dig_point.skip_event == 0 {
                let (mass, temp, disease_idx, disease_count) = if dig_point.backwall == 0 {
                    let mass = sim_data.updated_cells.mass.get(cell_idx);
                    let temp = sim_data.updated_cells.temperature.get(cell_idx);
                    let disease_idx = sim_data.updated_cells.disease_idx.get(cell_idx);
                    let disease_count = sim_data.updated_cells.disease_count.get(cell_idx);
                    (mass, temp, disease_idx, disease_count)
                } else {
                    let mass = sim_data.backwall.mass.get(cell_idx);
                    let temp = sim_data.backwall.temperature.get(cell_idx);
                    (mass, temp, 0xFFu8, 0i32)
                };
                
                if mass > 0.0 {
                    // 反向坐标转换：simCell → gameCell
                    let game_cell_out = (internal_cell % sim_data.width - 1)
                                      + (internal_cell / sim_data.width - 1) * game_w;
                    
                    if game_cell_out >= 0 && game_cell_out < sim_data.num_game_cells {
                        // 生成 MassConsumedCallback
                        // 对照源码 L668-687：callbackIdx 字段复用存 gameCell
                        // C1 实现：push 到 mass_consumed_callbacks
                        // 需要确认 MassConsumedCallback 结构体定义
                    }
                }
            }
            
            // 7. 移除单元格元素
            if dig_point.backwall == 0 {
                crate::c_simulation::sim_data_ops::clear_cell(sim_data, internal_cell);
            } else {
                crate::c_simulation::sim_data_ops::clear_backwall(sim_data, internal_cell);
                // 生成 BackwallElementChangedInfo
                let game_cell_out = (internal_cell % sim_data.width - 1)
                                  + (internal_cell / sim_data.width - 1) * game_w;
                if game_cell_out >= 0 && game_cell_out < sim_data.num_game_cells {
                    // push 到 backwall_element_changed_info
                }
            }
            
            // 8. 生成 SubstanceChangeInfo
            let game_cell_out = (internal_cell % sim_data.width - 1)
                              + (internal_cell / sim_data.width - 1) * game_w;
            if game_cell_out >= 0 && game_cell_out < sim_data.num_game_cells {
                // push 到 substance_change_info
                // old_element_idx = 0xFFFF, new_element_idx = 0xFFFF, cell_idx = game_cell_out
            }
            
            // 9. 设置定时器位标志（timers[cell] |= 0x1F）
            let timers_idx = cell_idx;
            if timers_idx < sim_data.timers.len() {
                sim_data.timers[timers_idx] |= 0x1F;
            }
        }
    }
    
    // 清空 dig_points
    frame.dig_points.clear_keep_capacity();
}

// 其他 Process* 子函数（C1 真实实现）

fn process_set_insulation_values(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    // 对照源码 ProcessFrame L1218-1234
    // 读取 set_insulation_values，写入 updated_cells.insulation
    let slice = frame.set_insulation_values.as_slice();
    let count = slice.len();
    let game_w = sim_data.width - 2;
    if game_w <= 0 { 
        frame.set_insulation_values.clear_keep_capacity();
        return;
    }
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = (entry.game_cell % game_w + 1) 
                          + (entry.game_cell / game_w + 1) * sim_data.width;
        sim_data.updated_cells.insulation.set(internal_cell as usize, entry.value as u8);
    }
    frame.set_insulation_values.clear_keep_capacity();
}

fn process_set_strength_values(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    // 对照源码 ProcessFrame L1235-1259
    let slice = frame.set_strength_values.as_slice();
    let count = slice.len();
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.set_strength_values.clear_keep_capacity();
        return;
    }
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = (entry.game_cell % game_w + 1) 
                          + (entry.game_cell / game_w + 1) * sim_data.width;
        sim_data.updated_cells.strength_info.set(internal_cell as usize, entry.value as u8);
    }
    frame.set_strength_values.clear_keep_capacity();
}

fn process_cell_properties(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    // 对照源码 06_process_messages.c L262-269
    let slice = frame.set_cell_properties.as_slice();
    let count = slice.len();
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.set_cell_properties.clear_keep_capacity();
        return;
    }
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = (entry.game_cell % game_w + 1) 
                          + (entry.game_cell / game_w + 1) * sim_data.width;
        let idx = internal_cell as usize;
        let mask = entry.skip_event;
        let set_flag = entry.backwall;
        let current = sim_data.updated_cells.properties.get(idx);
        let new_val = if set_flag == 0 {
            current & !mask
        } else {
            current | mask
        };
        sim_data.updated_cells.properties.set(idx, new_val);
    }
    frame.set_cell_properties.clear_keep_capacity();
}

fn process_cell_modifications(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    // 对照源码 06_process_messages.c ProcessCellModifications
    let slice = frame.cell_modifications.as_slice();
    let count = slice.len();
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.cell_modifications.clear_keep_capacity();
        return;
    }
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = (entry.game_cell % game_w + 1) 
                          + (entry.game_cell / game_w + 1) * sim_data.width;
        let idx = internal_cell as usize;
        
        // 写入 element_idx, temperature, mass, disease_count, disease_idx
        sim_data.updated_cells.element_idx.set(idx, entry.element_idx);
        sim_data.updated_cells.temperature.set(idx, entry.temperature);
        sim_data.updated_cells.mass.set(idx, entry.mass);
        sim_data.updated_cells.disease_count.set(idx, entry.disease_count);
        sim_data.updated_cells.disease_idx.set(idx, entry.disease_idx);
    }
    frame.cell_modifications.clear_keep_capacity();
}

fn process_cell_world_zone_modifications(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    // 对照源码 ProcessFrame L1331-1343
    let slice = frame.cell_world_zone_modifications.as_slice();
    let count = slice.len();
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.cell_world_zone_modifications.clear_keep_capacity();
        return;
    }
    for i in 0..count {
        let entry = slice[i];
        let internal_cell = (entry.game_cell % game_w + 1) 
                          + (entry.game_cell / game_w + 1) * sim_data.width;
        if (internal_cell as usize) < sim_data.world_zones.len() {
            sim_data.world_zones[internal_cell as usize] = entry.zone_id;
        }
    }
    frame.cell_world_zone_modifications.clear_keep_capacity();
}

// C2 桩子函数

fn process_cell_energy_modifications(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    frame.cell_energy_modifications.clear_keep_capacity();
}

fn process_mass_consumption(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    frame.mass_consumption_messages.clear_keep_capacity();
}

fn process_mass_emission(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    frame.mass_emission_messages.clear_keep_capacity();
}

fn process_consume_disease(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    frame.consume_disease.clear_keep_capacity();
}

fn process_cell_disease_modifications(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    frame.cell_disease_modifications.clear_keep_capacity();
}

fn process_cell_radiation_changes(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    frame.cell_radiation_modifications.clear_keep_capacity();
}

fn process_component_messages_stubs(frame: &mut SimFrameInfo, _sim_data: &mut SimData) {
    // 清空所有 ComponentMessages 的 adds/modifies/removes
    // C2 桩：不处理，只清空
    frame.building_heat_exchange_messages.adds.clear_keep_capacity();
    frame.building_heat_exchange_messages.modifies.clear_keep_capacity();
    frame.building_heat_exchange_messages.removes.clear_keep_capacity();
    // ... 对其他 7 个 ComponentMessages 执行类似操作 ...
    frame.add_building_in_contact_messages.clear_keep_capacity();
    frame.modify_building_energy_messages.clear_keep_capacity();
    frame.modify_element_chunk_adjuster_messages.clear_keep_capacity();
    frame.modify_element_chunk_energy_messages.clear_keep_capacity();
    frame.move_element_chunk_messages.clear_keep_capacity();
    frame.radiation_params_modifications.clear_keep_capacity();
    frame.modify_backwall_data_messages.clear_keep_capacity();
}
```

- [ ] **步骤 5.2：在 c_simulation/mod.rs 注册模块**

```rust
pub mod frame_processor;
```

- [ ] **步骤 5.3：验证编译并修正字段名**

运行：`cargo build --release`
预期：编译可能失败，需要核对字段名

根据编译错误修正字段名后：
运行：`cargo build --release`
预期：编译成功

---

### 任务 6：HandleMessage 真实实现

**文件：**
- 修改：`src/a_framework/sim_frame_manager.rs`

- [ ] **步骤 6.1：实现 HandleMessage 的 Dig 分支**

对照源码 `08_sim_frame_manager.c` L784-792, L853-863：

```rust
impl SimFrameManager {
    /// HandleMessage：处理消息（两阶段分派的阶段 1）。
    ///
    /// 对照源码 08_sim_frame_manager.c L352-1115。
    pub fn handle_message(&mut self, id: u32, reader: &mut BinaryBufferReader, 
                          out_result: &mut *mut c_void) -> bool {
        let _guard = self.m_game_mutex.lock();
        
        // 确保 current_frame 非空
        if self.current_frame.is_null() {
            // 如果 current_frame 为空，创建一个
            self.current_frame = Box::into_raw(Box::new(SimFrameInfo::new_zeroed()));
        }
        let frame = unsafe { &mut *self.current_frame };
        
        match id {
            // Dig 消息 (0x31A728A2 = 833038498)
            crate::a_framework::message_handler::MessageType::Dig => {
                // 读取 12 字节：gameCell(4) + callbackIdx(4) + flags(4)
                let game_cell = reader.read_int().unwrap_or(-1);
                let callback_idx = reader.read_int().unwrap_or(-1);
                let flags = reader.read_int().unwrap_or(0);
                let skip_event = (flags & 0xFF) as u8;
                let backwall = ((flags >> 8) & 0xFF) as u8;
                
                let dig_point = DigPoint {
                    game_cell,
                    callback_idx,
                    skip_event,
                    backwall,
                    _pad: [0, 0],
                };
                frame.dig_points.push(dig_point);
                true
            }
            
            // SimFrameManager_NewGameFrame 消息
            crate::a_framework::message_handler::MessageType::SimFrameManagerNewGameFrame => {
                // 读取 elapsed_seconds (float)
                let elapsed = reader.read_float().unwrap_or(0.0);
                frame.elapsed_seconds = elapsed;
                
                // 读取 active_regions（C1 桩：跳过 active_regions 解析，数量可能为 0）
                // 对照源码 L594-636：读取 active_regions 数组
                // C1 简化：不解析 active_regions，直接调用 new_frame
                
                // 切换帧
                drop(_guard); // 释放锁，因为 new_frame 会重新加锁
                self.new_frame();
                true
            }
            
            // ModifyCell 消息
            crate::a_framework::message_handler::MessageType::ModifyCell => {
                // 读取 28 字节
                let game_cell = reader.read_int().unwrap_or(-1);
                let callback_idx = reader.read_int().unwrap_or(-1);
                let temperature = f32::from_bits(reader.read_int().unwrap_or(0) as u32);
                let mass = f32::from_bits(reader.read_int().unwrap_or(0) as u32);
                let disease_count = reader.read_int().unwrap_or(0);
                let element_idx_disease = reader.read_int().unwrap_or(0);
                let element_idx = (element_idx_disease & 0xFFFF) as u16;
                let replace_type = ((element_idx_disease >> 16) & 0xFF) as u8;
                let disease_idx = ((element_idx_disease >> 24) & 0xFF) as u8;
                let add_sub_type = reader.read_uint().unwrap_or(0);
                
                let modification = CellModification {
                    game_cell, callback_idx, temperature, mass,
                    disease_count, element_idx, replace_type, disease_idx,
                    add_sub_type,
                };
                frame.cell_modifications.push(modification);
                true
            }
            
            // SetBackwallData 消息
            crate::a_framework::message_handler::MessageType::SetBackwallData => {
                // 读取 16 字节
                let game_cell = reader.read_int().unwrap_or(-1);
                let element_idx_etc = reader.read_int().unwrap_or(0);
                let element_idx = (element_idx_etc & 0xFFFF) as u16;
                let temperature = f32::from_bits(((element_idx_etc >> 16) & 0xFFFF) as u32 | 
                    ((reader.read_int().unwrap_or(0) as u32) << 16));
                // 简化：读取剩余字节
                let _ = reader.read_int();
                let _ = reader.read_int();
                
                let modification = BackwallDataModification {
                    game_cell, element_idx, temperature: 0.0, strength: 0,
                };
                frame.modify_backwall_data_messages.push(modification);
                true
            }
            
            // SetInsulation 消息
            crate::a_framework::message_handler::MessageType::SetInsulation => {
                let game_cell = reader.read_int().unwrap_or(-1);
                let value = reader.read_uint().unwrap_or(0);
                frame.set_insulation_values.push(SetValueEntry { game_cell, value });
                true
            }
            
            // SetStrength 消息
            crate::a_framework::message_handler::MessageType::SetStrength => {
                let game_cell = reader.read_int().unwrap_or(-1);
                let value = reader.read_uint().unwrap_or(0);
                frame.set_strength_values.push(SetValueEntry { game_cell, value });
                true
            }
            
            // SetCellProperties / ClearCellProperties 消息（共用同一 ID）
            crate::a_framework::message_handler::MessageType::SetCellProperties |
            crate::a_framework::message_handler::MessageType::ClearCellProperties => {
                let game_cell = reader.read_int().unwrap_or(-1);
                let callback_idx = reader.read_int().unwrap_or(-1);
                let flags = reader.read_int().unwrap_or(0);
                let skip_event = (flags & 0xFF) as u8;
                let backwall = ((flags >> 8) & 0xFF) as u8;
                
                let entry = DigPoint {
                    game_cell, callback_idx, skip_event, backwall, _pad: [0, 0],
                };
                frame.set_cell_properties.push(entry);
                true
            }
            
            // ModifyCellWorldZone 消息
            crate::a_framework::message_handler::MessageType::ModifyCellWorldZone => {
                let game_cell = reader.read_int().unwrap_or(-1);
                let zone_id_etc = reader.read_int().unwrap_or(0);
                let zone_id = (zone_id_etc & 0xFF) as u8;
                
                let modification = CellWorldZoneModification {
                    game_cell, zone_id, _pad: [0, 0, 3],
                };
                frame.cell_world_zone_modifications.push(modification);
                true
            }
            
            _ => {
                *out_result = std::ptr::null_mut();
                false
            }
        }
    }
}
```

- [ ] **步骤 6.2：验证编译**

运行：`cargo build --release`
预期：编译成功

---

### 任务 7：FrameSync::SimSync 真实实现

**文件：**
- 修改：`src/a_framework/frame_sync.rs`

- [ ] **步骤 7.1：实现 SimSync 和条件变量**

对照源码 `09_crashdump_logger.c` L655-690：

```rust
use parking_lot::{Condvar, Mutex};

pub struct FrameSync {
    // ... 现有字段 ...
    pub m_sim_ready_to_swap: Mutex<bool>,
    pub m_sim_cond: Condvar,
    pub m_game_cond: Condvar,
}

impl FrameSync {
    /// SimSync：sim 线程通知游戏线程数据已就绪，并等待游戏线程消费。
    ///
    /// 对照源码 09_crashdump_logger.c L655-690。
    pub fn sim_sync(&self) {
        let mut ready = self.m_sim_ready_to_swap.lock();
        *ready = true;
        self.m_game_cond.notify_one();
        while *ready {
            self.m_sim_cond.wait(&mut ready);
        }
    }
    
    /// GameSync：游戏线程消费数据并通知 sim 线程。
    /// 
    /// 对照源码 09_crashdump_logger.c。
    pub fn game_sync(&self) {
        let mut ready = self.m_sim_ready_to_swap.lock();
        *ready = false;
        self.m_sim_cond.notify_one();
    }
}
```

- [ ] **步骤 7.2：验证编译**

运行：`cargo build --release`
预期：编译成功

---

### 任务 8：Sim::Main 主循环 + Thread::Start

**文件：**
- 创建：`src/c_simulation/sim_thread.rs`
- 修改：`src/c_simulation/mod.rs`、`src/a_framework/save_load.rs`、`src/a_framework/sim_api.rs`、`src/lib.rs`

- [ ] **步骤 8.1：创建 sim_thread.rs**

创建 `src/c_simulation/sim_thread.rs`：

```rust
//! Sim 线程主循环和线程管理。
//!
//! 对照源码 08_sim_frame_manager.c L2086-2168（Sim::Main）和
//! 11_msvcrt_ignored.c L44032-44105（Thread::Start）。

use std::thread::{self, JoinHandle};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::a_framework::sim::Sim;

/// sim 线程入口：Sim::Main 主循环。
///
/// 对照源码 08_sim_frame_manager.c L2086-2168。
pub fn sim_main(sim_ptr: *mut Sim, exit_requested: *const AtomicBool) {
    // 设置 MXCSR（FTZ 模式）—— Rust 中使用 std::arch::x86_64::_mm_setcsr
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let mut csr = std::arch::x86_64::_mm_getcsr();
        csr |= 0x8000; // FTZ
        std::arch::x86_64::_mm_setcsr(csr);
    }
    
    let sim = unsafe { &mut *sim_ptr };
    
    while unsafe { (*exit_requested).load(Ordering::SeqCst) == false } {
        // 1. 锁 m_sim_mutex
        let _sim_guard = sim.frame_sync.m_sim_mutex.lock();
        
        // 获取 sim_data 和 game_data 指针
        let sim_data_ptr = crate::globals::G_SIM_DATA.lock().0;
        if sim_data_ptr.is_null() {
            tracing::error!("Sim::Main: gSimData is null");
            break;
        }
        
        let game_data_sim_ptr = sim.frame_sync.m_sim_data;
        let game_data_game_ptr = sim.frame_sync.m_game_data;
        
        if game_data_sim_ptr.is_null() || game_data_game_ptr.is_null() {
            tracing::error!("Sim::Main: game_data pointers are null");
            break;
        }
        
        // ConduitTemperatureManager::ReleaseQueuedHandles（C1 桩）
        
        // 2. BeginFrameProcessing
        let sim_frame_manager_ptr = sim.sim_frame_manager_ptr();
        let frame_count = unsafe { (*sim_frame_manager_ptr).begin_frame_processing() };
        
        let sim_data = unsafe { &mut *sim_data_ptr };
        
        if frame_count < 1 {
            // 无帧处理：只更新组件数据列表
            crate::c2_physics::update_components_data_list_only(sim_data);
        } else {
            for _ in 0..frame_count {
                // 3. ProcessNextFrame
                let elapsed = unsafe { (*sim_frame_manager_ptr).process_next_frame(sim_data) };
                
                if elapsed <= 0.0 {
                    // 时间步长 <= 0：直接拷贝 updated_cells → cells
                    crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);
                    crate::c2_physics::update_components_data_list_only(sim_data);
                } else {
                    // 计算子步数
                    let total_time = elapsed + sim.residual_time;
                    let sub_steps = std::cmp::max(1, (total_time * 5.0) as u32);
                    sim.residual_time = (total_time - sub_steps as f32 * 0.2).max(0.0);
                    
                    // 4. 按子步循环调用 UpdateData（C1 桩）
                    for _ in 0..sub_steps {
                        crate::c2_physics::update_data(sim_data);
                    }
                }
            }
        }
        
        // 5. EndFrameProcessing
        unsafe { (*sim_frame_manager_ptr).end_frame_processing(); }
        
        // 6. CopySimDataToGame
        let game_data_sim = unsafe { &mut *game_data_sim_ptr };
        crate::c_simulation::sim_data_ops::copy_sim_data_to_game(sim_data, game_data_sim, frame_count);
        
        // 7. 解锁 m_sim_mutex（_sim_guard drop）
        
        // 8. FrameSync::SimSync
        sim.frame_sync.sim_sync();
    }
    
    tracing::info!("Sim::Main exited");
}

/// Thread::Start：启动 sim 线程。
///
/// 对照源码 11_msvcrt_ignored.c L44032-44105。
/// 返回 JoinHandle 供 SIM_Shutdown join。
pub fn thread_start(sim_ptr: *mut Sim, exit_requested: *const AtomicBool) -> JoinHandle<()> {
    let sim_ptr = sim_ptr as usize;
    let exit_requested = exit_requested as usize;
    
    thread::Builder::new()
        .name("SimThread".to_string())
        .spawn(move || {
            sim_main(sim_ptr as *mut Sim, exit_requested as *const AtomicBool);
        })
        .expect("failed to spawn sim thread")
}
```

- [ ] **步骤 8.2：在 c_simulation/mod.rs 注册模块**

```rust
pub mod sim_thread;
```

- [ ] **步骤 8.3：在 lib.rs 添加 G_SIM_THREAD 全局**

在 `src/lib.rs` 中添加：

```rust
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;

/// sim 线程退出标志。
pub static G_SIM_EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// sim 线程句柄。
pub static G_SIM_THREAD: parking_lot::Mutex<Option<JoinHandle<()>>> = parking_lot::Mutex::new(None);
```

- [ ] **步骤 8.4：在 Sim 结构体添加 residual_time 字段**

在 `src/a_framework/sim.rs` 中添加：

```rust
pub struct Sim {
    // ... 现有字段 ...
    pub residual_time: f32,
}

impl Sim {
    pub fn sim_frame_manager_ptr(&self) -> *mut SimFrameManager {
        // SimFrameManager 位于 Sim 偏移 0x50
        unsafe { (self as *const Sim as *mut u8).add(0x50) as *mut SimFrameManager }
    }
}
```

- [ ] **步骤 8.5：在 handle_start 中调用 Thread::Start**

在 `src/a_framework/save_load.rs` 的 `handle_start` 函数末尾，将 Thread::Start 桩替换为真实实现：

```rust
// L252: Thread::Start(gSim)
// 启动 sim 线程
{
    let sim_ptr = crate::globals::G_SIM.lock().0;
    if !sim_ptr.is_null() {
        let exit_requested = &crate::G_SIM_EXIT_REQUESTED as *const AtomicBool;
        crate::G_SIM_EXIT_REQUESTED.store(false, std::sync::atomic::Ordering::SeqCst);
        let handle = crate::c_simulation::sim_thread::thread_start(sim_ptr, exit_requested);
        *crate::G_SIM_THREAD.lock() = Some(handle);
        tracing::info!("Sim thread started");
    } else {
        tracing::error!("Start: gSim is null, cannot start sim thread");
    }
}
```

- [ ] **步骤 8.6：在 SIM_Shutdown 中 join sim 线程**

在 `src/a_framework/sim_api.rs` 的 `SIM_Shutdown` 函数中：

```rust
#[no_mangle]
pub extern "C" fn SIM_Shutdown() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        tracing::info!("SIM_Shutdown called");
        
        // 设置退出标志并 join sim 线程
        crate::G_SIM_EXIT_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(handle) = crate::G_SIM_THREAD.lock().take() {
            let _ = handle.join();
            tracing::info!("Sim thread joined");
        }
        
        logger::flush();
    }));
}
```

- [ ] **步骤 8.7：验证编译**

运行：`cargo build --release`
预期：编译成功

---

### 任务 9：集成测试

**文件：**
- 创建：`tests/c1_sim_thread_smoke.rs`

- [ ] **步骤 9.1：创建集成测试**

创建 `tests/c1_sim_thread_smoke.rs`：

```rust
//! C1 阶段集成测试：Sim 线程 + Dig 消息完整流程。

use simdll_rs::*;
use std::os::raw::{c_int, c_schar, c_void};
use std::ptr;

const LIB_TESTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn setup() {
    SIM_Initialize(None);
}

fn teardown() {
    crate::a_framework::save_load::clean_up();
    // 关闭 sim 线程
    crate::G_SIM_EXIT_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(handle) = crate::G_SIM_THREAD.lock().take() {
        let _ = handle.join();
    }
}

#[test]
fn dig_message_clears_cell() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    setup();
    
    // AllocateCells: width=4, height=4
    let mut buf = Vec::new();
    buf.extend_from_slice(&4i32.to_le_bytes());
    buf.extend_from_slice(&4i32.to_le_bytes());
    buf.push(0u8);
    buf.push(0u8);
    SIM_HandleMessage(
        crate::a_framework::message_handler::MessageType::AllocateCells as c_int,
        buf.len() as c_int,
        buf.as_ptr() as *mut c_schar,
    );
    
    // Load（需要真实存档数据，这里用最小化存档）
    // ... 构造最小存档 ...
    
    // Start
    SIM_HandleMessage(
        crate::a_framework::message_handler::MessageType::Start as c_int,
        0,
        ptr::null_mut(),
    );
    
    // 等待 sim 线程启动
    std::thread::sleep(std::time::Duration::from_millis(100));
    
    // 发送 NewGameFrame 消息
    let mut frame_buf = Vec::new();
    frame_buf.extend_from_slice(&0.2f32.to_le_bytes()); // elapsed_seconds
    SIM_HandleMessage(
        crate::a_framework::message_handler::MessageType::SimFrameManagerNewGameFrame as c_int,
        frame_buf.len() as c_int,
        frame_buf.as_ptr() as *mut c_schar,
    );
    
    // 发送 Dig 消息（gameCell=0, callbackIdx=-1, flags=0）
    let mut dig_buf = Vec::new();
    dig_buf.extend_from_slice(&0i32.to_le_bytes());      // gameCell
    dig_buf.extend_from_slice(&(-1i32).to_le_bytes());   // callbackIdx
    dig_buf.extend_from_slice(&0i32.to_le_bytes());      // flags
    SIM_HandleMessage(
        crate::a_framework::message_handler::MessageType::Dig as c_int,
        dig_buf.len() as c_int,
        dig_buf.as_ptr() as *mut c_schar,
    );
    
    // 发送另一个 NewGameFrame 触发帧处理
    SIM_HandleMessage(
        crate::a_framework::message_handler::MessageType::SimFrameManagerNewGameFrame as c_int,
        frame_buf.len() as c_int,
        frame_buf.as_ptr() as *mut c_schar,
    );
    
    // 等待 sim 线程处理
    std::thread::sleep(std::time::Duration::from_millis(200));
    
    // 验证 cell 被清除（element_idx == vacuum_element_idx）
    // 需要访问 gSimData 验证
    let sim_data_ptr = crate::globals::G_SIM_DATA.lock().0;
    assert!(!sim_data_ptr.is_null());
    let sim_data = unsafe { &*sim_data_ptr };
    
    // gameCell=0 → internal_cell = (0 % 2 + 1) + (0 / 2 + 1) * 4 = 1 + 4 = 5
    let internal_cell = 5usize;
    let elem_idx = sim_data.updated_cells.element_idx.get(internal_cell);
    assert_eq!(elem_idx, sim_data.vacuum_element_idx, "cell should be cleared to vacuum");
    
    teardown();
}

#[test]
fn sim_thread_starts_and_stops_cleanly() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    setup();
    
    // AllocateCells + Start
    let mut buf = Vec::new();
    buf.extend_from_slice(&4i32.to_le_bytes());
    buf.extend_from_slice(&4i32.to_le_bytes());
    buf.push(0u8);
    buf.push(0u8);
    SIM_HandleMessage(
        crate::a_framework::message_handler::MessageType::AllocateCells as c_int,
        buf.len() as c_int,
        buf.as_ptr() as *mut c_schar,
    );
    SIM_HandleMessage(
        crate::a_framework::message_handler::MessageType::Start as c_int,
        0,
        ptr::null_mut(),
    );
    
    // 验证 sim 线程已启动
    std::thread::sleep(std::time::Duration::from_millis(100));
    let thread_handle = crate::G_SIM_THREAD.lock();
    assert!(thread_handle.is_some(), "sim thread should be started");
    
    teardown();
    
    // 验证 sim 线程已停止
    let thread_handle = crate::G_SIM_THREAD.lock();
    assert!(thread_handle.is_none(), "sim thread should be joined after teardown");
}
```

- [ ] **步骤 9.2：运行测试**

运行：`cargo test --release --test c1_sim_thread_smoke -- --nocapture`
预期：测试通过

---

### 任务 10：回归测试和最终验证

- [ ] **步骤 10.1：运行所有单元测试**

运行：`cargo test --release`
预期：所有测试通过（含原有测试 + 新增 C1 测试）

- [ ] **步骤 10.2：编译 Release 构建**

运行：`cargo build --release`
预期：编译成功，生成 SimDLL.dll

- [ ] **步骤 10.3：验证 SimDLL.dll 生成**

检查文件 `target/release/SimDLL.dll` 存在且时间戳为最新

- [ ] **步骤 10.4：通知用户部署测试**

告知用户：C1 阶段实现完成，请部署 `target/release/SimDLL.dll` 后测试 Sandbox Destroy 工具和小人挖掘功能。

---

## 自检清单

- [ ] 规格覆盖度：所有 C1 范围项都有对应任务
- [ ] 占位符扫描：无 TODO/待定
- [ ] 类型一致性：字段名跨任务一致
- [ ] dig_info 流转链路已实现（CopySimDataToGame 中 swap）
- [ ] ClearCell 清零 7 个字段
- [ ] ClearBackwall 清零 3 个字段
- [ ] 坐标转换公式正确（gameCell → simCell）
- [ ] MassConsumedCallback 生成条件正确（solid + !skip_event + mass>0）
- [ ] sim 线程启动/停止流程完整
- [ ] HandleMessage 实现 8 个非物理消息分支
- [ ] ProcessFrame 调用顺序正确
