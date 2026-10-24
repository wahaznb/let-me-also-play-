//! Frame-time profiling from MangoHud CSV logs.
//!
//! MangoHud writes one row per frame. Only the `frametime` column (milliseconds)
//! is required; if it is absent, the `fps` column is used and converted.
//!
//! Metrics are computed from measured frames only:
//! - average FPS = 1000 / mean frame time
//! - 1% low FPS = 1000 / mean of the slowest 1% of frame times
//! - 0.1% low FPS = same for the slowest 0.1%
//! - percentiles use linear interpolation
//! - stutter ratio = p99 frame time / median frame time

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tracing::info;

/// How much of a log file is inspected when checking whether it is MangoHud.
const HEADER_PROBE_LINES: usize = 20;

/// Aggregated frame-time statistics derived from measured frames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrameTimeStats {
    pub frame_count: usize,
    pub duration_seconds: f64,
    pub avg_fps: f64,
    pub min_fps: f64,
    pub max_fps: f64,
    pub avg_frame_time_ms: f64,
    pub median_frame_time_ms: f64,
    pub p95_frame_time_ms: f64,
    pub p99_frame_time_ms: f64,
    pub low_1pct_fps: f64,
    pub low_0_1pct_fps: f64,
    pub stutter_ratio: f64,
}

/// Result of profiling one MangoHud log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileResults {
    pub source: PathBuf,
    pub stats: FrameTimeStats,
}

/// Profiler for measured frame-time logs.
pub struct Profiler;

impl Profiler {
    pub fn new() -> Self {
        Self
    }

    /// Parse a MangoHud CSV log and compute statistics.
    ///
    /// `tail_frames == 0` analyzes the whole log; a positive value analyzes
    /// only the last N frames.
    pub fn profile_csv(
        &self,
        log_path: &Path,
        tail_frames: usize,
    ) -> Result<ProfileResults> {
        let file = File::open(log_path)
            .with_context(|| format!("Failed to open log {}", log_path.display()))?;
        let frame_times =
            parse_mangohud_reader(BufReader::new(file), tail_frames).with_context(|| {
                format!("Failed to parse MangoHud log {}", log_path.display())
            })?;
        let stats = compute_stats(&frame_times)?;

        info!(
            "Parsed {} measured frames from {}",
            stats.frame_count,
            log_path.display()
        );

        Ok(ProfileResults {
            source: log_path.to_path_buf(),
            stats,
        })
    }

    /// Find the newest MangoHud CSV/log in a directory.
    pub fn find_latest_log(dir: &Path) -> Result<PathBuf> {
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("Failed to read log directory {}", dir.display()))?;

