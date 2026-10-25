//! Binary patching module.
//!
//! This module supports two modes:
//!
//! * **Copy mode** (no spec): write a byte-identical copy, refusing an in-place
//!   overwrite unless a verified backup already exists.
//! * **Patch mode** (`PatchSpec`): apply bounds-checked byte patches and, at
//!   most, one appended executable section (a code cave) to a PE32/PE32+
//!   binary. Every patch records its original bytes in a JSON manifest, and
//!   `unpatch` restores the exact input bytes.
//!
//! Patch mode never trusts an address, length, or expected byte implicitly. It
//! refuses non-PE files, signed PE files by default, read-only virtual tails,
//! non-executable patch targets, overlapping patches, header overflow, and
//! section-count overflow.

pub(crate) mod pe;
#[cfg(test)]
mod test_support;

use crate::backup::write_atomic;
use crate::config::BinaryFormat;
use crate::hash::sha256_hex;
use anyhow::{bail, Context, Result};
use pe::{compute_checksum, NewSectionPlan, PeInfo};
use serde::{Deserialize, Serialize};
use std::fmt::Write as FmtWrite;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

/// Patch-spec schema version.
pub const PATCH_SCHEMA_VERSION: u32 = 1;
/// Patch-manifest schema version.
pub const PATCH_MANIFEST_SCHEMA_VERSION: u32 = 1;

const MAX_SPEC_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PATCHES: usize = 16 * 1024;
const MAX_PATCH_BYTES: usize = 1024 * 1024;
const MAX_SECTION_BYTES: usize = 64 * 1024 * 1024;
const MAX_NOTE_BYTES: usize = 4096;

/// User-supplied patch specification, loaded from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSpec {
    pub schema_version: u32,
    #[serde(default)]
    pub name: Option<String>,
    /// Optional full SHA-256 of the input file. Strongly recommended.
    #[serde(default)]
    pub target_sha256: Option<String>,
    /// Explicitly allow patching an Authenticode-signed PE. This invalidates
    /// the signature; the default is to refuse.
    #[serde(default)]
    pub allow_signed: bool,
    #[serde(default)]
    pub patches: Vec<BytePatchSpec>,
    /// At most one new executable section in schema version 1.
    #[serde(default)]
    pub sections: Vec<NewSectionSpec>,
}

/// One byte-for-byte patch or one rel32 jump to a code cave.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BytePatchSpec {
    /// Virtual address in hex, e.g. `0x109214b6`.
    pub address: String,
    /// Bytes expected at that address, as hex. Required as a version guard.
    pub expected: String,
    /// Replacement bytes. Must have the same length as `expected`.
    #[serde(default)]
    pub replace: Option<String>,
    /// Alternatively, jump to `section:<name>` or an executable hex VA.
    #[serde(default)]
    pub jump: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// One appended executable section, used as a code cave.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewSectionSpec {
    /// 1..=8 ASCII characters, e.g. `.gopt`.
    pub name: String,
    /// Section bytes as hex. Padded with zeros to FileAlignment.
    pub bytes: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// One patch recorded in a manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedPatch {
    pub address: u64,
    pub rva: u32,
    pub file_offset: u64,
    pub section: String,
    pub before_hex: String,
    pub after_hex: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// One appended section recorded in a manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedSection {
    pub name: String,
    pub address: u64,
    pub rva: u32,
    pub virtual_size: u32,
    pub raw_offset: u64,
    pub raw_size: u32,
    pub characteristics: u32,
    pub header_offset: u64,
    pub header_before_hex: String,
    pub header_after_hex: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// JSON manifest that makes a patch exactly reversible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchManifest {
    pub schema_version: u32,
    pub tool_version: String,
    pub created_unix_secs: u64,
    pub created_unix_nanos: u32,
    #[serde(default)]
    pub name: Option<String>,
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub input_sha256: String,
    pub output_sha256: String,
    pub original_file_len: u64,
    pub output_file_len: u64,
    pub pe_offset: u64,
    pub optional_header_offset: u64,
    pub checksum_offset: u64,
    pub checksum_before: u32,
    pub checksum_after: u32,
    pub pe_is_64: bool,
    pub pe_machine: u16,
    pub entry_point_rva: u32,
    pub optional_header_size: u16,
    pub number_of_sections_before: u16,
    pub number_of_sections_after: u16,
    pub size_of_code_before: u32,
    pub size_of_code_after: u32,
    pub size_of_image_before: u32,
    pub size_of_image_after: u32,
    pub patches: Vec<AppliedPatch>,
    pub sections: Vec<AppliedSection>,
}

/// Result of applying a patch spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchReport {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub manifest_path: PathBuf,
    pub dry_run: bool,
    pub manifest_written: bool,
    #[serde(default)]
    pub name: Option<String>,
    pub input_sha256: String,
    pub output_sha256: String,
    pub patches_applied: usize,
    pub sections_added: usize,
    pub checksum_before: u32,
    pub checksum_after: u32,
    pub patches: Vec<AppliedPatch>,
    pub sections: Vec<AppliedSection>,
}

/// Result of reverting a patch manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnpatchReport {
    pub target: PathBuf,
    pub manifest_path: PathBuf,
    pub dry_run: bool,
    pub patches_reverted: usize,
    pub sections_removed: usize,
    pub output_sha256: String,
    pub restored_sha256: String,
}

/// Patcher for modifying game binaries.
pub struct Patcher {}

impl Default for Patcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Patcher {
    pub fn new() -> Self {
        Self {}
    }

    /// Apply patches to a game binary (copy mode).
    ///
    /// This preserves the original file exactly. Use [`Patcher::apply_spec`] for
    /// real byte patches.
    pub async fn apply_patches(&self, game_path: &Path, output_path: &Path) -> Result<()> {
        info!("Loading game binary: {:?}", game_path);

        let mut data = Vec::new();
        std::fs::File::open(game_path)
            .with_context(|| format!("Failed to open {}", game_path.display()))?
            .read_to_end(&mut data)?;

        if data.is_empty() {
            bail!("Refusing to patch {}: file is empty", game_path.display());
        }

        info!("Binary size: {} bytes", data.len());
        let format = self.detect_format(&data).with_context(|| {
            format!(
                "Cannot patch {}: unsupported binary format",
                game_path.display()
            )
        })?;
        info!("Detected format: {:?}", format);
        self.prepare_output(output_path, false)?;
        if same_file(game_path, output_path) {
            ensure_backup_exists(game_path, &sha256_hex(&data))?;
        }

        info!("No patch spec supplied; writing a byte-for-byte copy");
        write_atomic(output_path, &data, Some(game_path)).with_context(|| {
            format!("Failed to write patched binary {}", output_path.display())
        })?;

        info!("Patched binary saved");
        Ok(())
    }

