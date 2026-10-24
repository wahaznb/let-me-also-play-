//! Backup and restore for game binaries.
//!
//! Every backup is content-addressed (SHA-256), recorded in a JSON manifest,
//! and restored atomically. A restore refuses to proceed if the backup no
//! longer matches the checksum recorded when it was created.
//!
//! Layout (next to the original file):
//!   `DeusEx.exe.<hash16>.<timestamp>.bak`       — byte-for-byte copy
//!   `DeusEx.exe.<hash16>.<timestamp>.bak.json`  — manifest
//!
//! Backups are stored beside the original so restoring never depends on a
//! database or a working directory. Restoring uses write-to-temp + rename,
//! so a crash cannot leave a half-written executable behind.

use crate::hash::sha256_hex;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

/// Bump only if the manifest format changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;

/// Manifests larger than this are treated as invalid (prevents pathological reads).
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// A recorded backup of one file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub schema_version: u32,
    /// Absolute, normalized path of the file that was backed up.
    pub original_path: PathBuf,
    /// Absolute path of the backup copy.
    pub backup_path: PathBuf,
    /// Lowercase hex SHA-256 of the backed-up bytes.
    pub sha256: String,
    pub size: u64,
    pub created_unix_secs: u64,
    pub created_unix_nanos: u32,
    pub tool_version: String,
    /// Unix permission bits captured at backup time (None on non-Unix).
    pub unix_mode: Option<u32>,
}

impl BackupRecord {
    /// Path of the JSON manifest that describes this backup.
    pub fn manifest_path(&self) -> PathBuf {
        let mut name = self.backup_path.clone().into_os_string();
        name.push(".json");
        PathBuf::from(name)
    }

    /// Short checksum for display.
    pub fn short_hash(&self) -> &str {
        self.sha256.get(..16).unwrap_or(&self.sha256)
    }
}

/// Result of a restore operation.
#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub target: PathBuf,
    pub record: BackupRecord,
    pub before_hash: Option<String>,
    pub after_hash: String,
    /// True when the target already matched the backup.
    pub already_restored: bool,
    pub dry_run: bool,
}

/// Create a new backup of `path`. Never overwrites an existing backup.
pub fn create(path: &Path) -> Result<BackupRecord> {
    let target = normalize(path)?;
    let metadata = fs::symlink_metadata(&target)
        .with_context(|| format!("Cannot back up {}: file not found", target.display()))?;

    if metadata.file_type().is_symlink() {
        bail!(
            "Refusing to back up {}: it is a symbolic link",
            target.display()
        );
    }
    if !metadata.is_file() {
        bail!(
            "Refusing to back up {}: not a regular file",
            target.display()
        );
    }

    let data = fs::read(&target)
        .with_context(|| format!("Failed to read {}", target.display()))?;
    if data.is_empty() {
        bail!("Refusing to back up {}: file is empty", target.display());
    }

    let digest = sha256_hex(&data);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .context("Target path has no usable file name")?;
    let backup_name = format!(
        "{}.{}.{}.bak",
        file_name,
        &digest[..16],
        now.as_nanos()
    );
    let backup_path = target.with_file_name(backup_name);

    // create_new guarantees we never clobber another backup.
    let mut backup_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup_path)
        .with_context(|| format!("Failed to create backup {}", backup_path.display()))?;
    let write_result = (|| -> Result<()> {
        backup_file.write_all(&data)?;
        backup_file.sync_all()?;
        Ok(())
    })();
    drop(backup_file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&backup_path);
        return Err(error).with_context(|| {
            format!("Failed to write backup {}", backup_path.display())
        });
    }

    // Preserve the original permissions on the backup copy.
    let _ = fs::set_permissions(&backup_path, metadata.permissions());

    #[cfg(unix)]
    let unix_mode = {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode())
    };
    #[cfg(not(unix))]
    let unix_mode = None;

    let record = BackupRecord {
        schema_version: SCHEMA_VERSION,
        original_path: target,
        backup_path,
        sha256: digest,
        size: data.len() as u64,
        created_unix_secs: now.as_secs(),
        created_unix_nanos: now.subsec_nanos(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        unix_mode,
    };

    write_manifest(&record)?;
    Ok(record)
}

