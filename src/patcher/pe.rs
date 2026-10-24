//! Minimal, strict PE32/PE32+ parsing and patching primitives.
//!
//! This parser is intentionally narrow: it validates the headers and section
//! table needed to map virtual addresses to file offsets, then refuses anything
//! ambiguous. It is not a general-purpose PE loader.
//!
//! All offsets are checked before use. A section is only patchable when the
//! target range is backed by file data (`min(VirtualSize, SizeOfRawData)`), so a
//! patch can never silently write into a zero-filled virtual tail.

use anyhow::{bail, Context, Result};

const DOS_HEADER_SIZE: usize = 0x40;
const PE_SIGNATURE: &[u8; 4] = b"PE\0\0";
const COFF_HEADER_SIZE: usize = 20;
const SECTION_HEADER_SIZE: usize = 40;
const MAX_SECTIONS: u16 = 96;

const IMAGE_SCN_CNT_CODE: u32 = 0x0000_0020;
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;
const NEW_SECTION_CHARACTERISTICS: u32 =
    IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ;

const PE32_MAGIC: u16 = 0x10B;
const PE32_PLUS_MAGIC: u16 = 0x20B;

/// A PE section entry plus the offset of its 40-byte header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
    pub header_offset: usize,
}

impl Section {
    pub fn is_executable(&self) -> bool {
        self.characteristics & IMAGE_SCN_MEM_EXECUTE != 0
    }

    /// Number of bytes backed by both the mapped virtual range and the file.
    pub fn file_backed_size(&self) -> u32 {
        self.virtual_size.min(self.raw_size)
    }
}

/// Parsed PE header fields required for safe patching.
#[derive(Debug, Clone)]
pub struct PeInfo {
    pub is_64: bool,
    pub machine: u16,
    pub image_base: u64,
    pub entry_point_rva: u32,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub size_of_code: u32,
    pub file_alignment: u32,
    pub section_alignment: u32,
    pub checksum: u32,
    pub number_of_sections: u16,
    pub file_len: usize,
    pub pe_offset: usize,
    pub optional_header_offset: usize,
    pub optional_header_size: u16,
    pub section_table_offset: usize,
    pub checksum_offset: usize,
    pub size_of_code_offset: usize,
    pub size_of_image_offset: usize,
    pub number_of_sections_offset: usize,
    pub certificate_directory: (u32, u32),
    pub sections: Vec<Section>,
}

/// A fully validated plan for appending one executable section.
#[derive(Debug, Clone)]
pub struct NewSectionPlan {
    pub name: String,
    pub original_file_len: usize,
    pub section_header_offset: usize,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub size_of_image: u32,
    pub size_of_code: u32,
    pub characteristics: u32,
}

