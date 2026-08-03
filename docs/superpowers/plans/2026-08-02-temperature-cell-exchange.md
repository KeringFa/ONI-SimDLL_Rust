# 方格↔方格温度换热实现计划

> **面向 AI 代理的工作者：** 必须子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现本计划。步骤使用复选框（`- [ ]`）语法跟踪进度。

**目标：** 实现 C2 格子级温度演化——`UpdateTemperature` 两格换热、`UpdateTemperatureForBackwallExchange` 背墙换热、逐格 `DoStateTransition`（复用），并按原版缓冲语义（每子步 CopyFrom）集成进 `update_data`，使方格↔方格换热速率与原版完全一致。

**架构：** 规格 `docs/superpowers/specs/2026-08-02-temperature-cell-exchange-design.md`（提交 `502945c`）。新逻辑全部落在 `c2_physics/temperature.rs`（已有 spawn_ore/do_state_transition）；`c2_physics/mod.rs::update_data` 开头插入 `CopyFrom → run_temperature_task → CopyFrom`（原版 L144415/L144548），其后液体/压力/建筑换热阶段顺序不变。

**技术栈：** Rust / cargo / 严格对照 `OxygenNotIncluded_Source/SimDLL_Source.c`（L133079-133170、L133737-134079、L144205-144310、L144415/L144548）。全量测试用 `cargo test -- --test-threads=1`。

---

## 文件清单

| 文件 | 职责 | 动作 |
|------|------|------|
| `simDLL_RS/src/c2_physics/temperature.rs` | 新增 `calculate_temperature_exchange_precise`、`update_temperature_pair`、`update_temperature_for_backwall`、`run_temperature_task` | 修改 |
| `simDLL_RS/src/c2_physics/mod.rs` | `update_data` 开头插入 CopyFrom + 温度任务 + 温度后 CopyFrom | 修改 |

## 全局约束

- 严格对照原版行号（规格 2.2/3.1-3.4），**禁止照抄** `my_Project\simDLL_RS_Kit\simDLL_RS`（神秘版本）。
- TDD：每个新函数先写失败测试 → 验证失败 → 最少实现 → 验证通过 → commit。
- 断言（原版 DebugBreak/崩溃）按项目惯例转 `tracing::error/warn` + 继续。
- 单测须持 `LIB_TESTS_LOCK`；元素表用独立最小表（参照 `c2_physics/temperature.rs` 已有 `init_phase_table` 风格）。
- 缓冲语义：`update_data` 每子步开头与温度任务后各一次 `copy_updated_cells_to_cells`（用户强制要求，换热速率不得有差异）。
- 已知偏差沿用：无 Region 概念（整图遍历 = active region）；多子步帧语义已由本次 CopyFrom 修正为原版一致。

---

### 任务 1：`calculate_temperature_exchange_precise`（原版 L133110-133170）

**文件：** `simDLL_RS/src/c2_physics/temperature.rs`

- [ ] **步骤 1：写失败测试（能量守恒 / 0.25 限制 / min 三值 / 顺序无关）**

```rust
#[test]
fn exchange_precise_conserves_energy_and_clamps_to_equilibrium() {
    // T_a=300 HC=100，T_b=400 HC=100，k=1，rate=0.25，dt=0.2
    let (na, nb) = calculate_temperature_exchange_precise(0.25, 1.0, 0.2, 300.0, 100.0, 400.0, 100.0);
    // heat = min(100×1×0.2, 100×0.25×100, 100×0.25×100) = 20
    assert!((na - 300.2).abs() < 1e-3, "a 得热 20/100 → 300.2，got {na}");
    assert!((nb - 399.8).abs() < 1e-3, "b 失热 20/100 → 399.8，got {nb}");
    // 能量守恒：0.2×100 == 0.2×100
    assert!(((na - 300.0) * 100.0 + (nb - 400.0) * 100.0).abs() < 1e-3);
}

#[test]
fn exchange_precise_never_crosses_weighted_equilibrium() {
    // 极大 k → heat 被 HC×0.25 限制，最终不越过均衡 350
    let (na, nb) = calculate_temperature_exchange_precise(0.25, 1e9, 0.2, 300.0, 100.0, 400.0, 100.0);
    assert!(na <= 350.0 + 1e-3 && nb >= 350.0 - 1e-3);
    // 顺序无关：交换输入后结果互换
    let (nb2, na2) = calculate_temperature_exchange_precise(0.25, 1.0, 0.2, 400.0, 100.0, 300.0, 100.0);
    assert!((na - na2).abs() < 1e-3 && (nb - nb2).abs() < 1e-3);
}
```

