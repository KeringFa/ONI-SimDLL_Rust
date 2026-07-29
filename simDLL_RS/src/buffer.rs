//! BinaryBufferReader/Writer 桩。Phase 1 实现。

#![allow(dead_code)]

use std::os::raw::{c_int, c_schar};

/// 对应原版 `BinaryBufferReader`。Phase 0 仅占位。
pub struct BinaryBufferReader {
    pub data: *const c_schar,
    pub size: c_int,
}

impl BinaryBufferReader {
    pub fn new(size: c_int, data: *const c_schar) -> Self {
        Self { data, size }
    }
}

/// 对应原版 `BinaryBufferWriter`。Phase 0 仅占位。
pub struct BinaryBufferWriter;

/// 对应原版 `Buffer`。Phase 0 仅占位。
pub struct Buffer;

impl Buffer {
    pub fn is_valid(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_is_valid_returns_false_in_stub() {
        let b = Buffer;
        assert!(!b.is_valid());
    }
}
