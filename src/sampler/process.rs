//! Process discovery, `/proc/<pid>/maps` parsing, and PE module mapping.
//!
//! A Wine PE image is mapped as a set of file-backed regions. The mapping with
//! file offset 0 is the image base, so a sampled runtime IP becomes a PE RVA
//! with `ip - runtime_base`. We then add the PE's declared image base to get
//! the address Ghidra uses, which keeps decompile and sample output in the
//! same address space.

use crate::patcher::pe::PeInfo;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// One `/proc/<pid>/maps` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntry {
    pub start: u64,
    pub end: u64,
    pub perms: String,
    pub offset: u64,
    pub dev: String,
    pub inode: u64,
    pub path: Option<PathBuf>,
}

impl MapEntry {
    #[cfg(test)]
    pub fn is_executable(&self) -> bool {
        self.perms.contains('x')
    }
}

/// A running process selected for sampling.
#[derive(Debug, Clone)]
pub struct TargetProcess {
    pub pid: u32,
    pub comm: String,
    pub warning: Option<String>,
}

/// Runtime placement of the game module inside a process.
#[derive(Debug, Clone)]
pub struct RuntimeModule {
    pub path: PathBuf,
    pub base: u64,
    pub size: u64,
    pub image_base: u64,
    pub image_size: u64,
    pub warning: Option<String>,
}

impl RuntimeModule {
    pub fn contains(&self, ip: u64) -> bool {
        ip >= self.base && ip < self.base.saturating_add(self.size)
    }

    pub fn rva(&self, ip: u64) -> Option<u64> {
        self.contains(ip).then(|| ip - self.base)
    }

    /// Convert a runtime IP to the address space used by Ghidra.
    pub fn ghidra_address(&self, ip: u64) -> Option<u64> {
        self.rva(ip)
            .and_then(|rva| self.image_base.checked_add(rva))
    }
}

/// Read and parse `/proc/<pid>/maps`.
pub fn read_maps(pid: u32) -> Result<Vec<MapEntry>> {
    let path = format!("/proc/{pid}/maps");
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {path} (is PID {pid} running?)"))?;
    let mut entries = Vec::new();
    for line in content.lines() {
        if let Some(entry) = parse_maps_line(line) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// Parse one maps line. Malformed lines are ignored by the caller.
fn parse_maps_line(line: &str) -> Option<MapEntry> {
    let mut rest = line;
    let mut fields = [""; 5];
    for field in &mut fields {
        rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }
        let end = rest
            .find(|character: char| character.is_whitespace())
            .unwrap_or(rest.len());
        *field = &rest[..end];
        rest = &rest[end..];
    }

    let (start, end) = fields[0].split_once('-')?;
    let start = u64::from_str_radix(start, 16).ok()?;
    let end = u64::from_str_radix(end, 16).ok()?;
    if end <= start {
        return None;
    }
    let offset = u64::from_str_radix(fields[2], 16).ok()?;
    let inode = fields[4].parse::<u64>().ok()?;
    let path = rest.trim_start();
    let path = if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(unescape_maps_path(path)))
    };

    Some(MapEntry {
        start,
        end,
        perms: fields[1].to_string(),
        offset,
        dev: fields[3].to_string(),
        inode,
        path,
    })
}

