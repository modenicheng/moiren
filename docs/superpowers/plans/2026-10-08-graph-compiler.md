# First Graph Compiler Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compile supported LogicalGraph DAGs into runnable engines without manual buffer or operation assembly.

**Architecture:** Non-RT compilation assigns separate slots, lowers each PostFader send, prepares IO/resources/parameters, and returns the engine plus logical parameter bindings. Existing BufferArena, control queues and executor perform realtime work.

**Tech Stack:** Rust 2024, moiren-core, moiren-engine, rtrb, thiserror; no new dependencies.

## Global Constraints

- 单音频 slab，safe PreparedIo，无新增生产代码 unsafe。
- 一个 Graph RT owner；所有创建、编译和析构留在非 RT。
- Source/Sink/Gain/Bus/Pan；stereo balance 为当前 Pan 的线性系数。
- PreFader 和非 stereo 非零 send pan 明确报错。
- 字节预算只覆盖音频 slab；无设备探针、默认设备或其他应用设置修改。

### Task 1: Compiler and Send lowering

**Files:** Create `crates/moiren-engine/src/compiler.rs`, `src/compiler/send.rs`, `tests/compiler.rs`; modify `src/lib.rs`.

**Interfaces:**

```rust
pub fn compile<S: ProcessingSample>(
    graph: &LogicalGraph,
    bindings: NodeBindings<S>,
    config: CompileConfig,
) -> Result<CompiledGraph<S>, CompileError>;
// CompiledGraph: engine, control, bindings, stats.
// NodeBindings: bind_source, bind_sink, bind_io, bind_gain, bind_pan.
// CompiledBindings: node(NodeId) -> Option<ProcessorId>,
//                   edge(EdgeId) -> Option<SendParameterKeys>.
```

- [x] Add end-to-end tests using `compile`, with exact constant/mixed stereo samples, send fan-out and parameter ramps. Run `cargo test --locked -p moiren-engine --test compiler`; initially the compiler API is absent.
- [x] Implement typed binding/config/error/report types, graph validation and stable scheduling; generate dedicated slots and PreparedIo through existing BufferArena.
- [x] Add Send DSP with gain/pan/mute schemas and current stereo coefficients; attach each edge's operation before its target node.
- [x] Run the compiler tests and existing engine tests. Exact example expectation is `[0.375, 0.0, 0.375, 0.0, 0.375, 0.1875, 0.375, 0.375, 0.1875, 0.375, 0.0, 0.375, 0.0, 0.375, 0.0, 0.375]` after renders `[3, 1, 4]` and a Pan ramp from -1 to 1 at frame 2 over 4 frames.

### Task 2: Safety and graph variation

**Files:** Modify `tests/compiler.rs`, `tests/runtime.rs`.

**Interfaces:** Consume Task 1's compiler and parameter bindings; no new public API.

- [x] Add empty Bus, unconnected input, multiple channel counts, duplicate/wrong/missing bindings, unsupported send and budget/config tests; require typed failure before Engine publication.
- [x] Add deterministic random DAG evaluation with independently calculated edge and node coefficients, both f32/f64 and variable render lengths.
- [x] Use the existing runtime test allocator to count allocations and deallocations only during compiled renders; require `(0, 0)` with parameter events and bounded bridge IO.
- [x] Run `cargo test --locked -p moiren-engine`; preserve all existing executor tests.

### Task 3: Example, roadmap and CI

**Files:** Modify `examples/logical_graph.rs`, `README.md`, `docs/designs/04-logical-graph.md`, `docs/Moiren-PRD-Roadmap.md`, `docs/plans/2026-10-08-windows-integration-plan.md`, `.github/workflows/runtime-foundation.yml`; create `docs/designs/05-graph-compiler.md`.

**Interfaces:** The example consumes CompiledGraph directly and obtains Pan's ProcessorId from CompiledBindings.

- [x] Replace manual resources/slab/OpSpec assembly with NodeBindings and `compile`; preserve the existing output assertion and parameter ACK.
- [x] Document compiler capabilities and limits, checked baseline and M0.5 gates in the existing roadmap; keep historical implementation records dated as historical.
- [x] Extend CI Windows job with `cargo check/test/clippy --locked -p moiren-windows-audio --all-targets` as applicable; fmt covers the whole workspace; run the compiled logical_graph example on both CI platforms.
- [x] Run `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, all three offline examples and `git diff --check`. Report Windows local results separately from remote CI, Miri and actual hardware tests.

## Evaluation corrections and implementation results

- 本地与远端 main 均为 `adc725df1625254333062ebc33c1ca429894017a`。原评估的核心缺口、平铺 slab、软件 IO 及 W00 实验范围与代码/实验记录一致；实验仅证明被测捕获和静音 render，未证明真实音频重定向或跨钟稳定。
- 基线完整 Windows workspace 为 80 项测试，原记录的 58 项仅覆盖 core/engine/app。查询时 main 的 [CI #37793556077](https://github.com/modenicheng/moiren/actions/runs/37793556077) 仍在执行，Graph 提交的 #37792967985 已取消，原 #37785848837 成功；这些状态是查询快照，均未运行本轮改动。
- 实际跨独立 capture/render 的最小 Clock Bridge 必须与首次闭环一同交付，不能在稳定闭环之后补做。Shared Render 仍是下一开发切片，不需要先完成完整目录/管理器、GUI 或 Named Pipe。
- 首版 Compiler 已交付；不进行 slot reuse，以参考实现确保 fan-out 与重复 Bus 输入正确。PreFader/nonstereo 非零 pan 的能力边界明确报错；Control 通过编译返回的参数键发送实时请求。
- Windows 本地最终 93 项测试通过：新增 12 项 Compiler 测试及 1 项编译链路 RT 分配计数测试。24 张随机 DAG 在 f32/f64、四种 block 长度下与独立样本求值一致。
- workspace all-targets check、严格 Clippy、fmt、三个离线入口和 diff whitespace 检查通过。编译示例的 8 帧输出与原示例一致；被测 render 分配/释放为 `(0, 0)`。
- 独立只读审查定位了极大 send gain 与声像衰减的中间溢出，已先复现回归失败，再调整为先计算有限衰减系数；补齐 IO role、channel mismatch、prepared InputNode/OutputNode telemetry 的测试，复核无阻塞问题。
- CI 配置扩展为 workspace，Miri 保留 core/engine/app；本机仅有 Windows stable toolchain，未运行 Linux/Miri，也未重新运行任何真实设备实验。LICENSE 留待用户选择，不擅自确定授权条款。
- 用户原工作区同时编辑的 Compressor 文件保持原样；本轮实现与验证位于独立 worktree `D:/coding/moiren-graph-compiler`，不把其未完成编译计入本轮结果。
