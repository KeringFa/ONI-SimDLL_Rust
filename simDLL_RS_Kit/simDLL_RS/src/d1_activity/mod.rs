//! D1 活动区域处理。
//!
//! 对照原版 UpdateData 的区域循环（SimDLL_Source.c L144400+）：主物理只算活动区域
//! （C# 每帧经 NewGameFrame 下发的**已发现世界整块矩形**，解析后存
//! SimFrameManager.active_regions → SimData.active_regions）。
//! 未发现星球不在区域列表 → 整块不参与模拟（"UNKNOWN 黑格永不变化"的 sim 侧机制）。
//!
//! 2026-08-04 实现：此前 update_data 的主物理循环遍历整张簇网格
//! （DLC 22.7 万格），原版只算已发现世界矩形（如 1.99 万格）——11.5× 浪费，
//! 这是 DLC 卡顿的根因。本模块提供区域边界计算，物理循环据此裁剪。

use crate::a_framework::sim_data::{ActiveRegion, SimData};

/// 区域循环边界（sim 内部坐标，含 +1 边界偏移；已钳制到内部格范围 1..w-2 / 1..h-2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionBounds {
    pub min_x: usize,
    pub min_y: usize,
    pub max_x: usize,
    pub max_y: usize,
}

impl RegionBounds {
    /// 空区域（min > max）→ 循环自然跳过。
    pub fn is_empty(&self) -> bool {
        self.min_x >= self.max_x || self.min_y >= self.max_y
    }

    /// 区域内格数。
    pub fn cell_count(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            ((self.max_x - self.min_x) * (self.max_y - self.min_y)) as u64
        }
    }
}

/// 由 ActiveRegion 计算普通区域循环边界（物理任务用）。
///
/// 对照原版 UpdateData：物理任务（温度/气体/液体/辐射）直接用区域原始边界
/// （sim 坐标 +1 偏移），本函数仅做内部格钳制 [1, w-2]×[1, h-2]。
/// ⚠️ 注意：/32 格块运算属于 **PostProcess 专属**（见 compute_post_process_bounds），
/// 不适用于本函数（2026-08-10 文档修正，D-9）。
pub fn compute_region_bounds(sim: &SimData, region: &ActiveRegion) -> RegionBounds {
    let w = sim.width as i64;
    let h = sim.height as i64;
    let interior_x = w - 2;
    let interior_y = h - 2;
    // 宽/高 < 3 时 interior < 1，clamp(1, interior) min>max 会 panic
    // （原版用 min/max 宏只产生退化边界不崩）——先早退返回空区域，循环自然跳过。
    if w < 3 || h < 3 {
        return RegionBounds { min_x: 1, min_y: 1, max_x: 0, max_y: 0 };
    }
    let min_x = (region.min_x as i64).clamp(1, interior_x) as usize;
    let min_y = (region.min_y as i64).clamp(1, interior_y) as usize;
    // max 为排他上界：最大有效 = interior+1 = w-1（原版 `x < width-1`）
    let max_x = (region.max_x as i64).clamp(1, interior_x + 1) as usize;
    let max_y = (region.max_y as i64).clamp(1, interior_y + 1) as usize;
    RegionBounds {
        min_x,
        min_y,
        max_x,
        max_y,
    }
}

/// PostProcess 专用边界（原版 SimDLL_Source.c L145435-145460）。
///
/// 原版仅 PostProcessCell 循环用此边界，其余阶段（温度/气体/液体/辐射）用原始
/// 区域矩形（compute_region_bounds）。公式：
/// ```text
/// x_min' = max(min_x, floor((min_x-1)/32)*32 + 1)   // 恒等于 min_x（no-op）
/// x_max' = min(w-1, floor((max_x+30)/32)*32 + 1, max_x + 1)  // max+1 怪癖
/// ```
/// max+1：原版 PostProcess 会处理区域右/下边界外的 1 格（★3 复刻）。
pub fn compute_post_process_bounds(sim: &SimData, region: &ActiveRegion) -> RegionBounds {
    let w = sim.width as i64;
    let h = sim.height as i64;
    if w < 3 || h < 3 {
        return RegionBounds { min_x: 1, min_y: 1, max_x: 0, max_y: 0 };
    }
    let floor_div32 = |x: i64| x.div_euclid(32);
    let min_x = region.min_x as i64;
    let min_y = region.min_y as i64;
    let max_x = region.max_x as i64;
    let max_y = region.max_y as i64;
    let x_min = min_x.max(floor_div32(min_x - 1) * 32 + 1);
    let y_min = min_y.max(floor_div32(min_y - 1) * 32 + 1);
    let x_max = (floor_div32(max_x + 30) * 32 + 1).min(max_x + 1).min(w - 1);
    let y_max = (floor_div32(max_y + 30) * 32 + 1).min(max_y + 1).min(h - 1);
    RegionBounds {
        min_x: x_min.max(1) as usize,
        min_y: y_min.max(1) as usize,
        max_x: x_max as usize,
        max_y: y_max as usize,
    }
}

