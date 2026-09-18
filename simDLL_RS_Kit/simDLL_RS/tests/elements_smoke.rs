//! FFI 集成测试：验证 B1 阶段 5 个元素表 C ABI 导出函数。
//!
//! 对照源码 03_elements.c 的 5 个导出：
//! - CreateElementsTable / DestroyElementsTable / GetElementIndex
//! - CreateElementInteractions / CreateElementInteractionsLocked
//!
//! **测试策略**：
//! - null reader 安全性（不崩溃）
//! - 空表状态查询
//! - 完整 CreateElementsTable 数据包 round-trip（count=2 → hash → idx 映射）
//! - CreateElementInteractions 解析 GasObliteration entry（含真实堆指针）

use SimDLL::a_framework::buffer::{BinaryBufferReader, BinaryBufferWriter};
use SimDLL::b_elements::element::{GAS_OBLITERATION_SIGNATURE, INVALID_ELEMENT_INDEX};
use SimDLL::b_elements::elements_table::{
    CreateElementInteractions, CreateElementInteractionsLocked, CreateElementsTable,
    DestroyElementsTable, GetElementIndex, get_element_count_pub,
};

/// 本文件级锁：串行化所有操作全局 ElementsTable 的测试，
/// 避免并行执行导致 setup 互相覆盖。
static ELEMENTS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 构造一个最小的 ElementsTable 数据包（2 个元素）。
///
/// 数据格式（对照 C# SimMessages.CreateSimElementsTable）：
/// - 4 字节 count
/// - count × 164 字节 Element
/// - count × KleiString（4 字节长度 + N 字节 UTF-8）
fn build_minimal_elements_data() -> Vec<u8> {
    let mut w = BinaryBufferWriter::new();
    // 4 字节 count = 2
    w.write_int(2);

    // 元素 0：id=100, state=1 (Gas)
    // Element 字段偏移对照 element.rs：
    //   id @0 (i32), state @16 (u8)
    let mut elem0 = [0u8; 164];
    elem0[0..4].copy_from_slice(&100_i32.to_le_bytes());
    elem0[16] = 1; // state = Gas
    w.write_bytes(&elem0);

    // 元素 1：id=200, state=3 (Solid)
    let mut elem1 = [0u8; 164];
    elem1[0..4].copy_from_slice(&200_i32.to_le_bytes());
    elem1[16] = 3; // state = Solid
    w.write_bytes(&elem1);

    // 元素名称（KleiString 格式：4 字节长度 + N 字节 UTF-8）
    w.write_int(3);
    w.write_bytes(b"Gas");
    w.write_int(6);
    w.write_bytes(b"Solid1");

    w.into_bytes()
}

// ===== null reader 安全性测试 =====

#[test]
fn create_elements_table_with_null_returns_null() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();
    let result = CreateElementsTable(std::ptr::null_mut());
    assert!(result.is_null());
}

#[test]
fn create_element_interactions_with_null_returns_null() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();
    let result = CreateElementInteractions(std::ptr::null_mut());
    assert!(result.is_null());
}

#[test]
fn create_element_interactions_locked_with_null_returns_null() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();
    let result = CreateElementInteractionsLocked(std::ptr::null_mut());
    assert!(result.is_null());
}

// ===== 空表状态查询测试 =====

#[test]
fn destroy_elements_table_does_not_crash() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();
    DestroyElementsTable();
}

#[test]
fn get_element_index_on_empty_table_returns_invalid() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();
    DestroyElementsTable();
    let idx = GetElementIndex(999);
    assert_eq!(idx, INVALID_ELEMENT_INDEX);
}

// ===== CreateElementsTable 完整 round-trip 测试 =====

#[test]
fn create_elements_table_full_round_trip() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();

    // 1. 清空旧表
    DestroyElementsTable();

    // 2. 构造数据并创建元素表
    let data = build_minimal_elements_data();
    let mut reader = BinaryBufferReader::new(&data);
    let result = CreateElementsTable(&mut reader);
    assert!(
        !result.is_null(),
        "CreateElementsTable should return non-null on success"
    );

    // 3. 返回值是 count=2 作为指针（匹配源码 `(void*)(longlong)local_c8`）
    let count = result as usize;
    assert_eq!(count, 2, "returned pointer should encode count=2");

    // 4. 验证 get_element_count_pub
    assert_eq!(get_element_count_pub(), 2);

    // 5. 验证 GetElementIndex(hash=100) == 0, hash=200 == 1
    assert_eq!(GetElementIndex(100), 0);
    assert_eq!(GetElementIndex(200), 1);

    // 6. 验证未注册的 hash 返回 INVALID_ELEMENT_INDEX
    assert_eq!(GetElementIndex(999), INVALID_ELEMENT_INDEX);

    // 7. 清理：DestroyElementsTable 后 count 应归零
    DestroyElementsTable();
    assert_eq!(get_element_count_pub(), 0);
}

