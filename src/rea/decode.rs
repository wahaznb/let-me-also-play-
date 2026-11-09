//! Linear-sweep x86/x86-64 disassembly for REA candidate detection.
//!
//! This is a *static* decoder: it reads bytes from a PE section and decodes
//! them with `iced-x86`. It never runs the game and never measures timing.
//!
//! The sweep is linear (top to bottom). When it hits a byte that does not
//! decode to a valid instruction it records a single-byte "data" skip and
//! continues, so a stray data blob cannot desync the rest of the scan. This
//! is a candidate *scanner*, not a control-flow-accurate disassembler, and the
//! report says so.

use iced_x86::{Decoder, DecoderOptions, Instruction, OpKind, Register};

/// One decoded instruction (or a single skipped data byte).
#[derive(Debug, Clone)]
pub struct DecodedInsn {
    /// Virtual address of the instruction (PE image base + RVA).
    pub ip: u64,
    /// Offset of the first byte within the scanned region.
    pub offset: usize,
    /// Byte length (1 for a data skip).
    pub len: usize,
    /// True when `iced-x86` decoded a valid instruction.
    pub valid: bool,
    /// Raw bytes of the instruction (or the single skipped byte).
    pub bytes: Vec<u8>,
    /// Decoded mnemonic, when valid.
    pub mnemonic: Option<iced_x86::Mnemonic>,
    /// Decoded code, when valid.
    pub code: Option<iced_x86::Code>,
    /// First operand register, when the first operand is a register.
    pub op0_reg: Option<Register>,
    /// Second operand register, when the second operand is a register.
    pub op1_reg: Option<Register>,
    /// First operand kind, when valid.
    pub op0_kind: Option<OpKind>,
    /// Second operand kind, when valid.
    pub op1_kind: Option<OpKind>,
}

impl DecodedInsn {
    fn register_of(instr: &Instruction, slot: u32) -> Option<Register> {
        let kind = match slot {
            0 => instr.op0_kind(),
            1 => instr.op1_kind(),
            _ => return None,
        };
        if kind != OpKind::Register {
            return None;
        }
        let reg = match slot {
            0 => instr.op0_register(),
            1 => instr.op1_register(),
            _ => return None,
        };
        (reg != Register::None).then_some(reg)
    }
}

/// Decode `bytes` as a linear sweep, starting at virtual address `base_ip`.
///
/// `bitness` must match the PE machine (32 or 64). Decoding stops at
/// `max_bytes` so a huge section cannot be scanned unbounded.
pub fn linear_sweep(bytes: &[u8], base_ip: u64, bitness: u32, max_bytes: usize) -> Vec<DecodedInsn> {
    let limit = max_bytes.min(bytes.len());
    let mut out = Vec::new();
    if limit == 0 {
        return out;
    }

    let mut decoder = match Decoder::try_with_ip(bitness, bytes, base_ip, DecoderOptions::NONE) {
        Ok(d) => d,
        Err(_) => return out,
    };

    let mut pos: usize = 0;
    while pos < limit {
        if decoder.set_position(pos).is_err() {
            break;
        }
        decoder.set_ip(base_ip + pos as u64);

        let mut instr = Instruction::default();
        decoder.decode_out(&mut instr);

        if instr.is_invalid() || instr.len() == 0 {
            // Data byte: skip one byte and keep sweeping.
            out.push(DecodedInsn {
                ip: base_ip + pos as u64,
                offset: pos,
                len: 1,
                valid: false,
                bytes: vec![bytes[pos]],
                mnemonic: None,
                code: None,
                op0_reg: None,
                op1_reg: None,
                op0_kind: None,
                op1_kind: None,
            });
            pos += 1;
            continue;
        }

        let len = instr.len();
        if pos + len > limit {
            // Truncated at the scan boundary: treat the remainder as data.
            out.push(DecodedInsn {
                ip: base_ip + pos as u64,
                offset: pos,
                len: limit - pos,
                valid: false,
                bytes: bytes[pos..limit].to_vec(),
                mnemonic: None,
                code: None,
                op0_reg: None,
                op1_reg: None,
                op0_kind: None,
                op1_kind: None,
            });
            break;
        }

        out.push(DecodedInsn {
            ip: base_ip + pos as u64,
            offset: pos,
            len,
            valid: true,
            bytes: bytes[pos..pos + len].to_vec(),
            mnemonic: Some(instr.mnemonic()),
            code: Some(instr.code()),
            op0_reg: register_of(&instr, 0),
            op1_reg: register_of(&instr, 1),
            op0_kind: Some(instr.op0_kind()),
            op1_kind: Some(instr.op1_kind()),
        });
        pos += len;
    }

    out
}

fn register_of(instr: &Instruction, slot: u32) -> Option<Register> {
    DecodedInsn::register_of(instr, slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_simple_sequence() {
        // xor eax,eax / mov eax,ebx / nop / ret
        let bytes: &[u8] = &[0x33, 0xC0, 0x89, 0xD8, 0x90, 0xC3];
        let insns = linear_sweep(bytes, 0x1000, 64, bytes.len());
        assert_eq!(insns.len(), 4);
        assert!(insns.iter().all(|i| i.valid));
        assert_eq!(insns[0].mnemonic, Some(iced_x86::Mnemonic::Xor));
        assert_eq!(insns[1].mnemonic, Some(iced_x86::Mnemonic::Mov));
        assert_eq!(insns[2].mnemonic, Some(iced_x86::Mnemonic::Nop));
        assert_eq!(insns[3].mnemonic, Some(iced_x86::Mnemonic::Ret));
    }

    #[test]
    fn skips_invalid_bytes_and_continues() {
        // A trailing lone 0x00 is not a complete instruction. The sweep must
        // record it as a data skip instead of panicking or stopping early.
        let bytes: &[u8] = &[0xC3, 0x00];
        let insns = linear_sweep(bytes, 0x1000, 64, bytes.len());
        assert!(insns
            .iter()
            .any(|i| i.mnemonic == Some(iced_x86::Mnemonic::Ret)));
        assert!(
            insns.iter().any(|i| !i.valid),
            "expected at least one invalid skip"
        );
    }

    #[test]
    fn respects_max_bytes() {
        let bytes: &[u8] = &[0x90; 64];
        let insns = linear_sweep(bytes, 0x1000, 64, 8);
        assert!(insns.iter().all(|i| i.offset < 8));
    }
}