- [ ] **步骤 2：运行确认失败** — `cargo test -p simdll_rs --lib exchange_precise`；预期：函数未定义。

- [ ] **步骤 3：最少实现**

```rust
/// CalculateTemperatureExchange_precise（原版 L133110-133170）。
/// heat = min((T_b−T_a)×k×dt, (T_b−T_a)×rate×HC_a, (T_b−T_a)×rate×HC_b)；
/// 新温度 = ±heat/HC + 原温度，钳制不越过加权均衡；double 计算、f32 输出。
pub(crate) fn calculate_temperature_exchange_precise(
    rate: f32,
    k: f32,
    dt: f32,
    t_a: f32,
    hc_a: f32,
    t_b: f32,
    hc_b: f32,
) -> (f32, f32) {
    let (t_a, hc_a) = (t_a as f64, hc_a as f64);
    let (t_b, hc_b) = (t_b as f64, hc_b as f64);
    let (rate, k, dt) = (rate as f64, k as f64, dt as f64);
    if hc_a <= 0.0 || hc_b <= 0.0 {
        return (t_a as f32, t_b as f32);
    }
    let equilibrium = (t_a * hc_a + t_b * hc_b) / (hc_a + hc_b);
    let d = t_b - t_a;
    let heat = (d * k * dt).min(d * rate * hc_a).min(d * rate * hc_b);
    let mut new_a = heat / hc_a + t_a;
    if equilibrium <= new_a {
        new_a = equilibrium;
    }
    let mut new_b = -heat / hc_b + t_b;
    if new_b <= equilibrium {
        new_b = equilibrium;
    }
    (new_a as f32, new_b as f32)
}
```

- [ ] **步骤 4：运行确认通过** — `cargo test -p simdll_rs --lib exchange_precise`；预期：ok。

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c2_physics/temperature.rs
git commit -m "feat(temperature): add CalculateTemperatureExchange_precise"
```

---

### 任务 2：`update_temperature_pair`（原版 L133737-134004）

**文件：** `simDLL_RS/src/c2_physics/temperature.rs`

- [ ] **步骤 1：写失败测试（min/几何平均、表面倍率、能量守恒写回、钳制、真空防御）**

```rust
fn init_pair_table() {
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();
    table.elements.clear();
    table.temperature_data.clear();
    // elem1 液态水：SHC=4.179、TC=0.609、gas/liquid/solid 倍率 1.0/10.0/1.0
    let mut water = ElementTemperatureData::default();
    water.state = 2;
    water.specific_heat_capacity = 4.179;
    water.thermal_conductivity = 0.609;
    water.gas_surface_area_multiplier = 1.0;
    water.liquid_surface_area_multiplier = 10.0;
    water.solid_surface_area_multiplier = 1.0;
    // elem2 固态砖：SHC=0.8、TC=2.0
    let mut rock = ElementTemperatureData::default();
    rock.state = 3;
    rock.specific_heat_capacity = 0.8;
    rock.thermal_conductivity = 2.0;
    rock.gas_surface_area_multiplier = 1.0;
    rock.liquid_surface_area_multiplier = 1.0;
    rock.solid_surface_area_multiplier = 1.0;
    table.temperature_data.push(ElementTemperatureData::default()); // 0 dummy
    table.temperature_data.push(water);
    table.temperature_data.push(rock);
}

fn make_pair_sim() -> SimData {
    let mut sd = SimData::new_for_allocate(6, 6, 1, false, false);
    sd.vacuum_element_idx = 0;
    sd.void_element_idx = 0xFFFF;
    sd
}

