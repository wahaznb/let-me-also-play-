//! End-to-end tests for the static REA command.
//!
//! The fixture is a minimal PE32+ image whose `.text` section contains a
//! true 64-bit no-op `mov rax,rax` and a `jmp` to the next instruction. REA
//! must detect both without running the binary, emit a guarded spec, apply it
//! to a copy, and restore the exact original bytes from the manifest.

use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_game-opt-tool");
const TEXT_OFFSET: usize = 0x400;
const ENTRY_VA: u64 = 0x0000_0001_4000_1000;

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

/// Minimal PE32+ image with one executable `.text` section.
fn pe64() -> Vec<u8> {
    const FILE_SIZE: usize = 0x600;
    let mut data = vec![0u8; FILE_SIZE];
    data[0] = b'M';
    data[1] = b'Z';
    data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    data[0x80..0x84].copy_from_slice(b"PE\0\0");

    let coff = 0x84;
    data[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    data[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    data[coff + 16..coff + 18].copy_from_slice(&0xF0u16.to_le_bytes());
    data[coff + 18..coff + 20].copy_from_slice(&0x0022u16.to_le_bytes());

    let optional = 0x98;
    data[optional..optional + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
    data[optional + 4..optional + 8].copy_from_slice(&0x200u32.to_le_bytes());
    data[optional + 16..optional + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 20..optional + 24].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 24..optional + 32]
        .copy_from_slice(&0x0000_0001_4000_0000u64.to_le_bytes());
    data[optional + 32..optional + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 36..optional + 40].copy_from_slice(&0x200u32.to_le_bytes());
    data[optional + 56..optional + 60].copy_from_slice(&0x2000u32.to_le_bytes());
    data[optional + 60..optional + 64].copy_from_slice(&0x400u32.to_le_bytes());
    data[optional + 68..optional + 70].copy_from_slice(&3u16.to_le_bytes());
    let dirs_offset = optional + 108;
    data[dirs_offset..dirs_offset + 4].copy_from_slice(&16u32.to_le_bytes());

    let section = 0x188;
    data[section..section + 5].copy_from_slice(b".text");
    data[section + 8..section + 12].copy_from_slice(&0x100u32.to_le_bytes());
    data[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    data[section + 16..section + 20].copy_from_slice(&0x200u32.to_le_bytes());
    data[section + 20..section + 24].copy_from_slice(&0x400u32.to_le_bytes());
    data[section + 36..section + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());

    // mov rax,rax ; jmp +0 (to the next instruction) ; ret
    data[TEXT_OFFSET..TEXT_OFFSET + 6].copy_from_slice(&[0x48, 0x89, 0xC0, 0xEB, 0x00, 0xC3]);
    data
}

fn write_fixture(dir: &Path) -> std::path::PathBuf {
    let input = dir.join("rea-fixture.exe");
    std::fs::write(&input, pe64()).unwrap();
    input
}

#[test]
fn rea_json_includes_the_candidate_list() {
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path());

    let output = run(&["rea", "--game", input.to_str().unwrap(), "--json"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let report = json(&output);
    assert_eq!(report["candidates_total"], 2);
    assert_eq!(report["candidates_by_class"]["dead_code"], 2);
    assert_eq!(report["candidates_by_kind"]["dead_move"], 1);
    assert_eq!(report["candidates_by_kind"]["dead_jump"], 1);

    let candidates = report["candidates"].as_array().expect("candidate list");
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0]["kind"], "dead_move");
    assert_eq!(candidates[0]["address"], ENTRY_VA);
    assert_eq!(
        candidates[0]["expected"],
        serde_json::json!([0x48, 0x89, 0xC0])
    );
    assert_eq!(
        candidates[0]["replacement"],
        serde_json::json!([0x0F, 0x1F, 0x00])
    );
    assert_eq!(candidates[1]["kind"], "dead_jump");
    assert_eq!(
        candidates[1]["address"],
        ENTRY_VA + 3,
        "the second candidate follows the three-byte move"
    );

    assert!(
        report["patch_spec"].is_string(),
        "the report must include the generated spec"
    );
}

#[test]
fn rea_spec_only_prints_a_guarded_patch_spec() {
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path());

    let output = run(&[
        "rea",
        "--game",
        input.to_str().unwrap(),
        "--spec-only",
        "--json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let spec = json(&output);
    assert_eq!(spec["schema_version"], 1);
    let patches = spec["patches"].as_array().expect("patches");
    assert_eq!(patches.len(), 2);
    assert_eq!(patches[0]["address"], format!("0x{ENTRY_VA:X}"));
    assert_eq!(patches[0]["expected"], "4889c0");
    assert_eq!(patches[0]["replace"], "0f1f00");
    assert_eq!(patches[1]["expected"], "eb00");
    assert_eq!(patches[1]["replace"], "6690");
}

#[test]
fn rea_apply_then_revert_restores_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path());
    let patched = dir.path().join("rea-fixture.patched.exe");
    let manifest = dir.path().join("rea-fixture.patch.json");
    let original = pe64();

    let output = run(&[
        "rea",
        "--game",
        input.to_str().unwrap(),
        "--output",
        patched.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let report = json(&output);
    assert_eq!(report["notes"][0], "Applied 2 patch(es) from REA candidates.");
    assert!(patched.exists());
    assert!(manifest.exists());

    let patched_bytes = std::fs::read(&patched).unwrap();
    assert_ne!(patched_bytes, original);
    assert_eq!(
        &patched_bytes[TEXT_OFFSET..TEXT_OFFSET + 5],
        &[0x0F, 0x1F, 0x00, 0x66, 0x90]
    );

    let output = run(&[
        "rea",
        "--game",
        patched.to_str().unwrap(),
        "--revert",
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let revert = json(&output);
    assert_eq!(revert["patches_reverted"], 2);
    assert_eq!(std::fs::read(&patched).unwrap(), original);
}

#[test]
fn rea_revert_uses_the_game_override_for_a_moved_copy() {
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path());
    let patched = dir.path().join("rea-fixture.patched.exe");
    let moved = dir.path().join("rea-fixture.moved.exe");
    let manifest = dir.path().join("rea-fixture.patch.json");
    let original = pe64();

    let output = run(&[
        "rea",
        "--game",
        input.to_str().unwrap(),
        "--output",
        patched.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    // The manifest records `patched`; reverting the moved copy must use the
    // --game path instead of the stale recorded path.
    std::fs::rename(&patched, &moved).unwrap();

    let output = run(&[
        "rea",
        "--game",
        moved.to_str().unwrap(),
        "--revert",
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(std::fs::read(&moved).unwrap(), original);
}

#[test]
fn rea_refuses_a_target_that_does_not_match_the_spec_guard() {
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path());
    let patched = dir.path().join("rea-fixture.patched.exe");
    let manifest = dir.path().join("rea-fixture.patch.json");

    let output = run(&[
        "rea",
        "--game",
        input.to_str().unwrap(),
        "--output",
        patched.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    // Corrupt a guarded byte, then try to revert. The manifest check must
    // refuse instead of overwriting a file it no longer recognizes.
    let mut bytes = std::fs::read(&patched).unwrap();
    bytes[TEXT_OFFSET] = 0x90;
    std::fs::write(&patched, &bytes).unwrap();

    let output = run(&[
        "rea",
        "--game",
        patched.to_str().unwrap(),
        "--revert",
        "--manifest",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        !output.status.success(),
        "revert must fail after the patched bytes were modified"
    );
    assert_eq!(
        std::fs::read(&patched).unwrap(),
        bytes,
        "a refused revert must not touch the file"
    );
}
