use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::process::{CommandExt, ExitStatusExt},
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessResult {
    pub termination: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub captured_bytes: usize,
    pub output_truncated: bool,
}

struct ChildGroup {
    child: Child,
    cleaned: bool,
}

impl ChildGroup {
    fn has_members(&self) -> bool {
        let pid = self.child.id();
        // Signal 0 queries group existence without sending a signal.
        pid > 1 && pid <= i32::MAX as u32 && unsafe { libc::kill(-(pid as i32), 0) == 0 }
    }

    fn kill_group(&mut self) -> bool {
        if self.cleaned {
            return false;
        }
        self.cleaned = true;
        // Each child starts a new process group whose ID is its PID. Never signal
        // the supervisor's own group. Killing the group also closes inherited pipes.
        let pid = self.child.id();
        if pid > 1 && pid <= i32::MAX as u32 {
            return unsafe { libc::kill(-(pid as i32), libc::SIGKILL) == 0 };
        }
        false
    }
}

impl Drop for ChildGroup {
    fn drop(&mut self) {
        self.kill_group();
        let _ = self.child.wait();
    }
}

fn nonblocking(fd: i32) -> Result<()> {
    // fcntl only changes flags on a live pipe owned by this process.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(())
}

fn drain(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    total: &mut usize,
    limit: usize,
    truncated: &mut bool,
) -> Result<bool> {
    let mut buffer = [0u8; 8192];
    // Bounded work per pipe keeps deadlines and cancellation responsive under a flood.
    for _ in 0..16 {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                let keep = n.min(limit.saturating_sub(*total));
                output.extend_from_slice(&buffer[..keep]);
                *total += keep;
                if keep != n {
                    *truncated = true;
                    return Ok(false);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("reading process output: {e}")),
        }
    }
    Ok(false)
}

pub struct Invocation<'a> {
    pub program: &'a Path,
    pub args: &'a [String],
    pub cwd: &'a Path,
    pub stdin: Option<&'a Path>,
    pub timeout_ms: u64,
    pub output_bytes: usize,
}

pub fn runtime_options() -> std::collections::BTreeMap<String, String> {
    [
        ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin"),
        ("LANG", "C"),
        ("LC_ALL", "C"),
        (
            "ASAN_OPTIONS",
            "halt_on_error=1:abort_on_error=0:detect_leaks=0",
        ),
        ("UBSAN_OPTIONS", "halt_on_error=1:print_stacktrace=1"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

pub fn run(invocation: Invocation<'_>, cancelled: &AtomicBool) -> Result<ProcessResult> {
    let start = Instant::now();
    if cancelled.load(Ordering::Acquire) {
        return Ok(ProcessResult {
            termination: "cancelled".into(),
            exit_code: None,
            signal: None,
            duration_ms: 0,
            stdout: String::new(),
            stderr: String::new(),
            captured_bytes: 0,
            output_truncated: false,
        });
    }
    let mut command = Command::new(invocation.program);
    command
        .args(invocation.args)
        .current_dir(invocation.cwd)
        .env_clear()
        .envs(runtime_options())
        .env("HOME", invocation.cwd.join("home"))
        .env("TMPDIR", invocation.cwd.join("tmp"))
        .stdin(match invocation.stdin {
            Some(path) => Stdio::from(File::open(path).map_err(|e| e.to_string())?),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut group = ChildGroup {
        cleaned: false,
        child: command
            .spawn()
            .map_err(|e| format!("starting {}: {e}", invocation.program.display()))?,
    };
    let mut stdout_pipe = group.child.stdout.take().ok_or("stdout pipe missing")?;
    let mut stderr_pipe = group.child.stderr.take().ok_or("stderr pipe missing")?;
    nonblocking(stdout_pipe.as_raw_fd())?;
    nonblocking(stderr_pipe.as_raw_fd())?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut captured_bytes = 0;
    let mut truncated = false;
    let mut termination = "exited".to_string();
    let status;
    loop {
        drain(
            &mut stdout_pipe,
            &mut stdout,
            &mut captured_bytes,
            invocation.output_bytes,
            &mut truncated,
        )?;
        drain(
            &mut stderr_pipe,
            &mut stderr,
            &mut captured_bytes,
            invocation.output_bytes,
            &mut truncated,
        )?;
        if cancelled.load(Ordering::Acquire) {
            termination = "cancelled".into();
        } else if truncated {
            termination = "output_limit".into();
        } else if start.elapsed() >= Duration::from_millis(invocation.timeout_ms) {
            termination = "timed_out".into();
        }
        if termination != "exited" {
            group.kill_group();
            status = group.child.wait().map_err(|e| e.to_string())?;
            break;
        }
        if let Some(exit) = group.child.try_wait().map_err(|e| e.to_string())? {
            status = exit;
            // Sanitizer symbolizers can exit just after the instrumented process.
            // Allow a bounded drain/reap interval, without extending its deadline.
            let until = (Instant::now() + Duration::from_millis(200))
                .min(start + Duration::from_millis(invocation.timeout_ms));
            loop {
                let out_done = drain(
                    &mut stdout_pipe,
                    &mut stdout,
                    &mut captured_bytes,
                    invocation.output_bytes,
                    &mut truncated,
                )?;
                let err_done = drain(
                    &mut stderr_pipe,
                    &mut stderr,
                    &mut captured_bytes,
                    invocation.output_bytes,
                    &mut truncated,
                )?;
                if truncated {
                    termination = "output_limit".into();
                    group.kill_group();
                    break;
                }
                if cancelled.load(Ordering::Acquire) {
                    termination = "cancelled".into();
                    group.kill_group();
                    break;
                }
                if out_done && err_done && !group.has_members() {
                    group.cleaned = true;
                    break;
                }
                if Instant::now() >= until {
                    if group.kill_group() {
                        termination = "descendants_terminated".into();
                    } else if !out_done || !err_done {
                        termination = "output_incomplete".into();
                    }
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    // Capture the tail after terminating a process, still within the same byte cap.
    if termination != "exited" {
        let _ = drain(
            &mut stdout_pipe,
            &mut stdout,
            &mut captured_bytes,
            invocation.output_bytes,
            &mut truncated,
        )?;
        let _ = drain(
            &mut stderr_pipe,
            &mut stderr,
            &mut captured_bytes,
            invocation.output_bytes,
            &mut truncated,
        )?;
    }
    Ok(ProcessResult {
        termination,
        exit_code: status.code(),
        signal: status.signal(),
        duration_ms: start.elapsed().as_millis().min(u64::MAX as u128) as u64,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        captured_bytes,
        output_truncated: truncated,
    })
}

impl ProcessResult {
    pub fn succeeded(&self) -> bool {
        self.termination == "exited" && self.exit_code == Some(0) && !self.output_truncated
    }
}
