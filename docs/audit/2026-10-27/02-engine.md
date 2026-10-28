# moiren-engine 审计报告

> 子区域：`crates/moiren-engine`（134 文件 / ~34000 行）。方法：全部 107 个 src 文件（~20150 行非测试）逐文件读——processor/builtin 全部 12 个内核、rack、compiler 全部 6 个子模块、runtime/swap+mix、host 全部 13 个文件、control/buffer/meter/spectrum/recorder/boundary；tests 结构化浏览+抽样；对照 moiren-signal / moiren-core / moiren-app 验证消费者。所有 dead-code 结论均经全 workspace grep 验证。

## 1. 执行摘要（最重要的重叠）

1. **两条电平表实现并存，其中队列版已被信号版全面支配**（EN-1）：每个图节点（含 Meter 节点自己）都由 `prepare_levels` 挂了 demand-gated `SignalLevelMeter`；`NodeConfig::Meter` 又额外挂无条件累计的队列版 `LevelMeter`——同一流每块测两次，且 app 只消费 `subscribe_level`，`meter_reader` 全 workspace 零生产调用。
2. **观察/记录读出端 4 组 API 无生产消费者**（EN-1/EN-2）：`subscribe_compressor`、`subscribe_spectrum`、`meter_reader`、`recorder_reader`（含整套 WAV worker ~470 行）只被 engine 自己的测试调用，而 app 节点目录却能创建 Recorder/Meter/Spectrum 节点——创建了 Recorder 节点会持续向无人排空的队列丢帧。
3. **host/project.rs 内部三重 ParamSpec 表 + 手写参数域**（EN-3）：`validate`/`validate_parameter`/`parameter_values` 三处独立重建同一 `[ParamSpec]` 表，且 Pan/Strip/Edge 的数值域手抄了一份 kernel 表中的域（当前值一致，属漂移风险最高的活重复）。
4. **kernel 构造知识在 rack 与 compiler 两处平行展开**（EN-5）：`rack/prepare.rs` 与 `compiler/plan.rs` 各自 match 出"构造 kernel + 生成 specs"，加上 `Delay::MAX_DELAY_MS` 静态约束在 3 处手写。
5. **Gain/Pan 双传输能力硬编码两处**（EN-6）：`compiler/plan.rs:378` 用 `NodeKind` 判、`runtime/swap.rs:547-551` 用 `TypeId` 判，同一事实两个表示。

## 2. 既往已知项核对表

| 项目 | 状态 | 证据 |
|---|---|---|
| gate/limiter `1-exp` vs compressor `-exp_m1` 平滑 | **FIXED** | 唯一实现 `dynamics.rs:34-40` `smooth()` 用 `-(-1000/(ms*sr)).exp_m1()`；gate.rs:116/118/138、limiter.rs:101、compressor.rs:171/176 全部调用它；回归测试 dynamics.rs:119-160（含 slow-ramp 几何衰减校验） |
| rack/prepare.rs 遗漏 delay MAX_BUFFER_BYTES 预检 | **FIXED** | rack/prepare.rs:45-49 先 `Delay::validate_settings(channels, *settings, sample_rate)?` 再 `Delay::new`；delay.rs:55-80 为单一来源，`new` 内部复验 delay.rs:87，双侧不可分叉 |
| process() Separate/InPlace 双臂 boilerplate | **部分收敛，仍开放（EN-4）** | FrameIo/SeparateFrames/InPlaceFrames（buffer/view.rs:150-216）已把 DSP 内核统一成单 `process_linked`；剩余每 kernel ~20 行 match 分发样板 ×7（grep 计 14 处 wrapper 构造） |
| 参数知识散布 4 处 | **数值域已单源，结构 match 仍平行（EN-3/EN-5）** | `float_specs`/`resolve_float`/`specs_accept_initials`（dynamics.rs:45-94、control/schema.rs:49-63）消除了数值漂移；但 NodeConfig→specs/kernel 仍有 ≥6 个 match 分布在 5 个文件，且 project.rs 存在手写域残留 |
| host/signals.rs(858) 是否重实现 moiren-signal | **结论：不是重复实现（分层正确）** | signals.rs 只调用 `bus.register_latest_block/register_latest_value/subscribe/unregister/pump`（signals.rs:276、328、391、549、585）；moiren-signal host/mod.rs:130-348 是所有权/预算/激活机制，signals.rs 是 NodeId→topic 目录+代际管理。engine 的 signal 模块（~190 行）是薄适配。残余内部重复见 EN-7 |
| runtime/swap.rs(1164) 平行分支 | **基本收敛，残余见 EN-6** | 兼容序已单源 `swap_rejection()`（swap.rs:436-451），被 publish(874)/RT apply(1038)/reuse basis(699) 三处共用；RT 提交流程已拆 4 个阶段函数；文件 1164 行中 56-418 是内嵌测试 |

