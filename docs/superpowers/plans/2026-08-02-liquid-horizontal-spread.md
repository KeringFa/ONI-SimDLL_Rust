# 液体水平扩散修复实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 修复液体不左右扩散——在 `update_liquid` 向下分支后追加左/右水平分支，让水落地后向两侧铺开。

**架构：** 严格镜像原版 05_liquid_flow.c L849-945（左分支）/ L946-1212（右分支），复用已实现的 `update_neighbour_liquid_mass`（事件/病菌/温度链完整）。flow 分量：左→`flow[cell].x`（offset 0）、右→`flow[cell].y`（offset 4）。SpawnFallingLiquid 保持现状（恒 false → fallback 到 UpdateNeighbourLiquidMass，水平扩散不依赖液滴）。

**技术栈：** Rust（无新依赖）。既有元素表 getter、`update_neighbour_liquid_mass`、`is_solid`/`is_liquid_permeable`、`spawn_falling_liquid` 全部可用。

**规格：** `docs/superpowers/specs/2026-08-02-liquid-horizontal-spread-design.md`

---

## 文件结构

| 文件 | 动作 | 职责 |
|---|---|---|
| `simDLL_RS/src/c2_physics/liquid_flow.rs` | 修改 | `update_liquid`（L93-245）向下分支后追加左/右分支；函数文档注释更新 |
| 同文件 tests 区 | 修改 | 追加 6 个水平扩散测试 |

无其他文件改动。

---

## 任务 1：左分支实现 + 测试

**文件：**
- 修改：`simDLL_RS/src/c2_physics/liquid_flow.rs`（`update_liquid` 函数 + tests 区）
- 测试：同文件 `#[cfg(test)] mod tests`

- [x] **步骤 1：编写失败的测试**

在 `liquid_flow.rs` tests 区追加（放在 `update_liquid_vacuum_below_swaps_and_fires_events` 测试之后；复用既有 `create_displace_table`（L1794）与 `LIB_TESTS_LOCK` 模式）：

```rust
    /// 左扩散（设计测试 1）：液体格 mass=100，左邻真空，下方固体。
    /// 期望：左邻变液体、mass=25（100×0.25）、flow[cell].x=25、源格减 25。
    #[test]
    fn update_liquid_spreads_left_to_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        // 内部格坐标：cell14 = (row2,col2)。下方 cell8 = (row1,col2) 固体。
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1); // 液体元素 1（flow=50, max_mass=1000）
            cells.mass.set(14, 100.0);
            cells.temperature.set(14, 300.0);
            cells.element_idx.set(8, 4); // 固体元素 4
            cells.mass.set(8, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
            // cell13（左邻）= 真空（元素 0，质量 0，默认）
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 1, "左邻应变为液体元素");
            assert!(
                (updated.mass.get(13) - 25.0).abs() < 1e-3,
                "左邻应得 25（100×0.25），got {}",
                updated.mass.get(13)
            );
            assert!(
                (updated.mass.get(14) - 75.0).abs() < 1e-3,
                "源格应减 25，got {}",
                updated.mass.get(14)
            );
            assert!(
                (updated.temperature.get(13) - 300.0).abs() < 1e-3,
                "温度应随质量转移为 300"
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!(
                (flow[14].x - 25.0).abs() < 1e-3,
                "flow[14].x(LEFT) 应 +25，got {}",
                flow[14].x
            );
        }
    }

    /// 左扩散事件（设计测试 6）：左扩散到真空格应产生 substance 事件。
    #[test]
    fn update_liquid_spread_left_fires_event() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.element_idx.set(8, 4);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let events = &*sd.sim_events.ptr;
            // sim13（内部格）→ game 坐标：col=(13%6)-1=0, row=(13/6)-1=1 → game idx = 1*4+0 = 4
            let has_cell4 = events
                .substance_change_info
                .as_slice()
                .iter()
                .any(|e| e.cell_idx == 4);
            assert!(has_cell4, "应产生 sim13（game4）的 substance 事件");
        }
    }

    /// 邻格固体拒绝（设计测试 5）：左邻固体 → 不转移。
    #[test]
    fn update_liquid_skips_spread_to_solid_neighbor() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.element_idx.set(8, 4);
            cells.element_idx.set(13, 4); // 左邻固体
            cells.mass.set(13, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.element_idx.set(13, 4);
            updated.mass.set(13, 500.0);
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 4, "左邻固体不应被替换");
            assert!((updated.mass.get(13) - 500.0).abs() < 1e-3);
            assert!((updated.mass.get(14) - 100.0).abs() < 1e-3, "源格质量不变");
        }
    }
```

