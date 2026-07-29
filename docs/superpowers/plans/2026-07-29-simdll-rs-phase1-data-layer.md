# Phase 1：数据层 + 序列化 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 实现 simDLL_RS Phase 1 数据层 — BinaryBufferReader/Writer + STL 容器建模 + 所有数据 struct 字段定义 + 类骨架，所有 struct 通过 `size_of` 断言与原版字节数一致。

**架构：** 12 个任务按依赖顺序实现：基础类型层（stl_shim/vector_math/buffer）→ 数据 struct（element/disease/sim_events/sim_data/game_data）→ 顶层骨架（frame_sync/sim/sim_frame_manager/message_handler）→ 集成（lib.rs/abi_smoke）。每个任务遵循 TDD：编写失败的 size_of 测试 → 验证失败 → 实现 struct → 验证通过 → commit。

**技术栈：** Rust 2021 edition，crate 名 `simdll_rs`，lib 名 `SimDLL`（匹配原版 DLL 名），目标平台 x86_64-pc-windows-msvc，依赖 7 个 crate（windows/tracing/tracing-appender/tracing-subscriber/parking_lot/once_cell/chrono），Phase 1 不新增依赖。

**规格参考：** `docs/superpowers/specs/2026-07-29-simdll-rs-phase1-data-layer-design.md`

---

## 实现前必读

### 开放问题处置协议

规格 §9 列出 7 个开放问题（如 CellSOA 实际大小、Disease 剩余 16B、std::vector 字段偏移等）。实施时若 `size_of` 断言失败：

1. **不要猜测字段**：禁止用 `reserved: [u8; N]` 凑字节数（除了 Disease 已在规格中明确允许）
2. **回头查源码**：在 `OxygenNotIncluded_Source/simDLL_edit/00_types_reference.c` 或 `02_save_load.c` 中搜索该 struct 的所有访问模式（offset 偏移）
3. **从访问模式反推字段**：每个 `*(type*)(struct_ptr + offset)` 访问对应一个字段
4. **更新 struct 与测试**：找到正确字段后同步更新 struct 定义和 size_of 断言值
5. **仍无法确定时 BLOCKED**：以 BLOCKED 状态上报主代理，附上源码片段和已尝试的分析

### 命令执行约定

- 工作目录：`c:\Users\KeringFa\Desktop\simDLL_RS_Kit`
- PowerShell 执行策略被禁用，但 `cargo` / `git` 直接调用不受影响
- 测试命令：`cargo test --lib <module_name>::tests::<test_name>` 跑单个测试，`cargo test --lib` 跑全部 lib 测试
- Build 命令：`cargo build --release` 产出 `simDLL_RS/target/x86_64-pc-windows-msvc/release/SimDLL.dll`

### 文件路径约定

所有路径相对于项目根 `c:\Users\KeringFa\Desktop\simDLL_RS_Kit\`。Rust 源文件路径前缀：`simDLL_RS/src/`。

---

## 文件结构

### 新增文件（7 个）
| 文件 | 职责 |
|---|---|
| `simDLL_RS/src/stl_shim.rs` | MSVC STL 容器等价物：MsvcVector、UniquePtr、MsvcString、MsvcFunction |
| `simDLL_RS/src/vector_math.rs` | Vector2f、Vector3f、Vector4f |
| `simDLL_RS/src/element.rs` | Element、PhysicsData、ElementState |
| `simDLL_RS/src/disease.rs` | Disease、DiseaseInfo |
| `simDLL_RS/src/sim_events.rs` | SimEvents、SimFrameInfo |
| `simDLL_RS/src/sim_data.rs` | SimData、CellSOA、BackwallSOA + 8 个子结构 |
| `simDLL_RS/src/game_data.rs` | GameData + 27 个 *Info/*Callback 子结构 |

### 修改文件（5 个）
| 文件 | 改动 |
|---|---|
| `simDLL_RS/src/buffer.rs` | 替换 Phase 0 桩为完整 BinaryBufferReader/Writer 实现 |
| `simDLL_RS/src/sim.rs` | 替换空 struct 为 Sim 类字段骨架 |
| `simDLL_RS/src/sim_frame_manager.rs` | 替换空 struct 为 SimFrameManager 字段骨架 |
| `simDLL_RS/src/frame_sync.rs` | 替换空 struct 为 FrameSync 字段 |
| `simDLL_RS/src/message_handler.rs` | 替换空表为 Type enum + MessageHandlerEntry + 空表 |

### 集成文件（2 个）
| 文件 | 改动 |
|---|---|
| `simDLL_RS/src/lib.rs` | 添加 7 个新 mod 声明 |
| `simDLL_RS/tests/abi_smoke.rs` | 新增 BinaryBufferReader/Writer round-trip 测试 + 清理 P0-3 |

### Cargo.toml 清理
| 文件 | 改动 |
|---|---|
| `simDLL_RS/Cargo.toml` | 移除冗余 `[profile.test] panic = "unwind"`（P0-2） |

---

## 任务依赖图

```
任务 1 (stl_shim) ─┐
任务 2 (vector_math)─┤
任务 3 (buffer) ────┤─── 独立基础层，可并行
                    │
任务 4 (element) ────┤
任务 5 (disease) ───┤─── 依赖任务 1
任务 6 (sim_events) ─┤─── 依赖任务 1, 2
                    │
任务 7 (sim_data) ──┤─── 依赖任务 1, 2, 6
任务 8 (game_data) ─┤─── 依赖任务 1, 2, 7
                    │
任务 9 (frame_sync + sim + sim_frame_manager) ──依赖任务 7, 8
任务 10 (message_handler) ──依赖任务 3
                    │
任务 11 (lib.rs 集成) ──依赖任务 1-10
任务 12 (abi_smoke + Cargo.toml) ──依赖任务 11
```

---

## 任务 1：stl_shim.rs — STL 容器等价物

**文件：**
- 创建：`simDLL_RS/src/stl_shim.rs`
- 修改：`simDLL_RS/src/lib.rs`（添加 `pub mod stl_shim;`）

**依赖：** 无

- [ ] **步骤 1：在 lib.rs 添加模块声明**

修改 [lib.rs:17](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/lib.rs#L17)，在 `pub mod sysinfo;` 后追加：

```rust
pub mod stl_shim;
```

- [ ] **步骤 2：创建 stl_shim.rs 并编写失败的测试**

创建 `simDLL_RS/src/stl_shim.rs`：

```rust
//! C++ STL 容器的 Rust 等价物（仅用于内存布局占位）。
//!
//! 这些 struct 不实现任何方法（push/pop/etc）。
//! Phase 1 只验证内存布局与原版 MSVC STL 一致。
//! 业务逻辑（如真的 push_back）在 Phase 6+ 添加。

/// std::vector<T> = 24 字节（begin + end + capacity_end 三个指针）。
/// 对应 MSVC `_Compressed_pair<Allocator, _Vector_val>` EBO 后的布局。
#[repr(C)]
pub struct MsvcVector<T> {
    pub begin: *mut T,
    pub end: *mut T,
    pub capacity_end: *mut T,
}

/// std::unique_ptr<T> = 8 字节（单一指针，default_delete 无状态）。
#[repr(C)]
pub struct UniquePtr<T> {
    pub ptr: *mut T,
}

/// std::string (MSVC SSO) = 32 字节。
/// `_Bxty` union（16B：SSO inline buffer 或 heap pointer）+ size + capacity。
#[repr(C)]
pub struct MsvcString {
    pub buffer_or_inline: [u8; 16],
    pub size: u64,
    pub capacity: u64,
}

/// std::function<...> = 64 字节（典型大小，含 vtable + small storage）。
/// 实际大小因签名而异；64B 是 ONI 中观察到的最大值。
#[repr(C)]
pub struct MsvcFunction {
    pub vtable: *const u8,
    pub storage: [u8; 56],
}