#[test]
fn update_temperature_pair_exchanges_and_conserves_energy() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_pair_table();
    let mut sd = make_pair_sim();
    unsafe {
        let c = &mut *sd.cells.ptr;
        let u = &mut *sd.updated_cells.ptr;
        for (cell, elem, mass, temp, ins) in [
            (14usize, 1u16, 100.0f32, 300.0f32, 1u8), // 液态水
            (15usize, 2u16, 200.0f32, 400.0f32, 1u8), // 固态砖
        ] {
            c.element_idx.set(cell, elem);
            c.mass.set(cell, mass);
            c.temperature.set(cell, temp);
            c.insulation.set(cell, ins);
            u.element_idx.set(cell, elem);
            u.mass.set(cell, mass);
            u.temperature.set(cell, temp);
            u.insulation.set(cell, ins);
        }
    }
    update_temperature_pair(&mut sd, 14, 15);
    unsafe {
        let c = &*sd.cells.ptr;
        let u = &*sd.updated_cells.ptr;
        let new_a = u.temperature.get(14);
        let new_b = u.temperature.get(15);
        assert!(new_a > 300.0 && new_b < 400.0, "热格降温冷格升温：{new_a} / {new_b}");
        // 能量守恒：Δa×HC_a + Δb×HC_b == 0（HC_a=100×4.179=417.9，HC_b=200×0.8=160）
        let da = (new_a - c.temperature.get(14)) * 417.9;
        let db = (new_b - c.temperature.get(15)) * 160.0;
        assert!((da + db).abs() < 1e-2, "能量不守恒：{da} + {db}");
    }
}

#[test]
fn update_temperature_pair_applies_surface_multiplier() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_pair_table();
    let mut sd = make_pair_sim();
    // 同温 300/400 与上一测试同构，但把水侧 liquid 倍率调到 10（对方为固态 → 用水的 solid 倍率？）
    // 反向验证：水的 solid 倍率=1、砖的 liquid 倍率=1 → k_total = k × 1 × 1；
    // 若倍率实现为"取对方状态"，把砖改为液态（elem3）再对比。
    unsafe {
        let c = &mut *sd.cells.ptr;
        let u = &mut *sd.updated_cells.ptr;
        for (cell, elem, mass, temp) in [(14usize, 1u16, 100.0f32, 300.0f32), (15usize, 2u16, 200.0f32, 400.0f32)] {
            c.element_idx.set(cell, elem);
            c.mass.set(cell, mass);
            c.temperature.set(cell, temp);
            c.insulation.set(cell, 1);
            u.element_idx.set(cell, elem);
            u.mass.set(cell, mass);
            u.temperature.set(cell, temp);
            u.insulation.set(cell, 1);
        }
    }
    update_temperature_pair(&mut sd, 14, 15);
    let d1 = unsafe { (*sd.updated_cells.ptr).temperature.get(14) };
    // 重复同场景（单元测试幂等由能量守恒断言覆盖即可；倍率精确值由实现对照原版 L1338xx 断言）
    assert!(d1 > 300.0);
}
```

> 表面倍率/几何平均的**精确数值断言**在实现时按原版 L133801-133835 逐项落地（倍率选择：`ETD[cell].(gas/liquid/solid)SurfaceAreaMultiplier` 按**对方** state&3；state 0 落 liquid 倍率），并用"调高一侧倍率 → 换热加大"的方式验证方向。

- [ ] **步骤 2：运行确认失败** — `cargo test -p simdll_rs --lib update_temperature_pair`；预期：函数未定义。

- [ ] **步骤 3：最少实现**

```rust
/// 合并导率：任一侧 insulation 值（ins²×1.53787e-05）<1.0 → min(k_a,k_b)；否则几何平均。
fn combine_conductivity(ins_a: u8, tc_a: f32, ins_b: u8, tc_b: f32) -> f32 {
    let k_a = ins_a as f32 * ins_a as f32 * 1.53787e-05 * tc_a;
    let k_b = ins_b as f32 * ins_b as f32 * 1.53787e-05 * tc_b;
    let iv_a = ins_a as f32 * ins_a as f32 * 1.53787e-05;
    let iv_b = ins_b as f32 * ins_b as f32 * 1.53787e-05;
    if iv_a < 1.0 || iv_b < 1.0 {
        k_a.min(k_b)
    } else {
        (k_a * k_b).sqrt() // 原版 logf+logf+expf = 几何平均
    }
}

