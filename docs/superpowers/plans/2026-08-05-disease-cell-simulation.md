# 病菌模块・阶段 2 实现计划（格子病菌模拟：扩散 + 生长）

> **面向 AI 代理的工作者：** 本计划由主代理内联执行（用户已明确"终止，切回内联执行"，不使用子代理驱动）。
> 每步使用复选（`- [x]`）语法跟踪进度。

**目标：** 实现 `Disease::UpdateCells`（扩散，L26646）/ `Disease::PostProcess`（生长，L26424）/ `Disease::GetDiffusionScale`（L26329），并按原版顺序挂接 update_data（L145069 CopyFrom → L145088 扩散 → L145539 生长）。

**架构：** 新建 `c2_physics/disease.rs`（辅助公式 + 扩散 + 生长 + 区域循环）；`c2_physics/mod.rs` update_data 区域循环内插入 CopyFrom + 扩散（太空真空前）+ 生长（post_process_loop 后）。

**技术栈：** Rust 2021；gDisease 全局表（阶段 1 已就绪）；dt 固定 0.2；公式对照 C# `Disease.HalfLifeToGrowthRate` / `CalculateRangeHalfLife` / `ElemGrowthInfo.CalculateDiseaseCountDelta`。

---

## 任务 1：辅助公式（新建 c2_physics/disease.rs 骨架）

**文件：**
- 创建：`simDLL_RS/src/c2_physics/disease.rs`（`half_life_to_growth_rate` + `calculate_range_half_life` + 测试）
- 修改：`simDLL_RS/src/c2_physics/mod.rs`（`pub mod disease;`）

- [x] **步骤 1：编写失败测试**

```rust
#[test]
fn half_life_to_growth_rate_zero_is_zero() {
    assert_eq!(half_life_to_growth_rate(0.0, 0.2), 0.0);
}

#[test]
fn half_life_to_growth_rate_infinity_is_one() {
    assert_eq!(half_life_to_growth_rate(f32::INFINITY, 0.2), 1.0);
}

#[test]
fn half_life_to_growth_rate_matches_pow2() {
    // 2^(-0.2/1.0) = 0.870550563296124
    assert!((half_life_to_growth_rate(1.0, 0.2) - 0.870550563296124).abs() < 1e-6);
    // 2^(-0.2/10.0) = 2^(-0.02) = 0.9862327044935591
    assert!((half_life_to_growth_rate(10.0, 0.2) - 0.9862327044935591).abs() < 1e-6);
}

fn test_range() -> RangeInfo {
    RangeInfo { min_viable: 0.0, min_growth: 20.0, max_growth: 40.0, max_viable: 60.0 }
}
fn test_hl() -> RangeInfo {
    RangeInfo { min_viable: 100.0, min_growth: 200.0, max_growth: 300.0, max_viable: 400.0 }
}

#[test]
fn calculate_range_half_life_optimal_band_is_infinity() {
    assert_eq!(calculate_range_half_life(30.0, &test_range(), &test_hl()), f32::INFINITY);
}

#[test]
fn calculate_range_half_life_interpolates_band() {
    // temp=10 ∈ [0,20)：t=0.5 → lerp(100,200)=150
    assert_eq!(calculate_range_half_life(10.0, &test_range(), &test_hl()), 150.0);
}

#[test]
fn calculate_range_half_life_clamps_below_and_above() {
    assert_eq!(calculate_range_half_life(-5.0, &test_range(), &test_hl()), 100.0);
    assert_eq!(calculate_range_half_life(80.0, &test_range(), &test_hl()), 400.0);
}

#[test]
fn calculate_range_half_life_inf_propagates() {
    let hl = RangeInfo { min_viable: f32::INFINITY, ..test_hl() };
    assert_eq!(calculate_range_half_life(10.0, &test_range(), &hl), f32::INFINITY);
}
```

- [x] **步骤 2：运行确认失败**（函数不存在 → 编译失败）
- [x] **步骤 3：最小实现**
  - `half_life_to_growth_rate(half_life, dt)`：0→0、INF→1、`powf(2, −dt/hl)`；
  - `calculate_range_half_life(temp, range, half_lives)`：找第一个 `temp <= range[i]`，lower=max(i−1,0)、upper=i；无匹配 lower=upper=3；`lower==1&&upper==2` 或任一半活期 INF → INF；否则插值。
- [x] **步骤 4：运行确认通过**
- [x] **步骤 5：Commit**（`git add simDLL_RS/src/c2_physics/disease.rs simDLL_RS/src/c2_physics/mod.rs` → `feat(c2): disease helpers - half-life formulas (task 1/4)`）

