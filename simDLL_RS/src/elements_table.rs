//! ElementsTable — 元素表全局状态（封装原版 15 个全局变量）。
//!
//! 对照源码 03_elements.c 中的 15 个全局变量：
//! gElements / gElementNames / gElementTemperatureData / gElementPostProcessData /
//! gElementLiquidData / gElementPhysicsData / gElementPressureData /
//! gElementPropertyTextureData / gElementLightAbsorptionData / gElementRadiationData /
//! gElementStateData / gElementIndices / gGasObliterations / gLiquidObliterations /
//! gLiquidConversions。
//!
//! Phase 2 T3：仅定义 struct + 全局单例，5 个函数实现见 T4-T6。

use std::collections::HashMap;
use std::os::raw::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

use once_cell::sync::Lazy;

use crate::element::*;
use crate::buffer::{BinaryBufferReader, BufferError};

/// 元素表全局状态——封装原版 13+ 个分散全局变量。
pub struct ElementsTable {
    pub elements: Vec<Element>,
    pub element_names: Vec<String>,
    pub temperature_data: Vec<ElementTemperatureData>,
    pub post_process_data: Vec<ElementPostProcessData>,
    pub liquid_data: Vec<ElementLiquidData>,
    pub physics_data: Vec<PhysicsData>,
    pub pressure_data: Vec<ElementPressureData>,
    pub property_texture_data: Vec<ElementPropertyTextureData>,
    pub light_absorption_data: Vec<ElementLightAbsorptionData>,
    pub radiation_data: Vec<ElementRadiationData>,
    pub state_data: Vec<ElementStateData>,
    pub element_indices: HashMap<u32, u16>,
    pub gas_obliterations: Vec<GasObliteration>,
    pub liquid_obliterations: Vec<LiquidObliteration>,
    pub liquid_conversions: Vec<LiquidConversion>,
}

impl Default for ElementsTable {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            element_names: Vec::new(),
            temperature_data: Vec::new(),
            post_process_data: Vec::new(),
            liquid_data: Vec::new(),
            physics_data: Vec::new(),
            pressure_data: Vec::new(),
            property_texture_data: Vec::new(),
            light_absorption_data: Vec::new(),
            radiation_data: Vec::new(),
            state_data: Vec::new(),
            element_indices: HashMap::new(),
            gas_obliterations: Vec::new(),
            liquid_obliterations: Vec::new(),
            liquid_conversions: Vec::new(),
        }
    }
}

/// 全局 ElementsTable 单例。
/// 用 std::sync::Mutex 保护（规格 §5.1）。
pub static G_ELEMENTS_TABLE: Lazy<Mutex<ElementsTable>> =
    Lazy::new(|| Mutex::new(ElementsTable::default()));

/// CreateElementInteractionsLocked 用的独立锁（决策 #2）。
/// 对应原版 gFrameSync.mSimMutex，Phase 3+ 替换 MsvcMutex 时无需回头改。
/// 用 parking_lot::Mutex（非 poisoning，避免析构时 panic 影响后续 DLL 调用）。
static G_INTERACTIONS_LOCK: Lazy<parking_lot::Mutex<()>> =
    Lazy::new(|| parking_lot::Mutex::new(()));

/// 销毁元素表——清空 11 个数据 vector + element_indices。
///
/// 对照源码 03_elements.c L430-453（DestroyElementsTable）。
/// 严格按源码：不清空 3 个交互 vector（gas_obliterations/liquid_obliterations/
/// liquid_conversions）。vector::clear 不释放容量（与源码 _Last = _First 等价）。
fn destroy_elements_table() {
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();
    table.elements.clear();
    table.element_names.clear();
    table.temperature_data.clear();
    table.post_process_data.clear();
    table.liquid_data.clear();
    table.physics_data.clear();
    table.pressure_data.clear();
    table.property_texture_data.clear();
    table.light_absorption_data.clear();
    table.radiation_data.clear();
    table.state_data.clear();
    table.element_indices.clear();
    // 注意：严格对照源码，3 个交互 vector 不清空
}

/// 内部函数：在 ElementsTable 引用上查找元素索引，**不加锁**。
///
/// 用于后处理阶段已持锁场景，避免 drop+relock 死锁模式（规格自检决策 #1）。
fn get_element_index_inner(table: &ElementsTable, hash: u32) -> u16 {
    *table
        .element_indices
        .get(&hash)
        .unwrap_or(&INVALID_ELEMENT_INDEX)
}

/// 查找元素索引（封装 G_ELEMENTS_TABLE 加锁 + 调用内部函数）。
///
/// 对照源码 03_elements.c L456-495（GetElementIndex）。
/// 找不到时返回 INVALID_ELEMENT_INDEX（0xffff）。
fn get_element_index(hash: u32) -> u16 {
    let table = G_ELEMENTS_TABLE.lock().unwrap();
    get_element_index_inner(&table, hash)
}

/// 公开包装：查找元素索引（供 save_load.rs 跨模块调用）。
pub fn get_element_index_pub(hash: u32) -> u16 {
    get_element_index(hash)
}

/// 公开包装：销毁元素表（供 save_load.rs 的 CleanUp 调用）。
pub fn destroy_elements_table_pub() {
    destroy_elements_table()
}

