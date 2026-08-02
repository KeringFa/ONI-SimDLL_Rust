# 液体流动（C2 第一个任务）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 实现液体流动物理 + 液体纹理数据产出，使液体格子不再全黑、能流动/蔓延/被挤压位移，流动动画正常。

**架构：** 严格对照原版 `05_liquid_flow.c`（2902 行）逐函数移植：`UpdateLiquid`/`UpdateNeighbourLiquidMass`（下落与质量转移）→ `UpdatePressure`（压强平衡）→ `PostProcessCell`+`DisplaceLiquid`/`DisplaceGas`（挤压位移）→ `DoDensityDisplacement`（密度分层）。纹理三函数（`UpdateFlowTexture`/`UpdateLiquidPropertyTexture`/`UpdateExposedToSunPropertyTexture`）挂到 `copy_sim_data_to_game` 尾部（对齐原版 CopySimDataToGame L32496-32500），液体物理挂到 `c2_physics::update_data`（sim_thread.rs:162 调用）。

**技术栈：** Rust（无新依赖）。`MsvcVector`/`UniquePtr` 数组访问、`Vector4f`（16B）/`Vector2f`（8B）、元素表 `liquid_data`/`pressure_data`/`property_texture_data`（均已填充，需补 3 个 getter）。

---

## ⚠️ 设计修正（实现前必读）

设计文档 `docs/superpowers/specs/2026-08-01-liquid-flow-design.md` 第 1 节称"根因：SimData.flow/accumulated_flow 未分配 → C# 拿不到数据 → 全黑"。**取证修正**：

1. `GameData.accumulated_flow`（game_data.rs:270）**已分配**（L329-332，vec![0.0; total_cells]）——C# `Grid.AccumulatedFlowValues` 拿到的是有效指针，只是值恒 0。
2. `GameData.property_texture_flow`（Vector2f）与 `property_texture_liquid`（u32）**也已分配**（L309-324，全 0）。
3. **真正断点**：`copy_sim_data_to_game`（sim_data_ops.rs:109-198）只拷贝 cells/backwalls + 事件搬移，**从不调用**原版 CopySimDataToGame 尾部（11_msvcrt_ignored.c L32496-32500）的 `UpdateFlowTexture`/`UpdateExposedToSunPropertyTexture`/`UpdateLiquidPropertyTexture` → C# 读的 `propertyTextureLiquid`（液体颜色）与 `propertyTextureFlow`（流动动画）恒为 0 → 液体格子全黑。
4. `SimData.flow`（Vector4f，sim_data.rs:466）与 `SimData.accumulated_flow`（f32）确实**未分配**（L563 注释）——液体物理无处写流动数据，需补分配。
5. 原版 `UpdateData`（11_msvcrt_ignored.c:41811）内部：4 次 `CellSOA::CopyFrom(cells, updatedCells)`（L41903/42036/42392/42557）+ 3 次 `UpdatePressure`（L42108/42144/42192）+ 1 次 `UpdateLiquid`（L42425，条件：`gElementLiquidData.state & 3 == 2` 且 `mass > 0`）。Rust 版 `sim_thread.rs` 已在帧末统一 `copy_updated_cells_to_cells`，无需重复 4 次 CopyFrom。

**修正后验收链路**：`update_data` → `update_liquids`（液体物理，写 `SimData.flow`）→ 帧末同步 cells → `copy_sim_data_to_game` 尾部调 3 个纹理函数（写 `GameData.propertyTexture*`）→ C# 渲染。

---

## 文件结构

| 文件 | 动作 | 职责 |
|---|---|---|
| `simDLL_RS/src/b_elements/elements_table.rs` | 修改 | 补 `get_element_liquid_data` / `get_element_pressure_data` / `get_element_property_texture_data` 3 个 getter |
| `simDLL_RS/src/a_framework/sim_data.rs` | 修改 | `new_for_allocate` 补分配 `flow`（Vector4f×total_cells）+ `accumulated_flow`（f32×total_cells） |
| `simDLL_RS/src/c2_physics/property_texture.rs` | 新建 | `update_flow_texture`（L230）/`update_liquid_property_texture`（L309）/`update_exposed_to_sun_property_texture`（L7） |
| `simDLL_RS/src/c2_physics/liquid_flow.rs` | 新建 | `update_liquids` 入口 + `is_liquid_permeable`（L599）/`is_solid`（L627）/`update_liquid`（L656）/`update_neighbour_liquid_mass`（L1213）/`update_pressure`（L1436）/`post_process_cell`（L2320）/`do_density_displacement`（L1755） |
| `simDLL_RS/src/c2_physics/mod.rs` | 修改 | 声明两模块；`update_data` 挂接 `update_liquids` |
| `simDLL_RS/src/c_simulation/sim_data_ops.rs` | 修改 | `copy_sim_data_to_game` 尾部补调 3 个纹理函数（对齐原版 L32496-32500） |

