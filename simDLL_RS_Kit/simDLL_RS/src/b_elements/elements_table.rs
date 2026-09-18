//! ElementsTable — 元素表（15 个全局容器 + 5 个 C ABI 导出）。
//!
//! 对照源码 03_elements.c L27-569。
//! ElementsTable 封装原版 15 个全局变量（gElements / gElementNames / ... / gLiquidConversions）。
//! **不是 `#[repr(C)]`**（Rust 内部状态，不导出给 C）。
//!
//! **数据格式**（对照 C# SimMessages.CreateSimElementsTable）：
//! - 4 字节 count
//! - count × 164 字节 Element 数据
//! - count × KleiString（4 字节长度 + N 字节 UTF-8）
//!
//! **CreateElementInteractions 数据格式**（对照 C# SimMessages.CreateElementInteractions）：
//! - 12 字节 CreateElementInteractionsMsg（4 字节 count + 8 字节指针）
//! - 指针指向 count × 32 字节 ElementInteraction 数组

use crate::a_framework::buffer::{BufferError, BinaryBufferReader};
use crate::b_elements::element::*;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, Ordering};
use once_cell::sync::Lazy;

// ===== ElementsTable struct =====

/// ElementsTable — 元素表（15 字段）。
/// 封装原版 03_elements.c 的 15 个全局变量。
#[derive(Default, Clone)]
pub struct ElementsTable {
    pub elements: Vec<Element>,                                // gElements
    pub element_names: Vec<String>,                            // gElementNames
    pub temperature_data: Vec<ElementTemperatureData>,         // gElementTemperatureData
    pub post_process_data: Vec<ElementPostProcessData>,        // gElementPostProcessData
    pub liquid_data: Vec<ElementLiquidData>,                   // gElementLiquidData
    pub physics_data: Vec<PhysicsData>,                        // gElementPhysicsData
    pub pressure_data: Vec<ElementPressureData>,               // gElementPressureData
    pub property_texture_data: Vec<ElementPropertyTextureData>,// gElementPropertyTextureData
    pub light_absorption_data: Vec<ElementLightAbsorptionData>,// gElementLightAbsorptionData
    pub radiation_data: Vec<ElementRadiationData>,             // gElementRadiationData
    pub state_data: Vec<ElementStateData>,                     // gElementStateData
    pub element_indices: HashMap<u32, u16>,                    // gElementIndices (hash → index)
    pub gas_obliterations: Vec<GasObliteration>,               // gGasObliterations
    pub liquid_obliterations: Vec<LiquidObliteration>,         // gLiquidObliterations
    pub liquid_conversions: Vec<LiquidConversion>,             // gLiquidConversions
}

// ===== 全局单例 =====

/// 全局 ElementsTable 单例。
/// 用 std::sync::Mutex 保护（会 poisoning，getter 用 into_inner() 恢复）。
pub static G_ELEMENTS_TABLE: Lazy<Mutex<ElementsTable>> =
    Lazy::new(|| Mutex::new(ElementsTable::default()));

/// 只读快照指针（性能专项，2026-08-05）。
///
/// 原版元素表是 11 个全局 std::vector（gElements 等），创建后**直接下标访问、无锁**
/// （03_elements.c 唯一的 Mutex 是帧同步握手）。此前每个 get_* 都 lock std Mutex，
/// 每格每子步约 150 万次加锁 → FPS 略低于原版（155-159 vs 160）。
///
/// 修复：CreateElementsTable 完成后把元素表发布为只读快照（AtomicPtr，store Release），
/// get_* 读快照（load Acquire + 裸指针解引用，**无锁**）；快照未发布时（测试/早期）
/// 回退 Mutex。**旧快照 leak 不释放**——CreateElementsTable 每次加载重建，sim 线程
/// 可能仍在读旧快照；旧表约 100KB/次，多次加载泄漏可忽略。
static ELEMENTS_SNAPSHOT: AtomicPtr<ElementsTable> = AtomicPtr::new(std::ptr::null_mut());

/// 把当前 Mutex 表内容发布为只读快照（生产 CreateElementsTable 完成后 +
/// 测试初始化元素表后调用）。克隆 ~200 条元素数据（~100KB），仅创建时一次。
pub fn refresh_elements_snapshot() {
    let table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let snapshot = Box::new(table.clone());
    ELEMENTS_SNAPSHOT.store(Box::into_raw(snapshot), Ordering::Release);
}

/// get_* 统一入口：生产 = 无锁读只读快照（快照未发布时回退 Mutex）；
/// 测试 = 直接 Mutex（与旧行为一致，避免逐个测试刷新快照的隔离负担）。
/// 安全性：快照 Box leak 永不释放 → 指针始终有效；Acquire/Release 保证发布可见；
/// 物理代码仅 sim 线程单线程执行，子步内同一快照稳定。
#[cfg(not(test))]
fn with_table<T>(f: impl FnOnce(&ElementsTable) -> T) -> T {
    let ptr = ELEMENTS_SNAPSHOT.load(Ordering::Acquire);
    if !ptr.is_null() {
        f(unsafe { &*ptr })
    } else {
        match G_ELEMENTS_TABLE.lock() {
            Ok(t) => f(&t),
            Err(e) => f(&e.into_inner()),
        }
    }
}

