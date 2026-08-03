# 温度模块补全实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans（本会话固定**内联执行**，
> 用户已两次终止子代理）逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 补齐温度域剩余 sim 逻辑——DoLoadTimeStateTransition、DoPartialHeatTransition、
DoPartialMelt 完整版，并对透气砖/网格砖换热做验证收尾（无 sim 改动）。

**架构：** 三个函数全部放入 `c2_physics/temperature.rs`（镜像 04_temperature.c 归属）；
`liquid_flow.rs` 的 `spawn_falling_liquid` / `do_evaporate` / `clear_disease` 提升 `pub(crate)`
供 temperature.rs 使用；PostProcessCell 液体分支 4 方向调用 + 气体分支 melt 门控改接新函数；
save_load.rs 调用点改接新函数，删除 sim_data.rs 空桩。

**技术栈：** Rust（cdylib），cargo test 串行（`--test-threads=1`，项目已知并行 flaky），
测试命令在 `simDLL_RS` 目录执行；元素表访问器 `get_element_by_idx` / `get_element_temperature_data`。

---

## 文件结构

| 文件 | 职责 | 变更 |
|------|------|------|
| `simDLL_RS/src/c2_physics/temperature.rs` | 温度域逻辑（含三个新函数 + 测试） | 修改 |
| `simDLL_RS/src/c2_physics/liquid_flow.rs` | 液体后处理（辅助函数可见性 + 两处接线 + 删除旧判定版） | 修改 |
| `simDLL_RS/src/a_framework/save_load.rs:296` | 加载逐格调用点改接新函数 | 修改 |
| `simDLL_RS/src/a_framework/sim_data.rs:631-638` | 删除 `do_load_time_state_transition` 空桩 | 修改 |
| `代办.txt` | 追加温度补全记录 + 验证清单 | 修改 |

## 任务 1：do_load_time_state_transition（加载时相变修正）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/temperature.rs`（新增函数 + 测试）
- 修改：`simDLL_RS/src/a_framework/save_load.rs:296`（调用点）
- 修改：`simDLL_RS/src/a_framework/sim_data.rs:631-638`（删空桩）

### 步骤 1：编写失败的测试

在 `temperature.rs` 的 `mod tests` 内（`init_phase_table` 之后）追加：

```rust
    #[test]
    fn load_time_transition_high_converts_element_and_cools() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 14, 1, 10.0, 105.0); // 水 105℃ > high+3(103)
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 3, "应转为蒸汽");
        assert!((u.temperature.get(14) - 103.5).abs() < 1e-4, "温度应 -1.5 → 103.5");
    }

    #[test]
    fn load_time_transition_low_converts_element_and_warms() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 14, 1, 10.0, -5.0); // 水 -5℃ < low-3(-3)
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 2, "应转为冰");
        assert!((u.temperature.get(14) + 3.5).abs() < 1e-4, "温度应 +1.5 → -3.5");
    }

    #[test]
    fn load_time_transition_within_window_and_boundary_is_noop() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table();
        let mut sd = make_sim();
        fill(&mut sd, 14, 1, 10.0, 97.0); // 窗口内
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 1);
        assert_eq!(u.temperature.get(14), 97.0);
        fill(&mut sd, 14, 1, 10.0, -3.0); // 恰在 low-3 边界 → 不触发
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 1);
        assert_eq!(u.temperature.get(14), -3.0);
        fill(&mut sd, 14, 1, 10.0, 103.0); // 恰在 high+3 边界 → 不触发
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 1);
        assert_eq!(u.temperature.get(14), 103.0);
    }

    #[test]
    fn load_time_transition_without_target_is_noop() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table();
        // 冰(2) 无转换目标：手动置 0xFFFF
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data[2].low_temp_transition_idx = 0xFFFF;
            table.temperature_data[2].high_temp_transition_idx = 0xFFFF;
        }
        let mut sd = make_sim();
        fill(&mut sd, 14, 2, 10.0, 2000.0); // 冰 2000℃ 但无高温目标
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 2);
        assert_eq!(u.temperature.get(14), 2000.0);
        fill(&mut sd, 14, 2, 10.0, -100.0); // 冰 无低温目标 → 也不动
        do_load_time_state_transition(&mut sd, 14);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 2);
        assert_eq!(u.temperature.get(14), -100.0);
    }
```

### 步骤 2：运行测试验证失败