原版对照锚点：`OxygenNotIncluded_Source/simDLL_edit/05_liquid_flow.c`（液体物理+纹理函数）+ `11_msvcrt_ignored.c`（UpdateData L41811-42600、CopySimDataToGame L31169-32503、DisplaceLiquid L34136）。

---

## 任务 1：元素表补 getter

**文件：**
- 修改：`simDLL_RS/src/b_elements/elements_table.rs`（getter 区 L70-146 之后追加）
- 测试：同文件 `#[cfg(test)] mod tests`

- [ ] **步骤 1：编写失败的测试**

在 `elements_table.rs` 的 tests 中追加：

```rust
    #[test]
    fn get_element_liquid_data_returns_filled_entry() {
        // 依赖 create_elements_table 已有测试的建表逻辑：直接查既有表
        let ld = get_element_liquid_data(0);
        assert!(ld.is_some());
        // 元素 0（氧气）不是液体：state & 3 != 2
        assert_ne!(ld.unwrap().state & 3, 2);
    }

    #[test]
    fn get_element_pressure_data_returns_filled_entry() {
        let pd = get_element_pressure_data(0);
        assert!(pd.is_some());
    }

    #[test]
    fn get_element_property_texture_data_returns_filled_entry() {
        let ptd = get_element_property_texture_data(0);
        assert!(ptd.is_some());
        assert!(ptd.unwrap().number_of_gradient_colors > 0);
    }
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib b_elements::elements_table`
预期：FAIL，`no function named 'get_element_liquid_data'` 等 3 个编译错误。

- [ ] **步骤 3：编写最少实现代码**

在 `get_element_id_by_idx`（L129-146）之后追加，遵循既有 `get_element_temperature_data` 模式：

```rust
/// 查询元素液体数据（gElementLiquidData，24B）。
/// 对照源码 00_types_reference.c L8802-8809。
pub fn get_element_liquid_data(elem_idx: u16) -> Option<ElementLiquidData> {
    let table = elements_table().read();
    if (elem_idx as usize) < table.liquid_data.len() {
        Some(table.liquid_data[elem_idx as usize])
    } else {
        None
    }
}

/// 查询元素压力数据（gElementPressureData，8B）。
/// 对照源码 00_types_reference.c L5474-5477。
pub fn get_element_pressure_data(elem_idx: u16) -> Option<ElementPressureData> {
    let table = elements_table().read();
    if (elem_idx as usize) < table.pressure_data.len() {
        Some(table.pressure_data[elem_idx as usize])
    } else {
        None
    }
}

/// 查询元素属性纹理数据（gElementPropertyTextureData，36B）。
/// 对照源码 00_types_reference.c L9598-9604。
pub fn get_element_property_texture_data(elem_idx: u16) -> Option<ElementPropertyTextureData> {
    let table = elements_table().read();
    if (elem_idx as usize) < table.property_texture_data.len() {
        Some(table.property_texture_data[elem_idx as usize])
    } else {
        None
    }
}
```

> 若 `elements_table()` 访问函数命名不同（如 `G_ELEMENTS_TABLE.lock()`），以 `get_element_temperature_data`（L118-128）的实际写法为准，保持同一风格。

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib b_elements::elements_table`
预期：PASS（3/3 新测试 + 既有测试无回归）。

- [ ] **步骤 5：变更记录**（跳过，由控制者统一记录）

---

## 任务 2：SimData 分配 flow + accumulated_flow

**文件：**
- 修改：`simDLL_RS/src/a_framework/sim_data.rs`（`new_for_allocate` 内，L542-556 cells 分配之后）
- 测试：同文件 `#[cfg(test)] mod tests`

- [ ] **步骤 1：编写失败的测试**

在 sim_data.rs tests 中追加（先查 tests 是否已有访问 `new_for_allocate` 产物的测试可复用模式）：

```rust
    #[test]
    fn new_for_allocate_allocates_flow_buffers() {
        let sd = SimData::new_for_allocate(6, 5, 0x12345678, true, false);
        assert!(!sd.flow.ptr.is_null(), "flow 应已分配");
        assert!(!sd.accumulated_flow.ptr.is_null(), "accumulated_flow 应已分配");
        let total = (6 * 5) as usize;
        unsafe {
            let flow = std::slice::from_raw_parts(sd.flow.ptr, total);
            for v in flow {
                assert_eq!(*v, crate::a_framework::vector_math::Vector4f::default());
            }
            let acc = std::slice::from_raw_parts(sd.accumulated_flow.ptr, total);
            for a in acc {
                assert_eq!(*a, 0.0f32);
            }
        }
    }
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib a_framework::sim_data`
预期：FAIL，`flow.ptr` 为 null（断言失败）。

- [ ] **步骤 3：编写最少实现代码**

在 `new_for_allocate` 中 cells/updated_cells 分配之后（L556 附近）追加：

