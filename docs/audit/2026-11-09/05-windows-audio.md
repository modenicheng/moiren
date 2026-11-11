# 2026-11-09 架构审计 — moiren-windows-audio
基线: main @ 27aa57d7（工作区干净，实测 `git rev-parse --short HEAD`）

范围：`crates/moiren-windows-audio/`（src 35,773 行 + tests/ 13 个集成文件 + examples/）。只读审计，聚焦架构与代码组织、可维护性，非 bug 狩猎。所有结论均先读代码后落笔，行号以 27aa57d7 为准。

## 执行摘要（≤10 条要点）

1. **总体组织是"域子树 + 小平铺叶"的混合，且行之有效**：10 个域子树（vad 11.1k、engine_backend 5.2k、asio 4.5k、virtual_owner 3.6k、clock_bridge 1.1k、physical/latency/process_loopback/capture/swdevice）+ ~16 个顶层平铺件，平铺件几乎全部 <650 行且角色单一。当前的分组标准事实上是**按传输/API 面**（WASAPI / ASIO / VAD 驱动 / 虚拟 / 进程环回）+**中性工具**（codec/resample/servo/stats/format/duration）+**引擎接缝**（engine_backend/）。在此规模下 navigable，**不建议**为整齐而强行收敛成少数大域目录。
2. **wasapi.rs 1231 行是"宽但浅"的编排层，不是 God 文件**：协商原语已单源在 negotiation.rs（PR#18-20 成果），数据面已收敛到 NativeStream trait 实现；混杂点是端点失效通知 COM 对象与格式选择矩阵同居一文件，拆分属可选项（WA-02）。
3. **三个同名 "owner" 模块是最大的命名混淆源**：`src/owner.rs`（Win32/COM RAII 原语）、`engine_backend/owner.rs`（worker/transport 生命周期）、`virtual_owner/owner.rs`（虚拟时钟 transport），语义互不相干（WA-01）。
4. **VAD ABI 三重编码：已消除且保持**——内核头文件为唯一权威，Rust 镜像 + `abi_conformance.rs` 以 `include_str!` 解析 C 头并钉住 24 标量/7 IOCTL/6 布局尺寸/GUID（commit 443a2b32）。
5. **virtual-IO 重复：已消除且保持**——`bind_playback_source`/`bind_microphone_sink` 单源（virtual_owner/provider.rs:79-131），唯一消费方 engine_backend/virtual_io.rs:71-74。
6. **2026-10-27 的 WA-5（依赖倒置）与 WA-6（role→ERole 三处重复）均已随单栈化落地修复**：resample/servo 已顶层化，virtual_owner 只依赖顶层中性模块；role→ERole 映射仅剩 wasapi.rs:105-113 一处。
7. **测试摆放四轨并存但内部一致**；唯一不可发现点是 `src/asio_rt_tests.rs` 与 `src/engine_backend/recovery_host_tests.rs`（共 2,540 行）只被 `tools/rt-check/tests/professional_rt.rs:32,75` 影子编译，crate 内零引用（WA-05）。
8. **unsafe/COM 组织良好**：487 处 unsafe、88 处 SAFETY 注释；RAII（Apartment/TaskMemory/OwnedHandle/PacketLease/Mmcss）集中在 owner.rs，raw COM 调用按 windows-rs 风格分布在传输文件属必然；抽查的非显然 unsafe 均有 SAFETY 契约。
9. **RT 纪律有据可查但分散**：asio/callback.rs:1、vad/pcm.rs:1、pcm_bridge.rs:1、servo.rs:13、resample.rs:2、native.rs:45（No Send 约束）等各自声明，缺一张集中的线程/RT 边界地图（WA-08）。
10. **残留 3 处双栈时代 stale 注释**（negotiation.rs:195-197、negotiation.rs:169 SAFETY 契约内 "Both stacks"、virtual_owner/provider.rs:77 "Both native backends"），建议随下次触碰删除（WA-03）。

## 发现清单

