//! Static optimization-candidate detection.
//!
//! REA does not measure anything. It scans a decoded instruction stream and
//! flags *provably behavior-preserving* simplifications that are safe to apply
//! without ever running the game:
//!
//! * **dead_code** — instructions that do nothing and can be NOP-removed.
//!
//! Every candidate is length-preserving: the replacement is exactly as many
//! bytes as the original, so no relocation or following instruction shifts.
//! This is what makes the patch safe to apply with the verified Patcher.
//!
//! The scan is linear, not control-flow-aware. A candidate found inside
//! embedded data is a false positive; the report says so, and the user must
//! review addresses before applying to a shipping binary.

use super::decode::DecodedInsn;
use iced_x86::{Mnemonic, OpKind};

/// Why a candidate was flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    /// `mov reg, reg` — a no-op move.
    DeadMove,
    /// `jmp rel` whose target is the immediately following instruction.
    DeadJump,
}

impl CandidateKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateKind::DeadMove => "dead_move",
            CandidateKind::DeadJump => "dead_jump",
        }
    }

    /// Every candidate in the current catalog is dead code. The method is kept
    /// so a future non-dead-code class cannot silently skip the report label.
    pub fn is_dead_code(&self) -> bool {
        true
    }
}

/// One safe, length-preserving patch candidate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub kind: CandidateKind,
    /// Virtual address.
    pub address: u64,
    /// Offset of the first byte within the scanned region.
    pub offset: usize,
    /// Number of bytes (original and replacement are the same length).
    pub len: usize,
    /// Bytes currently at `address` (the version guard).
    pub expected: Vec<u8>,
    /// Replacement bytes (same length as `expected`).
    pub replacement: Vec<u8>,
    /// Human-readable rationale.
    pub reason: String,
}

impl Candidate {
    /// The section of the report that describes the class of change.
    pub fn class(&self) -> &'static str {
        if self.kind.is_dead_code() {
            "dead_code"
        } else {
            "other"
        }
    }
}

/// Find all safe candidates in a decoded instruction stream.
pub fn find_candidates(insns: &[DecodedInsn]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (i, insn) in insns.iter().enumerate() {
        if !insn.valid {
            continue;
        }
        if let Some((kind, replacement, reason)) =
            classify(insn, insns.get(i + 1).map(|n| n.ip))
        {
            if replacement.len() == insn.len {
                out.push(Candidate {
                    kind,
                    address: insn.ip,
                    offset: insn.offset,
                    len: insn.len,
                    expected: insn.bytes.clone(),
                    replacement,
                    reason,
                });
            }
        }
    }
    out
}

/// Classify a single instruction. Returns `(kind, replacement_bytes, reason)`.
fn classify(insn: &DecodedInsn, next_ip: Option<u64>) -> Option<(CandidateKind, Vec<u8>, String)> {
    let mnemonic = insn.mnemonic?;
    match mnemonic {
        Mnemonic::Mov => classify_mov(insn),
        Mnemonic::Jmp => classify_jmp(insn, next_ip?),
        _ => None,
    }
}

fn classify_mov(insn: &DecodedInsn) -> Option<(CandidateKind, Vec<u8>, String)> {
    let a = insn.op0_reg?;
    let b = insn.op1_reg?;
    if a != b || insn.op0_kind != Some(OpKind::Register) {
        return None;
    }
    let replacement = nops_n(insn.len);
    let reason = format!(
        "mov {:?}, {:?} is a no-op move; replaced with {} nop byte(s)",
        a, b, insn.len
    );
    Some((CandidateKind::DeadMove, replacement, reason))
}

fn classify_jmp(insn: &DecodedInsn, next_ip: u64) -> Option<(CandidateKind, Vec<u8>, String)> {
    // In iced-x86, jmp rel8/rel32 reports as NearBranch64/NearBranch32.
    if !matches!(
        insn.op0_kind,
        Some(OpKind::NearBranch64) | Some(OpKind::NearBranch32)
    ) {
        return None;
    }
    let target = near_target(insn)?;
    if target != next_ip {
        return None;
    }
    let replacement = nops_n(insn.len);
    let reason = format!(
        "jmp to the immediately following instruction (0x{next_ip:X}); removed {} byte(s)",
        insn.len
    );
    Some((CandidateKind::DeadJump, replacement, reason))
}

/// Index of the opcode byte, skipping REX and operand/address-size prefixes.
fn opcode_offset(bytes: &[u8]) -> usize {
    let mut i = 0;
    if i < bytes.len() && (0x40..=0x4F).contains(&bytes[i]) {
        i += 1;
    }
    while i < bytes.len() && matches!(bytes[i], 0x66 | 0x67) {
        i += 1;
    }
    i
}