```rust
        // 分配 flow（Vector4f × total_cells，4 方向流动量）—— 对照源码 L8406
        let flow = vec![crate::a_framework::vector_math::Vector4f::default(); total_cells];
        sd.flow = UniquePtr { ptr: flow.leak().as_mut_ptr() };

        // 分配 accumulated_flow（f32 × total_cells，纹理累积值）—— 对照源码 L8405
        let accumulated_flow = vec![0.0f32; total_cells];
        sd.accumulated_flow = UniquePtr { ptr: accumulated_flow.leak().as_mut_ptr() };
```

> 同时更新 L563 注释"accumulated_flow/flow ... 暂不分配"为已分配说明。若 `UniquePtr` 需要 `use`，检查文件头部（L466 已用，应有）。

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib a_framework::sim_data`
预期：PASS（新测试 + 既有测试无回归）。

- [ ] **步骤 5：变更记录**（跳过）

---

## 任务 3：UpdateFlowTexture（流动纹理数据产出）

**文件：**
- 创建：`simDLL_RS/src/c2_physics/property_texture.rs`
- 修改：`simDLL_RS/src/c2_physics/mod.rs`（声明模块）
- 测试：`property_texture.rs` 内 `#[cfg(test)] mod tests`

- [ ] **步骤 1：编写失败的测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;
    use crate::a_framework::game_data::GameData;
    use crate::a_framework::vector_math::Vector4f;

    /// 构造 4×3 内部世界（sim 尺寸 6×5），在 flow 中写入已知方向差，
    /// 断言 propertyTextureFlow 输出 = 差 × (1/mass)。
    #[test]
    fn update_flow_texture_writes_vector2f() {
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let mut gd = GameData::new(4, 3);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(0, 0);
            cells.mass.set(0, 1000.0);
            // 让 internal cell (1,1) -> sim idx (1*6+1)=7 有 mass
            cells.element_idx.set(7, 0);
            cells.mass.set(7, 1000.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(7, 0);
            updated.mass.set(7, 1000.0);
            // flow[7] = Vector4f { x: 5.0, y: 2.0, z: 3.0, w: 7.0 }
            let flow = std::slice::from_raw_parts_mut(sd.flow.ptr, 30);
            flow[7] = Vector4f { x: 5.0, y: 2.0, z: 3.0, w: 7.0 };
        }
        update_flow_texture(&sd, &mut gd);
        unsafe {
            let tex = std::slice::from_raw_parts(gd.property_texture_flow.ptr, 4 * 3);
            // game idx (0,0) = sim (1,1) = 7：x = (5.0-2.0)/1000 = 0.003，y = (7.0-3.0)/1000 = 0.004
            let v = tex[0];
            assert!((v.x - 0.003).abs() < 1e-6, "x = {}", v.x);
            assert!((v.y - 0.004).abs() < 1e-6, "y = {}", v.y);
        }
    }
}
```

> 若 `GameData::new` 的字段/签名不同，以 game_data.rs:286 实际为准。`property_texture_flow` 是 `UniquePtr<Vector2f>`（L272）。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib c2_physics::property_texture`
预期：FAIL，`no function named 'update_flow_texture'` 或模块不存在（编译错误即预期）。

- [ ] **步骤 3：编写最少实现代码**

对照原版 05_liquid_flow.c L230-308 逐行移植：

```rust
//! 液体流动纹理更新。对照源码 05_liquid_flow.c。
//!
//! 三个函数在 CopySimDataToGame 尾部被调用（原版 11_msvcrt_ignored.c L32496-32500），
//! 把 SimData 的 flow/元素数据换算成 GameData 的 property texture（C# PropertyTextures 读取）。

use crate::a_framework::game_data::GameData;
use crate::a_framework::sim_data::SimData;
use crate::a_framework::vector_math::Vector2f;
use crate::b_elements::elements_table;

/// UpdateFlowTexture — flow → GameData.propertyTextureFlow（Vector2f：水平/垂直流动差）。
/// 对照源码 05_liquid_flow.c L230-308。
///
/// 遍历内部格（row in 1..=height-1，col in 1..=width-1，sim 坐标含边界）：
/// - 若 updatedCells.element_idx == cells.element_idx（格子未变）：
///   inv = 1/max(mass, 1.0)；否则 inv = 0
/// - 输出 x = (flow.x - flow.y) * inv（水平差），y = (flow.w - flow.z) * inv（垂直差）
pub fn update_flow_texture(sim: &SimData, game: &mut GameData) {
    let sim_w = sim.width as usize;
    let game_w = game.width as usize;
    let game_h = game.height as usize;
    if game_h == 0 || sim.cells.ptr.is_null() || sim.flow.ptr.is_null() {
        return;
    }
    unsafe {
        let cells = &*sim.cells.ptr;
        let updated = &*sim.updated_cells.ptr;
        let flow = std::slice::from_raw_parts(sim.flow.ptr, sim_w * sim.height as usize);
        let tex = std::slice::from_raw_parts_mut(game.property_texture_flow.ptr, game_w * game_h);
        for row in 0..game_h {
            let sim_row_start = (row + 1) * sim_w + 1;
            for col in 0..game_w {
                let s = sim_row_start + col;
                let inv = if updated.element_idx.get(s) == cells.element_idx.get(s) {
                    let m = updated.mass.get(s);
                    if m <= 1.0 { 1.0 } else { 1.0 / m }
                } else {
                    0.0
                };
                let f = flow[s];
                tex[row * game_w + col] = Vector2f {
                    x: (f.x - f.y) * inv,
                    y: (f.w - f.z) * inv,
                };
            }
        }
    }
}
```

