//! 方向 1（1 星 1 线程）：CPU 拓扑枚举、超线程检测与分组分配。
//!
//! 对照原理：Windows GetLogicalProcessorInformation（RelationProcessorCore）
//! 返回每个物理核的逻辑处理器数（1=无超线程，2=启用超线程）。分组算法按
//! Remix 后世界 tiles 降序：前（物理核−1）大世界独占物理核，其余最小世界
//! 两两合并 + 太空员舱集群走超线程逻辑核。

/// CPU 拓扑（GetLogicalProcessorInformation，RelationProcessorCore）。
pub struct CpuTopology {
    pub physical_cores: usize,
    pub logical_cores: usize,
    /// 每个物理核的逻辑处理器数（≥1；>1 表示启用超线程）。
    pub logical_per_core: Vec<usize>,
}

impl CpuTopology {
    pub fn query() -> Self {
        use windows::Win32::System::SystemInformation::{
            GetLogicalProcessorInformation, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
        };

        // 回退：枚举失败/异常时按 available_parallelism 保守处理（视为无 HT）。
        let fallback = || {
            let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
            CpuTopology {
                physical_cores: logical,
                logical_cores: logical,
                logical_per_core: vec![1; logical],
            }
        };

        unsafe {
            let mut needed: u32 = 0;
            // 第一次调用只取所需缓冲大小（预期 ERROR_INSUFFICIENT_BUFFER）。
            if GetLogicalProcessorInformation(None, &mut needed).is_err() || needed == 0 {
                return fallback();
            }
            let count =
                (needed as usize) / std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
            if count == 0 {
                return fallback();
            }
            let mut buf = vec![SYSTEM_LOGICAL_PROCESSOR_INFORMATION::default(); count];
            if GetLogicalProcessorInformation(Some(buf.as_mut_ptr()), &mut needed).is_err() {
                return fallback();
            }

            let mut physical = 0usize;
            let mut logical_total = 0usize;
            let mut per_core = Vec::new();
            for info in buf.iter() {
                if info.Relationship == RelationProcessorCore {
                    // ProcessorMask 的置位位数 = 该物理核的逻辑处理器数。
                    let n = info.ProcessorMask.count_ones() as usize;
                    physical += 1;
                    logical_total += n;
                    per_core.push(n);
                }
            }
            if physical == 0 {
                return fallback();
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
}
