# 升华/offgas（DoSublimation + 液体 offgas）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法跟踪进度。

**目标：** 实现自然固体格升华 `DoSublimation`（氧石/核废料/毒沙/毒泥 → 气体，含 1.8kg 上限/概率门/病菌转移/整格消耗/SpawnFX），并补全液体 offgas 产气（污染水 → 污染氧，含主路径与耗尽路径），严格对照原版。

**架构：** 规格 `docs/superpowers/specs/2026-08-03-sublimation-design.md`（提交 `cfd8c85`）。全部改动集中在 `c2_physics/liquid_flow.rs`：`do_sublimation` 替换 `do_sublimation_stub`（真空/气体分支调用，删除液体分支调用）；液体分支 offgas 块补产气；新增 `emit_sublimate_gas`（产气辅助）与 `push_spawn_fx`（SpawnFX 事件）。复用现有 `add_mass_and_update_temperature`/`displace_gas`/`push_substance_change`/`clear_cell`/`next_random`/`do_evaporate`。

**技术栈：** Rust / cargo。严格对照 `SimDLL_Source.c`（DoSublimation L148591-148880、液体 offgas L149677-149834、SpawnFX L149835+）。测试命令：`cargo test -p simdll_rs -- --test-threads=1`（工作目录 `simDLL_RS`），Release：`cargo build --release -p simdll_rs`。

---

## 文件清单

| 文件 | 职责 | 动作 |
|------|------|------|
| `simDLL_RS/src/c2_physics/liquid_flow.rs` | `push_spawn_fx`、`do_sublimation`、`emit_sublimate_gas`、offgas 块补全 + 全部测试 | 修改 |

## 全局约束

- 严格对照原版行号逐行镜像（含"怪癖"：同次调用前邻格成功后、后续邻格 DisplaceGas 失败 → 仍扣砖格质量/播 FX 但不产气）。
- **禁止参考** `my_Project\simDLL_RS_Kit\simDLL_RS`（神秘版本）与 `simDLL_RS.old`——当前项目进度领先。
- TDD：先写失败测试 → 运行确认失败 → 最少实现 → 运行确认通过 → commit。
- 单测持 `LIB_TESTS_LOCK`；元素表用新测试辅助 `create_sublimate_table`（见任务 2）。
- 事件断言用 `spawn_fx_info`/`substance_change_info` 向量；game cell 换算：`(row-1)*(width-2)+(col-1)`。
- 随机数断言：`next_random` 即 LCG `seed = seed*0x343fd+0x269ec3`，测试可独立计算期望值。

---

### 任务 1：`push_spawn_fx` 事件辅助

**文件：** `simDLL_RS/src/c2_physics/liquid_flow.rs:47`（import）、`liquid_flow.rs:996` 附近（函数）

- [ ] **步骤 1：写失败测试**

```rust
    /// SimEvents::SpawnFX（原版 L149835+）：sim→game 坐标转换 + 推 spawn_fx_info。
    /// sim14（row2,col2）→ game 5；字段 = {cell_idx, fx_id, rotation}（12B，L7251-7253）。
    #[test]
    fn push_spawn_fx_converts_cell_and_fields() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        DestroyElementsTable();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let events = &mut *sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 0);
        }
        push_spawn_fx(&mut sd, 14, 101, 180.0);
        push_spawn_fx(&mut sd, 20, 102, 0.0); // sim20（row3,col2）→ game 9
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 2);
            let fx0 = events.spawn_fx_info.get(0);
            assert_eq!(fx0.cell_idx, 5);
            assert_eq!(fx0.fx_id, 101);
            assert_eq!(fx0.rotation, 180.0);
            let fx1 = events.spawn_fx_info.get(1);
            assert_eq!(fx1.cell_idx, 9);
            assert_eq!(fx1.fx_id, 102);
            assert_eq!(fx1.rotation, 0.0);
        }
    }
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test -p simdll_rs push_spawn_fx -- --test-threads=1`
预期：FAIL，报错 "cannot find function `push_spawn_fx` in this scope"

- [ ] **步骤 3：实现**

在 `use crate::a_framework::game_data::SubstanceChangeInfo;`（L47）同一行追加：

```rust
use crate::a_framework::game_data::{SpawnFXInfo, SubstanceChangeInfo};
```

在 `do_evaporate`（L996）之后、`do_sublimation_stub`（L1010）之前新增：

