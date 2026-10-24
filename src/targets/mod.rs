//! Target-specific game configurations
//!
//! Modules for different game targets and their optimization profiles.

use crate::game::GameInfo;
pub mod deus_ex;

/// Detect and load configuration for a known game
pub fn detect_game(game_path: &std::path::Path) -> anyhow::Result<GameInfo> {

    // Try Deus Ex (2000) detection
    if crate::targets::deus_ex::DeusExInfo::detect(game_path).is_ok() {
        let game_info = GameInfo::from_path(game_path)?;
        return Ok(game_info);
    }

    // Add other game detections here...

    anyhow::bail!("Unsupported game binary");
}