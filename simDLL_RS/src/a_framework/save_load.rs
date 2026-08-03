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
use crate::a_framework::sim_data::{CellSOA, SimData};
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
                let _ = Box::from_raw(sim.0);
            }
            sim.0 = std::ptr::null_mut();
        }
    }

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
    // L545-551: if !_finite(temperature) → 300.0 (0x43928000)
    if !temperature.is_finite() {
        temperature = 300.0f32;
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
/// - if version > 13: 4B offset (int) + 4B skip (int)
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

    // L38-42: if version > 13, 读取 offset + skip
    let mut offset: i32 = 0;
    if version > 0xd {
        offset = reader.read_int().unwrap_or(0);
        let _ = reader.read_int(); // skip
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
        offset,
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

    // L53-114: cell 读取循环
    // 对照源码 L61: CellRead(*(CellSOA **)(gSimData + 0x28), ...) — 读取到 updated_cells
    if save_height > 0 && save_width > 0 {
        let cells_ptr = sim_data.updated_cells.ptr;
        if cells_ptr.is_null() {
            tracing::error!("Load: updated_cells is null");
            return std::ptr::null_mut();
        }
        let cells = unsafe { &mut *cells_ptr };

        let start = offset as i64;
        for row in 0..save_height as i64 {
            let row_start = start + row * (stride as i64);
            for col in 0..save_width as i64 {
                let cell_idx = (row_start + col) as usize;

                // CellRead
                if let Err(e) = cell_read(cells, cell_idx, reader, version) {
                    tracing::error!(?e, cell_idx, "Load: CellRead failed");
                    return std::ptr::null_mut();
                }

                // L62-68: if temperature <= 0 → 300.0
                let temp = cells.temperature.get(cell_idx);
                if temp <= 0.0 {
                    cells.temperature.set(cell_idx, 300.0);
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
            let row_start = offset as i64 + row * (stride as i64);
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
                let row_start = offset as i64 + row * (stride as i64);
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

    // L193-195: if version < 15, skip width × height × 4 字节（旧版辐射数据）
    if version < 0xf && save_height > 0 && save_width > 0 {
        let skip_bytes = (save_height as usize) * (save_width as usize) * 4;
        let _ = reader.skip(skip_bytes);
    }

    // L196-198: if version < 11, 设置 init_settle_thermal_boundaries = true
    if version < 0xb {
        sim_data.init_settle_thermal_boundaries = true;
    }

    tracing::info!(version, save_width, save_height, offset, "Load done");
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

    // L249: SimData::InitializeBoundary（A3 阶段桩，C 类阶段实现）
    // sim_data.initialize_boundary();
    tracing::debug!("Start: InitializeBoundary skipped (A3 stub)");

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

/// DefineWorldOffsets handler 桩。
/// 对照源码 02_save_load.c L297-342。
/// 任务 4 实现真实逻辑。
pub fn handle_define_world_offsets(_reader: &mut BinaryBufferReader) -> *mut c_void {
    tracing::warn!("DefineWorldOffsets handler stub (task 4 will implement)");
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
pub fn handle_prepare_game_data(_reader: &mut BinaryBufferReader) -> *mut c_void {
    let fs_ptr = crate::a_framework::frame_sync::FrameSync::get_ptr();
    if fs_ptr.is_null() {
        tracing::error!("PrepareGameData: G_FRAME_SYNC not initialized");
        return std::ptr::null_mut();
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

/// ClearUnoccupiedCells handler 桩。
/// 对照源码 02_save_load.c L405-465。
/// 任务 4 实现真实逻辑。
pub fn handle_clear_unoccupied_cells(_reader: &mut BinaryBufferReader) -> *mut c_void {
    tracing::warn!("ClearUnoccupiedCells handler stub (task 4 will implement)");
    std::ptr::null_mut()
}

/// SetSavedOptions handler 桩。
/// 对照源码 02_save_load.c L466-484。
/// 任务 4 实现真实逻辑。
pub fn handle_set_saved_options(_reader: &mut BinaryBufferReader) -> *mut c_void {
    tracing::warn!("SetSavedOptions handler stub (task 4 will implement)");
    std::ptr::null_mut()
}

/// ResizeAndInitializeVacuumCells handler 桩。
/// 对照源码 02_save_load.c L485-493。
/// 任务 4 实现真实逻辑。
pub fn handle_resize_and_initialize_vacuum_cells(_reader: &mut BinaryBufferReader) -> *mut c_void {
    tracing::warn!("ResizeAndInitializeVacuumCells handler stub (task 4 will implement)");
    std::ptr::null_mut()
}

/// CreateDiseaseTable handler 桩。
/// 对照源码 02_save_load.c L494-518。
/// 任务 4 实现真实逻辑。
pub fn handle_create_disease_table(_reader: &mut BinaryBufferReader) -> *mut c_void {
    tracing::warn!("CreateDiseaseTable handler stub (task 4 will implement)");
    std::ptr::null_mut()
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
                cells.strength_info.set(dst, reader.read_u8_at(src + 4).unwrap_or(0));
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
        sim_data.settle_thermal_boundaries();
        sim_data.initialize_boundary();
    }

    tracing::info!("InitializeFromCells done: {}x{} ({} cells), seed={:#010x}", width, height, num_cells, sim_seed);
    num_cells as *mut c_void
}

/// SimData_FreeCells handler 桩。
/// 对照源码 11_msvcrt_ignored.c L23727。
/// A3 阶段返回 null（不处理），C 类阶段实现真实逻辑。
pub fn handle_free_grid_cells(_reader: &mut BinaryBufferReader) -> *mut c_void {
    tracing::debug!("FreeGridCells handler stub (A3)");
    std::ptr::null_mut()
}

/// SetWorldZones handler 桩。
/// 对照源码 11_msvcrt_ignored.c L24760。
/// A3 阶段返回 null（不处理），C 类阶段实现真实逻辑。
pub fn handle_set_world_zones(_reader: &mut BinaryBufferReader) -> *mut c_void {
    tracing::debug!("SetWorldZones handler stub (A3)");
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

    // L57: cells = gSimData->cells
    let cells_ptr = sim_data.cells.ptr;
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();
    }

    #[test]
    fn clean_up_with_null_globals_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();
        clean_up();
    }

    #[test]
    fn handle_allocate_cells_returns_non_null() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
    fn handle_load_invalid_magic_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();

        let data = [0u8; 32]; // 全零，magic 不匹配
        let mut reader = BinaryBufferReader::new(&data);
        assert!(handle_load(&mut reader).is_null());

        clean_up();
    }

    #[test]
    fn handle_load_null_sim_data_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();
        let data = [0u8; 16];
        let mut reader = BinaryBufferReader::new(&data);
        assert!(handle_start(&mut reader).is_null());
        clean_up();
    }

    #[test]
    fn prepare_game_data_update_returns_non_null() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();

        let gd = GameData::new(2, 2);
        let result = prepare_game_data_update(&gd);
        assert!(!result.is_null(), "PrepareGameDataUpdate must return non-null");

        clean_up();
    }

    #[test]
    fn prepare_game_data_update_fills_soa_pointers() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();
        let mut size: std::os::raw::c_int = -1;
        let result = begin_save(&mut size, 0, 0);
        assert!(result.is_null());
        assert_eq!(size, 0);
        clean_up();
    }

    #[test]
    fn begin_save_null_out_size_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
        clean_up();
        let result = begin_save(std::ptr::null_mut(), 0, 0);
        assert!(result.is_null());
        clean_up();
    }

    #[test]
    fn begin_save_returns_valid_buffer_with_correct_size() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
    fn end_save_releases_buffer() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
    fn handle_initialize_from_cells_rejects_truncated_data() {
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
        let _lock = LIB_TESTS_LOCK.lock().unwrap();
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
}
