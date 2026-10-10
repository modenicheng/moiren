# Application Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将现有 Slint 原型接入真实目录、监听/测试音、参数与 tray，并使启动和退出保持 UI 可响应。

**Architecture:** 主线程拥有窗口和 tray，AppService 拥有业务状态和 ControlPort。准备任务返回尚未激活的 session，service 校验 generation 后激活；join/大对象销毁由有界 worker 执行。参数状态和调度继续在输出 owner 的 Engine 上运行。

**Tech Stack:** Rust 2024、Slint 1.18.1、windows 0.62.2、std::sync::mpsc、std::thread；复用现有 rtrb =0.4.0 参数通道。

## Global Constraints

- 无参数线程、独立 tray 线程、async runtime 或新 parameter crate。
- 主线程 1、AppService 1、每输出时钟域 1 个输出 owner、每捕获流按后端 1 个输入 owner；任务 worker 最多 2。
- GUI 第一版同时运行一个监听或测试音会话，保持现有音频格式限制；设备更换先停止回收旧会话。
- UI try_send；队列满返回 Busy。退出使用独立标记，不能因队列满丢失。
- 不在 UI/AppService 热循环 drop 或 join 含 JoinHandle 的 session；COM 仍在原 owner 上释放。
- 参数公共 API、Accepted/Applied、帧排序、ramp、事件预算、回复背压及换图拒绝语义保留。
- 用户现有 UI 工作区必须合并，禁止覆盖；本计划不新增产品功能或改主题布局。

---

## 文件与责任

| 文件 | 责任 |
| --- | --- |
| engine/src/control.rs、control/{schema,ramp,channel,runtime,view}.rs | 保留 control 门面，拆开 schema、ramp、提交、RT 调度和只读视图 |
| windows-audio/src/duration.rs、session/gate.rs | 明确持续时长；准备后的激活 gate 与取消 |
| app/src/service/{mod,model,mailbox,core,jobs,windows}.rs | 启动入口、DTO/快照、边界、状态机、有界 pool、真实后端适配 |
| app/src/ui/{mod,bindings,window}.rs、ui/backend.slint、ui/tray.slint | Slint model/callback、窗口/tray 生命周期；不持有后端 session |
| app/src/main.rs、monitor/windows.rs、tone.rs | 保留 CLI 分发；暴露可准备会话与已有图参数映射 |

路径前缀分别是 `crates/moiren-engine/`、`crates/moiren-windows-audio/` 与 `crates/moiren-app/`。任务 Files 列表使用完整路径。

### Task A1: 拆分参数职责，保持行为与公共 API

**Files:**

- Modify: `crates/moiren-engine/src/control.rs`。
- Create: `crates/moiren-engine/src/control/schema.rs`, `ramp.rs`, `channel.rs`, `runtime.rs`, `view.rs`。
- Test: 现有 `crates/moiren-engine/tests/runtime/automation.rs`, `swap.rs`, `allocation.rs`，不添加照抄实现的测试。

**Interfaces:** 保留公开 `control::{ParamDomain, ParamSpec, FloatRamp, ControlPort, ParameterRuntime, ProcessParameters, ControlError, parameter_channel}`；ParameterBindings 仍为 pub(crate)，不扩大公开面。`ControlPort::submit(&mut self, ParameterRequest, u64) -> ControlReply`、`poll_applied(&mut self) -> Option<ControlReply>` 及 RT/retire 方法签名保持现状。内部事件与状态的可见性限制为 `pub(super)`。

- [ ] 运行 `cargo test --locked -p moiren-engine --test runtime`，保存分拆前基线。
- [ ] 按表移动现有定义，不修改方法体；门面采用以下真实 Rust 布局：

```rust
mod channel;
mod ramp;
mod runtime;
mod schema;
mod view;

pub use channel::{ControlPort, parameter_channel};
pub use ramp::FloatRamp;
pub use runtime::ParameterRuntime;
pub use schema::{ControlError, ParamDomain, ParamSpec};
pub use view::ProcessParameters;
pub(crate) use view::ParameterBindings;
```

`ScheduledEvent`、参数状态与 reply ring 的内部定义由 runtime/channel 的实际消费者决定 `pub(super)`；不可为了模块移动把内部对象变成公开 API。注释只解释回复容量、块快照和旧队列退场等约束。

