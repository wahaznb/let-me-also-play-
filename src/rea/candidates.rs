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
        match classify(insn, insns.get(i + 1).map(|n| n.ip)) {
            Some((kind, replacement, reason)) => {
                let len = insn.len;
                if replacement.len() != len {
                    // Safety invariant: never emit a length-changing patch.
                    continue;
                }
                out.push(Candidate {
                    kind,
                    address: insn.ip,
                    offset: insn.offset,
                    len,
                    expected: insn.bytes.clone(),
                    replacement,
                    reason,
                });
            }
            None => {}
        }
    }
    out
}

/// Classify a single instruction. Returns `(kind, replacement_bytes, reason)`.
///
/// `next_ip` is the virtual address of the following instruction, used to
/// detect a `jmp` that simply falls through to the next byte.
fn classify(insn: &DecodedInsn, next_ip: Option<u64>) -> Option<(CandidateKind, Vec<u8>, String)> {
    let mnemonic = *insn.mnemonic?;

    match mnemonic {
        Mnemonic::Mov => {
            let (a, b) = (insn.op0_reg?, insn.op1_reg?);
            if a != b || insn.op0_kind != Some(OpKind::Register) {
                return None;
            }
            let nops = "nop".to_string();
            let replacement = nops_n(insn.len);
            let reason = format!(
                "mov {:?}, {:?} is a no-op move; replaced with {} nop byte(s)",
                a,
                b,
                insn.len
            );
            Some((CandidateKind::DeadMove, replacement, reason))
        }
        Mnemonic::Jmp => {
            // Only a relative near jump with a single memory/immediate target
            // whose destination is the next instruction.
            if insn.op0_kind != Some(OpKind::Memory) {
                // `jmp rel` encodes as a memory-style near target in iced-x86.
                return None;
            }
            let target = near_target(insn)?;
            let next = next_ip?;
            if target != next {
                return None;
            }
            let replacement = nops_n(insn.len);
            let reason = format!(
                "jmp to the immediately following instruction (0x{next:X}); removed {} byte(s)",
                insn.len
            );
            Some((CandidateKind::DeadJump, replacement, reason))
        }
        Mnemonic::Test | Mnemonic::And => {
            let (a, b) = (*insn.op0_reg?, *insn.op1_reg?);
            if a != b || insn.op0_kind != Some(OpKind::Register) {
                return None;
            }
            let (kind, from) = match mnemonic {
                Mnemonic::Test => (CandidateKind::TestSelf, "test"),
                Mnemonic::And => (CandidateKind::AndSelf, "and"),
                _ => unreachable!(),
            };
            let replacement = replace_with_xor(insn).unwrap_or_default();
            if replacement.len() != insn.len {
                return None;
            }
            let reason = format!(
                "{from} {:?}, {:?} normalizes to xor {:?}, {:?} (identical flags and result)",
                a, b, a, b
            );
            Some((kind, replacement, reason))
        }
        _ => None,
    }
}

/// `xor reg,reg` replacement for a `test reg,reg` / `and reg,reg` instruction.
///
/// The register operand (ModRM byte) is identical between test/and and xor, so
/// the only byte that changes is the opcode. We locate the opcode (after any
/// REX and 0x66/0x67 prefixes) and swap it, keeping the length identical.
fn replace_with_xor(insn: &DecodedInsn) -> Option<Vec<u8>> {
    let bytes = &insn.bytes;
    let op = opcode_offset(bytes);
    let op_byte = bytes.get(op)?;
    let xor_op = match *op_byte {
        // test r/m16/32/64, r  -> xor r/m16/32/64, r
        0x85 => 0x33,
        // test r/m8, r         -> xor r/m8, r
        0x84 => 0x30,
        // and  r/m16/32/64, r  -> xor r/m16/32/64, r
        0x23 => 0x33,
        // and  r/m8, r         -> xor r/m8, r
        0x20 => 0x30,
        _ => return None,
    };
    let mut replacement = bytes.clone();
    replacement[op] = xor_op;
    Some(replacement)
}

/// Index of the opcode byte, skipping REX (64-bit) and operand/address-size
/// prefixes. `test`/`and`/`xor reg,reg` never carry segment or lock prefixes.
fn opcode_offset(bytes: &[u8]) -> usize {
    let mut i = 0;
    // REX prefix (64-bit only): 0x40..=0x4F
    if i < bytes.len() && (0x40..=0x4F).contains(&bytes[i]) {
        i += 1;
    }
    // Operand-size / address-size prefixes.
    while i < bytes.len() && matches!(bytes[i], 0x66 | 0x67) {
        i += 1;
    }
    i
}

