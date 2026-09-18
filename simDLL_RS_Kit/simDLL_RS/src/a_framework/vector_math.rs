//! 向量数学类型：Vector2f/3f/4f
//!
//! 对照源码 00_types_reference.c 中的 Vector2/3/4 定义。

use std::os::raw::c_float;

/// 2D 向量（8 字节）。对照源码 Vector2。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Vector2f {
    pub x: c_float,
    pub y: c_float,
}

/// 3D 向量（12 字节）。对照源码 Vector3。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Vector3f {
    pub x: c_float,
    pub y: c_float,
    pub z: c_float,
}

/// 4D 向量（16 字节）。对照源码 Vector4。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Vector4f {
    pub x: c_float,
    pub y: c_float,
    pub z: c_float,
    pub w: c_float,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector2f_size_is_8() {
        assert_eq!(std::mem::size_of::<Vector2f>(), 8);
    }

    #[test]
    fn vector3f_size_is_12() {
        assert_eq!(std::mem::size_of::<Vector3f>(), 12);
    }

    #[test]
    fn vector4f_size_is_16() {
        assert_eq!(std::mem::size_of::<Vector4f>(), 16);
    }

    #[test]
    fn vector2f_default_is_zero() {
        let v = Vector2f::default();
        assert_eq!(v.x, 0.0);
        assert_eq!(v.y, 0.0);
    }
}
