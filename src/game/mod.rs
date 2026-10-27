//! Game detection and information

use crate::config::{Architecture, BinaryFormat, Config, GameConfig};
use anyhow::{Context, Result};
use goblin::elf::Elf;
use goblin::pe::PE;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

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
        let data =
            std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;

        if let Some(found) = detect_elf(&data) {
            return Ok(found);
        }
        if let Some(found) = detect_pe(&data) {
            return Ok(found);
        }
        if let Some(found) = detect_macho(&data) {
            return Ok(found);
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

fn detect_elf(data: &[u8]) -> Option<(BinaryFormat, Architecture)> {
    let elf = Elf::parse(data).ok()?;
    let architecture = match elf.header.e_type {
        goblin::elf::header::ET_DYN | goblin::elf::header::ET_EXEC => match elf.header.e_machine {
            goblin::elf::header::EM_X86_64 => Architecture::X64,
            goblin::elf::header::EM_386 => Architecture::X86,
            goblin::elf::header::EM_ARM => Architecture::ARM,
            goblin::elf::header::EM_AARCH64 => Architecture::ARM64,
            _ => Architecture::Unknown,
        },
        _ => Architecture::Unknown,
    };
    Some((BinaryFormat::ELF, architecture))
}

fn detect_pe(data: &[u8]) -> Option<(BinaryFormat, Architecture)> {
    let pe = PE::parse(data).ok()?;
    let architecture = match pe.header.coff_header.machine {
        0x14c => Architecture::X86,    // IMAGE_FILE_MACHINE_I386
        0x8664 => Architecture::X64,   // IMAGE_FILE_MACHINE_AMD64
        0xAA64 => Architecture::ARM64, // IMAGE_FILE_MACHINE_ARM64
        _ => Architecture::Unknown,
    };
    Some((BinaryFormat::PE, architecture))
}

fn detect_macho(data: &[u8]) -> Option<(BinaryFormat, Architecture)> {
    match goblin::mach::Mach::parse(data) {
        Ok(goblin::mach::Mach::Binary(binary)) => Some((
            BinaryFormat::MachO,
            macho_architecture(binary.header.cputype),
        )),
        Ok(goblin::mach::Mach::Fat(fat)) => {
            // Reject fat headers whose slices are not thin Mach-O images.
            // Java .class files share the FAT_MAGIC value, so this check
            // keeps them out of the Mach-O path.
            let arches = match fat.arches() {
                Ok(arches) if valid_fat_arches(data, &arches) => arches,
                Ok(_) => return None,
                Err(error) => {
                    debug!("failed to parse fat Mach-O slices: {error}");
                    return None;
                }
            };
            let arch = preferred_fat_arch(&arches)?;
            Some((BinaryFormat::MachO, macho_architecture(arch.cputype())))
        }
        Err(_) if is_fat_cigam(data) => Some((BinaryFormat::MachO, Architecture::Unknown)),
        Err(_) => None,
    }
}

fn macho_architecture(cputype: u32) -> Architecture {
    match cputype {
        goblin::mach::cputype::CPU_TYPE_X86 => Architecture::X86,
        goblin::mach::cputype::CPU_TYPE_X86_64 => Architecture::X64,
        goblin::mach::cputype::CPU_TYPE_ARM => Architecture::ARM,
        goblin::mach::cputype::CPU_TYPE_ARM64 => Architecture::ARM64,
        goblin::mach::cputype::CPU_TYPE_MIPS => Architecture::MIPS,
        goblin::mach::cputype::CPU_TYPE_POWERPC | goblin::mach::cputype::CPU_TYPE_POWERPC64 => {
            Architecture::PowerPC
        }
        _ => Architecture::Unknown,
    }
}

/// On macOS a universal binary usually contains both an Intel and an Apple
/// Silicon slice. Prefer the slice matching the running host. Other hosts have
/// no meaningful "current" slice, so they fall back to the first one.
fn preferred_fat_arch(
    arches: &[goblin::mach::fat::FatArch],
) -> Option<&goblin::mach::fat::FatArch> {
    const HOST_CPUTYPE: Option<u32> = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(goblin::mach::cputype::CPU_TYPE_ARM64)
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some(goblin::mach::cputype::CPU_TYPE_X86_64)
    } else {
        None
    };

    HOST_CPUTYPE
        .and_then(|host| arches.iter().find(|arch| arch.cputype() == host))
        .or_else(|| arches.first())
}

fn valid_fat_arches(data: &[u8], arches: &[goblin::mach::fat::FatArch]) -> bool {
    !arches.is_empty()
        && arches.len() <= 32
        && arches.iter().all(|arch| {
            let start = arch.offset as usize;
            let end = start.saturating_add(arch.size as usize);
            arch.size > 0 && end <= data.len() && data.get(start..end).is_some_and(is_thin_macho)
        })
}