## 3. 发现清单（按严重度）

### EN-1 队列版 LevelMeter 被 SignalLevelMeter 支配；Meter 节点双重测量
- **级别** P1（高价值可合并重复）｜**类别**：同一 DSP 原语两份实现 + dead 读出端
- **证据**：
  - 隐式路径：`host/signals.rs:311-372` `prepare_levels` 为 `project.graph.nodes()` 的**每个**节点挂 `SignalLevelMeter`（含 Meter 节点，359-362 还特意读取 Meter 的 window_frames）。
  - 显式路径：`host/preparation.rs:315-317` `NodeConfig::Meter` 又 `bind_meter` → `compiler/bindings.rs:319` 构造队列版 `LevelMeter`（meter.rs:62-180，rtrb 快照 + 无 demand 门控的无条件累计）。
  - 消费者：app 仅用 `subscribe_level`（`crates/moiren-app/src/service/managed/meters.rs:104`）；`meter_reader`/`ManagedMeterReader` 全 workspace 零生产调用（仅 engine tests/host/signals.rs）。
  - meter.rs:55-61 文档自认是 twin 并写明 "do not add new behavior here"。（主代理已亲证该注释。）
- **重叠详述**：两者都做 peak/rms 窗口累计（meter.rs:141-177 linked 跨通道 fold vs meter/signal.rs:123-150 per-channel 数组），输出语义可互相推导。隐式版 demand-gated、覆盖所有节点、信息严格更多（per-channel）。
- **建议**：删除 `NodeConfig::Meter` 的队列观察者路径：移除 `bind_meter`（bindings.rs:311-323）、`LevelMeter`/`MeterReader`/`level_meter`（meter.rs 约 380 行含测试）、`PreparedReaders.meters`、`SignalHub.meters` 及 `ManagedMeterReader`、`EngineObserver::meter_reader`（host.rs:331-336）、signals.rs 中 meters 槽位机制（signals.rs:247/261/283/445/483/491-506/652-654）。Meter 节点保留在图中（PassThrough + 隐式 level 信号已按其 window_frames 发布），`meter_reader(node)` 调用者迁移到 `subscribe_level(node)`。`meter/polling.rs` 的 MeterPollingTimer 同时服务信号泵，保留。
- **风险与验证**：LevelSnapshot 的 `dropped_snapshots`/`non_finite_samples` 统计字段没有信号版对应物——若无人消费即可删；tests/host/signals.rs、tests/host/recording.rs 需改写；`cargo test -p moiren-engine --test host`。app 端 meters.rs 已在信号路径上，无 UI 变化。

### EN-2 Recorder/Spectrum/Compressor-reduction 读出端无生产消费者
- **级别** P1（决策项：裁剪或接线）｜**类别**：无调用者的导出 + 平行保留
- **证据**（全部经全 workspace grep）：
  - `recorder_reader`/`record_wav`/`record_with_writer`/`RecordingSession`/`WavRecording`：零 app/windows-audio 调用；仅 engine tests/host/recording.rs。整套 recorder 家族 = recorder.rs(301)+wav.rs(206)+worker.rs(253)+三个 tests 文件(~700)+signals.rs 内 RecorderEndpoint/Lease/claim 机制(signals.rs:76-241)。
  - `subscribe_spectrum`/`ManagedSpectrumSubscription`：仅 tests/host/signals.rs；app 无。
  - `subscribe_compressor`/`ManagedSignalSubscription`（host.rs:313-318）：仅 tests/host/signals.rs:499；但 `preparation.rs:276-285` 为**每个** Compressor 节点无条件 `signals.prepare` topic + publisher + 编译绑定。
  - app 节点目录可创建这三类节点（`crates/moiren-app/src/ui/pages/node_catalog.rs:19-21,40-42`）。
