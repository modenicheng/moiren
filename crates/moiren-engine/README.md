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
- `EngineControl::parameter` uses stable node/edge identities. Its receipt reports
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