| ID | 级别 | 标题 | 位置 | 证据摘要 | 建议+权衡 |
|---|---|---|---|---|---|
| WA-01 | P2 | 三个 "owner" 模块同名异义 | src/owner.rs:21-286; src/engine_backend/owner.rs:1-840; src/virtual_owner/owner.rs:38-485 | `src/owner.rs` 实为 Win32/COM RAII 工具箱（Apartment、Streaming、PacketLease/LeaseClient、Mmcss、OwnedHandle、TaskMemory、wait_for_handles、Process）；`engine_backend/owner.rs` 是 native worker/Transport/BackendOwner 生命周期；`virtual_owner/owner.rs` 是 VirtualPreparedTransport。三者无共同语义，lib.rs:67 `mod owner` 还不导出，新开发者无法从名字判断该进哪个 | 将 `src/owner.rs` 更名 `win32.rs`（或 `com.rs`）。纯重命名一次 PR 可完成；代价是 38 个文件的 `crate::owner::` 路径引用需同改。收益是消除 crate 内最大命名歧义。两个子目录 owner.rs 语义各自成立（"engine_backend 的 owner"、"virtual_owner 的 owner"），保留 |
| WA-02 | P2 | wasapi.rs 1231 行：编排层内四个区块，可拆可不拆 | src/wasapi.rs:86-127（resolve_endpoint 枚举/身份）、148-272（EndpointNotifications COM 回调对象+注册）、274-761（open/格式选择 select_format/negotiate_exclusive/negotiate_shared_exact）、763-1081（NativeStream 数据面+wave_format/codec_for 描述符映射）、1083-1231（单测） | 协商原语（initialize_exclusive、shared_engine_periods、parse_descriptor、client3_rejects_format）已单源于 negotiation.rs:172-331，wasapi.rs 是策略/编排层；数据面通过 native.rs:49 NativeStream trait 暴露。文件呈线性流水、区块注释清楚，非 spaghetti | 两个选项：(a) 拆 `wasapi/`（notifications.rs + format_select.rs + stream.rs），与 capture/、asio/ 的目录化模式一致，成本低；(b) 维持平铺，在模块头加区块索引注释。倾向 (a) 但非紧迫——现文件内聚度尚可，"混杂"仅指关注点数量而非纠缠 |
| WA-03 | P3 | 双栈时代 stale 注释 3 处 | src/negotiation.rs:190-197（"The legacy stack additionally treats..."描述已删除栈的行为）；src/negotiation.rs:165-171 SAFETY 契约内 "Both stacks only pass..."（描述不存在的调用方）；src/virtual_owner/provider.rs:77-78 "Both native backends must route virtual IO through these" | negotiation.rs 头注释（1-6 行）以史实口吻提"retired dual stacks"是合理历史说明，但这 3 处是**现在时态的行为描述**，会误导读者去找第二个栈 | 下次触碰时改写为当前事实（"所有调用方"）。SAFETY 契约里的 stale 描述优先级略高，因为安全论证引用了不存在的调用方集合 |
| WA-04 | P3 | bridge.rs 顶层命名含糊且依赖方向上属 engine_backend 域 | src/bridge.rs:1-18（头注释"Complete stereo frames cross bounded SPSC rings"）；bridge.rs:3 `use crate::engine_backend::observations::Counters` | 内容是 engine_backend 的图边界适配器（RingSink/RingSource+QPC 前馈与 backlog 回收、Tone、ProcessSource、CaptureWriteClock），消费方只有 engine_backend.rs/owner.rs/测试；与 clock_bridge/、vad/pcm_bridge 三个 "bridge" 并存但语义不同（后两者是时钟域/PCM 格式域的桥） | 首选改名（如 `graph_boundary.rs` 或 `engine_backend/boundary.rs`）。并入 engine_backend/ 语义最准，但 bridge.rs 是 `pub(crate)` 且被 packet_phase_tests 等引用，移动成本略高于改名；两条路都只值得顺手做 |
| WA-05 | P2 | 测试摆放四轨并存；rt-check 影子文件在 crate 内不可发现 | tools/rt-check/tests/professional_rt.rs:32 `#[path = ".../src/asio_rt_tests.rs"]`、:75-76 `#[path = ".../src/engine_backend/recovery_host_tests.rs"] mod recovery_host_tests;`；src 内对这两个文件名零引用（rg 全库验证） | 四轨：(1) 内联 `#[cfg(test)] mod tests`（惯例主体）；(2) 子目录 tests.rs / `#[path]` 兄弟文件（vad/provider/lifecycle/tests.rs:851 行、pcm_worker/tests.rs:2397 行、virtual_owner/boundary/tests.rs:1212 行、engine_backend/owner/idle_tests.rs 等）；(3) tests/ 集成 13 文件+common/（clock_* 6 个、virtual_boundary、process_loopback、latency*2、duration、asio_acceptance）；(4) rt-check 影子编译（分配断言 harness）。前 3 轨与被测代码同址、惯例一致；(4) 轨的 2,540 行真实测试从 crate 任何 mod 声明都看不到，`cargo test -p moiren-windows-audio` 不执行它们 | 在两个影子文件头部加一行归属注释（"compiled only by tools/rt-check; not part of crate targets"），并在 lib.rs 的模块注释或 engine_backend.rs/asio.rs 相邻处提示 rt-check 的存在。不建议移文件——rt-check 的 #[path] 相对路径与隔离编译是有意设计（2026-11-03 审计已登记其常量镜像问题）。附带小项：engine_backend.rs:48-50 把 tests.rs 挂成 `mod virtual_tests` 但文件含通用 prepare 测试，模块名与内容不符 |
| WA-06 | P3 | run_playback/run_microphone 结构镜像约 400 行 | src/vad/provider.rs:558-687 vs 816-963 | 两循环共享同一骨架（gate/admission 检查→poll→quiesce/reset 握手→pcm_worker pump→teardown→RetiredPcm 泊车），差异集中在方向语义（playback 的 observed_reset 处理 vs microphone 的 reprime_after_reset、MicrophonePump）。teardown 已各自提取为 helper（800-814、965-981），RetiredPcm 已共用（699-798） | **保留镜像，不合并**。两个方向的 reset/reprime 语义有专门注释（849-858 re-prime 避免 ~20s 部分静音；979-980 并发图写竞争），强行抽象会把两组有向性约束塞进同一泛型。属于"可接受的镜像"，仅建议文件头注明两者的镜像关系 |
| WA-07 | P3 | VirtualPlaybackSource/VirtualMicrophoneSink builder 系列近乎逐字镜像 | src/virtual_owner/boundary.rs:204-523 vs 526-826 | with_probe/with_clock_drift/with_reset_epoch/with_reset_fence/with_epoch_handshake/synchronize_epoch 各 ~150 行镜像；语义方向相反（source 的 discard 需 producer fence 先行 339-367；sink 的 producer fence 由本侧发布 651-676），ClockDrift/EpochHandshake 已共用（68-98、74-77） | 同 WA-06：保留。共享部分（ClockDrift、EpochHandshake、staging/src_capacity 等）已抽出，剩余是方向性差异。合并会模糊 fence 所有权这一核心 RT 协议 |
| WA-08 | P2 | RT 边界识别靠逐模块头注释，缺集中地图 | src/asio/callback.rs:1（"No driver calls, allocation, locks...on callbacks"）；src/vad/pcm.rs:1、pcm_bridge.rs:1（"never Engine render"）；servo.rs:13、resample.rs:2、clock_bridge.rs:142（"Non-RT preparation"）；native.rs:45-48（No Send bound 说明）；engine_backend/owner.rs:1-2（master 借 Engine 的 unwind 边界） | 每个关键模块都有正确的 RT 声明，但新开发者需要拼图。docs/ 有 6 篇 VAD 专题验证文档，无 windows-audio 线程模型总览 | 在 lib.rs 模块注释（或 docs/platform）加 ~15 行线程/RT 边界表：控制线程（prepare/poll_resource）→ native owner 线程（Mmcss、Pump::cycle、stream.wait）→ Engine render（bridge.rs RingSource/RingSink、boundary.rs、resample/servo）→ ASIO 硬件回调（callback.rs，最严约束）→ VAD PCM worker。零代码改动，收益是 onboarding |
| WA-09 | P3 | lib.rs 模块文档与 NATIVE_SAMPLE_RATE 之外的 crate 面不符 | src/lib.rs:1 "Windows audio probes, Shared capture/render and backend clock adaptation" | 未提 ASIO、VAD/虚拟设备、engine_backend（引擎接缝）、process_loopback——这些占 src 的 80%+ | 一句话改写。属顺手项 |
| WA-10 | P3（正面登记） | probe-tools feature 把示例专用 COM 面挡在库外 | src/lib.rs:44-58；Cargo.toml:23-62 | probe/exclusive_probe/physical/swdevice/clock 五模块仅 `probe-tools` 下编译，examples 经 required-features 声明；vad-probe 独立 feature。设计正确，无需行动 | 无行动；审计确认该机制是本 crate 组织的亮点之一 |