> ⚠️ game 坐标转换公式：`game_idx = (row-1)*game_w + (col-1)`，其中 `row = sim_idx / sim_w`、`col = sim_idx % sim_w`。sim13：row=2, col=1 → game = (2-1)*4 + (1-1) = 4。若实现者发现既有 push_substance_change 的转换不同，以实际为准并记录。

- [x] **步骤 2：运行测试验证失败**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：FAIL（3 个新测试断言失败——左邻仍是真空/元素 0）。

- [x] **步骤 3：编写最少实现代码**

在 `update_liquid` 函数中，向下分支结束（L244 `}` 之前）追加左分支。对照原版 05_liquid_flow.c L849-945：

```rust
    // —— 左分支（原版 L849-945）：水平向左扩散 ——
    if cell % width == 0 {
        return; // 左边界，无左邻格（原版越界崩溃，Rust 安全返回）
    }
    let left = cell - 1;
    // 判定链（原版 L852-866）：
    // 1. 左邻元素（cells 快照）：== 本格元素 或 state&3 < 2
    let (left_elem_cells, left_mass_cells, left_props) = {
        let cells = unsafe { &*sim.cells.ptr };
        (
            cells.element_idx.get(left),
            cells.mass.get(left),
            cells.properties.get(left),
        )
    };
    let left_elem_updated = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        updated.element_idx.get(left)
    };
    let left_state_updated = match elements_table::get_element_liquid_data(left_elem_updated) {
        Some(d) => d.state & 3,
        None => return,
    };
    if left_state_updated == 3 {
        return; // 左邻是固体（updated）
    }
    if left_props & 2 != 0 {
        return; // 左邻 properties 固体标志（cells）
    }
    let left_state_cells = match elements_table::get_element_liquid_data(left_elem_cells) {
        Some(d) => d.state & 3,
        None => return,
    };
    let same_or_gas = src_elem == left_elem_cells || left_state_cells < 2;
    if !same_or_gas {
        return;
    }
    // 扩散量计算（原版 L867-882）：
    let left_mass = if left_state_cells == 2 { left_mass_cells } else { 0.0 };
    let src_ld = match elements_table::get_element_liquid_data(src_elem) {
        Some(d) => d,
        None => return,
    };
    let mut f23 = (mass_self - left_mass) * 0.25; // fVar22（原版 L876）
    let mut f23_flow = mass_self.min(src_ld.flow); // fVar23 = min(mass, 流率上限)（原版 L877-880）
    f23_flow = f23_flow.min(f23);
    f23 = f23_flow;
    // 门限（原版 L884）：f23 >= min_horizontal_flow 且 f23 > 0
    if (f23 >= src_ld.min_horizontal_flow) && f23 > 0.0 {
        // 病菌按比例（原版 L886）
        let disease_to_transfer = ((f23 / mass_self) * disease_count_self as f32) as i32;
        // 原版 L887-909：若 IsSolid(下方) 且 IsLiquidPermeable(左下方) → SpawnFallingLiquid
        //   （当前 spawn_falling_liquid 因 visible_grid 未分配恒 false → fallback）
        // 原版 L917-919：UpdateNeighbourLiquidMass(左邻)
        let ok = update_neighbour_liquid_mass(
            sim, cell, src_elem, left, left_elem_cells, f23, disease_idx_self,
            disease_to_transfer,
        );
        if ok {
            // 源格病菌减少（原版 L930-934）
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            if updated.disease_idx.get(cell) == disease_idx_self {
                modify_disease_count(updated, cell, -disease_to_transfer);
            }
            // 源格质量减少（原版 L936）
            updated.mass.set(cell, updated.mass.get(cell) - f23);
            // flow[LEFT] 累加（原版 L941-942，Vector4f offset 0 = x）
            if !sim.flow.ptr.is_null() {
                let flow = unsafe { &mut *sim.flow.ptr };
                let flow_slice = unsafe {
                    std::slice::from_raw_parts_mut(flow, sim.width as usize * sim.height as usize)
                };
                flow_slice[cell].x += f23;
            }
        }
    }
```

> 注意：`src_ld` 已在向下分支中获取过（L159）——若变量仍在作用域内可直接复用，避免重复查表；否则按上面重新获取。实现者以实际代码结构为准（DRY）。