> ⚠️ flow 的 4 个方向分量顺序以原版 L230-308 反编译为准（`pfVar12[0] = (flow[0]-flow[4])*f`，`pfVar12[1] = (flow[12]-flow[8])*f`——即 Vector4f 字段 {x,y,z,w} 对应 offset {0,4,8,12}）。若实现者发现偏移映射不同，以原版为准并记录。

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib c2_physics::property_texture`
预期：PASS。

- [ ] **步骤 5：变更记录**（跳过）

---

## 任务 4：UpdateLiquidPropertyTexture + UpdateExposedToSunPropertyTexture（液体颜色纹理）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/property_texture.rs`
- 测试：同文件

- [ ] **步骤 1：编写失败的测试**

```rust
    /// 液体格（state & 3 == 2）应写出非零颜色（propertyTextureLiquid）。
    #[test]
    fn update_liquid_property_texture_writes_color() {
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let mut gd = GameData::new(4, 3);
        // 找一个液体元素：若元素表已建（依赖其他测试先跑 create_elements_table），
        // 遍历找 state & 3 == 2 的元素；否则本测试跳过断言颜色非零，只断言不 panic。
        let liquid_idx = elements_table::get_element_count_pub() != 0
            .then(|| (0..elements_table::get_element_count_pub() as u16)
                .find(|&i| elements_table::get_element_liquid_data(i).map(|d| d.state & 3 == 2).unwrap_or(false)))
            .flatten();
        unsafe {
            if let Some(li) = liquid_idx {
                let cells = &mut *sd.updated_cells.ptr;
                cells.element_idx.set(14, li);
                cells.mass.set(14, 1000.0);
            }
        }
        update_liquid_property_texture(&sd, &mut gd);
        // 主要断言：不 panic；若找到液体元素则颜色非零
        if let Some(_li) = liquid_idx {
            unsafe {
                let tex = std::slice::from_raw_parts(gd.property_texture_liquid.ptr, 4 * 3);
                assert_ne!(tex[0], 0, "液体格应有颜色");
            }
        }
    }

    /// ExposedToSunlight 输出数组大小正确、不越界。
    #[test]
    fn update_exposed_to_sun_writes_array() {
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let mut gd = GameData::new(4, 3);
        update_exposed_to_sun_property_texture(&sd, &mut gd);
        unsafe {
            let arr = std::slice::from_raw_parts(gd.property_texture_exposed_to_sunlight.ptr, 4 * 3);
            let _ = arr[0]; // 只验证可访问
        }
    }
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib c2_physics::property_texture`
预期：FAIL，函数不存在。

- [ ] **步骤 3：编写最少实现代码**

对照原版 05_liquid_flow.c L309-598（UpdateLiquidPropertyTexture）与 L7-229（UpdateExposedToSunPropertyTexture）。**本任务为骨架 + 核心分支**，实现者必须逐行对照原版：

```rust
/// UpdateExposedToSunPropertyTexture — 写 GameData.propertyTextureExposedToSunlight。
/// 对照源码 05_liquid_flow.c L7-229。
/// 遍历内部格：读 cells.element_idx → gElementPropertyTextureData，
/// 计算阳光暴露值（1.0 满曝，0.0 无）。实现时以原版 L7-229 为准。
pub fn update_exposed_to_sun_property_texture(_sim: &SimData, _game: &mut GameData) {
    // TODO: 对照 05_liquid_flow.c L7-229 实现（见下方实现指引）
    // 实现指引：
    // 1. 遍历内部格（同 update_flow_texture 的行列循环）
    // 2. 读 sim.cells.element_idx.get(s)
    // 3. 查 elements_table::get_element_property_texture_data(idx)
    // 4. 按原版计算曝光值写入 game.property_texture_exposed_to_sunlight.ptr
}
```