```rust
/// SimEvents::SpawnFX（原版 L149835+）：sim→game 坐标转换 + 推 spawn_fx_info。
/// SpawnFXInfo 12B = {cell_idx: i32, fx_id: i32, rotation: f32}（L7251-7253）。
/// fx_id = 元素 sublimateFX；rotation = 方向角（镜像原版常量，C# 渲染端解释）。
pub(crate) fn push_spawn_fx(sim: &mut SimData, sim_cell: usize, fx_id: i32, rotation: f32) {
    if sim.sim_events.ptr.is_null() {
        return;
    }
    let width = sim.width;
    let game_w = width - 2;
    let internal = sim_cell as i32;
    let game_cell = (internal % width - 1) + (internal / width - 1) * game_w;
    if game_cell >= 0 && game_cell < sim.num_game_cells {
        let events = unsafe { &mut *sim.sim_events.ptr };
        events.spawn_fx_info.push(SpawnFXInfo {
            cell_idx: game_cell,
            fx_id,
            rotation,
        });
    }
}
```

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test -p simdll_rs push_spawn_fx -- --test-threads=1`
预期：PASS

- [ ] **步骤 5：Commit**

```bash
git add simDLL_RS/src/c2_physics/liquid_flow.rs
git commit -m "feat(liquid_flow): add push_spawn_fx event helper"
```

---

### 任务 2：`do_sublimation` 完整实现 + 调用点修正

**文件：** `simDLL_RS/src/c2_physics/liquid_flow.rs:1010`（替换 stub）、`liquid_flow.rs:1843/1959/2139`（调用点）

- [ ] **步骤 1：写失败测试（含测试表辅助）**

在 tests 模块内新增测试表辅助（放在 `create_displace_table` 之后）：

```rust
    /// 自建 8 元素升华表：
    /// - 0：真空（state=0）
    /// - 1：固体 A（state=3，sublimate_index=4→气体4，rate=0.4，eff=0.5，prob=1.0，fx=101）
    /// - 2：固体 B（state=3，sublimate_index=5→气体5，rate=0.1，eff=0.2，prob=1.0，fx=102）
    /// - 3：气体 X（state=1，molar=16）
    /// - 4：气体 Y（state=1，molar=44）
    /// - 5：气体 Z（state=1，molar=28）
    /// - 6：普通固体（state=3，sublimate_index=0xFFFF，不升华）
    /// - 7：液体（state=2，sublimate_index=5，prob=1.0，off_gas=0.5，eff=1.0，fx=107）
    fn create_sublimate_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(8);
        let elems = [
            Element::default(),
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 3;
                e.number_of_gradient_colors = 1;
                e.sublimate_index = 4;
                e.sublimate_rate = 0.4;
                e.sublimate_efficiency = 0.5;
                e.sublimate_probability = 1.0;
                e.sublimate_fx = 101;
                e
            },
            {
                let mut e = Element::default();
                e.id = 2;
                e.state = 3;
                e.number_of_gradient_colors = 1;
                e.sublimate_index = 5;
                e.sublimate_rate = 0.1;
                e.sublimate_efficiency = 0.2;
                e.sublimate_probability = 1.0;
                e.sublimate_fx = 102;
                e
            },
            {
                let mut e = Element::default();
                e.id = 3;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 16.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 4;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 44.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 5;
                e.state = 1;
                e.number_of_gradient_colors = 1;
                e.molar_mass = 28.0;
                e
            },
            {
                let mut e = Element::default();
                e.id = 6;
                e.state = 3;
                e.number_of_gradient_colors = 1;
                e.sublimate_index = 0xffff;
                e
            },
            {
                let mut e = Element::default();
                e.id = 7;
                e.state = 2;
                e.number_of_gradient_colors = 1;
                e.flow = 50.0;
                e.viscosity = 50.0;
                e.max_mass = 1000.0;
                e.min_vertical_flow = 0.0;
                e.low_temp = 0.0;
                e.high_temp = 10000.0;
                e.sublimate_index = 5;
                e.sublimate_probability = 1.0;
                e.off_gas_percentage = 0.5;
                e.sublimate_efficiency = 1.0;
                e.sublimate_fx = 107;
                e
            },
        ];
        for elem in &elems {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..8 {
            w.write_int(0);
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }
```

测试组 A（核心路径）：

```rust
    /// DoSublimation 入口（L148633）：调用格质量 >= 1.8 → 整函数跳过，randomSeed 不消耗。
    #[test]
    fn do_sublimation_skips_when_target_full() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 4);
                buf.mass.set(14, 1.8);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1); // 有效升华邻格
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
            }
        }
        let seed_before = sd.random_seed;
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(20) - 100.0).abs() < 1e-4);
        }
        assert_eq!(sd.random_seed, seed_before, "满格不消耗 randomSeed");
        DestroyElementsTable();
    }

    /// 同种气体合并（L148681-148702）：cell14=气体4 mass 1.0，上方砖 A（rate0.4→f18=0.08，eff0.5）。
    /// 目标 += 0.08×0.5=0.04；砖格 -= 0.08；accumulated_flow[砖] += 0.08；
    /// SpawnFX(砖, fx=101, rotation=180 上方)；同元素合并无 substance 事件；温度质量加权。
    #[test]
    fn do_sublimation_merges_same_element() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 4);
                buf.mass.set(14, 1.0);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 250.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(14) - 1.04).abs() < 1e-4, "目标 += 0.04");
            assert!((updated.mass.get(20) - 99.92).abs() < 1e-4, "砖格 -= 0.08");
            assert!((updated.temperature.get(14) - 298.0769).abs() < 1e-3, "温度加权");
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!((acc[20] - 0.08).abs() < 1e-4, "accumulated_flow 累积在砖格");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 0, "同元素合并无 substance 事件");
            assert_eq!(events.spawn_fx_info.len(), 1);
            let fx = events.spawn_fx_info.get(0);
            assert_eq!(fx.cell_idx, 9, "sim20 → game 9");
            assert_eq!(fx.fx_id, 101);
            assert_eq!(fx.rotation, 180.0, "上方邻格方向角");
        }
        DestroyElementsTable();
    }

    /// 1.8 上限缩放（L148692-148702）：目标 1.79，room=0.01 < f20=0.04 →
    /// f21 = 0.08×(0.01/0.04) = 0.02；目标恰好到 1.8，砖格只扣 0.02。
    #[test]
    fn do_sublimation_caps_at_1_8() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 4);
                buf.mass.set(14, 1.79);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(14) - 1.8).abs() < 1e-5, "目标恰好 1.8");
            assert!((updated.mass.get(20) - 99.98).abs() < 1e-4, "砖格扣缩放后的 0.02");
        }
        DestroyElementsTable();
    }

    /// 真空换元素（L148719-148753）：cell14 真空 → 元素=4、质量 += 0.04、温度/病菌拷贝、
    /// substance 事件；砖格扣 0.08、acc[20]+=0.08、SpawnFX 方向 180。
    #[test]
    fn do_sublimation_sets_element_in_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 250.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 4, "真空格换成升华产物");
            assert!((updated.mass.get(14) - 0.04).abs() < 1e-5);
            assert!((updated.temperature.get(14) - 250.0).abs() < 1e-4, "温度拷贝自砖格");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1);
            assert_eq!(events.substance_change_info.get(0).cell_idx, 5, "sim14 → game 5");
        }
        DestroyElementsTable();
    }

    /// 病菌按比例（L148676-148679）：砖格 mass=0.5、病菌 100 → i15 = (0.08/0.5)×100 = 16。
    /// 目标得病菌 idx 拷贝 + count=16；砖格病菌剩 84。
    #[test]
    fn do_sublimation_transfers_disease_proportionally() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 0.5);
                buf.temperature.set(20, 250.0);
                buf.disease_idx.set(20, 3);
                buf.disease_count.set(20, 100);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(20) - 0.42).abs() < 1e-5);
            assert_eq!(updated.disease_count.get(20), 84);
            assert_eq!(updated.disease_idx.get(14), 3, "病菌 idx 拷贝");
            assert_eq!(updated.disease_count.get(14), 16, "病菌按比例转移");
        }
        DestroyElementsTable();
    }
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test -p simdll_rs do_sublimation -- --test-threads=1`
预期：FAIL，报错 "cannot find function `do_sublimation` in this scope"

- [ ] **步骤 3：实现 `do_sublimation` 并修正调用点**

把 `do_sublimation_stub`（L1010）整体替换为：

```rust
/// DoSublimation（原版 SimDLL_Source.c L148591-148880）——自然固体格升华。
/// 由真空/气体格驱动（PostProcessCell 真空/气体分支末尾调用）：检查四邻固体砖
/// （上/右/左/下），概率门通过后把砖块质量以升华产物气体形式转移到调用格。
fn do_sublimation(sim: &mut SimData, cell: usize) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim.width as usize;
    let height = sim.height as usize;
    if width < 3 || height < 3 || cell < width {
        return;
    }
    let total = width * height;
    // 入口（L148633）：调用格质量 < 1.8（mass <= 1.8 && mass != 1.8）
    let entry_mass = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        if cell >= updated.mass.len() {
            return;
        }
        updated.mass.get(cell)
    };
    if entry_mass >= 1.8 {
        return;
    }
    // 入口快照（L148650 sVar4 读一次；param_4->state 用入口 ppd）
    let (entry_elem, entry_state) = {
        let updated = unsafe { &*sim.updated_cells.ptr };
        let elem = updated.element_idx.get(cell);
        match elements_table::get_element_post_process_data(elem) {
            Some(p) => (elem, p.state & 3),
            None => return,
        }
    };
    let mut bvar16 = false;
    // 四邻顺序 [cell+width, cell+1, cell-1, cell-width]（L148649-148652）
    let neighbors = [
        cell + width,
        cell + 1,
        cell.wrapping_sub(1),
        cell.wrapping_sub(width),
    ];
    for &nb in &neighbors {
        if nb >= total {
            continue;
        }
        // 邻格：固体（state&3==3）且 sublimateIndex 有效才消耗 randomSeed（L148656-148662）
        let nb_ppd = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            if nb >= updated.element_idx.len() {
                continue;
            }
            match elements_table::get_element_post_process_data(updated.element_idx.get(nb)) {
                Some(p) => p,
                None => continue,
            }
        };
        if (nb_ppd.state & 3) != 3 || nb_ppd.sublimate_index == 0xffff {
            continue;
        }
        let rnd = next_random(sim);
        if rnd > nb_ppd.sublimate_probability {
            continue; // 原版 random <= probability（fVar18 < p || fVar18 == p）
        }
        let f3 = {
            let updated = unsafe { &*sim.updated_cells.ptr };
            updated.mass.get(nb)
        };
        let f18 = nb_ppd.sublimate_rate * 0.2;
        let mut f21 = f18.min(f3);
        if f18 <= f3 - f21 {
            // —— 部分转移分支（L148666-148791）——
            let (disease_nb, temp_nb, disease_idx_nb) = {
                let updated = unsafe { &*sim.updated_cells.ptr };
                (
                    updated.disease_count.get(nb),
                    updated.temperature.get(nb),
                    updated.disease_idx.get(nb),
                )
            };
            let mut i15 = ((f21 / f3) * disease_nb as f32) as i32;
            if entry_elem == nb_ppd.sublimate_index {
                // 同种气体：1.8 上限缩放（L148681-148702）
                let mut f20 = f21 * nb_ppd.sublimate_efficiency;
                let room = {
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    1.8 - updated.mass.get(cell)
                };
                if room < f20 {
                    f21 = (room / f20) * f21;
                    i15 = ((f21 / f3) * disease_nb as f32) as i32;
                    f20 = room;
                }
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                add_mass_and_update_temperature(updated, cell, f20, temp_nb, disease_idx_nb, i15);
                bvar16 = true;
            } else {
                // 异种/真空目标（L148705-148758）
                let mut set_element = false;
                if entry_state == 0 {
                    set_element = true;
                } else {
                    let cur_elem = {
                        let updated = unsafe { &*sim.updated_cells.ptr };
                        updated.element_idx.get(cell)
                    };
                    if displace_gas(sim, cell, cur_elem) {
                        set_element = true;
                    }
                }
                if !set_element && !bvar16 {
                    continue; // 无先前成功：跳过该邻格（L148758）
                }
                if set_element {
                    // 换元素段（L148719-148753）
                    let updated = unsafe { &mut *sim.updated_cells.ptr };
                    updated.element_idx.set(cell, nb_ppd.sublimate_index);
                    updated.mass.set(
                        cell,
                        updated.mass.get(cell) + f21 * nb_ppd.sublimate_efficiency,
                    );
                    updated.temperature.set(cell, temp_nb);
                    updated.disease_idx.set(cell, disease_idx_nb);
                    updated.disease_count.set(cell, i15);
                    push_substance_change(sim, cell);
                    bvar16 = true;
                }
                // 怪癖（L148758 fall-through）：displace 失败但 bvar16==true →
                // 仍执行下方扣减段，目标格不产气（逐行镜像）。
            }
            // —— 共同扣减段（L148761-148791）——
            {
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                updated.mass.set(nb, updated.mass.get(nb) - f21);
                let remain = updated.disease_count.get(nb) - i15;
                if remain < 1 {
                    clear_disease(updated, nb);
                } else {
                    updated.disease_count.set(nb, remain);
                }
            }
            if !sim.accumulated_flow.ptr.is_null() {
                let acc = unsafe { std::slice::from_raw_parts_mut(sim.accumulated_flow.ptr, total) };
                acc[nb] += f21;
            }
            let dir = if nb == cell + width {
                180.0
            } else if nb == cell + 1 {
                90.0
            } else if nb == cell.wrapping_sub(1) {
                -90.0
            } else {
                0.0
            };
            push_spawn_fx(sim, nb, nb_ppd.sublimate_fx, dir);
            {
                let updated = unsafe { &*sim.updated_cells.ptr };
                if updated.mass.get(nb) <= 1e-09 {
                    do_evaporate(sim, nb);
                }
            }
        } else {
            // —— 整格消耗分支（L148848-148866）：砖块就地转气体 ——
            {
                let updated = unsafe { &mut *sim.updated_cells.ptr };
                updated.element_idx.set(nb, nb_ppd.sublimate_index);
                updated.mass.set(nb, f3 * nb_ppd.sublimate_efficiency);
            }
            push_substance_change(sim, nb);
            push_spawn_fx(sim, nb, nb_ppd.sublimate_fx, 0.0);
            bvar16 = true;
        }
    }
}
```

修正三个调用点：

1. 真空分支（L1843）：`do_sublimation_stub(sim, cell);` → `do_sublimation(sim, cell);`
2. 气体分支末尾（L1959）：`do_sublimation_stub(sim, cell);` → `do_sublimation(sim, cell);`
3. 液体分支末尾（L2139）：**删除** `do_sublimation_stub(sim, cell);`（原版液体分支不调 DoSublimation，offgas 是内联路径）

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test -p simdll_rs do_sublimation -- --test-threads=1`
预期：PASS（5 项）

