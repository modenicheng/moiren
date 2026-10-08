# Moiren App

首版 headless 应用负责 engine 的装配、控制和生命周期，运行软件 IO 全链路：

```text
interleaved input → AudioWriter → InputNode → Gain → OutputNode → AudioReader → interleaved output
```

在仓库根目录运行：

```sh
cargo run --locked -p moiren-app
cargo run --locked -p moiren-app -- --gain 0.25
cargo test --locked -p moiren-app
```

默认演示 stereo f32、48 kHz Processing SR、可变 block 和一个 planar slab；输入和输出由独立预分配 sample ring 隔离。程序打印输入、处理后样本和边界诊断，不打开音频设备。

library 的 `OfflineApp::process_interleaved` 接受等长、完整 frame 的输入/输出 slices，超过最大 block 自动拆分；空请求不推进时间。`set_gain` 经控制队列下发，`poll_applied` 返回运行确认；持续更新参数时需消费确认队列。`input_status` / `output_status` 返回有界队列中最新可用快照，队列满时的新快照可能已丢弃。

`OfflineApp` 是同步离线 owner；设备线程需在正式 backend 接入时另建 owner 与调度。`stop` 在处理结束后释放 engine 与桥。每块 slab/bridge 的 sample storage 上限为 8 MiB，队列及 metadata 不计入该上限。

engine 的物理设备配置目前表达选择意图；WASAPI、格式协商、PCM、SRC、跨时钟桥和 GUI 按后续里程碑实现。详细接口与限制见 [IO 节点设计](../../docs/designs/03-io-nodes.md)。
