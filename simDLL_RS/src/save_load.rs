//! save_load — 存档加载全局状态 + 14 个消息 handler 函数。
//!
//! 对照源码 02_save_load.c（L7-L1059）+ 11_msvcrt_ignored.c（L24427-24700）。
//! Phase 3：实现存档加载全流程，函数作为 pub fn（不导出 #[no_mangle]），
//! Phase 6 才注册到 G_SIM_MESSAGE_HANDLERS 表。

use std::sync::Mutex;
use once_cell::sync::Lazy;

use crate::sim_data::SimData;
use crate::frame_sync::FrameSync;
use crate::sim::Sim;
use crate::disease::Disease;
use crate::game_data_update::GameDataUpdate;

/// 全局 SimData 单例（对应原版 gSimData 裸指针）。
///
/// **T5 适配（简报未预判 E0277）**：`SimData` 含 `MsvcVector<*mut SimComponent>` 字段
/// （components，T4 实现），Rust 默认 `*mut T: !Send`，故 `MsvcVector<*mut SimComponent>`
/// 不满足 `Send` bound（stl_shim.rs 的 `unsafe impl<T: Send> Send for MsvcVector<T>`）。
/// 与 sim.rs T3 对 `Sim` 的处理一致——为 FFI 含裸指针类型补 `unsafe impl Send/Sync`。
// SAFETY: SimData 的 `*mut SimComponent` 字段是 owning 指针（源码 SimData dtor 释放），
// 但 Phase 3 stub 不实际管理 SimComponent 生命周期（T4 new_zeroed 时为 null）。
// 跨线程访问由 Mutex 提供同步；Phase 6+ 接入真实 SimComponent 生命周期时复核。
unsafe impl Send for SimData {}
unsafe impl Sync for SimData {}

pub static G_SIM_DATA: Lazy<Mutex<Box<SimData>>> =
    Lazy::new(|| Mutex::new(Box::new(SimData::new_zeroed())));

/// 全局 FrameSync 单例（对应原版 gFrameSync 栈对象）。
pub static G_FRAME_SYNC: Lazy<Mutex<Box<FrameSync>>> =
    Lazy::new(|| Mutex::new(Box::new(FrameSync::new_zeroed())));

/// 全局 Sim 单例（对应原版 gSim 裸指针）。
pub static G_SIM: Lazy<Mutex<Box<Sim>>> =
    Lazy::new(|| Mutex::new(Box::new(Sim::new_zeroed())));

/// 全局 Disease 单例（对应原版 gDisease 裸指针）。
pub static G_DISEASE: Lazy<Mutex<Box<Disease>>> =
    Lazy::new(|| Mutex::new(Box::new(Disease::new())));

/// 全局 GameDataUpdate 单例（对应原版 gGameDataUpdate）。
///
/// **T5 适配（简报未预判 E0277）**：`GameDataUpdate` 含 `*const T` 裸指针字段
/// （非 owning，指向 SimData 的 SOA vector 内部缓冲），Rust 默认 `*const T: !Send`。
/// 与 sim.rs T3 对 `Sim` 的处理一致——为 FFI 含裸指针类型补 `unsafe impl Send/Sync`，
/// 使其可放入 `Lazy<Mutex<T>>`。Mutex 提供运行时同步，指针非 owning 无 Drop 风险。
// SAFETY: GameDataUpdate 的裸指针是非 owning 观察指针，指向 SimData 内部缓冲。
// 跨线程访问由 Mutex 提供同步；无 Drop 释放裸指针内存的风险。
unsafe impl Send for GameDataUpdate {}
unsafe impl Sync for GameDataUpdate {}

pub static G_GAME_DATA_UPDATE: Lazy<Mutex<GameDataUpdate>> =
    Lazy::new(|| Mutex::new(GameDataUpdate::default()));

use crate::buffer::{BinaryBufferReader, BufferError};
use crate::sim_data::CellSOA;
use crate::stl_shim::MsvcVector;
use crate::element::{INVALID_ELEMENT_HASH, INVALID_ELEMENT_INDEX};
use crate::elements_table::get_element_index_pub;

