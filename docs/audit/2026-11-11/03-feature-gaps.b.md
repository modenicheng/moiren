# 03 功能缺口 — 初审 B（平台/引擎能力缺口与健壮性）

- 基线：main @ e2ed3d6a（2026-11-11），工作树含未跟踪的 docs/audit/ 草稿，代码与基线一致。
- 范围：EngineBackend port vs moiren-windows-audio 实现差集；WASAPI/ASIO 路径完整性；时钟适配；插件三后端；录制/离线渲染；issue #105 复核；panic/OOM；tools/driver/probe 与 docs/platform 偏差。不含 UI/产品层（另一代理负责）。
- 方法：亲读 `moiren-engine/src/host/backend.rs`（port trait）与 `moiren-windows-audio/src/engine_backend*.rs`、`wasapi.rs`、`negotiation.rs`、`codec.rs`、`asio/*`、`servo.rs`、`resample.rs`、`clock_bridge/*`、`catalog.rs`、`moiren-plugins/src/*`、`moiren-engine/src/processor/plugin.rs`、`recorder/*`，逐方法核对实现与门控；`rg` 定位 unsupported/todo/调用面；`gh issue view 105` 核对既往线索（代理已配置，成功）。未读 docs/audit/ 下其他代理报告。

## 发现

### B-1 卡死端点六缺陷（issue #105）在基线全部未修 [P1] [KNOWN-OPEN]
issue #105（OPEN，引用 5304bde）所列六缺陷逐条在 e2ed3d6a 亲验仍存在，git log 无引用 #105 的修复提交：
1. **无界 open + 取消后 join 卡死**：握手循环无 deadline（`crates/moiren-windows-audio/src/engine_backend/owner.rs:233-253`，仅 `recv_timeout(20ms)` 轮询取消位），且取消路径 `state.cancel(); let _ = worker.join()`（owner.rs:234-237、243-245）——worker 若停在 `factory()` 内的 hung COM open（`wasapi.rs:295-454`，Activate/GetMixFormat/Initialize 均无 deadline，且循环体内不检查 stop），join 永久阻塞，Stop/退出不可中断。
2. **组串行打开**：`engine_backend.rs:440-483` `for group in groups { spawn(...) }`，总启动延迟为各组之和；单个慢设备拖住全部设备。
3. **失败无归属**：`BackendError` 无 node/endpoint 字段（`crates/moiren-engine/src/host/backend.rs:86-98`）；WASAPI 错误仅 stage+HRESULT（`wasapi.rs:62-84`），endpoint 只进日志（wasapi.rs:283-288），用户面只见裸字符串。
4. **无 WASAPI 恢复**：RecoveryMonitor 只为 ASIO 组注册（`engine_backend.rs:426-439` 过滤、:469 `recovery: (group.backend == AudioBackend::Asio)`）；`IMMNotificationClient` 只 latch 失效（`wasapi.rs:192-225`），随流死亡；supervisor 的 resources→re-prepare 机器存在（`supervisor/requests.rs:233`）但无 WASAPI 事件源。断线重插后 session 停在 WaitingForResources，需手动 re-apply。
5. **无进度**：spawn 只回传最终 info/error，无阶段/elapsed 上报。
6. **catalog 无界**：`catalog.rs:128/184/220` 对每个 endpoint 做 Activate/GetMixFormat/session 枚举，无 deadline；`moiren-app/src/service/windows.rs:10-12` 同步调用，卡死端点拖延自动启动（issue #105 第 6 条原文照旧成立）。
另核：遗弃 worker 无登记防堆叠（issue D3 的前置防护不存在，`owner.rs:170-181` Worker 无 abandoned 态，`cleanup` 无条件 join 全部 worker，owner.rs:705-708）。
- 用户后果：一个退化 USB 端点可让启动挂 ~1 分钟、Stop/退出卡死、失败不可归因、恢复靠重启；catalog 枚举同被拖死。
- 后续开发指导：按 issue #105 Stage 1 实施——per-group 10s deadline + 遗弃（不 join）+ per-worker fence（先于并行化，防共享 RunState 误激活 master）；取消路径改为 abandon；Stage 2 补 BackendError 归属字段与 WASAPI 恢复监视（IMMNotificationClient ACTIVE/device-added latch + 2 连击确认 + 退避）。验收：fake factory（hang/late/cancel）单测 + 重插自动恢复实测。
- 工作量：L（Stage 1 M，Stage 2 M）

