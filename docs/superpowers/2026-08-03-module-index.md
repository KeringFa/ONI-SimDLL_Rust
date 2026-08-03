# simDLL_RS 模块索引（模块 → 项目文件 → 原版位置）

**日期**：2026-08-03
**用途**：后期维护查表。原版位置格式：`split 文件:行号`（括号内为 SimDLL_Source.c 行号，若不同）。
split 文件在 `OxygenNotIncluded_Source\simDLL_edit\`，原始反编译为 `OxygenNotIncluded_Source\SimDLL_Source.c`。

## A 类 — 框架与数据定义（均完成）

| 模块 | 项目文件 | 原版位置 |
|------|----------|----------|
| 消息回调 ABI | `a_framework/abi.rs` | SimDLL_Source.c 导出区 |
| 二进制缓冲 | `a_framework/buffer.rs` | —（工具） |
| STL 镜像 | `a_framework/stl_shim.rs` | —（工具） |
| 向量类型 | `a_framework/vector_math.rs` | 00_types_reference.c（Vector2f/4f） |
| 日志 | `a_framework/logger.rs` | — |
| 崩溃转储 | `a_framework/crashdump.rs` | — |
| 系统信息 | `a_framework/sysinfo.rs` | — |
| GameData | `a_framework/game_data.rs` | 00_types_reference.c L3200-3280（accumulatedFlow L3262） |
| GameDataUpdate（GDU） | `a_framework/game_data_update.rs` | 00_types_reference.c L12834+ |
| SimData | `a_framework/sim_data.rs` | 00_types_reference.c L8395-8460（accumulatedFlow L8405） |
| SimEvents | `a_framework/sim_events.rs` | 00_types_reference.c（SimEvents 0x280） |
| 消息 ID / handler 表 | `a_framework/message_handler.rs` | — |
| 帧同步双缓冲 | `a_framework/frame_sync.rs` | 00_types_reference.c（FrameSync） |
| Load/Save/Start/PrepareGameData | `a_framework/save_load.rs` | 02_save_load.c L211-255（Start/Load）、L688-1058（PrepareGameDataUpdate） |
| Sim 生命周期 | `a_framework/sim.rs` | 08_sim_frame_manager.c（Sim 结构） |
| FFI 导出 | `a_framework/sim_api.rs` | SimDLL_Source.c 导出区 |
| Sim::Main 主循环 | `a_framework/sim_thread.rs` | 08_sim_frame_manager.c L2110-2170（CopySimDataToGame 调用 L2158） |

## B 类 — 元素/病菌数据

| 模块 | 项目文件 | 原版位置 | 状态 |
|------|----------|----------|------|
| Element 164B + 附属结构 | `b_elements/element.rs` | 00_types_reference.c L11105+ | 完成 |
| 元素表 + 9 附属表 | `b_elements/elements_table.rs` | 03_elements.c L27-429（CreateElementsTable） | 完成 |
| 病菌表 | `b_elements/disease.rs` | 00_types_reference.c L12641+ | 简化（已知偏差） |

## C1 类 — 消息/组件/句柄层

| 模块 | 项目文件 | 原版位置 | 状态 |
|------|----------|----------|------|
| 消息路由/帧池 | `a_framework/sim_frame_manager.rs` | 08_sim_frame_manager.c（handle_message 45 分支 + NewGameFrame） | 完成 |
| ProcessFrame + Process 系列 | `c_simulation/frame_processor.rs` | 08_sim_frame_manager.c L1174-1330（ProcessCellModifications L1330、ProcessDigPoints L1329、ProcessMassConsumption/Emission L1262-1263、ProcessElementChunkMessages L1528） | 完成 |
| CopySimDataToGame + 事件交换 | `c_simulation/sim_data_ops.rs` | 11_msvcrt_ignored.c L31168-32200；accumulated_flow 同步 L32491-32498 | 完成 |
| 建筑温度句柄（AddBuildingHeatExchange 注册链） | `c_simulation/building_temperature.rs` | 消息 0x67A75D28/0x6C5C80A1/0xE4D1E06B；BuildingHeatExchange::Update 11_msvcrt_ignored.c L35496 | 完成 |
| 管道温度管理器 | `c_simulation/conduit_temperature.rs` | 07_conduit_temperature.c L8-152（Add/Set/Update；SimDLL_Source.c L94814-95886） | 完成 |
| 碎片组件 ElementChunk | `c_simulation/element_chunk.rs` | ExchangeHeatEnergyWithWorld 11_msvcrt_ignored.c L37923；ElementChunk::Update L38226；ProcessElementChunkMessages 08_sim_frame_manager.c L1528 | 完成 |
| 导热板 BuildingToBuilding | `c_simulation/building_to_building.rs` | BuildingToBuildingHeatExchange::Update 11_msvcrt_ignored.c L36492；消息 L116540-116650 | 完成 |
| ElementConsumer（水泵吸水） | `c_simulation/frame_processor.rs` + `a_framework/sim_data.rs` | ElementConsumer::Update 11_msvcrt_ignored.c L38910；ProcessMassConsumption 08_sim_frame_manager.c L1262 | 完成 |
| ElementEmitter（喷口/排液口类建筑） | —（未实现） | 04_temperature.c L1609-2000（Register/Modify/Update） | 本体未实现 |

## C2 类 — 格子级物理

| 模块 | 项目文件 | 原版位置 | 状态 |
|------|----------|----------|------|
| 液体流动全套 | `c2_physics/liquid_flow.rs` | UpdateLiquid 05_liquid_flow.c L656；DoDensityDisplacement L1755；UpdatePressure L1436；PostProcessCell L2320（液体 offgas 内联 L149677-149834）；AddGas/AddLiquid/AddSolid L134150-134789；DoPressureBreak 04_temperature.c L2662 | 完成 |
| 自然格升华 DoSublimation | `c2_physics/liquid_flow.rs`（真空/气体分支）+ `c2_physics/temperature.rs` | 04_temperature.c L2792（SimDLL_Source.c L148591-148880） | 完成 |
| 液体纹理 | `c2_physics/property_texture.rs` | UpdateExposedToSunPropertyTexture 05_liquid_flow.c L7；UpdateFlowTexture L230；UpdateLiquidPropertyTexture L309 | 完成 |
| 温度域 | `c2_physics/temperature.rs` | DoLoadTimeStateTransition 04_temperature.c L7；DoStateTransition L78；UpdateTemperature L407；UpdateTemperatureForBackwallExchange L674；CalculateCombinedTemperature L1459；DoPartialHeatTransition L2105；DoPartialMelt L2471；BuildingHeatExchange::Update 11_msvcrt_ignored.c L35496 | 完成（2026-08-03 全闭合） |
| 气体流动 | —（未实现） | 14_liquid_gas_flow.c 气体侧 | 未实现（PostProcessCell 气体分支部分就绪） |
| 辐射 | —（未实现） | — | 未实现 |
| 疾病模拟 | `b_elements/disease.rs`（表）+ liquid_flow.rs（按比例转移） | — | 简化 |

## 已知偏差与用户裁决（维护提醒）

- 病菌混合简化（无 gDisease 表）；powf alpha 取 1.0（仅影响液体透明度）；
  压力循环门控只跳固体（原版更严）；post_process 过压分支潜在 0/0（观察未修）；
  visible_grid 全可见兜底（屏幕外也发液滴事件）；元素表每格加锁性能。
- 气体分层行为（DoDensityDisplacement 对气体生效）——用户判定保留。
- add_solid 垂直位移（DoVerticalDisplacement）——用户裁决暂不做。
- 密度分层仅限同相态（DoDensityDisplacement 的 state 字节全等门，05_liquid_flow.c L1775-1777）；
  液体↔气体互换是液体流动路径（update_liquid 下方 state<2 分支），不是密度分层。
