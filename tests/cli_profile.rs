//! End-to-end tests for MangoHud frame-time ingestion.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_game-opt-tool");

const FOUR_FRAMES: &str = "fps,frametime,cpu_load\n\
100,10.0,20\n\
100,10.0,20\n\
100,10.0,20\n\
50,20.0,30\n";

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

fn make_game(dir: &Path) -> PathBuf {
    let game = dir.join("game.exe");
    std::fs::write(&game, b"MZ\x90\x00 fake test executable").unwrap();
    game
}

fn make_log(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn profile_computes_real_frame_time_metrics() {
    let dir = tempfile::tempdir().unwrap();
    let game = make_game(dir.path());
    let log = make_log(dir.path(), "MangoHud_deusex.csv", FOUR_FRAMES);

    let output = run(&[
        "profile",
        "--game",
        game.to_str().unwrap(),
        "--log",
        log.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "profile failed: {}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("Frames: 4 (0.05s)"), "output: {}", text);
    assert!(text.contains("Average FPS: 80.00"), "output: {}", text);
    assert!(text.contains("Min / max FPS: 50.00 / 100.00"), "output: {}", text);
    assert!(text.contains("1% low FPS: 50.00"), "output: {}", text);
    assert!(text.contains("0.1% low FPS: 50.00"), "output: {}", text);
    assert!(text.contains("Frame time avg / median: 12.50ms / 10.00ms"));
    assert!(text.contains("Frame time p95 / p99: 18.50ms / 19.70ms"));
    assert!(text.contains("Stutter ratio (p99 / median): 1.97x"));
    assert!(!text.to_lowercase().contains("synthetic"));
    assert!(!text.to_lowercase().contains("placeholder"));
}

#[test]
fn profile_frames_flag_analyzes_only_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let game = make_game(dir.path());
    let log = make_log(dir.path(), "MangoHud_tail.csv", FOUR_FRAMES);

    let output = run(&[
        "profile",
        "--game",
        game.to_str().unwrap(),
        "--log",
        log.to_str().unwrap(),
        "--frames",
        "2",
    ]);
    assert!(output.status.success(), "profile failed: {}", stderr(&output));

    let text = stdout(&output);
    // Last two frames are 10 ms and 20 ms: 15 ms average, 66.67 FPS.
    assert!(text.contains("Frames: 2 (0.03s)"), "output: {}", text);
    assert!(text.contains("Average FPS: 66.67"), "output: {}", text);
}

#[test]
fn profile_json_output_is_machine_readable() {
    let dir = tempfile::tempdir().unwrap();
    let game = make_game(dir.path());
    let log = make_log(dir.path(), "MangoHud_json.csv", FOUR_FRAMES);

    let output = run(&[
        "profile",
        "--game",
        game.to_str().unwrap(),
        "--log",
        log.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "profile failed: {}", stderr(&output));

    let json: serde_json::Value = serde_json::from_str(&stdout(&output))
        .expect("stdout should be valid JSON");
    let stats = &json["stats"];
    assert_eq!(stats["frame_count"], 4);
    assert!((stats["avg_fps"].as_f64().unwrap() - 80.0).abs() < 1e-9);
    assert!((stats["low_1pct_fps"].as_f64().unwrap() - 50.0).abs() < 1e-9);
    assert!(json["source"].as_str().unwrap().ends_with("MangoHud_json.csv"));
}

#[test]
fn profile_discovers_newest_log_in_directory() {
    let dir = tempfile::tempdir().unwrap();
    let game = make_game(dir.path());

    make_log(
        dir.path(),
        "older.csv",
        "fps,frametime\n100,10\n100,10\n",
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    let newest = make_log(
        dir.path(),
        "newest.csv",
        "fps,frametime\n100,20\n100,20\n",
    );
    // A normal CSV must not be mistaken for a MangoHud log.
    make_log(dir.path(), "notes.csv", "a,b\n1,2\n");

    let output = run(&[
        "profile",
        "--game",
        game.to_str().unwrap(),
        "--log-dir",
        dir.path().to_str().unwrap(),
    ]);
    assert!(output.status.success(), "profile failed: {}", stderr(&output));

    let text = stdout(&output);
    assert!(
        text.contains(&format!("Source: {}", newest.display())),
        "output: {}",
        text
    );
    assert!(text.contains("Average FPS: 50.00"), "output: {}", text);
}

#[test]
fn profile_without_logs_fails_with_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let game = make_game(dir.path());

    let output = run(&[
        "profile",
        "--game",
        game.to_str().unwrap(),
        "--log-dir",
        empty.path().to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("No MangoHud CSV logs found"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn profile_rejects_a_csv_without_measurement_columns() {
    let dir = tempfile::tempdir().unwrap();
    let game = make_game(dir.path());
    let log = make_log(dir.path(), "not-mangohud.csv", "a,b\n1,2\n");

    let output = run(&[
        "profile",
        "--game",
        game.to_str().unwrap(),
        "--log",
        log.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no 'frametime' or 'fps' column"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn analyze_summarizes_a_real_log() {
    let dir = tempfile::tempdir().unwrap();
    let game = make_game(dir.path());
    let log = make_log(dir.path(), "MangoHud_analyze.csv", FOUR_FRAMES);

    let output = run(&[
        "analyze",
        "--game",
        game.to_str().unwrap(),
        "--log",
        log.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "analyze failed: {}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("Detected game:"), "output: {}", text);
    assert!(text.contains("1% low FPS: 50.00"), "output: {}", text);
    assert!(
        text.contains("run `game-opt-tool sample --game <exe>`"),
        "output: {}",
        text
    );
    assert!(!text.to_lowercase().contains("synthetic"));
}

#[test]
fn analyze_with_explicit_empty_log_dir_fails() {
    let dir = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let game = make_game(dir.path());

    let output = run(&[
        "analyze",
        "--game",
        game.to_str().unwrap(),
        "--log-dir",
        empty.path().to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("No MangoHud CSV logs found"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn analyze_without_any_log_suggests_mangohud_command() {
    let dir = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let game = make_game(dir.path());

    // No --log/--log-dir: analyze searches the current directory. Run from an
    // empty directory so a developer's own logs cannot influence the result.
    let output = Command::new(BIN)
        .current_dir(empty.path())
        .args(["analyze", "--game", game.to_str().unwrap()])
        .output()
        .expect("failed to run game-opt-tool");
    assert!(output.status.success(), "analyze failed: {}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("No measurement log found"), "output: {}", text);
    assert!(text.contains("MANGOHUD_CONFIG"), "output: {}", text);
    assert!(text.contains("--log-dir"), "output: {}", text);
}
