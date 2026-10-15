//! Local diagnostic breadcrumbs, deliberately separate from sensitive evidence.
//! SECURITY: events accept typed phases and numeric process facts, never raw
//! errors, paths, inputs, arguments, source, environment values or process logs.
use crate::{
    Result,
    files::Directory,
    process::ProcessResult,
    report::{Report, Store, private_dir, valid_report_id},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const SCHEMA: &str = "boundscope.trace-event.v1";
pub const TRACE_BYTES: usize = 64 * 1024;
const EVENT_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Check,
    Retest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Configuration,
    Baseline,
    Compiler,
    Definition,
    Snapshot,
    Build,
    Case,
    Verification,
    Publication,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessFacts {
    pub termination: Termination,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub duration_ms: u64,
    pub captured_bytes: usize,
    pub output_truncated: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Termination {
    Exited,
    Cancelled,
    TimedOut,
    OutputLimit,
    DescendantsTerminated,
    OutputIncomplete,
    SupervisorError,
}

impl ProcessFacts {
    fn from_process(p: &ProcessResult) -> Result<Self> {
        let termination = match p.termination.as_str() {
            "exited" => Termination::Exited,
            "cancelled" => Termination::Cancelled,
            "timed_out" => Termination::TimedOut,
            "output_limit" => Termination::OutputLimit,
            "descendants_terminated" => Termination::DescendantsTerminated,
            "output_incomplete" => Termination::OutputIncomplete,
            "supervisor_error" => Termination::SupervisorError,
            _ => return Err("unknown process termination; trace not written".into()),
        };
        Ok(Self {
            termination,
            exit_code: p.exit_code,
            signal: p.signal,
            duration_ms: p.duration_ms,
            captured_bytes: p.captured_bytes,
            output_truncated: p.output_truncated,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Started {
        operation: Operation,
        created_unix_ms: u64,
        tool_version: String,
        os: String,
        architecture: String,
    },
    PhaseStarted {
        phase: Phase,
        case_index: Option<usize>,
    },
    ProcessFinished {
        phase: Phase,
        case_index: Option<usize>,
        process: ProcessFacts,
    },
    ReportSaved {
        report_id: String,
    },
    Failed {
        phase: Phase,
        case_index: Option<usize>,
        cancelled: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: String,
    pub trace_id: String,
    pub sequence: usize,
    pub elapsed_ms: u64,
    pub detail: Event,
}

#[derive(Debug, Serialize)]
pub struct TraceView {
    pub schema: &'static str,
    pub trace_id: String,
    pub state: &'static str,
    pub report_id: Option<String>,
    pub incomplete_tail: bool,
    pub events: Vec<Record>,
}

impl TraceView {
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"trace_id":self.trace_id,"state":self.state,"report_id":self.report_id,
            "incomplete_tail":self.incomplete_tail,"first":self.events.first(),"last":self.events.last()})
    }
}

pub struct Trace {
    pub id: String,
    file: File,
    started: Instant,
    sequence: usize,
    bytes: usize,
    phase: Phase,
    case_index: Option<usize>,
}

impl Trace {
    pub fn start(store: &Store, operation: Operation) -> Result<Self> {
        let directory = store.root.join("traces");
        private_dir(&directory)?;
        let mut random = [0u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| e.to_string())?;
        let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let temporary = tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
        let mut trace = Self {
            id,
            file: temporary.as_file().try_clone().map_err(|e| e.to_string())?,
            started: Instant::now(),
            sequence: 0,
            bytes: 0,
            phase: Phase::Configuration,
            case_index: None,
        };
        let created_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis()
            .min(u64::MAX as u128) as u64;
        trace.write(Event::Started {
            operation,
            created_unix_ms,
            tool_version: env!("CARGO_PKG_VERSION").into(),
            os: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
        })?;
        // Readers see a fully written initial record. Never replace another trace.
        temporary
            .persist_noclobber(directory.join(format!("{}.jsonl", trace.id)))
            .map_err(|e| e.to_string())?;
        Ok(trace)
    }

    fn write(&mut self, detail: Event) -> Result<()> {
        let record = Record {
            schema: SCHEMA.into(),
            trace_id: self.id.clone(),
            sequence: self.sequence,
            elapsed_ms: self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            detail,
        };
        let mut bytes = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if self.sequence >= EVENT_LIMIT || self.bytes + bytes.len() > TRACE_BYTES {
            return Err("execution trace exceeds its bounds".into());
        }
        self.file
            .write_all(&bytes)
            .and_then(|()| self.file.sync_all())
            .map_err(|e| format!("saving execution trace: {e}"))?;
        self.bytes += bytes.len();
        self.sequence += 1;
        Ok(())
    }

    pub fn phase(&mut self, phase: Phase, case_index: Option<usize>) -> Result<()> {
        self.phase = phase;
        self.case_index = case_index;
        self.write(Event::PhaseStarted { phase, case_index })
    }

    pub fn process(&mut self, process: &ProcessResult) -> Result<()> {
        self.write(Event::ProcessFinished {
            phase: self.phase,
            case_index: self.case_index,
            process: ProcessFacts::from_process(process)?,
        })
    }

    pub fn saved(&mut self, report: &Report) -> Result<()> {
        if report.id != self.id {
            return Err("trace/report identity mismatch".into());
        }
        self.write(Event::ReportSaved {
            report_id: report.id.clone(),
        })
    }

    pub fn failed(&mut self, cancelled: bool) -> Result<()> {
        self.write(Event::Failed {
            phase: self.phase,
            case_index: self.case_index,
            cancelled,
        })
    }
}

pub fn load(store: &Store, id: &str) -> Result<TraceView> {
    if !valid_report_id(id) {
        return Err("trace ID must be 32 lowercase hexadecimal characters".into());
    }
    let root = Directory::open(&store.root)?.directory(Path::new("traces"))?;
    let bytes = root.read(Path::new(&format!("{id}.jsonl")), TRACE_BYTES)?;
    let complete_len = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let mut events: Vec<Record> = Vec::new();
    let mut state = "unfinished";
    let mut report_id = None;
    for line in bytes[..complete_len].split_inclusive(|b| *b == b'\n') {
        if events.len() >= EVENT_LIMIT || state != "unfinished" {
            return Err("trace has too many records or records after completion".into());
        }
        let record: Record =
            serde_json::from_slice(line).map_err(|e| format!("invalid trace record: {e}"))?;
        if record.schema != SCHEMA
            || record.trace_id != id
            || record.sequence != events.len()
            || events
                .last()
                .is_some_and(|previous| previous.elapsed_ms > record.elapsed_ms)
        {
            return Err("trace identity, sequence or time mismatch".into());
        }
        if matches!(record.detail, Event::Started { .. }) != events.is_empty() {
            return Err("trace must start with exactly one started event".into());
        }
        match &record.detail {
            Event::ReportSaved { report_id: saved } => {
                if saved != id {
                    return Err("trace/report identity mismatch".into());
                }
                state = "report_saved";
                report_id = Some(saved.clone());
            }
            Event::Failed { cancelled, .. } => {
                state = if *cancelled { "cancelled" } else { "failed" }
            }
            _ => (),
        }
        events.push(record);
    }
    if events.is_empty() {
        return Err("trace has no complete start record".into());
    }
    if state != "unfinished" && complete_len != bytes.len() {
        return Err("trace has data after completion".into());
    }
    Ok(TraceView {
        schema: "boundscope.trace.v1",
        trace_id: id.into(),
        state,
        report_id,
        incomplete_tail: complete_len != bytes.len(),
        events,
    })
}

pub fn list(store: &Store, limit: usize) -> Result<Vec<serde_json::Value>> {
    if !(1..=100).contains(&limit) {
        return Err("trace list limit must be 1–100".into());
    }
    let directory = store.root.join("traces");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    // Validate the directory even for empty results; never follow a substituted link.
    Directory::open(&directory)?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".jsonl").filter(|id| valid_report_id(id)) {
            let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if !meta.is_file() {
                return Err("trace entry must be a regular file".into());
            }
            candidates.push((meta.modified().map_err(|e| e.to_string())?, id.to_string()));
            candidates.sort_unstable_by(|a, b| b.cmp(a));
            candidates.truncate(limit);
        }
    }
    candidates
        .into_iter()
        .map(|(_, id)| load(store, &id).map(|t| t.summary()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn trace_reads_handle_interrupted_tail_and_reject_corruption_links_and_oversize() {
        let scratch = tempfile::tempdir().unwrap();
        let store = Store::open(&scratch.path().join("store")).unwrap();
        assert!(list(&store, 20).unwrap().is_empty());
        let mut trace = Trace::start(&store, Operation::Check).unwrap();
        trace.phase(Phase::Snapshot, None).unwrap();
        let path = store
            .root
            .join("traces")
            .join(format!("{}.jsonl", trace.id));
        let original = fs::read(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let mut partial = original.clone();
        partial.extend_from_slice(b"{\"schema\":");
        fs::write(&path, partial).unwrap();
        let interrupted = load(&store, &trace.id).unwrap();
        assert_eq!(interrupted.state, "unfinished");
        assert!(interrupted.incomplete_tail);
        assert_eq!(interrupted.events.len(), 2);
        let corrupted = String::from_utf8(original.clone())
            .unwrap()
            .replace("\"sequence\":1", "\"sequence\":9");
        fs::write(&path, corrupted).unwrap();
        assert!(load(&store, &trace.id).is_err());
        fs::write(&path, vec![b'x'; TRACE_BYTES + 1]).unwrap();
        assert!(load(&store, &trace.id).is_err());
        fs::remove_file(&path).unwrap();
        let outside = scratch.path().join("private.jsonl");
        fs::write(&outside, original).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(load(&store, &trace.id).is_err());
        assert!(list(&store, 20).is_err());
        assert!(load(&store, "../private").is_err());
        assert!(list(&store, 0).is_err());
        assert!(list(&store, 101).is_err());
    }

    #[test]
    fn trace_writer_stops_at_a_fixed_event_bound() {
        let scratch = tempfile::tempdir().unwrap();
        let store = Store::open(&scratch.path().join("store")).unwrap();
        let mut trace = Trace::start(&store, Operation::Check).unwrap();
        for _ in 1..EVENT_LIMIT {
            trace.phase(Phase::Build, None).unwrap();
        }
        assert!(trace.phase(Phase::Build, None).is_err());
        assert_eq!(load(&store, &trace.id).unwrap().events.len(), EVENT_LIMIT);
    }
}
