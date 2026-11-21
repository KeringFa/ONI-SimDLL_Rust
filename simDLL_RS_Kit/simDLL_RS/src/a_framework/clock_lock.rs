//! 时钟锁 —— A/B 对拍专用（不参与正常游戏逻辑）
//!
//! 背景：原版 `SimData` 的随机种子来自 `_time64(NULL)`（反编译 L3869），
//! Rust 侧对应 `SystemTime::now()`。两边各自取"当前秒数"，因此**同一个存档
//! 每次载入都会得到不同的随机序列** —— 这是 ONI 本身的特性，不是缺陷。
//!
//! 后果：A/B 逐格对拍时，只要模拟里跑过一次 `next_random`（液体/气体位移的
//! 随机门，见 liquid_flow.rs），两边的结果必然分叉。实测"原版 vs 原版"两次
//! 载入，第 2 帧就有 30 格元素差异、1924 格温度差异 —— 与"原版 vs Rust"同量级。
//! 也就是说：不做种子对齐，对拍测到的全是噪声。
//!
//! 做法：把进程内 `ucrtbase!_time64` 的入口改写成绝对跳转，返回
//! `base + 调用序号`。这样：
//!   - 原版与 Rust 两轮拿到**同一个种子**，随机序列可比；
//!   - 时间仍是单调递增的，其它依赖它算时间差的代码不会异常；
//!   - 只在显式调用 `RS_InstallClockLock` 时装（由 ABRecorder 触发），
//!     正常游玩永远不走这条路。

use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

static CLOCK_BASE: AtomicI64 = AtomicI64::new(0);
static CLOCK_COUNTER: AtomicI64 = AtomicI64::new(0);
static CLOCK_INSTALLED: AtomicU32 = AtomicU32::new(0);

/// 供本 DLL 内部取种子用：已装时钟锁 → 返回锁定值；未装 → None（调用方回落真实时间）。
/// 必须走这个而不是 `SystemTime::now()`：后者直接调 Windows API，
/// 不经过被 patch 的 `_time64`，两边口径会不一致。
pub fn locked_seed() -> Option<u32> {
    if CLOCK_INSTALLED.load(Ordering::Acquire) != 0 {
        Some(CLOCK_BASE.load(Ordering::Relaxed) as u32)
    } else {
        None
    }
}

/// 安装时钟锁。`base` 是种子基准（两轮必须相同）。
/// 返回 1 = 安装成功，0 = 已安装，负值 = 失败（找不到 `_time64` 或写保护失败）。
#[no_mangle]
pub extern "C" fn RS_InstallClockLock(base: u32) -> i32 {
    if CLOCK_INSTALLED.load(Ordering::Acquire) != 0 {
        return 0;
    }
    let r = imp::install(base as i64);
    if r > 0 {
        CLOCK_INSTALLED.store(1, Ordering::Release);
    }
    r
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleA;
    use windows::Win32::System::Memory::{
        VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE,
    };

    const EPOCH_DIFF_SECS: i64 = 11_644_473_600; // 1601-01-01 → 1970-01-01

    /// 替换后的 `GetSystemTimeAsFileTime`：恒定返回锁定的时间点。
    /// 只对 SimDLL.dll 的 IAT 生效，进程内其它模块不受影响。
    unsafe extern "system" fn lock_get_system_time(lp: *mut u64) -> i32 {
        if !lp.is_null() {
            let secs = CLOCK_BASE.load(Ordering::Relaxed);
            *lp = ((secs + EPOCH_DIFF_SECS) * 10_000_000) as u64;
        }
        1
    }

    /// 把 `module` 导入表里 `func` 的 IAT 槽改成 `replacement`。
    /// 返回 true 表示替换成功。只在 x64 PE 上工作（本项目只出 x64）。
    unsafe fn patch_iat(module: isize, func: &[u8], replacement: *const ()) -> bool {
        let base = module as usize;
        let e_lfanew = *(base as *const u32).add(0x3c / 4) as usize;
        let nt = base + e_lfanew;
        if *(nt as *const u32) != 0x0000_4550 {
            return false; // "PE\0\0"
        }
        let opt = nt + 24;
        let magic = *(opt as *const u16);
        if magic != 0x20b {
            return false; // 仅 PE32+
        }
        let dd = opt + 112;
        let imp_rva = *(dd as *const u32).add(2) as usize; // 目录项 1 = Import
        if imp_rva == 0 {
            return false;
        }
        let mut desc = base + imp_rva;
        loop {
            let oft = *(desc as *const u32) as usize;
            let name_rva = *(desc as *const u32).add(3) as usize;
            let ft = *(desc as *const u32).add(4) as usize;
            if name_rva == 0 && ft == 0 {
                break;
            }
            if ft != 0 {
                let thunks = if oft != 0 { oft } else { ft };
                let mut i = 0usize;
                loop {
                    let v = *((base + thunks) as *const u64).add(i);
                    if v == 0 {
                        break;
                    }
                    if v & 0x8000_0000_0000_0000 == 0 {
                        let n = base + (v as usize & 0x7fff_ffff) + 2;
                        let mut len = 0usize;
                        while *(n as *const u8).add(len) != 0 {
                            len += 1;
                        }
                        let s = std::slice::from_raw_parts(n as *const u8, len);
                        if s == func {
                            let slot = ((base + ft) as *mut u64).add(i);
                            let mut old = PAGE_PROTECTION_FLAGS(0);
                            if VirtualProtect(
                                slot as *const _,
                                8,
                                PAGE_READWRITE,
                                &mut old,
                            )
                            .is_err()
                            {
                                return false;
                            }
                            *slot = replacement as u64;
                            let mut tmp = PAGE_PROTECTION_FLAGS(0);
                            let _ = VirtualProtect(slot as *const _, 8, old, &mut tmp);
                            return true;
                        }
                    }
                    i += 1;
                }
            }
            desc += 20;
        }
        false
    }

    pub(super) fn install(base: i64) -> i32 {
        CLOCK_BASE.store(base, Ordering::Relaxed);
        unsafe {
            // 目标：**原版** SimDLL.dll（我们的模块名是 SimDLL_RS.sim，不冲突）。
            // 它的 CRT 是静态链接的，_time64 是模块内副本，只能通过它自己的
            // GetSystemTimeAsFileTime 导入槽拦。
            let module = match GetModuleHandleA(windows::core::s!("SimDLL.dll")) {
                Ok(m) => m.0 as isize,
                Err(_) => {
                    tracing::warn!("clock_lock: SimDLL.dll not loaded");
                    return -1;
                }
            };
            if patch_iat(module, b"GetSystemTimeAsFileTime", lock_get_system_time as *const ()) {
                tracing::info!(base, "clock lock installed on SimDLL.dll IAT");
                1
            } else {
                tracing::warn!("clock_lock: GetSystemTimeAsFileTime thunk not found");
                -3
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn install(_base: i64) -> i32 {
        // Linux 下原版为 libSimDLL.so，时间戳来源不同，暂不实现。
        -99
    }
}
