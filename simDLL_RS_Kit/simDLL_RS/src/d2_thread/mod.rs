//! D2 线程：ParallelTaskQueue 复刻。
//!
//! 对照原版 SimDLL_Source.c L10786（ParallelTaskQueue）+ L130472（构造）+ L132751
//! （WorkerFunc）+ L144430（UpdateData 温度任务分发）。
//!
//! **原版实况**：`SimBase::InitializeTasks`（L132419）用 `ParallelTaskQueue(queue, 1)`
//! 创建队列——**只有 1 个 worker 线程**。UpdateData 把每个活动区域的温度任务
//! 按 worker 数切行带提交，主 sim 线程等 `mTaskCount==0`。所以原版物理实际是
//! 串行的，无多核并行；队列架构存在，供后续 d3（rayon 每星球分核）扩展 worker 数。
//!
//! **地址稳定性**：worker 通过原始指针访问队列（原版 `this` 指针），因此队列对象
//! 必须位于固定地址——用 `Box::new(...)` 分配后再 `start_workers()`（不要在
//! 返回后 move）。全局 `G_PARALLEL_TASK_QUEUE` 即按此初始化。
//!
//! 原版同步语义（worker 持锁检查 + cond_wait 原子释放，杜绝丢失唤醒）：
//! - 提交：lock → push → mTaskCount++ → signal mTaskCondition；
//! - worker：lock → 空则 wait(mTaskCondition) → pop → unlock → 执行 →
//!   lock → mTaskCount-- → 归零则 unlock + broadcast（完成条件变量）；
//! - wait_all：lock → while mTaskCount!=0 → wait(完成条件变量)。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

/// 任务：`FnOnce` 闭包。温度行带任务捕获 `SendSyncPtr<SimData>` + 区域边界 + 行带范围，
/// 在 worker 内 `unsafe { &mut *ptr }` 访问 sim（1 worker 串行，安全；
/// 多 worker 扩展需保证各带写不同行，且遵循原版原始指针模型）。
type Task = Box<dyn FnOnce() + Send>;

struct QueueState {
    tasks: VecDeque<Task>,
}

pub struct ParallelTaskQueue {
    desired_workers: usize,
    state: Mutex<QueueState>,
    task_cond: Condvar,
    completion_cond: Condvar,
    task_count: AtomicUsize,
    shutting_down: AtomicBool,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl ParallelTaskQueue {
    /// 构造队列（不启动 worker；必须先 Box 固定地址再调 `start_workers`）。
    pub fn new(worker_count: usize) -> Self {
        ParallelTaskQueue {
            desired_workers: worker_count,
            state: Mutex::new(QueueState {
                tasks: VecDeque::new(),
            }),
            task_cond: Condvar::new(),
            completion_cond: Condvar::new(),
            task_count: AtomicUsize::new(0),
            shutting_down: AtomicBool::new(false),
            workers: Mutex::new(Vec::new()),
        }
    }

    /// 在**最终地址**上启动 worker（原版构造时启动；本实现延迟到 Box 之后，
    /// 保证 worker 捕获的 `this` 指针稳定）。
    pub fn start_workers(&self) {
        for _ in 0..self.desired_workers {
            let this_ptr =
                crate::globals::SendSyncPtr(self as *const ParallelTaskQueue as *mut _);
            let worker = std::thread::Builder::new()
                .name("task_queue_worker".to_string())
                .spawn(move || Self::worker_entry(this_ptr))
                .expect("failed to spawn task queue worker");
            self.workers.lock().unwrap().push(worker);
        }
    }

    /// worker 线程入口：经 SendSyncPtr 捕获整个队列指针（避免 edition-2021
    /// disjoint capture 只捕获裸字段导致 !Send）。
    fn worker_entry(ptr: crate::globals::SendSyncPtr<ParallelTaskQueue>) {
        let q = unsafe { &*ptr.0 };
        q.worker_loop();
    }

    fn worker_loop(&self) {
        loop {
            let task = {
                let mut guard = self.state.lock().unwrap();
                loop {
                    if self.shutting_down.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Some(t) = guard.tasks.pop_front() {
                        break t;
                    }
                    guard = self.task_cond.wait(guard).unwrap();
                }
            };
            // 执行任务（锁外）
            task();
            // mTaskCount--；归零时广播完成条件变量（原版 WorkerFunc L132830-132850）
            let mut guard = self.state.lock().unwrap();
            let remaining = self.task_count.fetch_sub(1, Ordering::SeqCst) - 1;
            if remaining == 0 {
                drop(guard);
                self.completion_cond.notify_all();
            }
        }
    }

    /// 提交一个任务（原版：push + mTaskCount++ + signal mTaskCondition）。
    pub fn submit(&self, task: Task) {
        let mut guard = self.state.lock().unwrap();
        guard.tasks.push_back(task);
        self.task_count.fetch_add(1, Ordering::SeqCst);
        drop(guard);
        self.task_cond.notify_one();
    }

    /// worker 数量（原版 UpdateData L144430 用 `mWorkers` 数量切行带）。
    pub fn worker_count(&self) -> usize {
        self.workers.lock().unwrap().len()
    }

    /// 等待所有已提交任务完成（原版：while mTaskCount!=0 → wait 完成条件变量）。
    pub fn wait_all(&self) {
        let mut guard = self.state.lock().unwrap();
        while self.task_count.load(Ordering::SeqCst) != 0 {
            guard = self.completion_cond.wait(guard).unwrap();
        }
    }

    /// 关闭并等待 worker 退出（原版析构：置 mShuttingDown + join）。
    pub fn shutdown(&self) {
        // 丢失唤醒修复：标志必须在 state 锁内设置（worker 在 wait 释放锁前检查标志）。
        // 此前锁外 store + notify_all 存在窗口：worker 检查 false 后即将 wait，
        // shutdown 已 notify → worker 永久睡眠 → join 死锁。
        {
            let _guard = self.state.lock().unwrap();
            self.shutting_down.store(true, Ordering::SeqCst);
        }
        self.task_cond.notify_all();
        let workers = std::mem::take(&mut *self.workers.lock().unwrap());
        for w in workers {
            let _ = w.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed_queue(workers: usize) -> Box<ParallelTaskQueue> {
        let q = Box::new(ParallelTaskQueue::new(workers));
        q.start_workers();
        q
    }

    #[test]
    fn queue_runs_single_task() {
        let q = boxed_queue(1);
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag2 = flag.clone();
        q.submit(Box::new(move || {
            flag2.store(true, Ordering::SeqCst);
        }));
        q.wait_all();
        assert!(flag.load(Ordering::SeqCst));
        q.shutdown();
    }

    #[test]
    fn queue_runs_many_tasks_in_order_of_submit() {
        let q = boxed_queue(1);
        let seq = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        for i in 0..50 {
            let seq = seq.clone();
            q.submit(Box::new(move || {
                seq.lock().unwrap().push(i);
            }));
        }
        q.wait_all();
        let got = seq.lock().unwrap().clone();
        assert_eq!(got, (0..50).collect::<Vec<_>>());
        q.shutdown();
    }
}
