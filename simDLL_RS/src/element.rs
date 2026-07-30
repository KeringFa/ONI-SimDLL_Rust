//! Element struct — 元素定义（OxygenNotIncluded 的"物质"基础单位）。
//!
//! 字段对照源码：
//! - PhysicsData: 00_types_reference.c L10640-10644
//! - Element: 00_types_reference.c L11104-11143
//!
//! Phase 1 只定义字段，不实现方法（CreateElementsTable/GetElementIndex 在 Phase 2）。

/// 元素状态（固态/液态/气态/etc.）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementState {
    pub value: u8,
}

/// PhysicsData — Element 的默认物理属性。
/// 用于元素相变时还原默认温度/质量/压力等。
///
/// 对照源码 00_types_reference.c L10640-10644（3 字段：temperature/mass/pressure）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PhysicsData {
    pub temperature: f32,
    pub mass: f32,
    pub pressure: f32,
}

/// 无效元素 ID 哈希常量（对应源码 0x2d39bf75）。
/// oreID == 此值时表示"无相变"，GetElementIndex 结果设为 0xffff。
pub const INVALID_ELEMENT_HASH: u32 = 0x2d39bf75;

/// 无效元素索引（对应源码 0xffff = 65535）。
pub const INVALID_ELEMENT_INDEX: u16 = 0xffff;

/// GasObliteration 签名常量（对应源码 0x6826f67d as i32，源码用 *(int*) 比较）。
/// CreateElementInteractions 中 entry.signature == 此值才提取 GasObliteration。
pub const GAS_OBLITERATION_SIGNATURE: i32 = 0x6826f67d_u32 as i32;

/// Element — 单个元素定义。
///
/// 字段顺序严格对照源码 00_types_reference.c L11104-11143。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Element {
    pub id: i32,                                       // L11105
    pub low_temp_transition_idx: u16,                  // L11106
    pub high_temp_transition_idx: u16,                  // L11107
    pub elements_table_idx: u16,                       // L11108
    pub state: u8,                                     // L11109
    pub number_of_gradient_colors: u8,                 // L11110
    pub specific_heat_capacity: f32,                  // L11111
    pub thermal_conductivity: f32,                     // L11112
    pub molar_mass: f32,                               // L11113
    pub solid_surface_area_multiplier: f32,            // L11114
    pub liquid_surface_area_multiplier: f32,           // L11115
    pub gas_surface_area_multiplier: f32,              // L11116
    pub flow: f32,                                     // L11117
    pub viscosity: f32,                                // L11118
    pub min_horizontal_flow: f32,                      // L11119
    pub min_vertical_flow: f32,                        // L11120
    pub max_mass: f32,                                 // L11121
    pub low_temp: f32,                                 // L11122
    pub high_temp: f32,                                // L11123
    pub strength: f32,                                 // L11124
    pub low_temp_transition_ore_id: i32,               // L11125
    pub low_temp_transition_ore_mass_conversion: f32,   // L11126
    pub high_temp_transition_ore_id: i32,              // L11127
    pub high_temp_transition_ore_mass_conversion: f32, // L11128
    pub sublimate_index: u16,                          // L11129
    pub convert_index: u16,                            // L11130
    pub material_properties: u32,                      // L11131
    pub colour: u32,                                   // L11132
    pub gradient_colours: [u32; 6],                    // L11133
    pub sublimate_fx: i32,                             // L11134
    pub sublimate_rate: f32,                          // L11135
    pub sublimate_efficiency: f32,                    // L11136
    pub sublimate_probability: f32,                   // L11137
    pub off_gas_percentage: f32,                      // L11138
    pub light_absorption_factor: f32,                 // L11139
    pub radiation_absorption_factor: f32,             // L11140
    pub radiation_per_1000_mass: f32,                 // L11141
    pub default_values: PhysicsData,                   // L11142
}

/// 9 个附属 struct（按规格 §2.3 字段映射，全部 #[repr(C)] + Clone, Copy, Default）

