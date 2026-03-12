//! Integration tests for the `mbrkit inspect` command.

mod common;

use std::fs;

use common::*;

/// Confirm `inspect` reports a valid disk in table mode.
#[test]
fn inspect_reports_valid_disk_in_table_format() {
    let temp_dir = tempdir().unwrap();
    let (_, disk_path) = create_single_partition_disk(&temp_dir);

    let output = run_mbrkit(["inspect", disk_path.to_str().unwrap()]);
    assert_success(&output);

    let stdout = stdout_text(&output);
    assert!(stdout.contains("MBR signature: valid"));
    assert!(stdout.contains("Slot  Boot  Type"));
    assert!(stdout.contains("0x81"));
}

/// Confirm `inspect` still succeeds on an invalid signature and reports it.
#[test]
fn inspect_reports_invalid_signature_without_failing() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("broken.img");
    let mut header = MbrHeader::new([0_u8; MBR_BOOTSTRAP_CODE_SIZE], 0);
    header.signature = 0;
    write_disk_image(&disk_path, &header, 4 * 1024 * 1024);

    let output = run_mbrkit([
        "inspect".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_success(&output);

    let json = stdout_json(&output);
    assert_eq!(json["mbr_signature_valid"], false);
    assert_eq!(json["diagnostics"][0]["code"], "invalid_signature");
}

/// Confirm `inspect` rejects images that are smaller than one sector.
#[test]
fn inspect_rejects_images_smaller_than_one_sector() {
    let temp_dir = tempdir().unwrap();
    let disk_path = temp_dir.path().join("short.img");
    fs::write(&disk_path, vec![0_u8; 128]).unwrap();

    let output = run_mbrkit(["inspect", disk_path.to_str().unwrap()]);
    assert_exit_code(&output, 1);
    assert!(stderr_text(&output).contains("smaller than one sector"));
}
