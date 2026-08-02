# 液体水平扩散修复设计

**日期**：2026-08-02
**状态**：已批准（头脑风暴两节确认）
**类别**：C2 液体流动 · 实测反馈修复（第 1 轮）

## 1. 背景与现象

液体流动（2026-08-02 SDD 完成）实测反馈：
- ✅ 液体颜色恢复（亮度接近原版）
- ✅ 液体受重力正常下落
- ❌ **液体不左右扩散**——水落地后只堆成柱，不向两侧铺开

**根因**：`update_liquid`（simDLL_RS/src/c2_physics/liquid_flow.rs）只实现了原版 UpdateLiquid 的**向下分支**（05_liquid_flow.c L700-846）。原版在向下分支后还有**左分支（L849-945）**与**右分支（L946-1212）**，水平扩散公式 `fVar22 = (fVar25 - fVar23) * 0.25` 就在这两个分支里——Rust 实现完全省略（注释误标"水平/斜向流动属任务 7/8"，但任务 7 是压强平衡、任务 8 是挤压，水平蔓延没有归属）。

**范围**（用户裁决）：本次只修水平扩散。挤压链路（液滴 SpawnFallingLiquid / 建砖 DisplaceLiquid / 异类占格质量转移）用户判断需单独深度研究（涉及重力掉落与密度），留作后续独立任务。

## 2. 方案

**方案 A（已批准）：严格镜像原版左右分支。**

在 `update_liquid` 向下分支之后追加左分支 + 右分支，逐行对照原版：
- 左分支：05_liquid_flow.c L849-945
- 右分支：05_liquid_flow.c L946-1212

### 2.1 算法语义（左分支，右分支对称）

```
# 前置：fVar25 = mass_self（向下分支后的剩余质量）
if fVar25 <= 0: return

# 判定链（全部通过才继续）：
# 1. neighbor 元素（cells 快照，原版 L853-854）：
#    == 本格元素 或 state&3 < 2（真空/气体）
# 2. neighbor 元素（updated，原版 L956-960）：state&3 != 3（非固体）
# 3. neighbor properties（cells，原版 L964-966）& 2 == 0（非固体标志）

# 扩散量计算（原版 L867-882）：
fVar23 = neighbor_mass（若 neighbor 是液体 state&3==2）否则 0.0   # L867-874
fVar22 = (fVar25 - fVar23) * 0.25                                 # L876
fVar23 = min(fVar25, 元素流率上限)                                 # L877-880
fVar23 = min(fVar22, fVar23)                                      # L881-882

# 门限（原版 L884）：fVar23 >= min_horizontal_flow 且 fVar23 > 0
if 通过:
    disease_transfer = (fVar23 / fVar25) * disease_count           # L886
    # L887-909：若 IsSolid(下方) 且 IsLiquidPermeable(斜下方)：
    #   SpawnFallingLiquid(...) —— 当前实现恒 false → fallback
    # L917-919：UpdateNeighbourLiquidMass(neighbor) → 失败则跳过
    mass_self -= fVar23; updated.mass[cell] -= fVar23             # L936
    flow[cell].x(LEFT) += fVar23                                   # L941-942
```

### 2.2 关键实现决策

1. **复用 `update_neighbour_liquid_mass`**（已实现，事件/病菌/温度链完整）——水平转移与向下转移共用同一质量转移函数，原版正是如此（L917 与 L1213 同函数）。
2. **flow 分量映射**：左流 → `flow[cell].x`（offset 0）、右流 → `flow[cell].y`（offset 4）、向下 → `flow[cell].z`（offset 8，已有）。与任务 3 的 `update_flow_texture`（x=(flow.x-flow.y)、y=(flow.w-flow.z)）自洽。半成品 flow_dir 映射（LEFT=0, RIGHT=1, DOWN=2, UP=3）佐证。
3. **SpawnFallingLiquid 保持现状**：水平分支照原版调用，但当前实现因 visible_grid 未分配恒 false → fallback 到 `UpdateNeighbourLiquidMass`（原版 L917 正是失败后的路径）。**水平扩散不依赖液滴**，本次不改液滴（属挤压链路后续任务）。
4. **边界防护**：cell-1 / cell+1 越界检查——原版用 `_invalid_parameter_noinfo_noreturn` 崩溃，Rust 用安全返回（与既有向下分支模式一致）。
5. **调用顺序**：原版 UpdateLiquid 顺序为 下 → 左 → 右，严格按此追加，不改变既有向下分支行为。