- [ ] **步骤 5：运行既有升华/后处理回归**

运行：`cargo test -p simdll_rs post_process -- --test-threads=1`
预期：全部 PASS（调用点改动不破坏既有行为）

- [ ] **步骤 6：Commit**

```bash
git add simDLL_RS/src/c2_physics/liquid_flow.rs
git commit -m "feat(liquid_flow): implement DoSublimation (L148591-148880)"
```

---

### 任务 3：`do_sublimation` 边界行为测试

**文件：** `simDLL_RS/src/c2_physics/liquid_flow.rs`（tests 模块）

- [ ] **步骤 1：写边界测试**

```rust
    /// 异种气体 DisplaceGas 成功（L148705-148753）：cell14=气体X mass 0.5，上方砖 A（产物气体4）。
    /// tick0 候选 [20,13,15,8]：20 固体、13 真空 → 气体 X 全量移到 13；
    /// 目标换元素 4、质量 += 0.04；砖 A 扣 0.08。
    #[test]
    fn do_sublimation_displaces_heterogeneous_gas() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.tick_count = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 3); // 气体 X
                buf.mass.set(14, 0.5);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1); // 固体 A（产物 4）
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 250.0);
                buf.element_idx.set(13, 0); // 真空（DisplaceGas 目的地）
                buf.mass.set(13, 0.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 3, "异类气体被挤到 13");
            assert!((updated.mass.get(13) - 0.5).abs() < 1e-4);
            assert_eq!(updated.element_idx.get(14), 4, "目标换成升华产物");
            assert!((updated.mass.get(14) - 0.04).abs() < 1e-5, "挤走后只剩新增 0.04");
            assert!((updated.temperature.get(14) - 250.0).abs() < 1e-4);
            assert!((updated.mass.get(20) - 99.92).abs() < 1e-4);
        }
        DestroyElementsTable();
    }

    /// DisplaceGas 失败且无先前成功（L148758）：四邻全固体/无真空 → 跳过，砖格无损、无事件。
    #[test]
    fn do_sublimation_skips_when_displace_fails_and_no_prior_success() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 3);
                buf.mass.set(14, 0.5);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
                for nb in [15usize, 13, 8] {
                    buf.element_idx.set(nb, 6); // 普通固体，无升华
                    buf.mass.set(nb, 500.0);
                    buf.temperature.set(nb, 300.0);
                }
            }
        }
        let seed_before = sd.random_seed;
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(14) - 0.5).abs() < 1e-4);
            assert!((updated.mass.get(20) - 100.0).abs() < 1e-4);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 0);
        }
        let expected = seed_before.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        assert_eq!(sd.random_seed, expected, "仅 1 个有效邻格 → 恰好消耗 1 次");
        DestroyElementsTable();
    }

    /// 原版怪癖（L148758 fall-through）：前一邻格成功（bvar16=true）后，
    /// 后续邻格 DisplaceGas 失败 → 仍扣该砖质量/播 FX，但目标不产气。
    /// cell14=气体X 0.5；20=砖A(产物4)→先成功（挤到13）；15=砖B(产物5)→挤 4 失败（四邻堵死）。
    #[test]
    fn do_sublimation_quirk_mirrors_mass_loss_after_prior_success() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.tick_count = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 3);
                buf.mass.set(14, 0.5);
                buf.temperature.set(14, 300.0);
                buf.element_idx.set(20, 1); // 产物 4 ≠ 3 → 挤 3（成功，13 真空）
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
                buf.element_idx.set(15, 2); // 产物 5 → 挤当前 4（失败，四邻无真空/同气体）
                buf.mass.set(15, 100.0);
                buf.temperature.set(15, 300.0);
                buf.element_idx.set(13, 0);
                buf.mass.set(13, 0.0);
                buf.element_idx.set(8, 6);
                buf.mass.set(8, 500.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(13), 3, "气体 X 被挤到 13");
            assert_eq!(updated.element_idx.get(14), 4, "第一邻格成功换元素");
            assert!((updated.mass.get(14) - 0.04).abs() < 1e-5);
            assert!(
                (updated.mass.get(15) - 99.98).abs() < 1e-4,
                "怪癖：失败邻格仍扣 0.02（f18=0.1×0.2）"
            );
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!((acc[15] - 0.02).abs() < 1e-5, "怪癖：flow 仍累积");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 2, "两个邻格都有 FX");
        }
        DestroyElementsTable();
    }

    /// 整格消耗分支（L148848-148866）：砖质量 0.05 <= f18=0.08 → 砖格就地转气体、
    /// 质量 ×eff、substance 事件、SpawnFX(0.0)；调用格（真空）不变。
    #[test]
    fn do_sublimation_full_consumption_converts_in_place() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
                buf.element_idx.set(20, 1);
                buf.mass.set(20, 0.05);
                buf.temperature.set(20, 250.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(20), 4, "砖格就地转气体");
            assert!((updated.mass.get(20) - 0.025).abs() < 1e-5, "0.05×0.5");
            assert_eq!(updated.element_idx.get(14), 0, "调用格不变");
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert_eq!(acc[20], 0.0, "整格消耗分支不累积 flow");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1);
            assert_eq!(events.substance_change_info.get(0).cell_idx, 9);
            let fx = events.spawn_fx_info.get(0);
            assert_eq!(fx.rotation, 0.0);
        }
        DestroyElementsTable();
    }

    /// 入口快照语义（L148650/param_4->state）：入口真空 → 后续邻格走换元素段**不尝试 DisplaceGas**，
    /// 即使目标已被前一邻格换成气体 4。最终元素 = 后邻格产物 5。
    #[test]
    fn do_sublimation_uses_entry_snapshot_for_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                buf.temperature.set(14, 0.0);
                buf.element_idx.set(20, 1); // 产物 4
                buf.mass.set(20, 100.0);
                buf.temperature.set(20, 300.0);
                buf.element_idx.set(15, 2); // 产物 5
                buf.mass.set(15, 100.0);
                buf.temperature.set(15, 300.0);
                buf.element_idx.set(13, 6);
                buf.mass.set(13, 500.0);
                buf.element_idx.set(8, 6);
                buf.mass.set(8, 500.0);
            }
        }
        do_sublimation(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 5, "后邻格覆盖（入口快照驱动）");
            assert!((updated.mass.get(14) - 0.044).abs() < 1e-5, "0.04 + 0.02×0.2");
            assert!((updated.mass.get(20) - 99.92).abs() < 1e-4);
            assert!((updated.mass.get(15) - 99.98).abs() < 1e-4);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 2, "两次换元素各一次事件");
            assert_eq!(events.substance_change_info.get(0).cell_idx, 5);
            assert_eq!(events.substance_change_info.get(1).cell_idx, 5);
        }
        DestroyElementsTable();
    }

    /// 随机种子只对有效邻格消耗（L148656-148662）：四邻全为普通固体 → 种子不变。
    #[test]
    fn do_sublimation_consumes_seed_only_for_valid_neighbors() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            for buf in [&mut *sd.cells.ptr, &mut *sd.updated_cells.ptr] {
                buf.element_idx.set(14, 0);
                buf.mass.set(14, 0.0);
                for nb in [20usize, 15, 13, 8] {
                    buf.element_idx.set(nb, 6);
                    buf.mass.set(nb, 500.0);
                }
            }
        }
        let seed_before = sd.random_seed;
        do_sublimation(&mut sd, 14);
        assert_eq!(sd.random_seed, seed_before, "无有效邻格不消耗");
        DestroyElementsTable();
    }
```

