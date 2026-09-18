//! Element / PhysicsData 及附属子结构 — 元素静态定义。
//!
//! 字段严格对照源码 00_types_reference.c：
//! - PhysicsData: L10640-10644（12B）
//! - Element: L11104-11143（38 字段，164B）
//! - 9 附属子结构 + 3 交互子结构
//!
//! **关键**：Element 用 `#[repr(C)]` + derive Default（全零初始化）。
//! 从存档读取时必须用 `std::ptr::read_unaligned`（因缓冲区对齐 1，Element 对齐 4）。
//! 子结构不使用显式 pad 字段，由 `#[repr(C)]` 自动对齐（与源码自然对齐一致）。

// ===== 3 个 const 常量 =====

/// "无相变矿石"的特殊元素 hash。
/// 源码 03_elements.c L406/413：ore_id == 此值时跳过 GetElementIndex，直接设 0xffff。
/// 源码 02_save_load.c L585 / 01_sim_api.c L126 也用此 hash 调用 GetElementIndex。
pub const INVALID_ELEMENT_HASH: u32 = 0x2d39bf75;

/// 无效元素索引（u16 最大值）。
/// 源码 03_elements.c GetElementIndex L470：uVar4 = 0xffff（默认值）。
/// 找不到元素时原版**直接返回 0xffff**（不崩溃；仅哈希容器自身损坏才走
/// _invalid_parameter_noinfo_noreturn）。Rust 行为与原版一致
/// （2026-08-10 注释更正：此前误称"Rust 安全增强"）。
pub const INVALID_ELEMENT_INDEX: u16 = 0xffff;

/// CreateElementInteractions 中 entry 的 signature 值。
/// 源码 03_elements.c L537：if (*(int*)entry == 0x6826f67d) 提取 GasObliteration。
/// 注意：GasObliteration 结构本身无 signature 字段，signature 是 entry 数据前缀。
pub const GAS_OBLITERATION_SIGNATURE: i32 = 0x6826f67d_u32 as i32;

// ===== PhysicsData（12B）=====

/// PhysicsData — 元素默认物理数据（12B）。
/// 源码 00_types_reference.c L10640-10644。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct PhysicsData {
    pub temperature: f32,   // L10641
    pub mass: f32,          // L10642
    pub pressure: f32,      // L10643
}

// ===== Element（164B = 0xa4，38 字段）=====

