//! C++ STL 容器的 Rust 等价物（仅用于内存布局占位）。
//!
//! 这些 struct 不实现任何方法（push/pop/etc）。
//! Phase 1 只验证内存布局与原版 MSVC STL 一致。
//! 业务逻辑（如真的 push_back）在 Phase 6+ 添加。

/// std::vector<T> = 32 字节（container_proxy + begin + end + capacity_end 四个指针）。
///
/// **修订记录（Phase 1 Task 7 派发前发现）**：原 Task 1 实现为 24B（3 指针），
/// 假设 MSVC EBO 后 vector 仅含 `_Vector_val`。但通过 SimData 析构函数
/// (`01_sim_api.c` L6109-6174) 反推 8 个子结构实际偏移，及 `gSimData` 运行时
/// 访问模式 (`11_msvcrt_ignored.c` L24744-24746 `_Last`@+0x10、`_End`@+0x18)，
/// 证实此 simDLL.dll 编译时启用了 `_ITERATOR_DEBUG_LEVEL >= 1`，vector 多出
/// 8 字节 `_Container_proxy*` 头部。原版 `vector<float>` sizeof = 32 已在源码
/// `00_types_reference.c` L7768-7801 验证（field0_0x0..field31_0x1f）。
///
/// 此修正使 SimData 精确达 0x890=2192B（旧 24B 永远凑不齐），并使 Disease
/// 的 reserved[16] 占位变为多余（2×32B = 64B 已足够，无需补齐）。
#[repr(C)]
pub struct MsvcVector<T> {
    /// MSVC `_Container_proxy*`（iterator debugging 头，运行时由 vector ctor 初始化）。
    /// Phase 1 仅占内存布局，不实际使用。
    pub container_proxy: *mut u8,
    /// `_First` — 指向首元素（begin 迭代器）。
    pub begin: *mut T,
    /// `_Last` — 指向末元素之后（end 迭代器，size = end - begin）。
    pub end: *mut T,
    /// `_End` — 容量末尾（capacity = end - begin）。
    pub capacity_end: *mut T,
}

impl<T: Default + Copy> MsvcVector<T> {
    /// 构造空 vector（4 个指针全 null，对应 MSVC 默认构造）。
    pub fn new() -> Self {
        Self {
            container_proxy: std::ptr::null_mut(),
            begin: std::ptr::null_mut(),
            end: std::ptr::null_mut(),
            capacity_end: std::ptr::null_mut(),
        }
    }

    /// 是否为空（begin == end）。
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

    /// 读取索引处的元素副本（越界返回 T::default()）。
    pub fn get(&self, index: usize) -> T {
        if index < self.len() {
            unsafe { std::ptr::read(self.begin.add(index)) }
        } else {
            T::default()
        }
    }

    /// 获取索引处元素的可变引用（越界返回 None）。
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.len() {
            Some(unsafe { &mut *self.begin.add(index) })
        } else {
            None
        }
    }

    /// 写入索引处的元素（越界时静默忽略，对应源码 _invalid_parameter_noinfo_noreturn 的安全降级）。
    pub fn set(&mut self, index: usize, value: T) {
        if index < self.len() {
            unsafe { std::ptr::write(self.begin.add(index), value) }
        }
    }

    /// 调整容量到 new_len，新元素用 value 填充。
    ///
    /// 对应 MSVC vector::resize。Phase 3 简化实现：
    /// - 分配新缓冲区（Box<[T]>），释放旧缓冲区
    /// - 不维护 _Container_proxy（Phase 6+ 接入真实 STL 时补）
    pub fn resize(&mut self, new_len: usize, value: T) {
        use std::alloc::{alloc, dealloc, Layout};
        let elem_size = std::mem::size_of::<T>();
        let elem_align = std::mem::align_of::<T>().max(8);  // MSVC vector 对齐 8B
        if new_len == 0 {
            if !self.begin.is_null() {
                let old_len = self.len();
                let old_byte_len = old_len * elem_size;
                let layout = Layout::from_size_align(old_byte_len.max(1), elem_align)
                    .unwrap();
                unsafe { dealloc(self.begin as *mut u8, layout) };
            }
            self.begin = std::ptr::null_mut();
            self.end = std::ptr::null_mut();
            self.capacity_end = std::ptr::null_mut();
            return;
        }
        let new_byte_len = new_len.checked_mul(elem_size).expect("overflow");
        let layout = Layout::from_size_align(new_byte_len, elem_align).unwrap();
        let new_buf = unsafe { alloc(layout) as *mut T };
        if new_buf.is_null() {
            // 分配失败：保持原状（对应源码 std::bad_alloc 抛异常，Rust 降级为静默）
            return;
        }
        // 填充新值
        for i in 0..new_len {
            unsafe { std::ptr::write(new_buf.add(i), value) };
        }
        // 释放旧缓冲
        if !self.begin.is_null() {
            let old_len = self.len();
            let old_byte_len = old_len * elem_size;
            let old_layout = Layout::from_size_align(old_byte_len.max(1), elem_align).unwrap();
            unsafe { dealloc(self.begin as *mut u8, old_layout) };
        }
        self.begin = new_buf;
        self.end = unsafe { new_buf.add(new_len) };
        self.capacity_end = self.end;
    }
}

