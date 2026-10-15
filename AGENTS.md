# Working on Boundscope

Read [README.md](README.md) and [ROADMAP.md](ROADMAP.md) before changing behavior.
Use the source and tests for implementation details.

- Keep the sanitizer engine shared between CLI and MCP.
- Treat registered programs as local executable code, not sandboxed data.
- Bound configuration, protocol messages, files, process time, and captured output.
- Never turn missing evidence, timeouts, failed builds, or cancellation into a pass.
- Preserve original evidence on retest. Compare only compatible definitions.
- Keep trace events free of source, paths, input, arguments, raw logs/errors,
  environment values and client request IDs. Use typed phases and process facts.
- Preserve descriptor-relative file reads and exclusive report publication.
  Document ownership and validity at every unsafe OS call.
- Add meaningful regression tests for parser, execution, and protocol changes.
- Run `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`,
  and `cargo test --locked`. After CLI or MCP changes, run
  `python3 tests/python/test_interfaces.py`. After MCP changes, also run
  `tests/python/test_mcp_client.py` with the official client from
  `tests/python/requirements.txt` installed.
- Run `cargo audit --deny warnings` after dependency changes; do not ignore an
  advisory to obtain a pass. Record the advisory database revision and date.
- Report the tested platform, toolchain, results, and untested behavior honestly.
  Local checks do not establish hosted CI status.
- Keep examples synthetic and preserve their deliberate faults. Use disposable
  copies and a dedicated store; do not inspect unrelated local reports.
- Keep generated run data in target/, build/, or local/.
- Keep user instructions in the README, with working examples and expected results.
  Use "AI agent". Avoid documentation that restates the code.
- Keep reviewed README images in assets/. Keep image-authoring tools out of the
  project and its setup requirements.

## Code entry points

| Area | Source | Checks |
| --- | --- | --- |
| Configuration and execution | [config.rs](src/config.rs), [engine.rs](src/engine.rs) | [engine tests](tests/engine.rs) |
| File access and evidence | [files.rs](src/files.rs), [report.rs](src/report.rs) | Unit tests and engine tests |
| Process lifecycle | [process.rs](src/process.rs) | Engine and [interface tests](tests/python/test_interfaces.py) |
| MCP and JSON | [mcp.rs](src/mcp.rs), [json.rs](src/json.rs) | Unit, interface, and [official client tests](tests/python/test_mcp_client.py) |
| Trace schema and privacy | [trace.rs](src/trace.rs) | Unit, engine, and interface tests |
