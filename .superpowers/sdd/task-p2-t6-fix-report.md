# T6 审查修复报告

**修复目标 commit：** `0db5954` (feat(elements_table): implement create_element_interactions)
**修复日期：** 2026-07-30
**修复文件：** `simDLL_RS/src/elements_table.rs`

## 关键 bug 概述

每 entry 多读 2 字节（实际 34B/entry，源码 32B/entry）。

**根因：** 注释 `共读 4 + 2*4 + 5*4 = 30 字节` 算术错误（`4+8+20=32`≠`30`），导致实现者误判"只读了 30B 需要 skip(2) 补齐 32B"，实际已读 32B，`skip(2)` 后变 34B。

**运行时影响：** 真实游戏数据（32B/entry）反序列化时，entry 0 正常，但 entry 1 从 offset 34 开始读（应为 32），signature 字段读到错位数据，导致所有 entry ≥1 的 gas_obliteration 全部错误。

**测试盲区根因：** `build_interactions_buffer` 也写了 34B/entry（含末尾 `w.write_ushort(0)`），测试 buffer 与实现共享同一对齐错误，自洽但皆错。

## 修复内容

### 修复 1（关键 K1）：删除实现中的 `reader.skip(2)?;`

**位置：** `simDLL_RS/src/elements_table.rs:328`（修复前）

```rust
// 修复前
let elem_result_mass_creation_multiplier = reader.read_float()?;
// 共读 4 + 2*4 + 5*4 = 30 字节，剩 2 字节 padding @0x1e-0x1f
reader.skip(2)?;

// 修复后
let elem_result_mass_creation_multiplier = reader.read_float()?;
// 共读 4 + 2*4 + 5*4 = 32 字节，与源码 0x20 stride 对齐
```

修复后读取字节累加：`read_int`(4) + `read_ushort`×4(8) + `read_float`×5(20) = **32 字节/entry**，与源码 `lVar3 = lVar3 + 0x20`（03_elements.c L565）一致。

### 修复 2（关键 K1）：同步修测试 buffer `build_interactions_buffer`

**位置：** `simDLL_RS/src/elements_table.rs:674`（修复前）

```rust
// 修复前
w.write_float(create_mult).unwrap();
w.write_ushort(0).unwrap();  // 末尾 2 字节 padding

// 修复后
w.write_float(create_mult).unwrap();
// （删除末尾 padding 写入）
```

修复后测试 buffer 也是 32B/entry，与真实游戏数据一致。

### 修复 3（次要 M1）：修正注释算术

3 处注释统一修正：

1. **L317** `// 每 entry 0x20=32 字节，读 30 字节 + skip 2 字节 padding（决策 #2）` → `// 每 entry 0x20=32 字节，与源码 stride 一致`
2. **L328** `// 共读 4 + 2*4 + 5*4 = 30 字节，剩 2 字节 padding @0x1e-0x1f` → `// 共读 4 + 2*4 + 5*4 = 32 字节，与源码 0x20 stride 对齐`
3. **L662** `// 总计 30 字节，剩 2 字节 padding @0x1e-0x1f` → `// 总计 32 字节，与源码 0x20 stride 对齐`

### 修复 4（次要 M2）：补充 entry 2 字段断言

**位置：** `simDLL_RS/src/elements_table.rs:712-721`（修复后）

`create_element_interactions_pushes_only_matching_signature` 测试中，entry 2（push 到 `gas_obliterations[1]`）原仅验证 2 个字段（`elem_idx1`、`elem_result_idx`），现补充 6 个字段断言（含 5 个浮点字段），验证 32B 对齐下连续 entry 的浮点字段正确：

```rust
let g1 = &table.gas_obliterations[1];
assert_eq!(g1.elem_idx1, 7);
assert_eq!(g1.elem_idx2, 8);                                      // 新增
assert_eq!(g1.elem_result_idx, 9);
assert!((g1.interaction_probability - 0.3).abs() < 1e-6);        // 新增
assert!((g1.min_mass - 5.0).abs() < 1e-6);                       // 新增
assert!((g1.elem1_mass_destruction_percent - 0.1).abs() < 1e-6); // 新增
assert!((g1.elem2_mass_required_multiplier - 0.8).abs() < 1e-6); // 新增
assert!((g1.elem_result_mass_creation_multiplier - 0.4).abs() < 1e-6); // 新增
```

值来源：测试 buffer 构造 `(GAS_OBLITERATION_SIGNATURE, 7, 8, 9, 0.3, 5.0, 0.1, 0.8, 0.4)`。

## 验证

### 聚焦测试

命令：`cargo test --lib elements_table::tests::create_element_interactions`

```
running 3 tests
test elements_table::tests::create_element_interactions_clears_before_push ... ok
test elements_table::tests::create_element_interactions_pushes_only_matching_signature ... ok
test elements_table::tests::create_element_interactions_negative_count_returns_early ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 106 filtered out; finished in 0.00s
```

**3/3 通过。** 特别地，`create_element_interactions_pushes_only_matching_signature` 验证了 3-entry buffer（32B/entry）下 entry 2（push 到 [1]）的全部 8 个 pub 字段正确，包括 5 个浮点字段，证明 32B 对齐下连续 entry 解析正确。

### 全量回归

命令：`cargo test --lib`

```
running 109 tests
... (109 个测试详情省略) ...

test result: ok. 109 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

**109/109 通过。** 无回归。

### 编译警告检查

命令：`cargo build --tests`

```
   Compiling simdll_rs v0.1.0 (C:\Users\KeringFa\Desktop\simDLL_RS_Kit\simDLL_RS)
warning: crate `SimDLL` should have a snake case name
  |
  = help: convert the identifier to snake case: `sim_dll`
  = note: `#[warn(non_snake_case)]` (part of `#[warn(nonstandard_style)]`) on by default

warning: `simdll_rs` (lib) generated 1 warning
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.12s
```

**仅预存 1 个 warning**（crate `SimDLL` snake_case，与原版 DLL 命名约束相关，非本次修复引入）。无新增警告。

## 提交信息

**commit SHA：** 见下方 git log 输出
**commit 标题：** `fix(elements_table): correct 32B entry stride in create_element_interactions (K1)`

## 自审发现

1. **修复 1 已确认：** 实现中 `reader.skip(2)?;` 已删除（diff L328 -行）
2. **修复 2 已确认：** 测试 buffer 中末尾 `w.write_ushort(0).unwrap();` 已删除（diff L674 -行）
3. **修复 3 已确认：** 3 处注释算术已修正为 32 字节（diff L317、L328、L662）
4. **修复 4 已确认：** entry 2 断言已补充 6 个字段（diff L712-721，含 `elem_idx2` + 5 个浮点字段）
5. **聚焦测试 3/3 通过** ✓
6. **全量回归 109/109 通过** ✓
7. **编译警告仅剩预存 1 个**（crate SimDLL snake_case）✓

**特别验证：** 修复后 entry 2 的 5 个浮点字段（`interaction_probability`、`min_mass`、`elem1_mass_destruction_percent`、`elem2_mass_required_multiplier`、`elem_result_mass_creation_multiplier`）全部通过断言，证明 32B 对齐下连续 entry 的浮点字段解析正确——这正是修复前若真实数据传入会错位的字段，现在有了测试守护。

## 额外发现

- 工作树中 `.superpowers/sdd/progress.md` 也有修改，但非本次修复范畴，未纳入本次 commit。
- 仅 stage `simDLL_RS/src/elements_table.rs`（修复本体）+ 本报告文件提交。
