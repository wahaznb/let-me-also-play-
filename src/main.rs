//! Game Opt Tool — Main entry point
//!
//! A tool that reverse engineers and optimizes game binaries
//! to improve performance while preserving the original behavior.

mod cli;
mod config;
mod game;
mod profiler;
mod decompiler;
mod optimizer;
mod patcher;
mod verifier;
pub mod targets;

use anyhow::Result;
use clap::Parser;
use std::path::Path;
use tracing::{info, error};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser, Debug)]
#[command(name = "game-opt")]
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
    /// Analyze a game for performance issues
    Analyze {
        /// Path to the game executable
        #[arg(short, long)]
        game: std::path::PathBuf,
    },

    /// Find and profile hot functions
    Profile {
        /// Path to the game executable
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// Number of frames to profile
        #[arg(short, long, default_value = "100")]
        frames: u32,
    },

    /// Optimize specific functions
    Optimize {
        /// Path to the game executable
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// Functions to optimize (comma-separated)
        #[arg(short, long)]
        functions: Option<String>,
    },

    /// Apply optimization patches
    Patch {
        /// Path to the game executable
        #[arg(short, long)]
        game: std::path::PathBuf,

        /// Output path for patched executable
        #[arg(short, long)]
        output: std::path::PathBuf,
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
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let cli = Cli::parse();

    match cli.command {
        Some(Command::Analyze { game }) => {
            info!("Analyzing game: {:?}", game);
            
            // Check if this is Deus Ex
            if is_deus_ex(&game) {
                analyze_deus_ex(&game).await?;
            } else {
                analyze_game(&game).await?;
            }
        }
        Some(Command::Profile { game, frames }) => {
            info!("Profiling game: {:?} for {} frames", game, frames);
            profile_game(&game, frames).await?;
        }
        Some(Command::Optimize { game, functions, output }) => {
            info!("Optimizing game: {:?} (functions: {:?})", game, functions);
            
            if is_deus_ex(&game) {
                optimize_deus_ex(&game, functions.as_deref(), &output).await?;
            } else {
                optimize_game(&game, functions.as_deref()).await?;
            }
        }
        Some(Command::Patch { game, output }) => {
            info!("Patching game: {:?} → {:?}", game, output);
            
            if is_deus_ex(&game) {
                patch_deus_ex(&game, &output).await?;
            } else {
                patch_game(&game, &output).await?;
            }
        }
        Some(Command::Verify { original, patched }) => {
            info!("Verifying: {:?} vs {:?}", original, patched);
            
            if is_deus_ex(&original) {
                verify_deus_ex(&original, &patched).await?;
            } else {
                verify_optimization(&original, &patched).await?;
            }
        }
        Some(Command::ListGames) => {
            list_games();
        }
        Some(Command::Help { game }) => {
            show_help(&game);
        }
        None => {
            Cli::command().print_help()?;
            println!("\nTry: cargo run -- --help");
        }
    }

    Ok(())
}

fn is_deus_ex(path: &Path) -> bool {
    // Check if this looks like a Deus Ex installation
    path.join("System").join("DeusEx.exe").exists()
        || path.join("DeusEx.exe").exists()
        || path.join("Bin").join("DeusEx.exe").exists()
}

async fn analyze_deus_ex(game_path: &Path) -> Result<()> {
    use targets::deus_ex::{DeusExInfo, DeusExProfiler};
    
    let game_info = DeusExInfo::detect(game_path)?;
    let profiler = DeusExProfiler::new();
    let functions = profiler.find_hot_functions(&game_info).await?;
    
    println!("\n{}", profiler.generate_report(&functions));
    
    Ok(())
}

