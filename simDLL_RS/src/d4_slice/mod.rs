//! D4：32×32 切片并行（实验性）。
//!
//! feature "d4-slice" 门控；不带 feature 编译即 v1 稳定版。
//! 设计规格见 simDLL_RS/docs/superpowers/specs/2026-08-30-d4-slice-design.md。

// ⚠️ 本任务**不要**在这里写 `pub mod slice_grid;` / `pub mod temperature;`。
// 这两个文件分别由任务 3 / 任务 4 创建，届时由它们各自在本文件补一行声明
// （脚手架折进需要它的那个任务，避免提交空占位文件）。

use std::sync::atomic::{AtomicBool, Ordering};

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
