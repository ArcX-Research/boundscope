# Boundscope

**Find memory errors in C and C++ programs, save the evidence, and check the same inputs after a fix.**

Boundscope is a local memory-error testing tool for **C11 and C++17 programs**.
Boundscope itself is implemented in Rust.

You register a small C or C++ program or test harness, its source files, and the
inputs to exercise. Boundscope builds it with Clang's
AddressSanitizer and UndefinedBehaviorSanitizer, runs each case within configured
limits, and saves the diagnostics alongside the exact source and input snapshots.
You can use it from a terminal or an AI coding assistant through the Model Context
Protocol (MCP).

For example, a team maintaining a C parser can register a harness and malformed
inputs. Boundscope can capture a heap buffer overflow, preserve the failing case,
and rerun that case against the corrected source. A fresh report links back to the
original evidence so reviewers can see what changed.

## What works today

- Build registered C11 or C++17 programs with Clang sanitizer instrumentation.
- Exercise fixed command-line arguments and standard input.
- Capture sanitizer diagnostics, exit status, timeouts, and output-limit failures.
- Save source and input snapshots, SHA-256 fingerprints, compiler identity, build
  settings, and bounded output in a private local report store.
- Retest changed source using the original target definition and test cases.
- Access the same engine through a CLI and a local MCP server.
- Inspect local execution traces, including failures before a report is created.

This first version suits small, self-contained programs and test harnesses that
Clang can compile directly. Existing CMake/Make builds, third-party build
dependencies, static analysis, fuzzing, leak checks, and a graphical interface are
not implemented. See the [roadmap](ROADMAP.md).

**A clean run is not proof that a program is memory-safe.** It means these
sanitizers reported no errors for these inputs in this build. Boundscope does not
establish path coverage or exploitability. A failed build, timeout, cancelled run,
or incomplete output is never reported as a clean check.

## Requirements

