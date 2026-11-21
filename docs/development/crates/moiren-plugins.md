# moiren-plugins

Crate guide. Repository top rules: [AGENTS.md](../../../AGENTS.md); development index: [../README.md](../README.md). Paths below are relative to the crate unless linked.

## Role

Deliberately leaf crate: CLAP/VST2/VST3 binary loading, scanning, and realtime plugin instances. It knows the three plugin ABIs and its own `spec` types, nothing about the engine — the engine adapts live `PluginInstance`s onto its `RtProcessor` contract in `crates/moiren-engine/src/processor/plugin.rs`.

## Layout

- `src/clap/` (`abi.rs`, `mod.rs`), `src/vst2/`, `src/vst3/`: transcribed ABI declarations and per-format module/dispatch code. Do not "clean up" struct layouts, constants, or vtable order — they are byte-level contracts.
- `src/module.rs`: one live dynamic-library handle per canonical module path, with CLAP init/deinit running once per real load.
- `src/scan.rs`: standard-directory discovery, recursive search, VST3 bundle resolution, and per-file failure collection.
- `src/sandbox.rs`: out-of-process scanning. The parent spawns `moiren-plugin-scanner` once per root with a deadline and reads a JSON report from a file argument (not stdout).
- `src/instance.rs`, `src/spec.rs`: the format-agnostic realtime instance contract and parameter metadata.
- `src/guard.rs`: control-side panic guard. Every non-RT host call into plugin code goes through `guarded`, which turns a Rust panic unwinding out of the plugin into `PluginError::PluginPanic` (plugin path + call site).
- `src/bin/moiren-plugin-scanner.rs`: the scanner half of the sandbox.
- `tests/plugin_ffi.rs` builds a self-contained gain fixture cdylib and exercises all three entry points; `tests/sandbox.rs` covers the scanner process; `examples/probe.rs` is a manual probe.

## Contracts

- `PluginInstance::process` is the only realtime entry point. Its host plumbing (pointer tables, event queues, parameter containers) is preallocated at instantiation; no allocation, locking, or syscalls. A plugin's own RT behavior is its author's responsibility.
- Never load an untrusted plugin in the host process for scanning; the sandbox child crashing or hanging must surface as a typed `SandboxError` for that root only. Preserve the file-based report path.
- Keep the crate free of `moiren-*` dependencies (only `thiserror`, `serde`, `serde_json`, `libloading` today). The engine depends on this crate, never the reverse.
- No Rust panic or unwind may cross an FFI boundary; keep ABI declarations and format-specific parameter units intact (CLAP plain values, VST2/VST3 normalized `0..1`).
- Control-side panic isolation: `instantiate`, state save/load, and preset query/select are wrapped in `catch_unwind` (`src/guard.rs`); a panic becomes `PluginError::PluginPanic` and the host survives. Honest boundary (verified empirically on Windows MSVC): the guard catches panics that unwind through the *host's* Rust runtime — host plumbing inside the frame, or plugin code sharing the host's runtime. Panics raised by a separately compiled Rust cdylib abort as "foreign exceptions" (each cdylib links its own std), C++ exceptions translated into SEH are not Rust unwinds, and segfaults were never catchable — all of those still take the process down, and a caught panic deliberately leaks the half-built plugin object instead of calling back into it. `Drop` teardown is unguarded (nothing to report to; calling back into a panicking plugin is unsound). `preset_names` degrades to an empty list because its signature cannot carry an error.
- `process` (the RT path) has no panic guard by design: unwinding across an `extern "C"` boundary is UB (the Rust `"C"` ABI aborts on unwind), `catch_unwind`'s panic path allocates, and segfaults were never Rust unwinds anyway. Runtime crash isolation requires out-of-process plugin instantiation — that is the follow-up campaign line (2026-11-11 audit, M3: "C++ ABI 无崩溃隔离"; scanning is already out of process, instantiation is not).

## Verification

```sh
cargo test --locked -p moiren-plugins
cargo build --locked -p moiren-plugins --bin moiren-plugin-scanner
cargo run --locked -p moiren-plugins --example probe
```

Tests spawn `rustc` to build the fixture and load real dylibs, so this crate is excluded from Miri and from CI's Miri matrix by design. Run it on the host OS; do not add device or UI dependencies.

## Docs

- [Plugin host design](../../designs/19-plugin-host-clap-vst.md)
- Engine-side adapter: [crates/moiren-engine/src/processor/plugin.rs](../../../crates/moiren-engine/src/processor/plugin.rs)
- [Architecture rules](../architecture.md) for the cross-crate node wiring.
