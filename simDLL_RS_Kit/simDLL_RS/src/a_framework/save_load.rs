//! 存档加载/保存 + 全局单例访问。
//!
//! 对照源码 02_save_load.c。
//! A3 实现 10 个 save_load handler + 4 个辅助函数。
//!
//! **任务 1**：全局单例访问 + CleanUp + handler 桩。
//! **任务 2**：AllocateCells + Load + CellRead（存档读取核心）。

use crate::a_framework::buffer::{BinaryBufferReader, BufferError};
use crate::a_framework::game_data::GameData;
use crate::a_framework::game_data_update::GameDataUpdate;
use crate::a_framework::sim_data::{CellSOA, SimData, WorldOffsetData};
use crate::a_framework::stl_shim::UniquePtr;
use crate::b_elements::disease::Disease;
use crate::b_elements::elements_table;
use crate::globals;
use std::ffi::c_void;

// ===== 辅助函数 =====

/// CleanUp — 清理所有全局单例。
/// 对照源码 02_save_load.c L619-665。
///
/// 释放顺序：SaveBuffer → gSim → gDisease → gSimData。
/// FrameSync 不释放（OnceCell 静态生命周期）。
/// GameDataUpdate 不释放（OnceCell 静态生命周期）。
pub fn clean_up() {
    // 释放 SaveBuffer
    // SaveBuffer 存储为 Box<Vec<u8>>（BeginSave 分配）。
    // 对照源码 02_save_load.c L619-665：CleanUp 释放 SaveBuffer 调用析构函数。
    {
        let mut save_buffer = globals::G_SAVE_BUFFER.lock();
        if !save_buffer.0.is_null() {
            unsafe {
                // SaveBuffer 是 Box<Vec<u8>>（BeginSave 用 Box::into_raw 创建）
                let _ = Box::from_raw(save_buffer.0 as *mut Vec<u8>);
            }
            save_buffer.0 = std::ptr::null_mut();
        }
    }

    // 释放 gSim
    {
        let mut sim = globals::G_SIM.lock();
        if !sim.0.is_null() {
            unsafe {
                // 对照原版 02_save_load.c L632-645（CleanUp）：
                //   1. 若 sim 线程可 join → 先 FrameSync::GameSync 放行阻塞在
                //      SimSync 的 sim 线程（交换双缓冲并唤醒）
                //   2. Thread::Join 等待 sim 线程真正退出
                //   3. 最后才 delete gSim
                // 此前直接 Box::from_raw → sim 线程仍可能通过裸指针访问
                // gSim+0x50（frame_manager）→ use-after-free / 堆损坏。
                let thread_running = crate::a_framework::sim_thread::G_SIM_THREAD
                    .get()
                    .map(|h| h.lock().is_some())
                    .unwrap_or(false);
                if thread_running {
                    let fs_ptr =
                        crate::a_framework::frame_sync::FrameSync::get_ptr();
                    if !fs_ptr.is_null() {
                        // GameSync 会等待 m_sim_ready_to_swap（sim 到达 SimSync），
                        // 交换双缓冲后唤醒 sim 线程（与原版 GameSync 一致）。
                        crate::a_framework::frame_sync::FrameSync::game_sync(fs_ptr);
                    }
                }
                crate::a_framework::sim_thread::join_sim_thread();
                let _ = Box::from_raw(sim.0);
            }
            sim.0 = std::ptr::null_mut();
        }
    }

    // 对照原版 02_save_load.c L645：CleanUp 置 gGameMessageHandler = 0
    // （崩溃处理器回调已卸载 C# 委托）。
    *globals::G_GAME_MESSAGE_HANDLER.lock() = None;

    // 释放 gDisease
    {
        let mut disease = globals::G_DISEASE.lock();
        if !disease.0.is_null() {
            unsafe {
                let _ = Box::from_raw(disease.0);
            }
            disease.0 = std::ptr::null_mut();
        }
    }

    // 释放 gSimData
    {
        let mut sim_data = globals::G_SIM_DATA.lock();
        if !sim_data.0.is_null() {
            unsafe {
                let _ = Box::from_raw(sim_data.0);
            }
            sim_data.0 = std::ptr::null_mut();
        }
    }

    // 对照源码 02_save_load.c L653：CleanUp 调用 FrameSync::clear() 复位
    // m_initialized/m_sim_data/m_game_data。此前缺失 → 二次加载/世界生成时
    // initGameData 的"已初始化"判断残留 → 缓冲不重建 → 越界写（见 frame_sync.rs）。
    {
        let fs_ptr = crate::a_framework::frame_sync::FrameSync::get_ptr();
        if !fs_ptr.is_null() {
            unsafe { (*fs_ptr).clear(); }
        }
    }

    tracing::info!("CleanUp done");
}

// ===== save_load handler（任务 1 桩，后续任务填充真实实现）=====

/// AllocateCells — 分配 SimData。
/// 对照源码 02_save_load.c L259-293。
///
/// 数据格式：width(int) + height(int) + flag1(bool) + flag2(bool)
/// 创建 SimData（width+2, height+2，含边界）并替换全局 gSimData。
/// 返回 gSimData 指针。
pub fn handle_allocate_cells(reader: &mut BinaryBufferReader) -> *mut c_void {
    let width = reader.read_int().unwrap_or(0);
    let height = reader.read_int().unwrap_or(0);
    let flag1 = reader.read_bool().unwrap_or(false);
    let flag2 = reader.read_bool().unwrap_or(false);

    // 获取当前时间作为随机种子（对照源码 _time64）
    let random_seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);

    // 创建 SimData（width+2, height+2，含边界）
    // 对照源码 L282-283: SimData::SimData(this, width+2, height+2, time, flag1, flag2)
    let sim_data = Box::new(SimData::new_for_allocate(
        width + 2,
        height + 2,
        random_seed,
        flag1,
        flag2,
    ));
    let sim_data_ptr = Box::into_raw(sim_data);

    // 替换全局 gSimData（释放旧的）
    let mut global_sim_data = globals::G_SIM_DATA.lock();
    if !global_sim_data.0.is_null() {
        unsafe {
            let _ = Box::from_raw(global_sim_data.0);
        }
    }
    global_sim_data.0 = sim_data_ptr;

    // 新世界分配 = 存档切换：上一存档的 RadiationEmitter 记录全部作废。
    // 原版注册表在 Sim 内、跨存档靠 Remove 消息清理；本项目为全局静态，
    // 显式清空避免"上个存档辐射残留"（旧发射器继续向新世界发射）。
    crate::c_simulation::radiation_emitter::clear_radiation_emitters();
    crate::c_simulation::disease_component::clear_disease_emitters();
    // 同类全局静态注册表（原版均随 SimData 重建）：ElementEmitter 漏清会携带
    // 上一局 callbackManager 的旧句柄 → EMBARK 首帧 callback_info 越界崩溃；
    // ElementChunk / BuildingTemperature / BuildingToBuilding / DiseaseConsumer
    // 同为"全局静态 vs 原版随 SimData"模式，一并清空防残留。
    crate::c_simulation::element_emitter::clear_element_emitters();
    crate::c_simulation::element_chunk::clear_element_chunks();
    crate::c_simulation::building_temperature::clear_building_temperature();
    crate::c_simulation::building_to_building::clear_building_to_building();
    crate::c_simulation::disease_component::clear_disease_consumers();

    tracing::info!(width, height, flag1, flag2, "AllocateCells done");
    sim_data_ptr as *mut c_void
}

/// CellRead — 读取单个 cell 数据。
/// 对照源码 02_save_load.c L519-615。
///
/// 每个 cell 读取：element_hash(int) + temperature(float) + mass(float) [+ radiation(float) if version>13]
/// 处理元素 hash 替换、NaN 修复、真空元素替换。
fn cell_read(
    cells: &mut CellSOA,
    cell_idx: usize,
    reader: &mut BinaryBufferReader,
    version: i32,
) -> Result<(), BufferError> {
    // L529: 读取元素 hash
    let mut element_hash = reader.read_uint()?;

    // L530-535: hash 替换（旧存档兼容）
    if element_hash == 0x0280cf79 {
        element_hash = 0x987dac06;
    } else if element_hash == 0x2391c22b {
        element_hash = 0x4c76ae31;
    }

    // L536: GetElementIndex(hash)
    let elem_idx = elements_table::get_element_index_pub(element_hash);

    // L539: cells.element_idx[cell_idx] = elem_idx
    cells.element_idx.set(cell_idx, elem_idx);

    // L542: 读取 temperature
    let mut temperature = reader.read_float()?;
    // L545-551: if !_finite(temperature) → 293.0 (0x43928000，2026-08-07 修正：此前误译成 300.0)
    if !temperature.is_finite() {
        temperature = 293.0f32;
    }
    cells.temperature.set(cell_idx, temperature);

    // L554: 读取 mass
    let mut mass = reader.read_float()?;
    // L557-563: if !_finite(mass) → 100.0 (0x42c80000)
    if !mass.is_finite() {
        mass = 100.0f32;
    }
    cells.mass.set(cell_idx, mass);

    // L564-578: if version > 13, 读取 radiation
    if version > 0xd {
        let mut radiation = reader.read_float()?;
        if !radiation.is_finite() {
            radiation = 0.0f32;
        }
        cells.radiation.set(cell_idx, radiation);
    }

    // L580-606: if element_idx == 0xffff (未找到元素), 替换为真空元素
    let current_elem = cells.element_idx.get(cell_idx);
    if current_elem == 0xffff {
        let vacuum_idx = elements_table::get_element_index_pub(0x2d39bf75);
        cells.element_idx.set(cell_idx, vacuum_idx);
        cells.mass.set(cell_idx, 0.0);
        cells.temperature.set(cell_idx, 0.0);
        cells.radiation.set(cell_idx, 0.0);
    }

    Ok(())
}

/// Load — 加载存档。
/// 对照源码 02_save_load.c L7-207。
///
/// 存档头格式：
/// - 8B magic "SIMSAVE\0"
/// - 4B version (int)
/// - 4B width (int)
/// - 4B height (int)
/// - if version > 13: 4B x_offset (int) + 4B y_offset (int)（世界在簇网格中的列/行）
/// - if version > 12: 1B flag (byte) → ApplySaveSettings
/// - cell 数据（width × height 个 cell，stride = gSimData.width）
/// - if version > 8: disease 数据
/// - if version > 14: backwall 数据
/// - if version < 15: skip width × height × 4 字节
pub fn handle_load(reader: &mut BinaryBufferReader) -> *mut c_void {
    // L32: 检查 magic "SIMSAVE\0"
    let magic = reader.read_bytes(8).unwrap_or_default();
    if magic != b"SIMSAVE\0" {
        tracing::error!("Load: invalid magic");
        return std::ptr::null_mut();
    }

    // L34: 读取 version
    let version = reader.read_int().unwrap_or(0);
    if version >= 0x10 {
        tracing::error!(version, "Load: unsupported version (>= 16)");
        return std::ptr::null_mut();
    }

    // L36-37: 读取 save_width, save_height
    let save_width = reader.read_int().unwrap_or(0);
    let save_height = reader.read_int().unwrap_or(0);

    // L38-42: if version > 13, 读取 x_offset + y_offset
    // 头字段 = 世界在簇网格中的 (列, 行) 坐标（C# SIM_BeginSave 直接写
    // data.world.offset.x/y，BestFit 摆放位置；已用真实 worldgen 存档核对：
    // 例 TerraMoonlet(0,0)、TundraMoonlet(162,176)、RegolithMoonlet(360,155)）。
    // 反编译的 Load 只读 x 作线性起点、丢弃 y——但原版多星图各世界内容正常，
    // 且 y>0 世界若按 x-only 会与第 0 行世界重叠（实测：WarpOilySwamp 被
    // RegolithMoonlet 覆盖 → "内容错误/中子质边界消失"）。正确语义 = y*stride+x。
    let mut x_offset: i32 = 0;
    let mut y_offset: i32 = 0;
    if version > 0xd {
        x_offset = reader.read_int().unwrap_or(0);
        y_offset = reader.read_int().unwrap_or(0);
    }

    // L43-46: if version > 12, 读取 flag
    let mut flag: u8 = 0;
    if version > 0xc {
        flag = reader.read_byte().unwrap_or(0);
    }
    tracing::info!(
        version,
        save_width,
        save_height,
        x_offset,
        y_offset,
        remaining = reader.remaining(),
        "Load: header parsed"
    );

    // L47: ApplySaveSettings
    let sim_data_ptr = globals::G_SIM_DATA.lock().0;
    if sim_data_ptr.is_null() {
        tracing::error!("Load: gSimData is null");
        return std::ptr::null_mut();
    }

    let sim_data = unsafe { &mut *sim_data_ptr };
    sim_data.apply_save_settings(flag);

    let stride = sim_data.width; // 含边界的宽度
    let void_idx = sim_data.void_element_idx;
    let vacuum_idx = sim_data.vacuum_element_idx;
    let start = (y_offset as i64) * (stride as i64) + (x_offset as i64);

    // L53-114: cell 读取循环
    // 对照源码 L61: CellRead(*(CellSOA **)(gSimData + 0x28), ...) — 读取到 updated_cells
    if save_height > 0 && save_width > 0 {
        let cells_ptr = sim_data.updated_cells.ptr;
        if cells_ptr.is_null() {
            tracing::error!("Load: updated_cells is null");
            return std::ptr::null_mut();
        }
        let cells = unsafe { &mut *cells_ptr };

        for row in 0..save_height as i64 {
            let row_start = start + row * (stride as i64);
            for col in 0..save_width as i64 {
                let cell_idx = (row_start + col) as usize;

                // CellRead
                if let Err(e) = cell_read(cells, cell_idx, reader, version) {
                    tracing::error!(?e, cell_idx, "Load: CellRead failed");
                    return std::ptr::null_mut();
                }

                // L62-68: if temperature <= 0 → 293.0（0x43928000，2026-08-07 修正）
                let temp = cells.temperature.get(cell_idx);
                if temp <= 0.0 {
                    cells.temperature.set(cell_idx, 293.0);
                }

                // L69-75: if radiation <= 0 → 0
                let rad = cells.radiation.get(cell_idx);
                if rad <= 0.0 {
                    cells.radiation.set(cell_idx, 0.0);
                }

                // L76-103: if element == void or vacuum → clear cell
                let elem = cells.element_idx.get(cell_idx);
                if elem == void_idx || elem == vacuum_idx {
                    cells.temperature.set(cell_idx, 0.0);
                    cells.mass.set(cell_idx, 0.0);
                    cells.radiation.set(cell_idx, 0.0);
                    cells.disease_count.set(cell_idx, 0);
                }

                // L104: DoLoadTimeStateTransition
                crate::c2_physics::temperature::do_load_time_state_transition(sim_data, cell_idx);
            }
        }
    }

    // L116-157: if version > 8, 读取 disease 数据（同样读取到 updated_cells）
    if version > 8 && save_height > 0 && save_width > 0 {
        let cells_ptr = sim_data.updated_cells.ptr;
        let cells = unsafe { &mut *cells_ptr };
        let disease_ptr = globals::G_DISEASE.lock().0;

        for row in 0..save_height as i64 {
            let row_start = start + row * (stride as i64);
            for col in 0..save_width as i64 {
                let cell_idx = (row_start + col) as usize;

                // 读取 disease_hash
                let disease_hash = match reader.read_uint() {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::error!(?e, "Load: disease read failed");
                        return std::ptr::null_mut();
                    }
                };

                // 2026-08-04 修复（太空删除根因）：原版 Load 的 disease 段为 8B/格
                // （4B disease_hash + 4B disease_count，见 02_save_load.c L133-147）。
                // 此前漏读 disease_count → reader 每格落后 4B → 背墙段整体偏移
                // 4B/格 → 把 disease_count（无病菌时为 0）当成背墙元素 hash →
                // 全图背墙读成 0xFFFF → 太空删除判定 backwall==真空 恒失败。
                let disease_count = match reader.read_int() {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!(?e, "Load: disease count read failed");
                        return std::ptr::null_mut();
                    }
                };

                // L132-137: 获取 disease_idx
                let disease_idx = if disease_hash == 0 {
                    0xffu8
                } else if !disease_ptr.is_null() {
                    let disease = unsafe { &*disease_ptr };
                    disease.get_disease_index(disease_hash)
                } else {
                    0xffu8
                };

                cells.disease_idx.set(cell_idx, disease_idx);
                cells.disease_count.set(cell_idx, disease_count);

                // L143-147: if disease_idx == 0xff → disease_count = 0
                if disease_idx == 0xff {
                    cells.disease_count.set(cell_idx, 0);
                }
            }
        }
    }

    // L158-192: if version > 14, 读取 backwall 数据
    if version > 0xe && save_height > 0 && save_width > 0 {
        let backwall_ptr = sim_data.backwall.ptr;
        if !backwall_ptr.is_null() {
            let backwall = unsafe { &mut *backwall_ptr };

            for row in 0..save_height as i64 {
                let row_start = start + row * (stride as i64);
                for col in 0..save_width as i64 {
                    let cell_idx = (row_start + col) as usize;

                    // 读取 element_hash
                    let elem_hash = match reader.read_uint() {
                        Ok(h) => h,
                        Err(e) => {
                            tracing::error!(?e, "Load: backwall element read failed");
                            return std::ptr::null_mut();
                        }
                    };

                    // L166: GetElementIndex
                    let bw_elem = elements_table::get_element_index_pub(elem_hash);
                    backwall.element_idx.set(cell_idx, bw_elem);

                    // L177-178: 读取 mass
                    let bw_mass = match reader.read_float() {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::error!(?e, "Load: backwall mass read failed");
                            return std::ptr::null_mut();
                        }
                    };
                    backwall.mass.set(cell_idx, bw_mass);

                    // L181-182: 读取 temperature
                    let bw_temp = match reader.read_float() {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::error!(?e, "Load: backwall temp read failed");
                            return std::ptr::null_mut();
                        }
                    };
                    backwall.temperature.set(cell_idx, bw_temp);
                }
            }
        }
    }

    // L193-195: if version - 8U < 7（即 8..14）, skip width × height × 4 字节。
    // 2026-08-04 复核修正：此前 `version < 0xf` 会连 ≤7 的旧版本也跳过（原版不会）。
    if (8..0xf).contains(&version) && save_height > 0 && save_width > 0 {
        let skip_bytes = (save_height as usize) * (save_width as usize) * 4;
        let _ = reader.skip(skip_bytes);
    }

    // L196-198: if version < 11, 设置 init_settle_thermal_boundaries = true
    if version < 0xb {
        sim_data.init_settle_thermal_boundaries = true;
    }

    crate::c_simulation::sim_data_ops::derive_natural_solid_strength(
        sim_data,
        crate::d1_activity::full_grid_bounds(sim_data),
    );

    tracing::info!(version, save_width, save_height, x_offset, y_offset, "Load done");
    sim_data_ptr as *mut c_void
}

