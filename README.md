# Moiren

A real-time audio graph for Windows.

当前仓库包含 Rust 音频引擎基础骨架与独立 Windows 可行性实验，尚非可安装音频路由产品。

## 运行引擎骨架

在仓库根目录执行；Windows 与 Linux 使用相同 Cargo 命令：

```sh
cargo test --locked -p moiren-core -p moiren-engine
cargo clippy --locked -p moiren-core -p moiren-engine --all-targets -- -D warnings
cargo fmt -p moiren-core -p moiren-engine -- --check
cargo run --locked -p moiren-engine --example offline
```

`offline` 演示 `Source → Pre Meter → In-place Gain → Post Meter`，全部音频使用一个 planar slab；参数经版本化消息编解码、有界控制队列及 Processing Timeline 下发。示例不打开音频设备，不播放声音。

本轮接口、unsafe 不变量、IPC 分层、已实现范围及下一步验收，统一见[实时基础实施计划](docs/plans/2026-10-08-runtime-foundation.md)。这不是完整 Graph Compiler、Named Pipe 服务、标准 LUFS 响度计或正式 WASAPI 后端。

## 文档入口

- [产品与路线图](docs/Moiren-PRD-Roadmap.md)
- [Graph 设计](docs/designs/01-audio-graph.md)与[Engine 总体设计](docs/designs/02-engine-design.md)
- [实时基础实施计划](docs/plans/2026-10-08-runtime-foundation.md)：本轮具体实现契约；与历史 buffer 草案冲突时以该计划为准
- [Windows 接入计划](docs/plans/2026-10-08-windows-integration-plan.md)与[W00 实验说明](crates/moiren-windows-audio/README.md)

W00 探针与引擎保持分离；已有实验记录不表示已完成应用输出接管、多设备桥接或长期稳定性验收。