- [ ] 每移动一个责任模块执行 `cargo check --locked -p moiren-engine`；完成后执行 runtime 测试与 `cargo test --locked -p moiren-engine --test compiler`，预期原有全部测试通过。
- [ ] 显式暂存 control.rs 与新 control/ 文件，提交 `refactor: separate engine parameter responsibilities`。

### Task A2: 持续运行、准备 gate 与非 RT 会话回收

**Files:**

- Create: `crates/moiren-windows-audio/src/duration.rs`, `src/session/gate.rs`, `tests/duration.rs`。
- Modify: `crates/moiren-windows-audio/src/lib.rs`, `session.rs`, `capture.rs`, `capture/wasapi.rs`, `capture/wasapi/stream.rs`, `process_loopback.rs`, `process_loopback/stream.rs`, `render.rs`, `render/wasapi.rs`, `render/wasapi/stream.rs`。
- Modify: `crates/moiren-app/src/monitor/windows.rs`, `main.rs` 的现有 CLI options 构造；`crates/moiren-windows-audio/tests/capture.rs`, `render.rs`, `process_loopback.rs` 的时长参数。
- Modify: `crates/moiren-engine/src/runtime.rs` 的停止后控制清理入口。
- Test: `crates/moiren-windows-audio/src/render/wasapi/tests.rs`, `capture/wasapi/tests.rs`。

**Interfaces:**

- `SessionDuration::{UntilStopped, For(Duration)}`，`validate(self) -> Result<(), DurationError>`，`remaining(self, elapsed: Duration) -> Option<Duration>`，`requested_seconds(self) -> Option<f64>`。
- 现有 CaptureOptions/RenderOptions/ProcessLoopbackOptions/MonitorOptions 的 duration 改为 SessionDuration；CLI 显式构造 For。
- Windows `ActivationGate::new(stop: StopSignal) -> Result<Self, GateError>`；非阻塞 `activate(&self) -> Result<(), GateError>`、`cancel(&self) -> Result<(), GateError>`；owner `wait(&self) -> Result<Activation, GateError>`，Activation 为 Start/Cancelled。GateError 为 API stage/HRESULT 或 AlreadyResolved。
- `start_render_prepared(options: RenderOptions, renderer: DemandRenderer, stop: StopSignal) -> Result<PreparedRenderSession, RenderError>`：只在 worker 调用，等待设备 Ready 后返回；`PreparedRenderSession::activate(&self)`、`request_stop(&self)`、`into_session(self) -> RenderSession`。
- `prepare_capture_with_gate(options: CaptureOptions, stop: StopSignal, gate: ActivationGate) -> Result<PreparedCapture, CaptureError>`、`prepare_process_capture_with_gate(options: ProcessLoopbackOptions, stop: StopSignal, gate: ActivationGate) -> Result<PreparedCapture, CaptureError>`；握手只准备格式、桥和 native stream，AudioClient::Start 留到 gate Start。
- `start_render_prepared_with_gate(options: RenderOptions, renderer: DemandRenderer, stop: StopSignal, gate: ActivationGate) -> Result<PreparedRenderSession, RenderError>`：监听会话传同一个 gate 给 capture/render；单独测试音的 start_render_prepared 自建 gate。
- 原有 start_render/start_render_with_stop、start_capture/start_process_capture 作为“prepare 后立即 activate”的便捷入口保留；新增 `start_capture_with_stop` 支持物理捕获准备取消。
- owner 返回 `RenderOwnerExit { report: RenderReport, renderer: DemandRenderer }`；`RenderSession::join_with_renderer(self) -> Result<RenderOwnerExit, RenderError>` 供 GUI cleanup 使用，`join(self) -> Result<RenderReport, RenderError>` 保留 CLI 便利入口并在 join 调用方释放 renderer。native COM 仍在 owner 返回前释放。
- `DemandRenderer::retire_controls(&mut self) -> usize` 转发停止后的 `Engine::retire_controls(&mut self) -> usize`，返回仍未终结的请求数；只在 owner 已 join 后调用。该入口继续用已有 StaleRevision 表示已退役 runtime 的未应用请求，不新增协议回复码。