/// 公开包装：按 element_idx 查询元素状态（供 process_messages.rs 跨模块调用）。
///
/// 对照源码 elements.c 中 `gElementStateData[idx].state` 字段查询。
/// state 字段是位域：`state & 3` 决定状态：
/// - 0 → Vacuum
/// - 1 → Gas
/// - 2 → Liquid
/// - 3 → Solid
///
/// 返回 `ElementState` 枚举（与 process_messages 模块共享）。
/// 若元素表未初始化或 idx 越界，返回 `None`（调用方默认 Solid）。
pub fn get_element_state_by_idx(elem_idx: u16) -> Option<crate::process_messages::ElementState> {
    let table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let idx = elem_idx as usize;
    if idx >= table.state_data.len() {
        return None;
    }
    let state = table.state_data[idx].state;
    Some(match state & 3 {
        0 => crate::process_messages::ElementState::Vacuum,
        1 => crate::process_messages::ElementState::Gas,
        2 => crate::process_messages::ElementState::Liquid,
        3 => crate::process_messages::ElementState::Solid,
        _ => unreachable!(),  // state & 3 只会产生 0..=3
    })
}

/// 公开包装：按 element_idx 查询元素的 thermal_conductivity（T11 新增）。
///
/// 对照源码 04_temperature.c 中对 `gElementTemperatureData[idx].thermalConductivity`
/// 字段的访问（pbVar13 + 0x10 / pbVar15 + 0x10 偏移）。
/// thermal_conductivity 在 ElementTemperatureData 中位于 @0x10（见 element.rs L100）。
///
/// 返回 `Option<f32>`：
/// - `Some(value)`：元素表已初始化且 idx 在范围内
/// - `None`：元素表未初始化或 idx 越界（调用方应跳过热交换）
pub fn get_element_conductivity(elem_idx: u16) -> Option<f32> {
    let table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let idx = elem_idx as usize;
    if idx >= table.temperature_data.len() {
        return None;
    }
    Some(table.temperature_data[idx].thermal_conductivity)
}

/// 公开包装：按 element_idx 查询元素的 specific_heat_capacity（T11 新增）。
///
/// 对照源码 04_temperature.c 中对 `gElementTemperatureData[idx].specificHeatCapacity`
/// 字段的访问（pbVar13 + 0xc / pbVar15 + 0xc 偏移）。
/// specific_heat_capacity 在 ElementTemperatureData 中位于 @0xc（见 element.rs L99）。
///
/// 返回 `Option<f32>`：None 表示元素表未初始化或 idx 越界。
pub fn get_element_specific_heat_capacity(elem_idx: u16) -> Option<f32> {
    let table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let idx = elem_idx as usize;
    if idx >= table.temperature_data.len() {
        return None;
    }
    Some(table.temperature_data[idx].specific_heat_capacity)
}

/// 公开包装：按 element_idx 查询 ElementTemperatureData 全部字段（T11 新增）。
///
/// 返回 `Option<ElementTemperatureData>`（Copy）：None 表示元素表未初始化或 idx 越界。
/// 供 temperature.rs 一次性读取 thermal_conductivity / specific_heat_capacity /
/// gas/liquid/solid_surface_area_multiplier / low_temp / high_temp 等多个字段，
/// 避免多次加锁。
pub fn get_element_temperature_data(elem_idx: u16) -> Option<crate::element::ElementTemperatureData> {
    let table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let idx = elem_idx as usize;
    if idx >= table.temperature_data.len() {
        return None;
    }
    Some(table.temperature_data[idx])
}

