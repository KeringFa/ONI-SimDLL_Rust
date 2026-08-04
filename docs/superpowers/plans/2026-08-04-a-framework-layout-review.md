# a_framework 底层全量对照审查计划（结构布局/存档/FFI/纹理公式）

> **面向 AI 代理的工作者：** 本计划用于内联执行（用户固定内联；子代理消息投递在本环境不可用）。
> 每个任务对照一组结构体/函数，产出 `.superpowers/sdd/review-a-framework-layout.md` 分节记录。

**目标：** 对 simDLL_RS 的 A 类底层（a_framework 全部结构体布局、存档格式、FFI 导出、纹理公式）
做一次全量逐字段对照，严格核对 `00_types_reference.c` / `02_save_load.c` / `01_sim_api.c` /
`05_liquid_flow.c` / `11_msvcrt_ignored.c`（构造/析构/分配），找出偏移错位、字段类型/顺序不符、
分配释放不配对、导出签名不一致等"地基级"问题。

**方法：** 内联逐结构体对照。每个结构体给出：Rust 定义（file:line）+ 原版类型引用（00_types_reference.c
L 行号或字段注释）+ 逐字段偏移核对 + 构造/析构分配释放配对 + C# 消费端（Assembly-CSharp）
读取布局核对。发现的每项偏差须给出：Rust file:line + 原版引用 + 偏差说明 + 影响 + 修复方向。

**技术栈：** Rust（simDLL_RS crate）；对照材料 = `OxygenNotIncluded_Source/simDLL_edit/*.c` +
`OxygenNotIncluded_Source/SimDLL_Source.c` + `OxygenNotIncluded_Source/Assembly-CSharp/*.cs`。

---

## 已知偏差（不重复上报，可提及"已知"）

- 病菌混合简化（无 gDisease 表）；powf alpha 参数取 1.0（原版反编译丢失参数，占位）；气体分层
  行为（用户判定保留）；visible_grid 全可见兜底；元素表每格加锁性能；迭代方向（iterateDirection）
  遍历顺序近似；ElementEmitter/管道记录数组非压缩、temperatureInfo 每帧重建（输出等价）。
- 2026-08-03 已修复：GameData::Drop 7 缓冲区释放、压力循环门控/顺序、交叉分支冗余重算、
  post_process 0/0 防御、debug_properties 时机（确认一致）。

## 任务 1：GameData（结构 + 构造/析构）

- Rust：`src/a_framework/game_data.rs`（字段、new、Drop、Default）
- 对照：`00_types_reference.c` L3200-3280（GameData 字段）+ `11_msvcrt_ignored.c`
  L27089-27480（GameData::GameData）+ L27600+（GameData::~GameData）
- 重点：25 个事件容器 + 7 指针 + 4 标量，逐字段偏移/顺序；new() 分配清单与 Drop 释放清单配对；
  `flow`（Vector4f）与 `property_texture_flow`（Vector2f）区别；swapVisibleGrid。

## 任务 2：SimData（结构 + 构造/析构）

- Rust：`src/a_framework/sim_data.rs`（CellSOA、BackwallSOA、SimData、组件壳）
- 对照：`00_types_reference.c` L8395-8460（SimData）+ L11021+（CellSOA）+ 11_msvcrt_ignored.c
  （SimData::SimData / ~SimData / CellSOA 构造）
- 重点：SimData 0x890 布局、CellSOA 每字段类型（u8/u16/i32/f32 向量）、组件壳尺寸
  （ElementConsumer 312B 等）、new_for_allocate 分配与 Drop 配对、reserved 字节。

## 任务 3：GameDataUpdate（GDU）

- Rust：`src/a_framework/game_data_update.rs`
- 对照：`00_types_reference.c` L12834+（GDU 496B）+ `02_save_load.c` L1051+（PrepareGameDataUpdate
  指针填充）+ C# `Game.cs`/`Sim.cs` 消费端
- 重点：指针字段顺序、num* 计数与对应数组指针配对、ConduitTemperatureManager UpdateData 布局。

## 任务 4：SimEvents（0x280）

- Rust：`src/a_framework/sim_events.rs`
- 对照：`00_types_reference.c`（SimEvents 0x280）+ 各事件结构体（20B 协议：MeltedInfo/
  SubstanceChangeInfo/SpawnFallingLiquidInfo 等）+ C# 消费端
- 重点：全部事件容器偏移、事件条目布局（callbackIdx/elemIdx/mass/temperature/diseaseIdx/diseaseCount/
  cellIdx 等字段顺序与类型）、CopySimDataToGame 交换清单与 C# 读取协议一致。

## 任务 5：STL 镜像 + 工具结构

- Rust：`src/a_framework/stl_shim.rs`（MsvcVector/MsvcString/MsvcMutex/UniquePtr）+ `buffer.rs` +
  `vector_math.rs`（Vector2f/Vector4f）+ `abi.rs`
- 对照：`00_types_reference.c`（vector 24B/string 32B 布局）+ C# 侧互操作
- 重点：MsvcVector 三指针布局（begin/end/cap 偏移）、UniquePtr 8B、Vector2f 8B/Vector4f 16B、
  消息回调 ABI 签名。

## 任务 6：FFI 导出 + 消息路由

- Rust：`src/a_framework/sim_api.rs` + `message_handler.rs` + `sim_frame_manager.rs` +
  `frame_sync.rs` + `sim.rs` + `sim_thread.rs`
- 对照：`01_sim_api.c`（全部 SIM_* 导出签名/调用约定）+ `08_sim_frame_manager.c`
  （handle_message 45 分支消息 ID）+ C# `DLLImport` 声明
- 重点：导出名/参数/返回值一致、消息 ID 常量、帧循环（CopySimDataToGame 位置、双缓冲交换）、
  FrameSync m_sim_mutex。

## 任务 7：存档格式

- Rust：`src/a_framework/save_load.rs`
- 对照：`02_save_load.c`（Load/Save/BeginSave/PrepareGameDataUpdate 全量）+ C# SimLoader
- 重点：版本号/魔数、header 字段顺序（width/height/offset/…）、cells 序列化顺序与长度、
  backwall、事件容器、坐标换算（game→sim +1 边界）。

## 任务 8：纹理公式（C2 视觉）

- Rust：`src/c2_physics/property_texture.rs`（update_flow_texture / update_liquid_property_texture /
  update_exposed_to_sun）
- 对照：`05_liquid_flow.c` L1-656（三个纹理函数）+ C# `PropertyTextures.cs` 消费
- 重点：颜色插值（gradient alpha 区间）、alpha 计算、exposed 衰减、flow 纹理 x/y 分量、
  powf 丢失参数处（已知：取 1.0 占位，核对是否可进一步推断）。

---

## 报告契约

产出：`.superpowers/sdd/review-a-framework-layout.md`，含：
1. **优点**（逐字段核对通过、布局忠实处）
2. **问题**：Critical / Important / Minor 三级，每条含 file:line、原版引用、问题、影响、修复方向
3. **评估**：A 类底层"与原版布局/协议一致"的结论

修复原则：Critical/Important 立即修（TDD + 回归）；Minor 记录进代办"已知偏差"维护清单。
验证：全量 `cargo test -p simdll_rs -- --test-threads=1` + Release 构建。
