# Signal Engine and UI Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让任意逻辑输出端口和 processor 内部状态进入通用总线，在真实 Slint 页面按展示需求读取，并保证换图连续和 RT 零分配。

**Architecture:** Engine 持有稳定 RtHost，ExecutionPlan 持有 prepared 绑定表。每次 process/observe 增加独立 ProcessSignals 权限；compiler 在对应输出产生后安装只读观察。AppService 负责目录和 pump，UI 只读自己的业务订阅。

**Tech Stack:** Rust 2024、moiren-signal（计划 B）、现有 compiler/runtime/swap、Slint 1.18.1 和计划 A 的 AppService。

## Global Constraints

- 依赖 [应用计划 A](2026-10-10-application-runtime.md) 和 [signal 计划 B](2026-10-10-signal-crate.md) 的已验证接口。
- ProcessContext 保持 Copy 时间数据；ProcessParameters 保持独立；ProcessSignals 不可 Clone/Send/Sync，不得逃出调用。
- RT 零分配、零扩容、零堆内存释放、无锁/阻塞/日志/名称解析/Any/UI callback。
- 任意输出 PortId 可观察；观察不改变音频值、fan-out、延迟或 Bus/Pan 语义。
- 同一稳定 host 内换图复用同一 Topic 和实际 producer/槽；全候选验证失败保持当前音频及信号绑定。
- UI 可见页面、组件 visible、viewport 相交且窗口 visible && !minimized 才持业务订阅；全体恢复等待新启用区间。
- 第一版 UI 输入/输出逐声道 peak/RMS；内部压缩量通过示例及测试接入，不新增压缩器或波形产品页。

---

## 文件与责任

| 文件 | 责任 |
| --- | --- |
| engine/src/signal/{mod,process,plan}.rs | ProcessSignals 包装、每操作 prepared 表和 host/plan 校验 |
| engine/src/compiler/observers.rs | output PortId 验证与 observer 操作插入 |
| engine/src/meter/signal.rs | 逐声道窗口聚合与 LatestBlock 发布，保留旧 meter API |
| engine/src/processor/builtin/compressor.rs | 从真实 reduction_db 发布可选内部状态 |
| engine/src/runtime.rs、runtime/swap.rs | stable host、块开始 demand、事务式权限交接与 retire |
| app/src/service/signals.rs、src/ui/signals.rs | 服务注册/pump/typed 订阅请求与展示 token |

### Task C1: scoped ProcessSignals 与输出端口观察器编译

**Files:**

- Create: `crates/moiren-engine/src/signal/mod.rs`, `process.rs`, `plan.rs`, `src/compiler/observers.rs`。
- Modify: `Cargo.lock`, `crates/moiren-engine/Cargo.toml`, `src/lib.rs`, `src/processor/mod.rs`, `src/compiler.rs`, `src/compiler/bindings.rs`, `src/compiler/plan.rs`, `src/runtime.rs`, `src/boundary.rs`, `src/meter.rs` 与现有所有 RtProcessor/RtObserver 实现和测试实现。
- Test: `crates/moiren-engine/tests/compiler/observers.rs`，在 `tests/compiler/main.rs` 注册模块；`tests/runtime/signals.rs`，在 `tests/runtime/main.rs` 注册模块。

**Interfaces:**

```rust
pub trait RtProcessor<S: ProcessingSample>: Send {
    fn process(&mut self, ctx: &ProcessContext, io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>, signals: ProcessSignals<'_>);
    fn signal_bindings(&self) -> &[moiren_signal::ErasedBinding] { &[] }
}
pub trait RtObserver<S: ProcessingSample>: Send {
    fn observe(&mut self, ctx: &ProcessContext, inputs: ReadPorts<'_, S>,
        signals: ProcessSignals<'_>);
    fn signal_bindings(&self) -> &[moiren_signal::ErasedBinding] { &[] }
}
```

上述是已有 trait 的新增/修改项，其余 role/validate/state_type/latency API 原样保留。signal_bindings 只在非 RT prepare/snapshot 中调用，不在 render 做类型解析。