级别说明：P1（结构性障碍）本轮**无**；P2 为值得排期的可维护性项；P3 为顺手修。

## 模块地图（按实际域分组）

```
引擎接缝/控制面编排（实现 moiren-engine 的 EngineBackend/PreparedTransport/BackendOwner trait）
  engine_backend.rs(753)        ProfessionalBackend：分组、预算、WorkerRequest、process loopback 接入
  engine_backend/owner.rs(840)  RunState/EngineSlot/Timeline/Worker/Transport/Pump::cycle（native owner 线程）
  engine_backend/recovery.rs(611) RecoveryMonitor（ASIO 恢复票据）
  engine_backend/observations.rs  观察快照（原 WA-4 双观察体系随 legacy 删除收敛为单一 ProfessionalStreamObservation）
  engine_backend/validation.rs / virtual_io.rs(118)  准入校验 / 虚拟绑定薄层

WASAPI 物理传输
  wasapi.rs(1231)              open/格式选择/协商编排 + NativeStream 实现 + 端点通知
  negotiation.rs(493)          协商内核（单源：描述符解析/IAudioClient3 拒绝矩阵/Exclusive 对齐重试/限值常量）
  capture/(473)                独立共享捕获会话（wasapi/{packet,preparation,stream}）——供 CLI/process_loopback 路径
  capture_packet.rs(103)       GetBuffer 租借包验证（跨栈共享残留，现仅 WASAPI/物理 probe 用）
  codec.rs(247)                有界 LE PCM/浮点转换（含左对齐 valid bits）

ASIO 物理传输（仅 win64）
  asio/abi.rs(≈330)            独立声明的 IASIO wire ABI，对照 Steinberg SDK 2.3.4 校验
  asio/callback.rs(544)        硬件 RT 回调（reader barrier、零分配纪律）
  asio/{driver,buffer,sample,stream,registry,capabilities}.rs
  asio/probe.rs(779)           纯验收探针逻辑（与 asio_probe 示例共享）

图边界适配（Engine render 内执行）
  bridge.rs(605)               RingSource（QPC 前馈+backlog 回收）/RingSink/Tone/ProcessSource/CaptureWriteClock
  resample.rs(250) / servo.rs(106)   中性 DSP 内核（2026-10-27 WA-5 建议的顶层化已落地）

时钟适配
  clock_bridge/(1058)          捕获时钟桥（ingress/rate=RateClock/source/telemetry/timing）
  virtual_owner/{clock.rs,pacer.rs}  虚拟实时时钟（高分辨率 waitable timer）
  latency/(561)                QpcClock、预算、遥测
  clock.rs(296)                离线最小二乘漂移分析（probe-tools gated）

虚拟设备域
  virtual_owner/(3643)         boundary.rs（RT 侧源/汇）、provider.rs（trait+单源绑定+VirtualSessions）、owner.rs（虚拟 transport）
  vad/(11070)                  驱动 ABI→PCM IO 全栈：protocol.rs（LE ABI v2 镜像+abi_conformance）、discovery/control_io/client、pcm.rs（overlapped PCM owner）、pcm_slots/pcm_bridge/format、provider.rs（transport 适配+worker 循环）+provider/{lifecycle,pcm_worker,transport}、endpoint_name/
  process_loopback/(552)       进程环回（activation/identity/stream）
  swdevice/(349)               W00 软件设备生命周期（probe-tools gated）

设备枚举/目录
  catalog.rs(350)              端点/默认设备/会话快照

Win32/COM 原语与中性类型
  owner.rs(307)                Apartment/TaskMemory/OwnedHandle/PacketLease/Mmcss/Process/wait_for_handles
  session.rs(32)+gate.rs(131)  StopSignal/ActivationGate
  native.rs(118)               NativeStream trait + StereoFrame 载体
  stats.rs / format.rs / duration.rs   AUDCLNT 标志常量 / serde 形状 / 会话时长
```

