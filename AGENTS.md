# Working on Boundscope

Read README.md, ROADMAP.md, and docs/DESIGN.md before changing behavior.
Read [SECURITY.md](SECURITY.md) for trust boundaries and
[docs/TRACING.md](docs/TRACING.md) for diagnostic contracts and phase ownership.

- Keep the sanitizer engine shared between CLI and MCP.
- Treat registered programs as local executable code, not sandboxed data.
- Bound configuration, protocol messages, files, process time, and captured output.
- Never turn missing evidence, timeouts, failed builds, or cancellation into a pass.
- Preserve original evidence on retest. Compare only compatible definitions.
- Add meaningful regression tests for parser, execution, and protocol changes.
- Run cargo fmt --check, cargo clippy --locked --all-targets -- -D warnings, and
  cargo test --locked. After MCP changes, exercise the official MCP client too.
- Record tested platform/toolchain and unsupported or untested behavior honestly.
- Keep examples synthetic; keep generated files in target/, build/, or local/.
- Update the README when a user-visible capability or limitation changes.
- Keep trace events free of source, paths, input, arguments, raw logs/errors,
  environment values and client request IDs. Use typed phases and process facts.
- Preserve descriptor-relative file reads and exclusive report publication.
  Document ownership and validity at every unsafe OS call.
- Run `cargo audit --deny warnings` after dependency changes; do not ignore an
  advisory to obtain a pass. Record the advisory database revision and date.

## Where to start

| Task | Entry point and checks |
| --- | --- |
| Connect an agent | [docs/AGENT_SETUP.md](docs/AGENT_SETUP.md) |
| Diagnose a run or pre-report failure | CLI `list-traces` / `trace`; MCP `list_traces` / `get_trace`; [docs/TRACING.md](docs/TRACING.md) |
| Configuration bounds | [src/config.rs](src/config.rs), [tests/engine.rs](tests/engine.rs) |
| File access and publication | [src/files.rs](src/files.rs), [src/report.rs](src/report.rs) |
| Process lifecycle | [src/process.rs](src/process.rs), [tests/python/test_interfaces.py](tests/python/test_interfaces.py) |
| Shared execution | [src/engine.rs](src/engine.rs), [tests/engine.rs](tests/engine.rs) |
| MCP protocol | [src/mcp.rs](src/mcp.rs), [src/json.rs](src/json.rs), [tests/python/test_mcp_client.py](tests/python/test_mcp_client.py) |
| Trace schema and privacy | [src/trace.rs](src/trace.rs), privacy tests in [tests/python/test_interfaces.py](tests/python/test_interfaces.py) |

Check [docs/VERIFICATION.md](docs/VERIFICATION.md) before making coverage claims.
Do not read unrelated local reports or change another agent's registration just
to investigate this repository. Use disposable synthetic fixtures and a dedicated
store. The sample program intentionally contains a bug; preserve that baseline.
