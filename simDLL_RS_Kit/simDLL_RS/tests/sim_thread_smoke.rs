//! FFI 集成测试：Sim 线程生命周期 smoke 测试。
//!
//! 验证 C1 阶段 Sim 线程的启动、运行、停止全流程：
//! SIM_Initialize → AllocateCells → Load → Start（启动 sim 线程）→ SIM_Shutdown（join sim 线程）
//!
//! 对照源码 08_sim_frame_manager.c L2086-2168 (Sim::Main)。

use SimDLL::a_framework::message_handler::MessageType;
use SimDLL::a_framework::sim_api::*;
use std::os::raw::{c_int, c_schar};

/// 串行化所有修改全局状态的测试，避免并行竞争。
static SIM_THREAD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 构造 AllocateCells 消息数据。
/// 对照源码 02_save_load.c L259-293。
fn make_allocate_cells_data(width: i32, height: i32) -> Vec<u8> {
    let mut data = Vec::with_capacity(10);
    data.extend_from_slice(&width.to_le_bytes());
    data.extend_from_slice(&height.to_le_bytes());
    data.push(0u8); // radiation_enabled = false
    data.push(0u8); // headless = false
    data
}

/// 构造最小存档数据（version 15）。
/// 对照 save_load_smoke.rs 中的 make_save_data。
fn make_save_data(save_width: i32, save_height: i32) -> Vec<u8> {
    let total_cells = (save_width as usize) * (save_height as usize);
    let mut data = Vec::new();

    // Header (29 bytes)
    data.extend_from_slice(b"SIMSAVE\0");
    data.extend_from_slice(&15i32.to_le_bytes());
    data.extend_from_slice(&save_width.to_le_bytes());
    data.extend_from_slice(&save_height.to_le_bytes());
    data.extend_from_slice(&0i32.to_le_bytes()); // offset
    data.extend_from_slice(&0i32.to_le_bytes()); // skip
    data.push(0u8); // flag

    // Cell data: element_hash(u32) + temp(f32) + mass(f32) + radiation(f32)
    for _ in 0..total_cells {
        data.extend_from_slice(&0x2d39bf75u32.to_le_bytes()); // vacuum hash
        data.extend_from_slice(&300.0f32.to_le_bytes());
        data.extend_from_slice(&100.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
    }

    // Disease data
    for _ in 0..total_cells {
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0i32.to_le_bytes());
    }

    // Backwall data
    for _ in 0..total_cells {
        data.extend_from_slice(&0x2d39bf75u32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
    }

    data
}

/// 完整的初始化流程：SIM_Initialize → AllocateCells → Load → Start
fn do_full_init(width: i32, height: i32) {
    SIM_Initialize(None);

    let alloc_data = make_allocate_cells_data(width, height);
    let result = SIM_HandleMessage(
        MessageType::AllocateCells as c_int,
        alloc_data.len() as c_int,
        alloc_data.as_ptr() as *mut c_schar,
    );
    assert!(!result.is_null(), "AllocateCells should return non-null");

    let save_data = make_save_data(width, height);
    let result = SIM_HandleMessage(
        MessageType::Load as c_int,
        save_data.len() as c_int,
        save_data.as_ptr() as *mut c_schar,
    );
    assert!(!result.is_null(), "Load should return non-null");

    let start_data = vec![0u8; 4];
    let result = SIM_HandleMessage(
        MessageType::Start as c_int,
        start_data.len() as c_int,
        start_data.as_ptr() as *mut c_schar,
    );
    assert!(!result.is_null(), "Start should return non-null GDU pointer");
}

// ===== 测试用例 =====

#[test]
fn sim_shutdown_without_start_is_safe() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();
    // SIM_Shutdown 在未启动 sim 线程时应安全返回
    SIM_Initialize(None);
    SIM_Shutdown();
}

#[test]
fn start_message_starts_sim_thread() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();
    do_full_init(4, 4);

    // sim 线程应已启动并在运行
    // 给线程一点时间启动
    std::thread::sleep(std::time::Duration::from_millis(50));

    // SIM_Shutdown 应能成功 join sim 线程
    SIM_Shutdown();
}

