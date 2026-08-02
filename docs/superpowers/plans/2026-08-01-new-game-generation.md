# 新游戏生成（InitializeFromCells 启动分支）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 实现 `SimData_InitializeFromCells` 消息处理器及其两个依赖函数（SettleThermalBoundaries、InitializeBoundary），打通新游戏世界生成（DoSettleSim）链路。

**架构：** 严格镜像原版 C++（11_msvcrt_ignored.c L24200-24421）：handler 解析 14B 头 + 三块裸数组（元素为 u16 索引非哈希），`new_for_allocate` 建 SimData 替换全局，三循环填充 `updated_cells`/`backwall`，最后调两个新实现的 `SimData` 方法完成边界初始化。规格：`docs/superpowers/specs/2026-08-01-new-game-generation-design.md`。

**技术栈：** Rust（cdylib）、tracing 日志、cargo test。

**⚠️ 本项目特殊约定（优先级高于技能默认）：**
- **不执行任何 git 操作**（仓库整改暂缓，见代办.txt；所有"Commit"步骤替换为"变更记录"）
- **不部署 SimDLL.dll**（编译产物由用户自行复制，用户硬性规则）
- 测试使用全局单例 `G_SIM_DATA`，与现有测试套件同模式，结尾必须 `clean_up()`；若遇全局状态随机干扰，用 `cargo test -- --test-threads=1`

---

## 文件结构

| 文件 | 职责 | 改动 |
|---|---|---|
| `simDLL_RS/src/a_framework/buffer.rs` | 二进制读取器 | 新增 `remaining()` + 4 个 peek 读取（`read_u8_at/read_u16_at/read_i32_at/read_f32_at`），不移动 offset |
| `simDLL_RS/src/a_framework/sim_data.rs` | SimData/CellSOA/BackwallSOA | ① `new_for_allocate` 补 void/unobtanium 索引 + 5 辐射常数；② 新增 `initialize_boundary`；③ 新增 `settle_thermal_boundaries` |
| `simDLL_RS/src/a_framework/save_load.rs` | 消息 handler | 替换 L1068 `handle_initialize_from_cells` 桩为真实实现 + 4 个端到端测试 |

无新建文件。测试全部写在各文件内 `#[cfg(test)] mod tests`。

---

### 任务 1：BinaryBufferReader 增加 remaining + peek 读取

**文件：**
- 修改：`simDLL_RS/src/a_framework/buffer.rs`（impl 块内追加方法，约 L131 `skip` 之后）
- 测试：`simDLL_RS/src/a_framework/buffer.rs`（mod tests 追加）

- [ ] **步骤 1：编写失败的测试**

在 `buffer.rs` 的 `mod tests` 末尾追加：

```rust
    #[test]
    fn remaining_reflects_unread_bytes() {
        let bytes = vec![0u8; 10];
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.remaining(), 10);
        r.skip(4).unwrap();
        assert_eq!(r.remaining(), 6);
    }

    #[test]
    fn read_at_peeks_without_moving_offset() {
        let mut w = BinaryBufferWriter::new();
        w.write_byte(0xAA);
        w.write_ushort(0x1234);
        w.write_int(-7);
        w.write_float(1.5);
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_u8_at(0).unwrap(), 0xAA);
        assert_eq!(r.read_u16_at(1).unwrap(), 0x1234);
        assert_eq!(r.read_i32_at(3).unwrap(), -7);
        assert_eq!(r.read_f32_at(7).unwrap(), 1.5);
        assert_eq!(r.offset(), 0); // peek 不移动偏移
        assert_eq!(r.read_byte().unwrap(), 0xAA); // 顺序读不受影响
        assert!(r.read_u16_at(bytes.len() as u64 - 1).is_err()); // 越界报错
    }
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cd C:\Users\KeringFa\Desktop\simDLL_RS_Kit\simDLL_RS && cargo test --lib a_framework::buffer`
预期：编译失败，报错 `no method named 'remaining'` / `no method named 'read_u8_at'` 等。

- [ ] **步骤 3：编写实现代码**

在 `buffer.rs` 的 `impl BinaryBufferReader` 块内（`skip` 方法之后）追加：