    /// Apply a validated patch spec and write a manifest.
    ///
    /// `dry_run` validates and computes the result without writing the output
    /// or manifest. In-place writes require an existing backup, exactly like
    /// copy mode.
    pub fn apply_spec(
        &self,
        game_path: &Path,
        output_path: &Path,
        spec: &PatchSpec,
        custom_manifest: Option<&Path>,
        dry_run: bool,
    ) -> Result<PatchReport> {
        spec.validate()?;
        if !game_path.exists() {
            bail!("Game path does not exist: {}", game_path.display());
        }

        let input = std::fs::read(game_path)
            .with_context(|| format!("Failed to read {}", game_path.display()))?;
        if input.is_empty() {
            bail!("Refusing to patch {}: file is empty", game_path.display());
        }
        let format = self.detect_format(&input).with_context(|| {
            format!(
                "Cannot patch {}: unsupported binary format",
                game_path.display()
            )
        })?;
        if format != BinaryFormat::PE {
            bail!("Byte patches are currently supported only for PE binaries");
        }
        let pe = PeInfo::parse(&input)
            .with_context(|| format!("Cannot parse PE headers in {}", game_path.display()))?;

        if pe.is_signed() && !spec.allow_signed {
            bail!(
                "Refusing to patch an Authenticode-signed PE: this would invalidate its signature. \
                 Only use allow_signed in the spec if you own the binary and accept that."
            );
        }

        let input_sha = sha256_hex(&input);
        if let Some(expected) = &spec.target_sha256 {
            let expected = normalize_sha256(expected)?;
            if input_sha != expected {
                bail!(
                    "Input SHA-256 mismatch: spec expects {expected}, file is {input_sha}"
                );
            }
        }

        self.prepare_output(output_path, dry_run)?;
        if same_file(game_path, output_path) && !dry_run {
            ensure_backup_exists(game_path, &input_sha)?;
        }

        let section_plans = build_section_plans(&pe, &input, spec)?;
        let planned_patches = build_patch_plan(&self, &pe, &input, spec, &section_plans)?;

        let mut output = input.clone();
        for patch in &planned_patches {
            let end = patch
                .file_offset
                .checked_add(patch.after.len())
                .context("Patch file range overflows")?;
            if output[patch.file_offset..end] != patch.before {
                bail!(
                    "Patch at 0x{:X} no longer matches the expected bytes",
                    patch.address
                );
            }
            output[patch.file_offset..end].copy_from_slice(&patch.after);
        }

        let mut applied_sections = Vec::new();
        if let Some((plan, content, note)) = section_plans.first() {
            let header_end = plan.section_header_offset + 40;
            let header_before = input
                .get(plan.section_header_offset..header_end)
                .context("Section header slot is outside the input")?
                .to_vec();
            pe.add_section(&mut output, plan, content)
                .context("Failed to append the new executable section")?;
            let header_after = output
                .get(plan.section_header_offset..header_end)
                .context("Section header slot is outside the output")?
                .to_vec();
            applied_sections.push(AppliedSection {
                name: plan.name.clone(),
                address: pe.rva_to_va(plan.virtual_address),
                rva: plan.virtual_address,
                virtual_size: plan.virtual_size,
                raw_offset: u64::from(plan.raw_offset),
                raw_size: plan.raw_size,
                characteristics: plan.characteristics,
                header_offset: plan.section_header_offset as u64,
                header_before_hex: to_hex(&header_before),
                header_after_hex: to_hex(&header_after),
                note: note.clone(),
            });
        } else if pe.checksum != 0 {
            let checksum = compute_checksum(&output, pe.checksum_offset)?;
            write_le_u32(&mut output, pe.checksum_offset, checksum)?;
        }

        let output_sha = sha256_hex(&output);
        let manifest_path = match custom_manifest {
            Some(path) => absolute(path),
            None => absolute(&default_manifest_path(output_path, &output_sha)),
        };
        if same_file(&manifest_path, output_path) || same_file(&manifest_path, game_path) {
            bail!(
                "Refusing to write the patch manifest over the input or output binary: {}",
                manifest_path.display()
            );
        }
        if !dry_run {
            if let Ok(metadata) = std::fs::symlink_metadata(&manifest_path) {
                if metadata.file_type().is_symlink() {
                    bail!(
                        "Refusing to write the patch manifest over a symbolic link: {}",
                        manifest_path.display()
                    );
                }
                if metadata.is_dir() {
                    bail!(
                        "Patch manifest path is a directory: {}",
                        manifest_path.display()
                    );
                }
                if !metadata.is_file() {
                    bail!(
                        "Refusing to overwrite {}: not a regular file",
                        manifest_path.display()
                    );
                }
            }
        }

        let checksum_after = read_u32_checked(&output, pe.checksum_offset)?;
        let manifest = PatchManifest {
            schema_version: PATCH_MANIFEST_SCHEMA_VERSION,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            created_unix_secs: now_parts().0,
            created_unix_nanos: now_parts().1,
            name: spec.name.clone(),
            input_path: absolute(game_path),
            output_path: absolute(output_path),
            input_sha256: input_sha.clone(),
            output_sha256: output_sha.clone(),
            original_file_len: input.len() as u64,
            output_file_len: output.len() as u64,
            pe_offset: pe.pe_offset as u64,
            optional_header_offset: pe.optional_header_offset as u64,
            checksum_offset: pe.checksum_offset as u64,
            checksum_before: pe.checksum,
            checksum_after,
            pe_is_64: pe.is_64,
            pe_machine: pe.machine,
            entry_point_rva: pe.entry_point_rva,
            optional_header_size: pe.optional_header_size,
            number_of_sections_before: pe.number_of_sections,
            number_of_sections_after: pe.number_of_sections
                + u16::from(!applied_sections.is_empty()),
            size_of_code_before: pe.size_of_code,
            size_of_code_after: section_plans
                .first()
                .map(|(plan, _, _)| plan.size_of_code)
                .unwrap_or(pe.size_of_code),
            size_of_image_before: pe.size_of_image,
            size_of_image_after: section_plans
                .first()
                .map(|(plan, _, _)| plan.size_of_image)
                .unwrap_or(pe.size_of_image),
            patches: planned_patches
                .iter()
                .map(|patch| AppliedPatch {
                    address: patch.address,
                    rva: patch.rva,
                    file_offset: patch.file_offset as u64,
                    section: patch.section.clone(),
                    before_hex: to_hex(&patch.before),
                    after_hex: to_hex(&patch.after),
                    note: patch.note.clone(),
                })
                .collect(),
            sections: applied_sections.clone(),
        };

        manifest.validate()?;
        let manifest_json = serde_json::to_vec_pretty(&manifest)
            .context("Failed to serialize patch manifest")?;

        if !dry_run {
            if let Some(parent) = manifest_path.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create manifest directory {}", parent.display())
                })?;
            }

            write_atomic(output_path, &output, Some(game_path)).with_context(|| {
                format!("Failed to write patched binary {}", output_path.display())
            })?;
            let readback = std::fs::read(output_path)
                .with_context(|| format!("Failed to verify {}", output_path.display()))?;
            let readback_sha = sha256_hex(&readback);
            if readback_sha != output_sha {
                bail!(
                    "Post-write verification failed for {}: expected {}, got {}",
                    output_path.display(),
                    output_sha,
                    readback_sha
                );
            }

            write_atomic(&manifest_path, &manifest_json, None).with_context(|| {
                format!("Failed to write patch manifest {}", manifest_path.display())
            })?;
        }

        Ok(PatchReport {
            input_path: absolute(game_path),
            output_path: absolute(output_path),
            manifest_path,
            dry_run,
            manifest_written: !dry_run,
            name: spec.name.clone(),
            input_sha256: input_sha,
            output_sha256: output_sha,
            patches_applied: manifest.patches.len(),
            sections_added: applied_sections.len(),
            checksum_before: manifest.checksum_before,
            checksum_after: manifest.checksum_after,
            patches: manifest.patches,
            sections: manifest.sections,
        })
    }

    /// Revert exactly the changes recorded by a patch manifest.
    pub fn unpatch(
        manifest_path: &Path,
        target_override: Option<&Path>,
        dry_run: bool,
    ) -> Result<UnpatchReport> {
        let manifest = load_patch_manifest(manifest_path)?;
        manifest.validate()?;

        let target = match target_override {
            Some(target) => {
                if !same_file(target, &manifest.output_path) {
                    info!(
                        "Unpatching a copy at {} (manifest output was {})",
                        target.display(),
                        manifest.output_path.display()
                    );
                }
                target.to_path_buf()
            }
            None => manifest.output_path.clone(),
        };
        let metadata = std::fs::symlink_metadata(&target)
            .with_context(|| format!("Patched file not found: {}", target.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("Refusing to unpatch a symbolic link: {}", target.display());
        }
        if !metadata.is_file() {
            bail!("Refusing to unpatch a non-regular file: {}", target.display());
        }

        let current = std::fs::read(&target)
            .with_context(|| format!("Failed to read {}", target.display()))?;
        if current.len() as u64 != manifest.output_file_len {
            bail!(
                "Patched file size changed: manifest says {} bytes, file is {}",
                manifest.output_file_len,
                current.len()
            );
        }
        let current_sha = sha256_hex(&current);
        if current_sha != manifest.output_sha256 {
            bail!(
                "Patched file SHA-256 mismatch: manifest says {}, file is {}. \
                 Refusing to unpatch a changed file.",
                manifest.output_sha256,
                current_sha
            );
        }

        let pe = PeInfo::parse(&current)
            .context("Patched file is no longer a valid PE; refusing to unpatch")?;
        if pe.number_of_sections != manifest.number_of_sections_after
            || pe.size_of_code != manifest.size_of_code_after
            || pe.size_of_image != manifest.size_of_image_after
            || pe.checksum != manifest.checksum_after
            || pe.pe_offset as u64 != manifest.pe_offset
            || pe.optional_header_offset as u64 != manifest.optional_header_offset
            || pe.checksum_offset as u64 != manifest.checksum_offset
            || pe.is_64 != manifest.pe_is_64
            || pe.machine != manifest.pe_machine
            || pe.entry_point_rva != manifest.entry_point_rva
            || pe.optional_header_size != manifest.optional_header_size
        {
            bail!("Patched PE headers do not match the manifest; refusing to unpatch");
        }

        let mut restored = current.clone();
        for patch in &manifest.patches {
            let before = parse_hex_bytes(&patch.before_hex)?;
            let after = parse_hex_bytes(&patch.after_hex)?;
            if before.len() != after.len() || before.is_empty() {
                bail!("Manifest patch at 0x{:X} has invalid byte lengths", patch.address);
            }
            let offset = usize::try_from(patch.file_offset)
                .context("Manifest patch offset is too large")?;
            let end = offset
                .checked_add(after.len())
                .context("Manifest patch range overflows")?;
            if end > restored.len() {
                bail!("Manifest patch at 0x{:X} is outside the file", patch.address);
            }
            if restored[offset..end] != after {
                bail!(
                    "Patched bytes at 0x{:X} no longer match the manifest",
                    patch.address
                );
            }
            restored[offset..end].copy_from_slice(&before);
        }

        for section in &manifest.sections {
            let before = parse_hex_bytes(&section.header_before_hex)?;
            let after = parse_hex_bytes(&section.header_after_hex)?;
            if before.len() != 40 || after.len() != 40 {
                bail!("Manifest section header is not 40 bytes");
            }
            let offset = usize::try_from(section.header_offset)
                .context("Manifest section header offset is too large")?;
            let end = offset
                .checked_add(40)
                .context("Manifest section header range overflows")?;
            if end > restored.len() {
                bail!("Manifest section header is outside the file");
            }
            if restored[offset..end] != after {
                bail!("Section header {} no longer matches the manifest", section.name);
            }
            restored[offset..end].copy_from_slice(&before);
        }

        if manifest.sections.is_empty() {
            write_le_u32(&mut restored, manifest.checksum_offset as usize, manifest.checksum_before)?;
        } else {
            write_le_u16(
                &mut restored,
                pe.number_of_sections_offset,
                manifest.number_of_sections_before,
            )?;
            write_le_u32(
                &mut restored,
                pe.size_of_code_offset,
                manifest.size_of_code_before,
            )?;
            write_le_u32(
                &mut restored,
                pe.size_of_image_offset,
                manifest.size_of_image_before,
            )?;
            write_le_u32(
                &mut restored,
                pe.checksum_offset,
                manifest.checksum_before,
            )?;
        }
        let original_len = usize::try_from(manifest.original_file_len)
            .context("Manifest original file length is too large")?;
        if original_len > restored.len() {
            bail!("Manifest original file length is past the end of the patched file");
        }
        restored.truncate(original_len);

        let restored_sha = sha256_hex(&restored);
        if restored_sha != manifest.input_sha256 {
            bail!(
                "Refusing to write rollback: restored SHA-256 {} does not match original {}",
                restored_sha,
                manifest.input_sha256
            );
        }

        if !dry_run {
            write_atomic(&target, &restored, Some(&target))
                .with_context(|| format!("Failed to write rollback to {}", target.display()))?;
            let readback = std::fs::read(&target)
                .with_context(|| format!("Failed to verify rollback of {}", target.display()))?;
            let readback_sha = sha256_hex(&readback);
            if readback_sha != manifest.input_sha256 {
                bail!(
                    "Post-rollback verification failed for {}: expected {}, got {}",
                    target.display(),
                    manifest.input_sha256,
                    readback_sha
                );
            }
        }

        Ok(UnpatchReport {
            target: absolute(&target),
            manifest_path: absolute(manifest_path),
            dry_run,
            patches_reverted: manifest.patches.len(),
            sections_removed: manifest.sections.len(),
            output_sha256: current_sha,
            restored_sha256: restored_sha,
        })
    }

    /// Detect binary format from the file header.
    fn detect_format(&self, data: &[u8]) -> Result<BinaryFormat> {
        if data.len() >= 0x40 && data[0] == b'M' && data[1] == b'Z' {
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
                        return Ok(BinaryFormat::PE);
                    }
                }
            }
        }

        if data.len() >= 4
            && data[0] == 0x7F
            && data[1] == b'E'
            && data[2] == b'L'
            && data[3] == b'F'
        {
            return Ok(BinaryFormat::ELF);
        }

        if data.len() >= 4 {
            let bytes = [data[0], data[1], data[2], data[3]];
            let magic_be = u32::from_be_bytes(bytes);
            let magic_le = u32::from_le_bytes(bytes);
            const MACHO_MAGICS: [u32; 4] =
                [0xFEED_FACE, 0xFEED_FACF, 0xCAFE_BABE, 0xCAFE_BABF];

            if MACHO_MAGICS.contains(&magic_be) || MACHO_MAGICS.contains(&magic_le) {
                return Ok(BinaryFormat::MachO);
            }
        }

        bail!(
            "Unknown binary format (first bytes: {:02X?})",
            &data[..data.len().min(8)]
        )
    }

    fn prepare_output(&self, output_path: &Path, dry_run: bool) -> Result<()> {
        if let Ok(metadata) = std::fs::symlink_metadata(output_path) {
            if metadata.file_type().is_symlink() {
                bail!(
                    "Output path {} is a symbolic link; pass the real file path instead",
                    output_path.display()
                );
            }
            if metadata.is_dir() {
                bail!(
                    "Output path {} is a directory; pass an executable file path",
                    output_path.display()
                );
            }
            if !metadata.is_file() {
                bail!(
                    "Refusing to overwrite {}: not a regular file",
                    output_path.display()
                );
            }
        }
        if !dry_run {
            if let Some(parent) = output_path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).with_context(|| {
                        format!("Failed to create output directory {}", parent.display())
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Create an x86/x86-64 `jmp rel32` trampoline.
    fn create_trampoline(&self, original_address: u64, new_function_address: u64) -> Result<Vec<u8>> {
        let offset = new_function_address as i128 - original_address as i128 - 5;

        if offset < i32::MIN as i128 || offset > i32::MAX as i128 {
            bail!(
                "Trampoline target 0x{:X} is out of rel32 range from 0x{:X}",
                new_function_address,
                original_address
            );
        }

        let mut bytes = Vec::with_capacity(5);
        bytes.push(0xE9);
        bytes.extend_from_slice(&(offset as i32).to_le_bytes());
        Ok(bytes)
    }
}

impl PatchSpec {
    /// Load and validate a JSON patch spec.
    pub fn load(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path)
            .with_context(|| format!("Patch spec not found: {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("Patch spec {} is a symbolic link", path.display());
        }
        if !metadata.is_file() {
            bail!("Patch spec {} is not a regular file", path.display());
        }
        if metadata.len() > MAX_SPEC_BYTES {
            bail!("Patch spec {} is unreasonably large", path.display());
        }
        let mut content = String::new();
        std::fs::File::open(path)
            .with_context(|| format!("Failed to open patch spec {}", path.display()))?
            .take(MAX_SPEC_BYTES + 1)
            .read_to_string(&mut content)
            .with_context(|| format!("Failed to read patch spec {}", path.display()))?;
        if content.len() as u64 > MAX_SPEC_BYTES {
            bail!("Patch spec {} is unreasonably large", path.display());
        }
        let spec: PatchSpec = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse patch spec {}", path.display()))?;
        spec.validate()?;
        Ok(spec)
    }

    /// Validate the spec without touching a binary.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != PATCH_SCHEMA_VERSION {
            bail!(
                "Unsupported patch spec version {} (this build supports {})",
                self.schema_version,
                PATCH_SCHEMA_VERSION
            );
        }
        if self.patches.is_empty() && self.sections.is_empty() {
            bail!("Patch spec has no patches and no sections");
        }
        if self.patches.len() > MAX_PATCHES {
            bail!("Patch spec contains more than {MAX_PATCHES} patches");
        }
        if self.sections.len() > 1 {
            bail!("Patch spec schema version 1 supports at most one new section");
        }
        if let Some(name) = &self.name {
            if name.len() > 128 {
                bail!("Patch spec name is too long (maximum 128 bytes)");
            }
        }
        if let Some(expected) = &self.target_sha256 {
            normalize_sha256(expected)?;
        }

        let mut section_names = Vec::new();
        for section in &self.sections {
            if section.name.is_empty()
                || section.name.len() > 8
                || !section.name.is_ascii()
            {
                bail!("Section name must be 1..=8 ASCII characters");
            }
            if section_names.contains(&section.name) {
                bail!("Duplicate section name {:?}", section.name);
            }
            section_names.push(section.name.clone());
            let bytes = parse_hex_bytes(&section.bytes)?;
            if bytes.is_empty() {
                bail!("Section {:?} has no bytes", section.name);
            }
            if bytes.len() > MAX_SECTION_BYTES {
                bail!("Section {:?} is larger than {MAX_SECTION_BYTES} bytes", section.name);
            }
            validate_note(&section.note)?;
        }

        for (index, patch) in self.patches.iter().enumerate() {
            parse_hex_u64(&patch.address)
                .with_context(|| format!("Patch {index} has an invalid address"))?;
            let expected = parse_hex_bytes(&patch.expected)
                .with_context(|| format!("Patch {index} has invalid expected bytes"))?;
            if expected.is_empty() {
                bail!("Patch {index} has empty expected bytes");
            }
            if expected.len() > MAX_PATCH_BYTES {
                bail!("Patch {index} is larger than {MAX_PATCH_BYTES} bytes");
            }
            match (&patch.replace, &patch.jump) {
                (Some(_), Some(_)) => bail!("Patch {index} sets both replace and jump"),
                (None, None) => bail!("Patch {index} sets neither replace nor jump"),
                (Some(replace), None) => {
                    let replacement = parse_hex_bytes(replace)
                        .with_context(|| format!("Patch {index} has invalid replacement bytes"))?;
                    if replacement.is_empty() {
                        bail!("Patch {index} has an empty replacement");
                    }
                    if replacement.len() != expected.len() {
                        bail!(
                            "Patch {index} changes length: {} expected bytes, {} replacement bytes",
                            expected.len(),
                            replacement.len()
                        );
                    }
                }
                (None, Some(jump)) => {
                    if expected.len() != 5 {
                        bail!("Jump patch {index} must expect exactly 5 bytes");
                    }
                    validate_jump_target(jump)?;
                }
            }
            validate_note(&patch.note)?;
        }
        Ok(())
    }
}