/// `n` bytes that assemble to NOPs, using the standard multi-byte NOPs so the
/// result stays branchless and prefix-free.
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
/// The target is `ip + len + rel`, read straight from the raw bytes so the
/// result is independent of how iced-x86 reports the operand.
fn near_target(insn: &DecodedInsn) -> Option<u64> {
    let base = insn.ip.checked_add(insn.len as u64)?;
    let opcode = opcode_offset(&insn.bytes);
    let op = insn.bytes.get(opcode)?;
    match op {
        0xEB => {
            let rel = *insn.bytes.get(opcode + 1)? as i8;
            base.wrapping_add_signed(rel as i64)
        }
        0xE9 => {
            let rel = i32::from_le_bytes([
                *insn.bytes.get(opcode + 1)?,
                *insn.bytes.get(opcode + 2)?,
                *insn.bytes.get(opcode + 3)?,
                *insn.bytes.get(opcode + 4)?,
            ]);
            base.wrapping_add_signed(rel as i64)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rea::decode::linear_sweep;

    fn cand(bytes: &[u8], va: u64) -> Vec<DecodedInsn> {
        linear_sweep(bytes, va, 64, bytes.len())
    }

    #[test]
    fn detects_dead_move() {
        // mov eax,eax ; ret
        let insns = cand(&[0x89, 0xC0, 0xC3], 0x1000);
        let found = find_candidates(&insns);
        assert!(found.iter().any(|c| c.kind == CandidateKind::DeadMove));
        let c = found.iter().find(|c| c.kind == CandidateKind::DeadMove).unwrap();
        assert_eq!(c.len, 2);
        assert_eq!(c.expected, vec![0x89, 0xC0]);
        assert_eq!(c.replacement, vec![0x66, 0x90]);
    }

    #[test]
    fn detects_dead_jump_to_next() {
        // jmp short +0 (target == the following byte) ; nop
        let insns = cand(&[0xEB, 0x00, 0x90], 0x1000);
        let found = find_candidates(&insns);
        assert!(found.iter().any(|c| c.kind == CandidateKind::DeadJump));
    }

    #[test]
    fn jump_over_next_is_not_dead() {
        // jmp short +2 skips the next instruction -> not a fallthrough dead jump
        let insns = cand(&[0xEB, 0x02, 0x90, 0x90], 0x1000);
        assert!(!find_candidates(&insns).iter().any(|c| c.kind == CandidateKind::DeadJump));
    }

    #[test]
    fn test_self_becomes_xor_same_length() {
        // test eax,eax (0x85 C0) ; ret
        let insns = cand(&[0x85, 0xC0, 0xC3], 0x1000);
        let c = find_candidates(&insns)
            .into_iter()
            .find(|c| c.kind == CandidateKind::TestSelf)
            .expect("test_self candidate");
        assert_eq!(c.replacement, vec![0x33, 0xC0]);
        assert_eq!(c.len, 2);
    }

    #[test]
    fn test_self_64bit_kepex_prefix() {
        // test rax,rax (0x48 0x85 C0) ; ret
        let insns = cand(&[0x48, 0x85, 0xC0, 0xC3], 0x1000);
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
        let insns = cand(&[0x23, 0xC0, 0xC3], 0x1000);
        let c = find_candidates(&insns)
            .into_iter()
            .find(|c| c.kind == CandidateKind::AndSelf)
            .expect("and_self candidate");
        assert_eq!(c.replacement, vec![0x33, 0xC0]);
    }

    #[test]
    fn register_inequality_is_not_a_candidate() {
        // mov eax,ebx (0x89 C3) is NOT a self move
        let insns = cand(&[0x89, 0xC3, 0xC3], 0x1000);
        assert!(find_candidates(&insns).is_empty());
    }

    #[test]
    fn every_candidate_is_length_preserving() {
        // A mixed stream
        let insns = cand(&[0x89, 0xC0, 0x85, 0xC0, 0x23, 0xC0, 0xEB, 0x02, 0x90, 0xC3], 0x1000);
        for c in find_candidates(&insns) {
            assert_eq!(c.expected.len(), c.replacement.len());
        }
    }
}
