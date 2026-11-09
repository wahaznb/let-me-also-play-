//! Reverse-Engineering Analysis (REA) — static candidate detection.
//!
//! REA is the "analyze → decide → patch" loop without any measurement:
//!
//! 1. **analyze** — linear-sweep the executable sections with `iced-x86`.
//! 2. **decide** — flag *provably safe* dead code. Every candidate is
//!    length-preserving.
//! 3. **patch** — emit a `PatchSpec` and apply it with the verified Patcher,
//!    which backs up the original, writes atomically, and records a manifest.
//! 4. **revert** — `unpatch` restores exactly, byte for byte.
//!
//! No game is launched. No timing is measured. The sweep is linear, not
//! control-flow-aware, so a candidate found inside embedded data is a false
//! positive; the report says so and the user should review addresses before
//! applying to a shipping binary.

pub mod decode;
pub mod candidates;
pub mod patch;

use crate::game::GameInfo;
use crate::patcher::pe::PeInfo;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// One executable region that was scanned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannedSection {
    pub name: String,
    pub va: u64,
    pub bytes: usize,
    pub instructions: usize,
    pub data_skips: usize,
    pub candidates: usize,
}

/// Result of one REA run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReaReport {
    pub game: String,
    pub binary_format: String,
    pub architecture: String,
    pub executable: String,
    pub sections_scanned: Vec<ScannedSection>,
    pub candidates_total: usize,
    #[serde(default)]
    pub candidates: Vec<candidates::Candidate>,
    pub candidates_by_kind: std::collections::HashMap<String, usize>,
    pub candidates_by_class: std::collections::HashMap<String, usize>,
    pub patch_spec: Option<String>,
    pub patch_path: Option<String>,
    pub manifest_path: Option<String>,
    pub reverted: bool,
    pub notes: Vec<String>,
}

impl ReaReport {
    pub fn empty(game: &GameInfo) -> Self {
        Self {
            game: game.name.clone(),
            binary_format: format!("{:?}", game.binary_format),
            architecture: game.architecture.to_string(),
            executable: game.path.display().to_string(),
            sections_scanned: Vec::new(),
            candidates_total: 0,
            candidates: Vec::new(),
            candidates_by_kind: std::collections::HashMap::new(),
            candidates_by_class: std::collections::HashMap::new(),
            patch_spec: None,
            patch_path: None,
            manifest_path: None,
            reverted: false,
            notes: Vec::new(),
        }
    }
}

/// Scan the executable sections of a PE binary for REA candidates.
///
/// `max_bytes_per_section` bounds how much of each section is scanned. The
/// default is generous (1 MiB) but never unbounded.
pub fn analyze(
    game: &GameInfo,
    max_bytes_per_section: usize,
) -> Result<(Vec<candidates::Candidate>, ReaReport)> {
    if game.binary_format != crate::config::BinaryFormat::PE {
        bail!(
            "REA currently only scans PE binaries; {:?} is {:?}",
            game.path.display(),
            game.binary_format
        );
    }
    let machine = match game.architecture {
        crate::config::Architecture::X86 => 0x014C,
        crate::config::Architecture::X64 => 0x8664,
        crate::config::Architecture::ARM64 => 0xAA64,
        _ => 0x014C,
    };
    let bitness = if matches!(game.architecture, crate::config::Architecture::X64) {
        64
    } else {
        32
    };

    let data = std::fs::read(&game.path)
        .with_context(|| format!("Failed to read {}", game.path.display()))?;
    let pe = PeInfo::parse(&data)
        .with_context(|| format!("Cannot parse PE headers in {}", game.path.display()))?;
    if pe.machine != machine {
        bail!(
            "Architecture mismatch: config says {:?} ({:#X}), PE says {:#X}",
            game.architecture,
            machine,
            pe.machine
        );
    }

    let mut all_candidates = Vec::new();
    let mut report = ReaReport::empty(game);

    for section in &pe.sections {
        if !section.is_executable() {
            continue;
        }
        let raw = section.file_backed_size() as usize;
        if raw == 0 {
            continue;
        }
        let limit = max_bytes_per_section.min(raw);
        let raw_offset = section.raw_offset as usize;
        if raw_offset + limit > data.len() {
            continue;
        }
        let slice = &data[raw_offset..raw_offset + limit];
        let va = pe.rva_to_va(section.virtual_address);
        let insns = decode::linear_sweep(slice, va, bitness, limit);
        let instructions = insns.iter().filter(|i| i.valid).count();
        let data_skips = insns.iter().filter(|i| !i.valid).count();
        let cands = candidates::find_candidates(&insns, bitness);
        let n = cands.len();
        all_candidates.extend(cands);
        report.sections_scanned.push(ScannedSection {
            name: section.name.clone(),
            va,
            bytes: limit,
            instructions,
            data_skips,
            candidates: n,
        });
    }

    report.candidates_total = all_candidates.len();
    report.candidates = all_candidates.clone();
    for c in &all_candidates {
        *report.candidates_by_kind.entry(c.kind.as_str().to_string()).or_insert(0) += 1;
        *report.candidates_by_class.entry(c.class().to_string()).or_insert(0) += 1;
    }

    Ok((all_candidates, report))
}