impl PatchManifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != PATCH_MANIFEST_SCHEMA_VERSION {
            bail!(
                "Unsupported patch manifest version {} (this build supports {})",
                self.schema_version,
                PATCH_MANIFEST_SCHEMA_VERSION
            );
        }
        validate_sha256(&self.input_sha256)?;
        validate_sha256(&self.output_sha256)?;
        if self.original_file_len == 0 || self.output_file_len < self.original_file_len {
            bail!("Patch manifest has invalid file lengths");
        }
        if self.patches.len() > MAX_PATCHES {
            bail!("Patch manifest contains too many patches");
        }
        if self.sections.len() > 1 {
            bail!("Patch manifest contains too many sections");
        }
        if self.optional_header_size == 0 {
            bail!("Patch manifest has an empty optional header");
        }
        if self
            .checksum_offset
            .checked_add(4)
            .map_or(true, |end| end > self.output_file_len)
        {
            bail!("Patch manifest checksum field is outside the patched file");
        }
        if self.number_of_sections_after
            != self
                .number_of_sections_before
                .saturating_add(u16::from(!self.sections.is_empty()))
        {
            bail!("Patch manifest has an inconsistent section count");
        }
        let mut ranges = Vec::with_capacity(self.patches.len());
        for patch in &self.patches {
            let before = parse_hex_bytes(&patch.before_hex)?;
            let after = parse_hex_bytes(&patch.after_hex)?;
            if before.is_empty() || before.len() != after.len() {
                bail!("Patch manifest has invalid patch bytes at 0x{:X}", patch.address);
            }
            let end = patch
                .file_offset
                .checked_add(after.len() as u64)
                .context("Patch manifest range overflows")?;
            if end > self.original_file_len {
                bail!("Patch manifest patch at 0x{:X} is outside the original file", patch.address);
            }
            ranges.push((patch.file_offset, end));
        }
        ranges.sort_unstable();
        for pair in ranges.windows(2) {
            if pair[1].0 < pair[0].1 {
                bail!("Patch manifest contains overlapping byte patches");
            }
        }
        for section in &self.sections {
            let before = parse_hex_bytes(&section.header_before_hex)?;
            let after = parse_hex_bytes(&section.header_after_hex)?;
            if before.len() != 40 || after.len() != 40 {
                bail!("Patch manifest section header is not 40 bytes");
            }
            let header_end = section
                .header_offset
                .checked_add(40)
                .context("Patch manifest section header range overflows")?;
            if header_end > self.original_file_len || header_end > self.output_file_len {
                bail!("Patch manifest section header is outside the original file");
            }
            if section.virtual_size == 0 || section.raw_size == 0 {
                bail!("Patch manifest section has an empty virtual or raw size");
            }
            if section.raw_offset < self.original_file_len {
                bail!("Patch manifest section data overlaps the original file");
            }
            if section
                .raw_offset
                .checked_add(u64::from(section.raw_size))
                .map_or(true, |end| end > self.output_file_len)
            {
                bail!("Patch manifest section data is outside the patched file");
            }
            if section.characteristics & 0x2000_0000 == 0 {
                bail!("Patch manifest section is not executable");
            }
            if ranges.iter().any(|(start, end)| {
                section.header_offset < *end && *start < header_end
            }) {
                bail!("Patch manifest byte patch overlaps a new section header");
            }
        }
        Ok(())
    }
}

