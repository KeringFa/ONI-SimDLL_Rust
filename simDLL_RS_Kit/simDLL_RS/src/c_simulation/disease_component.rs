//! 病菌消息/组件（阶段 3）：ConsumeDisease、CellDiseaseModification、
//! DiseaseEmitter/DiseaseConsumer 组件。
//!
//! 对照原版 06_process_messages.c（ProcessConsumeDisease L422）、SimDLL_Source.c
//! （CellDiseaseModification L116412-116474）、11_msvcrt_ignored.c
//! （DiseaseEmitter L37492-37790、DiseaseConsumer L37036-37115、GetReachableCells L34451、
//! Flood L33466）。
use crate::a_framework::game_data::DiseaseConsumedCallback;
use crate::a_framework::sim_data::SimData;
use crate::a_framework::sim_frame_manager::SimFrameInfo;
use crate::d1_activity::RegionBounds;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// game→sim 坐标换算（原版 L37492 Modify 同款）。
fn game_to_sim_cell(game_cell: i32, width: i32) -> i32 {
    let gw = width - 2;
    (game_cell / gw + 1) * width + game_cell % gw + 1
}

/// AddDiseaseEmitterMessage（4B：{int callbackIdx}）。
#[derive(Clone, Copy, Default, Debug)]
pub struct AddDiseaseEmitterMsg {
    pub callback_idx: i32,
}

impl AddDiseaseEmitterMsg {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 4 {
            return None;
        }
        Some(Self {
            callback_idx: i32::from_le_bytes(b[0..4].try_into().ok()?),
        })
    }
}

/// ModifyDiseaseEmitterMessage（20B：{int handle, int gameCell, byte diseaseIdx,
/// byte maxDepth, pad, pad, float emitInterval, int emitCount}）。
#[derive(Clone, Copy, Default, Debug)]
pub struct ModifyDiseaseEmitterMsg {
    pub handle: i32,
    pub game_cell: i32,
    pub disease_idx: u8,
    pub max_depth: u8,
    pub emit_interval: f32,
    pub emit_count: i32,
}

impl ModifyDiseaseEmitterMsg {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 20 {
            return None;
        }
        Some(Self {
            handle: i32::from_le_bytes(b[0..4].try_into().ok()?),
            game_cell: i32::from_le_bytes(b[4..8].try_into().ok()?),
            disease_idx: b[8],
            max_depth: b[9],
            emit_interval: f32::from_le_bytes(b[12..16].try_into().ok()?),
            emit_count: i32::from_le_bytes(b[16..20].try_into().ok()?),
        })
    }
}

/// RemoveDiseaseEmitterMessage（8B：{int handle, int callbackIdx}）。
#[derive(Clone, Copy, Default, Debug)]
pub struct RemoveDiseaseEmitterMsg {
    pub handle: i32,
    pub callback_idx: i32,
}

impl RemoveDiseaseEmitterMsg {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 8 {
            return None;
        }
        Some(Self {
            handle: i32::from_le_bytes(b[0..4].try_into().ok()?),
            callback_idx: i32::from_le_bytes(b[4..8].try_into().ok()?),
        })
    }
}

/// DiseaseEmitterData（20B = 0x14，00_types_reference.c L3319）。
#[derive(Clone, Copy, Default, Debug)]
pub struct DiseaseEmitterData {
    pub cell: i32,          // sim 格 @0
    pub disease_idx: u8,    // @4
    pub range: u8,          // @5
    pub emit_count: i32,    // @8
    pub emit_interval: f32, // @0xc
    pub elapsed_time: f32,  // @0x10
}

/// DiseaseEmitter 注册表（版本句柄 + free list，仿 radiation_emitter.rs）。
pub struct DiseaseEmitterRegistry {
    records: Vec<DiseaseEmitterData>,
    items: Vec<i32>,
    versions: Vec<u8>,
    free_handles: Vec<i32>,
    emitted_info: Vec<crate::a_framework::game_data::DiseaseEmittedInfo>,
}

impl DiseaseEmitterRegistry {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            items: Vec::new(),
            versions: Vec::new(),
            free_handles: Vec::new(),
            emitted_info: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.items.clear();
        self.versions.clear();
        self.free_handles.clear();
        self.emitted_info.clear();
    }

    pub fn active_count(&self) -> usize {
        self.items.iter().filter(|&&ri| ri >= 0).count()
    }

    pub fn record(&self, handle: i32) -> Option<&DiseaseEmitterData> {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return None;
        }
        let ri = self.items[index];
        if ri < 0 {
            return None;
        }
        self.records.get(ri as usize)
    }

    /// Register（原版 L37549）：分配槽，默认 cell=-1/diseaseIdx=0xff/range=0/
    /// emitCount=0/interval=0/elapsedTime=0；返回 handle。
    pub fn register(&mut self) -> i32 {
        let index;
        if let Some(free) = self.free_handles.pop() {
            index = free as usize;
            let ri = self.records.len() as i32;
            self.items[index] = ri;
            self.records.push(DiseaseEmitterData::default());
            self.versions[index] = self.versions[index].wrapping_add(1);
        } else {
            index = self.items.len();
            let ri = self.records.len() as i32;
            self.items.push(ri);
            self.records.push(DiseaseEmitterData::default());
            self.versions.push(0);
        }
        let record = &mut self.records[self.items[index] as usize];
        record.cell = -1;
        record.disease_idx = 0xff;
        ((self.versions[index] as i32) << 24) | (index as i32 & 0xffffff)
    }

    /// Modify（原版 L37492）：版本校验后写字段；不重置 elapsedTime。
    pub fn modify(&mut self, sim: &SimData, msg: &ModifyDiseaseEmitterMsg) -> bool {
        let index = (msg.handle & 0xffffff) as usize;
        let version = ((msg.handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return false;
        }
        let ri = self.items[index];
        if ri < 0 {
            return false;
        }
        let rec = &mut self.records[ri as usize];
        rec.cell = game_to_sim_cell(msg.game_cell, sim.width);
        rec.disease_idx = msg.disease_idx;
        rec.range = msg.max_depth;
        rec.emit_count = msg.emit_count;
        rec.emit_interval = msg.emit_interval;
        true
    }

    /// Unregister（原版 L37635）：释放槽 + 版本+1（立即释放；原版 CompactedVector::Free）。
    pub fn remove(&mut self, handle: i32) -> bool {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return false;
        }
        let ri = self.items[index];
        if ri < 0 {
            return false;
        }
        let ri = ri as usize;
        self.records.swap_remove(ri);
        // 修复被 swap 移动的记录引用（原最后一条 → ri）
        for it in self.items.iter_mut() {
            if *it as usize == self.records.len() {
                *it = ri as i32;
            }
        }
        self.versions[index] = self.versions[index].wrapping_add(1);
        self.items[index] = -1;
        self.free_handles.push(index as i32);
        true
    }
}

