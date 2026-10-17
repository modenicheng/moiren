# Moiren managed engine

`host::EngineHost` is the non-RT lifecycle owner for an `EngineProject`. Opening a
host automatically prepares and starts the project when output resources are
available. Clients submit edits and parameters; they do not compile, publish,
restart, retire plans or join device workers.

```rust,ignore
let host = EngineHost::<f32>::open(config, project, backend)?;
let control = host.control();
let observer = host.observer();
let edit = control.edit(|project| {
    project.processing.audio_byte_budget = 8 * 1024 * 1024;
    Ok(())
})?;
let outcome = edit.wait(timeout);
let snapshot = observer.snapshot();
```

The complete portable example applies a parameter, inserts a Gain node, verifies
the changed samples and automatically reclaims its runtime on scope exit:

```text
cargo run --locked -p moiren-engine --example managed_graph
```

## Ownership and updates

- The project contains the real `LogicalGraph`, node configuration, desired
  parameters and bounded processing configuration. `EngineControl::edit`
  validates and commits the complete transaction on the control worker.
- `EngineControl::parameter` uses stable node/edge/Rack-slot identities. Its receipt reports
  application or rejection; graph activation does not acknowledge separate
  parameter commands that have not yet applied.
- Graph revision, plan revision, owner generation, resource generation and epoch
  have separate identities. Activation requires backend/RT confirmation.
  Superseded versions cannot overwrite the current project.
- Compatible edits transfer live IO endpoints and retain DSP state, node/send
  ramps and signal bindings at the block boundary. RT transfer performs no heap
  allocation or destruction. Preparation and reclamation have separate lanes.
- Missing resources retain the desired project. Events carry their observed IO
  configuration, so an old device cannot block a new selection on the same node.
  Static preparation failures retain the valid active plan without a retry loop.
- Incompatible transport changes automatically reclaim the old owner and create
  a replacement with a new epoch. Old-epoch automation is not migrated.

Release `EngineHost` off RT. Control/observation handles and signal subscriptions
do not keep audio alive. Drop waits for shutdown; if released inside its own
finite edit callback, the coordinator closes after the callback returns without
self-join. `Closed` retains `fault` and `cleanup_error` diagnostics.

## Backend contract

`EngineBackend` prepares IO bindings/observers and a waiting transport without
compiling the graph. Update preparation leaves reused IO nodes unbound and keeps
the compatible transport. The host owns compilation, parameter routing and
automatic signal-host attachment.

`PreparedTransport::attach` gives Engine to a waiting owner. It must not start
audio. The coordinator calls `BackendOwner::activate` after validating current
project/resource/owner identity and shutdown eligibility. Status/timeline queries
must be bounded and nonblocking. Cancellation wakes native work;
`stop_and_join` returns Engine even on failure, after all callbacks have stopped.
COM objects stay on their native owner threads.

The Windows implementation is
`moiren-windows-audio::engine_backend::WindowsBackend`, with no app dependency.

## Low-level execution

`runtime::Engine`, `compiler::compile` and explicit plan ports remain available
for offline execution and special integrations. `compile_update` uses metadata
reservations instead of duplicating SPSC endpoints; `compile_update_with_reuse`
also retains stable send state. `require_sink_reuse` protects a fixed output
consumer. Unbound, retired or incorrectly based candidates cannot activate.
Failed reuse terminalizes accepted candidate controls before destroying the
unrendered preparation. Stopped pending plans return through `take_pending_plan`
for non-RT retirement.

Portable host/runtime tests cover startup/shutdown, transactions, coalescing,
stale work, confirmation, parameters, resource recovery, signals, non-RT
destruction and allocation tracking. Native hardware checks are separate ignored
windows-audio tests with zero output gain.

## Builtin nodes

The host prepares Input/Output, Gain, stereo Balance/Mono/Swap, Bus, Compressor,
EQ, Gate, Limiter, Delay, ChannelMapper, Recorder, Meter, SpectrumAnalyzer and
Rack. DSP and graph compilation support both f32 and f64; native Windows IO
has its own narrower format contract.

Rack expands real leaf kernels into the existing plan. `create_rack` defines
independent stream ports; `RackSettings::routes` assigns each slot to a
nonbranching chain. Slots execute in vector order. `resolve_layout` reports
each slot's actual input/output widths, including ChannelMapper changes and
bypass. An empty routes list retains the original main-input/main-output chain.
Unrouted outputs are silent. Route port indices must be updated when preceding
ports are removed; slot IDs stay stable when slots move.

`ParameterTarget::RackSlot` resolves its stable node/slot/parameter identity
against the active plan. Saved desired values are prepared before the first
audio block. Compatible updates retain unchanged, active slots with matching
actual IO, including delay history, filter state and parameter ramps; changing
static settings or bypass does not reuse the previous kernel. Low-level callers
can request the same transfers with `UpdateReuse::rack_slots` and must assert
static configuration compatibility themselves.

`EngineObserver` exposes managed Meter/Recorder readers and typed level/spectrum
subscriptions. Reader handles do not keep the host alive, and owner generations
prevent an old handle from controlling a replacement owner's recorder. Recorder
audio passes through a bounded SPSC queue; file writing and WAV finalization stay
off RT. All observer and DSP storage is prepared before rendering.

`ManagedRecorderReader::record_wav(path, processing_rate)` leases the only queue
consumer to a dedicated file worker. `RecordingSession::status` reports written
and dropped frames, lifecycle and IO errors. `stop` requests shutdown;
`finish` waits for an in-flight audio block, drains the tail and finalizes the
IEEE-f32 WAV. Drop also stops and joins, but use `finish` to observe errors.
While leased, raw reader operations and additional recording claims cannot
consume the queue. Compatible graph updates retain the session; removing or
reconfiguring its Recorder, replacing the owner or closing the host revokes it
and automatically finalizes the old file without keeping audio alive. A slow
destination can cause whole-block queue drops and delay joining its file worker;
host shutdown does not join that worker. WAV remains limited to RIFF's 32-bit
lengths. Custom seekable writers use `record_with_writer`; low-level readers
offer the same worker API by consuming their reader.

EQ is one peaking band. Limiter measures sample peaks without lookahead or
oversampling. Meter reports Sample Peak/RMS, not LUFS or True Peak. Rack leaf
kernels currently have one input stream and one output stream; plugin buses and
plugin hosting are not implemented. See [backend acceptance](../../docs/validation/backend-nodes.md)
for the precise scope and verification commands.
