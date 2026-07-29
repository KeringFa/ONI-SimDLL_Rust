//! `FrameSync` 桩。对应原版 `FrameSync::clear`。
//! Phase 1 实现完整字段。

#![allow(dead_code)]

/// 对应原版 `FrameSync`。Phase 0 仅占位。
#[derive(Default)]
pub struct FrameSync;

impl FrameSync {
    /// 对应原版 `FrameSync::clear(&gFrameSync)`。
    pub fn clear(&mut self) {
        // Phase 0 无字段，no-op
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_is_safe_on_default() {
        let mut fs = FrameSync::default();
        fs.clear();
    }
}
