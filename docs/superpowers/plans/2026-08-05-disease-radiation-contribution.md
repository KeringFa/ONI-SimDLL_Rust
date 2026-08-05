# 病菌模块・阶段 4 实现计划（辐射病菌贡献）

> **面向 AI 代理的工作者：** 本计划由主代理内联执行（用户已明确"终止，切回内联执行"）。
> 每步使用复选（`- [x]`）语法跟踪进度。

**目标：** 在 radiation.rs 的 `run_radiation_task` 逐格循环中实现 RadiationSickness 病菌辐射贡献（原版 L42860-42870）：`radiation[cell] += disease_count[cell] × 0.001`（disease_idx == RS 索引时）。

**架构：** 修改 `simDLL_RS/src/c2_physics/radiation.rs` 一处插入 + 测试。

---

## 任务 1：病菌贡献实现 + 单元测试（radiation.rs）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/radiation.rs`

- [x] **步骤 1：编写失败测试**（沿用 init_radiation_table / make_sim / fill_both）

```rust
/// 病菌表挂 G_DISEASE：idx0 = RadiationSickness（hash 0xd49f77d6）、idx1 = 其他病菌。
fn install_rs_table(include_rs: bool) {
    let mut t = Disease::new();
    if include_rs {
        t.diseases.push_unchecked(DiseaseInfo { hash_id: 0xd49f77d6, ..Default::default() });
    }
    t.diseases.push_unchecked(DiseaseInfo { hash_id: 0x9999, ..Default::default() });
    // 挂 G_DISEASE（Box::into_raw）
}
fn uninstall_disease_table() { /* 释放 + null */ }

#[test]
fn radiation_sickness_disease_contributes_count_times_0001() {
    // RS 在 idx0；cell7 disease_idx=0、count=5000、radiation=0
    // run_radiation_task（full_grid_bounds、cosmic=0、elem0 rads=0）
    // → radiation[7] = 5000×0.001 = 5.0
}

#[test]
fn non_radiation_sickness_disease_does_not_contribute() {
    // cell7 disease_idx=1（非 RS）、count=5000 → radiation[7] 保持 0
}

#[test]
fn no_disease_cell_contributes_zero() {
    // cell7 disease_idx=0xff、count=0 → radiation[7] 保持 0
}

#[test]
fn missing_disease_table_skips_contribution() {
    // G_DISEASE null → run_radiation_task 不 panic、radiation 不变
}

#[test]
fn table_without_radiation_sickness_is_noop() {
    // 表只有 idx0=0x9999 → GetDiseaseIndex(0xd49f77d6)=0xff → 无贡献
}
```

- [x] **步骤 2：运行确认失败**（未实现 → RS 格 radiation=0）
- [x] **步骤 3：最小实现**（run_radiation_task 逐格循环，元素辐射扩散之后、宇宙辐射应用之前）

```rust
// 任务开头（循环外）：
let rs_idx = {
    let ptr = crate::globals::G_DISEASE.lock().0;
    if ptr.is_null() { None } else { Some(unsafe { &*ptr }.get_disease_index(0xd49f77d6)) }
};
// 循环内（元素扩散后）：
if let Some(rs) = rs_idx {
    let u = unsafe { &mut *sim.updated_cells.ptr };
    if u.disease_idx.get(cell) == rs {
        u.radiation.set(cell, u.radiation.get(cell) + u.disease_count.get(cell) as f32 * 0.001);
    }
}
```

- [x] **步骤 4：运行确认通过**
- [x] **步骤 5：Commit**（`feat(c2): radiation sickness disease contributes radiation (task 1/2)`）

---

## 任务 2：update_data 集成 + 回归 + release + 代办

**文件：**
- 修改：`simDLL_RS/src/c2_physics/radiation.rs`（集成测试）

- [x] **步骤 1：编写失败测试**

```rust
#[test]
fn update_data_applies_radiation_sickness_contribution() {
    // 固体世界（create_solid_table）+ RS 病菌表（idx0=0xd49f77d6，生长恒 0、
    // minDiffusionCount 巨大）→ active region {1,1,1,1}
    // cell7 disease_idx=0、count=5000；update_data 一次 → radiation[7] = 5.0
}
```

- [x] **步骤 2：运行确认失败**（未挂接 → radiation=0）
- [x] **步骤 3：最小实现**：无（任务 1 已实现；测试验证全管线挂接）。
- [x] **步骤 4：运行确认通过** + 全量回归
- [x] **步骤 5：Commit**（`feat(c2): disease radiation contribution done - regression green + release`）

---

## 自查

1. **规格覆盖度**：规格测试计划 6 条对应任务 1/2。✓
2. **占位符**：无"待定/TODO"。✓
3. **类型一致性**：`rs_idx: Option<u8>`、`get_disease_index(0xd49f77d6)` 与 Disease API 一致。✓