依赖方向（rg 全量验证，无环）：engine_backend → {wasapi, asio, virtual_owner, process_loopback, bridge}；vad → virtual_owner（实现 VirtualBoundaryProvider，provider.rs:275）；virtual_owner → {resample, servo, capture::CaptureSession, session}（顶层中性模块，倒置已修复）；capture → {clock_bridge, owner, session}；bridge → engine_backend::observations（唯一"平铺件反向引用子树"，见 WA-04）。crate 整体依赖 moiren-core/moiren-engine，方向正确（平台后端实现引擎 trait：engine_backend.rs:205 `impl EngineBackend`、owner.rs:735 `impl PreparedTransport`、owner.rs:814 `impl BackendOwner`）。

## unsafe/COM 封装现状

- 总量：src 内 487 处 `unsafe`、88 处 `SAFETY:` 注释、38 个文件引用 `windows::`。
- 分布（每文件 unsafe 计数）：wasapi.rs 41、asio/abi.rs 28、asio/callback.rs 22、owner.rs 20、negotiation.rs 20、physical/stream.rs 20、vad/endpoint_name/windows_reader.rs 37、vad/pcm.rs 17、exclusive_probe/render.rs 17、vad/control_io.rs 13。前两名是测试文件（asio/tests.rs 67、capability_tests.rs 49）。
- **集中封装**：COM 所有权 RAII 全部在 owner.rs——Apartment（CoInitialize/CoUninitialize 配对，wasapi.rs:144 特意声明在字段末序保证先释放）、TaskMemory（CoTaskMemFree）、OwnedHandle、PacketLease+LeaseFallback（GetBuffer 租借的 Drop 语义按传输声明，owner.rs:83-131）、Mmcss。negotiation.rs 集中裸 COM 协商调用并以 `# Safety` 契约+StageError 标签包装，策略留调用方。
- **散布部分属 windows-rs 风格必然**（每个 COM 方法调用即一处 unsafe），非封装失职。
- 抽查的非显然 unsafe 均有 SAFETY 论证：GetMixFormat 打包字段按值读取（wasapi.rs:345-348、negotiation.rs:172-188）、capture 包租借借用（wasapi.rs:834-845）、ASIO 回调 reader barrier（callback.rs:100）、PCM overlapped 槽（vad/pcm.rs:19-27 Storage/UnsafeCell）。
- `#[implement(IMMNotificationClient)]` 的回调对象（wasapi.rs:158）与 ASIO extern "system" 回调（callback.rs:22-25 手写 SetEvent 声明以避免 windows Result 构造分配）体现了 RT 约束在 unsafe 层的自觉处理。
- 结论：符合 AGENTS.md 要求；唯一瑕疵是 negotiation.rs:165-171 SAFETY 契约引用已删除的调用方（WA-03）。

