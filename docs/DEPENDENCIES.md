# Dependencies

The application is MIT-licensed. Cargo.lock pins the complete Rust dependency
graph. These direct versions were verified on 15 October 2026. Cargo downloads
crates; they are not vendored in this repository.

| Dependency | Locked version | License | Purpose |
| --- | --- | --- | --- |
| clap | 4.6.7 | MIT OR Apache-2.0 | CLI parsing and help. |
| ctrlc | 3.5.2 | MIT OR Apache-2.0 | Interrupt/termination handling. |
| libc | 0.2.190 | MIT OR Apache-2.0 | Unix process groups, pipe flags, owner checks. |
| serde | 1.0.229 | MIT OR Apache-2.0 | Typed configuration and reports. |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | JSON configuration, reports, MCP messages. |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | Source, input, and compiler fingerprints. |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Private build/evidence directories. |

The official Python `mcp` client 2.2.0 is a test-only dependency under MIT. It does
not ship in either executable. Other Python tests use the standard library.

`cargo-audit` 0.22.2 (Apache-2.0 OR MIT) is a development-only dependency-audit
tool. It is installed separately and is not linked into Boundscope. The
[security guide](../SECURITY.md) records its install and audit commands.

Clang and its sanitizer runtimes are external tools, not bundled. Tested Apple
Clang identifies itself as version 21.0.0. Upstream LLVM uses Apache-2.0 with LLVM
exceptions; distributions may include additional notices. Consult the
[LLVM license](https://llvm.org/LICENSE.txt) and the selected distribution.

Inspect the full dependency graph and its declared license metadata with:

```sh
cargo metadata --locked --format-version 1
```
