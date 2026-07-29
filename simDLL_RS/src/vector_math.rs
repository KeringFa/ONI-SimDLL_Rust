//! ONI 的 Vector2/3/4 数学类型，POD struct，无方法。
//! Phase 1 只定义内存布局，向量运算在 Phase 7+ 添加。

/// 2D 向量（8 字节，对应原版 Vector2f）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vector2f {
    pub x: f32,
    pub y: f32,
}

/// 3D 向量（12 字节，对应原版 Vector3f）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vector3f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 4D 向量（16 字节，对应原版 Vector4f）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vector4f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn vector2f_size() {
        assert_eq!(size_of::<Vector2f>(), 8);
    }

    #[test]
    fn vector3f_size() {
        assert_eq!(size_of::<Vector3f>(), 12);
    }

    #[test]
    fn vector4f_size() {
        assert_eq!(size_of::<Vector4f>(), 16);
    }
}