- [ ] **步骤 2：运行测试确认通过**

运行：`cargo test -p simdll_rs do_sublimation -- --test-threads=1`
预期：PASS（全部 12 项，含任务 2 的 5 项）

若个别失败，按原版行号核对实现（重点：四邻顺序、入口快照、displace 失败分支的 bvar16 判定）后修正重跑。

- [ ] **步骤 3：Commit**

```bash
git add simDLL_RS/src/c2_physics/liquid_flow.rs
git commit -m "test(liquid_flow): DoSublimation boundary behaviors"
```

---

### 任务 4：液体 offgas 主路径产气（`emit_sublimate_gas`）

**文件：** `simDLL_RS/src/c2_physics/liquid_flow.rs:996`（emit 辅助）、`liquid_flow.rs:2102-2139`（offgas 块重构）

- [ ] **步骤 1：写失败测试（主路径）**

```rust
    /// 液体 offgas 主路径产气（L149806-149819）：cell14 液体 mass=100@300、病菌 100，
    /// 目标 20 真空。f24 = min(0.5×100, 1.0) = 1.0；目标换元素 5、质量 1.0、温度 300、
    /// 病菌按比例 1；液体格扣 1.0/病菌 1；acc[14] += 1.0；SpawnFX(14, 107, 0.0)。
    #[test]
    fn offgas_main_path_emits_into_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7); // 液体
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.disease_idx.set(14, 3);
            updated.disease_count.set(14, 100);
            updated.element_idx.set(20, 0); // 目标真空
            updated.mass.set(20, 0.0);
            updated.temperature.set(20, 0.0);
            updated.element_idx.set(8, 0); // cell-width 真空，跳过同元素重算
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(20), 5, "目标换成污染氧");
            assert!((updated.mass.get(20) - 1.0).abs() < 1e-4);
            assert!((updated.temperature.get(20) - 300.0).abs() < 1e-4);
            assert_eq!(updated.disease_idx.get(20), 3);
            assert_eq!(updated.disease_count.get(20), 1, "病菌按 1/100 比例");
            assert!((updated.mass.get(14) - 99.0).abs() < 1e-4, "液体扣 1.0");
            assert_eq!(updated.disease_count.get(14), 99);
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!((acc[14] - 1.0).abs() < 1e-4);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 1);
            assert_eq!(events.substance_change_info.get(0).cell_idx, 9, "sim20 → game 9");
            assert_eq!(events.spawn_fx_info.len(), 1);
            let fx = events.spawn_fx_info.get(0);
            assert_eq!(fx.cell_idx, 5);
            assert_eq!(fx.fx_id, 107);
            assert_eq!(fx.rotation, 0.0);
        }
        DestroyElementsTable();
    }

    /// 同种气体合并 + 病菌合并（同菌种）：目标 20 = 气体5 mass 0.5@200、病菌 10。
    /// f24 = 1.0；room = 1.8-0.5 = 1.3 ≥ 1.0 → 不缩放；合并后质量 1.5、温度 266.67、
    /// 病菌 10+1=11；无 substance 事件（同元素）。
    #[test]
    fn offgas_main_path_merges_same_gas_and_disease() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.disease_idx.set(14, 3);
            updated.disease_count.set(14, 100);
            updated.element_idx.set(20, 5); // 同种气体
            updated.mass.set(20, 0.5);
            updated.temperature.set(20, 200.0);
            updated.disease_idx.set(20, 3);
            updated.disease_count.set(20, 10);
            updated.element_idx.set(8, 0);
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(20) - 1.5).abs() < 1e-4);
            assert!((updated.temperature.get(20) - 266.667).abs() < 1e-2, "温度加权");
            assert_eq!(updated.disease_count.get(20), 11, "同菌种合并");
            assert!((updated.mass.get(14) - 99.0).abs() < 1e-4);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.substance_change_info.len(), 0, "同元素合并无事件");
        }
        DestroyElementsTable();
    }

    /// room 缩放（L2809-2812）：目标 20 = 气体5 mass 1.0；scaled = 1.0，room = 0.8 →
    /// f24 = 0.8；目标恰好 1.8；液体扣 0.8；acc[14] = 0.8。
    #[test]
    fn offgas_main_path_scales_to_room() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(20, 5);
            updated.mass.set(20, 1.0);
            updated.temperature.set(20, 300.0);
            updated.element_idx.set(8, 0);
            updated.mass.set(8, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(20) - 1.8).abs() < 1e-4, "目标恰好 1.8");
            assert!((updated.mass.get(14) - 99.2).abs() < 1e-4);
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert!((acc[14] - 0.8).abs() < 1e-4);
        }
        DestroyElementsTable();
    }

    /// 异类气体 DisplaceGas 失败 → 直接 return：无产气、无扣质量、无 acc、无 FX。
    /// 目标 20 = 气体3（≠产物5），其四邻 [26,19,21,14] 全为固体/液体 → 挤不走。
    #[test]
    fn offgas_main_path_returns_when_displace_fails() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        sd.tick_count = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 100.0);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(20, 3); // 异类气体
            updated.mass.set(20, 0.1);
            updated.temperature.set(20, 300.0);
            updated.element_idx.set(8, 0);
            updated.mass.set(8, 0.0);
            for nb in [26usize, 19, 21] {
                updated.element_idx.set(nb, 6); // 固体堵死
                updated.mass.set(nb, 500.0);
            }
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert!((updated.mass.get(14) - 100.0).abs() < 1e-4, "挤不走则不产气");
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert_eq!(acc[14], 0.0);
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 0);
        }
        DestroyElementsTable();
    }
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test -p simdll_rs offgas_main_path -- --test-threads=1`
预期：FAIL（质量/元素断言失败，产气未实现）

