use crate::{
    Result,
    config::Target,
    digest,
    files::{Directory, rename_new},
    process::ProcessResult,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub const REPORT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Compiler {
    pub path: PathBuf,
    pub sha256: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotFile {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub sanitizer: String,
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseResult {
    pub id: String,
    pub status: String,
    pub findings: Vec<Finding>,
    pub process: ProcessResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseComparison {
    pub case_id: String,
    pub earlier: String,
    pub current: String,
    pub observation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comparison {
    pub baseline_report_id: String,
    pub cases: Vec<CaseComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub created_unix_ms: u64,
    pub tool_version: String,
    pub os: String,
    pub architecture: String,
    pub target: Target,
    pub compiler: Compiler,
    pub configuration_sha256: String,
    pub instrumentation_arguments: Vec<String>,
    pub runtime_options: std::collections::BTreeMap<String, String>,
    pub files: Vec<SnapshotFile>,
    pub build_arguments: Vec<String>,
    pub source_snapshot_at_execution: PathBuf,
    pub build: ProcessResult,
    pub cases: Vec<CaseResult>,
    pub outcome: String,
    pub execution: String,
    pub comparison: Option<Comparison>,
    pub limitations: Vec<String>,
}

impl Report {
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id, "target": self.target.id, "created_unix_ms": self.created_unix_ms,
            "trace_id": self.trace_id,
            "outcome": self.outcome, "execution": self.execution,
            "cases_planned": self.target.cases.len(), "cases_executed": self.cases.len(),
            "finding_count": self.cases.iter().map(|c| c.findings.len()).sum::<usize>(),
            "cases": self.cases.iter().map(|c| serde_json::json!({"id": c.id, "status": c.status, "findings": c.findings})).collect::<Vec<_>>(),
            "build_termination": self.build.termination, "build_exit_code": self.build.exit_code,
            "comparison": self.comparison,
            "interpretation": "No errors observed applies only to these inputs and this instrumented build; it is not proof of memory safety."
        })
    }
}

pub fn valid_report_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    // geteuid reads the current effective user ID and has no pointer arguments.
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
    {
        return Err(format!(
            "{} must be a directory owned by you with mode 0700 (no group/other access)",
            path.display()
        ));
    }
    Ok(())
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
}

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        private_dir(path)?;
        Ok(Self {
            root: path.canonicalize().map_err(|e| e.to_string())?,
        })
    }

    pub fn load(&self, id: &str) -> Result<Report> {
        if !valid_report_id(id) {
            return Err("report ID must be 32 lowercase hexadecimal characters".into());
        }
        let dir = Directory::open(&self.root)?.directory(Path::new(id))?;
        let bytes = dir.read(Path::new("report.json"), REPORT_BYTES)?;
        let report: Report =
            serde_json::from_slice(&bytes).map_err(|e| format!("invalid saved report: {e}"))?;
        if report.id != id || report.schema != "boundscope.report.v1" {
            return Err("saved report identity/schema mismatch".into());
        }
        if report.trace_id.as_ref().is_some_and(|trace| trace != id) {
            return Err("saved report trace identity mismatch".into());
        }
        report.target.validate()?;
        let definition = serde_json::to_vec(&(
            &report.target,
            &report.compiler,
            &report.instrumentation_arguments,
            &report.runtime_options,
        ))
        .map_err(|e| e.to_string())?;
        if digest(&definition) != report.configuration_sha256 {
            return Err("saved target definition fingerprint mismatch".into());
        }
        let expected = report
            .target
            .sources
            .iter()
            .chain(&report.target.headers)
            .map(|p| Path::new("project").join(p))
            .chain(
                report
                    .target
                    .cases
                    .iter()
                    .map(|c| Path::new("inputs").join(format!("{}.stdin", c.id))),
            )
            .collect::<Vec<_>>();
        if report.files.len() != expected.len() {
            return Err("saved snapshot file list mismatch".into());
        }
        for (file, expected_path) in report.files.iter().zip(expected) {
            if file.path != expected_path {
                return Err("saved snapshot file order/path mismatch".into());
            }
            let data = dir.read(&file.path, crate::config::FILE_BYTES)?;
            if data.len() != file.bytes || digest(&data) != file.sha256 {
                return Err(format!(
                    "saved evidence fingerprint mismatch: {}",
                    file.path.display()
                ));
            }
        }
        for case in &report.target.cases {
            let data = dir.read(
                &Path::new("inputs").join(format!("{}.stdin", case.id)),
                65_536,
            )?;
            if data != case.stdin.as_bytes() {
                return Err("saved case input differs from its definition".into());
            }
        }
        Ok(report)
    }

    pub fn list(&self, limit: usize) -> Result<Vec<serde_json::Value>> {
        if !(1..=100).contains(&limit) {
            return Err("report list limit must be 1–100".into());
        }
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if valid_report_id(&id) && entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                // Keep only bounded metadata while selecting the newest reports.
                let modified = fs::metadata(entry.path().join("report.json"))
                    .and_then(|m| m.modified())
                    .map_err(|e| format!("saved report {id}: {e}"))?;
                candidates.push((modified, id));
                candidates.sort_unstable_by(|a, b| b.cmp(a));
                candidates.truncate(limit);
            }
        }
        candidates
            .into_iter()
            .map(|(_, id)| self.load(&id).map(|r| r.summary()))
            .collect()
    }

    pub fn publish(&self, staging: tempfile::TempDir, report: &Report) -> Result<()> {
        if !valid_report_id(&report.id) {
            return Err("invalid report identity for publication".into());
        }
        let bytes = serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?;
        if bytes.len() > REPORT_BYTES {
            return Err("report exceeds size limit".into());
        }
        write_private(&staging.path().join("report.json"), &bytes)?;
        let destination = self.root.join(&report.id);
        rename_new(staging.path(), &destination)?;
        // The directory moved; dropping the guard only attempts to remove the old path.
        Ok(())
    }
}

