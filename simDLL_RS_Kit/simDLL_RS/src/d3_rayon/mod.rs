//! D3: rayon 分区并行（多星/多活动区域）。
//!
//! feature "d3-rayon" 门控：默认关闭 → 编译结果即 v1 稳定版（本目录不参与编译，
//! 物理移走本文件夹 + 关闭 feature 也能编译）。设计与实现依据
//! `docs/d3_rayon_技术参考.md`（提炼自桌面版旧 d3 线，不照抄旧代码）。
use std::sync::OnceLock;

pub mod affinity;
pub mod bin_pack;
pub mod d1_pipeline;
pub mod perf_probe;
pub mod pipeline;
pub mod scheduler;

/// 预留给游戏主线程/系统的物理核数（默认 1，接口 mod 可配置 1..=4）。
/// 由 `RS_SetReservedCores` 设置（OnLoad 时，池构建之前）；D1/D2 两条并行
/// 路径统一读取。曾为写死常量（2026-08-08 用户拍板 = 1），2026-08-21 改为可配置。
pub fn reserve_cores() -> usize {
    crate::d3_rayon::scheduler::reserved_cores()
}

/// 物理核数：统一走 Ex API 拓扑（affinity::CpuTopology）。
/// 注：旧 API GetLogicalProcessorInformation 在本机返回全 Group → fallback 逻辑核
/// （16），导致 rayon 池 15 线程超额订阅——已弃用。
pub fn physical_cores() -> usize {
    crate::d3_rayon::affinity::CpuTopology::cached().physical_cores
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
        // 物理核数用 Ex API 拓扑（旧 API 在本机返回全 Group → fallback 逻辑核 16，
        // 导致线程池 15 线程超额订阅）。复用 affinity::CpuTopology。
        let reserve = reserve_cores();
        let threads = crate::d3_rayon::affinity::CpuTopology::cached()
            .physical_cores
            .saturating_sub(reserve)
            .max(1);
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
        let _lock = crate::LIB_TESTS_LOCK.lock();
        // 固定默认预留数，避免与可配置测试并行交错影响池线程数
        assert_eq!(crate::d3_rayon::scheduler::RS_SetReservedCores(1), 1);
        let p = pool();
        assert_eq!(
            p.current_num_threads(),
            physical_cores().saturating_sub(reserve_cores()).max(1)
        );
    }

    #[test]
    fn pool_builds_once_and_reuses() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
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
