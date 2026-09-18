//! 液体流动纹理更新。对照源码 05_liquid_flow.c。
//!
//! 三个函数在 CopySimDataToGame 尾部被调用（原版 11_msvcrt_ignored.c L32496-32500），
//! 把 SimData 的 flow/元素数据换算成 GameData 的 property texture（C# PropertyTextures 读取）。

use crate::a_framework::game_data::GameData;
use crate::a_framework::sim_data::{CellSOA, SimData};
use crate::a_framework::vector_math::Vector2f;
use crate::b_elements::elements_table;

/// 阳光遮断用"不透明元素"的 hash（原版 05_liquid_flow.c L119 `GetElementIndex(0x2531469c)`）。
/// 该元素是游戏内充当"完全遮光"的固体（反编译中 properties & 0x30 == 0x30 的格子
/// 用它的吸光系数计算遮断）。Rust 侧在 update_exposed_to_sun_property_texture 内解析。
const OPAQUE_ELEMENT_HASH: u32 = 0x2531469c;

/// UpdateExposedToSunPropertyTexture — 阳光暴露纹理（原版 05_liquid_flow.c L7-229）。
/// 写 GameData.propertyTextureExposedToSunlight（u8/格，0=无光，255=满曝）。
///
/// 对照原版结构（世界区域遍历 + 逐列阳光衰减）：
/// 1. 对每个 world（`sim.worlds`，WorldOffsetData）：先把世界顶部一行（"天空行"，
///    原版 L54-63）的纹理清零为 0xff（满曝）。
/// 2. 维护每列阳光强度缓冲 `sun[col]`（初始 1.0，原版 L81-93）。
/// 3. 从世界顶行向下一行行处理（原版 L96-205）：
///    - 格 properties & 0x30 == 0x30（不透明/密封）：用不透明元素的吸光数据衰减该列
///      （L118-147）；
///    - 否则：用**上方格**（cell+width，阳光自上而下先经过）元素的吸光数据衰减（L149-190）；
///      若本格 properties & 0x20 置位（背墙/密封）该列阳光归零（L191-194）。
///    - 把衰减后强度 × 255 写入本格对应 game 索引（L197）。
///
/// **吸光公式**（原版指针算术可解码，任务 8 修复轮）：
/// L102 `lVar17 = (offset_y+row+2)*width + offset_x+1` = 当前格 + width = **上方格**
///（阳光自上而下，行循环 top→bottom，上方格先被处理）；
/// 第一次乘法（L133/L176）用 `gElementLightAbsorptionData` 的 `+4` 字段 = **massScale**，
/// 第二次乘法（L142/L185）用 `+0` 字段 = **factor**：
/// `sun = sun - min(massScale[上方元素] × mass[上方格], 1.0) × factor[上方元素]`
/// ⚠️ 2026-08-04 审查勘误：此前注释误标"下方格"（cell+width 实为上方格，
/// 与本项目液体约定 below=cell-width 一致）。
pub fn update_exposed_to_sun_property_texture(sim: &SimData, game: &mut GameData) {
    let sim_w = sim.width as usize;
    let sim_h = sim.height as usize;
    let game_w = game.width as usize;
    let game_h = game.height as usize;
    if sim_w == 0
        || sim_h == 0
        || game_w == 0
        || game_h == 0
        || sim.updated_cells.ptr.is_null()
        || game.property_texture_exposed_to_sunlight.ptr.is_null()
    {
        return;
    }
    // 元素表为空（测试环境）时无吸光系数可算，直接返回
    if elements_table::get_element_count_pub() == 0 {
        return;
    }
    let opaque_elem = elements_table::get_element_index_pub(OPAQUE_ELEMENT_HASH);
    let game_total = game_w * game_h;
    unsafe {
        let updated = &*sim.updated_cells.ptr;
        let exposed = std::slice::from_raw_parts_mut(
            game.property_texture_exposed_to_sunlight.ptr,
            game_total,
        );
        for world in sim.worlds.as_slice() {
            let w_x = world.offset_x.max(0) as usize;
            let w_y = world.offset_y.max(0) as usize;
            let w_w = world.width.max(0) as usize;
            let w_h = world.height.max(0) as usize;
            if w_w == 0 || w_h == 0 {
                continue;
            }
            // 原版 L54-63：先把世界**顶部行**（= 天空行，game 行 w_y+w_h-1）填 0xff（满曝）；
            // 下方各行走计算路径。
            // 原版 uVar9 = w_x + 1 + (w_y + w_h) * sim_w，对应 game 行 = w_y + w_h - 1。
            let sky_row = w_y + w_h;
            if sky_row > 0 {
                let g_start = (sky_row - 1) * game_w + w_x;
                let g_end = (g_start + w_w).min(game_total);
                if g_start < game_total {
                    for g in g_start..g_end {
                        exposed[g] = 0xff;
                    }
                }
            }
            // 原版 L64-93：每列阳光强度缓冲，初始 1.0
            let mut sun = vec![1.0f32; w_w];
            // 原版 L96：行循环从 w_h-2 递减到 0（世界顶行 → 下），sim 行 r = w_y + row + 1
            for row in (0..w_h.saturating_sub(1)).rev() {
                let r = w_y + row + 1;
                if r >= sim_h {
                    continue;
                }
                let cell_base = r * sim_w + w_x + 1;
                for col in 0..w_w {
                    let cell = cell_base + col;
                    if cell >= updated.element_idx.len() || cell >= updated.mass.len() {
                        continue;
                    }
                    let props = updated.properties.get(cell);
                    // 原版 L102：lVar17 = (offset_y+row+2)*width + offset_x+1 = 当前格 + width = **上方格**。
                    // 吸光计算读取的是上方格的元素与质量（L133/L163/L176），不是本格。
                    let above_cell = cell + sim_w;
                    let mut f22: f32;
                    if props & 0x30 == 0x30 {
                        // 不透明/密封格（原版 L118-147）：用不透明元素的吸光数据（massScale + factor）
                        let (mscale, factor) =
                            match elements_table::get_element_light_absorption_data(opaque_elem) {
                                Some(d) => (d.mass_scale, d.factor),
                                None => (0.0, 0.0),
                            };
                        // 原版 L133-137：fVar22 = massScale × mass[上方格]，>= 1.0 → 1.0
                        let mut absorb = mscale * updated.mass.get(above_cell);
                        // 2026-08-04 防御：INFINITY×0 = NaN 时按"无遮挡"处理（真空不挡光）
                        if absorb.is_nan() {
                            absorb = 0.0;
                        }
                        if absorb >= 1.0 {
                            absorb = 1.0;
                        }
                        // 原版 L142-147：f22 = sun - absorb × factor，<= 0 → 0
                        f22 = sun[col] - absorb * factor;
                        if f22 <= 0.0 {
                            f22 = 0.0;
                        }
                        sun[col] = f22;
                    } else {
                        // 普通格（原版 L149-194）：读**上方格**元素（L163），用其 massScale/factor
                        let below_elem = updated.element_idx.get(above_cell);
                        let (mscale, factor) =
                            match elements_table::get_element_light_absorption_data(below_elem) {
                                Some(d) => (d.mass_scale, d.factor),
                                None => (0.0, 0.0),
                            };
                        // 原版 L176-180：fVar22 = massScale[上方元素] × mass[上方格]，>= 1.0 → 1.0
                        let mut absorb = mscale * updated.mass.get(above_cell);
                        // 2026-08-04 防御：INFINITY×0 = NaN 时按"无遮挡"处理（真空不挡光）
                        if absorb.is_nan() {
                            absorb = 0.0;
                        }
                        if absorb >= 1.0 {
                            absorb = 1.0;
                        }
                        // 原版 L185-189：f22 = sun - absorb × factor[下方元素]，<= 0 → 0
                        f22 = sun[col] - absorb * factor;
                        if f22 <= 0.0 {
                            f22 = 0.0;
                        }
                        // 原版 L190-194：properties & 0x20 置位 → 该列阳光归零（截断光柱）
                        let mut f23 = f22;
                        if props & 0x20 != 0 {
                            f23 = 0.0;
                        }
                        sun[col] = f23;
                    }
                    // 原版 L197：写入本格 game 索引 = (r-1)*game_w + (w_x + col)
                    let g = (r - 1) * game_w + w_x + col;
                    if g < game_total {
                        exposed[g] = (f22 * 255.0) as u8;
                    }
                }
            }
        }
    }
}

