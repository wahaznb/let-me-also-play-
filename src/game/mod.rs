//! Game detection and information

use crate::config::{Architecture, BinaryFormat, Config, GameConfig};
use anyhow::{Context, Result};
use goblin::elf::Elf;
use goblin::pe::PE;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Information about a detected game
#[derive(Debug, Clone)]
pub struct GameInfo {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub binary_format: BinaryFormat,
    pub architecture: Architecture,
    pub config: Option<GameConfig>,
}

impl GameInfo {
    /// Detect game information from a path
    pub fn from_path(path: &Path) -> Result<Self> {
        let path = std::fs::canonicalize(path)?;

        // Try to find the executable
        let exe_path = if path.is_file() {
            path.clone()
        } else {
            // Look for common executable names
            let candidates = vec![
                path.join("Game.exe"),
                path.join("bin").join("Game.exe"),
                path.join("Game").with_extension("exe"),
                path.join("bin").join("x64").join("Game.exe"),
                path.join("Game").with_extension("elf"),
                path.join("bin").join("Game").with_extension("elf"),
            ];

            candidates.into_iter().find(|p| p.exists()).unwrap_or(path.clone())
        };

        info!("Analyzing: {:?}", exe_path);

        // Detect binary format
        let (binary_format, architecture) = Self::detect_format(&exe_path)?;

        // Try to detect game name from directory or executable
        let name = Self::detect_game_name(&exe_path, &path)?;

        // Load config if available
        let config = Self::load_config(&name)?;

        Ok(Self {
            name,
            version: String::from("unknown"),
            path: exe_path,
            binary_format,
            architecture,
            config,
        })
    }

    /// Detect binary format and architecture
    fn detect_format(path: &Path) -> Result<(BinaryFormat, Architecture)> {
        let data = std::fs::read(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        // Try ELF first (Linux)
        if let Ok(elf) = Elf::parse(&data) {
            let arch = match elf.header.ptype {
                goblin::elf::header::ET_DYN | goblin::elf::header::ET_EXEC => {
                    match elf.header.e_machine {
                        goblin::elf::header::EM_X86_64 => (BinaryFormat::ELF, Architecture::X64),
                        goblin::elf::header::EM_386 => (BinaryFormat::ELF, Architecture::X86),
                        goblin::elf::header::EM_ARM => (BinaryFormat::ELF, Architecture::ARM),
                        goblin::elf::header::EM_AARCH64 => (BinaryFormat::ELF, Architecture::ARM64),
                        _ => (BinaryFormat::ELF, Architecture::Unknown),
                    }
                }
                _ => (BinaryFormat::ELF, Architecture::Unknown),
            };
            return Ok(arch);
        }

        // Try PE (Windows)
        if let Ok(pe) = PE::parse(&data) {
            let arch = match pe.headers.native.headers.option.magic {
                goblin::pe::constants::IMAGE_FILE_MACHINE_AMD64 => {
                    (BinaryFormat::PE, Architecture::X64)
                }
                goblin::pe::constants::IMAGE_FILE_MACHINE_I386 => {
                    (BinaryFormat::PE, Architecture::X86)
                }
                goblin::pe::constants::IMAGE_FILE_MACHINE_ARM64 => {
                    (BinaryFormat::PE, Architecture::ARM64)
                }
                _ => (BinaryFormat::PE, Architecture::Unknown),
            };
            return Ok(arch);
        }

        warn!("Unknown binary format");
        Ok((BinaryFormat::Unknown, Architecture::Unknown))
    }

    /// Detect game name from path
    fn detect_game_name(exe_path: &Path, game_dir: &Path) -> Result<String> {
        // Try executable name
        if let Some(name) = exe_path.file_stem() {
            let name_str = name.to_string_lossy();
            if !name_str.is_empty() && name_str != "Game" {
                return Ok(name_str.to_string());
            }
        }

        // Try directory name
        if let Some(name) = game_dir.file_name() {
            let name_str = name.to_string_lossy();
            if !name_str.is_empty() {
                return Ok(name_str.to_string());
            }
        }

        Ok("UnknownGame".to_string())
    }

    /// Load game-specific configuration
    fn load_config(game_name: &str) -> Result<Option<GameConfig>> {
        let config_paths = vec![
            PathBuf::from("config/games.yaml"),
            PathBuf::from("config").join(format!("{}.yaml", game_name.to_lowercase())),
        ];

        for path in config_paths {
            if path.exists() {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                let config: GameConfig = serde_yaml::from_str(&content)
                    .with_context(|| format!("Failed to parse {}", path.display()))?;
                return Ok(Some(config));
            }
        }

        Ok(None)
    }
}