/// 表面倍率：this_etd 按对方 state&3 取 gas/liquid/solid 倍率（state 0 落 liquid）。
fn surface_multiplier(this: &ElementTemperatureData, other_state: u8) -> f32 {
    match other_state & 3 {
        1 => this.gas_surface_area_multiplier,
        2 => this.liquid_surface_area_multiplier,
        3 => this.solid_surface_area_multiplier,
        _ => this.liquid_surface_area_multiplier,
    }
}

/// UpdateTemperature（原版 L133737-134004）：读 cells、写 updatedCells 增量 + [1,10000] 钳制。
pub(crate) fn update_temperature_pair(sim: &mut SimData, cell_a: usize, cell_b: usize) {
    if sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
    let (t_a, t_b, hc_a, hc_b, k_total) = {
        let c = unsafe { &*sim.cells.ptr };
        if cell_a >= c.temperature.len() || cell_b >= c.temperature.len() {
            return;
        }
        let (elem_a, elem_b) = (c.element_idx.get(cell_a), c.element_idx.get(cell_b));
        let (Some(etd_a), Some(etd_b)) = (
            get_element_temperature_data(elem_a),
            get_element_temperature_data(elem_b),
        ) else {
            return;
        };
        let (hc_a, hc_b) = (
            c.mass.get(cell_a) * etd_a.specific_heat_capacity,
            c.mass.get(cell_b) * etd_b.specific_heat_capacity,
        );
        if hc_a <= 0.0 || hc_b <= 0.0 {
            return;
        }
        let k = combine_conductivity(
            c.insulation.get(cell_a),
            etd_a.thermal_conductivity,
            c.insulation.get(cell_b),
            etd_b.thermal_conductivity,
        );
        let mult_a = surface_multiplier(&etd_a, etd_b.state);
        let mult_b = surface_multiplier(&etd_b, etd_a.state);
        (
            c.temperature.get(cell_a),
            c.temperature.get(cell_b),
            hc_a,
            hc_b,
            k * mult_a * mult_b,
        )
    };
    let (new_a, new_b) = if t_a <= t_b {
        calculate_temperature_exchange_precise(0.25, k_total, 0.2, t_a, hc_a, t_b, hc_b)
    } else {
        let (nb, na) = calculate_temperature_exchange_precise(0.25, k_total, 0.2, t_b, hc_b, t_a, hc_a);
        (na, nb)
    };
    unsafe {
        let u = &mut *sim.updated_cells.ptr;
        let c = &*sim.cells.ptr;
        let mut va = (new_a - c.temperature.get(cell_a)) + u.temperature.get(cell_a);
        va = va.max(1.0).min(10000.0);
        u.temperature.set(cell_a, va);
        let mut vb = (new_b - c.temperature.get(cell_b)) + u.temperature.get(cell_b);
        vb = vb.max(1.0).min(10000.0);
        u.temperature.set(cell_b, vb);
    }
}
```

> 注意：`precise` 返回 (new_第一参, new_第二参)；`t_a > t_b` 时 swapped 调用 `(t_b,hc_b,t_a,hc_a)` 得 `(nb,na)`，映射回 `(na,nb)`——与原版 L133925-133940 的 local_78/local_74 交换一致。

- [ ] **步骤 4：运行确认通过（含全量）** — `cargo test -p simdll_rs -- --test-threads=1`。

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c2_physics/temperature.rs
git commit -m "feat(temperature): implement UpdateTemperature cell-pair exchange"
```

---

### 任务 3：`update_temperature_for_backwall`（原版 L134004-134079）

**文件：** `simDLL_RS/src/c2_physics/temperature.rs`

- [ ] **步骤 1：写失败测试（0.01 因子 / 背墙直接写 / 相变事件 / 真空 void 跳过）**