impl PeInfo {
    /// Parse and validate the PE headers and section table.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < DOS_HEADER_SIZE {
            bail!("PE is truncated before the DOS header (need at least 64 bytes)");
        }
        if &data[0..2] != b"MZ" {
            bail!("Missing DOS MZ signature");
        }
        let pe_offset = read_u32(data, 0x3C)? as usize;
        if pe_offset < DOS_HEADER_SIZE {
            bail!("e_lfanew points inside the DOS header (0x{pe_offset:X})");
        }
        let signature_end = pe_offset
            .checked_add(PE_SIGNATURE.len())
            .context("PE header offset overflows")?;
        if signature_end > data.len() {
            bail!("e_lfanew points past the end of the file");
        }
        if &data[pe_offset..signature_end] != PE_SIGNATURE {
            bail!("Missing PE signature at 0x{pe_offset:X}");
        }

        let coff_offset = signature_end;
        require(data, coff_offset, COFF_HEADER_SIZE)?;
        let machine = read_u16(data, coff_offset)?;
        let number_of_sections = read_u16(data, coff_offset + 2)?;
        let optional_header_size = read_u16(data, coff_offset + 16)?;
        if number_of_sections == 0 || number_of_sections > MAX_SECTIONS {
            bail!(
                "Invalid section count {number_of_sections} (expected 1..={MAX_SECTIONS})"
            );
        }

        let optional_header_offset = coff_offset
            .checked_add(COFF_HEADER_SIZE)
            .context("Optional header offset overflows")?;
        let optional_header_end = optional_header_offset
            .checked_add(optional_header_size as usize)
            .context("Optional header size overflows")?;
        if optional_header_end > data.len() {
            bail!("Optional header extends past the end of the file");
        }
        let magic = read_u16(data, optional_header_offset)?;
        let is_64 = match magic {
            PE32_MAGIC => false,
            PE32_PLUS_MAGIC => true,
            _ => bail!("Unsupported PE optional header magic 0x{magic:04X}"),
        };
        // Every optional-header field used below must be inside the declared
        // optional header. This also keeps checksum_offset and the section
        // table from aliasing each other in a malformed image.
        let minimum_optional_header_size = if is_64 { 112 } else { 96 };
        if optional_header_size < minimum_optional_header_size {
            bail!(
                "Optional header is too small ({optional_header_size} bytes; need at least {minimum_optional_header_size})"
            );
        }

        let checksum_offset = optional_header_offset
            .checked_add(64)
            .context("Checksum offset overflows")?;
        require(data, checksum_offset, 4)?;
        let checksum = read_u32(data, checksum_offset)?;

        let image_base = if is_64 {
            require(data, optional_header_offset + 24, 8)?;
            read_u64(data, optional_header_offset + 24)?
        } else {
            require(data, optional_header_offset + 28, 4)?;
            u64::from(read_u32(data, optional_header_offset + 28)?)
        };
        let entry_point_rva = read_u32(data, optional_header_offset + 16)?;
        let section_alignment = read_u32(data, optional_header_offset + 32)?;
        let file_alignment = read_u32(data, optional_header_offset + 36)?;
        let size_of_image = read_u32(data, optional_header_offset + 56)?;
        let size_of_headers = read_u32(data, optional_header_offset + 60)?;
        let size_of_code = read_u32(data, optional_header_offset + 4)?;

        if section_alignment == 0 || !section_alignment.is_power_of_two() {
            bail!("SectionAlignment is not a nonzero power of two");
        }
        if file_alignment == 0 || !file_alignment.is_power_of_two() {
            bail!("FileAlignment is not a nonzero power of two");
        }
        if file_alignment > section_alignment {
            bail!(
                "FileAlignment 0x{file_alignment:X} is larger than SectionAlignment 0x{section_alignment:X}"
            );
        }
        if size_of_image == 0 {
            bail!("SizeOfImage is zero");
        }
        if size_of_headers == 0 {
            bail!("SizeOfHeaders is zero");
        }
        if size_of_headers as usize > data.len() {
            bail!("SizeOfHeaders extends past the end of the file");
        }

        let number_of_rva_and_sizes = if is_64 {
            read_u32(data, optional_header_offset + 108)?
        } else {
            read_u32(data, optional_header_offset + 92)?
        };
        let data_directory_offset = optional_header_offset
            + if is_64 { 112 } else { 96 };
        if data_directory_offset > optional_header_end {
            bail!("Optional header is too small for the data-directory count");
        }
        let available_dirs = (optional_header_end - data_directory_offset) / 8;
        let directory_count = (number_of_rva_and_sizes as usize)
            .min(available_dirs)
            .min(16);
        let certificate_directory = if directory_count > 4 {
            let offset = data_directory_offset + 4 * 8;
            (read_u32(data, offset)?, read_u32(data, offset + 4)?)
        } else {
            (0, 0)
        };

        let section_table_offset = optional_header_end;
        let section_table_end = section_table_offset
            .checked_add(number_of_sections as usize * SECTION_HEADER_SIZE)
            .context("Section table size overflows")?;
        if section_table_end > data.len() {
            bail!("Section table extends past the end of the file");
        }
        if section_table_end > size_of_headers as usize {
            bail!("Section table does not fit inside SizeOfHeaders");
        }

        let number_of_sections_offset = coff_offset + 2;
        let size_of_code_offset = optional_header_offset + 4;
        let size_of_image_offset = optional_header_offset + 56;

        let mut sections = Vec::with_capacity(number_of_sections as usize);
        for index in 0..number_of_sections as usize {
            let header_offset = section_table_offset + index * SECTION_HEADER_SIZE;
            let name_bytes = &data[header_offset..header_offset + 8];
            let name_end = name_bytes.iter().position(|byte| *byte == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&name_bytes[..name_end])
                .trim()
                .to_string();
            let virtual_size = read_u32(data, header_offset + 8)?;
            let virtual_address = read_u32(data, header_offset + 12)?;
            let raw_size = read_u32(data, header_offset + 16)?;
            let raw_offset = read_u32(data, header_offset + 20)?;
            let characteristics = read_u32(data, header_offset + 36)?;

            let raw_end = u64::from(raw_offset) + u64::from(raw_size);
            if raw_end > data.len() as u64 {
                bail!(
                    "Section {name:?} raw range 0x{raw_offset:X}..0x{raw_end:X} exceeds the file"
                );
            }
            if raw_size > 0 && raw_offset < size_of_headers {
                bail!(
                    "Section {name:?} raw data starts at 0x{raw_offset:X}, inside SizeOfHeaders 0x{size_of_headers:X}"
                );
            }
            let virtual_end = u64::from(virtual_address)
                + u64::from(virtual_size.max(raw_size));
            if virtual_end > u64::from(size_of_image) {
                bail!(
                    "Section {name:?} virtual range ends at 0x{virtual_end:X}, past SizeOfImage 0x{size_of_image:X}"
                );
            }
            if virtual_end > u64::from(u32::MAX) {
                bail!("Section {name:?} virtual range overflows a 32-bit RVA");
            }

            sections.push(Section {
                name,
                virtual_address,
                virtual_size,
                raw_offset,
                raw_size,
                characteristics,
                header_offset,
            });
        }

        validate_non_overlapping(&sections, size_of_headers)?;

        Ok(Self {
            is_64,
            machine,
            image_base,
            entry_point_rva,
            size_of_image,
            size_of_headers,
            size_of_code,
            file_alignment,
            section_alignment,
            checksum,
            number_of_sections,
            file_len: data.len(),
            pe_offset,
            optional_header_offset,
            optional_header_size,
            section_table_offset,
            checksum_offset,
            size_of_code_offset,
            size_of_image_offset,
            number_of_sections_offset,
            certificate_directory,
            sections,
        })
    }

    pub fn is_x86_or_x64(&self) -> bool {
        matches!(self.machine, 0x014C | 0x8664)
    }

    pub fn is_signed(&self) -> bool {
        self.certificate_directory.1 > 0
    }

    /// Map a virtual address to an RVA.
    pub fn va_to_rva(&self, va: u64) -> Result<u32> {
        let rva = va
            .checked_sub(self.image_base)
            .context("Virtual address is below the PE image base")?;
        u32::try_from(rva).context("Virtual address is outside the 32-bit RVA range")
    }

    /// Map an RVA to a VA.
    pub fn rva_to_va(&self, rva: u32) -> u64 {
        self.image_base + u64::from(rva)
    }

    pub fn section_for_rva(&self, rva: u32) -> Option<&Section> {
        self.sections.iter().find(|section| {
            rva >= section.virtual_address
                && u64::from(rva)
                    < u64::from(section.virtual_address)
                        + u64::from(section.virtual_size.max(section.raw_size))
        })
    }

    /// Map a file-backed RVA range to a section and file offset.
    pub fn map_rva_range(&self, rva: u32, len: usize) -> Result<(&Section, usize)> {
        if len == 0 {
            bail!("Zero-length patch range");
        }
        let len32 = u32::try_from(len).context("Patch is too large")?;
        let end = rva
            .checked_add(len32)
            .context("Patch range overflows the 32-bit RVA space")?;
        let section = self
            .section_for_rva(rva)
            .with_context(|| format!("RVA 0x{rva:X} is not inside a PE section"))?;
        let backed_end = section
            .virtual_address
            .checked_add(section.file_backed_size())
            .context("Section backed range overflows")?;
        if end > backed_end {
            bail!(
                "Patch range 0x{rva:X}..0x{end:X} is not fully backed by file data in section {:?}",
                section.name
            );
        }
        let delta = rva - section.virtual_address;
        let file_offset = section
            .raw_offset
            .checked_add(delta)
            .context("File offset overflows")? as usize;
        let file_end = file_offset
            .checked_add(len)
            .context("File range overflows")?;
        if file_end > self.file_len {
            bail!("Patch range extends past the end of the file");
        }
        Ok((section, file_offset))
    }

    /// Compute a plan for appending one executable section.
    pub fn plan_new_section(
        &self,
        data_len: usize,
        content_len: usize,
        name: &str,
    ) -> Result<NewSectionPlan> {
        if content_len == 0 {
            bail!("A new code section must contain at least one byte");
        }
        if content_len > 64 * 1024 * 1024 {
            bail!("New code section is unreasonably large (maximum 64 MiB)");
        }
        if name.is_empty() || name.len() > 8 || !name.is_ascii() {
            bail!("Section name must be 1..=8 ASCII characters");
        }
        if data_len != self.file_len {
            bail!("PE changed size after it was parsed");
        }

        let new_header_end = self
            .section_table_offset
            .checked_add((self.number_of_sections as usize + 1) * SECTION_HEADER_SIZE)
            .context("New section header offset overflows")?;
        if new_header_end > self.size_of_headers as usize {
            bail!(
                "No room for another section header inside SizeOfHeaders (need {} bytes, have {})",
                new_header_end,
                self.size_of_headers
            );
        }
        let first_raw_offset = self
            .sections
            .iter()
            .filter(|section| section.raw_size > 0)
            .map(|section| section.raw_offset as usize)
            .min()
            .unwrap_or(self.size_of_headers as usize);
        if new_header_end > first_raw_offset {
            bail!("No room for another section header before the first section's raw data");
        }

        let last_virtual_end = self
            .sections
            .iter()
            .map(|section| {
                u64::from(section.virtual_address)
                    + u64::from(section.virtual_size.max(section.raw_size))
            })
            .max()
            .unwrap_or(u64::from(self.size_of_headers));
        let layout_floor = last_virtual_end
            .max(u64::from(self.size_of_image))
            .max(u64::from(self.size_of_headers));
        let virtual_address = align_up_u64(layout_floor, u64::from(self.section_alignment))?;
        let virtual_address = u32::try_from(virtual_address)
            .context("New section virtual address exceeds 32 bits")?;

        let raw_offset = align_up_usize(data_len, self.file_alignment as usize)?;
        let raw_size = align_up_usize(content_len, self.file_alignment as usize)?;
        let raw_size = u32::try_from(raw_size).context("New section raw size exceeds 32 bits")?;
        let raw_offset32 =
            u32::try_from(raw_offset).context("New section file offset exceeds 32 bits")?;
        let virtual_size = u32::try_from(content_len)
            .context("New section virtual size exceeds 32 bits")?;
        let image_end = u64::from(virtual_address)
            .checked_add(u64::from(virtual_size))
            .context("New section image range overflows")?;
        let size_of_image = align_up_u64(image_end, u64::from(self.section_alignment))?;
        let size_of_image = u32::try_from(size_of_image)
            .context("New SizeOfImage exceeds 32 bits")?;
        let size_of_code = self
            .size_of_code
            .checked_add(raw_size)
            .context("SizeOfCode overflows after adding the new section")?;

        Ok(NewSectionPlan {
            name: name.to_string(),
            original_file_len: data_len,
            section_header_offset: new_header_end - SECTION_HEADER_SIZE,
            virtual_address,
            virtual_size,
            raw_offset: raw_offset32,
            raw_size,
            size_of_image,
            size_of_code,
            characteristics: NEW_SECTION_CHARACTERISTICS,
        })
    }

    /// Append a planned section. `data` must be the same bytes that were parsed.
    pub fn add_section(
        &self,
        data: &mut Vec<u8>,
        plan: &NewSectionPlan,
        content: &[u8],
    ) -> Result<()> {
        if data.len() != plan.original_file_len {
            bail!("Binary changed size after the section plan was created");
        }
        if content.len() != plan.virtual_size as usize {
            bail!("Section content length does not match the plan");
        }
        if content.len() as u32 > plan.raw_size {
            bail!("Section content does not fit in its raw allocation");
        }
        if (plan.raw_offset as usize) < data.len() {
            bail!("Planned section raw offset overlaps existing file data");
        }
        if self.number_of_sections == u16::MAX {
            bail!("Section count would overflow");
        }

        write_u16(
            data,
            self.number_of_sections_offset,
            self.number_of_sections + 1,
        )?;
        write_u32(data, self.size_of_code_offset, plan.size_of_code)?;
        write_u32(data, self.size_of_image_offset, plan.size_of_image)?;

        let header = plan.section_header_offset;
        let header_end = header
            .checked_add(SECTION_HEADER_SIZE)
            .context("Section header offset overflows")?;
        if header_end > data.len() || header_end > self.size_of_headers as usize {
            bail!("Section header slot is outside the PE headers");
        }
        for byte in &mut data[header..header_end] {
            *byte = 0;
        }
        let name_bytes = plan.name.as_bytes();
        data[header..header + name_bytes.len()].copy_from_slice(name_bytes);
        write_u32(data, header + 8, plan.virtual_size)?;
        write_u32(data, header + 12, plan.virtual_address)?;
        write_u32(data, header + 16, plan.raw_size)?;
        write_u32(data, header + 20, plan.raw_offset)?;
        write_u32(data, header + 36, plan.characteristics)?;

        let final_len = (plan.raw_offset as usize)
            .checked_add(plan.raw_size as usize)
            .context("Final PE size overflows")?;
        if final_len < data.len() {
            bail!("Planned section would overlap existing file data");
        }
        data.resize(final_len, 0);
        let raw_start = plan.raw_offset as usize;
        data[raw_start..raw_start + content.len()].copy_from_slice(content);

        let checksum = compute_checksum(data, self.checksum_offset)?;
        write_u32(data, self.checksum_offset, checksum)?;
        Ok(())
    }
}