async fn optimize_deus_ex(
    game_path: &Path,
    functions: Option<&str>,
    output: &Path,
) -> Result<()> {
    use targets::deus_ex::{DeusExInfo, DeusExOptimizer, DeusExPatcher};
    
    let game_info = DeusExInfo::detect(game_path)?;
    let optimizer = DeusExOptimizer::new(game_info.clone());
    
    let optimizations = optimizer.optimize_all()?;
    
    for opt in &optimizations {
        println!("\n{}: {}% faster", opt.function_name, opt.estimated_speedup);
        println!("  Notes: {}", opt.notes);
    }
    
    let patcher = DeusExPatcher::new(game_info);
    patcher.apply_patches(&optimizations, output).await?;
    
    Ok(())
}

async fn patch_deus_ex(game_path: &Path, output: &Path) -> Result<()> {
    use targets::deus_ex::{DeusExInfo, DeusExOptimizer, DeusExPatcher};
    
    let game_info = DeusExInfo::detect(game_path)?;
    let optimizer = DeusExOptimizer::new(game_info.clone());
    let optimizations = optimizer.optimize_all()?;
    
    let patcher = DeusExPatcher::new(game_info);
    patcher.apply_patches(&optimizations, output).await?;
    
    Ok(())
}

async fn verify_deus_ex(original: &Path, patched: &Path) -> Result<()> {
    use targets::deus_ex::{DeusExInfo, DeusExVerifier};
    
    let game_info = DeusExInfo::detect(original)?;
    let verifier = DeusExVerifier::new(game_info);
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
    println!("\nUse 'game-opt help <game-name>' for more info.");
}

fn show_help(game: &str) {
    match game.to_lowercase().as_str() {
        "deus ex" | "deusex" => {
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

async fn analyze_game(game_path: &std::path::Path) -> Result<()> {
    use game::GameInfo;
    use profiler::Profiler;

    // Load game info
    let game = GameInfo::from_path(game_path)?;
    info!("Detected game: {} {}", game.name, game.version);
    info!("Architecture: {}", game.architecture);
    info!("Binary format: {:?}", game.binary_format);

    // Profile to find hot functions
    let profiler = Profiler::new();
    let hot_functions = profiler.find_hot_functions(game_path, 100).await?;

    info!("\nTop hot functions:");
    for (i, func) in hot_functions.iter().take(10).enumerate() {
        info!("  {}. {} — {:.2}% of frame time", i + 1, func.name, func.cpu_percent);
    }

    Ok(())
}

async fn profile_game(
    game_path: &std::path::Path,
    frames: u32,
) -> Result<()> {
    use profiler::Profiler;

    let profiler = Profiler::new();
    let stats = profiler.profile(game_path, frames).await?;

    info!("\nProfiling Results ({} frames):", frames);
    info!("  Average FPS: {:.2}", stats.avg_fps);
    info!("  Min FPS:     {:.2}", stats.min_fps);
    info!("  Max FPS:     {:.2}", stats.max_fps);
    info!("  Avg frame time: {:.2}ms", stats.avg_frame_time);
    info!("  P99 frame time: {:.2}ms", stats.p99_frame_time);

    Ok(())
}

async fn optimize_game(
    game_path: &std::path::Path,
    functions: Option<&str>,
) -> Result<()> {
    use decompiler::Decompiler;
    use optimizer::Optimizer;

    // Decompile target functions
    let decompiler = Decompiler::new();
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
                info!("  ✓ {} — {}% faster", result.function_name, result.speedup_pct);
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

    let patcher = Patcher::new();
    patcher.apply_patches(game_path, output_path).await?;

    info!("Patched game saved to: {:?}", output_path);
    Ok(())
}

async fn verify_optimization(
    original: &std::path::Path,
    patched: &std::path::Path,
) -> Result<()> {
    use verifier::Verifier;

    let verifier = Verifier::new();
    let report = verifier.verify(original, patched).await?;

    info!("\nVerification Report:");
    info!("  Behavioral match: {}", report.behavioral_match);
    info!("  Frame difference: {:.4}%", report.frame_diff_percent);
    info!("  Quirks preserved: {}", report.quirks_preserved);
    info!("  Performance gain: +{:.1}%", report.performance_gain_pct);

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
