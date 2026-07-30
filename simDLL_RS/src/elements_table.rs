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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Element, INVALID_ELEMENT_INDEX};

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
}