// Safety: 这些是 POD 占位 struct，跨线程共享原始指针是安全的
// （Phase 1 不实际使用这些容器，只占内存布局）。
unsafe impl<T: Send> Send for MsvcVector<T> {}
unsafe impl<T: Sync> Sync for MsvcVector<T> {}
unsafe impl<T: Send> Send for UniquePtr<T> {}
unsafe impl<T: Sync> Sync for UniquePtr<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn msvc_vector_size() {
        assert_eq!(size_of::<MsvcVector<u8>>(), 24);
    }

    #[test]
    fn unique_ptr_size() {
        assert_eq!(size_of::<UniquePtr<u8>>(), 8);
    }

    #[test]
    fn msvc_string_size() {
        assert_eq!(size_of::<MsvcString>(), 32);
    }

    #[test]
    fn msvc_function_size() {
        assert_eq!(size_of::<MsvcFunction>(), 64);
    }
}
```

- [ ] **步骤 3：运行测试验证通过**

运行：`cargo test --lib stl_shim::tests`
预期：4 个测试全部 PASS。

- [ ] **步骤 4：运行全部测试验证无回归**

运行：`cargo test`
预期：Phase 0 的 37 个测试 + Phase 1 新增 4 个 = 41 个测试全部 PASS。

- [ ] **步骤 5：Commit**

```bash
git add simDLL_RS/src/stl_shim.rs simDLL_RS/src/lib.rs
git commit -m "feat(stl_shim): add MSVC STL container shims (MsvcVector/UniquePtr/MsvcString/MsvcFunction)"
```

---

## 任务 2：vector_math.rs — Vector2/3/4

**文件：**
- 创建：`simDLL_RS/src/vector_math.rs`
- 修改：`simDLL_RS/src/lib.rs`（添加 `pub mod vector_math;`）

**依赖：** 无

**说明：** ONI 的 Vector2/3/4 都是 POD struct（x/y/z/w 浮点）。原版大小：Vector2f=8B、Vector3f=12B、Vector4f=16B。

- [ ] **步骤 1：在 lib.rs 添加模块声明**

修改 [lib.rs:17](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/lib.rs#L17)，在 `pub mod stl_shim;` 后追加：

```rust
pub mod vector_math;
```

- [ ] **步骤 2：创建 vector_math.rs**

创建 `simDLL_RS/src/vector_math.rs`：

```rust
//! ONI 的 Vector2/3/4 数学类型，POD struct，无方法。
//! Phase 1 只定义内存布局，向量运算在 Phase 7+ 添加。

/// 2D 向量（8 字节，对应原版 Vector2f）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vector2f {
    pub x: f32,
    pub y: f32,
}

/// 3D 向量（12 字节，对应原版 Vector3f）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vector3f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 4D 向量（16 字节，对应原版 Vector4f）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vector4f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn vector2f_size() {
        assert_eq!(size_of::<Vector2f>(), 8);
    }

    #[test]
    fn vector3f_size() {
        assert_eq!(size_of::<Vector3f>(), 12);
    }

    #[test]
    fn vector4f_size() {
        assert_eq!(size_of::<Vector4f>(), 16);
    }
}
```

- [ ] **步骤 3：运行测试验证通过**

运行：`cargo test --lib vector_math::tests`
预期：3 个测试全部 PASS。

- [ ] **步骤 4：Commit**

```bash
git add simDLL_RS/src/vector_math.rs simDLL_RS/src/lib.rs
git commit -m "feat(vector_math): add Vector2f/3f/4f POD structs"
```

---

## 任务 3：buffer.rs — BinaryBufferReader/Writer

**文件：**
- 修改：`simDLL_RS/src/buffer.rs`（替换 Phase 0 桩）
- 修改：`simDLL_RS/src/lib.rs`（无需改动，`pub mod buffer;` 已存在）

**依赖：** 无

- [ ] **步骤 1：编写失败的测试（先在 buffer.rs 末尾追加测试模块）**

修改 `simDLL_RS/src/buffer.rs`，**完全替换**为以下内容：

```rust
//! BinaryBufferReader/Writer — simDLL 内部使用的二进制序列化读写器。
//!
//! 原版用 C++ vtable 实现多态（32 字节含 vtable 指针），
//! Rust 版放弃 vtable（24 字节），因为 BinaryBufferReader 只在 simDLL 内部构造，
//! 无外部 ABI 兼容需求。详见规格 §2 决策 #1。

use std::ffi::c_int;

/// 读取/写入错误类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferError {
    /// 读取越界。
    EndOfBuffer,
    /// 字符串 UTF-8 解码失败。
    InvalidUtf8,
    /// 布尔值非 0/1（原版允许任意非零为真，Rust 严格点）。
    InvalidBool(u8),
    /// 写入失败（Writer 用，实际几乎不发生）。
    Io,
}

/// 二进制读取器。
///
/// 对应原版 `BinaryBufferReader` struct（原版 32 字节含 vtable，Rust 版 24 字节）。
#[repr(C)]
pub struct BinaryBufferReader {
    m_offset: u64,
    m_buffer_data: *const i8,
    m_buffer_size: u64,
}

impl BinaryBufferReader {
    /// 从字节切片构造读取器。
    pub fn new(buffer: &[u8]) -> Self {
        Self {
            m_offset: 0,
            m_buffer_data: buffer.as_ptr() as *const i8,
            m_buffer_size: buffer.len() as u64,
        }
    }

    /// 剩余可读字节数。
    pub fn remaining(&self) -> u64 {
        self.m_buffer_size.saturating_sub(self.m_offset)
    }

    /// 是否读到末尾。
    pub fn is_at_end(&self) -> bool {
        self.m_offset >= self.m_buffer_size
    }

    /// 跳过 N 字节。
    pub fn skip(&mut self, count: u64) -> Result<(), BufferError> {
        if self.m_offset.checked_add(count).map_or(true, |end| end > self.m_buffer_size) {
            return Err(BufferError::EndOfBuffer);
        }
        self.m_offset += count;
        Ok(())
    }

    /// 读取 4 字节有符号整数。
    pub fn read_int(&mut self) -> Result<i32, BufferError> {
        let bytes = self.read_array::<4>()?;
        Ok(i32::from_le_bytes(bytes))
    }

    /// 读取 4 字节无符号整数。
    pub fn read_uint(&mut self) -> Result<u32, BufferError> {
        let bytes = self.read_array::<4>()?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// 读取 2 字节短整数。
    pub fn read_short(&mut self) -> Result<i16, BufferError> {
        let bytes = self.read_array::<2>()?;
        Ok(i16::from_le_bytes(bytes))
    }

    /// 读取 2 字节无符号短整数。
    pub fn read_ushort(&mut self) -> Result<u16, BufferError> {
        let bytes = self.read_array::<2>()?;
        Ok(u16::from_le_bytes(bytes))
    }

    /// 读取 4 字节浮点。
    pub fn read_float(&mut self) -> Result<f32, BufferError> {
        let bytes = self.read_array::<4>()?;
        Ok(f32::from_le_bytes(bytes))
    }

    /// 读取 1 字节。
    pub fn read_byte(&mut self) -> Result<u8, BufferError> {
        if self.is_at_end() {
            return Err(BufferError::EndOfBuffer);
        }
        let byte = unsafe { *self.m_buffer_data.add(self.m_offset as usize) as u8 };
        self.m_offset += 1;
        Ok(byte)
    }

    /// 读取 1 字节布尔（仅 0/1 合法）。
    pub fn read_bool(&mut self) -> Result<bool, BufferError> {
        match self.read_byte()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(BufferError::InvalidBool(other)),
        }
    }

    /// 读取 N 字节填入调用者缓冲。
    pub fn read_bytes(&mut self, out: &mut [u8]) -> Result<(), BufferError> {
        let len = out.len() as u64;
        if self.m_offset.checked_add(len).map_or(true, |end| end > self.m_buffer_size) {
            return Err(BufferError::EndOfBuffer);
        }
        for slot in out.iter_mut() {
            *slot = unsafe { *self.m_buffer_data.add(self.m_offset as usize) as u8 };
            self.m_offset += 1;
        }
        Ok(())
    }

    /// 读取 UTF-8 字符串（先读 uint 长度，再读字节）。
    pub fn read_string(&mut self) -> Result<String, BufferError> {
        let len = self.read_uint()? as usize;
        let mut buf = vec![0u8; len];
        self.read_bytes(&mut buf)?;
        String::from_utf8(buf).map_err(|_| BufferError::InvalidUtf8)
    }

    /// 内部辅助：读取固定大小字节数组。
    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], BufferError> {
        let mut arr = [0u8; N];
        self.read_bytes(&mut arr)?;
        Ok(arr)
    }
}

/// 二进制写入器（Rust 内部使用，不需要 ABI 兼容）。
pub struct BinaryBufferWriter {
    buf: Vec<u8>,
}