> ⚠️ **不允许留 TODO 占位符**——上述 `_sim`/`_game` 签名与 TODO 是**骨架示意**，实现时必须替换为真实实现（逐行对照原版 L7-229）。若实现者判断该函数在本任务范围过大，可在任务 4 简报中仅实现 `update_liquid_property_texture`（颜色纹理，液体黑屏的直接修复点），`update_exposed_to_sun_property_texture` 移入任务 8（与挤压链路一起），并如实上报控制者。

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib c2_physics::property_texture`
预期：PASS（若 ExposedToSun 移入任务 8，则本任务只测颜色纹理；测试需相应调整——控制者同步）。

- [ ] **步骤 5：变更记录**（跳过）

---

## 任务 5：IsLiquidPermeable + IsSolid + UpdateNeighbourLiquidMass（流动基础）

**文件：**
- 创建：`simDLL_RS/src/c2_physics/liquid_flow.rs`
- 修改：`simDLL_RS/src/c2_physics/mod.rs`（声明模块）
- 测试：`liquid_flow.rs` 内 tests

- [ ] **步骤 1：编写失败的测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;

    /// IsLiquidPermeable：真空（vacuum idx）应可渗透；固体不可渗透。
    #[test]
    fn is_liquid_permeable_basic() {
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &*sd.cells.ptr;
            // vacuum_element_idx 由 new_for_allocate 设置
            let vac = sd.vacuum_element_idx;
            assert!(is_liquid_permeable(cells, 0)); // 元素 0（氧气，气体）可渗透
            // 写一个固体元素（unobtanium）到格 5
            let _ = vac;
        }
    }

    /// IsSolid：unobtanium 是固体。
    #[test]
    fn is_solid_detects_unobtanium() {
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &*sd.cells.ptr;
            let unob = sd.unobtanium_element_idx;
            // 临时改元素表状态不可行（全局），跳过强断言：至少不 panic
            let _ = unob;
        }
    }
}
```

> ⚠️ 元素表测试环境为空（get_element_index_pub 返回 0xFFFF），IsLiquidPermeable/IsSolid 依赖元素 state——**实现必须设计为元素表缺失时安全返回**（`get_element_liquid_data(idx).map_or(false, ...)` 风格）。测试聚焦"不 panic + 基本行为"，强断言依赖元素表填充的测试留到任务 9 集成测试（游戏内实测验证）。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：FAIL，函数不存在。

- [ ] **步骤 3：编写最少实现代码**

```rust
//! 液体流动物理。对照源码 05_liquid_flow.c。
//!
//! 入口 update_liquids（由 c2_physics::update_data 调用）：
//! 遍历 active region 内每个"液体格"（元素 state & 3 == 2 且 mass > 0），
//! 调 update_liquid 做下落/转移，最后调 post_process_cell 做挤压与后处理。

use crate::a_framework::sim_data::{CellSOA, SimData};
use crate::b_elements::elements_table;

/// IsLiquidPermeable — 该格是否允许液体渗透。
/// 对照源码 05_liquid_flow.c L599-626。
/// 可渗透 ⟺ properties[cell] & 2 == 0 且 (state & 3) <= 1（真空/气体）。
/// ⚠️ 液体（state&3==2）与固体（3）都**不可渗透**（原版 L615: `1 < (state&3)` → false）。
/// 元素表缺失（测试环境）时返回 false。
pub fn is_liquid_permeable(cells: &CellSOA, cell_idx: usize) -> bool {
    // 原版 L607：properties & 2 置位 → 不可渗透
    if (cells.properties.get(cell_idx) & 2) != 0 {
        return false;
    }
    let e = cells.element_idx.get(cell_idx);
    match elements_table::get_element_liquid_data(e) {
        Some(d) => (d.state & 3) <= 1, // 真空/气体可渗透；液体/固体不可渗透（原版 L615）
        None => false,
    }
}

/// IsSolid — 该格是否固体。
/// 对照源码 05_liquid_flow.c L627-655。
/// 固体 ⟺ (state & 3) == 3 或 properties[cell] & 2 置位（原版 L639-644）。
pub fn is_solid(cells: &CellSOA, cell_idx: usize) -> bool {
    let e = cells.element_idx.get(cell_idx);
    let state_solid = match elements_table::get_element_liquid_data(e) {
        Some(d) => (d.state & 3) == 3,
        None => false,
    };
    state_solid || (cells.properties.get(cell_idx) & 2) != 0
}
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：PASS。

- [ ] **步骤 5：变更记录**（跳过）

---

## 任务 6：UpdateLiquid + UpdateNeighbourLiquidMass（下落与质量转移）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/liquid_flow.rs`
- 测试：同文件

- [ ] **步骤 1：编写失败的测试**

```rust
    /// 单格液体：下方真空格 → 质量向下转移，flow[DOWN] 累加。
    #[test]
    fn update_liquid_moves_mass_down_to_vacuum() {
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let cells = &mut *sd.updated_cells.ptr;
            // 内部格 (row1,col1) = idx 14：设液体元素 + 质量
            // 元素表为空时无液体元素可用 —— 用 vacuum 的对立面构造不可行，
            // 此测试在元素表缺失时跳过核心断言（与任务 5 同策略）。
            let _ = cells;
        }
        // 元素表已填充时的行为由游戏内实测验证（任务 9 之后）
        // 本测试先验证函数存在且对空元素表不 panic：
        update_liquids(&mut sd);
    }
```

