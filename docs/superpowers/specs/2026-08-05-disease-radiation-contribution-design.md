# 病菌模块・阶段 4：辐射病菌贡献（设计规格，2026-08-05）

## 目标与范围

**本阶段**（对照原版 SimDLL_Source.c L145106-145464，radiationEnabled 块内的第二遍逐格循环）：
实现 **RadiationSickness 病菌自身辐射贡献**：每格 disease_idx == RadiationSickness 时，
`radiation[cell] += disease_count[cell] × 0.001`。

**不做**：元素辐射扩散（25 格权重核，L42810-42850）与宇宙辐射应用/钳制——radiation.rs
已实现并审查通过（辐射模块阶段 A/B）；本阶段只补病菌贡献这一项。

## 原版机制全解

辐射任务第二遍逐格循环（region 内每格）顺序：

```text
1. 元素辐射扩散（25 格权重核：radiation[target] += mass×0.001×radsPer1000×weight）   ← 已实现
2. 病菌辐射贡献（L42860-42870）：
   uVar14 = Disease::GetDiseaseIndex(gDisease, 0xd49f77d6)      // RadiationSickness，每任务一次
   若 disease_idx[cell] == uVar14：
     radiation[cell] += disease_count[cell] × 0.001              ← 本阶段
3. 宇宙辐射应用（occlusion>0 → radiation += cosmic/lingerRate × occlusion）          ← 已实现
4. 钳制 [0, 9e6] + ≤0.01 → 0                                                     ← 已实现
```

注意：
- uVar14 在循环外计算一次（GetDiseaseIndex 线性搜索，表无 RadiationSickness 时返回 0xff；
  此时无病菌格（idx=0xff、count=0）匹配 → +0，行为等价，无需额外门控）；
- 原版无 gDisease null 防御（表必存在）；本项目防御：表缺失 → 跳过该贡献；
- 贡献使用 updatedCells 的 disease_idx/disease_count/radiation。

## 实现设计

### 修改 `simDLL_RS/src/c2_physics/radiation.rs`

`run_radiation_task` 的逐格循环内、元素辐射扩散之后、宇宙辐射应用之前插入：

```rust
// 病菌辐射贡献（原版 L42860-42870）：RadiationSickness 病菌自身辐射
// radiation[cell] += disease_count[cell] × 0.001
let rs_idx = disease_rs_index;   // 循环外计算：GetDiseaseIndex(gDisease, 0xd49f77d6)
let u = unsafe { &mut *sim.updated_cells.ptr };
if u.disease_idx.get(cell) == rs_idx {
    u.radiation.set(cell, u.radiation.get(cell) + u.disease_count.get(cell) as f32 * 0.001);
}
```

`disease_rs_index`：任务开头锁 G_DISEASE 一次；null → `None`（跳过整个贡献）；
否则 `Some(disease.get_disease_index(0xd49f77d6))`。

## 测试计划（TDD）

1. **RadiationSickness 细胞贡献**：disease_idx=RS 索引、count=5000 → radiation += 5.0；
2. **非 RS 病菌不贡献**：其他病菌 idx → radiation 不变；
3. **无病菌格（0xff）不贡献**：count=0 → +0；
4. **表缺失跳过**：G_DISEASE null → radiation 不变（不 panic）；
5. **表无 RadiationSickness**：GetDiseaseIndex 返回 0xff → 无病菌格 +0、RS 格不存在；
6. **update_data 集成**：RS 病菌格经全管线辐射 += count×0.001（radiationEnabled 门控）。

## 回归与安全

- 贡献只在 radiationEnabled 且病菌表含 RadiationSickness 时生效（DLC 场景）；
- 单星（无 RS 病菌/辐射关闭）零开销；
- 全量测试串行 + release 构建。

## 自检

1. **占位符**：无"待定/TODO"。
2. **内部一致**：位置（元素扩散后、宇宙应用前）与钳制顺序对照原版 L42810-42910。
3. **范围**：单实现计划可覆盖（radiation.rs 一处插入 + 测试）。
4. **歧义**：无（公式与位置均从反编译逐行核实）。