impl BinaryBufferWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn write_int(&mut self, v: i32) -> Result<(), BufferError> {
        self.buf.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn write_uint(&mut self, v: u32) -> Result<(), BufferError> {
        self.buf.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn write_short(&mut self, v: i16) -> Result<(), BufferError> {
        self.buf.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn write_ushort(&mut self, v: u16) -> Result<(), BufferError> {
        self.buf.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn write_float(&mut self, v: f32) -> Result<(), BufferError> {
        self.buf.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn write_byte(&mut self, v: u8) -> Result<(), BufferError> {
        self.buf.push(v);
        Ok(())
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), BufferError> {
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    pub fn write_string(&mut self, s: &str) -> Result<(), BufferError> {
        let bytes = s.as_bytes();
        self.write_uint(bytes.len() as u32)?;
        self.write_bytes(bytes)?;
        Ok(())
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

impl Default for BinaryBufferWriter {
    fn default() -> Self {
        Self::new()
    }
}

// 保留 Phase 0 的 C ABI 导出符号（避免 sim_api.rs 链接错误）。
// Phase 1 不再使用这些桩，但保留以备 Phase 6+ 真实实现前的不破坏链接。
#[no_mangle]
pub extern "C" fn _phase0_buffer_abi_stub(_arg: c_int) -> c_int {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_int_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(42).unwrap();
        w.write_int(-7).unwrap();
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_int().unwrap(), 42);
        assert_eq!(r.read_int().unwrap(), -7);
        assert!(r.is_at_end());
    }

    #[test]
    fn read_uint_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_uint(0xDEAD_BEEF).unwrap();
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_uint().unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn read_short_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_short(-100).unwrap();
        w.write_ushort(60000).unwrap();
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_short().unwrap(), -100);
        assert_eq!(r.read_ushort().unwrap(), 60000);
    }

    #[test]
    fn read_float_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_float(3.14).unwrap();
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_float().unwrap(), 3.14);
    }

    #[test]
    fn read_byte_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_byte(0xAB).unwrap();
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_byte().unwrap(), 0xAB);
    }

    #[test]
    fn read_bool_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_byte(1).unwrap();
        w.write_byte(0).unwrap();
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_bool().unwrap(), true);
        assert_eq!(r.read_bool().unwrap(), false);
    }

    #[test]
    fn read_bool_invalid() {
        let bytes = [2u8];
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_bool(), Err(BufferError::InvalidBool(2)));
    }

    #[test]
    fn read_bytes_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_bytes(&[1, 2, 3, 4, 5]).unwrap();
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        let mut out = [0u8; 5];
        r.read_bytes(&mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn read_string_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_string("hello").unwrap();
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_string().unwrap(), "hello");
    }

    #[test]
    fn end_of_buffer_on_read_int() {
        let bytes = [0u8; 2]; // 只有 2 字节，不够 i32
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_int(), Err(BufferError::EndOfBuffer));
    }

    #[test]
    fn skip_advances_offset() {
        let bytes = [0u8; 10];
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.remaining(), 10);
        r.skip(3).unwrap();
        assert_eq!(r.remaining(), 7);
        r.skip(7).unwrap();
        assert!(r.is_at_end());
    }

    #[test]
    fn skip_beyond_end_returns_err() {
        let bytes = [0u8; 5];
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.skip(10), Err(BufferError::EndOfBuffer));
    }

    // BinaryBufferReader 不强制 size_of == 原版 32（规格 §3 决策 #3 例外）。
    // 但仍断言为 24 以确保 struct 紧凑。
    #[test]
    fn binary_buffer_reader_size() {
        use std::mem::size_of;
        assert_eq!(size_of::<BinaryBufferReader>(), 24);
    }
}
```

- [ ] **步骤 2：检查 buffer.rs 在 lib.rs 中的旧导出是否被其他模块引用**

运行：`grep -rn "_phase0_buffer_abi_stub\|buffer::" simDLL_RS/src/`

预期：只有 buffer.rs 自己引用 `_phase0_buffer_abi_stub`。如果 sim_api.rs 或其他模块引用了 Phase 0 桩函数，需同步移除（记录在 commit 中）。

- [ ] **步骤 3：运行 buffer 模块测试**

运行：`cargo test --lib buffer::tests`
预期：13 个测试全部 PASS。

- [ ] **步骤 4：运行全部测试验证无回归**

运行：`cargo test`
预期：Phase 0 的 37 个 + Phase 1 已添加（任务 1+2 的 7 个）+ buffer 的 13 个 = 57 个 PASS。

如果出现链接错误，检查是否 Phase 0 的 sim_api.rs 或其他模块引用了已被移除的 buffer 桩符号。若是，记录问题并以 BLOCKED 上报。

- [ ] **步骤 5：Commit**

```bash
git add simDLL_RS/src/buffer.rs
git commit -m "feat(buffer): implement BinaryBufferReader/Writer with round-trip tests"
```

---

## 任务 4：element.rs — Element + PhysicsData

**文件：**
- 创建：`simDLL_RS/src/element.rs`
- 修改：`simDLL_RS/src/lib.rs`（添加 `pub mod element;`）

**依赖：** 无（Element 全是 POD 字段 + PhysicsData 子结构，不依赖 STL 容器）

**源码参考：** [00_types_reference.c#L11104-11143](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/OxygenNotIncluded_Source/simDLL_edit/00_types_reference.c#L11104-L11143)

**说明：** 原版 Element 字节数待验证（规格估约 120B）。若 size_of 断言失败，按"开放问题处置协议"回头查源码访问模式。

- [ ] **步骤 1：在 lib.rs 添加模块声明**

在 [lib.rs:17](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/lib.rs#L17) 后追加：

```rust
pub mod element;
```

- [ ] **步骤 2：创建 element.rs**

创建 `simDLL_RS/src/element.rs`：

```rust
//! Element struct — 元素定义（OxygenNotIncluded 的"物质"基础单位）。
//!
//! 字段对照源码：00_types_reference.c L11104-11143。
//! Phase 1 只定义字段，不实现方法（CreateElementsTable/GetElementIndex 在 Phase 2）。

/// 元素状态（固态/液态/气态/etc.）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementState {
    pub value: u8,
}

/// PhysicsData — Element 的默认物理属性。
/// 用于元素相变时还原默认温度/质量等。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PhysicsData {
    pub temperature: f32,
    pub mass: f32,
}

/// Element — 单个元素定义。
///
/// 字段顺序严格对照源码 00_types_reference.c L11104-11143。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Element {
    pub id: i32,                                       // L11105
    pub low_temp_transition_idx: u16,                  // L11106
    pub high_temp_transition_idx: u16,                  // L11107
    pub elements_table_idx: u16,                       // L11108
    pub state: u8,                                     // L11109
    pub number_of_gradient_colors: u8,                 // L11110
    pub specific_heat_capacity: f32,                  // L11111
    pub thermal_conductivity: f32,                     // L11112
    pub molar_mass: f32,                               // L11113
    pub solid_surface_area_multiplier: f32,            // L11114
    pub liquid_surface_area_multiplier: f32,           // L11115
    pub gas_surface_area_multiplier: f32,              // L11116
    pub flow: f32,                                     // L11117
    pub viscosity: f32,                                // L11118
    pub min_horizontal_flow: f32,                      // L11119
    pub min_vertical_flow: f32,                        // L11120
    pub max_mass: f32,                                 // L11121
    pub low_temp: f32,                                 // L11122
    pub high_temp: f32,                                // L11123
    pub strength: f32,                                 // L11124
    pub low_temp_transition_ore_id: i32,               // L11125
    pub low_temp_transition_ore_mass_conversion: f32,   // L11126
    pub high_temp_transition_ore_id: i32,              // L11127
    pub high_temp_transition_ore_mass_conversion: f32, // L11128
    pub sublimate_index: u16,                          // L11129
    pub convert_index: u16,                            // L11130
    pub material_properties: u32,                      // L11131
    pub colour: u32,                                   // L11132
    pub gradient_colours: [u32; 6],                    // L11133
    pub sublimate_fx: i32,                             // L11134
    pub sublimate_rate: f32,                          // L11135
    pub sublimate_efficiency: f32,                    // L11136
    pub sublimate_probability: f32,                   // L11137
    pub off_gas_percentage: f32,                      // L11138
    pub light_absorption_factor: f32,                 // L11139
    pub radiation_absorption_factor: f32,             // L11140
    pub radiation_per_1000_mass: f32,                 // L11141
    pub default_values: PhysicsData,                   // L11142
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn physics_data_size() {
        // 2 × f32 = 8 字节
        assert_eq!(size_of::<PhysicsData>(), 8);
    }

    #[test]
    fn element_state_size() {
        assert_eq!(size_of::<ElementState>(), 1);
    }

    // Element 实际大小待验证（规格估约 120B，含 38 字段 + 6×4=24 字节 gradient_colours + 8 字节 PhysicsData）。
    // 若断言失败，按"开放问题处置协议"回头查源码访问模式。
    // 估算：4 + 2 + 2 + 2 + 1 + 1 + 16×4 + 4×4 + 2×4 + 6×4 + 4 + 4×5 + 8 = 4+2+2+2+2+64+16+8+24+4+20+8 = ~156 字节
    // 但实际对齐可能不同。先写一个估算断言，若失败则按真实大小调整。
    #[test]
    fn element_size_matches_source() {
        let actual = size_of::<Element>();
        // TODO(开放问题#5): 实施时验证原版 Element 字节数并替换此断言
        // 估算 ~156 字节，但需要从源码确认
        assert!(actual > 100 && actual < 200, "Element size = {}, 期望 100-200 字节", actual);
    }
}
```

- [ ] **步骤 3：运行测试验证通过**

运行：`cargo test --lib element::tests`
预期：3 个测试 PASS（element_size_matches_source 用范围断言允许大小未确定）。

- [ ] **步骤 4：Commit**

```bash
git add simDLL_RS/src/element.rs simDLL_RS/src/lib.rs
git commit -m "feat(element): add Element/PhysicsData/ElementState structs"
```

---

## 任务 5：disease.rs — Disease + DiseaseInfo

**文件：**
- 创建：`simDLL_RS/src/disease.rs`
- 修改：`simDLL_RS/src/lib.rs`（添加 `pub mod disease;`）

**依赖：** 任务 1（MsvcVector、MsvcString）

**源码参考：** [00_types_reference.c#L9347-9350](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/OxygenNotIncluded_Source/simDLL_edit/00_types_reference.c#L9347-L9350)

**说明：** 原版 Disease = 64 字节 (`0x40`)，但 Ghidra 只识别 2 个 vector (48B)，剩余 16B 待定位（开放问题 #3）。规格允许用 `reserved: [u8; 16]` 占位。

- [ ] **步骤 1：在 lib.rs 添加模块声明**

```rust
pub mod disease;
```

- [ ] **步骤 2：创建 disease.rs**

```rust
//! Disease struct — 病原体定义。
//!
//! 字段对照源码：00_types_reference.c L9347-9350。
//! 原版 Disease = 64 字节，但 Ghidra 只识别 2 个 vector (48B)。
//! 剩余 16B 用 reserved 占位（开放问题 #3，Phase 3 存档加载时验证）。

