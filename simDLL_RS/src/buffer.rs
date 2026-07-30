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

    /// 当前读取偏移（用于错误日志输出）。
    pub fn offset(&self) -> u64 {
        self.m_offset
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

    #[test]
    fn offset_starts_at_zero_and_advances() {
        let bytes = [0u8; 16];
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.offset(), 0);
        r.read_int().unwrap();  // 4 字节
        assert_eq!(r.offset(), 4);
        r.skip(3).unwrap();
        assert_eq!(r.offset(), 7);
    }

    #[test]
    fn offset_reflects_full_read() {
        let bytes = [0u8; 8];
        let mut r = BinaryBufferReader::new(&bytes);
        r.read_bytes(&mut [0u8; 8]).unwrap();
        assert_eq!(r.offset(), 8);
        assert!(r.is_at_end());
        assert_eq!(r.remaining(), 0);
    }
}
