//! Phase 2 集成测试：模拟 C# P/Invoke 调用 5 个导出函数。
//!
//! 运行：cargo test --test elements_smoke
//!
//! 对照 Phase 0 的 abi_smoke.rs 模式：验证不 panic + 边界条件。

use SimDLL::elements_table::*;
use SimDLL::buffer::{BinaryBufferReader, BinaryBufferWriter};
use SimDLL::element::{INVALID_ELEMENT_INDEX, GAS_OBLITERATION_SIGNATURE};

#[test]
fn create_elements_table_with_null_reader_does_not_panic() {
    let result = CreateElementsTable(std::ptr::null_mut());
    assert!(result.is_null(), "null reader 应返回 null");
}

#[test]
fn destroy_elements_table_does_not_panic() {
    // 多次调用应幂等
    DestroyElementsTable();
    DestroyElementsTable();
}

#[test]
fn get_element_index_returns_invalid_for_empty_table() {
    // 确保 table 为空
    DestroyElementsTable();
    let result = GetElementIndex(0);
    assert_eq!(result, INVALID_ELEMENT_INDEX);
}

#[test]
fn create_element_interactions_with_null_reader_does_not_panic() {
    let result = CreateElementInteractions(std::ptr::null_mut());
    assert!(result.is_null());
}

#[test]
fn create_element_interactions_locked_with_null_reader_does_not_panic() {
    let result = CreateElementInteractionsLocked(std::ptr::null_mut());
    assert!(result.is_null());
}

#[test]
fn create_elements_table_full_round_trip_via_ffi() {
    // 构造最小 buffer：1 个元素
    let mut w = BinaryBufferWriter::new();
    w.write_int(1).unwrap();
    let mut elem = SimDLL::element::Element::default();
    elem.id = 42;
    elem.state = 1;
    elem.default_values.mass = 2.0;
    let bytes: [u8; 164] = unsafe { std::mem::transmute(elem) };
    w.write_bytes(&bytes).unwrap();
    w.write_uint(4).unwrap();
    w.write_bytes(b"Test").unwrap();

    let buf = w.into_bytes();
    let mut reader = BinaryBufferReader::new(&buf);

    let result = CreateElementsTable(&mut reader as *mut _);
    // 源码返回 (void*)(longlong)count，count=1 不应为 null
    assert!(!result.is_null());

    // 后续可调用 GetElementIndex 验证
    let idx = GetElementIndex(42);
    assert_eq!(idx, 0);

    // 清理
    DestroyElementsTable();
}

#[test]
fn create_element_interactions_locked_full_round_trip_via_ffi() {
    // 构造 interactions buffer：1 个匹配签名的 entry
    // 每 entry 32 字节布局（与源码 03_elements.c L565 `lVar3 = lVar3 + 0x20` stride 一致）：
    //   @0  i32 signature (4)
    //   @4  u16 elem_idx1 (2)
    //   @6  u16 elem_idx2 (2)
    //   @8  u16 elem_result_idx (2)
    //   @0xa 2 字节 padding（read_ushort 占位）
    //   @0xc f32 min_mass (4)
    //   @0x10 f32 interaction_probability (4)
    //   @0x14 f32 elem1_mass_destruction_percent (4)
    //   @0x18 f32 elem2_mass_required_multiplier (4)
    //   @0x1c f32 elem_result_mass_creation_multiplier (4)
    //   合计 32 字节
    let mut w = BinaryBufferWriter::new();
    w.write_int(1).unwrap();  // count = 1
    w.write_int(GAS_OBLITERATION_SIGNATURE).unwrap();
    w.write_ushort(1).unwrap();
    w.write_ushort(2).unwrap();
    w.write_ushort(3).unwrap();
    w.write_ushort(0).unwrap();  // padding @0xa-0xb
    w.write_float(10.0).unwrap();      // min_mass
    w.write_float(0.5).unwrap();       // interaction_probability
    w.write_float(0.25).unwrap();      // elem1_mass_destruction_percent
    w.write_float(1.5).unwrap();       // elem2_mass_required_multiplier
    w.write_float(0.75).unwrap();     // elem_result_mass_creation_multiplier
    // ⚠️ 不要写 trailing 2 字节 padding！每 entry 共 32 字节（4+2*4+5*4=32），无 trailing pad。
    // 这是 T6 教训 K1 修复后的正确 stride（详见 .superpowers/sdd/progress.md T6 记录）。

    let buf = w.into_bytes();
    let mut reader = BinaryBufferReader::new(&buf);

    let result = CreateElementInteractionsLocked(&mut reader as *mut _);
    // 源码 03_elements.c L568: CreateElementInteractions 始终 `return (void *)0x0;`
    // L17+L22: Locked 版本 `pvVar2 = CreateElementInteractions(...); return pvVar2;`
    // → Locked 版本与 CreateElementInteractions 一致，始终返回 null（非成功/失败标志）
    assert!(result.is_null(), "源码 L568 始终返回 null（与 CreateElementInteractions 一致）");

    // 清理
    DestroyElementsTable();
}
