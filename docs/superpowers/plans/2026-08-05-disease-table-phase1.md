# 病菌模块・阶段 1 实现计划（病菌表解析 + 转移强度混合）

> **面向 AI 代理的工作者：** 本计划由主代理内联执行（用户已明确"终止，切回内联执行"，不使用子代理驱动）。
> 每步使用复选（`- [x]`）语法跟踪进度。

**目标：** 实现 `CreateDiseaseTable` 消息解析（对照原版 `Disease::Disease` 构造，11_msvcrt_ignored.c L25718-26091），挂接 `gDisease` 全局，并把 `AddDiseaseToCell` 从"同累加/异替换"升级为原版"病菌强度混合"（L26163-26263）。

**架构：** 解析逻辑放在 `b_elements/disease.rs`（`Disease::from_stream`），handler 挂接放在 `a_framework/save_load.rs`，强度混合放在 `c2_physics/liquid_flow.rs`。`gDisease` 全局（`globals::G_DISEASE`）已存在，`clean_up` 已释放。

**技术栈：** Rust 2021 + MsvcVector/MsvcString 内存布局精确对齐；消息 ID 0x31311BAF = 825301935；KleiString = u32 长度 + UTF-8 字节（无 NUL）。

---

## 任务 1：Disease::from_stream 解析（disease.rs + stl_shim.rs）

**文件：**
- 修改：`simDLL_RS/src/b_elements/disease.rs`（新增 `from_stream` + 测试）
- 修改：`simDLL_RS/src/a_framework/stl_shim.rs`（`MsvcString` 新增 `set_bytes`/`as_bytes`）

- [x] **步骤 1：编写失败测试**（disease.rs tests：构造 1 病菌 + 2 元素的消息字节）

```rust
#[test]
fn disease_from_stream_parses_all_fields() {
    let mut w = BinaryBufferWriter::new();
    w.write_int(1); // count
    w.write_int(2); // elements_count
    w.write_int(11); // KleiString len
    w.write_bytes(b"TestDisease");
    w.write_uint(0x11223344);
    w.write_float(1.5);
    for v in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0,
              9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0] {
        w.write_float(v);
    }
    w.write_float(17.5);
    for i in 0..2 {
        w.write_float(0.1 + i as f32);
        w.write_float(0.2 + i as f32);
        w.write_float(0.3 + i as f32);
        w.write_float(0.4 + i as f32);
        w.write_float(0.5 + i as f32);
        w.write_float(0.6 + i as f32);
        w.write_int(7 + i);
        w.write_byte(8 + i as u8);
    }
    let mut reader = BinaryBufferReader::new(&w.into_bytes());
    let d = Disease::from_stream(&mut reader).expect("parse ok");
    assert_eq!(d.disease_names.len(), 1);
    assert_eq!(d.disease_names.as_slice()[0].as_bytes(), b"TestDisease");
    let di = &d.diseases.as_slice()[0];
    assert_eq!(di.hash_id, 0x11223344);
    assert_eq!(di.strength, 1.5);
    assert_eq!(di.temperature_range.min_viable, 1.0);
    assert_eq!(di.temperature_half_lives.max_viable, 8.0);
    assert_eq!(di.pressure_range.min_growth, 9.0);
    assert_eq!(di.pressure_half_lives.max_growth, 16.0);
    assert_eq!(di.radiation_kill_rate, 17.5);
    let eg = &di.elem_growth_info;
    assert_eq!(eg.under_population_death_rate.as_slice(), &[0.1, 1.1]);
    assert_eq!(eg.population_half_life.as_slice(), &[0.2, 1.2]);
    assert_eq!(eg.over_population_half_life.as_slice(), &[0.3, 1.3]);
    assert_eq!(eg.diffusion_scale.as_slice(), &[0.4, 1.4]);
    assert_eq!(eg.min_count_per_kg.as_slice(), &[0.5, 1.5]);
    assert_eq!(eg.max_count_per_kg.as_slice(), &[0.6, 1.6]);
    assert_eq!(eg.min_diffusion_count.as_slice(), &[7, 8]);
    assert_eq!(eg.min_diffusion_infestation_tick_count.as_slice(), &[8, 9]);
}
```

- [x] **步骤 2：运行测试确认失败**（`from_stream`/`set_bytes` 不存在 → 编译失败，即红灯）
- [x] **步骤 3：最小实现**
  - `MsvcString::set_bytes(&mut self, bytes: &[u8])`：≤15B 走 SSO（buffer[15]=15），>15B 堆分配（buffer[0..8] 存指针，size=len, capacity=len）；`as_bytes(&self) -> Vec<u8>` 读回。堆内存不自动释放（与项目 MsvcVector 快照泄漏约定一致，注释说明）。
  - `Disease::from_stream(reader) -> Result<Disease, BufferError>`：读 count/elements_count → `get_element_count_pub()` 校验（不等 → `tracing::warn!`，不 panic，与 CreateElementsTable 同款）→ `disease_names.resize(count, default)` → 循环逐病菌读 name/hash/17×f32/elemGrowth 8 向量 resize 到 elements_count 并逐项读 6×f32+1×i32+1×u8。
- [x] **步骤 4：运行测试确认通过**（含 `disease_from_stream_elements_count_mismatch_is_warn_only`：elements_count 与表不匹配仍 Ok 可读；`disease_from_stream_empty`：count=0 → 空表；`disease_from_stream_truncated_returns_err`）
- [x] **步骤 5：Commit**（`git add simDLL_RS/src/b_elements/disease.rs simDLL_RS/src/a_framework/stl_shim.rs` → `feat(c2): disease table parsing - CreateDiseaseTable (task 1/3)`）