#[cfg(test)]
fn with_table<T>(f: impl FnOnce(&ElementsTable) -> T) -> T {
    match G_ELEMENTS_TABLE.lock() {
        Ok(t) => f(&t),
        Err(e) => f(&e.into_inner()),
    }
}

/// CreateElementInteractionsLocked 用的独立锁。
/// 对应原版 gFrameSync.mSimMutex。
/// 用 parking_lot::Mutex（非 poisoning，避免析构时 panic 影响后续 DLL 调用）。
static G_INTERACTIONS_LOCK: Lazy<parking_lot::Mutex<()>> =
    Lazy::new(|| parking_lot::Mutex::new(()));

// ===== 内部辅助函数 =====

/// 在已持锁场景下查找元素索引（避免 drop+relock 死锁）。
/// 找不到返回 INVALID_ELEMENT_INDEX。
fn get_element_index_inner(table: &ElementsTable, hash: u32) -> u16 {
    *table.element_indices.get(&hash).unwrap_or(&INVALID_ELEMENT_INDEX)
}

// ===== 7 个 pub getter 供跨模块调用 =====

/// 查找元素索引（pub 版本，自行加锁）。
pub fn get_element_index_pub(hash: u32) -> u16 {
    with_table(|table| get_element_index_inner(table, hash))
}

/// 返回元素数量。
pub fn get_element_count_pub() -> usize {
    with_table(|table| table.elements.len())
}

/// 销毁元素表（pub 版本，供 save_load CleanUp 调用）。
pub fn destroy_elements_table_pub() {
    destroy_elements_table();
}

/// 查询元素状态（低 2 bits：0=Vacuum, 1=Gas, 2=Liquid, 3=Solid）。
pub fn get_element_state_by_idx(elem_idx: u16) -> Option<u8> {
    let idx = elem_idx as usize;
    with_table(|table| table.state_data.get(idx).map(|s| s.state & 3))
}

/// 查询元素热导率（temperature.rs 用）。
pub fn get_element_conductivity(elem_idx: u16) -> Option<f32> {
    let idx = elem_idx as usize;
    with_table(|table| table.temperature_data.get(idx).map(|d| d.thermal_conductivity))
}

/// 查询元素比热容（temperature.rs 用）。
pub fn get_element_specific_heat_capacity(elem_idx: u16) -> Option<f32> {
    let idx = elem_idx as usize;
    with_table(|table| table.temperature_data.get(idx).map(|d| d.specific_heat_capacity))
}

/// 查询元素默认温度（对应 Element.defaultValues.temperature @0x98）。
///
/// 原版 ConduitTemperatureManager::Add/Set 在 contents_temperature > 10000（SIM_MAX_TEMPERATURE）
/// 时将其重置为元素默认温度（conduittemperaturemanager.cpp L0x35/L0x71）。温度数据表
/// `ElementTemperatureData` 不含该字段，故直接读 `Element.default_values.temperature`。
pub fn get_element_default_temperature(elem_idx: u16) -> Option<f32> {
    let idx = elem_idx as usize;
    with_table(|table| table.elements.get(idx).map(|e| e.default_values.temperature))
}

/// 一次性读取 ElementTemperatureData（避免多次加锁）。
pub fn get_element_temperature_data(elem_idx: u16) -> Option<ElementTemperatureData> {
    let idx = elem_idx as usize;
    with_table(|table| table.temperature_data.get(idx).copied())
}

/// 按索引读取元素 id（hash），用于 BeginSave 写存档。
/// 对照源码 01_sim_api.c L67-68: `*(undefined4 *)(uVar14 * 0xa4 + gElements._8_8_)`
/// 即 gElements[idx].id。
pub fn get_element_id_by_idx(elem_idx: u16) -> Option<i32> {
    let idx = elem_idx as usize;
    with_table(|table| table.elements.get(idx).map(|e| e.id))
}

/// 查询元素液体数据（gElementLiquidData，24B）。
/// 对照源码 00_types_reference.c L8802-8809。
pub fn get_element_liquid_data(elem_idx: u16) -> Option<ElementLiquidData> {
    let idx = elem_idx as usize;
    with_table(|table| table.liquid_data.get(idx).copied())
}

/// 查询元素压力数据（gElementPressureData，8B）。
/// 对照源码 00_types_reference.c L5474-5477。
pub fn get_element_pressure_data(elem_idx: u16) -> Option<ElementPressureData> {
    let idx = elem_idx as usize;
    with_table(|table| table.pressure_data.get(idx).copied())
}