/// ElementTemperatureData — 元素温度相变数据（stride 0x38=56B）。
/// 对照源码 00_types_reference.c L15462-15481（16 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementTemperatureData {
    pub state: u8,                                       // @0
    _pad1: u8,                                           // @1
    pub low_temp_transition_idx: u16,                    // @2
    pub high_temp_transition_idx: u16,                    // @4
    pub low_temp_transition_ore_idx: u16,                // @6（后处理填入）
    pub high_temp_transition_ore_idx: u16,                // @8（后处理填入）
    _pad2: u16,                                          // @0xa
    pub specific_heat_capacity: f32,                     // @0xc
    pub thermal_conductivity: f32,                       // @0x10
    pub default_mass: f32,                               // @0x14
    pub mass_area_scale: f32,                            // @0x18（运行时计算）
    pub gas_surface_area_multiplier: f32,                 // @0x1c
    pub liquid_surface_area_multiplier: f32,              // @0x20
    pub solid_surface_area_multiplier: f32,               // @0x24
    pub low_temp: f32,                                   // @0x28
    pub high_temp: f32,                                   // @0x2c
    pub low_temp_transition_ore_mass_conversion: f32,     // @0x30
    pub high_temp_transition_ore_mass_conversion: f32,    // @0x34
}

/// ElementPostProcessData — 元素后处理数据（stride 0x2c=44B）。
/// 对照源码 00_types_reference.c L10144-10157（12 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementPostProcessData {
    pub sublimate_index: u16,                            // @0
    pub convert_index: u16,                              // @2
    pub state: u8,                                       // @4
    _pad1: [u8; 3],                                      // @5-7
    pub sublimate_rate: f32,                             // @8
    pub sublimate_efficiency: f32,                       // @0xc
    pub sublimate_probability: f32,                      // @0x10
    pub off_gas_percentage: f32,                         // @0x14
    pub molar_mass: f32,                                 // @0x18
    pub strength: f32,                                   // @0x1c
    pub max_mass: f32,                                   // @0x20
    pub min_horizontal_flow: f32,                       // @0x24
    pub sublimate_fx: i32,                              // @0x28
}

/// ElementLiquidData — 元素液体数据（stride 0x18=24B）。
/// 对照源码 00_types_reference.c L8802-8809（6 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementLiquidData {
    pub state: u8,                                       // @0
    _pad1: [u8; 3],                                      // @1-3
    pub flow: f32,                                       // @4
    pub viscosity: f32,                                 // @8
    pub min_horizontal_flow: f32,                        // @0xc
    pub min_vertical_flow: f32,                          // @0x10
    pub max_mass: f32,                                   // @0x14
}

/// ElementPhysicsData — type alias，复用 Phase 1 的 PhysicsData（12B）。
/// 对照规格 §2.3.4：源码 L320-328 直接拷贝 Element.defaultValues 的 12 字节。
pub type ElementPhysicsData = PhysicsData;

/// ElementPressureData — 元素压力数据（stride 0x8=8B）。
/// 对照源码 00_types_reference.c L5474-5477（2 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementPressureData {
    pub state: u8,                                       // @0
    _pad1: [u8; 3],                                      // @1-3
    pub flow: f32,                                       // @4
}

/// ElementPropertyTextureData — 元素属性纹理数据（stride 0x24=36B）。
/// 对照源码 00_types_reference.c L9598-9604（5 字段，含 [u32;6] 数组）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementPropertyTextureData {
    pub colour: u32,                                     // @0
    pub gradient: [u32; 6],                              // @4
    pub material_properties: u32,                        // @0x1c
    pub state: u8,                                       // @0x20
    pub number_of_gradient_colors: u8,                  // @0x21
    _pad1: [u8; 2],                                      // @0x22-0x23
}

/// ElementLightAbsorptionData — 元素光吸收数据（stride 0x8=8B）。
/// 对照源码 00_types_reference.c L13306-13309（2 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementLightAbsorptionData {
    pub factor: f32,                                     // @0
    pub mass_scale: f32,                                 // @4（运行时计算）
}

/// ElementRadiationData — 元素辐射数据（stride 0x8=8B）。
/// 对照源码 00_types_reference.c L7624-7627（2 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementRadiationData {
    pub factor: f32,                                     // @0
    pub rads_per_1000: f32,                              // @4
}

