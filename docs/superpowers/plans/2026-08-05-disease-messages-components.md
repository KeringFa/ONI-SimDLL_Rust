# 病菌模块・阶段 3 实现计划（病菌消息/组件）

> **面向 AI 代理的工作者：** 本计划由主代理内联执行（用户已明确"终止，切回内联执行"）。
> 每步使用复选（`- [x]`）语法跟踪进度。

**目标：** 实现 DiseaseEmitter/DiseaseConsumer 消息与组件、ConsumeDisease（16B）、CellDiseaseModification（12B），并按原版挂接 frame_processor + update_data。

**架构：** 新建 `c_simulation/disease_component.rs`（BFS + ConsumeDisease + CellDiseaseModification + 发射器/消费者注册表 + Update）；`sim_frame_manager.rs` 修消息尺寸/emplace；`frame_processor.rs` 替换桩并接组件消息；`c2_physics/mod.rs` 挂 `update_disease_emitters`。

**技术栈：** Rust 2021；版本句柄注册表仿 radiation_emitter.rs；BFS 对照原版 Flood（L33466）；AddDiseaseToCell 复用阶段 1 强度混合。

---

## 任务 1：消息路由修正（sim_frame_manager.rs）

**文件：**
- 修改：`simDLL_RS/src/a_framework/sim_frame_manager.rs`

- [x] **步骤 1：编写失败测试**

```rust
#[test]
fn handle_message_consume_disease_emplaces_16b() {
    let mut mgr = SimFrameManager::new_zeroed();
    let mut result: *mut c_void = std::ptr::null_mut();
    let mut data = vec![0u8; 16];
    data[0..4].copy_from_slice(&5i32.to_le_bytes());
    data[4..8].copy_from_slice(&7i32.to_le_bytes());
    data[8..12].copy_from_slice(&0.5f32.to_le_bytes());
    data[12..16].copy_from_slice(&100i32.to_le_bytes());
    let mut reader = BinaryBufferReader::new(&data);
    assert!(mgr.handle_message(MessageType::ConsumeDisease as u32, &mut reader, &mut result));
    unsafe {
        assert_eq!((*mgr.current_frame).consume_disease.len(), 16, "ConsumeDisease 16B");
        let _ = Box::from_raw(mgr.current_frame);
    }
}

#[test]
fn handle_message_disease_emitter_messages_emplace_sizes() {
    // Add 4B / Modify 20B / Remove 8B → frame.disease_emitter_messages 收到原始字节
    let mut mgr = SimFrameManager::new_zeroed();
    let mut result: *mut c_void = std::ptr::null_mut();
    for (id, n) in [
        (MessageType::AddDiseaseEmitter as u32, 4usize),
        (MessageType::ModifyDiseaseEmitter as u32, 20usize),
        (MessageType::RemoveDiseaseEmitter as u32, 8usize),
    ] {
        let data = vec![0u8; n];
        let mut r = BinaryBufferReader::new(&data);
        assert!(mgr.handle_message(id, &mut r, &mut result));
    }
    unsafe {
        let f = &*mgr.current_frame;
        assert_eq!(f.disease_emitter_messages.adds.len(), 4);
        assert_eq!(f.disease_emitter_messages.modifies.len(), 20);
        assert_eq!(f.disease_emitter_messages.removes.len(), 8);
        let _ = Box::from_raw(mgr.current_frame);
    }
}

#[test]
fn handle_message_disease_consumer_messages_emplace_sizes() {
    // Add 12B / Modify 20B / Remove 8B
    // ... 同上，断言 disease_consumer_messages 三向量 12/20/8
}
```

- [x] **步骤 2：运行确认失败**（ConsumeDisease 现 12B → len 12；emitter/consumer 桩不 emplace → len 0）
- [x] **步骤 3：最小实现**
  - `MSG_CONSUME_DISEASE`：12 → 16；
  - 六个 emitter/consumer ID 从 "C2 stub no emplace" 分支拆出，分别 emplace 4/20/8 与 12/20/8。
