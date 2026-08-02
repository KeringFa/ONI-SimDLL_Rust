# 液体上推分支 + 蒸发清理设计

**日期**：2026-08-02
**状态**：已批准（用户确认"严格参考原版原理"）
**类别**：C2 液体物理 · UpdateLiquid 第四分支 + PostProcessCell 清理

## 1. 背景

用户实测两处与原理不符：
1. **满格过压无限超限**：液体格可达 200%+（如 2000kg），原版在三层及以上水时底层仅比满格多 ~20kg（≈1020kg）。
2. **表层残留 0.0mcg 水**：微克级残水永不消失。

## 2. 根因（对照原版 + 早期稳定版原理）

### 2.1 上推分支缺失
原版 `UpdateLiquid` 共 4 个分支：下落（L700）→ 左流（L849）→ 右流（L946）→ **上推（L1043-1110，LAB_18004a4b2）**。上推分支：
```
阈值   = max(上方质量 × 1.01, maxMass)          // L1056-1059（同款 max 公式）
超出量 = max(0, 自身质量 − 阈值)
转移量 = 超出量 × 0.5
若 自身质量 < 2×阈值 → 转移量 = min(转移量, viscosity, 自身质量)   // L1066-1072
转移量 > 0.01 才执行 UpdateNeighbourLiquidMass(上方格) + flow[UP](+0xc)
```
三层水时中层 ≈1010（1000×1.01）、底层阈值 = max(1010×1.01,1000) ≈1020 → 底层最多 ~1020，超出向上回流。我们缺此分支 → 超满无法回流。

### 2.2 蒸发清理留桩
原版 `PostProcessCell`：液体质量 ≤ 0.01（且 ≠0.01）→ `Evaporate`（05_liquid_flow.c L2194）：`ClearCell`（元素→真空、质量/温度清零、病菌四字段，11_msvcrt L23677）+ `ChangeSubstance`。气体分支：被挤或质量 <1e-9 同样 Evaporate。
我们的 `do_evaporate_stub` 是空实现 → 微克级残水永存（早期稳定版用独立清理阶段阈值 1e-4 处理同类问题）。

## 3. 实现

### 3.1 update_liquid 追加上推分支（右分支后）
- 守卫：`up = cell + width < total`；判定链同左右（up 同元素或 state<2、updated 非固体、properties&2==0）。
- 公式如上；`src_ld` = 源格 liquid_data（viscosity/max_mass）。
- 转移走 `update_neighbour_liquid_mass`；成功 → 源格病菌/质量递减、`flow[cell].w += transfer`（offset 0xc，纹理 y=w−z 自洽）。
- 原版右分支失败也落到上推（L1017 goto），保持无条件 fall-through。

### 3.2 do_evaporate 替代空桩
- `do_evaporate(sim, cell)` = `clear_cell(updated)` + `push_substance_change`。
- 替换 3 处 `do_evaporate_stub` 调用：液体 mass≤0.01、气体被挤、气体 mass<1e-9。

## 4. 测试
- 上推：1020/上 1000 → 推 5（阈值 1010，验证 ~1020 封顶）；2000/上 1000 → 推 50（流速上限）；1000/上 1000 → 不推。
- 蒸发：液体 0.005 → 清成真空 + 事件；恰好 0.01 → 不清（原版 `!=0.01` 守卫）；气体 <1e-9 → 清理。
- 全量回归 + release。

## 5. 验收
- `cargo test -- --test-threads=1` 全过；release 编译成功。
- 实测：超满水会向上回流至 ~1020 封顶；表层无微克级残水。
