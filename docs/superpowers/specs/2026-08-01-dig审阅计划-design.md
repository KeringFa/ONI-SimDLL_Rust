# dig 功能失效 — 审阅计划（设计文档）

日期：2026-08-01
状态：用户已批准，执行中
产出形式：诊断报告 + 修复方案（**用户批准后才动代码**）

## 1. 背景与目标

simDLL_RS 是用 Rust 重写《缺氧》SimDLL.dll 的项目。当前 bug：**游戏中挖掘（dig）功能完全失效，格子挖不动**。

审阅目标：聚焦 dig 消息链路，定位根因，产出诊断报告与修复方案。暂不做 A/B/C 三类全面审阅（用户已确认缩小范围）。

可参考的半成品项目 `C:\Users\KeringFa\Desktop\my_Project\simDLL_RS_Kit\01`（框架远逊于本项目，仅作思路参考，不照抄）。

## 2. 已确认的证据（来自 2026-08-01 游戏日志）

| 环节 | 状态 | 证据 |
|------|------|------|
| dig 消息到达 simDLL | 正常 | 544 条 `id=833038498 size=12`，`handled=true` |
| 元素表创建 | 正常 | 212 个元素全部映射成功 |
| sim_events 空指针告警 | 无 | `process_dig_points` 提前返回分支已排除 |
| 帧数据回传 C# | 正常 | PrepareGameDataUpdate 持续运行 |
| dig 实际处理 | 无声无息 | frame_processor 零日志，格子无变化 |

结论：消息成功入队，但在"入队 → 清格"之间被静默吞掉。

原版 C 源码对照锚点：`OxygenNotIncluded_Source\SimDLL_Source.c`
- `ProcessDigPoints`：115929-116149 行
- `SimFrameManager::HandleMessage` Dig 分支：115017-115027 行
- `ProcessFrame` 调用点：116476 行（顺序：辐射 → DigPoints → CellModifications）
- `DigPoint` 结构（12 字节）：11614 行；`SpawnOreInfo`（20 字节）：5967 行

## 3. 审阅范围（按嫌疑度排序）

| 顺序 | 文件 | 审查重点 |
|------|------|---------|
| ① | `src/a_framework/sim_frame_manager.rs` | 帧衔接：dig 消息排进的"当前帧"是否就是 sim 线程处理的帧？双缓冲/帧边界错位会导致消息永远不被处理（最大嫌疑） |
| ② | `src/c_simulation/frame_processor.rs` | `process_dig_points`（76-217 行）：坐标转换、固体判定、清格、事件推送，逐行对照原版 |
| ③ | `src/b_elements/elements_table.rs` | `get_element_state_by_idx`（91-97 行）：state_data 越界即返回 None → is_solid=false → 静默跳过 |
| ④ | `src/c_simulation/sim_data_ops.rs` | `clear_cell` / `clear_backwall` / 数据回传一致性 |
| ⑤ | `src/a_framework/sim_api.rs` + `game_data_update.rs` | FFI 边界 dig_info 指针回传 |

## 4. 审阅方法

- 与 `SimDLL_Source.c` 对应函数并排对照。
- 检查点清单：
  - game→sim 坐标转换公式（原版 115978 行）
  - 固体判定 `state & 3 == 3`（原版 116012 行）
  - `callbackIdx != -1` 时推送 callback_info（原版 115999 行）
  - `skipEvent==0 && mass>0` 时推送 dig_info/SpawnOreInfo（原版 116071 行）
  - ClearCell / ClearBackwall（原版 122969 行）
  - BackwallElementChangedInfo、SubstanceChangeInfo（old=0xFFFF, new=0xFFFF）
  - `timers[simCell] |= 0x1F`（原版 116140 行）
  - `digPoints.clear()` 时机
  - 双缓冲读写层一致性（参考 01 项目"双层同时清零"思路）
- 每个疑点记录：位置、原版行为、当前行为、是否可解释"挖不动"。

## 5. 插桩兜底（静态审阅未定位根因时启用）

在 5 个静默分支点加临时日志：消息入队数、进入处理的点数、固体判定结果、清格执行、事件推送数。编译测试版 DLL，用户照常游戏 1 分钟并挖几格，回收日志一次锁定。验证后移除临时日志。

## 6. 产出物

1. 诊断报告（小白比喻版 + 技术版）
2. 修复方案：改动文件、改法、安全性说明——**用户批准后执行**
3. 修复后编译验证 + 测试步骤

## 7. 规格自检

- 占位符：无 TODO/待定项。
- 一致性：证据表与审阅范围、检查点清单一一对应。
- 范围：聚焦 dig 链路，单次计划可覆盖；不牵连 A/B/C 全审。
- 模糊性：产出形式已明确为"修复方案待批"，插桩需先经用户同意编译测试 DLL。
