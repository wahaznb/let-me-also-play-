//! Game Opt Tool — Main entry point
//!
//! A tool that reverse engineers and optimizes game binaries
//! to improve performance while preserving the original behavior.

#[allow(dead_code)]
mod cli;
mod config;
mod game;
mod profiler;
mod decompiler;
mod optimizer;
mod patcher;
mod verifier;
mod backup;
mod hash;
pub mod targets;

use anyhow::{bail, Context, Result};
use clap::Parser;
use clap::CommandFactory;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use tracing::{info, error};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "game-opt-tool")]
#[command(version)]
#[command(disable_help_subcommand = true)]
#[command(about = "Reverse engineer and optimize game binaries", long_about = None)]
struct Cli {
    /// Path to the game directory
    #[arg(short, long)]
    game_path: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Analyze a game and summarize measured frame times
    Analyze {
        /// Path to the game executable
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// MangoHud CSV log to analyze
        #[arg(long)]
        log: Option<std::path::PathBuf>,

        /// Directory to search for the newest MangoHud CSV log
        #[arg(long)]
        log_dir: Option<std::path::PathBuf>,
    },

    /// Measure frame-time consistency from a MangoHud CSV log
    Profile {
        /// Path to the game executable
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// MangoHud CSV log to analyze (defaults to the newest log in --log-dir or the current directory)
        #[arg(long)]
        log: Option<std::path::PathBuf>,

        /// Directory to search for the newest MangoHud CSV log
        #[arg(long)]
        log_dir: Option<std::path::PathBuf>,

        /// Analyze only the last N frames (0 = all)
        #[arg(short, long, default_value = "0")]
        frames: u32,

        /// Print machine-readable JSON instead of text
        #[arg(long)]
        json: bool,
    },

    /// Compare frame-time metrics from two MangoHud logs
    Compare {
        /// Baseline MangoHud CSV log
        #[arg(long)]
        baseline: std::path::PathBuf,

        /// Patched/experimental MangoHud CSV log
        #[arg(long)]
        patched: std::path::PathBuf,

        /// Compare only the last N frames of both logs (0 = all)
        #[arg(short, long, default_value = "0")]
        frames: u32,

        /// Print machine-readable JSON instead of text
        #[arg(long)]
        json: bool,
    },

    /// Decompile a function with Ghidra
    Decompile {
        /// Path to the game executable
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// Function address in hex (defaults to the executable entry point)
        #[arg(long)]
        address: Option<String>,

        /// Include the disassembly listing in text output
        #[arg(long)]
        assembly: bool,

        /// Print machine-readable JSON instead of text
        #[arg(long)]
        json: bool,
    },

    /// Optimize specific functions
    Optimize {
        /// Path to the game executable
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// Functions to optimize (comma-separated)
        #[arg(short, long)]
        functions: Option<String>,

        /// Output directory for optimized code
        #[arg(short, long, default_value = "./optimized")]
        output: std::path::PathBuf,
    },

    /// Apply optimization patches to the game
    Patch {
        /// Path to the game executable
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// Output path for patched executable
        #[arg(short, long)]
        output: std::path::PathBuf,

        /// JSON patch spec (byte patches and optional code cave)
        #[arg(long)]
        spec: Option<std::path::PathBuf>,

        /// Write the patch manifest here instead of next to the output
        #[arg(long)]
        manifest: Option<std::path::PathBuf>,

        /// Validate and report without writing the output or manifest
        #[arg(long)]
        dry_run: bool,

        /// Print machine-readable JSON instead of text
        #[arg(long)]
        json: bool,
    },

    /// Revert exactly the changes recorded in a patch manifest
    Unpatch {
        /// Patch manifest written by `patch --spec`
        #[arg(long)]
        manifest: std::path::PathBuf,

        /// Patched executable (defaults to the path recorded in the manifest)
        #[arg(short, long)]
        game: Option<std::path::PathBuf>,

        /// Validate and report without writing anything
        #[arg(long)]
        dry_run: bool,

        /// Print machine-readable JSON instead of text
        #[arg(long)]
        json: bool,
    },

    /// Verify optimization correctness
    Verify {
        /// Original executable
        #[arg(short, long)]
        original: std::path::PathBuf,

        /// Patched executable
        #[arg(short, long)]
        patched: std::path::PathBuf,
    },