/// Element — 元素静态定义（164B = 0xa4）。
/// 字段严格对照源码 00_types_reference.c L11104-11143。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Element {
    pub id: i32,                                          // L11105 int id
    pub low_temp_transition_idx: u16,                     // L11106 ushort lowTempTransitionIdx
    pub high_temp_transition_idx: u16,                    // L11107 ushort highTempTransitionIdx
    pub elements_table_idx: u16,                          // L11108 ushort elementsTableIdx
    pub state: u8,                                        // L11109 uchar state
    pub number_of_gradient_colors: u8,                    // L11110 uchar numberOfGradientColors
    pub specific_heat_capacity: f32,                      // L11111 float specificHeatCapacity
    pub thermal_conductivity: f32,                        // L11112 float thermalConductivity
    pub molar_mass: f32,                                  // L11113 float molarMass
    pub solid_surface_area_multiplier: f32,               // L11114 float solidSurfaceAreaMultiplier
    pub liquid_surface_area_multiplier: f32,              // L11115 float liquidSurfaceAreaMultiplier
    pub gas_surface_area_multiplier: f32,                 // L11116 float gasSurfaceAreaMultiplier
    pub flow: f32,                                        // L11117 float flow
    pub viscosity: f32,                                   // L11118 float viscosity
    pub min_horizontal_flow: f32,                         // L11119 float minHorizontalFlow
    pub min_vertical_flow: f32,                           // L11120 float minVerticalFlow
    pub max_mass: f32,                                    // L11121 float maxMass
    pub low_temp: f32,                                    // L11122 float lowTemp
    pub high_temp: f32,                                   // L11123 float highTemp
    pub strength: f32,                                    // L11124 float strength
    pub low_temp_transition_ore_id: i32,                  // L11125 int lowTempTransitionOreID
    pub low_temp_transition_ore_mass_conversion: f32,     // L11126 float lowTempTransitionOreMassConversion
    pub high_temp_transition_ore_id: i32,                 // L11127 int highTempTransitionOreID
    pub high_temp_transition_ore_mass_conversion: f32,    // L11128 float highTempTransitionOreMassConversion
    pub sublimate_index: u16,                             // L11129 ushort sublimateIndex
    pub convert_index: u16,                               // L11130 ushort convertIndex
    pub material_properties: u32,                         // L11131 uint materialProperties
    pub colour: u32,                                      // L11132 uint colour
    pub gradient_colours: [u32; 6],                       // L11133 uint gradientColours[6]
    pub sublimate_fx: i32,                                // L11134 int sublimateFX
    pub sublimate_rate: f32,                              // L11135 float sublimateRate
    pub sublimate_efficiency: f32,                        // L11136 float sublimateEfficiency
    pub sublimate_probability: f32,                       // L11137 float sublimateProbability
    pub off_gas_percentage: f32,                          // L11138 float offGasPercentage
    pub light_absorption_factor: f32,                     // L11139 float lightAbsorptionFactor
    pub radiation_absorption_factor: f32,                 // L11140 float radiationAbsorptionFactor
    pub radiation_per_1000_mass: f32,                     // L11141 float radiationPer1000Mass
    pub default_values: PhysicsData,                      // L11142 struct PhysicsData defaultValues
}

// ===== 9 个附属子结构 =====

/// ElementTemperatureData — 元素温度数据（56B = 0x38）。
/// 源码 00_types_reference.c L15464-15481。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ElementTemperatureData {
    pub state: u8,                                      // L15465 uchar state @0
    pub low_temp_transition_idx: u16,                   // L15466 ushort lowTempTransitionIdx @2
    pub high_temp_transition_idx: u16,                  // L15467 ushort highTempTransitionIdx @4
    pub low_temp_transition_ore_idx: u16,               // L15468 ushort lowTempTransitionOreIdx @6
    pub high_temp_transition_ore_idx: u16,              // L15469 ushort highTempTransitionOreIdx @8
    pub specific_heat_capacity: f32,                    // L15470 float specificHeatCapacity @12
    pub thermal_conductivity: f32,                      // L15471 float thermalConductivity @16
    pub default_mass: f32,                              // L15472 float defaultMass @20
    pub mass_area_scale: f32,                           // L15473 float massAreaScale @24
    pub gas_surface_area_multiplier: f32,               // L15474 float gasSurfaceAreaMultiplier @28
    pub liquid_surface_area_multiplier: f32,            // L15475 float liquidSurfaceAreaMultiplier @32
    pub solid_surface_area_multiplier: f32,             // L15476 float solidSurfaceAreaMultiplier @36
    pub low_temp: f32,                                  // L15477 float lowTemp @40
    pub high_temp: f32,                                 // L15478 float highTemp @44
    pub low_temp_transition_ore_mass_conversion: f32,   // L15479 float lowTempTransitionOreMassConversion @48
    pub high_temp_transition_ore_mass_conversion: f32,  // L15481 float highTempTransitionOreMassConversion @52
}

/// ElementPostProcessData — 元素后处理数据（44B = 0x2c）。
/// 源码 00_types_reference.c L10144-10157。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ElementPostProcessData {
    pub sublimate_index: u16,           // L10145 ushort sublimateIndex @0
    pub convert_index: u16,             // L10146 ushort convertIndex @2
    pub state: u8,                      // L10147 uchar state @4
    pub sublimate_rate: f32,            // L10148 float sublimateRate @8
    pub sublimate_efficiency: f32,      // L10149 float sublimateEfficiency @12
    pub sublimate_probability: f32,     // L10150 float sublimateProbability @16
    pub off_gas_percentage: f32,        // L10151 float offGasPercentage @20
    pub molar_mass: f32,                // L10152 float molarMass @24
    pub strength: f32,                  // L10153 float strength @28
    pub max_mass: f32,                  // L10154 float maxMass @32
    pub min_horizontal_flow: f32,       // L10155 float minHorizontalFlow @36
    pub sublimate_fx: i32,              // L10156 enum Type sublimateFX (int) @40
}