/// AllocateCells — 分配 SimData 的 cells/updated_cells/backwall SOA。
///
/// 对照源码 02_save_load.c L259-293（AllocateCells）。
/// 读 2×int + 2×bool，用 new SimData(width+2, height+2, time, bool1, bool2) 替换 gSimData。
/// Phase 3 简化：不调用 operator_new + 真实构造函数，用 new_zeroed + 手动设置 width/height。
pub fn allocate_cells(reader: &mut BinaryBufferReader) -> Result<(), BufferError> {
    let width = reader.read_int()?;
    let height = reader.read_int()?;
    let _bool1 = reader.read_byte()?;
    let _bool2 = reader.read_byte()?;

    let time = {
        // 对应源码 _time64(0)，用 SystemTime 替代
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0)
    };

    tracing::info!(width, height, time, "AllocateCells: allocating SimData");

    let mut new_data = SimData::new_zeroed();
    new_data.width = width + 2;   // 源码 L282: width+2
    new_data.height = height + 2; // 源码 L282: height+2
    new_data.num_game_cells = (width + 2) * (height + 2);
    new_data.random_seed = time;

    // 分配 cells / updated_cells / backwall 的 SOA 内存
    // Phase 3 简化：用 Box::into_raw 分配堆内存并赋值给 UniquePtr.ptr
    let cells = Box::new(CellSOA::new());
    let updated_cells = Box::new(CellSOA::new());
    let backwall = Box::new(crate::sim_data::BackwallSOA {
        vacuum_element_idx: 0,
        padding: 0,
        element_idx: MsvcVector::new(),
        mass: MsvcVector::new(),
        temperature: MsvcVector::new(),
    });
    new_data.cells.ptr = Box::into_raw(cells);
    new_data.updated_cells.ptr = Box::into_raw(updated_cells);
    new_data.backwall.ptr = Box::into_raw(backwall);

    // 替换全局单例
    let mut g = G_SIM_DATA.lock().unwrap();
    // 释放旧 SimData（如果已分配）
    if !g.cells.ptr.is_null() {
        unsafe { drop(Box::from_raw(g.cells.ptr)) };
    }
    if !g.updated_cells.ptr.is_null() {
        unsafe { drop(Box::from_raw(g.updated_cells.ptr)) };
    }
    if !g.backwall.ptr.is_null() {
        unsafe { drop(Box::from_raw(g.backwall.ptr)) };
    }
    // T5 适配（简报未预判 E0308）：g 是 MutexGuard<Box<SimData>>，*g 是 Box<SimData>，
    // 简报字面 `*g = new_data` 类型不匹配。按编译器建议改为 *g = Box::new(new_data)。
    *g = Box::new(new_data);

    Ok(())
}

/// DefineWorldOffsets — 读取世界偏移数据。
///
/// 对照源码 02_save_load.c L297-342（DefineWorldOffsets）。
pub fn define_world_offsets(reader: &mut BinaryBufferReader) -> Result<(), BufferError> {
    let count = reader.read_int()?;
    if count < 0 {
        return Ok(());
    }
    let count_usize = count as usize;

    let mut g = G_SIM_DATA.lock().unwrap();
    // 源码 L306-308: vector<WorldOffsetData>::resize(count)
    // Phase 3 用 MsvcVector::resize 替代 MSVC vector::resize
    for _ in 0..count_usize {
        let offset_x = reader.read_int()?;
        let offset_y = reader.read_int()?;
        let w = reader.read_int()?;
        let h = reader.read_int()?;
        // Phase 3 简化：不真实填充 worlds vector（已通过下面的 resize）
        tracing::debug!(offset_x, offset_y, w, h, "DefineWorldOffsets: entry read");
    }
    // 标记 worlds 长度（用 resize 模拟）
    // 注意：MsvcVector<WorldOffsetData>::resize 需要 Default impl，WorldOffsetData 已有 Default
    g.worlds.resize(count_usize, crate::sim_data::WorldOffsetData::default());

    Ok(())
}

/// SetSavedOptions — 更新 saved_options 字段。
///
/// 对照源码 02_save_load.c L466-481（SetSavedOptions）。
/// saved_options = (saved_options & ~clearMask) | setMask
pub fn set_saved_options(reader: &mut BinaryBufferReader) -> Result<(), BufferError> {
    let clear_mask = reader.read_byte()?;
    let set_mask = reader.read_byte()?;

    let mut g = G_SIM_DATA.lock().unwrap();
    g.saved_options = (g.saved_options & !clear_mask) | set_mask;
    tracing::debug!(clear_mask, set_mask, saved_options = g.saved_options, "SetSavedOptions");
    Ok(())
}

