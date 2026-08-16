# Architecture

Outboard separates packaging, discovery, application semantics, and transport instead of collapsing them into one plugin ABI.

```text
application interface crate (clap structs, interface IDs, domain payloads)
       |                         |
       v                         v
 outboard-clap                plugin implementation
       |                         |
       +---- typed argv --------> executable
       |                         ^
 host -> outboard-core ----------+ discovery / manifest / resolution
       |
       +-> outboard-protocol <---- worker frames
       |
       +-> outboard-tokio -------- concurrent lifecycle
```

The stable contracts are: executable naming, the `__outboard` control plane, application-owned interface IDs + SemVer versions, and the independently versioned Outboard worker protocol.

Discovery never executes every program on a machine. It scans only the requested `<namespace>-<kind>-` prefix, retains shadowed duplicates, and lazily interrogates selected candidates. Resolution first applies generic compatibility filtering and then optionally applies application scoring. Outboard does not decide whether CUDA is preferable to CPU or one OCR engine is higher quality.

`run_capture` drains stdout and stderr on separate threads while polling a timeout, avoiding the classic child-waits-on-full-pipe/parent-waits-on-child deadlock.

Worker framing is `u32` big-endian JSON length followed by JSON bytes. `WireOsString` uses UTF-8 when possible and native Unix bytes or Windows UTF-16 code units otherwise. Large artifacts should be sent by path/reference or ordinary process streams.

`#[derive(ToArgv)]` is an inverse serializer for the *original* Rust clap type; it does not generate a second client model. `validate_args` and `validate_parser` round the resulting argv through clap, making clap authoritative for cross-field semantics that an inverse field serializer should not duplicate.

The synchronous worker API is intentionally sequential. `outboard-tokio` adds request multiplexing, progress/output events and cooperative cancellation without imposing Tokio on `outboard-core`.

Schemas are tooling surfaces: clap reflection produces a language-neutral command tree and Schemars produces JSON Schema for the framework wire contract. Runtime compatibility still uses explicit interface/protocol negotiation.