/// `/proc/maps` escapes whitespace and backslashes as `\040`, `\011`, etc.
fn unescape_maps_path(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 3 < bytes.len() {
            let octal = &bytes[index + 1..index + 4];
            if octal.iter().all(|byte| (b'0'..=b'7').contains(byte)) {
                let value = (octal[0] - b'0') * 64 + (octal[1] - b'0') * 8 + (octal[2] - b'0');
                out.push(value);
                index += 4;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn path_matches(map_path: &Path, game_path: &Path) -> bool {
    if map_path == game_path {
        return true;
    }
    match (fs::canonicalize(map_path), fs::canonicalize(game_path)) {
        (Ok(map), Ok(game)) => map == game,
        _ => false,
    }
}

fn read_comm(pid: u32) -> Result<String> {
    let path = format!("/proc/{pid}/comm");
    fs::read_to_string(&path)
        .map(|value| value.trim().to_string())
        .with_context(|| format!("Failed to read {path}; PID {pid} may have exited"))
}

/// Choose the process to sample.
///
/// Priority: explicit `pid`, explicit process-name substring, then the unique
/// process that maps `game_path`.
pub fn resolve_target(
    game_path: &Path,
    pid: Option<u32>,
    process: Option<&str>,
) -> Result<TargetProcess> {
    if let Some(pid) = pid {
        if pid == 0 {
            bail!("PID must be greater than zero");
        }
        if pid == std::process::id() {
            bail!("Refusing to sample the game-opt-tool process itself");
        }
        let comm = read_comm(pid)?;
        return Ok(TargetProcess {
            pid,
            comm,
            warning: None,
        });
    }

    if let Some(name) = process {
        let name = name.trim();
        if name.is_empty() {
            bail!("--process must not be empty");
        }
        return find_by_process_name(name);
    }

    find_by_game_mapping(game_path)
}

fn find_by_process_name(name: &str) -> Result<TargetProcess> {
    let needle = name.to_lowercase();
    let mut comm_matches = Vec::new();
    let mut cmdline_matches = Vec::new();

    for pid in proc_pids() {
        if pid == std::process::id() {
            continue;
        }
        if let Ok(comm) = read_comm(pid) {
            if comm.to_lowercase().contains(&needle) {
                comm_matches.push((pid, comm));
                continue;
            }
        }
        if let Ok(raw) = fs::read(format!("/proc/{pid}/cmdline")) {
            let cmdline = raw
                .split(|byte| *byte == 0)
                .map(|part| String::from_utf8_lossy(part))
                .collect::<Vec<_>>()
                .join(" ");
            if cmdline.to_lowercase().contains(&needle) {
                cmdline_matches.push((pid, read_comm(pid).unwrap_or_default()));
            }
        }
    }

    choose_process(comm_matches, cmdline_matches, &format!("process name {name:?}"))
}

fn find_by_game_mapping(game_path: &Path) -> Result<TargetProcess> {
    let game_path = fs::canonicalize(game_path)
        .with_context(|| format!("Failed to resolve game path {}", game_path.display()))?;
    let mut matches = Vec::new();

    for pid in proc_pids() {
        if pid == std::process::id() {
            continue;
        }
        let Ok(entries) = read_maps(pid) else {
            continue;
        };
        let count = entries
            .iter()
            .filter(|entry| {
                entry
                    .path
                    .as_deref()
                    .map(|path| path_matches(path, &game_path))
                    .unwrap_or(false)
            })
            .count();
        if count > 0 {
            let comm = read_comm(pid).unwrap_or_else(|_| "?".to_string());
            matches.push((pid, comm, count));
        }
    }

    if matches.is_empty() {
        bail!(
            "No running process maps {}. Launch the game first, then sample.",
            game_path.display()
        );
    }

    // The game process maps the executable several times (headers, code, data).
    // Wine helper processes do not map it at all, so the largest mapping count
    // is the actual game even when several wrappers share the name.
    matches.sort_by_key(|(pid, _, count)| (std::cmp::Reverse(*count), *pid));
    let (pid, comm, _) = matches.remove(0);
    let warning = if matches.is_empty() {
        None
    } else {
        Some(format!(
            "Multiple processes map the game; selected PID {pid} ({comm}). Others: {}",
            matches
                .iter()
                .map(|(pid, comm, _)| format!("{pid} ({comm})"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    };

    Ok(TargetProcess {
        pid,
        comm,
        warning,
    })
}

fn choose_process(
    comm_matches: Vec<(u32, String)>,
    cmdline_matches: Vec<(u32, String)>,
    description: &str,
) -> Result<TargetProcess> {
    let matches = if comm_matches.is_empty() {
        cmdline_matches
    } else {
        comm_matches
    };

    match matches.len() {
        0 => bail!("No running process matches {description}"),
        1 => {
            let (pid, comm) = matches.into_iter().next().unwrap();
            Ok(TargetProcess {
                pid,
                comm,
                warning: None,
            })
        }
        _ => {
            let list = matches
                .iter()
                .map(|(pid, comm)| format!("{pid} ({comm})"))
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "Multiple processes match {description}: {list}. Pass --pid to choose one."
            )
        }
    }
}

fn proc_pids() -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .collect()
}

/// Find the runtime module and derive the PE image base.
pub fn find_runtime_module(pid: u32, game_path: &Path) -> Result<RuntimeModule> {
    let game_path = fs::canonicalize(game_path)
        .with_context(|| format!("Failed to resolve game path {}", game_path.display()))?;
    let data = fs::read(&game_path)
        .with_context(|| format!("Failed to read game binary {}", game_path.display()))?;
    let pe = PeInfo::parse(&data)
        .with_context(|| format!("Failed to parse PE headers for {}", game_path.display()))?;

    let entries = read_maps(pid)?;
    let matching: Vec<&MapEntry> = entries
        .iter()
        .filter(|entry| {
            entry
                .path
                .as_deref()
                .map(|path| path_matches(path, &game_path))
                .unwrap_or(false)
        })
        .collect();

    if matching.is_empty() {
        bail!(
            "PID {pid} does not map {}. The game may have restarted; pass --pid again.",
            game_path.display()
        );
    }

    let base_candidates: Vec<u64> = matching
        .iter()
        .filter(|entry| entry.offset == 0)
        .map(|entry| entry.start)
        .collect();

    let (base, warning) = if let Some(base) = base_candidates.iter().min() {
        (*base, None)
    } else {
        // Newer Wine builds occasionally omit the header mapping. Reconstruct
        // the base from a section mapping and its file offset.
        let base = derive_base_from_sections(&pe, &matching).with_context(|| {
            format!(
                "Could not find a file-offset-0 mapping for {}; cannot derive its runtime base",
                game_path.display()
            )
        })?;
        (
            base,
            Some(
                "No file-offset-0 mapping was present; runtime base was derived from section mappings."
                    .to_string(),
            ),
        )
    };

    let size = u64::from(pe.size_of_image);
    if size == 0 {
        bail!("PE SizeOfImage is zero");
    }
    base.checked_add(size)
        .with_context(|| "Runtime base + SizeOfImage overflows u64")?;

    Ok(RuntimeModule {
        path: game_path,
        base,
        size,
        image_base: pe.image_base,
        image_size: u64::from(pe.size_of_image),
        warning,
    })
}

fn derive_base_from_sections(pe: &PeInfo, entries: &[&MapEntry]) -> Option<u64> {
    let mut candidates = Vec::new();
    for entry in entries {
        for section in &pe.sections {
            let raw_start = u64::from(section.raw_offset);
            let raw_end = raw_start.checked_add(u64::from(section.raw_size))?;
            if entry.offset < raw_start || entry.offset >= raw_end {
                continue;
            }
            let delta = entry.offset - raw_start;
            if let Some(base) = entry
                .start
                .checked_sub(u64::from(section.virtual_address).checked_add(delta)?)
            {
                if base % 0x1000 == 0 {
                    candidates.push(base);
                }
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patcher::pe::Section;

    #[test]
    fn parses_a_maps_line_with_escaped_spaces() {
        let line = "10900000-10901000 r-xp 00000000 08:04 18391   \
                    /path/to/your/game/DeusEx.exe";
        let entry = parse_maps_line(line).unwrap();
        assert_eq!(entry.start, 0x10900000);
        assert_eq!(entry.end, 0x10901000);
        assert!(entry.is_executable());
        assert_eq!(entry.offset, 0);
        assert_eq!(entry.inode, 18391);
        assert_eq!(
            entry.path.unwrap().to_string_lossy(),
            "/path/to/your/game/DeusEx.exe"
        );
    }

    #[test]
    fn ignores_malformed_maps_lines() {
        assert!(parse_maps_line("not a maps line").is_none());
        assert!(parse_maps_line("2000-1000 r-xp 00000000 00:00 0 /x").is_none());
        assert!(parse_maps_line("1000-2000 r-xp 00000000 00:00 nope /x").is_none());
    }

    #[test]
    fn unescapes_octal_sequences() {
        assert_eq!(unescape_maps_path(r"a\040b\134c"), "a b\\c");
    }

    fn test_pe(sections: Vec<Section>) -> PeInfo {
        PeInfo {
            is_64: false,
            machine: 0x14c,
            image_base: 0x10900000,
            entry_point_rva: 0x214b6,
            size_of_image: 0x66000,
            size_of_headers: 0x1000,
            size_of_code: 0x1000,
            file_alignment: 0x1000,
            section_alignment: 0x1000,
            checksum: 0,
            number_of_sections: sections.len() as u16,
            file_len: 0x50000,
            pe_offset: 0x80,
            optional_header_offset: 0x98,
            optional_header_size: 0xe0,
            section_table_offset: 0x178,
            checksum_offset: 0x0,
            size_of_code_offset: 0x0,
            size_of_image_offset: 0x0,
            number_of_sections_offset: 0x0,
            certificate_directory: (0, 0),
            sections,
        }
    }

    fn section(name: &str, rva: u32, raw_offset: u32, raw_size: u32) -> Section {
        Section {
            name: name.to_string(),
            virtual_address: rva,
            virtual_size: raw_size,
            raw_offset,
            raw_size,
            characteristics: 0x6000_0020,
            header_offset: 0,
        }
    }

    #[test]
    fn derives_the_runtime_base_from_a_section_mapping() {
        let pe = test_pe(vec![section(".text", 0x1000, 0x1000, 0x1000)]);
        // A mapping that starts at the .text section, not the headers.
        let entry = MapEntry {
            start: 0x5000_1000,
            end: 0x5000_2000,
            perms: "r-xp".to_string(),
            offset: 0x1000,
            dev: "00:00".to_string(),
            inode: 1,
            path: Some(PathBuf::from("/game.exe")),
        };
        let entries = [&entry];
        assert_eq!(derive_base_from_sections(&pe, &entries), Some(0x5000_0000));
    }

    #[test]
    fn runtime_module_converts_ip_to_ghidra_address() {
        let module = RuntimeModule {
            path: PathBuf::from("/game.exe"),
            base: 0x5000_0000,
            size: 0x66000,
            image_base: 0x10900000,
            image_size: 0x66000,
            warning: None,
        };
        assert!(module.contains(0x5002_14b6));
        assert_eq!(module.rva(0x5002_14b6), Some(0x214b6));
        assert_eq!(module.ghidra_address(0x5002_14b6), Some(0x109214b6));
        assert_eq!(module.rva(0x5000_70000), None);
    }

    #[test]
    fn process_choice_rejects_ambiguity() {
        let error = choose_process(
            vec![(10, "Game.exe".to_string()), (11, "Game.exe".to_string())],
            Vec::new(),
            "process name \"Game.exe\"",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Multiple processes"));
        assert!(error.contains("--pid"));
    }

    #[test]
    fn process_choice_uses_the_only_match() {
        let target = choose_process(
            vec![(10, "Game.exe".to_string())],
            Vec::new(),
            "process name \"Game.exe\"",
        )
        .unwrap();
        assert_eq!(target.pid, 10);
        assert_eq!(target.comm, "Game.exe");
    }
}