---

## 任务 2：handle_create_disease_table 挂接（save_load.rs）

**文件：**
- 修改：`simDLL_RS/src/a_framework/save_load.rs`（替换空桩 + 测试）

- [x] **步骤 1：编写失败测试**

```rust
#[test]
fn handle_create_disease_table_sets_global() {
    let _lock = LIB_TESTS_LOCK.lock().unwrap();
    clean_up();
    let data = make_disease_table_msg(1, 0x11223344);
    let mut reader = BinaryBufferReader::new(&data);
    let result = handle_create_disease_table(&mut reader);
    assert!(result.is_null(), "原版 CreateDiseaseTable 返回 null");
    let g = globals::G_DISEASE.lock().0;
    assert!(!g.is_null(), "gDisease 已挂接");
    let d = unsafe { &*g };
    assert_eq!(d.diseases.as_slice()[0].hash_id, 0x11223344);
    clean_up();
}
```

- [x] **步骤 2：运行确认失败**（空桩只 warn，不挂接 → 断言失败）
- [x] **步骤 3：最小实现**：`handle_create_disease_table` 调 `Disease::from_stream`；成功 → 释放旧 Box → `Box::into_raw` 挂 `G_DISEASE`，返回 null（对照原版 L507-517 替换 + 析构）；失败 → `tracing::error!` + 返回 null。
- [x] **步骤 4：运行确认通过**（含 `handle_create_disease_table_replaces_old`：两次不同 hash → 指针变化且新数据生效；`handle_create_disease_table_truncated_returns_null`：截断数据 → null 且全局保持原值；`clean_up_after_create_disease_table`：替换后 clean_up 不崩）
- [x] **步骤 5：Commit**（`feat(c2): wire CreateDiseaseTable handler to gDisease (task 2/3)`）

---

## 任务 3：AddDiseaseToCell 强度混合（liquid_flow.rs）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/liquid_flow.rs`（替换简化版 + 测试）

- [x] **步骤 1：编写失败测试**（构造 `Disease` 表挂 `G_DISEASE`：idx0 strength 2.0 / idx1 strength 4.0）

```rust
#[test]
fn add_disease_to_cell_same_disease_accumulates() {
    // cur idx3 count100 + new idx3 count50 → 150，idx 不变，infestation 不清
}

#[test]
fn add_disease_to_cell_one_side_invalid_replaces() {
    // cur 0xff → 替换为新病菌
}

#[test]
fn add_disease_to_cell_mixing_keep_current_when_weaker_new() {
    // cur(0,100,s2) + new(1,50,s4): f10=200<=f11=200, cur>=0 → 保留当前 (0,100)
}

#[test]
fn add_disease_to_cell_mixing_negative_cur_count_switches() {
    // cur(0,-100,s2) + new(1,50,s4): f10<=f11, cur<0 → (-cur=100, idx=1)
}

#[test]
fn add_disease_to_cell_mixing_stronger_current_keeps() {
    // cur(0,100,s2) + new(1,100,s4): f10=200<=400 → 保留当前 (0,100)
}

#[test]
fn add_disease_to_cell_mixing_negative_d_keeps_current() {
    // cur(0,100,s2) + new(1,100,s1): f10=200>100, d=-100 → (100, idx=0)
}

#[test]
fn add_disease_to_cell_mixing_positive_d_replaces() {
    // cur(0,100,s2) + new(1,150,s1): f10=200>150, d=(150-200/150*100)=16 → (16, idx=1)
}

#[test]
fn add_disease_to_cell_result_nonpositive_clears() {
    // cur(3,5) + new(3,-5) → 0 → 清四字段
}

#[test]
fn add_disease_to_cell_mixing_changes_idx_resets_infestation() {
    // infestation=7 → 混合后 idx 变化 → infestation=0
}

#[test]
fn add_disease_to_cell_without_table_keeps_current() {
    // G_DISEASE null + 双真实病菌 → 保留当前（防御回退）
}
```

- [x] **步骤 2：运行确认失败**（简化版无混合 → 断言失败）
- [x] **步骤 3：最小实现**：按原版 L26163-26263 逐分支重写（同累加 / 异默认替换 / 双真实默认保留当前 + f10/f11 混合 / result<=0 清四字段）。`G_DISEASE` 表为空或索引越界 → 防御保留当前。
- [x] **步骤 4：运行确认通过** + 回归液体测试
- [x] **步骤 5：Commit**（`feat(c2): AddDiseaseToCell strength mixing matches original (task 3/3)`）

---

## 任务 4：全量回归 + release + 更新代办

- [x] **步骤 1：全量测试串行**：`cargo test --test-threads=1`（基线 481，预计 +15 左右）
- [x] **步骤 2：release 构建**：`cargo build --release`
- [x] **步骤 3：更新 `代办.txt`**：病菌阶段 1 完成 → 状态推进到阶段 2（格子病菌模拟 UpdateCells/PostProcess/CalculateFinalDiseaseCount/GetDiffusionScale）
- [x] **步骤 4：Commit**（`feat(c2): disease table phase 1 done - regression green + release`）

---

## 自查

1. **规格覆盖度**：规格三条（解析 / gDisease 挂接 / AddDiseaseToCell 混合）分别对应任务 1/2/3；测试计划四条（消息解析全字段 / elements_count 校验不 panic / 混合四分支 / CleanUp）都有对应测试。✓
2. **占位符扫描**：无 "待定/TODO/后续实现"。✓
3. **类型一致性**：`from_stream`/`set_bytes`/`as_bytes`/`add_disease_to_cell` 签名在任务间一致。✓
