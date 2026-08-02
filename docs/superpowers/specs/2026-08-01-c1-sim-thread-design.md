# C1 阶段设计：Sim 线程基础设施 + ProcessFrame 框架

## 背景与目标

### 当前状态

A 类（底层框架）、B 类（元素地基）已完成，存档加载成功。但以下问题待解决：

- Sandbox 模式的 Destroy 工具无法删除自然方块、气体、液体
- 小人挖掘自然方块时陷入持续重复 dig 动画

### 根因

1. `SimFrameManager::handle_message` 是 A3 桩，对所有消息返回 false，Dig 消息被丢弃
2. `process_next_frame` 是桩返回 0.0，即使 emplace 了也不会执行挖掘
3. C# 端 `Game.cs` L1138-1148 消费 `digInfo` 调用 `OnDigComplete`，dig_info 永远为空导致小人动画无限重复

### 目标

实现 C1 阶段：让 sim 线程跑起来，ProcessDigPoints 真实实现，使 Dig/Destroy 功能可用。

## 范围划分

### C1 阶段范围（本设计）

1. `Sim::Main` 主循环
2. `SimFrameManager::HandleMessage` 真实实现（非物理消息的 emplace）
3. `SimFrameManager::ProcessFrame` 框架
4. `ProcessDigPoints` / `ProcessCellModifications` / `ProcessCellProperties` 真实实现
5. `SimData::ClearCell` / `ClearBackwall`
6. `CopySimDataToGame`
7. `Thread::Start` 启动 sim 线程
8. C2 的 Process* 子函数做桩（清空对应 vector，不处理）
9. `SimBase::UpdateData` 桩（调用 `c2_physics::update_data`，不改变物理状态）

### C2 阶段范围（未来）

- `UpdateTemperature` / `UpdateLiquid` / `UpdatePressure` 等真实物理计算
- 涉及物理的 Process* 子函数真实实现

## 目录结构

```
src/
├── a_framework/    (A 类：已完成)
├── b_elements/     (B 类：已完成)
├── c_simulation/   (C1：Sim 线程 + ProcessFrame + 非物理 Process* 子函数)
│   ├── mod.rs
│   ├── sim_thread.rs        (Sim::Main + Thread::Start)
│   ├── frame_processor.rs   (ProcessFrame 框架 + ProcessDigPoints/CellModifications/CellProperties)
│   ├── sim_data_ops.rs      (ClearCell/ClearBackwall/CopySimDataToGame)
│   └── conduit_temperature.rs (已有桩，保留)
├── c2_physics/     (C2：物理模拟，桩实现)
│   └── mod.rs               (update_data 桩 + 物理 Process* 桩)
└── lib.rs
```

## 关键技术决策

### 决策 1：Sim 线程实现方式

采用 `std::thread::spawn` 启动 sim 线程运行 `Sim::Main` 主循环。

- 线程句柄存全局 `G_SIM_THREAD: OnceCell<JoinHandle<()>>`
- `SIM_Shutdown` 中设置 `mExitRequested = true` 并 `join` sim 线程
- 严格遵循原版架构（独立 sim 线程），避免阻塞 C# 主线程

### 决策 2：线程同步机制

用 `parking_lot::Mutex` + `parking_lot::Condvar` 替代原版 `std::mutex` + `condition_variable`：

- `FrameSync` 的 `mSimMutex` / `mGameMutex` 保护帧缓冲
- `SimSync` / `GameSync` 用条件变量实现帧同步
- `HandleMessage` 中 emplace 到 `currentFrame` 时锁 `mGameMutex`

### 决策 3：HandleMessage 实现范围

**真实实现**（C1 必须）：
- `Dig` (833038498)
- `ModifyCell` (-1252920804)
- `SetInsulation` / `SetStrength` / `SetCellProperties` / `ClearCellProperties`
- `ModifyCellWorldZone`
- `SetBackwallData` (-605421515)
- `SimFrameManager_NewGameFrame`（帧切换消息，必须真实实现）

**桩实现**（C1 桩，C2 实现）：
- `ModifyCellEnergy` / `MassConsumption` / `MassEmission` / `ConsumeDisease`
- `CellDiseaseModification` / `CellRadiationModification`
- `ElementChunk` 系列 / `BuildingHeatExchange` 系列
- `ElementConsumer/Emitter` 系列 / `DiseaseEmitter/Consumer` 系列 / `RadiationEmitter` 系列

### 决策 4：ProcessFrame 子函数实现范围

**真实实现**（C1）：
- `ProcessDigPoints`
- `ProcessCellModifications`
- `ProcessCellProperties`
- cellWorldZoneModifications 处理

