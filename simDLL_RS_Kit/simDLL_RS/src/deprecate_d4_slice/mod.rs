//! D4：32×32 切片并行（实验性）。
//!
//! feature "d4-slice" 门控；不带 feature 编译即 v1 稳定版。
//! 设计规格见 simDLL_RS/docs/superpowers/specs/2026-08-30-d4-slice-design.md。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub mod slice_grid;
pub mod temperature;

/// 启用切片所需的最小物理核数。低于此值自动回退现有路径。
pub const MIN_SLICE_CORES: usize = 4;

/// 默认切片边长（格）。火箭舱内部世界尺寸同值。
pub const DEFAULT_SLICE_SIZE: usize = 32;

/// 切片边长允许范围（接口 mod 可配置区间；8..=64 兼顾并行度与任务粒度）。
pub const MIN_SLICE_SIZE: usize = 8;
pub const MAX_SLICE_SIZE: usize = 64;

/// 运行时开关。默认 true（实验期），后续接口 mod 适配后改默认 false。
static SLICE_ENABLED: AtomicBool = AtomicBool::new(true);

/// 运行时切片边长（默认 32，接口 mod 可配置）。用 `AtomicUsize`（非原子 `static mut`），
/// 与 `SliceEventPool` 的签名比较配合：尺寸变化会自动触发缓冲池重建。
static SLICE_SIZE: AtomicUsize = AtomicUsize::new(DEFAULT_SLICE_SIZE);

pub fn set_slice_enabled(on: bool) -> bool {
    SLICE_ENABLED.store(on, Ordering::Relaxed);
    on
}

pub fn slice_enabled() -> bool {
    SLICE_ENABLED.load(Ordering::Relaxed)
}

/// 当前生效的切片边长（clamp 到 `MIN_SLICE_SIZE..=MAX_SLICE_SIZE`）。
pub fn current_slice_size() -> usize {
    SLICE_SIZE.load(Ordering::Relaxed).clamp(MIN_SLICE_SIZE, MAX_SLICE_SIZE)
}

/// 设置切片边长（`MIN_SLICE_SIZE..=MAX_SLICE_SIZE`）。返回实际生效值；非法输入返回 -1。
/// 供接口 mod 的「切片尺寸」下拉选项调用（实验性，玩家可调）。
#[no_mangle]
pub extern "C" fn RS_SetSliceSize(size: i32) -> i32 {
    if !(MIN_SLICE_SIZE as i32..=MAX_SLICE_SIZE as i32).contains(&size) {
        return -1;
    }
    SLICE_SIZE.store(size as usize, Ordering::Relaxed);
    size
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

    /// 切片尺寸可配置：默认 32；合法值生效；越界返回 -1 且保持原值。
    /// 变异对照：若 `current_slice_size` 忘了 clamp，设回 7 后仍会读到 7 → 必红。
    #[test]
    fn slice_size_settable_and_clamped() {
        let _lock = crate::LIB_TESTS_LOCK.lock().unwrap();
        assert_eq!(current_slice_size(), DEFAULT_SLICE_SIZE, "默认应为 32");
        assert_eq!(RS_SetSliceSize(16), 16);
        assert_eq!(current_slice_size(), 16);
        assert_eq!(RS_SetSliceSize(64), 64);
        assert_eq!(current_slice_size(), 64);
        // 越界：拒绝并保持原值
        assert_eq!(RS_SetSliceSize(7), -1, "低于 MIN_SLICE_SIZE 应拒绝");
        assert_eq!(RS_SetSliceSize(65), -1, "高于 MAX_SLICE_SIZE 应拒绝");
        assert_eq!(current_slice_size(), 64, "非法输入不应改变当前值");
        // 恢复默认
        assert_eq!(RS_SetSliceSize(DEFAULT_SLICE_SIZE as i32), DEFAULT_SLICE_SIZE as i32);
        assert_eq!(current_slice_size(), DEFAULT_SLICE_SIZE);
    }
}
