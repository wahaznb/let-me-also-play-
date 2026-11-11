//! Deus Ex (2000) target detection.
//!
//! Deus Ex ships as a 32-bit Windows PE executable. Detection is deliberately
//! shallow: it checks the DOS header and the PE signature. It does not claim to
//! identify a particular build, patch level, or set of hot functions.

use anyhow::Context;

/// Deus Ex (2000) specific information.
#[derive(Debug, Clone)]
pub struct DeusExInfo;

impl DeusExInfo {
    /// Detect a PE executable that can be treated as a Deus Ex target.
    pub fn detect(game_path: &std::path::Path) -> anyhow::Result<DeusExInfo> {
        let data = std::fs::read(game_path).context("Failed to read game path")?;

        if data.len() >= 0x40 && data[0..2] == *b"MZ" {
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
                        return Ok(DeusExInfo);
                    }
                }
            }
        }

        anyhow::bail!("Not a PE executable")
    }
}
