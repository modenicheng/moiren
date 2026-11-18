# moiren-core

Crate guide. Repository top rules: [AGENTS.md](../../../AGENTS.md); development index: [../README.md](../README.md). Paths below are relative to the crate unless linked.

## Role

Editable, transport-neutral `LogicalGraph` plus the control protocol. Non-RT only: no DSP, no buffers, no threads, no platform code. The graph is the single topology truth; ports carry streams rather than channels or audio addresses, and only graph operations may mutate the model.

## Layout

- `src/graph.rs`: `LogicalGraph`, primitive mutations (`create_node`, `add_input_port`/`add_output_port`, `connect`, `disconnect`, `remove_node`, `set_send_params`), `topological_order`, and `validate`.
- `src/graph/model.rs`: `NodeKind` (closed enum), `Node`/`Port`/`Edge`, ID newtypes, `SendParams`, `GraphError`, and `initial_ports()`.
- `src/graph/edit.rs`: composed non-RT editor commands (for example `connect_to_new_bus_input`) that roll back on failure; IDs stay monotonic even when an edit fails.
- `src/protocol.rs`: transport-neutral control messages (`ParameterRequest`, `ControlReply`, `PROTOCOL_VERSION`, fixed body sizes). No realtime memory addresses cross IPC.
- `tests/graph.rs`: integration coverage for topology and validation.

## Invariants

- `NodeKind` is closed and matched exhaustively across the workspace. Do not replace it with a runtime registry. The root checklist starts here: update `NodeKind` and `initial_ports()`, then follow the engine and app touchpoints in [architecture.md](../architecture.md).
- Fixed port layouts are validated (`GraphError::FixedPortLayout`). Only Bus inputs and Rack inputs/outputs are dynamic; prepared port indices are u16-based, so keep the `MAX_INPUT_PORTS`/`MAX_OUTPUT_PORTS` address-space contract intact.
- `connect` rejects cycles, width/role mismatches, and occupied inputs. Any new validation rule belongs in `validate()` and in `tests/graph.rs`.
- Keep the crate leaf-like: the only dependency is `thiserror`, and it must not pull in engine, platform, or UI crates.

## Verification

```sh
cargo test --locked -p moiren-core
cargo clippy --locked -p moiren-core --all-targets -- -D warnings
```

CI runs Miri on this crate in stacked and Tree Borrows modes; keep the model free of unsafe and platform assumptions.

## Docs

- [design 04 logical graph](../../designs/04-logical-graph.md) and [design 03 IO nodes](../../designs/03-io-nodes.md) for intent; the source is the current contract.
- [Architecture rules](../architecture.md) for the cross-crate node-kind checklist.