/// 反序列化元素表——4 阶段 + 后处理。
///
/// 对照源码 03_elements.c L27-429（CreateElementsTable）。
///
/// 返回值：元素数量（源码返回 `(void*)(longlong)count`）。
/// 错误时返回 Err，调用方（C ABI 导出）记日志并返回 null。
fn create_elements_table(reader: &mut BinaryBufferReader) -> Result<i32, BufferError> {
    // 阶段 0: 清空旧数据
    destroy_elements_table();

    let mut table = G_ELEMENTS_TABLE.lock().unwrap();

    // 阶段 1: 读 count + memcpy Element 数据
    let count = reader.read_int()?;
    if count < 0 || count > 0xfffe {
        // 源码 L82: if (0xfffe < local_c8) && !gAssertsDisabled && IsDebuggerPresent() DebugBreak()
        tracing::warn!(count, "CreateElementsTable: count exceeds 0xfffe, may indicate corrupted data");
    }
    let count_usize = count.max(0) as usize;

    // 预分配 11 个 vector（resize 用 Default 填充，源码用 memset 0）
    table.elements.resize(count_usize, Element::default());
    table.element_names.resize(count_usize, String::new());
    table.temperature_data.resize(count_usize, ElementTemperatureData::default());
    table.post_process_data.resize(count_usize, ElementPostProcessData::default());
    table.liquid_data.resize(count_usize, ElementLiquidData::default());
    table.physics_data.resize(count_usize, PhysicsData::default());
    table.pressure_data.resize(count_usize, ElementPressureData::default());
    table.property_texture_data.resize(count_usize, ElementPropertyTextureData::default());
    table.light_absorption_data.resize(count_usize, ElementLightAbsorptionData::default());
    table.radiation_data.resize(count_usize, ElementRadiationData::default());
    table.state_data.resize(count_usize, ElementStateData::default());

    // memcpy Element 数据（直接读 count * 164 字节，源码 L99 memcpy 等价）
    for i in 0..count_usize {
        let mut bytes = [0u8; 164];
        reader.read_bytes(&mut bytes)?;
        // 安全转换：Element 是 #[repr(C)] 且 size_of == 164
        // 使用 read_unaligned：bytes 是 [u8; 164]（对齐 1），而 Element 含 i32/f32 字段（对齐 4），
        // read_unaligned 逐字节拷贝避免对齐假设（std::ptr::read 要求对齐 UB）。
        table.elements[i] = unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Element) };
    }

    // 阶段 2: 建立 element_indices（Element.id → index）
    // 源码 L101-179：循环 find/insert，重复 id 后插入覆盖前一个
    table.element_indices.clear();
    for i in 0..count_usize {
        let id = table.elements[i].id;
        table.element_indices.insert(id as u32, i as u16);
    }

    // 阶段 3: 读 element_names（每个元素：read_int(len) → read_bytes(len) → String）
    // 源码 L180-250
    for i in 0..count_usize {
        let len = reader.read_int()? as usize;
        let mut buf = vec![0u8; len];
        reader.read_bytes(&mut buf)?;
        match String::from_utf8(buf) {
            Ok(s) => table.element_names[i] = s,
            Err(_) => {
                tracing::warn!(element_index = i, "Element name is not valid UTF-8, replacing with empty");
                table.element_names[i] = String::new();
            }
        }
    }

    // 阶段 4: 逐字段填充 9 个附属 vector（按规格 §2.3 字段映射）
    // 源码 L259-392
    for i in 0..count_usize {
        let elem = table.elements[i];

        // ElementTemperatureData（§2.3.1）
        let mut td = ElementTemperatureData::default();
        td.state = elem.state;
        td.low_temp_transition_idx = elem.low_temp_transition_idx;
        td.high_temp_transition_idx = elem.high_temp_transition_idx;
        td.specific_heat_capacity = elem.specific_heat_capacity;
        td.thermal_conductivity = elem.thermal_conductivity;
        td.default_mass = elem.default_values.mass;
        td.mass_area_scale = if elem.state & 3 == 1 { 1.0 } else { 0.001 };
        td.gas_surface_area_multiplier = elem.gas_surface_area_multiplier;
        td.liquid_surface_area_multiplier = elem.liquid_surface_area_multiplier;
        td.solid_surface_area_multiplier = elem.solid_surface_area_multiplier;
        td.low_temp = elem.low_temp;
        td.high_temp = elem.high_temp;
        td.low_temp_transition_ore_mass_conversion = elem.low_temp_transition_ore_mass_conversion;
        td.high_temp_transition_ore_mass_conversion = elem.high_temp_transition_ore_mass_conversion;
        // low/high_temp_transition_ore_idx 在后处理阶段填入
        table.temperature_data[i] = td;

        // ElementPostProcessData（§2.3.2）
        let mut pp = ElementPostProcessData::default();
        pp.sublimate_index = elem.sublimate_index;
        pp.convert_index = elem.convert_index;
        pp.state = elem.state;
        pp.sublimate_rate = elem.sublimate_rate;
        pp.sublimate_efficiency = elem.sublimate_efficiency;
        pp.sublimate_probability = elem.sublimate_probability;
        pp.off_gas_percentage = elem.off_gas_percentage;
        pp.molar_mass = elem.molar_mass;
        pp.strength = elem.strength;
        pp.max_mass = elem.max_mass;
        pp.min_horizontal_flow = elem.min_horizontal_flow;
        pp.sublimate_fx = elem.sublimate_fx;
        table.post_process_data[i] = pp;

        // ElementLiquidData（§2.3.3）
        let mut ld = ElementLiquidData::default();
        ld.state = elem.state;
        ld.flow = elem.flow;
        ld.viscosity = elem.viscosity;
        ld.min_horizontal_flow = elem.min_horizontal_flow;
        ld.min_vertical_flow = elem.min_vertical_flow;
        ld.max_mass = elem.max_mass;
        table.liquid_data[i] = ld;

        // ElementPhysicsData = PhysicsData（§2.3.4，直接拷贝 default_values）
        table.physics_data[i] = elem.default_values;

        // ElementPressureData（§2.3.5）
        let mut pd = ElementPressureData::default();
        pd.state = elem.state;
        pd.flow = elem.flow;
        table.pressure_data[i] = pd;

        // ElementPropertyTextureData（§2.3.6）
        let mut ptd = ElementPropertyTextureData::default();
        ptd.colour = elem.colour;
        ptd.gradient = elem.gradient_colours;
        ptd.material_properties = elem.material_properties;
        ptd.state = elem.state;
        ptd.number_of_gradient_colors = elem.number_of_gradient_colors;
        table.property_texture_data[i] = ptd;

        // ElementLightAbsorptionData（§2.3.7）
        let mut la = ElementLightAbsorptionData::default();
        la.factor = elem.light_absorption_factor;
        la.mass_scale = if elem.state & 3 == 3 {
            f32::INFINITY
        } else {
            1.0 / elem.default_values.mass
        };
        table.light_absorption_data[i] = la;

        // ElementRadiationData（§2.3.8）
        let mut rd = ElementRadiationData::default();
        rd.factor = elem.radiation_absorption_factor;
        rd.rads_per_1000 = elem.radiation_per_1000_mass;
        table.radiation_data[i] = rd;

        // ElementStateData（§2.3.9）
        let mut sd = ElementStateData::default();
        sd.state = elem.state;
        table.state_data[i] = sd;
    }

    // 后处理阶段（源码 L396-425）：填充 ElementTemperatureData 的
    // low/high_temp_transition_ore_idx。
    // 使用 get_element_index_inner（持锁调用，避免死锁——决策 #1）
    for i in 0..count_usize {
        let elem = table.elements[i];
        let low_ore_id = elem.low_temp_transition_ore_id as u32;
        let high_ore_id = elem.high_temp_transition_ore_id as u32;

        let low_idx = if low_ore_id == INVALID_ELEMENT_HASH {
            INVALID_ELEMENT_INDEX
        } else {
            get_element_index_inner(&table, low_ore_id)
        };
        let high_idx = if high_ore_id == INVALID_ELEMENT_HASH {
            INVALID_ELEMENT_INDEX
        } else {
            get_element_index_inner(&table, high_ore_id)
        };

        table.temperature_data[i].low_temp_transition_ore_idx = low_idx;
        table.temperature_data[i].high_temp_transition_ore_idx = high_idx;
    }

    Ok(count)
}