- **重叠/问题详述**：Recorder 节点被创建后，其 SPSC 队列持续被 RT 侧填充、无人排空（dropped_frames 静默增长，recorder.rs:268-270）；compressor reduction 信号链路（compiler `Binding::Compressor(.., Option<PreparedPublisher>)`、`Compressor::with_signal`、signal_bindings、SignalHub topic 族、prune 分支）是只被测试点亮的完整第二套发布面。
- **建议**：二选一并落成工单：(a) app 接线（Recorder 面板/Spectrum 画布/压缩表显示）——则全部保留但 app 侧欠账要显式列出；(b) 暂时裁剪：删 `record_wav`/`record_with_writer`/worker（保留 Recorder 节点的 PassThrough 编译与 `RecorderSettings` 拓扑占位），删 `subscribe_compressor` 及其 per-node topic 准备（compressor.rs:96-104/205-207/260-266 的 signal 臂退化为 `Compressor::new`）。不建议半留：每条保留路径都要求测试持续维护。
- **风险与验证**：(b) 会改变 `Compressor::signal_bindings` 契约，tests/runtime/signals.rs、tests/host/signals.rs 大部需删改；`cargo test -p moiren-engine`。若近期有 UI 计划则选 (a)。

### EN-3 host/project.rs 三重 ParamSpec 构建 + 手写参数域（最高漂移风险）
- **级别** P1｜**类别**：同一逻辑多处实现
- **证据**：
  - 三重建表：`project.rs:227-275`（validate）、`project.rs:396-435`（validate_parameter）、`project.rs:506-568`（parameter_values）各自 match NodeConfig 并调 `X::parameters(ProcessorId(0), settings)`。
  - 手写域（与 kernel 表并存）：Pan 初始域 project.rs:232-234 与单参域 402-409 vs pan.rs:38-50 的 `ParamDomain::Float{-1,1}`；Strip 四参域 project.rs:450-461（含 `width.saturating_add(3)`）vs strip.rs:84-88 `mode_variants`(=channels+3) 与 strip.rs:93-131 的表；Edge 三参域 project.rs:463-481 vs send.rs:39-62 的表；`Delay::MAX_DELAY_MS` 手检 project.rs:248-250（另见 EN-5）。
- **重叠详述**：当前数值一致（已核对 pan/-1..1、strip/0..16、-1..1、mode+3、send gain>=0），但每处手写都是未来改域时的漏改点——这正是首轮整改想消除的"参数知识散布"的残留核心。
- **建议**（可直接开单）：
  1. project.rs 加私有 `fn node_specs(config: &NodeConfig, width_hint: usize) -> Vec<ParamSpec>`（内部路由各 kernel 的 `parameters(ProcessorId(0), ..)`），validate/validate_parameter/parameter_values 三处全部改走它 + `specs_accept_initials`/`accepts_parameter`（schema.rs:49-63 已备好）。
  2. Strip 域改走 `ChannelStrip::parameters(ProcessorId(0), width, config, true)` + `accepts_parameter`；删除 450-461 手写臂。Edge 域同理（`Send::parameters` 签名已带 channels，send.rs:22-26，直接可用）。
  3. 剩余不能走表的特殊规则（edge pan 的"非立体声仅允许 0"在表内已表达为 [0,0] 域，pan!=0 拒绝可完全由表推出）逐条核对后删除。
- **风险与验证**：纯控制侧重构，行为等价可由 tests/host/parameters.rs、tests/host/project.rs 锁定；注意 `validate_parameter` 的 Pan MONO/SWAP 布尔臂需保留（表已含 Bool 域，可统一）。

### EN-4 内置处理器 process() 双传输分发样板（7 处 × ~20 行）
- **级别** P2｜**类别**：结构性重复
- **证据**：gain.rs:72-88、pan.rs:166-186、compressor.rs:241-259、gate.rs:199-217、limiter.rs:144-166、eq.rs:172-194、delay.rs:235-257——完全同构的 `match io { Separate => process_linked(SeparateFrames…), InPlace => process_linked(InPlaceFrames…), ReadOnly => unreachable!() }`。
- **详述**：DSP 主体已单实现（FrameIo 单态化），重复的只是"取 0 号端口对 → 包 FrameIo → 调内核"。
- **建议**：提供宏 `linked_io!(io, kernel, params..)`（宏保持单态化、零动态分发，符合 view.rs:148-149 的约束），7 个 process() 各缩成 3-5 行，~130 行收益。
- **风险与验证**：宏的可读性代价是真实反证——若团队偏好显式 match，可降级为"新内核必须用宏"的约定。验证：tests/processor/tests/contracts.rs + 各 kernel 测试（含 in_place 两分支）。