### B-2 插件宿主栈全链路未接通，且扫描无进程外隔离 [P1] [NEW]
三层证据：
1. **扫描进程内、无崩溃/挂起隔离**：`moiren-plugins/src/scan.rs:135-152` 在本进程直接 LoadLibrary 式加载并探测，`collect_file` 只接 `Result`，panic/hang/access-violation 直接杀死宿主；examples/probe.rs:12-13 作者自述 "This crate has no crash isolation, so a misbehaving plugin can crash or hang the probe"。
2. **服务层断链**：`moiren-app` 全 crate 无 `scan_directories`/`instantiate` 调用（rg 零命中）；`NodeConfig::Plugin` 无法从 app 构造，service preparation 对 Plugin 节点直接报 "plugin node has no plugin assignment yet"（`service/graph/preparation.rs:256-259`）；node catalog 明示 "Plugin has none" 模板（`ui/pages/graph/node_catalog.rs:49`）。而引擎侧已完备：`engine/host/project.rs:37` 支持 Plugin 节点、`engine/src/processor/plugin.rs` 有完整 RT 适配（staging 平面、失败清零输出、块粒度自动化）。
3. **RT 无崩溃隔离**：插件是 C++ ABI，access violation 在 FFI 中是进程 abort，catch_unwind 无法捕获；RT 线程（`engine_backend/owner.rs:326-347` catch_unwind 只防 Rust panic）与未来扫描均无 SEH/进程隔离。
- 用户后果：双面——已投入的 CLAP/VST2/VST3 宿主能力对用户不存在（被憋住）；一旦接通，首个坏插件即崩整机/挂死扫描。隔离是接入的前置而非后补。
- 后续开发指导：入口 1）进程外扫描器（单独 helper 进程枚举+探测，IPC 回传 descriptors，超时杀进程）；入口 2）service 增加插件库管理与 Plugin 节点赋值路径。验收：故障 DLL 样本（崩溃/挂起/慢加载）扫描存活；插件节点全链路渲染。工作量按"先扫描沙箱"排序。
- 工作量：L

### B-3 全后端硬限 44.1–96 kHz 与立体声载体 [P2] [NEW]
- 采样率白名单 `SAMPLE_RATES=[44100,48000,88200,96000]`（`engine/src/audio_config.rs:12`）同时约束引擎速率（`engine_backend/validation.rs:103-108`）、设备请求（validation.rs:91-101）与 ASIO（`asio/buffer.rs:86-95`，测试明确断言 192000 被拒，buffer.rs:146）。WASAPI 侧同样无法以 176.4/192/384 kHz 打开。
- 通道：物理 IO 硬性 `channels != 2 → "professional graph IO is stereo"`（validation.rs:88-90）；载体全程 StereoFrame（`native.rs:7,54-55`）；WASAPI 多通道 mix 格式只解码/编码首两通道、其余静音（`codec.rs:84-121`，测试 `extra_native_channels_are_silent_and_mono_downmixes` codec.rs:238-246）；ASIO 只用 inputs[0..1]/outputs[0..1]（`asio/stream.rs:571-613`）。MAX_CHANNELS=32 的校验（negotiation.rs:44）形同虚设。
- 用户后果：192 kHz 接口、5.1/7.1 消费设备、多通道 ASIO 声卡（常见 8/16 出）在"professional"定位下完全不可用；这是产品能力面而非边角。
- 后续开发指导：先决策目标矩阵（建议：WASAPI/ASIO 采样率扩展到驱动支持集，通道先做固定 4/8 通道特化载体）。入口：audio_config.rs 白名单、validation.rs 通道检查、native.rs 载体、codec.rs、asio/stream.rs 通道映射。
- 工作量：L（采样率扩展 M；多通道 L）