// ===== PrepareGameDataUpdate（任务 5）=====

/// PrepareGameDataUpdate — 填充 GameDataUpdate 的 SOA 指针。
/// 对照源码 02_save_load.c L688-1059。
///
/// **关键约束**（项目 memory）：
/// PrepareGameData 必须返回真实 GDU 指针。
/// Game.StepTheSim 把返回值强转为 `GameDataUpdate*` 并立即解引用 `ptr->elementIdx`。
/// 0x1 哨兵 → null-page 读取 → Mono 报 NRE → SIM_Shutdown。
///
/// 逻辑：
/// 1. numFramesProcessed = game_data.num_frames_processed
/// 2. 从 cells (CellSOA) 提取 9 个 SOA 指针（element_idx/temperature/mass/properties/
///    insulation/strength_info/disease_idx/disease_count/radiation），空 vector → null
/// 3. 从 backwalls (BackwallSOA) 提取 3 个 SOA 指针（element_idx/mass/temperature）
/// 4. 提取 29 对 num+ptr（每个事件 vector），空 vector → ptr=null
/// 5. 提取 6 个 property texture 指针（accumulated_flow/property_texture_*）
/// 6. 返回 G_GAME_DATA_UPDATE 的地址
pub fn prepare_game_data_update(game_data: &GameData) -> *mut c_void {
    // G_GAME_DATA_UPDATE 是 OnceCell<Mutex<GameDataUpdate>>。
    // get_or_init 返回 &Mutex<GameDataUpdate>，lock() 返回 MutexGuard。
    // MutexGuard 的内存位置固定（Mutex 是 static），返回的指针在 guard drop 后仍有效。
    // 安全性：PrepareGameDataUpdate 仅在游戏主线程调用，C# 端只读访问返回的指针。
    let gdu_mutex = globals::G_GAME_DATA_UPDATE.get_or_init(|| parking_lot::Mutex::new(GameDataUpdate::default()));
    let mut gdu = gdu_mutex.lock();

    // L702: numFramesProcessed
    gdu.num_frames_processed = game_data.num_frames_processed;

    // ===== 2. 从 cells 提取 9 个 SOA 指针 =====
    let cells_ptr = game_data.cells.ptr;
    tracing::info!(
        cells_ptr_null = cells_ptr.is_null(),
        gd_width = game_data.width,
        gd_height = game_data.height,
        "PrepareGameDataUpdate: game_data state"
    );
    if !cells_ptr.is_null() {
        let cells = unsafe { &*cells_ptr };

        // 诊断：记录 cells 各 vector 的长度
        tracing::info!(
            elem_len = cells.element_idx.len(),
            temp_len = cells.temperature.len(),
            mass_len = cells.mass.len(),
            "PrepareGameDataUpdate: cells vector lengths"
        );

        // L705-709: elementIdx（如果 len==0 则 null）
        gdu.element_idx = if cells.element_idx.is_empty() {
            std::ptr::null()
        } else {
            cells.element_idx.as_ptr()
        };

        // L710-714: temperature
        gdu.temperature = if cells.temperature.is_empty() {
            std::ptr::null()
        } else {
            cells.temperature.as_ptr()
        };

        // L715-719: radiation（源码在 mass 之前赋值，但 GDU 字段顺序由 C# 决定）
        gdu.radiation = if cells.radiation.is_empty() {
            std::ptr::null()
        } else {
            cells.radiation.as_ptr()
        };

        // L720-724: mass
        gdu.mass = if cells.mass.is_empty() {
            std::ptr::null()
        } else {
            cells.mass.as_ptr()
        };

        // L725-729: properties
        gdu.properties = if cells.properties.is_empty() {
            std::ptr::null()
        } else {
            cells.properties.as_ptr()
        };

        // L730-734: insulation
        gdu.insulation = if cells.insulation.is_empty() {
            std::ptr::null()
        } else {
            cells.insulation.as_ptr()
        };

        // L735-739: strengthInfo
        gdu.strength_info = if cells.strength_info.is_empty() {
            std::ptr::null()
        } else {
            cells.strength_info.as_ptr()
        };

        // L740-744: diseaseIdx
        gdu.disease_idx = if cells.disease_idx.is_empty() {
            std::ptr::null()
        } else {
            cells.disease_idx.as_ptr()
        };

        // L745-749: diseaseCount（源码用 &callbackIdx 取地址，实际就是 begin）
        gdu.disease_count = if cells.disease_count.is_empty() {
            std::ptr::null()
        } else {
            cells.disease_count.as_ptr()
        };
    } else {
        // cells 为 null，所有 SOA 指针置 null
        gdu.element_idx = std::ptr::null();
        gdu.temperature = std::ptr::null();
        gdu.radiation = std::ptr::null();
        gdu.mass = std::ptr::null();
        gdu.properties = std::ptr::null();
        gdu.insulation = std::ptr::null();
        gdu.strength_info = std::ptr::null();
        gdu.disease_idx = std::ptr::null();
        gdu.disease_count = std::ptr::null();
    }

    // ===== 3. 从 backwalls 提取 3 个 SOA 指针 =====
    let backwalls_ptr = game_data.backwalls.ptr;
    if !backwalls_ptr.is_null() {
        let backwalls = unsafe { &*backwalls_ptr };

        // L751-755: backwallElementIdx
        gdu.backwall_element_idx = if backwalls.element_idx.is_empty() {
            std::ptr::null()
        } else {
            backwalls.element_idx.as_ptr()
        };

        // L756-760: backwallMasses
        gdu.backwall_masses = if backwalls.mass.is_empty() {
            std::ptr::null()
        } else {
            backwalls.mass.as_ptr()
        };

        // L761-766: backwallTemperatures
        gdu.backwall_temperatures = if backwalls.temperature.is_empty() {
            std::ptr::null()
        } else {
            backwalls.temperature.as_ptr()
        };
    } else {
        gdu.backwall_element_idx = std::ptr::null();
        gdu.backwall_masses = std::ptr::null();
        gdu.backwall_temperatures = std::ptr::null();
    }

    // ===== 4. 提取 29 对 num+ptr =====
    // 每对：num = vector.len()，ptr = vector 为空 ? null : vector.as_ptr()
    // 顺序严格对照源码 L767-1050。

    // L767-774: solidInfo (SolidInfo, 8B, >>3)
    gdu.num_solid_info = game_data.solid_info.len() as i32;
    gdu.solid_info = if game_data.solid_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.solid_info.as_ptr()
    };

    // L775-782: liquidChangeInfo (LiquidChangeInfo, 4B, >>2)
    gdu.num_liquid_change_info = game_data.liquid_change_info.len() as i32;
    gdu.liquid_change_info = if game_data.liquid_change_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.liquid_change_info.as_ptr()
    };

    // L802-810: solidSubstanceChangeInfo (SolidSubstanceChangeInfo, 4B, >>2)
    gdu.num_solid_substance_change_info = game_data.solid_substance_change_info.len() as i32;
    gdu.solid_substance_change_info = if game_data.solid_substance_change_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.solid_substance_change_info.as_ptr()
    };

    // L811-819: substanceChangeInfo (SubstanceChangeInfo, 8B, >>3)
    gdu.num_substance_change_info = game_data.substance_change_info.len() as i32;
    gdu.substance_change_info = if game_data.substance_change_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.substance_change_info.as_ptr()
    };

    // L820-827: callbackInfo (CallbackInfo, 4B, >>2)
    gdu.num_callback_info = game_data.callback_info.len() as i32;
    // 诊断：记录本帧 callbackInfo 总量（C# StepTheSim 遍历，越界崩溃排查用）
    tracing::info!(
        count = gdu.num_callback_info,
        "PrepareGameData callback_info count"
    );
    gdu.callback_info = if game_data.callback_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.callback_info.as_ptr()
    };

    // L828-838: spawnFallingLiquidInfo (SpawnFallingLiquidInfo, 20B=0x14, /0x14)
    gdu.num_spawn_falling_liquid_info = game_data.spawn_falling_liquid_info.len() as i32;
    gdu.spawn_falling_liquid_info = if game_data.spawn_falling_liquid_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.spawn_falling_liquid_info.as_ptr()
    };

    // L839-847: digInfo (SpawnOreInfo, 20B=0x14, /0x14)
    gdu.num_dig_info = game_data.dig_info.len() as i32;
    gdu.dig_info = if game_data.dig_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.dig_info.as_ptr()
    };

    // L848-856: spawnOreInfo (SpawnOreInfo, 20B=0x14, /0x14)
    gdu.num_spawn_ore_info = game_data.spawn_ore_info.len() as i32;
    gdu.spawn_ore_info = if game_data.spawn_ore_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.spawn_ore_info.as_ptr()
    };

    // L857-865: spawnFXInfo (SpawnFXInfo, 12B=0xc, /0xc)
    gdu.num_spawn_fx_info = game_data.spawn_fx_info.len() as i32;
    gdu.spawn_fx_info = if game_data.spawn_fx_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.spawn_fx_info.as_ptr()
    };

    // L866-875: unstableCellInfo (UnstableCellInfo, 20B=0x14, /0x14)
    gdu.num_unstable_cell_info = game_data.unstable_cell_info.len() as i32;
    gdu.unstable_cell_info = if game_data.unstable_cell_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.unstable_cell_info.as_ptr()
    };

    // L876-883: worldDamageInfo (WorldDamageInfo, 8B, >>3)
    gdu.num_world_damage_info = game_data.world_damage_info.len() as i32;
    gdu.world_damage_info = if game_data.world_damage_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.world_damage_info.as_ptr()
    };

    // L884-892: buildingTemperatureInfo (BuildingTemperatureInfo, 8B, >>3)
    gdu.num_building_temperature_info = game_data.building_temperature_info.len() as i32;
    gdu.building_temperature_info = if game_data.building_temperature_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.building_temperature_info.as_ptr()
    };

    // L893-903: massConsumedCallbacks (MassConsumedCallback, 20B=0x14, /0x14)
    gdu.num_mass_consumed_callbacks = game_data.mass_consumed_callbacks.len() as i32;
    gdu.mass_consumed_callbacks = if game_data.mass_consumed_callbacks.is_empty() {
        std::ptr::null()
    } else {
        game_data.mass_consumed_callbacks.as_ptr()
    };

    // L904-914: massEmittedCallbacks (MassEmittedCallback, 20B=0x14, /0x14)
    gdu.num_mass_emitted_callbacks = game_data.mass_emitted_callbacks.len() as i32;
    gdu.mass_emitted_callbacks = if game_data.mass_emitted_callbacks.is_empty() {
        std::ptr::null()
    } else {
        game_data.mass_emitted_callbacks.as_ptr()
    };

    // L915-925: diseaseConsumedCallbacks (DiseaseConsumedCallback, 12B=0xc, /0xc)
    gdu.num_disease_consumed_callbacks = game_data.disease_consumed_callbacks.len() as i32;
    gdu.disease_consumed_callbacks = if game_data.disease_consumed_callbacks.is_empty() {
        std::ptr::null()
    } else {
        game_data.disease_consumed_callbacks.as_ptr()
    };

    // L926-934: componentStateChangedMessages (ComponentStateChangedMessage, 8B, >>3)
    gdu.num_component_state_changed_messages = game_data.component_state_changed_messages.len() as i32;
    gdu.component_state_changed_messages = if game_data.component_state_changed_messages.is_empty() {
        std::ptr::null()
    } else {
        game_data.component_state_changed_messages.as_ptr()
    };

    // L935-944: removedMassEntries (ConsumedMassInfo, 20B=0x14, /0x14) — 来自 consumedMassInfo
    gdu.num_removed_mass_entries = game_data.consumed_mass_info.len() as i32;
    gdu.removed_mass_entries = if game_data.consumed_mass_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.consumed_mass_info.as_ptr()
    };

    // L945-952: emittedMassEntries (EmittedMassInfo, 16B, >>4)
    gdu.num_emitted_mass_entries = game_data.emitted_mass_info.len() as i32;
    gdu.emitted_mass_entries = if game_data.emitted_mass_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.emitted_mass_info.as_ptr()
    };

    // L953-960: elementChunkInfos (ElementChunkInfo, 8B, >>3) — 来自 elementChunkInfo
    gdu.num_element_chunk_infos = game_data.element_chunk_info.len() as i32;
    gdu.element_chunk_infos = if game_data.element_chunk_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.element_chunk_info.as_ptr()
    };

    // L961-969: elementChunkMeltedInfos (MeltedInfo, 4B, >>2) — 来自 elementChunkMeltedInfo
    gdu.num_element_chunk_melted_infos = game_data.element_chunk_melted_info.len() as i32;
    gdu.element_chunk_melted_infos = if game_data.element_chunk_melted_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.element_chunk_melted_info.as_ptr()
    };

    // L970-978: buildingOverheatInfos (MeltedInfo, 4B, >>2) — 来自 buildingOverheatInfo
    gdu.num_building_overheat_infos = game_data.building_overheat_info.len() as i32;
    gdu.building_overheat_infos = if game_data.building_overheat_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.building_overheat_info.as_ptr()
    };

    // L979-987: buildingNoLongerOverheatedInfos — 来自 buildingNoLongerOverheatedInfo
    gdu.num_building_no_longer_overheated_infos = game_data.building_no_longer_overheated_info.len() as i32;
    gdu.building_no_longer_overheated_infos = if game_data.building_no_longer_overheated_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.building_no_longer_overheated_info.as_ptr()
    };

    // L988-995: buildingMeltedInfos — 来自 buildingMeltedInfo
    gdu.num_building_melted_infos = game_data.building_melted_info.len() as i32;
    gdu.building_melted_infos = if game_data.building_melted_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.building_melted_info.as_ptr()
    };

    // L996-1003: cellMeltedInfos — 来自 cellMeltedInfo
    gdu.num_cell_melted_infos = game_data.cell_melted_info.len() as i32;
    gdu.cell_melted_infos = if game_data.cell_melted_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.cell_melted_info.as_ptr()
    };

    // L1004-1012: backwallElementChangedInfos — 来自 backwallElementChangedInfo
    gdu.num_backwall_element_changed_infos = game_data.backwall_element_changed_info.len() as i32;
    gdu.backwall_element_changed_infos = if game_data.backwall_element_changed_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.backwall_element_changed_info.as_ptr()
    };

    // L1013-1021: backwallShouldTransitionInfos — 来自 backwallShouldTransitionInfo
    gdu.num_backwall_should_transition_infos = game_data.backwall_should_transition_info.len() as i32;
    gdu.backwall_should_transition_infos = if game_data.backwall_should_transition_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.backwall_should_transition_info.as_ptr()
    };

    // L1022-1030: diseaseEmittedInfos — 来自 diseaseEmittedInfo
    gdu.num_disease_emitted_infos = game_data.disease_emitted_info.len() as i32;
    gdu.disease_emitted_infos = if game_data.disease_emitted_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.disease_emitted_info.as_ptr()
    };

    // L1031-1039: diseaseConsumedInfos — 来自 diseaseConsumedInfo
    gdu.num_disease_consumed_infos = game_data.disease_consumed_info.len() as i32;
    gdu.disease_consumed_infos = if game_data.disease_consumed_info.is_empty() {
        std::ptr::null()
    } else {
        game_data.disease_consumed_info.as_ptr()
    };

    // L1040-1050: radiationConsumedCallbacks (RadiationConsumedCallback, 12B=0xc, /0xc)
    gdu.num_radiation_consumed_callbacks = game_data.radiation_consumed_callbacks.len() as i32;
    gdu.radiation_consumed_callbacks = if game_data.radiation_consumed_callbacks.is_empty() {
        std::ptr::null()
    } else {
        game_data.radiation_consumed_callbacks.as_ptr()
    };

    // ===== 5. 提取 6 个 property texture 指针 =====
    // L1051-1057
    gdu.accumulated_flow = if !game_data.accumulated_flow.ptr.is_null() {
        game_data.accumulated_flow.ptr as *const f32
    } else {
        std::ptr::null()
    };
    gdu.property_texture_flow = if !game_data.property_texture_flow.ptr.is_null() {
        game_data.property_texture_flow.ptr as *const crate::a_framework::vector_math::Vector2f
    } else {
        std::ptr::null()
    };
    gdu.property_texture_liquid = if !game_data.property_texture_liquid.ptr.is_null() {
        game_data.property_texture_liquid.ptr as *const u32
    } else {
        std::ptr::null()
    };
    gdu.property_texture_liquid_data = if !game_data.property_texture_liquid_data.ptr.is_null() {
        game_data.property_texture_liquid_data.ptr as *const u32
    } else {
        std::ptr::null()
    };
    gdu.property_texture_material_data = if !game_data.property_texture_material_data.ptr.is_null() {
        game_data.property_texture_material_data.ptr as *const u32
    } else {
        std::ptr::null()
    };
    gdu.property_texture_exposed_to_sunlight = if !game_data.property_texture_exposed_to_sunlight.ptr.is_null() {
        game_data.property_texture_exposed_to_sunlight.ptr as *const u8
    } else {
        std::ptr::null()
    };

    // L1058: return &gGameDataUpdate
    // &*gdu 获取 &GameDataUpdate，Mutex 是 static 的，内存位置固定，
    // guard drop 后指针仍有效（C# 端只读访问）。
    let ptr = &*gdu as *const GameDataUpdate as *mut c_void;

    // 诊断：记录最终 SOA 指针值（确认非 null）
    // packed struct 不能直接引用字段，先拷贝到局部变量
    let elem_idx_val = gdu.element_idx;
    let temperature_val = gdu.temperature;
    let mass_val = gdu.mass;
    let radiation_val = gdu.radiation;
    let disease_idx_val = gdu.disease_idx;
    let disease_count_val = gdu.disease_count;
    let accumulated_flow_val = gdu.accumulated_flow;
    tracing::info!(
        gdu_ptr = ?ptr,
        elem_idx = ?elem_idx_val,
        temperature = ?temperature_val,
        mass = ?mass_val,
        radiation = ?radiation_val,
        disease_idx = ?disease_idx_val,
        disease_count = ?disease_count_val,
        accumulated_flow = ?accumulated_flow_val,
        "PrepareGameDataUpdate: final GDU pointers"
    );

    drop(gdu); // 显式释放锁
    ptr
}