/// ElementLiquidData — 元素液体数据（24B = 0x18）。
/// 源码 00_types_reference.c L8802-8809。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ElementLiquidData {
    pub state: u8,                      // L8803 uchar state @0
    pub flow: f32,                      // L8804 float flow @4
    pub viscosity: f32,                 // L8805 float viscosity @8
    pub min_horizontal_flow: f32,       // L8806 float minHorizontalFlow @12
    pub min_vertical_flow: f32,         // L8807 float minVerticalFlow @16
    pub max_mass: f32,                  // L8808 float maxMass @20
}

/// ElementPressureData — 元素压力数据（8B）。
/// 源码 00_types_reference.c L5474-5477。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ElementPressureData {
    pub state: u8,                      // L5475 uchar state @0
    pub flow: f32,                      // L5476 float flow @4
}

/// ElementPropertyTextureData — 元素属性纹理数据（36B = 0x24）。
/// 源码 00_types_reference.c L9598-9604。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ElementPropertyTextureData {
    pub colour: u32,                            // L9599 uint colour @0
    pub gradient: [u32; 6],                     // L9600 uint gradient[6] @4
    pub material_properties: u32,               // L9601 uint materialProperties @28
    pub state: u8,                              // L9602 uchar state @32
    pub number_of_gradient_colors: u8,          // L9603 uchar numberOfGradientColors @33
}

/// ElementLightAbsorptionData — 元素吸光数据（8B）。
/// 源码 00_types_reference.c L13306-13309。
/// mass_scale 为运行时计算值，CreateElementsTable 阶段 4 填充。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ElementLightAbsorptionData {
    pub factor: f32,                    // L13307 float factor @0
    pub mass_scale: f32,                // L13308 float massScale @4（运行时计算）
}

/// ElementRadiationData — 元素辐射数据（8B）。
/// 源码 00_types_reference.c L7624-7627。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ElementRadiationData {
    pub factor: f32,                    // L7625 float factor @0
    pub rads_per_1000: f32,             // L7626 float rads_per_1000 @4
}

/// ElementStateData — 元素状态数据（1B）。
/// 源码 00_types_reference.c L13476-13478。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ElementStateData {
    pub state: u8,                      // L13477 uchar state @0
}

// ===== 3 个交互子结构 =====

/// GasObliteration — 气体湮灭交互（28B = 0x1c）。
/// 源码 00_types_reference.c L12220-12229。
/// 注意：结构本身无 signature 字段；CreateElementInteractions 读取的 entry（32B）
/// 前 4 字节为 signature，匹配 GAS_OBLITERATION_SIGNATURE 时提取后续字段。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct GasObliteration {
    pub elem_idx1: u16,                             // L12221 ushort elemIdx1 @0
    pub elem_idx2: u16,                             // L12222 ushort elemIdx2 @2
    pub elem_result_idx: u16,                       // L12223 ushort elemResultIdx @4
    pub interaction_probability: f32,               // L12224 float interactionProbability @8
    pub min_mass: f32,                              // L12225 float minMass @12
    pub elem1_mass_destruction_percent: f32,        // L12226 float elem1MassDestructionPercent @16
    pub elem2_mass_required_multiplier: f32,        // L12227 float elem2MassRequiredMultiplier @20
    pub elem_result_mass_creation_multiplier: f32,  // L12228 float elemResultMassCreationMultiplier @24
}