### B-4 录制管线引擎完备但应用不可达；纯采集会话被拒 [P2] [NEW]
- 引擎侧能力完整：有界环形（64 MiB 上限、accepted/read/dropped 计数，`engine/src/recorder.rs:37-90`）、f32 WAV 写入与 finalize 补写、drain 有界、IO 失败即 fail-fast（`recorder/wav.rs:91-183`）、host 侧 start/stop/record_wav API（`engine/src/host/signals.rs:124-169`）。
- 但 `moiren-app` 全 crate 无任何 record_wav/RecordingSession/start 调用（rg 零命中）；UI 仅能调节点容量 "Add/Remove 1 second"（`ui/pages/graph/node_inspector.rs:856-884`）。录音到文件在产品内不存在——又一个"实现被憋住"。
- 且 `validation.rs:114-118` 要求 `outputs > 0`：纯采集（无监听输出）会话直接被拒，field-recording/VAD 录入等场景无法表达；`ProjectClock::VirtualRealtime` 分支只在"有输出但全虚拟"时可达。
- 次要：WAV 仅 f32、RIFF 4 GiB 封顶（48k 立体声约 78 分钟，`wav.rs:17,156-162`），无 RF64/WAV64。
- 后续开发指导：S 级先接 service/UI 到 record_wav；M 级为 capture-only 会话放开 validation（复用 VirtualRealtime 时钟，配合 B-10 的时钟桥审视漂移）；RF64 视产品需要。
- 工作量：S（接线）/M（capture-only）

### B-5 WASAPI 断线后无自动恢复、FollowDefault 不跟随默认设备 [P2] [KNOWN-OPEN]
（B-1 第 4 条的展开与补强）恢复机制（RecoveryMonitor + supervisor re-prepare）已按可复用形状建好（`engine_backend/recovery.rs` 单预算/代际围栏，`supervisor/requests.rs:233`），但 WASAPI 无事件源：invalidation 只把运行中流打死（wasapi.rs:246-254），`OnDefaultDeviceChanged` 为空实现（wasapi.rs:213-220），FollowDefault 在 prepare 时一次性解析（engine_backend.rs:372），之后默认设备切换/端点重插均需手动 re-apply。
- 用户后果：日常最常见故障（拔插 USB 耳机、切换默认设备）后音频停在故障态。
- 后续开发指导：backend 级（非 per-stream）WASAPI 恢复监视：IMMNotificationClient 状态→ACTIVE/device-added latch + 有界 GetState 轮询兜底 + 退避重臂；FollowDefault 重解析挂在默认设备变化事件上。可与 B-1 Stage 2 合并实施。
- 工作量：M

### B-6 ASIO 能力簇缺口：面板唤起缺失、单驱动会话、粒度 0 驱动仅收 preferred [P2] [NEW]
- 控制面板：vtable `control_panel` 指针存在（`asio/abi.rs:117`）但全仓库无任何调用（capability_tests.rs:238 甚至置 forbidden），用户改驱动缓冲/路由必须离开应用。
- 多驱动：>1 ASIO 组直接 `unsupported("direct ASIO hosts one driver session at a time")`（engine_backend.rs:416-425）——与 WASAPI 混用可以，两块 ASIO 声卡不行。
- 粒度 0 驱动（FL Studio ASIO 类）：只接受精确 preferred（`asio/buffer.rs:48-53`），无最近合法值协商/提示候选列表。
- 双工必须同率同缓冲（engine_backend.rs:145-174），冲突即整体失败而非裁剪。
- 后续开发指导：面板唤起是 S 级高价值项（STA 上调 control_panel，入口 asio/stream.rs owner 线程）；单驱动维持现状（见"明确不建议"）；粒度 0 驱动建议 probe 阶段暴露 preferred 供 UI 直选。
- 工作量：S（面板）/S（probe 暴露）