/// List backups for `path`, newest first.
pub fn list_backups(path: &Path) -> Result<Vec<BackupRecord>> {
    let target = normalize(path)?;
    let parent = target
        .parent()
        .context("Target path has no parent directory")?;

    let mut records = Vec::new();
    let entries = fs::read_dir(parent)
        .with_context(|| format!("Failed to read directory {}", parent.display()))?;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_string_lossy();
        if !entry_name.ends_with(".bak.json") {
            continue;
        }

        let manifest_path = entry.path();
        let metadata = match fs::symlink_metadata(&manifest_path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
            continue;
        }

        match load_manifest(&manifest_path) {
            Ok(record) if record.original_path == target => records.push(record),
            Ok(_) => {}
            Err(error) => warn!(
                "Skipping unreadable backup manifest {}: {}",
                manifest_path.display(),
                error
            ),
        }
    }

    records.sort_by(|a, b| {
        b.created_unix_secs
            .cmp(&a.created_unix_secs)
            .then(b.created_unix_nanos.cmp(&a.created_unix_nanos))
    });
    Ok(records)
}

/// Verify that the bytes on disk still match the recorded backup.
pub fn verify_record(record: &BackupRecord) -> Result<()> {
    read_verified_backup(record).map(|_| ())
}

/// Restore `target` from a backup.
///
/// With `manifest` unset the newest backup for the target is used.
/// With `dry_run` nothing is written.
pub fn restore(
    target: &Path,
    manifest: Option<&Path>,
    dry_run: bool,
) -> Result<RestoreReport> {
    let target = normalize(target)?;

    let record = match manifest {
        Some(manifest_path) => {
            let record = load_manifest(manifest_path)?;
            validate_record(&record, &target)?;
            record
        }
        None => {
            let mut records = list_backups(&target)?;
            if records.is_empty() {
                bail!(
                    "No backups found for {}. Run `backup --game <file>` first.",
                    target.display()
                );
            }
            records.remove(0)
        }
    };

    // Check the backup before touching the target.
    let bytes = read_verified_backup(&record)?;

    // Refuse to follow symlinks when overwriting.
    match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!(
                    "Refusing to restore over {}: target is a symbolic link",
                    target.display()
                );
            }
            if !metadata.is_file() {
                bail!(
                    "Refusing to restore over {}: target is not a regular file",
                    target.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Cannot inspect {}", target.display()))
        }
    }

    let before_hash = match fs::read(&target) {
        Ok(current) => Some(sha256_hex(&current)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to read {}", target.display()))
        }
    };
    let already_restored = before_hash.as_deref() == Some(record.sha256.as_str());

    if dry_run {
        return Ok(RestoreReport {
            target,
            record,
            before_hash,
            after_hash: sha256_hex(&bytes),
            already_restored,
            dry_run: true,
        });
    }

    if !already_restored {
        write_atomic(&target, &bytes, None)?;
    }

    // Apply the captured permission bits.
    #[cfg(unix)]
    if let Some(mode) = record.unix_mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&target, fs::Permissions::from_mode(mode))
            .with_context(|| format!("Failed to restore permissions on {}", target.display()))?;
    }

    // Read back and verify, so success is never assumed.
    let after_bytes = fs::read(&target)
        .with_context(|| format!("Failed to verify restored {}", target.display()))?;
    let after_hash = sha256_hex(&after_bytes);
    if after_hash != record.sha256 {
        bail!(
            "Post-restore verification failed for {}: expected {}, got {}. \
             The backup itself is intact and can be restored again.",
            target.display(),
            record.sha256,
            after_hash
        );
    }

    Ok(RestoreReport {
        target,
        record,
        before_hash,
        after_hash,
        already_restored,
        dry_run: false,
    })
}