/// UpdateFlowTexture — flow → GameData.propertyTextureFlow（Vector2f：水平/垂直流动差）。
/// 对照源码 05_liquid_flow.c L230-308。
///
/// 遍历内部格（row in 1..=height-1，col in 1..=width-1，sim 坐标含边界）：
/// - 若 updatedCells.element_idx == cells.element_idx（格子未变）：
///   inv = 1/max(mass, 1.0)；否则 inv = 0
/// - 输出 x = (flow.x - flow.y) * inv（水平差），y = (flow.w - flow.z) * inv（垂直差）
pub fn update_flow_texture(sim: &SimData, game: &mut GameData) {
    let sim_w = sim.width as usize;
    let game_w = game.width as usize;
    let game_h = game.height as usize;
    if game_h == 0 || sim.cells.ptr.is_null() || sim.flow.ptr.is_null() {
        return;
    }
    unsafe {
        let cells = &*sim.cells.ptr;
        let updated = &*sim.updated_cells.ptr;
        let flow = std::slice::from_raw_parts(sim.flow.ptr, sim_w * sim.height as usize);
        let tex = std::slice::from_raw_parts_mut(game.property_texture_flow.ptr, game_w * game_h);
        for row in 0..game_h {
            let sim_row_start = (row + 1) * sim_w + 1;
            for col in 0..game_w {
                let s = sim_row_start + col;
                let inv = if updated.element_idx.get(s) == cells.element_idx.get(s) {
                    let m = updated.mass.get(s);
                    if m <= 1.0 { 1.0 } else { 1.0 / m }
                } else {
                    0.0
                };
                let f = flow[s];
                tex[row * game_w + col] = Vector2f {
                    x: (f.x - f.y) * inv,
                    y: (f.w - f.z) * inv,
                };
            }
        }
    }
}

/// 液面 alpha / 梯度深度因子 = `mass / 1000`（满格质量比，clamp 0..1）。
///
/// 原版 05_liquid_flow.c L397（UpdateLiquidPropertyTexture）与
/// 11_msvcrt_ignored.c L43123（GetEstimatedLiquidGradientValue）各有一处
/// `powf()` 调用。
/// **2026-08-10 反汇编原版 SimDLL.dll 确认**（XMM 常量池）：
/// `powf(min(mass × 0.001, 1.0), 0.45)` —— 指数 = 0.45
/// （UpdateLiquidPropertyTexture @0x1800486CA：xmm13=0.001、xmm6=1.0、xmm14=0.45；
///  GetEstimatedLiquidGradientValue @0x180047CC6：div 1000.0、clamp 1.0、指数 0.45）。
/// 此前取线性 mass/1000（等价指数 1.0）→ 低质量液面偏矮；现按原版 0.45 次方。
fn liquid_mass_ratio(mass: f32) -> f32 {
    (mass / 1000.0).clamp(0.0, 1.0).powf(0.45)
}