运行：`cargo test -p simdll_rs c2_physics::temperature::tests::load_time_transition -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：FAIL，编译错误 "cannot find function `do_load_time_state_transition` in this scope"。

### 步骤 3：编写最少实现代码

在 `temperature.rs`（`do_state_transition` 之前的公共函数区）新增：

```rust
/// DoLoadTimeStateTransition — 加载时温度状态修正（原版 04_temperature.c L7-73）。
/// 读取 updated_cells：温度越界 [lowTemp−3, highTemp+3] 时 ±1.5 换元素，
/// 使存档中轻微越界的格子回到相变窗口内。
pub(crate) fn do_load_time_state_transition(sim: &mut SimData, cell: usize) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if cell >= updated.element_idx.len() || cell >= updated.temperature.len() {
        return;
    }
    let elem = updated.element_idx.get(cell);
    let Some(etd) = elements_table::get_element_temperature_data(elem) else {
        return;
    };
    let mut temp = updated.temperature.get(cell);
    // 原版 L22-24：temp >= lowTemp−3 或无低温转换目标 → 只查高温侧
    if temp >= etd.low_temp - 3.0 || etd.low_temp_transition_idx == 0xFFFF {
        // 原版 L25-32：highTemp+3 < temp 且有高温转换目标 → 高温转换（temp −1.5、换元素）
        if etd.high_temp + 3.0 < temp && etd.high_temp_transition_idx != 0xFFFF {
            if elements_table::get_element_temperature_data(etd.high_temp_transition_idx).is_none() {
                tracing::warn!(cell, target = etd.high_temp_transition_idx, "DoLoadTimeStateTransition: invalid high transition target");
                return;
            }
            temp -= 1.5;
            updated.temperature.set(cell, temp);
            updated.element_idx.set(cell, etd.high_temp_transition_idx);
        }
    } else {
        // 原版 L33-39：低温转换（temp +1.5、换元素）
        if elements_table::get_element_temperature_data(etd.low_temp_transition_idx).is_none() {
            tracing::warn!(cell, target = etd.low_temp_transition_idx, "DoLoadTimeStateTransition: invalid low transition target");
            return;
        }
        temp += 1.5;
        updated.temperature.set(cell, temp);
        updated.element_idx.set(cell, etd.low_temp_transition_idx);
    }
    if temp > 10000.0 {
        tracing::warn!(cell, temp, "DoLoadTimeStateTransition: temperature exceeds SIM_MAX_TEMPERATURE");
    }
}
```

检查 `temperature.rs` 顶部是否有 `use crate::b_elements::elements_table;`（或等价导入）；
现有代码已调用 `elements_table::get_element_temperature_data`，若无直接 `use`，用
`crate::b_elements::elements_table::get_element_temperature_data(...)` 全路径。

### 步骤 4：运行测试验证通过

运行：`cargo test -p simdll_rs c2_physics::temperature::tests::load_time_transition -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：4 条 load_time_transition 测试全部 PASS。

### 步骤 5：改接 save_load 调用点 + 删除空桩

`save_load.rs:296` 把：
```rust
                // L104: DoLoadTimeStateTransition
                sim_data.do_load_time_state_transition(cell_idx);
```
改为：
```rust
                // L104: DoLoadTimeStateTransition
                crate::c2_physics::temperature::do_load_time_state_transition(sim_data, cell_idx);
```

`sim_data.rs` 删除空桩方法（L631-638，含文档注释）：
```rust
    /// DoLoadTimeStateTransition — 加载时温度状态转变。
    /// 对照源码 04_temperature.c L7-73。
    ///
    /// A3 阶段实现桩版本（不执行温度转变），C 类阶段完善真实逻辑。
    pub fn do_load_time_state_transition(&mut self, _cell_idx: usize) {
        // 桩：A3 阶段不执行温度转变
    }
```

### 步骤 6：全量回归 + 提交

运行：`cargo test -p simdll_rs -- --test-threads=1 2>$null; $LASTEXITCODE` → 全部 PASS

提交：
```bash
git add simDLL_RS/src/c2_physics/temperature.rs simDLL_RS/src/a_framework/save_load.rs simDLL_RS/src/a_framework/sim_data.rs
git commit -m "feat(temperature): implement DoLoadTimeStateTransition (L7-73)"
```

## 任务 2：do_partial_heat_transition（部分沸腾/分批相变）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/liquid_flow.rs`（`spawn_falling_liquid` / `do_evaporate` /
  `clear_disease` 提升 `pub(crate)`；PostProcessCell 液体分支接线）