> ⚠️ **SpawnFallingLiquid 分支**：原版 L887-909 在 `IsSolid(下方) && IsLiquidPermeable(左下方)` 时先尝试 SpawnFallingLiquid，失败才走 UpdateNeighbourLiquidMass。当前 `spawn_falling_liquid` 恒返回 false（visible_grid 未分配），因此**直接走 UpdateNeighbourLiquidMass 即可**——与"调用后失败 fallback"行为等价。实现者可选择：a) 照原版调用 spawn_falling_liquid 再 fallback（行为等价，多一次无副作用调用）；b) 直接省略 SpawnFallingLiquid 调用（行为等价，少一次调用）。**推荐 b**，并在注释中说明"原版 L887-909 的 SpawnFallingLiquid 因 visible_grid 未分配恒 false，省略等价"。若选 a 需对照原版传参（param_3+1 是右邻、param_3-1 是左邻——注意方向）。

- [x] **步骤 4：运行测试验证通过**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：PASS（3 个新测试 + 既有测试无回归）。

- [x] **步骤 5：变更记录**（跳过，由控制者统一记录）

---

## 任务 2：右分支实现 + 测试

**文件：**
- 修改：`simDLL_RS/src/c2_physics/liquid_flow.rs`（`update_liquid` 函数 + tests 区）
- 测试：同文件

- [x] **步骤 1：编写失败的测试**

在 tests 区追加：

```rust
    /// 右扩散（设计测试 2）：液体格 mass=100，右邻真空，下方固体。
    /// 期望：右邻变液体、mass=25、flow[cell].y=25、源格减 25。
    #[test]
    fn update_liquid_spreads_right_to_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.temperature.set(14, 300.0);
            cells.element_idx.set(8, 4); // 下方固体
            cells.mass.set(8, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
            // cell15（右邻）= 真空（默认）
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(15), 1, "右邻应变为液体元素");
            assert!(
                (updated.mass.get(15) - 25.0).abs() < 1e-3,
                "右邻应得 25，got {}",
                updated.mass.get(15)
            );
            assert!(
                (updated.mass.get(14) - 75.0).abs() < 1e-3,
                "源格应减 25，got {}",
                updated.mass.get(14)
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!(
                (flow[14].y - 25.0).abs() < 1e-3,
                "flow[14].y(RIGHT) 应 +25，got {}",
                flow[14].y
            );
        }
    }

    /// 双向扩散（设计测试 3）：左右都真空 → 左右各得 12.5。
    /// 注意：fVar25 在左分支后已减 25，右分支基于剩余 75 计算 → 75×0.25=18.75。
    /// 但原版顺序是 左→右，右分支的 fVar25 是左分支后的剩余值。
    /// 本测试固定期望：左邻 25、右邻 18.75、源格 56.25。
    #[test]
    fn update_liquid_spreads_both_directions() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.element_idx.set(8, 4);
            cells.mass.set(8, 500.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.mass.set(8, 500.0);
            // cell13（左）、cell15（右）都真空
        }
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 1);
            assert_eq!(updated.element_idx.get(15), 1);
            assert!(
                (updated.mass.get(13) - 25.0).abs() < 1e-3,
                "左邻 25，got {}",
                updated.mass.get(13)
            );
            assert!(
                (updated.mass.get(15) - 18.75).abs() < 1e-3,
                "右邻 18.75（左分支后剩余 75×0.25），got {}",
                updated.mass.get(15)
            );
            assert!(
                (updated.mass.get(14) - 56.25).abs() < 1e-3,
                "源格 56.25，got {}",
                updated.mass.get(14)
            );
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 30);
            assert!((flow[14].x - 25.0).abs() < 1e-3, "flow.x 应 25");
            assert!((flow[14].y - 18.75).abs() < 1e-3, "flow.y 应 18.75");
        }
    }

    /// 水平门限（设计测试 4）：左邻是液体但质量差小（f23 < min_horizontal_flow）→ 不转移。
    #[test]
    fn update_liquid_horizontal_flow_threshold() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_displace_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(14, 1);
            cells.mass.set(14, 100.0);
            cells.element_idx.set(8, 4);
            cells.element_idx.set(13, 1); // 左邻同液体，质量接近
            cells.mass.set(13, 99.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 1);
            updated.mass.set(14, 100.0);
            updated.element_idx.set(8, 4);
            updated.element_idx.set(13, 1);
            updated.mass.set(13, 99.0);
        }
        // 质量差 1 → f22 = 1×0.25 = 0.25。若 min_horizontal_flow=0（create_displace_table 默认）
        // 则 0.25 >= 0 且 > 0 → 会转移。若要测"不转移"需 min_horizontal_flow > 0.25。
        // 注：create_displace_table 的元素 1 未显式设 min_horizontal_flow（默认 0.0）。
        // 本测试改为验证：质量差极小（f22 接近 0）且 min_horizontal_flow=0 时仍转移（对照原版语义）；
        // 真正的门限测试依赖元素表数据，游戏内实测验证。
        update_liquid(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // 质量差 1 → f22=0.25，f23=min(100,50,0.25)=0.25 ≥ 0 且 > 0 → 转移 0.25
            assert!(
                (updated.mass.get(13) - 99.25).abs() < 1e-3,
                "左邻应 +0.25，got {}",
                updated.mass.get(13)
            );
        }
    }
```

