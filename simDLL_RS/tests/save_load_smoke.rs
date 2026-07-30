//! Phase 3 集成测试：模拟 C# P/Invoke 调用 13 个存档加载函数。
//!
//! 运行：cargo test --test save_load_smoke
//!
//! 对照 Phase 2 的 elements_smoke.rs 模式：验证不 panic + 边界条件。

use SimDLL::save_load::*;
use SimDLL::buffer::{BinaryBufferReader, BinaryBufferWriter};

#[test]
fn load_with_empty_buffer_returns_invalid_magic() {
    let buf = [0u8; 0];
    let mut reader = BinaryBufferReader::new(&buf);
    let result = load(&mut reader);
    assert_eq!(result, Err(SimDLL::buffer::BufferError::InvalidMagic));
}

#[test]
fn load_with_valid_magic_high_version_does_not_panic() {
    let mut w = BinaryBufferWriter::new();
    w.write_bytes(&[0x53, 0x49, 0x4d, 0x53, 0x41, 0x56, 0x45, 0x00]).unwrap();
    w.write_int(0x10).unwrap();
    let buf = w.into_bytes();
    let mut reader = BinaryBufferReader::new(&buf);
    let _ = load(&mut reader);
}

#[test]
fn start_does_not_panic() {
    let buf = Vec::new();
    let mut reader = BinaryBufferReader::new(&buf);
    let _ = start(&mut reader);
}

#[test]
fn allocate_cells_does_not_panic() {
    let mut w = BinaryBufferWriter::new();
    w.write_int(4).unwrap();
    w.write_int(4).unwrap();
    w.write_byte(0).unwrap();
    w.write_byte(0).unwrap();
    let buf = w.into_bytes();
    let mut reader = BinaryBufferReader::new(&buf);
    let _ = allocate_cells(&mut reader);
}

#[test]
fn define_world_offsets_does_not_panic() {
    let mut w = BinaryBufferWriter::new();
    w.write_int(0).unwrap();  // count=0
    let buf = w.into_bytes();
    let mut reader = BinaryBufferReader::new(&buf);
    let _ = define_world_offsets(&mut reader);
}

#[test]
fn prepare_game_data_does_not_panic() {
    let buf = Vec::new();
    let mut reader = BinaryBufferReader::new(&buf);
    let _ = prepare_game_data(&mut reader);
}

#[test]
fn clear_unoccupied_cells_does_not_panic() {
    let buf = Vec::new();
    let mut reader = BinaryBufferReader::new(&buf);
    let _ = clear_unoccupied_cells(&mut reader);
}

#[test]
fn set_saved_options_does_not_panic() {
    let mut w = BinaryBufferWriter::new();
    w.write_byte(0x00).unwrap();
    w.write_byte(0x01).unwrap();
    let buf = w.into_bytes();
    let mut reader = BinaryBufferReader::new(&buf);
    let _ = set_saved_options(&mut reader);
}

#[test]
fn resize_and_initialize_vacuum_cells_does_not_panic() {
    // 先 allocate_cells
    let mut w = BinaryBufferWriter::new();
    w.write_int(2).unwrap();
    w.write_int(2).unwrap();
    w.write_byte(0).unwrap();
    w.write_byte(0).unwrap();
    let buf = w.into_bytes();
    let mut reader = BinaryBufferReader::new(&buf);
    let _ = allocate_cells(&mut reader);

    // 调用 resize_and_initialize_vacuum_cells
    let mut w2 = BinaryBufferWriter::new();
    for _ in 0..6 {
        w2.write_int(0).unwrap();
    }
    let buf2 = w2.into_bytes();
    let mut reader2 = BinaryBufferReader::new(&buf2);
    let _ = resize_and_initialize_vacuum_cells(&mut reader2);
}

#[test]
fn create_disease_table_does_not_panic() {
    let buf = Vec::new();
    let mut reader = BinaryBufferReader::new(&buf);
    let _ = create_disease_table(&mut reader);
}

#[test]
fn cleanup_does_not_panic() {
    cleanup();
    // 多次调用应幂等
    cleanup();
}

#[test]
fn game_sync_function_does_not_panic() {
    let buf = Vec::new();
    let mut reader = BinaryBufferReader::new(&buf);
    game_sync_function(&mut reader);
}

#[test]
fn prepare_game_data_update_does_not_panic() {
    prepare_game_data_update();
}