/// 查询元素属性纹理数据（gElementPropertyTextureData，36B）。
/// 对照源码 00_types_reference.c L9598-9604。
pub fn get_element_property_texture_data(elem_idx: u16) -> Option<ElementPropertyTextureData> {
    let idx = elem_idx as usize;
    with_table(|table| table.property_texture_data.get(idx).copied())
}

/// 查询元素后处理数据（gElementPostProcessData，44B）。
/// 对照源码 00_types_reference.c L10144-10157。
/// 任务 8（PostProcessCell / DoDensityDisplacement / DisplaceLiquid / DisplaceGas）用：
/// - state：state & 3 判定真空/气体/液体/固体
/// - molar_mass：密度分层（重的下沉）
/// - max_mass / off_gas_percentage / sublimate_*：液体过压与升华路径
pub fn get_element_post_process_data(elem_idx: u16) -> Option<ElementPostProcessData> {
    let idx = elem_idx as usize;
    with_table(|table| table.post_process_data.get(idx).copied())
}

/// 查询元素吸光数据（gElementLightAbsorptionData，8B）。
/// 对照源码 00_types_reference.c L13306-13309。
/// 任务 8（update_exposed_to_sun_property_texture）用：factor 参与阳光衰减计算。
pub fn get_element_light_absorption_data(elem_idx: u16) -> Option<ElementLightAbsorptionData> {
    let idx = elem_idx as usize;
    with_table(|table| table.light_absorption_data.get(idx).copied())
}

/// 查询元素辐射数据（gElementRadiationData，8B）。
/// 辐射模块（阶段 A）：factor 参与宇宙辐射遮挡，rads_per_1000 参与元素辐射扩散。
pub fn get_element_radiation_data(elem_idx: u16) -> Option<ElementRadiationData> {
    let idx = elem_idx as usize;
    with_table(|table| table.radiation_data.get(idx).copied())
}

// ===== 内部实现函数 =====

/// 销毁元素表（清空 11 个数据 vector + element_indices）。
/// **严格按源码不清空 3 个交互 vector**（03_elements.c L430-453）。
fn destroy_elements_table() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // 清空只读快照：后续 get_* 回退 Mutex（空表），避免读到旧数据。
        // 旧快照 Box leak（不释放）——destroy 多在进程/测试边界调用，泄漏可忽略。
        ELEMENTS_SNAPSHOT.store(std::ptr::null_mut(), Ordering::Release);
        let mut table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
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
        // 注意：不清空 gas_obliterations / liquid_obliterations / liquid_conversions
    }));
    if result.is_err() {
        tracing::error!("destroy_elements_table panicked");
    }
}

