//! C ABI 导出层：9 个 `SIM_*/SYSINFO_*` 函数。
//!
//! 严格对照源码 01_sim_api.c L98220-L98625。
//! A1 阶段全部为桩函数：SIM_Initialize 注册 handler + 初始化日志/崩溃处理，
//! 其余返回 null/void。
//! 每个函数包 `catch_unwind`，panic 不跨 C ABI 边界。

#![allow(dead_code)]

use std::os::raw::{c_char, c_int, c_schar, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};

use super::abi::GameMessageHandler;
use super::buffer::BinaryBufferReader;
use super::crashdump;
use super::frame_sync::FrameSync;
use super::save_load;
use super::sim::Sim;
use crate::globals::{G_FRAME_SYNC, G_GAME_MESSAGE_HANDLER, G_SIM};
use super::logger;
use super::sysinfo;

/// SIM_Initialize：注册消息回调 + 初始化日志和崩溃处理 + 创建 gSim。
///
/// 对照源码 01_sim_api.c L300-343。
/// 原版签名：`void SIM_Initialize(GameMessageHandler *param_1)`
/// Rust 用 `Option<GameMessageHandler>` 接收可空函数指针（ABI 等价）。
///
/// 源码流程（L312-342）：
/// 1. Timer::Initialize()（A3 桩）
/// 2. 创建 Logger（A1 已实现）
/// 3. SymInitialize + InitializeCrashDumpHandler（A1 已实现）
/// 4. **CleanUp()** — 清理旧资源
/// 5. gGameMessageHandler = param_1
/// 6. **FrameSync::clear(&gFrameSync)** — 清空 FrameSync
/// 7. **创建 gSim**：operator_new(0x228) + Sim::Sim(this, &gFrameSync)
/// 8. 替换旧 gSim
///
/// **关键**：gSim 必须在此创建，否则 SIM_HandleMessage 会因 gSim==null 直接返回 null，
/// 导致所有消息（包括 AllocateCells/Load/Start）无法处理。
#[no_mangle]
pub extern "C" fn SIM_Initialize(handler: Option<GameMessageHandler>) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // 初始化日志（仅当接口 mod 启用了日志；默认关闭 → 不创建日志文件）
        logger::ensure_init_if_enabled();
        // 初始化崩溃处理
        crashdump::init();

        // L327: CleanUp() — 清理旧资源
        save_load::clean_up();

        // 注册 handler（原版 gGameMessageHandler = param_1，无条件赋值）
        *G_GAME_MESSAGE_HANDLER.lock() = handler;

        // L329: FrameSync::clear(&gFrameSync)
        // 初始化 G_FRAME_SYNC（如果尚未初始化）并 clear
        let fs_mutex = G_FRAME_SYNC.get_or_init(|| {
            parking_lot::Mutex::new(FrameSync::new_zeroed())
        });
        {
            let mut fs = fs_mutex.lock();
            fs.clear();
        }

        // 原版 Sim::Sim → SimBase::InitializeTasks（L132419-132433）：创建 1-worker
        // ParallelTaskQueue 作为 this->taskQueue。D2：全局队列，温度任务分发用。
        crate::globals::init_parallel_task_queue();

        // L330-333: 创建 gSim
        // Sim::Sim(this, &gFrameSync) 需要指向 gFrameSync 的地址。
        // G_FRAME_SYNC 是 static 的 OnceCell<Mutex<FrameSync>>，
        // OnceCell 初始化后内部 FrameSync 地址在程序运行期间永不改变。
        let fs_ptr = {
            let fs = fs_mutex.lock();
            &*fs as *const FrameSync as *mut FrameSync
        };
        let sim = Box::new(Sim::new(fs_ptr));
        let sim_ptr = Box::into_raw(sim);

        // L334-341: 替换旧 gSim（释放旧的）
        let mut global_sim = G_SIM.lock();
        if !global_sim.0.is_null() {
            unsafe { let _ = Box::from_raw(global_sim.0); }
        }
        global_sim.0 = sim_ptr;

        tracing::info!("SIM_Initialize called, gSim created");
    }));
}