fn validate_non_overlapping(sections: &[Section], size_of_headers: u32) -> Result<()> {
    let mut virtual_ranges: Vec<(u64, u64, &str)> = sections
        .iter()
        .filter(|section| section.virtual_size.max(section.raw_size) > 0)
        .map(|section| {
            (
                u64::from(section.virtual_address),
                u64::from(section.virtual_address)
                    + u64::from(section.virtual_size.max(section.raw_size)),
                section.name.as_str(),
            )
        })
        .collect();
    virtual_ranges.sort_by_key(|(start, _, _)| *start);
    for pair in virtual_ranges.windows(2) {
        if pair[1].0 < pair[0].1 {
            bail!(
                "PE sections {:?} and {:?} have overlapping virtual ranges",
                pair[0].2,
                pair[1].2
            );
        }
    }
    if let Some((start, _, name)) = virtual_ranges.first() {
        if *start < u64::from(size_of_headers) {
            bail!("Section {name:?} starts inside the PE headers");
        }
    }

    let mut raw_ranges: Vec<(u64, u64, &str)> = sections
        .iter()
        .filter(|section| section.raw_size > 0)
        .map(|section| {
            (
                u64::from(section.raw_offset),
                u64::from(section.raw_offset) + u64::from(section.raw_size),
                section.name.as_str(),
            )
        })
        .collect();
    raw_ranges.sort_by_key(|(start, _, _)| *start);
    for pair in raw_ranges.windows(2) {
        if pair[1].0 < pair[0].1 {
            bail!(
                "PE sections {:?} and {:?} have overlapping raw file ranges",
                pair[0].2,
                pair[1].2
            );
        }
    }
    Ok(())
}

