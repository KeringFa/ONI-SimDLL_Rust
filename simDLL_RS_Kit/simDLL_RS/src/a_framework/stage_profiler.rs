//! sim 阶段耗时探针（2026-09-06 方向 A）：回答「StepTheSim 黑盒里
//! 气体/液体/辐射/组件/拷贝各占多少」，与 SimDLL_PerfProbe（C# 侧）
//! 的 5 秒窗口日志互补，拼出完整帧时间分解。
//!
//! 两层口径（输出时标注，解读规则不同）：
//! - **墙钟槽**（STAGE_COPY / STAGE_EMITTERS / STAGE_SIM_TO_GAME）：
//!   埋点在主线程串行段，累计值 == 真实墙钟。
//! - **CPU 槽**（温度/气体×2/液体×2/病菌/辐射/阶段C）：埋点在阶段 A/C 的
//!   rayon worker 内，多线程并发累加 → 数值 = CPU 总和（墙钟的若干倍），
//!   解读规则：占比上限判断——CPU 占比都低则墙钟必低；除以并行线程数
//!   可估墙钟下界。
//!
//! 无 feature 门控：埋点调用永远编译（Instant + 原子 add，开销 ~ns 级）；
//! 数据消费在 perf_probe（d3-rayon 构建才写日志），非 d3 构建数据累加
//! 但无人读——无害。
//!
//! 溢出：u64 纳秒累计，按每帧 20ms 算可连续运行 ~2.9 万年，无需回绕处理。

use std::sync::atomic::{AtomicU64, Ordering};

// ---- 阶段槽位 ----
pub const STAGE_COPY: usize = 0; // 每子步 cells←updated 拷贝（全网格+区域版合计）
pub const STAGE_TEMPERATURE: usize = 1; // 温度（含背墙换热/相变）
pub const STAGE_GAS_PRESSURE: usize = 2; // 气体压力均衡
pub const STAGE_GAS_DISPLACE: usize = 3; // 气体置换
pub const STAGE_LIQUID: usize = 4; // 液体流动
pub const STAGE_LIQUID_DISPLACE: usize = 5; // 液体置换
pub const STAGE_DISEASE_DIFFUSE: usize = 6; // 病菌扩散
pub const STAGE_RADIATION: usize = 7; // 辐射
pub const STAGE_EMITTERS: usize = 8; // 阶段 B：组件发射器（串行墙钟）
pub const STAGE_POSTPROCESS: usize = 9; // 阶段 C：太空真空+PostProcess+病菌生长
pub const STAGE_SIM_TO_GAME: usize = 10; // CopySimDataToGame（sim→C# 边界拷贝）
pub const STAGE_COUNT: usize = 11;

const STAGE_NAMES: [&str; STAGE_COUNT] = [
    "copy",
    "temperature",
    "gas_pressure",
    "gas_displace",
    "liquid",
    "liquid_displace",
    "disease_diffuse",
    "radiation",
    "emitters",
    "postprocess",
    "sim_to_game",
];

static STAGE_NANOS: [AtomicU64; STAGE_COUNT] = {
    #[allow(clippy::declare_interior_mutable_const)]
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [
        ZERO, ZERO, ZERO, ZERO, ZERO, ZERO, ZERO, ZERO, ZERO, ZERO, ZERO,
    ]
};

/// 阶段耗时累计（纳秒）。埋点：`let t0 = Instant::now(); <调用>;
/// stage_add(STAGE_X, t0.elapsed().as_nanos() as u64);`
pub fn stage_add(stage: usize, nanos: u64) {
    STAGE_NANOS[stage].fetch_add(nanos, Ordering::Relaxed);
}

/// 读走并清零全部阶段累计（perf_probe 每 5s 窗口调用一次）。
/// 返回 (阶段名数组引用, 各阶段纳秒)。
pub fn drain_stages() -> [u64; STAGE_COUNT] {
    let mut out = [0u64; STAGE_COUNT];
    for (i, slot) in STAGE_NANOS.iter().enumerate() {
        out[i] = slot.swap(0, Ordering::Relaxed);
    }
    out
}

/// 输出用：阶段名。
pub fn stage_name(i: usize) -> &'static str {
    STAGE_NAMES[i]
}

/// 受日志开关控制的阶段计时埋点（2026-09-06，发布版零探针开销）：
/// "Enable Logging" 关闭时不取时间戳、不累加——调用方代码路径上只剩
/// 一次 atomic load（is_log_enabled），秒级可忽略。
///
/// 用法：`sim_stage_timed!(STAGE_X, <表达式/语句块>)`。
/// `$call` 在展开中**只出现一次**（先行求值再按开关累加）——move 语义安全；
/// 之前"分支各写一份 $call"的宏形态会因 &mut reborrow 双份而编译失败。
#[macro_export]
macro_rules! sim_stage_timed {
    ($stage:expr, $call:expr) => {{
        let __logging = $crate::a_framework::logger::is_log_enabled();
        let __t0 = if __logging {
            Some(::std::time::Instant::now())
        } else {
            None
        };
        let __r = $call;
        if let Some(__t0) = __t0 {
            $crate::a_framework::stage_profiler::stage_add(
                $stage,
                __t0.elapsed().as_nanos() as u64,
            );
        }
        __r
    }};
}
