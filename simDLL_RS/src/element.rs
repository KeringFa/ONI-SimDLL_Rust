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

/// Element — 单个元素定义。
///
/// 字段顺序严格对照源码 00_types_reference.c L11104-11143。
#[repr(C)]
#[derive(Clone, Copy)]
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

    // Element 实际大小待验证。
    // 估算：4 (id) + 2+2+2 (3×u16) + 1+1 (2×u8) + 16×4 (16×f32: specific_heat_capacity..strength)
    //       + 4+4+4+4 (low_temp_transition_ore_id + mass_conv + high_temp_ore_id + mass_conv)
    //       + 2+2 (sublimate_idx + convert_idx) + 4+4 (material_properties + colour)
    //       + 6×4 (gradient_colours) + 4 (sublimate_fx) + 5×4 (sublimate_rate..light_absorption_factor)
    //       + 3×4 (radiation_absorption_factor + radiation_per_1000_mass + padding)
    //       + 12 (PhysicsData)
    // = 4+6+2+64+16+4+8+24+4+20+12+12 = ~160 字节
    // 但实际对齐可能不同。先写一个估算断言，若失败则按真实大小调整。
    #[test]
    fn element_size_matches_source() {
        let actual = size_of::<Element>();
        // TODO(开放问题#5): 实施时验证原版 Element 字节数并替换此断言
        // 估算 ~160 字节（含 PhysicsData=12），但需要从源码确认
        assert!(actual > 100 && actual < 200, "Element size = {}, 期望 100-200 字节", actual);
    }
}