/// LiquidObliteration — 液体湮灭交互（16B = 0x10）。
/// 源码 00_types_reference.c L10516-10522。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct LiquidObliteration {
    pub elem_idx1: u16,           // L10517 ushort elemIdx1 @0
    pub elem_idx2: u16,           // L10518 ushort elemIdx2 @2
    pub prefab_idx: i16,          // L10519 short prefabIdx @4
    pub mass_threshold: f32,      // L10520 float massThreshold @8
    pub probability: f32,         // L10521 float probability @12
}

/// LiquidConversion — 液体转换交互（12B = 0xc）。
/// 源码 00_types_reference.c L8963-8968。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct LiquidConversion {
    pub elem_idx1: u16,           // L8964 ushort elemIdx1 @0
    pub elem_idx2: u16,           // L8965 ushort elemIdx2 @2
    pub mass_ratio: f32,          // L8966 float massRatio @4
    pub probability: f32,         // L8967 float probability @8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn physics_data_size_is_12() {
        assert_eq!(size_of::<PhysicsData>(), 12);
    }

    #[test]
    fn element_size_is_164() {
        let actual = size_of::<Element>();
        assert_eq!(actual, 164, "Element size = {} (0x{:x}), 期望 164 (0xa4)", actual, actual);
    }

    #[test]
    fn element_temperature_data_size_is_56() {
        assert_eq!(size_of::<ElementTemperatureData>(), 56);
    }

    #[test]
    fn element_post_process_data_size_is_44() {
        assert_eq!(size_of::<ElementPostProcessData>(), 44);
    }

    #[test]
    fn element_liquid_data_size_is_24() {
        assert_eq!(size_of::<ElementLiquidData>(), 24);
    }

    #[test]
    fn element_pressure_data_size_is_8() {
        assert_eq!(size_of::<ElementPressureData>(), 8);
    }

    #[test]
    fn element_property_texture_data_size_is_36() {
        assert_eq!(size_of::<ElementPropertyTextureData>(), 36);
    }

    #[test]
    fn element_light_absorption_data_size_is_8() {
        assert_eq!(size_of::<ElementLightAbsorptionData>(), 8);
    }

    #[test]
    fn element_radiation_data_size_is_8() {
        assert_eq!(size_of::<ElementRadiationData>(), 8);
    }

    #[test]
    fn element_state_data_size_is_1() {
        assert_eq!(size_of::<ElementStateData>(), 1);
    }

    #[test]
    fn gas_obliteration_size_is_28() {
        assert_eq!(size_of::<GasObliteration>(), 28);
    }

    #[test]
    fn liquid_obliteration_size_is_16() {
        assert_eq!(size_of::<LiquidObliteration>(), 16);
    }

    #[test]
    fn liquid_conversion_size_is_12() {
        assert_eq!(size_of::<LiquidConversion>(), 12);
    }

    #[test]
    fn const_values_correct() {
        assert_eq!(INVALID_ELEMENT_HASH, 0x2d39bf75);
        assert_eq!(INVALID_ELEMENT_INDEX, 0xffff);
        assert_eq!(GAS_OBLITERATION_SIGNATURE, 0x6826f67d_u32 as i32);
    }

    #[test]
    fn element_default_all_zero() {
        let elem = Element::default();
        assert_eq!(elem.id, 0);
        assert_eq!(elem.specific_heat_capacity, 0.0);
        assert_eq!(elem.default_values.temperature, 0.0);
    }

    // ===== 字段偏移验证测试 =====
    // 防止未来结构体变更导致偏移错误（对照 00_types_reference.c L11104-11143）。
    // 这些偏移是 CreateElementsTable 字段映射的权威依据。

    #[test]
    fn element_field_offsets_match_source() {
        use core::mem::offset_of;
        assert_eq!(offset_of!(Element, id), 0x00);
        assert_eq!(offset_of!(Element, low_temp_transition_idx), 0x04);
        assert_eq!(offset_of!(Element, high_temp_transition_idx), 0x06);
        assert_eq!(offset_of!(Element, elements_table_idx), 0x08);
        assert_eq!(offset_of!(Element, state), 0x0a);
        assert_eq!(offset_of!(Element, number_of_gradient_colors), 0x0b);
        assert_eq!(offset_of!(Element, specific_heat_capacity), 0x0c);
        assert_eq!(offset_of!(Element, thermal_conductivity), 0x10);
        assert_eq!(offset_of!(Element, molar_mass), 0x14);
        assert_eq!(offset_of!(Element, solid_surface_area_multiplier), 0x18);
        assert_eq!(offset_of!(Element, liquid_surface_area_multiplier), 0x1c);
        assert_eq!(offset_of!(Element, gas_surface_area_multiplier), 0x20);
        assert_eq!(offset_of!(Element, flow), 0x24);
        assert_eq!(offset_of!(Element, viscosity), 0x28);
        assert_eq!(offset_of!(Element, min_horizontal_flow), 0x2c);
        assert_eq!(offset_of!(Element, min_vertical_flow), 0x30);
        assert_eq!(offset_of!(Element, max_mass), 0x34);
        assert_eq!(offset_of!(Element, low_temp), 0x38);
        assert_eq!(offset_of!(Element, high_temp), 0x3c);
        assert_eq!(offset_of!(Element, strength), 0x40);
        assert_eq!(offset_of!(Element, low_temp_transition_ore_id), 0x44);
        assert_eq!(offset_of!(Element, low_temp_transition_ore_mass_conversion), 0x48);
        assert_eq!(offset_of!(Element, high_temp_transition_ore_id), 0x4c);
        assert_eq!(offset_of!(Element, high_temp_transition_ore_mass_conversion), 0x50);
        assert_eq!(offset_of!(Element, sublimate_index), 0x54);
        assert_eq!(offset_of!(Element, convert_index), 0x56);
        assert_eq!(offset_of!(Element, material_properties), 0x58);
        assert_eq!(offset_of!(Element, colour), 0x5c);
        assert_eq!(offset_of!(Element, gradient_colours), 0x60);
        assert_eq!(offset_of!(Element, sublimate_fx), 0x78);
        assert_eq!(offset_of!(Element, sublimate_rate), 0x7c);
        assert_eq!(offset_of!(Element, sublimate_efficiency), 0x80);
        assert_eq!(offset_of!(Element, sublimate_probability), 0x84);
        assert_eq!(offset_of!(Element, off_gas_percentage), 0x88);
        assert_eq!(offset_of!(Element, light_absorption_factor), 0x8c);
        assert_eq!(offset_of!(Element, radiation_absorption_factor), 0x90);
        assert_eq!(offset_of!(Element, radiation_per_1000_mass), 0x94);
        assert_eq!(offset_of!(Element, default_values), 0x98);
    }

    #[test]
    fn element_post_process_data_field_offsets_match_source() {
        use core::mem::offset_of;
        assert_eq!(offset_of!(ElementPostProcessData, sublimate_index), 0x00);
        assert_eq!(offset_of!(ElementPostProcessData, convert_index), 0x02);
        assert_eq!(offset_of!(ElementPostProcessData, state), 0x04);
        assert_eq!(offset_of!(ElementPostProcessData, sublimate_rate), 0x08);
        assert_eq!(offset_of!(ElementPostProcessData, sublimate_efficiency), 0x0c);
        assert_eq!(offset_of!(ElementPostProcessData, sublimate_probability), 0x10);
        assert_eq!(offset_of!(ElementPostProcessData, off_gas_percentage), 0x14);
        assert_eq!(offset_of!(ElementPostProcessData, molar_mass), 0x18);
        assert_eq!(offset_of!(ElementPostProcessData, strength), 0x1c);
        assert_eq!(offset_of!(ElementPostProcessData, max_mass), 0x20);
        assert_eq!(offset_of!(ElementPostProcessData, min_horizontal_flow), 0x24);
        assert_eq!(offset_of!(ElementPostProcessData, sublimate_fx), 0x28);
    }
}