**桩实现**（C1 桩，C2 实现）：
- `ProcessCellEnergyModifications` / `ProcessMassConsumption` / `ProcessMassEmission`
- `ProcessConsumeDisease` / `ProcessCellRadiationChanges`
- `ProcessBuildingHeatExchangeMessages` / `ProcessElementChunkMessages`

**桩行为**：清空对应 vector（`clear_keep_capacity`），不执行任何实际处理。

### 决策 5：SimBase::UpdateData 桩行为

- 调用 `c2_physics::update_data(sim_data)` 空函数
- 不执行任何温度/液体/气体计算
- cell 状态保持不变（只有 `ProcessDigPoints` 等非物理操作会改变 cell）

## 数据流与实现细节

### Sim::Main 主循环流程

对照源码 `08_sim_frame_manager.c` L2086-2168：

```rust
fn sim_main(sim: &Sim) {
    // 设置 MXCSR（浮点异常掩码）
    loop {
        if sim.exit_requested.load() { break; }

        // 1. 锁 mSimMutex
        let _sim_guard = sim.frame_sync.m_sim_mutex.lock();

        let sim_data_ptr = sim.frame_sync.m_sim_data;
        let game_data_ptr = sim.frame_sync.m_game_data;

        // ConduitTemperatureManager::ReleaseQueuedHandles（C1 桩）

        // 2. BeginFrameProcessing → 返回待处理帧数
        let frame_count = sim_frame_manager.begin_frame_processing();

        if frame_count < 1 {
            // 无帧处理：只更新组件数据列表
            sim_data.update_components_data_list_only();
        } else {
            for _ in 0..frame_count {
                // 3. ProcessNextFrame → 返回 elapsedSeconds
                let elapsed = sim_frame_manager.process_next_frame(sim_data);

                if elapsed <= 0.0 {
                    sim_base::copy_updated_cells_to_cells(sim_data);
                    sim_data.update_components_data_list_only();
                } else {
                    // 计算子步数：elapsed * 5（每 0.2s 一个子步）
                    let total_time = elapsed + sim.residual_time;
                    let sub_steps = std::cmp::max(1, (total_time * 5.0) as u32);
                    sim.residual_time = (total_time - sub_steps as f32 * 0.2).max(0.0);

                    // 4. 按子步循环调用 UpdateData（C1 桩）
                    for _ in 0..sub_steps {
                        sim_base::update_data(sim_data);
                    }
                }
            }
        }

        // 5. EndFrameProcessing
        sim_frame_manager.end_frame_processing();

        // 6. CopySimDataToGame
        sim_base::copy_sim_data_to_game(sim_data, sim_data_ptr, game_data_ptr, frame_count);

        // 7. 解锁 mSimMutex（_sim_guard drop）

        // 8. FrameSync::SimSync
        sim.frame_sync.sim_sync();
    }
}
```

### HandleMessage 的 Dig 分支

对照源码 `08_sim_frame_manager.c` L784-792, L853-863：

```rust
pub fn handle_message(&mut self, id: u32, reader: &mut BinaryBufferReader,
                      out_result: &mut *mut c_void) -> bool {
    let _guard = self.m_game_mutex.lock();

    match id {
        MessageType::Dig => {
            let game_cell = reader.read_int().unwrap_or(-1);
            let callback_idx = reader.read_int().unwrap_or(-1);
            let flags = reader.read_int().unwrap_or(0);
            let skip_event = (flags & 0xFF) != 0;
            let backwall = ((flags >> 8) & 0xFF) != 0;

            let dig_point = DigPoint { game_cell, callback_idx, skip_event, backwall, _pad: [0, 0] };
            self.current_frame.dig_points.push(dig_point);
            true
        }
        MessageType::SimFrameManager_NewGameFrame => {
            let elapsed = reader.read_float().unwrap_or(0.0);
            // 读取 activeRegions
            self.current_frame.elapsed_seconds = elapsed;
            self.new_frame();
            true
        }
        // ModifyCell / SetInsulation / SetStrength / SetCellProperties / ClearCellProperties
        // ModifyCellWorldZone / SetBackwallData（真实实现，模式同 Dig）
        _ => {
            *out_result = std::ptr::null_mut();
            false
        }
    }
}
```

### ProcessDigPoints 实现

对照源码 `06_process_messages.c` L524-744：

