# 液体流动（C2 第一个任务）设计文档

日期：2026-08-01
状态：已经过头脑风暴两节确认（架构与数据流 + 核心算法与挤压链路），待用户最终审查
归属：C2（c2_physics，随 simDLL 200ms 变化的动态物理模块）
对照锚点：`OxygenNotIncluded_Source/simDLL_edit/05_liquid_flow.c`（2902 行）；C# 侧 `LiquidFlowVisualizer` / `Grid.AccumulatedFlowValues`；01 半成品 `liquid_flow.rs`/`liquid_ops.rs`（1132 行，仅思路参考不照抄）

---

## 1. 背景与现象

### 现象
游戏中所有液体格子渲染为**全黑**。

### 已确认的根因链
1. C# 液体渲染（`LiquidFlowVisualizer`）读取 `Grid.AccumulatedFlowValues`（即 GDU 的 `accumulatedFlow` 指针，f32 数组）
2. 我们的 `SimData.accumulated_flow` / `flow` 字段**从未分配**（`new_for_allocate` 注释明确"暂不分配保持 null"）→ GDU 里为 null → C# 拿不到任何流动数据 → 液体纹理无法渲染（含用户观察的"噪声背景动态变化底层"——即 flow 数据驱动的流动动画）
3. 更深层：液体**流动物理**（下落/蔓延/压强/密度分层）未实现——即使分配数组，液体也不会流动

### 用户观察（设计必须覆盖的物理行为）
- **液体被固体挤压**（如小人建造砖块放入液体格）：该格液体质量被**平均分配**给上下左右方向的液体（异类液体也生效）
- **气体被固体挤压**：**随机一个方向**转移全部质量
- 对应原版：`DisplaceLiquid` / `DisplaceGas`（PostProcessCell 内，由 `displacementDirection` 驱动）+ `DoDensityDisplacement`（密度分层）

### 参考（不照抄）
01 半成品 `liquid_flow.rs`（794 行，按 active_regions 逐区域模拟 + flow_dir 方向常量）+ `liquid_ops.rs`（338 行）——算法骨架可作对照，但其框架有问题，实现以原版为准。

## 2. 架构与数据流

### 2.1 挂接点
`c2_physics::update_data`（目前空桩）→ 新增 `liquid_flow::update_liquids(sim_data)`。每帧（200ms 子步）执行。

### 2.2 数据分配（`SimData::new_for_allocate` 补充）
| 字段 | 类型 | 用途 | 原版依据 |
|---|---|---|---|
| `flow` | `Vector4f[]`（每格 4×f32） | 每格 4 方向流动量 | UpdateLiquid 中 `flow + 8 + cell*0x10` |
| `accumulated_flow` | `f32[]` | 流动累积（纹理数据源） | C# `Grid.AccumulatedFlowValues` |

### 2.3 元素液体/压力数据表
| 表 | 每元素大小 | 用途 | 原版依据 |
|---|---|---|---|
| `gElementLiquidData` | 24B | 液体属性（最大质量/流率/密度/状态位） | 05_liquid_flow.c |
| `gElementPressureData` | 8B | 压力属性（状态位+压力系数） | UpdatePressure |

Rust `elements_table` 需补这两张表（B 类扩展，液体物理的数据基础）。

### 2.4 完整数据流（六段链）
```
update_data（C2 桩挂接）
  → update_liquids：遍历 active_regions 每格
      → UpdateLiquid（质量向下/邻格转移 + flow 4 方向累加）
      → UpdatePressure（压强平衡）
      → DoDensityDisplacement（密度分层）
      → PostProcessCell（不稳定格检查 + 挤压位移 DisplaceLiquid/DisplaceGas）
  → UpdateFlowTexture：flow → accumulated_flow 累积
  → GDU 携带 accumulated_flow 指针 → C# LiquidFlowVisualizer 渲染流动动画
```

## 3. 核心算法

### 3.1 UpdateLiquid（逐格质量转移，对照 L656）
对 active region 内每格：
- 检查下方格（`cell - width`）的可流动性（`IsLiquidPermeable`）
- 转移量 = `min(自身质量×1.01 − 下方质量, 元素流率上限, 自身质量×0.5)`（下限 0）
- 向下转移：`flow[DOWN] += 转移量`、自身质量 −= 转移量
- 病菌按比例随质量转移（`ModifyDiseaseCount`）
- 特例：下方为 Void → 转真空并清零

### 3.2 UpdatePressure（压强平衡，对照 L1437）
- 用 `gElementPressureData`（状态位 + 压力系数）
- 向邻格转移 `质量差 × 系数`，上限 12.5%
- 效果：相邻同质液体质量趋于平衡（水平找平）

### 3.3 DoDensityDisplacement（密度分层，对照 L1756）
- 异类液体/气体相遇时按密度置换：重的下沉、轻的上浮
- 效果：水压住石油、气体浮到液体上方

### 3.4 PostProcessCell + 挤压位移（对照 L2320，含用户观察链路）
- `displacementDirection`（每帧交替反转）驱动扫描顺序
- ModifyCell 建砖（`addSubType` 标志）触发强制位移：
  - **DisplaceLiquid**：质量**平均分配**给上下左右可容纳格（异类液体也接收）——用户观察验证
  - **DisplaceGas**：**随机一个方向**转移全部质量——用户观察验证
- `DoUnstableCheckBasic` / `DoUnstableCheckWithDiagonals`：不稳定格检查（塌方/下落）

### 3.5 UpdateFlowTexture（对照 L230）
- flow 4 方向量 → `accumulated_flow` 累积 → GDU 暴露给 C#

## 4. 错误处理与边界
- 元素表未初始化 → update_liquids 直接返回（与 01 半成品同思路）
- active_regions 为空 → 直接返回
- flow/accumulated_flow 指针为 null（旧存档/未分配）→ 防御性跳过纹理产出
- 边界格（+2 外围）不参与流动（active region 裁剪到内部格）

## 5. 测试计划（先单元后实测）

| # | 测试 | 验证点 |
|---|---|---|
| 1 | 单格水下落真空格 | 质量正确转移、flow[DOWN] 累加 |
| 2 | 相邻液体质量差 | UpdatePressure 找平 |
| 3 | 水+石油同格 | 密度分层正确 |
| 4 | 砖块放入液体格 | **质量平均分配 4 方向**（对照用户观察） |
| 5 | 砖块放入气体格 | 单方向全转移（对照用户观察） |
| 6 | accumulated_flow | 非零（纹理数据产出） |
| 7 | 游戏实测 | 液体显示颜色 + 流动动画；挖开蓄水池观察液体下落 |

## 6. 验收标准
- 液体格子不再全黑，显示正确元素颜色
- 液体有流动动画（flow 数据驱动）
- 液体物理行为：下落、水平找平、密度分层正确
- 挤压链路与用户观察一致（液体平均分配 / 气体单方向转移）
- 全量回归通过

## 7. 风险与已知边界
- **工程量大**：原版核心函数约 2000 行反编译 C，翻译 Rust 需仔细（含病菌按比例转移、Void 特例等细节）
- **性能**：每帧全区域逐格模拟，需对照原版的 active region 裁剪；C2 阶段可先不做性能优化，功能正确优先
- **与原版差异**：预期范围内（C2 物理整体未完成，温度/病菌联动后续任务接入）
- 气体流动**不在本任务**（独立后续任务，含气体挤压位移）
- 温度传导/蒸发/相变（Evaporate）不在本任务范围（后续 C2 任务）
