//! D4：32×32 切片并行（实验性）。
//!
//! feature "d4-slice" 门控；不带 feature 编译即 v1 稳定版。
//! 设计规格见 simDLL_RS/docs/superpowers/specs/2026-08-30-d4-slice-design.md。

use std::sync::atomic::{AtomicBool, Ordering};

pub mod slice_grid;
pub mod temperature;

/// 启用切片所需的最小物理核数。低于此值自动回退现有路径。
pub const MIN_SLICE_CORES: usize = 4;

/// 默认切片边长（格）。火箭舱内部世界尺寸同值。
pub const DEFAULT_SLICE_SIZE: usize = 32;

/// 运行时开关。默认 true（实验期），后续接口 mod 适配后改默认 false。
static SLICE_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_slice_enabled(on: bool) -> bool {
    SLICE_ENABLED.store(on, Ordering::Relaxed);
    on
}

pub fn slice_enabled() -> bool {
    SLICE_ENABLED.load(Ordering::Relaxed)
}

/// 是否启用切片路径：运行时开关打开 **且** 物理核数达到下限。
///
/// 两个条件缺一不可：核数不足时切片并行的同步开销盖过收益（原路径的串行温度
/// 本就走 ParallelTaskQueue 行带、同样在用多核）；开关则给 C# 实验性勾选项留逃生门。
/// `update_data` 的 D4 分支以本函数为准；不命中时走原路径，行为与改动前逐位一致。
pub fn should_slice(physical_cores: usize) -> bool {
    slice_enabled() && physical_cores >= MIN_SLICE_CORES
}

/// C# 侧运行时开关（对应后续接口 mod 的实验性勾选项）。
#[no_mangle]
pub extern "C" fn RS_SetSliceEnabled(enabled: bool) -> i32 {
    set_slice_enabled(enabled);
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;

    /// 计划任务 7 步骤 1 的开关语义测试：默认开、低核回退、开关关闭回退。
    #[test]
    fn disabled_falls_back_and_enabled_uses_slicing() {
        let _lock = crate::LIB_TESTS_LOCK.lock().unwrap();
        assert!(slice_enabled(), "默认应为 true");
        assert!(!should_slice(2), "2 核 < MIN_SLICE_CORES 应回退");
        assert!(should_slice(4), "4 核应满足");
        set_slice_enabled(false);
        assert!(!should_slice(8), "开关关闭应回退");
        set_slice_enabled(true);
        assert!(should_slice(8));
    }
}
