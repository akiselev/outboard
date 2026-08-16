# Outboard

Outboard is a Rust framework for **Cargo-style external executable plugins**. It discovers independently installed binaries, interrogates versioned manifests, resolves compatible implementations, launches them with typed clap-derived argv, and optionally keeps expensive plugins alive as concurrent persistent workers.

A host named `narrate` can discover `narrate-engine-piper`; an OCR host can discover `ocr-engine-tesseract`; an artifact manager can discover `artifact-provider-huggingface`. The executable process is the stable ABI, so the host does not link every provider crate or accumulate a feature per plugin.

## Implemented surface

- `<namespace>-<kind>-<name>` identity and executable naming.
- Deterministic discovery from explicit roots, `<NAMESPACE>_PLUGIN_PATH`, `PATH`, and explicit per-plugin overrides.
- Shadow detection and precedence diagnostics.
- Lazy manifest inspection and disabled/memory/XDG/explicit-file caches fingerprinted by executable metadata.
- Hidden `__outboard manifest|doctor|cli-schema|ping|serve` control plane.
- Separate plugin, framework, application-interface, capability, and worker-protocol versioning using Cargo-style SemVer requirements.
- Extensible capability properties and application-defined `resolve_best` ranking.
- Deadlock-resistant, timeout-bounded one-shot process capture that concurrently drains stdout/stderr.
- Lossless `OsString` worker transport on Unix and Windows.
- Length-prefixed JSON worker framing with a 16 MiB default frame ceiling.
- Synchronous sequential worker client/server.
- Tokio worker client/server with multiplexed requests, progress/output events, cooperative cancellation, ping, and graceful shutdown.
- `outboard-clap` inverse serialization from the original clap struct to `Vec<OsString>`.
- `#[derive(ToArgv)]` for `Option`, `Vec`, booleans, `ArgAction::Count`, `ValueEnum`, flatten, subcommands, positionals, delimiters, `require_equals`, last/trailing args, naming rules, paths, and OS strings.
- `validate_args` / `validate_parser`, which serialize then feed argv back into clap so clap remains authoritative for conflicts, requirements, groups, and related constraints.
- Reusable `plugins list|inspect|doctor|path|cache` host CLI.
- Machine-readable clap command schema and JSON Schema for manifests/worker frames.
- `outboard-testing` black-box executable conformance harness.
- A real external host/plugin demo covering install, discovery, precedence, typed one-shot invocation, persistent PID reuse, concurrency, progress, cancellation, doctor, schemas, and protocol failure cases.

## Workspace

| Crate | Responsibility |
|---|---|
| `outboard-core` | IDs, manifests, discovery, compatibility, cache, process execution, resolution |
| `outboard-protocol` | Worker wire types, `WireOsString`, framing, JSON Schema |
| `outboard` | Facade, control dispatcher, synchronous workers |
| `outboard-clap-derive` | `#[derive(ToArgv)]` |
| `outboard-clap` | typed argv, clap validation/reflection, reusable management commands |
| `outboard-tokio` | concurrent async workers and cancellation |
| `outboard-testing` | black-box conformance checks |

The `examples/` workspace contains `outboard-demo-api`, `outboard-demo-engine-echo`, and `outboard-demo-host`.

## Application-owned interface crate

Hosts and implementations share a small domain API crate, not an implementation crate:

```rust
use clap::Args;
use outboard_clap::ToArgv;

#[derive(Debug, Clone, Args, ToArgv)]
pub struct OcrArgs {
    pub input: std::path::PathBuf,
    #[arg(long, default_value_t = 300)]
    pub dpi: u32,
    #[arg(long)]
    pub languages: Vec<String>,
}
```

Then a host resolves an external implementation and uses the same typed value:

```rust
let registry = outboard::Registry::new("ocr")?;
let req = outboard::PluginRequirement::new("engine")?
    .named("tesseract")?
    .interface(outboard::InterfaceRequirement::new(
        "ocr.engine",
        semver::VersionReq::parse("^1.0")?,
    )?);
let plugin = registry.resolve(&req)?;
let argv = outboard_clap::validate_args(&ocr_args)?;
let command = plugin.command_with(argv);
```

`OsString` is deliberate: an executable-plugin framework must not corrupt valid non-UTF-8 Unix paths.

## Discovery precedence

For a given namespace + kind, lower precedence wins:

1. explicit per-plugin override;
2. explicit host plugin roots, in declaration order;
3. namespace plugin-path environment variable (for example `OCR_PLUGIN_PATH`);
4. `PATH`, in order.

Only filenames matching `<namespace>-<kind>-` are scanned. Duplicate identities remain visible as shadowed candidates.

## Control plane

Every plugin can intercept these reserved invocations before its normal clap parser:

```text
__outboard manifest
__outboard doctor
__outboard cli-schema
__outboard ping
__outboard serve
```

`manifest` is the compatibility contract. `doctor` is plugin-specific health. `cli-schema` reflects application CLI grammar. `serve` enters the persistent-worker protocol.

## Persistent worker model

Configuration stays in argv. Large data stays in files/stdin/stdout. Worker JSON frames are a control plane, not a bulk-blob RPC layer.

Handshake:

```text
host   -> Hello(framework version, protocol version, required interfaces)
plugin -> Hello(protocol version, full manifest)
```

Invocation:

```text
host   -> Invoke(id, interface, command, argv)
plugin -> Started(id)
plugin -> Progress(id, ...)*
plugin -> Output(id, ...)*
plugin -> Finished(id, result) | Error(id, error)
```

The Tokio runtime multiplexes IDs so a model-backed OCR/TTS engine can initialize once and process multiple jobs concurrently. Cancellation is cooperative through `CancellationToken`; dropping the client still retains the process boundary as the ultimate failure-isolation mechanism.

## Reusable host management commands

Embedding `outboard_clap::PluginCommands` gives a host:

```text
myapp plugins list --kind engine --inspect
myapp plugins inspect engine:foo
myapp plugins doctor engine:foo --deep
myapp plugins path
myapp plugins cache path
myapp plugins cache clear
```

Deep doctor performs an actual worker handshake, ping, and shutdown when the plugin advertises worker mode.

## Demo and validation

Run the whole installed-binary workflow with:

```bash
./scripts/agent-e2e.sh
```

Do not treat that as the whole validation plan. `AGENT_TESTING.md` specifies manual failure injection for identity lies, incompatible interfaces, broken worker declarations, process timeouts/pipe pressure, and non-UTF-8 argv, and specifies exactly what evidence an agent must inspect.

## Design constraints

Outboard intentionally does not use the unstable Rust dynamic-library ABI, automatically install crates, define a network package registry, or put arbitrary large payloads inside JSON frames. Those can be layered above the stable executable boundary later.

Licensed MIT OR Apache-2.0.
