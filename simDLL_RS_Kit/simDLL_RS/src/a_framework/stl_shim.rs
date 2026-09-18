//! MSVC STL 容器的 Rust 等价物
//!
//! 对照源码 00_types_reference.c 中的 STL 容器定义。
//! A1 阶段定义内存布局；A2 阶段为 MsvcVector 添加方法（resize/clear/push 等）。
//! B1 阶段放宽 MsvcVector 方法约束：无约束块（new/is_empty/len/as_slice/set/erase）
//! 与有约束块（get/resize/clear/push，需 T: Default + Copy）分离，支持非 Copy 类型
//! （如 DiseaseInfo）作为 MsvcVector 元素。
//!
//! **关键**：MsvcVector 是 32B（4 指针），不是 24B。
//! 原因：simDLL.dll 编译时启用 _ITERATOR_DEBUG_LEVEL >= 1，
//! MSVC std::vector 多出 8B _Container_proxy* 头部。

use std::os::raw::c_void;
use std::alloc::{alloc, dealloc, Layout};

/// MSVC std::vector 等价物（32 字节）。
///
/// 内部布局：_Container_proxy* + _First + _Last + _End（4 个 8 字节指针）。
#[repr(C)]
pub struct MsvcVector<T> {
    pub container_proxy: *mut c_void,  // _Container_proxy*（IDL 头部）
    pub begin: *mut T,                 // _First
    pub end: *mut T,                   // _Last
    pub capacity_end: *mut T,          // _End
}

/// 无约束方法：仅依赖指针运算，不需要 T: Default + Copy。
impl<T> MsvcVector<T> {
    /// 构造空 vector（4 个指针全 null，对应 MSVC 默认构造）。
    pub fn new() -> Self {
        Self {
            container_proxy: std::ptr::null_mut(),
            begin: std::ptr::null_mut(),
            end: std::ptr::null_mut(),
            capacity_end: std::ptr::null_mut(),
        }
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 元素数量（end - begin，按 T 大小除）。
    pub fn len(&self) -> usize {
        if self.begin.is_null() || self.end.is_null() {
            0
        } else {
            let byte_len = (self.end as usize).saturating_sub(self.begin as usize);
            byte_len / std::mem::size_of::<T>()
        }
    }

    /// 返回元素切片（只读）。空 vector 返回空切片。
    pub fn as_slice(&self) -> &[T] {
        if self.begin.is_null() || self.end.is_null() {
            &[]
        } else {
            let len = self.len();
            if len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(self.begin, len) } }
        }
    }

    /// 写入索引处的元素（越界时静默忽略）。
    pub fn set(&mut self, index: usize, value: T) {
        if index < self.len() {
            unsafe { std::ptr::write(self.begin.add(index), value) }
        }
    }

    /// 删除指定索引处元素（左移后续元素，end 退 1）。
    pub fn erase(&mut self, index: usize) {
        let len = self.len();
        if index >= len { return; }
        unsafe {
            for i in index..len.saturating_sub(1) {
                std::ptr::write(self.begin.add(i), std::ptr::read(self.begin.add(i + 1)));
            }
        }
        self.end = unsafe { self.end.sub(1) };
    }

    /// 清空 vector 但保留已分配的内存（end = begin）。
    /// 对照源码 Start 函数 L225-244 中对 sim_events 的 20 个 vector 执行的
    /// `*(lVar1 + offset + 0x10) = *(lVar1 + offset + 0x8)` 操作。
    pub fn clear_keep_capacity(&mut self) {
        self.end = self.begin;
    }

    /// 返回 _First（begin）指针作为 `*const T`。
    /// 用于 PrepareGameDataUpdate 中提取 SOA 数据指针。
    pub fn as_ptr(&self) -> *const T {
        self.begin as *const T
    }

    /// 追加元素到末尾（不要求 T: Default + Copy）。
    /// 对应 MSVC vector::push_back。2x 增长策略。
    /// 用于 *mut T 等不实现 Default 的裸指针类型。
    pub fn push_unchecked(&mut self, value: T) {
        let elem_size = std::mem::size_of::<T>();
        let elem_align = std::mem::align_of::<T>().max(8);
        let cur_len = self.len();
        let cur_cap = if !self.begin.is_null() && !self.capacity_end.is_null() {
            (self.capacity_end as usize).saturating_sub(self.begin as usize) / elem_size
        } else {
            0
        };
        if cur_len >= cur_cap {
            let new_cap = if cur_cap == 0 { 1 } else { cur_cap * 2 };
            let new_byte_len = new_cap.checked_mul(elem_size).expect("overflow");
            let layout = Layout::from_size_align(new_byte_len, elem_align).unwrap();
            let new_buf = unsafe { alloc(layout) as *mut T };
            if new_buf.is_null() {
                return;
            }
            if !self.begin.is_null() {
                for i in 0..cur_len {
                    unsafe { std::ptr::write(new_buf.add(i), std::ptr::read(self.begin.add(i))) };
                }
                let old_byte_len = cur_cap * elem_size;
                let old_layout = Layout::from_size_align(old_byte_len.max(1), elem_align).unwrap();
                unsafe { dealloc(self.begin as *mut u8, old_layout) };
            }
            self.begin = new_buf;
            self.end = unsafe { new_buf.add(cur_len) };
            self.capacity_end = unsafe { new_buf.add(new_cap) };
        }
        unsafe { std::ptr::write(self.end, value) };
        self.end = unsafe { self.end.add(1) };
    }
}