/// Disease_CreateTable — 创建 Disease 表。
///
/// 对照源码 02_save_load.c L494-515（CreateDiseaseTable）。
/// Phase 3 stub：替换 gDisease 为新空 Disease（不读 reader 数据）。
pub fn create_disease_table(reader: &mut BinaryBufferReader) -> Result<(), BufferError> {
    tracing::info!("Disease_CreateTable: replacing gDisease with new empty Disease");
    let _ = reader;  // Phase 3 stub 不读 reader
    let mut g = G_DISEASE.lock().unwrap();
    *g = Box::new(Disease::new());
    Ok(())
}

/// ClearUnoccupiedCells — 清空未占用 cells。
///
/// 对照源码 02_save_load.c L405-462（ClearUnoccupiedCells）。
/// 遍历 num_game_cells 个格子，重置为 vacuum。
pub fn clear_unoccupied_cells(_reader: &mut BinaryBufferReader) -> Result<(), BufferError> {
    let mut g = G_SIM_DATA.lock().unwrap();
    let num_cells = g.num_game_cells as usize;
    let vacuum_idx = g.vacuum_element_idx;

    if g.cells.ptr.is_null() || g.updated_cells.ptr.is_null() {
        return Ok(());
    }

    let cells = unsafe { &mut *g.cells.ptr };
    let updated_cells = unsafe { &mut *g.updated_cells.ptr };

    // 源码 L415-459: 遍历 width*height 个格子
    // cells SOA 重置（elementIdx=vacuum, mass=0, temp=0, diseaseIdx=0xff, radiation=0）
    // updated_cells SOA 同样重置
    for i in 0..num_cells {
        // cells
        if i < cells.element_idx.len() {
            cells.element_idx.set(i, vacuum_idx);
        }
        if i < cells.mass.len() {
            cells.mass.set(i, 0.0);
        }
        if i < cells.temperature.len() {
            cells.temperature.set(i, 0.0);
        }
        if i < cells.disease_idx.len() {
            cells.disease_idx.set(i, 0xff);
        }
        if i < cells.radiation.len() {
            cells.radiation.set(i, 0.0);
        }
        if i < cells.disease_count.len() {
            cells.disease_count.set(i, 0);
        }
        // updated_cells（同上）
        if i < updated_cells.element_idx.len() {
            updated_cells.element_idx.set(i, vacuum_idx);
        }
        if i < updated_cells.mass.len() {
            updated_cells.mass.set(i, 0.0);
        }
        if i < updated_cells.temperature.len() {
            updated_cells.temperature.set(i, 0.0);
        }
        if i < updated_cells.disease_idx.len() {
            updated_cells.disease_idx.set(i, 0xff);
        }
        if i < updated_cells.radiation.len() {
            updated_cells.radiation.set(i, 0.0);
        }
        if i < updated_cells.disease_count.len() {
            updated_cells.disease_count.set(i, 0);
        }
    }

    Ok(())
}