fn push_state_changed(sim: &mut SimData, callback_idx: i32, sim_handle: i32) {
    if sim.sim_events.ptr.is_null() {
        return;
    }
    let events = unsafe { &mut *sim.sim_events.ptr };
    events.component_state_changed_messages.push(
        crate::a_framework::game_data::ComponentStateChangedMessage {
            callback_idx,
            sim_handle,
        },
    );
}

static DISEASE_EMITTER_MANAGER: Lazy<Mutex<DiseaseEmitterRegistry>> =
    Lazy::new(|| Mutex::new(DiseaseEmitterRegistry::new()));

/// 新世界分配时清空（与 radiation emitter 同约定；原版组件在 SimData 内，随分配重建）。
pub fn clear_disease_emitters() {
    DISEASE_EMITTER_MANAGER.lock().clear();
}

pub fn disease_emitter_count() -> usize {
    DISEASE_EMITTER_MANAGER.lock().active_count()
}

/// 每帧输出交付（原版 CopySimDataToGame diseaseEmittedInfo 拷贝 + 源重置）：
/// resize game 到 sim 长度 → 逐条拷贝 → 源条目重置为 {0xFF, 0}（无发射帧不残留旧数据）。
/// 2026-08-07 补齐：此前 copy_sim_data_to_game 从未交付该通道，C# diseaseEmittedInfos 恒空。
pub fn swap_disease_emitted_output(game_data: &mut crate::a_framework::game_data::GameData) {
    let mut mgr = DISEASE_EMITTER_MANAGER.lock();
    let n = mgr.emitted_info.len();
    game_data.disease_emitted_info.resize(n, Default::default());
    for idx in 0..n {
        game_data.disease_emitted_info.set(idx, mgr.emitted_info[idx]);
        mgr.emitted_info[idx] = crate::a_framework::game_data::DiseaseEmittedInfo {
            disease_idx: 0xFF,
            padding: [0; 3],
            count: 0,
        };
    }
}

#[cfg(test)]
pub(crate) fn disease_emitter_record(handle: i32) -> Option<DiseaseEmitterData> {
    DISEASE_EMITTER_MANAGER.lock().record(handle).copied()
}

#[cfg(test)]
pub(crate) fn disease_emitter_handle(ordinal: usize) -> Option<i32> {
    let mgr = DISEASE_EMITTER_MANAGER.lock();
    let mut seen = 0usize;
    for (index, &ri) in mgr.items.iter().enumerate() {
        if ri >= 0 {
            if seen == ordinal {
                return Some(((mgr.versions[index] as i32) << 24) | (index as i32 & 0xffffff));
            }
            seen += 1;
        }
    }
    None
}

#[cfg(test)]
pub(crate) fn disease_emitter_emitted(ordinal: usize) -> Option<(u8, i32)> {
    let mgr = DISEASE_EMITTER_MANAGER.lock();
    mgr.emitted_info.get(ordinal).map(|e| (e.disease_idx, e.count))
}

/// 处理帧内 disease_emitter_messages：adds(4B) → modifies(20B) → removes(8B)。
/// Register 回调推 {handle, callbackIdx}；Remove 回调推 {−1, callbackIdx}（原版 frame manager）。
pub fn process_disease_emitter_messages(
    frame: &crate::a_framework::sim_frame_manager::SimFrameInfo,
    sim: &mut SimData,
) {
    let mut mgr = DISEASE_EMITTER_MANAGER.lock();
    for b in frame.disease_emitter_messages.adds.as_slice().chunks(4) {
        if let Some(msg) = AddDiseaseEmitterMsg::from_bytes(b) {
            let handle = mgr.register();
            if msg.callback_idx != -1 {
                push_state_changed(sim, msg.callback_idx, handle);
            }
        }
    }
    for b in frame.disease_emitter_messages.modifies.as_slice().chunks(20) {
        if let Some(msg) = ModifyDiseaseEmitterMsg::from_bytes(b) {
            mgr.modify(sim, &msg);
        }
    }
    for b in frame.disease_emitter_messages.removes.as_slice().chunks(8) {
        if let Some(msg) = RemoveDiseaseEmitterMsg::from_bytes(b) {
            mgr.remove(msg.handle);
            if msg.callback_idx != -1 {
                push_state_changed(sim, msg.callback_idx, -1);
            }
        }
    }
    // 注意：frame 向量清空由调用方（frame_processor）统一处理（与 radiation emitter 同约定）。
}

/// DiseaseEmitter::Update（原版 L37651）：区域门控 + interval 触发 +
/// GetReachableCells 发射（emitCount 正负两分支）+ emittedInfo 输出。
pub(crate) fn update_disease_emitters(sim: &mut SimData, dt: f32, bounds: RegionBounds) {
    let mut mgr = DISEASE_EMITTER_MANAGER.lock();
    let width = sim.width as usize;
    if width < 3 || sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
        return;
    }
    let record_count = mgr.records.len();
    mgr.emitted_info.resize(record_count, Default::default());
    let disease_ptr = crate::globals::G_DISEASE.lock().0;
    let disease = if disease_ptr.is_null() {
        None
    } else {
        Some(unsafe { &*disease_ptr })
    };
    for idx in 0..mgr.records.len() {
        let rec = mgr.records[idx];
        if rec.disease_idx == 0xff {
            continue;
        }
        let cell = rec.cell as usize;
        let x = cell % width;
        let y = cell / width;
                if x < bounds.min_x || x >= bounds.max_x || y < bounds.min_y || y >= bounds.max_y {
            continue;
        }
        let interval = rec.emit_interval;
        let elapsed = rec.elapsed_time;
        if interval <= elapsed {
            let emit_count = rec.emit_count;
            let disease_idx = rec.disease_idx;
            if let Some(d) = disease {
                if (disease_idx as usize) < d.diseases.len() {
                    let reachable = get_reachable_cells(sim, x, y, rec.range as usize);
                    if emit_count < 0 {
                        // 仅同病菌格累加（负 emitCount 用于清除）
                        for &c in &reachable {
                            let updated = unsafe { &mut *sim.updated_cells.ptr };
                            if updated.disease_idx.get(c) == disease_idx {
                                crate::c2_physics::liquid_flow::add_disease_to_cell(
                                    updated, c, disease_idx, emit_count,
                                );
                            }
                        }
                    } else {
                        for &c in &reachable {
                            let updated = unsafe { &mut *sim.updated_cells.ptr };
                            crate::c2_physics::liquid_flow::add_disease_to_cell(
                                updated, c, disease_idx, emit_count,
                            );
                        }
                    }
                    if idx < mgr.emitted_info.len() {
                        mgr.emitted_info[idx] =
                            crate::a_framework::game_data::DiseaseEmittedInfo {
                                disease_idx,
                                padding: [0; 3],
                                count: emit_count,
                            };
                    }
                }
            }
            mgr.records[idx].elapsed_time = elapsed - interval;
        }
        mgr.records[idx].elapsed_time += dt;
    }
}

