# LogicalGraph、Bus 与 Pan 实施计划

日期：2026-10-08。本轮按用户已授权的开发范围直接实现，保留工作区中的 LogicalGraph 草稿作为演进起点。

## 设计决策

`moiren-core::graph` 是唯一可编辑拓扑，engine 继续执行预先准备的 `ExecutionPlan`。本轮完成逻辑图与两个 DSP processor，不实现 Graph Compiler、设备接入或运行中计划切换。

- 沿用 `NodeId` / `PortId` / `EdgeId`，Edge 自带 `SendParams`；不再同时维护第二套 Send 身份与节点 send 列表。
- Source：0 入 / 1 出；Sink：1 入 / 0 出；Gain：1 入 / 1 出；Bus：0..N 入 / 1 出；Pan：1 入 / 1 出、固定 stereo。声道数沿用现有 `usize`。
- 节点与端口字段私有，仅提供只读访问。固定布局在创建时生成，只有 Bus 允许增删输入；不提供无有效用途的动态输出接口。
- 每个输入最多一条边，输出允许扇出，也允许同一输出分别连接同一 Bus 的不同输入。连接必须满足方向、存在性、声道相同、有限非负 gain、pan 在 `[-1, 1]` 与 DAG 约束。
- `connect()` 不创建端口。独立 `graph::edit::connect_to_new_bus_input()` 组合创建与连接，连接失败移除新端口。断开边保留输入；删除节点移除关联边；连接中的端口必须先断开才能删除。
- ID 单调分配、删除后不复用；非法操作与 ID 溢出不留下半个节点或边。新增端口不超过 engine 的 `u16` 端口索引容量。
- 提供稳定拓扑排序与显式校验，覆盖未连接节点、平行节点间边及深链；拓扑编辑全部在非 RT 侧执行。
- engine `Bus` 复用已有 `Sum` 内核与输出预清零契约。engine `Pan` 采用用户已确认的 stereo balance：中心左右均为 unity，向右仅衰减左侧、向左仅衰减右侧，线性系数；支持 separate / in-place、f32 / f64 和既有逐样本 ramp。
- Edge 的 gain/pan/mute/tap 本轮保存为逻辑意图，执行 lowering 由后续 Compiler 完成；独立 Pan 也可被显式放入图中。

## 实施与验收

1. [x] 先加入图行为测试，运行 `cargo test --locked -p moiren-core --test graph` 确认草稿不满足 API。
2. [x] 完成模型、事务化创建、拓扑编辑、排序、校验和编辑 helper；验证非法连接、删除/重连、失败回滚及 ID 边界。
3. [x] 先加入 Bus / Pan 端口、样本和参数测试，再实现 processor 与导出；现有 Sum API 保持可用。
4. [x] 增加完整 engine 链路：重复 source 输入 → Bus → Pan → 软件输出，覆盖 fan-out、参数事件、跨 block ramp 和 render 零分配/释放。
5. [x] 更新文档与离线演示，运行测试、Clippy 和格式检查；检查 diff，保留用户其他改动。

关键文件：`moiren-core/src/graph.rs`（编辑算法）、`graph/model.rs`（数据模型）、`graph/edit.rs`（组合编辑）、`moiren-core/tests/graph.rs`（拓扑契约）、`moiren-engine/src/processor/builtin/`（DSP）、`moiren-engine/tests/runtime/main.rs`（RT 回归）、`moiren-engine/examples/`（可运行离线示例）。

检查命令：

```sh
cargo test --locked -p moiren-core -p moiren-engine -p moiren-app
cargo clippy --locked -p moiren-core -p moiren-engine -p moiren-app --all-targets -- -D warnings
cargo fmt -p moiren-core -p moiren-engine -p moiren-app -- --check
cargo run --locked -p moiren-engine --example logical_graph
```

## 实际验证结果

- core / engine / app 共 58 项测试通过（包含 1 项 compile-fail doctest），本轮新增 17 项。
- 上述 Clippy（`-D warnings`）、fmt 检查与 `git diff --check` 全部通过。
- `logical_graph` 例子运行通过，5 个节点 / 4 条边，8 帧 stereo 样本与预期完全一致；Pan 请求在 frame 2 获得 Applied，ramp 跨 3 / 1 / 4 帧的 block 连续推进。
- 完整 Bus / Pan 链路在 f32 / f64、Separate / InPlace 四种组合下 render 分配 / 释放计数均为 0。
- 检查范围为上述三个 crate；Windows 设备实验与 Graph Compiler 未在本轮扩展。所有改动留在当前工作区。
