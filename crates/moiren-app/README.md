# Moiren App

headless 应用负责 engine 的装配、控制和生命周期，默认运行软件 IO 全链路：

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

`OfflineApp` 是同步离线 owner。`stop` 在处理结束后释放 engine 与桥。每块 slab/bridge 的 sample storage 上限为 8 MiB，队列及 metadata 不计入该上限。

## Windows 实际输出

```powershell
cargo run --locked -p moiren-app -- render --list
cargo run --locked -p moiren-app -- render --endpoint '<endpoint ID>' --seconds 10 --frequency 440 --gain 0.05 --pan 0
```

`render --list` 仅查询 active render endpoints，不依赖麦克风或默认设备存在。列表显示原生格式与查询错误；无法读取 ID 的诊断项为空 ID，不能选择。播放必须提供完整 opaque ID，不能按名称猜测或自动跟随默认设备。默认参数为 10 秒、440 Hz、线性 gain 0.05、居中；时长范围 1..600 秒。此版本只接受 native 48 kHz、stereo、32-bit float，不修改 endpoint/session 音量、mute 或默认设备。

`prepare_tone` 用 Compiler 自动准备 `Sine Source → Gain → Pan → Sink`。Gain/Pan 保留参数绑定和控制端口；命令行只设置初始值，运行中交互控制待后续接入。软件输出桥由 WASAPI owner 同步写入/读取；owner 按 padding demand 拆分 Engine block，并提交实际 interleaved PCM。桥不跨时钟，也不提供 SRC。

正常时长结束后 Stop、释放全部 stream/COM 对象，再输出 JSON 标量报告。库的 `RenderSession` 支持 `request_stop` / `join`，Drop 也会发 stop 并 join；命令行尚无 Ctrl+C 优雅停机处理。运行期报告区分处理帧、成功提交帧和失败阶段；空 padding 只是诊断，不能单独证明 underrun。

FreeDSP 10 秒实际输出已获用户试听确认，见[实机记录](../../docs/experiments/windows/2026-10-08-shared-render.md)。Engine 已提供 [Plan swap API 与离线例子](../../docs/designs/06-compressor-plan-swap.md)，实际输出应用的换图控制入口仍待接入。其他 native formats、capture/loopback 到 Graph 的连接、跨钟 bridge、设备恢复和 GUI 仍待后续实现；完整长期接口见 [IO 节点设计](../../docs/designs/03-io-nodes.md)。
