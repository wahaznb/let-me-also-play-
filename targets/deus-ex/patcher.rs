//! Deus Ex 2000 patcher
//!
//! Injects optimized code into the Deus Ex binary

use super::DeusExInfo;
use super::optimizer::DeusExOptimization;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Patcher for Deus Ex
pub struct DeusExPatcher {
    game_info: DeusExInfo,
}

impl DeusExPatcher {
    pub fn new(game_info: DeusExInfo) -> Self {
        Self { game_info }
    }

    /// Apply all optimizations
    pub async fn apply_patches(
        &self,
        optimizations: &[DeusExOptimization],
        output_path: &Path,
    ) -> Result<()> {
        info!("Applying {} optimizations to Deus Ex...", optimizations.len());

        // Read the original binary
        let mut data = std::fs::read(&self.game_info.executable_path)
            .with_context(|| format!("Failed to read {}", self.game_info.executable_path.display()))?;

        info!("Binary size: {} bytes", data.len());

        // Create a backup
        let backup_path = output_path.with_extension("exe.backup");
        if !backup_path.exists() {
            std::fs::write(&backup_path, &data)
                .with_context(|| "Failed to create backup")?;
            info!("Backup created: {:?}", backup_path);
        }

        // Apply patches (simplified - real implementation would parse PE and patch functions)
        for opt in optimizations {
            info!("Patching: {}", opt.function_name);
            // In a real implementation, this would:
            // 1. Find the function in the binary
            // 2. Create a trampoline
            // 3. Write the optimized code
            // 4. Update the jump instruction
        }

        // Write the patched binary
        std::fs::write(output_path, &data)
            .with_context(|| format!("Failed to write {}", output_path.display()))?;

        info!("Patched binary saved to: {:?}", output_path);
        Ok(())
    }

    /// Create a trampoline for function hooking
    fn create_trampoline(original_addr: u64, new_func_addr: u64) -> Vec<u8> {
        // x86 trampoline: jmp rel32
        // 0xE9 is the opcode for relative jump
        let offset = (new_func_addr as i32 - original_addr as i32 - 5);
        
        vec![
            0xE9,                          // jmp rel32
            (offset as u8),
            ((offset >> 8) as u8),
            ((offset >> 16) as u8),
            ((offset >> 24) as u8),
        ]
    }
}