### B-7 插件合同缺 MIDI、编辑器、传输信息与采样精确自动化 [P2] [NEW]
- `PluginInstance::process` 事件仅 `PluginParamEvent{parameter,value}`（`moiren-plugins/src/instance.rs:25-30`、`spec.rs:79-82`），无 MIDI/音符事件——`is_synth` 三后端都能检出（vst2/mod.rs:234、vst3/mod.rs:1429-1455、clap/mod.rs:259），但合成器无法被演奏，乐器插件整类不可用。
- 编辑器：VST3 扫描 `has_editor: false` 硬编码（vst3/mod.rs:1179,1456）；全仓库无任何打开编辑器代码（CLAP gui 探测到也只存 flag，clap/mod.rs:258）。所有插件交互只能走自动化数值。
- 事件仅块首生效（instance.rs:22-24），无 sample-accurate offset、无 transport/tempo/位置信息——时间同步类效果（delay 同步、LFO sync）失能。
- 等宽总线排除是文档化决策（spec.rs:62-64 + 三后端 `NoAudioPorts` vst2:301/vst3:1030/clap:245）：侧链、多总线插件被整体排除（当前合理，列出备查）。
- 后续开发指导：MIDI 事件通道是解锁乐器类插件的最小前置（事件类型扩展 + engine 侧 MIDI 源节点，后者涉及新输入栈，宜与产品路线对齐后立项）；传输信息次之；编辑器最后（见"明确不建议"）。
- 工作量：M（MIDI 合同）/L（含 MIDI 源栈）

### B-8 WASAPI 共享模式"精确 period + 格式转换"冲突时静默降级，且无事后复核 [P3] [NEW]
请求 Shared 精确 period 而客户端格式≠mix 时走 compatibility Initialize（"cannot promise an exact event period"，`wasapi.rs:663-674`，legacy 标志含 AUTOCONVERTPCM|SRC_DEFAULT_QUALITY，negotiation.rs:249-253）；而事后对 period_frames 的精确性复核仅对 ASIO 存在（engine_backend.rs:508-517），WASAPI 共享请求的实际 engine period 不被验证。正确性无虞（有日志、有兜底），但"精确低延迟"承诺静默失效。建议：协商结果回读（GetBufferSize/引擎 period）不符时升级为显式 Unsupported 或在 observation 里标记实际 period。
- 工作量：S

### B-9 CancellationToken 取消钩子 16 上限可被合法大图耗尽 [P3] [NEW]
`on_cancel` 超过 16 个钩子直接 `Failed("cancellation hook capacity exhausted")`（`engine/src/host/backend.rs:62-68`）。注册点：每个 process-loopback 节点一个（engine_backend.rs:676-678）、每个 transport attach 一个（owner.rs:602,767）、virtual owner attach 一个（virtual_owner/owner.rs:179）。validation 允许 ≤64 IO 节点（validation.rs:114），理论上 >16 个 process capture/虚拟会话的合法图在 prepare 末段爆掉。建议把钩子表改为小型分槽（Mutex<Vec<Vec<..>>> 分层）或按 owner_generation 聚合为单钩子。
- 工作量：S

### B-10 ClockBridge 全套时钟适配能力无生产接线 [P3] [NEW]
capture/QPC 斜率估计、±2000 ppm 速率校正、age/fault/startup/控制全套（`clock_bridge/rate.rs:8-10`、ingress/source/timing/telemetry）实现并有成体系集成测试（tests/clock_*.rs 共 10 个），但全仓库仅测试与 lib.rs 导出消费——engine_backend 生产路径用的是 RingSource 定率 SRC + rtrb 丢帧 + CaptureWriteClock 时间戳（bridge.rs:118、owner.rs:474-563），capture 端无速率自适应；follower 端有 FollowerServo（servo.rs ±2000 ppm，占用率控制）。独立时钟域适配的"重采样/丢样本"策略实际只有 follower 一侧完整；capture 为主时钟的适配（ClockBridge 的设计目标）悬空——与 B-4 capture-only 被禁互为因果。
- 后续开发指导：若 B-4 决定支持 capture-only 录制，ClockBridge 是现成地基，先补 ingress 与 engine source 的接线设计再动。
- 工作量：M

### B-11 探测/工具链产品化程度参差，docs/platform 仅 1 篇 [P3] [NEW]
- `tools/rt-check`：独立 harness（shadow-compile professional 模块，professional_rt.rs），脱离 root workspace，成熟度较高，但无 CI 入口证据。
- `tools/audio-hack-lab`：一次性排障脚本（e3/e4/e8/probe.ps1 + REPORT.md），无文档化入口。
- probe/exclusive_probe 仅以 examples 形态存在（`windows-audio/src/lib.rs:45-52`），无 CLI 命令；issue #105 等待的 stage-timed open probe 未沉淀。
- `docs/platform/` 仅 gpui-fork.md；WASAPI/ASIO 平台约束（D4 兼容矩阵、废弃 HRESULT 策略等）散落模块注释与测试命名中，无平台总览文档。
- 建议：把 stage-timed WASAPI open probe 做成 tools/ 常驻 CLI（正好服务 B-1 验证）；rt-check 纳入 CI 可选 job。文档偏差为记录性发现。
- 工作量：S