- `ProcessSignals<'a>` 包装 `Option<SignalScope<'a>>`；提供 `demand<P>(&PublisherHandle<P>) -> Result<DemandSnapshot, PublishError>`、`publish<P: ValueShape>(&mut self, &PublisherHandle<P>, P::Value) -> Result<PublishOutcome, PublishError>`、`try_write<P: BlockShape>(&mut self, &PublisherHandle<P>, usize) -> Result<BlockWriteGuard<'_, P::Header, P::Element>, WriteError>`、`publish_slice<P: BlockShape>(&mut self, &PublisherHandle<P>, P::Header, &[P::Element]) -> Result<PublishOutcome, PublishError>`。guard 生命周期绑定 &mut self。
- 无 signal 的旧图使用 crate 内 `ProcessSignals::empty(call: &'a mut ())`，借用当前栈调用 token 并带 PhantomData<Rc<()>>；此时 demand 返回 inactive，publish 返回 InactiveBinding，不通过 Default 构造可写权限。
- `NodeBindings::observe_output(&mut self, PortId, impl RtObserver<S> + 'static) -> Result<(), CompileError>`；observer 只读取该 output 的音频，不要求额外逻辑 node。
- `compile_with_signals<S>(graph: &LogicalGraph, bindings: NodeBindings<S>, config: CompileConfig, host: &HostControl) -> Result<CompiledGraph<S>, CompileError>`；现有 compile 保持无 signal 的入口，若 bindings 含 signal 而没 host 则明确拒绝。
- `CompiledGraph` 的 engine 携带已准备 SignalPlan，无 active host；`Engine::attach_signal_host(&mut self, RtHost) -> Result<(), SignalAttachFailure>` 仅非 RT 首次调用，SignalAttachFailure 持有原 host 与错误。候选有 SignalPlan 但不 attach 第二个 host。

- [ ] 用自定义 observer 写编译测试：源输出同接两个消费者，只安装一个 observer；实际读取源而不是某条 Send 之后的音频；wrong direction/不存在 PortId/重复 Topic owner/host 缺失全部拒绝。无观察与有观察输出逐 sample 相等。
- [ ] 更新 trait 和 Observer/SourceAdapter/SinkAdapter；Observer<O>::signal_bindings 必须转发 self.0.signal_bindings，否则 compiler 无法发现 observer 的 publisher。没有 signal 的 DSP 参数名用 `_signals`。processor 的业务运行不能因无观察而被跳过；只有额外测量通过 demand 决定。
- [ ] 编译顺序明确落在 producer op 与 edge/consumer op 之间：

```rust
ops.push(node_op);
for observer in output_observers {
    // Read the producer's exact output slot before any consumer can overwrite it.
    ops.push(observer_op(observer, produced_output_slot)?);
}
```

`observer_op` 在 observers.rs 定义，使用现有 PreparedIo/PortAccess 创建一个输入、零输出的只读 Observer 实例；PortId→slot 解析在 compile 阶段完成。输出方有多个端口时分别按准备映射插入；对同一端口多个不同 Topic 允许多个 observer，同一 Topic 的重复有效 owner 拒绝。

- [ ] ExecutionPlan::prepare 遍历 signal_bindings 构建每操作 OpSignalBindings，HostControl::prepare_update 校验 publisher 唯一性和类型/容量。Engine 外层 render 开始只调用一次 RtHost::begin_block；参数分段每次构造 ProcessSignals，使用同一块 demand 快照。SignalPlan 对当前 host identity 与 binding table identity 的检查在准备阶段完成，RT 留必要 constant-time 许可检查。
- [ ] 新增 engine→signal path dependency 后先执行一次 `cargo check -p moiren-engine` 更新 lock；之后更新所有 trait 消费者并执行 `cargo check --locked --workspace`、`cargo test --locked -p moiren-engine --test compiler --test runtime`，预期原测试和新观察测试通过。提交 `feat: bind scoped signals to graph output observers`。

### Task C2: 逐声道电平与真实 compressor 内部状态

**Files:**

