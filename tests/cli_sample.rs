//! CLI coverage for `sample`: argument validation, honest failure modes, and
//! the local perf probe. These tests do not need a game or a running process.

use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_game-opt-tool");

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

/// Minimal PE32 image, enough for `GameInfo::from_path` to accept it.
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

#[test]
fn sample_help_documents_the_sampling_contract() {
    let output = run(&["sample", "--help"]);
    assert!(output.status.success(), "help failed: {}", stderr(&output));
    let text = stdout(&output);
    for flag in [
        "--game",
        "--pid",
        "--process",
        "--duration",
        "--frequency",
        "--top",
        "--no-ghidra",
        "--refresh-functions",
        "--json",
    ] {
        assert!(text.contains(flag), "help is missing {flag}: {text}");
    }
    assert!(
        text.contains("CPU time per function"),
        "help should say what is sampled: {text}"
    );
}

#[test]
fn sample_rejects_an_out_of_range_duration() {
    let output = run(&["sample", "--game", "/tmp/missing.exe", "--duration", "0"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("invalid value") || text.contains("0 is not in"),
        "clap should reject duration 0: {text}"
    );
}

#[test]
fn sample_rejects_no_ghidra_plus_refresh() {
    let output = run(&[
        "sample",
        "--game",
        "/tmp/missing.exe",
        "--no-ghidra",
        "--refresh-functions",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot be used with"),
        "flags should conflict: {}",
        stderr(&output)
    );
}

#[test]
fn sample_missing_game_has_a_clear_error() {
    let output = run(&["sample", "--game", "/tmp/definitely-not-a-game.exe"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("Game executable not found") || text.contains("Failed to inspect game"),
        "unexpected error: {text}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn sample_without_a_running_game_explains_the_next_step() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("idle.exe");
    std::fs::write(&game, pe32()).unwrap();

    let output = run(&[
        "sample",
        "--game",
        game.to_str().unwrap(),
        "--duration",
        "1",
        "--no-ghidra",
    ]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("No running process maps"),
        "the error should tell the user to launch the game: {text}"
    );
    assert!(
        text.contains("Launch the game first"),
        "the error should be actionable: {text}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn sample_rejects_pid_zero_before_touching_perf() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.exe");
    std::fs::write(&game, pe32()).unwrap();

    let output = run(&[
        "sample",
        "--game",
        game.to_str().unwrap(),
        "--pid",
        "0",
        "--duration",
        "1",
        "--no-ghidra",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("PID must be greater than zero"),
        "unexpected error: {}",
        stderr(&output)
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn sample_explains_the_platform_limit_on_non_linux() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.exe");
    std::fs::write(&game, pe32()).unwrap();

    let output = run(&[
        "sample",
        "--game",
        game.to_str().unwrap(),
        "--duration",
        "1",
        "--no-ghidra",
    ]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("only implemented on Linux"),
        "non-Linux builds must say why sampling is unavailable: {text}"
    );
}

#[test]
fn doctor_reports_the_sampling_probe() {
    let output = run(&["doctor"]);
    assert!(
        output.status.success(),
        "doctor failed: {}",
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(
        text.contains("perf_event_open"),
        "doctor should probe perf_event_open: {text}"
    );
    assert!(
        text.contains("[ok]") || text.contains("[warn]"),
        "doctor should give a usable status: {text}"
    );
}

#[test]
fn sample_json_is_not_claimed_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("idle.exe");
    std::fs::write(&game, pe32()).unwrap();
    let output = run(&[
        "sample",
        "--game",
        game.to_str().unwrap(),
        "--duration",
        "1",
        "--json",
    ]);
    assert!(!output.status.success());
    assert!(
        Path::new(&game).is_file(),
        "the test fixture should still exist"
    );
    assert!(
        stdout(&output).trim().is_empty(),
        "a failed sample must not print a fake JSON report: {}",
        stdout(&output)
    );
}