struct PlannedPatch {
    address: u64,
    rva: u32,
    file_offset: usize,
    section: String,
    before: Vec<u8>,
    after: Vec<u8>,
    note: Option<String>,
}

fn build_section_plans(
    pe: &PeInfo,
    input: &[u8],
    spec: &PatchSpec,
) -> Result<Vec<(NewSectionPlan, Vec<u8>, Option<String>)>> {
    let mut plans = Vec::new();
    for section in &spec.sections {
        let content = parse_hex_bytes(&section.bytes)?;
        let plan = pe.plan_new_section(input.len(), content.len(), &section.name)?;
        plans.push((plan, content, section.note.clone()));
    }
    Ok(plans)
}

fn build_patch_plan(
    patcher: &Patcher,
    pe: &PeInfo,
    input: &[u8],
    spec: &PatchSpec,
    sections: &[(NewSectionPlan, Vec<u8>, Option<String>)],
) -> Result<Vec<PlannedPatch>> {
    let mut planned = Vec::with_capacity(spec.patches.len());
    for patch in &spec.patches {
        let address = parse_hex_u64(&patch.address)?;
        let rva = pe.va_to_rva(address)?;
        let (section, file_offset) = pe.map_rva_range(rva, parse_hex_bytes(&patch.expected)?.len())?;
        if !section.is_executable() {
            bail!(
                "Patch target 0x{address:X} is in non-executable section {:?}",
                section.name
            );
        }
        let before = parse_hex_bytes(&patch.expected)?;
        let after = match (&patch.replace, &patch.jump) {
            (Some(replace), None) => parse_hex_bytes(replace)?,
            (None, Some(jump)) => {
                if !pe.is_x86_or_x64() {
                    bail!(
                        "Jump patches are supported only for x86/x86-64 PE images; use a byte replacement here"
                    );
                }
                let target_va = resolve_jump_target(jump, pe, sections)?;
                patcher.create_trampoline(address, target_va)?
            }
            _ => unreachable!("PatchSpec::validate checked this"),
        };
        if before.len() != after.len() {
            bail!("Patch at 0x{address:X} changes length");
        }
        let end = file_offset
            .checked_add(before.len())
            .context("Patch file range overflows")?;
        if end > input.len() {
            bail!("Patch at 0x{address:X} is outside the input file");
        }
        if input[file_offset..end] != before {
            bail!(
                "Expected bytes at 0x{address:X} do not match the input file (found {})",
                to_hex(&input[file_offset..end])
            );
        }
        planned.push(PlannedPatch {
            address,
            rva,
            file_offset,
            section: section.name.clone(),
            before,
            after,
            note: patch.note.clone(),
        });
    }

    planned.sort_by_key(|patch| patch.rva);
    for pair in planned.windows(2) {
        let previous = &pair[0];
        let next = &pair[1];
        if previous.rva as u64 + previous.before.len() as u64 > next.rva as u64 {
            bail!(
                "Patch ranges overlap at 0x{:X} and 0x{:X}",
                previous.address,
                next.address
            );
        }
    }
    Ok(planned)
}