/// 创建元素表（对照源码 03_elements.c L27-429）。
/// 4 个阶段：读元素 → 建 HashMap → 读名称 → 填充附属数据 + 后处理 ore_idx。
fn create_elements_table(reader: &mut BinaryBufferReader) -> Result<usize, BufferError> {
    // 阶段 0：销毁旧数据
    destroy_elements_table();

    let mut table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());

    // 阶段 1：读元素数量 + 逐元素读取
    let count = reader.read_int()?;
    if count < 0 {
        tracing::warn!(count, "negative element count, treating as 0");
        return Ok(0);
    }
    let count = count as usize;
    // 源码在 count > 0xfffe 时 DebugBreak（仅调试器附加）；
    // Rust 实现仅 warn 不中断（无法安全调用 DebugBreak）
    if count > 0xfffe {
        tracing::warn!(count, "element count exceeds 0xfffe (source would DebugBreak)");
    }

    if count == 0 {
        return Ok(0);
    }

    // resize 11 个数据 vector
    table.elements.resize(count, Element::default());
    table.temperature_data.resize(count, ElementTemperatureData::default());
    table.post_process_data.resize(count, ElementPostProcessData::default());
    table.liquid_data.resize(count, ElementLiquidData::default());
    table.physics_data.resize(count, PhysicsData::default());
    table.pressure_data.resize(count, ElementPressureData::default());
    table.property_texture_data.resize(count, ElementPropertyTextureData::default());
    table.light_absorption_data.resize(count, ElementLightAbsorptionData::default());
    table.radiation_data.resize(count, ElementRadiationData::default());
    table.state_data.resize(count, ElementStateData::default());

    // 逐元素读取 164 字节（用 read_unaligned 因缓冲区对齐 1，Element 对齐 4）
    for i in 0..count {
        let bytes = reader.read_bytes(164)?;
        let elem: Element = unsafe {
            std::ptr::read_unaligned(bytes.as_ptr() as *const Element)
        };
        table.elements[i] = elem;
    }

    // 阶段 2：建立 element_indices HashMap（id → index）
    // 用索引循环避免同时 immutable 借用 elements + mutable 借用 element_indices
    for i in 0..count {
        let elem_id = table.elements[i].id as u32;
        table.element_indices.insert(elem_id, i as u16);
    }

    tracing::info!(
        count,
        mapped = table.element_indices.len(),
        "ElementsTable phase 1-2 done (elements read + HashMap built)"
    );

    // 阶段 3：读元素名称（KleiString 格式：4 字节长度 + N 字节 UTF-8）
    table.element_names.clear();
    for _ in 0..count {
        let name_len = reader.read_int()?;
        if name_len < 0 {
            tracing::warn!(name_len, "negative name length, using empty string");
            table.element_names.push(String::new());
            continue;
        }
        let name_bytes = reader.read_bytes(name_len as usize)?;
        let name = String::from_utf8(name_bytes).unwrap_or_default();
        table.element_names.push(name);
    }

    // 阶段 4：填充 9 个附属 vector（从 Element 字段派生）
    // 严格对照源码 03_elements.c L261-392 的字段映射。
    for i in 0..count {
        let elem = table.elements[i];

        // — temperature_data（源码 L268-288）—
        // mass_area_scale: (state & 3 == 1) ? 1.0 : 0.001（源码 0x3a83126f）
        let mass_area_scale = if (elem.state & 3) == 1 {
            1.0_f32
        } else {
            f32::from_bits(0x3a83126f)
        };
        table.temperature_data[i] = ElementTemperatureData {
            state: elem.state,                                          // @0
            low_temp_transition_idx: elem.low_temp_transition_idx,      // @2
            high_temp_transition_idx: elem.high_temp_transition_idx,    // @4
            low_temp_transition_ore_idx: 0,                             // @6 后处理填充
            high_temp_transition_ore_idx: 0,                            // @8 后处理填充
            specific_heat_capacity: elem.specific_heat_capacity,        // @0xc
            thermal_conductivity: elem.thermal_conductivity,            // @0x10
            default_mass: elem.default_values.mass,                     // @0x14
            mass_area_scale,                                            // @0x18
            gas_surface_area_multiplier: elem.gas_surface_area_multiplier, // @0x1c
            liquid_surface_area_multiplier: elem.liquid_surface_area_multiplier, // @0x20
            solid_surface_area_multiplier: elem.solid_surface_area_multiplier, // @0x24
            low_temp: elem.low_temp,                                    // @0x28
            high_temp: elem.high_temp,                                  // @0x2c
            low_temp_transition_ore_mass_conversion: elem.low_temp_transition_ore_mass_conversion, // @0x30
            high_temp_transition_ore_mass_conversion: elem.high_temp_transition_ore_mass_conversion, // @0x34
        };

        // — post_process_data（源码 03_elements.c L296-307）—
        // 严格按源码字节偏移映射（ppd_offset ← elem_offset）。
        // L305 为 ppd.maxMass(@0x20) ← elem.maxMass(@0x34)（2026-08-02 修正，
        // 旧实现误映射 solidSurfaceAreaMultiplier 导致超压判定失真）。
        table.post_process_data[i] = ElementPostProcessData {
            sublimate_index: elem.sublimate_index,                      // @0  ← elem.@0x54（L297）
            convert_index: elem.convert_index,                          // @2  ← elem.@0x56（L298）
            state: elem.state,                                          // @4  ← elem.@10  (L296)
            sublimate_rate: elem.sublimate_rate,                        // @8  ← elem.@0x7c（L299）
            sublimate_efficiency: elem.sublimate_efficiency,            // @0xc ← elem.@0x80（L300）
            sublimate_probability: elem.sublimate_probability,          // @0x10 ← elem.@0x84（L301）
            off_gas_percentage: elem.off_gas_percentage,                // @0x14 ← elem.@0x88（L302）
            molar_mass: elem.molar_mass,                                // @0x18 ← elem.@0x14（L303）
            strength: elem.strength,                                    // @0x1c ← elem.@0x40（L304）
            // ⚠️ 2026-08-02 修复：原版 L305 是 `ppd@0x20 ← elem@0x34`（= elem.max_mass），
            // 旧实现误映射为 solid_surface_area_multiplier（@0x18）→ 生产水 ppd.max_mass=1.0
            // （应为 1000）→ 超压/DoPressureBreak 对任何 >1kg 液体误触发。
            max_mass: elem.max_mass,                                    // @0x20 ← elem.@0x34（L305）
            min_horizontal_flow: elem.min_horizontal_flow,              // @0x24 ← elem.@0x2c（L306）
            sublimate_fx: elem.sublimate_fx,                            // @0x28 ← elem.@0x78（L307）
        };

        // — liquid_data（源码 L314-319）—
        table.liquid_data[i] = ElementLiquidData {
            state: elem.state,                                          // @0
            flow: elem.flow,                                            // @4
            viscosity: elem.viscosity,                                  // @8
            min_horizontal_flow: elem.min_horizontal_flow,              // @0xc
            min_vertical_flow: elem.min_vertical_flow,                  // @0x10
            max_mass: elem.max_mass,                                    // @0x14
        };

        // — physics_data（源码 L326-328）—
        table.physics_data[i] = elem.default_values;

        // — pressure_data（源码 L335-336）—
        table.pressure_data[i] = ElementPressureData {
            state: elem.state,                                          // @0
            flow: elem.flow,                                            // @4
        };

        // — property_texture_data（源码 L344-352）—
        table.property_texture_data[i] = ElementPropertyTextureData {
            colour: elem.colour,                                        // @0
            gradient: elem.gradient_colours,                            // @4-0x1f
            material_properties: elem.material_properties,              // @0x1c
            state: elem.state,                                          // @0x20
            number_of_gradient_colors: elem.number_of_gradient_colors,  // @0x21
        };

        // — light_absorption_data（源码 L360-368）—
        // mass_scale: (state & 3 == 3) ? INFINITY : 1.0 / default_values.mass
        // 2026-08-04 光照修复：defaultMass==0 的非固体（真空等）若取 1.0/0=INFINITY，
        // 曝光函数 `massScale × 质量0 = NaN` 会毒化整列阳光（空间生态区全黑，
        // 仅顶部填充行亮——用户实测症状）。old 稳定版对零质量元素用 multiplier=1.0，
        // 与原版实际表现一致；此处同款回退（固体仍为 INFINITY 全遮光）。
        let mass_scale = if (elem.state & 3) == 3 {
            f32::INFINITY
        } else if elem.default_values.mass > 0.0 {
            1.0 / elem.default_values.mass
        } else {
            1.0
        };
        table.light_absorption_data[i] = ElementLightAbsorptionData {
            factor: elem.light_absorption_factor,                       // @0
            mass_scale,                                                 // @4
        };

        // — radiation_data（源码 L375-377）—
        table.radiation_data[i] = ElementRadiationData {
            factor: elem.radiation_absorption_factor,                   // @0
            rads_per_1000: elem.radiation_per_1000_mass,                // @4
        };

        // — state_data（源码 L382）—
        table.state_data[i] = ElementStateData {
            state: elem.state,                                          // @0
        };
    }

    // 后处理：填充 temperature_data 的 ore_idx（源码 L396-426）
    // 严格按源码：ore_id == INVALID_ELEMENT_HASH → INVALID_ELEMENT_INDEX
    //            否则 → GetElementIndex(ore_id)
    for i in 0..count {
        let elem = table.elements[i];

        // high_temp_transition_ore_idx（源码 L406-412：先处理 high）
        let high_ore_idx = if elem.high_temp_transition_ore_id as u32 == INVALID_ELEMENT_HASH {
            INVALID_ELEMENT_INDEX
        } else {
            get_element_index_inner(&table, elem.high_temp_transition_ore_id as u32)
        };
        table.temperature_data[i].high_temp_transition_ore_idx = high_ore_idx;

        // low_temp_transition_ore_idx（源码 L413-419：后处理 low）
        let low_ore_idx = if elem.low_temp_transition_ore_id as u32 == INVALID_ELEMENT_HASH {
            INVALID_ELEMENT_INDEX
        } else {
            get_element_index_inner(&table, elem.low_temp_transition_ore_id as u32)
        };
        table.temperature_data[i].low_temp_transition_ore_idx = low_ore_idx;
    }

    tracing::info!(count, "ElementsTable created (all 4 phases done)");

    Ok(count)
}