### B-12 WASAPI driver_latency 恒为 None，延迟遥测单边 [P3] [NEW]
`NativeStream::driver_latency` 默认 None（native.rs:59-61），仅 ASIO 实现（asio/stream.rs:433-437），WasapiStream 未 override——`ProfessionalStreamObservation.driver_latency` 对所有 WASAPI 流（含 Exclusive）恒缺，UI/预算侧拿不到真实驱动延迟。IAudioClient 可用 GetStreamLatency（独占）/padding 推导补齐。
- 工作量：S

### B-13（正面确认，非缺口）panic 处置与 OOM 面总体受控 [—]
RT panic：worker 内 catch_unwind 全覆盖（owner.rs:220-227,326-366），WorkerPanicked 优先于其他错误（owner.rs:85-97），Engine 归还在 unwind 边界外保证（owner.rs:348-353）；supervisor 侧亦有 catch_unwind。无界增长点：RecoveryMonitor 单 pending 槽、观察快照按 IO 节点（≤64）、取消钩子有界（但见 B-9）、recorder 64 MiB 上限、遗弃线程堆叠是唯一确认的泄漏面（并入 B-1/D3）。prepare 预算按 audio_byte_budget 校验（engine_backend.rs:269-285）。

## 明确不建议（不该补的）
1. **不中断/终止进行中的 COM 调用**（TerminateThread、APC 注入、句柄关闭强断）——issue #105 D1 已论证 "bound the wait, not the OS call" + abandon 是唯一安全形态，强杀会留下持有 COM/驱动 pin 的僵尸对象。
2. **不给 RecoveryMonitor 加"健康运行自动补充预算"定时器**——recovery.rs:68-69 刻意不加；自动补充会把可恢复一次性故障变成 ASIO 反复重建风暴。退避重臂应建在 supervisor 侧（Stage 2 方案）。
3. **不做 ASIO 多驱动并发会话**——ASIO 驱动普遍全局单实例，跨驱动时间同步无解，真实用户价值低；保持"一次一个驱动会话"并给出清晰错误即可。
4. **现阶段不做进程内插件编辑器宿主**（VST3 IPlugView/CLAP gui → gpui 互操作）——先过 B-2 的扫描沙箱与实例化接通；编辑器是数量级更大的工程且与窗口模型耦合。
5. **不把立体声载体泛型化为任意 N 通道**——泛型化会伤 RT 布局与可审计性；多通道若立项，按固定宽度（4/8）特化 carrier + 通道掩码映射，保持无分配。
6. **不产品化 ClockBridge 为默认录制时钟**——在 B-4 capture-only 场景拍板前接线它属于过早建设。
7. **不用高频轮询 catalog/端点来"绕过"卡死**——B-1 的 deadline/abandon 才是正解，轮询只会把 hung call 变多。

## Top5
1. **B-1** issue #105 六缺陷全部未修（P1，KNOWN-OPEN）——无界 open/取消 join 卡死/组串行/无归属/无 WASAPI 恢复/无进度/catalog 无界，逐条带行号证据。
2. **B-2** 插件栈三层断链：进程内扫描无隔离（作者自认）+ app 无法创建 Plugin 节点 + C++ ABI 无崩溃隔离（P1）——能力被憋住且接入即风险。
3. **B-3** 全后端 44.1–96 kHz 与立体声硬上限（P2，L）——192k 与多通道接口整类不可用。
4. **B-4** 录制管线引擎完备但 app 零接线 + capture-only 会话被 validation 拒绝（P2）——第二个"被憋住"。
5. **B-5** WASAPI 断线/默认设备变化无自动恢复，恢复机器现成只差事件源（P2，M）——日常故障恢复体验的最大杠杆。
