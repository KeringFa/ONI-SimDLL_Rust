//! C++ STL 容器的 Rust 等价物（仅用于内存布局占位）。
//!
//! 这些 struct 不实现任何方法（push/pop/etc）。
//! Phase 1 只验证内存布局与原版 MSVC STL 一致。
//! 业务逻辑（如真的 push_back）在 Phase 6+ 添加。

/// std::vector<T> = 24 字节（begin + end + capacity_end 三个指针）。
/// 对应 MSVC `_Compressed_pair<Allocator, _Vector_val>` EBO 后的布局。
#[repr(C)]
pub struct MsvcVector<T> {
    pub begin: *mut T,
    pub end: *mut T,
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
        assert_eq!(size_of::<MsvcVector<u8>>(), 24);
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