        let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let is_log = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("csv") || ext.eq_ignore_ascii_case("log"))
                .unwrap_or(false);
            if !is_log || !has_mangohud_header(&path) {
                continue;
            }

            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            candidates.push((modified, path));
        }

        candidates.sort_by_key(|entry| std::cmp::Reverse(entry.0));

        candidates
            .into_iter()
            .next()
            .map(|(_, path)| path)
            .with_context(|| {
                format!(
                    "No MangoHud CSV logs found in {}. Run the game with \
                     MANGOHUD_CONFIG=\"output_folder=...,autostart_log=1,log_duration=0\"",
                    dir.display()
                )
            })
    }

    /// Serialize results as pretty JSON.
    pub fn export_json(&self, results: &ProfileResults) -> Result<String> {
        serde_json::to_string_pretty(results)
            .context("Failed to serialize profile results")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ColumnKind {
    FrameTime,
    Fps,
}

/// Parse a MangoHud CSV from any buffered reader.
fn parse_mangohud_reader<R: BufRead>(
    mut reader: R,
    tail_frames: usize,
) -> Result<Vec<f64>> {
    let mut header: Option<(usize, ColumnKind)> = None;
    let mut first_header_error: Option<String> = None;
    let mut prologue_lines = 0usize;
    let mut all_frames: Vec<f64> = Vec::new();
    let mut tail: Option<VecDeque<f64>> = if tail_frames > 0 {
        Some(VecDeque::with_capacity(tail_frames))
    } else {
        None
    };

    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let (index, kind) = match header {
            Some(header) => header,
            None => {
                // MangoHud writes metadata rows (os, cpu, gpu, ...) before
                // the real `fps,frametime,...` header. Scan past them.
                match parse_header(trimmed) {
                    Ok(parsed) => {
                        header = Some(parsed);
                    }
                    Err(error) => {
                        first_header_error
                            .get_or_insert_with(|| error.to_string());
                        prologue_lines += 1;
                        if prologue_lines >= HEADER_PROBE_LINES {
                            bail!(
                                "No MangoHud header row containing 'frametime' or 'fps' \
                                 found in the first {} lines",
                                HEADER_PROBE_LINES
                            );
                        }
                    }
                }
                continue;
            }
        };

        let fields = split_csv_line(trimmed);
        let Some(raw) = fields.get(index) else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(value) = raw.parse::<f64>() else {
            continue;
        };

        let frame_time_ms = match kind {
            ColumnKind::FrameTime => value,
            ColumnKind::Fps => {
                if value <= 0.0 {
                    continue;
                }
                1000.0 / value
            }
        };

        if !frame_time_ms.is_finite() || frame_time_ms <= 0.0 {
            continue;
        }

        match &mut tail {
            Some(buffer) => {
                if buffer.len() == tail_frames {
                    buffer.pop_front();
                }
                buffer.push_back(frame_time_ms);
            }
            None => all_frames.push(frame_time_ms),
        }
    }

    if header.is_none() {
        if let Some(message) = first_header_error {
            bail!("No MangoHud header found: {}", message);
        }
        bail!("File has no CSV header row");
    }

    let frames: Vec<f64> = match tail {
        Some(buffer) => buffer.into_iter().collect(),
        None => all_frames,
    };

    if frames.is_empty() {
        bail!("No valid frame-time samples found");
    }

    Ok(frames)
}

fn parse_header(line: &str) -> Result<(usize, ColumnKind)> {
    let fields = split_csv_line(line);
    let normalized: Vec<String> = fields.iter().map(|field| normalize_header(field)).collect();

    if let Some(index) = normalized.iter().position(|name| {
        matches!(
            name.as_str(),
            "frametime" | "frametimems" | "frametimemilliseconds" | "ft"
        )
    }) {
        return Ok((index, ColumnKind::FrameTime));
    }

    if let Some(index) = normalized.iter().position(|name| name == "fps") {
        return Ok((index, ColumnKind::Fps));
    }

    bail!(
        "CSV header has no 'frametime' or 'fps' column (found: {:?})",
        fields
    )
}

fn normalize_header(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('\u{feff}')
        .to_ascii_lowercase()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

/// Minimal CSV field splitter with quote support (MangoHud does not quote,
/// but hand-edited logs might).
fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut current));
            }
            _ => current.push(character),
        }
    }

    fields.push(current);
    fields
}

fn has_mangohud_header(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };

    for line in BufReader::new(file).lines().take(HEADER_PROBE_LINES) {
        let Ok(line) = line else {
            return false;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if parse_header(trimmed).is_ok() {
            return true;
        }
    }

    false
}

