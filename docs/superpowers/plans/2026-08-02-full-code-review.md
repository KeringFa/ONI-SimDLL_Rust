# 全量代码审查计划（严格对照原版原理）

> **面向 AI 代理的工作者：** 本计划用于分派并行审查子代理。每个任务审查一组模块，产出独立报告文件；全部完成后由控制者汇总分诊。

**目标：** 对 simDLL_RS 全部代码做一次全量审查，严格对照原版反编译源码（simDLL_edit 切片 + SimDLL_Source.c + Assembly-CSharp 消费端）查漏补缺，找出与原理不符的实现偏差。

**方法：** 子代理直接读 Rust 源码与反编译 C 源码逐段对照（项目约束：不执行 git 操作、无 diff 可用）。发现的每项偏差须给出：Rust file:line + 原版源码引用（文件名+L 行号或函数名）+ 偏差说明 + 影响 + 修复方向。

**技术栈：** Rust（simDLL_RS crate）；对照材料 = `OxygenNotIncluded_Source/simDLL_edit/*.c` + `OxygenNotIncluded_Source/SimDLL_Source.c` + `OxygenNotIncluded_Source/Assembly-CSharp/*.cs`（C# 消费端协议）。

---

## 审查范围与已知偏差（避免重复上报）

审查重点：**字段映射、公式（尤其 min/max 反转类）、缺失分支、留桩 vs 原版语义、消息结构布局、事件协议、缓冲区/内存**。

以下为**已知偏差**，不作为新发现上报（可提及"已知"）：
- 病菌混合简化（无 gDisease 表）；相变留桩（升华/熔融发射未实现）；powf 参数取 1.0；压力循环门控只跳固体；post_process 0/0 NaN 观察项；气体分层（用户判定暂不修）；visible_grid 全可见兜底；元素表每格加锁性能；game_data 释放不一致；ElementConsumer 组件整体未实现（水泵，已知待办）。

## 任务 1：框架与数据层（a_framework）

**审查文件：**
- Rust：`simDLL_RS/src/a_framework/` 全部（sim_data.rs、game_data.rs、game_data_update.rs、sim_events.rs、buffer.rs、stl_shim.rs、vector_math.rs、frame_sync.rs、abi.rs、logger.rs、crashdump.rs、message_handler.rs、sim_api.rs、sim_thread.rs、sim_frame_manager.rs、save_load.rs、process_messages.rs、sysinfo.rs）
- 对照：`simDLL_edit/00_types_reference.c`（结构布局/偏移）、`11_msvcrt_ignored.c`（构造/析构/分配）、`01_sim_api.c`（导出）、`02_save_load.c`（存档）

**重点：** 结构体布局与偏移是否与类型引用一致；分配/释放配对；消息 ID 与 handler 表；存档格式解析（字段顺序/长度）；事件结构体 20B 协议；C# 消费端（Game.cs/Sim.cs/SimMessages.cs）读取的字段布局。

## 任务 2：元素表 + 纹理/温度物理（b_elements + c2_physics 非液体）

**审查文件：**
- Rust：`simDLL_RS/src/b_elements/`（elements_table.rs、element.rs）；`simDLL_RS/src/c2_physics/mod.rs`、`property_texture.rs`（含 update_flow_texture / update_liquid_property_texture / update_exposed_to_sun）；`temperature.rs`（若存在）
- 对照：`simDLL_edit/03_elements.c`（9 个附属结构字段映射）、`00_types_reference.c`（Element/附属结构）、`05_liquid_flow.c` L1-656（三个纹理函数）

**重点：** 字段映射逐一核对（已知先例：ppd.max_mass 曾误映射 solidSurfaceAreaMultiplier；flow/viscosity 双源）；纹理公式（颜色/alpha/阳光衰减）与 C# PropertyTextures 消费；state 位含义。

## 任务 3：液体物理 + 帧处理（c2_physics/liquid_flow.rs + c_simulation）

**审查文件：**
- Rust：`simDLL_RS/src/c2_physics/liquid_flow.rs`（update_liquid 四分支 / update_neighbour_liquid_mass / update_pressure / post_process_cell / DoPressureBreak / Add 系列 / Flood 移除）；`simDLL_RS/src/c_simulation/frame_processor.rs`、`sim_data_ops.rs`
- 对照：`simDLL_edit/05_liquid_flow.c`（L656-1212 UpdateLiquid、L1213-1274 邻格转移、L1436-1629 UpdatePressure、L1755-1840 DoDensityDisplacement、L2320-2867 PostProcessCell）、`11_msvcrt_ignored.c`（DisplaceLiquid/DisplaceGas/AddMassAndUpdateTemperature/SwapCells/ClearCell/SpawnFallingLiquid）、`SimDLL_Source.c`（cellmodifications L134150-135230、ProcessMassConsumption L117050-117185、DoPressureBreak L2663-2792、Flood L135332+）、`Assembly-CSharp/FallingWater.cs`+`Game.cs`+`WorldDamage.cs`（消费端协议）

**重点：** 公式逐一对照（min/max、×0.5、×0.25、0.49751243、1.01）；四分支判定链；事件（substance/spawnLiquid/worldDamage/massConsumed）；消息处理（ModifyCell Add/Replace/ReplaceAndDisplace、Dig、MassConsumption）；copy_sim_data_to_game 事件交换完整性。

---

## 报告契约

每任务产出：`.superpowers/sdd/review-<组名>.md`，含：
1. **优点**（对照原版做得好、忠实处）
2. **问题**：Critical / Important / Minor 三级，每条含 file:line、原版引用、问题、影响、修复方向
3. **评估**：该组"对照原版原理的偏差风险"结论

子代理返回时仅报告：状态、发现数量（C/I/M）、一行小结。报告写入指定文件。

## 汇总（控制者执行）

- 合并三份报告 → 分诊：Critical/Important 是否立即修、Minor 记录；
- 更新代办.txt 备忘与遗留清单；
- 输出审查总结给用户，按严重度排修复顺序。
