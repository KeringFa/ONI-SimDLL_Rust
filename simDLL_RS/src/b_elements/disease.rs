//! Disease / DiseaseInfo / RangeInfo / ElemGrowthInfo — 病菌表。
//!
//! 字段严格对照源码 00_types_reference.c：
//! - RangeInfo: L12644-12646（16B，union RangeInfo_u_0 = 4×f32）
//! - ElemGrowthInfo: L12633-12642（256B = 0x100，8×MsvcVector）
//! - DiseaseInfo: L12648-12661（336B = 0x150）
//! - Disease: L9347-9350（64B = 0x40，2×MsvcVector）
//!
//! **设计说明**：Disease 是 simDLL 内部全局状态（gDisease，unique_ptr<Disease>），
//! C# 不直接访问 Disease/DiseaseInfo 内存（CreateDiseaseTable 返回 null，
//! 见源码 02_save_load.c L514）。但 simDLL 内部按 0x150 stride 访问 DiseaseInfo
//! 数组（见 01_sim_api.c L107），因此 DiseaseInfo 必须精确 336B。
//! 用 MsvcVector 匹配源码布局，便于 A3 的 CreateDiseaseTable 对照源码读取逻辑。
//!
//! **注意**：DiseaseInfo/Disease 含 MsvcVector（裸指针），不能 derive Copy/Clone。

use crate::a_framework::stl_shim::{MsvcString, MsvcVector};

/// RangeInfo — 范围信息（16B = 4×f32）。
/// 源码 00_types_reference.c L12644-12646（struct RangeInfo { union RangeInfo_u_0 }）。
/// union RangeInfo_u_0 = {struct{minViable,minGrowth,maxGrowth,maxViable} | float[4]}（L12621-12631）。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct RangeInfo {
    pub min_viable: f32,     // L12622 float minViable @0
    pub min_growth: f32,     // L12623 float minGrowth @4
    pub max_growth: f32,     // L12624 float maxGrowth @8
    pub max_viable: f32,     // L12625 float maxViable @12
}

/// ElemGrowthInfo — 元素生长信息（256B = 0x100，8×MsvcVector）。
/// 源码 00_types_reference.c L12633-12642。
#[repr(C)]
#[derive(Default, Debug)]
pub struct ElemGrowthInfo {
    pub under_population_death_rate: MsvcVector<f32>,              // L12634 vector<float> @0
    pub population_half_life: MsvcVector<f32>,                     // L12635 @32
    pub over_population_half_life: MsvcVector<f32>,                // L12636 @64
    pub diffusion_scale: MsvcVector<f32>,                          // L12637 @96
    pub min_count_per_kg: MsvcVector<f32>,                         // L12638 @128
    pub max_count_per_kg: MsvcVector<f32>,                         // L12639 @160
    pub min_diffusion_count: MsvcVector<i32>,                      // L12640 @192
    pub min_diffusion_infestation_tick_count: MsvcVector<u8>,      // L12641 @224
}

/// DiseaseInfo — 病菌信息（336B = 0x150）。
/// 源码 00_types_reference.c L12648-12661。
/// 布局：hashID@0, strength@4, 4×RangeInfo@8-71, ElemGrowthInfo@72-327,
/// radiationKillRate@328(0x148), field8-11@0x14c-0x14f（源码 undefined，Rust 尾部填充）。
#[repr(C)]
#[derive(Default, Debug)]
pub struct DiseaseInfo {
    pub hash_id: u32,                          // L12649 uint hashID @0
    pub strength: f32,                         // L12650 float strength @4
    pub temperature_range: RangeInfo,          // L12651 struct RangeInfo temperatureRange @8
    pub temperature_half_lives: RangeInfo,     // L12652 struct RangeInfo temperatureHalfLives @24
    pub pressure_range: RangeInfo,             // L12653 struct RangeInfo pressureRange @40
    pub pressure_half_lives: RangeInfo,        // L12654 struct RangeInfo pressureHalfLives @56
    pub elem_growth_info: ElemGrowthInfo,      // L12655 struct ElemGrowthInfo elemGrowthInfo @72
    pub radiation_kill_rate: f32,              // L12656 float radiationKillRate @328(0x148)
}

/// Disease — 病菌表（64B = 0x40，2×MsvcVector）。
/// 源码 00_types_reference.c L9347-9350。
/// operator_new(0x40) 分配（见 02_save_load.c L502），精确 64B。
#[repr(C)]
#[derive(Default, Debug)]
pub struct Disease {
    pub diseases: MsvcVector<DiseaseInfo>,     // L9348 vector<DiseaseInfo> @0
    pub disease_names: MsvcVector<MsvcString>, // L9349 vector<basic_string> @32
}

impl Disease {
    /// 构造空 Disease（2 个 MsvcVector 全空）。
    pub fn new() -> Self {
        Self::default()
    }

    /// GetDiseaseIndex — 按 hashId 查找病菌索引。
    /// 对照源码 11_msvcrt_ignored.c L26386-26419。
    ///
    /// 线性搜索 diseases vector，找到 hash_id 匹配的项返回索引（u8）。
    /// 未找到或索引 >= 0xff 时返回 0xff。
    pub fn get_disease_index(&self, hash: u32) -> u8 {
        let diseases = self.diseases.as_slice();
        for (i, d) in diseases.iter().enumerate() {
            if i >= 0xff {
                return 0xff;
            }
            if d.hash_id == hash {
                return i as u8;
            }
        }
        0xff
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn range_info_size_is_16() {
        assert_eq!(size_of::<RangeInfo>(), 16);
    }

    #[test]
    fn elem_growth_info_size_is_256() {
        let actual = size_of::<ElemGrowthInfo>();
        assert_eq!(actual, 256, "ElemGrowthInfo size = {} (0x{:x}), 期望 256 (0x100)", actual, actual);
    }

    #[test]
    fn disease_info_size_is_336() {
        let actual = size_of::<DiseaseInfo>();
        assert_eq!(actual, 336, "DiseaseInfo size = {} (0x{:x}), 期望 336 (0x150)", actual, actual);
    }

    #[test]
    fn disease_size_is_64() {
        let actual = size_of::<Disease>();
        assert_eq!(actual, 64, "Disease size = {} (0x{:x}), 期望 64 (0x40)", actual, actual);
    }

    #[test]
    fn disease_new_is_empty() {
        let d = Disease::new();
        assert!(d.diseases.is_empty());
        assert!(d.disease_names.is_empty());
    }

    #[test]
    fn disease_info_default_is_zeroed() {
        let di = DiseaseInfo::default();
        assert_eq!(di.hash_id, 0);
        assert_eq!(di.strength, 0.0);
        assert_eq!(di.radiation_kill_rate, 0.0);
        assert_eq!(di.temperature_range.min_viable, 0.0);
    }
}
