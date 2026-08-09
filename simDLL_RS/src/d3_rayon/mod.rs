//! D3: rayon 分区并行（多星/多活动区域）。
//!
//! feature "d3-rayon" 门控：默认关闭 → 编译结果即 v1 稳定版（本目录不参与编译，
//! 物理移走本文件夹 + 关闭 feature 也能编译）。设计与实现依据
//! `docs/d3_rayon_技术参考.md`（提炼自桌面版旧 d3 线，不照抄旧代码）。
use std::sync::OnceLock;

/// 预留给游戏主线程/系统的物理核数（2026-08-08 用户拍板 = 1）。
pub const RESERVE_CORES: usize = 1;

/// 物理核数：Windows `GetLogicalProcessorInformation` 按 RelationProcessorCore 统计
/// （8C16T → 8、4C8T → 4）；API 失败（老系统/权限）回退 `available_parallelism`。
pub fn physical_cores() -> usize {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformation, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
    };
    unsafe {
        let mut needed: u32 = 0;
        // 第一次调用只取所需缓冲区大小（预期 ERROR_INSUFFICIENT_BUFFER）
        if GetLogicalProcessorInformation(None, &mut needed).is_err() || needed == 0 {
            return fallback_parallelism();
        }
        let count = (needed as usize) / std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
        if count == 0 {
            return fallback_parallelism();
        }
        let mut buf = vec![SYSTEM_LOGICAL_PROCESSOR_INFORMATION::default(); count];
        if GetLogicalProcessorInformation(Some(buf.as_mut_ptr()), &mut needed).is_err() {
            return fallback_parallelism();
        }
        let cores = buf
            .iter()
            .filter(|info| info.Relationship == RelationProcessorCore)
            .count();
        if cores == 0 {
            fallback_parallelism()
        } else {
            cores
        }
    }
}

fn fallback_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// D1 门控：仅当活动区域数 > 1（多星/多已发现世界）时启用并行。
/// 单星/单区域永远走原版串行路径（行为零变化）。纯调度决策，无存档标志位。
pub fn should_parallelize(region_count: usize) -> bool {
    region_count > 1
}

/// D6 线程池：懒构建一次并缓存。线程数 = max(1, 物理核 − reserve)。
/// rayon scope 提交的 job 数 = 区域组数（LPT 装箱），组数 > 线程数时任务排队
/// （rayon 工作窃取，无超额订阅、无嵌套池争抢）。单区域路径不触碰本池。
static G_RAYON_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

pub fn pool() -> &'static rayon::ThreadPool {
    G_RAYON_POOL.get_or_init(|| {
        let threads = physical_cores().saturating_sub(RESERVE_CORES).max(1);
        rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("simdll-d3-{i}"))
            .num_threads(threads)
            .build()
            .expect("d3: rayon ThreadPool build failed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_single_region_stays_serial() {
        assert!(!should_parallelize(0));
        assert!(!should_parallelize(1));
    }

    #[test]
    fn gate_multi_region_enables_parallel() {
        assert!(should_parallelize(2));
        assert!(should_parallelize(11));
    }

    #[test]
    fn pool_size_is_physical_minus_reserve() {
        let p = pool();
        assert_eq!(
            p.current_num_threads(),
            physical_cores().saturating_sub(RESERVE_CORES).max(1)
        );
    }

    #[test]
    fn pool_builds_once_and_reuses() {
        let p1 = pool();
        let p2 = pool();
        assert!(std::ptr::eq(p1, p2), "线程池应只构建一次并复用");
    }

    #[test]
    fn physical_cores_sane() {
        let cores = physical_cores();
        assert!(cores >= 1, "物理核数应 ≥ 1");
        if let Ok(logical) = std::thread::available_parallelism() {
            assert!(
                cores <= logical.get(),
                "物理核 {cores} 不应超过逻辑核 {}",
                logical.get()
            );
        }
    }
}
