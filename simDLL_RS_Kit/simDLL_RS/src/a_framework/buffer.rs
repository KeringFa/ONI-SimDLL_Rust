//! 二进制缓冲区读写器
//!
//! 对照源码 02_save_load.c 中的 BinaryBufferReader 用法。
//! BinaryBufferReader 是 #[repr(C)] 24 字节，匹配原版内存布局。

use std::os::raw::c_char;

/// 缓冲区读取错误。
#[derive(Debug, Clone, PartialEq)]
pub enum BufferError {
    EndOfBuffer,
    InvalidBool,
    InvalidMagic,
    InvalidUtf8,
}

/// 二进制缓冲区读取器（24 字节，#[repr(C)]）。
///
/// 对照源码 BinaryBufferReader 内存布局：
/// - m_offset: u64（当前读取偏移）
/// - m_buffer_data: *const i8（数据指针）
/// - m_buffer_size: u64（数据总大小）
#[repr(C)]
pub struct BinaryBufferReader {
    pub offset: u64,
    pub buffer_data: *const c_char,
    pub buffer_size: u64,
}

impl BinaryBufferReader {
    /// 从字节切片构造读取器。
    pub fn new(data: &[u8]) -> Self {
        Self {
            offset: 0,
            buffer_data: data.as_ptr() as *const c_char,
            buffer_size: data.len() as u64,
        }
    }

    /// 返回当前偏移。
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// 是否已读到末尾。
    pub fn is_at_end(&self) -> bool {
        self.offset >= self.buffer_size
    }

    /// 读取 1 字节 bool（0=false, 非 0=true）。
    pub fn read_bool(&mut self) -> Result<bool, BufferError> {
        let b = self.read_byte()?;
        Ok(b != 0)
    }

