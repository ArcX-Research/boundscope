use crate::{
    Result,
    config::{Catalog, FILE_BYTES, Language, TOTAL_SOURCE_BYTES, Target, identifier},
    digest,
    files::Directory,
    process::{self, Invocation, ProcessResult},
    report::{
        self, CaseComparison, CaseResult, Comparison, Compiler, Report, SnapshotFile, Store,
        private_dir, valid_report_id, write_private,
    },
    trace::{Operation, Phase, Trace},
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct Engine {
    pub config: PathBuf,
    pub store: Store,
}

fn compiler_identity(
    path: &Path,
    work: &Path,
    cancel: &AtomicBool,
    trace: &mut Trace,
) -> Result<Compiler> {
    let args = vec!["--version".into()];
    let version = process::run(
        Invocation {
            program: path,
            args: &args,
            cwd: work,
            stdin: None,
            timeout_ms: 5000,
            output_bytes: 16_384,
        },
        cancel,
    )?;
    trace.process(&version)?;
    if !version.succeeded() || !version.stdout.to_ascii_lowercase().contains("clang") {
        return Err(format!(
            "compiler must successfully identify itself as Clang ({}): {}",
            version.termination, version.stderr
        ));
    }
    let mut file = File::open(path).map_err(|e| format!("compiler: {e}"))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > 512 * 1024 * 1024 {
        return Err("compiler must be a regular file no larger than 512 MiB".into());
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut read = 0usize;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        read += n;
        if read > 512 * 1024 * 1024 {
            return Err("compiler grew beyond size limit".into());
        }
        hasher.update(&buffer[..n]);
    }
    Ok(Compiler {
        path: path.to_path_buf(),
        sha256: format!("{:x}", hasher.finalize()),
        version: version.stdout.trim().into(),
    })
}

fn capture_file(
    staging: &Path,
    path: PathBuf,
    bytes: &[u8],
    files: &mut Vec<SnapshotFile>,
) -> Result<()> {
    let destination = staging.join(&path);
    if let Some(parent) = destination.parent() {
        private_dir(parent)?;
    }
    write_private(&destination, bytes)?;
    files.push(SnapshotFile {
        path,
        sha256: digest(bytes),
        bytes: bytes.len(),
    });
    Ok(())
}

fn execute(invocation: Invocation<'_>, cancel: &AtomicBool) -> ProcessResult {
    process::run(invocation, cancel).unwrap_or_else(|error| ProcessResult {
        termination: "supervisor_error".into(),
        exit_code: None,
        signal: None,
        duration_ms: 0,
        stdout: String::new(),
        stderr: error,
        captured_bytes: 0,
        output_truncated: false,
    })
}

fn instrumentation_arguments(language: Language) -> Vec<String> {
    let (language, standard) = match language {
        Language::C => ("c", "c11"),
        Language::Cpp => ("c++", "c++17"),
    };
    vec![
        "-x".into(),
        language.into(),
        format!("-std={standard}"),
        "-O1".into(),
        "-g".into(),
        "-fno-omit-frame-pointer".into(),
        "-fno-optimize-sibling-calls".into(),
        "-fsanitize=address,undefined".into(),
        "-fno-sanitize-recover=all".into(),
        "-fno-common".into(),
        "-Wall".into(),
        "-Wextra".into(),
        "-Wpedantic".into(),
    ]
}

fn compile_arguments(target: &Target, snapshot: &Path, program: &Path) -> Vec<String> {
    let mut args = instrumentation_arguments(target.language);
    for dir in &target.include_dirs {
        args.push("-I".into());
        args.push(snapshot.join(dir).to_string_lossy().into_owned());
    }
    args.extend(
        target
            .sources
            .iter()
            .map(|source| snapshot.join(source).to_string_lossy().into_owned()),
    );
    if matches!(target.language, Language::Cpp) {
        // clang, unlike clang++, does not add the C++ standard library at link time.
        args.push(
            if cfg!(target_os = "macos") {
                "-lc++"
            } else {
                "-lstdc++"
            }
            .into(),
        );
    }
    args.push("-o".into());
    args.push(program.to_string_lossy().into_owned());
    args
}

impl Engine {
    pub fn new(config: PathBuf, store: PathBuf) -> Result<Self> {
        Ok(Self {
            config,
            store: Store::open(&store)?,
        })
    }
    pub fn list_targets(&self) -> Result<serde_json::Value> {
        let catalog = Catalog::load(&self.config)?;
        Ok(
            serde_json::json!({"targets": catalog.targets, "compiler": catalog.compiler,
            "scope": "Registered C/C++ programs with fixed test inputs. Local execution with your permissions; no OS sandbox."}),
        )
    }

    pub fn check(&self, target_id: &str, cancel: &AtomicBool) -> Result<Report> {
        if !identifier(target_id) {
            return Err(
                "target ID must use 1–64 letters, digits, dots, underscores or hyphens".into(),
            );
        }
        self.traced_run(target_id, None, cancel)
    }

    pub fn retest(&self, report_id: &str, cancel: &AtomicBool) -> Result<Report> {
        if !valid_report_id(report_id) {
            return Err("report ID must be 32 lowercase hexadecimal characters".into());
        }
        self.traced_run("", Some(report_id), cancel)
    }

    fn traced_run(
        &self,
        target_id: &str,
        baseline_id: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Report> {
        let operation = if baseline_id.is_some() {
            Operation::Retest
        } else {
            Operation::Check
        };
        let mut trace = Trace::start(&self.store, operation)?;
        let result: Result<Report> = (|| {
            let baseline = if let Some(id) = baseline_id {
                trace.phase(Phase::Baseline, None)?;
                Some(self.store.load(id)?)
            } else {
                None
            };
            let target_id = baseline
                .as_ref()
                .map_or(target_id, |old| old.target.id.as_str())
                .to_string();
            let report = self.run(&target_id, baseline, cancel, &mut trace)?;
            trace.saved(&report)?;
            Ok(report)
        })();
        match result {
            Ok(report) => Ok(report),
            Err(error) => {
                let saved = trace.failed(cancel.load(Ordering::Acquire));
                let note = if saved.is_ok() {
                    ""
                } else {
                    " (trace could not be finalized)"
                };
                Err(format!(
                    "{error}. Trace ID: {}{note}; inspect with get_trace or the trace command.",
                    trace.id
                ))
            }
        }
    }

    fn run(
        &self,
        target_id: &str,
        baseline: Option<Report>,
        cancel: &AtomicBool,
        trace: &mut Trace,
    ) -> Result<Report> {
        trace.phase(Phase::Configuration, None)?;
        let catalog = Catalog::load(&self.config)?;
        let target = catalog.target(target_id)?;
        let working = tempfile::Builder::new()
            .prefix(".work-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(&self.store.root)
            .map_err(|e| e.to_string())?;
        private_dir(&working.path().join("home"))?;
        private_dir(&working.path().join("tmp"))?;
        trace.phase(Phase::Compiler, None)?;
        let compiler = compiler_identity(&catalog.compiler, working.path(), cancel, trace)?;
        trace.phase(Phase::Definition, None)?;
        let instrumentation_arguments = instrumentation_arguments(target.language);
        let runtime_options = process::runtime_options();
        let configuration_sha256 = digest(
            &serde_json::to_vec(&(
                &target,
                &compiler,
                &instrumentation_arguments,
                &runtime_options,
            ))
            .map_err(|e| e.to_string())?,
        );
        if baseline
            .as_ref()
            .is_some_and(|report| report.configuration_sha256 != configuration_sha256)
        {
            return Err(
                "registered definition or compiler changed; start a new check instead of a retest"
                    .into(),
            );
        }
        let staging = tempfile::Builder::new()
            .prefix(".evidence-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(&self.store.root)
            .map_err(|e| e.to_string())?;
        trace.phase(Phase::Snapshot, None)?;
        let project = Directory::open(&target.root)?;
        let mut files = Vec::new();
        let mut source_bytes = 0usize;
        for relative in target.sources.iter().chain(&target.headers) {
            let bytes = project.read(relative, FILE_BYTES)?;
            source_bytes += bytes.len();
            if source_bytes > TOTAL_SOURCE_BYTES {
                return Err("declared source exceeds 8 MiB combined limit".into());
            }
            capture_file(
                staging.path(),
                Path::new("project").join(relative),
                &bytes,
                &mut files,
            )?;
        }
        for dir in &target.include_dirs {
            project.directory(dir)?;
            private_dir(&staging.path().join("project").join(dir))?;
        }
        for case in &target.cases {
            capture_file(
                staging.path(),
                Path::new("inputs").join(format!("{}.stdin", case.id)),
                case.stdin.as_bytes(),
                &mut files,
            )?;
        }
        let program = working.path().join("program");
        let snapshot = staging.path().join("project");
        let args = compile_arguments(&target, &snapshot, &program);
        trace.phase(Phase::Build, None)?;
        let build = execute(
            Invocation {
                program: &compiler.path,
                args: &args,
                cwd: working.path(),
                stdin: None,
                timeout_ms: target.limits.build_timeout_ms,
                output_bytes: target.limits.output_bytes,
            },
            cancel,
        );
        trace.process(&build)?;
        let mut cases = Vec::new();
        if build.succeeded() {
            for (case_index, case) in target.cases.iter().enumerate() {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                trace.phase(Phase::Case, Some(case_index))?;
                let input = staging
                    .path()
                    .join("inputs")
                    .join(format!("{}.stdin", case.id));
                let case_directory = working.path().join(format!("case-{}", case.id));
                private_dir(&case_directory.join("home"))?;
                private_dir(&case_directory.join("tmp"))?;
                let process = execute(
                    Invocation {
                        program: &program,
                        args: &case.args,
                        cwd: &case_directory,
                        stdin: Some(&input),
                        timeout_ms: target.limits.test_timeout_ms,
                        output_bytes: target.limits.output_bytes,
                    },
                    cancel,
                );
                trace.process(&process)?;
                let findings = report::findings(&process.stderr);
                // Incomplete sanitizer output is still a failure, even if a full finding cannot be extracted.
                let diagnostic_fragment = process.stderr.contains("Sanitizer:")
                    || process.stderr.contains(": runtime error:");
                let status = if process.termination != "exited" {
                    process.termination.clone()
                } else if !findings.is_empty() {
                    "sanitizer_error".into()
                } else if process.succeeded() && !diagnostic_fragment {
                    "no_error_observed".into()
                } else {
                    "execution_failed".into()
                };
                cases.push(CaseResult {
                    id: case.id.clone(),
                    status,
                    findings,
                    process,
                });
            }
        }
        let complete = build.succeeded()
            && cases.len() == target.cases.len()
            && cases.iter().all(|case| {
                matches!(
                    case.status.as_str(),
                    "sanitizer_error" | "no_error_observed"
                )
            });
        let has_findings = cases.iter().any(|case| !case.findings.is_empty());
        let comparison = baseline.as_ref().map(|old| Comparison {
            baseline_report_id: old.id.clone(),
            cases: target
                .cases
                .iter()
                .map(|planned| {
                    let earlier = old.cases.iter().find(|c| c.id == planned.id);
                    let current = cases.iter().find(|c| c.id == planned.id);
                    let before = earlier.map_or("not_executed", |c| c.status.as_str());
                    let after = current.map_or("not_executed", |c| c.status.as_str());
                    let observation = match (before, after) {
                        ("sanitizer_error", "no_error_observed") => {
                            "earlier diagnostic not observed in this retest"
                        }
                        ("no_error_observed", "no_error_observed") => {
                            "no diagnostic observed in either execution"
                        }
                        ("no_error_observed", "sanitizer_error") => {
                            "new sanitizer diagnostic observed"
                        }
                        ("sanitizer_error", "sanitizer_error") => {
                            "sanitizer diagnostics remain; compare the saved logs"
                        }
                        _ => "incomplete evidence; no clean comparison established",
                    };
                    CaseComparison {
                        case_id: planned.id.clone(),
                        earlier: before.into(),
                        current: after.into(),
                        observation: observation.into(),
                    }
                })
                .collect(),
        });
        let cancelled = cancel.load(Ordering::Acquire)
            || build.termination == "cancelled"
            || cases.iter().any(|c| c.status == "cancelled");
        let report = Report {
            schema: "boundscope.report.v1".into(), id: trace.id.clone(), trace_id: Some(trace.id.clone()),
            created_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_millis().min(u64::MAX as u128) as u64,
            tool_version: env!("CARGO_PKG_VERSION").into(), os: std::env::consts::OS.into(), architecture: std::env::consts::ARCH.into(),
            target, compiler, configuration_sha256, instrumentation_arguments, runtime_options, files, build_arguments: args, source_snapshot_at_execution: snapshot,
            build, cases,
            outcome: if has_findings { "findings_observed" } else if complete && !cancelled { "no_errors_observed" } else { "incomplete" }.into(),
            execution: if cancelled { "cancelled" } else if complete { "complete" } else { "incomplete" }.into(),
            comparison,
            limitations: vec!["Only declared inputs and instrumented code were exercised; no path-coverage claim.".into(),
                "Leak detection, thread checking, fuzzing and static analysis were not performed.".into(),
                "Local execution uses your permissions; process limits are not an OS sandbox.".into(),
                "Compiler fingerprint identifies the driver, not every system header, library or toolchain component.".into(),
                "Logs are size-bounded and decoded as UTF-8 with replacement for invalid bytes.".into()],
        };
        // Test programs are trusted, but never silently preserve modified snapshots as original evidence.
        trace.phase(Phase::Verification, None)?;
        let evidence = Directory::open(staging.path())?;
        for file in &report.files {
            let bytes = evidence.read(&file.path, FILE_BYTES)?;
            if digest(&bytes) != file.sha256 {
                return Err(
                    "source/input snapshot changed during execution; evidence was not published"
                        .into(),
                );
            }
        }
        trace.phase(Phase::Publication, None)?;
        self.store.publish(staging, &report)?;
        Ok(report)
    }
}
