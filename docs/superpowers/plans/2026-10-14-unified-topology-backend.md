# Unified Topology Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 为 Graph、Mixer、Matrix 提供共用的后端读取与事务契约，暂不修改前端。

**Architecture:** EngineHost 保持唯一音频项目 owner；通过一个锁发布不可变的期望/活动项目与运行状态。保留现有 edit 接口，新增带返回值和执行时版本检查的事务；core 提供完整图上的只读邻接索引与连接预检。

**Tech Stack:** Rust 2024、现有 workspace crates、std::sync、现有软件音频测试宿主。

**Spec:** [三视图评估](../../designs/11-unified-topology-views-evaluation.md)，重点为 5.4、7、8、9.4 节。

## Global Constraints

- 不修改 `crates/moiren-app`、Slint、设备传输和 RT executor。
- 不添加依赖或新的 canonical topology；所有索引都是 LogicalGraph 的只读派生数据。
- 所有快照复制、索引构造、事务结果和校验在非 RT 侧完成。
- GraphRevision 只表达结构/静态配置版本；实时参数结果继续由 ParameterReceipt 表达。
- 活动项目描述编译/激活时的配置，不伪装为全部参数的实时实际值。
- 版本检查以宿主身份和 GraphRevision 为依据；参数命令保持接受顺序语义，不承诺参数批次同帧原子性。
- 保留原工作区的未提交改动；只提交本计划列出的后端路径和文档。

## Task 1: Coherent project snapshots

**Files:**

- Create: `crates/moiren-engine/src/host/snapshot.rs`
- Modify: `crates/moiren-engine/src/host.rs`
- Modify: `crates/moiren-engine/src/host/supervisor.rs`
- Modify: `crates/moiren-engine/src/host/supervisor/snapshot.rs`
- Modify: `crates/moiren-engine/src/host/supervisor/requests.rs`
- Test: `crates/moiren-engine/tests/host/snapshots.rs` and `main.rs`

**Interfaces:**

- Produces: `HostId`, `ProjectSnapshot { host_id, state: HostSnapshot, desired: Arc<EngineProject>, active: Option<Arc<EngineProject>> }`.
- Produces: `EngineObserver::read() -> ProjectSnapshot`; existing `snapshot()` and `project()` retain their return types.
- Immutable project Arcs are reused while their content is unchanged; parameters must refresh desired content without advancing GraphRevision.

- [x] Write tests for blocked updates, retained snapshots, unchanged Arc reuse and Closed state:

```rust
let before = observer.read();
assert_eq!(before.state.active_revision, Some(1));
// A blocked update advances desired only.
let pending = observer.read();
assert_eq!(pending.state.desired_revision, 2);
assert_eq!(pending.state.active_revision, Some(1));
assert_eq!(pending.desired.nodes[&gain], NodeConfig::Gain { initial: 0.5 });
assert_eq!(pending.active.as_ref().unwrap().nodes[&gain], NodeConfig::Gain { initial: 1.0 });
```

- [x] Run `cargo test --locked -p moiren-engine --test host snapshots`; expected missing `read` before implementation.
- [x] Implement one `Mutex<ProjectSnapshot>` replacing independently published state/project, plus unique process-local HostId allocation. Use `Arc::new(project.clone())` only when project content changes; clear active on final Closed publication.
- [x] Run the same test, then `cargo test --locked -p moiren-engine --test host`; expected all pass.
- [x] Commit exact task paths with `feat(engine): publish coherent project snapshots`.

## Task 2: Transaction results and conflict checks

**Files:**

- Create: `crates/moiren-engine/src/host/edits.rs`
- Modify: `crates/moiren-engine/src/host.rs`, `host/model.rs`, `host/snapshot.rs`
- Modify: `host/supervisor.rs`, `host/supervisor/requests.rs`, `host/supervisor/updates.rs`
- Test: `crates/moiren-engine/tests/host/edits.rs` and `main.rs`

**Interfaces:**

- Consumes: coherent snapshots and HostId from Task 1.
- Produces: `EditBasis { host_id: HostId, graph_revision: u64 }`, `ProjectSnapshot::edit_basis()`.
- Produces: `EditReceipt<T = ()>::result() -> Option<T>` for Clone results; `outcome()`/`wait()` retain existing lifecycle semantics and permit accounting.
- Produces: `EngineControl::edit_with_result<T: Send + 'static>(FnOnce(&mut EngineProject) -> Result<T, ProjectError>) -> Result<EditReceipt<T>, HostError>`.
- Produces: `EngineControl::edit_if_current<T: Send + 'static>(EditBasis, FnOnce(&mut EngineProject) -> Result<T, ProjectError>) -> Result<EditReceipt<T>, HostError>`.
- Produces: `EditOutcome::Conflict { expected: EditBasis, actual: EditBasis }`.

