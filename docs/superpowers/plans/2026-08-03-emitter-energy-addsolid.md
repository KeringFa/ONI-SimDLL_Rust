# ElementEmitter + ModifyCellEnergy + AddSolid 垂直位移 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans（本会话固定**内联执行**）
> 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法跟踪进度。

**目标：** 三项接线补全——A ElementEmitter 本体（注册表/消息/Update/TryEmit/输出）、
B ProcessCellEnergyModifications（火箭 3×9 热等 ModifyEnergy 落地）、
C add_solid 重写（Type 0 DoVerticalDisplacement / Type 1 OnlyIfSameElement，纠正四邻混合版）。

**架构：** 新文件 `c_simulation/element_emitter.rs`（仿 ElementChunkManager：版本句柄 + free list +
44B 记录 + emitted_mass_info 输出）；sim_frame_manager 把 3 条 emitter 消息从 C2 桩改为真实
emplace（16/32/8B）；frame_processor 新增 process_element_emitter_messages（adds→modifies→removes +
回调链）与 process_cell_energy_modifications 实现；liquid_flow.rs 重写 add_solid（加 add_sub_type
参数），frame_processor mode 0 固体分支传 entry.flags；update_data 挂载 update_element_emitters。

**技术栈：** Rust（cdylib），cargo test 串行（`--test-threads=1`），测试命令在 `simDLL_RS` 目录执行。

---

## 文件结构

| 文件 | 职责 | 变更 |
|------|------|------|
| `simDLL_RS/src/c_simulation/element_emitter.rs` | ElementEmitter 管理器 + 消息 + Update/TryEmit/Emit + BFS | 新建 |
| `simDLL_RS/src/c_simulation/mod.rs` | 挂载新模块 | 修改 |
| `simDLL_RS/src/a_framework/sim_frame_manager.rs:535-550` | 3 条 emitter 消息 C2 桩 → 真实 emplace | 修改 |
| `simDLL_RS/src/c_simulation/frame_processor.rs` | process_element_emitter_messages + process_cell_energy_modifications + add_solid 接线 | 修改 |
| `simDLL_RS/src/c2_physics/liquid_flow.rs` | add_solid 重写（Type 0/1） | 修改 |
| `simDLL_RS/src/c2_physics/mod.rs` | update_data 挂载 update_element_emitters | 修改 |
| `代办.txt` | 批次记录 + 实测清单 | 修改 |

## 任务 1：add_solid 重写（Type 0 DoVerticalDisplacement / Type 1 OnlyIfSameElement）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/liquid_flow.rs`（add_solid 重写 + 测试）
- 修改：`simDLL_RS/src/c_simulation/frame_processor.rs`（mode 0 固体分支传 entry.flags）

### 步骤 1：编写失败的测试

在 `liquid_flow.rs` 测试模块（`create_state_table` 建表，元素：0 气体 / 1 液体 / 2 固体）追加：

```rust
    #[test]
    fn add_solid_vertical_displacement_fills_above_and_places_down() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_state_table();
        let mut sd = make_test_sim_data_36();
        // 列（row1..3, col2）：上方 row1=同元素固体 900kg（maxMass 1000）；目标 row2=气体；
        // 下方 row3=气体。AddSolid Type 0：先补上方 100kg（900→1000），剩余 50kg 落到 row2。
        fill(&mut sd, 8, 2, 900.0, 300.0);   // row1 col2 = 同元素固体
        fill(&mut sd, 14, 0, 0.1, 300.0);    // row2 col2 = 气体(0)
        fill(&mut sd, 20, 0, 0.1, 300.0);    // row3 col2 = 气体(0)
        add_solid(&mut sd, 14, 2, 150.0, 400.0, 0xFF, 0, 0); // Type 0
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!((u.mass.get(8) - 1000.0).abs() < 1e-3, "上方应补满到 1000，got {}", u.mass.get(8));
        assert_eq!(u.element_idx.get(14), 2, "目标格应变成固体");
        assert!((u.mass.get(14) - 50.0).abs() < 1e-3, "剩余 50kg 应落到目标格，got {}", u.mass.get(14));
    }

    #[test]
    fn add_solid_vertical_displacement_skips_solid_and_displaces_gas() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_state_table();
        let mut sd = make_test_sim_data_36();
        fill(&mut sd, 8, 2, 900.0, 300.0);   // row1 同元素固体（maxMass 1000 → 可补 100）
        fill(&mut sd, 14, 2, 100.0, 300.0);  // row2 目标已是固体（同元素 2）→ 向下走
        fill(&mut sd, 20, 0, 0.5, 300.0);    // row3 气体 → 挤走并放置
        add_solid(&mut sd, 14, 2, 150.0, 400.0, 0xFF, 0, 0);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!((u.mass.get(8) - 1000.0).abs() < 1e-3, "上方补满");
        assert_eq!(u.element_idx.get(20), 2, "剩余落到 row3（跳过 row2 固体）");
        assert!((u.mass.get(20) - 50.0).abs() < 1e-3, "剩余 50kg，got {}", u.mass.get(20));
    }

    #[test]
    fn add_solid_only_same_element_solid_noop_and_merge() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_state_table();
        let mut sd = make_test_sim_data_36();
        // 目标固体不同元素 → no-op
        fill(&mut sd, 14, 2, 100.0, 300.0);
        add_solid(&mut sd, 14, 0, 50.0, 400.0, 0xFF, 0, 1);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 2, "不应替换");
        assert!((u.mass.get(14) - 100.0).abs() < 1e-3);
        // 目标同元素 → 合并
        add_solid(&mut sd, 14, 2, 50.0, 400.0, 0xFF, 0, 1);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!((u.mass.get(14) - 150.0).abs() < 1e-3, "应合并到 150");
    }

    #[test]
    fn add_solid_other_type_is_noop() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        create_state_table();
        let mut sd = make_test_sim_data_36();
        fill(&mut sd, 14, 0, 0.1, 300.0);
        add_solid(&mut sd, 14, 2, 50.0, 400.0, 0xFF, 0, 9); // 未知 Type
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.element_idx.get(14), 0, "不应放置");
    }
```

注：`make_test_sim_data_36` 是 6×6 sim 数据（36 格）的辅助，参照现有
`make_test_sim_data`（liquid_flow 测试）；`create_state_table` 建 3 元素表
（0=气体 state1 / 1=液体 state2 / 2=固体 state3，maxMass 需为 1000）——若元素序号/状态
与现有辅助不同，按实际表调整（断言随元素序号走）。垂直位移断言已按"补满 100 + 剩 50 落格"
校准。

