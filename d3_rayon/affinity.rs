//! 方向 1（1 星 1 线程）：CPU 拓扑枚举、超线程检测与分组分配。
//!
//! 对照原理：Windows GetLogicalProcessorInformationEx（RelationProcessorCore）
//! 返回每个物理核的逻辑处理器数与 HT 标志。分组算法按 Remix 后世界 tiles
//! 降序：前（物理核−1）大世界独占物理核，其余最小世界两两合并 +
//! 太空员舱集群走超线程逻辑核（亲和性当前未实际绑定，OS 自由调度）。

use std::sync::OnceLock;

/// CPU 拓扑（GetLogicalProcessorInformation，RelationProcessorCore）。
pub struct CpuTopology {
    pub physical_cores: usize,
    pub logical_cores: usize,
    /// 每个物理核的逻辑处理器数（≥1；>1 表示启用超线程）。
    pub logical_per_core: Vec<usize>,
}

/// 拓扑缓存（进程内只查询一次；失败回退已由 query 内部处理）。
static G_TOPOLOGY: OnceLock<CpuTopology> = OnceLock::new();

impl CpuTopology {
    pub fn cached() -> &'static CpuTopology {
        G_TOPOLOGY.get_or_init(CpuTopology::query)
    }
}

/// 一个调度组：region 索引 + 亲和掩码。
#[derive(Clone, PartialEq, Eq)]
pub struct AffinityGroup {
    pub region_indices: Vec<usize>,
    /// Windows 亲和掩码（位 = 逻辑处理器）。
    pub affinity_mask: u64,
}

/// 输入：普通世界 tiles（不含太空员舱集群，集群由 D1 执行器单独合并）、拓扑、
/// 预留物理核数（留给游戏主线程/系统，默认 1，可配置 1..=4）。
/// 输出：物理核组（前 physical_cores−reserve 大，各独占一物理核）+ 超线程组
/// （其余两两合并）。无 HT 时返回 None（调用方回退 D2）。
pub fn assign_groups(
    world_tiles: &[u64],
    topo: &CpuTopology,
    reserve: usize,
) -> Option<Vec<AffinityGroup>> {
    if !topo.has_hyper_threading() {
        return None;
    }
    if world_tiles.is_empty() {
        return Some(Vec::new());
    }
    // 至少保留 1 个物理核给模拟，避免极端配置（如 4 核机 reserve=4）饿死并行。
    let p = topo.physical_cores.saturating_sub(reserve).max(1);
    if p == 0 {
        return None;
    }

    // 每个物理核的起始逻辑核全局位索引（前缀和）。
    let mut core_start = Vec::with_capacity(topo.physical_cores);
    let mut acc = 0u64;
    for &n in topo.logical_per_core.iter().take(topo.physical_cores) {
        core_start.push(acc);
        acc += n as u64;
    }

    // 普通世界按 tiles 降序。
    let mut order: Vec<usize> = (0..world_tiles.len()).collect();
    order.sort_unstable_by(|&a, &b| world_tiles[b].cmp(&world_tiles[a]));

    let mut groups = Vec::new();
    // 物理核组：前 p 大世界各独占一个物理核（用该核的第 1 个逻辑核）。
    for (i, &idx) in order.iter().take(p).enumerate() {
        groups.push(AffinityGroup {
            region_indices: vec![idx],
            affinity_mask: 1u64 << core_start[i],
        });
    }

    // 超线程组：剩余世界按 tiles 升序两两合并（最小两个先合并）。
    let mut rest: Vec<usize> = order.iter().skip(p).copied().collect();
    rest.sort_unstable_by(|&a, &b| world_tiles[a].cmp(&world_tiles[b]));
    let mut ht_groups: Vec<Vec<usize>> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if i + 1 < rest.len() {
            ht_groups.push(vec![rest[i], rest[i + 1]]);
            i += 2;
        } else {
            ht_groups.push(vec![rest[i]]);
            i += 1;
        }
    }

    // 超线程组亲和性：优先复用"最小物理核组"（索引 p−1，负载最低大世界）的
    // 空闲逻辑核（start+1），组数超出时再轮流回退到次小物理核组的空闲逻辑核。
    for (j, g) in ht_groups.iter().enumerate() {
        let core_idx = (p - 1 - (j % p)) % p;
        let start = core_start[core_idx];
        let idle = start + 1; // has_ht 保证每核 ≥ 2 逻辑核
        groups.push(AffinityGroup {
            region_indices: g.clone(),
            affinity_mask: 1u64 << idle,
        });
    }

    Some(groups)
}