    /// Create, list, or verify a backup of a binary
    Backup {
        /// File to back up
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// List existing backups instead of creating a new one
        #[arg(long, conflicts_with = "verify")]
        list: bool,

        /// Re-check the checksum of every existing backup
        #[arg(long)]
        verify: bool,
    },

    /// Restore a binary from a verified backup
    Revert {
        /// File to restore
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// Specific backup manifest (defaults to the newest backup)
        #[arg(long)]
        backup: Option<std::path::PathBuf>,

        /// Show what would happen without writing anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Check prerequisites and environment readiness
    Doctor {
        /// Optional game executable or installation directory to inspect
        #[arg(short, long)]
        game: Option<std::path::PathBuf>,
    },

    /// List supported games and their configurations
    ListGames,

    /// Show help for a specific game target
    Help {
        /// Game name
        game: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing. Logs go to stderr; data goes to stdout.
    let subscriber = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let cli = Cli::parse();

    match cli.command {
        Some(Command::Analyze { game, log, log_dir }) => {
            info!("Analyzing game: {:?}", game);
            analyze_game(&game, log.as_deref(), log_dir.as_deref())?;
        }
        Some(Command::Profile { game, log, log_dir, frames, json }) => {
            info!("Profiling game: {:?}", game);
            profile_game(
                &game,
                log.as_deref(),
                log_dir.as_deref(),
                frames as usize,
                json,
            )?;
        }
        Some(Command::Compare { baseline, patched, frames, json }) => {
            info!("Comparing logs: {:?} vs {:?}", baseline, patched);
            compare_command(
                &baseline,
                &patched,
                frames as usize,
                json,
            )?;
        }
        Some(Command::Decompile { game, address, assembly, json }) => {
            info!("Decompiling game: {:?}", game);
            decompile_game(&game, address.as_deref(), assembly, json).await?;
        }
        Some(Command::Optimize { game, functions, output }) => {
            info!("Optimizing game: {:?} (functions: {:?})", game, functions);
            
            if is_deus_ex(&game) {
                optimize_deus_ex(&game, functions.as_deref(), &output).await?;
            } else {
                optimize_game(&game, functions.as_deref()).await?;
            }
        }
        Some(Command::Patch {
            game,
            output,
            spec,
            manifest,
            dry_run,
            json,
        }) => {
            info!("Patching game: {:?} → {:?}", game, output);

            if let Some(spec_path) = spec {
                patch_with_spec(
                    &game,
                    &output,
                    &spec_path,
                    manifest.as_deref(),
                    dry_run,
                    json,
                )?;
            } else if dry_run || json || manifest.is_some() {
                bail!(
                    "--dry-run, --json, and --manifest require --spec; without a spec, patch writes a byte-identical copy"
                );
            } else if is_deus_ex(&game) {
                patch_deus_ex(&game, &output).await?;
            } else {
                patch_game(&game, &output).await?;
            }
        }
        Some(Command::Unpatch {
            manifest,
            game,
            dry_run,
            json,
        }) => {
            info!("Reverting patch manifest: {:?}", manifest);
            unpatch_command(&manifest, game.as_deref(), dry_run, json)?;
        }
        Some(Command::Verify { original, patched }) => {
            info!("Verifying: {:?} vs {:?}", original, patched);
            
            if is_deus_ex(&original) {
                verify_deus_ex(&original, &patched).await?;
            } else {
                verify_optimization(&original, &patched).await?;
            }
        }
        Some(Command::Backup { game, list, verify }) => {
            backup_command(&game, list, verify)?;
        }
        Some(Command::Revert { game, backup, dry_run }) => {
            revert_command(&game, backup.as_deref(), dry_run)?;
        }
        Some(Command::Doctor { game }) => {
            doctor_command(game.as_deref())?;
        }
        Some(Command::ListGames) => {
            list_games();
        }
        Some(Command::Help { game }) => {
            show_help(&game);
        }
        None => {
            Cli::command().print_help()?;
            println!("\nTry: game-opt-tool --help");
        }
    }

    Ok(())
}

fn is_deus_ex(path: &Path) -> bool {
    // Check if this looks like a Deus Ex installation, or the executable itself.
    path.join("System").join("DeusEx.exe").exists()
        || path.join("DeusEx.exe").exists()
        || path.join("Bin").join("DeusEx.exe").exists()
        || path
            .file_name()
            .map(|name| name.eq_ignore_ascii_case("deusex.exe"))
            .unwrap_or(false)
}

async fn optimize_deus_ex(
    game_path: &Path,
    functions: Option<&str>,
    output: &Path,
) -> Result<()> {
    use optimizer::Optimizer;
    use patcher::Patcher;

    println!("WARNING: the optimizer is a stub; speedup values are placeholders, not measurements.");
    if functions.is_some() {
        println!("NOTE: --functions is ignored until real optimization exists.");
    }

    let optimizer = Optimizer::new();
    let optimizations = optimizer.optimize_all(game_path, &[]).await?;
    
    for opt in &optimizations {
        println!("\n{}: {}% (placeholder estimate)", opt.function_name, opt.speedup_pct);
        println!("  Notes: {}", opt.notes);
    }

    // `--output` is documented as a directory; also accept an explicit file.
    let patched_path = if output.exists() && output.is_file() {
        output.to_path_buf()
    } else {
        std::fs::create_dir_all(output).with_context(|| {
            format!("Failed to create output directory {}", output.display())
        })?;
        let file_name = game_path
            .file_name()
            .context("Game path has no file name")?;
        output.join(file_name)
    };

    backup_before_overwrite(&patched_path)?;

    let patcher = Patcher::new();
    patcher.apply_patches(game_path, &patched_path).await?;
    println!("Output: {}", patched_path.display());
    
    Ok(())
}

async fn patch_deus_ex(game_path: &Path, output: &Path) -> Result<()> {
    use patcher::Patcher;

    println!(
        "NOTE: no --spec supplied; writing a byte-identical copy. \
         Pass --spec <json> to apply real byte patches or a code cave."
    );
    backup_before_overwrite(output)?;

    let patcher = Patcher::new();
    patcher.apply_patches(game_path, output).await?;

    Ok(())
}

fn patch_with_spec(
    game_path: &Path,
    output_path: &Path,
    spec_path: &Path,
    manifest_path: Option<&Path>,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    use patcher::{PatchSpec, Patcher};

    if !game_path.exists() {
        bail!("Game path does not exist: {}", game_path.display());
    }
    let spec = PatchSpec::load(spec_path)?;

    // Refuse path aliases before creating a backup or writing anything.
    if patcher::same_file(output_path, spec_path) {
        bail!(
            "Refusing to write the patched binary over the patch spec: {}",
            spec_path.display()
        );
    }
    if let Some(manifest_path) = manifest_path {
        if patcher::same_file(manifest_path, spec_path) {
            bail!(
                "Refusing to write the patch manifest over the patch spec: {}",
                spec_path.display()
            );
        }
        if patcher::same_file(manifest_path, game_path) {
            bail!(
                "Refusing to write the patch manifest over the input binary: {}",
                game_path.display()
            );
        }
        if patcher::same_file(manifest_path, output_path) {
            bail!(
                "Refusing to write the patch manifest over the output binary: {}",
                output_path.display()
            );
        }
    }
    if !dry_run {
        backup_before_overwrite(output_path)?;
    }

    let patcher = Patcher::new();
    let report = patcher.apply_spec(
        game_path,
        output_path,
        &spec,
        manifest_path,
        dry_run,
    )?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let action = if dry_run { "Would apply" } else { "Applied" };
    println!(
        "{action} {} byte patch(es) and {} code section(s).",
        report.patches_applied, report.sections_added
    );
    println!("  input:    {}", report.input_path.display());
    println!("  output:   {}", report.output_path.display());
    println!("  input SHA-256:  {}", report.input_sha256);
    println!("  output SHA-256: {}", report.output_sha256);
    println!(
        "  checksum: 0x{:08X} -> 0x{:08X}",
        report.checksum_before, report.checksum_after
    );
    if report.manifest_written {
        println!("  manifest: {}", report.manifest_path.display());
        println!(
            "  revert with: game-opt-tool unpatch --manifest {}",
            report.manifest_path.display()
        );
    } else {
        println!("  manifest: not written (dry run)");
    }
    for section in &report.sections {
        println!(
            "  cave {:?}: VA 0x{:X}, {} bytes, executable",
            section.name, section.address, section.virtual_size
        );
    }
    for patch in &report.patches {
        println!(
            "  patch 0x{:X} ({}): {} -> {}",
            patch.address, patch.section, patch.before_hex, patch.after_hex
        );
    }
    Ok(())
}

fn unpatch_command(
    manifest_path: &Path,
    game_path: Option<&Path>,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    use patcher::Patcher;

    let report = Patcher::unpatch(manifest_path, game_path, dry_run)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let action = if dry_run { "Would revert" } else { "Reverted" };
    println!(
        "{action} {} byte patch(es) and {} code section(s).",
        report.patches_reverted, report.sections_removed
    );
    println!("  target:  {}", report.target.display());
    println!("  before SHA-256: {}", report.output_sha256);
    println!("  after SHA-256:  {}", report.restored_sha256);
    if dry_run {
        println!("  no files written (dry run)");
    }
    Ok(())
}

async fn verify_deus_ex(original: &Path, patched: &Path) -> Result<()> {
    use verifier::Verifier;
    
    let verifier = Verifier::new();
    let report = verifier.verify(original, patched).await?;
    
    println!("\n=== Verification Report ===");
    for detail in &report.details {
        println!("{}", detail);
    }
    
    if report.launched_successfully && report.gameplay_unchanged && report.quirks_preserved {
        println!("\n✓ Verification PASSED");
    } else {
        println!("\n✗ Verification FAILED");
        std::process::exit(1);
    }
    
    Ok(())
}

fn list_games() {
    println!("Supported games:");
    println!("  - Deus Ex (2000)");
    println!("  - The Elder Scrolls IV: Oblivion Remastered");
    println!("  - The Elder Scrolls V: Skyrim Special Edition");
    println!("  - Star Wars Jedi: Survivor");
    println!("\nUse 'game-opt-tool help <game-name>' for more info.");
}

fn show_help(game: &str) {
    let normalized = config::normalize_game_name(game);
    match normalized.as_str() {
        "deusex" => {
            println!("{}", include_str!("targets/deus-ex/README.md"));
        }
        _ => {
            println!("Unknown game: {}", game);
            println!("\nSupported games:");
            println!("  - Deus Ex (2000)");
            println!("  - The Elder Scrolls IV: Oblivion Remastered");
            println!("  - The Elder Scrolls V: Skyrim Special Edition");
            println!("  - Star Wars Jedi: Survivor");
        }
    }
}

fn analyze_game(
    game_path: &std::path::Path,
    log: Option<&std::path::Path>,
    log_dir: Option<&std::path::Path>,
) -> Result<()> {
    use game::GameInfo;
    use profiler::Profiler;

    let game = GameInfo::from_path(game_path)?;
    println!("Detected game: {} {}", game.name, game.version);
    println!("Architecture: {}", game.architecture);
    println!("Binary format: {:?}", game.binary_format);
    println!("Executable: {}", game.path.display());

    if let Some(config) = &game.config {
        println!(
            "Config: {} ({} hot targets, {} quirks)",
            config.name,
            config.hot_functions.len(),
            config.quirks.len()
        );
    }

    let explicit_source = log.is_some() || log_dir.is_some();
    match resolve_log_path(log, log_dir) {
        Ok(log_path) => {
            let profiler = Profiler::new();
            let results = profiler.profile_csv(&log_path, 0)?;
            println!();
            print_profile(&results);
            println!();
            println!(
                "NOTE: function-level hotspots require CPU instrumentation and are not \
                 inferred from frame times."
            );
        }
        Err(error) if explicit_source => return Err(error),
        Err(_) => {
            println!();
            println!("No measurement log found.");
            println!("Generate one by running the game through MangoHud:");
            println!(
                "  MANGOHUD_CONFIG=\"output_folder=/tmp/mangohud,autostart_log=1,log_duration=0\" \\"
            );
            println!("    mangohud --dlsym wine DeusEx.exe");
            println!();
            println!(
                "Then: game-opt-tool analyze --game <exe> --log-dir /tmp/mangohud"
            );
        }
    }

    Ok(())
}

fn profile_game(
    game_path: &std::path::Path,
    log: Option<&std::path::Path>,
    log_dir: Option<&std::path::Path>,
    tail_frames: usize,
    json: bool,
) -> Result<()> {
    use profiler::Profiler;

    if !game_path.exists() {
        bail!("Game path does not exist: {}", game_path.display());
    }

    let log_path = resolve_log_path(log, log_dir)?;
    let profiler = Profiler::new();
    let results = profiler.profile_csv(&log_path, tail_frames)?;

    if json {
        println!("{}", profiler.export_json(&results)?);
    } else {
        print_profile(&results);
    }

    Ok(())
}

fn resolve_log_path(
    log: Option<&std::path::Path>,
    log_dir: Option<&std::path::Path>,
) -> Result<PathBuf> {
    use profiler::Profiler;

    if let Some(path) = log {
        if !path.is_file() {
            bail!("Log file not found: {}", path.display());
        }
        return Ok(path.to_path_buf());
    }

    let directory = log_dir.unwrap_or_else(|| Path::new("."));
    if !directory.is_dir() {
        bail!("Log directory not found: {}", directory.display());
    }

    Profiler::find_latest_log(directory)
}

fn print_profile(results: &profiler::ProfileResults) {
    let stats = &results.stats;

    println!("Source: {}", results.source.display());
    println!(
        "Frames: {} ({:.2}s)",
        stats.frame_count, stats.duration_seconds
    );
    println!("Average FPS: {:.2}", stats.avg_fps);
    println!("Min / max FPS: {:.2} / {:.2}", stats.min_fps, stats.max_fps);
    println!("1% low FPS: {:.2}", stats.low_1pct_fps);
    println!("0.1% low FPS: {:.2}", stats.low_0_1pct_fps);
    println!(
        "Frame time avg / median: {:.2}ms / {:.2}ms",
        stats.avg_frame_time_ms, stats.median_frame_time_ms
    );
    println!(
        "Frame time p95 / p99: {:.2}ms / {:.2}ms",
        stats.p95_frame_time_ms, stats.p99_frame_time_ms
    );
    println!("Stutter ratio (p99 / median): {:.2}x", stats.stutter_ratio);
}

fn compare_command(
    baseline: &Path,
    patched: &Path,
    tail_frames: usize,
    json: bool,
) -> Result<()> {
    use profiler::Profiler;

    let profiler = Profiler::new();
    let comparison = profiler.compare_logs(baseline, patched, tail_frames)?;

    if json {
        println!("{}", profiler.export_comparison_json(&comparison)?);
    } else {
        print_comparison(&comparison);
    }

    Ok(())
}

fn print_comparison(comparison: &profiler::ProfileComparison) {
    let baseline = &comparison.baseline;
    let patched = &comparison.patched;

    println!(
        "Baseline: {} ({} frames, {:.2}s)",
        baseline.source.display(),
        baseline.stats.frame_count,
        baseline.stats.duration_seconds
    );
    println!(
        "Patched:  {} ({} frames, {:.2}s)",
        patched.source.display(),
        patched.stats.frame_count,
        patched.stats.duration_seconds
    );
    println!();
    println!(
        "{:<26} {:>10} {:>10} {:>10}  Verdict",
        "Metric", "Baseline", "Patched", "Change"
    );
    print_metric("Average FPS", &comparison.avg_fps);
    print_metric("1% low FPS", &comparison.low_1pct_fps);
    print_metric("0.1% low FPS", &comparison.low_0_1pct_fps);
    print_metric("p99 frame time (ms)", &comparison.p99_frame_time_ms);
    print_metric("Stutter ratio (x)", &comparison.stutter_ratio);
    println!();
    println!("Interpretation notes:");
    for warning in &comparison.warnings {
        println!("  - {}", warning);
    }
}

fn print_metric(label: &str, delta: &profiler::MetricDelta) {
    let verdict = if delta.absolute == 0.0 {
        "unchanged"
    } else if delta.improved {
        "better"
    } else {
        "worse"
    };

    println!(
        "{:<26} {:>10.2} {:>10.2} {:>+9.2}%  {}",
        label, delta.baseline, delta.patched, delta.percent, verdict
    );
}

async fn decompile_game(
    game_path: &Path,
    address: Option<&str>,
    show_assembly: bool,
    json: bool,
) -> Result<()> {
    use decompiler::{parse_hex_address, Decompiler};
    use game::GameInfo;

    if !game_path.exists() {
        bail!("Game path does not exist: {}", game_path.display());
    }

    let game = GameInfo::from_path(game_path)?;
    let parsed_address = match address {
        Some(value) => Some(parse_hex_address(value)?),
        None => None,
    };

    let decompiler = Decompiler::new()?;
    let function = match parsed_address {
        Some(value) => decompiler.decompile_function(&game, value).await?,
        None => decompiler.decompile_entry(&game).await?,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&function)?);
        return Ok(());
    }