### EN-5 kernel 构造 match 在 rack 与 compiler 平行；MAX_DELAY_MS 检查三处
- **级别** P2｜**类别**：平行结构
- **证据**：rack/prepare.rs:38-55 与 compiler/plan.rs:106-221 各自展开同一组构造 + 各自 push `X::parameters`；`Delay::MAX_DELAY_MS` 静态约束三处：delay.rs:60-62（canonical）、rack.rs:279-283、project.rs:248-250。
- **详述**：两处 match 的分叉点真实存在（compiler 侧 Compressor 可带 publisher、ChannelMapper 用 node 宽度，rack 侧统一 checked_parameters 预校验），但"给定 settings/sr/channels 构造 kernel"是同一知识。加一个新内置类型需改 ≥8 个落点。
- **建议**（保守版）：每个固定 schema 内核给出统一入口，如 `Delay::prepare(channels, settings, sr, id) -> Result<(Delay, Vec<ParamSpec>), ProcessorError>`（Eq/Gate/Limiter/Compressor 同构），两处 match 臂收敛为一行调用；MAX_DELAY_MS 检查收敛为 `DelaySettings::storage_valid(&self)`（delay.rs），另两处改调它。
- **风险与验证**：枚举穷尽性检查本来就强制各 match 同步更新，这是该重复的天然防线（反证）；收敛是降行数而非降漏改风险，故仅 P2。验证：tests/rack/tests/prepare.rs、tests/compiler/rack*.rs。

### EN-6 Gain/Pan "双传输等价"知识硬编码两处（NodeKind 与 TypeId 双表示）
- **级别** P2｜**类别**：知识散布
- **证据**：compiler/plan.rs:378 `in_place_candidate: !use_mix && matches!(node.kind(), NodeKind::Gain | NodeKind::Pan)`；runtime/swap.rs:547-551 `if instance.state_type == TypeId::of::<crate::processor::Gain>() || … Pan` 将 IoMode 归一化为 Separate 以便 reuse 比较。
- **详述**：同一事实在编译期用 NodeKind 判、在快照期用 TypeId 判。新增一个可 in-place 的内核必须同时记得改两处，否则 swap reuse 会误拒或误比。
- **建议**：`RtProcessor` 增加默认 `fn transport_interchangeable(&self) -> bool { false }`，Gain/Pan 覆写为 true；两处都改查该方法。
- **风险与验证**：trait 方法在 RT 之外调用，无性能顾虑；tests/runtime/swap.rs 的 reuse 用例 + tests/compiler/optimization.rs 的 in_place_count 断言。

### EN-7 SignalHub::prune 三段式重复 + 重复的错误映射惯用法
- **级别** P3｜**类别**：局部重复
- **证据**：signals.rs:613-628（topics）、629-639（spectra）、640-650（levels）三段完全同构的 collect-removed→unregister→remove；`.map_err(|error| BackendError::Failed(error.to_string()))` 模式 ≥9 次（356、358、364、368-369、388、413、416、557、559-560）。
- **建议**：prune 抽泛型 `unregister_stale` helper，三处变一行调用；错误映射加 `impl From<HostError> for BackendError` 或文件内 `fn failed(e: impl Display)`。`ManagedLevelSubscription`/`ManagedSpectrumSubscription`/`ManagedSignalSubscription` 同构三胞胎可合并为 `ManagedSubscription<T: Shape>`（保留类型别名）。
- **验证**：tests/host/signals.rs。