## 测试组织现状

四轨摆放（详见 WA-05）：
1. **内联 `mod tests`**：绝大多数模块（wasapi.rs:1083、negotiation.rs:347、owner.rs:288、bridge.rs:404、native.rs:99、provider.rs:996 等）。
2. **同址外置测试文件**：子目录 tests.rs（vad/provider/{lifecycle,pcm_worker}/tests.rs、virtual_owner/{boundary,owner}/tests.rs）与 `#[path]` 兄弟文件（engine_backend/{tests.rs→virtual_tests, tests/fakes.rs, packet_phase_tests.rs}、owner/{idle,lifecycle}_tests.rs、asio/{tests,capability_tests}.rs、vad/tests.rs、examples/vad_lifecycle/tests.rs）。惯例：与被测模块同目录、`#[cfg(test)] mod tests;` 或 `#[path]` 挂载，**一致**。
3. **tests/ 集成**：13 文件，clock_* 六件套 + virtual_boundary/process_loopback/latency*2/duration/asio_acceptance，共享 tests/common/（alloc_guard/clock_sim）。clock_bridge.rs 集成 1003 行属时钟域仿真主体，摆放合理。
4. **rt-check 影子**：src/asio_rt_tests.rs（真实回调零分配断言）+ recovery_host_tests.rs（真实 supervisor+fake NativeStream 端到端恢复，1286 行）仅由 tools/rt-check 编译——crate 内不可发现（WA-05）。

