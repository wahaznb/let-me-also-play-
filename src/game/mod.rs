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
            // Look for common executable names (including Deus Ex layouts)
            let candidates = vec![
                path.join("System").join("DeusEx.exe"),
                path.join("DeusEx.exe"),
                path.join("Bin").join("DeusEx.exe"),
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
            let arch = match elf.header.e_type {
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
            // COFF machine type from the PE headers
            // In goblin 0.8, access via pe.headers.file_header.machine
            let machine_type = pe.header.coff_header.machine;
            let arch = match machine_type {
                0x14c => (BinaryFormat::PE, Architecture::X86),   // IMAGE_FILE_MACHINE_I386
                0x8664 => (BinaryFormat::PE, Architecture::X64),  // IMAGE_FILE_MACHINE_AMD64
                0xAA64 => (BinaryFormat::PE, Architecture::ARM64), // IMAGE_FILE_MACHINE_ARM64
                _ => (BinaryFormat::PE, Architecture::Unknown),
            };
            return Ok(arch);
        }

        // Try Mach-O (macOS). This is host-independent: a Mach-O binary can be
        // inspected on Linux or Windows too.
        if let Ok(mach) = goblin::mach::Mach::parse(&data) {
            let cputype = match &mach {
                goblin::mach::Mach::Binary(macho) => Some(macho.header.cputype),
                goblin::mach::Mach::Fat(fat) => fat
                    .arches()
                    .ok()
                    .and_then(|arches| arches.first().map(|arch| arch.cputype())),
            };
            let arch = match cputype {
                Some(goblin::mach::cputype::CPU_TYPE_X86) => (BinaryFormat::MachO, Architecture::X86),
                Some(goblin::mach::cputype::CPU_TYPE_X86_64) => {
                    (BinaryFormat::MachO, Architecture::X64)
                }
                Some(goblin::mach::cputype::CPU_TYPE_ARM) => {
                    (BinaryFormat::MachO, Architecture::ARM)
                }
                Some(goblin::mach::cputype::CPU_TYPE_ARM64) => {
                    (BinaryFormat::MachO, Architecture::ARM64)
                }
                Some(goblin::mach::cputype::CPU_TYPE_MIPS) => {
                    (BinaryFormat::MachO, Architecture::MIPS)
                }
                Some(goblin::mach::cputype::CPU_TYPE_POWERPC) => {
                    (BinaryFormat::MachO, Architecture::PowerPC)
                }
                _ => (BinaryFormat::MachO, Architecture::Unknown),
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

    /// Load game-specific configuration.
    ///
    /// `config/games.yaml` is a full [`Config`] document (it has `games`,
    /// `optimizations`, and `quirks` sections), so it must be parsed as such
    /// and then looked up by name. Per-game files are `GameConfig` documents.
    fn load_config(game_name: &str) -> Result<Option<GameConfig>> {
        let shared_path = PathBuf::from("config/games.yaml");
        if shared_path.exists() {
            let content = std::fs::read_to_string(&shared_path)
                .with_context(|| format!("Failed to read {}", shared_path.display()))?;
            let config: Config = serde_yaml::from_str(&content)
                .with_context(|| format!("Failed to parse {}", shared_path.display()))?;

            if let Some(game) = config.find_game(game_name) {
                return Ok(Some(game.clone()));
            }
        }

        // Optional per-game override, e.g. config/deusex.yaml
        let specific_path =
            PathBuf::from("config").join(format!("{}.yaml", game_name.to_lowercase()));
        if specific_path.exists() {
            let content = std::fs::read_to_string(&specific_path)
                .with_context(|| format!("Failed to read {}", specific_path.display()))?;
            let config: GameConfig = serde_yaml::from_str(&content)
                .with_context(|| format!("Failed to parse {}", specific_path.display()))?;
            return Ok(Some(config));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_current_executable_format() {
        let path = std::env::current_exe().unwrap();
        let (format, architecture) = GameInfo::detect_format(&path).unwrap();

        #[cfg(target_os = "linux")]
        assert_eq!(format, BinaryFormat::ELF);
        #[cfg(target_os = "macos")]
        assert_eq!(format, BinaryFormat::MachO);
        #[cfg(target_os = "windows")]
        assert_eq!(format, BinaryFormat::PE);

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(architecture, Architecture::X64);
        #[cfg(all(target_os = "linux", target_arch = "x86"))]
        assert_eq!(architecture, Architecture::X86);
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        assert_eq!(architecture, Architecture::ARM64);
        #[cfg(target_os = "macos")]
        assert!(
            matches!(architecture, Architecture::X64 | Architecture::ARM64),
            "unexpected macOS architecture: {architecture:?}"
        );
    }

    #[test]
    fn detects_thin_macho_headers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thin-macho");
        let mut data = Vec::new();
        data.extend_from_slice(&0xFEED_FACFu32.to_le_bytes()); // MH_MAGIC_64
        data.extend_from_slice(&0x0100_0007u32.to_le_bytes()); // CPU_TYPE_X86_64
        data.extend_from_slice(&3u32.to_le_bytes()); // CPU_SUBTYPE_X86_64_ALL
        data.extend_from_slice(&2u32.to_le_bytes()); // MH_EXECUTE
        data.extend_from_slice(&0u32.to_le_bytes()); // ncmds
        data.extend_from_slice(&0u32.to_le_bytes()); // sizeofcmds
        data.extend_from_slice(&0u32.to_le_bytes()); // flags
        data.extend_from_slice(&0u32.to_le_bytes()); // reserved
        std::fs::write(&path, data).unwrap();

        let (format, architecture) = GameInfo::detect_format(&path).unwrap();
        assert_eq!(format, BinaryFormat::MachO);
        assert_eq!(architecture, Architecture::X64);
    }

    #[test]
    fn unknown_bytes_are_reported_as_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-binary.txt");
        std::fs::write(&path, b"hello, this is not an executable").unwrap();

        let (format, architecture) = GameInfo::detect_format(&path).unwrap();
        assert_eq!(format, BinaryFormat::Unknown);
        assert_eq!(architecture, Architecture::Unknown);
    }

    #[test]
    fn shared_config_is_looked_up_by_name() {
        // Unit tests run with the crate root as the working directory.
        let config = GameInfo::load_config("Oblivion Remastered")
            .expect("config parse must not fail");
        let config = config.expect("Oblivion should be found in config/games.yaml");
        assert_eq!(config.architecture, Architecture::X64);
        assert_eq!(config.binary_format, BinaryFormat::PE);
    }

    #[test]
    fn unknown_game_has_no_config() {
        let config = GameInfo::load_config("No Such Game 12345").unwrap();
        assert!(config.is_none());
    }
}