```rust
fn process_dig_points(frame: &SimFrameInfo, sim_data: &mut SimData,
                      sim_events: &mut SimEvents) {
    for dig_point in frame.dig_points.as_slice() {
        // 1. 坐标转换：gameCell → 内部 cellIdx
        let game_w = sim_data.width - 2;
        if game_w <= 0 { continue; }
        let internal_cell = (dig_point.game_cell % game_w + 1)
                          + (dig_point.game_cell / game_w + 1) * sim_data.width;

        // 2. 读取当前元素索引
        let elem_idx = if !dig_point.backwall {
            sim_data.updated_cells.element_idx[internal_cell as usize]
        } else {
            sim_data.backwall.element_idx[internal_cell as usize]
        };

        // 3. 查找元素数据
        let element = elements_table::get_element(elem_idx);

        // 4. 如果有 callbackIdx != -1，添加回调

        // 5. 检查元素是否为固体（state & 3 == 3）
        if (element.state & 3) == 3 {
            // 6. 如果 !skipEvent，生成 MassConsumedCallback
            if !dig_point.skip_event {
                let (mass, temp, disease_idx, disease_count) = if !dig_point.backwall {
                    (sim_data.updated_cells.mass[internal_cell as usize],
                     sim_data.updated_cells.temperature[internal_cell as usize],
                     sim_data.updated_cells.disease_idx[internal_cell as usize],
                     sim_data.updated_cells.disease_count[internal_cell as usize])
                } else {
                    (sim_data.backwall.mass[internal_cell as usize],
                     sim_data.backwall.temperature[internal_cell as usize],
                     0xFF, 0)
                };

                if mass > 0.0 {
                    let game_cell_out = (internal_cell % sim_data.width - 1)
                                      + (internal_cell / sim_data.width - 1) * game_w;
                    if game_cell_out >= 0 && game_cell_out < sim_data.num_game_cells {
                        sim_events.mass_consumed_callbacks.push(MassConsumedCallback {
                            callback_idx: game_cell_out,
                            elem_idx,
                            disease_idx,
                            mass,
                            temperature: temp,
                            disease_count,
                        });
                    }
                }
            }

            // 7. 移除单元格元素
            if !dig_point.backwall {
                sim_data.clear_cell(internal_cell);
            } else {
                sim_data.clear_backwall(internal_cell);
                sim_events.backwall_element_changed_info.push(...);
            }

            // 8. 生成 SubstanceChangeInfo
            sim_events.substance_change_info.push(SubstanceChangeInfo {
                cell_idx: dig_point.game_cell,
                old_element_idx: 0xFFFF,
                new_element_idx: 0xFFFF,
            });

            // 9. 设置定时器位标志
            sim_data.timers[internal_cell as usize] |= 0x1F;
        }
    }
}
```

### ClearCell 实现

对照源码 `11_msvcrt_ignored.c` L23677-23723：

```rust
pub fn clear_cell(sim_data: &mut SimData, cell_idx: i32) {
    let idx = cell_idx as usize;
    sim_data.updated_cells.element_idx[idx] = sim_data.vacuum_element_idx;
    sim_data.updated_cells.mass[idx] = 0.0;
    sim_data.updated_cells.temperature[idx] = 0.0;
    sim_data.updated_cells.disease_idx[idx] = 0xFF;
    sim_data.updated_cells.disease_count[idx] = 0;
    sim_data.updated_cells.disease_count2[idx] = 0;
    sim_data.updated_cells.accumulated_disease_error[idx] = 0.0;
}
```

### CopySimDataToGame 流程

对照源码 `11_msvcrt_ignored.c`：

```rust
fn copy_sim_data_to_game(sim_data: &SimData, sim_data_ptr: *mut SimData,
                         game_data_ptr: *mut GameData, frame_count: i32) {
    let game_data = unsafe { &mut *game_data_ptr };

    // 1. 拷贝 cells SOA
    game_data.cells.copy_from(&sim_data.updated_cells);

    // 2. 拷贝 backwalls SOA
    game_data.backwalls.copy_from(&sim_data.backwall);

    // 3. 拷贝 sim_events 到 game_data 的 29 个事件 vector
    game_data.substance_change_info.swap(&mut sim_data.sim_events.substance_change_info);
    // ... 对 29 个 vector 执行类似操作 ...

    // 4. 拷贝 dig_info（来源待查证，见下方未知点）
    // 5. 更新 num_frames_processed
    game_data.num_frames_processed = frame_count;
}
```

## dig_info 流转链路（已确认）

源码研究已确认 `dig_info` 的流转链路：