- [ ] **步骤 3：实现 `emit_sublimate_gas` + 重构 offgas 块**

在 `do_evaporate` 之后新增：

```rust
/// 液体 offgas 产气辅助（原版 L149810 匿名 lambda 的可观测语义）：
/// 源格扣 amount 质量（病菌按比例），目标格获得 sublimateIndex 气体
/// （真空/异种 → 换元素 + substance 事件；同种 → 质量合并），温度加权；
/// merge_disease=true 时按 add_mass_and_update_temperature 合并病菌，否则只合质量/温度。
/// src_cell == dst_cell 时等价于"就地转气体"（耗尽路径）。
fn emit_sublimate_gas(
    sim: &mut SimData,
    src_cell: usize,
    dst_cell: usize,
    amount: f32,
    merge_disease: bool,
    sublimate_index: u16,
) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let updated = unsafe { &mut *sim.updated_cells.ptr };
    if src_cell >= updated.mass.len() || dst_cell >= updated.mass.len() {
        return;
    }
    // 源格扣质量/病菌（比例），病菌 idx 先读
    let src_mass = updated.mass.get(src_cell);
    let src_disease_idx = updated.disease_idx.get(src_cell);
    let src_disease_count = updated.disease_count.get(src_cell);
    let d = if src_mass > 0.0 {
        (src_disease_count as f32 * (amount / src_mass)) as i32
    } else {
        0
    };
    updated.mass.set(src_cell, src_mass - amount);
    let remain = updated.disease_count.get(src_cell) - d;
    if remain < 1 {
        clear_disease(updated, src_cell);
    } else {
        updated.disease_count.set(src_cell, remain);
    }
    // 目标格：真空/异种 → 换元素 + 事件
    let dst_elem = updated.element_idx.get(dst_cell);
    if dst_elem != sublimate_index {
        updated.element_idx.set(dst_cell, sublimate_index);
        push_substance_change(sim, dst_cell);
    }
    let src_temp = updated.temperature.get(src_cell);
    if merge_disease {
        add_mass_and_update_temperature(updated, dst_cell, amount, src_temp, src_disease_idx, d);
    } else {
        let cur_mass = updated.mass.get(dst_cell);
        let new_mass = cur_mass + amount;
        let cur_temp = updated.temperature.get(dst_cell);
        let avg = (cur_mass * cur_temp + src_temp * amount) / new_mass;
        let mixed = avg.clamp(cur_temp.min(src_temp), cur_temp.max(src_temp));
        updated.mass.set(dst_cell, new_mass);
        updated.temperature.set(dst_cell, mixed);
    }
}
```

