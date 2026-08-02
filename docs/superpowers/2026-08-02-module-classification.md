# simDLL_RS 重写模块全量分类（2026-08-02）

分类依据：模块在模拟链路中的职责层次。

- **A 类 — 框架与数据定义**：不涉及模拟逻辑，仅提供容器/ABI/数据结构镜像/生命周期。
- **B 类 — 元素/病菌数据**：元素表与静态数据。
- **C1 类 — 消息/组件/句柄层（接线层）**：C# 与格子物理之间的"接线"——消息路由、
  组件注册、句柄分配、温度权威；不直接迭代格子状态。
- **C2 类 — 格子级物理**：直接迭代每个格子的状态演化（液体/气体/温度/相变/辐射/疾病）。

## A 类 — 框架与数据定义

| 模块 | 内容 | 状态 |
|------|------|------|
| a_framework/abi.rs | 消息回调 ABI | 完成 |
| a_framework/buffer.rs | BinaryBufferReader/Writer | 完成 |
| a_framework/stl_shim.rs | MsvcVector/MsvcMutex/UniquePtr | 完成 |
| a_framework/vector_math.rs | Vector2f/Vector4f | 完成 |
| a_framework/logger.rs | 日志（写 日志/ 目录） | 完成 |
| a_framework/crashdump.rs | 崩溃转储 | 完成 |
| a_framework/sysinfo.rs | 系统信息 | 完成 |
| a_framework/game_data.rs | GameData 1024B 镜像 + 25 事件容器 | 完成 |
| a_framework/game_data_update.rs | GDU 496B 镜像 | 完成 |
| a_framework/sim_data.rs | SimData 0x890 镜像 + 组件容器 | 完成 |
| a_framework/sim_events.rs | SimEvents 0x280 镜像 | 完成 |
| a_framework/message_handler.rs | 消息 ID 常量 + handler 表 | 完成 |
| a_framework/frame_sync.rs | m_sim_mutex / 双缓冲交换 | 完成 |
| a_framework/save_load.rs | Load/Save/CleanUp/Start/PrepareGameData | 完成 |
| a_framework/sim.rs | Sim 结构 + 生命周期 | 完成 |
| a_framework/sim_api.rs | FFI 导出入口（SIM_*） | 完成 |
| a_framework/sim_thread.rs | Sim::Main 主循环 + ReleaseQueuedHandles | 完成 |

## B 类 — 元素/病菌数据

| 模块 | 内容 | 状态 |
|------|------|------|
| b_elements/element.rs | Element 164B + 附属结构定义 | 完成 |
| b_elements/elements_table.rs | CreateElementsTable 等 5 导出 + 9 附属表 | 完成 |
| b_elements/disease.rs | 病菌表 | 简化（已知偏差） |

## C1 类 — 消息/组件/句柄层（接线层）

| 模块 | 内容 | 状态 |
|------|------|------|
| a_framework/sim_frame_manager.rs | handle_message 45 分支路由 + 帧池 + NewGameFrame | 完成 |
| c_simulation/frame_processor.rs | ProcessFrame 框架 + Process* 系列 | 完成 |
| c_simulation/sim_data_ops.rs | copy_sim_data_to_game 事件交换 + 后处理 | 完成 |
| c_simulation/building_temperature.rs | 建筑温度句柄（AddBuildingHeatExchange 注册链） | 完成（2026-08-02 20:40） |
| c_simulation/conduit_temperature.rs | 管道温度管理器（水珠/气珠温度权威 + 换热 + 相变句柄） | 完成（2026-08-02 20:14） |
| ElementConsumer（sim_data.rs/frame_processor.rs） | 水泵吸水链（注册→BFS 消费→consumedMassInfo） | 完成（2026-08-02 17:49） |
| ElementEmitter（部分） | emittedMassInfo 容器已建 | 本体未实现 |

### 管道子系统为何归 C1

- 水珠/气珠的 200ms 输送本身是 C# `ConduitFlow`（纯 C# 网格内容移动），不在 simDLL 内。
- simDLL 在管道系统中的职责全部是"接线层"：
  1. `ConduitTemperatureManager` —— 管道内容温度权威（Add/Set/Update 句柄管理）；
  2. `BuildingTemperatureManager` —— 建筑温度注册链，是管道入网的前置；
  3. `ElementConsumer` —— 水泵吸水（建筑↔格子交互）。
- 即便 `ConduitTemperatureManager::Update` 内含换热公式（借鉴 C2 温度公式），其作用对象是
  "管道内容物"这一组件，不是格子状态 → 仍属 C1。
- 边界：C2 是格子级物理（每个格子的液体/气体/温度演化）；管道内容是"C# 持有、sim 只做
  温度权威"的组件数据。

## C2 类 — 格子级物理

| 模块 | 内容 | 状态 |
|------|------|------|
| c2_physics/liquid_flow.rs | 液体流动全套（扩散/液滴/超压/蒸发/Flood/挤压/分层） | 完成 |
| c2_physics/property_texture.rs | 液体/流动/日照纹理 | 完成 |
| 温度更新（原版 04_temperature.c） | 格子温度演化 | 未实现 |
| 气体流动（原版 14_liquid_gas_flow.c 气体侧） | 气体物理 | 未实现 |
| 相变（原版 15_phase_change.c） | 沸腾/冻结/升华/熔融 | 未实现 |
| 辐射 | 辐射吸收/衰减 | 未实现 |
| 疾病模拟 | 病菌传播 | 简化 |

## 液体剩余项（截至 2026-08-02）

已完成并经用户实测：水平扩散、液滴轨道、超压（DoPressureBreak + 上下分支）、满格过压、
蒸发清理、Flood 系列、超压破坏漏水、建砖挤压、密度分层、合并/沉降、0.0mcg 残留修复、
液体纹理、管道输送。

剩余：
1. ElementEmitter 组件本体（喷口/排液口类建筑逐帧发射 + emittedMassInfo）——排液口滴液
   正常是因为走 C# vent→世界路径；该组件服务其他建筑类型。
2. 温度→格子级相变（液体沸腾/冻结；管道内容相变 frozen/melted 已实现）。
3. 升华/持续蒸发（post_process_data 的 sublimate_rate / off_gas_percentage 路径）。
4. add_solid 垂直位移（排液口/网格砖机制）——用户裁决暂不做。
5. 已知偏差：病菌混合简化、powf alpha、压力循环门控、post_process NaN 防御、
   visible_grid 屏幕外开销、元素表每格加锁性能、液体-液体压差（原版同款）。
6. 气体分层行为（DoDensityDisplacement 对气体生效）——用户判定保留。
