//! FrameSync — 游戏帧同步数据（双缓冲 + 同步原语占位）。
//!
//! 字段对照源码 00_types_reference.c L5696-5706。
//! A2 仅定义 struct + new_zeroed + 桩方法。
//! A3 添加 Rust 同步原语（rust_sync_state/rust_game_cond/rust_sim_cond）+ 真实 game_sync/sim_sync。
//!
//! **双缓冲模式**（源码 FrameSync::GameSync L615-617）：
//! - sim 线程写入 m_sim_data，game 线程读取 m_game_data
//! - GameSync 时两者交换：m_sim_data ↔ m_game_data
//! - m_sim_ready_to_swap 由 sim 线程置 true，由 GameSync 置 false

use crate::a_framework::game_data::GameData;
use crate::a_framework::sim_data::CellSOA;
use crate::a_framework::stl_shim::{MsvcCondVar, MsvcMutex, UniquePtr};

/// FrameSync — 游戏帧同步数据。
///
/// A2 版本：仅 C-ABI 兼容字段（offset 0-199B）。
/// A3 在末尾追加 Rust 同步原语字段（rust_sync_state/rust_game_cond/rust_sim_cond）。
#[repr(C)]
pub struct FrameSync {
    /// offset 0, 8B —— sim 线程私有的 GameData 副本
    pub m_sim_data: UniquePtr<GameData>,
    /// offset 8, 8B —— game 线程读取的权威 GameData
    pub m_game_data: UniquePtr<GameData>,
    /// offset 16, 1B —— sim 已就绪可交换
    pub m_sim_ready_to_swap: bool,
    /// offset 17, 1B —— FrameSync 已初始化
    pub m_initialized: bool,
    /// offset 20, 4B —— 源码 _padding_ 字段
    pub _padding: i32,
    /// offset 24, 80B —— 主锁（占位）
    pub m_mutex: MsvcMutex,
    /// offset 104, 8B —— game 线程等待条件（占位）
    pub m_game_cond: MsvcCondVar,
    /// offset 112, 8B —— sim 线程等待条件（占位）
    pub m_sim_cond: MsvcCondVar,
    /// offset 120, 80B —— sim 数据锁（占位）
    pub m_sim_mutex: MsvcMutex,
}