```rust
    /// 剩余可读字节数。
    pub fn remaining(&self) -> u64 {
        self.buffer_size.saturating_sub(self.offset)
    }

    /// 在指定绝对偏移读取 1 字节（不移动当前偏移）。
    pub fn read_u8_at(&self, offset: u64) -> Result<u8, BufferError> {
        if offset + 1 > self.buffer_size {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        Ok(data[offset as usize])
    }

    /// 在指定绝对偏移读取 u16（不移动当前偏移）。
    pub fn read_u16_at(&self, offset: u64) -> Result<u16, BufferError> {
        if offset + 2 > self.buffer_size {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        let o = offset as usize;
        Ok(u16::from_le_bytes([data[o], data[o + 1]]))
    }

    /// 在指定绝对偏移读取 i32（不移动当前偏移）。
    pub fn read_i32_at(&self, offset: u64) -> Result<i32, BufferError> {
        if offset + 4 > self.buffer_size {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        let o = offset as usize;
        Ok(i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]))
    }

    /// 在指定绝对偏移读取 f32（不移动当前偏移）。
    pub fn read_f32_at(&self, offset: u64) -> Result<f32, BufferError> {
        if offset + 4 > self.buffer_size {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        let o = offset as usize;
        Ok(f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]))
    }
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib a_framework::buffer`
预期：PASS，含 2 个新测试共 15 个 buffer 测试全部通过。

- [ ] **步骤 5：变更记录**

在工作日志 `.workbuddy/memory/2026-08-01.md` 追加一行："任务 1 完成：buffer.rs 新增 remaining + read_u8_at/u16_at/i32_at/f32_at（peek 语义）"。**不执行 git 操作。**

---

### 任务 2：new_for_allocate 补缺（void/unobtanium 索引 + 辐射常数）

**文件：**
- 修改：`simDLL_RS/src/a_framework/sim_data.rs`（`new_for_allocate` 函数体，约 L528-531 区域）
- 测试：`simDLL_RS/src/a_framework/sim_data.rs`（mod tests 追加）

- [ ] **步骤 1：编写失败的测试**

在 `sim_data.rs` 的 `mod tests` 末尾追加：

```rust
    #[test]
    fn new_for_allocate_sets_special_indices_and_radiation_constants() {
        let sd = SimData::new_for_allocate(6, 5, 42, true, true);
        // 对照源码 L22385-22390
        assert_eq!(sd.void_element_idx, elements_table::get_element_index_pub(0xa9360b34));
        assert_eq!(sd.vacuum_element_idx, elements_table::get_element_index_pub(0x2d39bf75));
        assert_eq!(sd.unobtanium_element_idx, elements_table::get_element_index_pub(0x6d95058c));
        // 对照源码 L22358-22362
        assert_eq!(sd.radiation_linger_rate, 1.1);
        assert_eq!(sd.radiation_max_mass, 2000.0);
        assert_eq!(sd.radiation_base_weight, 0.3);
        assert_eq!(sd.radiation_density_weight, 0.7);
        assert_eq!(sd.radiation_constructed_factor, 0.8);
        assert!(!sd.init_settle_thermal_boundaries);
    }
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib a_framework::sim_data`
预期：FAIL——`void_element_idx`/`unobtanium_element_idx` 为 0（与 `get_element_index_pub` 返回值不符），辐射常数为 0.0。

- [ ] **步骤 3：编写实现代码**

在 `new_for_allocate` 中，把现有的 vacuum 赋值段（`sd.vacuum_element_idx = elements_table::get_element_index_pub(0x2d39bf75);`）扩展为：

```rust
        // 对照源码 L22385-22390：三个特殊元素索引
        sd.void_element_idx = elements_table::get_element_index_pub(0xa9360b34);
        sd.vacuum_element_idx = elements_table::get_element_index_pub(0x2d39bf75);
        sd.unobtanium_element_idx = elements_table::get_element_index_pub(0x6d95058c);

        // 对照源码 L22358-22362：辐射常数
        sd.radiation_linger_rate = 1.1;
        sd.radiation_max_mass = 2000.0;
        sd.radiation_base_weight = 0.3;
        sd.radiation_density_weight = 0.7;
        sd.radiation_constructed_factor = 0.8;
        // 对照源码 L22355：initSettleThermalBoundaries = false（new_zeroed 已为 false，显式写出以对齐原版）
        sd.init_settle_thermal_boundaries = false;
```

