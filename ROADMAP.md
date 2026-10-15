# Boundscope roadmap

## 1. Reproducible sanitizer checks

- [x] Register a small C/C++ program, declared files, fixed inputs, and limits.
- [x] Compile with ASan and UBSan; separate observed findings from incomplete checks.
- [x] Save source/input snapshots, compiler identity, and bounded diagnostics.
- [x] Retest changed source with the same registered definition and link evidence.
- [x] Expose the shared engine through the CLI and MCP, with cancellation.
- [x] Verify fault/control pairs, limits, malformed inputs, and official MCP use.
- [x] Harden local file and protocol boundaries. Give agents and engineers bounded
  execution traces for saved runs and failures before report creation, without
  copying sensitive test payloads into the trace.

Locally verified on macOS arm64. Linux and hosted CI verification remain pending;
see [verification](docs/VERIFICATION.md). This is an initial development version.

## 2. Existing project builds

- [ ] Let a maintainer register a reviewed Make/CMake build recipe and test entry
  point. Record instrumentation and dependency provenance; reject unverifiable
  sanitizer coverage instead of implying a fully instrumented build.
- [ ] Import binary regression inputs without requiring UTF-8 text. Preserve exact
  bytes and offer a reproducible case export.

## 3. Static analysis

- [ ] Run a registered Clang Static Analyzer configuration using a compilation
  database. Save analyzer version, warnings, and source locations. Distinguish
  static warnings from errors reproduced by a test.

## 4. Fuzzing and reproductions

- [ ] Register a libFuzzer harness, initial corpus, and time/memory budgets. Save
  crashing inputs and rerun them as deterministic sanitizer cases.
- [ ] Minimize a reproducing input while preserving the observed diagnostic;
  retain the original and minimized inputs for review.

## 5. Broader checks and team use

- [ ] Add explicit platform capabilities for leak and thread checking. Show
  unsupported combinations separately from clean results.
- [ ] Produce a compact review/export artifact containing input provenance,
  diagnostics, source references, and linked retest results.
- [ ] Add a supported isolation backend for running less trusted test programs.
  Verify filesystem, network, process, and resource restrictions independently.