重构 offgas 块（L2102-2139）。保留概率门/1.8 上限/同元素重算/room 缩放不动，替换"剩余质量判定 + acc 累加 + stub 调用"为：

```rust
                // 原版 L2788-2790：剩余质量 >= 0.01（fVar24 <= 0 时原版直接 return，无 FX）
                if mass_cell - fvar24 >= 0.01 {
                    if fvar24 <= 0.0 {
                        return;
                    }
                    let below_elem = {
                        let updated = unsafe { &*sim.updated_cells.ptr };
                        updated.element_idx.get(below)
                    };
                    if below_elem != ppd.sublimate_index {
                        if below_elem != sim.vacuum_element_idx {
                            if !displace_gas(sim, below, below_elem) {
                                return;
                            }
                        }
                    }
                    let mass_below_now = {
                        let updated = unsafe { &*sim.updated_cells.ptr };
                        updated.mass.get(below)
                    };
                    let scaled = fvar24 * ppd.sublimate_efficiency;
                    let room = 1.8 - mass_below_now;
                    if room < scaled {
                        fvar24 = fvar24 * (room / scaled);
                    }
                    // 病菌合并判定（L149806-149819）：同菌种或目标无菌 → 合并
                    let merge = {
                        let updated = unsafe { &*sim.updated_cells.ptr };
                        let src_idx = updated.disease_idx.get(cell);
                        let dst_idx = updated.disease_idx.get(below);
                        src_idx == dst_idx || dst_idx == 0xff
                    };
                    emit_sublimate_gas(sim, cell, below, fvar24, merge, ppd.sublimate_index);
                    // 原版 L2831-2832：accumulated_flow[cell] += fVar24
                    let total = sim.width as usize * sim.height as usize;
                    let acc = unsafe { std::slice::from_raw_parts_mut(sim.accumulated_flow.ptr, total) };
                    if cell < total {
                        acc[cell] += fvar24;
                    }
                } else {
                    // 耗尽路径（L149821-149833）：液体格就地转气体；差额从 cell-width 补足
                    emit_sublimate_gas(sim, cell, cell, mass_cell, true, ppd.sublimate_index);
                    if fvar24 - mass_cell > 0.0 && heavier_up {
                        let down = cell.wrapping_sub(width);
                        let merge = {
                            let updated = unsafe { &*sim.updated_cells.ptr };
                            let src_idx = updated.disease_idx.get(down);
                            let cell_idx = updated.disease_idx.get(cell);
                            src_idx == cell_idx || cell_idx == 0xff
                        };
                        emit_sublimate_gas(sim, down, cell, fvar24 - mass_cell, merge, ppd.sublimate_index);
                    }
                }
                // 原版 LAB_18004e400：SpawnFX(cell, sublimateFX, 0.0)
                push_spawn_fx(sim, cell, ppd.sublimate_fx, 0.0);
            }
        }
    }
```

