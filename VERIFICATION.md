# Verification status for this archive

This source tree was assembled in a restricted build environment that did not contain `rustc`, `cargo`, or `rustfmt`, and shell-level network access could not install them. The archive therefore distinguishes **static assembly checks performed here** from the **full Rust and behavioral validation a downstream agent must perform**.

Checks performed before packaging:

- every `Cargo.toml` parses as TOML;
- `scripts/agent-e2e.sh` passes `bash -n`;
- `scripts/protocol-probe.py` passes Python bytecode compilation;
- all expected workspace members and source files are present;
- source was manually audited for cross-crate dependency coverage, plugin/control identities, protocol framing, worker lifecycle, discovery precedence, and demo command wiring;
- generated temporary Python bytecode and build artifacts are excluded from the archive.

Not claimed as performed in this environment:

- `cargo check`, `cargo test`, `cargo clippy`, `cargo doc`, or `cargo fmt`;
- compiling/installing the demo host/plugin;
- the live subprocess and worker-protocol E2E run.

`AGENT_TESTING.md` and `scripts/agent-e2e.sh` are the acceptance procedure. A validating agent should format first, then compile/test, install the binaries into isolated Cargo roots, exercise every discovery mode, run typed one-shot calls, prove persistent PID reuse and cancellation, run black-box conformance, run the independent Python protocol probe, and perform the documented failure-injection scenarios. Do not report Outboard as validated until those behavioral checks have been observed.
