//! T4：每区域独立持久 random_seed + displacement_direction（方案 A）。**常驻基础设施**：
//! 无路由（串行路径）→ 用 sim 全局字段，行为零变化。
//!
//! 原版区域串行：全局 LCG（`sim.random_seed`）按区域 0→1→… 顺序消耗，
//! `sim.displacement_direction` 每格交替反转。d3 并行路径下跨区域消耗顺序天然不确定
//! → 每区域独立**持久**种子/方向：随机流自洽、逐帧确定，世界间互不干扰
//! （统计等价，与原版全局序列不同）。

use crate::a_framework::sim_data::{ActiveRegion, SimData};
use parking_lot::Mutex;
use std::cell::Cell;

/// 单区域随机状态（并行路径每区域一份，跨子步持久）。
#[derive(Clone, Copy, Default)]
pub struct RegionRng {
    pub random_seed: u32,
    pub displacement_direction: i32,
}

thread_local! {
    /// 当前线程正在服务的区域随机状态（None = 串行路径，用 sim 全局字段）。
    static REGION_RNG: Cell<Option<*mut RegionRng>> = const { Cell::new(None) };
}

/// 在 `f` 执行期间把本线程的随机状态路由到 `rng`（结束后恢复原值）。
pub(crate) fn with_region_rng<T>(rng: *mut RegionRng, f: impl FnOnce() -> T) -> T {
    REGION_RNG.with(|s| {
        let prev = s.get();
        s.set(Some(rng));
        let result = f();
        s.set(prev);
        result
    })
}

pub(crate) fn region_rng_ptr() -> Option<*mut RegionRng> {
    REGION_RNG.with(|s| s.get())
}

/// 区域 key（偏移矩形），用于检测 active_regions 是否变化。
#[derive(Clone, Copy, PartialEq, Eq)]
struct RegionKey {
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
}

impl RegionKey {
    fn of(r: &ActiveRegion) -> Self {
        Self {
            min_x: r.min_x,
            min_y: r.min_y,
            max_x: r.max_x,
            max_y: r.max_y,
        }
    }
}

/// 区域随机状态集合（按 active_regions 顺序对齐，跨子步持久）。
#[derive(Default)]
pub struct RegionRngState {
    pub list: Vec<RegionRng>,
    keys: Vec<RegionKey>,
}

impl RegionRngState {
    pub const fn empty() -> Self {
        Self {
            list: Vec::new(),
            keys: Vec::new(),
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// 与当前 active_regions 对齐：key 相同保留原状态，新增/变化用
    /// `base_seed + index×0x9E3779B9` 派生（LCG 初始方向 -1，与原版一致）。
    pub fn realign(&mut self, base_seed: u32, regions: &[ActiveRegion]) {
        let keys: Vec<RegionKey> = regions.iter().map(RegionKey::of).collect();
        if keys == self.keys {
            return;
        }
        let mut list = Vec::with_capacity(keys.len());
        for (i, key) in keys.iter().enumerate() {
            let preserved = self
                .keys
                .iter()
                .position(|old| old == key)
                .map(|idx| self.list[idx]);
            let rng = match preserved {
                Some(r) => r,
                None => RegionRng {
                    random_seed: base_seed.wrapping_add((i as u32).wrapping_mul(0x9E37_79B9)),
                    displacement_direction: -1,
                },
            };
            list.push(rng);
        }
        self.list = list;
        self.keys = keys;
    }

    /// 返回各区域 RNG 的裸指针（供 rayon 任务以 with_region_rng 使用）。
    /// 调用方保证：scope 期间不 realign。
    pub fn ptrs(&mut self) -> Vec<*mut RegionRng> {
        self.list.iter_mut().map(|r| r as *mut RegionRng).collect()
    }
}

static REGION_RNG_STATE: Mutex<RegionRngState> = Mutex::new(RegionRngState::empty());

/// 并行路径每帧开头调用：按当前 active_regions 对齐区域随机状态。
pub(crate) fn realign_region_rngs(sim: &SimData, regions: &[ActiveRegion]) {
    let mut st = REGION_RNG_STATE.lock();
    st.realign(sim.random_seed, regions);
}

/// 返回各区域 RNG 裸指针（调用方保证 scope 期间不 realign）。
pub(crate) fn region_rng_ptrs() -> Vec<*mut RegionRng> {
    REGION_RNG_STATE.lock().ptrs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;

    fn region(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> ActiveRegion {
        ActiveRegion {
            min_x,
            min_y,
            max_x,
            max_y,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        }
    }

    #[test]
    fn realign_preserves_same_key_and_derives_new() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut st = RegionRngState::new();
        let regions = [region(1, 1, 10, 10), region(20, 1, 30, 10)];
        st.realign(100, &regions);
        let s0 = st.list[0].random_seed;
        let s1 = st.list[1].random_seed;
        // key 不变 → 保留原种子
        st.realign(100, &regions);
        assert_eq!(st.list[0].random_seed, s0);
        assert_eq!(st.list[1].random_seed, s1);
        // 移除区域 → 重排保留匹配 key
        st.realign(100, &regions[..1]);
        assert_eq!(st.list[0].random_seed, s0);
        // 新增区域 → 派生 seed = base + i×0x9E3779B9
        st.realign(100, &[regions[0], region(40, 1, 50, 10)]);
        assert_eq!(st.list[0].random_seed, s0);
        assert_eq!(
            st.list[1].random_seed,
            100u32.wrapping_add(1u32.wrapping_mul(0x9E37_79B9)),
            "新增区域种子 = base + index×0x9E3779B9"
        );
        assert_eq!(st.list[1].displacement_direction, -1, "新区域方向初始 -1");
    }

    #[test]
    fn route_prefers_region_rng_over_global() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut rng = RegionRng {
            random_seed: 42,
            displacement_direction: 1,
        };
        assert!(region_rng_ptr().is_none(), "无 with_region_rng → None");
        with_region_rng(&mut rng as *mut RegionRng, || {
            let p = region_rng_ptr();
            assert!(p.is_some(), "with_region_rng 内 → Some");
            unsafe {
                assert_eq!((*p.unwrap()).random_seed, 42);
            }
        });
        assert!(region_rng_ptr().is_none(), "退出后恢复 None");
    }
}