/// 有约束方法：需要 T: Default + Copy（构造或复制 T）。
impl<T: Default + Copy> MsvcVector<T> {
    /// 读取索引处的元素副本（越界返回 T::default()）。
    pub fn get(&self, index: usize) -> T {
        if index < self.len() {
            unsafe { std::ptr::read(self.begin.add(index)) }
        } else {
            T::default()
        }
    }

    /// 调整容量到 new_len，新元素用 value 填充。
    ///
    /// 对应 MSVC vector::resize（**容量感知**）：new_len ≤ 容量时不重新分配，
    /// 只调整 size（end），新增长段填 value、已有元素保留；收缩截断（超出
    /// size 的元素不可见，内存保留）。仅 new_len > 容量时分配新缓冲并拷贝
    /// 旧元素。**性能修复（2026-08-05）**：此前无条件 alloc+零填充+释放旧缓冲，
    /// 导致 copy_sim_data_to_game 每帧对 GameData cells 的 11 个字段 resize
    /// 触发全量重分配（~12MB/帧 抖动，日志实证 62 个不同缓冲地址），疑为
    /// 1x 相机卡顿主因之一。另修正：释放时按 capacity（非 len）计算尺寸，
    /// 避免收缩后 clear 的 dealloc 尺寸不匹配。
    pub fn resize(&mut self, new_len: usize, value: T) {
        let elem_size = std::mem::size_of::<T>();
        let elem_align = std::mem::align_of::<T>().max(8);
        if new_len == 0 {
            if !self.begin.is_null() {
                // 对照 MSVC std::vector 析构语义：clear()/析构释放必须按**容量**
                // （capacity_end − begin）计算字节数，与 push/resize 增长时的分配
                // 布局一致。此前误用 len——push 2x 增长后 len < capacity，dealloc
                // 尺寸 < 分配尺寸 → 违反 GlobalAlloc 契约（未定义行为）→ 堆损坏。
                let cap = if !self.capacity_end.is_null() {
                    (self.capacity_end as usize).saturating_sub(self.begin as usize) / elem_size
                } else {
                    0
                };
                let old_byte_len = cap * elem_size;
                let layout = Layout::from_size_align(old_byte_len.max(1), elem_align).unwrap();
                unsafe { dealloc(self.begin as *mut u8, layout) };
            }
            self.begin = std::ptr::null_mut();
            self.end = std::ptr::null_mut();
            self.capacity_end = std::ptr::null_mut();
            return;
        }
        let cur_len = self.len();
        let cur_cap = if !self.begin.is_null() && !self.capacity_end.is_null() {
            (self.capacity_end as usize).saturating_sub(self.begin as usize) / elem_size
        } else {
            0
        };
        if new_len <= cur_cap {
            // 容量内：只填新增长段（已有元素保留）；收缩仅调 end。
            if new_len > cur_len {
                for i in cur_len..new_len {
                    unsafe { std::ptr::write(self.begin.add(i), value) };
                }
            }
            self.end = unsafe { self.begin.add(new_len) };
            return;
        }
        // 增长：MSVC 2x 策略 max(old_cap*2, new_len)（★2，对齐原版 vector::resize
        // 容量语义）。此前分配精确 new_len → capacity==len，与 push 的 2x 增长
        // 布局不一致；clear()/析构按 capacity 释放已配套（resize(0) 修复）。
        let new_cap = if cur_cap == 0 {
            new_len
        } else {
            new_len.max(cur_cap * 2)
        };
        let new_byte_len = new_cap.checked_mul(elem_size).expect("overflow");
        let layout = Layout::from_size_align(new_byte_len, elem_align).unwrap();
        let new_buf = unsafe { alloc(layout) as *mut T };
        if new_buf.is_null() {
            return;
        }
        for i in 0..cur_len {
            unsafe { std::ptr::write(new_buf.add(i), std::ptr::read(self.begin.add(i))) };
        }
        for i in cur_len..new_len {
            unsafe { std::ptr::write(new_buf.add(i), value) };
        }
        if !self.begin.is_null() {
            let old_byte_len = cur_cap * elem_size;
            let old_layout = Layout::from_size_align(old_byte_len.max(1), elem_align).unwrap();
            unsafe { dealloc(self.begin as *mut u8, old_layout) };
        }
        self.begin = new_buf;
        self.end = unsafe { new_buf.add(new_len) };
        self.capacity_end = unsafe { new_buf.add(new_cap) };
    }