### 步骤 2：运行测试验证失败

运行：`cargo test -p simdll_rs c2_physics::liquid_flow::tests::add_solid -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：FAIL，编译错误 "this function takes 7 arguments but 8 were supplied"（add_solid 尚无 Type 参数）。

### 步骤 3：重写 add_solid

`liquid_flow.rs` 把现有 add_solid 整体替换为：

```rust
/// AddSolid（原版 SimDLL_Source.c L134555-134789）。
/// `add_sub_type`（ModifyCell 消息 addSubType）：0 = DoVerticalDisplacement（默认，
/// 流星/AddRemoveSubstance 固体）；1 = OnlyIfSameElement；其他 → no-op。
pub(crate) fn add_solid(
    sim: &mut SimData,
    cell: usize,
    elem: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
    add_sub_type: u8,
) {
    if sim.updated_cells.ptr.is_null() {
        return;
    }
    let width = sim.width as usize;
    let total = width * sim.height as usize;
    if cell >= total {
        return;
    }
    // —— Type 0：DoVerticalDisplacement（L134573-134743）——
    if add_sub_type == 0 {
        let row = cell / width;
        let col = cell % width;
        let mut remaining = mass;
        // 阶段 1：同列 4 格（row−1/0/+1/+2）同元素 → 按 ppd.maxMass 补满
        for dr in [-1i32, 0, 1, 2] {
            let r = row as i32 + dr;
            if r < 0 || r as usize >= sim.height as usize {
                continue;
            }
            let c = r as usize * width + col;
            let updated = unsafe { &*sim.updated_cells.ptr };
            if updated.element_idx.get(c) != elem {
                continue;
            }
            let Some(max_mass) = elements_table::get_element_post_process_data(elem)
                .map(|p| p.max_mass)
            else {
                continue;
            };
            let room = max_mass - updated.mass.get(c);
            if room <= 0.0 {
                continue;
            }
            let f16 = room.min(remaining);
            let d = (f16 / remaining * disease_count as f32) as i32;
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            add_mass_and_update_temperature(updated, c, f16, temperature, disease_idx, d);
            remaining -= f16;
            if remaining <= 0.0 {
                return;
            }
        }
        // 阶段 2：从目标格向下走，跳过固体、挤走气/液、落到第一个非固体格
        let mut cur = cell;
        loop {
            if cur >= total {
                return;
            }
            let updated = unsafe { &*sim.updated_cells.ptr };
            let cur_elem = updated.element_idx.get(cur);
            let is_unobtanium = matches!(
                elements_table::get_element_by_idx(cur_elem),
                Some(e) if e.id == crate::b_elements::element::UNOBTANIUM_ELEMENT_ID
            );
            if is_unobtanium {
                return;
            }
            let state = match elements_table::get_element_post_process_data(cur_elem) {
                Some(p) => p.state & 3,
                None => return,
            };
            match state {
                3 => {
                    cur += width;
                    continue;
                }
                1 => {
                    crate::c2_physics::liquid_flow::displace_gas(sim, cur, cur_elem);
                }
                2 => {
                    crate::c2_physics::liquid_flow::displace_liquid(sim, cur, cur_elem);
                }
                _ => {}
            }
            let updated = unsafe { &mut *sim.updated_cells.ptr };
            updated.element_idx.set(cur, elem);
            updated.mass.set(cur, remaining);
            updated.temperature.set(cur, temperature);
            updated.disease_idx.set(cur, disease_idx);
            updated.disease_count.set(cur, disease_count);
            push_substance_change(sim, cur);
            return;
        }
    }
    // —— Type 1：OnlyIfSameElement（L134744-134789）——
    if add_sub_type == 1 {
        let updated = unsafe { &*sim.updated_cells.ptr };
        let cur_elem = updated.element_idx.get(cell);
        let state = match elements_table::get_element_post_process_data(cur_elem) {
            Some(p) => p.state & 3,
            None => return,
        };
        match state {
            3 => {
                if cur_elem != elem {
                    return;
                }
            }
            1 => {
                crate::c2_physics::liquid_flow::displace_gas(sim, cell, cur_elem);
            }
            2 => {
                crate::c2_physics::liquid_flow::displace_liquid(sim, cell, cur_elem);
            }
            _ => {}
        }
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        add_mass_and_update_temperature(updated, cell, mass, temperature, disease_idx, disease_count);
        push_substance_change(sim, cell);
        return;
    }
    // —— 其他 Type → no-op（原版直接 return）——
}
```

检查 `crate::b_elements::element::UNOBTANIUM_ELEMENT_ID` 常量名是否存在（若为
`UNOBTANIUM` 等，按实际名称替换；元素 id 常量对照 00_types_reference.c 的
`-0x56c9f4cc`）。

### 步骤 4：接线 frame_processor

`frame_processor.rs` mode 0 分派的固体分支（当前 `3 => add_solid(sim_data, idx, elem, mass, temp, didx, dcount)`）改为：
```rust
                        3 => crate::c2_physics::liquid_flow::add_solid(
                            sim_data, idx, elem, mass, temp, didx, dcount, entry.flags,
                        ),
```

### 步骤 5：运行测试验证通过

运行：`cargo test -p simdll_rs c2_physics::liquid_flow::tests::add_solid -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：4 条 add_solid 测试 PASS。

### 步骤 6：全量回归 + 提交

运行：`cargo test -p simdll_rs -- --test-threads=1 2>$null; $LASTEXITCODE` → 全 PASS

提交：
```bash
git add simDLL_RS/src/c2_physics/liquid_flow.rs simDLL_RS/src/c_simulation/frame_processor.rs
git commit -m "feat(liquid_flow): rewrite AddSolid with DoVerticalDisplacement/OnlyIfSameElement (L134555-134789)"
```

## 任务 2：ProcessCellEnergyModifications（火箭 3×9 热等）

**文件：**
- 修改：`simDLL_RS/src/c_simulation/frame_processor.rs`（桩替换 + 测试）

### 步骤 1：编写失败的测试

在 `frame_processor.rs` 测试模块追加：

