//! Build a [`PatchSpec`] from REA candidates.
//!
//! Each candidate becomes one `BytePatchSpec`. The `expected` field is the
//! current byte sequence (a version guard) and `replace` is the same-length
//! replacement. The resulting spec is consumed by the verified Patcher, which
//! independently re-checks every address, section, and byte before writing.

use crate::patcher::{BytePatchSpec, PatchSpec, PATCH_SCHEMA_VERSION};
use anyhow::Context;
use super::candidates::Candidate;

/// Convert candidates into a patch spec. Returns `Ok(None)` when there are no
/// candidates so the caller can short-circuit without writing a binary.
pub fn build_patch_spec(
    name: Option<String>,
    target_sha256: Option<String>,
    allow_signed: bool,
    candidates: &[Candidate],
) -> anyhow::Result<Option<PatchSpec>> {
    if candidates.is_empty() {
        return Ok(None);
    }

    let mut seen = std::collections::HashSet::new();
    let mut patches = Vec::with_capacity(candidates.len());
    for cand in candidates {
        if !seen.insert(cand.address) {
            anyhow::bail!(
                "Duplicate candidate at 0x{:X}; refusing to emit two patches for one address",
                cand.address
            );
        }
        patches.push(BytePatchSpec {
            address: format!("0x{:X}", cand.address),
            expected: to_hex(&cand.expected),
            replace: Some(to_hex(&cand.replacement)),
            jump: None,
            note: Some(format!("[rea:{} {}]", cand.kind.as_str(), cand.reason)),
        });
    }

    let spec = PatchSpec {
        schema_version: PATCH_SCHEMA_VERSION,
        name: name.or_else(|| Some("rea".to_string())),
        target_sha256,
        allow_signed,
        patches,
        sections: Vec::new(),
    };
    spec.validate().context("Generated REA patch spec failed validation")?;
    Ok(Some(spec))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rea::candidates::CandidateKind;

    fn cand(addr: u64, expected: Vec<u8>, replacement: Vec<u8>) -> Candidate {
        Candidate {
            kind: CandidateKind::DeadMove,
            address: addr,
            offset: 0,
            len: expected.len(),
            expected,
            replacement,
            reason: "test".to_string(),
        }
    }

    #[test]
    fn empty_candidates_yield_none() {
        let spec = build_patch_spec(None, None, false, &[]).unwrap();
        assert!(spec.is_none());
    }

    #[test]
    fn builds_a_valid_spec() {
        let cands = vec![cand(0x1000, vec![0x89, 0xC0], vec![0x66, 0x90])];
        let spec = build_patch_spec(None, None, false, &cands).unwrap().unwrap();
        assert_eq!(spec.patches.len(), 1);
        assert_eq!(spec.patches[0].address, "0x1000");
        assert_eq!(spec.patches[0].expected, "89c0");
        assert_eq!(spec.patches[0].replace.as_deref(), Some("6690"));
    }

    #[test]
    fn duplicate_addresses_are_rejected() {
        let cands = vec![
            cand(0x1000, vec![0x89, 0xC0], vec![0x66, 0x90]),
            cand(0x1000, vec![0x85, 0xC0], vec![0x33, 0xC0]),
        ];
        assert!(build_patch_spec(None, None, false, &cands).is_err());
    }
}