    /// 清空 vector（释放内存，重置为空）。
    pub fn clear(&mut self) {
        self.resize(0, T::default());
    }

    /// 追加元素到末尾（对应 MSVC vector::push_back）。2x 增长策略。
    pub fn push(&mut self, value: T) {
        let elem_size = std::mem::size_of::<T>();
        let elem_align = std::mem::align_of::<T>().max(8);
        let cur_len = self.len();
        let cur_cap = if !self.begin.is_null() && !self.capacity_end.is_null() {
            (self.capacity_end as usize).saturating_sub(self.begin as usize) / elem_size
        } else { 0 };
        if cur_len >= cur_cap {
            let new_cap = if cur_cap == 0 { 1 } else { cur_cap * 2 };
            let new_byte_len = new_cap.checked_mul(elem_size).expect("overflow");
            let layout = Layout::from_size_align(new_byte_len, elem_align).unwrap();
            let new_buf = unsafe { alloc(layout) as *mut T };
            if new_buf.is_null() { return; }
            if !self.begin.is_null() {
                for i in 0..cur_len {
                    unsafe { std::ptr::write(new_buf.add(i), self.get(i)) };
                }
                let old_byte_len = cur_cap * elem_size;
                let old_layout = Layout::from_size_align(old_byte_len.max(1), elem_align).unwrap();
                unsafe { dealloc(self.begin as *mut u8, old_layout) };
            }
            self.begin = new_buf;
            self.end = unsafe { new_buf.add(cur_len) };
            self.capacity_end = unsafe { new_buf.add(new_cap) };
        }
        unsafe { std::ptr::write(self.end, value) };
        self.end = unsafe { self.end.add(1) };
    }
}

impl<T> Default for MsvcVector<T> {
    fn default() -> Self {
        Self {
            container_proxy: std::ptr::null_mut(),
            begin: std::ptr::null_mut(),
            end: std::ptr::null_mut(),
            capacity_end: std::ptr::null_mut(),
        }
    }
}

impl<T> std::fmt::Debug for MsvcVector<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MsvcVector")
            .field("len", &self.len())
            .finish()
    }
}

/// MSVC std::unique_ptr 等价物（8 字节）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct UniquePtr<T> {
    pub ptr: *mut T,
}

/// MSVC std::string 等价物（40 字节，_ITERATOR_DEBUG_LEVEL>=1）。
/// 布局：_Container_proxy*（8B）+ 16B SSO buffer + 8B size + 8B capacity。
/// 2026-08-04 审查修正：原实现 32B 缺 _Container_proxy（原版 00_types_reference.c
/// L941-980 反编译 field0_0x0..field39_0x27 = 40B）。目前仅 Disease::disease_names
/// 使用（C# 不读），但病菌表实现前必须对齐 40B。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MsvcString {
    pub container_proxy: *mut c_void, // _Container_proxy*（IDL 头部）
    pub buffer: [u8; 16],            // SSO buffer
    pub size: u64,
    pub capacity: u64,
}

impl Default for MsvcString {
    fn default() -> Self {
        Self {
            container_proxy: std::ptr::null_mut(),
            buffer: [0; 16],
            size: 0,
            capacity: 0,
        }
    }
}