```rust
    #[test]
    fn cell_energy_modification_heats_cell_and_caps() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table(); // 温度表：0 dummy / 1 水 / 2 冰 / 3 蒸汽
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();
        // 构造 16B 消息：gameCell=0（sim 7）、kj、maxT
        let mut buf = Vec::new();
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&500.0f32.to_le_bytes());
        buf.extend_from_slice(&3200.0f32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        for b in buf {
            frame.cell_energy_modifications.push(b);
        }
        // sim cell 7：水 50kg 300K → ΔT = 500/(50×4.179) ≈ 2.39 → 302.39（cap 3200 不生效）
        fill_sim_cell(&mut sd, 7, 1, 50.0, 300.0);
        process_cell_energy_modifications(&mut frame, &mut sd);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!((u.temperature.get(7) - 302.39).abs() < 0.05, "got {}", u.temperature.get(7));
        // cap 生效：maxT=301 → 302.39 被压到 301
        let mut frame = SimFrameInfo::default();
        let mut buf = Vec::new();
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&500.0f32.to_le_bytes());
        buf.extend_from_slice(&301.0f32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        for b in buf {
            frame.cell_energy_modifications.push(b);
        }
        fill_sim_cell(&mut sd, 7, 1, 50.0, 300.0);
        process_cell_energy_modifications(&mut frame, &mut sd);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert!((u.temperature.get(7) - 301.0).abs() < 1e-4, "cap 生效，got {}", u.temperature.get(7));
    }

    #[test]
    fn cell_energy_modification_skips_vacuum_and_tiny_mass() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        init_phase_table();
        let mut sd = make_test_sim_data();
        let mut frame = SimFrameInfo::default();
        let mut buf = Vec::new();
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&500.0f32.to_le_bytes());
        buf.extend_from_slice(&3200.0f32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        for b in buf {
            frame.cell_energy_modifications.push(b);
        }
        fill_sim_cell(&mut sd, 7, 0, 0.0, 300.0); // 真空
        process_cell_energy_modifications(&mut frame, &mut sd);
        let u = unsafe { &*sd.updated_cells.ptr };
        assert_eq!(u.temperature.get(7), 300.0, "真空不应加热");
    }
```

注：`fill_sim_cell(&mut sd, cell, elem, mass, temp)` 是 frame_processor 测试现有的辅助
（若无则按 `make_test_sim_data` 的 updated_cells 写入方式新增）；`init_phase_table`
为温度测试的 3 元素表辅助（若无则按 1=水 SHC 4.179 建最小表）。

### 步骤 2：运行测试验证失败

运行：`cargo test -p simdll_rs c_simulation::frame_processor::tests::cell_energy -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：FAIL（桩只清空不加热，温度保持 300）。

### 步骤 3：实现

`frame_processor.rs` 替换 `process_cell_energy_modifications`（L563）为：

```rust
/// ProcessCellEnergyModifications（原版 06_process_messages.c L134-211）。
/// 每条 16B：cellIdx i32 / kilojoules f32 / maxTemperature f32 / id i32（id 仅日志）。
/// ΔT = kilojoules/(mass×SHC)；new = min(旧+ΔT, max(旧, maxT))；钳制 [1,10000]；写后 DoStateTransition。
fn process_cell_energy_modifications(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    let game_w = sim_data.width - 2;
    if game_w <= 0 {
        frame.cell_energy_modifications.clear_keep_capacity();
        return;
    }
    if sim_data.updated_cells.ptr.is_null() || sim_data.sim_events.ptr.is_null() {
        frame.cell_energy_modifications.clear_keep_capacity();
        return;
    }
    let slice = frame.cell_energy_modifications.as_slice();
    let n = slice.len() / 16;
    for i in 0..n {
        let base = i * 16;
        let cell_idx =
            i32::from_le_bytes([slice[base], slice[base + 1], slice[base + 2], slice[base + 3]]);
        let kj = f32::from_le_bytes([
            slice[base + 4],
            slice[base + 5],
            slice[base + 6],
            slice[base + 7],
        ]);
        let max_temp = f32::from_le_bytes([
            slice[base + 8],
            slice[base + 9],
            slice[base + 10],
            slice[base + 11],
        ]);
        let internal = (cell_idx % game_w + 1) + (cell_idx / game_w + 1) * sim_data.width;
        let cell = internal as usize;
        let updated = unsafe { &*sim_data.updated_cells.ptr };
        if cell >= updated.mass.len() || cell >= updated.temperature.len() {
            continue;
        }
        let elem = updated.element_idx.get(cell);
        let Some(etd) = crate::b_elements::elements_table::get_element_temperature_data(elem) else {
            continue;
        };
        if (etd.state & 3) == 0 {
            continue; // 真空
        }
        let mass = updated.mass.get(cell);
        if mass <= 0.001 || kj <= 0.0 {
            continue;
        }
        let temp = updated.temperature.get(cell);
        let cap = temp.max(max_temp);
        let mut new_temp = temp + kj / (mass * etd.specific_heat_capacity);
        new_temp = new_temp.min(cap);
        if new_temp <= 0.0 || new_temp > 10000.0 {
            tracing::warn!(
                cell,
                kj,
                max_temp,
                mass,
                temp,
                new_temp,
                "ProcessCellEnergyModifications: invalid temperature"
            );
            new_temp = new_temp.clamp(1.0, 10000.0);
        }
        if new_temp > 0.0 {
            let updated = unsafe { &mut *sim_data.updated_cells.ptr };
            updated.temperature.set(cell, new_temp);
            crate::c2_physics::temperature::do_state_transition(sim_data, cell, &etd);
        }
    }
    frame.cell_energy_modifications.clear_keep_capacity();
}
```

### 步骤 4：运行测试验证通过

运行：`cargo test -p simdll_rs c_simulation::frame_processor::tests::cell_energy -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：2 条 cell_energy 测试 PASS。

### 步骤 5：全量回归 + 提交

```bash
git add simDLL_RS/src/c_simulation/frame_processor.rs
git commit -m "feat(frame_processor): implement ProcessCellEnergyModifications (L134-211, rocket exhaust heat)"
```

## 任务 3：ElementEmitter 注册表 + 消息路由 + 消息处理

**文件：**
- 新建：`simDLL_RS/src/c_simulation/element_emitter.rs`
- 修改：`simDLL_RS/src/c_simulation/mod.rs`（`pub mod element_emitter;`）
- 修改：`simDLL_RS/src/a_framework/sim_frame_manager.rs:535-550`（emplace 三消息）
- 修改：`simDLL_RS/src/c_simulation/frame_processor.rs`（process_element_emitter_messages）