/// 梯度颜色插值 — 由温度梯度 t（0..1）在 gradient 数组的相邻色之间做 RGB 线性插值。
/// 对照源码 05_liquid_flow.c L405-545（number_of_gradient_colors + 展开循环）。
///
/// 原版语义：对每个梯度色 i（i < n），若 `t >= gradient[i].alpha` 则在该区间
/// （[gradient[i], gradient[i+1]]，i+1 < n 时）插值；命中多个区间时最后一个覆盖
/// （alpha 单调递增时等价于"t 所在区间"）。插值只作用于低 24 位 RGB，alpha
/// 最终由调用方（液体透明度/温度归一化）覆盖。
fn interpolate_gradient_colour(
    colour: u32,
    gradient: &[u32; 6],
    number_of_gradient_colors: u8,
    t: f32,
) -> u32 {
    let mut result = colour;
    let n = number_of_gradient_colors as usize;
    for i in 0..n {
        let lower = gradient[i];
        let lower_alpha = (lower >> 24) as f32 * 0.003921569; // /255
        if lower_alpha <= t {
            result = lower;
            if i + 1 < n {
                result = gradient[i + 1];
            }
            let upper_alpha = (result >> 24) as f32 * 0.003921569;
            let denom = upper_alpha - lower_alpha;
            let mut f = if denom > 0.0 {
                (t - lower_alpha) / denom
            } else {
                // 原版分母 <= 0 会产生 NaN/±inf（数据异常），此处保守取 0（用下界色）
                0.0
            };
            f = f.clamp(0.0, 1.0);
            // RGB 插值（原版 L430-437：逐通道 `lower + (upper-lower)*f`，向零截断）
            let lb = (lower & 0xff) as i32;
            let lg = ((lower >> 8) & 0xff) as i32;
            let lr = ((lower >> 16) & 0xff) as i32;
            let ub = (result & 0xff) as i32;
            let ug = ((result >> 8) & 0xff) as i32;
            let ur = ((result >> 16) & 0xff) as i32;
            let b = (lb + ((ub - lb) as f32 * f) as i32) as u32 & 0xff;
            let g = (lg + ((ug - lg) as f32 * f) as i32) as u32 & 0xff;
            let r = (lr + ((ur - lr) as f32 * f) as i32) as u32 & 0xff;
            result = b | (g << 8) | (r << 16);
        }
    }
    result
}

/// GetEstimatedLiquidGradientValue — 液体垂直梯度估计（0..1）。
/// 对照源码 11_msvcrt_ignored.c L43098-43160。
///
/// 语义（对 sim 内部格 idx）：
/// - 当前格是液体：梯度 = min(自身深度衰减值, 下方格递归值)。
///   自身值 = 下方是液体/固体时 `1 - exposed[game_idx]*(1/255)`（乘 powf 衰减），
///   下方是气体/真空时为 0。
/// - 当前格是气体/真空：梯度 0；当前格是固体：梯度 1。
///
/// 安全性：下方格越界（异常/测试场景）视为无支撑，返回 0（防止无限递归）；
/// 元素表缺失时返回 0。
fn estimated_liquid_gradient_value(
    sim_w: usize,
    total_sim_cells: usize,
    updated: &CellSOA,
    exposed: &[u8],
    game_w: usize,
    idx: usize,
    recurse: bool,
) -> f32 {
    let below = idx + sim_w;
    // 下方格越界：原版不可达（sim 内格下方总在界内）；测试/异常场景保守返回 0
    if below >= total_sim_cells {
        return 0.0;
    }
    let elem = updated.element_idx.get(idx);
    let below_elem = updated.element_idx.get(below);
    let (Some(ptd), Some(ptd_below)) = (
        elements_table::get_element_property_texture_data(elem),
        elements_table::get_element_property_texture_data(below_elem),
    ) else {
        return 0.0; // 元素表缺失/元素无效：安全返回
    };
    let state = ptd.state & 3;
    if state == 2 {
        // 液体（原版 L43121-43148）
        let mut f_var8 = 1.0;
        let mass_ratio = liquid_mass_ratio(updated.mass.get(idx)); // 原版 L43123 powf（满格质量比）
        let mut f_var7 = if (ptd_below.state & 3) < 2 {
            // 下方是气体/真空：无支撑
            0.0
        } else {
            // 下方是液体/固体：由阳光暴露度衰减（原版 L43128-43140）
            // game 索引 = (row-1)*game_w + (col-1)（原版 L43129-43131 指针算术的等价形式）
            let row = idx / sim_w;
            let col = idx % sim_w;
            let g = if row > 0 && col > 0 {
                (row - 1) * game_w + (col - 1)
            } else {
                0
            };
            let mut v = 1.0 - exposed.get(g).copied().unwrap_or(0) as f32 * 0.003921569;
            v = v.clamp(0.0, 1.0);
            v * mass_ratio
        };
        if recurse && f_var7 > 0.0 {
            f_var8 = estimated_liquid_gradient_value(
                sim_w, total_sim_cells, updated, exposed, game_w, below, false,
            );
        }
        if f_var8 <= f_var7 {
            f_var7 = f_var8;
        }
        f_var7
    } else if state < 2 {
        0.0 // 气体/真空
    } else {
        1.0 // 固体
    }
}