#[test]
fn sim_thread_survives_multiple_frames() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();
    do_full_init(4, 4);

    // 让 sim 线程运行多个帧周期
    // sim 线程在 sim_sync 中等待 game_sync，如果没有 game_sync，
    // 它会阻塞在 sim_sync 的 wait 中，不会忙等
    std::thread::sleep(std::time::Duration::from_millis(200));

    // shutdown 应能正常 join
    SIM_Shutdown();
}

#[test]
fn multiple_init_start_shutdown_cycles() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();

    // 第一轮
    do_full_init(2, 2);
    std::thread::sleep(std::time::Duration::from_millis(30));
    SIM_Shutdown();

    // 第二轮
    do_full_init(3, 3);
    std::thread::sleep(std::time::Duration::from_millis(30));
    SIM_Shutdown();

    // 第三轮
    do_full_init(4, 4);
    std::thread::sleep(std::time::Duration::from_millis(30));
    SIM_Shutdown();
}

#[test]
fn sim_shutdown_unblocks_blocked_sim_thread() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();
    do_full_init(4, 4);

    // sim 线程此时应该在 sim_sync 中阻塞（等待 game_sync 唤醒）
    // 给它时间到达 sim_sync
    std::thread::sleep(std::time::Duration::from_millis(100));

    // SIM_Shutdown 应通过设置退出标志 + broadcast m_sim_cond 来唤醒并 join
    SIM_Shutdown();
}

#[test]
fn reinitialize_after_shutdown_works() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();

    // 第一次完整周期
    do_full_init(4, 4);
    std::thread::sleep(std::time::Duration::from_millis(50));
    SIM_Shutdown();

    // 再次初始化不应崩溃
    SIM_Initialize(None);
    // 不需要再次 Shutdown（test 结束时清理）
}

/// 并发压力回归测试（2026-08-01 隐患 1）：
/// 游戏线程持续发 NewGameFrame（触发 new_frame，操作 frame_pool/queued_frames/current_frame）
/// + PrepareGameData（game_sync 唤醒 sim 线程处理帧，begin/end_frame_processing 持 frame_mutex），
/// 验证两线程并发操作帧队列不崩溃、帧循环健康。
///
/// 背景：new_frame 此前缺 frame_mutex 锁（原版 NewFrame 全程持锁），
/// 与 sim 线程的 begin/end_frame_processing 并发操作同一批 vector = 数据竞争隐患。
#[test]
fn concurrent_new_frame_and_frame_processing_stress() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();
    do_full_init(4, 4);

    // 模拟 worldgen/游戏节奏：每轮 NewGameFrame + PrepareGameData
    for _ in 0..200 {
        let mut frame_msg = Vec::with_capacity(28);
        frame_msg.extend_from_slice(&0.2f32.to_le_bytes()); // elapsedSeconds
        frame_msg.extend_from_slice(&0i32.to_le_bytes()); // min_x
        frame_msg.extend_from_slice(&0i32.to_le_bytes()); // min_y
        frame_msg.extend_from_slice(&0i32.to_le_bytes()); // max_x
        frame_msg.extend_from_slice(&0i32.to_le_bytes()); // max_y
        frame_msg.extend_from_slice(&0.0f32.to_le_bytes()); // sunlight
        frame_msg.extend_from_slice(&0.0f32.to_le_bytes()); // cosmic_rad
        let r = SIM_HandleMessage(
            MessageType::SimFrameManager_NewGameFrame as c_int,
            frame_msg.len() as c_int,
            frame_msg.as_ptr() as *mut c_schar,
        );
        let _ = r;

        let gdu = SIM_HandleMessage(
            MessageType::PrepareGameData as c_int,
            0,
            std::ptr::null_mut(),
        );
        assert!(!gdu.is_null(), "PrepareGameData 应返回 GDU（轮内 sim 线程应处理完该帧）");
    }

    // sim 线程仍健康：再发一次 PrepareGameData 正常返回
    let gdu = SIM_HandleMessage(
        MessageType::PrepareGameData as c_int,
        0,
        std::ptr::null_mut(),
    );
    assert!(!gdu.is_null());

    // SIM_Shutdown 应能正常 join（帧队列结构未被并发破坏）
    SIM_Shutdown();
}