### 步骤 1：新建 element_emitter.rs（结构 + 注册表 + 消息处理）

```rust
//! ElementEmitter（元素发射器）组件。
//! 对照原版 04_temperature.c L1495-2104（Emit/Modify/Register/TryEmit/Update）
//! + GetReachableCells（11_msvcrt_ignored.c L34451）+ CopySimDataToGame emittedMassInfo（L31626）。

use crate::a_framework::game_data::{ComponentStateChangedMessage, EmittedMassInfo};
use crate::a_framework::sim_data::SimData;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// ElementEmitterData（原版 44B = 0x2c，L10551）。
#[derive(Clone, Copy, Debug)]
pub struct ElementEmitterData {
    pub elapsed_time: f32,
    pub emit_interval: f32,
    pub emit_mass: f32,
    pub emit_temperature: f32,
    pub max_pressure: f32,
    pub emit_disease_count: i32,
    pub cell: [u16; 2], // [row, col]
    pub elem_idx: u16,
    pub max_depth: u8,
    pub offset_idx: u8,
    pub blocked_state: u8,
    pub disease_idx: u8,
    pub blocked_cb_idx: u32,
    pub unblocked_cb_idx: u32,
}

impl Default for ElementEmitterData {
    fn default() -> Self {
        Self {
            elapsed_time: 0.0,
            emit_interval: 0.0,
            emit_mass: 0.0,
            emit_temperature: 0.0,
            max_pressure: 0.0,
            emit_disease_count: 0,
            cell: [0, 0],
            elem_idx: 0xFFFF,
            max_depth: 0,
            offset_idx: 0,
            blocked_state: 0,
            disease_idx: 0xFF,
            blocked_cb_idx: 0xFFFFFFFF,
            unblocked_cb_idx: 0xFFFFFFFF,
        }
    }
}

/// AddElementEmitterMessage（16B，原版 L14712）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AddElementEmitterMsg {
    pub max_pressure: f32,
    pub callback_idx: i32,
    pub on_blocked_cb: i32,
    pub on_unblocked_cb: i32,
}

/// ModifyElementEmitterMessage（32B，原版 L13733）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ModifyElementEmitterMsg {
    pub handle: i32,
    pub cell_idx: i32,
    pub emit_interval: f32,
    pub emit_mass: f32,
    pub emit_temperature: f32,
    pub max_pressure: f32,
    pub disease_count: i32,
    pub element_idx: u16,
    pub max_depth: u8,
    pub disease_idx: u8,
}

/// RemoveElementEmitterMessage（8B，原版 L7452）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RemoveElementEmitterMsg {
    pub handle: i32,
    pub callback_idx: i32,
}

/// game→sim 坐标（原版 Modify L1656：`(gc/(w-2)+1)*w + gc%(w-2)+1`）。
fn game_to_sim_cell(game_cell: i32, width: i32) -> i32 {
    let gw = width - 2;
    (game_cell / gw + 1) * width + game_cell % gw + 1
}

/// ElementEmitter 管理器（模式同 ElementChunkManager：版本句柄 + free list + swap-remove）。
pub struct ElementEmitterManager {
    records: Vec<ElementEmitterData>,
    items: Vec<i32>,
    versions: Vec<u8>,
    free_handles: Vec<i32>,
}

impl ElementEmitterManager {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            items: Vec::new(),
            versions: Vec::new(),
            free_handles: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.items.clear();
        self.versions.clear();
        self.free_handles.clear();
    }

    pub fn record(&self, handle: i32) -> Option<&ElementEmitterData> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let ri = self.items[index];
        if ri < 0 {
            return None;
        }
        self.records.get(ri as usize)
    }

    fn record_mut(&mut self, handle: i32) -> Option<&mut ElementEmitterData> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let ri = self.items[index];
        if ri < 0 {
            return None;
        }
        self.records.get_mut(ri as usize)
    }

    pub fn active_count(&self) -> usize {
        self.items.iter().filter(|&&ri| ri >= 0).count()
    }

    /// Register（原版 L1679）：分配记录 + 返回 handle。
    pub fn add(&mut self, msg: &AddElementEmitterMsg) -> i32 {
        let record = ElementEmitterData {
            max_pressure: msg.max_pressure,
            blocked_cb_idx: msg.on_blocked_cb as u32,
            unblocked_cb_idx: msg.on_unblocked_cb as u32,
            ..ElementEmitterData::default()
        };
        let index;
        if let Some(free) = self.free_handles.pop() {
            index = free as usize;
            let ri = self.items[index];
            debug_assert!(ri < 0);
            let ri = self.records.len() as i32;
            self.items[index] = ri;
            self.records.push(record);
            self.versions[index] = self.versions[index].wrapping_add(1);
        } else {
            index = self.items.len();
            let ri = self.records.len() as i32;
            self.items.push(ri);
            self.records.push(record);
            self.versions.push(0);
        }
        ((self.versions[index] as i32) << 24) | (index as i32 & 0xffffff)
    }

    /// Modify（原版 L1610）：校验 + 写发射参数；emitMass<=0 → blockedState=0xFF（停用）。
    pub fn modify(&mut self, sim: &SimData, msg: &ModifyElementEmitterMsg) -> bool {
        let Some(rec) = self.record_mut(msg.handle) else {
            return false;
        };
        let sim_cell = game_to_sim_cell(msg.cell_idx, sim.width);
        rec.cell = [(sim_cell / sim.width) as u16, (sim_cell % sim.width) as u16];
        rec.emit_interval = msg.emit_interval;
        rec.emit_mass = msg.emit_mass;
        rec.emit_temperature = msg.emit_temperature;
        rec.max_pressure = msg.max_pressure;
        rec.emit_disease_count = msg.disease_count;
        rec.elem_idx = msg.element_idx;
        rec.max_depth = msg.max_depth;
        rec.disease_idx = msg.disease_idx;
        rec.elapsed_time = 0.0;
        if msg.emit_mass <= 0.0 {
            rec.blocked_state = 0xFF;
        }
        true
    }

    /// Remove（原版 ElementChunk::Unregister 同款 swap-remove + 版本递增）。
    pub fn remove(&mut self, handle: i32) -> bool {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return false;
        }
        let ri = self.items[index];
        if ri < 0 {
            return false;
        }
        self.versions[index] = self.versions[index].wrapping_add(1);
        self.items[index] = -1;
        self.free_handles.push(index as i32);
        // swap-remove 记录数组
        let last = self.records.len() - 1;
        if ri as usize != last {
            self.records.swap(ri as usize, last);
            // 修正被换位置记录的 items 指向
            for item in self.items.iter_mut() {
                if *item == last as i32 {
                    *item = ri;
                    break;
                }
            }
        }
        self.records.pop();
        true
    }
}

static ELEMENT_EMITTER_MANAGER: Lazy<Mutex<ElementEmitterManager>> =
    Lazy::new(|| Mutex::new(ElementEmitterManager::new()));

pub fn clear_element_emitters() {
    ELEMENT_EMITTER_MANAGER.lock().clear();
}
pub fn add_element_emitter(msg: &AddElementEmitterMsg) -> i32 {
    ELEMENT_EMITTER_MANAGER.lock().add(msg)
}
pub fn modify_element_emitter(sim: &SimData, msg: &ModifyElementEmitterMsg) -> bool {
    ELEMENT_EMITTER_MANAGER.lock().modify(sim, msg)
}
pub fn remove_element_emitter(handle: i32) -> bool {
    ELEMENT_EMITTER_MANAGER.lock().remove(handle)
}
pub fn element_emitter_count() -> usize {
    ELEMENT_EMITTER_MANAGER.lock().active_count()
}
#[cfg(test)]
pub fn reset_for_test() {
    ELEMENT_EMITTER_MANAGER.lock().clear();
}
```