/// SIM_Shutdown：清理资源。
///
/// 对照源码 01_sim_api.c。
/// C1 实现：
/// 1. join sim 线程（设置退出标志 + 唤醒 + join）
/// 2. 调用 clean_up 释放全局资源
/// 3. flush 日志
#[no_mangle]
pub extern "C" fn SIM_Shutdown() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        tracing::info!("SIM_Shutdown called");

        // 1. join sim 线程（必须在 clean_up 之前，避免释放 gSim 时 sim 线程仍在访问）
        super::sim_thread::join_sim_thread();

        // 2. 清理全局资源（释放 gSim/gSimData/gDisease/SaveBuffer）
        // 先关停任务队列（join worker），再释放 sim 资源
        crate::globals::shutdown_parallel_task_queue();
        save_load::clean_up();

        // 3. flush 日志
        logger::flush();
    }));
}

/// SIM_HandleMessage：处理单条消息（两阶段分派）。
///
/// 对照源码 01_sim_api.c L208-244。
///
/// **两阶段分派**：
/// 1. 检查 gSim 是否非 null（L219-221）：如果为 null，返回 null
/// 2. 构造 BinaryBufferReader（L225）
/// 3. **阶段 1**：SimFrameManager::HandleMessage(gSim + 0x50, ...) 返回 bool（L226-228）
///    - 返回 true：已处理，返回 out_result
///    - 返回 false：未处理，进入阶段 2
/// 4. **阶段 2**：线性查找 14 项 gSimMessageHandlers 表（L230-238）
///    - 找到匹配的 handler：执行并返回结果
///    - 未找到：返回 null
#[no_mangle]
pub extern "C" fn SIM_HandleMessage(id: c_int, size: c_int, data: *mut c_schar) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        // 诊断日志：记录入口参数（info 级别，用于诊断 data=null 消息不分派问题）
        let data_null = data.is_null();
        // 噪音（每帧每条消息高频）：降为 debug，发布版 info 下不输出。
        tracing::debug!(id, size, data_null, "SIM_HandleMessage entry");

        // L219: 检查 gSim 是否非 null
        let sim_ptr = crate::globals::G_SIM.lock().0;
        if sim_ptr.is_null() {
            tracing::warn!(id, "SIM_HandleMessage: gSim is null, returning null");
            return std::ptr::null_mut();
        }

        // L225: 构造 BinaryBufferReader
        // 对照原版 01_sim_api.c L225：不检查 data=null/size<=0，直接构造 reader。
        // C# 的 Sim.Start() 传入 size=0, data=null（Start 消息不需要 reader 数据），
        // 原版用空 reader 继续分派，handler 自行决定是否读取。
        let data_slice: &[u8] = if data.is_null() || size <= 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) }
        };
        let mut reader = BinaryBufferReader::new(data_slice);

        // 阶段 1：SimFrameManager::HandleMessage(gSim + 0x50, ...)
        // 对照源码 L226-228: SimFrameManager 位于 gSim + 0x50
        let mut result: *mut c_void = std::ptr::null_mut();
        let handled = unsafe {
            let frame_manager_ptr = (sim_ptr as *mut u8).add(0x50) as *mut super::sim_frame_manager::SimFrameManager;
            let frame_manager = &mut *frame_manager_ptr;
            frame_manager.handle_message(id as u32, &mut reader, &mut result)
        };

        // 阶段 2：如果阶段 1 未处理，查 14 项 handler 表
        // 对照源码 L229-238
        if !handled {
            if let Some(handler_result) = super::message_handler::dispatch_handler_table(id as u32, &mut reader) {
                tracing::debug!(id, "phase 2 dispatched");
                result = handler_result;
            } else {
                tracing::warn!(id, "phase 2 no handler found");
            }
        }

        // 噪音（每帧高频）：降为 debug。
        tracing::debug!(id, handled, result_null = result.is_null(), "SIM_HandleMessage done");
        result
    }))
    .unwrap_or_else(|_| {
        tracing::error!("SIM_HandleMessage panicked");
        std::ptr::null_mut()
    })
}

