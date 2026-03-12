//! Integration tests for the `mbrkit extract` command.

mod common;

use std::fs;

use common::*;

/// Confirm `extract` can write the requested partition payload.
#[test]
fn extract_writes_requested_partition_payload() {
    let temp_dir = tempdir().unwrap();
    let (payload_path, disk_path) = create_single_partition_disk(&temp_dir);
    let extracted_path = temp_dir.path().join("extract.img");

    let output = run_mbrkit([
        "extract".to_string(),
        disk_path.display().to_string(),
        "--partition".to_string(),
        "1".to_string(),
        "--output".to_string(),
        extracted_path.display().to_string(),
    ]);
    assert_success(&output);

    let original = fs::read(&payload_path).unwrap();
    let extracted = fs::read(&extracted_path).unwrap();
    assert_eq!(&extracted[..original.len()], original.as_slice());
    assert_eq!(extracted.len(), 1536);
}

/// Confirm `extract` validates the requested partition index range.
#[test]
fn extract_rejects_invalid_partition_index() {
    let temp_dir = tempdir().unwrap();
    let (_, disk_path) = create_single_partition_disk(&temp_dir);
    let extracted_path = temp_dir.path().join("extract.img");

    let output = run_mbrkit([
        "extract".to_string(),
        disk_path.display().to_string(),
        "--partition".to_string(),
        "0".to_string(),
        "--output".to_string(),
        extracted_path.display().to_string(),
    ]);
    assert_exit_code(&output, 2);
    assert!(stderr_text(&output).contains("range 1..=4"));
}

/// Confirm `extract` rejects empty partition slots.
#[test]
fn extract_rejects_empty_partition_slot() {
    let temp_dir = tempdir().unwrap();
    let (_, disk_path) = create_single_partition_disk(&temp_dir);
    let extracted_path = temp_dir.path().join("extract.img");

    let output = run_mbrkit([
        "extract".to_string(),
        disk_path.display().to_string(),
        "--partition".to_string(),
        "2".to_string(),
        "--output".to_string(),
        extracted_path.display().to_string(),
    ]);
    assert_exit_code(&output, 2);
    assert!(stderr_text(&output).contains("partition 2 is empty"));
}

/// Confirm `extract` requires `--force` before overwriting an existing file.
#[test]
fn extract_requires_force_to_overwrite_output() {
    let temp_dir = tempdir().unwrap();
    let (_, disk_path) = create_single_partition_disk(&temp_dir);
    let extracted_path = temp_dir.path().join("extract.img");
    fs::write(&extracted_path, b"old").unwrap();

    let output = run_mbrkit([
        "extract".to_string(),
        disk_path.display().to_string(),
        "--partition".to_string(),
        "1".to_string(),
        "--output".to_string(),
        extracted_path.display().to_string(),
    ]);
    assert_exit_code(&output, 2);
    assert!(stderr_text(&output).contains("already exists"));

    let forced_output = run_mbrkit([
        "extract".to_string(),
        disk_path.display().to_string(),
        "--partition".to_string(),
        "1".to_string(),
        "--output".to_string(),
        extracted_path.display().to_string(),
        "--force".to_string(),
    ]);
    assert_success(&forced_output);
}

/// Confirm `extract` rejects partitions that extend beyond the disk image.
#[test]
fn extract_rejects_partitions_extending_past_end_of_file() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("broken.img");
    let extracted_path = temp_dir.path().join("extract.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
    header.signature = MBR_SIGNATURE;
    header.partitions[0] = partition_entry(0x80, 0x83, 4096, 8192);
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "extract".to_string(),
        disk_path.display().to_string(),
        "--partition".to_string(),
        "1".to_string(),
        "--output".to_string(),
        extracted_path.display().to_string(),
    ]);
    assert_exit_code(&output, 2);
    assert!(stderr_text(&output).contains("extends past the end of the disk image"));
}