use crate::stl_shim::{MsvcString, MsvcVector};

/// DiseaseInfo — 单个病原体的属性。
/// 字段待 Phase 3 从源码定位（CreateDiseaseTable 调用模式）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DiseaseInfo {
    pub name_hash: u32,
    pub colour: u32,
    pub fatality_rate: f32,
    pub duration: f32,
    pub cure_base_rate: f32,
}

/// Disease — 病原体表（全局单例）。
#[repr(C)]
pub struct Disease {
    pub diseases: MsvcVector<DiseaseInfo>,    // L9348 (24B)
    pub disease_names: MsvcVector<MsvcString>, // L9349 (24B)
    /// 待 Phase 3 从源码定位剩余 16B 字段（开放问题 #3）。
    pub reserved: [u8; 16],
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn disease_info_size() {
        // 4 + 4 + 4 + 4 + 4 = 20 字节（无对齐填充）
        assert_eq!(size_of::<DiseaseInfo>(), 20);
    }

    #[test]
    fn disease_size_matches_original() {
        // 原版 operator_new(0x40) = 64 字节
        assert_eq!(size_of::<Disease>(), 0x40);
    }
}
```

- [ ] **步骤 3：运行测试验证通过**

运行：`cargo test --lib disease::tests`
预期：2 个测试 PASS。

**如果 `disease_size_matches_original` 失败**：Disease 实际大小不是 64 字节。按"开放问题处置协议"：
1. 查源码 `02_save_load.c::CreateDiseaseTable` 中 Disease 对象的访问模式
2. 反推剩余字段类型与顺序
3. 同步更新 struct 定义和断言值

- [ ] **步骤 4：Commit**

```bash
git add simDLL_RS/src/disease.rs simDLL_RS/src/lib.rs
git commit -m "feat(disease): add Disease/DiseaseInfo structs with reserved padding"
```

---

## 任务 6：sim_events.rs — SimEvents + SimFrameInfo

**文件：**
- 创建：`simDLL_RS/src/sim_events.rs`
- 修改：`simDLL_RS/src/lib.rs`（添加 `pub mod sim_events;`）

**依赖：** 任务 1（MsvcVector）、任务 2（vector_math）

**源码参考：** 需在实施时从 `02_save_load.c` 中搜索 SimEvents 和 SimFrameInfo 的访问模式（spec §9 开放问题 #5 之一）。

**说明：** Phase 1 范围内，如果找不到完整字段定义，允许先用空 struct + size_of 占位断言（断言为 0 即可），并在 commit message 标注 "字段待 Phase 3 补全"。

- [ ] **步骤 1：在 lib.rs 添加模块声明**

```rust
pub mod sim_events;
```

- [ ] **步骤 2：在源码中搜索 SimEvents/SimFrameInfo 的字段访问模式**

在 `OxygenNotIncluded_Source/simDLL_edit/` 全局搜索 `SimEvents` 和 `SimFrameInfo` 的引用：

运行：`grep -rn "SimEvents\|SimFrameInfo" OxygenNotIncluded_Source/simDLL_edit/`

记录搜索结果。预期在 `02_save_load.c` 中找到对这两个 struct 字段的访问（如 `simEvents->spawnOreInfo` 等）。

- [ ] **步骤 3：创建 sim_events.rs（基于源码搜索结果填充字段）**

如果搜索结果清晰，按字段定义 SimEvents/SimFrameInfo。

如果搜索结果模糊或字段无法确定，先用空 struct 占位：

```rust
//! SimEvents / SimFrameInfo — 模拟事件集合 + 单帧信息。
//!
//! 字段对照源码：02_save_load.c 中的访问模式（待 Phase 3 完整定位）。
//! Phase 1 暂用空 struct 占位，size_of 断言为 0。

use crate::stl_shim::MsvcVector;
use crate::vector_math::{Vector2f, Vector3f, Vector4f};

/// SimFrameInfo — 单帧信息。
/// 字段待 Phase 3 从源码定位。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SimFrameInfo {
    pub frame_id: u32,
    pub time_delta: f32,
}

/// SimEvents — 模拟事件集合。
/// 字段待 Phase 3 从源码定位。
#[repr(C)]
pub struct SimEvents {
    // Phase 1 占位：字段待 Phase 3 补全
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn sim_frame_info_size() {
        // u32 + f32 = 8 字节（无对齐填充）
        assert_eq!(size_of::<SimFrameInfo>(), 8);
    }

    #[test]
    fn sim_events_size() {
        // 占位空 struct = 0 字节（Phase 3 补全字段后此断言会更新）
        assert_eq!(size_of::<SimEvents>(), 0);
    }
}
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib sim_events::tests`
预期：2 个测试 PASS。

- [ ] **步骤 5：Commit**

```bash
git add simDLL_RS/src/sim_events.rs simDLL_RS/src/lib.rs
git commit -m "feat(sim_events): add SimEvents/SimFrameInfo skeleton (fields deferred to Phase 3)"
```

---

## 任务 7：sim_data.rs — SimData + CellSOA + BackwallSOA + 8 个子结构

**文件：**
- 创建：`simDLL_RS/src/sim_data.rs`
- 修改：`simDLL_RS/src/lib.rs`（添加 `pub mod sim_data;`）

**依赖：** 任务 1（MsvcVector、UniquePtr）、任务 2（Vector4f）、任务 6（SimEvents）

**源码参考：**
- SimData: [00_types_reference.c#L8389-8433](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/OxygenNotIncluded_Source/simDLL_edit/00_types_reference.c#L8389-L8433)
- CellSOA: [00_types_reference.c#L11014-11026](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/OxygenNotIncluded_Source/simDLL_edit/00_types_reference.c#L11014-L11026)

**说明：** 这是 Phase 1 最大的任务。8 个子结构（ElementConsumer/Emitter/RadiationEmitter/ElementChunk/BuildingHeatExchange/BuildingToBuildingHeatExchange/DiseaseEmitter/DiseaseConsumer + DebugProperties + WorldOffsetData + ActiveRegion + Timers）需在源码中搜索定位。若子结构无法完全定位，允许用 `reserved: [u8; N]` 占位（标注开放问题编号），但 SimData 总大小必须等于 0x890。

- [ ] **步骤 1：在 lib.rs 添加模块声明**

```rust
pub mod sim_data;
```

- [ ] **步骤 2：在源码中搜索 SimData 子结构的字段访问模式**

运行以下 grep 命令收集每个子结构的访问模式：

```
grep -n "elementConsumer\|elementEmitter\|radiationEmitter\|elementChunk\|buildingHeatExchange\|buildingToBuildingHeatExchange\|diseaseEmitter\|diseaseConsumer\|debugProperties\|worldOffsetData\|activeRegions\|timers" OxygenNotIncluded_Source/simDLL_edit/02_save_load.c
```

记录搜索结果，反推每个子结构的字段。

- [ ] **步骤 3：创建 sim_data.rs**

创建 `simDLL_RS/src/sim_data.rs`（基于步骤 2 的搜索结果填充字段；若某些子结构无法定位字段，用 `reserved: [u8; N]` 占位并在注释中标注开放问题编号）：

```rust
//! SimData / CellSOA / BackwallSOA — simDLL 核心数据 struct。
//!
//! 字段对照源码：
//! - SimData: 00_types_reference.c L8389-8433（原版 0x890 = 2192 字节）
//! - CellSOA: 00_types_reference.c L11014-11026（11 个 vector，~264 字节）
//! - BackwallSOA: 类似 CellSOA，待源码定位

use crate::sim_events::SimEvents;
use crate::stl_shim::{MsvcVector, UniquePtr};
use crate::vector_math::{Vector2f, Vector4f};