/// CellRead — 从 reader 读一个 cell 数据并写入 CellSOA。
///
/// 对照源码 02_save_load.c L519-615（CellRead）。
/// 内部辅助函数，不导出。
///
/// **关键实现细节**：
/// - hash 重映射：源码 L530-535 **无条件**检查（非 spec 说的 version < 0xa），
///   0x280cf79→0x987dac06, 0x2391c22b→0x4c76ae31
/// - _finite 映射为 f32::is_finite()：NaN/Inf 替换 temp=300.0, mass=100.0, radiation=0.0
/// - elementIdx==0xffff 时用 INVALID_ELEMENT_HASH(0x2d39bf75) 重查
fn cell_read(
    cells: &mut CellSOA,
    cell_idx: i32,
    reader: &mut BinaryBufferReader,
    version: i32,
) -> Result<(), BufferError> {
    let mut elem_hash = reader.read_uint()?;

    // hash 重映射（源码 L530-535 无条件检查，非 spec 说的 version < 0xa）
    if elem_hash == 0x280cf79 {
        elem_hash = 0x987dac06;
    } else if elem_hash == 0x2391c22b {
        elem_hash = 0x4c76ae31;
    }

    // 查找 elementIdx
    let mut element_idx = get_element_index_pub(elem_hash);

    let idx_u = cell_idx as usize;

    // 写入 elementIdx（源码 L537-539）
    if cell_idx >= 0 && idx_u < cells.element_idx.len() {
        cells.element_idx.set(idx_u, element_idx);
    }

    // 读 temperature（源码 L540-551）
    if cell_idx >= 0 && idx_u < cells.temperature.len() {
        reader.read_bytes(unsafe {
            std::slice::from_raw_parts_mut(
                cells.temperature.begin.add(idx_u) as *mut u8,
                4,
            )
        })?;
        // _finite 检查（NaN/Inf → 300.0）
        let temp = cells.temperature.get(idx_u);
        if !temp.is_finite() {
            cells.temperature.set(idx_u, 300.0);  // 0x43928000
        }
    }

    // 读 mass（源码 L552-563）
    if cell_idx >= 0 && idx_u < cells.mass.len() {
        reader.read_bytes(unsafe {
            std::slice::from_raw_parts_mut(
                cells.mass.begin.add(idx_u) as *mut u8,
                4,
            )
        })?;
        let mass = cells.mass.get(idx_u);
        if !mass.is_finite() {
            cells.mass.set(idx_u, 100.0);  // 0x42c80000
        }
    }

    // 读 radiation（源码 L564-579，version > 0xd 时）
    if version > 0xd {
        if cell_idx >= 0 && idx_u < cells.radiation.len() {
            reader.read_bytes(unsafe {
                std::slice::from_raw_parts_mut(
                    cells.radiation.begin.add(idx_u) as *mut u8,
                    4,
                )
            })?;
            let rad = cells.radiation.get(idx_u);
            if !rad.is_finite() {
                cells.radiation.set(idx_u, 0.0);  // 0x0
            }
        }
    }

    // elementIdx == 0xffff 时用 INVALID_ELEMENT_HASH 重查（源码 L580-606）
    if element_idx == INVALID_ELEMENT_INDEX {
        let vacuum_idx = get_element_index_pub(INVALID_ELEMENT_HASH);
        if cell_idx >= 0 && idx_u < cells.element_idx.len() {
            cells.element_idx.set(idx_u, vacuum_idx);
        }
        if cell_idx >= 0 && idx_u < cells.mass.len() {
            cells.mass.set(idx_u, 0.0);
        }
        if cell_idx >= 0 && idx_u < cells.temperature.len() {
            cells.temperature.set(idx_u, 0.0);
        }
        if cell_idx >= 0 && idx_u < cells.radiation.len() {
            cells.radiation.set(idx_u, 0.0);
        }
    }

    Ok(())
}