（删除原来的单行 vacuum 赋值与旧注释，替换为以上内容。）

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib a_framework::sim_data`
预期：PASS，新测试通过且既有 sim_data 测试无回归。

- [ ] **步骤 5：变更记录**

在工作日志追加："任务 2 完成：new_for_allocate 补 void/unobtanium 索引 + 5 辐射常数"。**不执行 git 操作。**

---

### 任务 3：SimData::initialize_boundary

**文件：**
- 修改：`simDLL_RS/src/a_framework/sim_data.rs`（impl SimData 块，`do_load_time_state_transition` 之后追加）
- 测试：`simDLL_RS/src/a_framework/sim_data.rs`（mod tests 追加）

依赖：任务 2（需要 unobtanium/vacuum 索引已被设置）。

- [ ] **步骤 1：编写失败的测试**

在 `sim_data.rs` 的 `mod tests` 末尾追加：

```rust
    #[test]
    fn initialize_boundary_sets_four_edges_both_buffers() {
        let mut sd = SimData::new_for_allocate(6, 5, 0, false, false);
        sd.initialize_boundary();
        let unob = elements_table::get_element_index_pub(0x6d95058c);
        let vacuum = elements_table::get_element_index_pub(0x2d39bf75);
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let buf = &*buf_ptr;
                for x in 0..6usize {
                    // 底行（row 0）= unobtanium
                    assert_eq!(buf.element_idx.get(x), unob, "bottom x={}", x);
                    assert_eq!(buf.mass.get(x), 9999.0);
                    assert_eq!(buf.temperature.get(x), 0.0);
                    assert_eq!(buf.disease_count.get(x), 0);
                    assert_eq!(buf.disease_idx.get(x), 0xff);
                    // 顶行（row height-1）= vacuum
                    let top = 4 * 6 + x;
                    assert_eq!(buf.element_idx.get(top), vacuum, "top x={}", x);
                    assert_eq!(buf.mass.get(top), 9999.0);
                }
                for y in 1..4usize {
                    // 左右列 = unobtanium
                    assert_eq!(buf.element_idx.get(y * 6), unob, "left y={}", y);
                    assert_eq!(buf.element_idx.get(y * 6 + 5), unob, "right y={}", y);
                    assert_eq!(buf.mass.get(y * 6), 9999.0);
                }
                // 内部格不受影响（with_size 零值）
                assert_eq!(buf.element_idx.get(8), 0);
                assert_eq!(buf.mass.get(8), 0.0);
            }
            let bw = &*sd.backwall.ptr;
            for x in 0..6usize {
                assert_eq!(bw.element_idx.get(x), bw.vacuum_element_idx);
                assert_eq!(bw.mass.get(x), 0.0);
                assert_eq!(bw.temperature.get(x), 0.0);
            }
        }
    }
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib a_framework::sim_data`
预期：编译失败，`no method named 'initialize_boundary'`。

- [ ] **步骤 3：编写实现代码**

在 `impl SimData` 块内（`do_load_time_state_transition` 方法之后）追加：

```rust
    /// InitializeBoundary — 初始化外围边界格。
    /// 对照源码 11_msvcrt_ignored.c L23949-24195。
    ///
    /// 底行=unobtanium、顶行=vacuum、左右列=unobtanium；
    /// mass=9999.0（0x461C3C00）、temperature=0、病菌清零；
    /// 背墙=vacuum 索引/0/0。cells 与 updated_cells 双缓冲都写。
    pub fn initialize_boundary(&mut self) {
        let width = self.width as usize;
        let height = self.height as usize;
        let unobtanium = self.unobtanium_element_idx;
        let vacuum = self.vacuum_element_idx;

        // 辅助：写一个边界格（双缓冲 + 背墙）
        fn set_boundary_cell(cells: &mut CellSOA, updated: &mut CellSOA, backwall: &mut BackwallSOA, idx: usize, element: u16, bw_vacuum: u16) {
            for buf in [cells, updated] {
                buf.element_idx.set(idx, element);
                buf.mass.set(idx, 9999.0);
                buf.temperature.set(idx, 0.0);
                buf.disease_count.set(idx, 0);
                buf.disease_idx.set(idx, 0xff);
            }
            backwall.element_idx.set(idx, bw_vacuum);
            backwall.mass.set(idx, 0.0);
            backwall.temperature.set(idx, 0.0);
        }

        unsafe {
            let backwall = &mut *self.backwall.ptr;
            let bw_vacuum = backwall.vacuum_element_idx;

            // 底行（row 0）与顶行（row height-1）（对照 L23960-24073）
            for x in 0..width {
                let cells = &mut *self.cells.ptr;
                let updated = &mut *self.updated_cells.ptr;
                set_boundary_cell(cells, updated, backwall, x, unobtanium, bw_vacuum);
                let top = (height - 1) * width + x;
                set_boundary_cell(cells, updated, backwall, top, vacuum, bw_vacuum);
            }
            // 左右列（row 1..height-1）（对照 L24074-24195）
            for y in 1..height.saturating_sub(1) {
                let cells = &mut *self.cells.ptr;
                let updated = &mut *self.updated_cells.ptr;
                set_boundary_cell(cells, updated, backwall, y * width, unobtanium, bw_vacuum);
                set_boundary_cell(cells, updated, backwall, (y + 1) * width - 1, unobtanium, bw_vacuum);
            }
        }
    }
