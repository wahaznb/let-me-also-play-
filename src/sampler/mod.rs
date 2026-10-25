//! Real per-function CPU sampling for Wine/PE games on Linux.
//!
//! The sampler uses Linux `perf_event_open` directly (no `perf` binary and no
//! root when `perf_event_paranoid <= 2`). It samples every game thread, maps
//! instruction pointers through `/proc/<pid>/maps` to the PE image, and
//! aggregates them by Ghidra function. Ghidra function lists are cached per
//! binary, so repeated runs are fast.
//!
//! Scope and honesty:
//! * Samples are **self** CPU samples, not inclusive/call-graph samples.
//! * A hot function is where CPU time was spent; it is not proof that changing
//!   it will improve frame pacing.
//! * CPU sampling does not see GPU bottlenecks or time blocked in the driver.
//! * Behavior is only considered improved when a `compare` of two matched runs
//!   shows better frame-time metrics.

#[cfg(target_os = "linux")]
mod functions;
#[cfg(target_os = "linux")]
mod perf;
#[cfg(target_os = "linux")]
mod process;

#[cfg(target_os = "linux")]
use crate::game::GameInfo;
use anyhow::{bail, Result};
#[cfg(target_os = "linux")]
use anyhow::Context;
use serde::Serialize;
#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
#[cfg(target_os = "linux")]
use tracing::info;

#[cfg(target_os = "linux")]
pub use functions::FunctionMap;
#[cfg(target_os = "linux")]
pub use perf::probe as probe_sampling;
#[cfg(target_os = "linux")]
pub use process::{RuntimeModule, TargetProcess};

/// Probe Linux `perf_event_open` support.
///
/// Non-Linux builds expose the same function so `doctor` can report the
/// limitation without a platform-specific code path in the CLI.
#[cfg(not(target_os = "linux"))]
pub fn probe_sampling() -> std::result::Result<(), String> {
    Err("perf_event_open sampling is only implemented on Linux".to_string())
}

/// Inputs for one sampling run.
#[derive(Debug, Clone)]
pub struct SampleConfig {
    pub game: PathBuf,
    pub pid: Option<u32>,
    pub process: Option<String>,
    pub duration: Duration,
    pub frequency: u32,
    pub top: usize,
    pub refresh_functions: bool,
    pub resolve_functions: bool,
}

/// One function ranked by self CPU samples.
#[derive(Debug, Clone, Serialize)]
pub struct HotFunction {
    pub address: String,
    pub name: String,
    pub size: u64,
    pub samples: u64,
    pub game_percent: f64,
    pub total_percent: f64,
}

/// One raw instruction pointer offset when function names are not resolved.
#[derive(Debug, Clone, Serialize)]
pub struct HotOffset {
    pub address: String,
    pub offset: String,
    pub samples: u64,
    pub game_percent: f64,
    pub total_percent: f64,
}