### EN-8 Compressor 的 validate_io 脱离 linked 家族（缺 MAX_CHANNELS 边界）
- **级别** P2（一致性）｜**类别**：平行结构中的偏差
- **证据**：compressor.rs:208-217 手写 `1-in/1-out/等宽` 校验；gate.rs:179-181、limiter.rs:114-117、eq.rs:144-147 用共享 `validate_linked_io`（dynamics.rs:99-110，多出 `channels != 0 || channels > MAX_CHANNELS` 两判）。四个同族内核中 compressor 独缺宽度上界：512 通道 Compressor 图节点可编译，512 通道 Gate 被拒。rack 路径下两者又都被 `valid_channels`（rack.rs:220-222）限制在 ≤256——同一 kernel 因入口不同而契约不同。
- **建议**：compressor.rs 改用 `validate_linked_io`（行为变化：>256ch Compressor 节点开始被拒，与 Gate/Limiter/EQ 一致）；或在 dynamics.rs 注释写明 compressor 无界的理由。倾向前者。
- **风险与验证**：tests/compiler/nodes.rs、tests/processor/tests/contracts.rs。

### EN-9 `Sum`/`Bus` 双名导出
- **级别** P3｜**类别**：卫生
- **证据**：builtin/mod.rs:27-28 同一 kernel 双导出；生产路径只用 `Bus`（compiler/plan.rs:220）；`Sum` 仅被 engine 自身单测使用。
- **建议**：保留一个名字，删另一别名，测试改用保留名。node_catalog.rs:105 的文案顺带统一。

## 4. 明确不建议合并项（附理由）

1. **engine host/signals.rs vs moiren-signal host/registry/dispatcher**：分层正确——signal crate 拥有传输/预算/激活所有权，SignalHub 拥有 NodeId→topic 目录与 owner 代际；目录层无一处分发逻辑复刻。
2. **swap 的 control publish / RT apply 双重兼容校验**：swap.rs:427-435 注释明示为纵深防御，且已共享 `swap_rejection` 单源，无重复实现。
3. **compile vs compile_reference 双编译路径**：compiler.rs:159-161 声明 reference 路径是优化 lowering 的等价性 oracle（tests/compiler/optimization.rs 等消费）。测试资产不是冗余。
4. **runtime/mix.rs 的 process_stereo 与通用通道循环**：stereo 臂携带 pan law 与双 tile 累加器，通用臂无 pan——语义不同，合并会摊平特化反而变慢。
5. **LevelMeter 与 SignalLevelMeter 的累计内核**（若 EN-1 不采纳）：linked 全通道 fold vs per-channel 数组，输出 schema 不同；meter.rs:57-61 已冻结队列版行为。若 EN-1 采纳则自然消解。
6. **delay.rs:49 与 recorder.rs:29 的 `MAX_BUFFER_BYTES`（同为 64 MiB）**：两个独立预算域恰好同值，各有 doc 理由；合并成共享常量会耦合两个本应独立调度的预算。
7. **RackProcessor 与 NodeConfig/Binding 两套配置枚举**：前者是可持久化 slot 配置，后者是 plan 绑定期意图。合并会混淆拓扑持久化与编译绑定两个生命周期；只应收敛其下的 kernel 构造入口（EN-5）。
8. **（跨 crate 观察，非 engine 工单）app 侧 `service/signal_preparation.rs`（legacy：自建 bus + 自 compile + 挂表）与 `service/managed/*`（EngineHost 路径）并存**：tone.rs/monitor/windows.rs/session_graph.rs 仍走 legacy。engine 侧无需改动；整合是 app 侧决策——迁移完成后可删 SignalPreparation。

## 5. 统计信号

- 文件体量：swap.rs 1164（含 363 行内嵌测试）、signals.rs 858（含 59 测试）、project.rs 619、runtime.rs 615——均单一职责，无 god-module 迹象；processor/builtin 每文件 47-427 行、一 kernel 一文件，健康。
- DSP 原语单源率良好：smoothing、db_to_gain、窗口连续性（WindowContinuity/TimelineHeader 被 meter/spectrum 共享）、pan 法则（pan.rs:76 被 strip/mix/send 共享）均已单源；唯二例外是 meter 双实现（EN-1）与 IO 契约变体（EN-8）。
- dead surface 面积：读出端无消费者 API 涉及 ~1100+ 行实现 + ~900 行测试（EN-1/EN-2），是本轮最大的可回收面。
- 整改趋势：首轮两项已知问题（smoothing、delay 预检）确认修复且带回归测试；参数数值域已单源；剩余问题集中在"结构 match 展开"与"无消费者的完整功能面"。