/// SIM_HandleMessages：批量处理消息（两阶段分派）。
///
/// 对照源码 01_sim_api.c L248-298。
///
/// 对每条消息执行与 SIM_HandleMessage 相同的两阶段分派。
/// 每条消息的 data 偏移递增 size 字节。
/// 返回最后一条消息的处理结果（如果 count <= 0 返回 null）。
#[no_mangle]
pub extern "C" fn SIM_HandleMessages(
    id: c_int,
    size: c_int,
    count: c_int,
    data: *mut c_schar,
) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        // L266: 原版仅检查 count <= 0；data=null/size=0 时原版用空 reader 继续分派
        // （Rust 此前额外拒绝 data.is_null()/size<=0，可能拒绝合法批量消息）。
        if count <= 0 {
            return std::ptr::null_mut();
        }

        // L219: 检查 gSim 是否非 null
        let sim_ptr = crate::globals::G_SIM.lock().0;
        if sim_ptr.is_null() {
            tracing::trace!(id, "SIM_HandleMessages: gSim is null");
            return std::ptr::null_mut();
        }

        let mut result: *mut c_void = std::ptr::null_mut();
        let mut offset = 0usize;

        for _ in 0..count {
            // L272: 构造 BinaryBufferReader
            let data_slice: &[u8] = if data.is_null() || size <= 0 {
                &[]
            } else {
                unsafe {
                    std::slice::from_raw_parts(data.add(offset) as *const u8, size as usize)
                }
            };
            let mut reader = BinaryBufferReader::new(data_slice);

            // 阶段 1：SimFrameManager::HandleMessage(gSim + 0x50, ...)
            let mut msg_result: *mut c_void = std::ptr::null_mut();
            let handled = unsafe {
                let frame_manager_ptr = (sim_ptr as *mut u8).add(0x50) as *mut super::sim_frame_manager::SimFrameManager;
                let frame_manager = &mut *frame_manager_ptr;
                frame_manager.handle_message(id as u32, &mut reader, &mut msg_result)
            };

            // 阶段 2：如果阶段 1 未处理，查 handler 表
            if !handled {
                if let Some(handler_result) = super::message_handler::dispatch_handler_table(id as u32, &mut reader) {
                    msg_result = handler_result;
                }
            }

            result = msg_result;
            offset += size as usize;
        }

        tracing::trace!(id, count, "SIM_HandleMessages done");
        result
    }))
    .unwrap_or_else(|_| {
        tracing::error!("SIM_HandleMessages panicked");
        std::ptr::null_mut()
    })
}

/// SIM_BeginSave：开始保存存档。
///
/// 对照源码 01_sim_api.c L7-171。
/// 委托给 save_load::begin_save（A3 真实实现）。
#[no_mangle]
pub extern "C" fn SIM_BeginSave(out_size: *mut c_int, p2: c_int, p3: c_int) -> *mut c_schar {
    catch_unwind(AssertUnwindSafe(|| {
        save_load::begin_save(out_size, p2, p3)
    }))
    .unwrap_or_else(|_| {
        tracing::error!("SIM_BeginSave panicked");
        if !out_size.is_null() {
            unsafe { *out_size = 0; }
        }
        std::ptr::null_mut()
    })
}

/// SIM_EndSave：结束保存存档。
///
/// 对照源码 01_sim_api.c L195-204。
/// 委托给 save_load::end_save（A3 真实实现）。
#[no_mangle]
pub extern "C" fn SIM_EndSave() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        save_load::end_save();
    }));
}

/// SIM_DebugCrash：调试用触发崩溃。
///
/// 对照源码 01_sim_api.c。
#[no_mangle]
pub extern "C" fn SIM_DebugCrash() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        tracing::warn!("SIM_DebugCrash called");
        // A1 阶段：不实际触发崩溃
    }));
}

/// SYSINFO_Acquire：获取系统信息 JSON。
///
/// 对照源码 01_sim_api.c。
#[no_mangle]
pub extern "C" fn SYSINFO_Acquire() -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        tracing::trace!("SYSINFO_Acquire called");
        sysinfo::acquire()
    }))
    .unwrap_or_else(|_| {
        tracing::error!("SYSINFO_Acquire panicked");
        std::ptr::null_mut()
    })
}