impl CpuTopology {
    /// 解析异常时的近似拓扑：逻辑核/2 作为物理核数（默认每核 2 逻辑处理器）。
    /// 仅用于 `query()` 的加固判定（physical≤1 且 logical≥4）。
    pub fn approx_topology_for_broken_detection(logical: usize) -> CpuTopology {
        let physical = (logical / 2).max(1);
        CpuTopology {
            physical_cores: physical,
            logical_cores: logical,
            logical_per_core: vec![2; physical],
        }
    }

    pub fn query() -> Self {
        let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let fallback = || {
            CpuTopology {
                physical_cores: logical,
                logical_cores: logical,
                logical_per_core: vec![1; logical],
            }
        };

        // 用 GetLogicalProcessorInformationEx（RelationProcessorCore）+ 手写偏移读取，
        // 与 C# CpuInfo 一致——旧 API GetLogicalProcessorInformation 在本机
        // （Win11/部分环境）返回全 Group（无 ProcessorCore 条目），检测恒 false。
        unsafe {
            let mut len: u32 = 0;
            // 第一次调用查询所需缓冲区大小（预期失败，len 被设置）。
            let _ = windows::Win32::System::SystemInformation::GetLogicalProcessorInformationEx(
                windows::Win32::System::SystemInformation::RelationProcessorCore,
                None,
                &mut len,
            );
            if len == 0 {
                return fallback();
            }
            let mut buf = vec![0u8; len as usize];
            if windows::Win32::System::SystemInformation::GetLogicalProcessorInformationEx(
                windows::Win32::System::SystemInformation::RelationProcessorCore,
                Some(buf.as_mut_ptr() as *mut _),
                &mut len,
            )
            .is_err()
            {
                return fallback();
            }
            let mut physical = 0usize;
            let mut logical_total = 0usize;
            let mut per_core = Vec::new();
            let mut offset = 0usize;
            while offset + 8 <= buf.len() {
                // Windows ABI：条目 Relationship@0、Size@4；
                // RelationProcessorCore 的 Flags@8、GROUP_AFFINITY.Mask@32。
                let relationship = u32::from_le_bytes(
                    buf[offset..offset + 4].try_into().unwrap(),
                );
                let size =
                    u32::from_le_bytes(buf[offset + 4..offset + 8].try_into().unwrap());
                if relationship == 0 {
                    let mask = u64::from_le_bytes(
                        buf[offset + 32..offset + 40].try_into().unwrap(),
                    );
                    let n = mask.count_ones() as usize;
                    physical += 1;
                    logical_total += n;
                    per_core.push(n);
                }
                if size == 0 {
                    break;
                }
                offset += size as usize;
            }
            if physical == 0 {
                return fallback();
            }
            // 拓扑加固（2026-08-21，玩家反馈启发）：Ex API 解析"成功"但物理核数
            // 异常少（≤1）而系统逻辑核 ≥4 时，几乎必然是解析失败/异常环境——按
            // 逻辑核/2 近似物理核（默认每核 2 逻辑处理器），避免把 CPU 当成 1 核
            // 而走向错误调度路径。真 1~2 核机器的可用逻辑核 <4，不会误触发。
            if physical <= 1 && logical >= 4 {
                return Self::approx_topology_for_broken_detection(logical);
            }
            CpuTopology {
                physical_cores: physical,
                logical_cores: logical_total,
                logical_per_core: per_core,
            }
        }
    }

