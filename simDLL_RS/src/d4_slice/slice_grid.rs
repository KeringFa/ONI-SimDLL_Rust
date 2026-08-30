//! 切片网格：把每个活动区域切成 ~32x32 的矩形工作单元。

use crate::d1_activity::RegionBounds;

/// 一个切片。边界为排他上界，与 `RegionBounds` 语义一致。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SliceBounds {
    /// 全局序号（区域序 → 行优先），用于事件定序合并。
    pub index: usize,
    /// 所属区域在 `active_regions` 中的下标，用于事件定序合并。
    pub region_index: usize,
    pub min_x: usize,
    pub min_y: usize,
    pub max_x: usize,
    pub max_y: usize,
}

impl SliceBounds {
    pub fn is_empty(&self) -> bool {
        self.min_x >= self.max_x || self.min_y >= self.max_y
    }
}

/// 一次构建出的完整切片列表。
pub struct SliceGrid {
    pub slices: Vec<SliceBounds>,
}

impl SliceGrid {
    /// 把每个区域切成 `size x size` 的切片；边界切片自动收窄。
    /// 切片不跨区域 —— 区域之间本就不相邻。
    pub fn build(regions: &[RegionBounds], size: usize) -> Self {
        let size = size.max(1);
        let mut slices = Vec::new();
        let mut index = 0usize;
        for (region_index, r) in regions.iter().enumerate() {
            if r.is_empty() {
                continue;
            }
            let mut y = r.min_y;
            while y < r.max_y {
                let y_end = (y + size).min(r.max_y);
                let mut x = r.min_x;
                while x < r.max_x {
                    let x_end = (x + size).min(r.max_x);
                    slices.push(SliceBounds {
                        index,
                        region_index,
                        min_x: x,
                        min_y: y,
                        max_x: x_end,
                        max_y: y_end,
                    });
                    index += 1;
                    x = x_end;
                }
                y = y_end;
            }
        }
        Self { slices }
    }
}

/// 切片缓存：仅在签名（区域列表 + 切片尺寸 + 网格尺寸）变化时重建，
/// 避免每帧分配。
#[derive(Default)]
pub struct SliceCache {
    signature: Option<(Vec<RegionBounds>, usize, usize, usize)>,
    grid: Option<SliceGrid>,
    /// 重建次数。缓存契约「签名未变则不重建」本身不可观测 ——
    /// `grid` 内联存储使 `&SliceGrid` 地址在重建前后相同，无法用作信号，
    /// 故显式记账，供测试断言与运行时诊断使用。
    generation: u64,
}

impl SliceCache {
    pub fn get(
        &mut self,
        regions: &[RegionBounds],
        size: usize,
        width: usize,
        height: usize,
    ) -> &SliceGrid {
        let sig = (regions.to_vec(), size, width, height);
        let changed = self
            .signature
            .as_ref()
            .map(|s| *s != sig)
            .unwrap_or(true);
        if changed {
            self.grid = Some(SliceGrid::build(regions, size));
            self.signature = Some(sig);
            self.generation = self.generation.wrapping_add(1);
        }
        self.grid.as_ref().expect("grid 已初始化")
    }

