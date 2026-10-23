//! Configuration management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Main configuration file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub games: HashMap<String, GameConfig>,
    pub optimizations: Vec<OptimizationPass>,
    pub quirks: QuirkDatabase,
}

/// Per-game configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameConfig {
    pub name: String,
    pub version: String,
    pub engine: String,
    pub binary_format: BinaryFormat,
    pub architecture: Architecture,

    /// Known hot functions to optimize
    pub hot_functions: Vec<FunctionTarget>,

    /// Known quirks that must be preserved
    pub quirks: Vec<Quirk>,

    /// Optimization hints for this game
    pub optimization_hints: HashMap<String, String>,
}

/// A function to optimize
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionTarget {
    pub name: String,
    pub address: Option<u64>,
    pub symbol: Option<String>,
    pub estimated_impact: ImpactLevel,
}

/// Impact level estimation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ImpactLevel {
    Critical,  // Major performance impact
    High,      // Significant impact
    Medium,    // Moderate impact
    Low,       // Minor impact
}

/// A known quirk to preserve
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quirk {
    pub name: String,
    pub description: String,
    pub function: String,
    pub condition: String,
    pub preservation_required: bool,
}

/// Database of quirks per game
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuirkDatabase {
    pub games: HashMap<String, Vec<Quirk>>,
}

/// Binary format
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BinaryFormat {
    PE,      // Windows executable
    ELF,     // Linux executable
    MachO,   // macOS executable
    Unknown,
}

/// Architecture
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Architecture {
    X86,
    X64,
    ARM,
    ARM64,
    MIPS,
    PowerPC,
    Unknown,
}

/// An optimization pass to apply
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationPass {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub parameters: HashMap<String, String>,
}

impl Config {
    /// Load configuration from file
    pub fn load(path: &Path) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    /// Save configuration to file
    pub fn save(&self, path: &Path) -> Result<(), anyhow::Error> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get game config by name
    pub fn get_game(&self, name: &str) -> Option<&GameConfig> {
        self.games.get(name)
    }

    /// Get quirks for a game
    pub fn get_quirks(&self, game: &str) -> &[Quirk] {
        self.quirks.games.get(game).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            games: HashMap::new(),
            optimizations: vec![],
            quirks: QuirkDatabase {
                games: HashMap::new(),
            },
        }
    }
}