- [x] **步骤 4：运行确认通过**
- [x] **步骤 5：Commit**（`feat(c2): disease message routing sizes - 16B consume + component emplace (task 1/4)`）

---

## 任务 2：disease_component.rs — BFS + ConsumeDisease + CellDiseaseModification

**文件：**
- 创建：`simDLL_RS/src/c_simulation/disease_component.rs`
- 修改：`simDLL_RS/src/c_simulation/mod.rs`（`pub mod disease_component;`）
- 修改：`simDLL_RS/src/c_simulation/frame_processor.rs`（替换 process_consume_disease / process_cell_disease_modifications 桩）

- [x] **步骤 1：编写失败测试**

```rust
// get_reachable_cells（4 向 BFS，depth<range，跳过 x==0||y==0||越界，全元素可达）
#[test]
fn get_reachable_cells_depth_gate_and_bounds() {
    // SimData 6x5；起点 (2,2)=cell14；range=2 → {14, 8(上), 13(左), 15(右), 20(下)}
    // range=1 → 仅 {14}
}

// process_consume_disease（原版 L422-522）
#[test]
fn consume_disease_rounds_and_clamps() {
    // gameCell=0 → simCell=7（width=6）；cell7 disease0 count100
    // percent=0.25 → consumed=(int)(100×0.25+0.5)=25 → count75 + callback{cb=7,idx=0,count=25}
    // percent=0.5 + max=30 → consumed=50→30 → count 70
    // count=5,percent=1.0 → count0 → 清四字段；callback diseaseIdx=0xff
}

// process_cell_disease_modifications（SimDLL_Source.c L116412-116474）
#[test]
fn cell_disease_modification_direct_and_mixed() {
    // {cellIdx=0, diseaseIdx=0xff, count=+10} → cell7 count=10（idx 保持 0xff，原版只加 count）
    // {cellIdx=0, diseaseIdx=3, count=5} → AddDiseaseToCell：cur=0xff → 替换 → idx3 count5
}
```

- [x] **步骤 2：运行确认失败**
- [x] **步骤 3：最小实现**
  - `get_reachable_cells(sim, x, y, range) -> Vec<usize>`：4 向 BFS + visited；
  - `process_consume_disease(frame, sim)`：16B/条解析（gameCell/callbackIdx/percent f32/max），四舍五入 + 钳制 + 清字段 + DiseaseConsumedCallback；
  - `process_cell_disease_modifications(frame, sim)`：12B/条；idx==0xff 直加 + 清；否则 AddDiseaseToCell（game→sim 坐标转换）。
- [x] **步骤 4：运行确认通过**
- [x] **步骤 5：Commit**（`feat(c2): disease consume + cell modification processing (task 2/4)`）

---

## 任务 3：DiseaseEmitter 组件

**文件：**
- 修改：`simDLL_RS/src/c_simulation/disease_component.rs`（+注册表 + Update + 消息处理 + 测试）
- 修改：`simDLL_RS/src/c_simulation/frame_processor.rs`（接 process_disease_emitter_messages）
- 修改：`simDLL_RS/src/c2_physics/mod.rs`（区域循环内挂 update_disease_emitters）

- [x] **步骤 1：编写失败测试**

```rust
// 注册表（版本句柄 + free list + emitted_info）
#[test]
fn disease_emitter_register_modify_remove() {
    // Register(callback=5) → handle 有效；componentStateChanged{handle,5}
    // Modify(handle, gameCell=0, idx=0, range=1, interval=1.0, count=10) → cell=7（width=6 换算）
    // Remove(handle, callback=-1) → 释放 + 版本+1；无回调
}

// Update（原版 L37651）
#[test]
fn disease_emitter_update_emits_on_interval() {
    // interval=1.0；dt=0.2 ×5 → elapsedTime 1.0 → 发射 +10 到 range=1 可达格（仅中心 cell7）
    // cell7 disease0 count100 → 110；emitted_info[0]={0,10}；elapsedTime 复位后继续累加
}

#[test]
fn disease_emitter_negative_count_only_same_disease() {
    // range=2 中心 cell7；cell7/cell8=disease0 count100、cell13=disease1 count100
    // emitCount=-5 → 仅 disease0 格 −5（95/95）；disease1 格不变
}

#[test]
fn disease_emitter_region_gate_skips_outside() {
    // 发射器 cell 不在 bounds → 不发射
}
```