---

## 任务 2：GetDiffusionScale + UpdateCells（扩散）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/disease.rs`（+`get_diffusion_scale` / `update_cells` / `run_disease_diffusion_task` + 测试）

- [x] **步骤 1：编写失败测试**（helper：3 元素表 + 病菌表 + SimData）

```rust
/// 3 元素最小表：0=气体(state=1)、1=液体(state=2)、2=固体(state=3)。
fn create_state_table() { /* 同 liquid_flow::create_state_table */ }

/// 1 病菌表：idx0 strength=2.0；ElemGrowthInfo（3 元素）：
/// diffusionScale=[1.0,0.5,0.0] minDiffusionCount=[10,5,100] minDiffusionInfestationTickCount=[2,1,3]
fn install_disease_table() { /* 挂 G_DISEASE */ }
fn uninstall_disease_table() { /* 释放并置 null */ }

#[test]
fn get_diffusion_scale_gates() {
    // elem0: count=9 < minDiffusionCount=10 → 0.0
    // elem0: count=100, tick=1 < minInfestationTick=2 → 0.0
    // elem0: count=100, tick=5 → 1.0
}

#[test]
fn update_cells_same_disease_equilibrates() {
    // a=气体 elem0 disease0 count100；b=气体 elem0 disease0 count50；scale=1.0
    // transfer=(50-100)×0.125×1.0=-6.25 → (int)-6
    // a=100-6=94，b=50+6=56（updatedCells）
}

#[test]
fn update_cells_empty_a_absorbs_from_b() {
    // a 无病菌(0xff)，b=disease0 count100 → transfer=100×0.125×1.0=12.5→12
    // a=12(disease0)，b=88
}

#[test]
fn update_cells_empty_b_receives_from_a() {
    // b 无病菌，a=disease0 count100 → a=88，b=12(disease0)
}

#[test]
fn update_cells_different_disease_stronger_flows() {
    // a=disease0(s2) count100；b=disease1(s1) count100 → a 更强，transfer=12
    // a=88；b 走 AddDiseaseToCell 混合：f10=100×1=100, f11=12×2=24, d=-404 → b=404 idx 保持 disease1
}

#[test]
fn update_cells_element_state_gate_blocks() {
    // a=气体(elem0) b=液体(elem1) → 不扩散（state&3 不同），两格不变
}
```

- [x] **步骤 2：运行确认失败**
- [x] **步骤 3：最小实现**（对照 L26646-26812）
  - `get_diffusion_scale(disease, sim, cell)`：disease_idx/elem 越界→0.0；count<tick 门控；返回 diffusionScale；
  - `update_cells(disease, sim, cell_a, cell_b)`：元素相态门控（`get_element_post_process_data(e).state & 3` 相等）；四分支算 transfer（`(int)` 截断）；`add_disease_to_cell(updated, a, src_disease, +t)` / `(updated, b, src_disease, −t)`；
  - `run_disease_diffusion_task(sim, bounds)`：锁 G_DISEASE 一次（null → 返回）；区域每格按右/下邻门控调 update_cells。
- [x] **步骤 4：运行确认通过**
- [x] **步骤 5：Commit**（`feat(c2): disease cell diffusion - UpdateCells (task 2/4)`）

---

## 任务 3：PostProcess 生长（run_disease_growth_task）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/disease.rs`（+`run_disease_growth_task` + 测试）

- [x] **步骤 1：编写失败测试**

```rust
/// 1 病菌表（idx0）：
/// tempRange(0,20,40,60)、tempHalfLives(5,2,2,5)
/// elem0: minCountPerKG=100 maxCountPerKG=1000 popHL=10 overPopHL=10 underPopDeath=1.0
/// radiationKillRate=0.5
fn install_growth_table() { /* 挂 G_DISEASE */ }

#[test]
fn post_process_optimal_temp_within_population_range() {
    // temp=30(最优带→f=1.0)、count=500、mass=1.0、acc=0
    // growth=(2^-0.02-1)×500=-6.88365；f25=-6.88365
    // count=500+(-6)=494；acc=-0.88365
}

#[test]
fn post_process_below_min_viable_decays() {
    // temp=-10 → hl[0]=5 → f=2^-0.04=0.9726549
    // f25=(500×0.9726549-500)+(-6.88365)=-20.5562 → count=480、acc=-0.5562
}

#[test]
fn post_process_under_population_constant_death() {
    // count=50 < min=100 → growth=-1.0×0.2=-0.2；temp 最优
    // f25=-0.2 → count+=trunc(-0.2)=0 → 50；acc=-0.2
}

#[test]
fn post_process_over_population_half_life() {
    // count=2000 > max=1000 → growth=(2^-0.02-1)×2000=-27.5346
    // f25=-27.5346 → count=1973、acc=-0.5346
}

#[test]
fn post_process_accumulated_error_carries() {
    // acc=0.4、count=500、temp 最优 → f25=0.4-6.88365=-6.48365 → count=494、acc=-0.48365
}

#[test]
fn post_process_radiation_kills_when_enabled() {
    // radiation_enabled=true、radiation[cell]=10、killRate=0.5
    // f25=-6.88365-5.0=-11.88365 → count=489、acc=-0.88365
}

#[test]
fn post_process_second_pass_clears_and_increments_tick() {
    // count=0、disease=0、tick=0、acc=0.5 → 第二遍：count<1 → 清四字段 → tick=0+1=1
    // 存活格（count=500、tick=5）→ tick=6
}
```