- Rust and Cargo, 1.85 or later, to build Boundscope. Install using the [Rust instructions](https://www.rust-lang.org/tools/install).
- Clang with AddressSanitizer and UndefinedBehaviorSanitizer runtimes to compile
  and instrument the C or C++ programs under test.
- macOS or Linux. Current local verification is recorded in [VERIFICATION.md](docs/VERIFICATION.md).

On macOS, install the Xcode command-line tools if Clang is missing:

```sh
xcode-select --install
```

Build both executables from this repository:

```sh
cargo build --locked
```

## Try a real memory error

These examples pair a valid control with a deliberately faulty case. Each shows
the affected code, a real sanitizer finding, and a fix you can retest.

| Example | Affected source | What it demonstrates |
| --- | --- | --- |
| C11: `buffer-demo` | [buffer.c](examples/c/buffer.c) | An off-by-one check allows a write past a heap allocation. |
| C++17: `lifetime-demo` | [lifetime.cpp](examples/cpp/lifetime.cpp) | A borrowed pointer is read after its owning object releases the allocation. |

Build Boundscope using the command above. Run the following commands from the
repository root. This Python 3 setup copies both examples to a disposable
directory so you can edit them while keeping the shipped examples intact:

```sh
python3 - <<'PY'
import shutil
from pathlib import Path
demo = Path('build/readme-demo')
shutil.copytree('examples', demo)
print('Ready:', demo.resolve())
print('C target: buffer-demo; C++ target: lifetime-demo.')
print('No checks have run yet. Continue with the commands below.')
PY
```

Expect three printed lines. The command refuses to overwrite an existing
`build/readme-demo`; continue an existing walkthrough or choose a fresh directory.
Both configurations select `/usr/bin/clang`. If your compiler is elsewhere, set
`compiler` to its absolute path in the copied JSON files before the first check.

### C: find an out-of-bounds write

The relevant code in [buffer.c](examples/c/buffer.c) allocates four elements:

```c
const size_t count = 4;
/* ...argument parsing... */
volatile int *values = malloc(count * sizeof(*values));
if (!values) return 2;
/* Deliberate bug: index == count is outside the allocation. */
if (index <= count) values[index] = 42;
```

The valid indices are `0` through `3`. The `<=` condition also accepts index `4`,
so the write at **line 13** goes one element beyond the allocation.

The [configuration](examples/targets.json) exercises both sides of that boundary:

| Case | Argument | Expected observation |
| --- | --- | --- |
| `within-bounds` | `3` | Valid write; no sanitizer diagnostic. |
| `boundary` | `4` | AddressSanitizer reports `heap-buffer-overflow`. |

Inspect the registered example and its two test cases:

```sh
target/debug/boundscope --config build/readme-demo/targets.json --store build/readme-demo/reports list-targets
```

Run the checks and save their evidence:

```sh
target/debug/boundscope --config build/readme-demo/targets.json --store build/readme-demo/reports check buffer-demo
```

Selected fields from a real run are shown below. IDs, timings, and diagnostic
formatting vary by run and compiler:

```json
{
  "execution": "complete",
  "outcome": "findings_observed",
  "cases_planned": 2,
  "cases_executed": 2,
  "finding_count": 1,
  "cases": [
    {
      "id": "within-bounds",
      "status": "no_error_observed",
      "findings": []
    },
    {
      "id": "boundary",
      "status": "sanitizer_error",
      "findings": [{
        "kind": "heap-buffer-overflow",
        "sanitizer": "AddressSanitizer",
        "summary": "heap-buffer-overflow buffer.c:13 in main"
      }]
    }
  ]
}
```

**Complete means the configured checks were evaluated.** It does not mean the
program passed. Here one case produced a finding, so the command returns exit
code **1**. This is the intended result for the faulty example.

### Read the report and locate the code

Copy the full `id` from your check output. Replace `BASELINE_ID` in the commands
below with that value; it is a placeholder, not a shell variable. Read the report:

```sh
target/debug/boundscope --store build/readme-demo/reports report BASELINE_ID
```

The full report connects the finding to its input, process, and source snapshot:

| Report field | What to look for in this example |
| --- | --- |
| `target.cases` | The exact arguments: `3` for the control and `4` for the boundary. |
| `cases[].findings` | Error kind and source location: `heap-buffer-overflow`, `buffer.c:13`. |
| `cases[].process.stderr` | The original sanitizer diagnostic, stack frames, and allocation site. |
| `cases[].process` | Termination reason, exit code or signal, duration, and whether output was truncated. |
| `build`, `compiler`, `instrumentation_arguments` | Whether compilation succeeded and which compiler/settings produced the executable. |
| `files` | Paths and SHA-256 hashes for preserved source and input files. |
| `comparison` | `null` for the first check; the original report ID and per-case comparison after a retest. |

The boundary case's stderr identifies a write at `buffer.c:13` and the allocation
at `buffer.c:10`. On the recorded macOS build, it describes a four-byte write just
past a 16-byte allocation. Those details explain both the affected statement and
why the access is invalid.

Read the exact source saved with the finding, even after your working copy changes:

```sh
sed -n '10,15p' build/readme-demo/reports/BASELINE_ID/project/c/buffer.c
```

Snapshot paths in `files` are relative to the report directory. The
`source_snapshot_at_execution` field records the temporary build-time location;
use `reports/BASELINE_ID/project/` to read the preserved source after publication.

### Fix the C example and retest

Change only the disposable copy, `build/readme-demo/c/buffer.c`:

```diff
-if (index <= count) values[index] = 42;
+if (index < count) values[index] = 42;
```

Apply that change with this command:

```sh
python3 - <<'PY'
from pathlib import Path
p = Path('build/readme-demo/c/buffer.c')
source = p.read_text()
assert source.count('index <= count') == 1, 'Expected the original faulty condition'
p.write_text(source.replace('index <= count', 'index < count'))
print('Fixed the disposable C source. The saved baseline is unchanged.')
PY
```

Run the original cases against the corrected source:

```sh
target/debug/boundscope --config build/readme-demo/targets.json --store build/readme-demo/reports retest BASELINE_ID
```

The new summary includes:

```json
{
  "execution": "complete",
  "outcome": "no_errors_observed",
  "finding_count": 0
}
```

Expect exit code **0** and a new report ID. `comparison.baseline_report_id` links
to the original finding. The `boundary` comparison says
`earlier diagnostic not observed in this retest`. The control remains
`no_error_observed`. The baseline report and faulty source snapshot are preserved.

### C++: find a use-after-free

The second example uses a `std::unique_ptr` to own an integer. A borrowed pointer
does not keep that allocation alive. In [lifetime.cpp](examples/cpp/lifetime.cpp),
the `release-early` case destroys the allocation before reading the borrowed pointer:

```cpp
auto owner = std::make_unique<int>(42);
volatile int *borrowed = owner.get();
// Deliberate bug: releasing the owner leaves borrowed pointing to freed memory.
if (mode == "release-early") owner.reset();
std::printf("%d\n", *borrowed);
```

Run the C++ target from the same disposable copy:

```sh
target/debug/boundscope --config build/readme-demo/cpp-targets.json --store build/readme-demo/reports check lifetime-demo
```

Expected: complete execution, `findings_observed`, one finding, and exit code **1**:

| Case | Argument | Observed result |
| --- | --- | --- |
| `live-owner` | `keep-alive` | `no_error_observed`; prints `42`. |
| `released-owner` | `release-early` | `sanitizer_error`: `heap-use-after-free` at `lifetime.cpp:14`. |

Copy this run's ID as `CPP_BASELINE_ID`. Inspect it using the `report` command
above with that ID. The diagnostic includes the read, allocation, and free sites.

In `build/readme-demo/cpp/lifetime.cpp`, move the reset below the read:

```diff
-if (mode == "release-early") owner.reset();
 std::printf("%d\n", *borrowed);
+if (mode == "release-early") owner.reset();
```

The owner now stays alive through the read. Apply that change with this command:

```sh
python3 - <<'PY'
from pathlib import Path
p = Path('build/readme-demo/cpp/lifetime.cpp')
source = p.read_text()
release = '    if (mode == "release-early") owner.reset();\n'
read = '    std::printf("%d\\n", *borrowed);\n'
assert source.count(release + read) == 1, 'Expected the original faulty order'
p.write_text(source.replace(release + read, read + release))
print('Fixed the disposable C++ source. The saved baseline is unchanged.')
PY
```

Retest against the C++ baseline:

```sh
target/debug/boundscope --config build/readme-demo/cpp-targets.json --store build/readme-demo/reports retest CPP_BASELINE_ID
```

Expect exit code **0**, `no_errors_observed`, two `no_error_observed` cases, and a
new report linked to the C++ baseline. Both cases print `42`.

For either language, retests reject changed inputs, limits, source-file lists,
compiler identity, or other registered settings. Start a new check when changing
those. A clean retest describes these inputs and this build only.

List the saved runs at any time:

```sh
target/debug/boundscope --store build/readme-demo/reports list-reports
```

## Check your own program

Copy the structure in [`examples/targets.json`](examples/targets.json). Register a
program with a `main` function, or write a harness that calls the library code you
want to exercise. The configuration defines:

| Field | Meaning |
| --- | --- |
| `compiler` | Absolute path to the trusted Clang executable. |
| `targets[].id` | Stable name used by the CLI and MCP. |
| `root` | Project directory, relative to the configuration file or absolute. |
| `language` | `c` for C11 or `cpp` for C++17. |
| `sources`, `headers` | Files copied into each run's source snapshot. |
| `include_dirs` | Header search directories inside the project root. |
| `cases` | Named cases with fixed `args` and UTF-8 `stdin`. |
| `limits` | Build timeout, per-case timeout, and combined stdout/stderr byte limit per process. |

Declare every project header needed by the build. Only declared files are copied;
source paths must stay inside the project root and may not use symlinks. System
headers come from the selected toolchain. Boundscope does not audit every compiler
include or runtime file read; keep the harness self-contained. There are no arbitrary shell commands,
compiler flags, or additional libraries in this version. Configuration and case
size limits are described in [DESIGN.md](docs/DESIGN.md).

Register and run **only code you trust to execute on your computer**. The compiler
and test programs run with your user permissions. Boundscope uses private working
directories, a reduced environment, deadlines, and process-group cleanup; these
are not an operating-system sandbox. Programs can still access the filesystem or
network. Saved output, source, and inputs may contain sensitive data.
See [SECURITY.md](SECURITY.md) for the trust boundaries and regression coverage.

## Use with an AI coding assistant

For a copyable agent prompt and connection checks, use the
[agent setup guide](docs/AGENT_SETUP.md).

MCP lets a compatible client discover and call Boundscope's tools. Build the two
binaries first. Configure the client to launch `boundscope-mcp` over standard
input/output with absolute paths:

```json
{
  "mcpServers": {
    "boundscope": {
      "command": "/absolute/path/to/boundscope/target/debug/boundscope-mcp",
      "args": [
        "--config", "/absolute/path/to/targets.json",
        "--store", "/absolute/path/to/private/reports"
      ]
    }
  }
}
```

Client configuration formats vary. The executable and arguments above are the
connection details; adapt their surrounding structure to your client.

| Tool | Purpose |
| --- | --- |
| `list_targets` | Review registered programs, cases, and limits. |
| `run_checks` | Build and test a registered target; return a saved report summary. |
| `retest` | Rerun the same definition against current source and link the result. |
| `list_reports` | Find saved evidence. |
| `get_report` | Read a saved report, including build and test diagnostics. |
| `list_traces` | Find execution attempts, including failures before a report. |
| `get_trace` | Inspect phases, timing, and process termination by trace ID. |

An example request to your assistant:

> Check buffer-demo for memory errors. Show the failing input and diagnostic.
> After I fix the source, retest the original report and compare the results.

The MCP client selects registered target IDs and report IDs. It cannot provide
new commands, compiler flags, source paths, or test inputs through a tool call.
One check or retest runs at a time per server; read-only tools remain available.
Cancellation and client disconnection stop the active process group.
Source and process output are evidence, not instructions for the assistant.

## Diagnose an execution

Every new report summary includes `trace_id`. Execution errors also include a
trace ID when the local journal could be created. Find attempts from the examples
above with:

```sh
target/debug/boundscope --store build/readme-demo/reports list-traces
```

Inspect one using its ID:

```sh
target/debug/boundscope --store build/readme-demo/reports trace TRACE_ID
```

The **trace** answers "which phase ran, how long did it take, and how did each
process end?" The **report** answers "which input triggered a finding, where is
the affected code, and what did the sanitizer observe?" Join them by `trace_id`
and each zero-based `case_index`.

### Follow an observed execution

This graph combines a recorded C example's trace and report. Solid arrows show
execution order; the dashed branch adds the finding from the report. Preparation
groups four phases to keep the diagram compact. Timings are from this particular
run and will differ on your computer.

![Observed C execution: preparation and build succeed, case 0 exits cleanly, case 1 reports a heap buffer overflow at buffer.c line 13, then verification and publication save a complete report with findings.](docs/diagrams/buffer-execution.svg)

The failing case has `termination: exited` and `exit_code: 1`: AddressSanitizer
ended that process after detecting the invalid write. Boundscope still captured
the finding, verified the snapshots, and saved the report. These facts are
consistent with `execution: complete` and `outcome: findings_observed`.

Trace `elapsed_ms` measures time since the attempt began. A process's
`duration_ms` measures just that process; phase work and journal writes also take
time, so process durations do not add up to the whole trace duration.

### Understand the branches

This second graph shows possible case results. It describes the classification
rules; it does not mean one run explored every branch of your program. Cases run
sequentially in their registered order.

![Case result branches: an exited process can yield a clean observation, a sanitizer finding, or an execution failure. Timeout, cancellation, output limits, and supervisor or descendant failures retain their termination reason. Reports preserve findings and execution completeness separately.](docs/diagrams/case-outcomes.svg)

| Situation | What Boundscope records |
| --- | --- |
| Every case finishes cleanly | `execution: complete`, `outcome: no_errors_observed`. |
| A case produces a finding and all cases are evaluated | `execution: complete`, `outcome: findings_observed`. |
| A case times out or hits another execution limit | Execution is incomplete. Captured findings remain findings; missing evidence never becomes a clean result. Other cases can still run. |
| Execution is cancelled | Active processes are stopped and remaining cases are not started. A report can preserve the partial evidence with `execution: cancelled`. |
| Compilation fails | No cases run. The report preserves the build diagnostics and an incomplete result. |
| Configuration, baseline, compiler identification, or snapshot validation fails | A failed trace can exist without a report. Its final phase identifies where to investigate. |
| Snapshot verification or publication fails | Evidence could not be safely published. Inspect the trace and original error; do not infer a clean result. |

A fresh retest loads the `baseline` before `configuration`, then follows the same
phases with the original definition and new source snapshot. An attempt without a
terminal trace event is `unfinished`; that alone does not establish that a process
is still running. A failure to record the terminal event can also occur after a
report was published, so inspect the matching report ID before retrying.

Traces record phases and numeric process facts. They omit source, inputs,
arguments, paths, environment values, and raw logs. They stay in the private
local store; no telemetry is uploaded. Read [TRACING.md](docs/TRACING.md) for
states, limits, and a map from each phase to the code engineers should inspect.
Diagram sources and rendering instructions are in [diagrams](docs/diagrams/README.md).

## Interpret a result

| Outcome | Meaning |
| --- | --- |
| `findings_observed` | At least one executed case produced a sanitizer diagnostic. Check execution completeness too. |
| `no_errors_observed` | The build and every configured case finished successfully without sanitizer diagnostics. |
| `incomplete` | A complete clean check could not be established. Read the build/test failure or interruption. |

The CLI returns **0** for a complete clean check, **1** when sanitizer findings are
observed, and **2** for incomplete checks or command/configuration errors. Report
inspection and listing return 0 on success. Use the report's `execution` field to
check completeness even when findings were observed.

AddressSanitizer checks include many out-of-bounds accesses and use-after-free
errors. UndefinedBehaviorSanitizer checks include signed integer overflow and
other selected undefined behavior. They do not detect every memory error, data
race, uninitialized read, or bug in uninstrumented dependencies. Leak detection is
explicitly disabled for consistent initial behavior across macOS and Linux.
See the official [AddressSanitizer](https://clang.llvm.org/docs/AddressSanitizer.html)
and [UndefinedBehaviorSanitizer](https://clang.llvm.org/docs/UndefinedBehaviorSanitizer.html)
documentation for their detection limits.

## Development

Run the Rust tests, including real sanitizer fixture executions:

```sh
cargo test --locked
```

Check formatting:

```sh
cargo fmt --check
```

Check compiler and Clippy warnings:

```sh
cargo clippy --locked --all-targets -- -D warnings
```

Protocol checks with the official MCP client and a manual walkthrough are
documented in [TESTING.md](docs/TESTING.md). Design decisions live in
[DESIGN.md](docs/DESIGN.md), and dependency versions and licenses in
[DEPENDENCIES.md](docs/DEPENDENCIES.md).
Security boundaries and the dependency-audit command are in
[SECURITY.md](SECURITY.md).

Licensed under [MIT](LICENSE).