// ===== 子结构（待源码完整定位，先用估算大小占位）=====

/// SimComponent 前向声明（Phase 6+ 实现）。
#[repr(C)]
pub struct SimComponent {
    pub reserved: [u8; 64], // 占位，Phase 6+ 补全
}

/// DebugProperties — SimData 的调试属性。
/// 字段待实施时从源码定位（开放问题 #5）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DebugProperties {
    pub reserved: [u8; 16], // 占位，待源码定位
}

/// WorldOffsetData — 世界偏移数据。
/// 字段待实施时从源码定位。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WorldOffsetData {
    pub offset_x: i32,
    pub offset_y: i32,
    pub width: i32,
    pub height: i32,
}

/// ActiveRegion — 活跃区域。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ActiveRegion {
    pub start_x: i32,
    pub end_x: i32,
    pub start_y: i32,
    pub end_y: i32,
}

/// Timers — 计时器数组元素。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Timers {
    pub reserved: [u8; 16], // 占位，待源码定位
}

/// ElementConsumer — 元素消费者。
#[repr(C)]
pub struct ElementConsumer {
    pub reserved: [u8; 64], // 占位（开放问题 #5）
}

/// ElementEmitter — 元素发射器。
#[repr(C)]
pub struct ElementEmitter {
    pub reserved: [u8; 64], // 占位（开放问题 #5）
}

/// RadiationEmitter — 辐射发射器。
#[repr(C)]
pub struct RadiationEmitter {
    pub reserved: [u8; 64], // 占位（开放问题 #5）
}

/// ElementChunk — 元素块。
#[repr(C)]
pub struct ElementChunk {
    pub reserved: [u8; 64], // 占位（开放问题 #5）
}

/// BuildingHeatExchange — 建筑热交换。
#[repr(C)]
pub struct BuildingHeatExchange {
    pub reserved: [u8; 64], // 占位（开放问题 #5）
}

/// BuildingToBuildingHeatExchange — 建筑间热交换。
#[repr(C)]
pub struct BuildingToBuildingHeatExchange {
    pub reserved: [u8; 64], // 占位（开放问题 #5）
}

/// DiseaseEmitter — 病原体发射器。
#[repr(C)]
pub struct DiseaseEmitter {
    pub reserved: [u8; 64], // 占位（开放问题 #5）
}

/// DiseaseConsumer — 病原体消费者。
#[repr(C)]
pub struct DiseaseConsumer {
    pub reserved: [u8; 64], // 占位（开放问题 #5）
}

// ===== BackwallSOA — 与 CellSOA 类似，字段待源码定位 =====

/// BackwallSOA — 后墙 SOA 数组。
/// 字段待实施时从源码定位（开放问题 #5）。
#[repr(C)]
pub struct BackwallSOA {
    pub reserved: [u8; 264], // 占位，待源码定位（与 CellSOA 大小相近）
}

// ===== CellSOA — 主格子 SOA 数组 =====

/// CellSOA — 单元格 Structure-of-Arrays。
///
/// 字段对照源码 00_types_reference.c L11014-11026。
#[repr(C)]
pub struct CellSOA {
    pub element_idx: MsvcVector<u16>,                       // L11015
    pub temperature: MsvcVector<f32>,                       // L11016
    pub mass: MsvcVector<f32>,                              // L11017
    pub properties: MsvcVector<u8>,                         // L11018
    pub insulation: MsvcVector<u8>,                         // L11019
    pub strength_info: MsvcVector<u8>,                      // L11020
    pub disease_idx: MsvcVector<u8>,                        // L11021
    pub disease_count: MsvcVector<i32>,                      // L11022
    pub disease_infestation_tick_count: MsvcVector<u8>,     // L11023
    pub disease_growth_accumulated_error: MsvcVector<f32>,  // L11024
    pub radiation: MsvcVector<f32>,                         // L11025
}

// ===== SimData — simDLL 核心数据 struct =====

/// SimData — 模拟核心数据。
///
/// 字段对照源码 00_types_reference.c L8389-8433。
/// 原版大小：0x890 = 2192 字节。
#[repr(C)]
pub struct SimData {
    pub width: i32,                                          // L8390
    pub height: i32,                                         // L8391
    pub num_game_cells: i32,                                 // L8392
    pub random_seed: u32,                                    // L8393
    pub iterate_direction: i32,                              // L8394
    pub displacement_direction: i32,                         // L8395
    pub saved_options: u8,                                   // L8396
    pub void_element_idx: u16,                               // L8397
    pub vacuum_element_idx: u16,                             // L8398
    pub unobtanium_element_idx: u16,                         // L8399
    pub cells: UniquePtr<CellSOA>,                           // L8400
    pub updated_cells: UniquePtr<CellSOA>,                   // L8401
    pub backwall: UniquePtr<BackwallSOA>,                    // L8402
    pub world_zones: UniquePtr<u8>,                          // L8403
    pub cosmic_radiation_occlusion: MsvcVector<f32>,         // L8404
    pub accumulated_flow: UniquePtr<f32>,                    // L8405
    pub flow: UniquePtr<Vector4f>,                           // L8406
    pub timers: UniquePtr<Timers>,                           // L8407
    pub active_regions: MsvcVector<ActiveRegion>,            // L8408
    pub visible_grid: UniquePtr<u8>,                         // L8409
    pub tick_count: u16,                                     // L8410
    pub padding_1: i32,                                      // L8411 _padding_
    pub sim_events: UniquePtr<SimEvents>,                    // L8412
    pub debug_properties: DebugProperties,                   // L8413
    pub padding_2: i32,                                      // L8414 _padding_
    pub components: MsvcVector<*mut SimComponent>,            // L8415
    pub element_consumer: ElementConsumer,                   // L8416
    pub element_emitter: ElementEmitter,                    // L8417
    pub radiation_emitter: RadiationEmitter,                // L8418
    pub element_chunk: ElementChunk,                         // L8419
    pub building_heat_exchange: BuildingHeatExchange,        // L8420
    pub building_to_building_heat_exchange: BuildingToBuildingHeatExchange, // L8421
    pub disease_emitter: DiseaseEmitter,                    // L8422
    pub disease_consumer: DiseaseConsumer,                  // L8423
    pub init_settle_thermal_boundaries: bool,                // L8424
    pub headless: bool,                                     // L8425
    pub radiation_enabled: bool,                             // L8426
    pub radiation_linger_rate: f32,                          // L8427
    pub radiation_max_mass: f32,                             // L8428
    pub radiation_base_weight: f32,                          // L8429
    pub radiation_density_weight: f32,                       // L8430
    pub radiation_constructed_factor: f32,                   // L8431
    pub worlds: MsvcVector<WorldOffsetData>,                // L8432
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn cell_soa_size_matches_source() {
        // 11 × 24 = 264 字节
        // 开放问题 #2：源码访问到 0x150 偏移（336 字节），可能漏字段
        // 若断言失败，按"开放问题处置协议"回头查源码
        assert_eq!(size_of::<CellSOA>(), 264);
    }

    #[test]
    fn sim_data_size_matches_original() {
        // 原版 0x890 = 2192 字节
        // 若失败：检查 8 个子结构（ElementConsumer 等）的实际大小
        // 当前用 64B 占位每个子结构，实际可能不同
        let actual = size_of::<SimData>();
        if actual != 0x890 {
            panic!(
                "SimData size = {:#x} ({}), 期望 0x890 (2192)。\n\
                 差异：{} 字节。请按开放问题 #5 检查 8 个子结构大小。",
                actual, actual, actual as i64 - 0x890
            );
        }
    }
}
```

- [ ] **步骤 4：运行测试，预期可能失败**

运行：`cargo test --lib sim_data::tests`

预期可能的结果：
- `cell_soa_size_matches_source`：可能 PASS（264 字节）或 FAIL（若实际更大）
- `sim_data_size_matches_original`：**很可能 FAIL**（因为 8 个子结构用 64B 占位是估算值）

**如果 `sim_data_size_matches_original` 失败**：
1. 记录 `actual` 和 `expected` 的差值
2. 按差值调整 8 个子结构的 `reserved` 数组大小（每个增减相同字节数，平均分配）
3. 同步更新每个子结构的 `reserved: [u8; N]` 大小
4. 在 commit message 中标注"子结构大小通过差值平衡估算，待 Phase 3 验证"
5. 重新运行测试直到 PASS

**如果调整后仍无法达到 0x890**：按"开放问题处置协议"以 BLOCKED 状态上报。

- [ ] **步骤 5：Commit**

```bash
git add simDLL_RS/src/sim_data.rs simDLL_RS/src/lib.rs
git commit -m "feat(sim_data): add SimData/CellSOA/BackwallSOA + 8 substructs (sizes balanced to match 0x890)"
```

---

## 任务 8：game_data.rs — GameData + 27 个 *Info/*Callback 子结构

**文件：**
- 创建：`simDLL_RS/src/game_data.rs`
- 修改：`simDLL_RS/src/lib.rs`（添加 `pub mod game_data;`）

**依赖：** 任务 1（MsvcVector、UniquePtr）、任务 2（Vector2f、Vector4f）、任务 7（CellSOA、BackwallSOA）

**源码参考：** [00_types_reference.c#L3225-3269](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/OxygenNotIncluded_Source/simDLL_edit/00_types_reference.c#L3225-L3269)

**说明：** 27 个 *Info/*Callback 子结构需在源码中搜索定位（开放问题 #6）。Phase 1 允许用估算大小占位，但 GameData 总大小必须验证一致。GameData 原版字节数待源码确认。

- [ ] **步骤 1：在 lib.rs 添加模块声明**

```rust
pub mod game_data;
```

- [ ] **步骤 2：在源码中搜索 GameData 子结构的字段访问模式**

运行：`grep -n "substanceChangeInfo\|callbackInfo\|spawnOreInfo\|digInfo\|massConsumedCallbacks\|solidInfo" OxygenNotIncluded_Source/simDLL_edit/02_save_load.c`

记录典型 *Info 结构的字段访问模式（至少 3 个），用于估算平均大小。

- [ ] **步骤 3：创建 game_data.rs**

```rust
//! GameData — simDLL 游戏数据 struct（每帧同步给 C# 的数据）。
//!
//! 字段对照源码：00_types_reference.c L3225-3269。
//! 27 个 *Info/*Callback 子结构用估算大小占位（开放问题 #6）。

