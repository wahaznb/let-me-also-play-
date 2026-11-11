//! Configuration management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Main configuration file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub games: HashMap<String, GameConfig>,
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

    /// Functions you have identified as interesting
    pub hot_functions: Vec<FunctionTarget>,

    /// Known quirks that must be preserved
    pub quirks: Vec<Quirk>,

    /// Free-form notes for this game
    pub optimization_hints: HashMap<String, String>,
}

/// A function of interest in a game binary
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

impl std::fmt::Display for Architecture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Architecture::X86 => write!(f, "x86"),
            Architecture::X64 => write!(f, "x64"),
            Architecture::ARM => write!(f, "arm"),
            Architecture::ARM64 => write!(f, "arm64"),
            Architecture::MIPS => write!(f, "mips"),
            Architecture::PowerPC => write!(f, "powerpc"),
            Architecture::Unknown => write!(f, "unknown"),
        }
    }
}

impl Config {
    /// Load configuration from file, choosing the parser by extension.
    pub fn load(path: &Path) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)?;
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match extension.as_str() {
            "yaml" | "yml" => Ok(serde_yaml::from_str(&content)?),
            "toml" => Ok(toml::from_str(&content)?),
            other => Err(anyhow::anyhow!(
                "Unsupported config extension {:?} for {} (expected .yaml, .yml, or .toml)",
                other,
                path.display()
            )),
        }
    }

    /// Save the configuration, choosing the serializer by extension.
    ///
    /// Kept for round-tripping a config you edited in code or in tests.
    #[allow(dead_code)]
    pub fn save(&self, path: &Path) -> Result<(), anyhow::Error> {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let content = match extension.as_str() {
            "yaml" | "yml" => serde_yaml::to_string(self)?,
            "toml" => toml::to_string_pretty(self)?,
            other => {
                return Err(anyhow::anyhow!(
                    "Unsupported config extension {:?} for {} (expected .yaml, .yml, or .toml)",
                    other,
                    path.display()
                ))
            }
        };

        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get game config by key or display name, tolerating punctuation and case.
    /// Look up a game by key or display name.
    #[allow(dead_code)]
    pub fn get_game(&self, name: &str) -> Option<&GameConfig> {
        self.find_game(name)
    }

    /// Find a game entry by key or display name.
    pub fn find_game(&self, name: &str) -> Option<&GameConfig> {
        if let Some(exact) = self.games.get(name) {
            return Some(exact);
        }

        let wanted = normalize_game_name(name);
        if wanted.is_empty() {
            return None;
        }

        self.games
            .iter()
            .find(|(key, game)| {
                let key_norm = normalize_game_name(key);
                let name_norm = normalize_game_name(&game.name);

                key_norm == wanted
                    || name_norm == wanted
                    || (key_norm.len() >= 3
                        && (wanted.contains(&key_norm) || key_norm.contains(&wanted)))
                    || (name_norm.len() >= 3
                        && (wanted.contains(&name_norm) || name_norm.contains(&wanted)))
            })
            .map(|(_, game)| game)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            games: HashMap::new(),
            quirks: QuirkDatabase {
                games: HashMap::new(),
            },
        }
    }
}

/// Normalize a game name for comparison: lowercase alphanumerics only.
pub fn normalize_game_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_yaml_config_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/games.yaml");
        let config = Config::load(&path).expect("config/games.yaml must parse");

        let deus_ex = config.games.get("deus-ex").expect("deus-ex entry");
        assert_eq!(deus_ex.architecture, Architecture::X86);
        assert_eq!(deus_ex.binary_format, BinaryFormat::PE);
        assert!(deus_ex.hot_functions.is_empty());
        assert!(config.quirks.games.is_empty());
    }

    #[test]
    fn get_game_is_case_insensitive() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/games.yaml");
        let config = Config::load(&path).unwrap();
        assert!(config.get_game("Deus Ex").is_some());
        assert!(config.get_game("deus-ex").is_some());
        assert!(config.get_game("DEUS-EX").is_some());
        assert!(config.get_game("does-not-exist").is_none());
    }

    #[test]
    fn unsupported_extension_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{}").unwrap();
        assert!(Config::load(&path).is_err());
    }

    #[test]
    fn name_normalization_ignores_punctuation_and_case() {
        assert_eq!(
            normalize_game_name("The Elder Scrolls IV: Oblivion"),
            "theelderscrollsivoblivion"
        );
        assert_eq!(normalize_game_name("Deus Ex"), "deusex");
        assert_eq!(normalize_game_name("DEUS-EX"), "deusex");
    }

    #[test]
    fn toml_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Config::default().save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert!(loaded.games.is_empty());
    }
}
