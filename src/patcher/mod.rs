//! Binary patching module
//!
//! Injects optimized code into game binaries.
//!
//! NOTE: code generation is not implemented yet. `apply_patches` currently
//! writes a verified byte-for-byte copy, and it refuses to overwrite the
//! original in place unless a backup already exists.

use crate::backup::write_atomic;
use crate::config::BinaryFormat;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use tracing::info;

/// Patcher for modifying game binaries
pub struct Patcher {
    // In a full implementation, this would handle:
    // - Function address resolution
    // - Trampoline creation
    // - Code cave allocation
}

impl Patcher {
    pub fn new() -> Self {
        Self {}
    }

    /// Apply patches to a game binary.
    pub async fn apply_patches(
        &self,
        game_path: &Path,
        output_path: &Path,
    ) -> Result<()> {
        info!("Loading game binary: {:?}", game_path);

        let mut data = Vec::new();
        std::fs::File::open(game_path)
            .with_context(|| format!("Failed to open {}", game_path.display()))?
            .read_to_end(&mut data)?;

        if data.is_empty() {
            bail!("Refusing to patch {}: file is empty", game_path.display());
        }

        info!("Binary size: {} bytes", data.len());

        let format = self.detect_format(&data).with_context(|| {
            format!(
                "Cannot patch {}: unsupported binary format",
                game_path.display()
            )
        })?;
        info!("Detected format: {:?}", format);

        if output_path.is_dir() {
            bail!(
                "Output path {} is a directory; pass an executable file path",
                output_path.display()
            );
        }

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create output directory {}", parent.display())
            })?;
        }

        // In-place writes are only allowed when a backup exists. The CLI
        // creates one automatically, but library callers get the same guard.
        if same_file(game_path, output_path) {
            let backups = crate::backup::list_backups(game_path)?;
            if backups.is_empty() {
                bail!(
                    "Refusing to overwrite {} in place without a backup. \
                     Run `backup --game <path>` first.",
                    game_path.display()
                );
            }
            info!("In-place write detected; verified backup is available");
        }

        info!(
            "No code transformations are implemented yet; writing a byte-for-byte copy"
        );
        write_atomic(output_path, &data, Some(game_path)).with_context(|| {
            format!("Failed to write patched binary {}", output_path.display())
        })?;

        info!("Patched binary saved");
        Ok(())
    }

    /// Detect binary format from the file header.
    fn detect_format(&self, data: &[u8]) -> Result<BinaryFormat> {
        // PE: DOS "MZ" header, then "PE\0\0" at e_lfanew (0x3C).
        if data.len() >= 0x40 && data[0] == b'M' && data[1] == b'Z' {
            let pe_offset = u32::from_le_bytes([
                data[0x3C],
                data[0x3D],
                data[0x3E],
                data[0x3F],
            ]) as usize;

            if pe_offset >= 0x40 {
                if let Some(signature_end) = pe_offset.checked_add(4) {
                    if signature_end <= data.len()
                        && &data[pe_offset..signature_end] == b"PE\0\0"
                    {
                        return Ok(BinaryFormat::PE);
                    }
                }
            }
        }

        // ELF magic: 0x7F 'E' 'L' 'F'
        if data.len() >= 4
            && data[0] == 0x7F
            && data[1] == b'E'
            && data[2] == b'L'
            && data[3] == b'F'
        {
            return Ok(BinaryFormat::ELF);
        }

        // Mach-O magic, both byte orders (thin 32/64-bit and fat).
        if data.len() >= 4 {
            let bytes = [data[0], data[1], data[2], data[3]];
            let magic_be = u32::from_be_bytes(bytes);
            let magic_le = u32::from_le_bytes(bytes);
            const MACHO_MAGICS: [u32; 4] =
                [0xFEED_FACE, 0xFEED_FACF, 0xCAFE_BABE, 0xCAFE_BABF];

            if MACHO_MAGICS.contains(&magic_be) || MACHO_MAGICS.contains(&magic_le) {
                return Ok(BinaryFormat::MachO);
            }
        }

        bail!(
            "Unknown binary format (first bytes: {:02X?})",
            &data[..data.len().min(8)]
        )
    }

    /// Create an x86/x86-64 `jmp rel32` trampoline.
    fn create_trampoline(
        &self,
        original_address: u64,
        new_function_address: u64,
    ) -> Result<Vec<u8>> {
        // rel32 is measured from the end of the 5-byte instruction.
        let offset =
            new_function_address as i128 - original_address as i128 - 5;

        if offset < i32::MIN as i128 || offset > i32::MAX as i128 {
            bail!(
                "Trampoline target 0x{:X} is out of rel32 range from 0x{:X}",
                new_function_address,
                original_address
            );
        }

        let mut bytes = Vec::with_capacity(5);
        bytes.push(0xE9); // jmp rel32
        bytes.extend_from_slice(&(offset as i32).to_le_bytes());
        Ok(bytes)
    }
}

