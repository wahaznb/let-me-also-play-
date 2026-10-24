//! End-to-end tests for the PE byte-patch and unpatch CLI.
//!
//! The fixture is a minimal PE32 image with one executable `.text` section.
//! These tests exercise argument parsing, JSON reports, manifests, code-cave
//! allocation, and exact rollback through the real compiled binary.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_game-opt-tool");
const ENTRY_VA: u64 = 0x0040_1000;

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to run game-opt-tool")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> Value {
    serde_json::from_str(&stdout(output)).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            stdout(output),
            stderr(output)
        )
    })
}

/// Minimal PE32 image: .text at RVA 0x1000, raw 0x400, entry bytes 55 8B EC.
fn pe32() -> Vec<u8> {
    const FILE_SIZE: usize = 0x600;
    let mut data = vec![0u8; FILE_SIZE];
    data[0] = b'M';
    data[1] = b'Z';
    data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    data[0x80..0x84].copy_from_slice(b"PE\0\0");

    let coff = 0x84;
    data[coff..coff + 2].copy_from_slice(&0x014Cu16.to_le_bytes());
    data[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    data[coff + 16..coff + 18].copy_from_slice(&0xE0u16.to_le_bytes());
    data[coff + 18..coff + 20].copy_from_slice(&0x0102u16.to_le_bytes());

    let optional = 0x98;
    data[optional..optional + 2].copy_from_slice(&0x10Bu16.to_le_bytes());
    data[optional + 4..optional + 8].copy_from_slice(&0x200u32.to_le_bytes());
    data[optional + 16..optional + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 20..optional + 24].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 28..optional + 32].copy_from_slice(&0x0040_0000u32.to_le_bytes());
    data[optional + 32..optional + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 36..optional + 40].copy_from_slice(&0x200u32.to_le_bytes());
    data[optional + 56..optional + 60].copy_from_slice(&0x2000u32.to_le_bytes());
    data[optional + 60..optional + 64].copy_from_slice(&0x400u32.to_le_bytes());
    data[optional + 68..optional + 70].copy_from_slice(&3u16.to_le_bytes());
    data[optional + 92..optional + 96].copy_from_slice(&16u32.to_le_bytes());

    let section = 0x178;
    data[section..section + 5].copy_from_slice(b".text");
    data[section + 8..section + 12].copy_from_slice(&0x100u32.to_le_bytes());
    data[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    data[section + 16..section + 20].copy_from_slice(&0x200u32.to_le_bytes());
    data[section + 20..section + 24].copy_from_slice(&0x400u32.to_le_bytes());
    data[section + 36..section + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());

    data[0x400..0x408].copy_from_slice(&[0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x10, 0x90, 0xC3]);
    data
}

fn write_spec(dir: &Path, name: &str, value: Value) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    path
}

