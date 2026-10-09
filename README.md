# Moiren

A real-time audio graph for Windows.

当前仓库包含 Rust 音频引擎、自动 Graph Compiler、首条 WASAPI Shared 实际输出链路和独立 Windows 可行性实验，尚非可安装音频路由产品。

## 运行首版应用

```sh
cargo run --locked -p moiren-app
cargo run --locked -p moiren-app -- --gain 0.25
```

默认入口跑通 `软件输入 → InputNode → Gain → OutputNode → 软件输出`，打印处理后样本与 IO 状态。应用使用一个 planar slab 和两个预分配音频桥，支持多声道、可变 block 与经控制队列下发增益；此离线模式不打开设备。见 [App 说明](crates/moiren-app/README.md)与 [IO 节点设计](docs/designs/03-io-nodes.md)。

Windows 下可播放 Compiler 准备的 `Sine → Gain → Pan → Sink → WASAPI Shared` 测试信号：

```powershell
cargo run --locked -p moiren-app -- render --list
# 从列表选择 endpoint；默认 10 秒、440 Hz、gain 0.05，不修改设备或其他应用音量。
cargo run --locked -p moiren-app -- render --endpoint '<endpoint ID>'
```

首版仅接受 native 48 kHz / stereo / f32；不转换其他输出设备格式。FreeDSP 的 10 秒实机输出已获用户试听确认，见[验收记录](docs/experiments/windows/2026-10-08-shared-render.md)。持续多源 host 已接入独立物理/process capture、Clock Bridge、运行中参数控制与换图，命令及 UI 控制契约见 [App 说明](crates/moiren-app/README.md#windows-持续多源-host)。

## 运行引擎骨架

LogicalGraph 已提供节点/端口/边编辑、独立 Bus 动态输入、连接校验及稳定 DAG 排序；Compiler 自动准备 Source/Sink/Gain/Bus/Pan/Compressor 和 PostFader Edge 的 gain/pan/mute。执行 `cargo run --locked -p moiren-engine --example logical_graph` 可验证两个 Source → Bus → Pan → 软件输出及跨 block 声像 ramp，无需手写 OpSpec 或 BufferSlotId。当前使用独立槽位，PreFader 和非 stereo 非零 send pan 明确报错，见 [Compiler 契约](docs/designs/05-graph-compiler.md)。

Compressor 提供 10 个可自动化参数、多声道 peak 联动、soft knee、attack / hold / release 与干湿混合。Engine 支持控制侧准备候选、音频块边界切换和控制侧延迟回收；显式兼容复用可保持 DSP 状态、参数 ramp 和输出桥。热切换要求 EngineConfig 和 timeline epoch 不变，见 [Compressor 与 Plan 切换契约](docs/designs/06-compressor-plan-swap.md)。离线示例在线插入 Compressor 并校验连续增益 ramp：

```sh
cargo run --locked -p moiren-engine --example plan_swap
```

在仓库根目录执行；Windows 与 Linux 使用相同 Cargo 命令：

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run --locked -p moiren-engine --example offline
```

`offline` 演示 `Source → Pre Meter → In-place Gain → Post Meter`，全部音频使用一个 planar slab；参数经版本化消息编解码、有界控制队列及 Processing Timeline 下发。示例不打开音频设备，不播放声音。

平铺 buffer、unsafe 不变量与 IPC 分层见[实时基础实施计划](docs/plans/2026-10-08-runtime-foundation.md)；新增编译能力见 [Compiler 契约](docs/designs/05-graph-compiler.md)。Named Pipe、标准 LUFS、完整 Windows backend、优化 BufferPlanner 与跨 epoch / 采样率热切换尚未实现。

## 文档入口

- [产品与路线图](docs/Moiren-PRD-Roadmap.md)
- [Graph 设计](docs/designs/01-audio-graph.md)与[Engine 总体设计](docs/designs/02-engine-design.md)
- [基础 IO 节点](docs/designs/03-io-nodes.md)与[IO / App 实施记录](docs/plans/2026-10-08-basic-io-nodes.md)
- [LogicalGraph、Bus 与 Pan](docs/designs/04-logical-graph.md)与[实施记录](docs/plans/2026-10-08-logical-graph.md)
- [Graph Compiler](docs/designs/05-graph-compiler.md)与[实施计划](docs/superpowers/plans/2026-10-08-graph-compiler.md)
- [Compressor 与 Plan 切换](docs/designs/06-compressor-plan-swap.md)与[实施记录](docs/superpowers/plans/2026-10-09-compressor-plan-swap.md)
- [Shared Render 设计](docs/superpowers/specs/2026-10-08-shared-render-design.md)、[实施记录](docs/superpowers/plans/2026-10-08-shared-render.md)与[实机验收](docs/experiments/windows/2026-10-08-shared-render.md)
- [实时基础实施计划](docs/plans/2026-10-08-runtime-foundation.md)：本轮具体实现契约；与历史 buffer 草案冲突时以该计划为准
- [Windows 接入计划](docs/plans/2026-10-08-windows-integration-plan.md)与[W00 实验说明](crates/moiren-windows-audio/README.md)

W00 探针仍保持独立；新增 render 模块连接正式 Engine。已有实验与单输出结果不表示已完成应用输出接管、多设备桥接或长期稳定性验收。