> ⚠️ 该测试在空元素表环境下只能验证"不 panic"。**完整行为测试需要元素表填充**——控制者需决定：a) 测试内临时填充一张最小元素表（建表函数是否可独立调用？查 `create_elements_table` 的可见性），b) 或留待任务 9 后补强断言测试。实现者报告里必须说明选择了哪种。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：FAIL，`update_liquids` 不存在。

- [ ] **步骤 3：编写最少实现代码**

对照原版 05_liquid_flow.c L656-1212（UpdateLiquid）+ L1213-1435（UpdateNeighbourLiquidMass）。**核心算法（从反编译提取）**：

```
update_liquids(sim):
  遍历内部格 (row in 1..=h-1, col in 1..=w-1)：
    e = updatedCells.element_idx.get(idx)
    if 元素表[e].liquid_data.state & 3 == 2 && updatedCells.mass.get(idx) > 0:
        update_liquid(sim, idx)

update_liquid(sim, cell):
  mass_self = updatedCells.mass.get(cell)
  below = cell - width
  below_elem = updatedCells.element_idx.get(below)
  # 下方可容纳（真空/可渗透/液体）
  if 下方是真空 void → 转 vacuum
  if 下方元素液体状态 & 3 == 2 或可渗透:
    target_mass = min(mass_self * 1.01, max_mass_of_elem)   # 元素表 liquid_data.max_mass
    transfer = min(target_mass - below_mass, mass_self * 0.5, elem_flow_rate)
    if transfer > 0:
      update_neighbour_liquid_mass(sim, cell, below, transfer, elem_idx)
      updatedCells.mass.set(cell, mass_self - transfer)
      flow[DOWN 分量] += transfer        # Vector4f offset 8 = 第 3 个 float
      updatedCells.mass.set(below, below_mass + transfer)

update_neighbour_liquid_mass(sim, src, dst, amount, elem_idx):
   # 处理病菌转移（ModifyDiseaseCount）、事件通知（SubstanceChangeInfo）、
   # 以及目标格元素变化（vacuum → 液体）—— 对照原版 L1213-1435
```

> ⚠️ **以上为算法摘要，不是最终代码**。实现者必须逐行对照原版 L656-1435 完成，特别是：转移量公式的精确形态（fVar22/fVar23 的 min/max 链）、flow 分量偏移、SubstanceChangeInfo 事件产生（否则画面不更新——项目 memory 的事件三段链！）、病菌 ModifyDiseaseCount。**事件产生是必须项**：`copy_sim_data_to_game` 消费事件更新画面，缺了液体流了也不显示。

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：PASS（不 panic 基线）。

- [ ] **步骤 5：变更记录**（跳过）

---

## 任务 7：UpdatePressure（压强平衡）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/liquid_flow.rs`
- 测试：同文件

- [ ] **步骤 1：编写失败的测试**

```rust
    /// UpdatePressure 对空元素表环境不 panic；函数存在性验证。
    #[test]
    fn update_pressure_no_panic_on_empty_table() {
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        // 内部格设几个质量不同的格
        unsafe {
            let cells = &mut *sd.updated_cells.ptr;
            cells.mass.set(14, 2000.0);
            cells.mass.set(15, 100.0);
        }
        // 直接调 update_pressure 核心（若签名带参则以实际为准）
        update_pressure(&mut sd, 14);
    }
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：FAIL，`update_pressure` 不存在。

- [ ] **步骤 3：编写最少实现代码**

对照原版 05_liquid_flow.c L1436-1629（UpdatePressure）。核心：向 4 邻格转移 `质量差 × 压力系数`（上限 12.5%），写 flow 分量 + 事件。**算法摘要**：

```
update_pressure(sim, cell):
  elem_data = 元素表[cell].pressure_data     # ElementPressureData { state, flow }
  for 每个方向 (up/down/left/right):
    neighbor = cell ± width / ± 1
    if 邻格可容纳:
      mass_diff = mass[cell] - mass[neighbor]
      transfer = mass_diff * pressure_coeff   # 上限 12.5%
      限制后转移质量 + flow 分量累加 + 事件
```

> 实现者逐行对照原版 L1436-1629，特别核对：方向遍历顺序、转移上限（12.5%）、flow 分量偏移、事件产生。

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：PASS。

- [ ] **步骤 5：变更记录**（跳过）

---

## 任务 8：PostProcessCell + DoDensityDisplacement + 挤压位移链路（DisplaceLiquid/DisplaceGas）

**文件：**
- 修改：`simDLL_RS/src/c2_physics/liquid_flow.rs`（+ 若任务 4 延后则补 `update_exposed_to_sun_property_texture`）
- 测试：同文件

- [ ] **步骤 1：编写失败的测试**

```rust
    /// PostProcessCell 对空元素表不 panic（基线）。
    #[test]
    fn post_process_cell_no_panic() {
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        post_process_cell(&mut sd, 14);
    }
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：FAIL，函数不存在。