impl<T: Default + Copy> Default for MsvcVector<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// std::unique_ptr<T> = 8 字节（单一指针，default_delete 无状态）。
#[repr(C)]
pub struct UniquePtr<T> {
    pub ptr: *mut T,
}

/// std::string (MSVC SSO) = 32 字节。
/// `_Bxty` union（16B：SSO inline buffer 或 heap pointer）+ size + capacity。
///
/// **Phase 3 Task T3 授权 derive 补全**：`MsvcString` 是纯 POD（无 Drop、无指针
/// 语义业务字段），`Default`（全零 = 空 SSO string，与 MSVC `std::string` 默认
/// 构造语义一致）+ `Copy` + `Clone` 都是安全且语义正确的。此次 derive 用于让
/// `MsvcVector<MsvcString>::new()` 与 `is_empty()` 满足 `T: Default + Copy` bound。
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct MsvcString {
    pub buffer_or_inline: [u8; 16],
    pub size: u64,
    pub capacity: u64,
}

/// std::function<...> = 64 字节（典型大小，含 vtable + small storage）。
/// 实际大小因签名而异；64B 是 ONI 中观察到的最大值。
#[repr(C)]
pub struct MsvcFunction {
    pub vtable: *const u8,
    pub storage: [u8; 56],
}

/// std::mutex (MSVC, IDL>=1) = 80B。
///
/// 占位：内部 `_Mtx_internal_imp_t` struct，包含 SRWLOCK 或 RTL_CRITICAL_SECTION
/// + iterator debugging 头部 + 计数字段。Phase 6+ 接入业务逻辑（lock/unlock）时
/// 替换为真实实现或调用 Win32 SRW API。
///
/// **对齐**：`#[repr(C, align(8))]` 保证 8 字节对齐（std::mutex 在 MSVC x64
/// 上对齐为 8，因为内部含指针）。
#[repr(C, align(8))]
pub struct MsvcMutex {
    pub _internal: [u8; 80],
}

/// std::condition_variable (MSVC) = 8B。
///
/// 占位：基于 SRWCONDITION_VARIABLE（Win32 `SleepConditionVariableSRW`）。
/// 无论 IDL 等级，sizeof(std::condition_variable) 始终 8B。
#[repr(C, align(8))]
pub struct MsvcCondVar {
    pub _internal: [u8; 8],
}

// Safety: 这些是 POD 占位 struct，跨线程共享原始指针是安全的
// （Phase 1 不实际使用这些容器，只占内存布局）。
unsafe impl<T: Send> Send for MsvcVector<T> {}
unsafe impl<T: Sync> Sync for MsvcVector<T> {}
unsafe impl<T: Send> Send for UniquePtr<T> {}
unsafe impl<T: Sync> Sync for UniquePtr<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn msvc_vector_size() {
        // 32B = 4 指针（container_proxy + begin + end + capacity_end）。
        // 修订自原 24B：详见 struct 注释（MSVC iterator debugging 头部）。
        assert_eq!(size_of::<MsvcVector<u8>>(), 32);
    }

    #[test]
    fn unique_ptr_size() {
        assert_eq!(size_of::<UniquePtr<u8>>(), 8);
    }

    #[test]
    fn msvc_string_size() {
        assert_eq!(size_of::<MsvcString>(), 32);
    }

    #[test]
    fn msvc_function_size() {
        assert_eq!(size_of::<MsvcFunction>(), 64);
    }

    #[test]
    fn msvc_mutex_size() {
        // 80B —— MSVC std::mutex with _ITERATOR_DEBUG_LEVEL >= 1
        // （Concurrency::details::stl_critical_section_win7 + IDL 头部 + 计数字段）
        assert_eq!(size_of::<MsvcMutex>(), 80);
    }

    #[test]
    fn msvc_cond_var_size() {
        // 8B —— SRWCONDITION_VARIABLE，无论 IDL 等级
        assert_eq!(size_of::<MsvcCondVar>(), 8);
    }

    #[test]
    fn msvc_vector_new_is_empty() {
        let v: MsvcVector<u32> = MsvcVector::new();
        assert!(v.is_empty());
        assert_eq!(v.len(), 0);
    }

    #[test]
    fn msvc_vector_resize_and_get_set() {
        let mut v: MsvcVector<i32> = MsvcVector::new();
        v.resize(3, 0);
        assert_eq!(v.len(), 3);
        assert!(!v.is_empty());
        v.set(0, 42);
        v.set(1, -7);
        v.set(2, 100);
        assert_eq!(v.get(0), 42);
        assert_eq!(v.get(1), -7);
        assert_eq!(v.get(2), 100);
    }

    #[test]
    fn msvc_vector_get_mut_modifies() {
        let mut v: MsvcVector<f32> = MsvcVector::new();
        v.resize(2, 0.0);
        if let Some(slot) = v.get_mut(0) {
            *slot = 3.14;
        }
        assert_eq!(v.get(0), 3.14);
    }

    #[test]
    fn msvc_vector_get_out_of_bounds_returns_none() {
        let v: MsvcVector<u8> = MsvcVector::new();
        assert_eq!(v.get(0), 0);  // 空 vector，get 返回 T::default
    }
}
