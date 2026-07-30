//! Disease struct — 病原体定义。
//!
//! 字段对照源码：00_types_reference.c L9347-9350。
//! 原版 Disease = 64 字节 = 2 × MsvcVector(32B)。
//!
//! **修订记录（Phase 1 Task 7 派发前）**：原 Task 5 实现使用 24B MsvcVector +
//! 16B reserved 凑 64B（基于"vector=24B"的错误前提）。修正 MsvcVector 为 32B 后，
//! 2 × 32B = 64B 已精确匹配原版 sizeof，无需 reserved 占位。开放问题 #3 因此关闭
//! （剩余 16B 实为 MSVC iterator debugging 的 `_Container_proxy*` 头部分摊到两个
//! vector 上，每 vector 各 8B）。

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
    pub diseases: MsvcVector<DiseaseInfo>,    // L9348 (32B)
    pub disease_names: MsvcVector<MsvcString>, // L9349 (32B)
}

impl Disease {
    /// 构造空 Disease（2 个空 MsvcVector）。
    pub fn new() -> Self {
        Self {
            diseases: MsvcVector::new(),
            disease_names: MsvcVector::new(),
        }
    }
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

    #[test]
    fn disease_new_empty_vectors() {
        let d = Disease::new();
        assert!(d.diseases.is_empty());
        assert!(d.disease_names.is_empty());
    }
}
