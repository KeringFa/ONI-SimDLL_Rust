# 辐射模块·阶段 B（RadiationEmitter 组件）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现本计划
> （内联执行，本项目用户偏好）。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 实现 RadiationEmitter 组件（Constant/Pulsing/PulsingAveraged 三种发射类型），
覆盖生物辐射（发光虫/冰息萝卜/蜜蜂）与建筑辐射（蜂巢/人力粒子发生器/核反应堆含熔毁态）。

**架构：** 新增 `c_simulation/radiation_emitter.rs`（注册表 + Update/tick 逻辑，仿
element_emitter.rs 组件模式）；frame_manager 从"接受不 emplace"改为真实入帧
（Add 36B / Modify 40B / Remove 8B）；frame_processor 拆出
`process_radiation_emitter_messages`；update_data 在 `run_radiation_task` 后调用
`update_radiation_emitters`（dt=0.2，辐射组件阶段）。

**技术栈：** Rust（cargo test 串行 `--test-threads=1`）；对照 11_msvcrt_ignored.c
L39374/39168/39693/40172/40263 + inRadialRange(L40033)/RadiationAbsorptionAlongLine(L39249)。

---

### 任务 1：消息入帧 + 注册表（44B 记录 + 版本句柄）

**文件：**
- 修改：`src/a_framework/sim_frame_manager.rs`（radiation emitter 消息真实 emplace）
- 创建：`src/c_simulation/radiation_emitter.rs`（RadiationEmitterData + Registry）
- 测试：`src/c_simulation/radiation_emitter.rs` tests

- [ ] **步骤 1：写失败测试**（消息尺寸 + 注册/修改/释放 + 句柄版本）

```rust
#[test]
fn radiation_emitter_register_modify_remove_and_handle_versions() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    let mut reg = RadiationEmitterRegistry::new();
    // Add：callbackIdx=3, cell=14, radiusX=3, radiusY=2, emitRads=60, rate=0, speed=1,
    //       dir=90, angle=360, type=Constant
    let mut w = BinaryBufferWriter::new();
    w.write_int(3); w.write_int(14);
    w.write_short(3); w.write_short(2);
    w.write_float(60.0); w.write_float(0.0); w.write_float(1.0);
    w.write_float(90.0); w.write_float(360.0); w.write_int(0);
    let h = reg.register(&w.into_bytes(), 6, 8, &mut vec![]);
    assert!(h != -1);
    let rec = reg.record(h).expect("handle 有效");
    assert_eq!(rec.cell, 14);
    assert_eq!(rec.emit_radius_x, 3);
    assert!((rec.emit_rads - 60.0).abs() < 1e-6);
    // Modify：radiusX→5
    let mut w2 = BinaryBufferWriter::new();
    w2.write_int(h); w2.write_int(14); w2.write_int(3);
    w2.write_short(5); w2.write_short(2);
    w2.write_float(60.0); w2.write_float(0.0); w2.write_float(1.0);
    w2.write_float(90.0); w2.write_float(360.0); w2.write_int(0);
    assert!(reg.modify(h, &w2.into_bytes(), 6, 8, &mut vec![]));
    assert_eq!(reg.record(h).unwrap().emit_radius_x, 5);
    // Remove → 记录失效；回调推 -1
    let mut cbs = vec![];
    assert!(reg.remove(h, &mut cbs));
    assert!(reg.record(h).is_none());
}
```

- [ ] **步骤 2：运行测试确认失败**（模块/函数未定义）
- [ ] **步骤 3：实现**
  - frame_manager：`MSG_ADD_RADIATION_EMITTER` → emplace 36B；`MSG_MODIFY_RADIATION_EMITTER`
    → 40B；`MSG_REMOVE_RADIATION_EMITTER` → 8B（参照 building_heat_exchange 的 ComponentMessages
    入帧模式）。
  - radiation_emitter.rs：
    - `RadiationEmitterData`（44B）：cell(i32)/emit_radius_x(u16)/emit_radius_y(u16)/emit_rads/
      emit_rate/emit_speed/emit_direction/emit_angle/emit_timer/emit_step_timer/emit_type/emit_step；
    - `RadiationEmitterRegistry`：versions Vec<u8> + free_handles Vec<i32> + data Vec 选项；
      `handle = version<<24 | index`；register 解析 36B（含 game→sim cell 换算：
      `(gc%gameW+1) + (gc/gameW+1)×simW`）→ 分配句柄 → callbackIdx≠-1 时回调推句柄；
      modify 40B 更新字段；remove 8B → 释放 + 回调推 -1；延迟释放两轮
      （Remove 入缓冲，`release_queued_handles` 两轮后 swap-remove）。
- [ ] **步骤 4：运行测试确认通过**
- [ ] **步骤 5：commit**（`feat(c2): radiation emitter registry - 44B records, version handles (task 1/4)`）

### 任务 2：Update 时序 + tickConstant + 辅助函数

**文件：** 修改 `src/c_simulation/radiation_emitter.rs`

- [ ] **步骤 1：写失败测试**（Constant 椭圆 falloff + 扇形 + 吸收沿直线 + 抖动）