- 修改：`simDLL_RS/src/c2_physics/temperature.rs`（新增函数 + 测试）

### 步骤 1：提升 liquid_flow.rs 三个辅助函数可见性

`fn spawn_falling_liquid(` → `pub(crate) fn spawn_falling_liquid(`

`fn clear_disease(cells: &mut CellSOA, cell: usize) {` → `pub(crate) fn clear_disease(...)`

`fn do_evaporate(sim: &mut SimData, cell: usize) {` → `pub(crate) fn do_evaporate(...)`

### 步骤 2：编写失败的测试

在 `temperature.rs` 测试模块追加测试表与用例：

```rust
    /// 部分沸腾测试表：dummy(0) / 水 water(1, state2, high=100, high_trans=3) /
    /// 冰 ice(2) / 蒸汽 steam(3, state1) / 热固体 hot(4, state3, tc=1, low=-100, high=500) /
    /// 熔岩 lava(5, state2, tc=1)。
    fn init_partial_heat_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.temperature_data.clear();
        let mut mk = |state: u8, low: f32, high: f32, tc: f32, high_trans: u16| {
            let mut e = Element::default();
            e.state = state;
            e.low_temp = low;
            e.high_temp = high;
            e.thermal_conductivity = tc;
            e.high_temp_transition_idx = high_trans;
            e
        };
        table.elements.push(Element::default()); // 0 dummy
        table.elements.push(mk(2, 0.0, 100.0, 1.0, 3)); // 1 水 → 蒸汽
        table.elements.push(Element::default()); // 2 冰
        table.elements.push(mk(1, 100.0, 999.0, 1.0, 0)); // 3 蒸汽
        table.elements.push(mk(3, -100.0, 500.0, 1.0, 0xFFFF)); // 4 热固体
        table.elements.push(mk(2, 0.0, 500.0, 1.0, 0)); // 5 熔岩
        // temperature_data 与 elements 同序（dummy 默认 0 即可，函数不读它）
        table.temperature_data.resize(6, crate::b_elements::element::ElementTemperatureData::default());
    }

    #[test]
    fn partial_heat_gas_target_places_below() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_partial_heat_table();
        let mut sd = make_sim();
        // cell14=水 10kg 20℃；neighbor15=热固体 100kg 600℃；below20=蒸汽
        fill(&mut sd, 14, 1, 10.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 600.0);
        fill(&mut sd, 20, 3, 1.0, 300.0);
        assert!(do_partial_heat_transition(&mut sd, 14, 15));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!((u.mass.get(14) - 5.0).abs() < 1e-4, "本格应剩 5kg，got {}", u.mass.get(14));
        assert_eq!(u.element_idx.get(14), 1, "本格仍是水");
        assert_eq!(u.element_idx.get(20), 3, "下方格变蒸汽");
        assert!((u.mass.get(20) - 5.0).abs() < 1e-4);
        assert!((u.temperature.get(20) - 103.0).abs() < 1e-4, "下方蒸汽应 103℃（high+3）");
        assert!((u.temperature.get(15) - 595.85).abs() < 1e-2, "热固体应降温，got {}", u.temperature.get(15));
    }

    #[test]
    fn partial_heat_liquid_target_spawns_droplet() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_partial_heat_table();
        let mut sd = make_sim();
        // 水 → 目标改为熔岩(5)：水(1).high_trans=5
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements[1].high_temp_transition_idx = 5;
        }
        fill(&mut sd, 14, 1, 10.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 600.0);
        assert!(do_partial_heat_transition(&mut sd, 14, 15));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!(u.mass.get(14) < 0.01, "液体目标分支 DisplaceLiquid 后排空本格，got {}", u.mass.get(14));
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.spawn_liquid_info.len(), 1, "应生成 1 条液滴事件");
        let drop = events.spawn_liquid_info.get(0);
        assert_eq!(drop.element_idx, 5);
        assert!((drop.mass - 5.0).abs() < 1e-4);
        assert!((drop.temperature - 103.0).abs() < 1e-4);
    }

    #[test]
    fn partial_heat_precondition_gates_and_restore() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_partial_heat_table();
        let mut sd = make_sim();
        // 质量 < 5 → false
        fill(&mut sd, 14, 1, 4.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 600.0);
        assert!(!do_partial_heat_transition(&mut sd, 14, 15));
        // 邻格不够热（neigh_temp > high+3 不满足）→ false
        fill(&mut sd, 14, 1, 10.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 102.0);
        assert!(!do_partial_heat_transition(&mut sd, 14, 15));
        // 无高温转换目标（冰）→ false
        fill(&mut sd, 14, 2, 10.0, 20.0);
        assert!(!do_partial_heat_transition(&mut sd, 14, 15));
        // 四邻全固体 → DisplaceLiquid 失败 → 恢复并返回 false
        fill(&mut sd, 14, 1, 10.0, 20.0);
        fill(&mut sd, 15, 4, 100.0, 600.0);
        for n in [7usize, 13, 15, 21] {
            fill(&mut sd, n, 4, 100.0, 300.0); // 固体邻居（7/13/15/21 = 上/左/右/下）
        }
        assert!(!do_partial_heat_transition(&mut sd, 14, 15));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!((u.mass.get(14) - 10.0).abs() < 1e-4, "失败应恢复质量，got {}", u.mass.get(14));
        assert!((u.temperature.get(15) - 600.0).abs() < 1e-4, "失败应恢复邻格温度");
    }
```

