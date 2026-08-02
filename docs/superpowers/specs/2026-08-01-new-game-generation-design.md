# 新游戏生成（InitializeFromCells 启动分支）设计文档

日期：2026-08-01
状态：已经过头脑风暴两节确认，待用户最终审查
归属：A 类（底层框架）——启动序列的"新建"分支
对照锚点：`OxygenNotIncluded_Source/simDLL_edit/11_msvcrt_ignored.c` L24200（InitializeFromCells）、L23949（InitializeBoundary）、L24803（SettleThermalBoundaries）、L22355+（SimData 构造函数）；C# 侧 `SimMessages.SimDataInitializeFromCells`（L1123）、`WorldGenSimUtil.DoSettleSim`、`Sim.cs` 的 Cell/DiseaseCell/SimBackwall 结构体

---

## 1. 背景与问题

新建游戏时卡在世界生成阶段。2026-08-01 日志取证确认的消息序列：

1. `Elements_CreateTable` → 成功（212 元素）
2. `CreateDiseaseTable` → 桩（可接受：本分支病菌以索引透传，不查表）
3. `SimData_InitializeFromCells`（id=2062421945，783380 字节）→ **桩，返回 null，SimData 从未创建**
4. `Start` → 因 gSimData 为 null 报错返回 null
5. C# 拿不到 sim 世界 → 永久卡在世界生成

读档分支（AllocateCells → Load → Start）已打通，dig/沙盒工具正常。缺的是"新建"启动分支。

### 新游戏生成的完整链路（C# `WorldGenSimUtil.DoSettleSim`）

```
SIM_Initialize → CreateSimElementsTable → CreateDiseaseTable(桩)
→ SimDataInitializeFromCells   ← 本次实现的核心缺口
→ Sim.Start
→ 500 步沉降循环：NewGameFrame + PrepareGameData（机制已打通）
   └─ 第 498 步：ModifyCell（已实现）+ SetBackwallData（未实现，见风险节）
→ Sim.Save（BeginSave，已实现）→ Sim.Shutdown
```

之后游戏读取生成的存档进世界，走已打通的读档分支。**眼冒金星集群每个小行星都会跑一整轮 DoSettleSim**（WorldGen.cs L523），因此 SIM_Initialize → … → Shutdown 的可重入性是硬性要求（SIM_Initialize 已先调 CleanUp，结构上可行）。

## 2. 已破解的二进制布局（783KB 消息）

消息格式**不是** SIMSAVE 存档格式，是无魔数、无版本的裸格式。总字节数 = 14 + N×40（N = width×height；实测 783380 字节是 C# `MemoryStream.GetBuffer()` 的容量，头部按 `Marshal.SizeOf(bool)=4` 计入，N≈19584，实际尺寸以日志为准）。

### 头部（14 字节，C# 写入顺序）

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 | i32 | width |
| 4 | i32 | height |
| 8 | u32 | simSeed（世界生成器给的确定性种子，**不是** AllocateCells 的当前时间） |
| 12 | bool(1B) | radiationEnabled（`Sim.IsRadiationEnabled()`） |
| 13 | bool(1B) | headless（世界生成时恒为 true） |

### 块 1：Sim.Cell × N（每格 16 字节）→ updated_cells

| 偏移 | 类型 | 目标字段 | 备注 |
|---|---|---|---|
| 0 | u16 | element_idx | **是元素索引，不是哈希**——无需查表转换（与读档分支的关键区别） |
| 2 | u8 | properties | C# 恒写 0（properties 由 sim 后续计算） |
| 3 | u8 | insulation | 实际数据 |
| 4 | u8 | strength_info | C# 恒写 0 |
| 5-7 | 3B | （忽略） | pad0-2 |
| 8 | f32 | temperature | 实际数据 |
| 12 | f32 | mass | 实际数据 |
| — | — | radiation | 原版显式写 0（消息不含此字段） |

### 块 2：DiseaseCell × N（每格 12 字节）→ updated_cells

| 偏移 | 类型 | 目标字段 |
|---|---|---|
| 0 | u8 | disease_idx |
| 1 | u8 | disease_infestation_tick_count |
| 2-3 | 2B | （忽略，pad） |
| 4 | i32 | disease_count |
| 8 | f32 | （忽略：reservedAccumulatedError）→ disease_growth_accumulated_error 原版显式写 0 |

### 块 3：SimBackwall × N（每格 12 字节）→ backwall SOA

| 偏移 | 类型 | 目标字段 |
|---|---|---|
| 0 | u16 | backwall.element_idx |
| 2-3 | 2B | （忽略，pad） |
| 4 | f32 | backwall.mass |
| 8 | f32 | backwall.temperature |

### 格子索引公式

消息中的格 (row, col)（0-based）写入 `updated_cells[(row+1) × sim_width + (col+1)]`，其中 sim_width = width+2。+1 偏移为外围边界格留位，边界由 `InitializeBoundary` 填充。

### 原版数据定位手法

记录当前 buffer 指针 → SkipBytes(N×16) → 记录 → SkipBytes(N×12) → 记录，共三个基址；reader 最终位置 = 14 + N×40。Rust 侧需在 `BinaryBufferReader` 增加等价的"当前位置切片 / 按偏移读原始字节"能力（`buffer.rs`，约 15 行）。

## 3. 两个依赖函数（原版在 handler 内部调用）

### 3.1 InitializeBoundary（对照 L23949）

同时写 `cells` 和 `updated_cells` **两个缓冲** + `backwall`：