/// Start — 启动游戏模拟。
/// 对照源码 02_save_load.c L211-255。
///
/// **关键约束**（项目 memory）：
/// Start handler 必须返回真实可解引用的 GameDataUpdate 指针。
/// C# 立即解引用 ptr->elementIdx，0x1 哨兵导致 null-page 读取崩溃。
///
/// 启动序列：
/// 1. 如果 init_settle_thermal_boundaries 非零：CopyFrom + SettleThermalBoundaries（A3 桩）
/// 2. 对 sim_events 的 20 个 vector 执行 clear_keep_capacity（end = begin）
/// 3. FrameSync::initGameData(width-2, height-2, width, height, updated_cells)
/// 4. SimData::InitializeBoundary（A3 桩）
/// 5. CellSOA::CopyFrom(cells, updated_cells)
/// 6. Thread::Start(gSim)（A3 桩，不启动 sim 线程）
/// 7. 返回 PrepareGameDataUpdate(gFrameSync.m_game_data)
pub fn handle_start(_reader: &mut BinaryBufferReader) -> *mut c_void {
    let sim_data_ptr = globals::G_SIM_DATA.lock().0;
    if sim_data_ptr.is_null() {
        tracing::error!("Start: gSimData is null");
        return std::ptr::null_mut();
    }

    let sim_data = unsafe { &mut *sim_data_ptr };

    // L217-223: 如果 init_settle_thermal_boundaries 非零
    if sim_data.init_settle_thermal_boundaries {
        // CellSOA::CopyFrom(cells, updated_cells)
        let cells_ptr = sim_data.cells.ptr;
        let updated_cells_ptr = sim_data.updated_cells.ptr;
        if !cells_ptr.is_null() && !updated_cells_ptr.is_null() {
            unsafe {
                (*cells_ptr).copy_from(&*updated_cells_ptr);
            }
        }
        // SimData::SettleThermalBoundaries（源码 L220-222）
        // 读 cells、写 updated_cells（2026-08-01：A3 桩已替换为真实实现，
        // 见 SimData::settle_thermal_boundaries，对照 11_msvcrt_ignored.c L24803-24898）
        sim_data.settle_thermal_boundaries();
    }

    // L224-244: 对 sim_events 的 20 个 vector 执行 clear_keep_capacity（end = begin）
    // 对照源码 L225-244: *(lVar1 + offset + 0x10) = *(lVar1 + offset + 0x8)
    let sim_events_ptr = sim_data.sim_events.ptr;
    if !sim_events_ptr.is_null() {
        let sim_events = unsafe { &mut *sim_events_ptr };
        sim_events.substance_change_info.clear_keep_capacity();
        sim_events.spawn_liquid_info.clear_keep_capacity();
        sim_events.spawn_ore_info.clear_keep_capacity();
        sim_events.unstable_cell_info.clear_keep_capacity();
        sim_events.element_chunk_melted_info.clear_keep_capacity();
        sim_events.building_melted_info.clear_keep_capacity();
        sim_events.building_overheat_info.clear_keep_capacity();
        sim_events.building_no_longer_overheated_info.clear_keep_capacity();
        sim_events.cell_melted_info.clear_keep_capacity();
        sim_events.callback_info.clear_keep_capacity();
        sim_events.world_damage_info.clear_keep_capacity();
        sim_events.mass_consumed_callbacks.clear_keep_capacity();
        sim_events.radiation_consumed_callbacks.clear_keep_capacity();
        sim_events.mass_emitted_callbacks.clear_keep_capacity();
        sim_events.disease_consumed_callbacks.clear_keep_capacity();
        sim_events.spawn_fx_info.clear_keep_capacity();
        sim_events.component_state_changed_messages.clear_keep_capacity();
        sim_events.dig_info.clear_keep_capacity();
        sim_events.backwall_element_changed_info.clear_keep_capacity();
        sim_events.backwall_should_transition_info.clear_keep_capacity();
    }

    // L245-248: FrameSync::initGameData
    // 参数：game_width = width-2, game_height = height-2, full_width = width, full_height = height
    let game_width = sim_data.width - 2;
    let game_height = sim_data.height - 2;
    let full_width = sim_data.width;
    let full_height = sim_data.height;

    let frame_sync_mutex = globals::G_FRAME_SYNC.get_or_init(|| {
        parking_lot::Mutex::new(crate::a_framework::frame_sync::FrameSync::new_zeroed())
    });
    let mut frame_sync = frame_sync_mutex.lock();

    // 获取 updated_cells 引用用于 init_game_data
    let updated_cells_ptr = sim_data.updated_cells.ptr;
    if updated_cells_ptr.is_null() {
        tracing::error!("Start: updated_cells is null");
        return std::ptr::null_mut();
    }
    let updated_cells = unsafe { &*updated_cells_ptr };

    frame_sync.init_game_data(
        game_width,
        game_height,
        full_width,
        full_height,
        updated_cells,
    );

    // L249: SimData::InitializeBoundary（顶/底边界行，原版 L123236+）
    sim_data.initialize_boundary();

    // L250-251: CellSOA::CopyFrom(cells, updated_cells)
    let cells_ptr = sim_data.cells.ptr;
    if !cells_ptr.is_null() && !updated_cells_ptr.is_null() {
        unsafe {
            (*cells_ptr).copy_from(&*updated_cells_ptr);
        }
    }

    // L252: Thread::Start(gSim) — 启动 sim 线程
    // 对照源码 L252: Thread::Start(gSim)
    // C1 实现：使用 std::thread::spawn 启动后台线程运行 Sim::Main 主循环
    let sim_ptr = crate::globals::G_SIM.lock().0;
    if !sim_ptr.is_null() {
        crate::a_framework::sim_thread::start_sim_thread(sim_ptr);
    } else {
        tracing::error!("Start: gSim is null, cannot start sim thread");
    }

    // L253: return PrepareGameDataUpdate(gFrameSync.mGameData)
    let game_data_ptr = frame_sync.m_game_data.ptr;
    if game_data_ptr.is_null() {
        tracing::error!("Start: m_game_data is null after initGameData");
        return std::ptr::null_mut();
    }
    let game_data = unsafe { &*game_data_ptr };
    let result = prepare_game_data_update(game_data);

    tracing::info!(game_width, game_height, "Start done");
    result
}

/// DefineWorldOffsets handler。
/// 对照源码 02_save_load.c L297-342：读取世界数量 + 每世界 4 个 int
/// （offset_x / offset_y / size_x / size_y），存入 SimData.worlds（16B/条）。
/// DLC 多星图专用：游戏开始时 C# 通过本消息告知各世界在簇网格中的偏移与尺寸。
pub fn handle_define_world_offsets(reader: &mut BinaryBufferReader) -> *mut c_void {
    let sim_data_ptr = globals::G_SIM_DATA.lock().0;
    if sim_data_ptr.is_null() {
        tracing::warn!("DefineWorldOffsets: gSimData is null");
        return std::ptr::null_mut();
    }
    let sim_data = unsafe { &mut *sim_data_ptr };
    let count = match reader.read_int() {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(?e, "DefineWorldOffsets: count read failed");
            return std::ptr::null_mut();
        }
    };
    if count < 0 {
        tracing::warn!(count, "DefineWorldOffsets: negative count");
        return std::ptr::null_mut();
    }
    sim_data.worlds.clear();
    for _ in 0..count {
        let offset_x = match reader.read_int() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(?e, "DefineWorldOffsets: offset_x read failed");
                return std::ptr::null_mut();
            }
        };
        let offset_y = match reader.read_int() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(?e, "DefineWorldOffsets: offset_y read failed");
                return std::ptr::null_mut();
            }
        };
        let size_x = match reader.read_int() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(?e, "DefineWorldOffsets: size_x read failed");
                return std::ptr::null_mut();
            }
        };
        let size_y = match reader.read_int() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(?e, "DefineWorldOffsets: size_y read failed");
                return std::ptr::null_mut();
            }
        };
        sim_data.worlds.push(WorldOffsetData {
            offset_x,
            offset_y,
            width: size_x,
            height: size_y,
        });
    }
    tracing::info!(count, "DefineWorldOffsets done");
    std::ptr::null_mut()
}

/// PrepareGameData handler — 每帧同步前准备 GameDataUpdate。
/// 对照源码 02_save_load.c L346-404。
///
/// **关键约束**（项目 memory）：
/// PrepareGameData 也必须返回真实 GDU 指针。
/// Game.StepTheSim（Game.cs L1065-1066）把返回值强转为 `GameDataUpdate*`
/// 并立即解引用 `ptr->elementIdx`。返回 null → NRE → SIM_Shutdown。
///
/// **C1 阶段流程**：
/// 1. 调用 FrameSync::game_sync（等待 sim 线程数据就绪，交换双缓冲）
/// 2. 从交换后的 m_game_data 调用 prepare_game_data_update 填充 GDU
///
/// **注意**：game_sync 会阻塞等待 sim 线程的 SimSync。
/// 如果 sim 线程未启动（m_initialized=false），跳过 game_sync 直接读取 m_game_data。
pub fn handle_prepare_game_data(reader: &mut BinaryBufferReader) -> *mut c_void {
    let fs_ptr = crate::a_framework::frame_sync::FrameSync::get_ptr();
    if fs_ptr.is_null() {
        tracing::error!("PrepareGameData: G_FRAME_SYNC not initialized");
        return std::ptr::null_mut();
    }

    // 0. 摄取 C# 每帧上传的 Grid.Visible 载荷（Game.cs L1060：PrepareGameData 消息载荷，
    // 原版 GameSyncFunction 02_save_load.c L666-684 断言剩余 == width×height 后 memcpy 到
    // mGameData.visibleGrid，交换后成为 sim 下一帧的可见性缓冲）。
    // 2026-08-07 补齐：此前载荷被直接丢弃 → visible_grid 恒 0xFF → 未探索区域也持续生成
    // 液滴/矿石/碎片回调（与原版行为不符 + 无谓开销；原版特性.txt §2 结论已订正）。
    unsafe {
        let gd_ptr = (*fs_ptr).m_game_data.ptr;
        if !gd_ptr.is_null() {
            let gd = &mut *gd_ptr;
            let total = (gd.width as usize).saturating_mul(gd.height as usize);
            let remaining = reader.remaining() as usize;
            if !gd.visible_grid.ptr.is_null() && total > 0 {
                if remaining >= total {
                    let dst = std::slice::from_raw_parts_mut(gd.visible_grid.ptr, total);
                    for i in 0..total {
                        match reader.read_byte() {
                            Ok(b) => dst[i] = b,
                            Err(_) => break,
                        }
                    }
                } else {
                    tracing::warn!(
                        "PrepareGameData: payload {}B < game cells {}B",
                        remaining,
                        total
                    );
                }
            }
        }
    }

    // 1. game_sync：等待 sim 线程数据就绪，交换双缓冲
    // 仅在 sim 线程已启动（m_initialized=true）时调用，避免 Start handler 后首次调用死锁
    let need_game_sync = unsafe { (*fs_ptr).m_initialized };
    if need_game_sync {
        unsafe {
            crate::a_framework::frame_sync::FrameSync::game_sync(fs_ptr);
        }
    }

    // 2. 从交换后的 m_game_data 填充 GameDataUpdate
    let game_data_ptr = unsafe { (*fs_ptr).m_game_data.ptr };
    if game_data_ptr.is_null() {
        tracing::error!("PrepareGameData: m_game_data is null");
        return std::ptr::null_mut();
    }

    let game_data = unsafe { &*game_data_ptr };
    let result = prepare_game_data_update(game_data);

    tracing::debug!("PrepareGameData done (game_sync={})", need_game_sync);
    result
}