/// 回归测试（2026-08-01 新游戏生成问题 2）：
/// ModifyCell 写入 updated_cells 后，经一帧 elapsed>0（NewGameFrame 0.2s）处理，
/// cells（BeginSave 序列化的缓冲区）必须同步反映该写入。
///
/// 背景：sim 线程帧循环的 elapsed>0 分支此前缺失 updated→cells 同步
/// （update_data 为 C2 空桩），导致 worldgen 第 498 步模板 ModifyCell
/// （氧石/中子质基座/清空空间）永不进入存档 → POI 被自然方块掩埋。
#[test]
fn modify_cell_reaches_cells_after_elapsed_frame() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();
    do_full_init(4, 4);

    // ModifyCell 消息（28B，对照 sim_events::CellModification 的 repr(C) 布局）
    let mut msg = Vec::with_capacity(28);
    msg.extend_from_slice(&0i32.to_le_bytes()); // game_cell = 0 → 内部格 (0%4+1)+(0/4+1)*6 = 7
    msg.extend_from_slice(&(-1i32).to_le_bytes()); // callback_idx
    msg.extend_from_slice(&350.0f32.to_le_bytes()); // temperature
    msg.extend_from_slice(&500.0f32.to_le_bytes()); // mass
    msg.extend_from_slice(&0i32.to_le_bytes()); // disease_count
    msg.extend_from_slice(&10u16.to_le_bytes()); // element_idx
    msg.extend_from_slice(&1u8.to_le_bytes()); // replace_mode = 1 (Replace)
    msg.extend_from_slice(&0xFFu8.to_le_bytes()); // disease_idx
    msg.extend_from_slice(&1u8.to_le_bytes()); // flags (addSubType)
    msg.extend_from_slice(&[0u8; 3]); // pad
    let result = SIM_HandleMessage(
        MessageType::ModifyCell as c_int,
        msg.len() as c_int,
        msg.as_ptr() as *mut c_schar,
    );
    // ModifyCell 分支只入队不设置返回值（result 初始为 null）——返回 null 是正常行为，
    // 但 handled 应为 true（frame_manager.handle_message 的 MSG_MODIFY_CELL 分支）。
    let _ = result;

    // NewGameFrame（28B）：elapsed=0.2s → sim 线程走 elapsed>0 分支
    let mut frame_msg = Vec::with_capacity(28);
    frame_msg.extend_from_slice(&0.2f32.to_le_bytes()); // elapsedSeconds
    frame_msg.extend_from_slice(&0i32.to_le_bytes()); // min_x
    frame_msg.extend_from_slice(&0i32.to_le_bytes()); // min_y
    frame_msg.extend_from_slice(&0i32.to_le_bytes()); // max_x
    frame_msg.extend_from_slice(&0i32.to_le_bytes()); // max_y
    frame_msg.extend_from_slice(&0.0f32.to_le_bytes()); // sunlight
    frame_msg.extend_from_slice(&0.0f32.to_le_bytes()); // cosmic_rad
    let result2 = SIM_HandleMessage(
        MessageType::SimFrameManager_NewGameFrame as c_int,
        frame_msg.len() as c_int,
        frame_msg.as_ptr() as *mut c_schar,
    );
    // NewGameFrame 同样只设置帧状态，返回值可能为 null
    let _ = result2;

    // PrepareGameData 内含 game_sync：唤醒 sim 线程处理该帧并交换双缓冲。
    // 发 2 次：第 1 次唤醒 sim 线程处理 ModifyCell 帧；第 2 次确保该帧已处理完
    //（game_sync 阻塞等待 sim 线程就绪后返回）。
    let gdu = SIM_HandleMessage(
        MessageType::PrepareGameData as c_int,
        0,
        std::ptr::null_mut(),
    );
    assert!(!gdu.is_null(), "first PrepareGameData should return GDU");
    let gdu2 = SIM_HandleMessage(
        MessageType::PrepareGameData as c_int,
        0,
        std::ptr::null_mut(),
    );
    assert!(!gdu2.is_null(), "second PrepareGameData should return GDU");

    // cells（BeginSave 序列化的缓冲区）应已同步 ModifyCell 的写入
    let ptr = SimDLL::globals::G_SIM_DATA.lock().0;
    assert!(!ptr.is_null(), "gSimData should exist after Start");
    unsafe {
        let sd = &*ptr;
        let cells = &*sd.cells.ptr;
        let got = cells.element_idx.get(7);
        assert_eq!(
            got, 10,
            "cells[7] 应反映 ModifyCell 写入（帧同步缺失时仍为初始真空）"
        );
    }

    SIM_Shutdown();
}