```

（`backwall` 的借用贯穿循环；`cells`/`updated` 每轮循环重新从裸指针解引用获得，规避多重可变借用冲突。）

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib a_framework::sim_data`
预期：PASS，`initialize_boundary_sets_four_edges_both_buffers` 通过。

- [ ] **步骤 5：变更记录**

在工作日志追加："任务 3 完成：SimData::initialize_boundary（四边边界初始化，双缓冲+背墙）"。**不执行 git 操作。**

---

### 任务 4：SimData::settle_thermal_boundaries

**文件：**
- 修改：`simDLL_RS/src/a_framework/sim_data.rs`（impl SimData 块，`initialize_boundary` 之后追加）
- 测试：`simDLL_RS/src/a_framework/sim_data.rs`（mod tests 追加）

- [ ] **步骤 1：编写失败的测试**

在 `sim_data.rs` 的 `mod tests` 末尾追加：

```rust
    #[test]
    fn settle_thermal_boundaries_averages_neighbor_temperatures() {
        let mut sd = SimData::new_for_allocate(6, 5, 0, false, false);
        let void_idx = elements_table::get_element_index_pub(0x3fe0146e);
        let unob_idx = elements_table::get_element_index_pub(0x6d95058c);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            // 目标格 (row=2, col=2) → idx = 2*6+2 = 14，设为 Void
            cells.element_idx.set(14, void_idx);
            // 8 邻居：7, 8, 9（上排 row1 col1-3）；13, 15（左右）；19, 20, 21（下排 row3 col1-3）
            let neighbors: [(usize, f32); 8] = [
                (7, 100.0), (8, 200.0), (9, 300.0),
                (13, 400.0), (15, 500.0),
                (19, 600.0), (20, 700.0), (21, 800.0),
            ];
            for (idx, temp) in neighbors {
                cells.element_idx.set(idx, 1); // 非 void/unob 普通元素
                cells.mass.set(idx, 1.0);
                cells.temperature.set(idx, temp);
            }
            // idx 8 无质量 → 应被排除
            cells.mass.set(8, 0.0);
            // idx 9 是 unobtanium → 应被排除
            cells.element_idx.set(9, unob_idx);
        }
        sd.settle_thermal_boundaries();
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            // 排除 idx 8（mass=0）与 idx 9（unobtanium）后剩 6 个邻居：
            // 均值 = (100+400+500+600+700+800) / 6 = 3100/6
            let expected = (100.0f32 + 400.0 + 500.0 + 600.0 + 700.0 + 800.0) / 6.0;
            let got = updated.temperature.get(14);
            assert!((got - expected).abs() < 0.01, "expected {}, got {}", expected, got);
        }
    }
```