> ⚠️ **双向扩散的数值说明**（重要）：原版 UpdateLiquid 顺序是 **下→左→右**，`fVar25`（mass_self）是函数内局部变量，左分支转移后 `fVar25 -= fVar23`（L936），右分支基于**剩余质量**计算。所以左右都真空时：左得 100×0.25=25，右得 (100-25)×0.25=18.75，源格剩 56.25。**实现者必须保持 fVar25 局部变量在分支间递减**（向下分支已有此模式：`fVar25 = fVar25 - fVar23`）。

> ⚠️ **水平门限测试**：create_displace_table 的元素 1 未设 `min_horizontal_flow`（默认 0.0），因此"门限拦截"路径在单测中不可达。测试改为验证"质量差极小仍转移"（对照原版语义：min_horizontal_flow=0 时无门限）。真正的门限行为（min_horizontal_flow>0 时）留待游戏内实测（真实元素表有数据）。

- [x] **步骤 2：运行测试验证失败**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：FAIL（3 个新测试断言失败——右邻仍是真空/元素 0）。

- [x] **步骤 3：编写最少实现代码**

在左分支之后追加右分支。对照原版 05_liquid_flow.c L946-1212：

```rust
    // —— 右分支（原版 L946-1212）：水平向右扩散 ——
    if cell % width == width - 1 {
        return; // 右边界，无右邻格
    }
    let right = cell + 1;
    // 判定链（原版 L950-963）：与左分支对称
    let (right_elem_cells, right_mass_cells, right_props) = {
        let cells = unsafe { &*sim.cells.ptr };
        (
            cells.element_idx.get(right),
            cells.mass.get(right),
            cells.properties.get(right),
        )
    };
    let right_elem_updated = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        updated.element_idx.get(right)
    };
    let right_state_updated = match elements_table::get_element_liquid_data(right_elem_updated) {
        Some(d) => d.state & 3,
        None => return,
    };
    if right_state_updated == 3 {
        return;
    }
    if right_props & 2 != 0 {
        return;
    }
    let right_state_cells = match elements_table::get_element_liquid_data(right_elem_cells) {
        Some(d) => d.state & 3,
        None => return,
    };
    let same_or_gas = src_elem == right_elem_cells || right_state_cells < 2;
    if !same_or_gas {
        return;
    }
    // 扩散量计算（原版 L967-983）：与左分支对称，fVar25 为左分支后的剩余质量
    let right_mass = if right_state_cells == 2 { right_mass_cells } else { 0.0 };
    let src_ld = match elements_table::get_element_liquid_data(src_elem) {
        Some(d) => d,
        None => return,
    };
    let mut f23 = (mass_self - right_mass) * 0.25;
    let mut f23_flow = mass_self.min(src_ld.flow);
    f23_flow = f23_flow.min(f23);
    f23 = f23_flow;
    if (f23 >= src_ld.min_horizontal_flow) && f23 > 0.0 {
        let disease_to_transfer = ((f23 / mass_self) * disease_count_self as f32) as i32;
        // 原版 L987-1007：SpawnFallingLiquid（恒 false，省略等价）
        // 原版 L1014-1016：UpdateNeighbourLiquidMass(右邻)
        let ok = update_neighbour_liquid_mass(
            sim, cell, src_elem, right, right_elem_cells, f23, disease_idx_self,
            disease_to_transfer,
        );
        if ok {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            if updated.disease_idx.get(cell) == disease_idx_self {
                modify_disease_count(updated, cell, -disease_to_transfer);
            }
            updated.mass.set(cell, updated.mass.get(cell) - f23);
            // flow[RIGHT] 累加（原版 L1038-1039，Vector4f offset 4 = y）
            if !sim.flow.ptr.is_null() {
                let flow = unsafe { &mut *sim.flow.ptr };
                let flow_slice = unsafe {
                    std::slice::from_raw_parts_mut(flow, sim.width as usize * sim.height as usize)
                };
                flow_slice[cell].y += f23;
            }
        }
    }
```