    println!("Function: {}", function.name);
    println!("Address:  0x{:X}", function.address);
    println!("Signature: {}", function.signature);
    println!("Returns:  {}", function.return_type);
    if function.parameters.is_empty() {
        println!("Parameters: none");
    } else {
        println!("Parameters:");
        for parameter in &function.parameters {
            println!("  - {}", parameter);
        }
    }

    println!();
    println!("{}", function.pseudocode);

    if show_assembly {
        println!();
        println!("Disassembly:");
        if function.assembly.is_empty() {
            println!("  (none returned)");
        } else {
            println!("{}", function.assembly);
        }
    }

    println!();
    println!("Decompiled by Ghidra headless. Pseudocode is analysis output, not source.");

    Ok(())
}

async fn optimize_game(
    game_path: &std::path::Path,
    functions: Option<&str>,
) -> Result<()> {
    use decompiler::Decompiler;
    use optimizer::Optimizer;

    println!("WARNING: optimization passes are stubs; speedup values are placeholders, not measurements.");

    // Decompile target functions
    let decompiler = Decompiler::new()?;
    let funcs = decompiler.find_hot_functions(game_path, functions).await?;

    info!("Found {} functions to optimize:", funcs.len());
    for func in &funcs {
        info!("  - {}", func.name);
    }

    // Optimize each function
    let optimizer = Optimizer::new();
    let results = optimizer.optimize_all(game_path, &funcs).await?;

    info!("\nOptimization Results:");
    for result in &results {
        match &result.status {
            optimizer::OptimizeStatus::Success => {
                info!("  ✓ {} — {}% (placeholder estimate)", result.function_name, result.speedup_pct);
            }
            optimizer::OptimizeStatus::Failed(e) => {
                error!("  ✗ {} — {}", result.function_name, e);
            }
            optimizer::OptimizeStatus::Skipped => {
                info!("  ~ {} — skipped (no optimization needed)", result.function_name);
            }
        }
    }

    Ok(())
}

