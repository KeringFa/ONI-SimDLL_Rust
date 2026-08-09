//! 运行时调度器模式（Auto/D2/D1）。
//!
//! Auto（默认）：多区域 → D2（rayon 装箱），单区域 → 串行（与原版一致）；
//! D2：强制方向 2；D1：方向 1（1 星 1 线程 + 亲和性），无超线程拓扑回退 D2。
//! 模式由接口 mod 经 `RS_SetSchedulerMode` 设置（OnLoad 时，SIM_Initialize 之前）。

use std::sync::atomic::{AtomicI32, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SchedulerMode {
    Auto = 0,
    D2 = 1,
    D1 = 2,
}

static MODE: AtomicI32 = AtomicI32::new(0);

pub fn current_mode() -> SchedulerMode {
    match MODE.load(Ordering::Relaxed) {
        1 => SchedulerMode::D2,
        2 => SchedulerMode::D1,
        _ => SchedulerMode::Auto,
    }
}

/// 设置调度模式：0=Auto，1=D2，2=D1。返回实际生效模式：
/// D1 在无超线程拓扑下回退 D2（返回 1）；非法输入返回 -1。
#[no_mangle]
pub extern "C" fn RS_SetSchedulerMode(mode: i32) -> i32 {
    match mode {
        0 => {
            MODE.store(0, Ordering::Relaxed);
            0
        }
        1 => {
            MODE.store(1, Ordering::Relaxed);
            1
        }
        2 => {
            let topo = crate::d3_rayon::affinity::CpuTopology::cached();
            if topo.has_hyper_threading() {
                MODE.store(2, Ordering::Relaxed);
                2
            } else {
                MODE.store(1, Ordering::Relaxed);
                tracing::warn!("D1 requested but CPU has no hyper-threading; falling back to D2");
                1
            }
        }
        _ => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    pub fn reset_for_tests() {
        MODE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn scheduler_mode_invalid_rejected() {
        reset_for_tests();
        assert_eq!(RS_SetSchedulerMode(99), -1);
        assert_eq!(current_mode(), SchedulerMode::Auto);
    }

    #[test]
    fn scheduler_mode_set_and_query() {
        reset_for_tests();
        assert_eq!(RS_SetSchedulerMode(1), 1);
        assert_eq!(current_mode(), SchedulerMode::D2);
        assert_eq!(RS_SetSchedulerMode(0), 0);
        assert_eq!(current_mode(), SchedulerMode::Auto);
    }

    #[test]
    fn scheduler_d1_accepts_or_falls_back() {
        reset_for_tests();
        let r = RS_SetSchedulerMode(2);
        // 有 HT → 2；无 HT → 回退 1。两者均合法。
        assert!(r == 1 || r == 2, "D1 应接受(2)或回退 D2(1)，got {r}");
    }
}