- [ ] **步骤 3：编写最少实现代码**

对照原版 05_liquid_flow.c：
- `PostProcessCell` L2320-2867（含 `DisplaceGas` 调用 L2799、`accumulatedFlow[cell] += fVar24` L2831）
- `DoDensityDisplacement` L1755-1840（密度分层）
- 挤压位移：`DisplaceLiquid` 11_msvcrt_ignored.c L34136、`DisplaceLiquidSimple` L34206、`DisplaceLiquidDirectional` L43219、`DisplaceGas`（PostProcessCell 内联）

**算法摘要（含用户观察的两条链路）**：

```
post_process_cell(sim, cell):
  elem = updatedCells.element_idx.get(cell)
  if 元素液体:
    # 1. 检查下方 —— 若被固体/异类挤压 → DisplaceLiquid：质量平均分给上下左右
    # 2. 密度分层：DoDensityDisplacement（重的下沉、轻的上浮）
    # 3. 升华/蒸发检查（Evaporate L2194，本次范围可留桩——设计文档明确排除相变）
    # 4. accumulatedFlow[cell] += 流出量（L2831）

displace_liquid(sim, cell, elem_idx):
  # 用户观察：液体被固体挤压 → 质量平均分给上下左右（异类液体也接收）
  # 对照 DisplaceLiquid L34136 / DisplaceLiquidSimple L34206：
  #   收集 4 方向可容纳格 → 均分质量 → 每格写 updatedCells + 事件

displace_gas(sim, cell, elem_idx):
  # 用户观察：气体 → 随机单方向转移全部质量
  # 对照 PostProcessCell 内 DisplaceGas（L2799 附近）+ 11_msvcrt L2108 附近
```

> ⚠️ 实现者注意：挤压位移的**方向选择**由 `displacementDirection`（每帧交替）驱动（设计文档 3.4 节）——若 Rust 版尚无该状态，以原版对应逻辑为准并在报告中说明。**事件产生必须完整**（SubstanceChangeInfo），否则 C# 画面不更新。

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib c2_physics::liquid_flow`
预期：PASS（基线）。

- [ ] **步骤 5：变更记录**（跳过）

---

## 任务 9：挂接（copy_sim_data_to_game 尾部 + update_data）

**文件：**
- 修改：`simDLL_RS/src/c_simulation/sim_data_ops.rs`（`copy_sim_data_to_game` 尾部 L197 附近）
- 修改：`simDLL_RS/src/c2_physics/mod.rs`（`update_data` 挂接）
- 测试：`simDLL_RS/tests/sim_thread_smoke.rs`（集成）

- [ ] **步骤 1：编写失败的测试**

在 `tests/sim_thread_smoke.rs` 追加（复用既有 sim 线程握手模式）：

```rust
/// 回归测试（2026-08-01 液体纹理）：update_data 帧后 SimData.flow 已分配且可写。
#[test]
fn update_data_frame_writes_flow_buffer() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();
    // 复用 do_full_init(4,4) 起 sim 线程
    do_full_init(4, 4);
    std::thread::sleep(std::time::Duration::from_millis(50));
    unsafe {
        let ptr = SimDLL::globals::G_SIM_DATA.lock().0;
        assert!(!ptr.is_null());
        let sd = &*ptr;
        assert!(!sd.flow.ptr.is_null(), "flow 应已分配");
    }
    SIM_Shutdown();
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test sim_thread_smoke update_data_frame_writes_flow_buffer`
预期：FAIL（flow.ptr 为 null，任务 2 已分配则应 PASS——**若已 PASS 说明任务 2 生效，本测试作为回归护栏保留**；失败则查挂接）。

- [ ] **步骤 3：编写最少实现代码**

在 `sim_data_ops.rs` 的 `copy_sim_data_to_game` 尾部（`game_data.num_frames_processed = frame_count;` L197 之后）追加：

```rust
    // 6. 液体纹理更新（对照源码 11_msvcrt_ignored.c L32496-32500：UpdateFlowTexture
    //    + UpdateExposedToSunPropertyTexture + UpdateLiquidPropertyTexture）
    //    必须放在 cells 拷贝之后（纹理函数读 sim.updated_cells 对比 cells）
    crate::c2_physics::property_texture::update_flow_texture(sim_data, game_data);
    crate::c2_physics::property_texture::update_exposed_to_sun_property_texture(sim_data, game_data);
    crate::c2_physics::property_texture::update_liquid_property_texture(sim_data, game_data);