```rust
#[test]
fn radiation_emitter_constant_ticks_ellipse() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    init_table(); // 元素：0=真空(factor0), 1=固体(factor0.5), 2=辐射元素
    let mut sd = make_sim(); // radiation_enabled=true, linger=1.1
    // 注册 Constant 发射器：cell14（行2列4），radiusX=2, radiusY=2, emitRads=110,
    // direction=0, angle=360（全向），rate=0（恒发），speed=1
    let h = register_emitter(&mut sd, 14, 2, 2, 110.0, 0.0, 1.0, 0.0, 360.0, 0);
    // 目标 cell 与发射器之间为真空 → 线吸收=1.0；
    // falloff(中心 1.0) × emitRads/linger = 110/1.1 = 100
    let bounds = full_grid_bounds(&sd);
    update_radiation_emitters(&mut sd, 1.0, bounds);
    unsafe {
        let r = (*sd.updated_cells.ptr).radiation.get(14);
        assert!((r - 100.0).abs() < 1e-3, "中心格 100");
    }
}
```

- [ ] **步骤 2：运行测试确认失败**
- [ ] **步骤 3：实现**
  - `update_radiation_emitters(sim, dt, bounds)`：门控 `radiationEnabled`；遍历区域内的
    发射器记录；`emitTimer += dt`；`emitRate==0` 恒发、否则 `emitTimer>=emitRate` 才发；
    `emitSpeed` 步进（超时重置 timer/step）；每 tick 步数 `n = dt/(emitSpeed/max(rx,ry))`
    clamp ≥1；分发 tickConstant / tickPulsing。
  - `in_radial_range(tx, ty, ex, ey, angle, direction)`（L40033）：angle==360 或同格 → true；
    目标角 atan2 与 `[direction−angle/2, direction+angle/2]` 扇形（跨 0 处理）。
  - `radiation_absorption_along_line(sim, ex, ey, tx, ty)`（L39249）：Bresenham 线走查
    发射器→目标逐格，按元素辐射遮挡 `(mass/maxMass × factor × densityWeight +
    factor × baseWeight)`（构造格 `factor × constructedFactor`）累积透射率 [0,1]。
  - `tick_constant`（L40172）：椭圆区域逐格 → 边界 + 扇形过滤 → 椭圆内
    `falloff = 1 − |dx|/rx − (|dy| − |dx·dy|/rx)/ry` →
    `radiation[cell] += line_absorption × (falloff × emitRads / lingerRate)`；
    `falloff < 0.25` 加 `random×(±12.5%)` 抖动（sim.random_seed）。
- [ ] **步骤 4：运行测试确认通过**
- [ ] **步骤 5：commit**（`feat(c2): radiation emitter constant tick + radial/line helpers (task 2/4)`）

### 任务 3：tickPulsing + PulsingAveraged

**文件：** 修改 `src/c_simulation/radiation_emitter.rs`

- [ ] **步骤 1：写失败测试**（Pulsing 环扩散 + Averaged 均摊 + emitStep 递增）
- [ ] **步骤 2：运行测试确认失败**
- [ ] **步骤 3：实现 `tick_pulsing`（L40263）**
  - 当前环半径 `w = max(1, round(rx × (emitStep+1)/max(rx,ry)))`、`h` 同理；
  - PulsingAveraged：`emitRads / ((w−1)×(h−1))` 均摊；
  - `emitStep == 0` → 中心格 `radiation += 均摊后 emitRads`；
  - 否则环内（椭圆）逐格 `radiation += line_absorption × (emitRads / lingerRate)`；
  - `emitStep++`（上限 max(rx,ry) 后重置）。
- [ ] **步骤 4：运行测试确认通过**
- [ ] **步骤 5：commit**（`feat(c2): radiation emitter pulsing + averaged (task 3/4)`）

### 任务 4：接线（frame_processor + update_data）+ 门控 + 全量回归

**文件：** 修改 `src/c_simulation/frame_processor.rs`、`src/c2_physics/mod.rs`、
`src/c_simulation/radiation_emitter.rs`

- [ ] **步骤 1：写失败测试**（门控：radiationEnabled=false 不更新；消息处理清空）
- [ ] **步骤 2：运行测试确认失败**
- [ ] **步骤 3：实现**
  - frame_processor：`process_radiation_emitter_messages`（真实实现）从
    `frame.radiation_emitter_messages` 处理 adds→modifies→removes（含延迟释放），
    从 `process_component_messages_stubs` 拆出；
  - update_data：`run_radiation_task` 之后调用
    `radiation_emitter::update_radiation_emitters(sim, 0.2, bounds)`；
  - `release_queued_handles_global`（延迟释放）挂到 sim_thread 每帧（与管道温度管理器一致）。
- [ ] **步骤 4：运行测试确认通过**
- [ ] **步骤 5：全量串行测试 + release 构建**
- [ ] **步骤 6：commit**（`feat(c2): radiation emitter wiring + DLC gating + regression (task 4/4)`）

---

## 自检记录

- 规格覆盖：消息（36/40/8B）→ 任务 1；注册表/句柄 → 任务 1；Update 时序 → 任务 2；
  tickConstant → 任务 2；tickPulsing/PulsingAveraged → 任务 3；接线/门控 → 任务 4。
- SimplePulse/RadialBeams/Attractor 明确不在本计划（阶段 B 范围外，扩展点预留）。
- 类型一致：`RadiationEmitterRegistry`/`update_radiation_emitters`/`tick_constant`/
  `tick_pulsing`/`in_radial_range`/`radiation_absorption_along_line` 命名一致。
- 方向约定：cell=row×w+col，game→sim 换算沿用 +1 边界；行高=更大索引（与液体/阳光一致）。
