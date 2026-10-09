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

FreeDSP 10 秒实际输出已获用户试听确认，见[实机记录](../../docs/experiments/windows/2026-10-08-shared-render.md)。Engine 已提供 [Plan swap API 与离线例子](../../docs/designs/06-compressor-plan-swap.md)，持续 host 的换图控制入口见下文。其他 native output formats、多输出设备和设备恢复仍待后续实现；完整长期接口见 [IO 节点设计](../../docs/designs/03-io-nodes.md)。

## Windows 持续多源 host

```powershell
cargo run --locked -p moiren-app -- host --list
cargo run --locked -p moiren-app -- host --output '<render ID>' --input '<capture ID>' --process <PID>
# 有界测试：可超过 600 秒；stdin EOF 后仍运行到截止时间。
cargo run --locked -p moiren-app -- host --output '<render ID>' --input '<capture ID>' --seconds 7200
```

`host` 必须显式选择输出，可重复指定 `--input`、`--process` 并混合使用；不指定输入时运行空 Bus 的静音输出。每个输入独立 `Source → Gain(0.05) → Pan → [Compressor] → Bus → Output`。初值在首次发布前编译，避免短暂 unity gain。所有 capture 与 render 使用独立 stop event；一个输入退出/失败会立即 gate 为静音并 join 该输入，其他输入与输出继续运行。输出失败则 host 进入 `failed` 并停止、join 全部输入。

不带 `--seconds` 时会话持续到 stdin `stop` 或 EOF；带 `--seconds` 时，时长从启动握手完成后计算，EOF 保持输出到截止时间，`stop` 可提前结束。stdin 由独立线程读取到容量 32 的有界队列，控制侧每 10 ms 继续轮询 worker 与回执，即使没有输入也每秒输出状态。每行一个 JSON 命令，例如：

```json
{"op":"status"}
{"op":"graph"}
{"op":"gain","source":1,"value":0.1,"ramp_frames":480}
{"op":"pan","source":1,"value":-0.5,"ramp_frames":480}
{"op":"compressor","source":1,"enabled":true,"settings":{"threshold_db":-24,"ratio":4}}
{"op":"publish"}
{"op":"add","selection":{"kind":"physical","endpoint_id":"<capture ID>"}}
{"op":"publish"}
{"op":"enable","source":1,"enabled":false}
{"op":"stop_source","source":1}
{"op":"restart","source":1}
{"op":"publish"}
{"op":"remove","source":2}
{"op":"publish"}
{"op":"processes"}
{"op":"replace","source":1,"selection":{"kind":"process","pid":1234,"creation_time_100ns":987654321}}
{"op":"publish"}
{"op":"compressor","source":1,"enabled":false}
{"op":"publish"}
{"op":"stop"}
```

Source IDs 由 `started` / `status` / `add` 返回；这些命令是语法示例，应按实际 source ID 发送。源添加、替换、移除和 Compressor 编辑先 staged，再显式 `publish`；一个 candidate pending 时另一图编辑返回 Busy，先轮询 `plan_applied` 或 `plan_rejected`。`cancel` 请求取消 pending candidate，提交与取消竞态以终态回执为准。`gain` / `pan` 的 `Accepted` 是暂收，须按 `request_id` 等待 `parameter` 的 `Applied` / `AppliedLate` 或拒绝终态。`devices`、`processes` 只读查询；Process add/replace 必须使用列表中的 PID + 创建时间，过时身份返回明确错误，已退出 process 不能通过 `restart` 自动绑定复用 PID。物理输入可在同一 selection 上显式 restart。

JSONL 输出包含 `started`、带输入序号的 `ack`、parameter/plan 终态、`status`、`startup_failed` 和 `final`。`final` 保存 capture/render 原生报告、bridge XRUN/填充/时钟补偿、输出 peak/timeline/blocks/frames/segments 以及停机回收的终态。Bridge underrun/dropped 计数与原生设备故障分别报告；原生空 padding 不单独视为确定的 XRUN。source `staged` / `running` / `disabled` / `stopped` / `target_exited` / `failed` / `removed` 保留在注册表，`last_start_failure` 解释失败的替换尝试而不抹掉仍在运行的输入状态。

UI 使用 `host::windows::HostSession` 的纯控制接口：`start(SessionOptions)`、`add_source` / `replace_source` / `restart_source` / `remove_source` / `stop_source` / `enable_source`、gain/pan/Compressor、`submit_parameter`、typed `GraphCommand`、`publish` / `cancel_pending`、`poll`、`snapshot` / `runtime_snapshot` / `source_snapshots` / `graph_snapshot`、设备/进程目录和 `stop`。正常接口不暴露 Engine、DemandRenderer 或 WASAPI 对象。UI 应在自己的控制工作线程执行准备、轮询和 join，再把数据快照交给界面。`start` 仅在观察到实际 native Start 且初始图确认后返回 Running；partial startup 错误在控制侧停止回收全部 owner。`stop` 在 render join 返回 Engine 与 reader 后调用 `finish_parts`，不用额外 audible block 就能回收旧图和未来参数回执；Drop 同样负责停机回收。

`HostSession` 实现不依赖 GUI。普通回归测试使用软件源执行真实 DemandRenderer/Compiler/plan 控制，并只向 native API 传入无效 selection，不打开有效音频设备：

```powershell
cargo test --locked -p moiren-app
cargo clippy --locked -p moiren-app --all-targets -- -D warnings
```

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
