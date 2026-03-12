//! Shared helpers for `mbrkit` integration tests.
#![allow(dead_code, unused_imports)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

pub use mbrkit::mbr::{
    MBR_BOOTSTRAP_CODE_SIZE, MBR_SIGNATURE, MbrHeader, PartitionEntry, SECTOR_SIZE,
};
pub use tempfile::{TempDir, tempdir};

/// Execute the `mbrkit` binary with arbitrary argument types.
pub fn run_mbrkit<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_mbrkit"))
        .args(args)
        .output()
        .unwrap()
}

/// Decode stdout as UTF-8 text.
pub fn stdout_text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// Decode stderr as UTF-8 text.
pub fn stderr_text(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// Parse stdout as JSON.
pub fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Assert that a command succeeded and print rich context on failure.
pub fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "expected success\nstdout:\n{}\nstderr:\n{}",
        stdout_text(output),
        stderr_text(output)
    );
}

/// Assert that a command exited with the expected code.
pub fn assert_exit_code(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "unexpected exit status\nstdout:\n{}\nstderr:\n{}",
        stdout_text(output),
        stderr_text(output)
    );
}

/// Check whether a JSON report contains a diagnostic code.
pub fn diagnostics_contain(json: &Value, code: &str) -> bool {
    json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["code"] == code)
}

/// Write a deterministic payload file filled with one byte pattern.
pub fn write_payload(path: &Path, size: usize, fill: u8) {
    fs::write(path, vec![fill; size]).unwrap();
}

/// Build the standard explicit `pack` argument vector used by many tests.
pub fn single_partition_pack_args(disk_path: &Path, payload_path: &Path) -> Vec<String> {
    vec![
        "pack".into(),
        "--output".into(),
        disk_path.display().to_string(),
        "--disk-size".into(),
        "4MiB".into(),
        "--disk-signature".into(),
        "0x12345678".into(),
        "--partition".into(),
        format!(
            "file={},type=minix,bootable,start=2048",
            payload_path.display()
        ),
    ]
}

/// Create a valid single-partition disk image and return key paths.
pub fn create_single_partition_disk(temp_dir: &TempDir) -> (PathBuf, PathBuf) {
    let payload_path = temp_dir.path().join("rootfs.img");
    let disk_path = temp_dir.path().join("disk.img");
    write_payload(&payload_path, 1500, 0x5a);

    let output = run_mbrkit(single_partition_pack_args(&disk_path, &payload_path));
    assert_success(&output);

    (payload_path, disk_path)
}

/// Write a TOML manifest with one partition.
pub fn write_manifest(path: &Path, disk_path: &Path, payload_path: &Path) {
    fs::write(
        path,
        format!(
            r#"
output = "{}"
disk_size = "4MiB"
disk_signature = "0x12345678"

[[partition]]
file = "{}"
type = "linux_swap"
bootable = true
start_lba = 2048
"#,
            disk_path.display(),
            payload_path.display()
        ),
    )
    .unwrap();
}

/// Create a partition entry for synthetic disk images.
pub fn partition_entry(
    boot_indicator: u8,
    partition_type: u8,
    starting_lba: u32,
    sectors: u32,
) -> PartitionEntry {
    PartitionEntry {
        boot_indicator,
        partition_type,
        starting_lba,
        sectors,
        ..PartitionEntry::default()
    }
}

/// Write a raw disk image that starts with the supplied MBR sector.
pub fn write_disk_image(path: &Path, header: &MbrHeader, disk_size: usize) {
    let mut image = vec![0_u8; disk_size];
    image[..SECTOR_SIZE].copy_from_slice(&header.to_sector());
    fs::write(path, image).unwrap();
}