| 边界 | element_idx | mass | temperature | disease_count | disease_idx | backwall |
|---|---|---|---|---|---|---|
| 第 0 行（底部，全列） | unobtanium_idx | 9999.0 | 0.0 | 0 | 0xff | vacuum_idx / 0 / 0 |
| 第 height-1 行（顶部，全列） | vacuum_idx | 9999.0 | 0.0 | 0 | 0xff | 同上 |
| 第 0 列与 width-1 列（第 1..height-2 行） | unobtanium_idx | 9999.0 | 0.0 | 0 | 0xff | 同上 |

（9999.0 = 0x461C3C00。顶部真空行质量也是 9999.0，照抄原版。）

### 3.2 SettleThermalBoundaries（对照 L24803）

读 `cells`、写 `updated_cells`：对内部格中 `element == GetElementIndex(0x3fe0146e)` 的格子，统计 8 邻居（偏移 ±1、±w、±w±1）中**排除** 0x3fe0146e 与 unobtanium(0x6d95058c) 且 mass > 0 的格子，把温度均值写入 `updated_cells.temperature[cell]`。

注意：新开局时数据在 `updated_cells`、`cells` 全零，此函数在新开局近乎空转——仍忠实实现（Start 读 version<11 旧存档时也走它）。

### 3.3 new_for_allocate 缺口修补

原版构造函数（L22385-22390 + L22358-22362）设置的内容，Rust 版目前**只设置了 vacuum_element_idx**，需补齐：

- `void_element_idx = GetElementIndex(0xa9360b34)`
- `unobtanium_element_idx = GetElementIndex(0x6d95058c)`
- 辐射常数：LINGER_RATE=1.1、MAX_MASS=2000.0、BASE_WEIGHT=0.3、DENSITY_WEIGHT=0.7、CONSTRUCTED_FACTOR=0.8

`new_for_allocate` 参数序 (radiation_enabled, headless) 与原版构造函数 (param_4, param_5) 已一致，无需改动签名。

## 4. 实现方案（方案 A：严格镜像原版，已批准）

改动清单：

| 位置 | 改动 | 估算 |
|---|---|---|
| `a_framework/save_load.rs` | 替换 L1068 `handle_initialize_from_cells` 桩：读 5 头字段 → 验长度 → 定位三块 → `new_for_allocate(w+2, h+2, simSeed, radiation, headless)` 替换 gSimData（释放旧的）→ 三循环填 updated_cells/backwall → settle_thermal_boundaries → initialize_boundary → 返回 width*height | ~80 行 |
| `a_framework/sim_data.rs` | 新增 `settle_thermal_boundaries`（~35 行）与 `initialize_boundary`（~50 行，用辅助函数消除"两缓冲×四边"的重复，行为逐字节一致） | ~85 行 |
| `a_framework/sim_data.rs` | `new_for_allocate` 补 void/unobtanium 索引 + 5 个辐射常数 | ~10 行 |
| `a_framework/buffer.rs` | `BinaryBufferReader` 增加原始字节访问能力 | ~15 行 |

忠实原则：元素索引越界等不做额外校验（原版不校验，数据来自可信 C# 世界生成器），但加 tracing 日志。

### 已否决的方案（备查）

- **方案 B（转换复用 handle_load）**：语义不符（Load 做哈希→索引转换、NaN 修复、状态转变；存档格式无 insulation/properties 字段，转换丢数据 + 多拷贝 783KB）。
- **方案 C（初始化下沉到 handle_start）**：扰动已打通的读档分支；原版明确在 handler 内完成。衍生的遗留问题"读档分支是否补 InitializeBoundary"记到 A 类审阅。

## 5. 错误处理

- 头部读完后先验证：width > 0、height > 0、reader 剩余 ≥ N×40——不满足则报错返回 null，**gSimData 保持原样不被替换**（原版无此检查，属纯增强）。
- 解析中任何读取失败 → 报错返回 null（与现有 handler 风格一致）。

## 6. 测试计划（先单元测试后实测，已批准）

1. **合成数据全字段测试**：4×3 世界（N=12，总长 494 字节），每格写不同值 → 断言返回值=12、gSimData 6×5、num_game_cells=12、内部 12 格逐字段一致、radiation/accError 为 0。
2. **边界初始化测试**：底行 unobtanium/9999.0、顶行 vacuum、左右列 unobtanium；`cells` 与 `updated_cells` 双缓冲均写入。
3. **截断数据测试**：少 1 字节 → 返回 null 且旧 gSimData 不被替换。
4. **替换测试**：先 AllocateCells 再 InitializeFromCells → 旧 SimData 释放、全局指向新数据。
5. **SettleThermalBoundaries 单测**：构造 cells 中一个 0x3fe0146e 格 + 有质量邻居 → updated_cells 温度 = 均值（排除 Void/中子物质/无质量格）。
6. **游戏实测**（单测全过后）：编译 → **用户自行部署** → 新开游戏 → 日志序列 `InitializeFromCells done` → `Start done` → 500 步循环 → `BeginSave done` → 进入小人选定 → 进世界挖掘/沙盒正常。

## 7. 风险与已知边界

- **SetBackwallData 缺失**（hash -605421515）：第 498 步模板放置时日志出现 unknown message 警告，遗迹 POI 背景墙不生效——不阻塞世界生成，留作后续小补充。
- **多次 DoSettleSim 重入**：集群每个小行星一轮，第二轮起 sim 线程停启是否干净是实测重点。
- **handle_message 未持 frame_mutex** 的既有隐患：世界生成在 OfflineWorldGen 线程运行，并发模式与主游戏不同，实测观察。
- **沉降物理保真度**：C2 物理未全量实现，500 步沉降结果与原版有定性差异属预期；验收以代办清单标准为准（过世界生成 → 小人选定 → 挖掘/沙盒正常）。

## 8. 验收标准（沿用代办清单）

新建世界 → 顺利通过世界生成 → 进入初始小人选定 → 进世界后挖掘/沙盒工具正常。