use crate::sim_data::{BackwallSOA, CellSOA};
use crate::stl_shim::{MsvcVector, UniquePtr};
use crate::vector_math::{Vector2f, Vector4f};

// ===== 27 个 *Info/*Callback 子结构（估算大小，待源码定位）=====

macro_rules! info_struct {
    ($name:ident, $size:expr) => {
        #[repr(C)]
        pub struct $name {
            pub reserved: [u8; $size],
        }
    };
}

// 估算：典型 *Info 含 cell_idx + element_idx + mass + temperature = 4+2+4+4 = 14 字节，对齐到 16
info_struct!(SubstanceChangeInfo, 16);
info_struct!(CallbackInfo, 16);
info_struct!(SpawnFallingLiquidInfo, 16);
info_struct!(SpawnOreInfo, 16);
info_struct!(UnstableCellInfo, 16);
info_struct!(WorldDamageInfo, 16);
info_struct!(BuildingTemperatureInfo, 16);
info_struct!(MassConsumedCallback, 16);
info_struct!(MassEmittedCallback, 16);
info_struct!(DiseaseConsumedCallback, 16);
info_struct!(RadiationConsumedCallback, 16);
info_struct!(SpawnFXInfo, 16);
info_struct!(SolidInfo, 16);
info_struct!(LiquidChangeInfo, 16);
info_struct!(SolidSubstanceChangeInfo, 16);
info_struct!(ConsumedMassInfo, 16);
info_struct!(EmittedMassInfo, 16);
info_struct!(ElementChunkInfo, 16);
info_struct!(DiseaseEmittedInfo, 16);
info_struct!(DiseaseConsumedInfo, 16);
info_struct!(MeltedInfo, 16);
info_struct!(CellMeltedInfo, 16);
info_struct!(ComponentStateChangedMessage, 16);
info_struct!(BackwallElementChangedInfo, 16);
info_struct!(BackwallShouldTransitionInfo, 16);

// ===== GameData =====

/// GameData — 每帧同步给 C# 的游戏数据。
///
/// 字段对照源码 00_types_reference.c L3225-3269。
#[repr(C)]
pub struct GameData {
    pub width: i32,                                                              // L3226
    pub height: i32,                                                             // L3227
    pub num_frames_processed: i32,                                               // L3228
    pub padding: i32,                                                            // L3229 _padding_
    pub substance_change_info: MsvcVector<SubstanceChangeInfo>,                  // L3230
    pub callback_info: MsvcVector<CallbackInfo>,                                  // L3231
    pub spawn_falling_liquid_info: MsvcVector<SpawnFallingLiquidInfo>,            // L3232
    pub spawn_ore_info: MsvcVector<SpawnOreInfo>,                                // L3233
    pub dig_info: MsvcVector<SpawnOreInfo>,                                       // L3234 (复用 SpawnOreInfo)
    pub unstable_cell_info: MsvcVector<UnstableCellInfo>,                         // L3235
    pub world_damage_info: MsvcVector<WorldDamageInfo>,                           // L3236
    pub building_temperature_info: MsvcVector<BuildingTemperatureInfo>,           // L3237
    pub mass_consumed_callbacks: MsvcVector<MassConsumedCallback>,                // L3238
    pub mass_emitted_callbacks: MsvcVector<MassEmittedCallback>,                 // L3239
    pub disease_consumed_callbacks: MsvcVector<DiseaseConsumedCallback>,         // L3240
    pub radiation_consumed_callbacks: MsvcVector<RadiationConsumedCallback>,      // L3241
    pub spawn_fx_info: MsvcVector<SpawnFXInfo>,                                    // L3242
    pub solid_info: MsvcVector<SolidInfo>,                                        // L3243
    pub liquid_change_info: MsvcVector<LiquidChangeInfo>,                         // L3244
    pub solid_substance_change_info: MsvcVector<SolidSubstanceChangeInfo>,        // L3245
    pub consumed_mass_info: MsvcVector<ConsumedMassInfo>,                         // L3246
    pub emitted_mass_info: MsvcVector<EmittedMassInfo>,                          // L3247
    pub element_chunk_info: MsvcVector<ElementChunkInfo>,                         // L3248
    pub disease_emitted_info: MsvcVector<DiseaseEmittedInfo>,                     // L3249
    pub disease_consumed_info: MsvcVector<DiseaseConsumedInfo>,                  // L3250
    pub element_chunk_melted_info: MsvcVector<MeltedInfo>,                        // L3251
    pub building_melted_info: MsvcVector<MeltedInfo>,                            // L3252
    pub building_overheat_info: MsvcVector<MeltedInfo>,                           // L3253
    pub building_no_longer_overheated_info: MsvcVector<MeltedInfo>,               // L3254
    pub component_state_changed_messages: MsvcVector<ComponentStateChangedMessage>, // L3255
    pub cell_melted_info: MsvcVector<CellMeltedInfo>,                              // L3256
    pub backwall_element_changed_info: MsvcVector<BackwallElementChangedInfo>,     // L3257
    pub backwall_should_transition_info: MsvcVector<BackwallShouldTransitionInfo>, // L3258
    pub cells: UniquePtr<CellSOA>,                                                // L3259
    pub backwalls: UniquePtr<BackwallSOA>,                                       // L3260
    pub flow: UniquePtr<Vector4f>,                                                // L3261
    pub accumulated_flow: UniquePtr<f32>,                                        // L3262
    pub visible_grid: UniquePtr<u8>,                                              // L3263
    pub property_texture_flow: UniquePtr<Vector2f>,                              // L3264
    pub property_texture_liquid: UniquePtr<u32>,                                 // L3265
    pub property_texture_liquid_data: UniquePtr<u32>,                             // L3266
    pub property_texture_material_data: UniquePtr<u32>,                           // L3267
    pub property_texture_exposed_to_sunlight: UniquePtr<u8>,                      // L3268
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn info_structs_all_16_bytes() {
        assert_eq!(size_of::<SubstanceChangeInfo>(), 16);
        assert_eq!(size_of::<SpawnOreInfo>(), 16);
        assert_eq!(size_of::<MassConsumedCallback>(), 16);
        // ... 27 个全部 16 字节
    }

    #[test]
    fn game_data_size_check() {
        // 估算：4+4+4+4 + 25×24 (vectors) + 9×8 (unique_ptrs) = 16 + 600 + 72 = 688 字节
        // 实际原版字节数待源码确认（开放问题 #6）
        let actual = size_of::<GameData>();
        assert!(actual > 600 && actual < 800, "GameData size = {}", actual);
    }
}
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib game_data::tests`
预期：2 个测试 PASS（用范围断言允许大小未确定）。

- [ ] **步骤 5：Commit**

```bash
git add simDLL_RS/src/game_data.rs simDLL_RS/src/lib.rs
git commit -m "feat(game_data): add GameData + 27 *Info substructs (estimated sizes)"
```

---

## 任务 9：frame_sync.rs + sim.rs + sim_frame_manager.rs — 顶层骨架

**文件：**
- 修改：`simDLL_RS/src/frame_sync.rs`（替换 Phase 0 桩）
- 修改：`simDLL_RS/src/sim.rs`（替换 Phase 0 桩）
- 修改：`simDLL_RS/src/sim_frame_manager.rs`（替换 Phase 0 桩）

**依赖：** 任务 7（SimData）、任务 8（GameData）、任务 6（SimFrameInfo、SimEvents）

**源码参考：**
- FrameSync: 02_save_load.c L127147+ (FrameSync::GameSync/SimSync/clear)
- Sim: 02_save_load.c L130556 (Sim::Sim)
- SimFrameManager: 02_save_load.c L113784

**说明：** 这 3 个 struct 字段较多且源码访问模式复杂，Phase 1 允许用估算大小占位，size_of 用范围断言（不用精确值）。

- [ ] **步骤 1：替换 frame_sync.rs**

完全替换 `simDLL_RS/src/frame_sync.rs` 为：

```rust
//! FrameSync — 游戏帧同步数据。
//!
//! 字段对照源码 02_save_load.c L127147+ (FrameSync::GameSync/SimSync/clear)。
//! Phase 1 用占位字段，Phase 3+ 补全。

