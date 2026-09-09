//! sim 帧耗时探针（诊断用）：累计 update_data 耗时，5 秒窗口写
//! `<游戏 logs>/simdll_perf.log`（与 simdll.log 同目录）。
//! 用于对比 D1/D2/串行的 sim 侧总耗时，判断 sim 是否为主线程外的瓶颈。
//! 无侵入：只在 update_data 入口/出口各取一次时间戳。

use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Instant;

struct FrameSample {
    frames: u32,
    total_us: u64,
    max_us: u64,
    last_region_count: usize,
    last_mode: i32,
    last_threads: usize,
}

static PERF: Mutex<FrameSample> = Mutex::new(FrameSample {
    frames: 0,
    total_us: 0,
    max_us: 0,
    last_region_count: 0,
    last_mode: 0,
    last_threads: 1,
});

static WINDOW_START: OnceLock<Mutex<Instant>> = OnceLock::new();

fn window_start() -> &'static Mutex<Instant> {
    WINDOW_START.get_or_init(|| Mutex::new(Instant::now()))
}

/// update_data 完成一帧后调用（region_count、mode=0 串行/1 D2/2 D1）。
pub fn frame_done(region_count: usize, mode: i32, threads: usize, elapsed_us: u64) {
    // 受日志开关控制：默认关闭（发布版不生成 simdll_perf.log）；
    // 玩家开启日志后本探针才统计并写入。
    if !crate::a_framework::logger::is_log_enabled() {
        return;
    }
    let mut p = PERF.lock().unwrap();
    p.frames += 1;
    p.total_us += elapsed_us;
    if elapsed_us > p.max_us {
        p.max_us = elapsed_us;
    }
    p.last_region_count = region_count;
    p.last_mode = mode;
    p.last_threads = threads;

    let elapsed = window_start().lock().unwrap().elapsed();
    if elapsed.as_millis() >= 5000 {
        let win_ms = elapsed.as_millis();
        let avg_us = if p.frames > 0 {
            p.total_us / p.frames as u64
        } else {
            0
        };
        let line = format!(
            "[simdll-perf] window={win_ms}ms frames={} mode={} regions={} threads={} avg={:.2}ms max={:.2}ms\n",
            p.frames,
            p.last_mode,
            p.last_region_count,
            p.last_threads,
            avg_us as f64 / 1000.0,
            p.max_us as f64 / 1000.0
        );
        // 阶段分解行（2026-09-06 方向 A）：与 SimDLL_PerfProbe（C# 侧）互补。
        // 口径标注：copy/emitters/sim_to_game = 主线程串行段（真实墙钟）；
        // 其余 = 阶段 A/C rayon worker 内 CPU 累计（并行下 > 墙钟，除以并行
        // 线程数估墙钟下界；serial 区域数=1 时等于墙钟）。
        let stages = crate::a_framework::stage_profiler::drain_stages();
        let frames_f = p.frames.max(1) as f64;
        let mut stage_line = String::with_capacity(160);
        stage_line.push_str("[simdll-stages] ");
        for (i, ns) in stages.iter().enumerate() {
            if i > 0 {
                stage_line.push(' ');
            }
            stage_line.push_str(&format!(
                "{}={:.2}",
                crate::a_framework::stage_profiler::stage_name(i),
                *ns as f64 / frames_f / 1000.0
            ));
        }
        stage_line.push_str(" (ms/frame)\n");
        // 写游戏 logs 目录（与 simdll.log 同目录，动态推导）；失败回退临时目录。
        let dir = crate::a_framework::logger::resolve_log_dir();
        let path = if std::fs::create_dir_all(&dir).is_ok() {
            dir.join("simdll_perf.log")
        } else {
            std::env::temp_dir().join("simdll_perf.log")
        };
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = f.write_all(line.as_bytes());
            let _ = f.write_all(stage_line.as_bytes());
        }
        *p = FrameSample {
            frames: 0,
            total_us: 0,
            max_us: 0,
            last_region_count: 0,
            last_mode: 0,
            last_threads: 1,
        };
        *window_start().lock().unwrap() = Instant::now();
    }
}