async fn patch_game(
    game_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<()> {
    use patcher::Patcher;

    println!(
        "NOTE: no --spec supplied; writing a byte-identical copy. \
         Pass --spec <json> to apply real byte patches or a code cave."
    );
    backup_before_overwrite(output_path)?;

    let patcher = Patcher::new();
    patcher.apply_patches(game_path, output_path).await?;

    println!("Patched game saved to: {}", output_path.display());
    Ok(())
}

async fn verify_optimization(
    original: &std::path::Path,
    patched: &std::path::Path,
) -> Result<()> {
    use verifier::Verifier;

    let verifier = Verifier::new();
    let report = verifier.verify(original, patched).await?;

    println!("\nVerification Report:");
    println!("  Behavioral match: {}", report.behavioral_match);
    println!("  Frame difference: {:.4}%", report.frame_diff_percent);
    println!("  Quirks preserved: {}", report.quirks_preserved);
    println!("  Performance gain (measured): {:.1}%", report.performance_gain_pct);

    if report.behavioral_match && report.quirks_preserved {
        info!("\n✓ Verification PASSED");
    } else {
        error!("\n✗ Verification FAILED");
        if !report.behavioral_match {
            error!("  Behavioral mismatch detected");
        }
        if !report.quirks_preserved {
            error!("  Some quirks were not preserved");
        }
        std::process::exit(1);
    }

    Ok(())
}

/// Back up a file before it is overwritten (no-op if it does not exist).
fn backup_before_overwrite(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_file() {
        bail!(
            "Refusing to overwrite {}: not a regular file",
            path.display()
        );
    }

    let record = backup::create(path)?;
    // This is a diagnostic; keep stdout clean for --json commands.
    eprintln!(
        "Backup created before overwrite: {}",
        record.backup_path.display()
    );
    Ok(())
}

fn backup_command(path: &Path, list: bool, verify: bool) -> Result<()> {
    if list {
        let records = backup::list_backups(path)
            .with_context(|| format!("Failed to list backups for {}", path.display()))?;
        if records.is_empty() {
            println!("No backups found for {}", path.display());
            return Ok(());
        }

        println!("Backups for {} (newest first):", path.display());
        for record in records {
            println!(
                "  {}  {} bytes  {}",
                record.short_hash(),
                record.size,
                record.backup_path.display()
            );
            println!("       manifest: {}", record.manifest_path().display());
        }
        return Ok(());
    }

    if verify {
        let records = backup::list_backups(path)
            .with_context(|| format!("Failed to list backups for {}", path.display()))?;
        if records.is_empty() {
            println!("No backups found for {}", path.display());
            return Ok(());
        }

        let mut failures = 0u32;
        for record in &records {
            match backup::verify_record(record) {
                Ok(()) => println!(
                    "[ok]   {}  {}",
                    record.short_hash(),
                    record.backup_path.display()
                ),
                Err(error) => {
                    println!("[fail] {}  {:#}", record.short_hash(), error);
                    failures += 1;
                }
            }
        }

        if failures > 0 {
            bail!("{} backup(s) failed verification", failures);
        }
        println!("All {} backup(s) verified.", records.len());
        return Ok(());
    }

    let record = backup::create(path)
        .with_context(|| format!("Failed to back up {}", path.display()))?;
    println!("Backup created:");
    println!("  original: {}", record.original_path.display());
    println!("  backup:   {}", record.backup_path.display());
    println!("  manifest: {}", record.manifest_path().display());
    println!("  sha256:   {}", record.sha256);
    println!("  size:     {} bytes", record.size);
    println!(
        "\nRestore with: game-opt-tool revert --game {}",
        record.original_path.display()
    );
    Ok(())
}

fn revert_command(
    path: &Path,
    manifest: Option<&Path>,
    dry_run: bool,
) -> Result<()> {
    let report = backup::restore(path, manifest, dry_run)?;

    if report.already_restored {
        println!(
            "No changes needed: {} already matches backup {}",
            report.target.display(),
            report.record.short_hash()
        );
        return Ok(());
    }

    let before = report
        .before_hash
        .as_deref()
        .map(short_hash)
        .unwrap_or("(missing)");

    if dry_run {
        println!("[dry-run] Would restore {}", report.target.display());
        println!("  current: {}", before);
        println!("  backup:  {}", report.record.short_hash());
        println!("  source:  {}", report.record.backup_path.display());
        return Ok(());
    }

    println!(
        "Restored {} from verified backup",
        report.target.display()
    );
    println!("  before: {}", before);
    println!("  after:  {} (verified)", short_hash(&report.after_hash));
    Ok(())
}

fn doctor_command(game: Option<&Path>) -> Result<()> {
    let mut failures = 0u32;
    let mut warnings = 0u32;

    println!("game-opt-tool doctor");
    println!("====================");

    let config_path = Path::new("config/games.yaml");
    if config_path.is_file() {
        match config::Config::load(config_path) {
            Ok(config) => println!(
                "[ok]   config: {} parses ({} games)",
                config_path.display(),
                config.games.len()
            ),
            Err(error) => {
                println!(
                    "[fail] config: {} failed to parse: {:#}",
                    config_path.display(),
                    error
                );
                failures += 1;
            }
        }
    } else {
        println!(
            "[warn] config: {} not found (run from the project root)",
            config_path.display()
        );
        warnings += 1;
    }

    println!();
    println!("External tools:");
    for (binary, purpose) in [
        ("wine", "running Windows builds"),
        ("mangohud", "frame-time capture"),
    ] {
        match find_in_path(binary) {
            Some(found) => println!("[ok]   {}: {} ({})", binary, found.display(), purpose),
            None => {
                println!("[warn] {}: not found on PATH ({})", binary, purpose);
                warnings += 1;
            }
        }
    }

    let java_home = std::env::var_os("JAVA_HOME").map(PathBuf::from);
    let ghidra_dir = std::env::var_os("GHIDRA_INSTALL_DIR").map(PathBuf::from);
    let required_java = ghidra_dir
        .as_ref()
        .and_then(|dir| decompiler::required_java_version(dir))
        .and_then(|version| version.parse::<u32>().ok());

    let java_from_home = java_home
        .as_ref()
        .map(|home| home.join("bin").join("java"))
        .filter(|path| path.is_file());
    let java_bin = if let Some(home) = &java_home {
        match java_from_home {
            Some(path) => Some(path),
            None => {
                println!(
                    "[warn] java: JAVA_HOME is set to {}, but it has no bin/java",
                    home.display()
                );
                warnings += 1;
                None
            }
        }
    } else {
        find_in_path("java")
    };

    match java_bin {
        Some(path) => {
            println!(
                "[ok]   java: {} (Ghidra headless decompilation)",
                path.display()
            );
            let javac = java_home
                .as_ref()
                .map(|home| home.join("bin").join("javac"))
                .filter(|path| path.is_file())
                .or_else(|| find_in_path("javac"));
            if javac.is_none() {
                println!("[warn] javac: not found; Ghidra needs a full JDK, not a JRE");
                warnings += 1;
            }
            if let (Some(home), Some(required)) = (&java_home, required_java) {
                if let Some(actual) = decompiler::java_major_version(home) {
                    if actual < required {
                        println!(
                            "[warn] java: {} is Java {}, but the installed Ghidra requires Java {}",
                            home.display(),
                            actual,
                            required
                        );
                        warnings += 1;
                    }
                }
            }
        }
        None => {
            if java_home.is_none() {
                println!(
                    "[warn] java: not found on PATH (Ghidra headless decompilation)"
                );
                warnings += 1;
            }
        }
    }

    match ghidra_dir.as_ref() {
        Some(dir) if decompiler::is_ghidra_install(dir) => {
            match decompiler::required_java_version(dir) {
                Some(version) => println!(
                    "[ok]   ghidra: {} (requires Java {})",
                    dir.display(),
                    version
                ),
                None => println!("[ok]   ghidra: {}", dir.display()),
            }
        }
        Some(dir) => {
            println!(
                "[warn] ghidra: {} is not a Ghidra installation (missing support/analyzeHeadless)",
                dir.display()
            );
            warnings += 1;
        }
        None => {
            println!(
                "[warn] ghidra: GHIDRA_INSTALL_DIR is not set (Ghidra headless decompilation)"
            );
            warnings += 1;
        }
    }

    if let Some(path) = game {
        println!();
        println!("Game target:");
        match game::GameInfo::from_path(path) {
            Ok(detected) => {
                println!(
                    "[ok]   detected: {} ({:?}, {})",
                    detected.name, detected.binary_format, detected.architecture
                );
                println!("       executable: {}", detected.path.display());

                match detected.path.parent() {
                    Some(dir) if check_writable(dir) => println!(
                        "[ok]   backups: directory is writable: {}",
                        dir.display()
                    ),
                    Some(dir) => {
                        println!(
                            "[fail] backups: directory is not writable: {}",
                            dir.display()
                        );
                        failures += 1;
                    }
                    None => {
                        println!("[fail] backups: executable has no parent directory");
                        failures += 1;
                    }
                }

                match backup::list_backups(&detected.path) {
                    Ok(records) if records.is_empty() => {
                        println!("[warn] backups: none yet (run: game-opt-tool backup --game <exe>)")
                    }
                    Ok(records) => println!(
                        "[ok]   backups: {} available (newest {})",
                        records.len(),
                        records[0].short_hash()
                    ),
                    Err(error) => {
                        println!("[fail] backups: {:#}", error);
                        failures += 1;
                    }
                }
            }
            Err(error) => {
                println!("[fail] cannot analyze {}: {:#}", path.display(), error);
                failures += 1;
            }
        }
    } else {
        println!();
        println!("[warn] no --game given; skipping executable checks");
        warnings += 1;
    }

    println!();
    if failures > 0 {
        bail!("{} prerequisite check(s) failed", failures);
    }
    println!("All required checks passed ({} warning(s)).", warnings);
    Ok(())
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }

        #[cfg(windows)]
        {
            let candidate = dir.join(format!("{}.exe", binary));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn check_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(
        ".game-opt-tool-write-test-{}",
        std::process::id()
    ));

    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn short_hash(hash: &str) -> &str {
    let end = hash.len().min(16);
    &hash[..end]
}