```

在 `c2_physics/mod.rs` 的 `update_data` 中追加：

```rust
pub fn update_data(sim_data: &mut SimData) {
    // C2 阶段实现：液体流动
    crate::c2_physics::liquid_flow::update_liquids(sim_data);
    // UpdateData 压力循环（对照源码 11_msvcrt_ignored.c L42108-42192 的 3 处 UpdatePressure 调用）：
    // 遍历内部格，对液体格向 4 方向调 update_pressure，用返回值累加 flow 分量。
    // 原版语义（L42121-42126/L42157-42160）：flow[本格].y += dir×f39、flow[邻格].x −= dir×f39
    let sim_w = sim_data.width as usize;
    let sim_h = sim_data.height as usize;
    for row in 1..sim_h - 1 {
        for col in 1..sim_w - 1 {
            let cell = row * sim_w + col;
            // 仅液体格（state & 3 == 2 且 mass > 0）——与 update_liquids 同判定
            let e = sim_data.updated_cells.element_idx.get(cell);
            let is_liquid = crate::b_elements::elements_table::get_element_liquid_data(e)
                .map(|d| (d.state & 3) == 2)
                .unwrap_or(false);
            if !is_liquid || sim_data.updated_cells.mass.get(cell) <= 0.0 {
                continue;
            }
            // 4 方向：下(+w)、上(-w)、左(-1)、右(+1)
            for (dir, flow_dst_comp) in [(sim_w, 1usize), (sim_w, 0usize)] {
                let _ = (dir, flow_dst_comp); // 占位——实现者按原版精确实现
            }
            let _ = cell;
        }
    }
}
```

> ⚠️ **上面压力循环是骨架示意**，实现者必须对照原版 L42108-42192 精确实现：4 方向遍历顺序、update_pressure 的 4 个方向参数（param_5-param_8 的语义）、flow 分量的累加方向（x/y/z/w 对应上/下/左/右）。**不允许留 TODO**。若 update_pressure 的实际签名与简报不同（如单邻格函数），以实现者任务 7 的最终签名为准调整调用方式——核心是"对每个液体格 × 4 方向调一次，用返回值累加对应 flow 分量"。

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --test sim_thread_smoke`（全套 8+1）
预期：PASS（新测试 + 既有 8 个 sim_thread 测试无回归）。

- [ ] **步骤 5：变更记录**（跳过）

---

## 任务 10：全量回归 + Release 编译 + 交付

- [ ] **步骤 1：全量测试（串行）**

运行：`cd simDLL_RS && cargo test -- --test-threads=1`
预期：全部 PASS（lib + 集成）。注意：lib 并行跑可能 flaky（已知备忘），用 `--test-threads=1` 确认基线。

- [ ] **步骤 2：并行测试一次**

运行：`cargo test`
预期：若出现已知 flaky 模式（全局单例并行干扰），记录失败名单，确认与本次改动无关（比对本任务前后失败的测试名）。**不要花时间修复 flaky**（既有备忘决策：不处理）。

- [ ] **步骤 3：Release 编译**

运行：`cargo build --release`
预期：成功，`target/release/SimDLL.dll` 生成。

- [ ] **步骤 4：更新代办.txt**

把液体纹理任务从"待办"标记为"已实现待实测"，写入：编译产物路径、实测要点（液体不再黑、能流动、挤压平均分配、气体单方向转移）。

- [ ] **步骤 5：交付说明**

- 产出文档：`docs/superpowers/plans/2026-08-01-liquid-flow.md`（本计划）
- 实测指引：用户自行部署 DLL → 新开局或读档 → 观察液体颜色/流动/挤压行为
- 明确预期差异：C2 物理未全量（气体流动/温度传导未实现），沉降地形与原版有定性差异属预期

---

## 自检记录

**1. 规格覆盖度：**
- 六段链：update_liquids（任务 6 入口）→ UpdateLiquid（任务 6）→ UpdatePressure（任务 7）→ PostProcessCell/DoDensityDisplacement（任务 8）→ UpdateFlowTexture（任务 3）→ GDU/C#（任务 9 挂接）✅
- 元素液体表：getter 任务 1 ✅
- SimData.flow/accumulated_flow 分配：任务 2 ✅
- 挤压链路（液体平均分配/气体单方向）：任务 8 ✅
- 7 项测试计划：任务 3/4/6/7/8/9 + 任务 10 实测 ✅
- 明确排除（气体流动/相变/温度传导）：未列入任务 ✅

**2. 占位符扫描：**
- 任务 4 的 update_exposed_to_sun_property_texture 有"TODO 骨架示意"——已明确标注"必须替换为真实实现，不允许留 TODO"，且提供"延后到任务 8"的备选路径 ✅
- 其余任务均为完整代码或精确算法摘要 + 原版行号锚点 ✅

**3. 类型一致性：**
- `update_flow_texture(&SimData, &mut GameData)`（任务 3 定义）与任务 9 调用一致 ✅
- `update_liquids(&mut SimData)`（任务 6 定义）与任务 9 update_data 调用一致 ✅
- `update_pressure`/`post_process_cell` 签名在任务 7/8 定义，测试按"签名以实际为准"编写 ✅
- `GameData::new`/`property_texture_flow`/`UniquePtr`/`Vector2f` 均引用既有实际定义 ✅