- [ ] 添加独立时长测试，预期新类型尚不存在而失败：

```rust
use moiren_windows_audio::{DurationError, SessionDuration};
use std::time::Duration;

#[test]
fn unlimited_run_has_no_artificial_deadline() {
    let limit = SessionDuration::UntilStopped;
    assert_eq!(limit.validate(), Ok(()));
    assert_eq!(limit.remaining(Duration::from_secs(3600)), None);
    assert_eq!(limit.requested_seconds(), None);
    assert_eq!(SessionDuration::For(Duration::ZERO).validate(),
               Err(DurationError::OutOfRange));
    assert_eq!(SessionDuration::For(Duration::from_secs(601)).validate(),
               Err(DurationError::OutOfRange));
}
```

- [ ] 实现时长值域和明确剩余时间：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDuration { UntilStopped, For(std::time::Duration) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationError { OutOfRange }

impl SessionDuration {
    pub fn validate(self) -> Result<(), DurationError> {
        match self {
            Self::UntilStopped => Ok(()),
            Self::For(d) if (std::time::Duration::from_secs(1)
                ..=std::time::Duration::from_secs(600)).contains(&d) => Ok(()),
            Self::For(_) => Err(DurationError::OutOfRange),
        }
    }
    pub fn remaining(self, elapsed: std::time::Duration)
        -> Option<std::time::Duration> {
        match self {
            Self::UntilStopped => None,
            Self::For(d) => Some(d.saturating_sub(elapsed)),
        }
    }
    pub fn requested_seconds(self) -> Option<f64> {
        match self {
            Self::UntilStopped => None,
            Self::For(d) => Some(d.as_secs_f64()),
        }
    }
}
```

owner wait 超时采用 remaining 的有界转换，UntilStopped 使用原生 INFINITE 并同时等待 stop。限时在激活之后开始计时，不将 worker 准备时间算成运行时间。CaptureReport/RenderReport 的 requested_seconds 改为 Option；分别基于现有 schema_version 加 1，更新对应报告测试和 README。

- [ ] Gate 原子状态限定 `Prepared=0, Activated=1, Cancelled=2`；activate/cancel 用 compare_exchange(Prepared, target)。成功后 SetEvent；失败明确返回 AlreadyResolved。owner wait 同时等 gate 与 stop，stop 索引优先；cancel 即使 gate 已 Activated 也必须 signal 共享 stop。原生初始化失败通过固定容量 Ready 结果返回，未启动音频前允许阻塞握手。Monitor 的两个 owner 等同一个 gate，各自在观察 Start 后开始计 duration；取消准备不能留下仍在捕获或播放的有限会话。
- [ ] 在现有 Windows owner 测试中新增真实事件 gate 测试：Ready 后未 activate 不产生 render；cancel 先于 activate 时永不运行；失败/取消都 join；用 DropProbe 记录纯 Rust renderer 在调用 join 的线程析构。测试 owner 使用可注入的 prepare/run 函数，不要求音频硬件。
- [ ] 停止后控制清理使用现有非 RT retire/reject_pending 逻辑，不丢已 Accepted 的未来事件：

```rust
pub fn retire_controls(&mut self) -> usize {
    self.parameters.retire();
    self.parameters.reject_pending()
}
```

清理调用方先 poll 已有 Applied，再 retire/reject，若回复满则继续 poll 后重试，直到返回 0。renderer 及 ControlPort 在这之前不得析构。为停止时回复队列已满添加测试，断言每个已接受 request_id 有且只有一个最终 Applied/AppliedLate/StaleRevision。
- [ ] `cargo test --locked -p moiren-windows-audio` 和 `cargo test --locked -p moiren-app`；预期 CLI 的范围校验与旧有限会话测试仍通过。提交 `feat: prepare cancellable continuous audio sessions`，只包含本任务明确文件及报告文档。

### Task A3: 平台无关 AppService 状态机与有界调度

**Files:**

- Create: `crates/moiren-app/src/service/mod.rs`, `model.rs`, `mailbox.rs`, `core.rs`, `jobs.rs`。
- Modify: `crates/moiren-app/src/lib.rs`。
- Test: `crates/moiren-app/tests/service.rs`, `tests/service_jobs.rs`。

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionGeneration(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunLimit { UntilStopped, Seconds(u16) }
#[derive(Debug, Clone, PartialEq)]
pub enum InputSelection {
    Physical { endpoint_id: String },
    Process { pid: u32, creation_time_100ns: u64 },
    Tone { frequency_hz: f64 },
}
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSpec {
    pub source: InputSelection,
    pub output_endpoint_id: String,
    pub limit: RunLimit,
    pub gain: f64,
    pub pan: f64,
    pub max_block_frames: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase { Idle, Starting, Running, Stopping, Failed, Exiting, Exited }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError { Busy, Exiting, Disconnected, InvalidConfig, GenerationExhausted }
#[derive(Debug, Clone, PartialEq)]
pub enum AppRequest {
    Start(SessionSpec), Stop, RefreshCatalog,
    SetGainPan { gain: f64, pan: f64 },
}
```

- `ServiceCore::default()`、`start(&mut self, SessionSpec) -> Result<SessionGeneration, DispatchError>`、`stop(&mut self) -> Result<SessionGeneration, DispatchError>`、`may_activate(&self, SessionGeneration) -> bool`、`activated(&mut self, SessionGeneration) -> bool`、`owner_started(&mut self, SessionGeneration) -> bool`、`owner_reaped(&mut self, SessionGeneration) -> bool`、`begin_exit(&mut self)`、`phase(&self) -> SessionPhase`。core 分开记录 desired generation 与 Option<owner generation>；已激活到完全 join 完成之间 owner 始终存在，不能用新 desired 覆盖。
- `AppHandle::try_request(&self, AppRequest) -> Result<(), DispatchError>`、`request_exit(&self)`、`try_snapshot(&self) -> Option<Arc<AppSnapshot>>`。
- `AppSnapshot { generation, phase, desired: Option<SessionSpec>, error: Option<String>, catalog: Arc<DeviceCatalog>, control: ControlSummary, elapsed: Duration }`；DeviceCatalog 是平台无关输入/输出/进程行 DTO，ControlSummary 只记录有界的待应用状态与最后结果，elapsed 从实际 owner Running 确认起计算。
- `JobPool::new() -> io::Result<Self>`、`try_submit(&mut self, JobPriority, Box<dyn FnOnce() + Send>) -> Result<(), DispatchError>`、`reap_finished(&mut self)`；priority 为 Prepare/Cleanup。结果通道由 service 提供，每个任务必须提交其 generation 和拥有的结果。
- `ServiceRuntime` 持有 service JoinHandle；`handle(&self) -> AppHandle`、`join(self) -> Result<(), ServiceJoinError>`；ServiceJoinError 为 ThreadPanicked。UI 只在 Exited 后 join。

- [ ] 添加不依赖 Slint 或硬件的状态测试：

```rust
use moiren_app::service::{DispatchError, InputSelection, RunLimit,
    ServiceCore, SessionPhase, SessionSpec};

fn tone_spec() -> SessionSpec {
    SessionSpec {
        source: InputSelection::Tone { frequency_hz: 440.0 },
        output_endpoint_id: "test-output".into(), limit: RunLimit::UntilStopped,
        gain: 0.05, pan: 0.0, max_block_frames: 256,
    }
}
#[test]
fn stopped_or_superseded_start_never_gets_permission_to_play() {
    let mut core = ServiceCore::default();
    let first = core.start(tone_spec()).unwrap();
    let second = core.start(tone_spec()).unwrap();
    assert!(!core.may_activate(first));
    assert!(core.may_activate(second));
    core.stop().unwrap();
    assert!(!core.may_activate(second));
    core.begin_exit();
    assert_eq!(core.phase(), SessionPhase::Exiting);
    assert_eq!(core.start(tone_spec()), Err(DispatchError::Exiting));
}
```

- [ ] `cargo test --locked -p moiren-app --test service`，确认新模块缺失失败；实现 DTO 校验和状态转换。开始/停止 generation 使用 checked_add；保存 desired，Starting 不等同 Running；stop/cancel 会让旧任务失去许可。owner_started 只接受当前 generation 且 desired 仍为 Running 的确认。
- [ ] 增加真实 owner 占用的状态测试：activated(first) 后 owner_started(first)，再 start(second) 必须进入 Stopping，may_activate(second)=false；只有 owner_reaped(first) 后进入 Starting 并允许 second。清理旧 session 的回调依据 owner generation 处理，不因 desired 已变成 second 而被误判为无用结果。旧准备结果的 orphan cleanup 不调用 owner_reaped。
- [ ] 实现 mailbox 与 pool 的固定上限：请求 64、每 worker 队列 1、结果 8、service 暂存 16；最多一个 Prepare 占 worker，另一个保留 Cleanup。快照 `Arc<Mutex<Arc<AppSnapshot>>>` 只在 service 短临界区替换，UI 用 try_lock 复制 Arc 后立刻释放；退出独立 AtomicBool + 非 RT unpark。UI 不向 worker 发送任何业务请求。
- [ ] 新增有界性测试：填满 64 请求后下一次 Busy，退出标记仍到达；用 barrier 卡住 Prepare，Cleanup 仍能完成；用线程 ID 验证陈旧/失败结果被 worker drop；UI 不读取快照时后台 generation 和完成结果仍推进。barrier/可控时钟用于确定顺序，不用长 sleep 猜测并发。
- [ ] `cargo test --locked -p moiren-app --test service --test service_jobs`；预期状态、退出、清理公平性测试通过。提交 `feat: add bounded application service runtime`。

### Task A4: 服务接入现有 Windows 后端与参数

**Files:**

- Create: `crates/moiren-app/src/service/windows.rs`。
- Modify: `crates/moiren-app/src/service/{mod,model,core,jobs}.rs`, `src/monitor/windows.rs`, `src/monitor.rs`, `src/tone.rs`。
- Test: `crates/moiren-app/tests/service_backend.rs`, `tests/monitor.rs`, `tests/tone.rs`。

**Interfaces:**

- `start_service() -> Result<ServiceRuntime, ServiceStartError>`，ServiceStartError 为 WorkerSpawn(io::Error)/UnsupportedPlatform；Windows 启动 service + pool，非 Windows 明确返回 UnsupportedPlatform。
- `prepare_monitor_with_stop(options: MonitorOptions, stop: StopSignal) -> Result<PreparedMonitorSession, MonitorError>`；process 对应 prepare_process_monitor_with_stop。PreparedMonitorSession 持有已准备 capture、未激活 render、ControlPort 和 CompiledBindings。
- `PreparedMonitorSession::activate(&self) -> Result<(), MonitorError>`、`request_stop(&self)`、`into_session(self) -> MonitorSession`；准备时把同一 ActivationGate 交给 A2 的 capture/render gate 入口，activate 仅解析该 gate 一次。
- service 私有 `PreparedAppSession::{Monitor(PreparedMonitorSession), Tone(PreparedToneSession)}` 和 `ActiveAppSession`；分别统一 activate、request_stop、is_finished、control、join。PreparedToneSession 由既有 prepare_tone + start_render_prepared 组成，不新增音频源系统。
- `BackendAdapter` 是 service 内部 trait，`prepare(&self, SessionSpec, Cancellation) -> Result<Box<dyn PreparedSession>, BackendError>`、`catalog(&self) -> Result<DeviceCatalog, BackendError>`。PreparedSession/ActiveSession 是非 RT session 适配 trait，真实 PreparedAppSession 枚举与测试 fake 各实现；不把 fake 硬塞进 Windows enum。Cancellation 是非 RT 可复制取消 token，真实实现包 StopSignal，fake 包 AtomicBool；BackendError 为 `{ stage: &'static str, reason: String }`。
- `PreparedSession::activate(&self) -> Result<(), BackendError>`、`into_active(self: Box<Self>) -> Box<dyn ActiveSession>`、`cancel(&self) -> Result<(), BackendError>`、`join(self: Box<Self>) -> Result<SessionReport, BackendError>`；ActiveSession 提供 request_stop/is_finished/control/poll_started/join，control 返回 `Option<(&mut ControlPort, &CompiledBindings)>`，poll_started 返回 Starting/Running/Failed 标量状态；join 的 self 也是 Box<Self>，返回 SessionReport。两 trait 都 Send，所有调用都在非 RT。
- SessionReport 为 `{ status: SessionEnd, error: Option<String>, control_replies: Vec<ControlReply> }`，SessionEnd 为 Completed/Stopped/TargetExited/Failed；完整现有 Windows 报告另保留在非 RT 诊断适配中。control_replies 在 worker 预分配容量 64，对应 service 最多 64 个尚未终结 Accepted 请求，不运行无界积累。`ServiceCore::start_failed(&mut self, SessionGeneration, BackendError)` 只影响当前 generation。

- [ ] 建 fake backend 测试：准备返回时无音频推进；当前 generation 才 activate；旧结果进入清理；任一 owner 失败会停止对端；Requested stop 的 join 在 worker；不更改真实设备音量/默认设备。
- [ ] 按以下控制顺序实现真实适配，generation 判断与 join 所有权不可省略：

```rust
enum CompletionDecision {
    None,
    Activated(Box<dyn ActiveSession>),
    Cleanup(Box<dyn PreparedSession>),
}
fn accept_prepared(core: &mut ServiceCore, generation: SessionGeneration,
    completed: Result<Box<dyn PreparedSession>, BackendError>) -> CompletionDecision {
    match completed {
        Err(error) => {
            core.start_failed(generation, error);
            CompletionDecision::None
        }
        Ok(prepared) if !core.may_activate(generation) =>
            CompletionDecision::Cleanup(prepared),
        Ok(prepared) => match prepared.activate() {
            Ok(()) => {
                assert!(core.activated(generation));
                CompletionDecision::Activated(prepared.into_active())
            }
            Err(error) => {
                core.start_failed(generation, error);
                CompletionDecision::Cleanup(prepared)
            }
        },
    }
}
```

Activated 仍保持 Starting，直到 poll_started 确认 Running。service 只在无旧 owner 时派发新会话 prepare；旧 owner 的 join 结果调用 owner_reaped 后再按最新 desired 派发。Cleanup 对象进入固定有界待清理槽，派发前为每个 prepare/active session 保留 cleanup credit；不能在队列满时直接 drop。完成结果不使用 `?` 提前释放 prepared。完整所有权分支由 fake 的 DropProbe 验证。

- [ ] 目录任务复用 catalog::snapshot 与 inspect_process；传给 UI 的都是 owned DTO，不携带 COM。进程行保存 PID + creation time；输出/输入按 endpoint ID 选中，刷新不以旧行索引猜测身份。枚举与准备都占 Prepare 限额，不抢 Cleanup。
- [ ] AppService 唯一持有活动 ControlPort。SetGainPan 校验有限值与范围后，对当前 CompiledBindings 的 Gain::LEVEL/Pan::POSITION 构造 ParameterRequest；revision/epoch 来自当前计划。尚未 Accepted 的期望滑杆值可合并；QueueFull 保持待提交状态；Accepted 请求保持有界 ledger，Applied/失败回复到达后才能结束。服务每轮至多 poll 32 个回复。
- [ ] 停止时把 session 与其 ControlPort 一并交给 cleanup worker。worker join_with_renderer 后调用 A2 的 retire_controls，并交替 drain 控制回复，完整终结剩余 Accepted 请求；最终回复通过 SessionReport 回到 service 更新 ledger，然后才释放 renderer/图。退出仍检查 ledger 已终结，不能仅因音频 owner 已退出就宣称清理完成。
- [ ] 频率沿用当前固定 SineSource：运行时变更 frequency 视为重新准备会话，经过同一停止/generation/gate 流程；gain/pan 通过参数通路。不偷偷增加 RtAudioSource 参数接口。
- [ ] `cargo test --locked -p moiren-app --test service_backend --test monitor --test tone`；Windows 手工以显式 endpoint 启动、停止并观察 service Running/Failed，单独记录实机证据。提交 `feat: route audio sessions and parameters through app service`。

### Task A5: 接入 Slint model、主窗和同线程 tray

**Files:**

- Create: `crates/moiren-app/src/ui/mod.rs`, `bindings.rs`, `window.rs`, `ui/backend.slint`, `ui/tray.slint`。
- Modify: `crates/moiren-app/src/main.rs`, `build.rs`, `ui/main.slint`, `ui/pages/monitor.slint`, `render.slint`, `devices.slint`, `settings.slint`。
- Test: `crates/moiren-app/tests/ui_lifecycle.rs`；Slint 渲染/交互实测。

**Interfaces:**

- Rust `ui::run() -> anyhow::Result<()>`；main 保留 CLI 分发，GUI 入口调用 run；generated Slint types 放在 ui 模块，避免 main/lib 各 include_modules 两份类型。
- `Backend` global：输入/输出/进程 model、phase、error-text、gain/pan、elapsed；actions 为 start-monitor/start-tone/stop/refresh-catalog/set-gain-pan；DTO 沿用现有 AudioEndpoint/ProcessSource 字段并补稳定 ID。
- root MainWindow 保留 page、minimize/toggle-maximize/close-window；`AppTray` callbacks show-window/stop-session/exit-app；root tray 使用自己实例的 Backend 状态。
- `WindowPresentation` 纯 Rust 状态记录 visible/minimized/page；`hide_to_tray` 只改变展示，`request_exit` 调 service 退出。C4 在此基础接入真实订阅。

- [ ] 修改 model/callback 前核对 Slint 1.18.1，使用 slint 技能；对当前页面逐个建立 Rust callback 映射，保留用户主题与布局。
- [ ] backend.slint 新建 BackendPhase 为 idle/starting/running/stopping/failed/exiting/exited，避免沿用 mock 的四态 SessionState。输入/输出 ID 用原 endpoint String；进程选择用包含精确 PID/creation-time 的稳定字符串 key 或 Rust model 的原始 DTO，不能通过 Slint float 传递 u64 creation time。运行时间来自 elapsed，尚无真实 live 数据的统计显示“—”，删除硬编码 Mock 延迟、xrun、漂移和运行时间。
- [ ] 新增 tray 组件，真实 Slint 内容如下；在 main.slint import 并 re-export AppTray，使现有一次 build.rs 编译生成两种 root：

```slint
export component AppTray inherits SystemTrayIcon {
    icon: @image-url("icons/app.svg");
    tooltip: "Moiren";
    in property <bool> session-active;
    callback show-window();
    callback stop-session();
    callback exit-app();
    clicked => { root.show-window(); }
    Menu {
        MenuItem { title: "打开 Moiren"; activated => { root.show-window(); } }
        MenuItem {
            title: "停止音频"; enabled: root.session-active;
            activated => { root.stop-session(); }
        }
        MenuSeparator { }
        MenuItem { title: "退出"; activated => { root.exit-app(); } }
    }
}
```

- [ ] 将按钮的本地 `running = !running`/`playing = !playing` 替换为 service 请求和 phase 展示；Starting/Stopping 禁止重复动作。设备控件改用实际 model，空目录与失效选择禁用启动。音频真实电平在 C4 接入前显示无数据，不把原 mock 值当运行读数。
- [ ] 主窗与 tray 创建于主线程，隐藏保留 service，事件循环改为显式退出：

```rust
let window = MainWindow::new()?;
let tray = AppTray::new()?;
let weak = window.as_weak();
tray.on_show_window(move || {
    if let Some(window) = weak.upgrade() {
        window.window().set_minimized(false);
        let _ = window.show();
    }
});
window.show()?;
tray.show()?;
slint::run_event_loop_until_quit()?;
```

在完整 run 中保存 service、tray、UI Timer 到 loop 结束；窗口系统 close 与自定义 close-window 都走 hide_to_tray。service Exited 后才 quit_event_loop 并 join 完成线程。UI 异常返回也先 request_exit，清理对象由 worker 路径承接；不能 detached 仍有音频的 service。

- [ ] 33 ms UI Timer 用 AppHandle::try_snapshot 更新两实例；不逐 block invoke_from_event_loop。测试窗口隐藏/重建不改变后端状态，以及满普通请求时退出仍到达；不为纯属性转发写重复测试。
- [ ] `cargo check --locked -p moiren-app`、`cargo test --locked -p moiren-app`。运行 GUI 查看截图并交互验证真实目录、空列表、Starting/Failed、关闭到 tray、恢复、退出；截图保存在 `.tmp/`。预期窗口隐藏后会话继续、显式退出后进程与 owner 全部结束。
- [ ] 显式暂存上述 UI/适配文件，提交 `feat: connect Slint and tray to application service`；用户未要求的 UI 原稿和设计资产保持原状态。
