//! Sim 线程 — Sim::Main 主循环 + Thread::Start/Join。
//!
//! 对照源码 08_sim_frame_manager.c L2086-2168 (Sim::Main)。
//!
//! 使用 std::thread::spawn 启动 sim 线程，运行 sim_main_loop。
//! 线程句柄存全局 G_SIM_THREAD，SIM_Shutdown 设置 exit flag 并 join。
//!
//! **双缓冲一帧延迟机制**（项目 memory 约束）：
//! sim 线程写入 m_sim_data，game 线程读取 m_game_data，
//! 每帧通过 SimSync/GameSync 交换，绝不改为直接同步。

use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};

use once_cell::sync::OnceCell;
use parking_lot::Mutex;

use crate::a_framework::frame_sync::FrameSync;
use crate::a_framework::sim::Sim;
use crate::a_framework::sim_frame_manager::SimFrameManager;
use crate::c2_physics;
use crate::c_simulation::sim_data_ops;
use crate::globals;

// ===== 全局线程句柄 =====

/// sim 线程句柄存储。
/// OnceCell 确保 thread::spawn 只执行一次。
/// Mutex<Option<JoinHandle>> 允许 take + join。
pub static G_SIM_THREAD: OnceCell<Mutex<Option<JoinHandle<()>>>> = OnceCell::new();

// ===== Sim::Main 主循环 =====

