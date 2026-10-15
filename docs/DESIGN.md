# Design

Boundscope supervises existing compiler diagnostics. Rust owns configuration,
source snapshots, process lifecycle, report persistence, and both interfaces.
Clang's AddressSanitizer and UndefinedBehaviorSanitizer own runtime detection.

The first release compiles self-contained registered programs directly. A target
declares its source files, headers, language, input cases, and execution limits.
The MCP boundary accepts IDs only. Configuration is trusted local configuration;
target source is executable code with the current user's permissions.

## Bounds

- Configuration and incoming MCP messages: at most 1 MiB each.
- At most 16 targets, 16 source files, 64 total declared files, and 8 cases per target.
- Source/header content: at most 2 MiB per file and 8 MiB combined.
- At most 16 arguments per case, 4 KiB each; at most 64 KiB UTF-8 standard input.
- Per-case timeout: 100–30,000 ms. Build timeout: 100–120,000 ms.
- Combined stdout/stderr capture per process: 1 KiB–1 MiB.
- Saved report JSON: at most 64 MiB, including escaped log text.
- Local execution traces: at most 64 events and 64 KiB per attempt.

Counts and sizes are checked before building. Relative project file paths cannot
contain parent traversal or symlinks. The compiler path must be absolute. The
engine supplies instrumentation flags and does not invoke a shell.
Source, report and trace reads walk relative components from owned open directory
descriptors using `openat` with `O_NOFOLLOW` and `O_CLOEXEC`. Each opened component
stays anchored even if its pathname is replaced. Root locations and ancestors
remain trusted local configuration. Files must be regular and are read with a
hard byte bound; special files cannot block the reader.

## Execution and evidence

Each run copies declared source files into a private directory. Compilation and
cases use that snapshot; source edits after snapshotting do not alter the run.
Case input is saved exactly. The runtime environment is deliberately small and
does not inherit application credentials or sanitizer overrides. Leak detection
is disabled; undefined-behavior recovery is disabled.
Each case gets a separate working directory. Sanitizer options and compiler
instrumentation flags are recorded and included in the retest fingerprint.

The supervisor monitors nonblocking stdout/stderr, wall-clock time, cancellation,
and combined output limits. It kills the process group when limits are exceeded
and cleans up remaining members after the main process exits. This is lifecycle
management, not security isolation. Descendants that deliberately escape their
process group cannot be contained by this backend.
If live descendants remain after the main program exits, stopping them makes
that case incomplete. Test programs should wait for their background work.

The report store is owner-only. Reports contain source/input fingerprints,
compiler executable fingerprint/version, invocation settings, bounded logs,
termination reasons, and per-case findings. The final report is published only
after a run finishes. An unfinished temporary directory is not a completed run.
Reports are local evidence, not cryptographically signed attestations; users who
can modify the store can modify reports.

Sanitizer diagnostics are observations from trusted test programs. A program can
print misleading text. Boundscope does not independently prove the truth of a
diagnostic or whether a bug is exploitable. Raw stderr remains available for review.

Retests load and verify a saved report, compare the target definition and compiler
identity, then snapshot current source. They allow source-content changes and
retain the original case inputs and build settings. New reports link to the
baseline; baseline files are never overwritten. A missing diagnostic in a retest
does not prove every manifestation of the underlying bug is fixed.
System headers/libraries and undeclared external file reads are not snapshotted
or attested. This version assumes self-contained harnesses with declared project
files; it does not audit all compiler dependencies or runtime inputs.

Publication uses an exclusive atomic rename: `renamex_np(RENAME_EXCL)` on macOS
and `renameat2(RENAME_NOREPLACE)` on Linux. An existing destination, including an
empty directory or dangling link, causes failure. An unsupported filesystem
does not fall back to a rename that could overwrite evidence.

## Diagnostic traces

The engine creates an append-only local journal before configuration/baseline
validation. A fresh report reuses the trace ID. Typed phase events and numeric
process facts exclude raw errors, paths and test payloads. Calls that fail before
report publication still leave a trace when the store is writable. A missing
terminal event is unfinished evidence, never proof of a running or clean check.
Readers accept complete event prefixes while writing and reject malformed
complete records. The [trace contract](TRACING.md) documents limits, privacy and
failure recovery. No network telemetry or automatic deletion is implemented.

## Interfaces

The CLI and MCP call the same engine. MCP uses newline-delimited JSON-RPC over
stdio, negotiates protocol versions during initialization, and advertises tools
with JSON schemas. Only check/retest execution needs a worker thread. Read-only
calls remain usable during execution; a second execution is rejected as busy.
Cancellation notifications address the active request ID. EOF cancels active work
before shutdown. Protocol output is confined to stdout; diagnostics use stderr.
Duplicate object keys are rejected at every JSON nesting level rather than
silently choosing one tool argument. Discovery includes read-only trace inspection
and instructions to treat source and process output as data, not agent directives.

See the official [MCP lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)
and [stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).
