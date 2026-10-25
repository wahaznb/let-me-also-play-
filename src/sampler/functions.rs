//! Function maps for per-function sampling.
//!
//! Ghidra's function list is the authoritative source for function starts and
//! sizes. Running headless Ghidra re-analyzes the binary each time (~20-30 s),
//! so the list is cached by canonical path + file size + mtime. The cache is an
//! optimization only: a miss or unreadable cache falls back to Ghidra.

use crate::decompiler::{Decompiler, FunctionSummary};
use crate::game::GameInfo;
use crate::hash::sha256_hex;
use crate::patcher::pe::PeInfo;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tracing::{info, warn};

const CACHE_SCHEMA_VERSION: u32 = 1;

/// One function range in both runtime-PE and Ghidra address spaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSpan {
    pub address: u64,
    pub rva: u64,
    pub size: u64,
    pub name: String,
}

/// A PE-relative function index derived from Ghidra's function list.
#[derive(Debug, Clone)]
pub struct FunctionMap {
    spans: Vec<FunctionSpan>,
    max_end: Vec<u64>,
}

impl FunctionMap {
    /// Build a function map from Ghidra summaries.
    ///
    /// Functions outside `[image_base, image_base + size_of_image)` are
    /// discarded. Zero-sized functions borrow the distance to the next
    /// function, clamped to the image.
    pub fn from_summaries(pe: &PeInfo, functions: Vec<FunctionSummary>) -> Self {
        let image_base = pe.image_base;
        let image_size = u64::from(pe.size_of_image);
        let image_end = image_base.saturating_add(image_size);

        let mut spans: Vec<FunctionSpan> = functions
            .into_iter()
            .filter(|function| function.address >= image_base && function.address < image_end)
            .map(|function| FunctionSpan {
                address: function.address,
                rva: function.address - image_base,
                size: function.size,
                name: function.name,
            })
            .collect();

        spans.sort_by_key(|span| span.rva);
        spans.dedup_by_key(|span| span.rva);

        for index in 0..spans.len() {
            let remaining = image_size.saturating_sub(spans[index].rva).max(1);
            if spans[index].size == 0 {
                let next = spans
                    .get(index + 1)
                    .map(|next| next.rva)
                    .unwrap_or(spans[index].rva.saturating_add(1));
                spans[index].size = next.saturating_sub(spans[index].rva).max(1);
            }
            spans[index].size = spans[index].size.min(remaining);
        }

        let mut max_end = Vec::with_capacity(spans.len());
        let mut running = 0u64;
        for span in &spans {
            running = running.max(span.rva.saturating_add(span.size));
            max_end.push(running);
        }

        Self { spans, max_end }
    }

    pub fn spans(&self) -> &[FunctionSpan] {
        &self.spans
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Find the most specific function containing `rva`.
    ///
    /// The prefix-max index makes this correct even when Ghidra reports
    /// overlapping function bodies (thunks and shared tails).
    pub fn find_index(&self, rva: u64) -> Option<usize> {
        let upper = self.spans.partition_point(|span| span.rva <= rva);
        if upper == 0 {
            return None;
        }

        // First index whose prefix-max end is greater than rva.
        let mut low = 0usize;
        let mut high = upper;
        while low < high {
            let mid = low + (high - low) / 2;
            if self.max_end[mid] > rva {
                high = mid;
            } else {
                low = mid + 1;
            }
        }

        // Walk backwards so the function with the latest start wins when
        // Ghidra reports overlapping bodies.
        for index in (low..upper).rev() {
            let span = &self.spans[index];
            if span.rva <= rva && rva < span.rva.saturating_add(span.size) {
                return Some(index);
            }
        }
        None
    }

    #[cfg(test)]
    pub fn find(&self, rva: u64) -> Option<&FunctionSpan> {
        self.find_index(rva).map(|index| &self.spans[index])
    }
}

/// Stable identity for a cached function list.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheIdentity {
    path: PathBuf,
    file_size: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

impl CacheIdentity {
    fn from_path(game_path: &Path) -> Result<Self> {
        let path = fs::canonicalize(game_path)
            .with_context(|| format!("Failed to resolve game path {}", game_path.display()))?;
        let metadata = fs::metadata(&path)
            .with_context(|| format!("Failed to stat game binary {}", path.display()))?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .unwrap_or_default();
        Ok(Self {
            path,
            file_size: metadata.len(),
            modified_secs: modified.as_secs(),
            modified_nanos: modified.subsec_nanos(),
        })
    }

