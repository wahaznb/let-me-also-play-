//! End-to-end tests for the backup/revert CLI.
//!
//! These run the real compiled binary, so they cover argument parsing,
//! manifest writing, checksum verification, and atomic restore together.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_game-opt-tool");

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to run game-opt-tool")
}

fn backup_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("bak"))
        .collect();
    files.sort();
    files
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn backup_then_revert_restores_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.exe");
    let original = b"MZ\x90\x00 original executable bytes".to_vec();
    std::fs::write(&game, &original).unwrap();

    let output = run(&["backup", "--game", game.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "backup failed: {}",
        stderr(&output)
    );
    assert_eq!(backup_files(dir.path()).len(), 1, "one backup expected");

    std::fs::write(&game, b"a bad patch corrupted this file").unwrap();

    let output = run(&["revert", "--game", game.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "revert failed: {}",
        stderr(&output)
    );
    assert_eq!(std::fs::read(&game).unwrap(), original);
    assert!(stdout(&output).contains("verified"));
}

#[test]
fn revert_without_backup_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.exe");
    std::fs::write(&game, b"bytes").unwrap();

    let output = run(&["revert", "--game", game.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("No backups found"));
    assert_eq!(std::fs::read(&game).unwrap(), b"bytes");
}

#[test]
fn corrupted_backup_is_refused_and_target_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.exe");
    std::fs::write(&game, b"original bytes").unwrap();

    assert!(run(&["backup", "--game", game.to_str().unwrap()])
        .status
        .success());

    let backups = backup_files(dir.path());
    assert_eq!(backups.len(), 1);
    std::fs::write(&backups[0], b"tampered").unwrap();
    std::fs::write(&game, b"patched bytes").unwrap();

    let output = run(&["revert", "--game", game.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("checksum") || stderr(&output).contains("size mismatch"),
        "unexpected error: {}",
        stderr(&output)
    );
    assert_eq!(std::fs::read(&game).unwrap(), b"patched bytes");
}

#[test]
fn dry_run_reports_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.exe");
    std::fs::write(&game, b"original").unwrap();
    assert!(run(&["backup", "--game", game.to_str().unwrap()])
        .status
        .success());

    std::fs::write(&game, b"patched").unwrap();

    let output = run(&[
        "revert",
        "--game",
        game.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("dry-run"));
    assert_eq!(std::fs::read(&game).unwrap(), b"patched");
}

#[test]
fn backup_list_and_verify_work() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.exe");
    std::fs::write(&game, b"original").unwrap();

    assert!(run(&["backup", "--game", game.to_str().unwrap()])
        .status
        .success());

    let output = run(&["backup", "--game", game.to_str().unwrap(), "--list"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("Backups for"));

    let output = run(&["backup", "--game", game.to_str().unwrap(), "--verify"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("verified"));

    let backups = backup_files(dir.path());
    std::fs::write(&backups[0], b"corrupted").unwrap();

    let output = run(&["backup", "--game", game.to_str().unwrap(), "--verify"]);
    assert!(!output.status.success());
    assert!(stdout(&output).contains("[fail]"));
}

#[test]
fn analyze_no_longer_fails_on_config_parse() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("mystery-game.bin");
    // Not a real executable: format detection should degrade gracefully,
    // and config parsing must not abort the command.
    std::fs::write(&game, b"not an executable").unwrap();

    let output = run(&["analyze", "--game", game.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "analyze failed: {}",
        stderr(&output)
    );
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert!(!combined.contains("Failed to parse"));
    assert!(!combined.contains("invalid type"));
}

#[test]
fn doctor_reports_required_checks() {
    let output = run(&["doctor"]);
    assert!(
        output.status.success(),
        "doctor failed: {}",
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("config:"));
    assert!(text.contains("All required checks passed"));
}

#[test]
fn game_help_accepts_hyphenated_names() {
    let output = run(&["help", "deus-ex"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("Unknown game"), "unexpected output: {}", text);
    assert!(text.contains("Deus Ex"));
}