/// Serialized result of a sampling run.
#[derive(Debug, Clone, Serialize)]
pub struct SampleReport {
    pub pid: u32,
    pub process: String,
    pub game: String,
    pub duration_secs: f64,
    pub frequency_hz: u32,
    pub threads: usize,
    pub total_samples: u64,
    pub game_samples: u64,
    pub external_samples: u64,
    pub unknown_game_samples: u64,
    pub lost_samples: u64,
    pub overwritten_bytes: u64,
    pub game_percent: f64,
    pub effective_samples_per_sec: f64,
    pub runtime_base: String,
    pub image_base: String,
    pub image_size: u64,
    pub function_map: bool,
    pub functions_with_samples: usize,
    pub top_functions: Vec<HotFunction>,
    pub top_offsets: Vec<HotOffset>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

#[cfg(target_os = "linux")]
impl SampleReport {
    fn build(
        target: TargetProcess,
        module: RuntimeModule,
        raw: perf::RawSamples,
        function_map: Option<&FunctionMap>,
        top: usize,
    ) -> Self {
        let top = top.clamp(1, 100);
        let total = raw.total_samples;
        let percent = |count: u64| {
            if total == 0 {
                0.0
            } else {
                count as f64 / total as f64 * 100.0
            }
        };

        let mut warnings = Vec::new();
        if let Some(warning) = target.warning {
            warnings.push(warning);
        }
        if let Some(ref warning) = module.warning {
            warnings.push(warning.clone());
        }
        if raw.process_exited {
            warnings.push(
                "The process exited during sampling; results cover only the time before exit."
                    .to_string(),
            );
        }
        if raw.lost_samples > 0 {
            warnings.push(format!(
                "{} samples were dropped by the kernel ring buffer; increase the ring size or \
                 lower --frequency if this is large.",
                raw.lost_samples
            ));
        }
        if raw.overwritten_bytes > 0 {
            warnings.push(format!(
                "{} bytes of ring-buffer data were overwritten before they could be read.",
                raw.overwritten_bytes
            ));
        }
        if raw.thread_open_failures > 0 {
            warnings.push(format!(
                "{} thread(s) could not be sampled (they may have exited during the scan).",
                raw.thread_open_failures
            ));
        }
        if raw.elapsed_secs > 0.0 && raw.elapsed_secs < 1.0 {
            warnings.push(
                "The sampling window was shorter than 1 second; percentages are noisy."
                    .to_string(),
            );
        }
        if total < 100 {
            warnings.push(format!(
                "Only {total} samples were collected; treat percentages as a rough signal."
            ));
        }

        let mut game_samples = 0u64;
        let mut external_samples = 0u64;
        let mut unknown_game_samples = 0u64;
        let mut function_counts: HashMap<usize, u64> = HashMap::new();
        let mut offset_counts: HashMap<u64, u64> = HashMap::new();

        for (ip, count) in &raw.ip_counts {
            let Some(rva) = module.rva(*ip) else {
                external_samples = external_samples.saturating_add(*count);
                continue;
            };
            game_samples = game_samples.saturating_add(*count);
            if let Some(map) = function_map {
                match map.find_index(rva) {
                    Some(index) => {
                        *function_counts.entry(index).or_insert(0) += *count;
                    }
                    None => {
                        unknown_game_samples = unknown_game_samples.saturating_add(*count);
                    }
                }
            } else {
                *offset_counts.entry(rva).or_insert(0) += *count;
            }
        }

        let mut top_functions = Vec::new();
        let mut functions_with_samples = 0usize;
        if let Some(map) = function_map {
            functions_with_samples = function_counts.len();
            let mut ranked: Vec<(usize, u64)> = function_counts.into_iter().collect();
            ranked.sort_by_key(|(index, count)| {
                (
                    std::cmp::Reverse(*count),
                    map.spans()[*index].address,
                )
            });
            for (index, count) in ranked.into_iter().take(top) {
                let span = &map.spans()[index];
                top_functions.push(HotFunction {
                    address: format!("0x{:X}", span.address),
                    name: span.name.clone(),
                    size: span.size,
                    samples: count,
                    game_percent: if game_samples == 0 {
                        0.0
                    } else {
                        count as f64 / game_samples as f64 * 100.0
                    },
                    total_percent: percent(count),
                });
            }

            let unknown_percent = if game_samples == 0 {
                0.0
            } else {
                unknown_game_samples as f64 / game_samples as f64 * 100.0
            };
            if unknown_percent > 5.0 {
                warnings.push(format!(
                    "{:.1}% of in-game samples fell in gaps between Ghidra functions; \
                     decompile the raw addresses or refresh the function list.",
                    unknown_percent
                ));
            }
        }

        let mut top_offsets = Vec::new();
        if function_map.is_none() {
            functions_with_samples = offset_counts.len();
            let mut ranked: Vec<(u64, u64)> = offset_counts.into_iter().collect();
            ranked.sort_by_key(|(offset, count)| (std::cmp::Reverse(*count), *offset));
            for (rva, count) in ranked.into_iter().take(top) {
                top_offsets.push(HotOffset {
                    address: module
                        .ghidra_address(module.base.saturating_add(rva))
                        .map(|address| format!("0x{address:X}"))
                        .unwrap_or_else(|| format!("0x{:X}", module.image_base.saturating_add(rva))),
                    offset: format!("0x{rva:X}"),
                    samples: count,
                    game_percent: if game_samples == 0 {
                        0.0
                    } else {
                        count as f64 / game_samples as f64 * 100.0
                    },
                    total_percent: percent(count),
                });
            }
            warnings.push(
                "Function names were not resolved; addresses are Ghidra-space VAs and \
                 offsets are PE RVAs. Run again without --no-ghidra for per-function names."
                    .to_string(),
            );
        }

        let game_percent = percent(game_samples);
        if game_samples == 0 {
            warnings.push(
                "No samples landed in the game module. The game may be idle at a menu, \
                 waiting for input, or the module mapping changed."
                    .to_string(),
            );
        } else if game_percent < 50.0 {
            warnings.push(format!(
                "Only {:.1}% of samples were in the game module; the process may be idle \
                 or spending most of its CPU time in Wine/OS code.",
                game_percent
            ));
        }

        Self {
            pid: target.pid,
            process: target.comm,
            game: module.path.to_string_lossy().to_string(),
            duration_secs: raw.elapsed_secs,
            frequency_hz: 0,
            threads: raw.threads_started,
            total_samples: total,
            game_samples,
            external_samples,
            unknown_game_samples,
            lost_samples: raw.lost_samples,
            overwritten_bytes: raw.overwritten_bytes,
            game_percent,
            effective_samples_per_sec: if raw.elapsed_secs > 0.0 {
                total as f64 / raw.elapsed_secs
            } else {
                0.0
            },
            runtime_base: format!("0x{:X}", module.base),
            image_base: format!("0x{:X}", module.image_base),
            image_size: module.image_size,
            function_map: function_map.is_some(),
            functions_with_samples,
            top_functions,
            top_offsets,
            warnings,
            notes: vec![
                "Self CPU samples only: each sample is the instruction executing in a sampled \
                 thread; callees, blocked time, and GPU time are not attributed."
                    .to_string(),
                "A hot function is a hypothesis for a patch, not proof of a frame-time win. \
                 Verify with `compare` on two matched runs."
                    .to_string(),
                "Samples outside the game module are counted as external (Wine, ntdll, drivers); \
                 they are not ranked as game functions."
                    .to_string(),
            ],
        }
    }
}

/// Run one sampling session and build a bounded report.
#[cfg(target_os = "linux")]
pub async fn run(config: SampleConfig) -> Result<SampleReport> {
    if config.duration.is_zero() {
        bail!("Sampling duration must be greater than zero");
    }
    if !config.game.is_file() {
        bail!("Game executable not found: {}", config.game.display());
    }

    let game = GameInfo::from_path(&config.game)
        .with_context(|| format!("Failed to inspect game {}", config.game.display()))?;

    let target = process::resolve_target(&game.path, config.pid, config.process.as_deref())?;
    let module = process::find_runtime_module(target.pid, &game.path)?;

    let function_map = if config.resolve_functions {
        info!(
            "Loading function map for {} (Ghidra cache; first run can take ~30s)",
            game.path.display()
        );
        Some(functions::load_function_map(&game.path, config.refresh_functions).await?)
    } else {
        None
    };

    let pid = target.pid;
    let duration = config.duration;
    let frequency = config.frequency.clamp(1, 999);
    let raw = tokio::task::spawn_blocking(move || perf::sample_process(pid, duration, frequency))
        .await
        .context("Sampling task panicked")??;

    let mut report = SampleReport::build(target, module, raw, function_map.as_ref(), config.top);
    report.frequency_hz = frequency;
    Ok(report)
}

#[cfg(not(target_os = "linux"))]
pub async fn run(config: SampleConfig) -> Result<SampleReport> {
    if !config.game.is_file() {
        bail!("Game executable not found: {}", config.game.display());
    }
    bail!(
        "Per-function sampling is only implemented on Linux (perf_event_open). Run the sampler on \
         the Linux/Wine host. Game path checked: {}",
        config.game.display()
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::decompiler::FunctionSummary;
    use crate::patcher::pe::PeInfo;
    use std::path::PathBuf;

    fn test_pe() -> PeInfo {
        PeInfo {
            is_64: false,
            machine: 0x14c,
            image_base: 0x1000,
            entry_point_rva: 0x10,
            size_of_image: 0x100,
            size_of_headers: 0x200,
            size_of_code: 0x80,
            file_alignment: 0x200,
            section_alignment: 0x1000,
            checksum: 0,
            number_of_sections: 0,
            file_len: 0x200,
            pe_offset: 0x80,
            optional_header_offset: 0x98,
            optional_header_size: 0xe0,
            section_table_offset: 0x178,
            checksum_offset: 0,
            size_of_code_offset: 0,
            size_of_image_offset: 0,
            number_of_sections_offset: 0,
            certificate_directory: (0, 0),
            sections: Vec::new(),
        }
    }

    fn module() -> RuntimeModule {
        RuntimeModule {
            path: PathBuf::from("/game.exe"),
            base: 0x5000_0000,
            size: 0x100,
            image_base: 0x1000,
            image_size: 0x100,
            warning: None,
        }
    }

    fn target() -> TargetProcess {
        TargetProcess {
            pid: 42,
            comm: "Game.exe".to_string(),
            warning: None,
        }
    }

    fn raw(ips: &[(u64, u64)]) -> perf::RawSamples {
        let mut raw = perf::RawSamples {
            elapsed_secs: 2.0,
            threads_started: 2,
            ..Default::default()
        };
        for (ip, count) in ips {
            raw.ip_counts.insert(*ip, *count);
            raw.total_samples += *count;
        }
        raw
    }

    #[test]
    fn ranks_functions_and_splits_external_samples() {
        let pe = test_pe();
        let map = FunctionMap::from_summaries(
            &pe,
            vec![
                FunctionSummary {
                    address: 0x1000,
                    name: "first".to_string(),
                    size: 0x10,
                },
                FunctionSummary {
                    address: 0x1010,
                    name: "second".to_string(),
                    size: 0x20,
                },
            ],
        );
        let raw = raw(&[
            (0x5000_0000, 1),
            (0x5000_0004, 2),
            (0x5000_0010, 4),
            (0x5000_0090, 4),  // inside image, no function
            (0x7000_0000, 10), // external
        ]);
        let report = SampleReport::build(target(), module(), raw, Some(&map), 10);

        assert_eq!(report.total_samples, 21);
        assert_eq!(report.game_samples, 11);
        assert_eq!(report.external_samples, 10);
        assert_eq!(report.unknown_game_samples, 4);
        assert_eq!(report.top_functions[0].name, "second");
        assert_eq!(report.top_functions[0].samples, 4);
        assert_eq!(report.top_functions[1].name, "first");
        assert_eq!(report.top_functions[1].samples, 3);
        assert!((report.game_percent - (11.0 / 21.0 * 100.0)).abs() < 1e-9);
    }

    #[test]
    fn raw_mode_reports_offsets_and_a_clear_warning() {
        let raw = raw(&[(0x5000_0000, 2), (0x5000_0010, 1)]);
        let report = SampleReport::build(target(), module(), raw, None, 10);
        assert_eq!(report.function_map, false);
        assert_eq!(report.top_offsets.len(), 2);
        assert_eq!(report.top_offsets[0].address, "0x1000");
        assert_eq!(report.top_offsets[0].offset, "0x0");
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("Function names were not resolved"))
        );
    }

    #[test]
    fn empty_game_window_is_explained() {
        let raw = raw(&[(0x7000_0000, 5)]);
        let report = SampleReport::build(target(), module(), raw, None, 10);
        assert_eq!(report.game_samples, 0);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("No samples landed in the game module"))
        );
    }

    #[test]
    fn top_is_clamped_to_a_bounded_list() {
        let pe = test_pe();
        let functions = (0..20)
            .map(|index| FunctionSummary {
                address: 0x1000 + index * 4,
                name: format!("f{index}"),
                size: 4,
            })
            .collect();
        let map = FunctionMap::from_summaries(&pe, functions);
        let raw = raw(&[(0x5000_0000, 1)]);
        let report = SampleReport::build(target(), module(), raw, Some(&map), 999);
        assert_eq!(report.top_functions.len(), 1);
    }
}
