//! T3：区域事件收集器（d3 并行路径的事件安全前提）。**常驻基础设施**：
//! sink 为 None（串行路径/组件路径）→ 写全局 `sim.sim_events`，行为与原版一致。
//!
//! 原版区域串行时，物理 helper 直接把事件 push 到全局 `sim.sim_events`。
//! d3 并行路径下，每个区域任务必须写**自己的**事件缓冲，scope 结束后按区域序
//! 合并进全局——否则并行任务竞争全局事件且跨区域顺序不确定。
//! 组件事件（发射器/建筑换热等，阶段 B 串行）不经过本收集器，保持写全局。
//!
//! 覆盖 8 类网格物理事件（对应 `SimEvents` 同名字段）：
//! substance_change / spawn_liquid / spawn_ore / unstable_cell / cell_melted /
//! spawn_fx / world_damage / backwall_should_transition。

use crate::a_framework::game_data::{
    BackwallShouldTransitionInfo, CellMeltedInfo, SpawnFXInfo, SpawnFallingLiquidInfo,
    SpawnOreInfo, SubstanceChangeInfo, UnstableCellInfo, WorldDamageInfo,
};
use crate::a_framework::sim_data::SimData;
use crate::a_framework::stl_shim::MsvcVector;
use std::cell::Cell;

/// 单区域事件缓冲（与全局 SimEvents 对应字段同名）。
#[derive(Default)]
pub struct RegionEvents {
    pub substance_change_info: MsvcVector<SubstanceChangeInfo>,
    pub spawn_liquid_info: MsvcVector<SpawnFallingLiquidInfo>,
    pub spawn_ore_info: MsvcVector<SpawnOreInfo>,
    pub unstable_cell_info: MsvcVector<UnstableCellInfo>,
    pub cell_melted_info: MsvcVector<CellMeltedInfo>,
    pub spawn_fx_info: MsvcVector<SpawnFXInfo>,
    pub world_damage_info: MsvcVector<WorldDamageInfo>,
    pub backwall_should_transition_info: MsvcVector<BackwallShouldTransitionInfo>,
}

thread_local! {
    /// 当前线程正在服务的区域事件缓冲（None = 串行路径，写全局 SimEvents）。
    static REGION_EVENTS_SINK: Cell<Option<*mut RegionEvents>> = const { Cell::new(None) };
}

/// 在 `f` 执行期间把本线程的事件路由到 `sink`（结束后恢复原值）。
/// 供 d3 并行路径的每个区域任务调用。
pub(crate) fn with_region_sink<T>(sink: *mut RegionEvents, f: impl FnOnce() -> T) -> T {
    REGION_EVENTS_SINK.with(|s| {
        let prev = s.get();
        s.set(Some(sink));
        let result = f();
        s.set(prev);
        result
    })
}

fn sink() -> Option<*mut RegionEvents> {
    REGION_EVENTS_SINK.with(|s| s.get())
}

fn push_to_sink_or_global<T>(
    sim: &mut SimData,
    sink_field: fn(*mut RegionEvents) -> *mut MsvcVector<T>,
    global_field: fn(*mut crate::a_framework::sim_events::SimEvents) -> *mut MsvcVector<T>,
    value: T,
) where
    T: Default + Copy,
{
    unsafe {
        if let Some(ptr) = sink() {
            let v = &mut *sink_field(ptr);
            v.push(value);
            return;
        }
        if sim.sim_events.ptr.is_null() {
            return;
        }
        let v = &mut *global_field(sim.sim_events.ptr);
        v.push(value);
    }
}

macro_rules! emit {
    ($name:ident, $ty:ty, $field:ident) => {
        pub(crate) fn $name(sim: &mut SimData, value: $ty) {
            push_to_sink_or_global(
                sim,
                |p: *mut RegionEvents| unsafe { std::ptr::addr_of_mut!((*p).$field) },
                |p: *mut crate::a_framework::sim_events::SimEvents| unsafe {
                    std::ptr::addr_of_mut!((*p).$field)
                },
                value,
            );
        }
    };
}

emit!(emit_substance_change, SubstanceChangeInfo, substance_change_info);
emit!(emit_spawn_liquid, SpawnFallingLiquidInfo, spawn_liquid_info);
emit!(emit_spawn_ore, SpawnOreInfo, spawn_ore_info);
emit!(emit_unstable_cell, UnstableCellInfo, unstable_cell_info);
emit!(emit_cell_melted, CellMeltedInfo, cell_melted_info);
emit!(emit_spawn_fx, SpawnFXInfo, spawn_fx_info);
emit!(emit_world_damage, WorldDamageInfo, world_damage_info);
emit!(
    emit_backwall_should_transition,
    BackwallShouldTransitionInfo,
    backwall_should_transition_info
);

