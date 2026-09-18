//! C2 辐射模拟（阶段 A：格子级辐射核心）。
//!
//! 对照原版 11_msvcrt_ignored.c L42594-42914（UpdateData 辐射段）：
//! 1. 宇宙辐射 occlusion（自顶向下，元素 radiationAbsorptionFactor 遮挡）；
//! 2. 衰减（radiation -= radiation/lingerRate，零值 −1 清理）；
//! 3. 元素辐射扩散（25 格权重核，mass×0.001×radsPer1000×weight）；
//! 4. 宇宙辐射应用（occlusion>0 → radiation += cosmic/lingerRate × occlusion）；
//! 5. 钳制 [0, 9e6]，≤0.01 → 0。
//!
//! ⚠️ DLC 专属门控：全部逻辑包在 `sim.radiation_enabled` 内（原版 L42594），
//! 单星（AllocateCells flag1=false）零执行，cells.radiation 恒 0。
use crate::a_framework::sim_data::SimData;
use crate::b_elements::elements_table;
use crate::d1_activity::RegionBounds;

/// 25 格元素辐射权重核（原版 L42602-42658 adjacentOffsets：dx, dy, weight）。
const KERNEL: [(i32, i32, f32); 25] = [
    (-2, -2, 0.1),
    (-1, -2, 0.15),
    (0, -2, 0.25),
    (1, -2, 0.15),
    (2, -2, 0.1),
    (-2, -1, 0.15),
    (-1, -1, 0.5),
    (0, -1, 0.75),
    (1, -1, 0.5),
    (2, -1, 0.15),
    (-2, 0, 0.25),
    (-1, 0, 0.75),
    (0, 0, 1.0),
    (1, 0, 0.75),
    (2, 0, 0.25),
    (-2, 1, 0.15),
    (-1, 1, 0.5),
    (0, 1, 0.75),
    (1, 1, 0.5),
    (2, 1, 0.15),
    (-2, 2, 0.1),
    (-1, 2, 0.15),
    (0, 2, 0.25),
    (1, 2, 0.15),
    (2, 2, 0.1),
];

/// 原版 SIM_MAX_RADIATION（L42903）。
const SIM_MAX_RADIATION: f32 = 9e6;

