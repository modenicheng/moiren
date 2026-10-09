# Audio Backend Implementation Plan

> **For agentic workers:** Use inline execution task-by-task in this session. Steps use checkbox syntax for tracking. The user has requested planning followed by immediate audio-backend implementation; no additional design approval is required.

**Goal:** 先交付单物理 `Capture → Graph → Render` 闭环，再以独立切片接入 Process Loopback 和多设备。

**Architecture:** Capture owner 保有 WASAPI/COM，Render 为主时钟。backend Clock Bridge 负责固定容量缓冲、SRC 与漂移补偿，通过现有 `RtAudioSource<f32>` 绑定 Graph，不增加 UI 拓扑或设备对象到 core/engine。

**Tech Stack:** Rust 2024、windows 0.62.2、rtrb 0.4.0、现有 Graph Compiler/Engine、serde 标量报告。

## Global Constraints

- 首切片单物理输入、单物理输出；输出 native 48 kHz stereo f32 Shared。
- 输入 native 44.1/48 kHz mono/stereo f32；其他格式明确拒绝。
- Capture/Render owner 不共享 COM；RT 无分配、释放、锁、日志或等待另一线程。
- SILENT、DATA_DISCONTINUITY、TIMESTAMP_ERROR、overflow/underrun 计数与处理均可观察。
- 不改 endpoint/session 音量、mute、默认设备；硬件测试显式选择 endpoint。
- 本轮不实现 GUI，Figma 原型由用户推进。
- 详细约束见 [设计](../specs/2026-10-09-physical-capture-clock-bridge-design.md)；实机结果与仿真结果分别记录。

## 后端主线

| 阶段 | 交付 | Gate |
| --- | --- | --- |
| A，已交付 `f72b6fc` | Physical Capture、最小 Clock Bridge、CLI 闭环与控制侧报告 | 自动检查与 FreeDSP 10 秒实机通过，用户确认听感/延迟 |
| B，已实现 | Process Loopback 复用同一 ingress/bridge，include-process-tree，PID + creation-time identity | activation 生命周期测试、指定测试进程/子进程捕获与退出通过；冷启动短读单独记录 |
| C | 多输入、Bus、SRC 质量升级、双小时跨钟压力 | 漂移无持续积累、已知回流可诊断后才开放多输出 |
| D | 设备失效/重新绑定、epoch/generation、睡眠恢复与长期状态接口 | 故障不会错误绑定，停止无需 master 音频事件 |

## Task 1: 有界 Capture ingress 与 Clock Source

**Files:** Create `crates/moiren-windows-audio/src/clock_bridge.rs`, `src/clock_bridge/{ingress,source,telemetry}.rs`, `tests/clock_bridge.rs`; modify `src/lib.rs`, `Cargo.toml`, `Cargo.lock`。

**Interfaces:**

```rust
pub fn capture_bridge(config: ClockBridgeConfig)
    -> Result<(CaptureIngress, ClockSource, BridgeObserver), ClockBridgeError>;
impl CaptureIngress {
    pub fn push_packet(&mut self, bytes: &[u8], packet: CapturePacket)
        -> Result<usize, ClockBridgeError>;
}
impl ClockSource {
    pub fn read_interleaved(&mut self, output: &mut [f32])
        -> Result<BoundaryReport, ClockBridgeError>;
}
// ClockSource also implements RtAudioSource<f32>; BridgeObserver::snapshot()
// returns copied scalar telemetry and never transfers audio/COM ownership.
```

- [x] 写行为测试：mono `1,-1` 映射为 stereo `1,1,-1,-1`；SILENT 不读取 pointer；错误 packet 不改变 bridge；ramp 的 44.1→48 输出等于连续输入相位；欠载尾部为零并重预填充；溢出丢尾部且 generation 不跨断点插值。
- [x] `cargo test --locked -p moiren-windows-audio --test clock_bridge`，先确认缺少上述接口导致失败，再实现。
- [x] ingress 做完整 frame 发布、有限采样校验和 packet metadata；source 保有两帧 lookahead 与连续相位、fill PI；telemetry 只有 atomic 标量，所有内存准备时分配。
- [x] 仿真 120 秒的 ±1000 ppm 输入，加有界 packet jitter；断言无启动后短读/丢帧，fill 有界且 correction 符号正确。allocation counter 覆盖 ingress + source。
- [x] 运行目标 crate 测试和 Clippy，检查 phase 与 correction 不依赖 Engine segment 的任意切分。

## Task 2: 正式物理 Capture owner