impl FrameSync {
    /// 构造零初始化的 FrameSync。
    pub fn new_zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }

    /// initGameData — 初始化 m_sim_data 和 m_game_data。
    /// 对照源码 09_crashdump_logger.c L716-975（FrameSync::initGameData）。
    ///
    /// 参数（对照源码 L718 + Start L245-248 传入值）：
    /// - `game_width` = sim_data.width - 2（不含边界的游戏宽度）
    /// - `game_height` = sim_data.height - 2（不含边界的游戏高度）
    /// - `full_width` = sim_data.width（含边界）
    /// - `full_height` = sim_data.height（含边界）
    /// - `source_cells` = sim_data.updated_cells.ptr（加载了存档数据的 CellSOA）
    ///
    /// 为 m_sim_data 和 m_game_data 各分配一个 GameData，然后将 source_cells 的内部 cell
    /// （去掉边界）逐行拷贝到 GameData 的 CellSOA 中。
    pub fn init_game_data(
        &mut self,
        game_width: i32,
        game_height: i32,
        full_width: i32,
        _full_height: i32,
        source_cells: &CellSOA,
    ) {
        // 对照源码 09_crashdump_logger.c L757-773：已初始化仅调试器断点告警，
        // **照常重建**（重新分配 m_sim_data/m_game_data 并拷贝）。
        // 此前 early-return 导致二次加载/世界生成时缓冲停留在旧尺寸，
        // 新世界按新尺寸写入旧缓冲 → 越界写（0xc0000374 世界生成崩溃根因之一）。
        if self.m_initialized {
            tracing::warn!("FrameSync::initGameData: already initialized, rebuilding");
        }

        let total_game_cells = (game_width as usize) * (game_height as usize);

        // 为 m_sim_data 和 m_game_data 各分配 GameData
        // 对照源码 L777-783: operator_new(0x400) + GameData::GameData(_, game_width, game_height)
        let slots: [&mut UniquePtr<GameData>; 2] = [&mut self.m_sim_data, &mut self.m_game_data];
        for slot in slots {
            // 释放旧的
            if !slot.ptr.is_null() {
                unsafe { let _ = Box::from_raw(slot.ptr); }
                slot.ptr = std::ptr::null_mut();
            }
            // 分配新的
            let gd = Box::new(GameData::new(game_width, game_height));
            slot.ptr = Box::into_raw(gd);
        }

        // 逐行拷贝内部 cell（去掉边界）到 m_sim_data 和 m_game_data 的 CellSOA
        // 对照源码 L791-945 的嵌套循环
        // 源码：source_idx = full_width + 1 + row * full_width（跳过第一行和第一列边界）
        //       dest_idx = row * game_width
        for game_data_ptr in [self.m_sim_data.ptr, self.m_game_data.ptr] {
            if game_data_ptr.is_null() { continue; }
            let game_data = unsafe { &mut *game_data_ptr };
            let dest_cells_ptr = game_data.cells.ptr;
            if dest_cells_ptr.is_null() { continue; }
            let dest_cells = unsafe { &mut *dest_cells_ptr };

            for row in 0..game_height as usize {
                let src_start = (full_width as usize + 1) + row * (full_width as usize);
                let dst_start = row * (game_width as usize);

                // 拷贝一行（game_width 个 cell）的每个字段
                for col in 0..game_width as usize {
                    let src_idx = src_start + col;
                    let dst_idx = dst_start + col;

                    if src_idx < source_cells.element_idx.len()
                        && dst_idx < dest_cells.element_idx.len()
                    {
                        dest_cells.element_idx.set(dst_idx, source_cells.element_idx.get(src_idx));
                        dest_cells.temperature.set(dst_idx, source_cells.temperature.get(src_idx));
                        dest_cells.mass.set(dst_idx, source_cells.mass.get(src_idx));
                        dest_cells.properties.set(dst_idx, source_cells.properties.get(src_idx));
                        dest_cells.insulation.set(dst_idx, source_cells.insulation.get(src_idx));
                        dest_cells.strength_info.set(dst_idx, source_cells.strength_info.get(src_idx));
                        dest_cells.disease_idx.set(dst_idx, source_cells.disease_idx.get(src_idx));
                        dest_cells.disease_count.set(dst_idx, source_cells.disease_count.get(src_idx));
                        dest_cells.disease_infestation_tick_count.set(
                            dst_idx,
                            source_cells.disease_infestation_tick_count.get(src_idx),
                        );
                        dest_cells.disease_growth_accumulated_error.set(
                            dst_idx,
                            source_cells.disease_growth_accumulated_error.get(src_idx),
                        );
                        dest_cells.radiation.set(dst_idx, source_cells.radiation.get(src_idx));
                    }
                }
            }
        }

        let _ = total_game_cells; // 调试用
        self.m_initialized = true;
        tracing::info!(game_width, game_height, "FrameSync::initGameData done");
    }

    /// game_sync — 游戏线程消费 sim 数据并通知 sim 线程。
    ///
    /// 对照源码 09_crashdump_logger.c L562-651。
    ///
    /// **双缓冲一帧延迟机制**（项目 memory 约束）：
    /// sim 线程写入 m_sim_data，game 线程读取 m_game_data，每帧 game_sync 交换。
    /// 绝不改为直接同步。
    ///
    /// 流程：
    /// 1. lock m_mutex（握手锁）
    /// 2. while (!m_sim_ready_to_swap) wait m_game_cond（等待 sim 数据就绪）
    /// 3. lock m_sim_mutex（数据锁，保护 m_sim_data/m_game_data 交换）
    /// 4. swap(m_sim_data, m_game_data)
    /// 5. m_sim_ready_to_swap = false
    /// 6. unlock m_sim_mutex
    /// 7. unlock m_mutex
    /// 8. signal m_sim_cond（唤醒 SimSync 中等待的 sim 线程）
    ///
    /// **注意**：此函数通过裸指针访问，不持有 G_FRAME_SYNC 的外层 Mutex。
    /// FrameSync 地址在 OnceCell static 中固定，内部字段由 m_mutex/m_sim_mutex 保护。
    pub unsafe fn game_sync(fs: *mut FrameSync) {
        let fs = &mut *fs;

        // 1. lock m_mutex
        fs.m_mutex.lock_raw();

        // 2. while (!m_sim_ready_to_swap) wait m_game_cond
        // 对照源码 L601-606
        while !fs.m_sim_ready_to_swap {
            if !fs.m_game_cond.wait(&fs.m_mutex) {
                tracing::warn!("game_sync: wait failed");
                break;
            }
        }

        // 3. lock m_sim_mutex（保护数据交换）
        fs.m_sim_mutex.lock_raw();

        // 4. swap(m_sim_data, m_game_data)
        // 对照源码 L635-637
        std::mem::swap(&mut fs.m_sim_data, &mut fs.m_game_data);

        // 5. m_sim_ready_to_swap = false
        // 对照源码 L639
        fs.m_sim_ready_to_swap = false;

        // 6. unlock m_sim_mutex
        fs.m_sim_mutex.unlock_raw();

        // 7. unlock m_mutex
        fs.m_mutex.unlock_raw();

        // 8. signal m_sim_cond（唤醒 SimSync 中等待的 sim 线程）
        // 对照源码 L649
        fs.m_sim_cond.signal();
    }

    /// sim_sync — sim 线程处理完帧后通知游戏线程并等待消费。
    ///
    /// 对照源码 09_crashdump_logger.c L655-690。
    ///
    /// **必须在 Sim::Main 中 mSimMutex 解锁之后调用**，否则会死锁
    /// （GameSync 需要 lock mSimMutex 才能交换数据）。
    ///
    /// 流程：
    /// 1. lock m_mutex（握手锁）
    /// 2. m_sim_ready_to_swap = true（通知游戏线程数据就绪）
    /// 3. signal m_game_cond（唤醒 GameSync 中等待的游戏线程）
    /// 4. while (m_sim_ready_to_swap) wait m_sim_cond（等待游戏线程消费完毕）
    /// 5. unlock m_mutex
    ///
    /// **Rust 侧安全关闭机制**（C1 新增）：
    /// 在 while 循环中额外检查 G_SIM_EXIT_REQUESTED 退出标志。
    /// 当 SIM_Shutdown 调用 join_sim_thread 时：
    /// 1. 设置 G_SIM_EXIT_REQUESTED = true
    /// 2. broadcast m_sim_cond 唤醒阻塞的 sim 线程
    /// 3. sim_sync 检测到退出标志后 break，避免永远阻塞
    pub unsafe fn sim_sync(fs: *mut FrameSync) {
        let fs = &mut *fs;

        // 1. lock m_mutex
        fs.m_mutex.lock_raw();

        // 2. m_sim_ready_to_swap = true
        fs.m_sim_ready_to_swap = true;

        // 3. signal m_game_cond
        fs.m_game_cond.signal();

        // 4. while (m_sim_ready_to_swap) wait m_sim_cond
        // 对照源码 L678-683
        // C1 新增：检查退出标志，避免 shutdown 时死锁
        while fs.m_sim_ready_to_swap {
            // 检查退出标志（Rust 侧安全关闭机制）
            if crate::globals::G_SIM_EXIT_REQUESTED.load(std::sync::atomic::Ordering::Acquire) {
                tracing::debug!("sim_sync: exit requested, breaking wait");
                fs.m_sim_ready_to_swap = false;
                break;
            }
            if !fs.m_sim_cond.wait(&fs.m_mutex) {
                tracing::warn!("sim_sync: wait failed");
                break;
            }
        }

        // 5. unlock m_mutex
        fs.m_mutex.unlock_raw();
    }

    /// 获取 FrameSync 的裸指针（用于 sim_sync/game_sync）。
    ///
    /// FrameSync 在 G_FRAME_SYNC 的 OnceCell static 中分配，地址在程序运行期间固定。
    /// 此函数短暂锁 G_FRAME_SYNC 获取地址后立即释放锁，不影响后续访问。
    pub fn get_ptr() -> *mut FrameSync {
        let mutex = match crate::globals::G_FRAME_SYNC.get() {
            Some(m) => m,
            None => return std::ptr::null_mut(),
        };
        let guard = mutex.lock();
        &*guard as *const FrameSync as *mut FrameSync
    }

    /// clear — 释放 m_sim_data/m_game_data 并重置状态。
    pub fn clear(&mut self) {
        if !self.m_sim_data.ptr.is_null() {
            unsafe { let _ = Box::from_raw(self.m_sim_data.ptr); }
            self.m_sim_data.ptr = std::ptr::null_mut();
        }
        if !self.m_game_data.ptr.is_null() {
            unsafe { let _ = Box::from_raw(self.m_game_data.ptr); }
            self.m_game_data.ptr = std::ptr::null_mut();
        }
        self.m_initialized = false;
        self.m_sim_ready_to_swap = false;
    }
}

