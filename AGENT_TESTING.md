# Agent testing instructions

The goal is to prove Outboard as an **installed external-plugin system**, not merely compile its libraries. A validating agent must capture failures with command, exit status, stdout and stderr, and must inspect observable results rather than assuming exit code 0 means correct behavior.

## Baseline

From a clean extraction, record `rustc --version` and `cargo --version`, then run:

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets
cargo doc --workspace --no-deps
./scripts/agent-e2e.sh
```

The E2E script installs the demo plugin and host into temporary Cargo roots and uses a temporary `XDG_CACHE_HOME`. Inspect its output, especially selected executable paths, generated typed argv, returned JSON, worker PID reuse, cancellation error, conformance checks, and independent protocol probe.

## Manual scenarios an agent must additionally perform

### 1. Direct control plane

Run the installed plugin with `__outboard manifest`, `doctor`, `cli-schema`, and `ping`. Verify identity `outboard-demo/engine/echo`, interface `demo.echo@1.0.0`, both `one_shot` and `worker`, progress/cancellation capabilities, healthy doctor output, and `echo`/`delay` in the CLI schema. An unknown control command must fail explicitly.

### 2. Discovery sources and precedence

Prove all of: ordinary `PATH`; `OUTBOARD_DEMO_PLUGIN_PATH` with the plugin absent from `PATH`; explicit `--plugin-dir ... --no-path`; and `--echo-override PATH`. For an override, verify that the override wins even when another valid copy appears earlier elsewhere.

### 3. Shadowing

Place identical plugin binaries in two roots. Run `plugins list --kind engine --inspect --all`, verify exactly one is selected and the other is reported as shadowed. Reverse root order and verify selection reverses.

### 4. Typed one-shot

Run:

```bash
outboard-demo --no-path --plugin-dir "$PLUGIN_ROOT" \
  echo "Hello World" --repeat 2 --case upper \
  --tag alpha --tag beta --prefix '>'
```

Inspect stderr for the typed argv and JSON for `>HELLO WORLD HELLO WORLD`, tags `alpha`/`beta`, and an external process ID. This is the proof that typed Rust -> argv -> external clap parser works.

### 5. Persistent events and PID reuse

Run `worker-echo` and require Started, Progress/Output, and Finished events. Run `worker-batch` and require the explicit assertion that both concurrent requests ran in the same plugin PID. A sequential implementation or two process launches does not pass.

### 6. Cancellation

Run `worker-cancel`. It requests five seconds of work and then cancels. Require the specific `cancelled` worker error; a completed `should-not-finish` result fails the scenario.

### 7. Black-box conformance

Run `outboard-demo conformance /path/to/outboard-demo-engine-echo`. Inspect every check: manifest JSON/validation/identity, interface, doctor JSON, CLI-schema JSON, worker handshake, ping and shutdown. Then run it against `/bin/echo`; it must fail diagnostically rather than panic.

### 8. Independent protocol implementation

Run `python3 scripts/protocol-probe.py PLUGIN`. This intentionally does not use the Rust host implementation. Require valid handshake, ping/pong, a structured `duplicate_hello` error, shutdown acknowledgement, malformed JSON rejection, and oversized-frame rejection. This catches bugs where Rust host and Rust plugin accidentally agree on the same wrong wire encoding.

### 9. Manifest cache

Set a temporary `XDG_CACHE_HOME`, inspect a plugin, and confirm `plugins cache path` exists. Clear it, verify entries disappear, inspect again and verify repopulation. Touch or replace the executable and verify its fingerprint causes safe reinspection rather than stale identity/version data.

### 10. Identity-lie failure injection

In a throwaway source copy, keep the executable filename `outboard-demo-engine-echo` but change its manifest name to `liar`. Install it in an isolated root. `plugins inspect engine:echo` must reject it with an **identity mismatch** naming discovered and declared identities.

### 11. Interface incompatibility

In another throwaway copy, change `demo.echo` from `1.0.0` to `2.0.0`. The host requiring `^1.0` must reject the candidate during resolution and report required vs actual interface versions before application invocation.

### 12. Broken worker declaration

Advertise worker mode but make `__outboard serve` exit immediately. Static manifest inspection should still pass; `plugins doctor engine:echo --deep` and the conformance harness must fail the worker handshake. This proves deep health is behavioral.

### 13. Pipe pressure + timeout

Create a temporary fixture process/plugin that writes more than normal OS pipe capacity to both stdout and stderr and then sleeps beyond the configured timeout. Exercise `run_capture`; require no deadlock, child termination, complete captured output up to termination, and `ProcessError::Timeout`.

### 14. Non-UTF-8 argv (Unix)

Use `OsStringExt::from_vec` with invalid UTF-8 bytes. Send it through `WireOsString` and, if practical, a real worker invocation. Require byte-for-byte equality after round trip; do not use `to_string_lossy` for the assertion. On Windows mark this specific scenario SKIP with platform reason.

## Final agent report

Finish with a PASS/FAIL/SKIP table. Evidence should include selected plugin path, generated argv, worker PID(s), cancellation diagnostic, number of conformance checks, independent-probe result, cache path/invalidation observation, exact identity/interface mismatch messages, and any platform-specific skips. Do not silently omit scenarios.