    pub fn has_hyper_threading(&self) -> bool {
        self.logical_per_core.iter().any(|&n| n > 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_sane_on_this_machine() {
        let t = CpuTopology::query();
        assert!(t.physical_cores >= 1, "物理核 ≥ 1");
        assert!(t.logical_cores >= t.physical_cores, "逻辑核 ≥ 物理核");
        // 无 HT 的机器上 has_hyper_threading()==false 也合法；有 HT 时每核逻辑核数 ≥ 2
        if t.has_hyper_threading() {
            assert!(t.logical_cores >= t.physical_cores * 2);
        }
    }

    #[test]
    fn assign_groups_big_worlds_never_share_physical_core() {
        let topo = CpuTopology {
            physical_cores: 8,
            logical_cores: 16,
            logical_per_core: vec![2; 8],
        };
        // 11 世界：5 卫星(6144) + 6 遥远星球（Regolith 15360 / Water 13920 /
        // MiniRegolith 9216 / Tundra 8192 / Moo 7680 / Wetland 6144）
        let mut tiles: Vec<u64> = vec![6144; 5];
        tiles.extend([15360, 13920, 9216, 8192, 7680, 6144]);
        let groups = assign_groups(&tiles, &topo, 1).expect("有 HT 应可分配");

        // 物理核组 = 7（8−1），每组恰好 1 个 region（最大 7 世界独占）
        let physical: Vec<_> = groups.iter().filter(|g| g.region_indices.len() == 1).collect();
        assert_eq!(physical.len(), 7, "前 7 大世界独占物理核");
        // 大世界组互不共享物理核：mask 两两交集为空
        for (i, a) in physical.iter().enumerate() {
            for b in physical.iter().skip(i + 1) {
                assert_eq!(a.affinity_mask & b.affinity_mask, 0, "大世界不得共享物理核");
            }
        }
        // 超线程组：其余最小世界两两合并
        let ht: Vec<_> = groups.iter().filter(|g| g.region_indices.len() > 1).collect();
        assert!(ht.len() <= 8, "超线程组数 {0} 应在空闲逻辑核内", ht.len());
        // 两两合并后每组 tiles 总和 ≤ 19584（变体后上限）
        for g in &ht {
            let sum: u64 = g.region_indices.iter().map(|&i| tiles[i]).sum();
            assert!(sum <= 19584, "合并组 tiles {sum} 超上限");
        }
        // 所有组覆盖全部 region
        let mut all: Vec<usize> = groups
            .iter()
            .flat_map(|g| g.region_indices.iter().copied())
            .collect();
        all.sort_unstable();
        let expect: Vec<usize> = (0..tiles.len()).collect();
        assert_eq!(all, expect, "分组应覆盖全部世界");
    }

    /// 回归（2026-08-21）：预留物理核数可配置（1..=4）——reserve 增加时物理核组
    /// 减少、超线程组对应轮转；覆盖全部 region 且大世界组互不共享物理核。
    #[test]
    fn assign_groups_respects_configurable_reserve() {
        let topo = CpuTopology {
            physical_cores: 8,
            logical_cores: 16,
            logical_per_core: vec![2; 8],
        };
        let mut tiles: Vec<u64> = vec![6144; 5];
        tiles.extend([15360, 13920, 9216, 8192, 7680, 6144]);

        for reserve in 1..=4usize {
            let groups = assign_groups(&tiles, &topo, reserve).expect("有 HT 应可分配");
            let expect_p = 8usize.saturating_sub(reserve).max(1);
            // assign_groups 契约：前 expect_p 组是物理核组（各独占一个物理核，
            // 每组恰好 1 个 region）；剩余是超线程组（两两合并，可能含单组——
            // 不能用 region_indices.len()==1 识别物理核组）。
            assert!(
                groups.len() >= expect_p,
                "reserve={reserve} 组数应 ≥ {expect_p}，got {}",
                groups.len()
            );
            let physical = &groups[..expect_p];
            assert!(
                physical.iter().all(|g| g.region_indices.len() == 1),
                "reserve={reserve} 物理核组应每组 1 个 region"
            );
            // 物理核组互不共享物理核
            for (i, a) in physical.iter().enumerate() {
                for b in physical.iter().skip(i + 1) {
                    assert_eq!(a.affinity_mask & b.affinity_mask, 0);
                }
            }
            // 覆盖全部 region
            let mut all: Vec<usize> = groups
                .iter()
                .flat_map(|g| g.region_indices.iter().copied())
                .collect();
            all.sort_unstable();
            let expect: Vec<usize> = (0..tiles.len()).collect();
            assert_eq!(all, expect, "reserve={reserve} 分组应覆盖全部世界");
        }
    }

    /// 回归（2026-08-21）：解析异常（physical≤1 且逻辑核≥4）时按逻辑核/2 近似
    /// 物理核，避免调度路径把 CPU 当成 1 核。
    #[test]
    fn broken_topology_fallback_approximates_physical_cores() {
        // 7800X3D 场景：解析失败到 physical=1，但系统有 16 逻辑核 → 近似 8 物理核
        let t = CpuTopology::approx_topology_for_broken_detection(16);
        assert_eq!(t.physical_cores, 8);
        assert_eq!(t.logical_cores, 16);
        assert_eq!(t.logical_per_core, vec![2; 8]);
        assert!(t.has_hyper_threading(), "近似拓扑应视为启用超线程");

        // 奇数逻辑核：向下取整，仍 ≥1
        let t2 = CpuTopology::approx_topology_for_broken_detection(6);
        assert_eq!(t2.physical_cores, 3);
        assert_eq!(t2.logical_cores, 6);

        // 边界：逻辑核 <4 不应走加固（真实 1~2 核机器），此处验证近似下限仍安全
        let t3 = CpuTopology::approx_topology_for_broken_detection(1);
        assert_eq!(t3.physical_cores, 1);
        assert_eq!(t3.logical_per_core, vec![2; 1]);
    }
}
