# moiren-engine

Crate guide. Repository top rules: [AGENTS.md](../../../AGENTS.md); development index: [../README.md](../README.md). Paths below are relative to the crate unless linked.

## Role

Realtime execution, non-RT compilation, and managed hosts. Transports live in `moiren-windows-audio`; this crate supports both f32 and f64 processing. `EngineHost` owns lifecycle and update coordination, `compiler` prepares immutable plans, and processors run them.

## Layout

- `src/host.rs` plus `src/host/`: `EngineHost`, `EngineProject`, control/observer handles, edits, parameters, preparation, retirement, supervisor, snapshot, and signals. The host validates and commits transactions, activates at block boundaries, and keeps control-side reclamation separate.
- `src/compiler/` (`bindings.rs`, `plan.rs`, `storage.rs`, `update.rs`, `observers.rs`, `send.rs`): binding resolution, lowering, prepared storage, compatible updates, and send routing.
- `src/processor/`: `builtin` DSP (Gain/Balance/Mono/Swap, Compressor, Eq, Gate, Limiter, Delay, Bus, ChannelMapper, Rack helpers), the `plugin` adapter, and the `RtProcessor`/`ProcessorError` contracts.
- `src/runtime.rs`, `src/buffer.rs`, `src/node.rs`, `src/boundary.rs`, `src/control.rs`, `src/sample.rs`: render loop, prepared buffers/IO, node configs, clock boundary, parameter routing, and sample traits.
- `src/recorder.rs`, `src/meter.rs`, `src/spectrum.rs`, `src/rack.rs`: recorder sinks/queue, observation taps, and rack expansion.
- `src/threads.rs`: process-wide thread registry (`register_thread` parent-side, `ThreadGuard::enter` on the spawned thread); register every new spawn site.
- `tests/` is split into `compiler/`, `host/`, `runtime/`, `common/`, plus `audio_config.rs`, `buffer.rs`, `io.rs`; `examples/` has the offline, logical-graph, managed-graph, plan-swap, compiler-optimization, and signal-state demos.

## Contracts

- No allocation, deallocation, blocking, or COM on the render path. Preparation, compilation, buffer/signal binding, and reclamation belong to control threads; retired plans and hosts return to non-RT.
- `EngineHost` owns compilation and launching; clients submit edits/parameters and must not compile, publish, restart, retire plans, or join device workers themselves. Handles and signal subscriptions do not keep audio alive; release the host off RT.
- Compatible updates transfer live IO endpoints and retain DSP state, node/send ramps, and signal bindings at a block boundary. Incompatible transport changes reclaim the owner and create a new epoch; old-epoch automation is not migrated.
- Parameter application is asynchronous and reported as requested → accepted → applied; do not treat submission as applied. Revision, owner, resource, and epoch identities are separate.
- Recorder audio crosses a bounded SPSC queue; file writing and finalization stay off RT. A slow destination can drop whole blocks.
- Rack slots are stable identities; do not reuse a previous kernel when static settings or bypass changed.

## Verification

```sh
cargo test --locked -p moiren-engine
cargo test --locked -p moiren-engine --features mp3,flac --lib recorder
cargo test --locked -p moiren-engine --features mp3,flac --test host recording
```

CI's Miri matrix covers `--lib --test audio_config --test buffer --test io --test runtime`, `--test compiler`, and `--test host`, in stacked and Tree Borrows modes. Native RT contracts for the Windows owner live in `tools/rt-check` (`cargo test --locked --manifest-path tools/rt-check/Cargo.toml`); run it when touching the native boundary.

## Docs

- [crate README](../../../crates/moiren-engine/README.md): ownership/update/backend contracts and builtin-node behavior.
- Design records: [engine design](../../designs/02-engine-design.md), [graph compiler](../../designs/05-graph-compiler.md), [compressor plan swap](../../designs/06-compressor-plan-swap.md), [Windows audio threads](../../platform/windows-audio-threads.md).
- Node-kind checklist and add-node wiring: [architecture.md](../architecture.md).
