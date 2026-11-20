# moiren-signal

Crate guide. Repository top rules: [AGENTS.md](../../../AGENTS.md); development index: [../README.md](../README.md). Paths below are relative to the crate unless linked.

## Role

Independent typed `Latest`/`Stream` signal storage used by the engine and the app control plane. Registration, subscription, name resolution, pumping, and owner cleanup are non-RT; publication happens through one `RtHost` per owner thread and a current operation scope. Values and block headers/elements must be `Copy + Send + Sync + 'static`.

Do not turn this crate into a general pub/sub broker: its contract is fixed-capacity, budgeted, fully preallocated storage with explicit lifecycle ownership.

## Layout

- `src/host/` (`mod.rs`, `binding.rs`, `scope.rs`, `slot.rs`): `RtHost`, host ownership, activation, scopes, and prepared updates.
- `src/transport/latest.rs`, `src/transport/stream.rs`: the two transport shapes (`Latest` coalesces; `Stream` is a bounded ordered queue).
- `src/dispatcher.rs`, `src/demand.rs`: block fanout, demand caching, and pump budgeting (`PumpBudget`).
- `src/registry.rs`, `src/lifecycle.rs`, `src/subscriber.rs`, `src/types.rs`, `src/budget.rs`, `src/storage.rs`, `src/sequence.rs`, `src/error.rs`: `Bus`, topic lifecycle, subscriber guards, config/limits, budget accounting, and storage/sequence primitives.
- `src/test_alloc.rs`: TLS allocation counting used by the tests.
- `tests/`: `allocation.rs`, `concurrency.rs`, `demand.rs`, `host.rs`, `latest.rs`, `lifecycle.rs`, `registry.rs`, `stream.rs`; `examples/custom_block.rs` is the end-to-end example.

## Invariants

- Never allocate, lock, or make syscalls on the RT path. Preparation allocates and charges storage; `commit` moves only preallocated owners; `RetiredHostUpdate`, activation failures, and the stopped host must return to non-RT before being dropped (destruction can free heap storage and final lifecycle leases).
- Publication requires a `scope(&op)` authorized by a current operation table. One `BindingKey` may authorize several topics for one operation, while every update has its own table identity. A superseded table fails even if the key/handle is reused; scopes and block guards borrow the exclusive host access and are not `Send`.
- `Latest` is observation with coalescing; `Stream` is a bounded ordered queue with local drop on full subscribers and no replay for late subscribers. Preserve those semantics when touching fanout.
- Budget the complete transport layout (metadata plus payload plus padding) and keep `required_copy_bytes` and the saturating statistics truthful. Pump visits directory slots round-robin; no worker or wakeup is created.
- Keep the `BusConfig` limits and the "no concurrent host migration" rule; a new host may rebind a registered topic only after the old host is stopped and released on non-RT.
- `Bus::unregister` is transactional: it returns `Busy` without changing anything while a publisher binding or lifecycle lease remains. Status distinguishes `Closed`, `NoPublisher`, and an empty read; previously held guards stay valid, and removing a publisher alone keeps old snapshots and queued messages. Stable cells keep the original transport producer and checked sequence across activation and never reset the demand cache.

## Verification

```sh
cargo test --locked -p moiren-signal
cargo run --locked -p moiren-signal --example custom_block
```

CI runs Miri in stacked and Tree Borrows modes. The allocator tests enable the TLS allocator only around RT calls; registration, subscription, pumping, thread joins, and final cleanup are intentionally outside that window. The concurrency test uses finite work, channels, and barriers instead of sleeps — follow that pattern.

## Docs

- This guide is the single canonical contract for ownership, activation, budgeting, and verification boundaries; the [crate README](../../../crates/moiren-signal/README.md) is a short pointer to it. Keep contract details here instead of duplicating them in the README.
- [RT/UI signal bus design](../../superpowers/specs/2026-10-10-rt-ui-signal-bus-design.md) is the design record.
- [Architecture rules](../architecture.md) for the repo-wide RT rules.
