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

#[cfg(test)]
mod tests {
    use super::*;

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
}