/// True when both paths refer to the same file. Works when the destination
/// does not exist yet by normalizing it against the current directory.
fn same_file(a: &Path, b: &Path) -> bool {
    let canonical_a = std::fs::canonicalize(a).ok();
    let canonical_b = std::fs::canonicalize(b).ok();

    match (canonical_a, canonical_b) {
        (Some(a), Some(b)) => a == b,
        _ => absolute(a) == absolute(b),
    }
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pe_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[0] = b'M';
        data[1] = b'Z';
        let pe_offset: u32 = 0x80;
        data[0x3C..0x40].copy_from_slice(&pe_offset.to_le_bytes());
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        data
    }

    #[test]
    fn detects_pe_binary() {
        assert_eq!(
            Patcher::new().detect_format(&pe_bytes()).unwrap(),
            BinaryFormat::PE
        );
    }

    #[test]
    fn rejects_mz_without_pe_signature() {
        let mut data = pe_bytes();
        data[0x80..0x84].copy_from_slice(b"NE\0\0");
        assert!(Patcher::new().detect_format(&data).is_err());
    }

    #[test]
    fn detects_elf_binary() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"\x7FELF");
        assert_eq!(
            Patcher::new().detect_format(&data).unwrap(),
            BinaryFormat::ELF
        );
    }

    #[test]
    fn detects_macho_both_endiannesses() {
        let patcher = Patcher::new();

        let mut big_endian = vec![0u8; 4];
        big_endian.copy_from_slice(&0xFEED_FACEu32.to_be_bytes());
        assert_eq!(patcher.detect_format(&big_endian).unwrap(), BinaryFormat::MachO);

        let mut little_endian = vec![0u8; 4];
        little_endian.copy_from_slice(&0xFEED_FACFu32.to_le_bytes());
        assert_eq!(
            patcher.detect_format(&little_endian).unwrap(),
            BinaryFormat::MachO
        );
    }

    #[test]
    fn rejects_unknown_and_tiny_inputs() {
        let patcher = Patcher::new();
        assert!(patcher.detect_format(b"not a binary").is_err());
        assert!(patcher.detect_format(b"").is_err());
        assert!(patcher.detect_format(b"MZ").is_err());
    }

    #[test]
    fn trampoline_encoding_is_correct() {
        let bytes = Patcher::new()
            .create_trampoline(0x1000, 0x2000)
            .unwrap();
        // 0x2000 - 0x1000 - 5 = 0xFFB
        assert_eq!(bytes, vec![0xE9, 0xFB, 0x0F, 0x00, 0x00]);
    }

    #[test]
    fn trampoline_rejects_out_of_range_target() {
        let patcher = Patcher::new();
        assert!(patcher.create_trampoline(0, u32::MAX as u64).is_err());
        assert!(patcher.create_trampoline(u32::MAX as u64, 0).is_err());
    }

    #[tokio::test]
    async fn apply_patches_writes_an_identical_copy() {
        let dir = tempfile::tempdir().unwrap();
        let game = std::env::current_exe().unwrap();
        let output = dir.path().join("patched.bin");

        Patcher::new().apply_patches(&game, &output).await.unwrap();

        assert_eq!(
            std::fs::read(&output).unwrap(),
            std::fs::read(&game).unwrap()
        );
    }

    #[tokio::test]
    async fn in_place_write_requires_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game.bin");
        std::fs::copy(std::env::current_exe().unwrap(), &game).unwrap();

        let error = Patcher::new()
            .apply_patches(&game, &game)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("without a backup"));

        crate::backup::create(&game).unwrap();
        Patcher::new().apply_patches(&game, &game).await.unwrap();
    }

    #[tokio::test]
    async fn directory_output_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let game = std::env::current_exe().unwrap();

        let error = Patcher::new()
            .apply_patches(&game, dir.path())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("is a directory"));
    }
}
