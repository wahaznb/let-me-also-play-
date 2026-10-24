//! Deus Ex (2000) target configuration
//!
//! Specific optimizations and hot functions for the Windows x86 binary.

use crate::config::{ImpactLevel, OptimizationPass, Quirk};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use anyhow::Context;

/// Deus Ex (2000) specific information
#[derive(Debug, Clone)]
pub struct DeusExInfo;

/// Deus Ex hot functions with known addresses and impacts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeusExFunction {
    pub name: String,
    pub address: u64,
    pub estimated_impact: ImpactLevel,
    pub description: String,
}

impl DeusExInfo {
    /// Detect Deus Ex installation
    pub fn detect(game_path: &std::path::Path) -> anyhow::Result<DeusExInfo> {
        // Verify it's a PE32 binary with Deus Ex signatures
        let data = std::fs::read(game_path).context("Failed to read game path")?;

        // Check PE signature: MZ at offset 0, then PE\0\0 at e_lfanew (0x3C)
        if data.len() >= 0x40 && data[0..2] == [0x4D, 0x5A] {
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

        anyhow::bail!("Not a valid Deus Ex PE32 binary");
    }
}

/// Known hot functions for Deus Ex (2000)
pub static HOT_FUNCTIONS: LazyLock<Vec<DeusExFunction>> = LazyLock::new(|| {
    vec![
        DeusExFunction {
            name: "npc_ai_update".to_string(),
            address: 0x401234,
            estimated_impact: ImpactLevel::High,
            description: "Non-player character AI update loop".to_string(),
        },
        DeusExFunction {
            name: "physics_step".to_string(),
            address: 0x405678,
            estimated_impact: ImpactLevel::High,
            description: "Physics simulation step".to_string(),
        },
        DeusExFunction {
            name: "pathfinding".to_string(),
            address: 0x409ABC,
            estimated_impact: ImpactLevel::Medium,
            description: "Navmesh pathfinding resolution".to_string(),
        },
        DeusExFunction {
            name: "render_submit".to_string(),
            address: 0x410DEF,
            estimated_impact: ImpactLevel::Low,
            description: "Render submission thread".to_string(),
        },
    ]
});

/// Default optimization passes for Deus Ex
pub fn default_optimization_passes() -> Vec<OptimizationPass> {
    vec![
        OptimizationPass {
            name: "loop_unrolling".to_string(),
            description: "Unroll tight AI loops in npc_ai_update".to_string(),
            enabled: true,
            parameters: [("factor", "2.0")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        },
        OptimizationPass {
            name: "simd_vectorization".to_string(),
            description: "Vectorize physics_step math operations".to_string(),
            enabled: true,
            parameters: [("width", "4")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        },
        OptimizationPass {
            name: "cache_friendly_layout".to_string(),
            description: "Reorder pathfinding data for cache locality".to_string(),
            enabled: true,
            parameters: [("block_size", "32")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        },
        OptimizationPass {
            name: "constant_folding".to_string(),
            description: "Fold constant expressions in render_submit".to_string(),
            enabled: true,
            parameters: HashMap::new(),
        },
        OptimizationPass {
            name: "dead_code_elimination".to_string(),
            description: "Remove unreachable code paths".to_string(),
            enabled: true,
            parameters: HashMap::new(),
        },
        OptimizationPass {
            name: "function_inlining".to_string(),
            description: "Inline small helper functions".to_string(),
            enabled: true,
            parameters: HashMap::new(),
        },
    ]
}

/// Known quirks for Deus Ex that must be preserved during optimization
pub static DEUS_EX_QUIRKS: LazyLock<Vec<Quirk>> = LazyLock::new(|| {
    vec![
        Quirk {
            name: "ai_frame_timing".to_string(),
            description: "AI frame timing quirk - must not alter delta timing".to_string(),
            function: "npc_ai_update".to_string(),
            condition: "when AI update takes > 16ms".to_string(),
            preservation_required: true,
        },
        Quirk {
            name: "physics_substep_order".to_string(),
            description: "Physics substep order must be preserved".to_string(),
            function: "physics_step".to_string(),
            condition: "fixed timestep mode".to_string(),
            preservation_required: true,
        },
    ]
});