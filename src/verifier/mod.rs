//! Verification module
//!
//! Compares an original and a patched binary.
//!
//! Runtime behavior comparison (frames, input replay, quirk scenarios) is not
//! implemented yet. Until it is, a patched binary that differs from the
//! original is reported as **not verified**, never as passing.

use crate::hash::sha256_hex;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tracing::info;

/// Verification results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub behavioral_match: bool,
    pub launched_successfully: bool,
    pub frame_diff_percent: f64,
    pub gameplay_unchanged: bool,
    pub quirks_preserved: bool,
    pub performance_gain_pct: f64,
    pub details: Vec<String>,
}

/// Verifier for checking optimization correctness
pub struct Verifier {}

impl Verifier {
    pub fn new() -> Self {
        Self {}
    }

    /// Verify a patched game against the original.
    pub async fn verify(
        &self,
        original: &Path,
        patched: &Path,
    ) -> Result<VerificationReport> {
        info!("Starting verification...");
        info!("  Original: {:?}", original);
        info!("  Patched:  {:?}", patched);

        let original_bytes = fs::read(original)
            .with_context(|| format!("Failed to read original {}", original.display()))?;
        let patched_bytes = fs::read(patched)
            .with_context(|| format!("Failed to read patched {}", patched.display()))?;

        let original_hash = sha256_hex(&original_bytes);
        let patched_hash = sha256_hex(&patched_bytes);
        let identical = original_bytes == patched_bytes;

        let mut details = vec![
            format!(
                "Original: {} ({} bytes)",
                original.display(),
                original_bytes.len()
            ),
            format!("  sha256: {}", original_hash),
            format!(
                "Patched:  {} ({} bytes)",
                patched.display(),
                patched_bytes.len()
            ),
            format!("  sha256: {}", patched_hash),
            "File readability check: PASS".to_string(),
        ];

        if identical {
            details.push("Byte comparison: IDENTICAL".to_string());
            details.push(
                "Behavioral equivalence: guaranteed (no bytes were changed)".to_string(),
            );
            details.push("Performance gain: 0.0% (no bytes were changed)".to_string());

            Ok(VerificationReport {
                behavioral_match: true,
                launched_successfully: true,
                frame_diff_percent: 0.0,
                gameplay_unchanged: true,
                quirks_preserved: true,
                performance_gain_pct: 0.0,
                details,
            })
        } else {
            let differing = original_bytes
                .iter()
                .zip(patched_bytes.iter())
                .filter(|(a, b)| a != b)
                .count();
            let length_delta =
                patched_bytes.len() as i64 - original_bytes.len() as i64;

            details.push("Byte comparison: DIFFERENT".to_string());
            details.push(format!(
                "  Differing bytes: {} (length delta: {})",
                differing, length_delta
            ));
            details.push(
                "Behavioral equivalence: NOT VERIFIED — runtime comparison is not implemented"
                    .to_string(),
            );
            details.push(
                "Performance gain: NOT MEASURED — frame-time capture is not implemented"
                    .to_string(),
            );
            details.push(
                "Reporting FAIL so an unverified binary is never marked safe".to_string(),
            );

            Ok(VerificationReport {
                behavioral_match: false,
                launched_successfully: true,
                frame_diff_percent: 0.0,
                gameplay_unchanged: false,
                quirks_preserved: false,
                performance_gain_pct: 0.0,
                details,
            })
        }
    }

    /// Render a verification report as text.
    pub fn generate_report(&self, report: &VerificationReport) -> String {
        let mut output = String::new();
        output.push_str("=== Verification Report ===\n\n");

        output.push_str(&format!("Behavioral Match: {}\n", report.behavioral_match));
        output.push_str(&format!("Frame Difference: {:.4}%\n", report.frame_diff_percent));
        output.push_str(&format!("Quirks Preserved: {}\n", report.quirks_preserved));
        output.push_str(&format!(
            "Performance Gain (measured): {:.1}%\n\n",
            report.performance_gain_pct
        ));

        output.push_str("Details:\n");
        for detail in &report.details {
            output.push_str(&format!("  {}\n", detail));
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    #[tokio::test]
    async fn identical_files_pass() {
        let dir = tempfile::tempdir().unwrap();
        let original = write(dir.path(), "original.exe", b"MZ game bytes");
        let patched = write(dir.path(), "patched.exe", b"MZ game bytes");

        let report = Verifier::new().verify(&original, &patched).await.unwrap();
        assert!(report.behavioral_match);
        assert!(report.quirks_preserved);
        assert_eq!(report.performance_gain_pct, 0.0);
        assert!(report.details.iter().any(|d| d.contains("IDENTICAL")));
    }

    #[tokio::test]
    async fn modified_files_fail_verification() {
        let dir = tempfile::tempdir().unwrap();
        let original = write(dir.path(), "original.exe", b"MZ game bytes");
        let patched = write(dir.path(), "patched.exe", b"MZ patched bytes!");

        let report = Verifier::new().verify(&original, &patched).await.unwrap();
        assert!(!report.behavioral_match);
        assert!(!report.gameplay_unchanged);
        assert!(!report.quirks_preserved);
        assert_eq!(report.performance_gain_pct, 0.0);
        assert!(report.details.iter().any(|d| d.contains("NOT VERIFIED")));
    }

    #[tokio::test]
    async fn missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let original = write(dir.path(), "original.exe", b"bytes");
        let missing = dir.path().join("missing.exe");

        assert!(Verifier::new().verify(&original, &missing).await.is_err());
    }

    #[tokio::test]
    async fn report_contains_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let original = write(dir.path(), "original.exe", b"MZ game bytes");
        let patched = write(dir.path(), "patched.exe", b"MZ game bytes");

        let report = Verifier::new().verify(&original, &patched).await.unwrap();
        let text = Verifier::new().generate_report(&report);
        assert!(text.contains("Behavioral Match: true"));
        assert!(text.contains(&crate::hash::sha256_hex(b"MZ game bytes")));
    }
}
