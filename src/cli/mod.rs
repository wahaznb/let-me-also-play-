//! Command-line interface definitions

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "game-opt")]
#[command(about = "Reverse engineer and optimize game binaries for better performance")]
pub struct Cli {
    /// Path to the game directory or executable
    #[arg(short, long)]
    pub game_path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Analyze a game for performance issues
    Analyze {
        /// Path to the game executable
        #[arg(short, long)]
        game: PathBuf,
    },

    /// Profile the game to find hot functions
    Profile {
        /// Path to the game executable
        #[arg(short, long)]
        game: PathBuf,

        /// Number of frames to profile
        #[arg(short, long, default_value = "100")]
        frames: u32,
    },

    /// Decompile and optimize specific functions
    Optimize {
        /// Path to the game executable
        #[arg(short, long)]
        game: PathBuf,

        /// Specific functions to optimize (comma-separated names or addresses)
        #[arg(short, long)]
        functions: Option<String>,

        /// Output directory for optimized code
        #[arg(short, long, default_value = "./optimized")]
        output: PathBuf,
    },

    /// Apply optimization patches to the game
    Patch {
        /// Path to the game executable
        #[arg(short, long)]
        game: PathBuf,

        /// Output path for patched executable
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Verify the patched game behaves identically
    Verify {
        /// Original executable
        #[arg(short, long)]
        original: PathBuf,

        /// Patched executable
        #[arg(short, long)]
        patched: PathBuf,
    },

    /// List supported games and their configurations
    ListGames,

    /// Show help for a specific game target
    Help {
        /// Game name
        game: String,
    },
}