/// DiseaseConsumerData（12B，00_types_reference.c L1514）。
#[derive(Clone, Copy, Default, Debug)]
pub struct DiseaseConsumerData {
    pub consumption_rate: f32,          // @0
    pub max_depth: u8,                  // @4
    pub cell_x: u16,                    // @5
    pub cell_y: u16,                    // @7
    pub property_mask: u16,             // @9
}

/// DiseaseConsumer 注册表（防御：C# 仅 SimMessageHashes 枚举、无发送方；
/// 原版 Register 不读消息字段，注册零值记录）。
pub struct DiseaseConsumerRegistry {
    records: Vec<DiseaseConsumerData>,
    items: Vec<i32>,
    versions: Vec<u8>,
    free_handles: Vec<i32>,
}

impl DiseaseConsumerRegistry {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            items: Vec::new(),
            versions: Vec::new(),
            free_handles: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.items.clear();
        self.versions.clear();
        self.free_handles.clear();
    }

    pub fn active_count(&self) -> usize {
        self.items.iter().filter(|&&ri| ri >= 0).count()
    }

    /// Register（原版 L37036）：分配槽，注册零值记录（消息字段未读）。
    pub fn register(&mut self) -> i32 {
        let index;
        if let Some(free) = self.free_handles.pop() {
            index = free as usize;
            let ri = self.records.len() as i32;
            self.items[index] = ri;
            self.records.push(DiseaseConsumerData::default());
            self.versions[index] = self.versions[index].wrapping_add(1);
        } else {
            index = self.items.len();
            let ri = self.records.len() as i32;
            self.items.push(ri);
            self.records.push(DiseaseConsumerData::default());
            self.versions.push(0);
        }
        ((self.versions[index] as i32) << 24) | (index as i32 & 0xffffff)
    }

    /// Unregister（原版 L37115）：释放槽 + 版本+1。
    pub fn remove(&mut self, handle: i32) -> bool {
        let index = (handle & 0xffffff) as usize;
        let version = ((handle >> 24) & 0xff) as u8;
        if index >= self.versions.len() || self.versions[index] != version {
            return false;
        }
        let ri = self.items[index];
        if ri < 0 {
            return false;
        }
        let ri = ri as usize;
        self.records.swap_remove(ri);
        for it in self.items.iter_mut() {
            if *it as usize == self.records.len() {
                *it = ri as i32;
            }
        }
        self.versions[index] = self.versions[index].wrapping_add(1);
        self.items[index] = -1;
        self.free_handles.push(index as i32);
        true
    }
}

static DISEASE_CONSUMER_MANAGER: Lazy<Mutex<DiseaseConsumerRegistry>> =
    Lazy::new(|| Mutex::new(DiseaseConsumerRegistry::new()));

/// 新世界分配时清空（与 emitter 同约定）。
pub fn clear_disease_consumers() {
    DISEASE_CONSUMER_MANAGER.lock().clear();
}

pub fn disease_consumer_count() -> usize {
    DISEASE_CONSUMER_MANAGER.lock().active_count()
}

/// 处理帧内 disease_consumer_messages：adds(12B) → modifies(20B，接受忽略) → removes(8B)。
/// C# 无发送方 → 生产环境注册表恒空；本实现为防御性完整消息链路。
pub fn process_disease_consumer_messages(
    frame: &crate::a_framework::sim_frame_manager::SimFrameInfo,
    sim: &mut SimData,
) {
    let mut mgr = DISEASE_CONSUMER_MANAGER.lock();
    // adds：12B/条；原版 Register 不读字段（注册零值记录）；回调推 {handle, callbackIdx}
    for b in frame.disease_consumer_messages.adds.as_slice().chunks(12) {
        if b.len() < 8 {
            continue;
        }
        let callback_idx = i32::from_le_bytes(b[4..8].try_into().unwrap_or([0; 4]));
        let handle = mgr.register();
        if callback_idx != -1 {
            push_state_changed(sim, callback_idx, handle);
        }
    }
    // modifies：20B/条；原版 vtable 未解析 + C# 无发送方 → 接受忽略（预留）
    for _ in frame.disease_consumer_messages.modifies.as_slice().chunks(20) {}
    // removes：8B/条 {handle, callbackIdx}
    for b in frame.disease_consumer_messages.removes.as_slice().chunks(8) {
        if b.len() < 8 {
            continue;
        }
        let handle = i32::from_le_bytes(b[0..4].try_into().unwrap_or([0; 4]));
        let callback_idx = i32::from_le_bytes(b[4..8].try_into().unwrap_or([0; 4]));
        mgr.remove(handle);
        if callback_idx != -1 {
            push_state_changed(sim, callback_idx, -1);
        }
    }
}

/// GetReachableCells — 4 向 BFS 泛洪（原版 Flood L33466 + GetReachableCells L34451）。
///
/// 从 (x,y) 起，深度 < range 的格全部可达（三个 state 过滤 flag 全 false →
/// 不过滤元素内容）；跳过 x==0/y==0/越界格（原版 L33506-33513 边界检查）。
/// 返回可达 sim 格索引（BFS 顺序：上、左、右、下）。
pub(crate) fn get_reachable_cells(sim: &SimData, x: usize, y: usize, range: usize) -> Vec<usize> {
    let width = sim.width as usize;
    let height = sim.height as usize;
    let mut reachable = Vec::new();
    if range == 0 || x >= width || y >= height || width == 0 || height == 0 {
        return reachable;
    }
    let total = width * height;
    // visited 用线程本地世代戳池复用（省每次 218KB 分配+清零，行为等价）。
    crate::c_simulation::bfs_scratch::with_visited_scratch(total, |visited, generation| {
        let mut queue = std::collections::VecDeque::new();
        visited[y * width + x] = generation;
        queue.push_back((x, y, 0usize));
        while let Some((cx, cy, depth)) = queue.pop_front() {
            if depth >= range {
                continue;
            }
            if cx == 0 || cy == 0 || cx >= width || cy >= height {
                continue;
            }
            reachable.push(cy * width + cx);
            let neighbors = [
                (cx, cy.wrapping_sub(1)), // 上
                (cx.wrapping_sub(1), cy), // 左
                (cx + 1, cy),             // 右
                (cx, cy + 1),             // 下
            ];
            for (nx, ny) in neighbors {
                if nx < width && ny < height {
                    let nc = ny * width + nx;
                    if visited[nc] != generation {
                        visited[nc] = generation;
                        queue.push_back((nx, ny, depth + 1));
                    }
                }
            }
        }
    });
    reachable
}