fn resolve_jump_target(
    target: &str,
    pe: &PeInfo,
    sections: &[(NewSectionPlan, Vec<u8>, Option<String>)],
) -> Result<u64> {
    if let Some(name) = target.strip_prefix("section:") {
        let (plan, _, _) = sections
            .iter()
            .find(|(plan, _, _)| plan.name == name)
            .with_context(|| format!("Jump target section {name:?} was not declared"))?;
        return Ok(pe.rva_to_va(plan.virtual_address));
    }
    let address = parse_hex_u64(target)?;
    let rva = pe.va_to_rva(address)?;
    let (section, _) = pe.map_rva_range(rva, 1)?;
    if !section.is_executable() {
        bail!("Jump target 0x{address:X} is not in an executable section");
    }
    Ok(address)
}

fn validate_jump_target(target: &str) -> Result<()> {
    if let Some(name) = target.strip_prefix("section:") {
        if name.is_empty() || name.len() > 8 || !name.is_ascii() {
            bail!("Jump target section name must be 1..=8 ASCII characters");
        }
    } else {
        parse_hex_u64(target)?;
    }
    Ok(())
}

fn validate_note(note: &Option<String>) -> Result<()> {
    if let Some(note) = note {
        if note.len() > MAX_NOTE_BYTES {
            bail!("Patch note is too long (maximum {MAX_NOTE_BYTES} bytes)");
        }
    }
    Ok(())
}

fn ensure_backup_exists(path: &Path, expected_sha256: &str) -> Result<()> {
    let backups = crate::backup::list_backups(path)?;
    let matching = backups
        .iter()
        .find(|record| record.sha256 == expected_sha256);
    let Some(record) = matching else {
        bail!(
            "Refusing to overwrite {} in place without a matching backup of the current bytes. \
             Run `backup --game <path>` first.",
            path.display()
        );
    };
    crate::backup::verify_record(record).with_context(|| {
        format!(
            "Refusing to overwrite {}: its matching backup {} failed verification",
            path.display(),
            record.backup_path.display()
        )
    })?;
    Ok(())
}