/// Write `bytes` to `path` atomically (temp file + fsync + rename).
///
/// Used for patched outputs as well as restores, so a crash never leaves a
/// truncated executable behind.
pub(crate) fn write_atomic(
    path: &Path,
    bytes: &[u8],
    perms_from: Option<&Path>,
) -> Result<()> {
    let parent = path
        .parent()
        .context("Output path has no parent directory")?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("Output path has no usable file name")?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        file_name,
        std::process::id(),
        stamp
    ));

    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .with_context(|| format!("Failed to create {}", temp_path.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);

        if let Some(source) = perms_from {
            if let Ok(metadata) = fs::metadata(source) {
                let _ = fs::set_permissions(&temp_path, metadata.permissions());
            }
        }

        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path).with_context(|| {
                format!("Failed to replace existing {}", path.display())
            })?;
        }

        fs::rename(&temp_path, path).with_context(|| {
            format!(
                "Failed to move temporary file into place at {}",
                path.display()
            )
        })?;

        // Best-effort directory sync so the rename is durable.
        #[cfg(unix)]
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }

        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn read_verified_backup(record: &BackupRecord) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(&record.backup_path).with_context(|| {
        format!("Backup is missing: {}", record.backup_path.display())
    })?;
    if metadata.file_type().is_symlink() {
        bail!(
            "Backup {} is a symbolic link; refusing to read it",
            record.backup_path.display()
        );
    }
    if !metadata.is_file() {
        bail!(
            "Backup {} is not a regular file",
            record.backup_path.display()
        );
    }
    if metadata.len() != record.size {
        bail!(
            "Backup size mismatch for {}: manifest says {} bytes, file is {} bytes",
            record.backup_path.display(),
            record.size,
            metadata.len()
        );
    }

    let bytes = fs::read(&record.backup_path)
        .with_context(|| format!("Failed to read backup {}", record.backup_path.display()))?;
    let actual = sha256_hex(&bytes);
    if actual != record.sha256 {
        bail!(
            "Backup checksum mismatch for {}: manifest says {}, file is {}. \
             Refusing to restore corrupted data.",
            record.backup_path.display(),
            record.sha256,
            actual
        );
    }
    Ok(bytes)
}

fn validate_record(record: &BackupRecord, target: &Path) -> Result<()> {
    if record.schema_version != SCHEMA_VERSION {
        bail!(
            "Unsupported backup manifest version {} (this build supports {})",
            record.schema_version,
            SCHEMA_VERSION
        );
    }
    if record.original_path != target {
        bail!(
            "This backup is for {}, not {}",
            record.original_path.display(),
            target.display()
        );
    }
    if record.sha256.len() != 64 || !record.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Backup manifest contains an invalid SHA-256 digest");
    }

    let target_parent = target
        .parent()
        .context("Target path has no parent directory")?;
    let backup_parent = record
        .backup_path
        .parent()
        .context("Backup path has no parent directory")?;
    if backup_parent != target_parent {
        bail!(
            "Backup lives outside the target directory ({}); refusing to restore",
            record.backup_path.display()
        );
    }

    let target_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .context("Target path has no usable file name")?;
    let backup_name = record
        .backup_path
        .file_name()
        .and_then(|n| n.to_str())
        .context("Backup path has no usable file name")?;
    if !backup_name.starts_with(target_name) || !backup_name.ends_with(".bak") {
        bail!(
            "Backup file name {:?} does not belong to {:?}; refusing to restore",
            backup_name,
            target_name
        );
    }

    Ok(())
}

fn load_manifest(path: &Path) -> Result<BackupRecord> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Backup manifest not found: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("Backup manifest {} is a symbolic link", path.display());
    }
    if !metadata.is_file() {
        bail!("Backup manifest {} is not a regular file", path.display());
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        bail!("Backup manifest {} is unreasonably large", path.display());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read backup manifest {}", path.display()))?;
    let record: BackupRecord = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse backup manifest {}", path.display()))?;
    Ok(record)
}

fn write_manifest(record: &BackupRecord) -> Result<()> {
    let manifest_path = record.manifest_path();
    let json = serde_json::to_vec_pretty(record)
        .context("Failed to serialize backup manifest")?;
    write_atomic(&manifest_path, &json, None).with_context(|| {
        format!("Failed to write backup manifest {}", manifest_path.display())
    })
}

