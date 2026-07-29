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