### 步骤 3：运行测试验证失败

运行：`cargo test -p simdll_rs c2_physics::temperature::tests::partial_heat -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：FAIL，编译错误 "cannot find function `do_partial_heat_transition`"。

### 步骤 4：编写最少实现代码

在 `temperature.rs` 新增：

```rust
/// DoPartialHeatTransition — 部分沸腾/分批相变（原版 04_temperature.c L2105-2471）。
/// 大质量格（≥5kg）被极热邻格加热：抽邻格热量 heat=(high+3−本格温)×TC×5，
/// 把恰好 5kg 转化为高温相变目标（气体→排下方格；液体→SpawnFallingLiquid）；
/// 失败全额恢复（质量/病菌/邻格温度）并返回 false。
pub(crate) fn do_partial_heat_transition(sim: &mut SimData, cell: usize, neighbor: usize) -> bool {
    let width = sim.width as usize;
    let total = width * sim.height as usize;
    if cell >= total || neighbor >= total {
        return false;
    }
    if sim.updated_cells.ptr.is_null() || sim.sim_events.ptr.is_null() {
        return false;
    }
    let updated = unsafe { &*sim.updated_cells.ptr };
    // 原版 L2138：mass < 5.0 → false（== 5.0 放行）
    let mass_cell = updated.mass.get(cell);
    if mass_cell < 5.0 {
        return false;
    }
    let cell_elem = updated.element_idx.get(cell);
    let neigh_elem = updated.element_idx.get(neighbor);
    let (Some(cell_el), Some(neigh_el)) = (
        elements_table::get_element_by_idx(cell_elem),
        elements_table::get_element_by_idx(neigh_elem),
    ) else {
        return false;
    };
    // 原版 L2146：cell 高温转换目标（Element offset 6）
    let target = cell_el.high_temp_transition_idx;
    if target == 0xFFFF {
        return false;
    }
    let Some(target_el) = elements_table::get_element_by_idx(target) else {
        return false;
    };
    let cell_high = cell_el.high_temp;
    let neigh_low = neigh_el.low_temp;
    let cell_tc = cell_el.thermal_conductivity;
    let neigh_tc = neigh_el.thermal_conductivity;
    let cell_temp = updated.temperature.get(cell);
    let neigh_temp = updated.temperature.get(neighbor);
    let f22 = cell_high + 3.0;
    // 原版 L2147-2152：邻格温度 > cell_high+3；邻格温度 >= neigh_low+13；本格温度 < cell_high−3
    if neigh_temp <= f22 {
        return false;
    }
    if neigh_temp < neigh_low + 13.0 {
        return false;
    }
    if cell_high - 3.0 <= cell_temp {
        return false;
    }
    // 原版 L2156：heat = (f22 − cell_temp) × cell_tc × 5.0
    let heat = (f22 - cell_temp) * cell_tc * 5.0;
    // 原版 L2158-2162：f21 = mass[neighbor] × neigh_tc；f21×10 < heat → false
    let mass_neigh = updated.mass.get(neighbor);
    let mut f21 = mass_neigh * neigh_tc;
    if f21 * 10.0 < heat {
        return false;
    }
    // 原版 L2165：邻格新温度
    f21 = (neigh_temp * f21 - heat) / f21;
    let disease_cell = updated.disease_count.get(cell);
    let disease_idx_cell = updated.disease_idx.get(cell);
    let i16 = (disease_cell as f32 * (5.0 / mass_cell)) as i32;
    let target_state = target_el.state & 3;

    // —— 目标为气体（原版 L2171-2290）——
    if target_state == 1 {
        // 扣减段：本格 −5kg、病菌按比例、邻格降温
        {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.mass.set(cell, mass_cell - 5.0);
            let remain = disease_cell - i16;
            if remain < 1 {
                crate::c2_physics::liquid_flow::clear_disease(updated, cell);
            } else {
                updated.disease_count.set(cell, remain);
            }
            updated.temperature.set(neighbor, f21);
        }
        let below = cell + width;
        let below_elem = if below < total {
            unsafe { (*sim.updated_cells.ptr).element_idx.get(below) }
        } else {
            u16::MAX
        };
        let below_is_gas = matches!(
            elements_table::get_element_by_idx(below_elem),
            Some(e) if (e.state & 3) == 1
        );
        if below_is_gas {
            if crate::c2_physics::liquid_flow::displace_gas(sim, below, below_elem) {
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                updated.element_idx.set(below, target);
                updated.mass.set(below, 5.0);
                updated.temperature.set(below, f22);
                crate::c2_physics::liquid_flow::push_substance_change(sim, below);
                return true;
            }
            // 原版此处进崩溃标签（L2286）；安全镜像：走恢复分支
        } else if crate::c2_physics::liquid_flow::displace_liquid(sim, cell, cell_elem) {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.element_idx.set(cell, target);
            updated.mass.set(cell, 5.0);
            updated.temperature.set(cell, f22);
            crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
            return true;
        }
        // 恢复段（原版 L2264-2268 同款：质量/病菌/邻格温度还原）
        {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.mass.set(cell, mass_cell);
            updated.disease_count.set(cell, disease_cell);
            updated.temperature.set(neighbor, neigh_temp);
        }
        return false;
    }

    // —— 目标为液体（原版 L2291-2370）——
    if target_state == 2 {
        {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.mass.set(cell, mass_cell - 5.0);
            let remain = disease_cell - i16;
            if remain < 1 {
                crate::c2_physics::liquid_flow::clear_disease(updated, cell);
            } else {
                updated.disease_count.set(cell, remain);
            }
            updated.temperature.set(neighbor, f21);
        }
        if crate::c2_physics::liquid_flow::displace_liquid(sim, cell, cell_elem) {
            if crate::c2_physics::liquid_flow::spawn_falling_liquid(
                sim, cell, target, 5.0, f22, disease_idx_cell, i16,
            ) {
                return true;
            }
            // 液滴失败 → 就地转化（原版 L2342-2350）
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.element_idx.set(cell, target);
            updated.mass.set(cell, 5.0);
            updated.temperature.set(cell, f22);
            crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
            return true;
        }
        {
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.mass.set(cell, mass_cell);
            updated.disease_count.set(cell, disease_cell);
            updated.temperature.set(neighbor, neigh_temp);
        }
        return false;
    }

    tracing::warn!(target_state, "DoPartialHeatTransition: unexpected target state");
    false
}
```

### 步骤 5：接线 PostProcessCell 液体分支

`liquid_flow.rs` L2309 附近，把：
```rust
    // DoPartialHeatTransition 4 方向（L2705-2721）—— 留桩
    // —— DoSublimation（L2722-2863）：相变发射留桩，仅保留 accumulatedFlow 累加（L2831）——