/// 辐射模拟（原版 UpdateData L42594-42914，radiationEnabled 门控）。
/// `cosmic_intensity` = 当前 active region 的 currentCosmicRadiationIntensity。
pub fn run_radiation_task(sim: &mut SimData, bounds: RegionBounds, cosmic_intensity: f32) {
    if !sim.radiation_enabled {
        return;
    }
    let w = sim.width as usize;
    let h = sim.height as usize;
    if w < 3 || h < 3 || sim.updated_cells.ptr.is_null() {
        return;
    }
    let (min_x, min_y, max_x, max_y) = (bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y);
    if min_x > max_x || min_y > max_y || max_x >= w || max_y >= h {
        return;
    }
    let max_mass = sim.radiation_max_mass;
    let linger = sim.radiation_linger_rate;
    let base_w = sim.radiation_base_weight;
    let dens_w = sim.radiation_density_weight;
    let const_f = sim.radiation_constructed_factor;
    // 阶段 4：RadiationSickness 病菌索引（原版 L42860-42870 在循环外算一次；
    // gDisease 缺失 → 跳过贡献）。
    let rs_idx = {
        let ptr = crate::globals::G_DISEASE.lock().0;
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { &*ptr }.get_disease_index(0xd49f77d6))
        }
    };

    // 区域局部 occlusion 缓冲（d3 并行安全）。原版 L42688 对 SimData 全局字段
    // cosmicRadiationOcclusion 执行 resize(total, 1.0)，随后在 active region 循环内
    // 自顶向下填充 region 矩形——原版单线程串行，字段复用无竞争；并行路径下多个
    // region 线程同时对该共享字段 resize/全填会产生数据竞争：一个线程的"填 1.0"
    // 会覆盖另一线程已算好的遮挡，导致宇宙辐射间歇性穿透方块（全图照射）。
    // 改为区域局部缓冲：region 顶行以上视为 1.0（原版 `iVar17 < fStack_1fc` 检查，
    // region 顶行 y+1 == max_y 不满足 < max_y → 1.0），region 内自顶向下传播，
    // 逐格语义与原版一致，且零共享、天然并行安全。
    let rw = max_x - min_x;
    let rh = max_y - min_y;
    let mut occlusion = vec![1.0f32; rw * rh];

    // —— 第一遍：top→bottom（y 从 max_y 递减，上方格 cell+width 先算）——
    // occlusion + 衰减
    for y in (min_y..max_y).rev() {
        for x in min_x..max_x {
            let cell = y * w + x;
            let (elem, props, mass, rad) = {
                let u = unsafe { &*sim.updated_cells.ptr };
                (
                    u.element_idx.get(cell),
                    u.properties.get(cell),
                    u.mass.get(cell),
                    u.radiation.get(cell),
                )
            };
            let factor = match elements_table::get_element_radiation_data(elem) {
                Some(d) => d.factor,
                None => 0.0,
            };
            // block：构造格（props & 0x80）→ factor×constructedFactor；否则质量加权
            let block = if props & 0x80 != 0 {
                factor * const_f
            } else {
                (mass / max_mass) * factor * dens_w + factor * base_w
            };
            let block = block.clamp(0.0, 1.0);
            // 原版 L42760：`iVar17 = y + 1; if (iVar17 < region.max_y)` 才读上方格
            // occlusion，否则视为 1.0（区域顶行及以上独立于区域外格子）。
            let above_occ = if y + 1 < max_y {
                occlusion[(y + 1 - min_y) * rw + (x - min_x)]
            } else {
                1.0
            };
            // 原版 L42760-42775：occlusion = (1-block)×above，<0.01 → 0
            let mut occ = (1.0 - block) * above_occ;
            if occ < 0.01 {
                occ = 0.0;
            }
            occlusion[(y - min_y) * rw + (x - min_x)] = occ;

            // 衰减（原版 L42785-42810）
            let f = rad / linger;
            let new_rad = if f == 0.0 {
                (rad - 1.0).max(0.0)
            } else {
                (rad - f).max(0.0)
            };
            let u = unsafe { &mut *sim.updated_cells.ptr };
            u.radiation.set(cell, new_rad);
        }
    }

    // —— 第二遍：元素扩散 → 宇宙应用 → 钳制 ——
    for y in min_y..max_y {
        for x in min_x..max_x {
            let cell = y * w + x;
            let (elem, mass) = {
                let u = unsafe { &*sim.updated_cells.ptr };
                (u.element_idx.get(cell), u.mass.get(cell))
            };
            let rads_per_1000 = match elements_table::get_element_radiation_data(elem) {
                Some(d) => d.rads_per_1000,
                None => 0.0,
            };
            // 元素辐射扩散（原版 L42810-42850）
            if rads_per_1000 > 0.0 {
                for &(dx, dy, weight) in &KERNEL {
                    let tx = x as i32 + dx;
                    let ty = y as i32 + dy;
                    if tx >= min_x as i32
                        && tx <= max_x as i32
                        && ty >= min_y as i32
                        && ty <= max_y as i32
                    {
                        let t = (ty as usize) * w + (tx as usize);
                        let add = mass * 0.001 * rads_per_1000 * weight;
                        let u = unsafe { &mut *sim.updated_cells.ptr };
                        u.radiation.set(t, u.radiation.get(t) + add);
                    }
                }
            }
            // 病菌辐射贡献（原版 L42860-42870）：RadiationSickness 病菌自身辐射
            // radiation[cell] += disease_count[cell] × 0.001
            if let Some(rs) = rs_idx {
                let u = unsafe { &mut *sim.updated_cells.ptr };
                if u.disease_idx.get(cell) == rs {
                    u.radiation.set(
                        cell,
                        u.radiation.get(cell) + u.disease_count.get(cell) as f32 * 0.001,
                    );
                }
            }
            // 宇宙辐射应用（原版 L42875-42884）
            let occ = occlusion[(y - min_y) * rw + (x - min_x)];
            if occ > 0.0 {
                let u = unsafe { &mut *sim.updated_cells.ptr };
                u.radiation.set(cell, u.radiation.get(cell) + (cosmic_intensity / linger) * occ);
            }
            // 钳制（原版 L42885-42910）
            let u = unsafe { &mut *sim.updated_cells.ptr };
            let mut r = u.radiation.get(cell).clamp(0.0, SIM_MAX_RADIATION);
            if r <= 0.01 {
                r = 0.0;
            }
            u.radiation.set(cell, r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
    use crate::a_framework::sim_data::SimData;
    use crate::b_elements::element::Element;
    use crate::b_elements::elements_table::{CreateElementsTable, DestroyElementsTable, G_ELEMENTS_TABLE};
    use crate::LIB_TESTS_LOCK;

    /// 最小元素表：0=真空（factor=0）、1=固体（factor=0.5, rads=0）、
    /// 2=放射性元素（factor=0.2, radsPer1000=1000）。
    fn init_radiation_table() {
        let mut table = G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        table.elements.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        table.radiation_data.clear();
        for (id, state, factor, rads) in [
            (0i32, 0u8, 0.0f32, 0.0f32),
            (1i32, 3u8, 0.5f32, 0.0f32),
            (2i32, 3u8, 0.2f32, 1000.0f32),
            (3i32, 3u8, 1.0f32, 0.0f32),
        ] {
            let mut e = Element::default();
            e.id = id;
            e.state = state;
            table.elements.push(e);
            table.state_data.push(crate::b_elements::element::ElementStateData { state });
            table.liquid_data.push(crate::b_elements::element::ElementLiquidData {
                state,
                ..Default::default()
            });
            table.pressure_data.push(crate::b_elements::element::ElementPressureData {
                state,
                ..Default::default()
            });
            table.post_process_data.push(crate::b_elements::element::ElementPostProcessData {
                state,
                ..Default::default()
            });
            table.radiation_data.push(crate::b_elements::element::ElementRadiationData {
                factor,
                rads_per_1000: rads,
            });
        }
        table
            .element_indices
            .insert(0u32, 0u16);
        table.element_indices.insert(1u32, 1u16);
        table.element_indices.insert(2u32, 2u16);
        table.element_indices.insert(3u32, 3u16);
    }

    fn make_sim() -> SimData {
        let mut sd = SimData::new_for_allocate(8, 8, 1, true, false); // radiation_enabled=true
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        sd.radiation_linger_rate = 1.1;
        sd.radiation_max_mass = 2000.0;
        sd.radiation_base_weight = 0.3;
        sd.radiation_density_weight = 0.7;
        sd.radiation_constructed_factor = 0.8;
        sd
    }

    fn fill_both(sd: &mut SimData, f: impl Fn(&mut crate::a_framework::sim_data::CellSOA)) {
        unsafe {
            f(&mut *sd.cells.ptr);
            f(&mut *sd.updated_cells.ptr);
        }
    }

    // ===== 阶段 4：RadiationSickness 病菌辐射贡献（原版 L42860-42870）=====

    /// 病菌表挂 G_DISEASE：idx0 = RadiationSickness（hash 0xd49f77d6）、idx1 = 其他病菌。
    fn install_rs_table(include_rs: bool) {
        use crate::b_elements::disease::{Disease, DiseaseInfo};
        let mut t = Disease::new();
        if include_rs {
            t.diseases.push_unchecked(DiseaseInfo {
                hash_id: 0xd49f77d6,
                ..Default::default()
            });
        }
        t.diseases.push_unchecked(DiseaseInfo {
            hash_id: 0x9999,
            ..Default::default()
        });
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
        }
        g.0 = Box::into_raw(Box::new(t));
    }

    fn uninstall_disease_table() {
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
            g.0 = std::ptr::null_mut();
        }
    }

    fn set_cell_disease(sd: &mut SimData, cell: usize, d_idx: u8, count: i32) {
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let c = &mut *buf_ptr;
                c.disease_idx.set(cell, d_idx);
                c.disease_count.set(cell, count);
            }
        }
    }

    fn set_cell_radiation(sd: &mut SimData, cell: usize, rad: f32) {
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.radiation.set(cell, rad);
        }
    }

    #[test]
    fn radiation_sickness_disease_contributes_count_times_0001() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        init_radiation_table();
        install_rs_table(true);
        let mut sd = make_sim();
        // cell9 = (1,1) 在 full_grid_bounds(8×8) 区域内；
        // disease_idx=0（RS）、count=5000 → radiation += 5000×0.001 = 5.0
        set_cell_disease(&mut sd, 9, 0, 5000);
        set_cell_radiation(&mut sd, 9, 0.0);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, bounds, 0.0);
        unsafe {
            assert!(((*sd.updated_cells.ptr).radiation.get(9) - 5.0).abs() < 1e-4);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn non_radiation_sickness_disease_does_not_contribute() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        init_radiation_table();
        install_rs_table(true);
        let mut sd = make_sim();
        // cell9 disease_idx=1（非 RS）、count=5000 → 不贡献
        set_cell_disease(&mut sd, 9, 1, 5000);
        set_cell_radiation(&mut sd, 9, 0.0);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, bounds, 0.0);
        unsafe {
            assert_eq!((*sd.updated_cells.ptr).radiation.get(9), 0.0);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn no_disease_cell_contributes_zero() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        init_radiation_table();
        install_rs_table(true);
        let mut sd = make_sim();
        // cell9 无病菌（0xff、count=0）→ 原版 uVar14==0xff 匹配但 count=0 → +0
        set_cell_disease(&mut sd, 9, 0xff, 0);
        set_cell_radiation(&mut sd, 9, 1.0);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, bounds, 0.0);
        unsafe {
            // 辐射任务第一遍先衰减：1.0 − 1.0/1.1 = 0.090909；病菌贡献 count=0 → +0
            let expected = 1.0 - 1.0 / 1.1;
            assert!(
                ((*sd.updated_cells.ptr).radiation.get(9) - expected).abs() < 1e-5,
                "count=0 → +0（仅衰减）"
            );
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn missing_disease_table_skips_contribution() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        init_radiation_table();
        uninstall_disease_table();
        let mut sd = make_sim();
        set_cell_disease(&mut sd, 9, 0, 5000);
        set_cell_radiation(&mut sd, 9, 0.0);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, bounds, 0.0); // 不 panic
        unsafe {
            assert_eq!((*sd.updated_cells.ptr).radiation.get(9), 0.0);
        }
        DestroyElementsTable();
    }

    #[test]
    fn table_without_radiation_sickness_is_noop() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        init_radiation_table();
        install_rs_table(false); // 表无 RS → GetDiseaseIndex 返回 0xff
        let mut sd = make_sim();
        set_cell_disease(&mut sd, 9, 0, 5000);
        set_cell_radiation(&mut sd, 9, 0.0);
        let bounds = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, bounds, 0.0);
        unsafe {
            assert_eq!((*sd.updated_cells.ptr).radiation.get(9), 0.0, "无 RS 病菌 → 无贡献");
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    /// 1 固体元素表（state=3，low=-273/high=1000 → update_data 全管线不触发相变）。
    fn create_solid_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(1);
        let mut elem = Element::default();
        elem.id = 0;
        elem.state = 3;
        elem.low_temp = -273.0;
        elem.high_temp = 1000.0;
        elem.number_of_gradient_colors = 1;
        let elem_bytes = unsafe {
            std::slice::from_raw_parts(&elem as *const Element as *const u8, 164).to_vec()
        };
        w.write_bytes(&elem_bytes);
        w.write_int(0);
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// RS 病菌表（idx0=0xd49f77d6）：生长恒 0（popHL/overPopHL=INF、underPopDeath=0、
    /// min=0、max=INF、tempHL 全 INF）、minDiffusionCount 巨大 → 扩散门控 0。
    fn install_rs_integration_table() {
        use crate::b_elements::disease::{Disease, DiseaseInfo, ElemGrowthInfo, RangeInfo};
        let mut t = Disease::new();
        let mut eg = ElemGrowthInfo::default();
        eg.diffusion_scale.resize(1, 1.0);
        eg.min_diffusion_count.resize(1, 1_000_000);
        eg.min_diffusion_infestation_tick_count.resize(1, 0);
        eg.min_count_per_kg.resize(1, 0.0);
        eg.max_count_per_kg.resize(1, f32::INFINITY);
        eg.population_half_life.resize(1, f32::INFINITY);
        eg.over_population_half_life.resize(1, f32::INFINITY);
        eg.under_population_death_rate.resize(1, 0.0);
        t.diseases.push_unchecked(DiseaseInfo {
            hash_id: 0xd49f77d6,
            strength: 1.0,
            temperature_range: RangeInfo {
                min_viable: -f32::INFINITY,
                min_growth: -f32::INFINITY,
                max_growth: f32::INFINITY,
                max_viable: f32::INFINITY,
            },
            temperature_half_lives: RangeInfo {
                min_viable: f32::INFINITY,
                min_growth: f32::INFINITY,
                max_growth: f32::INFINITY,
                max_viable: f32::INFINITY,
            },
            elem_growth_info: eg,
            radiation_kill_rate: 0.0,
            ..Default::default()
        });
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
        }
        g.0 = Box::into_raw(Box::new(t));
    }

    #[test]
    fn update_data_applies_radiation_sickness_contribution() {
        let _lock = crate::LIB_TESTS_LOCK.lock();
        create_solid_table();
        install_rs_integration_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false); // radiation_enabled=true
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let c = &mut *buf_ptr;
                for i in 0..30usize {
                    c.element_idx.set(i, 0);
                    c.temperature.set(i, 300.0);
                    c.mass.set(i, 1000.0);
                    c.disease_idx.set(i, 0xff);
                    c.disease_count.set(i, 0);
                    c.radiation.set(i, 0.0);
                }
            }
        }
        set_cell_disease(&mut sd, 7, 0, 5000); // cell7=(1,1) RS count 5000
        sd.active_regions.push(crate::a_framework::sim_data::ActiveRegion {
            min_x: 1,
            min_y: 1,
            max_x: 2,
            max_y: 2,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        });
        crate::c2_physics::update_data(&mut sd);
        unsafe {
            assert!(
                ((*sd.updated_cells.ptr).radiation.get(7) - 5.0).abs() < 1e-3,
                "update_data 全管线：RS 病菌 radiation += count×0.001 = 5.0"
            );
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    /// 任务 3a：DLC 门控——radiation_enabled=false → 零执行（辐射不变、occlusion 不被填）。
    #[test]
    fn radiation_task_gated_by_enabled_flag() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_radiation_table();
        let mut sd = make_sim();
        sd.radiation_enabled = false;
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 2);
            b.mass.set(14, 10.0);
            b.radiation.set(14, 5.0);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, b, 100.0);
        unsafe {
            assert_eq!((*sd.updated_cells.ptr).radiation.get(14), 5.0, "单星门控：辐射不变");
        }
        DestroyElementsTable();
    }

    /// 任务 3b：元素辐射扩散——cell14 放射性元素（mass=10, rads=1000）
    /// → 中心格 +10×0.001×1000×1.0 = 10；距离 1 权重 0.75 → +7.5。
    #[test]
    fn radiation_task_spreads_element_kernel() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_radiation_table();
        let mut sd = make_sim();
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 2);
            b.mass.set(14, 10.0);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, b, 0.0);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!((u.radiation.get(14) - 10.0).abs() < 1e-4, "中心格 +10");
            assert!((u.radiation.get(13) - 7.5).abs() < 1e-4, "左邻 0.75 权重 +7.5");
            assert!((u.radiation.get(12) - 2.5).abs() < 1e-4, "距离2 0.25 权重 +2.5");
        }
        DestroyElementsTable();
    }

    /// 任务 3c：宇宙 occlusion + 应用——上方真空 → occlusion=1 → cosmic 累加；
    /// 固体（factor=0.5, mass 大）→ occlusion 衰减。
    #[test]
    fn radiation_task_cosmic_occlusion_and_apply() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_radiation_table();
        let mut sd = make_sim();
        // cell 13（行1 列5）= 固体 1；cell 21（行2 列5）= 真空 0；cell 29（行3 列5）= 真空 0（顶）
        fill_both(&mut sd, |b| {
            b.element_idx.set(13, 1);
            b.mass.set(13, 2000.0);
            b.element_idx.set(21, 0);
            b.element_idx.set(29, 0);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, b, 110.0);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            // 顶行 29：occlusion=1 → cosmic/linger × 1 = 110/1.1 = 100
            assert!((u.radiation.get(29) - 100.0).abs() < 1e-3, "顶行 cosmic 应用 100");
            // 行2 真空：occlusion=1 → +100
            assert!((u.radiation.get(21) - 100.0).abs() < 1e-3, "真空行 cosmic 100");
            // 行1 固体：block=(2000/2000)×0.5×0.7 + 0.5×0.3 = 0.5；occlusion=0.5 → +50
            assert!((u.radiation.get(13) - 50.0).abs() < 1e-3, "固体遮挡后 cosmic 50");
        }
        DestroyElementsTable();
    }

    /// 任务 3e：occlusion < 0.01 → 0（原版 L42760-42775）——近全遮挡格
    /// 的微小漏射被清零，不再累加 cosmic。
    #[test]
    fn radiation_task_occlusion_below_threshold_zeroed() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_radiation_table();
        let mut sd = make_sim();
        // cell 13（行1 列5）= 元素3（factor 1.0）mass=1986 →
        // block = (1986/2000)×1.0×0.7 + 1.0×0.3 = 0.9951 →
        // occlusion = (1-0.9951)×1.0 = 0.0049 < 0.01 → 应清零。
        // cell 21（行2 列5）= 真空，above occlusion = 1.0。
        fill_both(&mut sd, |b| {
            b.element_idx.set(13, 3);
            b.mass.set(13, 1986.0);
            b.element_idx.set(21, 0);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, b, 110.0);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.radiation.get(13), 0.0, "occlusion=0 → 不累加 cosmic");
        }
        DestroyElementsTable();
    }

    /// 任务 3f：区域遮挡隔离（d3 并行根因回归）——垂直堆叠的两个 region 各自独立，
    /// 下方 region 顶行即使上方存在别的 region 的方块，也视为 1.0（原版
    /// `iVar17 < fStack_1fc` 区域边界检查，region 顶行及以上独立于区域外格子）。
    /// 此前实现按全网格边界读取共享 cosmic_radiation_occlusion：先跑上方 region
    /// （固体遮挡）后，下方 region 顶行会错误继承上方遮挡（cosmic 50 而非 100）；
    /// 并行下该共享缓冲还会被多 region 线程竞争写，导致宇宙辐射间歇性穿透方块。
    #[test]
    fn radiation_task_vertical_regions_occlusion_isolated() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_radiation_table();
        let mut sd = make_sim();
        // 上方 region B：y 4..7。y=6/y=5 真空，y=4 固体（elem1, mass2000, factor0.5）
        //   → occlusion：y6=1.0, y5=1.0, y4=0.5
        // 下方 region A：y 1..4，全真空。A 顶行 y=3 上方是 B 的 y=4（固体 occlusion 0.5）。
        fill_both(&mut sd, |b| {
            for x in 1..7 {
                b.element_idx.set(4 * 8 + x, 1);
                b.mass.set(4 * 8 + x, 2000.0);
            }
        });
        let b_top = RegionBounds { min_x: 1, min_y: 4, max_x: 7, max_y: 7 };
        let b_bot = RegionBounds { min_x: 1, min_y: 1, max_x: 7, max_y: 4 };
        // 先跑上方 region（旧实现会留下 occlusion 残留），再跑下方 region。
        run_radiation_task(&mut sd, b_top, 110.0);
        run_radiation_task(&mut sd, b_bot, 110.0);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            // B 顶行 y=6 真空：occlusion=1 → 100
            assert!(
                (u.radiation.get(6 * 8 + 1) - 100.0).abs() < 1e-3,
                "上方 region 顶行 cosmic 应 100"
            );
            // B 底部 y=4 固体：occlusion=0.5 → 50
            assert!(
                (u.radiation.get(4 * 8 + 1) - 50.0).abs() < 1e-3,
                "上方 region 固体行 cosmic 应 50"
            );
            // A 顶行 y=3 真空：独立于上方遮挡 → 100（旧实现错误继承 0.5 → 50）
            assert!(
                (u.radiation.get(3 * 8 + 1) - 100.0).abs() < 1e-3,
                "下方 region 顶行 cosmic 应 100"
            );
            // A 内部 y=1 真空 → 100
            assert!(
                (u.radiation.get(1 * 8 + 1) - 100.0).abs() < 1e-3,
                "下方 region 内部 cosmic 应 100"
            );
        }
        DestroyElementsTable();
    }

    /// 任务 3d：衰减 + 钳制——radiation=5，linger=1.1 → 5−5/1.1≈0.4545；
    /// 极小值（0.005）→ 清零；超大值（1e7）→ 钳到 9e6。
    #[test]
    fn radiation_task_decay_and_clamp() {
        let _lock = LIB_TESTS_LOCK.lock();
        init_radiation_table();
        let mut sd = make_sim();
        fill_both(&mut sd, |b| {
            b.element_idx.set(14, 0);
            b.radiation.set(14, 5.0);
            b.radiation.set(21, 0.005);
            b.radiation.set(28, 1e8);
        });
        let b = crate::d1_activity::full_grid_bounds(&sd);
        run_radiation_task(&mut sd, b, 0.0);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            let expect = 5.0 - 5.0 / 1.1;
            assert!((u.radiation.get(14) - expect).abs() < 1e-3, "衰减 5−5/1.1");
            assert_eq!(u.radiation.get(21), 0.0, "≤0.01 清零");
            assert!((u.radiation.get(28) - 9e6).abs() < 1.0, "钳到 9e6（衰减后仍超上限）");
        }
        DestroyElementsTable();
    }
}