同时：在 `let mut fvar24 = ...`（L2082）之后把"cell-width 同元素重算"块（L2087-2100）替换为带 `heavier_up` 标志跟踪的版本（标志供耗尽路径第二段判定使用，原版 L2770-2785 bVar9）：

```rust
                let mut heavier_up = false;
                let up = cell.wrapping_sub(width);
                {
                    let updated = unsafe { &*sim.updated_cells.ptr };
                    if updated.element_idx.get(up) == elem {
                        let mass_up = updated.mass.get(up);
                        if mass_cell < mass_up {
                            heavier_up = true;
                            fvar24 = ppd.off_gas_percentage * mass_up;
                            if fvar24 >= 1.0 {
                                fvar24 = 1.0;
                            }
                        }
                    }
                }
```

注：液体分支末尾的 stub 调用已在任务 2 步骤 3 删除；本任务替换的 offgas 块尾部（原 L2139 位置）不再有任何 stub 调用。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test -p simdll_rs offgas -- --test-threads=1`
预期：PASS（4 项新测试 + 既有 offgas/升华路径测试全部通过）

- [ ] **步骤 5：Commit**

```bash
git add simDLL_RS/src/c2_physics/liquid_flow.rs
git commit -m "feat(liquid_flow): liquid offgas emission (main path)"
```

---

### 任务 5：液体 offgas 耗尽路径测试

**文件：** `simDLL_RS/src/c2_physics/liquid_flow.rs`（tests 模块）

- [ ] **步骤 1：写测试**

```rust
    /// 耗尽路径 + 差额补足（L149821-149833）：cell14 液体 mass=0.015，cell8（cell-width）
    /// 同液体更重（mass=10）→ f24 = min(0.5×10, 1.0) = 1.0；剩余 <0.01 →
    /// cell14 就地转气体 5（质量 0.015、病菌保留）；差额 0.985 从 cell8 补足 → cell14 质量 1.0；
    /// 耗尽路径**不累积** accumulated_flow；SpawnFX(14, 107, 0.0)。
    #[test]
    fn offgas_depleted_path_converts_in_place_with_top_up() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 0.015);
            updated.temperature.set(14, 300.0);
            updated.disease_idx.set(14, 3);
            updated.disease_count.set(14, 100);
            updated.element_idx.set(8, 7); // cell-width 同液体，更重
            updated.mass.set(8, 10.0);
            updated.temperature.set(8, 300.0);
            updated.element_idx.set(20, 0); // 目标真空
            updated.mass.set(20, 0.0);
            updated.temperature.set(20, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 5, "液体格就地转气体");
            assert!((updated.mass.get(14) - 1.0).abs() < 1e-4, "剩余 + 补足差额");
            assert!((updated.mass.get(8) - 9.015).abs() < 1e-4, "cell8 扣差额");
            assert_eq!(updated.disease_idx.get(14), 3, "就地转化保留病菌");
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, 30);
            assert_eq!(acc[14], 0.0, "耗尽路径不累积 flow");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 1);
            assert_eq!(events.spawn_fx_info.get(0).cell_idx, 5);
        }
        DestroyElementsTable();
    }

    /// 耗尽路径无补足：cell8 非同元素 → heavier_up=false；f24 = min(0.5×0.015,1)=0.0075；
    /// 只就地转化（质量 0.015），无第二段调用。
    #[test]
    fn offgas_depleted_path_without_top_up() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_sublimate_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.vacuum_element_idx = 0;
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(14, 7);
            updated.mass.set(14, 0.015);
            updated.temperature.set(14, 300.0);
            updated.element_idx.set(8, 0); // 非液体 → 不重算
            updated.mass.set(8, 0.0);
            updated.element_idx.set(20, 0);
            updated.mass.set(20, 0.0);
        }
        post_process_cell(&mut sd, 14);
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            assert_eq!(updated.element_idx.get(14), 5);
            assert!((updated.mass.get(14) - 0.015).abs() < 1e-5, "就地转化全部剩余");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.spawn_fx_info.len(), 1);
        }
        DestroyElementsTable();
    }