### 步骤 2：sim_frame_manager 消息 emplace

把 L535-537 的 C2 桩 arm 拆开，三条 emitter 消息改为：
```rust
            MSG_ADD_ELEMENT_EMITTER => {
                emplace_bytes(&mut frame.element_emitter_messages.adds, reader, 16)
            }
            MSG_MODIFY_ELEMENT_EMITTER => {
                emplace_bytes(&mut frame.element_emitter_messages.modifies, reader, 32)
            }
            MSG_REMOVE_ELEMENT_EMITTER => {
                emplace_bytes(&mut frame.element_emitter_messages.removes, reader, 8)
            }
```
其余 C2 桩消息（disease/radiation emitter、disease consumer）留在原 fallback arm。

### 步骤 3：frame_processor 消息处理

在 `process_frame` 步骤 13 末尾（process_element_chunk_messages 之后）追加调用：
```rust
    // 原版 ProcessFrame：ElementConsumer removes → ElementEmitter adds→modifies→removes（L1628-1665）
    process_element_emitter_messages(frame, sim_data);
```

实现（仿 process_building_heat_exchange_messages）：
```rust
/// ElementEmitter 消息：adds→modifies→removes + componentStateChanged 回调（原版 L1628-1665）。
fn process_element_emitter_messages(frame: &mut SimFrameInfo, sim_data: &mut SimData) {
    if sim_data.sim_events.ptr.is_null() {
        frame.element_emitter_messages.adds.clear_keep_capacity();
        frame.element_emitter_messages.modifies.clear_keep_capacity();
        frame.element_emitter_messages.removes.clear_keep_capacity();
        return;
    }
    // adds
    {
        let adds = frame.element_emitter_messages.adds.as_slice();
        let n = adds.len() / 16;
        for i in 0..n {
            let msg = unsafe {
                std::ptr::read_unaligned(
                    adds.as_ptr().add(i * 16)
                        as *const crate::c_simulation::element_emitter::AddElementEmitterMsg,
                )
            };
            let handle = crate::c_simulation::element_emitter::add_element_emitter(&msg);
            if msg.callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events
                    .component_state_changed_messages
                    .push(ComponentStateChangedMessage {
                        callback_idx: msg.callback_idx,
                        sim_handle: handle,
                    });
            }
        }
        frame.element_emitter_messages.adds.clear_keep_capacity();
    }
    // modifies
    {
        let mods = frame.element_emitter_messages.modifies.as_slice();
        let n = mods.len() / 32;
        for i in 0..n {
            let msg = unsafe {
                std::ptr::read_unaligned(
                    mods.as_ptr().add(i * 32)
                        as *const crate::c_simulation::element_emitter::ModifyElementEmitterMsg,
                )
            };
            crate::c_simulation::element_emitter::modify_element_emitter(sim_data, &msg);
        }
        frame.element_emitter_messages.modifies.clear_keep_capacity();
    }
    // removes
    {
        let rems = frame.element_emitter_messages.removes.as_slice();
        let n = rems.len() / 8;
        for i in 0..n {
            let msg = unsafe {
                std::ptr::read_unaligned(
                    rems.as_ptr().add(i * 8)
                        as *const crate::c_simulation::element_emitter::RemoveElementEmitterMsg,
                )
            };
            crate::c_simulation::element_emitter::remove_element_emitter(msg.handle);
            if msg.callback_idx != -1 {
                let events = unsafe { &mut *sim_data.sim_events.ptr };
                events
                    .component_state_changed_messages
                    .push(ComponentStateChangedMessage {
                        callback_idx: msg.callback_idx,
                        sim_handle: -1,
                    });
            }
        }
        frame.element_emitter_messages.removes.clear_keep_capacity();
    }
}
```

### 步骤 4：编写失败测试（注册表 + 消息）