/// `n` bytes that assemble to NOPs, using standard multi-byte NOPs.
fn nops_n(n: usize) -> Vec<u8> {
    let multi: &[(usize, &[u8])] = &[
        (5, &[0x0F, 0x1F, 0x44, 0x00, 0x00]),
        (4, &[0x0F, 0x1F, 0x40, 0x00]),
        (3, &[0x0F, 0x1F, 0x00]),
        (2, &[0x66, 0x90]),
    ];
    let mut out = Vec::with_capacity(n);
    let mut remaining = n;
    for (size, bytes) in multi {
        while remaining >= *size {
            out.extend_from_slice(bytes);
            remaining -= size;
        }
    }
    while remaining > 0 {
        out.push(0x90);
        remaining -= 1;
    }
    out
}

/// Resolve the target of a near relative jump (`jmp rel8`/`jmp rel32`).
fn near_target(insn: &DecodedInsn) -> Option<u64> {
    let base = insn.ip.checked_add(insn.len as u64)?;
    let opcode = opcode_offset(&insn.bytes);
    let op = insn.bytes.get(opcode)?;
    match op {
        0xEB => {
            let rel = *insn.bytes.get(opcode + 1)? as i8;
            Some(base.wrapping_add(rel as i64 as u64))
        }
        0xE9 => {
            let rel = i32::from_le_bytes([
                *insn.bytes.get(opcode + 1)?,
                *insn.bytes.get(opcode + 2)?,
                *insn.bytes.get(opcode + 3)?,
                *insn.bytes.get(opcode + 4)?,
            ]);
            Some(base.wrapping_add(rel as i64 as u64))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rea::decode::linear_sweep;

    fn scan(bytes: &[u8], va: u64) -> Vec<DecodedInsn> {
        linear_sweep(bytes, va, 64, bytes.len())
    }

    #[test]
    fn detects_dead_move() {
        // mov eax,eax ; ret
        let insns = scan(&[0x89, 0xC0, 0xC3], 0x1000);
        let found = find_candidates(&insns);
        assert!(found.iter().any(|c| c.kind == CandidateKind::DeadMove));
        let c = found
            .iter()
            .find(|c| c.kind == CandidateKind::DeadMove)
            .unwrap();
        assert_eq!(c.len, 2);
        assert_eq!(c.expected, vec![0x89, 0xC0]);
        assert_eq!(c.replacement, vec![0x66, 0x90]);
    }

    #[test]
    fn detects_dead_jump_to_next() {
        // jmp rel8 with offset 0: target = ip + 2 + 0 = ip + 2 (next instruction)
        let insns = scan(&[0xEB, 0x00, 0x90], 0x1000);
        let found = find_candidates(&insns);
        assert!(found.iter().any(|c| c.kind == CandidateKind::DeadJump));
    }

    #[test]
    fn jump_over_next_is_not_dead() {
        // jmp short +2 skips the next instruction -> not a fallthrough dead jump
        let insns = scan(&[0xEB, 0x02, 0x90, 0x90], 0x1000);
        assert!(!find_candidates(&insns)
            .iter()
            .any(|c| c.kind == CandidateKind::DeadJump));
    }

    #[test]
    fn register_inequality_is_not_a_candidate() {
        // mov eax,ebx (0x89 C3) is NOT a self move
        let insns = scan(&[0x89, 0xC3, 0xC3], 0x1000);
        assert!(find_candidates(&insns).is_empty());
    }

    #[test]
    fn test_and_xor_are_not_rewritten() {
        // test eax,eax / and eax,eax must not be flagged: replacing either with
        // xor would zero the register and change its flags.
        let insns = scan(&[0x85, 0xC0, 0x23, 0xC0, 0xC3], 0x1000);
        assert!(find_candidates(&insns).is_empty());
    }

    #[test]
    fn nops_are_real_nops_at_every_length() {
        // 0F AE E8 is lfence, not a NOP; the 3-byte form must be 0F 1F 00.
        assert_eq!(nops_n(3), vec![0x0F, 0x1F, 0x00]);
        assert_eq!(nops_n(1), vec![0x90]);
        assert_eq!(nops_n(2), vec![0x66, 0x90]);
        for n in 3..=8 {
            let nops = nops_n(n);
            assert_eq!(nops.len(), n, "NOP sequence must preserve length");
            assert_eq!(nops[0], 0x0F, "multi-byte NOP opcode");
        }
    }

    #[test]
    fn every_candidate_is_length_preserving() {
        // A mixed stream: mov eax,eax ; jmp +0 ; nop ; ret
        let insns = scan(&[0x89, 0xC0, 0xEB, 0x00, 0x90, 0xC3], 0x1000);
        let found = find_candidates(&insns);
        assert!(!found.is_empty());
        for c in found {
            assert_eq!(c.expected.len(), c.replacement.len());
        }
    }
}