/// UpdateLiquidPropertyTexture — 液体颜色纹理更新。
/// 对照源码 05_liquid_flow.c L309-598。
///
/// 遍历 game 内部格（sim 坐标含边界）：读 updatedCells.element_idx → 元素表
/// property_texture_data；对每个内部格**无条件写**三个纹理：
/// - propertyTextureLiquid（u32）：alpha=液体透明度，RGB=梯度色插值
/// - propertyTextureLiquidData（u32）：alpha=温度归一化，RGB=相变目标元素颜色
/// - propertyTextureMaterialData（u32）：material_properties & 0xff00000f
/// 非液体格写 0（原版 L386-388 先清零、L580-583 无条件写）——避免跨帧残留鬼影。
/// 液体判定（state & 3 == 2，原版 L390）只决定是否做颜色插值计算。
///
/// **这是液体黑屏的直接修复点**：C# PropertyTextures 每帧读 externalLiquidTex
/// 渲染液体颜色；此前 propertyTextureLiquid 恒 0 → 液体全黑。
pub fn update_liquid_property_texture(sim: &SimData, game: &mut GameData) {
    let sim_w = sim.width as usize;
    let sim_h = sim.height as usize;
    let game_w = game.width as usize;
    let game_h = game.height as usize;
    if game_w == 0
        || game_h == 0
        || sim.updated_cells.ptr.is_null()
        || sim.cells.ptr.is_null()
        || game.property_texture_liquid.ptr.is_null()
        || game.property_texture_liquid_data.ptr.is_null()
        || game.property_texture_material_data.ptr.is_null()
        || game.property_texture_exposed_to_sunlight.ptr.is_null()
    {
        return;
    }
    // 元素表为空（测试环境）时无液体元素可算，直接返回
    if elements_table::get_element_count_pub() == 0 {
        return;
    }
    unsafe {
        let updated = &*sim.updated_cells.ptr;
        let cells = &*sim.cells.ptr;
        let total_cells = game_w * game_h;
        let liquid = std::slice::from_raw_parts_mut(game.property_texture_liquid.ptr, total_cells);
        let liquid_data = std::slice::from_raw_parts_mut(
            game.property_texture_liquid_data.ptr,
            total_cells,
        );
        let material =
            std::slice::from_raw_parts_mut(game.property_texture_material_data.ptr, total_cells);
        let exposed =
            std::slice::from_raw_parts(game.property_texture_exposed_to_sunlight.ptr, total_cells);
        let total_sim_cells = sim_w * sim_h;

        // 原版行循环（L363-368）：iVar13 即 sim 行号，从 1 到 height-1（=game_h）；
        // sim 内部行 = 1..=game_h（game 行 = sim 行 - 1），列循环 col in 0..game_w（L371-372）。
        // 注意与 update_flow_texture 不同：后者行循环从 game 行 0 起，sim 行 = game 行 + 1。
        for row in 1..=game_h {
            let sim_row_start = row * sim_w + 1; // 原版 iVar22 = width*iVar13 + 1
            let g_row = row - 1;
            for col in 0..game_w {
                let s = sim_row_start + col; // sim 内部格索引（原版 iVar22）
                let below = s + sim_w; // 下方格（原版 uVar25）
                let g = g_row * game_w + col; // game 索引
                let elem = updated.element_idx.get(s);

                // 原版 L386-388：每轮先把 uVar18/uVar17/uVar28 置 0。
                // 液体格在下方计算中覆盖；非液体格/元素表缺失保持 0。
                let mut liquid_val = 0u32;
                let mut liquid_data_val = 0u32;
                let mut material_val = 0u32;

                // 原版 L390：液体判定（state & 3 == 2）只决定是否做颜色插值计算；
                // 是否写入与判定无关——L580-583 对每个内部格无条件写三纹理。
                if let Some(ptd) = elements_table::get_element_property_texture_data(elem) {
                    // 元素表缺失（原版 goto invalid 崩溃，不可达）安全跳过计算，仍写 0
                    if (ptd.state & 3) == 2 {
                        // fVar35 —— 液体 alpha（原版 L391-402）
                        // 下方是气体/真空 且 cells.properties[below] bit1 未置位 → 衰减因子；
                        // 否则 alpha=1.0（不透明）
                        let below_elem = updated.element_idx.get(below);
                        let below_state =
                            elements_table::get_element_property_texture_data(below_elem)
                                .map(|d| d.state & 3)
                                .unwrap_or(0);
                        let f_var35 = if below_state < 2 && (cells.properties.get(below) & 2) == 0 {
                            // 液面格（上方为气体/真空）：alpha = 质量/1000（原版 L397 powf）
                            liquid_mass_ratio(updated.mass.get(s))
                        } else {
                            1.0
                        };

                        // fVar33 —— 垂直梯度（原版 L403-404）
                        let t = estimated_liquid_gradient_value(
                            sim_w,
                            total_sim_cells,
                            updated,
                            exposed,
                            game_w,
                            s,
                            true,
                        );

                        // 梯度颜色插值 → colour（原版 L405-545）
                        let colour = interpolate_gradient_colour(
                            ptd.colour, &ptd.gradient, ptd.number_of_gradient_colors, t,
                        );

                        // 温度归一化（原版 L546-548）：
                        //   f = (temp - (low_temp - 3)) / ((high_temp + 3) - (low_temp - 3))
                        let td = elements_table::get_element_temperature_data(elem);
                        let (low_temp, high_temp) = match td {
                            Some(d) => (d.low_temp, d.high_temp),
                            None => (0.0, 0.0),
                        };
                        let mut f = (updated.temperature.get(s) - (low_temp - 3.0))
                            / ((high_temp + 3.0) - (low_temp - 3.0));

                        // propertyTextureLiquid（原版 L549）：alpha=fVar35*255，RGB=插值色
                        liquid_val = ((f_var35 * 255.0) as i32 as u32) << 24 | (colour & 0xffffff);

                        // 相变目标元素（原版 L550-564）：
                        //   f>1 → 高温相变元素，f=1；f<0 → 低温相变元素，f=0；
                        //   否则按 f<0.5 选低/高温相变元素
                        let (low_trans_idx, high_trans_idx) = match td {
                            Some(d) => (d.low_temp_transition_idx, d.high_temp_transition_idx),
                            None => (0xffff, 0xffff),
                        };
                        let transition_idx = if f > 1.0 {
                            f = 1.0;
                            high_trans_idx
                        } else if f < 0.0 {
                            f = 0.0;
                            low_trans_idx
                        } else if f < 0.5 {
                            low_trans_idx
                        } else {
                            high_trans_idx
                        };

                        // 相变目标颜色（原版 L565-574）：无目标（0xffff）→ 白
                        let transition_colour = if transition_idx == 0xffff {
                            0xffffff
                        } else {
                            elements_table::get_element_property_texture_data(transition_idx)
                                .map(|d| d.colour)
                                .unwrap_or(0xffffff)
                        };

                        // propertyTextureLiquidData（原版 L575）：alpha=f*255，RGB=相变目标色
                        liquid_data_val =
                            ((f * 255.0) as i32 as u32) << 24 | (transition_colour & 0xffffff);
                        // propertyTextureMaterialData（原版 L576）：material_properties 保留 bit24-31 与 bit0-3
                        material_val = ptd.material_properties & 0xff00000f;
                    }
                }

                // 原版 L580-583：对每个内部格无条件写三纹理；非液体格写 0。
                // 若不写，GameData 缓冲跨帧持久会残留旧颜色/旧 material（鬼影），
                // 与液体排空/被替换后的原版表现（格子变 0）相悖。
                liquid[g] = liquid_val;
                liquid_data[g] = liquid_data_val;
                material[g] = material_val;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
    use crate::a_framework::game_data::GameData;
    use crate::a_framework::sim_data::{SimData, WorldOffsetData};
    use crate::a_framework::vector_math::Vector4f;
    use crate::b_elements::element::Element;
    use crate::b_elements::elements_table::{CreateElementsTable, DestroyElementsTable};
    use crate::LIB_TESTS_LOCK;

    /// 自建一张含 2 个元素的最小表：
    /// - 元素 0：气体占位（state=1，colour 白，兼作温度相变目标）
    /// - 元素 1：液体（state=2，colour 绿 0xff00ff00，material_properties 0x12345678）
    /// 调用方需持 LIB_TESTS_LOCK，并在测试末尾 DestroyElementsTable() 清理。
    fn create_test_elements_table() {
        let mut gas = Element::default();
        gas.id = 0;
        gas.state = 1; // Gas
        gas.colour = 0xffffffff;
        let mut liquid = Element::default();
        liquid.id = 1;
        liquid.state = 2; // Liquid
        liquid.colour = 0xff00ff00;
        liquid.material_properties = 0x12345678;
        liquid.number_of_gradient_colors = 1;
        liquid.gradient_colours[0] = 0xff00ff00;
        liquid.gradient_colours[1] = 0xff00ff00;
        liquid.low_temp = 0.0;
        liquid.high_temp = 100.0;

        let mut w = BinaryBufferWriter::new();
        w.write_int(2);
        // 数据格式（对照 C# SimMessages.CreateSimElementsTable / 03_elements.c L27-429）：
        // 所有元素连续在前（count × 164B），所有名字连续在后（count × KleiString）。
        // 注意不能"元素+名字交错"（单元素时恰好等价，多元素会错位 4 字节）。
        let mut elems = Vec::new();
        for elem in [&gas, &liquid] {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(&*elem as *const Element as *const u8, 164).to_vec()
            };
            elems.push(elem_bytes);
        }
        for b in &elems {
            w.write_bytes(b);
        }
        for _ in 0..2 {
            w.write_int(0); // 空名称（长度=0）
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 构造 4×3 内部世界（sim 尺寸 6×5），在 flow 中写入已知方向差，
    /// 断言 propertyTextureFlow 输出 = 差 × (1/mass)。
    #[test]
    fn update_flow_texture_writes_vector2f() {
        let _lock = LIB_TESTS_LOCK.lock();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let mut gd = GameData::new(4, 3);
        unsafe {
            let cells = &mut *sd.cells.ptr;
            cells.element_idx.set(0, 0);
            cells.mass.set(0, 1000.0);
            // 让 internal cell (1,1) -> sim idx (1*6+1)=7 有 mass
            cells.element_idx.set(7, 0);
            cells.mass.set(7, 1000.0);
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(7, 0);
            updated.mass.set(7, 1000.0);
            // flow[7] = Vector4f { x: 5.0, y: 2.0, z: 3.0, w: 7.0 }
            let flow = std::slice::from_raw_parts_mut(sd.flow.ptr, 6 * 5);
            flow[7] = Vector4f { x: 5.0, y: 2.0, z: 3.0, w: 7.0 };
        }
        update_flow_texture(&sd, &mut gd);
        unsafe {
            let tex = std::slice::from_raw_parts(gd.property_texture_flow.ptr, 4 * 3);
            // game idx (0,0) = sim (1,1) = 7：x = (5.0-2.0)/1000 = 0.003，y = (7.0-3.0)/1000 = 0.004
            let v = tex[0];
            assert!((v.x - 0.003).abs() < 1e-6, "x = {}", v.x);
            assert!((v.y - 0.004).abs() < 1e-6, "y = {}", v.y);
        }
    }

    /// 液体格（state & 3 == 2）应写出非零颜色（propertyTextureLiquid）——
    /// 液体黑屏的直接修复点。自建元素表，强断言精确值。
    #[test]
    fn update_liquid_property_texture_writes_color() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_test_elements_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let mut gd = GameData::new(4, 3);
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            // sim(1,1) = game(0,0) = idx 7：液体元素 1
            updated.element_idx.set(7, 1);
            updated.mass.set(7, 1000.0);
            updated.temperature.set(7, 50.0);
            // 下方格 sim(2,1) = idx 13：气体元素 0（终止梯度递归）
            updated.element_idx.set(13, 0);
            // 预置非液体格输出，验证被写 0（原版 L386-388 每轮先清零、L580-583 无条件写）：
            // game(0,1) = sim(1,2) = idx 8（气体元素 0）
            updated.element_idx.set(8, 0);
            let liquid = std::slice::from_raw_parts_mut(gd.property_texture_liquid.ptr, 4 * 3);
            let liquid_data = std::slice::from_raw_parts_mut(gd.property_texture_liquid_data.ptr, 4 * 3);
            let material = std::slice::from_raw_parts_mut(gd.property_texture_material_data.ptr, 4 * 3);
            liquid[1] = 0x55555555;
            liquid_data[1] = 0x55555555;
            material[1] = 0x55555555;
        }
        update_liquid_property_texture(&sd, &mut gd);
        unsafe {
            let liquid = std::slice::from_raw_parts(gd.property_texture_liquid.ptr, 4 * 3);
            let liquid_data = std::slice::from_raw_parts(gd.property_texture_liquid_data.ptr, 4 * 3);
            let material = std::slice::from_raw_parts(gd.property_texture_material_data.ptr, 4 * 3);
            // 液体格：alpha=255 | 元素 colour(绿 0x00ff00) → 0xff00ff00
            assert_ne!(liquid[0], 0, "液体格应有颜色（黑屏修复点）");
            assert_eq!(liquid[0], 0xff00ff00, "liquid 颜色应等于元素 colour");
            // liquid_data：alpha=f*255=0.5*255≈127 | 高温相变目标色（元素 0 白）→ 0x7fffffff
            assert_eq!(liquid_data[0], 0x7fffffff, "liquid_data 应为温度 alpha+相变色");
            // material_data：material_properties & 0xff00000f
            assert_eq!(material[0], 0x12000008, "material_data 只保留 bit24-31 与 bit0-3");
            // 非液体格：无条件写 0（原版 L386-388 清零 + L580-583 写入）——
            // 若跳过写入，GameData 缓冲跨帧持久会残留旧颜色/旧 material（鬼影）
            assert_eq!(liquid[1], 0, "非液体格 liquid 应写 0（防鬼影）");
            assert_eq!(liquid_data[1], 0, "非液体格 liquid_data 应写 0");
            assert_eq!(material[1], 0, "非液体格 material 应写 0");
        }
        DestroyElementsTable();
    }

    /// 空元素表（测试环境默认）下不 panic，输出保持 0。
    #[test]
    fn update_liquid_property_texture_no_panic_empty_table() {
        let _lock = LIB_TESTS_LOCK.lock();
        DestroyElementsTable(); // 确保空表
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let mut gd = GameData::new(4, 3);
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(7, 0);
            updated.mass.set(7, 1000.0);
        }
        update_liquid_property_texture(&sd, &mut gd);
        unsafe {
            let liquid = std::slice::from_raw_parts(gd.property_texture_liquid.ptr, 4 * 3);
            assert_eq!(liquid[0], 0, "空元素表下应保持 0");
        }
    }

    /// 液面高度回归：surface 格（上方为气体/真空）的 alpha = powf(mass/1000, 0.45)
    /// （原版 L397 powf，2026-08-10 反汇编确认指数 0.45）。
    /// 此前 liquid_powf()=1.0 占位 → 任何质量都满格高度（用户实测）；
    /// 再此前线性 mass/1000 → 低质量液面偏矮。
    #[test]
    fn liquid_alpha_scales_with_mass_ratio() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_test_elements_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let mut gd = GameData::new(4, 3);
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            // sim(1,1) = game(0,0) = idx 7：液体元素 1，质量 100kg
            updated.element_idx.set(7, 1);
            updated.mass.set(7, 100.0);
            updated.temperature.set(7, 50.0);
            // cell+width = idx 13：气体（上方，液面格判定）
            updated.element_idx.set(13, 0);
        }
        update_liquid_property_texture(&sd, &mut gd);
        unsafe {
            let liquid = std::slice::from_raw_parts(gd.property_texture_liquid.ptr, 4 * 3);
            let alpha = (liquid[0] >> 24) & 0xff;
            assert_eq!(
                alpha, 90,
                "100kg/1000kg → alpha=powf(0.1,0.45)×255≈90"
            );
            assert_eq!(liquid[0] & 0xffffff, 0x00ff00, "RGB 仍为元素 colour（绿）");
        }
        DestroyElementsTable();
    }

    /// 梯度深度因子回归：GetEstimatedLiquidGradientValue 乘 powf(mass/1000, 0.45)
    /// （原版 L43123 powf，指数 0.45 已反汇编确认）。
    #[test]
    fn estimated_gradient_scales_with_mass_ratio() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_test_elements_table();
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            updated.element_idx.set(7, 1); // 液体
            updated.mass.set(7, 100.0);
            updated.element_idx.set(13, 1); // cell+width 液体（支撑）
            updated.mass.set(13, 1000.0);
        }
        let exposed = [0u8; 12]; // 全暗 → (1 - 0/255) = 1.0
        unsafe {
            let updated = &*sd.updated_cells.ptr;
            let v = estimated_liquid_gradient_value(
                6,
                30,
                updated,
                &exposed,
                4,
                7,
                false,
            );
            assert!(
                (v - 0.354813).abs() < 1e-3,
                "梯度 = 光深(1.0) × powf(0.1,0.45)=0.3548，got {}",
                v
            );
        }
        DestroyElementsTable();
    }

    /// 诊断：100kg 气体单格扩散 5 子步后 flow / flow 纹理的量级。
    /// 对照原版公式（flow 累加结构已核对），若量级异常则存在累加 bug。
    #[test]
    fn diagnostic_flow_texture_magnitude_gas_diffusion() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 专用表：0=真空(state0,flow 0)、1=气体(state1,flow 0.1)、2=固体(state3)
        let mut w = BinaryBufferWriter::new();
        w.write_int(3);
        for (id, state, flow) in [(0i32, 0u8, 0.0f32), (1, 1, 0.1), (2, 3, 0.0)] {
            let mut e = Element::default();
            e.id = id;
            e.state = state;
            e.flow = flow;
            e.low_temp = -273.0;
            e.high_temp = 2000.0;
            e.number_of_gradient_colors = 1;
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(&e as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..3 {
            w.write_int(0);
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null());

        let mut sd = SimData::new_for_allocate(16, 8, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        let w16 = 16usize;
        let src = 3 * w16 + 4; // (4,3) 100kg 气体
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let c = &mut *buf_ptr;
                for i in 0..(16 * 8usize) {
                    let x = i % w16;
                    let y = i / w16;
                    if x == 0 || x == 15 || y == 0 || y == 7 {
                        c.element_idx.set(i, 2); // 固体边界
                        c.mass.set(i, 1000.0);
                        c.temperature.set(i, 300.0);
                    } else {
                        c.element_idx.set(i, 0); // 真空
                        c.temperature.set(i, 300.0);
                    }
                }
                c.element_idx.set(src, 1);
                c.mass.set(src, 100.0);
                c.temperature.set(src, 300.0);
            }
        }
        sd.active_regions.push(crate::a_framework::sim_data::ActiveRegion {
            min_x: 1,
            min_y: 1,
            max_x: 15,
            max_y: 7,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        });
        for _ in 0..5 {
            crate::c2_physics::update_data(&mut sd);
        }
        unsafe {
            let flow = std::slice::from_raw_parts(sd.flow.ptr, 16 * 8);
            let updated = &*sd.updated_cells.ptr;
            let mut max_tex = 0.0f32;
            for i in 0..(16 * 8usize) {
                let f = flow[i];
                if updated.element_idx.get(i) == 1 {
                    let inv = 1.0 / updated.mass.get(i).max(1.0);
                    let tx = (f.x - f.y) * inv;
                    let ty = (f.w - f.z) * inv;
                    max_tex = max_tex.max(tx.abs()).max(ty.abs());
                }
            }
            // 源格 mass 应下降、flow 应显著（> 10）
            assert!(updated.mass.get(src) < 100.0, "100kg 应已扩散");
            // 纹理值量级合理（flow 累加公式与原版一致；若出现 >100 说明累加 bug）
            assert!(
                max_tex < 100.0,
                "flow 纹理量级异常大：max={}",
                max_tex
            );
        }
        DestroyElementsTable();
    }

    // ===== update_exposed_to_sun_property_texture（任务 4 延后，任务 8 实现）=====

    /// 自建 2 元素最小表：
    /// - 元素 0：真空（state=0，light_absorption_factor=0.0，default_mass=1.0 → mass_scale=1.0）
    /// - 元素 1：固体（state=3，light_absorption_factor=0.5，default_mass=1.0 → mass_scale=INFINITY）
    fn create_exposed_table() {
        let mut w = BinaryBufferWriter::new();
        w.write_int(2);
        let elems = [
            {
                let mut e = Element::default();
                e.id = 0;
                e.state = 0; // 真空
                e.number_of_gradient_colors = 1;
                e.light_absorption_factor = 0.0;
                e.default_values.mass = 1.0; // 非固体 → mass_scale = 1/mass = 1.0（有限，避免 NaN）
                e
            },
            {
                let mut e = Element::default();
                e.id = 1;
                e.state = 3; // 固体
                e.number_of_gradient_colors = 1;
                e.light_absorption_factor = 0.5;
                e.default_values.mass = 1.0;
                e
            },
        ];
        for elem in &elems {
            let elem_bytes = unsafe {
                std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
            };
            w.write_bytes(&elem_bytes);
        }
        for _ in 0..2 {
            w.write_int(0); // 空名称
        }
        let data = w.into_bytes();
        let mut reader = BinaryBufferReader::new(&data);
        let result = CreateElementsTable(&mut reader);
        assert!(!result.is_null(), "CreateElementsTable should succeed");
    }

    /// 阳光衰减（原版可解码公式）：sun = sun - min(massScale[上方元素] × mass[上方格], 1.0) × factor[上方元素]。
    /// 原版指针算术（05_liquid_flow.c L102/L133/L163/L176）读的是**上方格**（cell+width，
    /// 阳光自上而下先经过）的
    /// 元素与质量，不是本格——第一次乘法用 +4 字段（massScale），第二次用 +0 字段（factor）。
    /// sim 6x5（game 4x3），world = {x:0,y:0,w:4,h:3}：
    /// - 天空行 r=3 → game 行 2（exposed[8..12]）清零为 0xff
    /// - 光列 0：cell13（r=2,col0，真空）的**上方格** cell19（r=3,col0，固体 factor=0.5，mass=1.0）
    ///   → absorb = min(INF×1.0, 1.0) = 1.0 → f22 = 1 - 1.0×0.5 = 0.5 → exposed[4] = 127；
    ///   r=1 col0（sim7 真空）上方格=13（真空 factor=0）→ 继承 sun=0.5 → exposed[0] = 127
    /// - 光列 1：无遮挡 → exposed[5] == exposed[1] == 255
    #[test]
    fn update_exposed_to_sun_writes_attenuated_light_column() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_exposed_table();
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.worlds.push(WorldOffsetData {
            offset_x: 0,
            offset_y: 0,
            width: 4,
            height: 3,
        });
        unsafe {
            let updated = &mut *sd.updated_cells.ptr;
            // 吸光固体放在 cell13 的**上方格** cell19（原版读上方格的元素/质量）
            updated.element_idx.set(19, 1);
            updated.mass.set(19, 1.0);
            // cell13（r=2,col0）为真空，仅作为其上方格 cell7 的吸光元素来源
            updated.element_idx.set(13, 0);
            updated.mass.set(13, 0.0);
        }
        let mut gd = GameData::new(4, 3);
        update_exposed_to_sun_property_texture(&sd, &mut gd);
        unsafe {
            let exposed = std::slice::from_raw_parts(
                gd.property_texture_exposed_to_sunlight.ptr,
                4 * 3,
            );
            // game(1,0) = sim(2,1) = 13：f22 = 1 - min(INF×1,1.0)×0.5 = 0.5 → 127
            assert_eq!(exposed[4], 127, "吸光固体（上方格）所在列应衰减到 0.5 → 127");
            // game(0,0) = sim(1,1) = 7：继承 0.5
            assert_eq!(exposed[0], 127, "下方真空格应继承衰减 0.5 → 127");
            // 光列 1：无遮挡，保持满曝 255
            assert_eq!(exposed[5], 255, "game(1,1) 应满曝");
            assert_eq!(exposed[1], 255, "game(0,1) 应满曝");
            // 天空行清零为 0xff
            for g in 8..12 {
                assert_eq!(exposed[g], 255, "天空行 {} 应为满曝", g);
            }
        }
        DestroyElementsTable();
    }

    /// 回归（2026-08-04 用户实测：太阳光只亮顶部行 / 真空阻隔光）：
    /// 真实游戏真空 defaultMass=0 → 旧 massScale = 1/0 = INFINITY；
    /// 曝光函数 `INFINITY × 上方格质量(0) = NaN` → NaN 毒化整列 → 空间生态区全 0。
    /// 修复：零质量非固体 massScale 回退 1.0 + 曝光函数 NaN 防御。
    /// 期望：全真空列（含空间区）曝光 = 255（真空不挡光）。
    #[test]
    fn exposed_sun_passes_through_vacuum_space_biome() {
        let _lock = LIB_TESTS_LOCK.lock();
        // 元素表：0=真空(state0, factor0, **defaultMass=0**)；1=固体(state3, factor0.5)
        {
            let mut w = BinaryBufferWriter::new();
            w.write_int(2);
            let elems = [
                {
                    let mut e = Element::default();
                    e.id = 0;
                    e.state = 0;
                    e.number_of_gradient_colors = 1;
                    e.light_absorption_factor = 0.0;
                    e.default_values.mass = 0.0; // 真实游戏真空值
                    e
                },
                {
                    let mut e = Element::default();
                    e.id = 1;
                    e.state = 3;
                    e.number_of_gradient_colors = 1;
                    e.light_absorption_factor = 0.5;
                    e.default_values.mass = 1.0;
                    e
                },
            ];
            for elem in &elems {
                let elem_bytes = unsafe {
                    std::slice::from_raw_parts(elem as *const Element as *const u8, 164).to_vec()
                };
                w.write_bytes(&elem_bytes);
            }
            for _ in 0..2 {
                w.write_int(0);
            }
            let data = w.into_bytes();
            let mut reader = BinaryBufferReader::new(&data);
            let result = CreateElementsTable(&mut reader);
            assert!(!result.is_null(), "CreateElementsTable should succeed");
        }
        let mut sd = SimData::new_for_allocate(6, 5, 1, true, false);
        sd.worlds.push(WorldOffsetData {
            offset_x: 0,
            offset_y: 0,
            width: 4,
            height: 3,
        });
        // 全真空：sim 内部行 1..3（game 行 0..2）全为真空、质量 0
        unsafe {
            for buf in [&mut *sd.updated_cells.ptr, &mut *sd.cells.ptr] {
                for idx in 0..30usize {
                    buf.element_idx.set(idx, 0);
                    buf.mass.set(idx, 0.0);
                    buf.temperature.set(idx, 300.0);
                }
            }
        }
        let mut gd = GameData::new(4, 3);
        update_exposed_to_sun_property_texture(&sd, &mut gd);
        unsafe {
            let exposed = std::slice::from_raw_parts(
                gd.property_texture_exposed_to_sunlight.ptr,
                4 * 3,
            );
            // 修复前：上方格真空 massScale=INF × mass 0 = NaN → 除填充行外全 0
            // 修复后：真空不挡光 → 所有 game 行（含空间区）255
            for gr in 0..3 {
                for gc in 0..4 {
                    let v = exposed[gr * 4 + gc];
                    assert_eq!(
                        v, 255,
                        "真空空间区 game({gr},{gc}) 应满曝光 255（NaN 防御），got {v}"
                    );
                }
            }
        }
        DestroyElementsTable();
    }

    /// 空元素表 / 无 world 下不 panic（基线）。
    #[test]
    fn update_exposed_to_sun_no_panic_empty_table() {
        let _lock = LIB_TESTS_LOCK.lock();
        DestroyElementsTable(); // 确保空表
        let sd = SimData::new_for_allocate(6, 5, 1, true, false);
        let mut gd = GameData::new(4, 3);
        update_exposed_to_sun_property_texture(&sd, &mut gd);
        unsafe {
            let exposed = std::slice::from_raw_parts(
                gd.property_texture_exposed_to_sunlight.ptr,
                4 * 3,
            );
            assert_eq!(exposed[0], 0, "空元素表下应保持 0");
        }
    }
}
