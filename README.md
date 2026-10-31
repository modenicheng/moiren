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

Settings 中的 Compatible、Balanced、Low 保存期望的延迟档位，保存后下次启动生效。档位映射为设备 buffer 请求（Balanced ≈ 512 帧、Low ≈ 256 帧 @ 48 kHz，无显式配置时），不是队列模型；实际设备周期和初始化 fallback 来自原生观测，请求值不代表实测延迟。档位语义的 canonical 描述见[托管路由与低延迟](docs/designs/14-managed-routing-low-latency.md)。

后端节点覆盖 Input/Output、Gain、Balance/Mono/Swap、Bus、Compressor、EQ、Gate、Limiter、Delay、ChannelMapper、Recorder、Meter、SpectrumAnalyzer 和 Rack（多端口、显式无分支链、slot 稳定身份兼容换图保留 DSP 状态）。完整清单与逐节点能力边界见[后端节点验收](docs/validation/backend-nodes.md)；桌面编辑入口尚未覆盖所有后端节点。

正式 native 边界仍为 WASAPI Shared、双声道 IO、48 kHz 输出；中间 DSP 的端口宽度独立于原生 IO 限制。mono/stereo PCM/float 格式经 Windows 转换保持声道宽度，多声道与非常规 mask 明确拒绝。该边界的 canonical 描述见 [windows-audio README](crates/moiren-windows-audio/README.md)。

## 验证

```powershell
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo test --locked -p moiren-app --features gpui-test
cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test -- -D warnings
cargo fmt --all -- --check
```

普通测试不打开音频设备。有限时长的实机探针（`asio_probe`、`exclusive_probe`、W00 系列）与硬件验证记录见 [windows-audio README](crates/moiren-windows-audio/README.md)；这些探针不改变系统默认设备或音量。硬件验证与普通测试分别记录。实时路径的图、buffer 和信号绑定均在控制侧准备；音频线程不进行堆分配/回收、阻塞等待或 COM 重建。

## 当前边界

- Process Loopback 是 Application Capture / Monitor；真正的逐应用 Redirect / Takeover 尚未解决。独占探针保持独立实验。
- ProfessionalBackend 已接入 WASAPI Shared/Exclusive 和单驱动立体声 ASIO，支持 f32/f64 Engine；专业路径使用预计算 windowed-sinc SRC，旧 Clock Bridge 路径仍保留原有算法。没有承诺多输出延迟补偿、物理端到端时延或长期无 XRUN。
- ASIO 已有能力查询、原生回调诊断和有界通知恢复；FL Studio ASIO 能力查询成功，但持续 IO 与正常停止验收尚未通过，不代表原生声卡已验证。任意多通道路由、多 ASIO 驱动并用、虚拟音频驱动、CLAP/VST3、Named Pipe 和远程控制仍属后续能力。
- EQ 当前为单段 peaking band；Limiter 为零前视 sample-peak；Meter 提供 Sample Peak/RMS，标准 LUFS/True Peak 尚未实现。Recorder 使用有界队列，托管文件线程自动排空和 finalize；慢磁盘造成的整块丢帧、写入失败和最终完成状态可查询。
- Rack 的内置 slot 为单输入流/单输出流，不包含插件 sidechain/multi-bus kernel。PreFader 支持 Gain 或已绑定 channel strip 的节点，并按各输出端口独立取 tap。

## 文档

- [托管路由与低延迟](docs/designs/14-managed-routing-low-latency.md)（其验证记录原在
  superpowers 计划目录，该目录已于 2026-10-31 从仓库移除，见 git 历史）
- [EngineHost 所有权与更新](crates/moiren-engine/README.md)
- [后端节点验收与能力边界](docs/validation/backend-nodes.md)
- [Windows 音频后端、探针与硬件边界](crates/moiren-windows-audio/README.md)
- [桌面应用与 CLI](crates/moiren-app/README.md)
- [产品与路线图](docs/Moiren-PRD-Roadmap.md)（早期进度段落以当前实现和本轮验收记录为准）
- [Graph Compiler](docs/designs/05-graph-compiler.md)、[IO 节点](docs/designs/03-io-nodes.md)、[Compressor 与 Plan swap](docs/designs/06-compressor-plan-swap.md)