/// Sim::Main — sim 线程主循环。
///
/// 对照源码 08_sim_frame_manager.c L2086-2168。
///
/// 流程：
/// ```text
/// while !mExitRequested:
///   1. lock frameSync->m_sim_mutex
///   2. 保存 m_sim_data 指针（CopySimDataToGame 写入目标）
///   3. ConduitTemperatureManager::ReleaseQueuedHandles（C1 桩，跳过）
///   4. 获取 gSimData
///   5. BeginFrameProcessing → 返回活跃帧数 N
///   6. if N < 1:
///        UpdateComponentsDataListOnly(gSimData)
///      else:
///        循环 N 次:
///          elapsed = ProcessNextFrame(gSimData)
///          if elapsed <= 0:
///            CopyUpdatedCellsToCells(gSimData)
///            UpdateComponentsDataListOnly(gSimData)
///          else:
///            total = elapsed + timeRemainder
///            subSteps = max(1, (uint)(total * 5.0))
///            timeRemainder = max(0, total - subSteps * 0.2)
///            循环 subSteps 次: UpdateData(gSimData)
///   7. EndFrameProcessing
///   8. CopySimDataToGame(gSimData → m_sim_data)
///   9. unlock m_sim_mutex
///  10. SimSync（通知游戏线程数据就绪）
/// ```
pub fn sim_main_loop(sim_ptr: *mut Sim) {
    let sim = unsafe { &*sim_ptr };
    let frame_sync_ptr = sim.frame_sync;
    if frame_sync_ptr.is_null() {
        tracing::error!("Sim::Main: frame_sync is null, thread exiting");
        return;
    }

    tracing::info!("Sim::Main loop started");

    while !globals::G_SIM_EXIT_REQUESTED.load(Ordering::Acquire) {
        // 1. lock m_sim_mutex（数据锁，保护帧处理 + CopySimDataToGame）
        // 对照源码 L2106-2110
        unsafe { (*frame_sync_ptr).m_sim_mutex.lock_raw(); }

        // 2. 保存 m_sim_data 指针（CopySimDataToGame 的写入目标）
        // 对照源码 L2111: local_res20 = this->frameSync->mSimData
        let m_sim_data_ptr = unsafe { (*frame_sync_ptr).m_sim_data.ptr };

        // 2b. 保存 m_game_data 指针（CopySimDataToGame 后处理读取旧可见状态）
        // 对照源码 CopySimDataToGame 的 param_3 = mGameData
        let m_game_data_ptr = unsafe { (*frame_sync_ptr).m_game_data.ptr };

        // 3. ConduitTemperatureManager::ReleaseQueuedHandles
        // 对照源码 L2113-2116: if (gConduitTemperatureManager != null) ReleaseQueuedHandles
        // 回收上一轮延迟释放的管道温度句柄（Remove 后两轮真正释放，防止遍历中复用）
        crate::c_simulation::conduit_temperature::release_queued_handles_global();

        // 4. 获取 gSimData
        // 对照源码 L2117: this_00 = gSimData
        let sim_data_ptr = globals::G_SIM_DATA.lock().0;
        if sim_data_ptr.is_null() {
            tracing::warn!("Sim::Main: gSimData is null, skipping frame");
            unsafe { (*frame_sync_ptr).m_sim_mutex.unlock_raw(); }
            // 避免忙等
            std::thread::sleep(std::time::Duration::from_millis(1));
            continue;
        }

        // SimFrameManager 位于 Sim + 0x50（对照 sim_api.rs L142）
        let frame_manager_ptr = unsafe {
            (sim_ptr as *mut u8).add(0x50) as *mut SimFrameManager
        };

        // 5. BeginFrameProcessing → 返回活跃帧数
        // 对照源码 L2118
        let active_frame_count = unsafe { (*frame_manager_ptr).begin_frame_processing() };

        // 6. 处理帧
        // 对照源码 L2121-2156
        if active_frame_count < 1 {
            // 无活跃帧：只更新组件数据列表
            // 对照源码 L2122: SimData::UpdateComponentsDataListOnly
            unsafe { c2_physics::update_components_data_list_only(&mut *sim_data_ptr); }
        } else {
            // 循环处理每帧
            // 对照源码 L2125-2155: do { ... } while (--lVar6 != 0)
            for _ in 0..active_frame_count {
                // ProcessNextFrame 返回 elapsed_seconds
                // 对照源码 L2126-2127
                let elapsed = unsafe {
                    (*frame_manager_ptr).process_next_frame(&mut *sim_data_ptr)
                };

                if elapsed <= 0.0 {
                    // 无时间流逝：直接同步 cells + 更新组件
                    // 对照源码 L2128-2130
                    unsafe {
                        sim_data_ops::copy_updated_cells_to_cells(&mut *sim_data_ptr);
                        c2_physics::update_components_data_list_only(&mut *sim_data_ptr);
                    }
                } else {
                    // 有时间流逝：计算子步数并执行 UpdateData
                    // 对照源码 L2132-2152

                    // 加上上一帧余量
                    // 对照源码 L2133: fVar8 = fVar8 + *(float *)&this->_padding_
                    let remainder = f32::from_bits(
                        globals::G_SIM_TIME_REMAINDER.load(Ordering::Acquire)
                    );
                    let total_time = elapsed + remainder;

                    // 计算子步数（200ms = 0.2s 一个子步，5.0 = 1/0.2）
                    // 对照源码 L2134: uVar2 = (uint)(fVar8 * 5.0)
                    let sub_steps_raw = (total_time * 5.0) as u32;
                    // 对照源码 L2135-2138: uVar4 = max(1, uVar2)
                    let sub_steps = if sub_steps_raw < 1 { 1 } else { sub_steps_raw };

                    // 计算新余量
                    // 对照源码 L2139-2145:
                    //   newRemainder = total - (int)subSteps * 0.2
                    //   if (newRemainder <= 0) newRemainder = 0
                    let new_remainder = total_time - (sub_steps as f32) * 0.2;
                    let new_remainder = if new_remainder <= 0.0 { 0.0 } else { new_remainder };
                    globals::G_SIM_TIME_REMAINDER.store(new_remainder.to_bits(), Ordering::Release);

                    // 执行 UpdateData 子步
                    // 对照源码 L2146-2152: do { UpdateData } while (--uVar5 != 0)
                    for _ in 0..sub_steps {
                        unsafe { c2_physics::update_data(&mut *sim_data_ptr); }
                    }

                    // 帧末同步 updated_cells → cells（对照原版 SimBase::UpdateData
                    // 内部多处的 CellSOA::CopyFrom(cells, updatedCells)，
                    // 见 11_msvcrt_ignored.c L41903/42036/42392/42557）。
                    // C2 update_data 目前是空桩，若不在此同步，Start 之后所有写入
                    // updated_cells 的修改（worldgen 模板 ModifyCell、游戏内 dig/沙盒）
                    // 永远不会进入 cells，而 BeginSave 序列化的是 cells ——
                    // 导致 worldgen 第 498 步的 POI 模板方块（氧石/中子质基座/空间）
                    // 丢失、POI 被自然方块掩埋（2026-08-01 实测问题 2）。
                    unsafe {
                        sim_data_ops::copy_updated_cells_to_cells(&mut *sim_data_ptr);
                    }
                }
            }
        }

        // 7. EndFrameProcessing
        // 对照源码 L2157
        unsafe { (*frame_manager_ptr).end_frame_processing(); }

        // 8. CopySimDataToGame(gSimData → m_sim_data)
        // 对照源码 L2158-2160:
        //   SimBase::CopySimDataToGame(this, gSimData, mSimData, mGameData, frameCount)
        // 注意：源码传递了 mSimData 和 mGameData 两个参数，但实际只写入 mSimData。
        // GameSync 时 mSimData ↔ mGameData 交换，游戏线程读取 mGameData。
        if !m_sim_data_ptr.is_null() {
            unsafe {
                sim_data_ops::copy_sim_data_to_game(
                    &mut *sim_data_ptr,
                    &mut *m_sim_data_ptr,
                    m_game_data_ptr as *const crate::a_framework::game_data::GameData,
                    active_frame_count,
                );
            }
        } else {
            tracing::warn!("Sim::Main: m_sim_data is null, skip CopySimDataToGame");
        }

        // 9. unlock m_sim_mutex
        // 对照源码 L2161
        unsafe { (*frame_sync_ptr).m_sim_mutex.unlock_raw(); }

        // 10. SimSync（通知游戏线程数据就绪，等待消费）
        // 对照源码 L2165: FrameSync::SimSync(this->frameSync)
        // **必须在 m_sim_mutex 解锁之后调用**，否则 GameSync 死锁。
        unsafe { FrameSync::sim_sync(frame_sync_ptr); }
    }

    tracing::info!("Sim::Main loop exited");
}

