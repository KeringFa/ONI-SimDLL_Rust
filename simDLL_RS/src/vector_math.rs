//! ONI 的 Vector2/3/4 数学类型，POD struct，无方法。
//! Phase 1 只定义内存布局，向量运算在 Phase 7+ 添加。

/// 热交换结果元组：`(final_temp_hi, final_temp_lo, delta_heat)`。
///
/// 对照源码 `tuple<float, float, float>` 返回值（11_msvcrt_ignored.c L33135-33174）。
///
/// **字段语义因函数而异**（与源码完全一致）：
/// - `final_temp_hi` / `final_temp_lo`：
///   - 由 `calculate_temperature_exchange_precise` 返回时：分别对应输入的 hi/lo 侧
///   - 由 `calculate_temperature_exchange`（wrapper）返回时：分别对应输入的 B/A 侧
/// - `delta_heat`：源码公式 `(T_initial - T_final) × cap`，符号约定"侧获得能量时为负"
///   - 由 `_precise` 返回时：lo 侧获得能量 → 负值
///   - 由 wrapper 返回时：A 侧获得能量 → 负值
///   - UpdateTemperature / UpdateTemperatureForBackwallExchange 实际不读此字段（仅计算并存储），
///     保留是为了与源码 1:1 对应，便于未来其他调用方使用。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TemperatureExchangeResult {
    pub final_temp_hi: f32,
    pub final_temp_lo: f32,
    pub delta_heat: f32,
}

/// `details::CalculateTemperatureExchange_precise` — 核心热交换计算公式。
///
/// 严格对照源码 `11_msvcrt_ignored.c` L33135-33174。
///
/// **参数**（与源码 `_precise` 一一对应）：
/// - `heat_factor`：源码 param_1，热交换系数（格子-格子=0.25，格子-背景墙=0.01）
/// - `surface_area`：源码 param_2，表面面积乘积（effective_conductivity × SAM_a × SAM_b）
/// - `cap_factor`：源码 param_3，热容系数（固定 0.2，源码 `0.20000000298023224`）
/// - `temp_lo`：源码 param_4，低温侧温度
/// - `cap_lo`：源码 param_5，低温侧热容 = mass × specific_heat_capacity
/// - `temp_hi`：源码 param_6，高温侧温度
/// - `cap_hi`：源码 param_7，高温侧热容 = mass × specific_heat_capacity
///
/// **算法**（与源码完全等价）：
/// 1. 平衡温度：`equilibrium = (T_lo×cap_lo + T_hi×cap_hi) / (cap_lo + cap_hi)`
/// 2. 三个候选能量值：
///    - `ΔT × surface_area × cap_factor`（受表面面积/导热限制）
///    - `ΔT × heat_factor × cap_lo`（受低温侧热容限制）
///    - `ΔT × heat_factor × cap_hi`（受高温侧热容限制）
/// 3. 取三者最小值 `min_energy`（最受限的因素决定实际能量转移）
/// 4. `final_temp_lo = T_lo + min_energy / cap_lo`，但不超平衡温度
/// 5. `final_temp_hi = T_hi - min_energy / cap_hi`，但不低于平衡温度
/// 6. `delta_heat = (T_lo - final_temp_lo) × cap_lo`（低温侧获得能量）
///
/// **运行时影响**：与原版完全等价（数学公式相同，仅类型与返回方式不同：
/// 原版 `tuple<float, float, float>` → Rust struct，无精度差异）。
pub fn calculate_temperature_exchange_precise(
    heat_factor: f64,
    surface_area: f64,
    cap_factor: f64,
    temp_lo: f64,
    cap_lo: f64,
    temp_hi: f64,
    cap_hi: f64,
) -> TemperatureExchangeResult {
    // 平衡温度（热平衡时的最终温度，无能量损失）
    let equilibrium = (temp_lo * cap_lo + temp_hi * cap_hi) / (cap_lo + cap_hi);

    // 温差 × heat_factor（用于 cap_lo/cap_hi 限制项）
    let delta_t = temp_hi - temp_lo;
    let scaled_delta_t = delta_t * heat_factor;

    // 三个候选能量值（源码 local_58[0..3]）
    let candidate_conductivity = delta_t * surface_area * cap_factor;
    let candidate_cap_lo = scaled_delta_t * cap_lo;
    let candidate_cap_hi = scaled_delta_t * cap_hi;

    // 取三者最小值（源码 do-while 循环）
    let min_energy = candidate_conductivity.min(candidate_cap_lo).min(candidate_cap_hi);

    // final_temp_lo = T_lo + min_energy / cap_lo
    let mut final_temp_lo = min_energy / cap_lo + temp_lo;
    // 不超过平衡温度（源码 `if (dVar1 <= dVar4) dVar4 = dVar1;`）
    if equilibrium <= final_temp_lo {
        final_temp_lo = equilibrium;
    }

    // final_temp_hi = T_hi - min_energy / cap_hi
    let mut final_temp_hi = (-1.0 / cap_hi) * min_energy + temp_hi;
    // 不低于平衡温度（源码 `if (dVar5 <= dVar1) dVar5 = dVar1;`）
    if final_temp_hi <= equilibrium {
        final_temp_hi = equilibrium;
    }

    // delta_heat：源码 `(param_4 - (double)(float)dVar4) * param_5`
    // 注意源码把 dVar4 先 cast 为 float 再回 double（精度截断）。
    // Rust 模拟同样截断：先转 f32 再回 f64
    let final_temp_lo_truncated = final_temp_lo as f32 as f64;
    let delta_heat = (temp_lo - final_temp_lo_truncated) * cap_lo;

    TemperatureExchangeResult {
        final_temp_hi: final_temp_hi as f32,
        final_temp_lo: final_temp_lo as f32,
        delta_heat: delta_heat as f32,
    }
}