    /// 重建次数。首次 `get` 后为 1；签名未变则不再增长。
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn invalidate(&mut self) {
        self.signature = None;
        self.grid = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d1_activity::RegionBounds;

    #[test]
    fn grid_splits_region_into_32x32_tiles() {
        // 区域 70x40，min=(1,1) → 切片应为 3 列 x 2 行 = 6 片
        let b = RegionBounds { min_x: 1, min_y: 1, max_x: 71, max_y: 41 };
        let g = SliceGrid::build(&[b], 32);
        assert_eq!(g.slices.len(), 6, "70x40 按 32 切应为 3x2=6 片");
        // 第一片
        assert_eq!((g.slices[0].min_x, g.slices[0].min_y), (1, 1));
        assert_eq!((g.slices[0].max_x, g.slices[0].max_y), (33, 33));
        // 末列收窄：x 从 65 到 71（宽 6）
        assert_eq!((g.slices[2].min_x, g.slices[2].max_x), (65, 71));
        // 末行收窄：y 从 33 到 41（高 8）
        assert_eq!((g.slices[3].min_y, g.slices[3].max_y), (33, 41));
    }

    #[test]
    fn grid_cover_exactly_matches_region_union() {
        let b = RegionBounds { min_x: 1, min_y: 1, max_x: 71, max_y: 41 };
        let g = SliceGrid::build(&[b], 32);
        let mut covered = vec![false; 80 * 50];
        for s in &g.slices {
            for y in s.min_y..s.max_y {
                for x in s.min_x..s.max_x {
                    assert!(!covered[y * 80 + x], "格 ({x},{y}) 被重复覆盖");
                    covered[y * 80 + x] = true;
                }
            }
        }
        for y in b.min_y..b.max_y {
            for x in b.min_x..b.max_x {
                assert!(covered[y * 80 + x], "格 ({x},{y}) 未被覆盖");
            }
        }
    }

    /// 设计规格 §3.2 要求覆盖「多区域」。简报未列此用例，补上：
    /// 切片不得跨区域，全局序号跨区域连续递增。
    #[test]
    fn grid_keeps_slices_within_their_own_region() {
        // 两个不相交区域：a=40x40，b=10x10（间隔 ≥2 格）
        let a = RegionBounds { min_x: 1, min_y: 1, max_x: 41, max_y: 41 };
        let b = RegionBounds { min_x: 10, min_y: 60, max_x: 20, max_y: 70 };
        let g = SliceGrid::build(&[a, b], 32);
        // a 切成 2x2=4 片，b 只有 1 片
        assert_eq!(g.slices.len(), 5, "40x40 为 2x2=4 片，10x10 为 1 片");
        for s in &g.slices {
            let r = if s.region_index == 0 { a } else { b };
            assert!(
                s.min_x >= r.min_x && s.max_x <= r.max_x && s.min_y >= r.min_y && s.max_y <= r.max_y,
                "切片 {s:?} 越出了区域 {r:?}"
            );
        }
        // 全局序号按区域序 → 行优先连续递增
        for (i, s) in g.slices.iter().enumerate() {
            assert_eq!(s.index, i, "全局序号应连续");
        }
        assert_eq!(g.slices[4].region_index, 1, "第 5 片应属于第 2 个区域");
    }

    /// 空区域（min >= max）不产生任何切片 —— 退化区域由循环自然跳过。
    #[test]
    fn grid_skips_empty_regions() {
        let empty = RegionBounds { min_x: 1, min_y: 1, max_x: 0, max_y: 0 };
        let ok = RegionBounds { min_x: 1, min_y: 1, max_x: 3, max_y: 3 };
        let g = SliceGrid::build(&[empty, ok], 32);
        assert_eq!(g.slices.len(), 1, "空区域应被跳过");
        assert_eq!(g.slices[0].region_index, 1, "唯一切片属于第 2 个区域");
        assert!(SliceGrid::build(&[empty], 32).slices.is_empty());
    }

    #[test]
    fn cache_rebuilds_only_when_signature_changes() {
        let b = RegionBounds { min_x: 1, min_y: 1, max_x: 71, max_y: 41 };
        let mut cache = SliceCache::default();
        cache.get(&[b], 32, 80, 50);
        let gen1 = cache.generation();
        assert_eq!(gen1, 1, "首次构建应记 1 代");
        // 同签名再取：不重建
        cache.get(&[b], 32, 80, 50);
        assert_eq!(cache.generation(), gen1, "签名未变不应重建");
        // 换区域：重建
        let b2 = RegionBounds { min_x: 1, min_y: 1, max_x: 91, max_y: 41 };
        cache.get(&[b2], 32, 80, 50);
        assert_eq!(cache.generation(), gen1 + 1, "区域变化应重建一次");
        // 换切片尺寸：也要重建（尺寸参与签名）
        cache.get(&[b2], 16, 80, 50);
        assert_eq!(cache.generation(), gen1 + 2, "切片尺寸变化应重建");
        // invalidate 后强制重建
        cache.invalidate();
        cache.get(&[b2], 16, 80, 50);
        assert_eq!(cache.generation(), gen1 + 3, "invalidate 后应重建");
    }

    #[test]
    fn cache_returns_grid_matching_latest_regions() {
        let b = RegionBounds { min_x: 1, min_y: 1, max_x: 71, max_y: 41 };
        let mut cache = SliceCache::default();
        let n1 = cache.get(&[b], 32, 80, 50).slices.len();
        let b2 = RegionBounds { min_x: 1, min_y: 1, max_x: 91, max_y: 41 };
        let n2 = cache.get(&[b2], 32, 80, 50).slices.len();
        assert_eq!((n1, n2), (6, 6), "70x40 与 90x40 按 32 切都是 3x2=6 片");
        let b3 = RegionBounds { min_x: 1, min_y: 1, max_x: 71, max_y: 51 };
        let n3 = cache.get(&[b3], 32, 80, 60).slices.len();
        assert_eq!(n3, 6, "70x50 按 32 切应为 3x2=6 片（y 两段 32+18）");
    }
}