/// 反序列化元素交互——只 push 匹配 GAS_OBLITERATION_SIGNATURE 的 entry。
///
/// 对照源码 03_elements.c L517-569（CreateElementInteractions）。
/// 严格按源码：liquid_obliterations / liquid_conversions 只清空不填充
/// （源码缺失分支，规格 §2.2.4 注释）。
fn create_element_interactions(reader: &mut BinaryBufferReader) -> Result<(), BufferError> {
    let mut table = G_ELEMENTS_TABLE.lock().unwrap();

    // 清空 3 个交互 vector（源码 L517-519）
    table.liquid_obliterations.clear();
    table.liquid_conversions.clear();
    table.gas_obliterations.clear();

    let count = reader.read_int()?;
    if count < 0 {
        // 防御性：负 count 视为 0（源码无此检查但 Rust 边界安全优先）
        tracing::warn!(count, "CreateElementInteractions: negative count, treating as 0");
        return Ok(());
    }
    let count_usize = count as usize;

    for _ in 0..count_usize {
        // 每 entry 0x20=32 字节，与源码 stride 一致
        let signature = reader.read_int()?;
        let elem_idx1 = reader.read_ushort()?;
        let elem_idx2 = reader.read_ushort()?;
        let elem_result_idx = reader.read_ushort()?;
        let _padding_a = reader.read_ushort()?;  // @0xa-0xb 源码未使用
        let min_mass = reader.read_float()?;
        let interaction_probability = reader.read_float()?;
        let elem1_mass_destruction_percent = reader.read_float()?;
        let elem2_mass_required_multiplier = reader.read_float()?;
        let elem_result_mass_creation_multiplier = reader.read_float()?;
        // 共读 4 + 2*4 + 5*4 = 32 字节，与源码 0x20 stride 对齐

        if signature == GAS_OBLITERATION_SIGNATURE {
            // 用 default() 初始化（_pad1 是 element.rs 私有字段，跨模块 struct literal
            // 与 functional update syntax 都会触发 E0451；改用 default + 逐字段赋值，
            // _pad1 默认 0 正是所需——与简报预判偏差 #1 的意图一致）
            let mut g = GasObliteration::default();
            g.elem_idx1 = elem_idx1;
            g.elem_idx2 = elem_idx2;
            g.elem_result_idx = elem_result_idx;
            g.interaction_probability = interaction_probability;
            g.min_mass = min_mass;
            g.elem1_mass_destruction_percent = elem1_mass_destruction_percent;
            g.elem2_mass_required_multiplier = elem2_mass_required_multiplier;
            g.elem_result_mass_creation_multiplier = elem_result_mass_creation_multiplier;
            table.gas_obliterations.push(g);
        }
        // 签名不匹配：源码跳过此 entry（不 push 到任何 vector）
    }

    Ok(())
}

/// C ABI: CreateElementsTable。
///
/// 对照源码 03_elements.c L27（导出符号）。
/// 返回 (void*)(longlong)count，错误时返回 null。
#[no_mangle]
pub extern "C" fn CreateElementsTable(reader: *mut BinaryBufferReader) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        match unsafe { reader.as_mut() } {
            Some(reader) => match create_elements_table(reader) {
                Ok(count) => count as *mut c_void,  // 源码返回 (void*)(longlong)count
                Err(e) => {
                    tracing::error!(?e, offset = reader.offset(), "CreateElementsTable failed");
                    std::ptr::null_mut()
                }
            },
            None => {
                tracing::warn!("CreateElementsTable called with null reader");
                std::ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        tracing::error!("CreateElementsTable panicked");
        std::ptr::null_mut()
    })
}

/// C ABI: DestroyElementsTable。
///
/// 对照源码 03_elements.c L430（导出符号）。
#[no_mangle]
pub extern "C" fn DestroyElementsTable() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        destroy_elements_table();
    }));
}

/// C ABI: GetElementIndex。
///
/// 对照源码 03_elements.c L456（导出符号）。
/// 找不到时返回 INVALID_ELEMENT_INDEX (0xffff)。
#[no_mangle]
pub extern "C" fn GetElementIndex(hash: u32) -> u16 {
    catch_unwind(AssertUnwindSafe(|| {
        get_element_index(hash)
    }))
    .unwrap_or(INVALID_ELEMENT_INDEX)
}

/// C ABI: CreateElementInteractions。
///
/// 对照源码 03_elements.c L517（导出符号）。
/// 返回 null（源码 L569: return null）。
#[no_mangle]
pub extern "C" fn CreateElementInteractions(reader: *mut BinaryBufferReader) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        match unsafe { reader.as_mut() } {
            Some(reader) => match create_element_interactions(reader) {
                Ok(()) => std::ptr::null_mut(),  // 源码返回 null
                Err(e) => {
                    tracing::error!(?e, offset = reader.offset(), "CreateElementInteractions failed, partial data may remain");
                    std::ptr::null_mut()
                }
            },
            None => {
                tracing::warn!("CreateElementInteractions called with null reader");
                std::ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        tracing::error!("CreateElementInteractions panicked");
        std::ptr::null_mut()
    })
}