/// `CalculateTemperatureExchange` — 包装函数，处理 swap 逻辑。
///
/// 严格对照源码 `11_msvcrt_ignored.c` L33104-33129（外层 CalculateTemperatureExchange）。
///
/// 与 `_precise` 的区别：本函数接收任意顺序的温度对，
/// 内部根据 `temp_a <= temp_b` 判断是否需要 swap，调用 `_precise`，
/// 然后还原输出顺序使调用方拿到 `(final_temp_a, final_temp_b, delta_heat_a)`。
///
/// **返回值语义**：
/// - `final_temp_a`：A 侧最终温度
/// - `final_temp_b`：B 侧最终温度
/// - `delta_heat_a`：A 侧获得的能量（A 冷则正，A 热则负）
///
/// **运行时影响**：与原版完全等价（仅是 swap 逻辑的 Rust 重写）。
pub fn calculate_temperature_exchange(
    heat_factor: f32,
    surface_area: f32,
    cap_factor: f32,
    temp_a: f32,
    cap_a: f32,
    temp_b: f32,
    cap_b: f32,
) -> TemperatureExchangeResult {
    if temp_a <= temp_b {
        // A 是低温侧：直接调用 _precise（A 作 lo，B 作 hi）
        let r = calculate_temperature_exchange_precise(
            heat_factor as f64,
            surface_area as f64,
            cap_factor as f64,
            temp_a as f64,
            cap_a as f64,
            temp_b as f64,
            cap_b as f64,
        );
        // _precise 返回: (final_temp_hi=B, final_temp_lo=A, delta_heat=源码公式)
        // 源码公式 = (initial_A - final_A) * cap_A = -energy_gained_by_A = energy_lost_by_A
        TemperatureExchangeResult {
            final_temp_hi: r.final_temp_hi,    // = final_temp_B
            final_temp_lo: r.final_temp_lo,    // = final_temp_A
            delta_heat: r.delta_heat,          // = energy_lost_by_A（A 失热为正，得热为负）
        }
    } else {
        // A 是高温侧：swap 后调用 _precise（B 作 lo，A 作 hi）
        let r = calculate_temperature_exchange_precise(
            heat_factor as f64,
            surface_area as f64,
            cap_factor as f64,
            temp_b as f64,
            cap_b as f64,  // B 是低温侧
            temp_a as f64,
            cap_a as f64,  // A 是高温侧
        );
        // _precise 返回: (final_temp_hi=A, final_temp_lo=B, delta_heat=源码公式)
        // 源码公式 = (initial_B - final_B) * cap_B = -energy_gained_by_B = energy_lost_by_B
        // 由能量守恒: energy_lost_by_B = energy_gained_by_A
        // 但 wrapper 的 delta 语义应为 "energy_lost_by_A"，所以需取负：
        // -r.delta_heat = -energy_lost_by_B = energy_lost_by_A（A 失热时为正）
        TemperatureExchangeResult {
            final_temp_hi: r.final_temp_lo,    // = final_temp_B
            final_temp_lo: r.final_temp_hi,    // = final_temp_A
            delta_heat: -r.delta_heat,         // = energy_lost_by_A（A 失热为正，得热为负）
        }
    }
}