/// ElementStateData — 元素状态数据（stride 0x1=1B）。
/// 对照源码 00_types_reference.c L13476-13478（1 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ElementStateData {
    pub state: u8,                                       // @0
}

/// 3 个交互 struct

/// GasObliteration — 气体湮灭交互（stride 0x1c=28B）。
/// 对照源码 00_types_reference.c L12220-12229（8 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GasObliteration {
    pub elem_idx1: u16,                                  // @0
    pub elem_idx2: u16,                                  // @2
    pub elem_result_idx: u16,                           // @4
    _pad1: u16,                                          // @6
    pub interaction_probability: f32,                   // @8
    pub min_mass: f32,                                   // @0xc
    pub elem1_mass_destruction_percent: f32,             // @0x10
    pub elem2_mass_required_multiplier: f32,             // @0x14
    pub elem_result_mass_creation_multiplier: f32,      // @0x18
}

/// LiquidObliteration — 液体湮灭交互（stride 0x10=16B）。
/// 对照源码 00_types_reference.c L10516-10522（5 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LiquidObliteration {
    pub elem_idx1: u16,                                  // @0
    pub elem_idx2: u16,                                  // @2
    pub prefab_idx: i16,                                 // @4
    _pad1: u16,                                          // @6
    pub mass_threshold: f32,                             // @8
    pub probability: f32,                                // @0xc
}

/// LiquidConversion — 液体转换交互（stride 0xc=12B）。
/// 对照源码 00_types_reference.c L8963-8968（4 字段）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LiquidConversion {
    pub elem_idx1: u16,                                  // @0
    pub elem_idx2: u16,                                  // @2
    pub mass_ratio: f32,                                 // @4
    pub probability: f32,                                // @8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn physics_data_size() {
        // 3 × f32 = 12 字节（源码 00_types_reference.c L10640-10644 确认 3 字段）
        assert_eq!(size_of::<PhysicsData>(), 12);
    }

    #[test]
    fn element_state_size() {
        assert_eq!(size_of::<ElementState>(), 1);
    }

    // Phase 1 遗留观察项 #2 清理：把宽松断言改为精确 164B
    #[test]
    fn element_size_matches_source() {
        assert_eq!(size_of::<Element>(), 164, "Element 必须 164 字节（源码 0xa4）");
    }

    #[test]
    fn element_temperature_data_size() {
        assert_eq!(size_of::<ElementTemperatureData>(), 56);
    }

    #[test]
    fn element_post_process_data_size() {
        assert_eq!(size_of::<ElementPostProcessData>(), 44);
    }

    #[test]
    fn element_liquid_data_size() {
        assert_eq!(size_of::<ElementLiquidData>(), 24);
    }

    #[test]
    fn element_pressure_data_size() {
        assert_eq!(size_of::<ElementPressureData>(), 8);
    }

    #[test]
    fn element_property_texture_data_size() {
        assert_eq!(size_of::<ElementPropertyTextureData>(), 36);
    }

    #[test]
    fn element_light_absorption_data_size() {
        assert_eq!(size_of::<ElementLightAbsorptionData>(), 8);
    }

    #[test]
    fn element_radiation_data_size() {
        assert_eq!(size_of::<ElementRadiationData>(), 8);
    }

    #[test]
    fn element_state_data_size() {
        assert_eq!(size_of::<ElementStateData>(), 1);
    }

    #[test]
    fn gas_obliteration_size() {
        assert_eq!(size_of::<GasObliteration>(), 28);
    }

    #[test]
    fn liquid_obliteration_size() {
        assert_eq!(size_of::<LiquidObliteration>(), 16);
    }

    #[test]
    fn liquid_conversion_size() {
        assert_eq!(size_of::<LiquidConversion>(), 12);
    }

    #[test]
    fn element_physics_data_alias_is_physics_data() {
        // type alias 应与 PhysicsData 同大小
        assert_eq!(size_of::<ElementPhysicsData>(), size_of::<PhysicsData>());
        assert_eq!(size_of::<ElementPhysicsData>(), 12);
    }
}