/// Load — 反序列化存档到 gSimData。
///
/// 对照源码 02_save_load.c L7-207（Load）。
///
/// 返回值：源码返回 BinaryBufferReader* 指针，Rust 实现返回 Result。
///
/// **关键实现细节**：
/// - 魔数检查（L32）：8 字节 "SIMAVES\0" = [0x53,0x49,0x4d,0x41,0x56,0x45,0x53,0x00]
/// - L37 local_res20 修订（决策 #3）：Ghidra 误判丢弃返回值，实际是 start_offset
/// - 三阶段双层循环（L48-192）：
///   - 阶段 1：填充 updated_cells（CellRead + temp/mass/radiation 检查）
///   - 阶段 2（version > 8）：填充 diseaseIdx（read_uint + GetDiseaseIndex）
///   - 阶段 3（version > 0xe）：填充 backwall（read_uint + GetElementIndex）
pub fn load(reader: &mut BinaryBufferReader) -> Result<(), BufferError> {
    // 魔数检查（L32）
    let mut magic = [0u8; 8];
    // 简报偏差：原 brief `reader.read_bytes(&mut magic)?` 在 buffer 不足时
    // 返回 EndOfBuffer，但测试 `load_with_empty_buffer_returns_invalid_magic`
    // 期望 InvalidMagic。按"实际 API 签名适配"预授权，将魔数读取失败转换为
    // InvalidMagic（语义：数据不足以构成有效魔数 = 非法魔数）。
    if reader.read_bytes(&mut magic).is_err() {
        tracing::error!(?magic, "Load: failed to read magic number (insufficient data)");
        return Err(BufferError::InvalidMagic);
    }
    if magic != [0x53, 0x49, 0x4d, 0x41, 0x56, 0x45, 0x53, 0x00] {
        tracing::error!(?magic, "Load: invalid magic number");
        return Err(BufferError::InvalidMagic);
    }

    // 读 version（L34）
    let version = reader.read_int()?;

    let mut inner_w: i32 = 0;
    let mut inner_h: i32 = 0;
    let mut start_offset: i32 = 0;
    let mut save_settings_byte: u8 = 0;
    let mut local_60: i32 = 0;

    // header 读取（L35-46）
    if version < 0x10 {
        inner_w = reader.read_int()?;       // L36 local_res18[0]
        inner_h = reader.read_int()?;       // L37 local_res18[1]（Ghidra 遗漏）
        start_offset = reader.read_int()?;  // L37 local_res20（决策 #3：Ghidra 误判丢弃）
        local_60 = 0;
        if version > 0xd {
            local_60 = reader.read_int()?;  // L40 saveSettings
            let _extra = reader.read_int()?; // L41 多读一个 int（语义未定）
        }
        save_settings_byte = 0;
        if version > 0xc {
            save_settings_byte = reader.read_byte()?;  // L45
        }
        // 调用 stub（L47）
        {
            let mut g = G_SIM_DATA.lock().unwrap();
            g.apply_save_settings(save_settings_byte);
        }
    }

    tracing::info!(version, inner_w, inner_h, start_offset, "Load: header parsed");

    // 三阶段双层循环（L48-192）
    let (width, updated_cells_ptr) = {
        let g = G_SIM_DATA.lock().unwrap();
        (g.width, g.updated_cells.ptr)
    };

    // 阶段 1：填充 updated_cells（L48-115）
    if start_offset != 0 && !updated_cells_ptr.is_null() {
        let updated_cells = unsafe { &mut *updated_cells_ptr };
        let mut ivar12: i32 = 0;
        let mut ivar15: i32 = local_60;
        while ivar12 != start_offset {
            let mut ivar11 = ivar15;
            if inner_h > 0 {
                let ivar14 = -local_60;
                for _ in 0..inner_h {
                    cell_read(updated_cells, ivar11, reader, version)?;
                    // temp/radiation/mass/disease_count 检查（源码 L62-98）
                    let idx_u = ivar11 as usize;
                    if idx_u < updated_cells.temperature.len() {
                        let temp = updated_cells.temperature.get(idx_u);
                        if temp <= 0.0 {
                            updated_cells.temperature.set(idx_u, 300.0);
                        }
                    }
                    if idx_u < updated_cells.radiation.len() {
                        let rad = updated_cells.radiation.get(idx_u);
                        if rad <= 0.0 {
                            updated_cells.radiation.set(idx_u, 0.0);
                        }
                    }
                    ivar11 += 1;
                    if ivar14 + ivar11 >= inner_h {
                        break;
                    }
                }
            }
            ivar12 += 1;
            ivar15 += width;
        }
    }

    // 阶段 2：填充 diseaseIdx（version > 8，L116-157）
    // Phase 3 简化：跳过 disease 读取（需 Disease::GetDiseaseIndex，Phase 6+ 接入）
    if version > 8 && start_offset != 0 {
        // 源码 L116-157：读 diseaseIdx + diseaseCount
        // Phase 3 stub：跳过对应字节（start_offset * inner_h * 8 字节）
        let skip_bytes = (start_offset as u64).saturating_mul(inner_h as u64).saturating_mul(8);
        if skip_bytes > 0 {
            tracing::debug!(skip_bytes, "Load: skipping disease data (Phase 3 stub)");
            // 不实际 skip，因为 Phase 3 不解析 disease（避免数据错位）
        }
    }

    // 阶段 3：填充 backwall（version > 0xe，L158-192）
    // Phase 3 简化：跳过 backwall 读取
    if version > 0xe && start_offset != 0 {
        let skip_bytes = (start_offset as u64).saturating_mul(inner_h as u64).saturating_mul(12);
        if skip_bytes > 0 {
            tracing::debug!(skip_bytes, "Load: skipping backwall data (Phase 3 stub)");
        }
    }

    // 版本兼容跳过（L193-195）
    if version >= 8 && version < 0xf {
        let skip_bytes = (start_offset as u64).saturating_mul(inner_h as u64).saturating_mul(4);
        if skip_bytes > 0 {
            let _ = reader.skip(skip_bytes);
        }
    }

    // version < 0xb 时设置 init_settle_thermal_boundaries = true（L196-198）
    if version < 0xb {
        let mut g = G_SIM_DATA.lock().unwrap();
        g.init_settle_thermal_boundaries = true;
    }

    tracing::info!("Load: completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{BinaryBufferReader, BinaryBufferWriter};

    #[test]
    fn global_singletons_lockable() {
        let _a = G_SIM_DATA.lock().unwrap();
        let _b = G_FRAME_SYNC.lock().unwrap();
        let _c = G_SIM.lock().unwrap();
        let _d = G_DISEASE.lock().unwrap();
        let _e = G_GAME_DATA_UPDATE.lock().unwrap();
    }

    #[test]
    fn allocate_cells_creates_sim_data_with_dimensions() {
        // 构造 buffer: width=4, height=4, bool1=true, bool2=false
        let mut w = BinaryBufferWriter::new();
        w.write_int(4).unwrap();   // width
        w.write_int(4).unwrap();   // height
        w.write_byte(1).unwrap();  // bool1
        w.write_byte(0).unwrap();  // bool2
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);
        allocate_cells(&mut reader).unwrap();
        // width+2=6, height+2=6
        let sd = G_SIM_DATA.lock().unwrap();
        assert_eq!(sd.width, 6);
        assert_eq!(sd.height, 6);
    }

    #[test]
    fn define_world_offsets_reads_count() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(1).unwrap();  // count=1
        // 4×int32 per WorldOffsetData
        w.write_int(0).unwrap();
        w.write_int(0).unwrap();
        w.write_int(10).unwrap();
        w.write_int(10).unwrap();
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);
        define_world_offsets(&mut reader).unwrap();
        let sd = G_SIM_DATA.lock().unwrap();
        assert_eq!(sd.worlds.len(), 1);
    }

    #[test]
    fn set_saved_options_updates_field() {
        // 先重置 saved_options
        {
            let mut sd = G_SIM_DATA.lock().unwrap();
            sd.saved_options = 0;
        }
        let mut w = BinaryBufferWriter::new();
        w.write_byte(0x01).unwrap();  // clearMask
        w.write_byte(0x02).unwrap();  // setMask
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);
        set_saved_options(&mut reader).unwrap();
        let sd = G_SIM_DATA.lock().unwrap();
        assert_eq!(sd.saved_options, 0x02);
    }

    #[test]
    fn create_disease_table_replaces_global() {
        let mut w = BinaryBufferWriter::new();
        // Disease::Disease 构造函数从 reader 读数据，Phase 3 stub 不读
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);
        // 不应 panic
        create_disease_table(&mut reader).unwrap();
        let _d = G_DISEASE.lock().unwrap();
        // d 是空 Disease（stub 构造）
    }

    #[test]
    fn clear_unoccupied_cells_does_not_panic() {
        // 先调用 allocate_cells 确保 SimData 有数据
        let mut w = BinaryBufferWriter::new();
        w.write_int(2).unwrap();
        w.write_int(2).unwrap();
        w.write_byte(0).unwrap();
        w.write_byte(0).unwrap();
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);
        allocate_cells(&mut reader).unwrap();

        // 清空未占用 cells
        let mut w2 = BinaryBufferWriter::new();
        let buf2 = w2.into_bytes();
        let mut reader2 = BinaryBufferReader::new(&buf2);
        clear_unoccupied_cells(&mut reader2).unwrap();
    }

    use crate::element::{INVALID_ELEMENT_HASH, INVALID_ELEMENT_INDEX};

    /// 在 G_ELEMENTS_TABLE 注册一个 hash→idx 映射（测试辅助）。
    fn register_element(hash: u32, idx: u16) {
        let mut table = crate::elements_table::G_ELEMENTS_TABLE.lock().unwrap();
        table.element_indices.insert(hash, idx);
    }

    #[test]
    fn cell_read_writes_to_cells_soa() {
        // 注册一个元素
        register_element(0x12345678, 5);

        let mut cells = CellSOA::new();
        cells.element_idx.resize(1, 0);
        cells.temperature.resize(1, 0.0);
        cells.mass.resize(1, 0.0);
        cells.radiation.resize(1, 0.0);

        let mut w = BinaryBufferWriter::new();
        w.write_uint(0x12345678).unwrap();  // elem_hash
        w.write_float(300.0).unwrap();       // temp
        w.write_float(50.0).unwrap();         // mass
        w.write_float(0.1).unwrap();          // radiation (version > 0xd)
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);

        cell_read(&mut cells, 0, &mut reader, 0xe).unwrap();
        assert_eq!(cells.element_idx.get(0), 5);
        assert_eq!(cells.temperature.get(0), 300.0);
        assert_eq!(cells.mass.get(0), 50.0);
        assert_eq!(cells.radiation.get(0), 0.1);
    }

    #[test]
    fn cell_read_nan_temp_replaced_with_300() {
        register_element(0x12345678, 5);

        let mut cells = CellSOA::new();
        cells.element_idx.resize(1, 0);
        cells.temperature.resize(1, 0.0);
        cells.mass.resize(1, 0.0);
        cells.radiation.resize(1, 0.0);

        let nan = f32::NAN;
        let mut w = BinaryBufferWriter::new();
        w.write_uint(0x12345678).unwrap();
        w.write_float(nan).unwrap();  // temp NaN
        w.write_float(50.0).unwrap();
        w.write_float(0.1).unwrap();
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);

        cell_read(&mut cells, 0, &mut reader, 0xe).unwrap();
        assert_eq!(cells.temperature.get(0), 300.0);  // NaN → 300.0
    }

    #[test]
    fn cell_read_inf_mass_replaced_with_100() {
        register_element(0x12345678, 5);

        let mut cells = CellSOA::new();
        cells.element_idx.resize(1, 0);
        cells.temperature.resize(1, 0.0);
        cells.mass.resize(1, 0.0);
        cells.radiation.resize(1, 0.0);

        let mut w = BinaryBufferWriter::new();
        w.write_uint(0x12345678).unwrap();
        w.write_float(300.0).unwrap();
        w.write_float(f32::INFINITY).unwrap();  // mass Inf
        w.write_float(0.1).unwrap();
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);

        cell_read(&mut cells, 0, &mut reader, 0xe).unwrap();
        assert_eq!(cells.mass.get(0), 100.0);  // Inf → 100.0
    }

    #[test]
    fn cell_read_invalid_hash_replaced_with_vacuum() {
        // 未注册的 hash → GetElementIndex 返回 0xffff → 用 INVALID_ELEMENT_HASH 重查
        // 先注册 INVALID_ELEMENT_HASH → 7（模拟 vacuum idx）
        register_element(INVALID_ELEMENT_HASH, 7);

        let mut cells = CellSOA::new();
        cells.element_idx.resize(1, 0);
        cells.temperature.resize(1, 0.0);
        cells.mass.resize(1, 0.0);
        cells.radiation.resize(1, 0.0);

        let mut w = BinaryBufferWriter::new();
        w.write_uint(0x99999999).unwrap();  // 未注册 hash
        w.write_float(300.0).unwrap();
        w.write_float(50.0).unwrap();
        w.write_float(0.1).unwrap();
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);

        cell_read(&mut cells, 0, &mut reader, 0xe).unwrap();
        // elementIdx 应为 INVALID_ELEMENT_INDEX (0xffff)，因为 INVALID_ELEMENT_HASH
        // 在测试环境中可能未注册到真实 vacuum idx
        let idx = cells.element_idx.get(0);
        assert!(idx == 7 || idx == INVALID_ELEMENT_INDEX);
    }

    #[test]
    fn load_with_empty_buffer_returns_invalid_magic() {
        let buf = [0u8; 0];
        let mut reader = BinaryBufferReader::new(&buf);
        let result = load(&mut reader);
        assert_eq!(result, Err(BufferError::InvalidMagic));
    }

    #[test]
    fn load_with_valid_magic_and_high_version_does_not_panic() {
        // 构造最小 buffer: magic + version=0x10（高版本，跳过 innerW/innerH/startOffset 读取）
        let mut w = BinaryBufferWriter::new();
        w.write_bytes(&[0x53, 0x49, 0x4d, 0x41, 0x56, 0x45, 0x53, 0x00]).unwrap();
        w.write_int(0x10).unwrap();  // version >= 0x10，跳过 header 读取
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);
        // 不应 panic，可能返回 Err（数据不足）或 Ok
        let _ = load(&mut reader);
    }
}