/// C ABI: CreateElementInteractionsLocked。
///
/// 对照源码 03_elements.c L7-23（导出符号）。
/// 用 G_INTERACTIONS_LOCK（parking_lot::Mutex 独立锁，决策 #2）
/// 替代原版 gFrameSync.mSimMutex，Phase 3+ 替换 MsvcMutex 时无需回头改。
#[no_mangle]
pub extern "C" fn CreateElementInteractionsLocked(reader: *mut BinaryBufferReader) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        // 决策 #2：用 parking_lot::Mutex 独立锁
        let _guard = G_INTERACTIONS_LOCK.lock();
        match unsafe { reader.as_mut() } {
            Some(reader) => match create_element_interactions(reader) {
                Ok(()) => std::ptr::null_mut(),
                Err(e) => {
                    tracing::error!(?e, offset = reader.offset(), "CreateElementInteractionsLocked failed");
                    std::ptr::null_mut()
                }
            },
            None => {
                tracing::warn!("CreateElementInteractionsLocked called with null reader");
                std::ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        tracing::error!("CreateElementInteractionsLocked panicked");
        std::ptr::null_mut()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 跨模块共享的测试序列化锁（定义在 crate::LIB_TESTS_LOCK）：
    /// 串行化 elements_table::tests / save_load::tests / sim_api::tests
    /// 中所有操作全局状态的测试，避免并行执行导致 setup 互相覆盖。
    use crate::LIB_TESTS_LOCK as PHASE4_TEST_LOCK;
    use crate::buffer::{BinaryBufferReader, BinaryBufferWriter};
    use crate::element::{Element, INVALID_ELEMENT_HASH, INVALID_ELEMENT_INDEX};

    #[test]
    fn elements_table_default_all_empty() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let table = ElementsTable::default();
        assert!(table.elements.is_empty());
        assert!(table.element_names.is_empty());
        assert!(table.temperature_data.is_empty());
        assert!(table.post_process_data.is_empty());
        assert!(table.liquid_data.is_empty());
        assert!(table.physics_data.is_empty());
        assert!(table.pressure_data.is_empty());
        assert!(table.property_texture_data.is_empty());
        assert!(table.light_absorption_data.is_empty());
        assert!(table.radiation_data.is_empty());
        assert!(table.state_data.is_empty());
        assert!(table.element_indices.is_empty());
        assert!(table.gas_obliterations.is_empty());
        assert!(table.liquid_obliterations.is_empty());
        assert!(table.liquid_conversions.is_empty());
    }

    #[test]
    fn g_elements_table_lock_acquirable() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = G_ELEMENTS_TABLE.lock().unwrap();
        // 锁可获取即通过
    }

    #[test]
    fn g_interactions_lock_acquirable() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = G_INTERACTIONS_LOCK.lock();
        // parking_lot::Mutex::lock() 直接返回 guard（非 Result）
    }

    #[test]
    fn destroy_clears_12_vectors_keeps_3_interaction_vectors() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 先在 table 中塞入数据
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.push(Element::default());
            table.element_names.push("test".to_string());
            table.temperature_data.push(crate::element::ElementTemperatureData::default());
            table.post_process_data.push(crate::element::ElementPostProcessData::default());
            table.liquid_data.push(crate::element::ElementLiquidData::default());
            table.physics_data.push(crate::element::PhysicsData::default());
            table.pressure_data.push(crate::element::ElementPressureData::default());
            table.property_texture_data.push(crate::element::ElementPropertyTextureData::default());
            table.light_absorption_data.push(crate::element::ElementLightAbsorptionData::default());
            table.radiation_data.push(crate::element::ElementRadiationData::default());
            table.state_data.push(crate::element::ElementStateData::default());
            table.element_indices.insert(42, 0);
            table.gas_obliterations.push(crate::element::GasObliteration::default());
            table.liquid_obliterations.push(crate::element::LiquidObliteration::default());
            table.liquid_conversions.push(crate::element::LiquidConversion::default());
        }

        destroy_elements_table();

        let table = G_ELEMENTS_TABLE.lock().unwrap();
        // 11 个数据 vector + element_indices 应清空
        assert!(table.elements.is_empty());
        assert!(table.element_names.is_empty());
        assert!(table.temperature_data.is_empty());
        assert!(table.post_process_data.is_empty());
        assert!(table.liquid_data.is_empty());
        assert!(table.physics_data.is_empty());
        assert!(table.pressure_data.is_empty());
        assert!(table.property_texture_data.is_empty());
        assert!(table.light_absorption_data.is_empty());
        assert!(table.radiation_data.is_empty());
        assert!(table.state_data.is_empty());
        assert!(table.element_indices.is_empty());
        // 3 个交互 vector 不应被清空（严格对照源码 DestroyElementsTable L430-453）
        assert!(!table.gas_obliterations.is_empty());
        assert!(!table.liquid_obliterations.is_empty());
        assert!(!table.liquid_conversions.is_empty());
    }

    #[test]
    fn get_element_index_returns_invalid_for_missing() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 先清空 table 确保 element_indices 为空
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.element_indices.clear();
        }
        assert_eq!(get_element_index(999), INVALID_ELEMENT_INDEX);
    }

    #[test]
    fn get_element_index_returns_inserted() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.element_indices.clear();
            table.element_indices.insert(0xABCD, 7);
        }
        assert_eq!(get_element_index(0xABCD), 7);
    }

    #[test]
    fn get_element_index_inner_does_not_lock() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 验证 get_element_index_inner 接受 &ElementsTable 引用，避免死锁
        let table = G_ELEMENTS_TABLE.lock().unwrap();
        // 持锁状态下调用 inner 函数（不应死锁）
        let result = get_element_index_inner(&table, 0x1234);
        assert_eq!(result, INVALID_ELEMENT_INDEX);
    }

    /// 构造一个最小测试用 buffer（1 个元素）。
    fn build_single_element_buffer() -> Vec<u8> {
        let mut w = BinaryBufferWriter::new();

        // 阶段 1: count = 1
        w.write_int(1).unwrap();

        // 阶段 1: 1 × 164 字节 Element 数据
        // 构造一个 Element，关键字段：
        // - id = 100（用于 element_indices 查找）
        // - state = 1（state & 3 == 1 → mass_area_scale = 1.0）
        // - low_temp_transition_ore_id = INVALID_ELEMENT_HASH（→ ore_idx = 0xffff）
        let mut elem = Element::default();
        elem.id = 100;
        elem.state = 1;  // state & 3 == 1
        elem.low_temp_transition_idx = 5;
        elem.high_temp_transition_idx = 6;
        elem.specific_heat_capacity = 0.5;
        elem.thermal_conductivity = 0.7;
        elem.default_values.mass = 2.0;
        elem.gas_surface_area_multiplier = 1.5;
        elem.liquid_surface_area_multiplier = 2.5;
        elem.solid_surface_area_multiplier = 3.5;
        elem.low_temp = 100.0;
        elem.high_temp = 500.0;
        elem.low_temp_transition_ore_id = INVALID_ELEMENT_HASH as i32;  // → 0xffff
        elem.high_temp_transition_ore_id = 200;  // 200 不在表里 → 0xffff
        elem.low_temp_transition_ore_mass_conversion = 0.1;
        elem.high_temp_transition_ore_mass_conversion = 0.2;
        elem.light_absorption_factor = 0.8;
        elem.radiation_absorption_factor = 0.4;
        elem.radiation_per_1000_mass = 0.3;
        elem.molar_mass = 16.0;
        elem.strength = 50.0;
        elem.max_mass = 1000.0;
        elem.min_horizontal_flow = 0.01;
        elem.sublimate_fx = 42;
        elem.sublimate_rate = 0.05;
        elem.sublimate_efficiency = 0.9;
        elem.sublimate_probability = 0.3;
        elem.off_gas_percentage = 0.6;
        elem.flow = 5.0;
        elem.viscosity = 0.5;
        elem.min_vertical_flow = 0.02;
        elem.material_properties = 0xABCD;
        elem.colour = 0x12345678;
        elem.gradient_colours = [1, 2, 3, 4, 5, 6];
        elem.number_of_gradient_colors = 6;
        elem.sublimate_index = 9;
        elem.convert_index = 10;
        elem.default_values.temperature = 300.0;
        elem.default_values.pressure = 1.0;
        elem.default_values.mass = 2.0;
        // 序列化为 164 字节
        let bytes: [u8; 164] = unsafe { std::mem::transmute(elem) };
        w.write_bytes(&bytes).unwrap();

        // 阶段 3: 1 个字符串名（len + bytes）
        w.write_uint(8).unwrap();  // len
        w.write_bytes(b"TestElem").unwrap();

        Ok::<_, crate::buffer::BufferError>(w.into_bytes()).unwrap()
    }

    #[test]
    fn create_elements_table_round_trip_single() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let buf = build_single_element_buffer();
        let mut reader = BinaryBufferReader::new(&buf);

        let count = create_elements_table(&mut reader).unwrap();
        assert_eq!(count, 1);

        let table = G_ELEMENTS_TABLE.lock().unwrap();

        // 阶段 1: elements 已读取
        assert_eq!(table.elements.len(), 1);
        assert_eq!(table.elements[0].id, 100);

        // 阶段 2: element_indices 已建立
        assert_eq!(table.element_indices.get(&100), Some(&0u16));

        // 阶段 3: element_names 已读
        assert_eq!(table.element_names.len(), 1);
        assert_eq!(table.element_names[0], "TestElem");

        // 阶段 4: 9 个附属 vector 已填（抽查关键字段）
        assert_eq!(table.temperature_data.len(), 1);
        let td = &table.temperature_data[0];
        assert_eq!(td.state, 1);
        assert_eq!(td.low_temp_transition_idx, 5);
        assert_eq!(td.high_temp_transition_idx, 6);
        assert_eq!(td.specific_heat_capacity, 0.5);
        assert_eq!(td.thermal_conductivity, 0.7);
        assert_eq!(td.default_mass, 2.0);
        // mass_area_scale: state & 3 == 1 → 1.0
        assert_eq!(td.mass_area_scale, 1.0);
        assert_eq!(td.gas_surface_area_multiplier, 1.5);
        assert_eq!(td.liquid_surface_area_multiplier, 2.5);
        assert_eq!(td.solid_surface_area_multiplier, 3.5);
        assert_eq!(td.low_temp, 100.0);
        assert_eq!(td.high_temp, 500.0);
        assert_eq!(td.low_temp_transition_ore_mass_conversion, 0.1);
        assert_eq!(td.high_temp_transition_ore_mass_conversion, 0.2);

        // 后处理: low_temp_transition_ore_idx（INVALID_ELEMENT_HASH → 0xffff）
        assert_eq!(td.low_temp_transition_ore_idx, INVALID_ELEMENT_INDEX);
        // 后处理: high_temp_transition_ore_idx（ore_id=200 不在表 → 0xffff）
        assert_eq!(td.high_temp_transition_ore_idx, INVALID_ELEMENT_INDEX);

        // light_absorption_data: mass_scale 计算（state & 3 == 1, 非 3 → 1.0/mass = 0.5）
        assert_eq!(table.light_absorption_data.len(), 1);
        assert_eq!(table.light_absorption_data[0].factor, 0.8);
        assert!((table.light_absorption_data[0].mass_scale - 0.5).abs() < 1e-6);

        // 抽查其他附属 vector 长度
        assert_eq!(table.post_process_data.len(), 1);
        assert_eq!(table.liquid_data.len(), 1);
        assert_eq!(table.physics_data.len(), 1);
        assert_eq!(table.pressure_data.len(), 1);
        assert_eq!(table.property_texture_data.len(), 1);
        assert_eq!(table.radiation_data.len(), 1);
        assert_eq!(table.state_data.len(), 1);
        assert_eq!(table.state_data[0].state, 1);

        // 抽查 physics_data（应等于 elem.default_values）
        assert_eq!(table.physics_data[0].mass, 2.0);
        assert_eq!(table.physics_data[0].temperature, 300.0);
        assert_eq!(table.physics_data[0].pressure, 1.0);

        // 抽查 property_texture_data
        assert_eq!(table.property_texture_data[0].colour, 0x12345678);
        assert_eq!(table.property_texture_data[0].gradient, [1, 2, 3, 4, 5, 6]);
        assert_eq!(table.property_texture_data[0].material_properties, 0xABCD);
        assert_eq!(table.property_texture_data[0].state, 1);
        assert_eq!(table.property_texture_data[0].number_of_gradient_colors, 6);
    }

    #[test]
    fn create_elements_table_mass_scale_solid_state_infinity() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 构造 state & 3 == 3 的元素，验证 mass_scale == INFINITY
        let mut w = BinaryBufferWriter::new();
        w.write_int(1).unwrap();
        let mut elem = Element::default();
        elem.id = 1;
        elem.state = 3;  // state & 3 == 3 → solid → mass_scale = INFINITY
        elem.default_values.mass = 5.0;
        let bytes: [u8; 164] = unsafe { std::mem::transmute(elem) };
        w.write_bytes(&bytes).unwrap();
        w.write_uint(1).unwrap();
        w.write_bytes(b"S").unwrap();

        // 注意：必须将 Vec 绑定到变量，否则临时 Vec 在语句结束时被 drop，
        // reader 持有的原始指针会变成悬垂指针（BinaryBufferReader 存裸指针不持生命周期）。
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);
        create_elements_table(&mut reader).unwrap();

        let table = G_ELEMENTS_TABLE.lock().unwrap();
        assert!(table.light_absorption_data[0].mass_scale.is_infinite());
    }

    #[test]
    fn create_elements_table_mass_area_scale_non_gas_state() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // state & 3 != 1 → mass_area_scale = 0.001
        let mut w = BinaryBufferWriter::new();
        w.write_int(1).unwrap();
        let mut elem = Element::default();
        elem.id = 1;
        elem.state = 0;  // state & 3 == 0 → 非 gas → 0.001
        let bytes: [u8; 164] = unsafe { std::mem::transmute(elem) };
        w.write_bytes(&bytes).unwrap();
        w.write_uint(1).unwrap();
        w.write_bytes(b"S").unwrap();

        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);
        create_elements_table(&mut reader).unwrap();

        let table = G_ELEMENTS_TABLE.lock().unwrap();
        assert!((table.temperature_data[0].mass_area_scale - 0.001).abs() < 1e-6);
    }

    use crate::element::GAS_OBLITERATION_SIGNATURE;

    /// 构造 interactions buffer：count=N，每 entry 32B
    fn build_interactions_buffer(entries: &[(i32, u16, u16, u16, f32, f32, f32, f32, f32)]) -> Vec<u8> {
        let mut w = BinaryBufferWriter::new();
        w.write_int(entries.len() as i32).unwrap();
        for &(sig, idx1, idx2, result_idx, prob, min_mass, destroy_pct, req_mult, create_mult) in entries {
            // 每 entry 32 字节布局：
            // @0  i32 signature (4)
            // @4  u16 elem_idx1 (2)
            // @6  u16 elem_idx2 (2)
            // @8  u16 elem_result_idx (2)
            // @0xa 2 字节 padding（read_ushort 占位）
            // @0xc f32 min_mass (4)
            // @0x10 f32 interaction_probability (4)
            // @0x14 f32 elem1_mass_destruction_percent (4)
            // @0x18 f32 elem2_mass_required_multiplier (4)
            // @0x1c f32 elem_result_mass_creation_multiplier (4)
            // 总计 32 字节，与源码 0x20 stride 对齐
            w.write_int(sig).unwrap();
            w.write_ushort(idx1).unwrap();
            w.write_ushort(idx2).unwrap();
            w.write_ushort(result_idx).unwrap();
            w.write_ushort(0).unwrap();  // padding @0xa-0xb
            w.write_float(min_mass).unwrap();
            w.write_float(prob).unwrap();
            w.write_float(destroy_pct).unwrap();
            w.write_float(req_mult).unwrap();
            w.write_float(create_mult).unwrap();
        }
        w.into_bytes()
    }

    #[test]
    fn create_element_interactions_pushes_only_matching_signature() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 清空交互 vector（创建初始状态）
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.gas_obliterations.clear();
            table.liquid_obliterations.clear();
            table.liquid_conversions.clear();
        }

        let buf = build_interactions_buffer(&[
            // entry 0: 匹配签名
            (GAS_OBLITERATION_SIGNATURE, 1, 2, 3, 0.5, 10.0, 0.25, 1.5, 0.75),
            // entry 1: 不匹配签名
            (0x12345678, 4, 5, 6, 0.9, 20.0, 0.5, 2.0, 1.0),
            // entry 2: 匹配签名
            (GAS_OBLITERATION_SIGNATURE, 7, 8, 9, 0.3, 5.0, 0.1, 0.8, 0.4),
        ]);
        let mut reader = BinaryBufferReader::new(&buf);
        create_element_interactions(&mut reader).unwrap();

        let table = G_ELEMENTS_TABLE.lock().unwrap();
        assert_eq!(table.gas_obliterations.len(), 2, "只有 2 个 entry 匹配签名");

        // 验证 entry 0 字段
        let g0 = &table.gas_obliterations[0];
        assert_eq!(g0.elem_idx1, 1);
        assert_eq!(g0.elem_idx2, 2);
        assert_eq!(g0.elem_result_idx, 3);
        assert!((g0.interaction_probability - 0.5).abs() < 1e-6);
        assert!((g0.min_mass - 10.0).abs() < 1e-6);
        assert!((g0.elem1_mass_destruction_percent - 0.25).abs() < 1e-6);
        assert!((g0.elem2_mass_required_multiplier - 1.5).abs() < 1e-6);
        assert!((g0.elem_result_mass_creation_multiplier - 0.75).abs() < 1e-6);

        // 验证 entry 2 字段（应该 push 到 [1]）
        let g1 = &table.gas_obliterations[1];
        assert_eq!(g1.elem_idx1, 7);
        assert_eq!(g1.elem_idx2, 8);
        assert_eq!(g1.elem_result_idx, 9);
        assert!((g1.interaction_probability - 0.3).abs() < 1e-6);
        assert!((g1.min_mass - 5.0).abs() < 1e-6);
        assert!((g1.elem1_mass_destruction_percent - 0.1).abs() < 1e-6);
        assert!((g1.elem2_mass_required_multiplier - 0.8).abs() < 1e-6);
        assert!((g1.elem_result_mass_creation_multiplier - 0.4).abs() < 1e-6);

        // liquid_obliterations / liquid_conversions 严格按源码只清空不填充
        assert!(table.liquid_obliterations.is_empty());
        assert!(table.liquid_conversions.is_empty());
    }

    #[test]
    fn create_element_interactions_clears_before_push() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 先塞入旧数据
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.gas_obliterations.push(crate::element::GasObliteration::default());
            table.liquid_obliterations.push(crate::element::LiquidObliteration::default());
            table.liquid_conversions.push(crate::element::LiquidConversion::default());
        }

        // 调用后应清空（即使新 buffer 为空也清空）
        let buf = build_interactions_buffer(&[]);
        let mut reader = BinaryBufferReader::new(&buf);
        create_element_interactions(&mut reader).unwrap();

        let table = G_ELEMENTS_TABLE.lock().unwrap();
        assert!(table.gas_obliterations.is_empty());
        assert!(table.liquid_obliterations.is_empty());
        assert!(table.liquid_conversions.is_empty());
    }

    #[test]
    fn create_element_interactions_negative_count_returns_early() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut w = BinaryBufferWriter::new();
        w.write_int(-1).unwrap();  // 负 count
        let buf = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&buf);
        // 不应 panic，应正常返回 Ok(())
        let result = create_element_interactions(&mut reader);
        assert!(result.is_ok());
    }

    // ===== T11 新增：get_element_conductivity / specific_heat_capacity / temperature_data 测试 =====

    #[test]
    fn get_element_conductivity_returns_none_for_empty_table() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 先清空表
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
        }
        assert_eq!(get_element_conductivity(0), None);
        assert_eq!(get_element_conductivity(100), None);
    }

    #[test]
    fn get_element_conductivity_returns_value_for_valid_idx() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 直接塞入 1 个 ElementTemperatureData，thermal_conductivity = 0.7
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
            let mut td = crate::element::ElementTemperatureData::default();
            td.thermal_conductivity = 0.7;
            td.specific_heat_capacity = 0.5;
            table.temperature_data.push(td);
        }
        assert_eq!(get_element_conductivity(0), Some(0.7));
        // 越界
        assert_eq!(get_element_conductivity(1), None);
        // 清理
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
        }
    }

    #[test]
    fn get_element_specific_heat_capacity_returns_value_for_valid_idx() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
            let mut td = crate::element::ElementTemperatureData::default();
            td.specific_heat_capacity = 0.42;
            table.temperature_data.push(td);
        }
        assert_eq!(get_element_specific_heat_capacity(0), Some(0.42));
        assert_eq!(get_element_specific_heat_capacity(1), None);
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
        }
    }

    #[test]
    fn get_element_temperature_data_returns_all_fields() {
        let _lock = PHASE4_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
            let mut td = crate::element::ElementTemperatureData::default();
            td.thermal_conductivity = 0.8;
            td.specific_heat_capacity = 0.3;
            td.gas_surface_area_multiplier = 1.5;
            td.liquid_surface_area_multiplier = 2.5;
            td.solid_surface_area_multiplier = 3.5;
            td.low_temp = 100.0;
            td.high_temp = 500.0;
            td.state = 2;  // Liquid
            table.temperature_data.push(td);
        }
        let td = get_element_temperature_data(0).expect("should exist");
        assert_eq!(td.thermal_conductivity, 0.8);
        assert_eq!(td.specific_heat_capacity, 0.3);
        assert_eq!(td.gas_surface_area_multiplier, 1.5);
        assert_eq!(td.liquid_surface_area_multiplier, 2.5);
        assert_eq!(td.solid_surface_area_multiplier, 3.5);
        assert_eq!(td.low_temp, 100.0);
        assert_eq!(td.high_temp, 500.0);
        assert_eq!(td.state, 2);
        // 越界
        assert!(get_element_temperature_data(1).is_none());
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.temperature_data.clear();
        }
    }
}