fn load_patch_manifest(path: &Path) -> Result<PatchManifest> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("Patch manifest not found: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("Patch manifest {} is a symbolic link", path.display());
    }
    if !metadata.is_file() {
        bail!("Patch manifest {} is not a regular file", path.display());
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        bail!("Patch manifest {} is unreasonably large", path.display());
    }
    let mut content = String::new();
    std::fs::File::open(path)
        .with_context(|| format!("Failed to open patch manifest {}", path.display()))?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut content)
        .with_context(|| format!("Failed to read patch manifest {}", path.display()))?;
    if content.len() as u64 > MAX_MANIFEST_BYTES {
        bail!("Patch manifest {} is unreasonably large", path.display());
    }
    serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse patch manifest {}", path.display()))
}

fn default_manifest_path(output: &Path, output_sha: &str) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("patched");
    let manifest_name = format!(
        "{}.{}.patch.json",
        name,
        output_sha.get(..16).unwrap_or("unknown")
    );
    output.with_file_name(manifest_name)
}

fn parse_hex_u64(input: &str) -> Result<u64> {
    let value = input.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Invalid hexadecimal address: {input:?}");
    }
    u64::from_str_radix(value, 16).with_context(|| format!("Invalid hexadecimal address: {input:?}"))
}

fn parse_hex_bytes(input: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for token in input.split(|c: char| c.is_ascii_whitespace() || c == ',' || c == '_') {
        let token = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);
        if token.is_empty() {
            continue;
        }
        if !token.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("Invalid hexadecimal byte string: {input:?}");
        }
        if token.len() % 2 != 0 {
            bail!("Hexadecimal byte string has an odd number of digits: {input:?}");
        }
        for pair in token.as_bytes().chunks_exact(2) {
            let text = std::str::from_utf8(pair).context("Invalid byte pair")?;
            bytes.push(u8::from_str_radix(text, 16)?);
        }
    }
    Ok(bytes)
}

fn normalize_sha256(input: &str) -> Result<String> {
    let value = input.trim().to_ascii_lowercase();
    if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Invalid SHA-256 digest: {input:?}");
    }
    Ok(value)
}

fn validate_sha256(input: &str) -> Result<()> {
    normalize_sha256(input).map(|_| ())
}

fn to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn read_u32_checked(data: &[u8], offset: usize) -> Result<u32> {
    if offset.checked_add(4).map_or(true, |end| end > data.len()) {
        bail!("Read offset 0x{offset:X} is outside the file");
    }
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

fn write_le_u16(data: &mut [u8], offset: usize, value: u16) -> Result<()> {
    if offset.checked_add(2).map_or(true, |end| end > data.len()) {
        bail!("Write offset 0x{offset:X} is outside the file");
    }
    data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_le_u32(data: &mut [u8], offset: usize, value: u32) -> Result<()> {
    if offset.checked_add(4).map_or(true, |end| end > data.len()) {
        bail!("Write offset 0x{offset:X} is outside the file");
    }
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn now_parts() -> (u64, u32) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (now.as_secs(), now.subsec_nanos())
}

pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            if a.dev() == b.dev() && a.ino() == b.ino() {
                return true;
            }
        }
    }

    let canonical_a = std::fs::canonicalize(a).ok();
    let canonical_b = std::fs::canonicalize(b).ok();

    match (canonical_a, canonical_b) {
        (Some(a), Some(b)) => a == b,
        _ => absolute(a) == absolute(b),
    }
}