/// ClearUnoccupiedCells handler。
/// 对照源码 02_save_load.c L405-465：全图 cells + updatedCells 重置为真空
/// （element=vacuum、mass/temp/radiation=0、disease_idx=0xff、disease_count=0），
/// 不触碰背墙与世界分区。DLC 多星图加载世界前 C# 会调用。
pub fn handle_clear_unoccupied_cells(_reader: &mut BinaryBufferReader) -> *mut c_void {
    let sim_data_ptr = globals::G_SIM_DATA.lock().0;
    if sim_data_ptr.is_null() {
        tracing::warn!("ClearUnoccupiedCells: gSimData is null");
        return std::ptr::null_mut();
    }
    let sim_data = unsafe { &mut *sim_data_ptr };
    if sim_data.cells.ptr.is_null() || sim_data.updated_cells.ptr.is_null() {
        tracing::warn!("ClearUnoccupiedCells: cells/updated_cells is null");
        return std::ptr::null_mut();
    }
    let total = (sim_data.width as usize).saturating_mul(sim_data.height as usize);
    let vacuum = sim_data.vacuum_element_idx;
    for buf_ptr in [sim_data.cells.ptr, sim_data.updated_cells.ptr] {
        let cells = unsafe { &mut *buf_ptr };
        for i in 0..total {
            cells.element_idx.set(i, vacuum);
            cells.mass.set(i, 0.0);
            cells.temperature.set(i, 0.0);
            cells.disease_count.set(i, 0);
            cells.disease_idx.set(i, 0xff);
            cells.radiation.set(i, 0.0);
        }
    }
    tracing::info!(total, "ClearUnoccupiedCells done");
    std::ptr::null_mut()
}

/// SetSavedOptions handler（原版 02_save_load.c L466-484 + C# SimMessages L366-374）。
///
/// 消息 2B：{clearBits u8, setBits u8} →
/// `saved_options = (saved_options & ~clear_bits) | set_bits`。
/// 位定义（C# SimSavedOptions）：bit0 = ENABLE_DIAGONAL_FALLING_SAND（斜向塌方），
/// 门控 DoUnstableCheckBasic/WithDiagonals 分派；当前 C# 无调用方 → 恒 0。
pub fn handle_set_saved_options(reader: &mut BinaryBufferReader) -> *mut c_void {
    let sim_data_ptr = globals::G_SIM_DATA.lock().0;
    if sim_data_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let clear_bits = match reader.read_byte() {
        Ok(v) => v,
        Err(_) => return std::ptr::null_mut(),
    };
    let set_bits = match reader.read_byte() {
        Ok(v) => v,
        Err(_) => return std::ptr::null_mut(),
    };
    let sim = unsafe { &mut *sim_data_ptr };
    sim.saved_options = (sim.saved_options & !clear_bits) | set_bits;
    std::ptr::null_mut()
}

/// ResizeAndInitializeVacuumCells handler。
/// 对照源码 SimDLL_Source.c L123719-123950（SimData::ResizeAndInitializeVacuumCells）。
///
/// 消息：grid_size.x(i32) + grid_size.y(i32) + width + height + x_offset + y_offset。
/// （grid_size 原版读取后不使用；DLC 新增小行星/火箭舱时 C# 调用。）
///
/// 效果：在簇网格 (x_offset, y_offset) 处初始化一个 width×height 的矩形区域：
/// - 内部（行 y_offset+1..y_offset+height、列 x_offset+1..x_offset+width）：
///   cells/updatedCells = 真空、mass/temp/radiation=0、disease 清零；backwall = 真空。
/// - 边界（上 y_offset / 下 y_offset+height+1 / 左 x_offset / 右 x_offset+width+1）：
///   cells/updatedCells = 中子(unobtanium)、mass=9999、temp/radiation=0；backwall = 真空。
pub fn handle_resize_and_initialize_vacuum_cells(reader: &mut BinaryBufferReader) -> *mut c_void {
    let sim_data_ptr = globals::G_SIM_DATA.lock().0;
    if sim_data_ptr.is_null() {
        tracing::warn!("ResizeAndInitializeVacuumCells: gSimData is null");
        return std::ptr::null_mut();
    }
    let mut read_int = || match reader.read_int() {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(?e, "ResizeAndInitializeVacuumCells: read failed");
            -1
        }
    };
    let _grid_size_x = read_int();
    let _grid_size_y = read_int();
    let width = read_int();
    let height = read_int();
    let x_offset = read_int();
    let y_offset = read_int();
    if width < 0 || height < 0 {
        tracing::warn!(width, height, "ResizeAndInitializeVacuumCells: invalid size");
        return std::ptr::null_mut();
    }
    let sim_data = unsafe { &mut *sim_data_ptr };
    if sim_data.cells.ptr.is_null()
        || sim_data.updated_cells.ptr.is_null()
        || sim_data.backwall.ptr.is_null()
    {
        tracing::warn!("ResizeAndInitializeVacuumCells: buffers null");
        return std::ptr::null_mut();
    }
    let vacuum = sim_data.vacuum_element_idx;
    let unobtanium = sim_data.unobtanium_element_idx;
    let sim_w = sim_data.width as i64;

    // 写一个格：双缓冲 + 背墙（元素/质量按参数，其余清零）。
    // 对照原版各循环体：element/mass/temp/disease_count/disease_idx/radiation + backwall。
    fn write_cell(
        sim: &mut SimData,
        cell: i64,
        elem: u16,
        mass: f32,
        bw_elem: u16,
    ) {
        if cell < 0 {
            return;
        }
        let cell = cell as usize;
        for buf_ptr in [sim.cells.ptr, sim.updated_cells.ptr] {
            if buf_ptr.is_null() {
                continue;
            }
            let cells = unsafe { &mut *buf_ptr };
            if cell >= cells.element_idx.len() {
                continue;
            }
            cells.element_idx.set(cell, elem);
            cells.mass.set(cell, mass);
            cells.temperature.set(cell, 0.0);
            cells.disease_count.set(cell, 0);
            cells.disease_idx.set(cell, 0xff);
            cells.radiation.set(cell, 0.0);
        }
        if !sim.backwall.ptr.is_null() {
            let backwall = unsafe { &mut *sim.backwall.ptr };
            if cell < backwall.element_idx.len() {
                backwall.element_idx.set(cell, bw_elem);
                backwall.mass.set(cell, 0.0);
                backwall.temperature.set(cell, 0.0);
            }
        }
    }

    // 内部：行 y_offset+1..y_offset+height，列 x_offset+1..x_offset+width（原版 L123756）
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let cell = (y_offset as i64 + 1 + y) * sim_w + (x_offset as i64 + x) + 1;
            write_cell(sim_data, cell, vacuum, 0.0, vacuum);
        }
    }
    // 边界行：上 y_offset、下 y_offset+height+1，列 x_offset..x_offset+width+1（原版 Loop A）
    for i in 0..width as i64 + 2 {
        let top = sim_w * y_offset as i64 + (x_offset as i64 + i);
        write_cell(sim_data, top, unobtanium, 9999.0, vacuum);
        let bottom = (y_offset as i64 + 1 + height as i64) * sim_w + (x_offset as i64 + i);
        write_cell(sim_data, bottom, unobtanium, 9999.0, vacuum);
    }
    // 边界列：左 x_offset、右 x_offset+width+1，行 y_offset+1..y_offset+height（原版 Loop B）
    for i in 1..height as i64 + 1 {
        let row = (y_offset as i64 + i) * sim_w;
        write_cell(sim_data, row + x_offset as i64, unobtanium, 9999.0, vacuum);
        write_cell(
            sim_data,
            row + x_offset as i64 + width as i64 + 1,
            unobtanium,
            9999.0,
            vacuum,
        );
    }
    tracing::info!(
        width,
        height,
        x_offset,
        y_offset,
        "ResizeAndInitializeVacuumCells done"
    );
    std::ptr::null_mut()
}

/// CreateDiseaseTable — 解析病菌表并挂接 gDisease。
/// 对照源码 02_save_load.c L494-517：
/// 1. `Disease::Disease(this, param_1)` 从消息流构造病菌表（L25718-26091）；
/// 2. unique_ptr 替换 gDisease，旧表析构释放（L507-516）；
/// 3. 返回 `(void*)0x0`（L517）。
pub fn handle_create_disease_table(reader: &mut BinaryBufferReader) -> *mut c_void {
    match Disease::from_stream(reader) {
        Ok(table) => {
            let mut g = globals::G_DISEASE.lock();
            if !g.0.is_null() {
                unsafe {
                    let _ = Box::from_raw(g.0);
                }
            }
            g.0 = Box::into_raw(Box::new(table));
            tracing::info!("CreateDiseaseTable success");
            std::ptr::null_mut()
        }
        Err(e) => {
            tracing::error!(error = ?e, "CreateDiseaseTable failed");
            std::ptr::null_mut()
        }
    }
}

// ===== 4 个非 save_load handler（任务 6 桩）=====

/// SimData_InitializeFromCells handler。
/// 对照源码 11_msvcrt_ignored.c L24200-24421。
///
/// 消息布局：width(i32) + height(i32) + simSeed(u32) + radiationEnabled(bool) + headless(bool)
/// + Sim.Cell[N]×16B + DiseaseCell[N]×12B + SimBackwall[N]×12B（N = width×height）。
/// 元素为 u16 索引（非哈希，无需查表转换）；数据写入 updated_cells / backwall；
/// 内部格索引 = (row+1)×sim_width + (col+1)。末尾调 SettleThermalBoundaries + InitializeBoundary。
/// 返回值：格子总数 width*height（原版语义；C# 侧忽略）。
pub fn handle_initialize_from_cells(reader: &mut BinaryBufferReader) -> *mut c_void {
    // 读头（对照 L24226-24230）
    let width = match reader.read_int() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read width failed: {:?}", e); return std::ptr::null_mut(); }
    };
    let height = match reader.read_int() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read height failed: {:?}", e); return std::ptr::null_mut(); }
    };
    let sim_seed = match reader.read_uint() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read simSeed failed: {:?}", e); return std::ptr::null_mut(); }
    };
    let radiation_enabled = match reader.read_bool() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read radiation flag failed: {:?}", e); return std::ptr::null_mut(); }
    };
    let headless = match reader.read_bool() {
        Ok(v) => v,
        Err(e) => { tracing::error!("InitializeFromCells: read headless flag failed: {:?}", e); return std::ptr::null_mut(); }
    };

    // 参数与长度校验（增强：原版无检查；不满足时 gSimData 保持原样）
    if width <= 0 || height <= 0 {
        tracing::error!("InitializeFromCells: invalid size {}x{}", width, height);
        return std::ptr::null_mut();
    }
    let num_cells = (width as usize) * (height as usize);
    let needed = num_cells * 40;
    if (reader.remaining() as usize) < needed {
        tracing::error!("InitializeFromCells: buffer too small: remaining {} < needed {}", reader.remaining(), needed);
        return std::ptr::null_mut();
    }

    // 三块基址（对照 L24232-24236：记录位置 + skip）
    let cells_base = reader.offset();
    if reader.skip(num_cells * 16).is_err() {
        return std::ptr::null_mut();
    }
    let disease_base = reader.offset();
    if reader.skip(num_cells * 12).is_err() {
        return std::ptr::null_mut();
    }
    let backwall_base = reader.offset();

    // 建 SimData 替换全局（对照 L24237-24249，替换模式同 handle_allocate_cells）
    let new_sim_data = Box::new(SimData::new_for_allocate(
        width + 2,
        height + 2,
        sim_seed,
        radiation_enabled,
        headless,
    ));
    let new_ptr = Box::into_raw(new_sim_data);
    let mut global_sim_data = globals::G_SIM_DATA.lock();
    if !global_sim_data.0.is_null() {
        unsafe {
            let _ = Box::from_raw(global_sim_data.0);
        }
    }
    global_sim_data.0 = new_ptr;
    crate::c_simulation::radiation_emitter::clear_radiation_emitters();
    crate::c_simulation::disease_component::clear_disease_emitters();
    // 同 handle_allocate_cells：新世界分配 = 存档切换，清空全部全局静态注册表。
    crate::c_simulation::element_emitter::clear_element_emitters();
    crate::c_simulation::element_chunk::clear_element_chunks();
    crate::c_simulation::building_temperature::clear_building_temperature();
    crate::c_simulation::building_to_building::clear_building_to_building();
    crate::c_simulation::disease_component::clear_disease_consumers();

    unsafe {
        let sim_data = &mut *new_ptr;
        let sim_width = sim_data.width as usize;
        let cells = &mut *sim_data.updated_cells.ptr;
        let backwall = &mut *sim_data.backwall.ptr;
        let msg_width = width as usize;

        // 循环 1：Sim.Cell → updated_cells（对照 L24252-24322）
        for row in 0..height as usize {
            for col in 0..msg_width {
                let src = cells_base + ((row * msg_width + col) * 16) as u64;
                let dst = (row + 1) * sim_width + (col + 1);
                cells.element_idx.set(dst, reader.read_u16_at(src).unwrap_or(0));
                cells.properties.set(dst, reader.read_u8_at(src + 2).unwrap_or(0));
                cells.insulation.set(dst, reader.read_u8_at(src + 3).unwrap_or(0));
                let strength = reader.read_u8_at(src + 4).unwrap_or(0);
                cells.strength_info.set(dst, strength);
                cells.temperature.set(dst, reader.read_f32_at(src + 8).unwrap_or(0.0));
                cells.mass.set(dst, reader.read_f32_at(src + 12).unwrap_or(0.0));
                cells.radiation.set(dst, 0.0);
            }
        }

        // 循环 2：DiseaseCell → updated_cells（对照 L24323-24372）
        for row in 0..height as usize {
            for col in 0..msg_width {
                let src = disease_base + ((row * msg_width + col) * 12) as u64;
                let dst = (row + 1) * sim_width + (col + 1);
                cells.disease_idx.set(dst, reader.read_u8_at(src).unwrap_or(0));
                cells.disease_infestation_tick_count.set(dst, reader.read_u8_at(src + 1).unwrap_or(0));
                cells.disease_count.set(dst, reader.read_i32_at(src + 4).unwrap_or(0));
                cells.disease_growth_accumulated_error.set(dst, 0.0);
            }
        }
        // 循环 3：SimBackwall → backwall（对照 L24373-24415）
        for row in 0..height as usize {
            for col in 0..msg_width {
                let src = backwall_base + ((row * msg_width + col) * 12) as u64;
                let dst = (row + 1) * sim_width + (col + 1);
                backwall.element_idx.set(dst, reader.read_u16_at(src).unwrap_or(0));
                backwall.mass.set(dst, reader.read_f32_at(src + 4).unwrap_or(0.0));
                backwall.temperature.set(dst, reader.read_f32_at(src + 8).unwrap_or(0.0));
            }
        }

        // Settle + InitializeBoundary（对照 L24416-24419）
        crate::c_simulation::sim_data_ops::derive_natural_solid_strength(
            sim_data,
            crate::d1_activity::full_grid_bounds(sim_data),
        );
        sim_data.settle_thermal_boundaries();
        sim_data.initialize_boundary();
    }

    tracing::info!("InitializeFromCells done: {}x{} ({} cells), seed={:#010x}", width, height, num_cells, sim_seed);
    num_cells as *mut c_void
}

