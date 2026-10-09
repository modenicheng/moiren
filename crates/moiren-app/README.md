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

FreeDSP 10 秒实际输出已获用户试听确认，见[实机记录](../../docs/experiments/windows/2026-10-08-shared-render.md)。Engine 已提供 [Plan swap API 与离线例子](../../docs/designs/06-compressor-plan-swap.md)，实际输出应用的换图控制入口仍待接入。其他 native output formats、多设备、设备恢复和 GUI 仍待后续实现；完整长期接口见 [IO 节点设计](../../docs/designs/03-io-nodes.md)。

## Windows 物理输入闭环

```powershell
cargo run --locked -p moiren-app -- monitor --list
cargo run --locked -p moiren-app -- render --list
cargo run --locked -p moiren-app -- monitor --input '<capture ID>' --output '<render ID>' --seconds 10 --gain 0.05 --pan 0
```

输入仅支持原生 44.1/48 kHz、mono/stereo f32；输出保持 48 kHz stereo f32 Shared。mono 明确复制为 stereo，再由 Compiler 准备 `Capture ClockSource → Gain → Pan → Sink`。输入 owner 与输出 master 通过预分配 Clock Bridge 隔离；连续相位线性插值及 fill PI 补偿独立时钟，不将 COM 或 native packet 放入 Graph。

`monitor::prepare_monitor` 可接受任意 stereo `RtAudioSource<f32>`，并返回真实 compiled Graph、Gain/Pan NodeId 和 output reader。Windows 的 `start_monitor` 返回 `MonitorSession`，暴露原有 `ControlPort`、参数 bindings 与标量 bridge observer，供后续应用控制层使用；CLI 只设置初值。`request_stop` 尝试停止两端，`join` 在控制侧协调完成并回收，任一端失败不会被另一端的静音/成功掩盖。

JSON 分别报告 capture、render 与 bridge。2048 输入帧的目标缓冲约为 42.7 ms（48 kHz）或 46.4 ms（44.1 kHz），另有设备/输出缓冲；启动时主动丢弃的旧缓存与 overflow 分别计数。线性 SRC 是功能基线，尚未提供专业重采样音质或无缝故障恢复。CLI 尚无 Ctrl+C 优雅退出入口；库 session Drop 和显式 stop 会协调释放。

两端共享独立于音频事件的 kernel stop signal，在 streaming 结束时立即通知 peer，再完成 COM 清理；控制侧的 join 轮询只负责回收。Capture 时长先开始计时，自然完成时会停止稍后启动的 Render；报告保留各自实际 elapsed 和状态，整体 Completed 不要求两端都单独耗尽计时器。

测试范围、实机结果与后续验收见[本次记录](../../docs/experiments/windows/2026-10-09-capture-clock-bridge.md)；后续任务按[音频后端计划](../../docs/superpowers/plans/2026-10-09-audio-backend.md)独立推进。

## Windows 应用声音闭环

```powershell
cargo run --locked -p moiren-app -- monitor --process <PID> --output '<render ID>' --seconds 10 --gain 0.05 --pan 0
```

`--process` 与 `--input` 互斥。CLI 先读取 PID + 创建时间，owner 启动时再次校验；当前仅支持包含目标进程及其子进程的模式。目标退出后显示 `target_exited` 并停止输出，不自动选择同名的新进程。不允许目标进程树包含此音频 host，以免自身输出形成回流。

Process Loopback 由 Windows 转成 48 kHz stereo f32，再复用 `ClockSource → Gain → Pan → Sink`。`ProcessMonitorOptions` 保存已解析的身份；`start_process_monitor` 返回现有 `MonitorSession`，控制端口和参数绑定与物理输入相同。异步启动可在控制侧先创建 `StopSignal`，通过 `start_process_monitor_with_stop` 传入，同时保留 clone 来取消准备过程。

Capture 报告 schema 2 包含 `source`、`process`、`windows_auto_conversion` 和 activation 耗时；虚拟源无 endpoint ID。原生设备位置恒为零的 Process client 不使用位置差推断断点，仍处理 Windows discontinuity flags；物理 Capture 保留原有位置检测。静默的目标是有效来源，报告音频计数不会把静音冒充非零声音。验收与剩余限制见 [Process Loopback 计划](../../docs/superpowers/plans/2026-10-09-process-loopback.md)。
