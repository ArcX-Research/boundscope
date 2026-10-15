use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
};

pub const CONFIG_BYTES: usize = 1024 * 1024;
pub const FILE_BYTES: usize = 2 * 1024 * 1024;
pub const TOTAL_SOURCE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: String,
    pub compiler: PathBuf,
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub id: String,
    pub root: PathBuf,
    pub language: Language,
    pub sources: Vec<PathBuf>,
    #[serde(default)]
    pub headers: Vec<PathBuf>,
    #[serde(default)]
    pub include_dirs: Vec<PathBuf>,
    pub cases: Vec<Case>,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    C,
    Cpp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub stdin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub build_timeout_ms: u64,
    pub test_timeout_ms: u64,
    pub output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            build_timeout_ms: 30_000,
            test_timeout_ms: 5_000,
            output_bytes: 65_536,
        }
    }
}

pub fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}

pub fn relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.as_os_str().len() > 1024
        || path.to_str().is_none_or(|s| s.contains('\0'))
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!(
            "expected a relative project path without traversal: {}",
            path.display()
        ));
    }
    Ok(())
}

pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    crate::files::read_file(file, limit)
}

impl Target {
    pub fn validate(&self) -> Result<()> {
        if !identifier(&self.id) {
            return Err(
                "target ID must use 1–64 letters, digits, dots, underscores or hyphens".into(),
            );
        }
        if self.sources.is_empty()
            || self.sources.len() > 16
            || self.sources.len() + self.headers.len() > 64
        {
            return Err(format!(
                "{} requires 1–16 sources and at most 64 total files",
                self.id
            ));
        }
        if self.include_dirs.len() > 16 {
            return Err("at most 16 include directories are supported".into());
        }
        let mut files = HashSet::new();
        for path in self.sources.iter().chain(&self.headers) {
            relative_path(path)?;
            if !files.insert(path) {
                return Err(format!("duplicate declared file: {}", path.display()));
            }
        }
        for path in &self.include_dirs {
            relative_path(path)?;
        }
        if self.cases.is_empty() || self.cases.len() > 8 {
            return Err("a target requires 1–8 cases".into());
        }
        let mut cases = HashSet::new();
        for case in &self.cases {
            if !identifier(&case.id) || !cases.insert(&case.id) {
                return Err("case IDs must be valid and unique".into());
            }
            if case.args.len() > 16
                || case.args.iter().any(|a| a.len() > 4096 || a.contains('\0'))
                || case.stdin.len() > 65_536
            {
                return Err(format!(
                    "case {} exceeds argument/input limits or contains a NUL argument",
                    case.id
                ));
            }
        }
        if !(100..=30_000).contains(&self.limits.test_timeout_ms)
            || !(100..=120_000).contains(&self.limits.build_timeout_ms)
            || !(1024..=1_048_576).contains(&self.limits.output_bytes)
        {
            return Err("limits require test_timeout_ms 100–30000, build_timeout_ms 100–120000, output_bytes 1024–1048576".into());
        }
        Ok(())
    }
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_bounded(path, CONFIG_BYTES)?;
        let mut catalog: Self = serde_json::from_slice(&bytes)
            .map_err(|e| format!("invalid target configuration: {e}"))?;
        if catalog.schema != "boundscope.targets.v1" {
            return Err("unsupported target configuration schema".into());
        }
        if !catalog.compiler.is_absolute() || !catalog.compiler.is_file() {
            return Err("compiler must be an absolute path to a Clang executable".into());
        }
        if catalog.targets.is_empty() || catalog.targets.len() > 16 {
            return Err("configuration requires 1–16 targets".into());
        }
        let base = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let mut ids = HashSet::new();
        for target in &mut catalog.targets {
            target.validate()?;
            if !ids.insert(target.id.clone()) {
                return Err(format!("duplicate target: {}", target.id));
            }
            target.root = base
                .join(&target.root)
                .canonicalize()
                .map_err(|e| format!("target {} root: {e}", target.id))?;
            if !target.root.is_dir() {
                return Err("target root must be a directory".into());
            }
        }
        Ok(catalog)
    }

    pub fn target(&self, id: &str) -> Result<Target> {
        self.targets
            .iter()
            .find(|t| t.id == id)
            .cloned()
            .ok_or_else(|| format!("unknown registered target: {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_and_identifiers_are_bounded() {
        for path in ["", "../a.c", "/a.c", "a/../../b.c"] {
            assert!(relative_path(Path::new(path)).is_err());
        }
        assert!(relative_path(Path::new("src/a.c")).is_ok());
        for id in ["", "../report", "a b", "-bad"] {
            assert!(!identifier(id));
        }
        assert!(identifier("parser-1"));
    }
    #[test]
    fn malformed_config_and_special_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("targets.json");
        fs::write(&path, b"{\"schema\":").unwrap();
        assert!(Catalog::load(&path).is_err());
        fs::write(&path, vec![b' '; CONFIG_BYTES + 1]).unwrap();
        assert!(Catalog::load(&path).is_err());
        assert!(read_bounded(dir.path(), 1024).is_err());
        std::os::unix::fs::symlink("targets.json", dir.path().join("link")).unwrap();
        assert!(read_bounded(&dir.path().join("link"), CONFIG_BYTES).is_err());
        assert!(
            crate::files::Directory::open(dir.path())
                .unwrap()
                .read(Path::new("link"), CONFIG_BYTES)
                .is_err()
        );
    }
}