pub fn findings(stderr: &str) -> Vec<Finding> {
    let mut result = Vec::new();
    for sanitizer in ["AddressSanitizer", "UndefinedBehaviorSanitizer"] {
        let marker = format!("SUMMARY: {sanitizer}:");
        let summary = stderr
            .lines()
            .find_map(|line| line.trim().strip_prefix(&marker));
        let evidence = match sanitizer {
            "AddressSanitizer" => stderr.contains("ERROR: AddressSanitizer:"),
            _ => stderr.lines().any(|line| line.contains(": runtime error:")),
        };
        if let Some(summary) = summary.filter(|_| evidence) {
            let kind = if sanitizer == "AddressSanitizer" {
                summary
                    .split_whitespace()
                    .next()
                    .unwrap_or("unknown")
                    .to_string()
            } else {
                stderr
                    .lines()
                    .find_map(|line| {
                        line.split_once(": runtime error:")
                            .map(|(_, description)| description.trim().to_string())
                    })
                    .unwrap_or_else(|| "undefined behavior".into())
            };
            result.push(Finding {
                sanitizer: sanitizer.into(),
                kind: kind.chars().take(1024).collect(),
                summary: summary.trim().chars().take(2048).collect(),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_need_both_error_and_summary() {
        assert!(findings("SUMMARY: AddressSanitizer: heap-buffer-overflow").is_empty());
        assert!(findings("a.c:1:2: runtime error: signed integer overflow").is_empty());
        assert_eq!(findings("==1==ERROR: AddressSanitizer: heap-buffer-overflow\nSUMMARY: AddressSanitizer: heap-buffer-overflow a.c:2")[0].kind, "heap-buffer-overflow");
        assert_eq!(findings("a.c:1:2: runtime error: signed integer overflow\nSUMMARY: UndefinedBehaviorSanitizer: undefined-behavior a.c:1:2")[0].sanitizer, "UndefinedBehaviorSanitizer");
    }
}