在 `element_emitter.rs` 测试模块：
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;

    #[test]
    fn add_returns_versioned_handle_and_modify_writes_fields() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        reset_for_test();
        let msg = AddElementEmitterMsg {
            max_pressure: 2.0,
            callback_idx: 7,
            on_blocked_cb: 8,
            on_unblocked_cb: 9,
        };
        let h = add_element_emitter(&msg);
        assert_eq!(h & 0xffffff, 0, "首个 handle index 0");
        let sim = SimData::new_for_allocate(6, 6, 1, false, false);
        let m = ModifyElementEmitterMsg {
            handle: h,
            cell_idx: 0,
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: 350.0,
            max_pressure: 2.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 2,
            disease_idx: 0xFF,
        };
        assert!(modify_element_emitter(&sim, &m));
        {
            let mgr = ELEMENT_EMITTER_MANAGER.lock();
            let rec = mgr.record(h).unwrap();
            assert_eq!(rec.emit_interval, 0.2);
            assert_eq!(rec.emit_mass, 0.002);
            assert_eq!(rec.elem_idx, 3);
            assert_eq!(rec.max_depth, 2);
            // gameCell 0 → sim cell (0/(4)+1)*6 + 0%4 + 1 = 7 → row 1 col 1
            assert_eq!(rec.cell, [1, 1]);
        }
    }

    #[test]
    fn modify_with_zero_mass_sets_blocked_state_ff() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        reset_for_test();
        let h = add_element_emitter(&AddElementEmitterMsg::default());
        let sim = SimData::new_for_allocate(6, 6, 1, false, false);
        let m = ModifyElementEmitterMsg {
            handle: h,
            emit_mass: 0.0,
            ..ModifyElementEmitterMsg::default()
        };
        assert!(modify_element_emitter(&sim, &m));
        let mgr = ELEMENT_EMITTER_MANAGER.lock();
        assert_eq!(mgr.record(h).unwrap().blocked_state, 0xFF);
    }

    #[test]
    fn remove_frees_and_versions_bump() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        reset_for_test();
        let h = add_element_emitter(&AddElementEmitterMsg::default());
        assert!(remove_element_emitter(h));
        assert!(!remove_element_emitter(h), "重复移除应失败");
        let h2 = add_element_emitter(&AddElementEmitterMsg::default());
        assert_ne!(h2, h, "版本应递增");
        assert_eq!(h2 & 0xffffff, h & 0xffffff, "index 复用");
    }
}
```

### 步骤 5：运行测试验证通过

运行：`cargo test -p simdll_rs c_simulation::element_emitter -- --test-threads=1 2>$null; $LASTEXITCODE`
预期：3 条测试 PASS；全量回归 PASS。

### 步骤 6：提交

```bash
git add simDLL_RS/src/c_simulation/element_emitter.rs simDLL_RS/src/c_simulation/mod.rs simDLL_RS/src/a_framework/sim_frame_manager.rs simDLL_RS/src/c_simulation/frame_processor.rs
git commit -m "feat(element_emitter): registry + messages + callbacks (Register/Modify/Remove)"
```

## 任务 4：ElementEmitter Update/TryEmit/Emit + 挂载 + 输出

**文件：**
- 修改：`simDLL_RS/src/c_simulation/element_emitter.rs`（Update/TryEmit/Emit/BFS + 测试）
- 修改：`simDLL_RS/src/c2_physics/mod.rs`（update_data 挂载）

### 步骤 1：实现 BFS + Update + TryEmit + Emit

在 `element_emitter.rs` 追加：

```rust
/// GetReachableCells BFS（原版 L34451 + Flood L33466，param_5=true）：
/// 从 (row,col) 出发，maxDepth 内穿越**非固体**格，收集所有可达格。
fn get_reachable_cells(
    sim: &SimData,
    row: usize,
    col: usize,
    max_depth: usize,
    width: usize,
) -> Vec<usize> {
    let mut out = Vec::new();
    if max_depth == 0 {
        return out;
    }
    let total = width * sim.height as usize;
    let start = row * width + col;
    if start >= total {
        return out;
    }
    let mut visited = vec![false; total];
    let mut queue = std::collections::VecDeque::new();
    visited[start] = true;
    queue.push_back((start, 0usize));
    let is_solid = |c: usize| -> bool {
        let e = unsafe { (*sim.updated_cells.ptr).element_idx.get(c) };
        matches!(
            crate::b_elements::elements_table::get_element_post_process_data(e),
            Some(p) if (p.state & 3) == 3
        )
    };
    while let Some((c, depth)) = queue.pop_front() {
        if is_solid(c) {
            continue; // 原版 Flood param_5=true：固体格不收集不穿越
        }
        out.push(c);
        if depth + 1 >= max_depth {
            continue;
        }
        for nb in [
            c.wrapping_sub(width),
            c + 1,
            c.wrapping_sub(1),
            c + width,
        ] {
            if nb >= total || visited[nb] {
                continue;
            }
            visited[nb] = true;
            queue.push_back((nb, depth + 1));
        }
    }
    out
}

/// Emit（原版 L1495-1609）：目标格置为发射元素 + 温度质量合并 + 病菌 + 事件 + timers。
fn emit_into_cell(
    sim: &mut SimData,
    cell: usize,
    elem_idx: u16,
    mass: f32,
    temperature: f32,
    disease_idx: u8,
    disease_count: i32,
) {
    let updated = unsafe { &*sim.updated_cells.ptr };
    let cur = updated.element_idx.get(cell);
    if cur != elem_idx && cur != sim.vacuum_element_idx {
        return; // 不应发生（TryEmit 已 displace）；原版 DebugBreak
    }
    let was_vacuum = cur == sim.vacuum_element_idx;
    {
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        updated.element_idx.set(cell, elem_idx);
        crate::c2_physics::liquid_flow::add_mass_and_update_temperature(
            updated,
            cell,
            mass,
            temperature,
            disease_idx,
            disease_count,
        );
    }
    if was_vacuum {
        crate::c2_physics::liquid_flow::push_substance_change(sim, cell);
        // timers[cell] |= 0x1F（原版 L1557；timers 为 1B/格数组）
        if !sim.timers.ptr.is_null() {
            unsafe {
                let t = sim.timers.ptr.add(cell);
                *t = *t | 0x1F;
            }
        }
    }
}

