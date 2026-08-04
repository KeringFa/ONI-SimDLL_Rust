# 辐射模块·阶段 A 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现本计划（内联执行，
> 本项目用户偏好）。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 实现格子级辐射核心——元素辐射（25 格权重核扩散）+ 宇宙辐射（occlusion 自顶向下）+ 辐射参数
消息落地 + 衰减/钳制 + CellRadiationModification 消息与消费回调，严格以 `radiation_enabled` 门控
（DLC 专属，单星零执行）。

**架构：** 新增 `c2_physics/radiation.rs`（run_radiation_task + 两条消息处理），接入 update_data
（液体/太空真空/PostProcess 之后、组件阶段之前，包在 radiation_enabled 内）；修正 frame_manager
的 RadiationParamsModification 解析尺寸（12B→8B）；复用既有 SimData 辐射字段/参数/occlusion 缓冲
与 sim_events.radiation_consumed_callbacks。

**技术栈：** Rust（cargo test 串行 `--test-threads=1`，项目已知 flaky 对策）；对照
11_msvcrt_ignored.c L42594-42914、06_process_messages.c L355-414。

---

### 任务 1：RadiationParamsModification 8B 修正 + 参数落地

**文件：**
- 修改：`src/a_framework/sim_frame_manager.rs`（MSG_RADIATION_PARAMS_MOD emplace 12→8）
- 修改：`src/c_simulation/frame_processor.rs`（新增 process_radiation_params 真实实现，替换桩）
- 测试：`src/c_simulation/frame_processor.rs` tests 模块

- [ ] **步骤 1：写失败测试**（params 8B 解析 + type 映射 + 未映射 type 跳过）

```rust
#[test]
fn radiation_params_applies_mapping_and_skips_unknown() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
    // 构造 3 条 8B 条目：type 0 (linger=2.0)、type 3 (density=0.9)、type 1 (应跳过)
    let mut w = BinaryBufferWriter::new();
    w.write_int(0); w.write_float(2.0);
    w.write_int(3); w.write_float(0.9);
    w.write_int(1); w.write_float(123.0);
    let data = w.into_bytes();
    process_radiation_params(&mut sd, &data);
    assert!((sd.radiation_linger_rate - 2.0).abs() < 1e-6);
    assert!((sd.radiation_density_weight - 0.9).abs() < 1e-6);
    assert!((sd.radiation_base_weight - 0.3).abs() < 1e-6, "type1 应跳过");
}
```

- [ ] **步骤 2：运行测试确认失败**（process_radiation_params 未定义）
- [ ] **步骤 3：实现**
  - frame_manager：`emplace_bytes(&mut frame.radiation_params_modifications, reader, 12)` → `8`。
  - frame_processor：实现 `process_radiation_params(sim: &mut SimData, entries: &[u8])`——按 8B
    迭代原始字节，`type` 0→linger_rate、2→base_weight、3→density_weight、4→constructed_factor、
    5→max_mass，其余跳过；`process_radiation_params_frame` 从
    `frame.radiation_params_modifications` 取 slice 调用后清空。
- [ ] **步骤 4：运行测试确认通过**
- [ ] **步骤 5：commit**（`feat(c2): radiation params 8B message + parameter mapping`）

### 任务 2：CellRadiationModification 处理 + 消费回调

**文件：**
- 修改：`src/c_simulation/frame_processor.rs`（process_cell_radiation_changes 真实实现，替换桩）
- 测试：`src/c_simulation/frame_processor.rs` tests 模块

- [ ] **步骤 1：写失败测试**（delta 累加 + ≤0 清零 + callback 推入）

```rust
#[test]
fn cell_radiation_modification_applies_delta_and_callback() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
    // gameCell=4 → sim cell = (4/4+1)*6 + (4%4+1) = 2*6+1 = 13（sim 行 2 列 1）
    unsafe { (*sd.updated_cells.ptr).radiation.set(13, 5.0); }
    // 条目：gameCell=4、delta=+2.5、callbackIdx=7
    let mut w = BinaryBufferWriter::new();
    w.write_int(4);
    w.write_float(2.5);
    w.write_int(7);
    let data = w.into_bytes();
    process_cell_radiation_changes(&mut sd, &data);
    unsafe {
        let r = (*sd.updated_cells.ptr).radiation.get(13);
        assert!((r - 7.5).abs() < 1e-6);
    }
    let events = &*sd.sim_events.ptr;
    assert_eq!(events.radiation_consumed_callbacks.len(), 1);
}
```

- [ ] **步骤 2：运行测试确认失败**
- [ ] **步骤 3：实现**——`process_cell_radiation_changes(sim: &mut SimData, entries: &[u8])` 按 12B 迭代：
  `gameCell→sim cell`（`(gc % gameW + 1) + (gc / gameW + 1) × simW`）；
  `radiation += delta`，结果 ≤0 → 0；`callbackIdx != -1` → 推
  `RadiationConsumedCallback { gameCell, callbackIdx, radiation: delta }` 到
  `sim_events.radiation_consumed_callbacks`；frame 版本取 slice 调用后清空。
