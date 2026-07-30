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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{BinaryBufferReader, BinaryBufferWriter};
    use crate::element::{Element, INVALID_ELEMENT_HASH, INVALID_ELEMENT_INDEX};

    #[test]
    fn elements_table_default_all_empty() {
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
        let _guard = G_ELEMENTS_TABLE.lock().unwrap();
        // 锁可获取即通过
    }

    #[test]
    fn g_interactions_lock_acquirable() {
        let _guard = G_INTERACTIONS_LOCK.lock();
        // parking_lot::Mutex::lock() 直接返回 guard（非 Result）
    }

    #[test]
    fn destroy_clears_12_vectors_keeps_3_interaction_vectors() {
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
        // 先清空 table 确保 element_indices 为空
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.element_indices.clear();
        }
        assert_eq!(get_element_index(999), INVALID_ELEMENT_INDEX);
    }

    #[test]
    fn get_element_index_returns_inserted() {
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.element_indices.clear();
            table.element_indices.insert(0xABCD, 7);
        }
        assert_eq!(get_element_index(0xABCD), 7);
    }

    #[test]
    fn get_element_index_inner_does_not_lock() {
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
}
