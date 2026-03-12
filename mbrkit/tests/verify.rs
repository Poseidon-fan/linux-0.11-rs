//! Integration tests for the `mbrkit verify` command.

mod common;

use common::*;

/// Confirm `verify` accepts a valid disk image.
#[test]
fn verify_accepts_valid_disk() {
    let temp_dir = tempdir().unwrap();
    let (_, disk_path) = create_single_partition_disk(&temp_dir);

    let output = run_mbrkit(["verify", disk_path.to_str().unwrap()]);
    assert_success(&output);

    let stdout = stdout_text(&output);
    assert!(stdout.contains("Verification: ok"));
}

/// Confirm `verify` fails when partition entries overlap.
#[test]
fn verify_rejects_overlapping_partitions() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("overlap.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
    header.signature = MBR_SIGNATURE;
    header.partitions[0] = partition_entry(0x80, 0x83, 2048, 128);
    header.partitions[1] = partition_entry(0x00, 0x81, 2100, 128);
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "verify".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_exit_code(&output, 2);

    let json = stdout_json(&output);
    assert_eq!(json["ok"], false);
    assert!(diagnostics_contain(&json, "overlap"));
}

/// Confirm `verify` rejects invalid signatures.
#[test]
fn verify_rejects_invalid_signature() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("bad-signature.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
    header.signature = 0;
    header.partitions[0] = partition_entry(0x80, 0x83, 2048, 128);
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "verify".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_exit_code(&output, 2);

    let json = stdout_json(&output);
    assert!(diagnostics_contain(&json, "invalid_signature"));
}

/// Confirm `verify` rejects invalid boot flags.
#[test]
fn verify_rejects_invalid_boot_flags() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("bad-boot-flag.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
    header.signature = MBR_SIGNATURE;
    header.partitions[0] = partition_entry(0x7f, 0x83, 2048, 128);
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "verify".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_exit_code(&output, 2);

    let json = stdout_json(&output);
    assert!(diagnostics_contain(&json, "invalid_boot_flag"));
}

/// Confirm `verify` rejects out-of-bounds partitions.
#[test]
fn verify_rejects_out_of_bounds_partitions() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("out-of-bounds.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
    header.signature = MBR_SIGNATURE;
    header.partitions[0] = partition_entry(0x80, 0x83, 8190, 32);
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "verify".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_exit_code(&output, 2);

    let json = stdout_json(&output);
    assert!(diagnostics_contain(&json, "out_of_bounds"));
}

/// Confirm warnings stay non-fatal without `--strict`.
#[test]
fn verify_non_strict_allows_unknown_partition_types() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("unknown-type.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
    header.signature = MBR_SIGNATURE;
    header.partitions[0] = partition_entry(0x80, 0x99, 2048, 64);
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "verify".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_success(&output);

    let json = stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert!(diagnostics_contain(&json, "unknown_partition_type"));
}

/// Confirm `--strict` promotes warnings to failures.
#[test]
fn verify_strict_rejects_unknown_partition_types() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("unknown-type.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
    header.signature = MBR_SIGNATURE;
    header.partitions[0] = partition_entry(0x80, 0x99, 2048, 64);
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "verify".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
        "--strict".to_string(),
    ]);
    assert_exit_code(&output, 2);

    let json = stdout_json(&output);
    assert_eq!(json["ok"], false);
    assert!(diagnostics_contain(&json, "unknown_partition_type"));
}

/// Confirm `--strict` rejects a zero disk signature warning.
#[test]
fn verify_strict_rejects_zero_disk_signature() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("zero-signature.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0);
    header.signature = MBR_SIGNATURE;
    header.partitions[0] = partition_entry(0x80, 0x83, 2048, 64);
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "verify".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
        "--strict".to_string(),
    ]);
    assert_exit_code(&output, 2);

    let json = stdout_json(&output);
    assert!(diagnostics_contain(&json, "zero_disk_signature"));
}

/// Confirm `--strict` rejects multiple active partitions.
#[test]
fn verify_strict_rejects_multiple_active_partitions() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("multiple-active.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
    header.signature = MBR_SIGNATURE;
    header.partitions[0] = partition_entry(0x80, 0x83, 2048, 64);
    header.partitions[1] = partition_entry(0x80, 0x81, 4096, 64);
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "verify".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
        "--strict".to_string(),
    ]);
    assert_exit_code(&output, 2);

    let json = stdout_json(&output);
    assert!(diagnostics_contain(&json, "multiple_active_partitions"));
}
