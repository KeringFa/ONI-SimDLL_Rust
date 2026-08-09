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
}

static PERF: Mutex<FrameSample> = Mutex::new(FrameSample {
    frames: 0,
    total_us: 0,
    max_us: 0,
    last_region_count: 0,
    last_mode: 0,
});

static WINDOW_START: OnceLock<Mutex<Instant>> = OnceLock::new();

fn window_start() -> &'static Mutex<Instant> {
    WINDOW_START.get_or_init(|| Mutex::new(Instant::now()))
}

/// update_data 完成一帧后调用（region_count、mode=0 串行/1 D2/2 D1）。
pub fn frame_done(region_count: usize, mode: i32, elapsed_us: u64) {
    let mut p = PERF.lock().unwrap();
    p.frames += 1;
    p.total_us += elapsed_us;
    if elapsed_us > p.max_us {
        p.max_us = elapsed_us;
    }
    p.last_region_count = region_count;
    p.last_mode = mode;

    let elapsed = window_start().lock().unwrap().elapsed();
    if elapsed.as_millis() >= 5000 {
        let win_ms = elapsed.as_millis();
        let avg_us = if p.frames > 0 {
            p.total_us / p.frames as u64
        } else {
            0
        };
        let line = format!(
            "[simdll-perf] window={win_ms}ms frames={} mode={} regions={} avg={:.2}ms max={:.2}ms\n",
            p.frames,
            p.last_mode,
            p.last_region_count,
            avg_us as f64 / 1000.0,
            p.max_us as f64 / 1000.0
        );
        // 写游戏 logs 目录（与 simdll.log 同目录）；失败回退临时目录。
        let game_logs = "D:\\Games\\steam\\steamapps\\common\\OxygenNotIncluded\\logs";
        let path = if std::fs::create_dir_all(game_logs).is_ok() {
            format!("{game_logs}\\simdll_perf.log")
        } else {
            std::env::temp_dir()
                .join("simdll_perf.log")
                .to_string_lossy()
                .into_owned()
        };
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = f.write_all(line.as_bytes());
        }
        *p = FrameSample {
            frames: 0,
            total_us: 0,
            max_us: 0,
            last_region_count: 0,
            last_mode: 0,
        };
        *window_start().lock().unwrap() = Instant::now();
    }
}
