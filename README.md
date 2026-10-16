# Moiren

A real-time audio graph for Windows.

Moiren 是 Rust 2024 音频工作区。桌面界面使用 gpui-kit，Graph、Mixer 和 Matrix 读取同一份真实 `LogicalGraph`。正式 Windows 路径使用 `EngineHost` 管理图编译、兼容热更新、资源等待和控制侧回收。

## 运行

```powershell
cargo run --locked -p moiren-app
# 离线音频演示，不打开设备
cargo run --locked -p moiren-app -- --gain 0.25
```

Graph 中的每个 Source、Sink 独立绑定音源或设备，多个输入可通过 Bus 混合到多个输出。首个输出驱动主时钟，其他输出通过有界时钟桥跟随。Compatible 图修改在块边界更新；改变设备或 IO 集合可能重建音频 owner。界面区分候选与引擎确认的活动版本。

Settings 中的 Compatible、Balanced、Low 保存期望的延迟档位，通过 Apply 生效。实际设备周期、队列和初始化 fallback 来自原生观测。请求 5 ms 不代表设备实际为 5 ms，队列帧数也不代表物理端到端时延。

支持项目保存/加载、物理输入、进程树捕获、Gain、Balance/Mono/Swap、Bus、Compressor、发送参数和真实电平。正式 native 边界仍为 WASAPI Shared、双声道 graph、48 kHz 输出；物理输入接受 44.1/48 kHz 单/双声道 f32。

## 验证

```powershell
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo test --locked -p moiren-app --features gpui-test
cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test -- -D warnings
cargo fmt --all -- --check
```

普通测试不打开音频设备。`routing_probe` 提供有限时长、全输出静音的多输出/低延迟实机检查，不改变系统默认设备或音量：

```powershell
cargo run --locked -p moiren-windows-audio --example routing_probe -- --list
cargo run --locked -p moiren-windows-audio --example routing_probe -- --output '<master endpoint ID>' --output '<follower endpoint ID>' --seconds 10 --profile low
```

可重复指定 `--input '<capture endpoint ID>'`；不指定时混合两个软件音源。硬件验证与普通测试分别记录。实时路径的图、buffer 和信号绑定均在控制侧准备；音频线程不进行堆分配/回收、阻塞等待或 COM 重建。

## 当前边界

- Process Loopback 是 Application Capture / Monitor；真正的逐应用 Redirect / Takeover 尚未解决。独占探针保持独立实验。
- SRC 仍是线性插值基线。没有承诺专业重采样质量、多输出延迟补偿、物理端到端时延或长期无 XRUN。
- 正式 Exclusive、ASIO、多通道、虚拟音频驱动、CLAP/VST3、Named Pipe 和远程控制仍属后续能力。
- EQ/Gate/Limiter/Delay/Recorder 和标准 LUFS/True Peak 尚未补齐；PreFader 目前只支持 Gain 或已绑定 channel strip 的节点。

## 文档

- [托管路由与低延迟](docs/designs/14-managed-routing-low-latency.md)
- [本轮验证记录](docs/superpowers/plans/2026-10-16-managed-routing-low-latency-validation.md)
- [EngineHost 所有权与更新](crates/moiren-engine/README.md)
- [Windows 音频后端、探针与硬件边界](crates/moiren-windows-audio/README.md)
- [桌面应用与 CLI](crates/moiren-app/README.md)
- [产品与路线图](docs/Moiren-PRD-Roadmap.md)（早期进度段落以当前实现和本轮验收记录为准）
- [Graph Compiler](docs/designs/05-graph-compiler.md)、[IO 节点](docs/designs/03-io-nodes.md)、[Compressor 与 Plan swap](docs/designs/06-compressor-plan-swap.md)