/// ConsumeDisease 处理（原版 06_process_messages.c L422-522）。
///
/// 16B/条：{gameCell i32, callbackIdx i32, percentToConsume f32, maxToConsume i32}。
/// consumed = (int)(count × percent + 0.5)（四舍五入）→ 钳制 maxToConsume →
/// count 扣减 → count<1 清四字段 → callbackIdx≠-1 时 push DiseaseConsumedCallback{12B}。
pub(crate) fn process_consume_disease(frame: &mut SimFrameInfo, sim: &mut SimData) {
    let width = sim.width as i32;
    let game_w = width - 2;
    let bytes = frame.consume_disease.as_slice();
    let mut off = 0usize;
    while off + 16 <= bytes.len() {
        let game_cell = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let callback_idx = i32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
        let percent = f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
        let max_to_consume = i32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap());
        off += 16;
        let sim_cell = (game_cell / game_w + 1) * width + game_cell % game_w + 1;
        if sim.cells.ptr.is_null() || sim.updated_cells.ptr.is_null() {
            continue;
        }
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        if (sim_cell as usize) >= updated.disease_idx.len()
            || (sim_cell as usize) >= updated.disease_count.len()
        {
            continue;
        }
        let mut disease_idx = updated.disease_idx.get(sim_cell as usize);
        let mut consumed = 0i32;
        if disease_idx != 0xff {
            let count = updated.disease_count.get(sim_cell as usize);
            consumed = (count as f32 * percent + 0.5) as i32;
            if max_to_consume <= consumed {
                consumed = max_to_consume;
            }
            updated
                .disease_count
                .set(sim_cell as usize, count - consumed);
            if updated.disease_count.get(sim_cell as usize) < 1 {
                crate::c2_physics::liquid_flow::clear_disease(updated, sim_cell as usize);
            }
            disease_idx = updated.disease_idx.get(sim_cell as usize);
        }
        if callback_idx != -1 && !sim.sim_events.ptr.is_null() {
            let events = unsafe { &mut *sim.sim_events.ptr };
            events.disease_consumed_callbacks.push(DiseaseConsumedCallback {
                callback_idx,
                disease_idx,
                pad: [0; 3],
                disease_count: consumed,
            });
        }
    }
    frame.consume_disease.clear_keep_capacity();
}