#[test]
fn patch_then_unpatch_roundtrips_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    let output = dir.path().join("patched.exe");
    let original = pe32();
    std::fs::write(&input, &original).unwrap();

    let spec = write_spec(
        dir.path(),
        "patch.json",
        serde_json::json!({
            "schema_version": 1,
            "name": "cli-roundtrip",
            "patches": [{
                "address": format!("0x{ENTRY_VA:X}"),
                "expected": "55",
                "replace": "90",
                "note": "NOP the first entry byte"
            }]
        }),
    );

    let result = run(&[
        "patch",
        "--game",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success(), "patch failed: {}", stderr(&result));
    let report = json(&result);
    assert_eq!(report["patches_applied"], 1);
    assert_eq!(report["sections_added"], 0);
    assert_eq!(report["dry_run"], false);
    assert_eq!(report["manifest_written"], true);

    let manifest = PathBuf::from(report["manifest_path"].as_str().unwrap());
    assert!(manifest.exists());
    let patched = std::fs::read(&output).unwrap();
    assert_eq!(patched[0x400], 0x90);
    assert_eq!(&patched[0x401..0x403], &[0x8B, 0xEC]);

    let result = run(&[
        "unpatch",
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success(), "unpatch failed: {}", stderr(&result));
    let report = json(&result);
    assert_eq!(report["patches_reverted"], 1);
    assert_eq!(report["sections_removed"], 0);
    assert_eq!(std::fs::read(&output).unwrap(), original);
}

#[test]
fn code_cave_patch_is_applied_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    let output = dir.path().join("patched.exe");
    let original = pe32();
    std::fs::write(&input, &original).unwrap();

    let spec = write_spec(
        dir.path(),
        "cave.json",
        serde_json::json!({
            "schema_version": 1,
            "patches": [{
                "address": format!("0x{ENTRY_VA:X}"),
                "expected": "55 8B EC 83 EC",
                "jump": "section:stub"
            }],
            "sections": [{
                "name": "stub",
                "bytes": "CC C3"
            }]
        }),
    );

    let result = run(&[
        "patch",
        "--game",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success(), "patch failed: {}", stderr(&result));
    let report = json(&result);
    assert_eq!(report["patches_applied"], 1);
    assert_eq!(report["sections_added"], 1);
    assert_eq!(report["sections"][0]["address"], 0x0040_2000u64);

    let patched = std::fs::read(&output).unwrap();
    assert_eq!(patched[0x400], 0xE9);
    let rel = i32::from_le_bytes([
        patched[0x401],
        patched[0x402],
        patched[0x403],
        patched[0x404],
    ]);
    assert_eq!(ENTRY_VA as i64 + 5 + rel as i64, 0x0040_2000);

    let manifest = PathBuf::from(report["manifest_path"].as_str().unwrap());
    let result = run(&[
        "unpatch",
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success(), "unpatch failed: {}", stderr(&result));
    assert_eq!(std::fs::read(&output).unwrap(), original);
}

#[test]
fn wrong_expected_bytes_are_refused_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    let output = dir.path().join("patched.exe");
    std::fs::write(&input, pe32()).unwrap();

    let spec = write_spec(
        dir.path(),
        "bad.json",
        serde_json::json!({
            "schema_version": 1,
            "patches": [{
                "address": format!("0x{ENTRY_VA:X}"),
                "expected": "90",
                "replace": "91"
            }]
        }),
    );

    let result = run(&[
        "patch",
        "--game",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(stderr(&result).contains("do not match"), "{}", stderr(&result));
    assert!(!output.exists());
}

#[test]
fn dry_run_does_not_write_output_or_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    let output = dir.path().join("patched.exe");
    std::fs::write(&input, pe32()).unwrap();

    let spec = write_spec(
        dir.path(),
        "patch.json",
        serde_json::json!({
            "schema_version": 1,
            "patches": [{
                "address": format!("0x{ENTRY_VA:X}"),
                "expected": "55",
                "replace": "90"
            }]
        }),
    );

    let result = run(&[
        "patch",
        "--game",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert!(result.status.success(), "dry-run failed: {}", stderr(&result));
    let report = json(&result);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["manifest_written"], false);
    assert!(!output.exists());
    assert!(!PathBuf::from(report["manifest_path"].as_str().unwrap()).exists());
}

#[test]
fn patch_flags_that_need_a_spec_fail_cleanly_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    let output = dir.path().join("patched.exe");
    std::fs::write(&input, pe32()).unwrap();

    let result = run(&[
        "patch",
        "--game",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(!result.status.success());
    assert!(stderr(&result).contains("require --spec"), "{}", stderr(&result));
    assert!(!output.exists());
}

#[test]
fn unpatch_refuses_a_changed_patched_file() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    let output = dir.path().join("patched.exe");
    std::fs::write(&input, pe32()).unwrap();

    let spec = write_spec(
        dir.path(),
        "patch.json",
        serde_json::json!({
            "schema_version": 1,
            "patches": [{
                "address": format!("0x{ENTRY_VA:X}"),
                "expected": "55",
                "replace": "90"
            }]
        }),
    );

    let result = run(&[
        "patch",
        "--game",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success(), "patch failed: {}", stderr(&result));
    let report = json(&result);
    let manifest = PathBuf::from(report["manifest_path"].as_str().unwrap());

    let mut changed = std::fs::read(&output).unwrap();
    changed[0x401] ^= 0xFF;
    std::fs::write(&output, &changed).unwrap();

    let result = run(&[
        "unpatch",
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(stderr(&result).contains("SHA-256 mismatch"), "{}", stderr(&result));
    assert_eq!(std::fs::read(&output).unwrap(), changed);
}

#[test]
fn in_place_patch_auto_backs_up_and_keeps_json_clean() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.exe");
    let original = pe32();
    std::fs::write(&game, &original).unwrap();

    let spec = write_spec(
        dir.path(),
        "patch.json",
        serde_json::json!({
            "schema_version": 1,
            "patches": [{
                "address": format!("0x{ENTRY_VA:X}"),
                "expected": "55",
                "replace": "90"
            }]
        }),
    );

    let result = run(&[
        "patch",
        "--game",
        game.to_str().unwrap(),
        "--output",
        game.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success(), "patch failed: {}", stderr(&result));
    let report = json(&result);
    assert_eq!(report["patches_applied"], 1);
    let manifest = PathBuf::from(report["manifest_path"].as_str().unwrap());
    assert!(manifest.exists());
    assert_eq!(std::fs::read(&game).unwrap()[0x400], 0x90);

    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".bak"))
        .collect();
    assert_eq!(backups.len(), 1, "expected one automatic backup");

    let result = run(&[
        "unpatch",
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success(), "unpatch failed: {}", stderr(&result));
    assert_eq!(std::fs::read(&game).unwrap(), original);
}

#[test]
fn patch_refuses_to_overwrite_its_own_spec() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    std::fs::write(&input, pe32()).unwrap();

    let spec = write_spec(
        dir.path(),
        "patch.json",
        serde_json::json!({
            "schema_version": 1,
            "patches": [{
                "address": format!("0x{ENTRY_VA:X}"),
                "expected": "55",
                "replace": "90"
            }]
        }),
    );
    let spec_before = std::fs::read(&spec).unwrap();

    let result = run(&[
        "patch",
        "--game",
        input.to_str().unwrap(),
        "--output",
        spec.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(
        stderr(&result).contains("over the patch spec"),
        "{}",
        stderr(&result)
    );
    assert_eq!(std::fs::read(&spec).unwrap(), spec_before);
}

#[test]
fn custom_manifest_and_game_override_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    let output = dir.path().join("patched.exe");
    let manifest = dir.path().join("custom-manifest.json");
    let original = pe32();
    std::fs::write(&input, &original).unwrap();

    let spec = write_spec(
        dir.path(),
        "patch.json",
        serde_json::json!({
            "schema_version": 1,
            "patches": [{
                "address": format!("0x{ENTRY_VA:X}"),
                "expected": "55",
                "replace": "90"
            }]
        }),
    );

    let result = run(&[
        "patch",
        "--game",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success(), "patch failed: {}", stderr(&result));
    let report = json(&result);
    assert_eq!(PathBuf::from(report["manifest_path"].as_str().unwrap()), manifest);
    assert!(manifest.exists());

    let moved = dir.path().join("moved.exe");
    std::fs::rename(&output, &moved).unwrap();
    let result = run(&[
        "unpatch",
        "--manifest",
        manifest.to_str().unwrap(),
        "--game",
        moved.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success(), "unpatch failed: {}", stderr(&result));
    assert_eq!(std::fs::read(&moved).unwrap(), original);
}