impl MsvcString {
    /// 写入字符串字节（MSVC std::string 布局：SSO ≤15B / 堆 >15B）。
    ///
    /// 仅 `Disease::disease_names` 使用（C# 不读、Rust 侧也不依赖内容），
    /// 因此内部存储自洽即可。长度 ≤15 走 SSO（buffer[15] 存 capacity=15）；
    /// 更长走堆分配（buffer[0..8] 存指针，size=len, capacity=len）。
    /// 注意：本结构 derive Copy 且无 Drop，堆内存不自动释放——与项目 MsvcVector
    /// 快照泄漏约定一致（CreateDiseaseTable 每次加载重建，泄漏量极小，已知可接受）。
    pub fn set_bytes(&mut self, bytes: &[u8]) {
        let len = bytes.len();
        self.size = len as u64;
        if len <= 15 {
            self.buffer[..len].copy_from_slice(bytes);
            self.buffer[15] = 15; // MSVC SSO 末字节存 capacity
            self.capacity = 15;
            return;
        }
        let layout = std::alloc::Layout::from_size_align(len + 1, 1).unwrap();
        let ptr = unsafe { std::alloc::alloc(layout) } as *mut u8;
        if ptr.is_null() {
            // 分配失败：降级为 SSO 截断（仅诊断用途，名字不被任何代码读取）
            let n = len.min(15);
            self.buffer[..n].copy_from_slice(&bytes[..n]);
            self.buffer[15] = 15;
            self.size = n as u64;
            self.capacity = 15;
            return;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len);
            std::ptr::write(ptr.add(len), 0);
        }
        self.buffer[0..8].copy_from_slice(&(ptr as usize).to_ne_bytes());
        self.capacity = len as u64;
    }

    /// 读回字符串字节（测试/诊断用）。
    pub fn as_bytes(&self) -> Vec<u8> {
        let len = self.size as usize;
        if len == 0 {
            return Vec::new();
        }
        if len <= 15 {
            self.buffer[..len].to_vec()
        } else {
            let ptr = usize::from_ne_bytes(self.buffer[0..8].try_into().unwrap()) as *const u8;
            unsafe { std::slice::from_raw_parts(ptr, len).to_vec() }
        }
    }
}

/// MSVC std::function 等价物（64 字节）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MsvcFunction {
    pub buffer: [u8; 64],
}

/// MSVC std::mutex 等价物（80 字节，align 8）。
///
/// 布局约束：80 字节 / align 8 是 SimData 结构布局的一部分（单测锁死），跨平台不变。
/// - Windows：内部使用 SRWLOCK（8 字节）实现互斥，前 8 字节作为 SRWLOCK 存储，
///   其余 72 字节为 padding 以匹配 MSVC _Mtx_internal_imp_t 的 80 字节布局。
/// - Linux：bytes[0..40] 作为 glibc `pthread_mutex_t`（40B；全零 == PTHREAD_MUTEX_INITIALIZER，
///   与 Windows SRWLOCK 同样支持零初始化语义），其余 padding。SimDLL 已完整替换原版
///   C++ 代码，锁只被 Rust 侧操作，无需匹配 glibc 的 _Mtx 布局。
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct MsvcMutex {
    pub bytes: [u8; 80],
}

/// MsvcMutex 的锁守卫。Drop 时自动释放锁。
pub struct MsvcMutexGuard<'a> {
    mutex: &'a MsvcMutex,
}

