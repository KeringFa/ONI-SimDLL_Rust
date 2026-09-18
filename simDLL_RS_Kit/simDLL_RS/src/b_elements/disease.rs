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

use crate::a_framework::buffer::{BinaryBufferReader, BufferError};
use crate::a_framework::stl_shim::{MsvcString, MsvcVector};
use crate::b_elements::elements_table;

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

    /// Disease::Disease 构造 — 从 CreateDiseaseTable 消息流解析病菌表。
    ///
    /// 对照原版 11_msvcrt_ignored.c L25718-26091 + C# SimMessages.CreateDiseaseTable：
    /// ```text
    /// count(i32) + elements_count(i32)
    /// repeat count:
    ///   name KleiString(u32 len + UTF-8) + hash_id(u32) + strength(f32)
    ///   + 4×RangeInfo(4×f32: minViable/minGrowth/maxGrowth/maxViable)
    ///   + radiation_kill_rate(f32)
    ///   + elements_count × ElemGrowthInfo(6×f32 + min_diffusion_count i32
    ///     + min_diffusion_infestation_tick_count u8)
    /// ```
    /// elements_count 与元素表数量不匹配时：原版 DebugBreak（仅调试器）+ 
    /// KCrashReporterReportMessage，本项目 warn 不中断（与 CreateElementsTable 同款）。
    pub fn from_stream(reader: &mut BinaryBufferReader) -> Result<Disease, BufferError> {
        let count = reader.read_int()?;
        let elements_count = reader.read_int()?;
        let table_count = elements_table::get_element_count_pub() as i64;
        if elements_count as i64 != table_count {
            tracing::warn!(
                elements_count,
                table_count,
                "CreateDiseaseTable: elements_count != element table count (original DebugBreak)"
            );
        }
        let count = count.max(0) as usize;
        let elements_count = elements_count.max(0) as usize;

        let mut disease = Disease::new();
        disease.disease_names.resize(count, MsvcString::default());
        for i in 0..count {
            // KleiString：u32 字节长度 + UTF-8 字节（C# WriteKleiString，无 NUL）
            let name_len = reader.read_uint()? as usize;
            let name_bytes = reader.read_bytes(name_len)?;
            let hash_id = reader.read_uint()?;
            let strength = reader.read_float()?;
            let temperature_range = read_range_info(reader)?;
            let temperature_half_lives = read_range_info(reader)?;
            let pressure_range = read_range_info(reader)?;
            let pressure_half_lives = read_range_info(reader)?;
            let radiation_kill_rate = reader.read_float()?;

            // 8 个 ElemGrowthInfo vector resize 到 elements_count（对照 L25790-25804）
            let mut eg = ElemGrowthInfo::default();
            eg.under_population_death_rate.resize(elements_count, 0.0);
            eg.population_half_life.resize(elements_count, 0.0);
            eg.over_population_half_life.resize(elements_count, 0.0);
            eg.diffusion_scale.resize(elements_count, 0.0);
            eg.min_count_per_kg.resize(elements_count, 0.0);
            eg.max_count_per_kg.resize(elements_count, 0.0);
            eg.min_diffusion_count.resize(elements_count, 0);
            eg.min_diffusion_infestation_tick_count.resize(elements_count, 0);
            // 逐元素读取（对照 L25808-25875：6×f32 + i32 + u8）
            for j in 0..elements_count {
                eg.under_population_death_rate.set(j, reader.read_float()?);
                eg.population_half_life.set(j, reader.read_float()?);
                eg.over_population_half_life.set(j, reader.read_float()?);
                eg.diffusion_scale.set(j, reader.read_float()?);
                eg.min_count_per_kg.set(j, reader.read_float()?);
                eg.max_count_per_kg.set(j, reader.read_float()?);
                eg.min_diffusion_count.set(j, reader.read_int()?);
                eg.min_diffusion_infestation_tick_count.set(j, reader.read_byte()?);
            }

            let info = DiseaseInfo {
                hash_id,
                strength,
                temperature_range,
                temperature_half_lives,
                pressure_range,
                pressure_half_lives,
                elem_growth_info: eg,
                radiation_kill_rate,
            };
            let mut name = MsvcString::default();
            name.set_bytes(&name_bytes);
            disease.disease_names.set(i, name);
            // DiseaseInfo 非 Copy（含 MsvcVector），用 push_unchecked（2x 增长）。
            disease.diseases.push_unchecked(info);
        }
        Ok(disease)
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

/// 读取一个 RangeInfo（4×f32，顺序 minViable/minGrowth/maxGrowth/maxViable）。
fn read_range_info(reader: &mut BinaryBufferReader) -> Result<RangeInfo, BufferError> {
    Ok(RangeInfo {
        min_viable: reader.read_float()?,
        min_growth: reader.read_float()?,
        max_growth: reader.read_float()?,
        max_viable: reader.read_float()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
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

    /// 按 C# SimMessages.CreateDiseaseTable 写入顺序构造消息字节：
    /// count(i32) + elements_count(i32) +
    /// 每病菌：KleiString(名字) + hash(u32) + strength(f32) + 4×RangeInfo(4×f32) +
    ///         radiationKillRate(f32) + elements_count×(6×f32 + i32 + u8)。
    fn build_disease_table_msg(count: i32, elements_count: i32, hash: u32) -> Vec<u8> {
        let mut w = BinaryBufferWriter::new();
        w.write_int(count);
        w.write_int(elements_count);
        for _ in 0..count {
            w.write_int(11); // KleiString 字节长度
            w.write_bytes(b"TestDisease");
            w.write_uint(hash);
            w.write_float(1.5); // strength
            // 4×RangeInfo（minViable/minGrowth/maxGrowth/maxViable）
            for v in [
                1.0, 2.0, 3.0, 4.0,   // temperature_range
                5.0, 6.0, 7.0, 8.0,   // temperature_half_lives
                9.0, 10.0, 11.0, 12.0, // pressure_range
                13.0, 14.0, 15.0, 16.0, // pressure_half_lives
            ] {
                w.write_float(v);
            }
            w.write_float(17.5); // radiationKillRate
            for i in 0..elements_count {
                w.write_float(0.1 + i as f32);
                w.write_float(0.2 + i as f32);
                w.write_float(0.3 + i as f32);
                w.write_float(0.4 + i as f32);
                w.write_float(0.5 + i as f32);
                w.write_float(0.6 + i as f32);
                w.write_int(7 + i);
                w.write_byte(8 + i as u8);
            }
        }
        w.into_bytes()
    }

    #[test]
    fn disease_from_stream_parses_all_fields() {
        let data = build_disease_table_msg(1, 2, 0x11223344);
        let mut reader = BinaryBufferReader::new(&data);
        let d = Disease::from_stream(&mut reader).expect("parse ok");

        assert_eq!(d.disease_names.len(), 1);
        assert_eq!(d.disease_names.as_slice()[0].as_bytes(), b"TestDisease");

        let di = &d.diseases.as_slice()[0];
        assert_eq!(di.hash_id, 0x11223344);
        assert_eq!(di.strength, 1.5);
        assert_eq!(di.temperature_range.min_viable, 1.0);
        assert_eq!(di.temperature_range.max_viable, 4.0);
        assert_eq!(di.temperature_half_lives.min_growth, 6.0);
        assert_eq!(di.temperature_half_lives.max_viable, 8.0);
        assert_eq!(di.pressure_range.min_viable, 9.0);
        assert_eq!(di.pressure_range.max_growth, 11.0);
        assert_eq!(di.pressure_half_lives.min_viable, 13.0);
        assert_eq!(di.pressure_half_lives.max_viable, 16.0);
        assert_eq!(di.radiation_kill_rate, 17.5);

        let eg = &di.elem_growth_info;
        assert_eq!(eg.under_population_death_rate.as_slice(), &[0.1, 1.1]);
        assert_eq!(eg.population_half_life.as_slice(), &[0.2, 1.2]);
        assert_eq!(eg.over_population_half_life.as_slice(), &[0.3, 1.3]);
        assert_eq!(eg.diffusion_scale.as_slice(), &[0.4, 1.4]);
        assert_eq!(eg.min_count_per_kg.as_slice(), &[0.5, 1.5]);
        assert_eq!(eg.max_count_per_kg.as_slice(), &[0.6, 1.6]);
        assert_eq!(eg.min_diffusion_count.as_slice(), &[7, 8]);
        assert_eq!(eg.min_diffusion_infestation_tick_count.as_slice(), &[8, 9]);
    }

    #[test]
    fn disease_from_stream_multiple_diseases_preserves_order() {
        let data = build_disease_table_msg(2, 1, 0x99);
        let mut reader = BinaryBufferReader::new(&data);
        let d = Disease::from_stream(&mut reader).expect("parse ok");
        assert_eq!(d.diseases.len(), 2);
        assert_eq!(d.disease_names.len(), 2);
        assert_eq!(d.diseases.as_slice()[0].hash_id, 0x99);
        assert_eq!(d.diseases.as_slice()[1].hash_id, 0x99);
        assert_eq!(d.diseases.as_slice()[0].elem_growth_info.min_diffusion_count.as_slice(), &[7]);
    }

    #[test]
    fn disease_from_stream_elements_count_mismatch_is_warn_only() {
        // elements_count 与元素表数量（测试环境无表 = 0）不匹配：
        // 原版 DebugBreak + KCrashReporterReportMessage，本项目 warn 不中断、表仍可读。
        let data = build_disease_table_msg(1, 2, 0x1234);
        let mut reader = BinaryBufferReader::new(&data);
        let d = Disease::from_stream(&mut reader).expect("mismatch should still parse");
        assert_eq!(d.diseases.len(), 1);
        assert_eq!(d.diseases.as_slice()[0].hash_id, 0x1234);
    }

    #[test]
    fn disease_from_stream_empty_is_empty_table() {
        let data = build_disease_table_msg(0, 0, 0);
        let mut reader = BinaryBufferReader::new(&data);
        let d = Disease::from_stream(&mut reader).expect("empty parse ok");
        assert!(d.diseases.is_empty());
        assert!(d.disease_names.is_empty());
    }

    #[test]
    fn disease_from_stream_truncated_returns_err() {
        let data = build_disease_table_msg(1, 2, 0x1234);
        let mut reader = BinaryBufferReader::new(&data[..data.len() - 4]);
        assert!(Disease::from_stream(&mut reader).is_err());
    }

    #[test]
    fn disease_from_stream_long_name_roundtrips() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(1);
        w.write_int(0);
        let long_name = b"RadiationSickness"; // 16 字节，超过 SSO 15 字节上限
        w.write_int(long_name.len() as i32);
        w.write_bytes(long_name);
        w.write_uint(0x5);
        w.write_float(1.0);
        for _ in 0..16 {
            w.write_float(0.0);
        }
        w.write_float(0.5);
        // 注意：必须先把字节绑定到变量，reader 持有裸指针；若写成
        // `new(&w.into_bytes())`，临时 Vec 在语句结束即释放 → 悬垂指针
        // （单测侥幸通过、全量下堆复用后读到脏数据，已实测踩坑）。
        let bytes = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&bytes);
        let d = Disease::from_stream(&mut reader).expect("parse ok");
        assert_eq!(d.disease_names.as_slice()[0].as_bytes(), long_name);
    }
}