> 说明：测试环境元素表为空，`get_element_index_pub` 对两个哈希都返回 0xFFFF——此时 void_idx == unob_idx，但排除逻辑（`e == void || e == unob`）对同值邻居同样排除，测试意图不受影响。
> 修正记录：期望值原为 3400/7（误将必被排除的 unob 邻居 idx 9 计入），实现阶段经审查独立核算更正为 3100/6。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib a_framework::sim_data`
预期：编译失败，`no method named 'settle_thermal_boundaries'`。

- [ ] **步骤 3：编写实现代码**

在 `impl SimData` 块内（`initialize_boundary` 之后）追加：

```rust
    /// SettleThermalBoundaries — Void 格温度取邻居均值。
    /// 对照源码 11_msvcrt_ignored.c L24803-24898。
    ///
    /// 读 cells、写 updated_cells：内部格中元素为 0x3fe0146e 的格子，
    /// 温度 = 8 邻居中（排除 0x3fe0146e 与 unobtanium 0x6d95058c，且 mass>0）的温度均值。
    pub fn settle_thermal_boundaries(&mut self) {
        let width = self.width as usize;
        let height = self.height as usize;
        let void_idx = elements_table::get_element_index_pub(0x3fe0146e);
        let unobtanium_idx = elements_table::get_element_index_pub(0x6d95058c);
        let w = width as i32;
        let offsets: [i32; 8] = [-w - 1, -w, 1 - w, -1, 1, w - 1, w, w + 1];
        unsafe {
            let cells = &*self.cells.ptr;
            let updated = &mut *self.updated_cells.ptr;
            // 行 1..height-1、列 1..width-1（对照 C 循环 iVar14 ∈ [1, height-2]）
            for row in 1..height.saturating_sub(1) {
                for col in 1..width.saturating_sub(1) {
                    let idx = row * width + col;
                    if cells.element_idx.get(idx) != void_idx {
                        continue;
                    }
                    let mut sum = 0.0f32;
                    let mut count = 0.0f32;
                    for off in offsets {
                        let nidx = (idx as i32 + off) as usize;
                        let e = cells.element_idx.get(nidx);
                        if e == void_idx || e == unobtanium_idx {
                            continue;
                        }
                        if cells.mass.get(nidx) > 0.0 {
                            count += 1.0;
                            sum += cells.temperature.get(nidx);
                        }
                    }
                    if count > 0.0 {
                        updated.temperature.set(idx, sum / count);
                    }
                }
            }
        }
    }
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib a_framework::sim_data`
预期：PASS，`settle_thermal_boundaries_averages_neighbor_temperatures` 通过。

- [ ] **步骤 5：变更记录**

在工作日志追加："任务 4 完成：SimData::settle_thermal_boundaries（Void 格温度邻居均值）"。**不执行 git 操作。**

---

### 任务 5：handle_initialize_from_cells 主体

**文件：**
- 修改：`simDLL_RS/src/a_framework/save_load.rs`（L1065-1072 桩替换为真实实现；mod tests 追加）
- 测试：`simDLL_RS/src/a_framework/save_load.rs`（mod tests 追加；需在 tests 的 use 区加 `use crate::a_framework::buffer::BinaryBufferWriter;`）

依赖：任务 1/2/3/4 全部完成。

- [ ] **步骤 1：编写失败的测试**

在 `save_load.rs` 的 `mod tests` 中追加辅助函数与 4 个测试。

> 修正记录（实现阶段）：① 简报记者原写的 `(*G_SIM_DATA).0` 有误——`G_SIM_DATA` 是 `parking_lot::Mutex`，正确写法为 `globals::G_SIM_DATA.lock().0`（同 handle_allocate_cells 模式）；② 每个测试需遵循同文件既有模式：开头 `let _lock = LIB_TESTS_LOCK.lock().unwrap(); clean_up();`、结尾 `clean_up()`；③ 需在 tests 的 use 区加 `use crate::LIB_TESTS_LOCK;` 与 `use crate::a_framework::buffer::BinaryBufferWriter;`。

```rust
    /// 构造 4×3 世界的 InitializeFromCells 合成消息（总长 14 + 12×40 = 494 字节）。
    fn build_initialize_from_cells_message() -> Vec<u8> {
        let mut w = BinaryBufferWriter::new();
        w.write_int(4); // width
        w.write_int(3); // height
        w.write_uint(0x12345678); // simSeed
        w.write_bool(true); // radiationEnabled
        w.write_bool(true); // headless
        // 块 1：Sim.Cell × 12（每格 16B）
        for i in 0..12u16 {
            w.write_ushort(100 + i); // elementIdx @0
            w.write_byte(0); // @2 → properties
            w.write_byte(50 + i as u8); // insulation @3
            w.write_byte(0); // @4 → strength_info
            w.write_byte(0);
            w.write_byte(0);
            w.write_byte(0); // pad @5-7
            w.write_float(300.0 + i as f32); // temperature @8
            w.write_float(1000.0 + i as f32); // mass @12
        }
        // 块 2：DiseaseCell × 12（每格 12B）
        for i in 0..12u8 {
            w.write_byte(200 + i); // diseaseIdx @0
            w.write_byte(7); // tick @1
            w.write_byte(0);
            w.write_byte(0); // pad @2-3
            w.write_int(5000 + i as i32); // count @4
            w.write_float(9.9); // accErr @8（应被忽略，写 0）
        }
        // 块 3：SimBackwall × 12（每格 12B）
        for i in 0..12u16 {
            w.write_ushort(30 + i); // elementIdx @0
            w.write_byte(0);
            w.write_byte(0); // pad @2-3
            w.write_float(2000.0 + i as f32); // mass @4
            w.write_float(280.0 + i as f32); // temperature @8
        }
        w.into_bytes()
    }

    #[test]
    fn handle_initialize_from_cells_populates_sim_data() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();
        let data = build_initialize_from_cells_message();
        let mut reader = BinaryBufferReader::new(&data);
        let result = handle_initialize_from_cells(&mut reader);
        assert_eq!(result as usize, 12); // 返回值 = width*height
        let ptr = globals::G_SIM_DATA.lock().0;
        assert!(!ptr.is_null());
        unsafe {
            let sd = &*ptr;
            assert_eq!(sd.width, 6); // 4+2
            assert_eq!(sd.height, 5); // 3+2
            assert_eq!(sd.num_game_cells, 12);
            assert_eq!(sd.random_seed, 0x12345678);
            assert!(sd.radiation_enabled);
            assert!(sd.headless);
            let cells = &*sd.updated_cells.ptr;
            let backwall = &*sd.backwall.ptr;
            for row in 0..3usize {
                for col in 0..4usize {
                    let i = row * 4 + col;
                    let dst = (row + 1) * 6 + (col + 1);
                    assert_eq!(cells.element_idx.get(dst), 100 + i as u16, "element_idx @{}", i);
                    assert_eq!(cells.properties.get(dst), 0);
                    assert_eq!(cells.insulation.get(dst), 50 + i as u8);
                    assert_eq!(cells.strength_info.get(dst), 0);
                    assert_eq!(cells.temperature.get(dst), 300.0 + i as f32);
                    assert_eq!(cells.mass.get(dst), 1000.0 + i as f32);
                    assert_eq!(cells.radiation.get(dst), 0.0);
                    assert_eq!(cells.disease_idx.get(dst), 200 + i as u8);
                    assert_eq!(cells.disease_infestation_tick_count.get(dst), 7);
                    assert_eq!(cells.disease_count.get(dst), 5000 + i as i32);
                    assert_eq!(cells.disease_growth_accumulated_error.get(dst), 0.0);
                    assert_eq!(backwall.element_idx.get(dst), 30 + i as u16);
                    assert_eq!(backwall.mass.get(dst), 2000.0 + i as f32);
                    assert_eq!(backwall.temperature.get(dst), 280.0 + i as f32);
                }
            }
        }
        clean_up();
    }

    #[test]
    fn handle_initialize_from_cells_initializes_boundary() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();
        let data = build_initialize_from_cells_message();
        let mut reader = BinaryBufferReader::new(&data);
        handle_initialize_from_cells(&mut reader);
        let ptr = globals::G_SIM_DATA.lock().0;
        assert!(!ptr.is_null());
        unsafe {
            let sd = &*ptr;
            let unob = elements_table::get_element_index_pub(0x6d95058c);
            let vacuum = elements_table::get_element_index_pub(0x2d39bf75);
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let buf = &*buf_ptr;
                for x in 0..6usize {
                    assert_eq!(buf.element_idx.get(x), unob, "bottom x={}", x);
                    assert_eq!(buf.mass.get(x), 9999.0);
                    let top = 4 * 6 + x;
                    assert_eq!(buf.element_idx.get(top), vacuum, "top x={}", x);
                }
                for y in 1..4usize {
                    assert_eq!(buf.element_idx.get(y * 6), unob, "left y={}", y);
                    assert_eq!(buf.element_idx.get(y * 6 + 5), unob, "right y={}", y);
                }
            }
        }
        clean_up();
    }

    #[test]
    fn handle_initialize_from_cells_rejects_truncated_data() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();
        let mut data = build_initialize_from_cells_message();
        data.truncate(data.len() - 1); // 少 1 字节
        let mut reader = BinaryBufferReader::new(&data);
        let result = handle_initialize_from_cells(&mut reader);
        assert!(result.is_null());
        assert!(globals::G_SIM_DATA.lock().0.is_null(), "旧 gSimData 不应被替换");
        clean_up();
    }

    #[test]
    fn handle_initialize_from_cells_replaces_old_sim_data() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();
        // 先经 AllocateCells 放一个旧 SimData
        let mut w = BinaryBufferWriter::new();
        w.write_int(4);
        w.write_int(4);
        w.write_bool(true);
        w.write_bool(true);
        let alloc_data = w.into_bytes();
        let mut alloc_reader = BinaryBufferReader::new(&alloc_data);
        handle_allocate_cells(&mut alloc_reader);
        let old_ptr = globals::G_SIM_DATA.lock().0;
        assert!(!old_ptr.is_null());

        let data = build_initialize_from_cells_message();
        let mut reader = BinaryBufferReader::new(&data);
        handle_initialize_from_cells(&mut reader);
        let new_ptr = globals::G_SIM_DATA.lock().0;
        assert!(!new_ptr.is_null());
        assert!(new_ptr != old_ptr, "应替换为新 SimData");
        unsafe {
            assert_eq!((*new_ptr).width, 6);
        }
        clean_up();
    }
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --lib a_framework::save_load`
预期：FAIL——`handle_initialize_from_cells_populates_sim_data` 断言 `result as usize == 12` 失败（桩返回 null）。

- [ ] **步骤 3：编写实现代码**

把 `save_load.rs` L1065-1072 的桩整体替换为：

```rust
/// SimData_InitializeFromCells handler。
/// 对照源码 11_msvcrt_ignored.c L24200-24421。
///
/// 消息布局：width(i32) + height(i32) + simSeed(u32) + radiationEnabled(bool) + headless(bool)
/// + Sim.Cell[N]×16B + DiseaseCell[N]×12B + SimBackwall[N]×12B（N = width×height）。
/// 元素为 u16 索引（非哈希，无需查表转换）；数据写入 updated_cells / backwall；
/// 内部格索引 = (row+1)×sim_width + (col+1)。末尾调 SettleThermalBoundaries + InitializeBoundary。
/// 返回值：格子总数 width*height（原版语义；C# 侧忽略）。
pub fn handle_initialize_from_cells(reader: &mut BinaryBufferReader) -> *mut c_void {
    // 读头（对照 L24226-24230）
    let width = match reader.read_int() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read width failed: {:?}", e); return std::ptr::null_mut(); }
    };
    let height = match reader.read_int() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read height failed: {:?}", e); return std::ptr::null_mut(); }
    };
    let sim_seed = match reader.read_uint() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read simSeed failed: {:?}", e); return std::ptr::null_mut(); }
    };
    let radiation_enabled = match reader.read_bool() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read radiation flag failed: {:?}", e); return std::ptr::null_mut(); }
    };
    let headless = match reader.read_bool() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read headless flag failed: {:?}", e); return std::ptr::null_mut(); }
    };

    // 参数与长度校验（增强：原版无检查；不满足时 gSimData 保持原样）
    if width <= 0 || height <= 0 {
        tracing::error!("InitializeFromCells: invalid size {}x{}", width, height);
        return std::ptr::null_mut();
    }
    let num_cells = (width as usize) * (height as usize);
    let needed = num_cells * 40;
    if (reader.remaining() as usize) < needed {
        tracing::error!("InitializeFromCells: buffer too small: remaining {} < needed {}", reader.remaining(), needed);
        return std::ptr::null_mut();
    }

    // 三块基址（对照 L24232-24236：记录位置 + skip）
    let cells_base = reader.offset();
    if reader.skip(num_cells * 16).is_err() {
        return std::ptr::null_mut();
    }
    let disease_base = reader.offset();
    if reader.skip(num_cells * 12).is_err() {
        return std::ptr::null_mut();
    }
    let backwall_base = reader.offset();

    // 建 SimData 替换全局（对照 L24237-24249，替换模式同 handle_allocate_cells）
    let new_sim_data = Box::new(SimData::new_for_allocate(
        width + 2,
        height + 2,
        sim_seed,
        radiation_enabled,
        headless,
    ));
    let new_ptr = Box::into_raw(new_sim_data);
    // guard 持有至函数返回：保护初始化完成前 gSimData 不被并发观察到半成品。
    // 锁序安全（G_SIM_DATA → G_ELEMENTS_TABLE 单向，全库无反向边）。
    let mut global_sim_data = globals::G_SIM_DATA.lock();
    if !global_sim_data.0.is_null() {
        unsafe {
            let _ = Box::from_raw(global_sim_data.0);
        }
    }
    global_sim_data.0 = new_ptr;

    unsafe {
        let sim_data = &mut *new_ptr;
        let sim_width = sim_data.width as usize;
        let cells_ptr = sim_data.updated_cells.ptr;
        let backwall_ptr = sim_data.backwall.ptr;
        let cells = &mut *cells_ptr;
        let backwall = &mut *backwall_ptr;
        let msg_width = width as usize;

        // 循环 1：Sim.Cell → updated_cells（对照 L24252-24322）
        for row in 0..height as usize {
            for col in 0..msg_width {
                let src = cells_base + ((row * msg_width + col) * 16) as u64;
                let dst = (row + 1) * sim_width + (col + 1);
                cells.element_idx.set(dst, reader.read_u16_at(src).unwrap_or(0));
                cells.properties.set(dst, reader.read_u8_at(src + 2).unwrap_or(0));
                cells.insulation.set(dst, reader.read_u8_at(src + 3).unwrap_or(0));
                cells.strength_info.set(dst, reader.read_u8_at(src + 4).unwrap_or(0));
                cells.temperature.set(dst, reader.read_f32_at(src + 8).unwrap_or(0.0));
                cells.mass.set(dst, reader.read_f32_at(src + 12).unwrap_or(0.0));
                cells.radiation.set(dst, 0.0);
            }
        }
        // 循环 2：DiseaseCell → updated_cells（对照 L24323-24372）
        for row in 0..height as usize {
            for col in 0..msg_width {
                let src = disease_base + ((row * msg_width + col) * 12) as u64;
                let dst = (row + 1) * sim_width + (col + 1);
                cells.disease_idx.set(dst, reader.read_u8_at(src).unwrap_or(0));
                cells.disease_infestation_tick_count.set(dst, reader.read_u8_at(src + 1).unwrap_or(0));
                cells.disease_count.set(dst, reader.read_i32_at(src + 4).unwrap_or(0));
                cells.disease_growth_accumulated_error.set(dst, 0.0);
            }
        }
        // 循环 3：SimBackwall → backwall（对照 L24373-24415）
        for row in 0..height as usize {
            for col in 0..msg_width {
                let src = backwall_base + ((row * msg_width + col) * 12) as u64;
                let dst = (row + 1) * sim_width + (col + 1);
                backwall.element_idx.set(dst, reader.read_u16_at(src).unwrap_or(0));
                backwall.mass.set(dst, reader.read_f32_at(src + 4).unwrap_or(0.0));
                backwall.temperature.set(dst, reader.read_f32_at(src + 8).unwrap_or(0.0));
            }
        }

        // Settle + InitializeBoundary（对照 L24416-24419）
        sim_data.settle_thermal_boundaries();
        sim_data.initialize_boundary();
    }

    tracing::info!("InitializeFromCells done: {}x{} ({} cells), seed={:#010x}", width, height, num_cells, sim_seed);
    num_cells as *mut c_void
}
```

（`cells`/`backwall` 借自局部裸指针解引用，不与 `sim_data` 的可变借用冲突；peek 读取已经长度校验，`unwrap_or` 仅作防御。）

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib a_framework::save_load`
预期：PASS，4 个新测试全部通过，既有 save_load 测试无回归。