/// 回归测试（2026-08-01 液体纹理）：update_data 帧后 SimData.flow 已分配且可写。
/// 挂接（任务 9）：update_data → update_liquids + 压力循环写 flow，copy_sim_data_to_game
/// 尾部调 3 个纹理函数读 flow。此测试跑完整 init 流程，验证挂接不崩。
#[test]
fn update_data_frame_writes_flow_buffer() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();
    do_full_init(4, 4);
    std::thread::sleep(std::time::Duration::from_millis(50));
    unsafe {
        let ptr = SimDLL::globals::G_SIM_DATA.lock().0;
        assert!(!ptr.is_null());
        let sd = &*ptr;
        assert!(!sd.flow.ptr.is_null(), "flow 应已分配");
    }
    SIM_Shutdown();
}

/// 回归测试（2026-08-05，孢子兰病菌发射）：CellDiseaseModification 消息
/// 必须经帧生命周期（handle_message emplace → NewGameFrame 入队 → sim 线程
/// process_frame 步骤 8）把病菌写入 updated_cells——C# DiseaseDropper（EvilFlower
/// 等）走 SimMessages.ModifyDiseaseOnCell 就是这条链路。
#[test]
fn cell_disease_modification_reaches_updated_cells() {
    let _guard = SIM_THREAD_LOCK.lock().unwrap();
    do_full_init(4, 4);

    // CellDiseaseModification 12B：gameCell=0 → simCell=7（width=6, game_w=4）
    let mut msg = Vec::with_capacity(12);
    msg.extend_from_slice(&0i32.to_le_bytes()); // cellIdx
    msg.push(3u8); // diseaseIdx
    msg.push(0u8);
    msg.push(0u8);
    msg.push(0u8);
    msg.extend_from_slice(&1000i32.to_le_bytes()); // diseaseCount
    let result = SIM_HandleMessage(
        MessageType::CellDiseaseModification as c_int,
        msg.len() as c_int,
        msg.as_ptr() as *mut c_schar,
    );
    let _ = result;

    // NewGameFrame（elapsed=0.2）→ 入队 → PrepareGameData 唤醒 sim 线程处理
    let mut frame_msg = Vec::with_capacity(28);
    frame_msg.extend_from_slice(&0.2f32.to_le_bytes());
    frame_msg.extend_from_slice(&0i32.to_le_bytes());
    frame_msg.extend_from_slice(&0i32.to_le_bytes());
    frame_msg.extend_from_slice(&0i32.to_le_bytes());
    frame_msg.extend_from_slice(&0i32.to_le_bytes());
    frame_msg.extend_from_slice(&0.0f32.to_le_bytes());
    frame_msg.extend_from_slice(&0.0f32.to_le_bytes());
    let _ = SIM_HandleMessage(
        MessageType::SimFrameManager_NewGameFrame as c_int,
        frame_msg.len() as c_int,
        frame_msg.as_ptr() as *mut c_schar,
    );
    let gdu = SIM_HandleMessage(MessageType::PrepareGameData as c_int, 0, std::ptr::null_mut());
    assert!(!gdu.is_null(), "first PrepareGameData should return GDU");
    let gdu2 = SIM_HandleMessage(MessageType::PrepareGameData as c_int, 0, std::ptr::null_mut());
    assert!(!gdu2.is_null(), "second PrepareGameData should return GDU");

    // updated_cells[7] 应已写入病菌（1000 @ idx 3）
    let ptr = SimDLL::globals::G_SIM_DATA.lock().0;
    assert!(!ptr.is_null());
    unsafe {
        let sd = &*ptr;
        let updated = &*sd.updated_cells.ptr;
        assert_eq!(
            updated.disease_count.get(7),
            1000,
            "CellDiseaseModification 应写入 updated_cells[7]"
        );
        assert_eq!(updated.disease_idx.get(7), 3);
    }

    SIM_Shutdown();
}