**Files:** Create `src/capture.rs`, `src/capture/wasapi.rs`, `src/capture/wasapi/{endpoint,format,packet,stream,tests}.rs`, `src/session.rs`; modify `src/catalog.rs`, `src/lib.rs`, `src/render.rs`, `src/render/wasapi.rs`, `src/render/wasapi/stream.rs`。

**Interfaces:**

```rust
pub fn list_capture_endpoints() -> Result<Vec<CaptureEndpoint>, CaptureError>;
pub fn start_capture(options: CaptureOptions)
    -> Result<PreparedCapture, CaptureError>;
// PreparedCapture owns session, source and observer, plus resolved input rate/channels.
// CaptureSession exposes request_stop(), is_finished(), join().
```

- [x] fake capture client 测试 GetBuffer/ReleaseBuffer 完整 lease、SILENT/null、无效 packet 仍释放、zero packet 不释放；kernel event 验证 stop 赢过 audio。
- [x] owner 验证 endpoint flow 与 native float format，按原生 format 构造 bridge；启动握手返回纯 Rust 数据。初始化失败、控制端消失和线程 panic 都关闭 worker。
- [x] 实机统计发现启动积压与停机竞态后，加入预填充旧帧丢弃、积分抗饱和、欠载位置诊断及 `CaptureSession::stop_signal` / `start_render_with_stop`；两端在停止时直接广播共同 kernel event，再进行原生清理。
- [x] 每次 audio wake 排空 packets，但在 packets 之间检查 stop 与 duration，不无限追赶 producer；Stop HRESULT 与原失败分别保留。
- [x] `cargo test --locked -p moiren-windows-audio` 与严格 Clippy。不在 `cargo test` 打开实际设备。

## Task 3: 应用装配与 monitor 命令

**Files:** Create `crates/moiren-app/src/monitor.rs`, `src/monitor_cli.rs`, `tests/monitor.rs`; modify `src/lib.rs`, `src/main.rs`, 两个 crate README。

**Interfaces:**

```rust
pub fn prepare_monitor(source: impl RtAudioSource<f32> + 'static,
    config: MonitorConfig) -> Result<MonitorGraph, MonitorError>;
pub fn parse_monitor_args(args: impl IntoIterator<Item = String>)
    -> Result<MonitorCommand, MonitorCliError>;
// MonitorGraph returns CompiledGraph, output reader, gain_node, pan_node.
// Windows runner coordinates both owners and reports capture/render/bridge.
```

- [x] 真实 Compiler/Engine 离线测试 `Capture source → Gain → Pan → Sink`，检验数值、timeline 与控制端参数 binding；CLI 拒绝缺 ID、重复、NaN、时长越界及混合 list。
- [x] CLI 使用显式 input/output IDs，Capture 初始化成功后编译并启动 Render。两端共享停止事件，先取消 peer 再 Stop/Release；每 10 ms 在控制侧检查 owner 完成并 join，最终输出独立报告。
- [x] `cargo run --locked -p moiren-app -- monitor --help` 和 `monitor --list`；list 只枚举，不启动麦克风。文档提供显式 ID 的闭环命令与线性 SRC 限制。

## Task 4: 验证与交付记录

**Files:** Update 本计划；Create `docs/experiments/windows/2026-10-09-capture-clock-bridge.md`。

- [x] `cargo fmt --all -- --check`、`cargo test --locked --workspace`、`cargo clippy --locked --workspace --all-targets -- -D warnings`、`git diff --check` 全部通过。
- [x] 记录仿真结果、read-only 设备能力和未执行的实机验收：指定设备闭环试听、30 分钟、两小时 drift、拔插/睡眠。
- [x] 自查 thread/COM/lease Drop 顺序、RT allocation、capture 提前失败传播、stale generation、queue 预算；修复后只重跑相关检查与最终 workspace 检查。
- [x] 保留可审阅的 feature branch 与明确文件清单；用户确认听感和延迟后，按独立功能切片原子提交。

## 本次执行结果

A 切片代码与文档已完成；FreeDSP 麦克风 → FreeDSP 耳机、gain 0.05 的 10 秒闭环经停机竞态修复后为 0 欠载、0 溢出、0 重置。用户确认“听感与延迟都还行”，主观验收通过；未测量端到端毫秒延迟。44.1/48 kHz × ±1000 ppm 四组各两小时模拟时钟测试通过。B 已接入正式应用闭环，验证与限制见 [Process Loopback 计划](2026-10-09-process-loopback.md)。C/D、实机长期漂移和故障恢复仍待验证。完整数字及异常保留在本地 [验收记录](../../experiments/windows/2026-10-09-capture-clock-bridge.md)，该目录按仓库规则忽略。
