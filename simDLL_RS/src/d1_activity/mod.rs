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
        self.min_x > self.max_x || self.min_y > self.max_y
    }

    /// 区域内格数。
    pub fn cell_count(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            ((self.max_x - self.min_x + 1) * (self.max_y - self.min_y + 1)) as u64
        }
    }
}

/// 由 ActiveRegion 计算循环边界。
///
/// 对照原版 L145435-145460 的边界推导：`local_17c..local_18c` 是区域坐标的
/// 32 格块（floor 除法），边界 = 块×32+1 与 区域±1 取 min/max，再钳制到
/// width-1/height-1；循环用 `<` 上界 → 实际最大行/列 = min(region.max, w-2/h-2)。
/// 对正常区域（宽高 ≥ 32）该运算等价于：区域矩形直接钳制到内部格。
pub fn compute_region_bounds(sim: &SimData, region: &ActiveRegion) -> RegionBounds {
    let w = sim.width as i64;
    let h = sim.height as i64;
    let interior_x = w - 2;
    let interior_y = h - 2;
    let min_x = (region.min_x as i64).clamp(1, interior_x) as usize;
    let min_y = (region.min_y as i64).clamp(1, interior_y) as usize;
    let max_x = (region.max_x as i64).clamp(1, interior_x) as usize;
    let max_y = (region.max_y as i64).clamp(1, interior_y) as usize;
    RegionBounds {
        min_x,
        min_y,
        max_x,
        max_y,
    }
}

/// 整张内部网格的边界（测试/诊断用，等价于 D1 之前的全网格遍历）。
pub fn full_grid_bounds(sim: &SimData) -> RegionBounds {
    RegionBounds {
        min_x: 1,
        min_y: 1,
        max_x: (sim.width as usize).saturating_sub(2),
        max_y: (sim.height as usize).saturating_sub(2),
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
        assert_eq!(b.cell_count(), 129 * 154);
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
        assert_eq!(b.max_x, 8);
        assert_eq!(b.max_y, 8);
        assert!(!b.is_empty());
    }
}