```
改为：
```rust
    // DoPartialHeatTransition 4 方向（原版 L2705-2721）：任一成功 → 提前返回
    if crate::c2_physics::temperature::do_partial_heat_transition(sim, cell, cell.wrapping_sub(width))
        || crate::c2_physics::temperature::do_partial_heat_transition(sim, cell, cell.wrapping_sub(1))
        || crate::c2_physics::temperature::do_partial_heat_transition(sim, cell, cell + 1)
        || crate::c2_physics::temperature::do_partial_heat_transition(sim, cell, cell + width)
    {
        return;
    }
    // —— DoSublimation（L2722-2863）——
```

### 步骤 6：运行测试验证通过

运行：`cargo test -p simdll_rs c2_physics::temperature::tests::partial_heat -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：3 条 partial_heat 测试全部 PASS。

### 步骤 7：全量回归 + 提交

运行：`cargo test -p simdll_rs -- --test-threads=1 2>$null; $LASTEXITCODE` → 全部 PASS

提交：
```bash
git add simDLL_RS/src/c2_physics/temperature.rs simDLL_RS/src/c2_physics/liquid_flow.rs
git commit -m "feat(temperature): implement DoPartialHeatTransition (L2105-2471)"
```

## 任务 3：do_partial_melt（部分熔化完整版）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/liquid_flow.rs`（删除旧判定版 `do_partial_melt`；气体分支门控改接）
- 修改：`simDLL_RS/src/c2_physics/temperature.rs`（新增完整版 + 测试）