fn is_thin_macho(data: &[u8]) -> bool {
    data.len() >= 4
        && matches!(
            u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
            0xFEED_FACE | 0xFEED_FACF | 0xCEFA_EDFE | 0xCFFA_EDFE
        )
}

/// Goblin does not parse byte-swapped fat headers. Classify them by magic,
/// but leave the architecture unknown because the slice table was not read.
fn is_fat_cigam(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }
    let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let nfat_arch = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    magic == goblin::mach::fat::FAT_CIGAM && (1..=32).contains(&nfat_arch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thin_macho(cputype: u32) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&0xFEED_FACFu32.to_le_bytes()); // MH_MAGIC_64
        data.extend_from_slice(&cputype.to_le_bytes());
        data.extend_from_slice(&3u32.to_le_bytes()); // CPU_SUBTYPE_X86_64_ALL
        data.extend_from_slice(&2u32.to_le_bytes()); // MH_EXECUTE
        data.extend_from_slice(&0u32.to_le_bytes()); // ncmds
        data.extend_from_slice(&0u32.to_le_bytes()); // sizeofcmds
        data.extend_from_slice(&0u32.to_le_bytes()); // flags
        data.extend_from_slice(&0u32.to_le_bytes()); // reserved
        data
    }

    fn fat_macho(cputypes: &[u32]) -> Vec<u8> {
        const SLICE_SIZE: usize = 32;
        let header_size = 8 + cputypes.len() * 20;
        let mut data = Vec::new();
        data.extend_from_slice(&0xCAFE_BABEu32.to_be_bytes()); // FAT_MAGIC
        data.extend_from_slice(&(cputypes.len() as u32).to_be_bytes());

        let mut offset = header_size;
        for cputype in cputypes {
            data.extend_from_slice(&cputype.to_be_bytes());
            data.extend_from_slice(&3u32.to_be_bytes()); // cpusubtype
            data.extend_from_slice(&(offset as u32).to_be_bytes());
            data.extend_from_slice(&(SLICE_SIZE as u32).to_be_bytes());
            data.extend_from_slice(&2u32.to_be_bytes()); // align 2^2
            offset += SLICE_SIZE;
        }

        for cputype in cputypes {
            data.extend_from_slice(&thin_macho(*cputype));
        }
        data
    }

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
        std::fs::write(&path, thin_macho(goblin::mach::cputype::CPU_TYPE_X86_64)).unwrap();

        let (format, architecture) = GameInfo::detect_format(&path).unwrap();
        assert_eq!(format, BinaryFormat::MachO);
        assert_eq!(architecture, Architecture::X64);
    }

    #[test]
    fn fat_macho_prefers_the_host_slice() {
        let data = fat_macho(&[
            goblin::mach::cputype::CPU_TYPE_X86_64,
            goblin::mach::cputype::CPU_TYPE_ARM64,
        ]);
        let (format, architecture) = detect_macho(&data).unwrap();
        assert_eq!(format, BinaryFormat::MachO);

        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert_eq!(architecture, Architecture::ARM64);
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        assert_eq!(architecture, Architecture::X64);
    }

    #[test]
    fn unknown_macho_cputype_reports_unknown_architecture() {
        let (format, architecture) = detect_macho(&thin_macho(0xDEAD_BEEF)).unwrap();
        assert_eq!(format, BinaryFormat::MachO);
        assert_eq!(architecture, Architecture::Unknown);
    }

    #[test]
    fn fat_macho_rejects_non_macho_slices() {
        let mut data = fat_macho(&[goblin::mach::cputype::CPU_TYPE_X86_64]);
        let slice_offset = 8 + 20;
        data[slice_offset] = 0;
        assert!(detect_macho(&data).is_none());
    }

    #[test]
    fn java_class_magic_is_not_macho() {
        let mut data = Vec::new();
        data.extend_from_slice(&0xCAFE_BABEu32.to_be_bytes()); // Java class magic
        data.extend_from_slice(&52u32.to_be_bytes()); // class version, read as nfat_arch
        data.resize(2048, 0);
        assert!(detect_macho(&data).is_none());
    }

    #[test]
    fn byte_swapped_fat_macho_is_recognized_by_magic() {
        let mut data = Vec::new();
        data.extend_from_slice(&0xBEBA_FECAu32.to_be_bytes()); // FAT_CIGAM
        data.extend_from_slice(&1u32.to_le_bytes()); // nfat_arch
        let (format, architecture) = detect_macho(&data).unwrap();
        assert_eq!(format, BinaryFormat::MachO);
        assert_eq!(architecture, Architecture::Unknown);
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