- [x] **步骤 2：运行确认失败**
- [x] **步骤 3：最小实现**（仿 radiation_emitter.rs）
  - `DiseaseEmitterRegistry`：`data: Vec<DiseaseEmitterData>` + `versions` + `free_list` + `emitted_info: Vec<DiseaseEmittedInfo>`；`register/modify/remove`（componentStateChanged 回调）；
  - `process_disease_emitter_messages(frame, sim)`：adds(4B)→modifies(20B)→removes(8B) + 清空；
  - `update_disease_emitters(sim, dt, bounds)`：逐发射器（diseaseIdx!=0xff + cell 在 region + interval 触发 → emittedInfo 输出 + BFS 发射（emitCount 正负分支））；
  - `clear_disease_emitters()`：新世界分配时清空（与 radiation emitter 同约定）。
- [x] **步骤 4：运行确认通过**
- [x] **步骤 5：Commit**（`feat(c2): disease emitter component - register/update/emit (task 3/4)`）

---

## 任务 4：DiseaseConsumer 防御 + 集成 + 回归

**文件：**
- 修改：`simDLL_RS/src/c_simulation/disease_component.rs`（+consumer 消息处理 + 集成测试）
- 修改：`simDLL_RS/src/c_simulation/frame_processor.rs`（接 process_disease_consumer_messages；从 stub 清空列表移除 disease 向量）
- 修改：`simDLL_RS/src/a_framework/save_load.rs`（AllocateCells 清发射器注册表）

- [x] **步骤 1：编写失败测试**

```rust
// DiseaseConsumer（无 C# 发送方，防御）
#[test]
fn disease_consumer_messages_register_and_remove() {
    // Add 12B → 注册零值记录（原版 Register 不读字段）；Remove 8B → 释放
}

// update_data 集成：发射器经全管线发射
#[test]
fn update_data_emits_disease_via_emitter() {
    // 固体世界 + 生长恒 0 表（minDiffusionCount 巨大 → 扩散门控 0）
    // 注册发射器 gameCell=0 → simCell7，idx=0, range=1, interval=0.2, count=10
    // cell7 disease0 count100；active region {1,1,1,1}
    // update_data 一次 → cell7 count=110（发射 +10），tick=1
}
```

- [x] **步骤 2：运行确认失败**
- [x] **步骤 3：最小实现**：`process_disease_consumer_messages`（adds 12B 注册零值 / modifies 20B 接受忽略 / removes 8B 释放）+ consumedInfo 清空；AllocateCells 清发射器注册表。
- [x] **步骤 4：运行确认通过** + 全量回归
- [x] **步骤 5：Commit**（`feat(c2): disease consumer defense + update_data integration (task 4/4)`）

---

## 任务 5：全量回归 + release + 更新代办

- [x] **步骤 1：全量测试串行**：`cargo test -- --test-threads=1`（基线 522，预计 +20 左右）
- [x] **步骤 2：release 构建**：`cargo build --release`
- [x] **步骤 3：更新 `代办.txt`**：病菌阶段 3 完成 → 阶段 4（辐射病菌贡献 RadiationSickness → +count×0.001）
- [x] **步骤 4：Commit**（`feat(c2): disease messages/components done - regression green + release`）

---

## 自查

1. **规格覆盖度**：规格六节（消息结构/ConsumeDisease/CellDiseaseModification/Emitter/Consumer/BFS/挂接）对应任务 1-4。✓
2. **占位符**：无"待定/TODO"；DiseaseConsumer::Update 以"接受忽略/预留"注释标注。✓
3. **类型一致性**：`get_reachable_cells`/`process_consume_disease`/`process_cell_disease_modifications`/`process_disease_emitter_messages`/`update_disease_emitters`/`process_disease_consumer_messages` 签名一致。✓
