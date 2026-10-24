//! End-to-end tests for A/B frame-time comparison.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_game-opt-tool");

const BASELINE: &str = "fps,frametime\n\
100,10.0\n\
100,10.0\n\
100,10.0\n\
50,20.0\n";

const PATCHED: &str = "fps,frametime\n\
125,8.0\n\
125,8.0\n\
125,8.0\n\
62.5,16.0\n";

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

fn write_log(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn compare_reports_improvements_and_notes_short_runs() {
    let dir = tempfile::tempdir().unwrap();
    let baseline = write_log(dir.path(), "baseline.csv", BASELINE);
    let patched = write_log(dir.path(), "patched.csv", PATCHED);

    let output = run(&[
        "compare",
        "--baseline",
        baseline.to_str().unwrap(),
        "--patched",
        patched.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "compare failed: {}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("Baseline:"), "output: {}", text);
    assert!(text.contains("Average FPS"), "output: {}", text);
    assert!(text.contains("80.00"), "output: {}", text);
    assert!(text.contains("100.00"), "output: {}", text);
    assert!(text.contains("+25.00%"), "output: {}", text);
    assert!(text.contains("p99 frame time (ms)"), "output: {}", text);
    assert!(text.contains("-20.00%"), "output: {}", text);
    assert!(text.contains("better"), "output: {}", text);
    assert!(
        text.contains("fewer than 100 frames"),
        "missing short-run warning: {}",
        text
    );
    assert!(
        text.contains("not a statistical significance test"),
        "missing caveat: {}",
        text
    );
}

#[test]
fn compare_json_is_machine_readable() {
    let dir = tempfile::tempdir().unwrap();
    let baseline = write_log(dir.path(), "baseline.csv", BASELINE);
    let patched = write_log(dir.path(), "patched.csv", PATCHED);

    let output = run(&[
        "compare",
        "--baseline",
        baseline.to_str().unwrap(),
        "--patched",
        patched.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "compare failed: {}", stderr(&output));

    let json: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("stdout should be valid JSON");
    assert_eq!(json["baseline"]["stats"]["frame_count"], 4);
    assert_eq!(json["patched"]["stats"]["frame_count"], 4);
    assert!((json["avg_fps"]["percent"].as_f64().unwrap() - 25.0).abs() < 1e-9);
    assert_eq!(json["avg_fps"]["improved"], true);
    assert!((json["p99_frame_time_ms"]["percent"].as_f64().unwrap() + 20.0).abs() < 1e-9);
    assert_eq!(json["p99_frame_time_ms"]["improved"], true);
    assert!(json["warnings"].as_array().unwrap().len() >= 2);
}

#[test]
fn compare_frames_trims_both_logs() {
    let dir = tempfile::tempdir().unwrap();
    let baseline = write_log(dir.path(), "baseline.csv", BASELINE);
    let patched = write_log(dir.path(), "patched.csv", PATCHED);

    let output = run(&[
        "compare",
        "--baseline",
        baseline.to_str().unwrap(),
        "--patched",
        patched.to_str().unwrap(),
        "--frames",
        "2",
    ]);
    assert!(output.status.success(), "compare failed: {}", stderr(&output));

    let text = stdout(&output);
    // Last two baseline frames: 10 ms + 20 ms -> 66.67 FPS.
    // Last two patched frames: 8 ms + 16 ms -> 83.33 FPS.
    assert!(text.contains("66.67"), "output: {}", text);
    assert!(text.contains("83.33"), "output: {}", text);
    assert!(
        text.contains("trimmed to their last 2 frames"),
        "missing tail note: {}",
        text
    );
}

#[test]
fn compare_rejects_the_same_log_for_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    let log = write_log(dir.path(), "same.csv", BASELINE);

    let output = run(&[
        "compare",
        "--baseline",
        log.to_str().unwrap(),
        "--patched",
        log.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("same file"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn compare_rejects_a_missing_log() {
    let dir = tempfile::tempdir().unwrap();
    let baseline = write_log(dir.path(), "baseline.csv", BASELINE);
    let missing = dir.path().join("missing.csv");

    let output = run(&[
        "compare",
        "--baseline",
        baseline.to_str().unwrap(),
        "--patched",
        missing.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Failed to resolve log"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn compare_rejects_non_mangohud_logs() {
    let dir = tempfile::tempdir().unwrap();
    let baseline = write_log(dir.path(), "baseline.csv", "a,b\n1,2\n");
    let patched = write_log(dir.path(), "patched.csv", PATCHED);

    let output = run(&[
        "compare",
        "--baseline",
        baseline.to_str().unwrap(),
        "--patched",
        patched.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no 'frametime' or 'fps' column"),
        "stderr: {}",
        stderr(&output)
    );
}