    fn cache_key(&self) -> String {
        sha256_hex(
            format!(
                "{}\0{}\0{}\0{}",
                self.path.display(),
                self.file_size,
                self.modified_secs,
                self.modified_nanos
            )
            .as_bytes(),
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedFunctionList {
    schema_version: u32,
    game_path: String,
    file_size: u64,
    modified_secs: u64,
    modified_nanos: u32,
    functions: Vec<FunctionSummary>,
}

impl CachedFunctionList {
    fn matches(&self, identity: &CacheIdentity) -> bool {
        self.schema_version == CACHE_SCHEMA_VERSION
            && self.game_path == identity.path.to_string_lossy().as_ref()
            && self.file_size == identity.file_size
            && self.modified_secs == identity.modified_secs
            && self.modified_nanos == identity.modified_nanos
    }
}

fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("game-opt-tool")
        .join("functions")
}

fn read_cache(dir: &Path, identity: &CacheIdentity) -> Option<Vec<FunctionSummary>> {
    let path = dir.join(format!("{}.json", identity.cache_key()));
    let data = fs::read_to_string(path).ok()?;
    let cached: CachedFunctionList = serde_json::from_str(&data).ok()?;
    cached.matches(identity).then_some(cached.functions)
}

fn write_cache(
    dir: &Path,
    identity: &CacheIdentity,
    functions: &[FunctionSummary],
) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create function cache {}", dir.display()))?;
    let key = identity.cache_key();
    let path = dir.join(format!("{key}.json"));
    let temporary = dir.join(format!(".{key}.{}.tmp", uuid::Uuid::new_v4()));
    let cached = CachedFunctionList {
        schema_version: CACHE_SCHEMA_VERSION,
        game_path: identity.path.to_string_lossy().to_string(),
        file_size: identity.file_size,
        modified_secs: identity.modified_secs,
        modified_nanos: identity.modified_nanos,
        functions: functions.to_vec(),
    };
    let json = serde_json::to_vec(&cached).context("Failed to serialize function cache")?;
    fs::write(&temporary, json)
        .with_context(|| format!("Failed to write function cache {}", temporary.display()))?;
    fs::rename(&temporary, &path)
        .with_context(|| format!("Failed to publish function cache {}", path.display()))?;
    Ok(())
}

/// Load a function map, using the on-disk cache unless `refresh` is true.
///
/// The first call for a binary runs headless Ghidra; subsequent calls are a
/// small JSON read.
pub async fn load_function_map(game_path: &Path, refresh: bool) -> Result<FunctionMap> {
    let identity = CacheIdentity::from_path(game_path)?;
    let directory = cache_dir();

    let mut functions = if refresh {
        None
    } else {
        read_cache(&directory, &identity)
    };

    if functions.is_none() {
        info!(
            "Building function map with Ghidra (first run for this binary; this can take ~30s)"
        );
        let game = GameInfo::from_path(&identity.path)?;
        let decompiler = Decompiler::new()?;
        let listed = decompiler.list_functions(&game).await?;
        if let Err(error) = write_cache(&directory, &identity, &listed) {
            warn!("Function-map cache is not writable: {error:#}");
        }
        functions = Some(listed);
    }

    let data = fs::read(&identity.path)
        .with_context(|| format!("Failed to read game binary {}", identity.path.display()))?;
    let pe = PeInfo::parse(&data)
        .with_context(|| format!("Failed to parse PE headers for {}", identity.path.display()))?;
    let map = FunctionMap::from_summaries(&pe, functions.unwrap_or_default());
    if map.is_empty() {
        warn!("Ghidra returned no functions inside the PE image");
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patcher::pe::PeInfo;
    use tempfile::tempdir;

    fn test_pe() -> PeInfo {
        PeInfo {
            is_64: false,
            machine: 0x14c,
            image_base: 0x1000,
            entry_point_rva: 0x1000,
            size_of_image: 0x100,
            size_of_headers: 0x200,
            size_of_code: 0x80,
            file_alignment: 0x200,
            section_alignment: 0x1000,
            checksum: 0,
            number_of_sections: 0,
            file_len: 0x200,
            pe_offset: 0x80,
            optional_header_offset: 0x98,
            optional_header_size: 0xe0,
            section_table_offset: 0x178,
            checksum_offset: 0,
            size_of_code_offset: 0,
            size_of_image_offset: 0,
            number_of_sections_offset: 0,
            certificate_directory: (0, 0),
            sections: Vec::new(),
        }
    }

    fn summary(address: u64, size: u64, name: &str) -> FunctionSummary {
        FunctionSummary {
            address,
            name: name.to_string(),
            size,
        }
    }

    #[test]
    fn maps_boundaries_and_gaps() {
        let pe = test_pe();
        let map = FunctionMap::from_summaries(
            &pe,
            vec![
                summary(0x1000, 0x10, "first"),
                summary(0x1010, 0x20, "second"),
                summary(0x1040, 0x10, "third"),
                // Outside the image, must be dropped.
                summary(0x2000, 0x10, "outside"),
            ],
        );

        assert_eq!(map.find(0x0).unwrap().name, "first");
        assert_eq!(map.find(0xf).unwrap().name, "first");
        assert_eq!(map.find(0x10).unwrap().name, "second");
        assert_eq!(map.find(0x2f).unwrap().name, "second");
        assert!(map.find(0x30).is_none());
        assert_eq!(map.find(0x40).unwrap().name, "third");
        assert!(map.find(0x50).is_none());
        assert_eq!(map.spans().len(), 3);
    }

    #[test]
    fn zero_sized_functions_extend_to_the_next_start() {
        let map = FunctionMap::from_summaries(
            &test_pe(),
            vec![summary(0x1000, 0, "zero"), summary(0x1020, 0x10, "next")],
        );
        assert_eq!(map.find(0x1f).unwrap().name, "zero");
        assert_eq!(map.find(0x20).unwrap().name, "next");
    }

    #[test]
    fn overlapping_functions_choose_the_containing_span() {
        let map = FunctionMap::from_summaries(
            &test_pe(),
            vec![
                summary(0x1000, 0x80, "large"),
                summary(0x1010, 0x10, "inner"),
            ],
        );
        // The inner function starts later and is more specific.
        assert_eq!(map.find(0x15).unwrap().name, "inner");
        assert_eq!(map.find(0x40).unwrap().name, "large");
    }

    #[test]
    fn cache_round_trips_and_validates_identity() {
        let dir = tempdir().unwrap();
        let game = dir.path().join("Game.exe");
        fs::write(&game, b"fake").unwrap();
        let identity = CacheIdentity::from_path(&game).unwrap();
        let cache = dir.path().join("cache");
        let functions = vec![summary(0x1000, 0x10, "cached")];

        write_cache(&cache, &identity, &functions).unwrap();
        assert_eq!(read_cache(&cache, &identity).unwrap(), functions);

        // A changed file size must not reuse the old list.
        fs::write(&game, b"fake but longer").unwrap();
        let changed = CacheIdentity::from_path(&game).unwrap();
        assert!(read_cache(&cache, &changed).is_none());
    }

    #[test]
    fn corrupted_cache_is_ignored() {
        let dir = tempdir().unwrap();
        let game = dir.path().join("Game.exe");
        fs::write(&game, b"fake").unwrap();
        let identity = CacheIdentity::from_path(&game).unwrap();
        let cache = dir.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        fs::write(
            cache.join(format!("{}.json", identity.cache_key())),
            b"not json",
        )
        .unwrap();
        assert!(read_cache(&cache, &identity).is_none());
    }
}