### 步骤 1：删除 liquid_flow.rs 旧判定版

删除 `liquid_flow.rs` L1270-1290 附近的 `do_partial_melt` 函数（含其文档注释），
整段以 `/// DoPartialMelt 的"是否熔融"判定近似...` 开头到 `fn do_partial_melt(...) { ... }` 结束。

### 步骤 2：编写失败的测试

在 `temperature.rs` 测试模块追加（复用任务 2 的 `init_partial_heat_table`）：

```rust
    #[test]
    fn partial_melt_spawns_droplet_and_deducts_solid() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_partial_heat_table();
        // 热固体(4) 需 high_temp_transition_idx → 熔岩(5)；蒸汽(3) 作为高温气体格
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements[4].high_temp_transition_idx = 5; // 固体 → 熔岩
            table.elements[4].low_temp = -100.0;
            table.elements[4].high_temp = 500.0;
            table.elements[4].thermal_conductivity = 1.0;
        }
        let mut sd = make_sim();
        // cell14=蒸汽(气体格) 100kg 800℃；neighbor15=热固体 100kg 20℃
        fill(&mut sd, 14, 3, 100.0, 800.0);
        fill(&mut sd, 15, 4, 100.0, 20.0);
        assert!(do_partial_melt(&mut sd, 14, 15));
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!((u.mass.get(15) - 95.0).abs() < 1e-4, "固体应扣 5kg，got {}", u.mass.get(15));
        assert!((u.temperature.get(14) - 775.85).abs() < 1e-2, "气体格应降温到 775.85");
        let events = unsafe { &*sd.sim_events.ptr };
        assert_eq!(events.spawn_liquid_info.len(), 1, "应生成 1 条液滴");
        let drop = events.spawn_liquid_info.get(0);
        assert_eq!(drop.element_idx, 5);
        assert!((drop.mass - 5.0).abs() < 1e-4);
        assert!((drop.temperature - 503.0).abs() < 1e-4, "液滴温度应为固体熔点+3");
    }

    #[test]
    fn partial_melt_precondition_gates() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_partial_heat_table();
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements[4].high_temp_transition_idx = 5;
        }
        let mut sd = make_sim();
        // 邻格非固体 → false
        fill(&mut sd, 14, 3, 100.0, 800.0);
        fill(&mut sd, 15, 3, 100.0, 20.0); // 蒸汽
        assert!(!do_partial_melt(&mut sd, 14, 15));
        // 固体质量 <= 5 → false
        fill(&mut sd, 15, 4, 5.0, 20.0);
        assert!(!do_partial_melt(&mut sd, 14, 15));
        // 气体格不够热（cell_temp <= 熔点+3）→ false
        fill(&mut sd, 15, 4, 100.0, 20.0);
        fill(&mut sd, 14, 3, 100.0, 300.0);
        assert!(!do_partial_melt(&mut sd, 14, 15));
    }
```

注：`partial_melt_spawns_droplet_and_deducts_solid` 中 heat=(503−20)×1×5=2415；
f17=100×1=100；(800−(0+6))×100=79400 ≥ 2415 ✓；f17=(100×800−2415)/100=775.85。

### 步骤 3：运行测试验证失败

