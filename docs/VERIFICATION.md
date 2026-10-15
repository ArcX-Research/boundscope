# Local verification

Date: 15 October 2026. This records local execution, not hosted CI status.

## Environment

- macOS, Darwin 25.5.0, arm64.
- Rust/Cargo 1.95.0.
- Apple Clang 21.0.0, `clang-2100.1.1.101`.
- Python 3.9.6 for the latest standard-library interface tests; the initial
  implementation review used Python 3.14.7.
- Python 3.13.9 with official MCP client 2.2.0 for client integration.

## Results

| Check | Result |
| --- | --- |
| `cargo fmt --check` | Passed. |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed, no warnings. |
| `cargo test --locked` | 19 tests passed; none skipped. |
| `cargo build --locked` | Both debug executables built. |
| `cargo build --locked --release` | Both optimized executables built. |
| `python3 tests/python/test_interfaces.py` | Five tests passed. |
| Official MCP client test | All seven tools, real finding, evidence/trace inspection, fixed-source retest, pre-report failure trace, and rejection cases passed. |
| `cargo audit --deny warnings` | No known vulnerabilities or warnings reported; no ignored advisory IDs. |
| Manual terminal walkthrough | Finding → fix → retest, baseline preservation, changed-input guard, and failed build passed. |

Rust coverage includes real heap buffer overflow, use-after-free, signed integer
overflow, a valid stdin control, C++ standard-library/header compilation, ordinary
nonzero exits, deadline enforcement, output floods, and failed builds. It also
covers malformed/oversized configuration, traversal, symlink rejection, private
store permissions, snapshot integrity, cancellation, unfinished background work,
and a helper that finishes during process shutdown.

Interface coverage checks CLI exit codes 0/1/2; MCP initialization, discovery,
malformed and oversized messages; execution/read-tool concurrency; busy responses;
cancellation; and client disconnection. Cancellation and disconnection tests
create a real child process, wait until it is running, and verify it is stopped.
Cancelled evidence remains incomplete. Official-client tests verify that an
unregistered command argument cannot alter an execution request.

## Manual evidence

The documented example was copied into a disposable directory. The initial
source wrote just past a four-element allocation. Its valid case completed; its
boundary case produced an AddressSanitizer `heap-buffer-overflow`.

- Baseline: `5c4dad3ea58213f539d79e03d1ee1197`, complete, findings observed.
- Fixed-source retest: `4b3d6291464c82d0cd4106f6b3173e24`, complete, no errors observed.
- Build-error check: `b05edb439c1fbcb53589dec098c27930`, incomplete, zero cases executed.

The retest links to the baseline. The original report was read again and compared
afterward; its contents were unchanged. Changing the boundary argument from `4`
to `2` rejected the retest with exit code 2 and created no new completed report.
Valid source and original case configuration were restored afterward.

Generated review files are archived under `build/agent-review-2026-10-15`, leaving
`build/manual-review` available for the user's fresh walkthrough. These archived
reports remain readable; their recorded execution paths describe the original
location. Starting a new review uses the current paths and a new baseline.

The manual review exposed a transient sanitizer-helper shutdown race. The runner
now allows a bounded 200 ms drain interval within the process deadline. A helper
that finishes naturally does not cause a false incomplete result. Unfinished
background work is still stopped and reported as incomplete. An explicit
regression and the full local suites passed after the change.

## Security and trace review

The follow-up review replaced pathname checks followed by opens with reads
anchored to open directories. Regression tests replace a directory path with a
symlink and confirm that an existing descriptor still reads the original file.
Intermediate and final symlinks are rejected. Exclusive publication tests verify
that an empty destination directory or dangling link cannot be replaced.
Working and final evidence directories are explicitly created with mode 0700.

Protocol tests reject duplicate JSON keys, including duplicate target arguments,
before any execution trace is created. Excessive JSON nesting is rejected too.
Trace tests cover path/ID guards, sequence corruption, size and event limits,
partial final records, and private file/directory permissions. A missing declared
source leaves a failed `snapshot` trace and no report. Active-run MCP tests read
the trace while the child is running, then verify cancellation and saved evidence.

The environment/privacy regression passes synthetic credential and sanitizer
override values to Boundscope. The child confirms that the credential was not
inherited and the fixed sanitizer settings were retained. Distinct markers in
source output, arguments and stdin remain in the private report and are absent
from the trace, along with local paths and case names. These are synthetic
canaries, not customer data or real credentials.

The active-run test exposed a test-harness race: its report glob could select
`.evidence-*/report.json` just before publication moved that directory. The test
now waits for a valid final report ID, matching the application's discovery rule.
The full five-test interface suite passed after that correction.

Dependency audit: cargo-audit 0.22.2 checked 55 lockfile packages with zero known
vulnerabilities and zero warnings. RustSec database revision:
`edb58f5974b83eb8302332821cb6d389777aa351`, updated 14 October 2026.
No advisory IDs or platforms were excluded. Generated audit output is in
`build/dependency-audit.json`. This checks published advisories, not all possible
dependency defects. The new on-demand dependency CI job has not been dispatched.

A separate terminal walkthrough used `build/security-review-2026-10-15`:

- Baseline/report trace: `22fa43e835d6d7d1d9dc204330565b64`; complete, one real
  AddressSanitizer finding and a clean control; CLI exit 1.
- Unknown-target attempt: `787cde208c7c57969e2e6789dc4789e2`; failed at
  `configuration`, three trace events, no report; CLI exit 2.
- Fixed-source retest: `46be2f30dbf66637903e16619b619d4d`; complete, no errors
  observed, linked baseline, 16 trace events; CLI exit 0.

Listing returned three traces and two reports. The original report's SHA-256
remained unchanged. The repository's intentionally faulty example was not edited.
The earlier report `5c4dad3ea58213f539d79e03d1ee1197`, created before trace fields
existed, was also loaded successfully with its snapshot verification intact.

## Limits of this verification

- Linux, other architectures, and the minimum declared Rust version were not
  executed locally. The on-demand CI workflow defines Linux and macOS jobs but
  has not been dispatched. No hosted CI success is claimed.
- The official MCP SDK was tested; no assistant application's connection UI was
  configured or tested.
- Fixtures are synthetic. No production repository, external Make/CMake build,
  OS sandbox, fuzzing campaign, static analyzer, leak detector, or thread detector
  was tested or implemented in this slice.
- Clean fixture results establish observations for those inputs only. They are
  not proof of memory safety or full source/path coverage.