/// Compute frame-time statistics. Requires at least two samples.
pub fn compute_stats(frame_times: &[f64]) -> Result<FrameTimeStats> {
    if frame_times.len() < 2 {
        bail!(
            "Need at least 2 measured frames, found {}",
            frame_times.len()
        );
    }

    let mut sorted = frame_times.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let count = sorted.len();
    if sorted.iter().any(|value| !value.is_finite() || *value <= 0.0) {
        bail!("Frame times must be finite and positive");
    }

    let total: f64 = sorted.iter().sum();
    let mean = total / count as f64;
    if !mean.is_finite() || mean <= 0.0 {
        bail!("Frame-time mean is not finite; log contains unusable values");
    }

    let median = percentile(&sorted, 0.5);
    let p95 = percentile(&sorted, 0.95);
    let p99 = percentile(&sorted, 0.99);

    let low_1pct_ms = slowest_average(&sorted, 0.01);
    let low_0_1pct_ms = slowest_average(&sorted, 0.001);

    Ok(FrameTimeStats {
        frame_count: count,
        duration_seconds: total / 1000.0,
        avg_fps: 1000.0 / mean,
        min_fps: 1000.0 / sorted[count - 1],
        max_fps: 1000.0 / sorted[0],
        avg_frame_time_ms: mean,
        median_frame_time_ms: median,
        p95_frame_time_ms: p95,
        p99_frame_time_ms: p99,
        low_1pct_fps: 1000.0 / low_1pct_ms,
        low_0_1pct_fps: 1000.0 / low_0_1pct_ms,
        stutter_ratio: if median > 0.0 { p99 / median } else { 0.0 },
    })
}

/// Linear-interpolation percentile on a sorted slice.
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    debug_assert!(!sorted.is_empty());

    if sorted.len() == 1 {
        return sorted[0];
    }

    let rank = (sorted.len() - 1) as f64 * fraction;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;

    if lower == upper {
        sorted[lower]
    } else {
        sorted[lower] + (sorted[upper] - sorted[lower]) * (rank - lower as f64)
    }
}

