//! End-to-end tests for Ghidra-backed decompilation.
//!
//! The real Ghidra test is ignored by default because it needs a Ghidra
//! installation, a compatible full JDK, and a real PE/ELF test binary.

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_game-opt-tool");

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to run game-opt-tool")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn decompile_rejects_an_invalid_address_before_starting_ghidra() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.exe");
    std::fs::write(&game, b"MZ\x90\x00 fake test executable").unwrap();

    let output = run(&[
        "decompile",
        "--game",
        game.to_str().unwrap(),
        "--address",
        "not-hex",
    ]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Invalid hexadecimal address"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn decompile_rejects_a_missing_game() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.exe");

    let output = run(&["decompile", "--game", missing.to_str().unwrap()]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Game path does not exist"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
#[ignore = "requires GHIDRA_INSTALL_DIR, a compatible JDK, and GAME_OPT_TEST_BINARY"]
fn decompiles_a_real_binary_entry_point() {
    let binary = std::env::var("GAME_OPT_TEST_BINARY")
        .expect("set GAME_OPT_TEST_BINARY to a real PE or ELF binary before running this test");

    let output = run(&["decompile", "--game", &binary, "--json"]);
    assert!(
        output.status.success(),
        "decompile failed: {}",
        stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert!(json["address"].as_u64().unwrap() > 0);
    assert!(!json["name"].as_str().unwrap().is_empty());
    let pseudocode = json["pseudocode"].as_str().unwrap();
    assert!(pseudocode.contains('{'), "pseudocode: {}", pseudocode);
    assert!(
        !pseudocode.to_lowercase().contains("placeholder"),
        "pseudocode: {}",
        pseudocode
    );
}