impl MsvcMutex {
    /// 获取互斥锁（阻塞）。返回 guard，Drop 时自动释放。
    ///
    /// 对照源码 _Mtx_lock。Windows 用 SRWLOCK 独占锁，Linux 用 pthread_mutex_lock。
    pub fn lock(&self) -> MsvcMutexGuard<'_> {
        #[cfg(windows)]
        unsafe {
            windows::Win32::System::Threading::AcquireSRWLockExclusive(
                self.bytes.as_ptr() as *mut windows::Win32::System::Threading::SRWLOCK,
            );
        }
        #[cfg(target_os = "linux")]
        unsafe {
            let m = self.bytes.as_ptr() as *mut libc::pthread_mutex_t;
            let r = libc::pthread_mutex_lock(m);
            debug_assert_eq!(r, 0, "pthread_mutex_lock 失败: {r}");
        }
        MsvcMutexGuard { mutex: self }
    }

    /// 获取互斥锁（阻塞）。不返回 guard，需手动调用 `unlock_raw`。
    ///
    /// 用于 SimSync/GameSync 等需要在持有锁时调用 `MsvcCondVar::wait` 的场景
    /// （wait 会自动释放并重新获取锁，guard 的自动 Drop 会导致双重释放）。
    pub fn lock_raw(&self) {
        #[cfg(windows)]
        unsafe {
            windows::Win32::System::Threading::AcquireSRWLockExclusive(
                self.bytes.as_ptr() as *mut windows::Win32::System::Threading::SRWLOCK,
            );
        }
        #[cfg(target_os = "linux")]
        unsafe {
            let m = self.bytes.as_ptr() as *mut libc::pthread_mutex_t;
            let r = libc::pthread_mutex_lock(m);
            debug_assert_eq!(r, 0, "pthread_mutex_lock 失败: {r}");
        }
    }

    /// 释放互斥锁（配合 `lock_raw` 使用）。
    pub fn unlock_raw(&self) {
        #[cfg(windows)]
        unsafe {
            windows::Win32::System::Threading::ReleaseSRWLockExclusive(
                self.bytes.as_ptr() as *mut windows::Win32::System::Threading::SRWLOCK,
            );
        }
        #[cfg(target_os = "linux")]
        unsafe {
            let m = self.bytes.as_ptr() as *mut libc::pthread_mutex_t;
            let r = libc::pthread_mutex_unlock(m);
            debug_assert_eq!(r, 0, "pthread_mutex_unlock 失败: {r}");
        }
    }
}

impl Drop for MsvcMutexGuard<'_> {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows::Win32::System::Threading::ReleaseSRWLockExclusive(
                self.mutex.bytes.as_ptr() as *mut windows::Win32::System::Threading::SRWLOCK,
            );
        }
        #[cfg(target_os = "linux")]
        unsafe {
            let m = self.mutex.bytes.as_ptr() as *mut libc::pthread_mutex_t;
            let r = libc::pthread_mutex_unlock(m);
            debug_assert_eq!(r, 0, "pthread_mutex_unlock 失败: {r}");
        }
    }
}

/// MSVC std::condition_variable 等价物（8 字节，align 8）。
///
/// - Windows：内部使用 CONDITION_VARIABLE（8 字节）。零初始化即为有效状态
///   （INITIALIZE_CONDITION_VARIABLE 实际就是置零）。
/// - Linux：bytes[0..4] 作为 futex 字（u32 序号）。pthread_cond_t 48B 装不进
///   8 字节布局，改用经典 futex 条件变量（Drepper 模式）：
///   wait 读序号 → 放锁 → futex_wait(序号未变) → 重取锁；
///   signal/broadcast 递增序号后 futex_wake。序号在放锁前读取，
///   放锁与 futex_wait 之间到达的 signal 会使序号变化 → futex_wait
///   立即返回 → 不丢唤醒。
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct MsvcCondVar {
    pub bytes: [u8; 8],
}

#[cfg(target_os = "linux")]
mod condvar_futex {
    // musl 的 libc crate 未导出 _PRIVATE 变体——内核 ABI 值固定，自行定义：
    // FUTEX_PRIVATE_FLAG=128，FUTEX_WAIT=0，FUTEX_WAKE=1。
    const FUTEX_WAIT_PRIVATE: i32 = 0x80;
    const FUTEX_WAKE_PRIVATE: i32 = 0x81;

    #[inline]
    fn futex_word(cv: &super::MsvcCondVar) -> *mut u32 {
        cv.bytes.as_ptr() as *mut u32
    }

    /// futex(FUTEX_WAIT_PRIVATE, 期望值)。值已变（被唤醒过）→ EAGAIN 立即返回。
    pub fn futex_wait(cv: &super::MsvcCondVar, expected: u32) {
        unsafe {
            loop {
                let r = libc::syscall(
                    libc::SYS_futex,
                    futex_word(cv),
                    FUTEX_WAIT_PRIVATE,
                    expected,
                    std::ptr::null::<libc::timespec>(),
                );
                if r == 0 {
                    return; // 被唤醒
                }
                let err = *libc::__errno_location();
                if err == libc::EAGAIN {
                    return; // 序号已变 → 不丢唤醒
                }
                if err == libc::EINTR {
                    continue; // 信号打断 → 重试
                }
                return; // 其他错误：保守返回（退化为此处无阻塞）
            }
        }
    }