- [ ] **步骤 4：运行测试确认通过**
- [ ] **步骤 5：commit**（`feat(c2): cell radiation modification + consumption callback`）

### 任务 3：run_radiation_task（宇宙 occlusion + 衰减 + 元素扩散 + cosmic 应用 + 钳制）

**文件：**
- 创建：`src/c2_physics/radiation.rs`
- 修改：`src/c2_physics/mod.rs`（pub mod radiation；update_data 接线 + 区域 cosmic 强度传入）
- 修改：`src/c2_physics/mod.rs` tests（或 radiation.rs 内 tests）

- [ ] **步骤 1：写失败测试**（单星门控 + 宇宙 occlusion + 元素扩散 + 衰减/钳制）

```rust
// 门控：radiation_enabled=false → 零执行（cells.radiation 不变）
#[test]
fn radiation_task_gated_by_enabled_flag() { ... }

// 元素扩散：cell 中心放射性元素（radsPer1000=1000, mass=10）→ 中心格 +10×0.001×1000×1.0=10
#[test]
fn radiation_task_spreads_element_kernel() { ... }

// 宇宙 occlusion：顶行真空 → occlusion=1；下方固体（factor=0.7, mass 大）→ 衰减
#[test]
fn radiation_task_cosmic_occlusion_and_apply() { ... }

// 衰减 + 钳制：[0, 9e6]，≤0.01 → 0
#[test]
fn radiation_task_decay_and_clamp() { ... }
```

- [ ] **步骤 2：运行测试确认失败**（radiation.rs 不存在）
- [ ] **步骤 3：实现 `run_radiation_task(sim, bounds, cosmic_intensity)`**
  - `if !sim.radiation_enabled { return; }`；
  - `cosmic_radiation_occlusion` resize 区域大小并填 1.0；
  - **第一遍（top→bottom，row 从 max_y 递减）**：
    - block：`updated.properties[cell] & 0x80` → `factor × constructedFactor`；
      否则 `(mass/maxMass) × factor × densityWeight + factor × baseWeight`；
      factor = `gElementRadiationData[cell].factor`；clamp [0,1]；
    - `occlusion[cell] = (1 - block) × occlusion[cell + width]`（上方格，越界视为 1.0）；
    - 衰减：`r = radiation[cell]; f = r / linger; if f == 0 { r -= 1.0 } else { r -= f }`，clamp ≥0；
  - **第二遍（全区域）**：
    - 元素扩散：`radsPer1000[cell] > 0` → 25 核
      `[(-2,-2,0.1),(-1,-2,0.15),(0,-2,0.25),(1,-2,0.15),(2,-2,0.1), ... ]`（中心 (0,0,1.0)）：
      目标 = cell + dy×width + dx，边界内 → `radiation[target] += mass[cell] × 0.001 × radsPer1000 × w`；
    - cosmic 应用：`occlusion[cell] > 0` → `radiation += (cosmic/linger) × occlusion`；
    - 钳制：clamp [0, 9e6]；≤0.01 → 0。
- [ ] **步骤 4：接线 update_data**——`region_bounds` 循环改为带 cosmic 强度：
  构建 `let region_cosmic: Vec<f32> = regions.iter().map(|r| r.current_cosmic_radiation_intensity).collect();`
  与 region_bounds 同步；在 `run_space_vacuum_task` 之后、`post_process_loop` 之前
  （原版 L42594 位于 UpdateComponents 前）调用 `run_radiation_task(sim, bounds, cosmic)`。
- [ ] **步骤 5：运行测试确认通过**
- [ ] **步骤 6：commit**（`feat(c2): radiation core - cosmic occlusion, element kernel, decay/clamp`）

### 任务 4：全量回归 + 双模式验证

**文件：** 无新增

- [ ] **步骤 1：全量串行测试**（`cargo test -- --test-threads=1`，预期全部通过）
- [ ] **步骤 2：release 构建**
- [ ] **步骤 3：commit**（`test(c2): radiation phase A full regression`）

---

## 自检记录

- 规格覆盖：消息（params 8B / cell 12B+callback）、occlusion、衰减、25 核扩散、cosmic 应用、钳制、
  DLC 门控——任务 1-3 全覆盖；病菌贡献明确跳过（依赖病菌表，规格记录）。
- 类型一致：`run_radiation_task`、`process_radiation_params`、`process_cell_radiation_changes_frame`
  在任务间保持一致命名；SimData 字段名与现有代码一致。
- 方向约定：occlusion 用 `cell + width`（上方格），与阳光/液体约定一致（更大索引=更高行）。
