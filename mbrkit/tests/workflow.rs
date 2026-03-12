//! End-to-end integration tests that span multiple `mbrkit` commands.

mod common;

use std::fs;

use common::*;

/// Confirm the full `pack -> inspect -> extract -> verify` workflow remains stable.
#[test]
fn pack_inspect_extract_and_verify_form_a_closed_loop() {
    let temp_dir = tempdir().unwrap();
    let (payload_path, disk_path) = create_single_partition_disk(&temp_dir);
    let extracted_path = temp_dir.path().join("extract.img");

    let inspect = run_mbrkit([
        "inspect".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_success(&inspect);
    let inspect_json = stdout_json(&inspect);
    assert_eq!(inspect_json["mbr_signature_valid"], true);
    assert_eq!(inspect_json["partitions"][0]["partition_type"], 129);

    let extract = run_mbrkit([
        "extract".to_string(),
        disk_path.display().to_string(),
        "--partition".to_string(),
        "1".to_string(),
        "--output".to_string(),
        extracted_path.display().to_string(),
    ]);
    assert_success(&extract);
    let extracted = fs::read(&extracted_path).unwrap();
    let original = fs::read(&payload_path).unwrap();
    assert_eq!(&extracted[..original.len()], original.as_slice());

    let verify = run_mbrkit([
        "verify".to_string(),
        disk_path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_success(&verify);
    let verify_json = stdout_json(&verify);
    assert_eq!(verify_json["ok"], true);
}