    /// futex(FUTEX_WAKE_PRIVATE, n)。
    pub fn futex_wake(cv: &super::MsvcCondVar, n: i32) {
        unsafe {
            libc::syscall(libc::SYS_futex, futex_word(cv), FUTEX_WAKE_PRIVATE, n);
        }
    }

    #[inline]
    pub fn load_seq(cv: &super::MsvcCondVar) -> u32 {
        unsafe { futex_word(cv).read_volatile() }
    }

    #[inline]
    pub fn bump_seq(cv: &super::MsvcCondVar) -> u32 {
        unsafe {
            let p = futex_word(cv);
            let old = p.read_volatile();
            p.write_volatile(old.wrapping_add(1));
            old.wrapping_add(1)
        }
    }
}

impl MsvcCondVar {
    /// 唤醒一个等待此条件变量的线程。
    /// 对照 MSVC _Cnd_signal → WakeConditionVariable。
    pub fn signal(&self) {
        #[cfg(windows)]
        unsafe {
            windows::Win32::System::Threading::WakeConditionVariable(
                self.bytes.as_ptr() as *mut windows::Win32::System::Threading::CONDITION_VARIABLE,
            );
        }
        #[cfg(target_os = "linux")]
        {
            condvar_futex::bump_seq(self);
            condvar_futex::futex_wake(self, 1);
        }
    }

    /// 唤醒所有等待此条件变量的线程。
    /// 对照 MSVC _Cnd_broadcast → WakeAllConditionVariable。
    pub fn broadcast(&self) {
        #[cfg(windows)]
        unsafe {
            windows::Win32::System::Threading::WakeAllConditionVariable(
                self.bytes.as_ptr() as *mut windows::Win32::System::Threading::CONDITION_VARIABLE,
            );
        }
        #[cfg(target_os = "linux")]
        {
            condvar_futex::bump_seq(self);
            condvar_futex::futex_wake(self, i32::MAX);
        }
    }

    /// 等待条件变量（必须在持有 `mutex` 的情况下调用）。
    /// 对照 MSVC _Cnd_wait → SleepConditionVariableSRW。
    ///
    /// 会自动释放 `mutex` 并阻塞，被 `signal`/`broadcast` 唤醒后重新获取 `mutex`。
    /// 返回 `true` 表示成功等到信号，`false` 表示系统错误。
    pub fn wait(&self, mutex: &MsvcMutex) -> bool {
        #[cfg(windows)]
        unsafe {
            // windows 0.58: SleepConditionVariableSRW 返回 Result<(), Error>
            // INFINITE = 0xFFFFFFFF (u32)
            windows::Win32::System::Threading::SleepConditionVariableSRW(
                self.bytes.as_ptr() as *mut windows::Win32::System::Threading::CONDITION_VARIABLE,
                mutex.bytes.as_ptr() as *mut windows::Win32::System::Threading::SRWLOCK,
                0xFFFFFFFFu32, // INFINITE
                0,             // 0 = exclusive lock
            )
            .is_ok()
        }
        #[cfg(target_os = "linux")]
        {
            // 经典 futex condvar：读序号 → 放锁 → 等序号变化 → 取锁。
            let seq = condvar_futex::load_seq(self);
            mutex.unlock_raw();
            condvar_futex::futex_wait(self, seq);
            mutex.lock_raw();
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msvc_vector_size_is_32() {
        assert_eq!(std::mem::size_of::<MsvcVector<u8>>(), 32);
    }

    #[test]
    fn unique_ptr_size_is_8() {
        assert_eq!(std::mem::size_of::<UniquePtr<u8>>(), 8);
    }

    #[test]
    fn msvc_string_size_is_40() {
        // 原版 basic_string 40B（_Container_proxy + SSO 16 + size 8 + cap 8）
        assert_eq!(std::mem::size_of::<MsvcString>(), 40);
    }

    #[test]
    fn msvc_function_size_is_64() {
        assert_eq!(std::mem::size_of::<MsvcFunction>(), 64);
    }

    #[test]
    fn msvc_mutex_size_is_80() {
        assert_eq!(std::mem::size_of::<MsvcMutex>(), 80);
    }

    #[test]
    fn msvc_condvar_size_is_8() {
        assert_eq!(std::mem::size_of::<MsvcCondVar>(), 8);
    }

    #[test]
    fn msvc_vector_new_is_empty() {
        let v: MsvcVector<i32> = MsvcVector::new();
        assert!(v.is_empty());
        assert_eq!(v.len(), 0);
    }

    #[test]
    fn msvc_vector_resize_and_get() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.resize(5, 42);
        assert_eq!(v.len(), 5);
        assert_eq!(v.get(0), 42);
        assert_eq!(v.get(4), 42);
    }

    #[test]
    fn msvc_vector_set() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.resize(3, 0);
        v.set(1, 99);
        assert_eq!(v.get(1), 99);
    }

