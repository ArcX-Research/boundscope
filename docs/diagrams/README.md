# Execution diagrams

The README uses SVG images so the graphs remain readable in Markdown viewers
without a diagram extension. Text labels carry the meaning as well as color.

- [buffer-execution.dot](buffer-execution.dot) combines the trace and report of
  synthetic C run `d4decf72e73733177d1361d62649da03`. It groups preparation phases
  and preserves case order. The dashed edge is a report annotation, not a second
  execution path. It contains no local paths or raw diagnostics.
- [case-outcomes.dot](case-outcomes.dot) shows the case-classification branches in
  [engine.rs](../../src/engine.rs). It is a conceptual map, not path-coverage
  evidence. Case findings are preserved even when execution is incomplete.

These are documentation illustrations. The current CLI returns JSON; it does not
automatically render a graph for each new run. Use `trace` for phase/process facts
and `report` for case names, inputs, findings, and source locations.

The images were rendered with Graphviz 15.0.0 (EPL-2.0). Graphviz is an optional
documentation tool; building and running Boundscope does not require it.
From the repository root, regenerate the observed-execution image:

```sh
dot -Tsvg docs/diagrams/buffer-execution.dot -o docs/diagrams/buffer-execution.svg
```

Regenerate the case-classification image:

```sh
dot -Tsvg docs/diagrams/case-outcomes.dot -o docs/diagrams/case-outcomes.svg
```

Review the rendered images after editing. Keep the conceptual graph aligned with
the order of classification in the engine: termination first, then complete
sanitizer findings, then successful exits without partial diagnostics. The trace
can reach `report_saved` for a clean, faulty, or incomplete execution.