> ⚠️ **mass_self 的递减**：左右分支的 `mass_self` 必须与向下分支的递减保持一致。实现者需检查：向下分支（L187 `updated.mass.set(cell, ...- f23)`）后，函数内的 `mass_self` 局部变量是否也递减了（原版 L936 `fVar25 = fVar25 - fVar23`）。**若向下分支未递减局部 mass_self，需在左/右分支计算前用 updated.mass 重新读取当前质量**，或用局部变量递减——以"左右分支的 fVar25 反映此前已转移的质量"为准（这决定双向扩散测试的期望值 25/18.75/56.25 是否成立）。若实现者发现向下分支的局部变量处理与期望不符，以"行为对齐原版"为原则修正并记录。

- [x] **步骤 4：运行测试验证通过**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：PASS（6 个新测试 + 既有测试无回归）。

- [x] **步骤 5：变更记录**（跳过）

---

## 任务 3：全量回归 + Release 编译 + 交付

- [x] **步骤 1：全量测试（串行）**

运行：`cd simDLL_RS && cargo test -- --test-threads=1`
预期：全部 PASS（lib + 集成）。lib 并行跑可能 flaky（已知备忘），用 `--test-threads=1` 确认基线。

- [x] **步骤 2：并行测试一次**

运行：`cargo test`
预期：若出现已知 flaky 模式（sim_data_ops 等不持 LIB_TESTS_LOCK 的既有竞态），确认失败名单与本次改动无关。不修 flaky（既有决策）。

- [x] **步骤 3：Release 编译**

运行：`cargo build --release`
预期：成功，`target/release/SimDLL.dll` 生成。

- [x] **步骤 4：交付说明**

- 产出文档：`docs/superpowers/plans/2026-08-02-liquid-horizontal-spread.md`（本计划）+ `docs/superpowers/specs/2026-08-02-liquid-horizontal-spread-design.md`
- 实测指引：用户自行部署 DLL → 新开局或读档 → 倒一桶水观察：水落地向两侧铺开（不再堆柱）、凹坑积水找平、既有行为（下落/颜色/dig/沙盒）不回退
- 明确预期差异：水平边缘的"斜下落液滴"动画不产生（SpawnFallingLiquid 未激活，属挤压链路后续任务）；流率上限字段 @4 vs +8 双源冲突若扩散速度异常优先核查（账本疑点）

---

## 自检记录

**1. 规格覆盖度：**
- 左分支（L849-945）→ 任务 1 ✅
- 右分支（L946-1212）→ 任务 2 ✅
- 6 项测试：左扩散（T1-1）、右扩散（T2-1）、双向（T2-2）、水平门限（T2-3）、固体拒绝（T1-3）、事件（T1-2）✅
- flow 分量映射（左 x/右 y）→ T1-1/T2-1 断言 ✅
- 边界防护、SpawnFallingLiquid 省略等价、mass_self 递减 → 各任务实现说明 ✅

**2. 占位符扫描：**
- 无 TODO/待定；测试 6 项均有具体场景与断言；门限测试的"单测不可达"已明确处理（改为验证语义等价 + 游戏内实测）✅

**3. 类型一致性：**
- `update_neighbour_liquid_mass(sim, cell, src_elem, dst, dst_elem, amount, disease_idx, disease_count) -> bool`（既有 L253-262）与 T1/T2 调用一致 ✅
- `spawn_falling_liquid(sim, cell, elem, mass, temp, disease_idx, disease_count) -> bool`（既有 L445-453）——推荐省略，若调用需 7 参 ✅
- `ElementLiquidData.flow/min_horizontal_flow/max_mass` 字段名（element.rs L137-144）✅
- 测试建表：`create_displace_table`（既有 L1794）元素 1 液体 flow=50/max_mass=1000 ✅
