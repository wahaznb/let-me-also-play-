//! Target-specific game detection.
//!
//! Each target module answers one question: can this executable be treated as
//! that game? Configuration and addresses live in `config/games.yaml` and in
//! the patch specs you write, not in hard-coded tables here.

use crate::game::GameInfo;
pub mod deus_ex;

/// Detect a known game and load its shared configuration.
pub fn detect_game(game_path: &std::path::Path) -> anyhow::Result<GameInfo> {
    if crate::targets::deus_ex::DeusExInfo::detect(game_path).is_ok() {
        let game_info = GameInfo::from_path(game_path)?;
        return Ok(game_info);
    }

    anyhow::bail!("Unsupported game binary");
}
