# Testing Boundscope

Use small synthetic programs. Boundscope runs registered code with your local user
permissions. These checks need no application accounts or customer data.
For shorter C and C++ fault → fix → retest examples, including affected code and
execution diagrams, start with the [README walkthrough](../README.md#try-a-real-memory-error).

## Automated checks

Build both executables from the repository root:

```sh
cargo build --locked
```

Run the Rust tests. These execute real Clang sanitizer builds; missing compilers
or runtimes cause failures, not skips. Set `BOUNDSCOPE_TEST_CLANG` to an absolute
Clang path if it is not `/usr/bin/clang`.

```sh
cargo test --locked
```

Check CLI exit codes and raw MCP lifecycle behavior:

```sh
python3 tests/python/test_interfaces.py
```

Create an environment for the official MCP client. This client requires Python
3.10 or later. If your system `python3` is older, replace `python3` below with the
path to a newer interpreter (for example `python3.13`):

```sh
python3 -m venv .venv
```

Install the pinned test client:

```sh
.venv/bin/python -m pip install -r tests/python/requirements.txt
```

Exercise all seven tools, execution, evidence/trace inspection, pre-report failure
diagnostics, and fixed-source retesting:

```sh
.venv/bin/python tests/python/test_mcp_client.py
```

Check formatting:

```sh
cargo fmt --check
```

Check compiler and Clippy warnings:

```sh
cargo clippy --locked --all-targets -- -D warnings
```

Audit dependencies using the install command in [SECURITY.md](../SECURITY.md):

```sh
cargo audit --deny warnings
```

## Manual walkthrough

### 1. Prepare a disposable example

Build using the command above. Run the following setup command from the repository
root. It copies the faulty example into `build/manual-review`, writes a target
definition, and prints its location. It does not execute a check or create a
report. It refuses to overwrite an existing review directory.

```sh
python3 - <<'PY'
import json
import shutil
from pathlib import Path
review = Path('build/manual-review')
review.mkdir(parents=True, exist_ok=False)
shutil.copyfile('examples/c/buffer.c', review / 'buffer.c')
config = json.loads(Path('examples/targets.json').read_text())
config['targets'][0]['sources'] = ['buffer.c']
(review / 'targets.json').write_text(json.dumps(config, indent=2) + '\n')
print('Ready:', review.resolve())
print('Target: buffer-demo; cases: within-bounds and boundary.')
print('No test has run yet. Continue with step 2.')
PY
```

Expected: three lines beginning `Ready:`, `Target:`, and `No test has run yet.` If
the directory exists, continue that review or use a new directory. Keep evidence
you still need.

### 2. Review registration

List the target before executing it:

```sh
target/debug/boundscope --config build/manual-review/targets.json --store build/manual-review/reports list-targets
```

Expected: target `buffer-demo`, source `buffer.c`, and cases `within-bounds` with
argument `3` and `boundary` with argument `4`. Check the time and output limits.
This command does not run the target.

### 3. Capture a real finding

Run the registered cases:

```sh
target/debug/boundscope --config build/manual-review/targets.json --store build/manual-review/reports check buffer-demo
```

Expected:

- A brief build/check message appears immediately on stderr.
- JSON on stdout contains a new 32-character report `id`.
- `outcome` is `findings_observed`; `execution` is `complete`.
- `within-bounds` is `no_error_observed`.
- `boundary` is `sanitizer_error`, with `heap-buffer-overflow`.
- Exit code is 1, as expected for the faulty example.

Copy the printed ID. Replace `REPORT_ID` below with that exact value; it is a
placeholder, not a shell variable.

### 4. Inspect the evidence

Open the original report:

```sh
target/debug/boundscope --store build/manual-review/reports report REPORT_ID
```

Expected: compiler version/hash, sanitizer flags/options, source/input hashes,
build output, and the diagnostic in the boundary case's stderr.

Inspect the trace with the same ID:

```sh
target/debug/boundscope --store build/manual-review/reports trace REPORT_ID
```

Expected: `state: report_saved`, matching `report_id`, consecutive event numbers,
compiler/build/case process metrics, and a final `report_saved` event. There are
two zero-based case indices. Source text, paths and inputs appear in the report,
not the trace. Trace completion is not a clean-result claim.

To check a failure before a report exists, request an unregistered target:

```sh
target/debug/boundscope --config build/manual-review/targets.json --store build/manual-review/reports check not-registered
```

Expected: exit code 2, an unknown-target error and a trace ID. This runs no target
code. List attempts:

```sh
target/debug/boundscope --store build/manual-review/reports list-traces
```

Expected: the latest trace is `failed`, with final phase `configuration`, and no
report ID. Read it using `trace TRACE_ID`. Keep using the original successful
report ID for the retest below. A trace from a failed attempt is not a baseline.
`output_truncated` should be false. Evidence files live under
`build/manual-review/reports/REPORT_ID/`; source is in `project/buffer.c`, and
case inputs are in `inputs/`.

### 5. Correct the source

Apply the boundary fix to the disposable copy:

```sh
python3 - <<'PY'
from pathlib import Path
p = Path('build/manual-review/buffer.c')
source = p.read_text()
assert 'index <= count' in source, 'Expected the original faulty example'
p.write_text(source.replace('index <= count', 'index < count'))
print('Fixed the disposable source. The saved baseline was not edited.')
PY
```

### 6. Retest the same definition

Run the same registered cases against changed source:

```sh
target/debug/boundscope --config build/manual-review/targets.json --store build/manual-review/reports retest REPORT_ID
```

Expected: a different report ID, `no_errors_observed`, complete execution, and
exit code 0. `comparison.baseline_report_id` matches the original ID. The boundary
comparison says the earlier diagnostic was not observed in this retest. This is
evidence about these cases, not proof of universal memory safety.

Inspect the original report again using step 4. It must still contain the original
finding and faulty source snapshot.

### 7. Check the comparison guard

Change the boundary input in the disposable configuration:

```sh
python3 - <<'PY'
import json
from pathlib import Path
p = Path('build/manual-review/targets.json')
config = json.loads(p.read_text())
config['targets'][0]['cases'][1]['args'] = ['2']
p.write_text(json.dumps(config, indent=2) + '\n')
print('Changed the boundary input from 4 to 2 for the guard test.')
PY
```

Attempt the original retest again:

```sh
target/debug/boundscope --config build/manual-review/targets.json --store build/manual-review/reports retest REPORT_ID
```

Expected: exit code 2 and `registered definition or compiler changed`. No new
completed report is created. A new input is a new check, not a comparable retest.

Restore the input:

```sh
python3 - <<'PY'
import json
from pathlib import Path
p = Path('build/manual-review/targets.json')
config = json.loads(p.read_text())
config['targets'][0]['cases'][1]['args'] = ['4']
p.write_text(json.dumps(config, indent=2) + '\n')
print('Original input restored.')
PY
```

### 8. Verify MCP access

Configure your client using the README's MCP example, pointing to the disposable
configuration and store with absolute paths. Confirm all seven tools appear. Ask
the assistant to list targets and inspect the original report. Request a retest
and confirm a new linked clean run.

For a reproducible client check without configuring an assistant, run the official
MCP client test above. It creates its own example and exercises the same
fault → fix → retest sequence.

### 9. Check incomplete evidence

Introduce a syntax error in the disposable `buffer.c`, then use step 3's command.
Expect exit code 2, `incomplete`, build diagnostics, and no case executions.
Restore valid source afterward. A syntax error must never become a clean result.

The automated interface tests also run a program that creates a sleeping child.
They cancel its MCP request and separately disconnect the client. Both checks
verify that the descendant stopped and execution is marked cancelled. Rust tests
verify nonzero exits, test timeouts, and output floods as distinct incomplete
outcomes.