impl RegionEvents {
    /// 按区域序合并进全局 SimEvents（字段声明顺序 = 原版区域串行 push 顺序），
    /// 合并后清空本区域缓冲（阶段 A/C 各合并一次，防重复追加）。
    pub fn merge_into(&mut self, sim: &mut SimData) {
        if sim.sim_events.ptr.is_null() {
            return;
        }
        let events = unsafe { &mut *sim.sim_events.ptr };
        for i in 0..self.substance_change_info.len() {
            events
                .substance_change_info
                .push(self.substance_change_info.get(i));
        }
        self.substance_change_info.clear();
        for i in 0..self.spawn_liquid_info.len() {
            events.spawn_liquid_info.push(self.spawn_liquid_info.get(i));
        }
        self.spawn_liquid_info.clear();
        for i in 0..self.spawn_ore_info.len() {
            events.spawn_ore_info.push(self.spawn_ore_info.get(i));
        }
        self.spawn_ore_info.clear();
        for i in 0..self.unstable_cell_info.len() {
            events.unstable_cell_info.push(self.unstable_cell_info.get(i));
        }
        self.unstable_cell_info.clear();
        for i in 0..self.cell_melted_info.len() {
            events.cell_melted_info.push(self.cell_melted_info.get(i));
        }
        self.cell_melted_info.clear();
        for i in 0..self.spawn_fx_info.len() {
            events.spawn_fx_info.push(self.spawn_fx_info.get(i));
        }
        self.spawn_fx_info.clear();
        for i in 0..self.world_damage_info.len() {
            events.world_damage_info.push(self.world_damage_info.get(i));
        }
        self.world_damage_info.clear();
        for i in 0..self.backwall_should_transition_info.len() {
            events
                .backwall_should_transition_info
                .push(self.backwall_should_transition_info.get(i));
        }
        self.backwall_should_transition_info.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIB_TESTS_LOCK;

    fn dummy_sim() -> SimData {
        let sd = SimData::new_for_allocate(6, 6, 1, false, false);
        sd
    }

    #[test]
    fn sink_routes_to_region_events() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut region = RegionEvents::default();
        let mut sim = dummy_sim();
        with_region_sink(&mut region as *mut RegionEvents, || {
            emit_substance_change(&mut sim, SubstanceChangeInfo::default());
        });
        assert_eq!(region.substance_change_info.len(), 1, "sink 有值 → 写区域缓冲");
        // 全局不脏
        let events = unsafe { &*sim.sim_events.ptr };
        assert_eq!(events.substance_change_info.len(), 0, "sink 有值时不应写全局");
    }

    #[test]
    fn no_sink_writes_global() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut sim = dummy_sim();
        emit_substance_change(&mut sim, SubstanceChangeInfo::default());
        let events = unsafe { &*sim.sim_events.ptr };
        assert_eq!(events.substance_change_info.len(), 1, "sink 为 None → 写全局");
    }

    #[test]
    fn with_region_sink_restores_previous() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut r_outer = RegionEvents::default();
        let mut r_inner = RegionEvents::default();
        let mut sim = dummy_sim();
        with_region_sink(&mut r_outer as *mut RegionEvents, || {
            with_region_sink(&mut r_inner as *mut RegionEvents, || {
                emit_substance_change(&mut sim, SubstanceChangeInfo::default());
            });
            emit_substance_change(&mut sim, SubstanceChangeInfo::default());
        });
        assert_eq!(r_outer.substance_change_info.len(), 1);
        assert_eq!(r_inner.substance_change_info.len(), 1);
    }

    #[test]
    fn merge_into_appends_in_field_order_and_clears() {
        let _lock = LIB_TESTS_LOCK.lock();
        let mut region = RegionEvents::default();
        region
            .substance_change_info
            .push(SubstanceChangeInfo::default());
        region.spawn_liquid_info.push(SpawnFallingLiquidInfo::default());
        let mut sim = dummy_sim();
        region.merge_into(&mut sim);
        let events = unsafe { &*sim.sim_events.ptr };
        assert_eq!(events.substance_change_info.len(), 1);
        assert_eq!(events.spawn_liquid_info.len(), 1);
        assert_eq!(region.substance_change_info.len(), 0, "合并后应清空");
        assert_eq!(region.spawn_liquid_info.len(), 0);
    }
}