impl ElementEmitterManager {
    /// Update（原版 L1966-2104）。
    pub fn update(&mut self, sim: &mut SimData, dt: f32, width: i32) {
        if sim.updated_cells.ptr.is_null() || sim.sim_events.ptr.is_null() {
            return;
        }
        // 输出按 handle index resize（原版 Update 首行 _Resize）
        let n_items = self.items.len();
        {
            let out = &mut sim.element_emitter.emitted_mass_info;
            out.resize(n_items, EmittedMassInfo::default());
            for (idx, &ri) in self.items.iter().enumerate() {
                if ri < 0 {
                    continue;
                }
                out.set(idx, EmittedMassInfo::default());
            }
        }
        let w = width as usize;
        for (idx, &ri) in self.items.iter().enumerate() {
            if ri < 0 {
                continue;
            }
            let rec = &mut self.records[ri as usize];
            rec.elapsed_time += dt;
            if rec.elapsed_time < rec.emit_interval {
                continue;
            }
            let cell = rec.cell[1] as usize * w + rec.cell[0] as usize;
            let cell_elem = {
                let u = unsafe { &*sim.updated_cells.ptr };
                u.element_idx.get(cell)
            };
            // 发射器格元素 ∈ {0xFFFF, vacuum, elemIdx} 才尝试（原版 L2020-2023）
            if cell_elem == 0xFFFF
                || cell_elem == sim.vacuum_element_idx
                || cell_elem == rec.elem_idx
            {
                let reachable =
                    get_reachable_cells(sim, rec.cell[1] as usize, rec.cell[0] as usize, rec.max_depth as usize, w);
                // 堵/通判定（任一可达格 mass < maxPressure → 有空间）
                let has_room = {
                    let u = unsafe { &*sim.updated_cells.ptr };
                    reachable
                        .iter()
                        .any(|&c| u.mass.get(c) < rec.max_pressure && rec.max_pressure != u.mass.get(c))
                };
                if has_room {
                    if rec.blocked_state != 0 {
                        let cb = rec.unblocked_cb_idx;
                        rec.blocked_state = 0;
                        if cb != 0xFFFFFFFF {
                            let events = unsafe { &mut *sim.sim_events.ptr };
                            events.component_state_changed_messages.push(
                                ComponentStateChangedMessage {
                                    callback_idx: cb as i32,
                                    sim_handle: -1,
                                },
                            );
                        }
                    }
                } else if rec.blocked_state != 1 {
                    let cb = rec.blocked_cb_idx;
                    rec.blocked_state = 1;
                    if cb != 0xFFFFFFFF {
                        let events = unsafe { &mut *sim.sim_events.ptr };
                        events.component_state_changed_messages.push(
                            ComponentStateChangedMessage {
                                callback_idx: cb as i32,
                                sim_handle: -1,
                            },
                        );
                    }
                }
                // TryEmit
                let emitted = self.try_emit(sim, rec, &reachable);
                if emitted.mass > 0.0 {
                    let out = &mut sim.element_emitter.emitted_mass_info;
                    let cur = out.get(idx);
                    let new_temp = crate::c2_physics::temperature::calculate_temperature_exchange_precise(
                        1.0, 1.0, 0.0, cur.temperature, cur.mass, emitted.temperature, emitted.mass,
                    );
                    // 用 FastCalculateCombinedTemperature 语义（质量加权，含钳制）
                    let combined = crate::c2_physics::liquid_flow::fast_combined_temperature(
                        cur.mass,
                        cur.temperature,
                        emitted.mass,
                        emitted.temperature,
                    );
                    out.set(
                        idx,
                        EmittedMassInfo {
                            elem_idx: emitted.elem_idx,
                            disease_idx: 0xFF,
                            pad: 0,
                            mass: cur.mass + emitted.mass,
                            temperature: combined,
                            disease_count: 0,
                        },
                    );
                }
            }
            rec.elapsed_time -= rec.emit_interval;
        }
    }

    /// TryEmit（原版 L1792-1965）：轮转可达格，气/液入格、固体推回调。
    fn try_emit(
        &mut self,
        sim: &mut SimData,
        rec: &mut ElementEmitterData,
        reachable: &[usize],
    ) -> EmittedMassInfo {
        let mut result = EmittedMassInfo::default();
        if reachable.is_empty() {
            return result;
        }
        let count = reachable.len();
        let start = rec.offset_idx as usize % count;
        let emit_state = match crate::b_elements::elements_table::get_element_by_idx(rec.elem_idx) {
            Some(e) => e.state & 3,
            None => return result,
        };
        for k in 0..count {
            let c = reachable[(start + k) % count];
            let (mass_c, cur_elem) = {
                let u = unsafe { &*sim.updated_cells.ptr };
                (u.mass.get(c), u.element_idx.get(c))
            };
            if !(mass_c < rec.max_pressure && rec.max_pressure != mass_c) {
                continue; // 压力门
            }
            if emit_state == 0 {
                break;
            }
            match emit_state {
                1 => {
                    if cur_elem != rec.elem_idx && cur_elem != sim.vacuum_element_idx {
                        if !crate::c2_physics::liquid_flow::displace_gas(sim, c, cur_elem) {
                            continue;
                        }
                    }
                }
                2 => {
                    if cur_elem != rec.elem_idx && cur_elem != sim.vacuum_element_idx {
                        if !crate::c2_physics::liquid_flow::displace_liquid(sim, c, cur_elem) {
                            let cur2 = unsafe { (*sim.updated_cells.ptr).element_idx.get(c) };
                            if cur2 != sim.vacuum_element_idx {
                                if !crate::c2_physics::liquid_flow::displace_gas(sim, c, cur2) {
                                    continue;
                                }
                            }
                        }
                    }
                }
                3 => {
                    // 固体：推 MassConsumedCallback（含可见性检查，原版 L1816-1844）
                    if rec.emit_mass > 0.0 {
                        let width = sim.width as usize;
                        let game_cell = ((c / width) as i32 - 1) * (sim.width - 2)
                            + (c % width) as i32
                            - 1;
                        if game_cell >= 0 && game_cell < sim.num_game_cells {
                            let visible = unsafe { std::ptr::read(sim.visible_grid.ptr.add(game_cell as usize)) };
                            if sim.debug_properties.is_debug_editing || visible != 0 {
                                let events = unsafe { &mut *sim.sim_events.ptr };
                                events.mass_consumed_callbacks.push(
                                    crate::a_framework::game_data::MassConsumedCallback {
                                        callback_idx: game_cell,
                                        elem_idx: rec.elem_idx,
                                        disease_idx: 0xFF,
                                        pad: 0,
                                        mass: rec.emit_mass,
                                        temperature: rec.emit_temperature,
                                        disease_count: 0,
                                    },
                                );
                            }
                        }
                    }
                    continue; // 原版固体分支 goto 下一候选
                }
                _ => continue,
            }
            // Emit 入格
            emit_into_cell(
                sim,
                c,
                rec.elem_idx,
                rec.emit_mass,
                rec.emit_temperature,
                rec.disease_idx,
                rec.emit_disease_count,
            );
            result.elem_idx = rec.elem_idx;
            result.mass = rec.emit_mass;
            result.temperature = rec.emit_temperature;
            result.disease_idx = 0xFF;
            result.disease_count = 0;
            break;
        }
        rec.offset_idx = rec.offset_idx.wrapping_add(1);
        result
    }
}