- [ ] **步骤 5：变更记录**

在工作日志追加："任务 5 完成：handle_initialize_from_cells 真实实现（三循环 + Settle + InitializeBoundary），4 个端到端测试通过"。**不执行 git 操作。**

---

### 任务 6：全量回归 + Release 编译 + 交付

**文件：** 无代码改动。

- [ ] **步骤 1：全量测试**

运行：`cd C:\Users\KeringFa\Desktop\simDLL_RS_Kit\simDLL_RS && cargo test`
预期：全部测试通过（此前基线为 188 lib + 集成；新增 2+1+1+1+4 = 9 个 lib 测试）。若有全局状态随机干扰，改跑 `cargo test -- --test-threads=1`。

- [ ] **步骤 2：Release 编译**

运行：`cargo build --release`
预期：编译成功，产物 `simDLL_RS\target\release\SimDLL.dll`。

- [ ] **步骤 3：交付说明（不部署）**

告知用户：产物在 `simDLL_RS\target\release\SimDLL.dll`，请自行复制到 `OxygenNotIncluded_Data\Plugins\x86_64\SimDLL.dll` 后实测。实测验收流程：
1. 新开游戏 → 观察日志应依次出现：`InitializeFromCells done` → `Start done` → 500 步 `NewGameFrame`/`PrepareGameData` → `BeginSave done`
2. 通过世界生成 → 进入初始小人选定 → 存档命名
3. 进世界后挖掘/沙盒工具正常
4. （眼冒金星集群）多个小行星依次生成时观察第二轮起是否有异常