// ===== Thread::Start / Join =====

/// Thread::Start — 启动 sim 线程。
///
/// 对照源码 Thread::Start + Sim::Main。
/// 使用 std::thread::spawn 创建后台线程运行 sim_main_loop。
///
/// **调用时机**：handle_start 末尾（FrameSync::initGameData 之后）。
/// **线程安全**：sim 线程通过 G_SIM_DATA Mutex 访问 gSimData，
/// 通过 FrameSync 的 m_sim_mutex/m_mutex 保护帧数据交换。
pub fn start_sim_thread(sim_ptr: *mut Sim) {
    // 重置退出标志和时间余量
    globals::G_SIM_EXIT_REQUESTED.store(false, Ordering::Release);
    globals::G_SIM_TIME_REMAINDER.store(0, Ordering::Release);

    // 获取或创建线程句柄存储
    let thread_holder = G_SIM_THREAD.get_or_init(|| {
        Mutex::new(None)
    });

    let mut holder = thread_holder.lock();

    // 如果已有线程在运行，先 join（防御性处理）
    if let Some(old_handle) = holder.take() {
        tracing::warn!("start_sim_thread: old thread still running, joining...");
        globals::G_SIM_EXIT_REQUESTED.store(true, Ordering::Release);
        wake_sim_thread();
        let _ = old_handle.join();
        globals::G_SIM_EXIT_REQUESTED.store(false, Ordering::Release);
    }

    // 启动新线程
    // 将裸指针转为 usize 以满足 Send 约束（usize 是 Send）
    let sim_ptr_addr = sim_ptr as usize;
    let handle = thread::Builder::new()
        .name("sim_thread".to_string())
        .spawn(move || {
            let sim_ptr = sim_ptr_addr as *mut Sim;
            sim_main_loop(sim_ptr);
        })
        .expect("failed to spawn sim thread");

    *holder = Some(handle);
    tracing::info!("sim thread started");
}