- Create: `crates/moiren-engine/src/meter/signal.rs`, `examples/signal_internal_state.rs`, `tests/compiler/signal_measurement.rs`。
- Modify: `crates/moiren-engine/src/meter.rs`, `src/processor/builtin/compressor.rs`, `src/compiler/bindings.rs`, `tests/compiler/main.rs`, `src/processor/tests/compressor.rs`。
- Test: `crates/moiren-engine/tests/runtime/signals.rs`。

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioWindow {
    pub timeline_epoch: u64,
    pub frame_start: u64,
    pub frame_end: u64,
    pub sample_rate: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelLevel { pub peak: f32, pub rms: f32 }
pub struct SignalLevelMeter {
    channels: usize,
    window_frames: usize,
    accumulated_frames: usize,
    frame_start: u64,
    generation: u64,
    peaks: Box<[f64]>,
    squares: Box<[f64]>,
    publisher: moiren_signal::PublisherHandle<
        moiren_signal::LatestBlock<AudioWindow, ChannelLevel>>,
    bindings: Box<[moiren_signal::ErasedBinding]>,
}
```

- `SignalLevelMeter::new(channels: usize, window_frames: usize, PreparedPublisher<LatestBlock<AudioWindow,ChannelLevel>>) -> Result<Self, MeterError>`；MeterError 为 InvalidChannels/InvalidWindow/CapacityMismatch。准备时确认 Topic capacity == channels，窗口非零。
- `NodeBindings::bind_compressor_with_signal(node: NodeId, settings: CompressorSettings, PreparedPublisher<LatestValue<f32>>) -> Result<(), CompileError>`。
- Compressor 可选 typed publisher + erased bindings；Compressor::signal_bindings 返回准备表。原有 bind_compressor 不注册 signal，功能不变。

- [ ] 准備 2 channel、4 frame 的电平测试，输入 L=`1,0,-1,0`、R=`0.5,0.5,0.5,0.5`，断言 L peak=1、R peak=0.5、R rms=0.5、L rms=√0.5；一次 4 frame 与 1+3 参数分段得到同窗口/值/帧区间。
- [ ] 聚合只在 demand.active 时执行，换 activation generation 或业务 timeline_epoch discontinuity 重置额外窗口；无需求直接返回，正常 DSP 继续。逐 frame 积累 peak 和 sum squares；满 window 在预分配写槽提交，header frame_end 为排他上界。

```rust
for channel in 0..channels {
    let sample = input.channel(channel)[frame].to_f64();
    peaks[channel] = peaks[channel].max(sample.abs());
    squares[channel] += sample * sample;
}
// Only at a completed producer-owned window:
for channel in 0..channels {
    levels[channel] = ChannelLevel {
        peak: peaks[channel] as f32,
        rms: (squares[channel] / window_frames as f64).sqrt() as f32,
    };
}
```

使用现有 ProcessingSample::to_f64，`levels` 来自 BlockWriteGuard；累积数组只在 new 分配。窗口跨参数分段保持，demand 暂停间隙不累计；发布失败只影响遥测、不改变 DSP。

- [ ] Compressor 在已完成一段 DSP 后发布当前 `reduction_db as f32`，输出单位为正向 dB 衰减；先检查 demand，不新增独立测量。测试用 processor 模块内部可见的实际 reduction_db 对照，不从输入输出比猜测。证明 f32 负载不需要音频 header。内部示例开启 compression、订阅 Latest、运行有限 blocks 后打印真实衰减。
- [ ] 现有 LevelMeter/MeterReader 保留兼容，但新 AppService 不再读旧 meter queue；老 observer signature 适配新 signals 参数。`cargo test --locked -p moiren-engine --test compiler --test runtime`、`cargo run --locked -p moiren-engine --example signal_internal_state`；预期 sample 数值不变，电平和压缩量真实且无需求时额外计数为零。提交 `feat: publish channel levels and compressor reduction`。

### Task C3: 块边界信号权限与图切换的事务式交接

**Files:**

- Modify: `crates/moiren-engine/src/runtime.rs`, `src/runtime/swap.rs`, `src/signal/plan.rs`。
- Test: `crates/moiren-engine/tests/runtime/signals.rs`, `swap.rs`, `allocation.rs`, `support.rs`。

**Interfaces:**

- 保留 `Engine::enable_plan_switching`、`PreparedPlan::new/with_reuse`、`PlanControlPort::publish/cancel_pending/poll_retired` 公共语义。
- PlanSnapshot 增加 immutable signal schemas、actual retained binding 身份及 host identity；PreparedPlan 转移表增加已准备 SignalPlan/PreparedHostUpdate。RetiredPlan 增加旧 binding/update 的非 RT 所有权，不负责关闭新图复用的 Topic。
- `RetiredPlan::reject_pending()` 继续保留所有被接受参数的终态；signal 退场与参数回复满容量的现有保护共存。
- `Engine::into_parts(self) -> EngineParts<S>`，EngineParts 为 `{ plan: ExecutionPlan<S>, resources: RtResources<S>, parameters: ParameterRuntime, signals: Option<RtHost> }`；仅停止后非 RT 拆解。现有 drop(engine.into_parts()) 消费者无需语义改动，swap.rs 的 tuple 解构改为读取 `.resources`。全部 ownership 一起返回，不能遗漏 signal host。

- [ ] 加真实换图测试：同 Topic 旧/new processor ID 不同，换图前已有 Latest 和两个 Stream queued；切换后 subscription 不重建、原缓冲地址相同、seq 连续、queued 顺序不变；旧 Latest 在新图首发前完整可读。
- [ ] Signal candidate 与 processor reuse 一起准备。复用 processor 若保留其旧 typed handle，新执行表必须授权实际留下的 handle/binding，而不是新 candidate processor 的假想句柄；从 basis PlanSnapshot 准备最终表。old OpSignalBindings 属于旧 table identity，激活后不能创建有效 scope；复用对象只在新 active scope 内使用其保留的句柄。
- [ ] 现有 apply_pending_plan 中先保留 retire slot、核对取消/basis/config/epoch 与 signal update；全部通过后才执行所有权交换。RT 的 commit 区间只交换预分配对象、切换 active permissions 和更新 snapshot/revision，任何验证失败都返回原 candidate 到 retire：

```rust
// The exact variable owners are prepared before render starts.
std::mem::swap(&mut active_plan, &mut candidate_plan);
std::mem::swap(&mut active_parameters, &mut candidate_parameters);
std::mem::swap(&mut active_signal_plan, &mut candidate_signal_plan);
// The stable RtHost stays with the active Engine; its endpoints are unchanged.
```

以上 swap 前先取得 B5 的 `ActivationTransaction`；所有 plan/reuse 校验和 retire 预留已完成后，先 transaction.commit，再执行无失败的字段交换。transaction 的借用确保校验后 host 不会变化，commit 不含新验证或失败分支。任何替换出的 update 都装入已保留的 RetiredPlan，而不是局部 drop。signal host 独立于 plan/resource/parameter 字段，不能随整个 candidate Engine 交换；停止后的 EngineParts 返回该 host 供非 RT 回收。

- [ ] 加取消/错误 Topic/类型/模式/容量/duplicate owner/stale basis/retire full/新 Topic slot 满的拒绝测试，验证当前音频与 current binding 都保持原状。Busy 覆盖 candidate、active 和未退场的旧图；退场最终完成后注销成功。
- [ ] 在现有 TLS allocator 测试追加上述成功/拒绝路径与新 Topic 的预备 endpoint 激活，assert alloc/realloc/dealloc 为零。DropProbe 同时检查 processor、binding table 与 endpoint 最终在非 RT cleanup 线程销毁；native COM 不跨 owner。
- [ ] `cargo test --locked -p moiren-engine --test runtime`、`cargo clippy --locked -p moiren-engine --all-targets -- -D warnings`；预期现有自动化背压/ramp/换图与新 signal 契约全通过。提交 `feat: preserve signal publishers across prepared plan swaps`。

### Task C4: AppService pump、typed 订阅和 UI 展示需求

**Files:**

- Create: `crates/moiren-app/src/service/signals.rs`, `src/ui/signals.rs`, `tests/signal_visibility.rs`。
- Modify: `Cargo.lock`, `crates/moiren-app/Cargo.toml`, `src/service/{mod,model,core,jobs,windows}.rs`, `src/ui/{mod,bindings,window}.rs`, `src/monitor.rs`, `src/tone.rs`, `ui/backend.slint`, `ui/main.slint`, `ui/pages/monitor.slint`, `render.slint`。
- Test: `crates/moiren-app/tests/service_backend.rs`, `tests/monitor.rs`, `tests/tone.rs`。

**Interfaces:**

- 注册稳定名 `monitor/input/output/level`、`monitor/output/level`、`tone/output/level`；类型为 LatestBlock<AudioWindow,ChannelLevel>。跨图 revision 不入名称，源真正重绑定采用同 Topic，不伪造 generation。
- 启动前 service 注册并 create_host，准备任务取得 HostControl/Topic 与预绑定访问；首次 engine attach 唯一 RtHost 后交给 output owner。后台 session 与 cleanup 保活 signal 根，UI 不拥有 RtHost。
- `AppHandle::subscribe_topic<P: Shape>(&self, Topic<P>, SubscriptionOptions) -> Result<SubscriptionTicket<P>, DispatchError>`；ticket `try_take(&mut self) -> Result<Option<Subscriber<P>>, SubscriptionDeliveryError>`，错误包括总线 SubscribeError 和 Disconnected；泛型请求用非 RT boxed closure 在 service 操作 Bus，响应通道容量 1。
- `PresentationDemand { window_visible, minimized, page_active, component_visible, intersects_viewport }`；`needs_subscription(self) -> bool`。`VisibleSignal<P>` 持有可取消 ticket/Subscriber，`update_demand(&mut self, PresentationDemand)` 在状态变化时建立/释放；drop 释放自己的 demand。
- 未完成 ticket 的取消必须有独立标记；service 在建订阅前检查，若刚建完响应端已取消/断开则 drop 新 subscriber，不能留下一份无人消费的业务需求。

- [ ] 写纯 Rust 展示判定及 ticket 取消测试，覆盖每个条件：

```rust
#[derive(Debug, Clone, Copy)]
pub struct PresentationDemand {
    pub window_visible: bool,
    pub minimized: bool,
    pub page_active: bool,
    pub component_visible: bool,
    pub intersects_viewport: bool,
}
impl PresentationDemand {
    pub fn needs_subscription(self) -> bool {
        self.window_visible && !self.minimized && self.page_active
            && self.component_visible && self.intersects_viewport
    }
}
```

测试 hide→show 都发生在两块之间，producer 仍观察到新 demand generation；持续显示的两次 timer 之间不重复订阅。分析 subscriber 留存时 UI Drop 不暂停来源。

- [ ] 在 AppService 既有循环末尾加入有界 pump，采用 B4 类型：

```rust
if signals.has_business_demand() && now >= next_signal_poll {
    let report = signals.pump(PumpBudget {
        topic_checks: 32, deliveries: 64, copy_bytes: next_copy_budget,
    });
    next_copy_budget = (256 * 1024).max(report.required_copy_bytes);
    next_signal_poll = now + std::time::Duration::from_millis(5);
}
```

next_copy_budget 不超过当前已注册的最大单消息成本与配置上限；注册时确保可接受的最大 payload，不允许无界一次复制。control/applied/retire/exit 在 pump 前处理。无业务需求不轮询 ingress，但继续管理工作。

- [ ] 在 prepare_monitor/prepare_tone 的 signal-aware 入口保存 input/output PortId 并 observe_output；窗口长度默认 1536 frames（48 kHz 下 32 ms），cap=实际 channel 数。GUI 拿两个声道的完整同一 AudioWindow 读数更新已有 StereoMeter；没有新数据显示等待、停会话清零本地显示，不能发假零值 Topic 消息。
- [ ] MainWindow 的 ScrollView 使用 Slint 1.18.1 的 `sv.content-y`、`sv.visible-height`（viewport-y 已废弃）。页面暴露 meter 在页面坐标内的 top/height，再加 page 在 content 中的位置与 content-y，计算 viewport 相交：`top < visible-height && top + height > 0px`。对横向亦计算 left/width，随后与所有祖先 clipping 相交；暴露 Backend 的每区域 demand bool，合并 page conditional/组件 visible/window state。滚动到仅半个 meter 时仍 active，完全移出才退订。
- [ ] UI Timer 每 33 ms 检查 `window().is_minimized()`、Rust 维护的 visible 和最新快照，读取自己的 Latest guard 后立即释放；隐藏/切页显式立即释放，OS 最小化最多延迟一轮 timer。不可见不刷 signal，保留轻量恢复状态检测；不做 OS 遮挡检测，不从 RT 唤醒 UI。
- [ ] 服务停止后丢弃的后台 owners/HostUpdate 全部进入 A 的 cleanup；若订阅尚在显示，NoPublisher 状态单独提示而非把旧 payload 当持续更新。用户主动停止时 UI 清读数并退订；重启恢复新代次，真实 graph swap 则保留持续业务订阅。
- [ ] 新增 app→signal path dependency 后先执行一次 `cargo check -p moiren-app` 更新 lock；再执行 `cargo test --locked -p moiren-app`、`cargo check --locked --workspace`。运行 GUI 与低音量显式硬件会话，验证真实 L/R、电平随 gain/pan、最小化/隐藏/切页/滚动暂停、恢复新窗口。用非 RT debug stats 检查 demand 和 publication 计数，debug 不进入 RT 日志。
- [ ] 查看运行截图并操作 tray、两页面与退出；完成 [总计划](2026-10-10-application-and-signal.md) 的最终门槛。提交 `feat: display engine signals only for visible UI subscriptions`。