- [x] **步骤 2：运行确认失败**
- [x] **步骤 3：最小实现**（对照 L26424-26640）
  - 第一遍：跳过 disease==0xff；温度因子（calculate_range_half_life + half_life_to_growth_rate，dt=0.2）；种群因子三分支；`f25=(count×f+acc−count)+growth`；radiationEnabled 时 `−=radiation×killRate`；`acc=frac(f25)`、`count+=trunc(f25)`（防御越界/非有限跳过）；
  - 第二遍：count<1 → 清四字段；`tick=min(tick+1,0xfe)`（清除后 tick=1 怪癖保留）。
- [x] **步骤 4：运行确认通过**
- [x] **步骤 5：Commit**（`feat(c2): disease cell growth - PostProcess (task 3/4)`）

---

## 任务 4：update_data 挂接 + 集成测试 + 回归

**文件：**
- 修改：`simDLL_RS/src/c2_physics/mod.rs`（挂接）
- 修改：`simDLL_RS/src/c2_physics/disease.rs`（集成测试）

- [x] **步骤 1：编写失败测试**（集成：update_data 全管线）

```rust
#[test]
fn update_data_hooks_disease_diffusion_after_liquid() {
    // 全固体世界（elem2 state3，不流动）；两相邻固体格 disease0 count100/50
    // 病菌表：popHL=INF overPopHL=INF underPopDeath=0 min=0 max=INF tempHL 全 INF → 生长恒 0
    // scale=1.0 minDiffusionCount=0 minInfestationTick=0
    // update_data 一次 → updatedCells a=94 b=56（扩散生效）；tick 递增
}
```

- [x] **步骤 2：运行确认失败**（未挂接 → 不变）
- [x] **步骤 3：最小实现**（update_data 区域循环）

```rust
// 现有：update_liquid_loop(sim_data, bounds);
crate::c_simulation::sim_data_ops::copy_updated_cells_to_cells(sim_data); // 原版 L145069
crate::c2_physics::disease::run_disease_diffusion_task(sim_data, bounds);  // 原版 L145088
// 原版 L145106 辐射病菌贡献（RadiationSickness）→ 阶段 4 预留位
// 现有：run_space_vacuum_task / radiation / emitters / post_process_loop
crate::c2_physics::disease::run_disease_growth_task(sim_data, bounds);     // 原版 L145539
```

- [x] **步骤 4：运行确认通过** + 全量回归
- [x] **步骤 5：Commit**（`feat(c2): hook disease diffusion/growth into update_data (task 4/4)`）

---

## 任务 5：全量回归 + release + 更新代办

- [x] **步骤 1：全量测试串行**：`cargo test -- --test-threads=1`（基线 500，预计 +25 左右）
- [x] **步骤 2：release 构建**：`cargo build --release`
- [x] **步骤 3：更新 `代办.txt`**：病菌阶段 2 完成 → 阶段 3（病菌消息/组件 DiseaseEmitter/Consumer、ConsumeDisease、CellDiseaseModification）
- [x] **步骤 4：Commit**（`feat(c2): disease cell simulation done - regression green + release`）

---

## 自查

1. **规格覆盖度**：规格四条（GetDiffusionScale / UpdateCells / PostProcess / 挂接）对应任务 2/2/3/4；辅助公式（任务 1）是 PostProcess 前置。✓
2. **占位符**：无"待定/TODO"；阶段 4 辐射病菌以注释占位。✓
3. **类型一致性**：`half_life_to_growth_rate`/`calculate_range_half_life`/`get_diffusion_scale`/`update_cells`/`run_disease_diffusion_task`/`run_disease_growth_task` 签名在任务间一致。✓
