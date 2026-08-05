//! 病菌格子模拟：扩散（Disease::UpdateCells L26646）+ 生长（Disease::PostProcess L26424）。
//!
//! 对照原版 11_msvcrt_ignored.c + SimDLL_Source.c（阶段 2）：
//! - GetDiffusionScale（L26329）：扩散门控（minDiffusionCount / minDiffusionInfestationTickCount / diffusionScale）；
//! - UpdateCells（L26646-26812）：相邻两格病菌扩散（读 cells 稳定缓冲，写 updatedCells，经 AddDiseaseToCell）；
//! - PostProcess（L26424-26640）：逐格生长（温度因子 + 种群因子 + 辐射杀伤 + 误差结转 + 清除 + tick 递增）。
//!
//! 公式对照 C# Klei.AI.Disease（HalfLifeToGrowthRate / CalculateRangeHalfLife）与
//! Klei.AI.DiseaseGrowthRules.ElemGrowthInfo.CalculateDiseaseCountDelta；dt 固定 0.2。
use crate::a_framework::sim_data::SimData;
use crate::b_elements::disease::RangeInfo;

/// HalfLifeToGrowthRate — 半活期转生长率（C# Klei.AI.Disease L314-322）。
///
/// half_life==0 → 0；half_life==+INF → 1；否则 `powf(2, −dt/half_life)`。
pub(crate) fn half_life_to_growth_rate(half_life: f32, dt: f32) -> f32 {
    if half_life == 0.0 {
        return 0.0;
    }
    if half_life == f32::INFINITY {
        return 1.0;
    }
    f32::powf(2.0, -dt / half_life)
}

/// CalculateRangeHalfLife — 温度带间插值半活期（C# Klei.AI.Disease L348-381）。
///
/// 找第一个 `temp <= range[i]`（range = minViable/minGrowth/maxGrowth/maxViable），
/// lower = max(i−1, 0)、upper = i；无匹配 → lower=upper=3；
/// 最优带 [minGrowth,maxGrowth]（lower==1 && upper==2）或任一半活期 INF → INF；
/// 否则按 t=(temp−range[lower])/(range[upper]−range[lower])（分母≤0 → t=0）插值。
pub(crate) fn calculate_range_half_life(
    temp: f32,
    range: &RangeInfo,
    half_lives: &RangeInfo,
) -> f32 {
    // 找第一个 temp <= range[i]（range 顺序：minViable/minGrowth/maxGrowth/maxViable）
    let vals = [
        range.min_viable,
        range.min_growth,
        range.max_growth,
        range.max_viable,
    ];
    let hls = [
        half_lives.min_viable,
        half_lives.min_growth,
        half_lives.max_growth,
        half_lives.max_viable,
    ];
    let mut lower = 3usize;
    let mut upper = 3usize;
    for i in 0..4usize {
        if temp <= vals[i] {
            lower = i.saturating_sub(1);
            upper = i;
            break;
        }
    }
    // 最优带 [minGrowth, maxGrowth]（lower==1 && upper==2）→ INF（生长率 1.0）
    if lower == 1 && upper == 2 {
        return f32::INFINITY;
    }
    if hls[lower] == f32::INFINITY || hls[upper] == f32::INFINITY {
        return f32::INFINITY;
    }
    let mut t = 0.0;
    let span = vals[upper] - vals[lower];
    if span > 0.0 {
        t = (temp - vals[lower]) / span;
    }
    (1.0 - t) * hls[lower] + t * hls[upper]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;

    #[test]
    fn half_life_to_growth_rate_zero_is_zero() {
        assert_eq!(half_life_to_growth_rate(0.0, 0.2), 0.0);
    }

    #[test]
    fn half_life_to_growth_rate_infinity_is_one() {
        assert_eq!(half_life_to_growth_rate(f32::INFINITY, 0.2), 1.0);
    }

    #[test]
    fn half_life_to_growth_rate_matches_pow2() {
        // 2^(-0.2/1.0) = 0.870550563296124
        assert!((half_life_to_growth_rate(1.0, 0.2) - 0.870550563296124).abs() < 1e-6);
        // 2^(-0.2/10.0) = 2^(-0.02) = 0.9862327044935591
        assert!((half_life_to_growth_rate(10.0, 0.2) - 0.9862327044935591).abs() < 1e-6);
    }

    fn test_range() -> RangeInfo {
        RangeInfo {
            min_viable: 0.0,
            min_growth: 20.0,
            max_growth: 40.0,
            max_viable: 60.0,
        }
    }

    fn test_hl() -> RangeInfo {
        RangeInfo {
            min_viable: 100.0,
            min_growth: 200.0,
            max_growth: 300.0,
            max_viable: 400.0,
        }
    }

    #[test]
    fn calculate_range_half_life_optimal_band_is_infinity() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        assert_eq!(calculate_range_half_life(30.0, &test_range(), &test_hl()), f32::INFINITY);
    }

    #[test]
    fn calculate_range_half_life_interpolates_band() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        // temp=10 ∈ [0,20)：t=0.5 → lerp(100,200)=150
        assert_eq!(calculate_range_half_life(10.0, &test_range(), &test_hl()), 150.0);
    }

    #[test]
    fn calculate_range_half_life_clamps_below_and_above() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        assert_eq!(calculate_range_half_life(-5.0, &test_range(), &test_hl()), 100.0);
        assert_eq!(calculate_range_half_life(80.0, &test_range(), &test_hl()), 400.0);
    }

    #[test]
    fn calculate_range_half_life_inf_propagates() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        let hl = RangeInfo {
            min_viable: f32::INFINITY,
            ..test_hl()
        };
        assert_eq!(calculate_range_half_life(10.0, &test_range(), &hl), f32::INFINITY);
    }
}