/// 顶层 Update（update_data 组件阶段调用）。
pub fn update_element_emitters(sim: &mut SimData, dt: f32, width: i32) {
    ELEMENT_EMITTER_MANAGER.lock().update(sim, dt, width);
}
```

注：`fast_combined_temperature` 在 liquid_flow.rs 是私有——需提升 `pub(crate)`；
`add_mass_and_update_temperature` 亦需 `pub(crate)`（当前私有）；`MassConsumedCallback`
字段名以 game_data.rs 实际为准（callback_idx/elem_idx/disease_idx/pad/mass/temperature/
disease_count）；`sim.timers.ptr.add(cell)` 为 1B 数组访问（timers_size_is_1）。
emitted_mass_info 的 `MsvcVector::get/set/resize` 以 stl_shim 实际 API 为准。

### 步骤 2：挂载 update_data

`c2_physics/mod.rs` update_data 组件阶段（element_chunk 之后）追加：
```rust
    crate::c_simulation::element_emitter::update_element_emitters(sim_data, 0.2, sim_data.width);
```

### 步骤 3：编写失败测试（Update/TryEmit）

在 `element_emitter.rs` 测试模块：
```rust
    #[test]
    fn update_emits_after_interval_and_accumulates_output() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        reset_for_test();
        let h = add_element_emitter(&AddElementEmitterMsg {
            max_pressure: 10.0,
            ..AddElementEmitterMsg::default()
        });
        let mut sim = SimData::new_for_allocate(6, 6, 1, false, false);
        sim.vacuum_element_idx = 0;
        sim.void_element_idx = 0xFFFF;
        sim.unobtanium_element_idx = 0xFFFF;
        // 发射器格 sim (1,1)=7（真空）；maxDepth 2 → 可达 5 格非固体
        let m = ModifyElementEmitterMsg {
            handle: h,
            cell_idx: 0,
            emit_interval: 0.2,
            emit_mass: 0.002,
            emit_temperature: 350.0,
            max_pressure: 10.0,
            disease_count: 0,
            element_idx: 3,
            max_depth: 2,
            disease_idx: 0xFF,
        };
        assert!(modify_element_emitter(&sim, &m));
        // 元素表：3 = 气体（需建表，见注）
        crate::b_elements::elements_table::... // 建 3 元素表（0 真空/1 液体/2 固体/3 气体）
        update_element_emitters(&mut sim, 0.2, 6);
        let out = &sim.element_emitter.emitted_mass_info;
        assert!(out.get(0).mass > 0.0, "应累积发射量");
        assert_eq!(out.get(0).elem_idx, 3);
    }

    #[test]
    fn update_fires_blocked_callback_when_no_room() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        reset_for_test();
        let h = add_element_emitter(&AddElementEmitterMsg {
            max_pressure: 1.0,
            on_blocked_cb: 55,
            ..AddElementEmitterMsg::default()
        });
        let mut sim = SimData::new_for_allocate(6, 6, 1, false, false);
        sim.vacuum_element_idx = 0;
        // 发射器格与可达格全部质量 >= maxPressure(1.0) → 堵住
        // （可达格填满 1.0+ 液体/气体）
        ...
        update_element_emitters(&mut sim, 0.2, 6);
        let events = unsafe { &*sim.sim_events.ptr };
        let cb = events
            .component_state_changed_messages
            .as_slice()
            .iter()
            .find(|m| m.callback_idx == 55);
        assert!(cb.is_some(), "应推 blocked 回调");
    }
```

注：测试需先建元素表（仿 liquid_flow `create_state_table`/`create_sublimate_table`
的 CreateElementsTable 序列化建表，或直接操作 G_ELEMENTS_TABLE 填充 elements/
post_process_data/state_data），并让发射器格为真空（element 0）、可达格含真空。

### 步骤 4：运行测试验证通过 + 全量回归

运行：`cargo test -p simdll_rs c_simulation::element_emitter -- --test-threads=1 2>$null; $LASTEXITCODE`
→ PASS；`cargo test -p simdll_rs -- --test-threads=1 2>$null; $LASTEXITCODE` → 全 PASS；
`cargo build --release -p simdll_rs 2>$null; $LASTEXITCODE` → 0。

### 步骤 5：提交

```bash
git add simDLL_RS/src/c_simulation/element_emitter.rs simDLL_RS/src/c2_physics/mod.rs
git commit -m "feat(element_emitter): Update/TryEmit/Emit + emitted mass output (L1495-2104)"
```

## 任务 5：代办记录 + 实测清单

### 步骤 1：追加代办

在 `代办.txt` 追加：
```markdown
## [待实测] ElementEmitter + 火箭热 + AddSolid 垂直位移（2026-08-03）

- 规格：docs/superpowers/specs/2026-08-03-emitter-energy-addsolid-design.md（80cc55a）
- 计划：docs/superpowers/plans/2026-08-03-emitter-energy-addsolid.md
- A ElementEmitter：注册表（44B）+ Add/Modify/Remove（16/32/8B）+ Update（计时/BFS/堵通回调/
  TryEmit 气液固三态）+ emitted_mass_info 输出（copy 交换已就绪）。
- B ProcessCellEnergyModifications（06_process_messages.c L134-211）：ModifyEnergy 落地
  （火箭 3×9 热、Burner 建筑通用）。
- C add_solid 重写：Type 0 DoVerticalDisplacement（同列 4 格补满 + 向下落格，流星专属）/
  Type 1 OnlyIfSameElement；frame_processor 传 entry.flags；删除四邻混合版。
- 待实测：① 化肥机持续出天然气 + Emitting 状态 + 压力满 blocked；② 火箭发射/降落
  地面 3×9 升温；③ 流星撞击生成固体方块（含落点行为）；④ 反应堆融毁核废料流星 +
  3000°C 核废液。
```

### 步骤 2：提交

```bash
git add 代办.txt
git commit -m "docs: record emitter + cell energy + add_solid batch"
```

## 自检

**规格覆盖度**：A→任务 3/4；B→任务 2；C→任务 1；实测清单→任务 5。全覆盖。

**占位符扫描**：无 TODO；每步含完整代码或精确 diff；测试代码完整（测试表辅助的建表
细节按现有测试模式补齐）。

**类型一致性**：`add_solid(sim, cell, elem, mass, temp, didx, dcount, add_sub_type)` 在任务 1
定义并被 frame_processor 使用；`ModifyElementEmitterMsg` 字段与 C# 线序一致
（handle/cellIdx/emitInterval/emitMass/emitTemperature/maxPressure/diseaseCount/elementIdx/
maxDepth/diseaseIdx）；`EmittedMassInfo` 字段与 game_data.rs 一致；`fast_combined_temperature`
与 `add_mass_and_update_temperature` 提升 `pub(crate)` 后供 element_emitter 使用。