/// 创建元素交互（对照源码 03_elements.c L517-569）。
/// 仅填充 gas_obliterations（源码缺失 liquid 分支）。
///
/// **数据格式**（对照 C# CreateElementInteractionsMsg）：
/// - 4 字节 count (int)
/// - 8 字节 pointer (指向 ElementInteraction 数组)
/// - 每个 ElementInteraction 32 字节
fn create_element_interactions(reader: &mut BinaryBufferReader) -> Result<(), BufferError> {
    let mut table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());

    // 清空 3 个交互 vector（源码 L528-530）
    table.gas_obliterations.clear();
    table.liquid_obliterations.clear();
    table.liquid_conversions.clear();

    // 读取 count（4 字节）
    let count = reader.read_int()?;
    // Rust 安全性：负 count 视为 0（源码无此检查但会 wrap-around 导致巨大分配）
    if count <= 0 {
        return Ok(());
    }
    let count = count as usize;

    // 读取 pointer（8 字节，指向 ElementInteraction 数组）
    // 对照 C# CreateElementInteractionsMsg：int numInteractions + ElementInteraction* interactions
    let ptr_bytes = reader.read_bytes(8)?;
    let entries_ptr = usize::from_le_bytes([
        ptr_bytes[0], ptr_bytes[1], ptr_bytes[2], ptr_bytes[3],
        ptr_bytes[4], ptr_bytes[5], ptr_bytes[6], ptr_bytes[7],
    ]);

    if entries_ptr == 0 {
        tracing::warn!("CreateElementInteractions: null interactions pointer");
        return Ok(());
    }

    let entries_base = entries_ptr as *const u8;

    // 循环读 entry（每 entry 32 字节 = 0x20 stride，源码 L535-566）
    for i in 0..count {
        let entry_ptr = unsafe { entries_base.add(i * 32) };

        // 读取 signature（@0，4 字节）
        let signature = unsafe {
            std::ptr::read_unaligned(entry_ptr as *const i32)
        };

        // 仅当 signature == GAS_OBLITERATION_SIGNATURE 时提取（源码 L537）
        if signature == GAS_OBLITERATION_SIGNATURE {
            // 严格按源码 L538-545 的字段偏移映射
            let elem_idx1 = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(4) as *const u16)
            };
            let elem_idx2 = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(6) as *const u16)
            };
            let elem_result_idx = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(8) as *const u16)
            };
            let min_mass = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(12) as *const f32)
            };
            let interaction_probability = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(16) as *const f32)
            };
            let elem1_mass_destruction_percent = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(20) as *const f32)
            };
            let elem2_mass_required_multiplier = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(24) as *const f32)
            };
            let elem_result_mass_creation_multiplier = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(28) as *const f32)
            };

            let go = GasObliteration {
                elem_idx1,
                elem_idx2,
                elem_result_idx,
                interaction_probability,
                min_mass,
                elem1_mass_destruction_percent,
                elem2_mass_required_multiplier,
                elem_result_mass_creation_multiplier,
            };
            table.gas_obliterations.push(go);
        }
        // 签名不匹配：源码跳过此 entry（不 push 到任何 vector）
    }

    tracing::info!(
        count,
        gas_obliterations = table.gas_obliterations.len(),
        "CreateElementInteractions done"
    );

    Ok(())
}

