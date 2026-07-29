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

/// std::unique_ptr<T> = 8 字节（单一指针，default_delete 无状态）。
#[repr(C)]
pub struct UniquePtr<T> {
    pub ptr: *mut T,
}

/// std::string (MSVC SSO) = 32 字节。
/// `_Bxty` union（16B：SSO inline buffer 或 heap pointer）+ size + capacity。
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
}