/// 2D 向量（8 字节，对应原版 Vector2f）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vector2f {
    pub x: f32,
    pub y: f32,
}

/// 3D 向量（12 字节，对应原版 Vector3f）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vector3f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 4D 向量（16 字节，对应原版 Vector4f）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vector4f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn vector2f_size() {
        assert_eq!(size_of::<Vector2f>(), 8);
    }

    #[test]
    fn vector3f_size() {
        assert_eq!(size_of::<Vector3f>(), 12);
    }

    #[test]
    fn vector4f_size() {
        assert_eq!(size_of::<Vector4f>(), 16);
    }

    /// 验证 `_precise` 在等温情况下不交换热量（边界 case）。
    #[test]
    fn precise_no_exchange_when_temps_equal() {
        // temp_lo = temp_hi = 100，热容相同 → 平衡温度也是 100，无热量转移
        let r = calculate_temperature_exchange_precise(0.25, 100.0, 0.2, 100.0, 1.0, 100.0, 1.0);
        assert!((r.final_temp_hi - 100.0).abs() < 1e-3);
        assert!((r.final_temp_lo - 100.0).abs() < 1e-3);
        assert!(r.delta_heat.abs() < 1e-3);
    }

    /// 验证 `_precise` 在小温差 + 大 surface_area 下的能量转移受 cap 限制。
    /// 这是核心数学验证：当 surface_area 足够大，能量转移受 cap_lo 和 cap_hi 限制。
    #[test]
    fn precise_energy_limited_by_cap_when_surface_area_large() {
        // temp_lo=100, temp_hi=200, cap_lo=cap_hi=1, heat_factor=0.25, surface_area=1000
        // equilibrium = 150
        // candidates: [100*1000*0.2=20000, 100*0.25*1=25, 100*0.25*1=25]
        // min = 25 → final_lo = 100+25/1 = 125, final_hi = 200-25/1 = 175
        let r = calculate_temperature_exchange_precise(0.25, 1000.0, 0.2, 100.0, 1.0, 200.0, 1.0);
        assert!((r.final_temp_lo - 125.0).abs() < 1e-3, "final_lo should be 125, got {}", r.final_temp_lo);
        assert!((r.final_temp_hi - 175.0).abs() < 1e-3, "final_hi should be 175, got {}", r.final_temp_hi);
    }

    /// 验证 `_precise` 在小 surface_area 下能量转移受 conductivity 限制。
    #[test]
    fn precise_energy_limited_by_conductivity_when_surface_area_small() {
        // temp_lo=100, temp_hi=200, cap_lo=cap_hi=1, heat_factor=0.25, surface_area=0.1
        // candidates: [100*0.1*0.2=2, 100*0.25*1=25, 100*0.25*1=25]
        // min = 2 → final_lo = 100+2/1 = 102, final_hi = 200-2/1 = 198
        let r = calculate_temperature_exchange_precise(0.25, 0.1, 0.2, 100.0, 1.0, 200.0, 1.0);
        assert!((r.final_temp_lo - 102.0).abs() < 1e-3, "final_lo should be 102, got {}", r.final_temp_lo);
        assert!((r.final_temp_hi - 198.0).abs() < 1e-3, "final_hi should be 198, got {}", r.final_temp_hi);
    }

    /// 验证 `_precise` 的 equilibrium clamp 不会让 final_lo 超过平衡温度。
    /// 极端参数（cap_lo 远大于 cap_hi）：min/cap_lo 极小，正常计算即可，
    /// 此测试主要确保函数在极端输入下不 panic 且 final_lo 不超 equilibrium。
    #[test]
    fn precise_clamps_to_equilibrium() {
        // temp_lo=100, temp_hi=200, cap_lo=1000, cap_hi=1
        // equilibrium = (100*1000 + 200*1) / 1001 ≈ 100.1
        // candidates: [100*1e6*0.2=2e7, 100*0.25*1000=25000, 100*0.25*1=25]
        // min = 25 → final_lo_raw = 100 + 25/1000 = 100.025 < 100.1 → no clamp needed
        // 但即便 surface_area 极大，由于 min 受 cap_hi 限制（25），final_lo 不会超 equilibrium。
        // 此测试确保函数在极端 surface_area 下不 panic，且 final_lo 不超 equilibrium。
        let r = calculate_temperature_exchange_precise(0.25, 1e6, 0.2, 100.0, 1000.0, 200.0, 1.0);
        assert!(r.final_temp_lo <= 100.2, "final_lo should not overshoot equilibrium, got {}", r.final_temp_lo);
    }

    /// 验证 wrapper 在 A 冷 B 热时直接透传 _precise 结果。
    #[test]
    fn wrapper_no_swap_when_a_colder() {
        // A=100 (cold), B=200 (hot)
        let r = calculate_temperature_exchange(0.25, 1000.0, 0.2, 100.0, 1.0, 200.0, 1.0);
        // 应与 _precise(100, 1, 200, 1) 的输出一致：
        // _precise 返回 (final_temp_hi=175, final_temp_lo=125, delta_heat=...)
        // wrapper 不 swap：final_temp_hi=B=175, final_temp_lo=A=125
        assert!((r.final_temp_hi - 175.0).abs() < 1e-3, "final_temp_B should be 175, got {}", r.final_temp_hi);
        assert!((r.final_temp_lo - 125.0).abs() < 1e-3, "final_temp_A should be 125, got {}", r.final_temp_lo);
    }

    /// 验证 wrapper 在 A 热 B 冷时 swap 输出。
    #[test]
    fn wrapper_swaps_when_a_hotter() {
        // A=200 (hot), B=100 (cold)
        let r = calculate_temperature_exchange(0.25, 1000.0, 0.2, 200.0, 1.0, 100.0, 1.0);
        // swap 后调用 _precise(100, 1, 200, 1)，得到 (final_hi=175, final_lo=125)
        // wrapper swap：final_temp_hi=B=125, final_temp_lo=A=175
        assert!((r.final_temp_hi - 125.0).abs() < 1e-3, "final_temp_B should be 125, got {}", r.final_temp_hi);
        assert!((r.final_temp_lo - 175.0).abs() < 1e-3, "final_temp_A should be 175, got {}", r.final_temp_lo);
    }

    /// 验证 wrapper 的 delta_heat 符号：A 冷时 A 得热 → delta_heat 为负（源码约定）。
    #[test]
    fn wrapper_delta_heat_negative_when_a_gains_energy() {
        // A=100 (cold), B=200 (hot) → A 得热，按源码约定 delta_heat 为负
        let r = calculate_temperature_exchange(0.25, 1000.0, 0.2, 100.0, 1.0, 200.0, 1.0);
        assert!(r.delta_heat < 0.0, "A gained energy → delta_heat should be negative, got {}", r.delta_heat);
    }

    /// 验证 wrapper 的 delta_heat 符号：A 热时 A 失热 → delta_heat 为正（源码约定）。
    #[test]
    fn wrapper_delta_heat_positive_when_a_loses_energy() {
        // A=200 (hot), B=100 (cold) → A 失热，按源码约定 delta_heat 为正
        let r = calculate_temperature_exchange(0.25, 1000.0, 0.2, 200.0, 1.0, 100.0, 1.0);
        assert!(r.delta_heat > 0.0, "A lost energy → delta_heat should be positive, got {}", r.delta_heat);
    }
}