/// CellDiseaseModification 处理（SimDLL_Source.c L116412-116474，位于 ConsumeDisease 之后）。
///
/// 12B/条：{cellIdx(gameCell) i32, diseaseIdx u8, pad×3, diseaseCount i32}。
/// diseaseIdx==0xff → 直加 count（不改 idx），count<1 清四字段；
/// 否则 Disease::AddDiseaseToCell（阶段 1 强度混合）。
pub(crate) fn process_cell_disease_modifications(frame: &mut SimFrameInfo, sim: &mut SimData) {
    let width = sim.width as i32;
    let game_w = width - 2;
    let bytes = frame.cell_disease_modifications.as_slice();
    let mut off = 0usize;
    while off + 12 <= bytes.len() {
        let cell_idx = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let disease_idx = bytes[off + 4];
        let disease_count = i32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
        off += 12;
        let sim_cell = (cell_idx / game_w + 1) * width + cell_idx % game_w + 1;
        if sim.updated_cells.ptr.is_null() {
            continue;
        }
        let updated = unsafe { &mut *sim.updated_cells.ptr };
        if (sim_cell as usize) >= updated.disease_count.len() {
            continue;
        }
        if disease_idx == 0xff {
            updated
                .disease_count
                .set(sim_cell as usize, updated.disease_count.get(sim_cell as usize) + disease_count);
            if updated.disease_count.get(sim_cell as usize) < 1 {
                crate::c2_physics::liquid_flow::clear_disease(updated, sim_cell as usize);
            }
        } else {
            crate::c2_physics::liquid_flow::add_disease_to_cell(
                updated,
                sim_cell as usize,
                disease_idx,
                disease_count,
            );
        }
    }
    frame.cell_disease_modifications.clear_keep_capacity();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
    use crate::a_framework::sim_data::SimData;
    use crate::a_framework::sim_frame_manager::SimFrameInfo;
    use crate::b_elements::disease::{Disease, DiseaseInfo};
    use crate::b_elements::element::Element;
    use crate::b_elements::elements_table::{CreateElementsTable, DestroyElementsTable};
    use crate::LIB_TESTS_LOCK;

    /// 1 固体元素（state=3），温度范围覆盖 300K（避免 DoStateTransition）。
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

    /// 1 病菌表（idx0 strength=1.0）挂 G_DISEASE（add_disease_to_cell 混合需要）。
    fn install_disease_table() {
        let mut t = Disease::new();
        t.diseases.push_unchecked(DiseaseInfo {
            hash_id: 0xD3,
            strength: 1.0,
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

    fn make_sim() -> SimData {
        let mut sd = SimData::new_for_allocate(6, 5, 1, false, false);
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
                    c.disease_infestation_tick_count.set(i, 0);
                }
            }
        }
        sd
    }

    fn push_u8(frame: &mut SimFrameInfo, v: u8) {
        frame.consume_disease.push_unchecked(v);
    }

    #[test]
    fn get_reachable_cells_depth_gate_and_bounds() {
        let _lock = LIB_TESTS_LOCK.lock();
        let sd = make_sim();
        // 起点 (2,2)=cell14；range=2 → 14 + 上8 + 左13 + 右15 + 下20
        let r = get_reachable_cells(&sd, 2, 2, 2);
        assert_eq!(r, vec![14usize, 8, 13, 15, 20]);
        // range=1 → 仅中心
        let r1 = get_reachable_cells(&sd, 2, 2, 1);
        assert_eq!(r1, vec![14usize]);
        // 边界：起点 (1,1)=cell7，range=3 → x==0/y==0 格被排除
        let r3 = get_reachable_cells(&sd, 1, 1, 3);
        assert!(!r3.contains(&1usize), "x==0 排除");
        assert!(!r3.contains(&6usize), "y==0 排除");
    }

    /// visited 世代戳池的隔离性：同一参数**连续多次调用**必须结果完全一致。
    /// 若世代戳失效（残留世代被误判为"已访问"），第 2/3 次调用会返回空/缺格。
    #[test]
    fn get_reachable_cells_repeated_calls_are_isolated() {
        let _lock = LIB_TESTS_LOCK.lock();
        let sd = make_sim();
        let expected = vec![14usize, 8, 13, 15, 20];
        for i in 0..3 {
            let r = get_reachable_cells(&sd, 2, 2, 2);
            assert_eq!(r, expected, "第 {} 次调用应与首次完全一致（世代戳隔离）", i + 1);
        }
        // 交叉参数再验证：不同起点/范围混用后回到首参数，结果仍一致
        let _ = get_reachable_cells(&sd, 1, 1, 3);
        let _ = get_reachable_cells(&sd, 3, 3, 1);
        let r = get_reachable_cells(&sd, 2, 2, 2);
        assert_eq!(r, expected, "交叉调用后结果仍须一致");
    }

    #[test]
    fn consume_disease_rounds_and_clamps() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        // gameCell=0 → simCell=7（width=6, game_w=4）
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.disease_idx.set(7, 0);
            u.disease_count.set(7, 100);
        }
        let mut frame = SimFrameInfo::default();
        // 消息 1：percent=0.25 → consumed=(int)(100×0.25+0.5)=25 → count75 + callback{7,0,25}
        for b in 0i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 7i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 0.25f32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 1000i32.to_le_bytes() { push_u8(&mut frame, b); }
        // 消息 2：percent=0.5 → 50，max=30 → consumed=30 → count45
        for b in 0i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 8i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 0.5f32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 30i32.to_le_bytes() { push_u8(&mut frame, b); }
        process_consume_disease(&mut frame, &mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 45, "25+30=55 消费 → 45");
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.disease_consumed_callbacks.len(), 2);
            let a = events.disease_consumed_callbacks.as_slice();
            assert_eq!(a[0].callback_idx, 7);
            assert_eq!(a[0].disease_idx, 0);
            assert_eq!(a[0].disease_count, 25);
            assert_eq!(a[1].callback_idx, 8);
            assert_eq!(a[1].disease_count, 30);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn consume_disease_clears_and_reports_zero() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        unsafe {
            let u = &mut *sd.updated_cells.ptr;
            u.disease_idx.set(7, 0);
            u.disease_count.set(7, 5);
            u.disease_infestation_tick_count.set(7, 9);
            u.disease_growth_accumulated_error.set(7, 1.5);
        }
        let mut frame = SimFrameInfo::default();
        for b in 0i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 42i32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 1.0f32.to_le_bytes() { push_u8(&mut frame, b); }
        for b in 1000i32.to_le_bytes() { push_u8(&mut frame, b); }
        process_consume_disease(&mut frame, &mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 0);
            assert_eq!(u.disease_idx.get(7), 0xff, "count<1 → 清病菌");
            assert_eq!(u.disease_infestation_tick_count.get(7), 0);
            assert_eq!(u.disease_growth_accumulated_error.get(7), 0.0);
            let events = &*sd.sim_events.ptr;
            let a = events.disease_consumed_callbacks.as_slice();
            assert_eq!(a[0].callback_idx, 42);
            assert_eq!(a[0].disease_idx, 0xff, "清除后回调 diseaseIdx=0xff");
            assert_eq!(a[0].disease_count, 5);
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn cell_disease_modification_direct_and_mixed() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        let mut frame = SimFrameInfo::default();
        // {cellIdx=0, diseaseIdx=0xff, count=+10} → cell7 count=10（idx 保持 0xff，原版只加 count）
        for b in 0i32.to_le_bytes() { frame.cell_disease_modifications.push_unchecked(b); }
        frame.cell_disease_modifications.push_unchecked(0xff);
        frame.cell_disease_modifications.push_unchecked(0);
        frame.cell_disease_modifications.push_unchecked(0);
        frame.cell_disease_modifications.push_unchecked(0);
        for b in 10i32.to_le_bytes() { frame.cell_disease_modifications.push_unchecked(b); }
        // {cellIdx=0, diseaseIdx=3, count=5} → AddDiseaseToCell：cur=0xff → 替换 → idx3 count5
        for b in 0i32.to_le_bytes() { frame.cell_disease_modifications.push_unchecked(b); }
        frame.cell_disease_modifications.push_unchecked(3);
        frame.cell_disease_modifications.push_unchecked(0);
        frame.cell_disease_modifications.push_unchecked(0);
        frame.cell_disease_modifications.push_unchecked(0);
        for b in 5i32.to_le_bytes() { frame.cell_disease_modifications.push_unchecked(b); }
        process_cell_disease_modifications(&mut frame, &mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_idx.get(7), 3, "非 0xff → AddDiseaseToCell");
            assert_eq!(u.disease_count.get(7), 5, "替换（cur=0xff）");
        }
        // 消息 1 的直加语义：先加 count=10（idx 保持 0xff），再被消息 2 替换 → 覆盖验证
        // （用第二个独立用例直接验证直加，见下）
        let mut sd2 = make_sim();
        let mut frame2 = SimFrameInfo::default();
        for b in 0i32.to_le_bytes() { frame2.cell_disease_modifications.push_unchecked(b); }
        frame2.cell_disease_modifications.push_unchecked(0xff);
        frame2.cell_disease_modifications.push_unchecked(0);
        frame2.cell_disease_modifications.push_unchecked(0);
        frame2.cell_disease_modifications.push_unchecked(0);
        for b in 10i32.to_le_bytes() { frame2.cell_disease_modifications.push_unchecked(b); }
        process_cell_disease_modifications(&mut frame2, &mut sd2);
        unsafe {
            let u = &*sd2.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 10, "idx=0xff 直加 count");
            assert_eq!(u.disease_idx.get(7), 0xff, "直加路径不改 idx（原版语义）");
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    // ===== 任务 3：DiseaseEmitter 组件 =====

    fn push_add_emitter(frame: &mut SimFrameInfo, callback_idx: i32) {
        for b in callback_idx.to_le_bytes() {
            frame.disease_emitter_messages.adds.push_unchecked(b);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_modify_emitter(
        frame: &mut SimFrameInfo,
        handle: i32,
        game_cell: i32,
        disease_idx: u8,
        max_depth: u8,
        emit_interval: f32,
        emit_count: i32,
    ) {
        for b in handle.to_le_bytes() {
            frame.disease_emitter_messages.modifies.push_unchecked(b);
        }
        for b in game_cell.to_le_bytes() {
            frame.disease_emitter_messages.modifies.push_unchecked(b);
        }
        frame.disease_emitter_messages.modifies.push_unchecked(disease_idx);
        frame.disease_emitter_messages.modifies.push_unchecked(max_depth);
        frame.disease_emitter_messages.modifies.push_unchecked(0);
        frame.disease_emitter_messages.modifies.push_unchecked(0);
        for b in emit_interval.to_le_bytes() {
            frame.disease_emitter_messages.modifies.push_unchecked(b);
        }
        for b in emit_count.to_le_bytes() {
            frame.disease_emitter_messages.modifies.push_unchecked(b);
        }
    }

    fn push_remove_emitter(frame: &mut SimFrameInfo, handle: i32, callback_idx: i32) {
        for b in handle.to_le_bytes() {
            frame.disease_emitter_messages.removes.push_unchecked(b);
        }
        for b in callback_idx.to_le_bytes() {
            frame.disease_emitter_messages.removes.push_unchecked(b);
        }
    }

    #[test]
    fn disease_emitter_register_modify_remove() {
        let _lock = LIB_TESTS_LOCK.lock();
        clear_disease_emitters();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        let mut frame = SimFrameInfo::default();
        // Register(callback=5) → handle + componentStateChanged{5, handle}
        push_add_emitter(&mut frame, 5);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.adds.clear_keep_capacity();
        assert_eq!(disease_emitter_count(), 1);
        let handle = {
            let events = unsafe { &*sd.sim_events.ptr };
            assert_eq!(events.component_state_changed_messages.len(), 1);
            events.component_state_changed_messages.as_slice()[0].sim_handle
        };
        // 默认记录：diseaseIdx=0xff、range=0、emitCount=0、interval=0
        let rec = disease_emitter_record(handle).expect("record exists");
        assert_eq!(rec.disease_idx, 0xff);

        // Modify(handle, gameCell=0 → simCell=7, idx=0, range=1, interval=1.0, count=10)
        push_modify_emitter(&mut frame, handle, 0, 0, 1, 1.0, 10);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.modifies.clear_keep_capacity();
        let rec = disease_emitter_record(handle).expect("record exists");
        assert_eq!(rec.cell, 7, "gameCell=0 → simCell=7（width=6）");
        assert_eq!(rec.disease_idx, 0);
        assert_eq!(rec.range, 1);
        assert_eq!(rec.emit_interval, 1.0);
        assert_eq!(rec.emit_count, 10);

        // Remove(handle, callback=-1) → 释放 + 版本+1，无回调
        push_remove_emitter(&mut frame, handle, -1);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.removes.clear_keep_capacity();
        assert_eq!(disease_emitter_count(), 0);
        assert!(disease_emitter_record(handle).is_none(), "释放后句柄失效");
        unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.component_state_changed_messages.len(), 1, "无回调");
        }
        clear_disease_emitters();
        uninstall_disease_table();
        DestroyElementsTable();
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

    #[test]
    fn disease_emitter_update_emits_on_interval() {
        let _lock = LIB_TESTS_LOCK.lock();
        clear_disease_emitters();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        set_cell_disease(&mut sd, 7, 0, 100);
        let mut frame = SimFrameInfo::default();
        push_add_emitter(&mut frame, -1);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.adds.clear_keep_capacity();
        let handle = disease_emitter_handle(0).expect("handle 0");
        push_modify_emitter(&mut frame, handle, 0, 0, 1, 1.0, 10);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.modifies.clear_keep_capacity();
        let bounds = crate::d1_activity::RegionBounds {
    min_x: 1,
    min_y: 1,
    max_x: 2,
    max_y: 2,
        };
        // interval=1.0：elapsedTime 0.2×5=1.0；第 6 次调用时 1.0<=1.0 触发 → cell7 +10
        for _ in 0..5 {
            update_disease_emitters(&mut sd, 0.2, bounds);
        }
        unsafe {
            assert_eq!((*sd.updated_cells.ptr).disease_count.get(7), 100, "未到 interval 不发射");
        }
        update_disease_emitters(&mut sd, 0.2, bounds);
        unsafe {
            assert_eq!((*sd.updated_cells.ptr).disease_count.get(7), 110, "range=1 仅中心 +10");
        }
        let (idx, cnt) = disease_emitter_emitted(0).expect("emitted info");
        assert_eq!(idx, 0);
        assert_eq!(cnt, 10);
        clear_disease_emitters();
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn disease_emitter_negative_count_only_same_disease() {
        let _lock = LIB_TESTS_LOCK.lock();
        clear_disease_emitters();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        set_cell_disease(&mut sd, 7, 0, 100);
        set_cell_disease(&mut sd, 8, 0, 100);
        set_cell_disease(&mut sd, 13, 1, 100);
        let mut frame = SimFrameInfo::default();
        push_add_emitter(&mut frame, -1);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.adds.clear_keep_capacity();
        let handle = disease_emitter_handle(0).expect("handle 0");
        push_modify_emitter(&mut frame, handle, 0, 0, 2, 0.2, -5);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.modifies.clear_keep_capacity();
        let bounds = crate::d1_activity::RegionBounds {
    min_x: 1,
    min_y: 1,
    max_x: 4,
    max_y: 4,
        };
        // interval=0.2：第 1 次 elapsed=0 不触发；第 2 次 elapsed=0.2 触发
        update_disease_emitters(&mut sd, 0.2, bounds);
        update_disease_emitters(&mut sd, 0.2, bounds);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 95, "同病菌格 −5");
            assert_eq!(u.disease_count.get(8), 95);
            assert_eq!(u.disease_count.get(13), 100, "异病菌格不变（emitCount<0 仅同病菌）");
        }
        clear_disease_emitters();
        uninstall_disease_table();
        DestroyElementsTable();
    }

    #[test]
    fn disease_emitter_region_gate_skips_outside() {
        let _lock = LIB_TESTS_LOCK.lock();
        clear_disease_emitters();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        set_cell_disease(&mut sd, 7, 0, 100);
        let mut frame = SimFrameInfo::default();
        push_add_emitter(&mut frame, -1);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.adds.clear_keep_capacity();
        let handle = disease_emitter_handle(0).expect("handle 0");
        push_modify_emitter(&mut frame, handle, 0, 0, 1, 0.2, 10);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.modifies.clear_keep_capacity();
        // bounds 不含 cell7（(1,1)）→ 不发射
        let bounds = crate::d1_activity::RegionBounds {
    min_x: 2,
    min_y: 2,
    max_x: 3,
    max_y: 3,
        };
        update_disease_emitters(&mut sd, 0.2, bounds);
        unsafe {
            assert_eq!((*sd.updated_cells.ptr).disease_count.get(7), 100, "区域外不发射");
        }
        clear_disease_emitters();
        uninstall_disease_table();
        DestroyElementsTable();
    }

    // ===== 任务 4：DiseaseConsumer 防御 + update_data 集成 =====

    #[test]
    fn disease_consumer_messages_register_and_remove() {
        let _lock = LIB_TESTS_LOCK.lock();
        clear_disease_consumers();
        create_solid_table();
        install_disease_table();
        let mut sd = make_sim();
        let mut frame = SimFrameInfo::default();
        // Add 12B：{gameCell=0, callbackIdx=9, ...4B} → 注册零值记录 + 回调{9, handle}
        for b in 0i32.to_le_bytes() {
            frame.disease_consumer_messages.adds.push_unchecked(b);
        }
        for b in 9i32.to_le_bytes() {
            frame.disease_consumer_messages.adds.push_unchecked(b);
        }
        for _ in 0..4 {
            frame.disease_consumer_messages.adds.push_unchecked(0);
        }
        process_disease_consumer_messages(&mut frame, &mut sd);
        frame.disease_consumer_messages.adds.clear_keep_capacity();
        assert_eq!(disease_consumer_count(), 1);
        let handle = unsafe {
            let events = &*sd.sim_events.ptr;
            assert_eq!(events.component_state_changed_messages.len(), 1);
            events.component_state_changed_messages.as_slice()[0].sim_handle
        };
        // Remove 8B {handle, callback=-1} → 释放
        for b in handle.to_le_bytes() {
            frame.disease_consumer_messages.removes.push_unchecked(b);
        }
        for b in (-1i32).to_le_bytes() {
            frame.disease_consumer_messages.removes.push_unchecked(b);
        }
        process_disease_consumer_messages(&mut frame, &mut sd);
        assert_eq!(disease_consumer_count(), 0);
        clear_disease_consumers();
        uninstall_disease_table();
        DestroyElementsTable();
    }

    /// 集成用病菌表：生长恒 0（popHL/overPopHL=INF、underPopDeath=0、min=0、max=INF、
    /// tempHL 全 INF → 温度因子 1.0）；minDiffusionCount 巨大 → 扩散门控 0。
    fn install_integration_table() {
        let mut t = Disease::new();
        let mut eg = crate::b_elements::disease::ElemGrowthInfo::default();
        eg.diffusion_scale.resize(1, 1.0);
        eg.min_diffusion_count.resize(1, 1_000_000);
        eg.min_diffusion_infestation_tick_count.resize(1, 0);
        eg.min_count_per_kg.resize(1, 0.0);
        eg.max_count_per_kg.resize(1, f32::INFINITY);
        eg.population_half_life.resize(1, f32::INFINITY);
        eg.over_population_half_life.resize(1, f32::INFINITY);
        eg.under_population_death_rate.resize(1, 0.0);
        t.diseases.push_unchecked(crate::b_elements::disease::DiseaseInfo {
            hash_id: 0xE3,
            strength: 1.0,
            temperature_range: crate::b_elements::disease::RangeInfo {
                min_viable: -f32::INFINITY,
                min_growth: -f32::INFINITY,
                max_growth: f32::INFINITY,
                max_viable: f32::INFINITY,
            },
            temperature_half_lives: crate::b_elements::disease::RangeInfo {
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
    fn update_data_emits_disease_via_emitter() {
        let _lock = LIB_TESTS_LOCK.lock();
        clear_disease_emitters();
        create_solid_table();
        install_integration_table();
        let mut sd = make_sim();
        set_cell_disease(&mut sd, 7, 0, 100);
        sd.active_regions.push(crate::a_framework::sim_data::ActiveRegion {
            min_x: 1,
            min_y: 1,
            max_x: 2,
            max_y: 2,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        });
        // 注册发射器 gameCell=0 → simCell7；idx=0, range=1, interval=0.0（每次子步触发）, count=10
        let mut frame = SimFrameInfo::default();
        push_add_emitter(&mut frame, -1);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.adds.clear_keep_capacity();
        let handle = disease_emitter_handle(0).expect("handle 0");
        push_modify_emitter(&mut frame, handle, 0, 0, 1, 0.0, 10);
        process_disease_emitter_messages(&mut frame, &mut sd);
        frame.disease_emitter_messages.modifies.clear_keep_capacity();
        crate::c2_physics::update_data(&mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert_eq!(u.disease_count.get(7), 110, "update_data 经发射器 +10");
            assert_eq!(u.disease_idx.get(7), 0);
            assert_eq!(u.disease_infestation_tick_count.get(7), 1, "生长第二遍 tick+1");
        }
        clear_disease_emitters();
        uninstall_disease_table();
        DestroyElementsTable();
    }

    // ===== 2026-08-06：孢子兰病菌流动环境丢失诊断 =====
    // 结论：病菌被气流携带（原版 UpdatePressure/DoGasPressureDisplacement 语义），
    // 非生长公式杀死。以下辅助表 + 回归测试验证真实 ZombieSpores 数据解析与存活。

    /// 5 元素表：0=真空、1=氧气(气体,flow=1.0)、2=CO2(气体,flow=1.0)、3=固体、4=液体。
    fn create_gas_table() {
        use crate::b_elements::element::{
            ElementLiquidData, ElementPostProcessData, ElementPressureData, ElementStateData,
            ElementTemperatureData,
        };
        let mut table =
            crate::b_elements::elements_table::G_ELEMENTS_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        table.elements.clear();
        table.state_data.clear();
        table.liquid_data.clear();
        table.pressure_data.clear();
        table.post_process_data.clear();
        table.temperature_data.clear();
        table.radiation_data.clear();
        table.property_texture_data.clear();
        for (id, state, flow) in [
            (0i32, 0u8, 1.0f32),
            (1, 1, 1.0),
            (2, 1, 1.0),
            (3, 3, 0.0),
            (4, 2, 1.0),
        ] {
            let mut e = Element::default();
            e.id = id;
            e.state = state;
            e.low_temp = -273.0;
            e.high_temp = 1000.0;
            table.elements.push(e);
            table.state_data.push(ElementStateData { state });
            table.liquid_data.push(ElementLiquidData { state, ..Default::default() });
            table.pressure_data.push(ElementPressureData { state, flow });
            table.post_process_data.push(ElementPostProcessData { state, ..Default::default() });
            // 2026-08-06：温度数据必须给真实范围（low=-273/high=2000），
            // 否则温度任务在 300K 触发"高温相变"（high_temp=0 → 阈值 3K），
            // 把 CO2 变成元素 0，导致后续气体压力不流动（此前诊断假象）。
            table.temperature_data.push(ElementTemperatureData {
                state,
                low_temp: -273.0,
                high_temp: 2000.0,
                thermal_conductivity: 0.1,
                specific_heat_capacity: 1.0,
                ..Default::default()
            });
            table.radiation_data.push(Default::default());
            table.property_texture_data.push(Default::default());
        }
        table.element_indices.insert(0, 0);
        table.element_indices.insert(1, 1);
        table.element_indices.insert(2, 2);
        table.element_indices.insert(3, 3);
        table.element_indices.insert(4, 4);
    }

    /// ZombieSpores 式病菌表：idx0（ZombieSpores），CO2 生长恒 0、minDiffusionCount=5100。
    fn install_zombie_table() {
        use crate::b_elements::disease::{Disease, DiseaseInfo, ElemGrowthInfo, RangeInfo};
        let mut t = Disease::new();
        let mut eg = ElemGrowthInfo::default();
        eg.diffusion_scale.resize(4, 0.005);
        eg.min_diffusion_count.resize(4, 5100);
        eg.min_diffusion_infestation_tick_count.resize(4, 1);
        eg.min_count_per_kg.resize(4, 250.0);
        eg.max_count_per_kg.resize(4, 10000.0);
        eg.population_half_life.resize(4, f32::INFINITY);
        eg.over_population_half_life.resize(4, 6000.0);
        eg.under_population_death_rate.resize(4, 0.0);
        t.diseases.push_unchecked(DiseaseInfo {
            hash_id: 0xd49f77d6,
            strength: 50.0,
            temperature_range: RangeInfo {
                min_viable: 168.15,
                min_growth: 258.15,
                max_growth: 513.15,
                max_viable: 563.15,
            },
            temperature_half_lives: RangeInfo {
                min_viable: 10.0,
                min_growth: 1200.0,
                max_growth: 1200.0,
                max_viable: 10.0,
            },
            elem_growth_info: eg,
            radiation_kill_rate: 1.0,
            ..Default::default()
        });
        let mut g = crate::globals::G_DISEASE.lock();
        if !g.0.is_null() {
            unsafe { let _ = Box::from_raw(g.0); }
        }
        g.0 = Box::into_raw(Box::new(t));
    }

    /// 2026-08-06 诊断：真实 ZombieSpores 数据（C# ZombieSpores.cs CO2 规则）下，
    /// 2kg CO2 格 + 1000 病菌经过一个 update_data 子步是否存活（区分"生长公式 bug"
    /// 与"真实病菌表数据解析错误"）。
    #[test]
    fn zombie_spores_in_co2_survive_one_substep() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_gas_table();
        install_zombie_table();
        let mut sd = SimData::new_for_allocate(8, 8, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let c = &mut *buf_ptr;
                for i in 0..64usize {
                    c.element_idx.set(i, 0);
                    c.mass.set(i, 0.0);
                    c.temperature.set(i, 300.0);
                    c.disease_idx.set(i, 0xff);
                    c.disease_count.set(i, 0);
                    c.disease_infestation_tick_count.set(i, 0);
                    let x = i % 8;
                    let y = i / 8;
                    if x == 0 || x == 7 || y == 0 || y == 7 {
                        c.element_idx.set(i, 3); // 边界固体
                        c.mass.set(i, 1000.0);
                    }
                }
            }
            // 中央 2kg CO2 + 1000 病菌
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let c = &mut *buf_ptr;
                c.element_idx.set(36, 2);
                c.mass.set(36, 2.0);
                c.temperature.set(36, 300.0);
                c.disease_idx.set(36, 0);
                c.disease_count.set(36, 1000);
            }
        }
        sd.active_regions.push(crate::a_framework::sim_data::ActiveRegion {
            min_x: 2,
            min_y: 2,
            max_x: 6,
            max_y: 6,
            current_sunlight_intensity: 0.0,
            current_cosmic_radiation_intensity: 0.0,
        });
        crate::c2_physics::update_data(&mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            assert!(
                u.disease_count.get(36) > 0,
                "正确数据下 ZombieSpores 在 CO2 应存活（count={}）",
                u.disease_count.get(36)
            );
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

    /// 2026-08-06 回归：拔除孢子兰 → +100000 病菌注入 2kg CO2 格。
    /// 完整 update_data 一子步后：生长过密死亡仅 -2、扩散门控 tick=0 未激活、
    /// 无干净质量补充 → 病菌应基本保留（原版行为，用户实测原版保留在一格）。
    #[test]
    fn zombie_spores_100k_burst_survives_update_data() {
        let _lock = LIB_TESTS_LOCK.lock();
        create_gas_table();
        install_zombie_table();
        let w = 16usize;
        let flower = 3 * w + 6;
        let mut sd = SimData::new_for_allocate(16, 8, 1, false, false);
        sd.vacuum_element_idx = 0;
        sd.void_element_idx = 0xFFFF;
        unsafe {
            for buf_ptr in [sd.cells.ptr, sd.updated_cells.ptr] {
                let c = &mut *buf_ptr;
                for i in 0..(16 * 8usize) {
                    let x = i % w;
                    let y = i / w;
                    if x == 0 || x == 15 || y == 0 || y == 7 {
                        c.element_idx.set(i, 3);
                        c.mass.set(i, 1000.0);
                        c.temperature.set(i, 300.0);
                    } else {
                        c.element_idx.set(i, 2); // CO2
                        c.mass.set(i, 2.0);
                        c.temperature.set(i, 300.0);
                    }
                    c.disease_idx.set(i, 0xff);
                    c.disease_count.set(i, 0);
                    c.disease_infestation_tick_count.set(i, 0);
                }
                c.disease_idx.set(flower, 0);
                c.disease_count.set(flower, 100000);
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
        crate::c2_physics::update_data(&mut sd);
        unsafe {
            let u = &*sd.updated_cells.ptr;
            let flower_count = u.disease_count.get(flower);
            assert!(
                flower_count >= 99000,
                "拔除爆发 100000 病菌一子步后应基本保留（原版），got {}",
                flower_count
            );
        }
        uninstall_disease_table();
        DestroyElementsTable();
    }

}