/// SYSINFO_Release：释放系统信息字符串。
///
/// 对照源码 01_sim_api.c：`void SYSINFO_Release(void)`——无参，从全局指针释放
/// （2026-08-07 修正：此前多一个 ptr 形参，与 C# DllImport 声明不符）。
#[no_mangle]
pub extern "C" fn SYSINFO_Release() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        tracing::trace!("SYSINFO_Release called");
        unsafe { sysinfo::release(); }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;

    #[test]
    fn sim_initialize_with_null_handler_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        SIM_Initialize(None);
        // 验证 gSim 已创建
        let sim_ptr = crate::globals::G_SIM.lock().0;
        assert!(!sim_ptr.is_null(), "SIM_Initialize should create gSim");
        // 清理
        crate::a_framework::save_load::clean_up();
    }

    #[test]
    fn sim_initialize_creates_gsim_with_frame_sync() {
        let _lock = LIB_TESTS_LOCK.lock();
        SIM_Initialize(None);

        // 验证 gSim 已创建且 frame_sync 字段指向 G_FRAME_SYNC 内部
        let sim_ptr = crate::globals::G_SIM.lock().0;
        assert!(!sim_ptr.is_null());
        let sim = unsafe { &*sim_ptr };
        assert!(!sim.frame_sync.is_null(), "Sim.frame_sync should be non-null");

        // 验证 G_FRAME_SYNC 已初始化
        assert!(crate::globals::G_FRAME_SYNC.get().is_some());

        // 清理
        crate::a_framework::save_load::clean_up();
    }

    #[test]
    fn sim_shutdown_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        SIM_Shutdown();
    }

    #[test]
    fn sim_handle_message_null_data_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        // gSim 未创建（null）→ 返回 null
        crate::a_framework::save_load::clean_up();
        let result = SIM_HandleMessage(0, 0, std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn sim_handle_messages_zero_count_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        let result = SIM_HandleMessages(0, 0, 0, std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn sim_handle_message_unknown_id_returns_null_after_init() {
        let _lock = LIB_TESTS_LOCK.lock();
        SIM_Initialize(None);

        // 构造未知 ID 的消息（不在 14 项 handler 表中）
        let data = [0u8; 4];
        let result = SIM_HandleMessage(
            0xDEADBEEFu32 as i32 as c_int,
            4,
            data.as_ptr() as *mut c_schar,
        );
        assert!(result.is_null(), "unknown id should return null");

        crate::a_framework::save_load::clean_up();
    }

    #[test]
    fn sim_handle_message_allocate_cells_returns_non_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        SIM_Initialize(None);

        // 构造 AllocateCells 数据包：width=2, height=2, flag1=false, flag2=false
        // int(4) + int(4) + bool(1) + bool(1) = 10 bytes
        let mut buf = Vec::new();
        buf.extend_from_slice(&2i32.to_le_bytes());       // width
        buf.extend_from_slice(&2i32.to_le_bytes());       // height
        buf.push(0u8);                                     // flag1
        buf.push(0u8);                                     // flag2

        let result = SIM_HandleMessage(
            crate::a_framework::message_handler::MessageType::AllocateCells as c_int,
            buf.len() as c_int,
            buf.as_ptr() as *mut c_schar,
        );
        assert!(!result.is_null(), "AllocateCells should return non-null via two-phase dispatch");

        // 验证 gSimData 已设置
        let sim_data_ptr = crate::globals::G_SIM_DATA.lock().0;
        assert!(!sim_data_ptr.is_null());

        crate::a_framework::save_load::clean_up();
    }

    #[test]
    fn sim_begin_save_null_sim_data_returns_null_and_zero_size() {
        let _lock = LIB_TESTS_LOCK.lock();
        save_load::clean_up();
        let mut size: c_int = -1;
        let result = SIM_BeginSave(&mut size, 0, 0);
        assert!(result.is_null());
        assert_eq!(size, 0);
        save_load::clean_up();
    }

    #[test]
    fn sim_begin_save_with_null_out_size_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        save_load::clean_up();
        let result = SIM_BeginSave(std::ptr::null_mut(), 0, 0);
        assert!(result.is_null());
        save_load::clean_up();
    }

    #[test]
    fn sim_end_save_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        save_load::clean_up();
        SIM_EndSave();
        save_load::clean_up();
    }

    #[test]
    fn sim_debug_crash_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        SIM_DebugCrash();
    }

    #[test]
    fn sysinfo_acquire_returns_non_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        let ptr = SYSINFO_Acquire();
        assert!(!ptr.is_null());
        SYSINFO_Release();
    }

    #[test]
    fn sysinfo_release_no_arg_is_safe() {
        let _lock = LIB_TESTS_LOCK.lock();
        SYSINFO_Release();
    }

    #[test]
    fn sim_begin_save_returns_null_via_ffi() {
        let _lock = LIB_TESTS_LOCK.lock();
        save_load::clean_up();
        let mut size: c_int = -1;
        let result = SIM_BeginSave(&mut size, 0, 0);
        assert!(result.is_null());
        save_load::clean_up();
    }
}
