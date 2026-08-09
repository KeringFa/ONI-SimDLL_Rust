//! T6.3：LPT 面积装箱——按实际面积从大到小放入当前最轻组，不拆世界。
//!
//! 输入区域边界（含 cell_count），输出区域索引分组（Vec<Vec<usize>>）。
//! 组数 ≤ min(区域数, group_count)；每组累计面积尽量接近理想值
//! （总面积/组数），且任一单组不超 max(理想, 最大单世界)。
use crate::d1_activity::RegionBounds;

/// LPT 贪心装箱：返回区域索引分组。
pub fn bin_pack_region_groups(
    bounds: &[RegionBounds],
    group_count: usize,
) -> Vec<Vec<usize>> {
    let n = bounds.len();
    if n == 0 {
        return Vec::new();
    }
    let groups = group_count.min(n);
    // 按面积降序（LPT：先放大任务）
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by(|&a, &b| {
        bounds[b]
            .cell_count()
            .cmp(&bounds[a].cell_count())
            .then(a.cmp(&b))
    });
    let mut loads = vec![0u64; groups];
    let mut out: Vec<Vec<usize>> = (0..groups).map(|_| Vec::new()).collect();
    for &i in &order {
        let g = loads
            .iter()
            .enumerate()
            .min_by_key(|(_, &l)| l)
            .map(|(gi, _)| gi)
            .expect("groups >= 1");
        loads[g] += bounds[i].cell_count();
        out[g].push(i);
    }
    out
}

/// 各组累计面积（测试/诊断用）。
pub fn group_loads(bounds: &[RegionBounds], groups: &[Vec<usize>]) -> Vec<u64> {
    groups
        .iter()
        .map(|g| g.iter().map(|&i| bounds[i].cell_count()).sum())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d1_activity::RegionBounds;

    fn b(x: usize, y: usize) -> RegionBounds {
        RegionBounds {
            min_x: 1,
            min_y: 1,
            max_x: x,
            max_y: y,
        }
    }

    #[test]
    fn empty_input_yields_no_groups() {
        assert!(bin_pack_region_groups(&[], 4).is_empty());
    }

    #[test]
    fn single_region_single_group() {
        let bounds = [b(128, 153)];
        let groups = bin_pack_region_groups(&bounds, 7);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0], vec![0]);
    }

    #[test]
    fn groups_never_exceed_region_count_or_group_count() {
        // 11 区域、7 组 → 组数 = 7
        let bounds = vec![b(1, 2); 11];
        let groups = bin_pack_region_groups(&bounds, 7);
        assert_eq!(groups.len(), 7);
        // 组数 > 区域数 → 组数 = 区域数
        let groups2 = bin_pack_region_groups(&bounds, 20);
        assert_eq!(groups2.len(), 11);
    }

    #[test]
    fn lpt_balances_02_layout_within_25_percent() {
        // 02_test_02 实测布局（内部格面积近似）
        let bounds = vec![
            b(128, 153), // 0 中型
            b(128, 153), // 1
            b(128, 153), // 2
            b(128, 153), // 3
            b(128, 153), // 4
            b(64, 128),  // 5
            b(64, 96),   // 6
            b(96, 80),   // 7
            b(80, 174),  // 8 WaterMoonlet
            b(64, 96),   // 9
            b(160, 96),  // 10
        ];
        let groups = bin_pack_region_groups(&bounds, 8);
        let loads: Vec<u64> = groups
            .iter()
            .map(|g| g.iter().map(|&i| bounds[i].cell_count()).sum())
            .collect();
        let total: u64 = loads.iter().sum();
        let ideal = total / 8;
        let max_load = *loads.iter().max().unwrap();
        // 最重组含最大单世界（9,584），允许 25% 超理想
        assert!(
            max_load <= ideal + ideal / 4,
            "makespan {max_load} 超理想 {ideal} 的 25%"
        );
        // 分组覆盖所有区域且不重复
        let mut seen = vec![false; bounds.len()];
        for g in &groups {
            for &i in g {
                assert!(!seen[i], "区域 {i} 重复入组");
                seen[i] = true;
            }
        }
        assert!(seen.iter().all(|&v| v), "所有区域都应入组");
    }
}