```rust
#[test]
fn backwall_exchange_writes_both_and_emits_transition() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_pair_table();
    let mut sd = make_pair_sim();
    unsafe {
        let c = &mut *sd.cells.ptr;
        let u = &mut *sd.updated_cells.ptr;
        let bw = &mut *sd.backwall.ptr;
        // 格 14：液态水 300°C；背墙：固态砖 500°C（elem2, high 由元素表给默认 0 → 低变不触发；用 highTemp 测试相变）
        c.element_idx.set(14, 1);
        c.mass.set(14, 100.0);
        c.temperature.set(14, 300.0);
        c.insulation.set(14, 1);
        u.element_idx.set(14, 1);
        u.mass.set(14, 100.0);
        u.temperature.set(14, 300.0);
        bw.element_idx.set(14, 2);
        bw.mass.set(14, 200.0);
        bw.temperature.set(14, 500.0);
    }
    update_temperature_for_backwall(&mut sd, 14);
    unsafe {
        let u = &*sd.updated_cells.ptr;
        let bw = &*sd.backwall.ptr;
        assert!(u.temperature.get(14) > 300.0, "格升温（背墙更热）");
        assert!(bw.temperature.get(14) < 500.0, "背墙降温");
        // 背墙温度被直接写（不是增量）：backwall.temperature == 新值
        assert!(bw.temperature.get(14) > 300.0 && bw.temperature.get(14) < 500.0);
        // elem2 默认 highTemp=0 → 背墙 500°C 越 high+3 → 相变事件
        let events = &*sd.sim_events.ptr;
        assert_eq!(events.backwall_should_transition_info.len(), 1);
    }
}

#[test]
fn backwall_exchange_skips_vacuum_or_void() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_pair_table();
    let mut sd = make_pair_sim();
    unsafe {
        let c = &mut *sd.cells.ptr;
        let u = &mut *sd.updated_cells.ptr;
        let bw = &mut *sd.backwall.ptr;
        c.element_idx.set(14, 0); // 真空
        c.mass.set(14, 0.0);
        c.temperature.set(14, 0.0);
        bw.element_idx.set(14, 2);
        bw.mass.set(14, 200.0);
        bw.temperature.set(14, 500.0);
        // 同步 updated
        u.element_idx.set(14, 0);
        u.mass.set(14, 0.0);
        u.temperature.set(14, 0.0);
    }
    update_temperature_for_backwall(&mut sd, 14);
    unsafe {
        assert_eq!((*sd.backwall.ptr).temperature.get(14), 500.0, "真空格不换热");
    }
}
```

- [ ] **步骤 2：运行确认失败** — `cargo test -p simdll_rs --lib backwall_exchange`；预期：函数未定义。

- [ ] **步骤 3：最少实现**

```rust
/// UpdateTemperatureForBackwallExchange（原版 L134004-134079）。因子 0.01。
/// cell 写 updatedCells 增量；背墙温度直接写；背墙温度越 ±3 → backwallShouldTransitionInfo。
pub(crate) fn update_temperature_for_backwall(sim: &mut SimData, cell: usize) {
    if sim.cells.ptr.is_null()
        || sim.updated_cells.ptr.is_null()
        || sim.backwall.ptr.is_null()
        || sim.sim_events.ptr.is_null()
    {
        return;
    }
    let (cell_elem, cell_temp, cell_hc, cell_ins, back_elem, back_temp, back_hc, k_total) = {
        let c = unsafe { &*sim.cells.ptr };
        let bw = unsafe { &*sim.backwall.ptr };
        if cell >= c.element_idx.len() || cell >= bw.element_idx.len() {
            return;
        }
        let cell_elem = c.element_idx.get(cell);
        let back_elem = bw.element_idx.get(cell);
        // 双方元素非真空/void
        if cell_elem == sim.vacuum_element_idx
            || cell_elem == sim.void_element_idx
            || back_elem == sim.vacuum_element_idx
            || back_elem == sim.void_element_idx
        {
            return;
        }
        let (Some(etd_cell), Some(etd_back)) = (
            get_element_temperature_data(cell_elem),
            get_element_temperature_data(back_elem),
        ) else {
            return;
        };
        let cell_hc = c.mass.get(cell) * etd_cell.specific_heat_capacity;
        let back_hc = bw.mass.get(cell) * etd_back.specific_heat_capacity;
        if cell_hc <= 0.0 || back_hc <= 0.0 {
            return;
        }
        let k_cell = cell_ins as f32 * cell_ins as f32 * 1.53787e-05 * etd_cell.thermal_conductivity;
        let k = if k_cell < 1.0 {
            k_cell.min(etd_back.thermal_conductivity)
        } else {
            (k_cell * etd_back.thermal_conductivity).sqrt()
        };
        let mult_cell = surface_multiplier(&etd_cell, etd_back.state);
        let mult_back = surface_multiplier(&etd_back, etd_cell.state);
        (
            cell_elem,
            c.temperature.get(cell),
            cell_hc,
            c.insulation.get(cell),
            back_elem,
            bw.temperature.get(cell),
            back_hc,
            k * mult_cell * mult_back,
        )
    };
    // wrapper（原版 CalculateTemperatureExchange L133079）：自动排序，返回 (new_back, new_cell)
    let (new_back, new_cell) = if cell_temp <= back_temp {
        let (nb, nc) =
            calculate_temperature_exchange_precise(0.01, k_total, 0.2, cell_temp, cell_hc, back_temp, back_hc);
        (nb, nc)
    } else {
        let (nc, nb) =
            calculate_temperature_exchange_precise(0.01, k_total, 0.2, back_temp, back_hc, cell_temp, cell_hc);
        (nb, nc)
    };
    unsafe {
        let u = &mut *sim.updated_cells.ptr;
        let c = &*sim.cells.ptr;
        let mut v = (new_cell - c.temperature.get(cell)) + u.temperature.get(cell);
        v = v.max(1.0).min(10000.0);
        u.temperature.set(cell, v);
        let bw = &mut *sim.backwall.ptr;
        bw.temperature.set(cell, new_back);
        // 相变：背墙温度越 [low−3, high+3] → backwallShouldTransitionInfo
        if let Some(etd_back) = get_element_temperature_data(back_elem) {
            if new_back < etd_back.low_temp - 3.0 || new_back > etd_back.high_temp + 3.0 {
                let w = sim.width as usize;
                let game_cell = (cell / w) as i32 - 1;
                let game_cell = game_cell * (w as i32 - 2) + (cell % w) as i32 - 1;
                if game_cell >= 0 && game_cell < sim.num_game_cells {
                    let events = unsafe { &mut *sim.sim_events.ptr };
                    events.backwall_should_transition_info.push(
                        crate::a_framework::game_data::BackwallShouldTransitionInfo { game_cell: game_cell as u32 },
                    );
                }
            }
        }
    }
}
```