```

- [ ] **步骤 2：运行测试确认通过**

运行：`cargo test -p simdll_rs offgas_depleted -- --test-threads=1`
预期：PASS（实现已在任务 4 就位）

若失败，按原版 L149821-149833 核对耗尽路径实现（重点：第一段调用参数 = 液体格全质量、第二段条件 `fVar24 > fVar23 && bVar9`、耗尽路径无 acc）后修正重跑。

- [ ] **步骤 3：Commit**

```bash
git add simDLL_RS/src/c2_physics/liquid_flow.rs
git commit -m "test(liquid_flow): offgas depleted path"
```

---

### 任务 6：全量回归 + Release

**文件：** 无新增

- [ ] **步骤 1：全量测试**

运行：`cargo test -p simdll_rs -- --test-threads=1`
预期：全部 PASS（原 376 项 + 新增约 14 项）

- [ ] **步骤 2：Release 构建**

运行：`cargo build --release -p simdll_rs`
预期：成功

- [ ] **步骤 3：若有修复则 Commit**

若步骤 1/2 暴露问题并修正：`git add simDLL_RS/src/c2_physics/liquid_flow.rs` + `git commit -m "fix(liquid_flow): ..."`；
无修复则跳过（工作树应保持干净）。

## 交付后验收

- 用户游戏实测：氧石自然格持续出氧（自身质量减少、目标格 1.8kg 封顶、满后暂停、异种气体被挤走）；
  污染水缓慢冒污染氧（1% 概率/子步、0.1% 质量/次）；漂白石方块不升华；升华动画（SpawnFX）正常。