大测试文件（pcm_worker/tests.rs 2397、boundary/tests.rs 1212）主题单一（一个被测模块的全部相位用例），不构成拆分压力。

## 与既往审计对账

| 既往风险 | 现状 | 证据 |
|---|---|---|
| VAD ABI 三重编码（2026-10-22 H1） | **已消除，保持** | 驱动头 `driver/vad/include/moiren_vad.h` 为唯一权威；Rust 镜像 src/vad/protocol.rs（"No native struct layout crosses the wire"，:1）；abi_conformance.rs:8 `include_str!` 解析头文件并钉住标量/IOCTL/sizeof(static_assert)/派生 MAX_PCM_BYTES/GUID（commit 443a2b32，2026-10-22 深夜）。PCM 交织编解码单一实现 format.rs:17,76，pcm_bridge/examples 复用无副本 |
| virtual-IO 重复（2026-10-27 M2） | **已消除，保持** | 单源 bind_playback_source/bind_microphone_sink（virtual_owner/provider.rs:79-131，含 fence/probe/drift 组装）；唯一栈消费点 engine_backend/virtual_io.rs:71-74；无第二份组装逻辑残留 |
| 2026-10-27 WA-5 依赖倒置（resample/servo 住 professional/ 下） | **已修复** | resample.rs/servo.rs 已是顶层模块（commit 91d22dd3 提升布局）；virtual_owner 仅 `use crate::{resample::…, servo::…}`（boundary.rs:1）+StopSignal/capture（owner.rs:2），无 engine_backend 反向依赖 |
| 2026-10-27 WA-6 role→ERole 三处重复 | **已收敛为单处** | 映射仅 wasapi.rs:105-113（resolve_endpoint）；catalog.rs:263-265 是遍历全部角色的默认端点枚举（用途不同）；engine_backend/resources.rs 已随 legacy 栈删除 |
| 2026-10-27 双后端与六缺陷（issue #105） | 不在本轮范围 | 单栈化（PR#18/#19/#20）已删除 legacy 栈；卡死端点六缺陷按协调方指示不重报 |
| 2026-11-03 rt-check 常量镜像（P3） | 保持已知状态 | tools/rt-check/tests/professional_rt.rs:43-54 镜像 negotiation.rs:41-44 四个限值；本轮补充发现该 harness 还影子编译两个 crate 内不可见测试文件（WA-05，属新登记的摆放问题而非重复数据） |

## 维护性建议 Top 5（含权衡）

1. **重命名 `src/owner.rs` → `win32.rs`（WA-01，P2）**。收益：消除三 "owner" 歧义，让 `engine_backend/owner` 与 `virtual_owner/owner` 的"owner=资源属主线程"语义独占该词。代价：~38 文件 import 路径机械替换，一次 PR，无行为变化。
2. **补线程/RT 边界地图（WA-08，P2）**。在 lib.rs 模块注释或 docs/platform 加一张 ~15 行表（控制线程 / native owner（Mmcss）/ Engine render / ASIO 回调 / VAD PCM worker / rt-check harness）。零代码风险，直接改善新开发者对"哪些代码跑在音频线程"的识别——这是 AGENTS.md 实时约束的可发现性投资。
3. **给 rt-check 影子文件加归属注释并在 crate 内提示（WA-05，P2）**。两个文件头各加一行"仅由 tools/rt-check 编译"，lib.rs 注释点名。不移文件（保持隔离编译设计），纯文档改动；防止未来有人当作死文件删除或重复造轮子。
4. **wasapi.rs 目录化（WA-02，P2，可缓）**。拆 `wasapi/{notifications,format_select,stream}.rs` 对齐 capture/、asio/ 模式；或在头注释列区块索引作为零成本替代。权衡：拆分改善定位但增加 3 个小文件；当前文件线性可读，不拆也不构成债务，建议与下一次 wasapi 功能 PR 捆绑执行。
5. **清理 3 处 stale 双栈注释 + lib.rs 模块文档（WA-03/WA-09，P3）**。其中 negotiation.rs:165-171 的 SAFETY 契约描述优先（安全论证应指向现存调用方）。合计 <10 行改动，可搭任意顺风车。