/// Absolute, parent-normalized path. Does not require the file to exist and
/// does not resolve a final symlink (that is checked separately).
fn normalize(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("Cannot determine current directory")?
            .join(path)
    };
    let parent = absolute
        .parent()
        .context("Path has no parent directory")?;
    let parent = parent
        .canonicalize()
        .with_context(|| format!("Directory not found: {}", parent.display()))?;
    let file_name = absolute
        .file_name()
        .context("Path has no file name")?;
    Ok(parent.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn write_file(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).expect("write test file");
    }

    #[test]
    fn roundtrip_restores_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");
        let original = b"MZ\x90\x00 original game bytes \x01\x02\x03".to_vec();
        write_file(&target, &original);

        let record = create(&target).unwrap();
        assert_eq!(record.size, original.len() as u64);
        assert!(record.backup_path.exists());
        assert!(record.manifest_path().exists());

        write_file(&target, b"corrupted by a bad patch");

        let report = restore(&target, None, false).unwrap();
        assert!(!report.already_restored);
        assert_eq!(fs::read(&target).unwrap(), original);
        assert_eq!(report.after_hash, record.sha256);
    }

    #[test]
    fn restore_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");
        write_file(&target, b"original");

        create(&target).unwrap();
        let report = restore(&target, None, false).unwrap();
        assert!(report.already_restored);
        assert_eq!(fs::read(&target).unwrap(), b"original");
    }

    #[test]
    fn corrupted_backup_is_refused_and_target_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");
        write_file(&target, b"original");
        let record = create(&target).unwrap();

        write_file(&target, b"patched");
        write_file(&record.backup_path, b"tampered backup data");

        let error = restore(&target, None, false).unwrap_err().to_string();
        assert!(error.contains("size mismatch") || error.contains("checksum mismatch"));
        assert_eq!(fs::read(&target).unwrap(), b"patched");
    }

    #[test]
    fn manifest_tamper_detection() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");
        write_file(&target, b"original");
        let record = create(&target).unwrap();

        // Same size, different bytes -> checksum must catch it.
        write_file(&record.backup_path, b"originaL");
        let error = restore(&target, None, false).unwrap_err().to_string();
        assert!(error.contains("checksum mismatch"));
    }

    #[test]
    fn dry_run_does_not_modify_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");
        write_file(&target, b"original");
        create(&target).unwrap();
        write_file(&target, b"patched");

        let report = restore(&target, None, true).unwrap();
        assert!(report.dry_run);
        assert_eq!(fs::read(&target).unwrap(), b"patched");
    }

    #[test]
    fn newest_backup_wins() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");

        write_file(&target, b"version one");
        let first = create(&target).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        write_file(&target, b"version two");
        let second = create(&target).unwrap();

        let records = list_backups(&target).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].sha256, second.sha256);
        assert_eq!(records[1].sha256, first.sha256);

        let report = restore(&target, None, false).unwrap();
        assert_eq!(report.after_hash, second.sha256);
        assert_eq!(fs::read(&target).unwrap(), b"version two");
    }

    #[test]
    fn explicit_manifest_restores_older_version() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");

        write_file(&target, b"version one");
        let first = create(&target).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        write_file(&target, b"version two");
        create(&target).unwrap();

        restore(&target, Some(&first.manifest_path()), false).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"version one");
    }

    #[test]
    fn refuses_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("empty.bin");
        write_file(&target, b"");
        assert!(create(&target).unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn refuses_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("missing.bin");
        assert!(create(&target).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.bin");
        let link = dir.path().join("link.bin");
        write_file(&real, b"data");
        symlink(&real, &link).unwrap();

        let error = create(&link).unwrap_err().to_string();
        assert!(error.contains("symbolic link"));
    }

    #[cfg(unix)]
    #[test]
    fn preserves_permissions_on_restore() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");
        write_file(&target, b"original");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).unwrap();

        create(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        restore(&target, None, false).unwrap();

        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750);
    }

    #[test]
    fn verify_record_catches_missing_backup() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");
        write_file(&target, b"original");
        let record = create(&target).unwrap();
        fs::remove_file(&record.backup_path).unwrap();
        assert!(verify_record(&record).is_err());
    }

    #[test]
    fn atomic_write_replaces_contents() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.bin");
        write_file(&target, b"old-and-longer");
        write_atomic(&target, b"new", None).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn manifest_file_is_valid_json_and_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("game.exe");
        write_file(&target, b"original");
        let record = create(&target).unwrap();

        let mut contents = String::new();
        File::open(record.manifest_path())
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        let parsed: BackupRecord = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
        assert_eq!(parsed.sha256, record.sha256);
    }
}