/// Thread::Join — 等待 sim 线程退出。
///
/// 对照源码 Thread::Join。
/// 设置退出标志，唤醒可能阻塞的 sim 线程，然后 join。
pub fn join_sim_thread() {
    let thread_holder = match G_SIM_THREAD.get() {
        Some(h) => h,
        None => {
            tracing::debug!("join_sim_thread: no sim thread started");
            return;
        }
    };

    // 1. 设置退出标志
    globals::G_SIM_EXIT_REQUESTED.store(true, Ordering::Release);

    // 2. 唤醒可能在 sim_sync 中阻塞的 sim 线程
    // sim_sync 等待 m_sim_cond（游戏线程通过 game_sync 唤醒）。
    // 如果游戏线程不再调用 game_sync，sim 线程会永远阻塞。
    // 通过 broadcast m_sim_cond 唤醒，sim_sync 中的退出标志检查会 break。
    wake_sim_thread();

    // 3. join
    let mut holder = thread_holder.lock();
    if let Some(handle) = holder.take() {
        match handle.join() {
            Ok(()) => tracing::info!("sim thread joined"),
            Err(e) => tracing::warn!("join_sim_thread: thread join failed: {:?}", e),
        }
    }

    // 4. 重置退出标志和时间余量
    globals::G_SIM_EXIT_REQUESTED.store(false, Ordering::Release);
    globals::G_SIM_TIME_REMAINDER.store(0, Ordering::Release);
}

/// 唤醒可能在 sim_sync 中阻塞的 sim 线程。
///
/// sim_sync 在 `while m_sim_ready_to_swap { wait m_sim_cond }` 中阻塞。
/// 通过 broadcast m_sim_cond 唤醒，sim_sync 检查退出标志后 break。
///
/// **注意**：此函数不设置 m_sim_ready_to_swap = false，
/// 因为 sim_sync 中的退出标志检查会处理。
fn wake_sim_thread() {
    let fs_ptr = FrameSync::get_ptr();
    if fs_ptr.is_null() {
        return;
    }
    unsafe {
        // lock m_mutex（sim_sync 持有 m_mutex 时 wait 会释放它）
        (*fs_ptr).m_mutex.lock_raw();
        // broadcast 唤醒所有等待 m_sim_cond 的线程
        (*fs_ptr).m_sim_cond.broadcast();
        (*fs_ptr).m_mutex.unlock_raw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;

    #[test]
    fn sim_exit_flag_defaults_to_false() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        // 重置
        globals::G_SIM_EXIT_REQUESTED.store(false, Ordering::Release);
        assert!(!globals::G_SIM_EXIT_REQUESTED.load(Ordering::Acquire));
    }

    #[test]
    fn sim_time_remainder_defaults_to_zero() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        globals::G_SIM_TIME_REMAINDER.store(0, Ordering::Release);
        let remainder = f32::from_bits(globals::G_SIM_TIME_REMAINDER.load(Ordering::Acquire));
        assert_eq!(remainder, 0.0);
    }

    #[test]
    fn join_sim_thread_without_start_is_safe() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        // 未启动 sim 线程时调用 join 应安全返回
        join_sim_thread();
    }

    #[test]
    fn start_and_join_sim_thread_with_null_frame_sync() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        // 创建一个 frame_sync 为 null 的 Sim
        let mut sim = Sim::new_zeroed();
        sim.frame_sync = std::ptr::null_mut();
        let sim_ptr = &mut sim as *mut Sim;

        // 启动 sim 线程（会因 frame_sync=null 立即退出）
        start_sim_thread(sim_ptr);
        // 等待线程退出
        std::thread::sleep(std::time::Duration::from_millis(100));
        join_sim_thread();
    }
}