    #[test]
    fn msvc_vector_push() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.push(10);
        v.push(20);
        v.push(30);
        assert_eq!(v.len(), 3);
        assert_eq!(v.get(0), 10);
        assert_eq!(v.get(1), 20);
        assert_eq!(v.get(2), 30);
    }

    #[test]
    fn msvc_vector_clear() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.resize(5, 42);
        v.clear();
        assert!(v.is_empty());
    }

    #[test]
    fn msvc_vector_erase() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.push(1);
        v.push(2);
        v.push(3);
        v.erase(1);
        assert_eq!(v.len(), 2);
        assert_eq!(v.get(0), 1);
        assert_eq!(v.get(1), 3);
    }

    #[test]
    fn msvc_vector_as_slice() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.push(10);
        v.push(20);
        let s = v.as_slice();
        assert_eq!(s, &[10, 20]);
    }

    #[test]
    fn msvc_vector_default_is_empty() {
        let v: MsvcVector<i32> = MsvcVector::default();
        assert!(v.is_empty());
    }

    /// 容量感知 resize：同长度 resize 不重新分配（begin 指针不变）、旧值保留。
    /// 这是性能专项的核心断言——此前每帧同尺寸 resize 全量重分配（12MB/帧）。
    #[test]
    fn msvc_vector_resize_same_len_keeps_buffer_and_values() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.resize(4, 7);
        v.set(0, 1);
        v.set(1, 2);
        let begin_before = v.begin as usize;
        v.resize(4, 9); // 同长度：不重分配、不覆盖已有值
        assert_eq!(v.begin as usize, begin_before, "同长度 resize 不得重分配");
        assert_eq!(v.len(), 4);
        assert_eq!(v.get(0), 1, "已有值必须保留");
        assert_eq!(v.get(1), 2, "已有值必须保留");
        // 同长度 resize（MSVC 语义 n == size）不填任何值：2/3 保留初始 7
        assert_eq!(v.get(2), 7);
        assert_eq!(v.get(3), 7);
    }

    /// 容量内增长：旧值保留、新段填 value；收缩截断（超出不可见）。
    #[test]
    fn msvc_vector_resize_grow_within_capacity_and_shrink() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.resize(2, 0);
        v.set(0, 5);
        v.set(1, 6);
        v.resize(4, 8); // 容量内增长（旧实现会重分配；容量感知仅填新段）
        assert_eq!(v.len(), 4);
        assert_eq!(v.get(0), 5);
        assert_eq!(v.get(1), 6);
        assert_eq!(v.get(2), 8);
        assert_eq!(v.get(3), 8);
        v.resize(1, 0); // 收缩：截断
        assert_eq!(v.len(), 1);
        assert_eq!(v.get(0), 5);
        v.resize(2, 9); // 收缩后同容量增长：新段填 value
        assert_eq!(v.len(), 2);
        assert_eq!(v.get(0), 5);
        assert_eq!(v.get(1), 9);
    }

    /// 增长超容量：分配新缓冲，旧元素保留（此前增长会丢旧值）。
    #[test]
    fn msvc_vector_resize_grow_beyond_capacity_keeps_values() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.resize(2, 0);
        v.set(0, 11);
        v.set(1, 22);
        v.resize(6, 7);
        assert_eq!(v.len(), 6);
        assert_eq!(v.get(0), 11, "增长必须保留旧值");
        assert_eq!(v.get(1), 22);
        assert_eq!(v.get(2), 7);
        assert_eq!(v.get(5), 7);
    }

    /// clear（resize 0）释放内存；之后 resize 全新分配。
    #[test]
    fn msvc_vector_clear_frees_then_resize_fresh() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.resize(5, 3);
        v.clear();
        assert!(v.begin.is_null());
        assert!(v.is_empty());
        v.resize(3, 9);
        assert_eq!(v.len(), 3);
        assert_eq!(v.get(0), 9);
        assert_eq!(v.get(2), 9);
    }
}