运行：`cargo test -p simdll_rs c2_physics::temperature::tests::partial_melt -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：FAIL，编译错误 "cannot find function `do_partial_melt`"。

### 步骤 4：编写最少实现代码

在 `temperature.rs` 新增：

```rust
/// DoPartialMelt — 部分熔化（原版 04_temperature.c L2471-2662）。
/// 极热气体格把相邻固体 5kg 熔为液滴（SpawnFallingLiquid，温度=固体熔点+3）；
/// 液滴失败则气体格就地转为熔融液体；固体格扣质量/病菌，扣尽则 Evaporate。
pub(crate) fn do_partial_melt(sim: &mut SimData, cell: usize, neighbor: usize) -> bool {
    let width = sim.width as usize;
    let total = width * sim.height as usize;
    if cell >= total || neighbor >= total {
        return false;
    }
    if sim.updated_cells.ptr.is_null() || sim.sim_events.ptr.is_null() {
        return false;
    }
    let updated = unsafe { &*sim.updated_cells.ptr };
    let cell_elem = updated.element_idx.get(cell);
    let neigh_elem = updated.element_idx.get(neighbor);
    let (Some(cell_el), Some(neigh_el)) = (
        elements_table::get_element_by_idx(cell_elem),
        elements_table::get_element_by_idx(neigh_elem),
    ) else {
        return false;
    };
    // 原版 L2495-2499：cell state&3 <= 1（真空/气体），neighbor state&3 == 3（固体）
    if (cell_el.state & 3) > 1 || (neigh_el.state & 3) != 3 {
        return false;
    }
    // 原版 L2500-2502：neighbor 质量 > 5.0
    let mass_neigh = updated.mass.get(neighbor);
    if mass_neigh <= 5.0 {
        return false;
    }
    // 原版 L2503-2506：neighbor props & 0x48 == 0
    if updated.properties.get(neighbor) & 0x48 != 0 {
        return false;
    }
    let cell_temp = updated.temperature.get(cell);
    let neigh_temp = updated.temperature.get(neighbor);
    let f19 = neigh_el.high_temp + 3.0;
    // 原版 L2510-2514：cell_temp > neigh_high+3；neigh_temp < neigh_high−3
    if cell_temp <= f19 {
        return false;
    }
    if neigh_el.high_temp - 3.0 <= neigh_temp {
        return false;
    }
    // 原版 L2516-2519：neighbor 高温转换目标（Element offset 6）有效
    let target = neigh_el.high_temp_transition_idx;
    if target == 0xFFFF {
        return false;
    }
    let Some(target_el) = elements_table::get_element_by_idx(target) else {
        return false;
    };
    // 原版 L2527-2529：目标必须为液体
    if (target_el.state & 3) != 2 {
        return false;
    }
    // 原版 L2521：heat = (f19 − neigh_temp) × 目标TC × 5.0
    let heat = (f19 - neigh_temp) * target_el.thermal_conductivity * 5.0;
    // 原版 L2523-2525：f17 = mass[cell] × cell_TC；(cell_temp − (cell_low+6)) × f17 < heat → false
    let cell_tc = cell_el.thermal_conductivity;
    let mut f17 = updated.mass.get(cell) * cell_tc;
    if (cell_temp - (cell_el.low_temp + 6.0)) * f17 < heat {
        return false;
    }
    // 原版 L2531：气体格新温度
    f17 = (f17 * cell_temp - heat) / f17;
    let disease_neigh = updated.disease_count.get(neighbor);
    let disease_idx_neigh = updated.disease_idx.get(neighbor);
    // 原版 L2533：i14 = (int)((5.0 / mass[neighbor]) × disease[neighbor])
    let i14 = ((5.0 / mass_neigh) * disease_neigh as f32) as i32;

    if crate::c2_physics::liquid_flow::spawn_falling_liquid(
        sim,
        cell,
        target,
        5.0,
        f19,
        disease_idx_neigh,
        i14,
    ) {
        // 原版 L2535-2545：成功 → 气体格降温、固体格扣 5kg/病菌
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.temperature.set(cell, f17);
        updated.mass.set(neighbor, mass_neigh - 5.0);
        let remain = disease_neigh - i14;
        if remain < 1 {
            crate::c2_physics::liquid_flow::clear_disease(updated, neighbor);
        } else {
            updated.disease_count.set(neighbor, remain);
        }
        if updated.mass.get(neighbor) <= 1.1754944e-38 {
            crate::c2_physics::liquid_flow::do_evaporate(sim, neighbor);
        }
        return true;
    }
    // 原版 L2546-2562：液滴失败 → DisplaceGas(cell) 后就地转化 + 固体扣减
    if !crate::c2_physics::liquid_flow::displace_gas(sim, cell, cell_elem) {
        return false;
    }
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.element_idx.set(cell, target);
        updated.mass.set(cell, 5.0);
        updated.temperature.set(cell, f19);
        crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
    }
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.mass.set(neighbor, mass_neigh - 5.0);
        let remain = disease_neigh - i14;
        if remain < 1 {
            crate::c2_physics::liquid_flow::clear_disease(updated, neighbor);
        } else {
            updated.disease_count.set(neighbor, remain);
        }
        if updated.mass.get(neighbor) <= 1.1754944e-38 {
            crate::c2_physics::liquid_flow::do_evaporate(sim, neighbor);
        }
    }
    true
}
```

### 步骤 5：接线气体分支门控

`liquid_flow.rs` 气体分支（L2158-2161），把：
```rust
            let melt_blocked = do_partial_melt(sim, cell, cell.wrapping_sub(width))
                || do_partial_melt(sim, cell, cell.wrapping_sub(1))
                || do_partial_melt(sim, cell, cell + 1)
                || do_partial_melt(sim, cell, cell + width);