// ===== 5 个 C ABI 导出函数 =====

/// CreateElementsTable — 创建元素表。
/// 对照源码 03_elements.c L27-429。
/// 返回值：成功返回 count 作为 *mut c_void（匹配源码 `(void*)(longlong)local_c8`），
///         失败返回 null。
#[no_mangle]
pub extern "C" fn CreateElementsTable(reader: *mut BinaryBufferReader) -> *mut c_void {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if reader.is_null() {
            tracing::error!("CreateElementsTable: null reader");
            return std::ptr::null_mut();
        }
        let reader = unsafe { &mut *reader };
        match create_elements_table(reader) {
            Ok(count) => {
                refresh_elements_snapshot();
                tracing::info!(count, "CreateElementsTable success");
                count as usize as *mut c_void
            }
            Err(e) => {
                tracing::error!(error = ?e, "CreateElementsTable failed");
                std::ptr::null_mut()
            }
        }
    }));
    result.unwrap_or_else(|_| {
        tracing::error!("CreateElementsTable panicked");
        std::ptr::null_mut()
    })
}

/// DestroyElementsTable — 销毁元素表。
/// 对照源码 03_elements.c L430-453。
#[no_mangle]
pub extern "C" fn DestroyElementsTable() {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        destroy_elements_table();
    }));
}

/// GetElementIndex — 查找元素索引。
/// 对照源码 03_elements.c L456-495。
#[no_mangle]
pub extern "C" fn GetElementIndex(hash: u32) -> u16 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        get_element_index_pub(hash)
    }));
    result.unwrap_or(INVALID_ELEMENT_INDEX)
}

/// CreateElementInteractions — 创建元素交互。
/// 对照源码 03_elements.c L517-569。
#[no_mangle]
pub extern "C" fn CreateElementInteractions(reader: *mut BinaryBufferReader) -> *mut c_void {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if reader.is_null() {
            tracing::error!("CreateElementInteractions: null reader");
            return std::ptr::null_mut();
        }
        let reader = unsafe { &mut *reader };
        match create_element_interactions(reader) {
            Ok(()) => std::ptr::null_mut(),
            Err(e) => {
                tracing::error!(error = ?e, "CreateElementInteractions failed");
                std::ptr::null_mut()
            }
        }
    }));
    result.unwrap_or_else(|_| {
        tracing::error!("CreateElementInteractions panicked");
        std::ptr::null_mut()
    })
}