// FrameSync 包含 UniquePtr<GameData> 等裸指针字段（对应原版 C++ 的 std::unique_ptr）。
// 全局 G_FRAME_SYNC 受 Mutex 保护，可安全跨线程访问。
unsafe impl Send for FrameSync {}
unsafe impl Sync for FrameSync {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn frame_sync_size_is_200() {
        // A2 版本：仅 C-ABI 字段
        // 2×UniquePtr(16) + 2×bool(2) + i32(4) + MsvcMutex(80) + MsvcCondVar(8) + MsvcCondVar(8) + MsvcMutex(80)
        // = 16 + 2 + 2(padding to 4) + 4 + 80 + 8 + 8 + 80 = 200B
        let actual = size_of::<FrameSync>();
        assert_eq!(actual, 200, "FrameSync size = {}, 期望 200 (A2 C-ABI only)", actual);
    }

    #[test]
    fn frame_sync_new_zeroed() {
        let fs = FrameSync::new_zeroed();
        assert!(!fs.m_sim_ready_to_swap);
        assert!(!fs.m_initialized);
        assert!(fs.m_sim_data.ptr.is_null());
        assert!(fs.m_game_data.ptr.is_null());
    }

    #[test]
    fn game_sync_with_sim_ready_swaps_buffers() {
        // game_sync 需要 m_sim_ready_to_swap = true 才能不阻塞
        let mut fs = FrameSync::new_zeroed();
        fs.m_sim_ready_to_swap = true;
        fs.m_initialized = true;
        // 交换两个 null UniquePtr 不会崩溃
        let fs_ptr = &mut fs as *mut FrameSync;
        unsafe { FrameSync::game_sync(fs_ptr); }
        // 交换后 m_sim_ready_to_swap 应为 false
        assert!(!fs.m_sim_ready_to_swap);
    }

    #[test]
    fn clear_stub_does_not_panic() {
        let mut fs = FrameSync::new_zeroed();
        fs.clear();
    }
}