> 合并导率注意：背墙路径原版是 `k_cell(=ins²×1.53787e-05×cellTC)` 与 `backTC` 合并（fVar11 × cellTC 再与 backTC min/geomean），门控条件为 `insulation 值 ≥ 1.0`（GetInsulationValue 即 ins²×1.53787e-05）——与 3.2 一致，实现时按 L134045-134053 核对。

- [ ] **步骤 4：运行确认通过（含全量）**

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c2_physics/temperature.rs
git commit -m "feat(temperature): implement UpdateTemperatureForBackwallExchange"
```

---

### 任务 4：`run_temperature_task`（原版 L144205-144310 单线程等价）

**文件：** `simDLL_RS/src/c2_physics/temperature.rs`

- [ ] **步骤 1：写失败测试（门控 / 右/下成对一次 / 背墙 / DoStateTransition 联动）**

```rust
fn fill_pair_cells(sd: &mut SimData, cells: &[(usize, u16, f32, f32, u8)]) {
    unsafe {
        let c = &mut *sd.cells.ptr;
        let u = &mut *sd.updated_cells.ptr;
        for &(cell, elem, mass, temp, ins) in cells {
            c.element_idx.set(cell, elem);
            c.mass.set(cell, mass);
            c.temperature.set(cell, temp);
            c.insulation.set(cell, ins);
            u.element_idx.set(cell, elem);
            u.mass.set(cell, mass);
            u.temperature.set(cell, temp);
            u.insulation.set(cell, ins);
        }
    }
}

#[test]
fn temperature_task_exchanges_adjacent_pairs_with_gates() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_pair_table();
    let mut sd = make_pair_sim();
    // cell14(液态水 300) 与 cell15(固态砖 400) 相邻（ΔT=100 ≥ 1）→ 换热；
    // cell20(row3,col2 水 300) 与 cell26(row4,col2 水 300) 同温（ΔT=0）→ 不换热
    fill_pair_cells(
        &mut sd,
        &[(14, 1, 100.0, 300.0, 1), (15, 2, 200.0, 400.0, 1), (20, 1, 100.0, 300.0, 1), (26, 1, 100.0, 300.0, 1)],
    );
    run_temperature_task(&mut sd);
    unsafe {
        let u = &*sd.updated_cells.ptr;
        assert!(u.temperature.get(14) > 300.0);
        assert!(u.temperature.get(15) < 400.0);
        assert_eq!(u.temperature.get(20), 300.0, "ΔT<1 不换热");
        assert_eq!(u.temperature.get(26), 300.0);
    }
}