    /// 读取 1 字节。
    pub fn read_byte(&mut self) -> Result<u8, BufferError> {
        if self.offset + 1 > self.buffer_size {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        let val = data[self.offset as usize];
        self.offset += 1;
        Ok(val)
    }

    /// 读取 2 字节 short (i16)。
    pub fn read_short(&mut self) -> Result<i16, BufferError> {
        let bytes = self.read_bytes(2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// 读取 2 字节 ushort (u16)。
    pub fn read_ushort(&mut self) -> Result<u16, BufferError> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// 读取 4 字节 int (i32)。
    pub fn read_int(&mut self) -> Result<i32, BufferError> {
        let bytes = self.read_bytes(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// 读取 4 字节 uint (u32)。
    pub fn read_uint(&mut self) -> Result<u32, BufferError> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// 读取 4 字节 float。
    pub fn read_float(&mut self) -> Result<f32, BufferError> {
        let bytes = self.read_bytes(4)?;
        Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// 读取 N 字节。
    pub fn read_bytes(&mut self, count: usize) -> Result<Vec<u8>, BufferError> {
        if self.offset + count as u64 > self.buffer_size {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        let val = data[self.offset as usize..(self.offset as usize + count)].to_vec();
        self.offset += count as u64;
        Ok(val)
    }

    /// 读取 NUL 终止字符串。
    pub fn read_string(&mut self) -> Result<String, BufferError> {
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        let start = self.offset as usize;
        let mut end = start;
        while end < data.len() && data[end] != 0 {
            end += 1;
        }
        if end >= data.len() {
            return Err(BufferError::EndOfBuffer);
        }
        let s = std::str::from_utf8(&data[start..end]).map_err(|_| BufferError::InvalidUtf8)?;
        self.offset = (end + 1) as u64; // 跳过 NUL
        Ok(s.to_string())
    }

    /// 跳过 N 字节。
    pub fn skip(&mut self, count: usize) -> Result<(), BufferError> {
        if self.offset + count as u64 > self.buffer_size {
            return Err(BufferError::EndOfBuffer);
        }
        self.offset += count as u64;
        Ok(())
    }

    /// 剩余可读字节数。
    pub fn remaining(&self) -> u64 {
        self.buffer_size.saturating_sub(self.offset)
    }

    /// 在指定绝对偏移读取 1 字节（不移动当前偏移）。
    /// 越界检查用 checked_add 防 offset 溢出 panic（2026-08-01 统一修规格）。
    pub fn read_u8_at(&self, offset: u64) -> Result<u8, BufferError> {
        if offset.checked_add(1).map_or(true, |end| end > self.buffer_size) {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        Ok(data[offset as usize])
    }

    /// 在指定绝对偏移读取 u16（不移动当前偏移）。
    /// 越界检查用 checked_add 防 offset 溢出 panic（2026-08-01 统一修规格）。
    pub fn read_u16_at(&self, offset: u64) -> Result<u16, BufferError> {
        if offset.checked_add(2).map_or(true, |end| end > self.buffer_size) {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        let o = offset as usize;
        Ok(u16::from_le_bytes([data[o], data[o + 1]]))
    }

    /// 在指定绝对偏移读取 i32（不移动当前偏移）。
    /// 越界检查用 checked_add 防 offset 溢出 panic（2026-08-01 统一修规格）。
    pub fn read_i32_at(&self, offset: u64) -> Result<i32, BufferError> {
        if offset.checked_add(4).map_or(true, |end| end > self.buffer_size) {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        let o = offset as usize;
        Ok(i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]))
    }

    /// 在指定绝对偏移读取 f32（不移动当前偏移）。
    /// 越界检查用 checked_add 防 offset 溢出 panic（2026-08-01 统一修规格）。
    pub fn read_f32_at(&self, offset: u64) -> Result<f32, BufferError> {
        if offset.checked_add(4).map_or(true, |end| end > self.buffer_size) {
            return Err(BufferError::EndOfBuffer);
        }
        let data = unsafe { std::slice::from_raw_parts(self.buffer_data as *const u8, self.buffer_size as usize) };
        let o = offset as usize;
        Ok(f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]))
    }
}

/// 二进制缓冲区写入器（Vec<u8> 后端，无 #[repr(C)]）。
pub struct BinaryBufferWriter {
    buffer: Vec<u8>,
}

impl BinaryBufferWriter {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn write_byte(&mut self, val: u8) {
        self.buffer.push(val);
    }

    pub fn write_bool(&mut self, val: bool) {
        self.buffer.push(if val { 1 } else { 0 });
    }

    pub fn write_short(&mut self, val: i16) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    pub fn write_ushort(&mut self, val: u16) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    pub fn write_int(&mut self, val: i32) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    pub fn write_uint(&mut self, val: u32) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    pub fn write_float(&mut self, val: f32) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    pub fn write_bytes(&mut self, val: &[u8]) {
        self.buffer.extend_from_slice(val);
    }

    pub fn write_string(&mut self, val: &str) {
        self.buffer.extend_from_slice(val.as_bytes());
        self.buffer.push(0); // NUL 终止
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }
}

impl Default for BinaryBufferWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_size_is_24() {
        assert_eq!(std::mem::size_of::<BinaryBufferReader>(), 24);
    }

    #[test]
    fn read_write_byte_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_byte(42);
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_byte().unwrap(), 42);
        assert!(r.is_at_end());
    }

    #[test]
    fn read_write_int_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(-12345);
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_int().unwrap(), -12345);
    }

    #[test]
    fn read_write_float_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_float(3.14);
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_float().unwrap(), 3.14);
    }

    #[test]
    fn read_write_string_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_string("hello");
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_string().unwrap(), "hello");
    }

    #[test]
    fn read_write_ushort_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_ushort(60000);
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_ushort().unwrap(), 60000);
    }

    #[test]
    fn read_write_bool_round_trip() {
        let mut w = BinaryBufferWriter::new();
        w.write_bool(true);
        w.write_bool(false);
        let bytes = w.into_bytes();
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.read_bool().unwrap(), true);
        assert_eq!(r.read_bool().unwrap(), false);
    }

    #[test]
    fn read_past_end_returns_error() {
        let bytes = vec![1u8, 2, 3];
        let mut r = BinaryBufferReader::new(&bytes);
        r.read_int().expect_err("should fail");
    }

    #[test]
    fn skip_advances_offset() {
        let bytes = vec![1u8, 2, 3, 4, 5];
        let mut r = BinaryBufferReader::new(&bytes);
        r.skip(2).unwrap();
        assert_eq!(r.offset(), 2);
        assert_eq!(r.read_byte().unwrap(), 3);
    }

    #[test]
    fn offset_returns_current_position() {
        let bytes = vec![1u8, 2, 3];
        let mut r = BinaryBufferReader::new(&bytes);
        assert_eq!(r.offset(), 0);
        r.read_byte().unwrap();
        assert_eq!(r.offset(), 1);
    }

    #[test]
    fn read_bytes_returns_correct_slice() {
        let bytes = vec![1u8, 2, 3, 4, 5];
        let mut r = BinaryBufferReader::new(&bytes);
        let chunk = r.read_bytes(3).unwrap();
        assert_eq!(chunk, vec![1, 2, 3]);
        assert_eq!(r.offset(), 3);
    }

    #[test]
    fn invalid_bool_variant_exists() {
        let _err = BufferError::InvalidBool;
    }

    #[test]
    fn invalid_magic_variant_exists() {
        let _err = BufferError::InvalidMagic;
    }

    #[test]
    fn writer_default_works() {
        let w = BinaryBufferWriter::default();
        assert!(w.into_bytes().is_empty());
    }

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

    #[test]
    fn read_at_extreme_offset_returns_error_not_panic() {
        // 回归（2026-08-01 隐患 3）：offset 接近 u64::MAX 时，
        // checked_add 溢出应返回 Err(EndOfBuffer)，不得 panic
        let bytes = vec![0u8; 4];
        let r = BinaryBufferReader::new(&bytes);
        assert!(r.read_u8_at(u64::MAX).is_err());
        assert!(r.read_u16_at(u64::MAX - 1).is_err());
        assert!(r.read_i32_at(u64::MAX - 2).is_err());
        assert!(r.read_f32_at(u64::MAX - 2).is_err());
    }
}
