//! Integration tests for the `mbrkit pack` command.

mod common;

use std::fs;

use common::*;

/// Confirm `pack` writes a valid disk image when explicit flags are used.
#[test]
fn pack_supports_explicit_flags_and_writes_disk_image() {
    let temp_dir = tempdir().unwrap();
    let payload_path = temp_dir.path().join("rootfs.img");
    let disk_path = temp_dir.path().join("disk.img");
    write_payload(&payload_path, 1500, 0x5a);

    let output = run_mbrkit(single_partition_pack_args(&disk_path, &payload_path));
    assert_success(&output);

    assert_eq!(fs::metadata(&disk_path).unwrap().len(), 4 * 1024 * 1024);
}

/// Confirm `pack` also supports manifest-driven input.
#[test]
fn pack_supports_manifest_input() {
    let temp_dir = tempdir().unwrap();
    let payload_path = temp_dir.path().join("swap.img");
    let disk_path = temp_dir.path().join("manifest-disk.img");
    let manifest_path = temp_dir.path().join("disk.toml");
    write_payload(&payload_path, 2048, 0x33);
    write_manifest(&manifest_path, &disk_path, &payload_path);

    let output = run_mbrkit([
        "pack".to_string(),
        "--manifest".to_string(),
        manifest_path.display().to_string(),
    ]);
    assert_success(&output);

    let inspect = run_mbrkit([
        "inspect".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_success(&inspect);

    let json = stdout_json(&inspect);
    assert_eq!(json["partitions"][0]["partition_type"], 130);
}

/// Confirm `pack --dry-run` resolves a layout without creating output files.
#[test]
fn pack_dry_run_prints_layout_without_creating_image() {
    let temp_dir = tempdir().unwrap();
    let payload_path = temp_dir.path().join("rootfs.img");
    let disk_path = temp_dir.path().join("disk.img");
    write_payload(&payload_path, 1024, 0x11);

    let mut args = single_partition_pack_args(&disk_path, &payload_path);
    args.push("--dry-run".into());

    let output = run_mbrkit(args);
    assert_success(&output);

    let stdout = stdout_text(&output);
    assert!(stdout.contains("Output:"));
    assert!(stdout.contains("Slot  Boot  Type"));
    assert!(!disk_path.exists());
}

/// Confirm `pack` refuses to overwrite an existing image without `--force`.
#[test]
fn pack_rejects_existing_output_without_force() {
    let temp_dir = tempdir().unwrap();
    let payload_path = temp_dir.path().join("rootfs.img");
    let disk_path = temp_dir.path().join("disk.img");
    write_payload(&payload_path, 1024, 0x22);

    let output = run_mbrkit(single_partition_pack_args(&disk_path, &payload_path));
    assert_success(&output);

    let second_output = run_mbrkit(single_partition_pack_args(&disk_path, &payload_path));
    assert_exit_code(&second_output, 2);
    assert!(stderr_text(&second_output).contains("already exists"));
}

/// Confirm `pack --force` can replace an existing image.
#[test]
fn pack_force_overwrites_existing_output() {
    let temp_dir = tempdir().unwrap();
    let payload_path = temp_dir.path().join("rootfs.img");
    let disk_path = temp_dir.path().join("disk.img");
    write_payload(&payload_path, 1024, 0x22);

    let output = run_mbrkit(single_partition_pack_args(&disk_path, &payload_path));
    assert_success(&output);

    write_payload(&payload_path, 1024, 0x44);
    let mut args = single_partition_pack_args(&disk_path, &payload_path);
    args.push("--force".into());

    let second_output = run_mbrkit(args);
    assert_success(&second_output);
}

/// Confirm `pack` rejects bootstrap payloads that exceed the supported region.
#[test]
fn pack_rejects_bootstrap_code_larger_than_supported_area() {
    let temp_dir = tempdir().unwrap();
    let payload_path = temp_dir.path().join("rootfs.img");
    let disk_path = temp_dir.path().join("disk.img");
    let boot_code_path = temp_dir.path().join("boot.bin");
    write_payload(&payload_path, 1024, 0x55);
    write_payload(&boot_code_path, MBR_BOOTSTRAP_CODE_SIZE + 1, 0x90);

    let mut args = single_partition_pack_args(&disk_path, &payload_path);
    args.extend(["--boot-code".into(), boot_code_path.display().to_string()]);

    let output = run_mbrkit(args);
    assert_exit_code(&output, 2);
    assert!(stderr_text(&output).contains("exceeds 440 bytes"));
}

/// Confirm `pack` rejects partitions smaller than their source payload.
#[test]
fn pack_rejects_partition_size_smaller_than_payload() {
    let temp_dir = tempdir().unwrap();
    let payload_path = temp_dir.path().join("rootfs.img");
    let disk_path = temp_dir.path().join("disk.img");
    write_payload(&payload_path, 1500, 0x66);

    let args = vec![
        "pack".into(),
        "--output".into(),
        disk_path.display().to_string(),
        "--disk-size".into(),
        "4MiB".into(),
        "--partition".into(),
        format!(
            "file={},type=minix,start=2048,size=1KiB",
            payload_path.display()
        ),
    ];

    let output = run_mbrkit(args);
    assert_exit_code(&output, 2);
    assert!(stderr_text(&output).contains("larger than the declared partition size"));
}

/// Confirm `pack --manifest` surfaces parse failures cleanly.
#[test]
fn pack_rejects_invalid_manifest() {
    let temp_dir = tempdir().unwrap();
    let manifest_path = temp_dir.path().join("disk.toml");
    fs::write(&manifest_path, "this = [is not valid toml").unwrap();

    let output = run_mbrkit([
        "pack".to_string(),
        "--manifest".to_string(),
        manifest_path.display().to_string(),
    ]);
    assert_exit_code(&output, 1);
    assert!(stderr_text(&output).contains("failed to parse manifest"));
}