- [x] Write tests for returned node/port/edge IDs, failed candidate/capacity/panic results, superseded-but-accepted results, same-basis conflicts, cross-host conflicts and closure-not-executed conflicts.

```rust
let basis = observer.read().edit_basis();
let receipt = control.edit_if_current(basis, |project| {
    let node = project.graph.create_node(NodeKind::Bus, 1)?;
    project.nodes.insert(node, NodeConfig::Bus);
    Ok(node)
})?;
assert!(matches!(receipt.wait(TIMEOUT), EditOutcome::Activated { .. }));
assert!(observer.read().desired.graph.get_node(receipt.result().unwrap()).is_ok());
```

- [x] Run `cargo test --locked -p moiren-engine --test host edits`; expected new methods missing before implementation.
- [x] Type-erase a deferred result publication closure on the control side; invoke it only after candidate validation, host capacity and revision checks succeed. Compare the expected basis before invoking the user's edit closure. Keep accepted result available after Superseded or Closed.
- [x] Preserve `edit` as a unit-result compatibility wrapper and retain lifecycle permits until caller and coordinator release the receipt.
- [x] Run the focused tests and all host tests; expected all pass.
- [x] Commit with `feat(engine): return transaction identities and reject stale edits`.

## Task 3: View-neutral topology queries and parameter reads

**Files:**

- Create: `crates/moiren-core/src/graph/query.rs`
- Modify: `crates/moiren-core/src/graph.rs`
- Test: `crates/moiren-core/src/graph/query.rs` tests
- Modify: `crates/moiren-engine/src/host/project.rs`
- Test: `crates/moiren-engine/tests/host/project.rs`, `snapshots.rs`

**Interfaces:**

- Produces: `GraphIndex::new(&LogicalGraph)`, `incoming(NodeId)`, `outgoing(NodeId)`, `between(NodeId, NodeId)` returning checked edge slices; `input_edge(PortId)` returning checked occupancy.
- Produces: `LogicalGraph::validate_connection(PortId, PortId, SendParams) -> Result<(), GraphError>`, shared by `connect`.
- Produces: public `EngineProject::parameter_values() -> Vec<DesiredParameter>` using the existing override precedence.

- [x] Write tests proving parallel Bus routes, orphan nodes, invalid IDs/directions, direct-versus-transitive pairs, full-graph occupied inputs/cycles and no mutation during preflight.

```rust
let index = GraphIndex::new(&graph);
assert_eq!(index.between(source, bus).unwrap().len(), 2);
assert!(index.between(source, sink).unwrap().is_empty());
assert_eq!(index.input_edge(occupied).unwrap().unwrap().id(), edge);
let before = graph.clone();
assert_eq!(graph.validate_connection(other, occupied, SendParams::default()),
           Err(GraphError::InputAlreadyConnected));
assert_eq!(graph, before);
```

- [x] Run `cargo test --locked -p moiren-core graph::query` and host parameter-read tests; expected missing APIs before implementation.
- [x] Build sparse borrowed indices retaining Edge identity and all valid nodes. Share exact connect validation; do not add UI filter predicates to core.
- [x] Expose existing effective desired parameter enumeration and test override precedence and snapshot refresh on realtime edits.
- [x] Run core and engine tests; expected all pass.
- [x] Commit with `feat(core): expose topology queries for shared editors`.

## Task 4: Documentation, review and integration

**Files:**

- Create: `docs/designs/12-unified-topology-backend-contract.md`
- Add this plan and the evaluation report to the scoped documentation commit.

- [x] Document runnable examples for reading one snapshot, checking/creating a Bus route and obtaining IDs, handling conflicts, parameter reads, and directly indexing the full graph. State result acceptance versus activation and active-description versus actual-parameter boundaries.
- [x] Run `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `git diff --check` in the isolated worktree. Hardware tests remain separate.
- [x] Request one fresh review of the entire change as required by executing-plans. Focus on publication ordering, stale same-drain edits, result leakage on rejection, permits/close, unchanged RT paths, parallel-edge/hidden-occupancy semantics and source compatibility.
- [x] Fix material findings with regression tests, rerun affected checks, and record actual validation in the backend contract.
- [x] Commit documentation, then fast-forward the original feature branch if its HEAD still matches the base and the changed paths do not overlap existing work. Verify preserved UI changes and report local commit state.