## 3. 修改位置

| 文件 | 改动 |
|---|---|
| `simDLL_RS/src/c2_physics/liquid_flow.rs` | `update_liquid`（L93-245）向下分支之后追加左/右分支；函数文档注释更新 |

无其他文件改动。`update_neighbour_liquid_mass` / `is_solid` / `is_liquid_permeable` / `spawn_falling_liquid` / `push_substance_change` 均已存在。

## 4. 测试计划（先单元后实测，6 项）

测试自建最小元素表（复用既有 `init_pressure_table` 模式：0=真空 state0 / 1=液体 state2 flow=50 max_mass 大 / 2=固体 state3），构造 6×5 或 6×6 SimData（内部格），持 `LIB_TESTS_LOCK`。

| # | 测试 | 场景 | 断言 |
|---|---|---|---|
| 1 | 左扩散 | 液体格 mass=100，左邻真空，下方固体 | 左邻变液体、mass=25、flow[cell].x=25、源格减 25 |
| 2 | 右扩散 | 对称，右邻真空 | 右邻变液体、flow[cell].y=25 |
| 3 | 双向扩散 | 左右都真空，下方固体 | 左右各得 12.5、flow.x=flow.y=12.5 |
| 4 | 水平门限 | 邻格液体但质量差小（fVar23 < min_horizontal_flow） | 不转移 |
| 5 | 邻格固体拒绝 | 邻格固体（properties&2 或 state==3） | 不转移 |
| 6 | 事件产生 | 左扩散到真空格 | substance_change_info 含该格（game 坐标转换） |

注意测试 1/2/3 的精确数值依赖流率上限字段（见风险 1），实现时若数值偏差以原版公式为准并记录。

## 5. 风险与已知边界

1. **流率上限字段双源冲突**（账本已有疑点）：原版水平分支读 `lVar1 + 8`（=viscosity，05_liquid_flow.c L877），既有向下分支用 `src_ld.flow`（@4）。类型引用（00_types_reference.c L8802-8809）标 flow@4/viscosity@8，但反编译两处分支都读 +8。本次与既有向下分支一致用 @4，**实测若水平扩散速度异常（太快/太慢）优先核查此字段**。
2. **SpawnFallingLiquid 不激活**：水平边缘的"斜下落液滴"动画不产生（恒 false），水平扩散本身正常——符合本次范围。
3. **气体**：neighbor state&3<2 判定允许液体流向气体格（原版语义）——气体格被液体替换，属正常物理。
4. **性能**：每液体格多 2 次邻格判定 + 元素表查询，与既有向下分支同量级，无新增热点。
5. **液滴/挤压链路**：明确不在本次范围（用户裁决留作后续独立任务，涉及 SpawnFallingLiquid 激活、visible_grid 分配、DisplaceLiquid 4 方向均分验证）。

## 6. 验证标准

单测全过后编译 → 用户自行部署 → 新开局或读档 → 倒一桶水观察：
- 水落地后应向两侧铺开（不再堆柱）
- 凹坑积水会向四周找平
- 既有行为不回退：下落、颜色、dig/沙盒

## 7. 后续任务（不在本次范围）

- 挤压链路深度研究：液滴（SpawnFallingLiquid + visible_grid 分配）、建砖 DisplaceLiquid 4 方向均分验证、异类占格质量转移、气体分层 bug