/// Mean of the slowest `fraction` of frames (at least one frame).
fn slowest_average(sorted: &[f64], fraction: f64) -> f64 {
    let count = sorted.len();
    let take = ((count as f64 * fraction).ceil() as usize)
        .max(1)
        .min(count);
    let slice = &sorted[count - take..];
    slice.iter().sum::<f64>() / take as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse(content: &str) -> Result<Vec<f64>> {
        parse_mangohud_reader(Cursor::new(content.as_bytes()), 0)
    }

    #[test]
    fn parses_frametime_column() {
        let csv = "fps,frametime,cpu_load\n100,10.0,20\n50,20.0,30\n";
        assert_eq!(parse(csv).unwrap(), vec![10.0, 20.0]);
    }

    #[test]
    fn skips_mangohud_metadata_preamble() {
        // Real MangoHud logs start with metadata rows before the CSV header.
        let csv = "os,cpu,gpu,ram,kernel\n\
                   CachyOS,AMD Ryzen 5 5600H,AMD Radeon,16GB,7.2\n\
                   fps,frametime,cpu_load\n\
                   100,10.0,20\n\
                   50,20.0,30\n";
        assert_eq!(parse(csv).unwrap(), vec![10.0, 20.0]);
    }

    #[test]
    fn caps_preamble_scanning() {
        let mut csv = String::new();
        for _ in 0..HEADER_PROBE_LINES {
            csv.push_str("a,b\n");
        }
        csv.push_str("fps,frametime\n100,10\n");

        let error = parse(&csv).unwrap_err().to_string();
        assert!(error.contains("No MangoHud header row"));
    }

    #[test]
    fn parses_case_bom_and_crlf() {
        let csv = "\u{feff}FPS,FrameTime\r\n100,16.5\r\n60,16.7\r\n";
        assert_eq!(parse(csv).unwrap(), vec![16.5, 16.7]);
    }

    #[test]
    fn falls_back_to_fps_column() {
        let csv = "fps,gpu_load\n100,50\n50,60\n";
        assert_eq!(parse(csv).unwrap(), vec![10.0, 20.0]);
    }

    #[test]
    fn skips_comments_blank_and_invalid_rows() {
        let csv = "# Mangohud log\nfps,frametime\n\n100,10\nnot,a,number\n100,\n0,0\n100,16.6\n";
        assert_eq!(parse(csv).unwrap(), vec![10.0, 16.6]);
    }

    #[test]
    fn rejects_logs_without_known_columns() {
        let error = parse("gpu_load,cpu_load\n10,20\n").unwrap_err().to_string();
        assert!(error.contains("no 'frametime' or 'fps' column"));
    }

    #[test]
    fn rejects_empty_logs() {
        assert!(parse("").is_err());
        assert!(parse("fps,frametime\n").is_err());
    }

    #[test]
    fn tail_keeps_only_last_n_frames() {
        let csv = "fps,frametime\n100,1\n100,2\n100,3\n100,4\n";
        let frames =
            parse_mangohud_reader(Cursor::new(csv.as_bytes()), 2).unwrap();
        assert_eq!(frames, vec![3.0, 4.0]);
    }

    #[test]
    fn computes_known_statistics() {
        let stats = compute_stats(&[10.0, 10.0, 10.0, 20.0]).unwrap();

        assert_eq!(stats.frame_count, 4);
        assert!((stats.avg_frame_time_ms - 12.5).abs() < 1e-9);
        assert!((stats.avg_fps - 80.0).abs() < 1e-9);
        assert!((stats.min_fps - 50.0).abs() < 1e-9);
        assert!((stats.max_fps - 100.0).abs() < 1e-9);
        assert!((stats.median_frame_time_ms - 10.0).abs() < 1e-9);
        assert!((stats.p95_frame_time_ms - 18.5).abs() < 1e-9);
        assert!((stats.p99_frame_time_ms - 19.7).abs() < 1e-9);
        assert!((stats.low_1pct_fps - 50.0).abs() < 1e-9);
        assert!((stats.low_0_1pct_fps - 50.0).abs() < 1e-9);
        assert!((stats.stutter_ratio - 1.97).abs() < 1e-9);
        assert!((stats.duration_seconds - 0.05).abs() < 1e-9);
    }

    #[test]
    fn one_percent_low_uses_slowest_frames() {
        // Frames of 1..=100 ms: the slowest 1% is the single 100 ms frame.
        let frames: Vec<f64> = (1..=100).map(|value| value as f64).collect();
        let stats = compute_stats(&frames).unwrap();
        assert!((stats.low_1pct_fps - 10.0).abs() < 1e-9);
        assert!((stats.low_0_1pct_fps - 10.0).abs() < 1e-9);
    }

    #[test]
    fn keeps_long_frames_instead_of_hiding_stalls() {
        // A 65-second frame is a real stall (loading hitch), not a bad sample.
        let stats = compute_stats(&[16.6, 65_000.0]).unwrap();
        assert_eq!(stats.frame_count, 2);
        assert!(stats.min_fps < 1.0);
    }

    #[test]
    fn rejects_overflowing_frame_times() {
        assert!(compute_stats(&[f64::MAX, f64::MAX]).is_err());
    }

    #[test]
    fn rejects_too_few_or_invalid_samples() {
        assert!(compute_stats(&[]).is_err());
        assert!(compute_stats(&[16.6]).is_err());
        assert!(compute_stats(&[16.6, 0.0]).is_err());
        assert!(compute_stats(&[16.6, f64::NAN]).is_err());
    }

    #[test]
    fn profile_csv_reads_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MangoHud_test.csv");
        std::fs::write(&path, "fps,frametime\n100,10\n100,10\n50,20\n").unwrap();

        let results = Profiler::new().profile_csv(&path, 0).unwrap();
        assert_eq!(results.stats.frame_count, 3);
        assert_eq!(results.source, path);
    }

    #[test]
    fn finds_newest_mangohud_log_only() {
        let dir = tempfile::tempdir().unwrap();

        let older = dir.path().join("older.csv");
        std::fs::write(&older, "fps,frametime\n100,10\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        let not_mangohud = dir.path().join("other.csv");
        std::fs::write(&not_mangohud, "a,b\n1,2\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        let newer = dir.path().join("newer.csv");
        std::fs::write(&newer, "fps,frametime\n100,11\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        let ignored = dir.path().join("notes.txt");
        std::fs::write(&ignored, "fps,frametime\n100,12\n").unwrap();

        assert_eq!(Profiler::find_latest_log(dir.path()).unwrap(), newer);
    }

    #[test]
    fn missing_log_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(Profiler::find_latest_log(&missing).is_err());
    }
}