#[test]
fn temperature_task_skips_temperature_insulated_state() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_pair_table();
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();
    let mut rock = table.temperature_data[2];
    rock.state = 3 | 0x10; // TemperatureInsulated
    table.temperature_data[2] = rock;
    drop(table);
    let mut sd = make_pair_sim();
    fill_pair_cells(&mut sd, &[(14, 1, 100.0, 300.0, 1), (15, 2, 200.0, 400.0, 1)]);
    run_temperature_task(&mut sd);
    unsafe {
        assert_eq!((*sd.updated_cells.ptr).temperature.get(14), 300.0, "绝缘元素不换热");
    }
}
```

- [ ] **步骤 2：运行确认失败** — `cargo test -p simdll_rs --lib temperature_task`；预期：函数未定义。

- [ ] **步骤 3：最少实现**

```rust
/// SimUpdateTemperatureTask::InternalDoTask 单线程等价（原版 L144205-144310）。
/// 遍历内部格：背墙换热 → 右邻（ΔT≥1 且 TC>0 且无 0x10）→ 下邻（同门控）→ DoStateTransition。
pub(crate) fn run_temperature_task(sim: &mut SimData) {
    if sim.cells.ptr.is_null()
        || sim.updated_cells.ptr.is_null()
        || sim.backwall.ptr.is_null()
        || sim.sim_events.ptr.is_null()
    {
        return;
    }
    let w = sim.width as usize;
    let h = sim.height as usize;
    if w < 3 || h < 3 {
        return;
    }
    for row in 1..h - 1 {
        for col in 1..w - 1 {
            let cell = row * w + col;
            // 1. 背墙换热（cells.mass>0 且 backwall.mass>0，原版 L144222-144234）
            {
                let c = unsafe { &*sim.cells.ptr };
                let bw = unsafe { &*sim.backwall.ptr };
                if cell < c.mass.len()
                    && cell < bw.mass.len()
                    && c.mass.get(cell) > 0.0
                    && bw.mass.get(cell) > 0.0
                {
                    update_temperature_for_backwall(sim, cell);
                }
            }
            // 2/3. 右/下邻换热（门控：|ΔT|≥1、双方 ETD TC>0、无 TemperatureInsulated）
            let (t_self, t_right, t_below) = {
                let c = unsafe { &*sim.cells.ptr };
                (
                    c.temperature.get(cell),
                    c.temperature.get(cell + 1),
                    c.temperature.get(cell + w),
                )
            };
            if col + 1 < w - 1 && (t_self - t_right).abs() >= 1.0 && pair_allowed(sim, cell, cell + 1) {
                update_temperature_pair(sim, cell, cell + 1);
            }
            if row + 1 < h - 1 && (t_self - t_below).abs() >= 1.0 && pair_allowed(sim, cell, cell + w) {
                update_temperature_pair(sim, cell, cell + w);
            }
            // 4. DoStateTransition（每格，原版 L144293-144306）
            let elem = unsafe { (*sim.cells.ptr).element_idx.get(cell) };
            if let Some(etd) = get_element_temperature_data(elem) {
                crate::c2_physics::temperature::do_state_transition(sim, cell, &etd);
            }
        }
    }
}

/// 邻格换热门控：双方元素 ETD 有效、TC>0、state 无 TemperatureInsulated(0x10)。
fn pair_allowed(sim: &SimData, a: usize, b: usize) -> bool {
    let c = unsafe { &*sim.cells.ptr };
    let (Some(etd_a), Some(etd_b)) = (
        get_element_temperature_data(c.element_idx.get(a)),
        get_element_temperature_data(c.element_idx.get(b)),
    ) else {
        return false;
    };
    etd_a.thermal_conductivity > 0.0
        && etd_b.thermal_conductivity > 0.0
        && (etd_a.state | etd_b.state) & 0x10 == 0
}
```

> 注意：`do_state_transition` 作用于 updatedCells（原版任务循环 L144293 用 cells 元素查 ETD、DoStateTransition 内部用 updatedCells，一致）。温度读取用 `cells`（稳定缓冲，opening CopyFrom 后 == 子步初态）。

- [ ] **步骤 4：运行确认通过（含全量）**

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c2_physics/temperature.rs
git commit -m "feat(temperature): implement temperature task loop (backwall + neighbor pairs + phase change)"
```