/// SimData_FreeCells handler。
/// 对照源码 SimDLL_Source.c L123019-123208（SimData::FreeGridCells）。
///
/// 消息：width(i32) + height(i32) + x_offset(i32) + y_offset(i32)。
/// DLC 移除世界/火箭舱时 C# 调用（Grid.FreeGridSpace → SimDataFreeCells）：
/// 1. 把 (x_offset, y_offset) 处 (width+2)×(height+2) 矩形（含边界一圈）
///    的 cells/updatedCells 清为真空、backwall 清为真空；
/// 2. 从 SimData.worlds 移除 offset/尺寸完全匹配的世界条目。
pub fn handle_free_grid_cells(reader: &mut BinaryBufferReader) -> *mut c_void {
    let sim_data_ptr = globals::G_SIM_DATA.lock().0;
    if sim_data_ptr.is_null() {
        tracing::warn!("FreeGridCells: gSimData is null");
        return std::ptr::null_mut();
    }
    let mut read_int = || match reader.read_int() {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(?e, "FreeGridCells: read failed");
            -1
        }
    };
    let width = read_int();
    let height = read_int();
    let x_offset = read_int();
    let y_offset = read_int();
    if width < 0 || height < 0 {
        tracing::warn!(width, height, "FreeGridCells: invalid size");
        return std::ptr::null_mut();
    }
    let sim_data = unsafe { &mut *sim_data_ptr };
    if sim_data.cells.ptr.is_null()
        || sim_data.updated_cells.ptr.is_null()
        || sim_data.backwall.ptr.is_null()
    {
        tracing::warn!("FreeGridCells: buffers null");
        return std::ptr::null_mut();
    }
    let vacuum = sim_data.vacuum_element_idx;
    let sim_w = sim_data.width as i64;

    // 清空矩形（原版 L123030-123160）：行 y_offset..y_offset+height+1，
    // 列 x_offset..x_offset+width+1，双缓冲 + 背墙全置真空。
    for y in 0..height as i64 + 2 {
        for x in 0..width as i64 + 2 {
            let cell = (y_offset as i64 + y) * sim_w + (x_offset as i64 + x);
            if cell < 0 {
                continue;
            }
            let cell = cell as usize;
            for buf_ptr in [sim_data.cells.ptr, sim_data.updated_cells.ptr] {
                let cells = unsafe { &mut *buf_ptr };
                if cell >= cells.element_idx.len() {
                    continue;
                }
                cells.element_idx.set(cell, vacuum);
                cells.mass.set(cell, 0.0);
                cells.temperature.set(cell, 0.0);
                cells.disease_count.set(cell, 0);
                cells.disease_idx.set(cell, 0xff);
                cells.radiation.set(cell, 0.0);
            }
            let backwall = unsafe { &mut *sim_data.backwall.ptr };
            if cell < backwall.element_idx.len() {
                backwall.element_idx.set(cell, vacuum);
                backwall.mass.set(cell, 0.0);
                backwall.temperature.set(cell, 0.0);
            }
        }
    }

    // 从 worlds 移除匹配条目（原版 L123164-123208：offset_x/offset_y/width/height
    // 完全一致才移除，vector 通过 memcpy 前移 + _Mylast-=16 实现 erase）。
    let old_len = sim_data.worlds.len();
    let keep: Vec<WorldOffsetData> = sim_data
        .worlds
        .as_slice()
        .iter()
        .copied()
        .filter(|w| {
            !(w.width == width && w.height == height && w.offset_x == x_offset && w.offset_y == y_offset)
        })
        .collect();
    if keep.len() != old_len {
        sim_data.worlds.clear();
        for w in keep {
            sim_data.worlds.push(w);
        }
    }
    tracing::info!(width, height, x_offset, y_offset, "FreeGridCells done");
    std::ptr::null_mut()
}

/// SetWorldZones handler。
/// 对照源码 11_msvcrt_ignored.c L24760-24805（SimData::SetWorldZones）：
/// 1. 分配 width×height 字节（含边界 total_cells），释放旧的；
/// 2. memset 清零；
/// 3. 逐行读取 (width−2) 字节，写入 row×width+1（去边界，sim 坐标内部）。
/// 2026-08-04 审查修正：A3 桩不处理 → world_zones 恒 null → 世界区域数据缺失。
/// 消费端：PostProcessCell 世界区域分支（L42954）按 worldZones[cell]==-1 特判。
pub fn handle_set_world_zones(reader: &mut BinaryBufferReader) -> *mut c_void {
    let sim_data_ptr = globals::G_SIM_DATA.lock().0;
    if sim_data_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let sim_data = unsafe { &mut *sim_data_ptr };
    let width = sim_data.width as usize;
    let height = sim_data.height as usize;
    if width == 0 || height == 0 {
        return std::ptr::null_mut();
    }
    let total = width * height;
    // 释放旧的 world_zones（vec![...].leak() 分配；初始 null）
    if !sim_data.world_zones.ptr.is_null() {
        unsafe {
            let _ = Vec::from_raw_parts(sim_data.world_zones.ptr, total, total);
        }
    }
    // 原版 L24784-24790：memset 清零后逐行读
    let mut zones = vec![0u8; total];
    let row_len = width.saturating_sub(2);
    let rows = height.saturating_sub(2);
    for row in 0..rows {
        let dst = (row + 1) * width + 1;
        for col in 0..row_len {
            match reader.read_byte() {
                Ok(b) => zones[dst + col] = b,
                Err(_) => {
                    tracing::warn!(row, col, "SetWorldZones: buffer exhausted");
                    sim_data.world_zones = UniquePtr { ptr: zones.leak().as_mut_ptr() };
                    return std::ptr::null_mut();
                }
            }
        }
    }
    sim_data.world_zones = UniquePtr { ptr: zones.leak().as_mut_ptr() };
    std::ptr::null_mut()
}

// ===== 保存函数（对照源码 01_sim_api.c L7-204）=====

/// BeginSave — 开始保存存档。
/// 对照源码 01_sim_api.c L7-171。
///
/// 存档布局（version 0xf，total_cells = width × height）：
/// ```text
/// 偏移   大小    字段
/// 0x00   8B     magic "SIMSAVE\0"
/// 0x08   4B     version = 0xf
/// 0x0c   4B     width
/// 0x10   4B     height
/// 0x14   4B     param_2
/// 0x18   4B     param_3
/// 0x1c   1B     saved_options (gSimData+0x18)
/// 0x1d   cells × 36B  cell 数据
/// ```
///
/// 每个 cell 36 字节 = 3 个循环：
/// - 循环1 (16B)：4B element_hash + 4B temperature + 4B mass + 4B radiation
/// - 循环2 (8B)：4B disease_hash + 4B disease_count
/// - 循环3 (12B)：4B backwall_element_hash + 4B backwall_mass + 4B backwall_temperature
///
/// SaveBuffer 用 Box<Vec<u8>> 存储，返回 Vec 的 data 指针。
/// EndSave 或 CleanUp 释放 Box。
pub fn begin_save(
    out_size: *mut std::os::raw::c_int,
    p2: std::os::raw::c_int,
    p3: std::os::raw::c_int,
) -> *mut std::os::raw::c_schar {
    // L30-35: 如果已有 SaveBuffer，先释放它（调用 end_save）
    end_save();

    // L36: 检查 gSimData
    let sim_data_ptr = globals::G_SIM_DATA.lock().0;
    if sim_data_ptr.is_null() {
        tracing::error!("BeginSave: gSimData is null");
        if !out_size.is_null() {
            unsafe { *out_size = 0; }
        }
        return std::ptr::null_mut();
    }

    let sim_data = unsafe { &*sim_data_ptr };

    // L36-38: 计算总大小 = width × height × 0x24 + 0x1d
    let width = sim_data.width;
    let height = sim_data.height;
    let total_cells = (width as usize).saturating_mul(height as usize);
    let total_size = total_cells * 0x24 + 0x1d;

    // L38: *param_1 = uVar15
    if !out_size.is_null() {
        unsafe { *out_size = total_size as i32; }
    }

    // L39-44: 创建新 Buffer（Rust 用 Vec<u8> 替代 C++ Buffer + BinaryBufferWriter）
    let mut buffer: Vec<u8> = Vec::with_capacity(total_size);

    // L46: WriteBytes(8, "SIMSAVE\0")
    buffer.extend_from_slice(b"SIMSAVE\0");
    // L47: WriteInt(0xf) — version
    buffer.extend_from_slice(&15i32.to_le_bytes());
    // L48: WriteInt(gSimData->width)
    buffer.extend_from_slice(&width.to_le_bytes());
    // L49: WriteInt(gSimData->height)
    buffer.extend_from_slice(&height.to_le_bytes());
    // L50: WriteInt(param_2)
    buffer.extend_from_slice(&p2.to_le_bytes());
    // L51: WriteInt(param_3)
    buffer.extend_from_slice(&p3.to_le_bytes());
    // L52: WriteByte(gSimData->saved_options)
    buffer.push(sim_data.saved_options);

    // 2026-08-04 复核修正：原版 BeginSave 的 cells/disease 循环读取的是
    // `*(longlong *)(gSimData + 0x28)` = updatedCells（非 cells）。
    // 此前用 cells（稳定缓冲）→ 保存的可能是上一帧状态，与 C# 读取的最新帧不一致。
    let cells_ptr = sim_data.updated_cells.ptr;
    let backwall_ptr = sim_data.backwall.ptr;

    if total_cells > 0 {
        // ===== 循环1 (L56-89): 每个 cell 写 element_hash + temperature + mass + radiation =====
        if !cells_ptr.is_null() {
            let cells = unsafe { &*cells_ptr };
            let element_idx_slice = cells.element_idx.as_slice();
            let temperature_slice = cells.temperature.as_slice();
            let mass_slice = cells.mass.as_slice();
            let radiation_slice = cells.radiation.as_slice();

            for i in 0..total_cells {
                // L62-68: element_idx → gElements[idx].id (hash)
                let elem_idx = element_idx_slice.get(i).copied().unwrap_or(0);
                let elem_hash = elements_table::get_element_id_by_idx(elem_idx).unwrap_or(0);
                buffer.extend_from_slice(&elem_hash.to_le_bytes());

                // L73-74: temperature
                let temp = temperature_slice.get(i).copied().unwrap_or(0.0);
                buffer.extend_from_slice(&temp.to_le_bytes());

                // L79-80: mass
                let mass = mass_slice.get(i).copied().unwrap_or(0.0);
                buffer.extend_from_slice(&mass.to_le_bytes());

                // L85-86: radiation (cells+0x148 = radiation begin)
                let rad = radiation_slice.get(i).copied().unwrap_or(0.0);
                buffer.extend_from_slice(&rad.to_le_bytes());
            }

            // ===== 循环2 (L90-117): 每个 cell 写 disease_hash + disease_count =====
            // 仅当 version > 8 时 Load 才读取此循环，但 BeginSave 始终写入（version=0xf > 8）
            let disease_idx_slice = cells.disease_idx.as_slice();
            let disease_count_slice = cells.disease_count.as_slice();
            let disease_ptr = globals::G_DISEASE.lock().0;

            for i in 0..total_cells {
                // L96: disease_idx[i] (byte)
                let disease_idx = disease_idx_slice.get(i).copied().unwrap_or(0xff);

                // L98-108: 如果 disease_idx != 0xff，从 gDisease 查找 hash_id
                let disease_hash: u32 = if disease_idx != 0xff && !disease_ptr.is_null() {
                    let disease = unsafe { &*disease_ptr };
                    disease.diseases.as_slice()
                        .get(disease_idx as usize)
                        .map(|d| d.hash_id)
                        .unwrap_or(0)
                } else {
                    0
                };

                // L109: WriteUInt(disease_hash) — vtable[0x30]
                buffer.extend_from_slice(&disease_hash.to_le_bytes());

                // L114-115: WriteInt(disease_count[i]) — vtable[0x38]
                let disease_count = disease_count_slice.get(i).copied().unwrap_or(0);
                buffer.extend_from_slice(&disease_count.to_le_bytes());
            }
        }

        // ===== 循环3 (L118-157): 每个 backwall 写 element_hash + mass + temperature =====
        // 仅当 version > 0xe 时 Load 才读取此循环，但 BeginSave 始终写入（version=0xf > 0xe）
        if !backwall_ptr.is_null() {
            let backwall = unsafe { &*backwall_ptr };
            let bw_elem_idx_slice = backwall.element_idx.as_slice();
            let bw_mass_slice = backwall.mass.as_slice();
            let bw_temp_slice = backwall.temperature.as_slice();

            for i in 0..total_cells {
                // L124: backwall_element_idx[i] (u16)
                let bw_elem_idx = bw_elem_idx_slice.get(i).copied().unwrap_or(0xffff);

                // L125-137: 如果 element_idx == 0xffff，转换为真空元素 + 清零 mass/temperature
                let (final_hash, final_mass, final_temp) = if bw_elem_idx == 0xffff {
                    // L126: GetElementIndex(0x2d39bf75) — 获取真空元素 idx
                    let vacuum_idx = elements_table::get_element_index_pub(0x2d39bf75);
                    let vacuum_hash = elements_table::get_element_id_by_idx(vacuum_idx).unwrap_or(0);
                    // L131, L136: mass = 0, temperature = 0
                    (vacuum_hash, 0.0f32, 0.0f32)
                } else {
                    let hash = elements_table::get_element_id_by_idx(bw_elem_idx).unwrap_or(0);
                    let mass = bw_mass_slice.get(i).copied().unwrap_or(0.0);
                    let temp = bw_temp_slice.get(i).copied().unwrap_or(0.0);
                    (hash, mass, temp)
                };

                // L142-143: WriteInt(element_hash) — vtable[0x38]
                buffer.extend_from_slice(&final_hash.to_le_bytes());
                // L148-149: WriteFloat(mass) — vtable[0x18]
                buffer.extend_from_slice(&final_mass.to_le_bytes());
                // L154-155: WriteFloat(temperature) — vtable[0x18]
                buffer.extend_from_slice(&final_temp.to_le_bytes());
            }
        } else {
            // backwall 为 null：每个 cell 写 12B 零（保持存档大小一致）
            tracing::warn!("BeginSave: backwall is null, writing zeros");
            for _ in 0..total_cells {
                buffer.extend_from_slice(&[0u8; 12]);
            }
        }
    }

    // L159-167: 校验 buffer 大小
    if buffer.len() != total_size {
        tracing::warn!(
            expected = total_size,
            actual = buffer.len(),
            "BeginSave: size mismatch"
        );
    }

    // L168: 获取 data 指针（对应源码 SaveBuffer->data()）
    // 存储 Box<Vec<u8>> 到 G_SAVE_BUFFER，返回 Vec 的 data 指针。
    // Box 在堆上，Vec 的数据也在堆上，只要 Box 不 drop，data 指针有效。
    let boxed: Box<Vec<u8>> = Box::new(buffer);
    let raw_ptr = Box::into_raw(boxed) as *mut c_void;
    *globals::G_SAVE_BUFFER.lock() = crate::globals::SendSyncPtr(raw_ptr);

    // 返回 data 指针（Vec<u8> 的 heap 数据指针）
    let data_ptr = unsafe { &*(raw_ptr as *mut Vec<u8>) }.as_ptr() as *mut std::os::raw::c_schar;

    tracing::info!(total_size, total_cells, "BeginSave done");
    data_ptr
}