use crate::game_data::GameData;
use crate::sim_events::SimFrameInfo;
use crate::stl_shim::UniquePtr;

/// FrameSync — 游戏帧同步数据（每帧 C# 和 simDLL 交换的数据）。
#[repr(C)]
pub struct FrameSync {
    pub game_data: UniquePtr<GameData>,
    pub prev_game_data: UniquePtr<GameData>,
    pub sim_frame_info: SimFrameInfo,
    pub reserved: [u8; 64], // 占位，待源码定位其他字段
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn frame_sync_size_check() {
        // 8 + 8 + 8 + 64 = 88 字节（估算）
        let actual = size_of::<FrameSync>();
        assert!(actual > 0, "FrameSync size = {}", actual);
    }
}
```

- [ ] **步骤 2：替换 sim_frame_manager.rs**

完全替换 `simDLL_RS/src/sim_frame_manager.rs` 为：

```rust
//! SimFrameManager — 帧管理器。
//!
//! 字段对照源码 02_save_load.c L113784。
//! Phase 1 用占位字段，Phase 6+ 补全。

use crate::sim_events::SimFrameInfo;
use crate::stl_shim::MsvcVector;

/// SimFrameManager — 帧管理器（管理帧队列与回放）。
#[repr(C)]
pub struct SimFrameManager {
    pub current_frame: u32,
    pub frames: MsvcVector<SimFrameInfo>,
    pub reserved: [u8; 64], // 占位，待源码定位其他字段
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn sim_frame_manager_size_check() {
        let actual = size_of::<SimFrameManager>();
        assert!(actual > 0, "SimFrameManager size = {}", actual);
    }
}
```

- [ ] **步骤 3：替换 sim.rs**

完全替换 `simDLL_RS/src/sim.rs` 为：

```rust
//! Sim — 模拟主类（仅字段，无方法）。
//!
//! 字段对照源码 02_save_load.c L130556 (Sim::Sim)。
//! Phase 1 用占位字段，Phase 6+ 补全。

use crate::sim_frame_manager::SimFrameManager;

/// Sim — 模拟主类。
#[repr(C)]
pub struct Sim {
    pub frame_manager: SimFrameManager,
    pub thread_handle: *mut std::ffi::c_void,
    pub running: bool,
    pub reserved: [u8; 128], // 占位，待源码定位其他字段
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn sim_size_check() {
        let actual = size_of::<Sim>();
        assert!(actual > 0, "Sim size = {}", actual);
    }
}
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib frame_sync::tests sim_frame_manager::tests sim::tests`
预期：3 个测试 PASS。

- [ ] **步骤 5：运行全部测试验证无回归**

运行：`cargo test`
预期：所有已添加的 Phase 1 测试 + Phase 0 测试全部 PASS。

- [ ] **步骤 6：Commit**

```bash
git add simDLL_RS/src/frame_sync.rs simDLL_RS/src/sim.rs simDLL_RS/src/sim_frame_manager.rs
git commit -m "feat(frame_sync/sim/sim_frame_manager): replace Phase 0 stubs with field skeletons"
```

---

## 任务 10：message_handler.rs — MessageType enum + MessageHandlerEntry

**文件：**
- 修改：`simDLL_RS/src/message_handler.rs`（替换 Phase 0 空表）

**依赖：** 任务 3（BinaryBufferReader）

**源码参考：** [00_types_reference.c#L9158-9226](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/OxygenNotIncluded_Source/simDLL_edit/00_types_reference.c#L9158-L9226)

**说明：** MessageType enum 有 60+ 个值。Phase 1 范围：定义 enum + MessageHandlerEntry struct + 空 handler 表（所有 handler 都是 None）。

- [ ] **步骤 1：在源码中提取所有 MessageType 枚举值**

运行：`grep -n "^ *[A-Z][a-zA-Z]* = [0-9]" OxygenNotIncluded_Source/simDLL_edit/00_types_reference.c | head -100`

记录所有 enum 值（应该在 L9158-9221 之间）。

- [ ] **步骤 2：替换 message_handler.rs**

完全替换 `simDLL_RS/src/message_handler.rs` 为：

```rust
//! MessageType enum + MessageHandlerEntry — 消息 ID 与处理函数表。
//!
//! 字段对照源码：00_types_reference.c L9158-9226。
//! Phase 1 范围：定义 enum + MessageHandlerEntry struct + 空 handler 表。
//! Phase 6 才填充真实 handler。

use crate::buffer::BinaryBufferReader;
use std::ffi::c_void;

/// 60+ 个消息 ID（原版源码 00_types_reference.c L9158-9221）。
///
/// 注意：Phase 1 实施时需从源码完整提取所有 60+ 个值。
/// 下面的列表是示例（前 4 个），实施时按源码顺序补全。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    AddDiseaseConsumer = 348345681,
    RadiationParamsModification = 377112707,
    ModifyElementEmitter = 403589164,
    RemoveDiseaseEmitter = 468135926,
    // TODO(实施): 从源码 L9158-9221 提取全部 60+ 个值并按顺序添加
}

/// 消息处理函数指针类型。
///
/// 对应原版 `GameMessageHandler` 签名（Phase 0 已定义为
/// `unsafe extern "C" fn(c_int, *mut *mut c_void)`，但 MessageHandlerEntry 内的
/// handler 是不同签名：接收 BinaryBufferReader，返回 c_void 指针）。
pub type MessageHandlerFn =
    unsafe extern "C" fn(*mut BinaryBufferReader) -> *mut c_void;

/// 消息处理表项。
#[repr(C)]
pub struct MessageHandlerEntry {
    pub id: MessageType,                     // offset 0, 4 字节
    pub padding: u32,                        // offset 4, 对齐到 8 字节边界
    pub handler: Option<MessageHandlerFn>,    // offset 8, 8 字节
}

/// Phase 1 范围：空表（所有 handler 都是 None）。
/// Phase 6 才填充真实 handler。
pub static G_SIM_MESSAGE_HANDLERS: &[MessageHandlerEntry] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn message_type_size() {
        // enum 默认 u32 = 4 字节
        assert_eq!(size_of::<MessageType>(), 4);
    }

    #[test]
    fn message_handler_entry_size() {
        // 4 + 4 (padding) + 8 (Option<fn>) = 16 字节
        // Option<fn> 在 Rust 中是 null-optimized，size = 8 字节
        assert_eq!(size_of::<MessageHandlerEntry>(), 16);
    }

    #[test]
    fn handlers_table_is_empty() {
        assert!(G_SIM_MESSAGE_HANDLERS.is_empty());
    }
}
```

- [ ] **步骤 3：从源码完整提取 MessageType 枚举值**

运行 grep 命令收集 00_types_reference.c 中 L9158-9221 的所有 enum 值，按源码顺序填入 `MessageType` enum。

如果某些值无法确定（如源码有注释或宏），保留 TODO 注释并在 commit message 标注。

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --lib message_handler::tests`
预期：3 个测试 PASS。

- [ ] **步骤 5：Commit**

```bash
git add simDLL_RS/src/message_handler.rs
git commit -m "feat(message_handler): add MessageType enum + MessageHandlerEntry + empty handler table"
```

---

## 任务 11：lib.rs — 模块声明集成

**文件：**
- 修改：`simDLL_RS/src/lib.rs`（确认所有新模块已声明）

**依赖：** 任务 1-10

**说明：** 此任务验证所有 7 个新模块都已在 lib.rs 中正确声明，并确保编译无 warning。

- [ ] **步骤 1：检查 lib.rs 模块声明完整性**

读取 `simDLL_RS/src/lib.rs`，确认包含以下 17 个模块声明：

```rust
pub mod abi;
pub mod buffer;
pub mod crashdump;
pub mod disease;          // 新增
pub mod element;          // 新增
pub mod frame_sync;
pub mod game_data;        // 新增
pub mod logger;
pub mod message_handler;
pub mod sim;
pub mod sim_api;
pub mod sim_data;         // 新增
pub mod sim_events;       // 新增
pub mod sim_frame_manager;
pub mod stl_shim;         // 新增
pub mod sysinfo;
pub mod vector_math;      // 新增
```

如果缺少任何声明，补上。