```
改为：
```rust
            let melt_blocked = crate::c2_physics::temperature::do_partial_melt(sim, cell, cell.wrapping_sub(width))
                || crate::c2_physics::temperature::do_partial_melt(sim, cell, cell.wrapping_sub(1))
                || crate::c2_physics::temperature::do_partial_melt(sim, cell, cell + 1)
                || crate::c2_physics::temperature::do_partial_melt(sim, cell, cell + width);
```

### 步骤 6：运行测试验证通过

运行：`cargo test -p simdll_rs c2_physics::temperature::tests::partial_melt -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：2 条 partial_melt 测试全部 PASS。

### 步骤 7：全量回归 + 提交

运行：`cargo test -p simdll_rs -- --test-threads=1 2>$null; $LASTEXITCODE` → 全部 PASS

提交：
```bash
git add simDLL_RS/src/c2_physics/temperature.rs simDLL_RS/src/c2_physics/liquid_flow.rs
git commit -m "feat(temperature): implement DoPartialMelt full side effects (L2471-2662)"
```

## 任务 4：透气砖/网格砖验证收尾 + 记录

**文件：**
- 修改：`代办.txt`

### 步骤 1：追加代办记录

在 `代办.txt` 追加温度补全小节（含机制归属结论 + 用户实测清单）：

```markdown
## [待实测] 温度模块补全（2026-08-03）

- 规格：docs/superpowers/specs/2026-08-03-temperature-completion-design.md（cb58b4f）
- 计划：docs/superpowers/plans/2026-08-03-temperature-completion.md
- ① DoLoadTimeStateTransition（L7-73）：加载温度越界 ±1.5 换元素，save_load 已接线。
- ② DoPartialHeatTransition（L2105-2471）：大格≥5kg 分批沸腾 5kg（气体排下方/液体液滴），失败恢复。
- ③ DoPartialMelt（L2471-2662）：极热气体格熔化相邻固体 5kg（液滴/就地转化 + 双方温度质量病菌变化）。
- ④ 透气砖/网格砖换热：simDLL 通用原理（SimTemperatureTransfer→AddElementChunk→
  ElementChunk::Update 本格1.0 + 下方固体 groundTransferScale 0.0625），C# 仅注册不特判；
  本项目 ElementChunk 已实现 → 无 sim 改动，纯验证。
- 待实测：① 加载越界存档格修正；② 大质量水/污染水被火山气体加热分批沸腾；
  ③ 热气体熔化固体；④ 真空透气砖/网格砖仅与下方密实方块换热、变温板/导热板不生效。
```

### 步骤 2：提交

```bash
git add 代办.txt
git commit -m "docs: record temperature completion batch (load transition + partial heat/melt + tile verification)"
```

## 自检

**规格覆盖度**：①→任务 1；②→任务 2；③→任务 3；④→任务 4。全部覆盖。

**占位符扫描**：无 TODO/待定；每步含完整代码或精确 diff；测试代码完整。

**类型一致性**：`do_load_time_state_transition(sim, cell)`、`do_partial_heat_transition(sim, cell, neighbor)`、
`do_partial_melt(sim, cell, neighbor)` 签名在任务间一致；`spawn_falling_liquid(sim, cell, elem, mass, temp, disease_idx, disease_count)`
与现有实现签名一致（无 is_debug 参数，内部读 debug_properties）；`clear_disease(cells, cell)` /
`do_evaporate(sim, cell)` / `displace_gas(sim, cell, elem)` / `displace_liquid(sim, cell, elem)` /
`push_substance_change(sim, sim_cell)` 均与现有签名一致。