/// 整张内部网格的边界（测试/诊断用，等价于 D1 之前的全网格遍历）。
pub fn full_grid_bounds(sim: &SimData) -> RegionBounds {
    RegionBounds {
        min_x: 1,
        min_y: 1,
        // 排他上界：内部最后格 w-2 的后一位 = w-1
        max_x: (sim.width as usize).saturating_sub(1),
        max_y: (sim.height as usize).saturating_sub(1),
    }
}

/// 整张网格的区域覆盖统计（D1 诊断/测试用）。
pub fn grid_coverage(sim: &SimData) -> (u64, usize, u64) {
    let total = (sim.width as u64).saturating_mul(sim.height as u64);
    let regions = sim.active_regions.as_slice();
    let mut cells = 0u64;
    for r in regions {
        cells += compute_region_bounds(sim, r).cell_count();
    }
    (total, regions.len(), cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::sim_data::SimData;

    fn test_sim(w: i32, h: i32) -> SimData {
        let mut sd = SimData::new_zeroed();
        sd.width = w;
        sd.height = h;
        sd
    }

    #[test]
    fn region_bounds_clamp_to_interior() {
        let sd = test_sim(1162, 196);
        let r = ActiveRegion {
            min_x: 83,
            min_y: 1,
            max_x: 211,
            max_y: 154,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        };
        let b = compute_region_bounds(&sd, &r);
        assert_eq!(b.min_x, 83);
        assert_eq!(b.min_y, 1);
        assert_eq!(b.max_x, 211);
        assert_eq!(b.max_y, 154);
        // max 为排他上界：x 覆盖 83..211 = 128 列，y 覆盖 1..154 = 153 行
        assert_eq!(b.cell_count(), 128 * 153);
    }

    #[test]
    fn region_bounds_clamp_oversized_to_grid() {
        let sd = test_sim(10, 10);
        let r = ActiveRegion {
            min_x: -5,
            min_y: 0,
            max_x: 999,
            max_y: 999,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        };
        let b = compute_region_bounds(&sd, &r);
        assert_eq!(b.min_x, 1);
        assert_eq!(b.min_y, 1);
        // 排他上界钳到 interior+1 = 9（覆盖内部 1..8）
        assert_eq!(b.max_x, 9);
        assert_eq!(b.max_y, 9);
        assert!(!b.is_empty());
    }

    #[test]
    fn post_process_bounds_match_original_formula() {
        let sd = test_sim(40, 40);
        // max≡1 mod 32 → 块项生效：x_max = max（非 max+1）
        let r = ActiveRegion {
            min_x: 1,
            min_y: 1,
            max_x: 33,
            max_y: 33,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        };
        let b = compute_post_process_bounds(&sd, &r);
        assert_eq!(b.min_x, 1);
        assert_eq!(b.max_x, 33, "max≡1 mod 32 时块项 min(33,34)=33");
        assert_eq!(b.max_y, 33);

        // 其他 max → max+1 怪癖：x_max = max+1
        let r2 = ActiveRegion {
            min_x: 1,
            min_y: 1,
            max_x: 34,
            max_y: 34,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        };
        let b2 = compute_post_process_bounds(&sd, &r2);
        assert_eq!(b2.max_x, 35, "max+1 怪癖：min(39,65,35)=35");
        assert_eq!(b2.max_y, 35);
    }
}