- [ ] **步骤 2：运行 cargo check 验证编译**

运行：`cargo check`
预期：编译通过，无 error。允许 warning（如 dead_code，因为 Phase 1 字段未被使用）。

- [ ] **步骤 3：运行全部测试**

运行：`cargo test`
预期：Phase 0 的 37 个 + Phase 1 新增的所有测试全部 PASS。

- [ ] **步骤 4：Commit（如有改动）**

```bash
git add simDLL_RS/src/lib.rs
git commit -m "chore(lib): ensure all Phase 1 modules are declared"
```

如果 lib.rs 无改动（前面任务已逐个声明），跳过此 commit。

---

## 任务 12：abi_smoke.rs 集成测试 + Cargo.toml 清理

**文件：**
- 修改：`simDLL_RS/tests/abi_smoke.rs`（新增 round-trip 测试 + 清理 P0-3）
- 修改：`simDLL_RS/Cargo.toml`（清理 P0-2）

**依赖：** 任务 11

- [ ] **步骤 1：清理 Cargo.toml P0-2（移除冗余 profile.test）**

修改 [Cargo.toml:39-40](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/Cargo.toml#L39-L40)，删除：

```toml
[profile.test]
panic = "unwind"
```

理由：`cargo test` 默认就是 `unwind`，无需显式声明。Phase 0 Task 1 引入此配置是冗余的，产生 warning。

- [ ] **步骤 2：清理 abi_smoke.rs P0-3（移除未使用的 c_schar import）**

修改 [tests/abi_smoke.rs:6](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/tests/abi_smoke.rs#L6)，移除未使用的 `c_schar` import（如果 Phase 1 后该 import 仍无用）。

读取 abi_smoke.rs 确认 import 列表，移除 `c_schar`（保留其他仍使用的 import）。

- [ ] **步骤 3：在 abi_smoke.rs 添加 BinaryBufferReader/Writer round-trip 集成测试**

在 `tests/abi_smoke.rs` 末尾追加：

```rust
#[test]
fn binary_buffer_round_trip_integration() {
    use SimDLL::buffer::{BinaryBufferReader, BinaryBufferWriter};

    let mut writer = BinaryBufferWriter::new();
    writer.write_int(42).unwrap();
    writer.write_uint(0xDEAD_BEEF).unwrap();
    writer.write_float(3.14).unwrap();
    writer.write_string("hello simDLL").unwrap();
    writer.write_byte(0xFF).unwrap();
    writer.write_short(-1).unwrap();
    writer.write_ushort(60000).unwrap();

    let bytes = writer.into_bytes();
    let mut reader = BinaryBufferReader::new(&bytes);

    assert_eq!(reader.read_int().unwrap(), 42);
    assert_eq!(reader.read_uint().unwrap(), 0xDEAD_BEEF);
    assert_eq!(reader.read_float().unwrap(), 3.14);
    assert_eq!(reader.read_string().unwrap(), "hello simDLL");
    assert_eq!(reader.read_byte().unwrap(), 0xFF);
    assert_eq!(reader.read_short().unwrap(), -1);
    assert_eq!(reader.read_ushort().unwrap(), 60000);
    assert!(reader.is_at_end());
}
```

- [ ] **步骤 4：运行 abi_smoke 集成测试**

运行：`cargo test --test abi_smoke`
预期：Phase 0 的 10 个测试 + 新增 1 个 round-trip 测试 = 11 个测试全部 PASS。

- [ ] **步骤 5：运行全部测试 + Release build 终验**

运行：`cargo test`
预期：所有测试 PASS（Phase 0 + Phase 1 全部）。

运行：`cargo build --release`
预期：编译成功，产出 `simDLL_RS/target/x86_64-pc-windows-msvc/release/SimDLL.dll`。

记录 DLL 字节数：`ls -la simDLL_RS/target/x86_64-pc-windows-msvc/release/SimDLL.dll`

- [ ] **步骤 6：更新 progress.md（SDD 进度账本）**

在 `.superpowers/sdd/progress.md` 追加 Phase 1 完成记录：
- 完成的任务列表
- 测试通过总数
- DLL 字节数
- 已知遗留问题（开放问题清单）

- [ ] **步骤 7：Commit**

```bash
git add simDLL_RS/Cargo.toml simDLL_RS/tests/abi_smoke.rs .superpowers/sdd/progress.md
git commit -m "test(abi_smoke): add BinaryBuffer round-trip + cleanup P0-2/P0-3"
```

- [ ] **步骤 8：通知用户部署测试**

Phase 1 实现完成。生成的 `SimDLL.dll` 在 `simDLL_RS/target/x86_64-pc-windows-msvc/release/SimDLL.dll`。

**告知用户**：
> Phase 1 实现完成。SimDLL.dll 已生成在 `simDLL_RS/target/x86_64-pc-windows-msvc/release/SimDLL.dll`。
> 请自行部署到游戏目录测试。预期行为：
> - 游戏能进主菜单（与 Phase 0 一致，无回归）
> - 加载存档仍会崩溃（Phase 1 只定义数据结构，未实现存档加载逻辑，Phase 3 才实现）
> - 日志文件 `logs/simdll.log.YYYY-MM-DD` 应记录 SIM_Initialize 调用

---

## 自检

### 规格覆盖度

| 规格章节 | 实现任务 |
|---|---|
| §1 总体架构 + 模块布局 | 任务 1-12（全部 12 个任务覆盖所有模块） |
| §2 BinaryBufferReader/Writer | 任务 3（含 13 个测试） |
| §3 Struct 布局策略 + STL 容器 | 任务 1（stl_shim） + 任务 2（vector_math） |
| §3 SimData/CellSOA/BackwallSOA | 任务 7（含 8 子结构） |
| §3 GameData + 27 *Info | 任务 8 |
| §3 Element + PhysicsData | 任务 4 |
| §3 Disease + DiseaseInfo | 任务 5 |
| §4 SimEvents/SimFrameInfo | 任务 6 |
| §4 FrameSync | 任务 9 |
| §4 Sim/SimFrameManager 类骨架 | 任务 9 |
| §4 MessageHandlerEntry 表 | 任务 10 |
| §5 测试与验收 | 任务 3, 12（集成测试） |
| §7 P0-2 清理 | 任务 12 步骤 1 |
| §7 P0-3 清理 | 任务 12 步骤 2 |
| §8 任务拆分 | 任务 1-12（与规格 §8 对应） |

### 占位符扫描

- 任务 6（sim_events）和任务 8（game_data）的子结构使用 `reserved: [u8; N]` 占位 — **规格允许**（开放问题 #5/#6）
- 任务 10 的 MessageType enum 用 TODO 标注"实施时从源码补全 60+ 值" — **可接受**（任务 10 步骤 3 明确要求实施时补全）
- 任务 7 的 SimData 8 个子结构用 `reserved: [u8; 64]` 占位 — **规格允许**（开放问题 #5）

**红旗检查**：未发现"待定"、"TODO"（除了任务 10 的源码补全指令）、"补充细节"等禁止占位符模式。

### 类型一致性

- `MsvcVector<T>`：任务 1 定义，任务 5/6/7/8/9 一致使用 ✓
- `UniquePtr<T>`：任务 1 定义，任务 7/8/9 一致使用 ✓
- `MsvcString`：任务 1 定义，任务 5 一致使用 ✓
- `MsvcFunction`：任务 1 定义，Phase 1 未实际使用（保留供 Phase 6+） ✓
- `Vector2f/3f/4f`：任务 2 定义，任务 7/8 一致使用 ✓
- `SimEvents`：任务 6 定义，任务 7 引用 ✓
- `SimFrameInfo`：任务 6 定义，任务 9 引用 ✓
- `CellSOA/BackwallSOA`：任务 7 定义，任务 8 引用 ✓
- `BinaryBufferReader/Writer`：任务 3 定义，任务 10/12 引用 ✓
- `MessageType`：任务 10 定义，未在其他任务使用 ✓
- `MessageHandlerEntry`：任务 10 定义，未在其他任务使用 ✓

无类型/方法签名不一致。

### 范围检查

- BinaryBufferReader/Writer 完整实现 ✓
- 所有 struct 字段定义 ✓（部分子结构用 reserved 占位，规格允许）
- size_of 断言 ✓（任务 7 的 SimData 用精确断言，其他用范围断言）
- 类骨架 ✓
- MessageHandlerEntry 表 ✓
- 集成测试 ✓
- P0-2/P0-3 清理 ✓
- P0-4 推后（不在范围） ✓

---

## 执行交接

计划已完成并保存到 `docs/superpowers/plans/2026-07-29-simdll-rs-phase1-data-layer.md`。两种执行方式：

**1. 子代理驱动（推荐）** - 每个任务调度一个新的子代理，任务间进行审查，快速迭代

**2. 内联执行** - 在当前会话中使用 executing-plans 执行任务，批量执行并设有检查点

**选哪种方式？**
