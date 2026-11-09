//! Static optimization-candidate detection.
//!
//! REA does not measure anything. It scans a decoded instruction stream and
//! flags *provably behavior-preserving* simplifications that are safe to apply
//! without ever running the game:
//!
//! * **dead_code** — instructions that do nothing and can be NOP-removed.
//! * **normalization** — register idioms rewritten to their canonical form.
//!   These are flag- and result-preserving but are *not* a measured speedup;
//!   the report labels them as such.
//!
//! Every candidate is length-preserving: the replacement is exactly as many
//! bytes as the original, so no relocation or following instruction shifts.
//! This is what makes the patch safe to apply with the verified Patcher.

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
    /// `test reg, reg` — rewritten to the canonical `xor reg, reg`.
    TestSelf,
    /// `and reg, reg` — rewritten to the canonical `xor reg, reg`.
    AndSelf,
}

impl CandidateKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateKind::DeadMove => "dead_move",
            CandidateKind::DeadJump => "dead_jump",
            CandidateKind::TestSelf => "test_self",
            CandidateKind::AndSelf => "and_self",
        }
    }

    /// Whether the transformation removes work (dead code) or merely
    /// normalizes an idiom (no measured win).
    pub fn is_dead_code(&self) -> bool {
        matches!(self, CandidateKind::DeadMove | CandidateKind::DeadJump)
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
            "normalization"
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
        Mnemonic::Test | Mnemonic::And => classify_test_or_and(insn, mnemonic),
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
    if !matches!(insn.op0_kind, Some(OpKind::NearBranch64) | Some(OpKind::NearBranch32)) {
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

fn classify_test_or_and(
    insn: &DecodedInsn,
    mnemonic: Mnemonic,
) -> Option<(CandidateKind, Vec<u8>, String)> {
    let a = insn.op0_reg?;
    let b = insn.op1_reg?;
    if a != b || insn.op0_kind != Some(OpKind::Register) {
        return None;
    }
    let (kind, from) = match mnemonic {
        Mnemonic::Test => (CandidateKind::TestSelf, "test"),
        Mnemonic::And => (CandidateKind::AndSelf, "and"),
        _ => return None,
    };
    let replacement = replace_with_xor(insn)?;
    let reason = format!(
        "{from} {:?}, {:?} normalizes to xor {:?}, {:?} (identical flags and result)",
        a, b, a, b
    );
    Some((kind, replacement, reason))
}

/// `xor reg,reg` replacement for a `test reg,reg` / `and reg,reg` instruction.
fn replace_with_xor(insn: &DecodedInsn) -> Option<Vec<u8>> {
    let bytes = &insn.bytes;
    let op = opcode_offset(bytes);
    let op_byte = bytes.get(op)?;
    let xor_op = match *op_byte {
        0x85 => 0x33, // test r/m, r
        0x84 => 0x30, // test r/m8, r
        0x23 => 0x33, // and r/m, r
        0x20 => 0x30, // and r/m8, r
        _ => return None,
    };
    let mut replacement = bytes.clone();
    replacement[op] = xor_op;
    Some(replacement)
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
        (3, &[0x0F, 0xAE, 0xE8]),
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
        let c = found.iter().find(|c| c.kind == CandidateKind::DeadMove).unwrap();
        assert_eq!(c.len, 2);
        assert_eq!(c.expected, vec![0x89, 0xC0]);
        assert_eq!(c.replacement, vec![0x66, 0x90]);
    }

    #[test]
    fn detects_dead_jump_to_next() {
        // jmp rel8 with offset 0: target = ip + 2 + 0 = ip + 2 (next instruction)
        // Bytes: EB 00 = jmp rel8, disp=0
        // Next byte: 90 = nop
        let insns = scan(&[0xEB, 0x00, 0x90], 0x1000);
        for (i, insn) in insns.iter().enumerate() {
            eprintln!("insn[{}] valid={} ip=0x{:X} mnemo={:?}", i, insn.valid, insn.ip, insn.mnemonic);
        }
        let found = find_candidates(&insns);
        for c in &found {
            eprintln!("candidate kind={:?} addr=0x{:X}", c.kind, c.address);
        }
        assert!(found.iter().any(|c| c.kind == CandidateKind::DeadJump));
    }

    #[test]
    fn jump_over_next_is_not_dead() {
        // jmp short +2 skips the next instruction -> not a fallthrough dead jump
        let insns = scan(&[0xEB, 0x02, 0x90, 0x90], 0x1000);
        assert!(!find_candidates(&insns).iter().any(|c| c.kind == CandidateKind::DeadJump));
    }

    #[test]
    fn test_self_becomes_xor_same_length() {
        // test eax,eax (0x85 C0) ; ret
        let insns = scan(&[0x85, 0xC0, 0xC3], 0x1000);
        let c = find_candidates(&insns)
            .into_iter()
            .find(|c| c.kind == CandidateKind::TestSelf)
            .expect("test_self candidate");
        assert_eq!(c.replacement, vec![0x33, 0xC0]);
        assert_eq!(c.len, 2);
    }

    #[test]
    fn test_self_64bit_keeps_rex_prefix() {
        // test rax,rax (0x48 0x85 C0) ; ret
        let insns = scan(&[0x48, 0x85, 0xC0, 0xC3], 0x1000);
        let c = find_candidates(&insns)
            .into_iter()
            .find(|c| c.kind == CandidateKind::TestSelf)
            .expect("test_self candidate");
        assert_eq!(c.replacement, vec![0x48, 0x33, 0xC0]);
        assert_eq!(c.len, 3);
    }

    #[test]
    fn and_self_becomes_xor() {
        // and eax,eax (0x23 C0) ; ret
        let insns = scan(&[0x23, 0xC0, 0xC3], 0x1000);
        let c = find_candidates(&insns)
            .into_iter()
            .find(|c| c.kind == CandidateKind::AndSelf)
            .expect("and_self candidate");
        assert_eq!(c.replacement, vec![0x33, 0xC0]);
    }

    #[test]
    fn register_inequality_is_not_a_candidate() {
        // mov eax,ebx (0x89 C3) is NOT a self move
        let insns = scan(&[0x89, 0xC3, 0xC3], 0x1000);
        assert!(find_candidates(&insns).is_empty());
    }

    #[test]
    fn every_candidate_is_length_preserving() {
        // A mixed stream
        let insns = scan(&[0x89, 0xC0, 0x85, 0xC0, 0x23, 0xC0, 0xEB, 0x02, 0x90, 0xC3], 0x1000);
        for c in find_candidates(&insns) {
            assert_eq!(c.expected.len(), c.replacement.len());
        }
    }
}
