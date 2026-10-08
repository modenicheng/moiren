# Moiren

A real-time audio graph for Windows.

当前仓库包含 Rust 音频引擎基础骨架与独立 Windows 可行性实验，尚非可安装音频路由产品。

## 运行首版应用

```sh
cargo run --locked -p moiren-app
cargo run --locked -p moiren-app -- --gain 0.25
```

`moiren-app` 跑通 `软件输入 → InputNode → Gain → OutputNode → 软件输出`，打印处理后样本与 IO 状态。应用使用一个 planar slab 和两个预分配音频桥，支持多声道、可变 block 与经控制队列下发增益；当前入口为离线 headless 模式，不打开设备。见 [App 说明](crates/moiren-app/README.md)与 [IO 节点设计](docs/designs/03-io-nodes.md)。

## 运行引擎骨架

LogicalGraph 已提供节点/端口/边编辑、独立 Bus 动态输入、连接校验及稳定 DAG 排序；engine 新增 Bus 与立体声 Pan。执行 `cargo run --locked -p moiren-engine --example logical_graph` 可验证两个 Source → Bus → Pan → 软件输出及跨 block 声像 ramp。例子为已知拓扑手工准备执行计划，见 [LogicalGraph / Bus / Pan 契约](docs/designs/04-logical-graph.md)。

在仓库根目录执行；Windows 与 Linux 使用相同 Cargo 命令：

```sh
cargo test --locked -p moiren-core -p moiren-engine -p moiren-app
cargo clippy --locked -p moiren-core -p moiren-engine -p moiren-app --all-targets -- -D warnings
cargo fmt -p moiren-core -p moiren-engine -p moiren-app -- --check
cargo run --locked -p moiren-engine --example offline
```

`offline` 演示 `Source → Pre Meter → In-place Gain → Post Meter`，全部音频使用一个 planar slab；参数经版本化消息编解码、有界控制队列及 Processing Timeline 下发。示例不打开音频设备，不播放声音。

本轮接口、unsafe 不变量、IPC 分层、已实现范围及下一步验收，统一见[实时基础实施计划](docs/plans/2026-10-08-runtime-foundation.md)。这不是完整 Graph Compiler、Named Pipe 服务、标准 LUFS 响度计或正式 WASAPI 后端。

## 文档入口

- [产品与路线图](docs/Moiren-PRD-Roadmap.md)
- [Graph 设计](docs/designs/01-audio-graph.md)与[Engine 总体设计](docs/designs/02-engine-design.md)
- [基础 IO 节点](docs/designs/03-io-nodes.md)与[IO / App 实施记录](docs/plans/2026-10-08-basic-io-nodes.md)
- [LogicalGraph、Bus 与 Pan](docs/designs/04-logical-graph.md)与[实施记录](docs/plans/2026-10-08-logical-graph.md)
- [实时基础实施计划](docs/plans/2026-10-08-runtime-foundation.md)：本轮具体实现契约；与历史 buffer 草案冲突时以该计划为准
- [Windows 接入计划](docs/plans/2026-10-08-windows-integration-plan.md)与[W00 实验说明](crates/moiren-windows-audio/README.md)

W00 探针与引擎保持分离；已有实验记录不表示已完成应用输出接管、多设备桥接或长期稳定性验收。