/// Normalize `.` and `..` lexically without resolving symlinks.
///
/// This is used for collision checks against paths that do not exist yet (for
/// example, an output file before it has been written). Two lexical aliases of
/// the same nonexistent path must compare equal so a manifest can never
/// overwrite the binary it describes.
pub(crate) fn absolute(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patcher::test_support::{pe32, PE32_ENTRY_VA};

    fn patch_spec(
        address: u64,
        expected: &str,
        replace: &str,
    ) -> PatchSpec {
        PatchSpec {
            schema_version: PATCH_SCHEMA_VERSION,
            name: Some("test".to_string()),
            target_sha256: None,
            allow_signed: false,
            patches: vec![BytePatchSpec {
                address: format!("0x{address:X}"),
                expected: expected.to_string(),
                replace: Some(replace.to_string()),
                jump: None,
                note: None,
            }],
            sections: Vec::new(),
        }
    }

    #[test]
    fn hex_parsing_accepts_common_forms() {
        assert_eq!(parse_hex_u64("0x109214b6").unwrap(), 0x1092_14B6);
        assert_eq!(parse_hex_bytes("55 8B EC").unwrap(), vec![0x55, 0x8B, 0xEC]);
        assert_eq!(parse_hex_bytes("0x55,0x8b_ec").unwrap(), vec![0x55, 0x8B, 0xEC]);
        assert!(parse_hex_bytes("5").is_err());
        assert!(parse_hex_bytes("zz").is_err());
        assert!(parse_hex_u64("0x").is_err());
    }

    #[test]
    fn applies_a_byte_patch_and_writes_a_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        let manifest = dir.path().join("patch.json");
        std::fs::write(&input, pe32()).unwrap();

        let spec = patch_spec(PE32_ENTRY_VA, "55", "90");
        let report = Patcher::new()
            .apply_spec(&input, &output, &spec, Some(&manifest), false)
            .unwrap();

        assert_eq!(report.patches_applied, 1);
        assert_eq!(report.sections_added, 0);
        assert!(report.manifest_written);
        assert!(manifest.exists());
        let patched = std::fs::read(&output).unwrap();
        assert_eq!(patched[0x400], 0x90);
        assert_eq!(&patched[0x401..0x403], &[0x8B, 0xEC]);

        let manifest = load_patch_manifest(&manifest).unwrap();
        manifest.validate().unwrap();
        assert_eq!(manifest.patches[0].before_hex, "55");
        assert_eq!(manifest.patches[0].after_hex, "90");
    }

    #[test]
    fn rejects_an_expected_byte_mismatch_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        std::fs::write(&input, pe32()).unwrap();
        let spec = patch_spec(PE32_ENTRY_VA, "90", "91");

        let error = Patcher::new()
            .apply_spec(&input, &output, &spec, None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("do not match"), "error: {error}");
        assert!(!output.exists());
    }

    #[test]
    fn rejects_length_changes_and_overlaps() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        std::fs::write(&input, pe32()).unwrap();

        let mut spec = patch_spec(PE32_ENTRY_VA, "55 8B", "90");
        assert!(spec.validate().is_err());
        spec = patch_spec(PE32_ENTRY_VA, "55 8B", "90 90");
        let other = BytePatchSpec {
            address: format!("0x{:X}", PE32_ENTRY_VA + 1),
            expected: "8B".to_string(),
            replace: Some("90".to_string()),
            jump: None,
            note: None,
        };
        spec.patches.push(other);
        let error = Patcher::new()
            .apply_spec(&input, &output, &spec, None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("overlap"), "error: {error}");
    }

    #[test]
    fn rejects_non_executable_and_signed_targets() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        let mut signed = pe32();
        let cert = 0x98 + 96 + 4 * 8;
        signed[cert..cert + 4].copy_from_slice(&0x200u32.to_le_bytes());
        signed[cert + 4..cert + 8].copy_from_slice(&8u32.to_le_bytes());
        std::fs::write(&input, signed).unwrap();

        let spec = patch_spec(PE32_ENTRY_VA, "55", "90");
        let error = Patcher::new()
            .apply_spec(&input, &output, &spec, None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("signed"), "error: {error}");
    }

    #[test]
    fn allows_signed_when_explicitly_requested() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        let mut signed = pe32();
        let cert = 0x98 + 96 + 4 * 8;
        signed[cert..cert + 4].copy_from_slice(&0x200u32.to_le_bytes());
        signed[cert + 4..cert + 8].copy_from_slice(&8u32.to_le_bytes());
        std::fs::write(&input, signed).unwrap();

        let mut spec = patch_spec(PE32_ENTRY_VA, "55", "90");
        spec.allow_signed = true;
        Patcher::new()
            .apply_spec(&input, &output, &spec, None, false)
            .unwrap();
        assert!(output.exists());
    }

    #[test]
    fn applies_a_jump_to_a_new_code_cave() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        std::fs::write(&input, pe32()).unwrap();

        let spec = PatchSpec {
            schema_version: PATCH_SCHEMA_VERSION,
            name: Some("cave".to_string()),
            target_sha256: None,
            allow_signed: false,
            patches: vec![BytePatchSpec {
                address: format!("0x{PE32_ENTRY_VA:X}"),
                expected: "55 8B EC 83 EC".to_string(),
                replace: None,
                jump: Some("section:stub".to_string()),
                note: None,
            }],
            sections: vec![NewSectionSpec {
                name: "stub".to_string(),
                bytes: "90 90 90 C3".to_string(),
                note: None,
            }],
        };
        let report = Patcher::new()
            .apply_spec(&input, &output, &spec, None, false)
            .unwrap();
        assert_eq!(report.patches_applied, 1);
        assert_eq!(report.sections_added, 1);

        let patched = std::fs::read(&output).unwrap();
        assert_eq!(patched[0x400], 0xE9);
        let rel = i32::from_le_bytes([
            patched[0x401],
            patched[0x402],
            patched[0x403],
            patched[0x404],
        ]);
        let target = PE32_ENTRY_VA as i64 + 5 + rel as i64;
        assert_eq!(target, 0x0040_2000);

        let pe = PeInfo::parse(&patched).unwrap();
        let stub = pe.sections.iter().find(|section| section.name == "stub").unwrap();
        assert_eq!(stub.virtual_address, 0x2000);
        assert!(stub.is_executable());
        assert_eq!(
            &patched[stub.raw_offset as usize..stub.raw_offset as usize + 4],
            &[0x90, 0x90, 0x90, 0xC3]
        );
    }

    #[test]
    fn dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        std::fs::write(&input, pe32()).unwrap();
        let spec = patch_spec(PE32_ENTRY_VA, "55", "90");

        let report = Patcher::new()
            .apply_spec(&input, &output, &spec, None, true)
            .unwrap();
        assert!(report.dry_run);
        assert!(!report.manifest_written);
        assert!(!output.exists());
        assert!(!report.manifest_path.exists());
    }

    #[test]
    fn target_hash_guards_the_input() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        std::fs::write(&input, pe32()).unwrap();
        let mut spec = patch_spec(PE32_ENTRY_VA, "55", "90");
        spec.target_sha256 = Some("0".repeat(64));
        let error = Patcher::new()
            .apply_spec(&input, &output, &spec, None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("SHA-256 mismatch"), "error: {error}");
    }

    #[test]
    fn rejects_a_short_header_and_non_pe() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        std::fs::write(&input, b"not a PE").unwrap();
        let spec = patch_spec(PE32_ENTRY_VA, "55", "90");
        assert!(Patcher::new()
            .apply_spec(&input, &output, &spec, None, false)
            .is_err());
    }

    #[test]
    fn unpatch_restores_exact_bytes_and_removes_a_cave() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        let manifest = dir.path().join("patch.json");
        let original = pe32();
        std::fs::write(&input, &original).unwrap();

        let spec = PatchSpec {
            schema_version: PATCH_SCHEMA_VERSION,
            name: None,
            target_sha256: None,
            allow_signed: false,
            patches: vec![BytePatchSpec {
                address: format!("0x{PE32_ENTRY_VA:X}"),
                expected: "55 8B EC 83 EC".to_string(),
                replace: None,
                jump: Some("section:stub".to_string()),
                note: None,
            }],
            sections: vec![NewSectionSpec {
                name: "stub".to_string(),
                bytes: "CC C3".to_string(),
                note: None,
            }],
        };
        Patcher::new()
            .apply_spec(&input, &output, &spec, Some(&manifest), false)
            .unwrap();
        assert_ne!(std::fs::read(&output).unwrap(), original);

        let report = Patcher::unpatch(&manifest, Some(&output), false).unwrap();
        assert_eq!(report.patches_reverted, 1);
        assert_eq!(report.sections_removed, 1);
        assert_eq!(report.restored_sha256, sha256_hex(&original));
        assert_eq!(std::fs::read(&output).unwrap(), original);

        let repeat = Patcher::unpatch(&manifest, Some(&output), false);
        assert!(repeat.is_err(), "second unpatch must fail safely");
        assert_eq!(std::fs::read(&output).unwrap(), original);
    }

    #[test]
    fn unpatch_rejects_a_changed_output() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        let manifest = dir.path().join("patch.json");
        std::fs::write(&input, pe32()).unwrap();
        let spec = patch_spec(PE32_ENTRY_VA, "55", "90");
        Patcher::new()
            .apply_spec(&input, &output, &spec, Some(&manifest), false)
            .unwrap();

        let mut patched = std::fs::read(&output).unwrap();
        patched[0x400] ^= 0xFF;
        std::fs::write(&output, patched).unwrap();
        let error = Patcher::unpatch(&manifest, Some(&output), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("SHA-256 mismatch"), "error: {error}");
    }

    #[test]
    fn copy_mode_still_writes_identical_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let game = std::env::current_exe().unwrap();
        let output = dir.path().join("patched.bin");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(Patcher::new().apply_patches(&game, &output))
            .unwrap();
        assert_eq!(
            std::fs::read(&output).unwrap(),
            std::fs::read(&game).unwrap()
        );
    }

    #[test]
    fn copy_mode_still_guards_in_place_writes() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game.bin");
        std::fs::copy(std::env::current_exe().unwrap(), &game).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(Patcher::new().apply_patches(&game, &game))
            .unwrap_err()
            .to_string();
        assert!(error.contains("without a matching backup"), "error: {error}");
        crate::backup::create(&game).unwrap();
        runtime
            .block_on(Patcher::new().apply_patches(&game, &game))
            .unwrap();
    }

    #[test]
    fn copy_mode_rejects_directory_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let game = std::env::current_exe().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(Patcher::new().apply_patches(&game, dir.path()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("is a directory"));
    }

    #[test]
    fn rejects_unknown_spec_fields() {
        let result = serde_json::from_str::<PatchSpec>(
            r#"{"schema_version":1,"patches":[],"sections":[],"typo":true}"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn detects_pe_binary() {
        assert_eq!(
            Patcher::new().detect_format(&pe32()).unwrap(),
            BinaryFormat::PE
        );
    }

    #[test]
    fn rejects_mz_without_pe_signature() {
        let mut data = pe32();
        data[0x80..0x84].copy_from_slice(b"NE\0\0");
        assert!(Patcher::new().detect_format(&data).is_err());
    }

    #[test]
    fn detects_elf_binary() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"\x7FELF");
        assert_eq!(
            Patcher::new().detect_format(&data).unwrap(),
            BinaryFormat::ELF
        );
    }

    #[test]
    fn detects_macho_both_endiannesses() {
        let patcher = Patcher::new();
        let mut big_endian = vec![0u8; 4];
        big_endian.copy_from_slice(&0xFEED_FACEu32.to_be_bytes());
        assert_eq!(patcher.detect_format(&big_endian).unwrap(), BinaryFormat::MachO);

        let mut little_endian = vec![0u8; 4];
        little_endian.copy_from_slice(&0xFEED_FACFu32.to_le_bytes());
        assert_eq!(
            patcher.detect_format(&little_endian).unwrap(),
            BinaryFormat::MachO
        );
    }

    #[test]
    fn rejects_unknown_and_tiny_inputs() {
        let patcher = Patcher::new();
        assert!(patcher.detect_format(b"not a binary").is_err());
        assert!(patcher.detect_format(b"").is_err());
        assert!(patcher.detect_format(b"MZ").is_err());
    }

    #[test]
    fn trampoline_encoding_is_correct() {
        let bytes = Patcher::new()
            .create_trampoline(0x1000, 0x2000)
            .unwrap();
        assert_eq!(bytes, vec![0xE9, 0xFB, 0x0F, 0x00, 0x00]);
    }

    #[test]
    fn trampoline_rejects_out_of_range_target() {
        let patcher = Patcher::new();
        assert!(patcher.create_trampoline(0, u32::MAX as u64).is_err());
        assert!(patcher.create_trampoline(u32::MAX as u64, 0).is_err());
    }
    #[test]
    fn patches_pe32_plus_and_roundtrips_a_cave() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game64.exe");
        let output = dir.path().join("patched64.exe");
        let manifest = dir.path().join("patch64.json");
        let entry = 0x0000_0001_4000_0000u64 + 0x1000;
        let original = crate::patcher::test_support::pe64();
        std::fs::write(&input, &original).unwrap();

        let spec = patch_spec(entry, "55", "90");
        Patcher::new()
            .apply_spec(&input, &output, &spec, Some(&manifest), false)
            .unwrap();
        let patched = std::fs::read(&output).unwrap();
        assert_eq!(patched[0x400], 0x90);
        assert!(PeInfo::parse(&patched).unwrap().is_64);

        Patcher::unpatch(&manifest, Some(&output), false).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), original);

        let cave_spec = PatchSpec {
            schema_version: PATCH_SCHEMA_VERSION,
            name: Some("pe64-cave".to_string()),
            target_sha256: None,
            allow_signed: false,
            patches: vec![BytePatchSpec {
                address: format!("0x{entry:X}"),
                expected: "55 8B EC 83 EC".to_string(),
                replace: None,
                jump: Some("section:stub".to_string()),
                note: None,
            }],
            sections: vec![NewSectionSpec {
                name: "stub".to_string(),
                bytes: "CC C3".to_string(),
                note: None,
            }],
        };
        let report = Patcher::new()
            .apply_spec(&input, &output, &cave_spec, Some(&manifest), false)
            .unwrap();
        assert_eq!(report.sections[0].address, 0x0000_0001_4000_2000u64);
        let patched = std::fs::read(&output).unwrap();
        assert_eq!(patched[0x400], 0xE9);
        let rel = i32::from_le_bytes([
            patched[0x401],
            patched[0x402],
            patched[0x403],
            patched[0x404],
        ]);
        assert_eq!(entry as i64 + 5 + rel as i64, 0x0000_0001_4000_2000);
        Patcher::unpatch(&manifest, Some(&output), false).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), original);
    }

    #[test]
    fn stale_backup_does_not_authorize_in_place_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game.exe");
        std::fs::write(&game, pe32()).unwrap();
        crate::backup::create(&game).unwrap();

        let mut changed = std::fs::read(&game).unwrap();
        changed[0x400] = 0x90;
        std::fs::write(&game, &changed).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(Patcher::new().apply_patches(&game, &game))
            .unwrap_err()
            .to_string();
        assert!(error.contains("matching backup"), "error: {error}");

        crate::backup::create(&game).unwrap();
        runtime
            .block_on(Patcher::new().apply_patches(&game, &game))
            .unwrap();
    }

    #[test]
    fn manifest_cannot_alias_the_output_or_input() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        std::fs::write(&input, pe32()).unwrap();
        let spec = patch_spec(PE32_ENTRY_VA, "55", "90");

        let output_alias = dir.path().join("not-created").join("..").join("patched.exe");
        let error = Patcher::new()
            .apply_spec(&input, &output, &spec, Some(&output_alias), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("manifest over"), "error: {error}");
        assert!(!output.exists());

        let input_alias = dir.path().join(".").join("game.exe");
        let error = Patcher::new()
            .apply_spec(&input, &output, &spec, Some(&input_alias), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("manifest over"), "error: {error}");
        assert!(!output.exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_outputs() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let target = dir.path().join("target.exe");
        let link = dir.path().join("link.exe");
        std::fs::write(&input, pe32()).unwrap();
        std::fs::write(&target, pe32()).unwrap();
        symlink(&target, &link).unwrap();
        let spec = patch_spec(PE32_ENTRY_VA, "55", "90");

        let error = Patcher::new()
            .apply_spec(&input, &link, &spec, None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("symbolic link"), "error: {error}");
        assert_eq!(std::fs::read(&link).unwrap(), pe32());
    }

    #[cfg(unix)]
    #[test]
    fn unpatch_preserves_output_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        let manifest = dir.path().join("patch.json");
        let original = pe32();
        std::fs::write(&input, &original).unwrap();
        let spec = patch_spec(PE32_ENTRY_VA, "55", "90");
        Patcher::new()
            .apply_spec(&input, &output, &spec, Some(&manifest), false)
            .unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o750)).unwrap();

        Patcher::unpatch(&manifest, Some(&output), false).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), original);
        let mode = std::fs::metadata(&output).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750);
    }

    #[test]
    fn unpatch_allows_a_moved_patched_copy() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("game.exe");
        let output = dir.path().join("patched.exe");
        let moved = dir.path().join("moved.exe");
        let manifest = dir.path().join("patch.json");
        let original = pe32();
        std::fs::write(&input, &original).unwrap();
        let spec = patch_spec(PE32_ENTRY_VA, "55", "90");
        Patcher::new()
            .apply_spec(&input, &output, &spec, Some(&manifest), false)
            .unwrap();
        std::fs::rename(&output, &moved).unwrap();

        let report = Patcher::unpatch(&manifest, Some(&moved), false).unwrap();
        assert_eq!(report.target, absolute(&moved));
        assert_eq!(std::fs::read(&moved).unwrap(), original);
        assert!(!output.exists());
    }

}
