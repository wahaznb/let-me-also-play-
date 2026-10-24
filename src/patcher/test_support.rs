//! Synthetic PE fixtures used by patcher unit and CLI tests.

pub const PE32_ENTRY_RVA: u32 = 0x1000;
pub const PE32_ENTRY_VA: u64 = 0x0040_0000 + PE32_ENTRY_RVA as u64;
pub const PE32_TEXT_OFFSET: usize = 0x400;
pub const PE32_TEXT: [u8; 8] = [0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x10, 0x90, 0xC3];

/// A minimal valid PE32 image with one executable `.text` section.
pub fn pe32() -> Vec<u8> {
    make_pe(false)
}

/// A minimal valid PE32+ image with one executable `.text` section.
pub fn pe64() -> Vec<u8> {
    make_pe(true)
}

fn make_pe(is_64: bool) -> Vec<u8> {
    const HEADER_SIZE: usize = 0x400;
    const FILE_SIZE: usize = 0x600;
    let section_table: usize = if is_64 { 0x188 } else { 0x178 };
    const OPTIONAL_HEADER: usize = 0x98;
    let optional_size: usize = if is_64 { 0xF0 } else { 0xE0 };

    let mut data = vec![0u8; FILE_SIZE];
    data[0] = b'M';
    data[1] = b'Z';
    data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    data[0x80..0x84].copy_from_slice(b"PE\0\0");

    let coff = 0x84;
    let machine = if is_64 { 0x8664u16 } else { 0x014Cu16 };
    data[coff..coff + 2].copy_from_slice(&machine.to_le_bytes());
    data[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    data[coff + 16..coff + 18]
        .copy_from_slice(&(optional_size as u16).to_le_bytes());
    let characteristics = if is_64 { 0x0022u16 } else { 0x0102u16 };
    data[coff + 18..coff + 20].copy_from_slice(&characteristics.to_le_bytes());

    let optional = OPTIONAL_HEADER;
    let magic = if is_64 { 0x20Bu16 } else { 0x10Bu16 };
    data[optional..optional + 2].copy_from_slice(&magic.to_le_bytes());
    data[optional + 4..optional + 8].copy_from_slice(&0x200u32.to_le_bytes());
    data[optional + 16..optional + 20].copy_from_slice(&PE32_ENTRY_RVA.to_le_bytes());
    data[optional + 20..optional + 24].copy_from_slice(&0x1000u32.to_le_bytes());
    if is_64 {
        data[optional + 24..optional + 32]
            .copy_from_slice(&0x0000_0001_4000_0000u64.to_le_bytes());
    } else {
        data[optional + 28..optional + 32].copy_from_slice(&0x0040_0000u32.to_le_bytes());
    }
    data[optional + 32..optional + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 36..optional + 40].copy_from_slice(&0x200u32.to_le_bytes());
    data[optional + 56..optional + 60].copy_from_slice(&0x2000u32.to_le_bytes());
    data[optional + 60..optional + 64].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
    data[optional + 68..optional + 70].copy_from_slice(&3u16.to_le_bytes());
    let dirs_offset = optional + if is_64 { 108 } else { 92 };
    data[dirs_offset..dirs_offset + 4].copy_from_slice(&16u32.to_le_bytes());

    let section = section_table;
    data[section..section + 5].copy_from_slice(b".text");
    data[section + 8..section + 12].copy_from_slice(&0x100u32.to_le_bytes());
    data[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    data[section + 16..section + 20].copy_from_slice(&0x200u32.to_le_bytes());
    data[section + 20..section + 24].copy_from_slice(&0x400u32.to_le_bytes());
    data[section + 36..section + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());

    data[PE32_TEXT_OFFSET..PE32_TEXT_OFFSET + PE32_TEXT.len()].copy_from_slice(&PE32_TEXT);
    data
}