fn align_up_usize(value: usize, alignment: usize) -> Result<usize> {
    debug_assert!(alignment.is_power_of_two());
    let mask = alignment - 1;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .context("Alignment calculation overflows")
}

fn align_up_u64(value: u64, alignment: u64) -> Result<u64> {
    debug_assert!(alignment.is_power_of_two());
    let mask = alignment - 1;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .context("Alignment calculation overflows")
}

/// Compute the PE checksum with the checksum field treated as zero.
///
/// The checksum field is four bytes and normally starts on an even offset, but
/// this implementation masks individual bytes so it stays correct even for a
/// malformed image where the field is not word-aligned.
pub fn compute_checksum(data: &[u8], checksum_offset: usize) -> Result<u32> {
    let checksum_end = checksum_offset
        .checked_add(4)
        .context("Checksum field offset overflows")?;
    if checksum_end > data.len() {
        bail!("Checksum field is outside the file");
    }

    let mut sum: u64 = 0;
    let mut offset = 0;
    while offset < data.len() {
        let mut word = 0u16;
        for index in 0..2 {
            let global = offset + index;
            let Some(&byte) = data.get(global) else {
                break;
            };
            if global < checksum_offset || global >= checksum_end {
                word |= u16::from(byte) << (index * 8);
            }
        }
        sum += u64::from(word);
        sum = (sum & 0xFFFF) + (sum >> 16);
        offset += 2;
    }
    sum = (sum & 0xFFFF) + (sum >> 16);
    let checksum = (sum as u32).wrapping_add(data.len() as u32);
    Ok(checksum)
}