/// EndSave — 结束保存存档。
/// 对照源码 01_sim_api.c L195-204。
///
/// 释放 SaveBuffer（Box<Vec<u8>>），置空全局指针。
pub fn end_save() {
    let mut save_buffer = globals::G_SAVE_BUFFER.lock();
    if !save_buffer.0.is_null() {
        unsafe {
            // 释放 Box<Vec<u8>>
            let _ = Box::from_raw(save_buffer.0 as *mut Vec<u8>);
        }
        save_buffer.0 = std::ptr::null_mut();
        tracing::info!("EndSave: SaveBuffer released");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::BinaryBufferWriter;
    use crate::LIB_TESTS_LOCK;

    #[test]
    fn clean_up_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
    }

    #[test]
    fn clean_up_with_null_globals_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        clean_up();
    }

    #[test]
    fn handle_allocate_cells_returns_non_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let mut w = BinaryBufferWriter::new();
        w.write_int(4);  // width
        w.write_int(3);  // height
        w.write_bool(false);  // flag1
        w.write_bool(false);  // flag2

        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = handle_allocate_cells(&mut reader);
        assert!(!result.is_null(), "AllocateCells should return non-null");

        // 验证全局 gSimData 已设置
        let sim_data_ptr = globals::G_SIM_DATA.lock().0;
        assert!(!sim_data_ptr.is_null());

        // 验证 SimData 尺寸（width+2, height+2）
        let sim_data = unsafe { &*sim_data_ptr };
        assert_eq!(sim_data.width, 6);  // 4 + 2
        assert_eq!(sim_data.height, 5); // 3 + 2
        assert_eq!(sim_data.num_game_cells, 12); // 4 * 3

        clean_up();
    }

    #[test]
    fn handle_allocate_cells_replaces_old_sim_data() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 第一次分配
        let mut w = BinaryBufferWriter::new();
        w.write_int(2);
        w.write_int(2);
        w.write_bool(false);
        w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&w.into_bytes());
        let first = handle_allocate_cells(&mut reader);
        assert!(!first.is_null());

        // 第二次分配（应释放第一次的）
        let mut w2 = BinaryBufferWriter::new();
        w2.write_int(4);
        w2.write_int(4);
        w2.write_bool(false);
        w2.write_bool(false);
        let mut reader2 = BinaryBufferReader::new(&w2.into_bytes());
        let second = handle_allocate_cells(&mut reader2);
        assert!(!second.is_null());
        assert_ne!(first, second, "second allocation should differ");

        // 验证全局指向第二次
        let sim_data_ptr = globals::G_SIM_DATA.lock().0;
        assert_eq!(sim_data_ptr as *mut c_void, second);

        clean_up();
    }

    #[test]
    fn allocate_cells_clears_radiation_emitter_registry() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        crate::c_simulation::radiation_emitter::clear_radiation_emitters();

        // 模拟上个存档注册了一个辐射发射器（全局注册表残留）
        let mut sd = crate::a_framework::sim_data::SimData::new_for_allocate(8, 8, 1, true, false);
        sd.vacuum_element_idx = 0;
        let mut w = BinaryBufferWriter::new();
        w.write_int(-1);
        w.write_int(3);
        w.write_short(2);
        w.write_short(2);
        w.write_float(110.0);
        w.write_float(0.0);
        w.write_float(1.0);
        w.write_float(0.0);
        w.write_float(360.0);
        w.write_int(0);
        let bytes = w.into_bytes();
        let mut frame = crate::a_framework::sim_frame_manager::SimFrameInfo::default();
        for &b in &bytes {
            frame.radiation_emitter_messages.adds.push_unchecked(b);
        }
        crate::c_simulation::radiation_emitter::process_radiation_emitter_messages(&frame, &mut sd);
        assert_eq!(
            crate::c_simulation::radiation_emitter::radiation_emitter_count(),
            1,
            "前置：旧存档发射器已注册"
        );

        // 新世界分配（存档切换）→ 注册表必须清空，否则旧发射器继续向新世界发射
        let mut w2 = BinaryBufferWriter::new();
        w2.write_int(4);
        w2.write_int(4);
        w2.write_bool(true);
        w2.write_bool(false);
        let mut reader = BinaryBufferReader::new(&w2.into_bytes());
        let result = handle_allocate_cells(&mut reader);
        assert!(!result.is_null());
        assert_eq!(
            crate::c_simulation::radiation_emitter::radiation_emitter_count(),
            0,
            "新世界分配必须清空旧存档发射器（防辐射残留）"
        );

        crate::c_simulation::radiation_emitter::clear_radiation_emitters();
        clean_up();
    }

    #[test]
    fn allocate_cells_clears_all_global_registries() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        // 清空所有全局注册表（避免跨测试污染）
        crate::c_simulation::element_emitter::clear_element_emitters();
        crate::c_simulation::element_chunk::clear_element_chunks();
        crate::c_simulation::building_temperature::clear_building_temperature();
        crate::c_simulation::building_to_building::clear_building_to_building();
        crate::c_simulation::disease_component::clear_disease_consumers();
        crate::c_simulation::radiation_emitter::clear_radiation_emitters();
        crate::c_simulation::disease_component::clear_disease_emitters();

        // 建最小元素表：elem5 液态（SHC=2.0、TC=4.0、low=200、high=5000）。
        {
            use crate::b_elements::element::ElementTemperatureData;
            let mut table = crate::b_elements::elements_table::G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.clear();
            table.element_names.clear();
            table.state_data.clear();
            table.liquid_data.clear();
            table.pressure_data.clear();
            table.post_process_data.clear();
            table.temperature_data.clear();
            let mut etd = ElementTemperatureData::default();
            etd.state = 2;
            etd.specific_heat_capacity = 2.0;
            etd.thermal_conductivity = 4.0;
            etd.high_temp = 5000.0;
            etd.low_temp = 200.0;
            table.elements.resize(6, crate::b_elements::element::Element::default());
            table.temperature_data.resize(6, ElementTemperatureData::default());
            table.temperature_data[5] = etd;
        }

        // 模拟上个存档：每个全局注册表各残留 1 条记录。
        let mut sd = crate::a_framework::sim_data::SimData::new_for_allocate(8, 8, 1, true, false);
        sd.vacuum_element_idx = 0;
        let h_emitter = crate::c_simulation::element_emitter::add_element_emitter(
            &mut sd,
            &crate::c_simulation::element_emitter::AddElementEmitterMsg::default(),
        );
        assert!(h_emitter >= 0, "前置：element_emitter 注册失败");
        let h_chunk = crate::c_simulation::element_chunk::add_element_chunk(
            8,
            &crate::c_simulation::element_chunk::AddElementChunkMsg {
                game_cell: 0,
                callback_idx: 7,
                mass: 100.0,
                temperature: 300.0,
                surface_area: 2.0,
                thickness: 0.5,
                ground_transfer_scale: 0.5,
                element_idx: 5,
                pad: [0; 2],
            },
        );
        assert!(h_chunk >= 0, "前置：element_chunk 注册失败");
        let h_building = crate::c_simulation::building_temperature::add_building(
            &crate::a_framework::sim_events::AddBuildingHeatExchangeMsg {
                callback_idx: 7,
                elem_idx: 5,
                pad0: 0,
                pad1: 0,
                mass: 100.0,
                temperature: 300.0,
                thermal_conductivity: 1.0,
                overheat_temperature: 2000.0,
                operating_kilowatts: 0.0,
                min_x: 1,
                min_y: 2,
                max_x: 2,
                max_y: 3,
            },
        );
        assert!(h_building >= 0, "前置：building_temperature 注册失败");
        let h_b2b =
            crate::c_simulation::building_to_building::register_building_to_building(7, h_building);
        assert!(h_b2b >= 0, "前置：building_to_building 注册失败");
        // disease_consumer：C# 无发送方，但防御链路注册表同样应被清空。
        let mut frame = crate::a_framework::sim_frame_manager::SimFrameInfo::default();
        for &b in &0i32.to_le_bytes() {
            frame.disease_consumer_messages.adds.push_unchecked(b);
        }
        for &b in &(-1i32).to_le_bytes() {
            frame.disease_consumer_messages.adds.push_unchecked(b);
        }
        for _ in 0..4 {
            frame.disease_consumer_messages.adds.push_unchecked(0);
        }
        crate::c_simulation::disease_component::process_disease_consumer_messages(&frame, &mut sd);

        // 前置：各注册表均有残留
        assert_eq!(crate::c_simulation::element_emitter::element_emitter_count(), 1);
        assert_eq!(crate::c_simulation::element_chunk::element_chunk_count(), 1);
        assert_eq!(
            crate::c_simulation::building_temperature::building_temperature_count(),
            1
        );
        assert_eq!(
            crate::c_simulation::building_to_building::building_to_building_count(),
            1
        );
        assert_eq!(crate::c_simulation::disease_component::disease_consumer_count(), 1);

        // 新世界分配（存档切换）→ 全部清空，否则旧句柄/旧数据残留在新局生效
        let mut w2 = BinaryBufferWriter::new();
        w2.write_int(4);
        w2.write_int(4);
        w2.write_bool(true);
        w2.write_bool(false);
        let mut reader = BinaryBufferReader::new(&w2.into_bytes());
        let result = handle_allocate_cells(&mut reader);
        assert!(!result.is_null());
        assert_eq!(
            crate::c_simulation::element_emitter::element_emitter_count(),
            0,
            "element_emitter 必须清空（EMBARK 首帧旧句柄越界崩溃根因）"
        );
        assert_eq!(crate::c_simulation::element_chunk::element_chunk_count(), 0);
        assert_eq!(
            crate::c_simulation::building_temperature::building_temperature_count(),
            0
        );
        assert_eq!(
            crate::c_simulation::building_to_building::building_to_building_count(),
            0
        );
        assert_eq!(crate::c_simulation::disease_component::disease_consumer_count(), 0);

        crate::c_simulation::element_emitter::clear_element_emitters();
        crate::c_simulation::element_chunk::clear_element_chunks();
        crate::c_simulation::building_temperature::clear_building_temperature();
        crate::c_simulation::building_to_building::clear_building_to_building();
        crate::c_simulation::disease_component::clear_disease_consumers();
        crate::c_simulation::radiation_emitter::clear_radiation_emitters();
        crate::c_simulation::disease_component::clear_disease_emitters();
        clean_up();
    }

    #[test]
    fn handle_load_invalid_magic_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let data = [0u8; 32]; // 全零，magic 不匹配
        let mut reader = BinaryBufferReader::new(&data);
        assert!(handle_load(&mut reader).is_null());

        clean_up();
    }

    #[test]
    fn handle_load_null_sim_data_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 有效 magic 但 gSimData 为 null
        let mut w = BinaryBufferWriter::new();
        w.write_bytes(b"SIMSAVE\0");
        w.write_int(13);
        w.write_int(1);
        w.write_int(1);
        w.write_byte(0);

        let mut reader = BinaryBufferReader::new(&w.into_bytes());
        assert!(handle_load(&mut reader).is_null());

        clean_up();
    }

    #[test]
    fn handle_load_valid_save_returns_sim_data() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 先 AllocateCells（2x2，含边界 4x4=16 cells）
        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(2);
        alloc_w.write_int(2);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut alloc_reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let sim_ptr = handle_allocate_cells(&mut alloc_reader);
        assert!(!sim_ptr.is_null());

        // 构造存档数据（version 13，2x2）
        let mut w = BinaryBufferWriter::new();
        w.write_bytes(b"SIMSAVE\0");  // magic
        w.write_int(13);               // version
        w.write_int(2);                // width
        w.write_int(2);                // height
        // version <= 13: 无 offset/skip
        w.write_byte(0);               // flag (version > 12)
        // 4 cells × 12 bytes (element_hash + temp + mass)
        for _ in 0..4 {
            w.write_uint(0x2d39bf75);  // vacuum hash
            w.write_float(300.0);
            w.write_float(0.0);
        }
        // disease: version > 8, 4 × 4 bytes
        for _ in 0..4 {
            w.write_uint(0);  // no disease
        }
        // skip: version < 15, 2*2*4 = 16 bytes
        for _ in 0..16 {
            w.write_byte(0);
        }

        let mut reader = BinaryBufferReader::new(&w.into_bytes());
        let result = handle_load(&mut reader);
        assert!(!result.is_null(), "Load should return non-null");

        // 验证 SimData 仍然有效
        let sim_data_ptr = globals::G_SIM_DATA.lock().0;
        assert!(!sim_data_ptr.is_null());

        clean_up();
    }

    #[test]
    fn handle_load_high_version_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let mut w = BinaryBufferWriter::new();
        w.write_bytes(b"SIMSAVE\0");
        w.write_int(16);  // version >= 16, unsupported
        let mut reader = BinaryBufferReader::new(&w.into_bytes());
        assert!(handle_load(&mut reader).is_null());

        clean_up();
    }

    #[test]
    fn handle_start_null_sim_data_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        let data = [0u8; 16];
        let mut reader = BinaryBufferReader::new(&data);
        assert!(handle_start(&mut reader).is_null());
        clean_up();
    }

    #[test]
    fn prepare_game_data_update_returns_non_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let gd = GameData::new(2, 2);
        let result = prepare_game_data_update(&gd);
        assert!(!result.is_null(), "PrepareGameDataUpdate must return non-null");

        clean_up();
    }

    #[test]
    fn prepare_game_data_update_fills_soa_pointers() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 创建有数据的 GameData
        let gd = GameData::new(2, 2);
        // 验证 cells 已分配
        assert!(!gd.cells.ptr.is_null());
        let cells = unsafe { &*gd.cells.ptr };
        assert!(!cells.element_idx.is_empty());

        let result = prepare_game_data_update(&gd);
        assert!(!result.is_null());

        // 验证 GDU 的 element_idx 指针非空（因为 cells.element_idx 有 4 个元素）
        let gdu_mutex = globals::G_GAME_DATA_UPDATE.get().expect("GDU should be initialized");
        let gdu = gdu_mutex.lock();
        assert!(!gdu.element_idx.is_null(), "element_idx pointer should be non-null");
        assert!(!gdu.temperature.is_null(), "temperature pointer should be non-null");
        assert!(!gdu.mass.is_null(), "mass pointer should be non-null");
        assert_eq!(gdu.num_frames_processed, 0);

        clean_up();
    }

    #[test]
    fn handle_start_full_flow_returns_gdu_pointer() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 1. AllocateCells (2x2, 含边界 4x4)
        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(2);
        alloc_w.write_int(2);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut alloc_reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let sim_ptr = handle_allocate_cells(&mut alloc_reader);
        assert!(!sim_ptr.is_null());

        // 2. Load (version 13, 2x2)
        let mut load_w = BinaryBufferWriter::new();
        load_w.write_bytes(b"SIMSAVE\0");
        load_w.write_int(13);
        load_w.write_int(2);
        load_w.write_int(2);
        load_w.write_byte(0);
        for _ in 0..4 {
            load_w.write_uint(0x2d39bf75);  // vacuum
            load_w.write_float(300.0);
            load_w.write_float(0.0);
        }
        for _ in 0..4 {
            load_w.write_uint(0);
        }
        for _ in 0..16 {
            load_w.write_byte(0);
        }
        let mut load_reader = BinaryBufferReader::new(&load_w.into_bytes());
        let load_result = handle_load(&mut load_reader);
        assert!(!load_result.is_null());

        // 3. Start
        let start_data = [0u8; 4];
        let mut start_reader = BinaryBufferReader::new(&start_data);
        let start_result = handle_start(&mut start_reader);

        // 关键约束：Start 必须返回真实可解引用的 GDU 指针（非 null）
        assert!(!start_result.is_null(), "Start must return non-null GDU pointer");

        // 验证返回的指针可以安全解引用 element_idx
        let gdu_ptr = start_result as *const crate::a_framework::game_data_update::GameDataUpdate;
        unsafe {
            let _element_idx = (*gdu_ptr).element_idx;
            // 只要能读取 element_idx 而不崩溃就说明指针有效
        }

        clean_up();
    }

    #[test]
    fn begin_save_null_sim_data_returns_null_and_zero_size() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        let mut size: std::os::raw::c_int = -1;
        let result = begin_save(&mut size, 0, 0);
        assert!(result.is_null());
        assert_eq!(size, 0);
        clean_up();
    }

    #[test]
    fn begin_save_null_out_size_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        let result = begin_save(std::ptr::null_mut(), 0, 0);
        assert!(result.is_null());
        clean_up();
    }

    #[test]
    fn begin_save_returns_valid_buffer_with_correct_size() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 先 AllocateCells（2x2，含边界 4x4=16 cells）
        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(2);  // width
        alloc_w.write_int(2);  // height
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let _ = handle_allocate_cells(&mut reader);

        // BeginSave
        let mut size: std::os::raw::c_int = -1;
        let result = begin_save(&mut size, 0, 0);
        assert!(!result.is_null(), "BeginSave should return non-null with valid gSimData");
        // total_cells = 4*4 = 16，total_size = 16*36 + 29 = 605
        assert_eq!(size, 16 * 36 + 29);

        // 验证 magic + version（读取前 12 字节：8B magic + 4B version）
        let header = unsafe { std::slice::from_raw_parts(result as *const u8, 12) };
        assert_eq!(&header[0..8], b"SIMSAVE\0");

        // 验证 version = 0xf
        let version = i32::from_le_bytes([
            header[8], header[9], header[10], header[11]
        ]);
        assert_eq!(version, 15);

        clean_up();
    }

    #[test]
    fn begin_save_converts_ffff_backwall_to_vacuum_hash() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 填最小元素表：真空 id=0x2d39bf75 在索引 211。
        {
            let mut table = crate::b_elements::elements_table::G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.clear();
            table.element_indices.clear();
            table.elements.resize(212, Default::default());
            table.elements[211].id = 0x2d39bf75i32;
            table.element_indices.insert(0x2d39bf75u32, 211u16);
        }

        // AllocateCells（2x2 → sim 4x4 = 16 cells）
        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(2);
        alloc_w.write_int(2);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let _ = handle_allocate_cells(&mut reader);

        // 把 backwall 全部填 0xFFFF
        unsafe {
            let sd = &mut *globals::G_SIM_DATA.lock().0;
            let bw = &mut *sd.backwall.ptr;
            for i in 0..16usize {
                bw.element_idx.set(i, 0xffff);
            }
        }

        let mut size: std::os::raw::c_int = -1;
        let result = begin_save(&mut size, 0, 0);
        assert!(!result.is_null());
        let bytes = unsafe { std::slice::from_raw_parts(result as *const u8, size as usize) };

        // 解析 backwall 段：header 29 + cells 16*16 + disease 16*8 + backwall 16*12
        let bw_start = 29 + 16 * 16 + 16 * 8;
        for i in 0..16usize {
            let off = bw_start + i * 12;
            let hsh = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            assert_eq!(
                hsh, 0x2d39bf75,
                "backwall[{}] should be vacuum hash after 0xffff conversion, got {:#x}",
                i, hsh
            );
        }

        end_save();
        clean_up();
    }

    #[test]
    fn load_disease_8b_per_cell_keeps_backwall_aligned() {
        // 回归测试：太空删除根因——原版 Load 的 disease 段是 8B/格（hash+count）。
        // 若只读 4B/格，背墙段会整体偏移 4B/格，全图背墙读成 0xFFFF。
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 最小元素表：真空 id=0x2d39bf75 → 索引 211。
        {
            let mut table = crate::b_elements::elements_table::G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.clear();
            table.element_indices.clear();
            table.elements.resize(212, Default::default());
            table.elements[211].id = 0x2d39bf75i32;
            table.element_indices.insert(0x2d39bf75u32, 211u16);
        }

        // AllocateCells（2x2 → sim 4x4 = 16 cells）
        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(2);
        alloc_w.write_int(2);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let _ = handle_allocate_cells(&mut reader);

        // 构建 version 15 存档：header 29B + cells 16×16B + disease 16×8B + backwall 16×12B
        let mut w = BinaryBufferWriter::new();
        w.write_bytes(b"SIMSAVE\0");
        w.write_int(15);
        w.write_int(4);
        w.write_int(4);
        w.write_int(0);
        w.write_int(0);
        w.write_byte(0);
        for _ in 0..16u32 {
            w.write_uint(0x2d39bf75); // elem hash = vacuum
            w.write_float(300.0);
            w.write_float(1000.0);
            w.write_float(0.0);
        }
        for i in 0..16u32 {
            w.write_uint(if i % 2 == 0 { 0 } else { 0x1234abcd }); // disease hash
            w.write_int((i * 7) as i32); // disease count（若漏读，会污染背墙段）
        }
        for _ in 0..16u32 {
            w.write_uint(0x2d39bf75); // backwall hash = vacuum
            w.write_float(0.0);
            w.write_float(0.0);
        }
        let data = w.into_bytes();
        let mut load_reader = BinaryBufferReader::new(&data);
        let result = handle_load(&mut load_reader);
        assert!(!result.is_null());

        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            let bw = &*sd.backwall.ptr;
            for i in 0..16usize {
                assert_eq!(
                    bw.element_idx.get(i),
                    211,
                    "backwall[{}] should be vacuum index after aligned load, got {}",
                    i,
                    bw.element_idx.get(i)
                );
            }
        }
        clean_up();
    }


    #[test]
    fn end_save_releases_buffer() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 先 AllocateCells + BeginSave
        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(2);
        alloc_w.write_int(2);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let _ = handle_allocate_cells(&mut reader);

        let mut size: std::os::raw::c_int = -1;
        let _ = begin_save(&mut size, 0, 0);

        // 验证 SaveBuffer 已设置
        assert!(!globals::G_SAVE_BUFFER.lock().0.is_null());

        // EndSave
        end_save();

        // 验证 SaveBuffer 已释放
        assert!(globals::G_SAVE_BUFFER.lock().0.is_null());

        clean_up();
    }

    #[test]
    fn end_save_with_null_buffer_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        end_save(); // 无 SaveBuffer，不应 panic
        clean_up();
    }

    // ===== 任务 5：handle_initialize_from_cells 端到端测试 =====

    /// 构造 4×3 世界的 InitializeFromCells 合成消息（总长 14 + 12×40 = 494 字节）。
    fn build_initialize_from_cells_message() -> Vec<u8> {
        let mut w = BinaryBufferWriter::new();
        w.write_int(4); // width
        w.write_int(3); // height
        w.write_uint(0x12345678); // simSeed
        w.write_bool(true); // radiationEnabled
        w.write_bool(true); // headless
        // 块 1：Sim.Cell × 12（每格 16B）
        for i in 0..12u16 {
            w.write_ushort(100 + i); // elementIdx @0
            w.write_byte(0); // @2 → properties
            w.write_byte(50 + i as u8); // insulation @3
            w.write_byte(0); // @4 → strength_info
            w.write_byte(0);
            w.write_byte(0);
            w.write_byte(0); // pad @5-7
            w.write_float(300.0 + i as f32); // temperature @8
            w.write_float(1000.0 + i as f32); // mass @12
        }
        // 块 2：DiseaseCell × 12（每格 12B）
        for i in 0..12u8 {
            w.write_byte(200 + i); // diseaseIdx @0
            w.write_byte(7); // tick @1
            w.write_byte(0);
            w.write_byte(0); // pad @2-3
            w.write_int(5000 + i as i32); // count @4
            w.write_float(9.9); // accErr @8（应被忽略，写 0）
        }
        // 块 3：SimBackwall × 12（每格 12B）
        for i in 0..12u16 {
            w.write_ushort(30 + i); // elementIdx @0
            w.write_byte(0);
            w.write_byte(0); // pad @2-3
            w.write_float(2000.0 + i as f32); // mass @4
            w.write_float(280.0 + i as f32); // temperature @8
        }
        w.into_bytes()
    }

    #[test]
    fn handle_initialize_from_cells_populates_sim_data() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let data = build_initialize_from_cells_message();
        let mut reader = BinaryBufferReader::new(&data);
        let result = handle_initialize_from_cells(&mut reader);
        assert_eq!(result as usize, 12); // 返回值 = width*height
        unsafe {
            let ptr = globals::G_SIM_DATA.lock().0;
            assert!(!ptr.is_null());
            let sd = &*ptr;
            assert_eq!(sd.width, 6); // 4+2
            assert_eq!(sd.height, 5); // 3+2
            assert_eq!(sd.num_game_cells, 12);
            assert_eq!(sd.random_seed, 0x12345678);
            assert!(sd.radiation_enabled);
            assert!(sd.headless);
            let cells = &*sd.updated_cells.ptr;
            let backwall = &*sd.backwall.ptr;
            for row in 0..3usize {
                for col in 0..4usize {
                    let i = row * 4 + col;
                    let dst = (row + 1) * 6 + (col + 1);
                    assert_eq!(cells.element_idx.get(dst), 100 + i as u16, "element_idx @{}", i);
                    assert_eq!(cells.properties.get(dst), 0);
                    assert_eq!(cells.insulation.get(dst), 50 + i as u8);
                    assert_eq!(cells.strength_info.get(dst), 0);
                    assert_eq!(cells.temperature.get(dst), 300.0 + i as f32);
                    assert_eq!(cells.mass.get(dst), 1000.0 + i as f32);
                    assert_eq!(cells.radiation.get(dst), 0.0);
                    assert_eq!(cells.disease_idx.get(dst), 200 + i as u8);
                    assert_eq!(cells.disease_infestation_tick_count.get(dst), 7);
                    assert_eq!(cells.disease_count.get(dst), 5000 + i as i32);
                    assert_eq!(cells.disease_growth_accumulated_error.get(dst), 0.0);
                    assert_eq!(backwall.element_idx.get(dst), 30 + i as u16);
                    assert_eq!(backwall.mass.get(dst), 2000.0 + i as f32);
                    assert_eq!(backwall.temperature.get(dst), 280.0 + i as f32);
                }
            }
        }
        clean_up();
    }

    #[test]
    fn handle_initialize_from_cells_initializes_boundary() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let data = build_initialize_from_cells_message();
        let mut reader = BinaryBufferReader::new(&data);
        handle_initialize_from_cells(&mut reader);
        unsafe {
            let ptr = globals::G_SIM_DATA.lock().0;
            assert!(!ptr.is_null());
            let sd = &*ptr;
            let unob = elements_table::get_element_index_pub(0x6d95058c);
            let vacuum = elements_table::get_element_index_pub(0x2d39bf75);
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let buf = &*buf_ptr;
                for x in 0..6usize {
                    assert_eq!(buf.element_idx.get(x), unob, "bottom x={}", x);
                    assert_eq!(buf.mass.get(x), 9999.0);
                    let top = 4 * 6 + x;
                    assert_eq!(buf.element_idx.get(top), vacuum, "top x={}", x);
                }
                for y in 1..4usize {
                    assert_eq!(buf.element_idx.get(y * 6), unob, "left y={}", y);
                    assert_eq!(buf.element_idx.get(y * 6 + 5), unob, "right y={}", y);
                }
            }
        }
        clean_up();
    }

    #[test]
    fn handle_load_applies_y_offset_when_placing_world() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 元素表：vacuum=211，测试元素=200
        {
            let mut table = crate::b_elements::elements_table::G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.clear();
            table.element_indices.clear();
            table.elements.resize(212, Default::default());
            table.elements[211].id = 0x2d39bf75i32; // vacuum
            table.element_indices.insert(0x2d39bf75u32, 211u16);
            table.elements[200].id = 0x12345678i32;
            table.element_indices.insert(0x12345678u32, 200u16);
        }

        // AllocateCells(6,6) → sim 8×8（stride=8）
        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(6);
        alloc_w.write_int(6);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let _ = handle_allocate_cells(&mut reader);

        // 预填 updated_cells 为真空，便于断言"未被覆盖"
        unsafe {
            let sd = &mut *globals::G_SIM_DATA.lock().0;
            let cells = &mut *sd.updated_cells.ptr;
            for i in 0..64usize {
                cells.element_idx.set(i, 211);
            }
        }

        // Save：w=3 h=2 x_offset=2 y_offset=3 → start = 3*8+2 = 26
        let mut w = BinaryBufferWriter::new();
        w.write_bytes(b"SIMSAVE\0");
        w.write_int(15);
        w.write_int(3); // save_width
        w.write_int(2); // save_height
        w.write_int(2); // x_offset
        w.write_int(3); // y_offset
        w.write_byte(0);
        for i in 0..6u32 {
            w.write_uint(0x12345678);
            w.write_float(300.0 + i as f32);
            w.write_float(1000.0 + i as f32);
            w.write_float(0.0);
        }
        for _ in 0..6u32 {
            w.write_uint(0);
            w.write_int(0);
        }
        for _ in 0..6u32 {
            w.write_uint(0x2d39bf75);
            w.write_float(0.0);
            w.write_float(0.0);
        }
        let data = w.into_bytes();
        let mut load_reader = BinaryBufferReader::new(&data);
        let result = handle_load(&mut load_reader);
        assert!(!result.is_null());

        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            let cells = &*sd.updated_cells.ptr;
            // 期望：row3 col2..4 = 26,27,28；row4 col2..4 = 34,35,36（y*stride+x）
            let expected = [26usize, 27, 28, 34, 35, 36];
            for (i, &idx) in expected.iter().enumerate() {
                assert_eq!(
                    cells.element_idx.get(idx),
                    200,
                    "cell {} 应在 sim idx {}（y*stride+x）",
                    i,
                    idx
                );
            }
            // 旧 x-only 实现会放到 row0：2,3,4,10,11,12 —— 必须仍是真空
            let wrong = [2usize, 3, 4, 10, 11, 12];
            for &idx in &wrong {
                assert_eq!(
                    cells.element_idx.get(idx),
                    211,
                    "sim idx {} 不应被覆盖（x-only 旧行为）",
                    idx
                );
            }
            // 其余位置保持真空
            for idx in 0..64usize {
                if !expected.contains(&idx) && !wrong.contains(&idx) {
                    assert_eq!(cells.element_idx.get(idx), 211, "sim idx {} 应保持真空", idx);
                }
            }
        }
        clean_up();
    }

    #[test]
    fn handle_initialize_from_cells_rejects_truncated_data() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let mut data = build_initialize_from_cells_message();
        data.truncate(data.len() - 1); // 少 1 字节
        let mut reader = BinaryBufferReader::new(&data);
        let result = handle_initialize_from_cells(&mut reader);
        assert!(result.is_null());
        assert!(globals::G_SIM_DATA.lock().0.is_null(), "旧 gSimData 不应被替换");
        clean_up();
    }

    #[test]
    fn handle_initialize_from_cells_replaces_old_sim_data() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 先经 AllocateCells 放一个旧 SimData
        let mut w = BinaryBufferWriter::new();
        w.write_int(4);
        w.write_int(4);
        w.write_bool(true);
        w.write_bool(true);
        let alloc_data = w.into_bytes();
        let mut alloc_reader = BinaryBufferReader::new(&alloc_data);
        handle_allocate_cells(&mut alloc_reader);
        let old_ptr = globals::G_SIM_DATA.lock().0;
        assert!(!old_ptr.is_null());

        let data = build_initialize_from_cells_message();
        let mut reader = BinaryBufferReader::new(&data);
        handle_initialize_from_cells(&mut reader);
        unsafe {
            let new_ptr = globals::G_SIM_DATA.lock().0;
            assert!(!new_ptr.is_null());
            assert!(new_ptr != old_ptr, "应替换为新 SimData");
            assert_eq!((*new_ptr).width, 6);
        }
        clean_up();
    }

    /// 2026-08-04 审查修正：SetWorldZones 从 A3 桩改为真实实现
    /// （原版 11_msvcrt_ignored.c L24760-24805：分配 width×height + memset 清零 +
    /// 逐行读 (width−2) 字节写入 row×width+1）。
    #[test]
    fn handle_set_world_zones_allocates_and_writes_rows() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let data = build_initialize_from_cells_message();
        let mut reader = BinaryBufferReader::new(&data);
        handle_initialize_from_cells(&mut reader);
        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            assert_eq!(sd.width, 6); // 4+2
            assert_eq!(sd.height, 5); // 3+2
            assert!(sd.world_zones.ptr.is_null(), "初始 world_zones 应为 null");
        }

        // SetWorldZones 消息体：3 行 × 4 列 = 12 字节（width-2=4, height-2=3）
        let wz_data: Vec<u8> = (1..=12).collect();
        let mut wz_reader = BinaryBufferReader::new(&wz_data);
        handle_set_world_zones(&mut wz_reader);

        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            assert!(!sd.world_zones.ptr.is_null(), "world_zones 应已分配");
            for row in 0..3usize {
                for col in 0..4usize {
                    let sim_idx = (row + 1) * 6 + (col + 1);
                    let expected = (row * 4 + col + 1) as u8;
                    let got = std::ptr::read(sd.world_zones.ptr.offset(sim_idx as isize));
                    assert_eq!(got, expected, "wz @row={} col={}", row, col);
                }
            }
            // 边界格保持 0（memset 清零）
            assert_eq!(std::ptr::read(sd.world_zones.ptr), 0, "边界格应为 0");
        }
        clean_up();
    }

    /// 最小元素表：真空 id=0x2d39bf75 → 211，中子 id=0x6d95058c → 0。
    fn init_min_elements_for_dlc_handlers() {
        let mut table = crate::b_elements::elements_table::G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        table.element_indices.clear();
        table.elements.resize(212, Default::default());
        table.elements[211].id = 0x2d39bf75i32;
        table.elements[0].id = 0x6d95058cu32 as i32;
        table.element_indices.insert(0x2d39bf75u32, 211u16);
        table.element_indices.insert(0x6d95058cu32, 0u16);
    }

    #[test]
    fn handle_define_world_offsets_populates_worlds() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        init_min_elements_for_dlc_handlers();

        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(4);
        alloc_w.write_int(4);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let _ = handle_allocate_cells(&mut reader);

        let mut w = BinaryBufferWriter::new();
        w.write_int(2);
        w.write_int(10);
        w.write_int(20);
        w.write_int(100);
        w.write_int(200);
        w.write_int(30);
        w.write_int(40);
        w.write_int(300);
        w.write_int(400);
        let mut reader = BinaryBufferReader::new(&w.into_bytes());
        handle_define_world_offsets(&mut reader);

        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            assert_eq!(sd.worlds.len(), 2);
            let a = sd.worlds.as_slice();
            assert_eq!(a[0].offset_x, 10);
            assert_eq!(a[0].offset_y, 20);
            assert_eq!(a[0].width, 100);
            assert_eq!(a[0].height, 200);
            assert_eq!(a[1].offset_x, 30);
            assert_eq!(a[1].offset_y, 40);
            assert_eq!(a[1].width, 300);
            assert_eq!(a[1].height, 400);
        }
        clean_up();
    }

    #[test]
    fn handle_clear_unoccupied_cells_resets_all_cells_to_vacuum() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        init_min_elements_for_dlc_handlers();

        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(3);
        alloc_w.write_int(2);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let _ = handle_allocate_cells(&mut reader); // sim 5x4 = 20 cells

        unsafe {
            let sd = &mut *globals::G_SIM_DATA.lock().0;
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let cells = &mut *buf_ptr;
                for i in 0..20usize {
                    cells.element_idx.set(i, 7);
                    cells.mass.set(i, 123.0);
                    cells.temperature.set(i, 456.0);
                    cells.disease_count.set(i, 99);
                    cells.disease_idx.set(i, 3);
                    cells.radiation.set(i, 5.0);
                }
            }
        }

        let mut reader = BinaryBufferReader::new(&[]);
        handle_clear_unoccupied_cells(&mut reader);

        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            assert_eq!(sd.vacuum_element_idx, 211);
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let cells = &*buf_ptr;
                for i in 0..20usize {
                    assert_eq!(cells.element_idx.get(i), 211, "elem[{}]", i);
                    assert_eq!(cells.mass.get(i), 0.0);
                    assert_eq!(cells.temperature.get(i), 0.0);
                    assert_eq!(cells.disease_count.get(i), 0);
                    assert_eq!(cells.disease_idx.get(i), 0xff);
                    assert_eq!(cells.radiation.get(i), 0.0);
                }
            }
        }
        clean_up();
    }

    #[test]
    fn handle_resize_and_initialize_vacuum_cells_fills_region() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        init_min_elements_for_dlc_handlers();

        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(10);
        alloc_w.write_int(10);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let _ = handle_allocate_cells(&mut reader); // sim 12x12 = 144 cells

        // grid_size(2 ints, 忽略) + width=4 + height=3 + x_offset=3 + y_offset=2
        let mut w = BinaryBufferWriter::new();
        w.write_int(0);
        w.write_int(0);
        w.write_int(4);
        w.write_int(3);
        w.write_int(3);
        w.write_int(2);
        let mut reader = BinaryBufferReader::new(&w.into_bytes());
        handle_resize_and_initialize_vacuum_cells(&mut reader);

        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            assert_eq!(sd.vacuum_element_idx, 211);
            assert_eq!(sd.unobtanium_element_idx, 0);
            let w = sd.width as usize;
            let cells = &*sd.cells.ptr;
            let bw = &*sd.backwall.ptr;
            // 内部：行 3..=5、列 4..=7 → 真空
            for y in 3..=5usize {
                for x in 4..=7usize {
                    let cell = y * w + x;
                    assert_eq!(cells.element_idx.get(cell), 211, "interior @{}", cell);
                    assert_eq!(cells.mass.get(cell), 0.0);
                    assert_eq!(bw.element_idx.get(cell), 211, "bw interior @{}", cell);
                }
            }
            // 边界：上 2 / 下 6 / 左 3 / 右 8 → 中子 + mass 9999
            for x in 3..=8usize {
                for y in [2usize, 6] {
                    let cell = y * w + x;
                    assert_eq!(cells.element_idx.get(cell), 0, "border @{}", cell);
                    assert_eq!(cells.mass.get(cell), 9999.0);
                }
            }
            for y in 3..=5usize {
                for x in [3usize, 8] {
                    let cell = y * w + x;
                    assert_eq!(cells.element_idx.get(cell), 0, "border col @{}", cell);
                    assert_eq!(cells.mass.get(cell), 9999.0);
                }
            }
        }
        clean_up();
    }

    #[test]
    fn handle_free_grid_cells_clears_region_and_removes_world() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();
        init_min_elements_for_dlc_handlers();

        let mut alloc_w = BinaryBufferWriter::new();
        alloc_w.write_int(10);
        alloc_w.write_int(10);
        alloc_w.write_bool(false);
        alloc_w.write_bool(false);
        let mut reader = BinaryBufferReader::new(&alloc_w.into_bytes());
        let _ = handle_allocate_cells(&mut reader); // sim 12x12 = 144 cells

        // 先塞两个世界条目
        unsafe {
            let sd = &mut *globals::G_SIM_DATA.lock().0;
            sd.worlds.push(WorldOffsetData { offset_x: 3, offset_y: 2, width: 4, height: 3 });
            sd.worlds.push(WorldOffsetData { offset_x: 9, offset_y: 9, width: 1, height: 1 });
        }
        // 在区域里放点数据
        unsafe {
            let sd = &mut *globals::G_SIM_DATA.lock().0;
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let cells = &mut *buf_ptr;
                for i in 0..144usize {
                    cells.element_idx.set(i, 5);
                    cells.mass.set(i, 77.0);
                }
            }
        }

        // FreeGridCells：width=4 height=3 x_offset=3 y_offset=2
        let mut w = BinaryBufferWriter::new();
        w.write_int(4);
        w.write_int(3);
        w.write_int(3);
        w.write_int(2);
        let mut reader = BinaryBufferReader::new(&w.into_bytes());
        handle_free_grid_cells(&mut reader);

        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            // worlds 移除匹配条目，保留另一个
            assert_eq!(sd.worlds.len(), 1);
            let a = sd.worlds.as_slice();
            assert_eq!(a[0].offset_x, 9);
            assert_eq!(a[0].offset_y, 9);
            // 矩形（行 2..=6、列 3..=8，含边界一圈）清为真空
            let w = sd.width as usize;
            let cells = &*sd.cells.ptr;
            for y in 2..=6usize {
                for x in 3..=8usize {
                    let cell = y * w + x;
                    assert_eq!(cells.element_idx.get(cell), 211, "cleared @{}", cell);
                    assert_eq!(cells.mass.get(cell), 0.0);
                }
            }
            // 区域外保持原值
            let outside = 0 * w + 0;
            assert_eq!(cells.element_idx.get(outside), 5);
        }
        clean_up();
    }

    // ===== 任务 2：CreateDiseaseTable handler 挂接 =====

    /// 按 C# SimMessages.CreateDiseaseTable 写入顺序构造病菌表消息。
    fn make_disease_table_message(count: i32, elements_count: i32, hash: u32) -> Vec<u8> {
        let mut w = BinaryBufferWriter::new();
        w.write_int(count);
        w.write_int(elements_count);
        for _ in 0..count {
            w.write_int(4); // KleiString 长度
            w.write_bytes(b"Test");
            w.write_uint(hash);
            w.write_float(2.0); // strength
            for v in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0,
                      9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0] {
                w.write_float(v);
            }
            w.write_float(0.5); // radiationKillRate
            for i in 0..elements_count {
                w.write_float(0.1 + i as f32);
                w.write_float(0.2 + i as f32);
                w.write_float(0.3 + i as f32);
                w.write_float(0.4 + i as f32);
                w.write_float(0.5 + i as f32);
                w.write_float(0.6 + i as f32);
                w.write_int(7 + i);
                w.write_byte(8 + i as u8);
            }
        }
        w.into_bytes()
    }

    #[test]
    fn handle_create_disease_table_sets_global() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let data = make_disease_table_message(1, 0, 0x11223344);
        let mut reader = BinaryBufferReader::new(&data);
        let result = handle_create_disease_table(&mut reader);
        assert!(result.is_null(), "原版 CreateDiseaseTable 返回 null");

        let g = globals::G_DISEASE.lock().0;
        assert!(!g.is_null(), "gDisease 应已挂接");
        let d = unsafe { &*g };
        assert_eq!(d.diseases.len(), 1);
        assert_eq!(d.diseases.as_slice()[0].hash_id, 0x11223344);
        assert_eq!(d.diseases.as_slice()[0].strength, 2.0);
        assert_eq!(d.disease_names.len(), 1);

        clean_up();
        assert!(globals::G_DISEASE.lock().0.is_null(), "clean_up 应释放 gDisease");
    }

    #[test]
    fn handle_create_disease_table_replaces_old() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let data1 = make_disease_table_message(1, 0, 0x11111111);
        let mut reader1 = BinaryBufferReader::new(&data1);
        handle_create_disease_table(&mut reader1);
        let old_ptr = globals::G_DISEASE.lock().0;
        assert!(!old_ptr.is_null());

        let data2 = make_disease_table_message(2, 0, 0x22222222);
        let mut reader2 = BinaryBufferReader::new(&data2);
        let result = handle_create_disease_table(&mut reader2);
        assert!(result.is_null());

        let new_ptr = globals::G_DISEASE.lock().0;
        assert!(!new_ptr.is_null());
        let d = unsafe { &*new_ptr };
        // 内容必须来自第二张表（旧表 1 条 / 新表 2 条，hash 不同）：
        // 注意：分配器可能复用旧表地址，不能断言指针不同。
        assert_eq!(d.diseases.len(), 2, "应替换为新表");
        assert_eq!(d.diseases.as_slice()[0].hash_id, 0x22222222);

        clean_up();
    }

    #[test]
    fn handle_create_disease_table_truncated_returns_null_and_keeps_old() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 先挂一张有效表
        let data1 = make_disease_table_message(1, 0, 0x33333333);
        let mut reader1 = BinaryBufferReader::new(&data1);
        handle_create_disease_table(&mut reader1);
        let old_ptr = { globals::G_DISEASE.lock().0 };

        // 截断消息 → 解析失败 → 返回 null，旧表保留
        let mut data2 = make_disease_table_message(1, 0, 0x44444444);
        data2.truncate(data2.len() - 8);
        let mut reader2 = BinaryBufferReader::new(&data2);
        let result = handle_create_disease_table(&mut reader2);
        assert!(result.is_null(), "截断消息应返回 null");
        // 注意：不要用 `let d = unsafe { &*globals::G_DISEASE.lock().0 };` ——
        // let 绑定引用会延长临时 MutexGuard 的生命周期到函数结束，
        // 末尾 clean_up 同线程重锁 G_DISEASE 会死锁。必须显式作用域持锁。
        let ptr_now = { globals::G_DISEASE.lock().0 };
        assert_eq!(ptr_now, old_ptr, "失败时不应替换旧表");
        let hash_now = {
            let guard = globals::G_DISEASE.lock();
            let d = unsafe { &*guard.0 };
            d.diseases.as_slice()[0].hash_id
        };
        assert_eq!(hash_now, 0x33333333);

        clean_up();
    }

    #[test]
    fn clean_up_after_create_disease_table_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        let data = make_disease_table_message(3, 2, 0x55555555);
        let mut reader = BinaryBufferReader::new(&data);
        handle_create_disease_table(&mut reader);
        assert!(!globals::G_DISEASE.lock().0.is_null());

        clean_up(); // 替换后 clean_up 释放 Box，不应崩溃
        clean_up(); // 幂等
        assert!(globals::G_DISEASE.lock().0.is_null());
    }

    #[test]
    fn handle_set_saved_options_clear_and_set_bits() {
        let _lock = LIB_TESTS_LOCK.lock();
        clean_up();

        // 先经 AllocateCells 建 SimData
        let mut w = BinaryBufferWriter::new();
        w.write_int(4);
        w.write_int(4);
        w.write_bool(true);
        w.write_bool(true);
        let alloc_data = w.into_bytes();
        let mut alloc_reader = BinaryBufferReader::new(&alloc_data);
        handle_allocate_cells(&mut alloc_reader);
        unsafe {
            let sd = &mut *globals::G_SIM_DATA.lock().0;
            sd.saved_options = 0x03;
        }

        // 2B 消息 {clearBits=0x01, setBits=0x02} → (0x03 & ~0x01) | 0x02 = 0x02
        let data = [0x01u8, 0x02u8];
        let mut reader = BinaryBufferReader::new(&data);
        let result = handle_set_saved_options(&mut reader);
        assert!(result.is_null());
        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            assert_eq!(sd.saved_options, 0x02, "clear bit0 + set bit1");
        }

        // 再清 bit1 + 置 bit0 → (0x02 & ~0x02) | 0x01 = 0x01
        let data2 = [0x02u8, 0x01u8];
        let mut reader2 = BinaryBufferReader::new(&data2);
        handle_set_saved_options(&mut reader2);
        unsafe {
            let sd = &*globals::G_SIM_DATA.lock().0;
            assert_eq!(sd.saved_options, 0x01);
        }

        clean_up();
    }
}