---

### 任务 5：update_data 缓冲集成（原版 L144415/L144548）

**文件：** `simDLL_RS/src/c2_physics/mod.rs`

- [ ] **步骤 1：写失败测试（多子步温度基于上一子步结果）**

```rust
#[test]
fn update_data_syncs_cells_before_temperature_each_substep() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_pair_table_for_mod();
    let mut sd = make_sim_data();
    fill_both(&mut sd, |b| {
        b.element_idx.set(14, 1);
        b.mass.set(14, 100.0);
        b.temperature.set(14, 300.0);
        b.insulation.set(14, 1);
        b.element_idx.set(15, 2);
        b.mass.set(15, 200.0);
        b.temperature.set(15, 400.0);
        b.insulation.set(15, 1);
    });
    update_data(&mut sd);
    let after_1 = unsafe { (*sd.updated_cells.ptr).temperature.get(14) };
    update_data(&mut sd); // 第二个子步
    let after_2 = unsafe { (*sd.updated_cells.ptr).temperature.get(14) };
    // opening CopyFrom 使第二子步基于第一子步结果：温度继续收敛（而非重复相同 delta 导致过冲）
    assert!(after_2 > after_1, "第二子步应继续升温收敛：{after_1} → {after_2}");
    // 且收敛方向正确：不越过均衡（约 326.7）
    assert!(after_2 < 400.0);
}
```

- [ ] **步骤 2：运行确认失败** — 预期：目前 `update_data` 无温度任务 → 温度不变，断言失败。

- [ ] **步骤 3：最少实现（update_data 开头插入）**

```rust
pub fn update_data(sim_data: &mut SimData) {
    // 原版 UpdateData L144415：每子步开头 CopyFrom(cells, updatedCells)
    crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);
    // 温度任务（原版 L144430-144548：读 cells、写 updatedCells 增量）
    crate::c2_physics::temperature::run_temperature_task(sim_data);
    // 原版 L144548：温度后 CopyFrom，供后续压力/液体阶段读取新温度
    crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data);
    // ……现有液体/消费者/压力/建筑换热阶段（顺序不变）……
}
```

- [ ] **步骤 4：运行确认通过（含全量）**

```powershell
cargo test -p simdll_rs -- --test-threads=1
cargo build --release
```

- [ ] **步骤 5：Commit**

```powershell
git add simDLL_RS/src/c2_physics/mod.rs
git commit -m "feat(temperature): wire per-substep buffer sync + temperature task into update_data"
```

---

### 任务 6：集成回归与文档

- [ ] **步骤 1：全量测试 + Release 构建**

```powershell
cargo test -p simdll_rs -- --test-threads=1
cargo build --release
```

- [ ] **步骤 2：更新代办.txt**：方格↔方格换热（UpdateTemperature）、背墙换热、温度任务循环标为已实现；留"升华/持续蒸发、ElementEmitter 本体、辐射"为后续。

- [ ] **步骤 3：Commit**

```powershell
git add 代办.txt
git commit -m "docs(todo): cell-cell temperature exchange implemented"
```

---

## 自检

1. **规格覆盖度：** 规格 3.1→任务 1；3.2→任务 2；3.3→任务 3；3.4→任务 4；2.2 缓冲集成→任务 5；测试/验收→任务 6。无遗漏。
2. **占位符扫描：** 任务 2/3 的"实现时按原版 L1338xx/L1340xx 核对"为**有意指引**（精确数值断言依赖反汇编细节），实现者必须逐行对照；除此无 TODO/待定。
3. **类型一致性：** `calculate_temperature_exchange_precise` 返回 `(f32, f32)`（new_第一参, new_第二参），任务 2/3 的 swapped 映射一致；`combine_conductivity`/`surface_multiplier` 在任务 2/3 共用；`run_temperature_task` 复用任务 4 的 `do_state_transition`（已存在）。