- [ ] **步骤 4：更新代办.txt**

把代办.txt 中"新游戏生成"条目的状态从"未开始"更新为"已实现待实测"，并附上本计划与设计文档路径。

---

## 自检结果

**规格覆盖度：** 规格 §2 布局 → 任务 5；§3.1 InitializeBoundary → 任务 3；§3.2 SettleThermalBoundaries → 任务 4；§3.3 new_for_allocate 补缺 → 任务 2；§4 buffer 能力 → 任务 1；§5 错误处理 → 任务 5 校验段；§6 测试 1-5 → 任务 3/4/5，测试 6 实测 → 任务 6 步骤 3；§7 SetBackwallData 明确留作后续（不在本计划）。无遗漏。

**占位符扫描：** 所有步骤含完整代码/命令/预期输出，无"待定/TODO/类似任务 N"类缺陷。

**类型一致性：** `remaining`/`read_u8_at`/`read_u16_at`/`read_i32_at`/`read_f32_at`（任务 1 定义，任务 5 使用）一致；`initialize_boundary`/`settle_thermal_boundaries`（任务 3/4 定义，任务 5 调用）一致；`BinaryBufferWriter` 的 `write_int/write_uint/write_bool/write_ushort/write_float/write_byte` 与 buffer.rs 现有 API 一致；测试断言用 `MsvcVector::get`（stl_shim.rs L136 存在）。