#[test]
fn create_elements_table_with_count_zero_returns_null() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();
    DestroyElementsTable();

    let mut w = BinaryBufferWriter::new();
    w.write_int(0);
    let data = w.into_bytes();
    let mut reader = BinaryBufferReader::new(&data);
    let result = CreateElementsTable(&mut reader);
    assert!(result.is_null(), "count=0 should return null (count as ptr = 0)");
    assert_eq!(get_element_count_pub(), 0);
}

// ===== CreateElementInteractions 测试 =====

#[test]
fn create_element_interactions_with_count_zero_returns_null() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();

    // count=0 + pointer=0（8 字节）
    let mut w = BinaryBufferWriter::new();
    w.write_int(0);
    w.write_bytes(&[0u8; 8]);

    let data = w.into_bytes();
    let mut reader = BinaryBufferReader::new(&data);
    let result = CreateElementInteractions(&mut reader);
    assert!(
        result.is_null(),
        "CreateElementInteractions always returns null on success"
    );
}

#[test]
fn create_element_interactions_extracts_gas_obliteration() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();

    // 构造 1 个 GasObliteration entry（32 字节）
    // 字段偏移对照源码 03_elements.c L538-545：
    //   signature @0 (i32)
    //   elem_idx1 @4 (u16)
    //   elem_idx2 @6 (u16)
    //   elem_result_idx @8 (u16)
    //   min_mass @12 (f32)
    //   interaction_probability @16 (f32)
    //   elem1_mass_destruction_percent @20 (f32)
    //   elem2_mass_required_multiplier @24 (f32)
    //   elem_result_mass_creation_multiplier @28 (f32)
    let mut entry = [0u8; 32];
    entry[0..4].copy_from_slice(&GAS_OBLITERATION_SIGNATURE.to_le_bytes());
    entry[4..6].copy_from_slice(&10_u16.to_le_bytes());
    entry[6..8].copy_from_slice(&20_u16.to_le_bytes());
    entry[8..10].copy_from_slice(&30_u16.to_le_bytes());
    entry[12..16].copy_from_slice(&1.5_f32.to_le_bytes());
    entry[16..20].copy_from_slice(&0.25_f32.to_le_bytes());
    entry[20..24].copy_from_slice(&0.5_f32.to_le_bytes());
    entry[24..28].copy_from_slice(&2.0_f32.to_le_bytes());
    entry[28..32].copy_from_slice(&3.0_f32.to_le_bytes());

    // 用 Box 让数据在堆上稳定（跨 FFI 调用期间不被释放）
    let entry_box = Box::new(entry);
    let entry_ptr = Box::into_raw(entry_box) as *const u8;

    // 构造 reader 数据：count=1 + pointer=entry_ptr
    let mut w = BinaryBufferWriter::new();
    w.write_int(1);
    w.write_bytes(&(entry_ptr as usize).to_le_bytes());
    let data = w.into_bytes();

    let mut reader = BinaryBufferReader::new(&data);
    let result = CreateElementInteractions(&mut reader);
    assert!(result.is_null(), "should return null on success");

    // 清理堆内存
    unsafe {
        drop(Box::from_raw(entry_ptr as *mut [u8; 32]));
    }
}

#[test]
fn create_element_interactions_locked_extracts_gas_obliteration() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();

    // 同上，但用 Locked 版本（额外加 G_INTERACTIONS_LOCK）
    let mut entry = [0u8; 32];
    entry[0..4].copy_from_slice(&GAS_OBLITERATION_SIGNATURE.to_le_bytes());
    entry[4..6].copy_from_slice(&11_u16.to_le_bytes());
    entry[6..8].copy_from_slice(&22_u16.to_le_bytes());
    entry[8..10].copy_from_slice(&33_u16.to_le_bytes());

    let entry_box = Box::new(entry);
    let entry_ptr = Box::into_raw(entry_box) as *const u8;

    let mut w = BinaryBufferWriter::new();
    w.write_int(1);
    w.write_bytes(&(entry_ptr as usize).to_le_bytes());
    let data = w.into_bytes();

    let mut reader = BinaryBufferReader::new(&data);
    let result = CreateElementInteractionsLocked(&mut reader);
    assert!(result.is_null());

    unsafe {
        drop(Box::from_raw(entry_ptr as *mut [u8; 32]));
    }
}

#[test]
fn create_element_interactions_ignores_non_matching_signature() {
    let _lock = ELEMENTS_TEST_LOCK.lock().unwrap();

    // 构造 1 个签名不匹配的 entry（signature = 0x12345678）
    let mut entry = [0u8; 32];
    entry[0..4].copy_from_slice(&0x12345678_u32.to_le_bytes());

    let entry_box = Box::new(entry);
    let entry_ptr = Box::into_raw(entry_box) as *const u8;

    let mut w = BinaryBufferWriter::new();
    w.write_int(1);
    w.write_bytes(&(entry_ptr as usize).to_le_bytes());
    let data = w.into_bytes();

    let mut reader = BinaryBufferReader::new(&data);
    let result = CreateElementInteractions(&mut reader);
    assert!(result.is_null(), "should not crash on non-matching signature");

    unsafe {
        drop(Box::from_raw(entry_ptr as *mut [u8; 32]));
    }
}