1. **物理/游戏逻辑执行期间**：dig 事件被 push 到 `SimData.simEvents->digInfo`（偏移 0x220，类型 `vector<SpawnOreInfo>`，SpawnOreInfo 大小 20 字节）
2. **每帧末尾 `SimBase::CopySimDataToGame`**（11_msvcrt_ignored.c L32100-L32122）：通过 `std::swap` 把 `SimEvents.digInfo` 整个 vector 内容转移到 `GameData.digInfo`，SimEvents 中的 vector 变空
3. **`PrepareGameDataUpdate`**（02_save_load.c L839-L847）：从 `GameData.digInfo` 取出 `_Myfirst` 指针和元素数 `(end-begin)/20`，写入全局 `gGameDataUpdate.digInfo` 和 `gGameDataUpdate.numDigInfo`，供 C# 侧通过 FFI 读取

**注意**：源码中 `ProcessDigPoints` 生成的是 `massConsumedCallbacks`（MassConsumedCallback），而非 `digInfo`（SpawnOreInfo）。`digInfo` 的 push 位置未在 `ProcessDigPoints` 中找到，可能在：
- `UpdateTemperature` 等物理更新函数中（当固体被挖掘后，温度更新触发矿石生成）
- 或由 C# 端通过其他消息直接写入

### C1 实现时的处理方案

1. 实现 `ProcessDigPoints` 生成 `massConsumedCallbacks`（已确认的路径）
2. `CopySimDataToGame` 中实现 `digInfo` 的 swap（保持原版行为）
3. `digInfo` vector 在 C1 阶段保持空（因为没有物理更新触发矿石生成）
4. 部署测试验证 Dig 是否生效（cell 是否被清除、小人动画是否停止）
5. 如果 C# 端仍然报"小人挖掘动画无限重复"，说明 `dig_info` 必须有内容，再深入源码追查其 push 位置

这是务实的增量验证策略：先实现已确认的部分，观察游戏表现，再根据反馈决定是否需要追查 `dig_info` 的 push 位置。

## 测试策略

### 单元测试

- `SimFrameManager::HandleMessage` 的 Dig emplace（验证 dig_points 增加）
- `ProcessDigPoints` 的 cell 清除逻辑（验证 element_idx 变为 vacuum）
- `ClearCell` / `ClearBackwall`（验证各字段清零）
- `HandleMessage` 的 ModifyCell / SetBackwallData 等其他非物理消息 emplace

### 集成测试

完整流程验证：
```
SIM_Initialize → AllocateCells → Load → Start
  → 发送 Dig 消息
  → 等待 sim 线程处理
  → 验证 cell 被清除（element_idx == vacuum_element_idx）
  → 验证 mass_consumed_callbacks 非空
```

### 线程安全测试

- 多个 `SIM_HandleMessage` 并发调用不 panic
- sim 线程与游戏线程正确同步（无死锁）

## 验证标准

C1 阶段完成后，应满足：

1. `cargo test --release` 全部通过（含新增 C1 测试）
2. `cargo build --release` 生成 SimDLL.dll
3. 部署后载入存档，Sandbox Destroy 工具能删除自然方块
4. 小人挖掘自然方块后动画停止（dig_info 来源确认后）
5. 游戏不崩溃，无 NRE 异常

## 后续阶段

### C2 阶段（物理模拟）

- `UpdateTemperature` / `UpdateLiquid` / `UpdatePressure` 等真实物理计算
- 涉及物理的 Process* 子函数真实实现
- `c2_physics` 文件夹内的模块逐步替换桩实现

## 参考源码文件

| 函数 | 源码文件 | 行号 |
|------|---------|------|
| Sim::Main | 08_sim_frame_manager.c | 2086 |
| Sim::Sim（构造） | 08_sim_frame_manager.c | 1963 |
| SimFrameManager::HandleMessage | 08_sim_frame_manager.c | 352 |
| SimFrameManager::NewFrame | 08_sim_frame_manager.c | 1116 |
| SimFrameManager::ProcessFrame | 08_sim_frame_manager.c | 1178 |
| SimFrameManager::ProcessNextFrame | 08_sim_frame_manager.c | 1903 |
| SimFrameManager::BeginFrameProcessing | 08_sim_frame_manager.c | 182 |
| SimFrameManager::EndFrameProcessing | 08_sim_frame_manager.c | 247 |
| ProcessDigPoints | 06_process_messages.c | 524 |
| SimData::ClearCell | 11_msvcrt_ignored.c | 23677 |
| SimData::ClearBackwall | 11_msvcrt_ignored.c | 23648 |
| SimBase::UpdateData | 11_msvcrt_ignored.c | 41811 |
| Thread::Start | 11_msvcrt_ignored.c | 44032 |