fn require(data: &[u8], offset: usize, len: usize) -> Result<()> {
    let end = offset
        .checked_add(len)
        .with_context(|| format!("Offset 0x{offset:X} overflows"))?;
    if end > data.len() {
        bail!(
            "PE is truncated: need bytes 0x{offset:X}..0x{end:X}, file has {} bytes",
            data.len()
        );
    }
    Ok(())
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    require(data, offset, 2)?;
    Ok(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    require(data, offset, 4)?;
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64> {
    require(data, offset, 8)?;
    Ok(u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]))
}

fn write_u16(data: &mut [u8], offset: usize, value: u16) -> Result<()> {
    if offset.checked_add(2).map_or(true, |end| end > data.len()) {
        bail!("Write offset 0x{offset:X} is outside the file");
    }
    data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_u32(data: &mut [u8], offset: usize, value: u32) -> Result<()> {
    if offset.checked_add(4).map_or(true, |end| end > data.len()) {
        bail!("Write offset 0x{offset:X} is outside the file");
    }
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patcher::test_support::{pe32, pe64, PE32_ENTRY_VA};

    #[test]
    fn parses_pe32_and_pe32_plus() {
        let pe = PeInfo::parse(&pe32()).unwrap();
        assert!(!pe.is_64);
        assert_eq!(pe.machine, 0x014C);
        assert_eq!(pe.image_base, 0x0040_0000);
        assert_eq!(pe.sections.len(), 1);
        assert_eq!(pe.sections[0].name, ".text");
        assert!(pe.sections[0].is_executable());

        let pe = PeInfo::parse(&pe64()).unwrap();
        assert!(pe.is_64);
        assert_eq!(pe.machine, 0x8664);
        assert_eq!(pe.image_base, 0x0000_0001_4000_0000);
    }

    #[test]
    fn maps_virtual_addresses_to_file_offsets() {
        let data = pe32();
        let pe = PeInfo::parse(&data).unwrap();
        let rva = pe.va_to_rva(PE32_ENTRY_VA).unwrap();
        assert_eq!(rva, 0x1000);
        let (section, offset) = pe.map_rva_range(rva, 3).unwrap();
        assert_eq!(section.name, ".text");
        assert_eq!(offset, 0x400);
        assert_eq!(&data[offset..offset + 3], &[0x55, 0x8B, 0xEC]);
    }

    #[test]
    fn rejects_ranges_outside_file_backed_data() {
        let data = pe32();
        let pe = PeInfo::parse(&data).unwrap();
        // VirtualSize is 0x100; the raw tail is not patchable.
        assert!(pe.map_rva_range(0x1100, 1).is_err());
        assert!(pe.map_rva_range(0x0FFF, 1).is_err());
        assert!(pe.map_rva_range(0x1000, 0).is_err());
    }

    #[test]
    fn rejects_truncated_headers_and_bad_magic() {
        assert!(PeInfo::parse(&pe32()[..0x3F]).is_err());
        assert!(PeInfo::parse(&pe32()[..0x100]).is_err());

        let mut data = pe32();
        data[0x98] = 0x99;
        data[0x99] = 0x01;
        assert!(PeInfo::parse(&data).is_err());
    }

    #[test]
    fn rejects_section_raw_range_past_eof() {
        let mut data = pe32();
        let header = 0x178;
        data[header + 16..header + 20].copy_from_slice(&0x4000u32.to_le_bytes());
        assert!(PeInfo::parse(&data).is_err());
    }

    #[test]
    fn detects_signed_pe() {
        let mut data = pe32();
        // Data directory 4 (certificate table) starts at optional+96 + 32.
        let cert = 0x98 + 96 + 4 * 8;
        data[cert..cert + 4].copy_from_slice(&0x200u32.to_le_bytes());
        data[cert + 4..cert + 8].copy_from_slice(&8u32.to_le_bytes());
        assert!(PeInfo::parse(&data).unwrap().is_signed());
    }

    #[test]
    fn plans_a_new_executable_section() {
        let data = pe32();
        let pe = PeInfo::parse(&data).unwrap();
        let plan = pe.plan_new_section(data.len(), 32, ".gopt").unwrap();
        assert_eq!(plan.virtual_address, 0x2000);
        assert_eq!(plan.raw_offset, 0x600);
        assert_eq!(plan.raw_size, 0x200);
        assert_eq!(plan.virtual_size, 32);
        assert_eq!(plan.size_of_image, 0x3000);
        assert_eq!(plan.section_header_offset, 0x1A0);
    }

    #[test]
    fn rejects_a_new_section_when_header_space_is_missing() {
        let mut data = pe32();
        let pe = PeInfo::parse(&data).unwrap();
        let section_table_end = pe.section_table_offset + 40;
        data[pe.optional_header_offset + 60..pe.optional_header_offset + 64]
            .copy_from_slice(&(section_table_end as u32).to_le_bytes());
        let pe = PeInfo::parse(&data).unwrap();
        assert!(pe.plan_new_section(data.len(), 16, ".gopt").is_err());
    }

    #[test]
    fn adds_a_section_and_reparses_it() {
        let mut data = pe32();
        let pe = PeInfo::parse(&data).unwrap();
        let plan = pe.plan_new_section(data.len(), 4, ".gopt").unwrap();
        let original_len = data.len();
        pe.add_section(&mut data, &plan, &[0xCC, 0x90, 0x90, 0xC3])
            .unwrap();

        assert!(data.len() > original_len);
        let reparsed = PeInfo::parse(&data).unwrap();
        assert_eq!(reparsed.number_of_sections, 2);
        let new_section = reparsed
            .sections
            .iter()
            .find(|section| section.name == ".gopt")
            .unwrap();
        assert_eq!(new_section.virtual_address, 0x2000);
        assert!(new_section.is_executable());
        assert_eq!(
            &data[new_section.raw_offset as usize..new_section.raw_offset as usize + 4],
            &[0xCC, 0x90, 0x90, 0xC3]
        );
        assert_eq!(reparsed.size_of_image, 0x3000);
        assert_eq!(reparsed.checksum, compute_checksum(&data, reparsed.checksum_offset).unwrap());
    }

    #[test]
    fn checksum_changes_with_file_contents() {
        let data = pe32();
        let pe = PeInfo::parse(&data).unwrap();
        let before = compute_checksum(&data, pe.checksum_offset).unwrap();
        let mut changed = data.clone();
        changed[0x400] ^= 0xFF;
        let after = compute_checksum(&changed, pe.checksum_offset).unwrap();
        assert_ne!(before, after);
    }
    #[test]
    fn rejects_optional_header_too_small_for_required_fields() {
        let mut data = pe32();
        let coff = 0x84;
        data[coff + 16..coff + 18].copy_from_slice(&0x40u16.to_le_bytes());
        let error = PeInfo::parse(&data).unwrap_err().to_string();
        assert!(error.contains("Optional header is too small"), "error: {error}");
    }

    #[test]
    fn rejects_raw_data_inside_the_headers() {
        let mut data = pe32();
        let header = 0x178;
        data[header + 16..header + 20].copy_from_slice(&0x200u32.to_le_bytes());
        data[header + 20..header + 24].copy_from_slice(&0x200u32.to_le_bytes());
        let error = PeInfo::parse(&data).unwrap_err().to_string();
        assert!(error.contains("inside SizeOfHeaders"), "error: {error}");
    }

    #[test]
    fn checksum_masks_an_unaligned_checksum_field() {
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(compute_checksum(&data, 1).unwrap(), 0x0E10);
        assert!(compute_checksum(&data, 5).is_err());
    }

    #[test]
    fn add_section_rejects_a_plan_that_overlaps_existing_data() {
        let mut data = pe32();
        let pe = PeInfo::parse(&data).unwrap();
        let mut plan = pe.plan_new_section(data.len(), 4, ".gopt").unwrap();
        plan.raw_offset = 0;
        let error = pe
            .add_section(&mut data, &plan, &[0xCC, 0x90, 0x90, 0xC3])
            .unwrap_err()
            .to_string();
        assert!(error.contains("overlaps existing file data"), "error: {error}");
    }

}