/// CreateElementInteractionsLocked — 创建元素交互（加锁版）。
/// 对照源码 03_elements.c L7-23。
#[no_mangle]
pub extern "C" fn CreateElementInteractionsLocked(reader: *mut BinaryBufferReader) -> *mut c_void {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if reader.is_null() {
            tracing::error!("CreateElementInteractionsLocked: null reader");
            return std::ptr::null_mut();
        }
        // 先获取 G_INTERACTIONS_LOCK（对应原版 gFrameSync.mSimMutex）
        let _guard = G_INTERACTIONS_LOCK.lock();
        let reader = unsafe { &mut *reader };
        match create_element_interactions(reader) {
            Ok(()) => std::ptr::null_mut(),
            Err(e) => {
                tracing::error!(error = ?e, "CreateElementInteractionsLocked failed");
                std::ptr::null_mut()
            }
        }
    }));
    result.unwrap_or_else(|_| {
        tracing::error!("CreateElementInteractionsLocked panicked");
        std::ptr::null_mut()
    })
}

/// 查询完整元素记录（gElements，0xa4B）。DoStateTransition 低温非固态分支读 low_temp/high_temp。
pub fn get_element_by_idx(elem_idx: u16) -> Option<Element> {
    let idx = elem_idx as usize;
    with_table(|table| table.elements.get(idx).copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;

    #[test]
    fn elements_table_default_is_empty() {
        let t = ElementsTable::default();
        assert!(t.elements.is_empty());
        assert!(t.element_indices.is_empty());
        assert!(t.gas_obliterations.is_empty());
    }

    #[test]
    fn get_element_index_inner_returns_invalid_for_missing() {
        let t = ElementsTable::default();
        assert_eq!(get_element_index_inner(&t, 12345), INVALID_ELEMENT_INDEX);
    }

    #[test]
    fn get_element_index_inner_finds_existing() {
        let mut t = ElementsTable::default();
        t.element_indices.insert(42, 5);
        assert_eq!(get_element_index_inner(&t, 42), 5);
    }

    #[test]
    fn get_element_by_idx_returns_element_or_none() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut table = G_ELEMENTS_TABLE.lock().unwrap();
        table.elements.clear();
        let mut e = Element::default();
        e.id = 0x12345678;
        e.low_temp = 100.0;
        e.high_temp = 500.0;
        table.elements.push(e);
        drop(table);
        let got = get_element_by_idx(0).expect("elem 0 exists");
        assert_eq!(got.id, 0x12345678);
        assert_eq!(got.low_temp, 100.0);
        assert_eq!(got.high_temp, 500.0);
        assert!(get_element_by_idx(0xffff).is_none());
    }

    #[test]
    fn create_elements_table_with_null_reader_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        let result = CreateElementsTable(std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn get_element_index_with_empty_table_returns_invalid() {
        let _lock = LIB_TESTS_LOCK.lock();
        let result = GetElementIndex(999);
        assert_eq!(result, INVALID_ELEMENT_INDEX);
    }

    #[test]
    fn destroy_elements_table_does_not_panic() {
        let _lock = LIB_TESTS_LOCK.lock();
        DestroyElementsTable();
    }

    #[test]
    fn create_element_interactions_with_null_reader_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        let result = CreateElementInteractions(std::ptr::null_mut());
        assert!(result.is_null());
    }

    #[test]
    fn create_element_interactions_locked_with_null_reader_returns_null() {
        let _lock = LIB_TESTS_LOCK.lock();
        let result = CreateElementInteractionsLocked(std::ptr::null_mut());
        assert!(result.is_null());
    }

    // ===== post_process_data 字段映射验证测试 =====
    // 严格对照源码 03_elements.c L302-305 的偏移映射，防止字段映射回归。

    #[test]
    fn post_process_data_field_mapping_matches_source() {
        let _lock = LIB_TESTS_LOCK.lock();
        use crate::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};

        // 构造 1 个 Element，给关键字段赋可识别值
        let mut elem = Element::default();
        elem.id = 1;
        elem.state = 1; // Gas（影响 mass_area_scale 计算，但不影响 PPD 映射）
        // PPD 映射涉及的字段（对照源码 L302-307）：
        elem.off_gas_percentage = 1.1;                  // @0x88 → ppd.off_gas_percentage @0x14（L302）
        elem.molar_mass = 2.2;                          // @0x14 → ppd.molar_mass @0x18（L303）
        elem.strength = 3.3;                            // @0x40 → ppd.strength @0x1c（L304）
        elem.max_mass = 4.4;                            // @0x34 → ppd.max_mass @0x20（L305）
        elem.solid_surface_area_multiplier = 5.5;       // @0x18 → 不应映射到 ppd.max_mass

        // 构造数据包：count=1 + 164B Element + 1 个空名称（长度=0）
        let mut w = BinaryBufferWriter::new();
        w.write_int(1);
        let elem_bytes = unsafe {
            std::slice::from_raw_parts(&elem as *const Element as *const u8, 164).to_vec()
        };
        w.write_bytes(&elem_bytes);
        w.write_int(0); // 空名称（长度=0）

        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);

        // 调用 CreateElementsTable
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");

        // 读回 post_process_data 并验证字段映射
        {
            let table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
            let ppd = &table.post_process_data[0];

            // L302: ppd.offGasPercentage(@0x14) ← elem.offGasPercentage(@0x88)
            assert_eq!(ppd.off_gas_percentage, 1.1,
                "ppd.off_gas_percentage should map from elem.off_gas_percentage (L302)");
            // L303: ppd.molarMass(@0x18) ← elem.molarMass(@0x14)
            assert_eq!(ppd.molar_mass, 2.2,
                "ppd.molar_mass should map from elem.molar_mass (L303)");
            // L304: ppd.strength(@0x1c) ← elem.strength(@0x40)
            assert_eq!(ppd.strength, 3.3,
                "ppd.strength should map from elem.strength (L304)");
            // L305: ppd.maxMass(@0x20) ← elem.maxMass(@0x34)（2026-08-02 修正映射）
            assert_eq!(ppd.max_mass, 4.4,
                "ppd.max_mass should map from elem.max_mass (L305: @0x20 ← elem@0x34)");
            // 确保 elem.solid_surface_area_multiplier(@0x18) 没有被错误映射到 ppd.max_mass
            assert_ne!(ppd.max_mass, 5.5,
                "ppd.max_mass must NOT map from elem.solid_surface_area_multiplier");
        }

        // 清理
        DestroyElementsTable();
    }

    // ===== 液体/压力/属性纹理数据 getter 测试 =====
    // 说明：简报测试依赖"既有表已被填充"，但唯一建表测试
    // （post_process_data_field_mapping_matches_source）在结束前会
    // DestroyElementsTable() 清理，导致查表必为空。因此这里为每个测试
    // 自建一张最小表（1 个气体元素，梯度色数=1），断言保持简报原样。

    /// 自建一张含 1 个元素的最小表（state=1 气体，number_of_gradient_colors=1）。
    /// 调用方需持 LIB_TESTS_LOCK，并在测试末尾 DestroyElementsTable() 清理。
    fn create_single_element_table() {
        use crate::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
        let mut elem = Element::default();
        elem.id = 0;
        elem.state = 1; // Gas：满足 liquid 测试 state & 3 != 2
        elem.number_of_gradient_colors = 1; // 满足 texture 测试 > 0

        let mut w = BinaryBufferWriter::new();
        w.write_int(1);
        let elem_bytes = unsafe {
            std::slice::from_raw_parts(&elem as *const Element as *const u8, 164).to_vec()
        };
        w.write_bytes(&elem_bytes);
        w.write_int(0); // 空名称（长度=0）
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    #[test]
    fn get_element_liquid_data_returns_filled_entry() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_single_element_table();
        let ld = get_element_liquid_data(0);
        assert!(ld.is_some());
        // 元素 0（氧气）不是液体：state & 3 != 2
        assert_ne!(ld.unwrap().state & 3, 2);
        DestroyElementsTable();
    }

    #[test]
    fn get_element_pressure_data_returns_filled_entry() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_single_element_table();
        let pd = get_element_pressure_data(0);
        assert!(pd.is_some());
        DestroyElementsTable();
    }

    #[test]
    fn get_element_property_texture_data_returns_filled_entry() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_single_element_table();
        let ptd = get_element_property_texture_data(0);
        assert!(ptd.is_some());
        assert!(ptd.unwrap().number_of_gradient_colors > 0);
        DestroyElementsTable();
    }

    /// 快照发布/销毁指针语义（生产无锁读路径的基础）：
    /// refresh 后指针非空且指向克隆数据；destroy 后置 null（回退 Mutex）。
    #[test]
    fn elements_snapshot_publish_and_destroy() {
        let _lock = LIB_TESTS_LOCK.lock();
        destroy_elements_table();
        assert!(ELEMENTS_SNAPSHOT.load(Ordering::Acquire).is_null(), "初始无快照");

        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements.clear();
            table.elements.push(Element::default());
            table.elements[0].id = 0x1234;
        }
        refresh_elements_snapshot();
        let ptr = ELEMENTS_SNAPSHOT.load(Ordering::Acquire);
        assert!(!ptr.is_null(), "refresh 后快照非空");
        let snap = unsafe { &*ptr };
        assert_eq!(snap.elements.len(), 1);
        assert_eq!(snap.elements[0].id, 0x1234, "快照内容 = Mutex 表克隆");

        // 修改 Mutex 表不影响已发布快照（旧表 leak 语义：读取者持旧数据）
        {
            let mut table = G_ELEMENTS_TABLE.lock().unwrap();
            table.elements[0].id = 0x9999;
        }
        let snap2 = unsafe { &*ptr };
        assert_eq!(snap2.elements[0].id, 0x1234, "快照不可变（克隆）");

        destroy_elements_table();
        assert!(ELEMENTS_SNAPSHOT.load(Ordering::Acquire).is_null(), "destroy 清空快照");
    }
}
