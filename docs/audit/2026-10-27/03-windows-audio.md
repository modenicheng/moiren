# moiren-windows-audio 审计报告

> 子区域：`crates/moiren-windows-audio`（191 文件 / ~56500 行）。方法：先读 lib.rs 模块树，再按子系统通读/抽读 professional/、legacy 栈（engine_backend/、capture/、render/、shared_stream/）、negotiation.rs、clock/clock_bridge/latency、vad/、virtual_owner/、session/preparation/owner/catalog/stats/process_loopback，交叉核对 moiren-app 侧路由与观察消费。全程只读。

## 1. 执行摘要

1. **前两轮整改基本属实且质量高**：8 项既往已知项中 6 项核实 FIXED（含 5 项带单源化 + 钉扎测试），D4 已升格为"有意的参数化分歧"，唯一 STILL-OPEN 的结构性问题就是双后端本身与 D5 小决策。negotiation.rs 已成为两栈共用的 WASAPI 协商内核，残余重复已收缩到"栈内编排"层。
2. **双后端是本 crate 仅存的大额重复**：legacy ~7.6k 非测试 LOC 与 professional ~8.1k（含 ASIO 3.3k）平行，路由仍由 `moiren-app/src/service/managed.rs:212-240` 的 `settings != &AudioSettings::default()` 决定（主代理已亲证）。退役的技术前置（协商内核、virtual 绑定、进程环回、MMCSS、包租约）已全部就位，剩下的是**能力对齐**而非**代码抽取**。
3. **切换默认路径前有 4 个具体能力缺口**（WA-1）：latency profile 三档语义在 professional 无映射、捕获侧时钟适配降级（occupancy-only servo，无 QPC 前馈/时效回收）、虚拟端点在 professional 无 observation、每唤醒延迟遥测缺失。
4. **新的行为分叉**（WA-2）：同名 `moiren:tone:` 软件源在两栈幅度不同（1.0 vs 0.1）、频率上限不同——用户改一个设置项就会听到不同响度（主代理已亲证两处源码）。
5. **命名/依赖方向问题**（WA-5）：`virtual_owner`（两栈共用的中立层）反向依赖 `professional::{resample,servo}` 命名空间与 `engine_backend` 的观察类型；clock 家族有 4 个模块但功能互不重叠（WA-7，不建议合并）。

## 2. 既往已知项核对表

| 项目 | 状态 | 证据 |
|---|---|---|
| 双后端并存（W-K 统一方向） | **STILL-OPEN**（路由未切；内核前置已就位） | 路由：`crates/moiren-app/src/service/managed.rs:212-240`；协商已单源：`src/negotiation.rs:1-10,477-518`（`open_shared_client` 被 `capture/wasapi/physical.rs:32`、`render/wasapi/stream.rs:182` 复用；professional/wasapi.rs:9-14,601,728,940 全部经 negotiation 原语） |
| virtual-IO 绑定逐字重复（M2） | **FIXED** | 单源 `src/virtual_owner/provider.rs:79-131`（`bind_playback_source`/`bind_microphone_sink`，含 fence/probe/drift 组装）；两栈调用：`professional/backend/virtual_io.rs:68,71`、`engine_backend/preparation.rs:186,313`，各自仅剩 NodeConfig 遍历 |
| professional 缺 MMCSS（M1） | **FIXED** | `src/professional/owner.rs:260-261`（`Mmcss::audio()`，注释对齐 physical/virtual 路径，失败非致命）；基础设施 `src/owner.rs:138-163` |
| professional 缺 ENGINE_PERIODICITY_LOCKED 重试 | **FIXED（按 D4 转为有意分歧）** | `src/negotiation.rs:203-227`（`EnginePeriodicityPolicy` + `periodicity_locked_retry`）；professional 钉扎 `ExactPeriod`：`src/professional/wasapi.rs:51-55`（const assert）；legacy `EnginePeriodRetry` 单次重试：`src/shared_stream/wasapi.rs:26,150-177`；测试 `src/negotiation.rs:671-685` |
| VAD ABI 三重编码（H1） | **FIXED（测试真实且全面）** | `src/vad/protocol/abi_conformance.rs:8`（`include_str!` 内核头）+ 5 个测试：24 个标量（64-100）、7 个 IOCTL 码（102-121）、6 个 `static_assert(sizeof)` 布局尺寸（123-140）、派生 `MAX_PCM_BYTES`（143-147）、GUID 从头文件逐字解析比对 `CONTROL_INTERFACE_GUID_U128`（149-171）；GUID 单源 `src/vad/protocol.rs:53` → `src/vad/discovery.rs:20-21` |
| D5：legacy 是否实时 SetSend | **STILL-OPEN**（但已降级为 wontfix 候选） | `crates/moiren-app/src/service/driver.rs:434-442`（legacy 对 SetSend/SetMixer 自动重建会话而非静默丢弃）；决策记录 `docs/superpowers/plans/2026-10-19-repo-steward-remediation.md:47-49` |
| allocator guard ×7 拷贝 | **FIXED** | 唯一副本 `tests/common/alloc_guard.rs`（经 `tests/common/mod.rs` 复用）；`src/render/wasapi/stream.rs:530-573` 保留独立 in-binary guard 并注释理由（`#[global_allocator]` 每二进制一个） |
| T4：测试 spawn 计数 ~104 | **FIXED（大幅改善）** | `tests/` 下 `.spawn(` 计 0 处；src 内 12 处、每文件 1 处（每类 owner 线程一个 spawn 点）；测试改用仿真 harness `tests/common/clock_sim.rs` |

## 3. 发现清单（按严重度）

### WA-1（P0-漂移/正确性边界）双后端能力不对称：默认路径切换前必须对齐的 4 个缺口
**证据**：
- 路由条件：`moiren-app/src/service/managed.rs:212-240`。默认 `AudioSettings`（Wasapi / 48k / F32 / block 256 / Shared / Native / 无 buffer / 空 overrides，`moiren-engine/src/audio_config.rs:95-102,112-121`）走 legacy；任何偏离走 professional。f64 恒 professional（managed.rs:241-250）。
- 缺口 1 — **latency profile 只在 legacy 生效**：`managed.rs:212-226` 的 professional 分支忽略 `profile` 参数；且 legacy 的 advisory 毫秒档位（`shared_stream.rs:84-96` Compatible/Balanced/Low → `choose_shared_period` 任意 ms 取整；Balanced=10ms@48k=480 帧、Low=5ms=240 帧）在 professional **不可表达**：professional 校验强制 2 的幂 64..4096 帧（`professional/backend/validation.rs:91-101`；`BUFFER_SIZES` 仅 `[64,128,256,512,1024,2048,4096]`，`moiren-engine/src/audio_config.rs:13`）。
- 缺口 2 — **捕获时钟适配降级**：legacy 物理捕获经 `clock_bridge`（PI 伺服 + `RateClock` QPC 前馈 + prime/trim/backlog/age 回收 + position-gap 检测，`clock_bridge/source.rs:240-283,161-239`、`clock_bridge/rate.rs:42-87`）；professional 捕获是 `RingSource`（occupancy-only `FollowerServo` + `StereoResampler`，无前馈、无时效回收，`professional/bridge.rs:53-108`、`professional/servo.rs:40-46`）。
- 缺口 3 — **虚拟端点观察缺失**：legacy 为 VirtualPlayback/VirtualMicrophone 发布 `NativeStreamObservation`（`engine_backend/preparation.rs:204-211,330-337`）；professional 的 `virtual_io::prepare`（`professional/backend/virtual_io.rs:31-80`）不 push 任何 `ProfessionalStreamObservation`，`backend.rs` 仅在物理组（498-500,514-516）与进程捕获（665-686）push。
- 缺口 4 — **每唤醒延迟遥测缺失**：legacy capture/render 记录 `CallbackObservation`/Trace（`capture/wasapi/stream.rs:149-155`、`render/wasapi/stream.rs:100-137,217-234`）；professional 仅 `NativeStreamStats`（wait/write 计数，`professional/native.rs`）。
**建议**：见第 4 节步骤 1a-1d。**风险**：切换默认后 UI 三档延迟设置静默失效（用户可见）；捕获长期漂移与积压恢复行为退化（音质）。**验证**：parity 集成测试（同图两栈 observation 对比）+ 硬件 soak。

### WA-2（P1）`Tone` 软件源双实现且行为分叉
- **证据**：`engine_backend/preparation/tone.rs:6-39`（f32、幅度 **1.0**（`self.phase.sin() as f32`，主代理亲证）、选型期校验 0<f<24kHz `selectors.rs:3-11`、`phase % TAU`）vs `professional/bridge.rs:131-162`（泛型 S、幅度 **0.1**（`sin * 0.1`，主代理亲证）、绑定期校验 f<sr/2、`phase.fract()`）。同名协议 `moiren:tone:` 产自 `engine_backend.rs:106-115` 与 `moiren-app/src/service/graph/preparation.rs:137`。
- **重叠**：同一逻辑特性（软件正弦源）两份实现、两份校验语义。
- **建议**：合并为单一 `Tone`（置于中立模块，泛型 S 实现可直接服务两栈），幅度与上限取一个显式决定并加测试钉住；`selectors::validate_frequency` 与 bridge 内联校验合一。
- **风险**：低（纯软件源）；幅度选择会改变现有任一路径的可听行为，需在 PR 说明。**验证**：单测（频率、幅度、相位连续性）。

### WA-3（P1）进程环回准备逻辑双份
- **证据**：`professional/backend.rs:572-582`（`process_selector` 解析 `moiren:process:`）与 `engine_backend/selectors.rs:24-38` 逐字同构；creation-time 一致性检查双份（`professional/backend.rs:629-634` vs `engine_backend/preparation.rs:398-403`）；两边各自包装 `prepare_process_capture_with_gate` + observation push（`professional/backend.rs:619-688` vs `preparation.rs:404-439`）。professional 已经复用 legacy 的 `CaptureSession`（`professional/backend.rs:583-617` ProcessOwner）。
- **建议**：把"selector 解析 + 身份校验 + 准备 + observation"抽成 `process_loopback` 侧的单一入口（返回 session+元数据），两栈只各自处理绑定与观察模式差异。**风险**：低。**验证**：现有 `tests/process_loopback.rs` + 两后端集成测试。

### WA-4（P2）双观察体系贯穿到 UI
- **证据**：`engine_backend/observations.rs:20-40`（`NativeStreamObservation`，6 种 `NativeStreamKind`，serde JSON 形状）vs `professional/observations.rs:12-31`（`ProfessionalStreamObservation`，不同字段集）；app 双份消费：`moiren-app/src/service/model.rs:182-189`、`ui/state.rs:905-920,1086-1120`、`ui/presentation.rs:55`。
- **建议**：不急于合并 schema，但退役步骤 3 中以 professional 形状为准收敛（补齐缺口 3 后 professional 是超集），legacy 侧随栈删除。**风险**：UI 展示字段名变化。**验证**：gpui-test 快照。

### WA-5（P2）`virtual_owner` 的依赖方向倒置
- **证据**：`src/virtual_owner/boundary.rs:1`（`use crate::professional::{resample::StereoResampler, servo::FollowerServo}`）；`src/virtual_owner/owner.rs:6-9`（`use crate::engine_backend::{NativeObservations, NativeStreamKind, ...}`）。而 `virtual_owner` 被两栈共同消费（`professional/backend.rs:12-15`、`engine_backend/preparation.rs:17-21`）。
- **详述**：不是功能重复而是归属错位——`StereoResampler`/`FollowerServo` 事实上是三处共享内核（professional 物理路径、虚拟边界、未来统一捕获），却住在 `professional/` 命名空间下；`VirtualPreparedTransport` 的 legacy 观察字段把中立传输绑死在 legacy schema 上。
- **建议**：`resample.rs`/`servo.rs` 提升为 crate 顶层模块或并入 `virtual_owner`；legacy 观察参数随退役删除。**风险**：纯移动，低。**验证**：`cargo test -p moiren-windows-audio` + `tests/virtual_boundary.rs`。

### WA-6（P2）默认端点 role→ERole 映射三处重复
- **证据**：`professional/wasapi.rs:100-110`、`engine_backend/resources.rs:264` 附近、`catalog.rs:336` 附近各自维护 `DefaultDeviceRole::{Console,Multimedia,Communications}` → `eConsole/eMultimedia/eCommunications` 映射与 `GetDefaultAudioEndpoint` 调用。
- **建议**：在 `negotiation.rs`（已拥有端点原语）加一个小 helper（role→ERole + 默认端点解析），三处调用。**风险**：低；catalog 需要快照式批量语义，仅共享映射部分。**验证**：现有 catalog 测试。

### WA-7（P3）clock 家族命名易混（不合并，仅文档指引）
- **证据**：`clock.rs`（离线最小二乘漂移分析，296 行）、`clock_bridge/`（运行时捕获时钟自适应）、`latency/clock.rs:1-34`（QPC 包装 `QpcClock`）、`virtual_owner/clock.rs`（QPC 虚拟实时调度 + 债务上限）。逐文件读后确认**功能互不重叠**。
- **建议**：在 `clock.rs` 头注释加一行"本模块仅做离线分析；运行时适配见 clock_bridge/virtual_owner/clock"。

## 4. 双后端退役剩余步骤清单（有序）

前提：W3/W4（协商内核单源）、M1（MMCSS）、M2（virtual 绑定单源）、进程环回/`capture_packet`/`StopSignal`/`owner.rs` COM 工具共享**均已完成**；剩余工作集中在能力对齐 → 切换 → 收敛 → 删除。

1. **能力对齐（默认路径切 professional 前的阻塞项，对应 WA-1）**
   - a. **latency profile 映射**：方案 A——`AudioLatencyProfile` → 输出 2 的幂 `buffer_frames`（如 Balanced→512、Low→256，改 `moiren-app/src/service/latency.rs:38-51` 映射并让 professional 分支消费）；方案 B——放宽 professional 的 2 的幂约束以接受 480/240。验证：单测（档位→实际协商 period 断言）+ 硬件回退路径日志。
   - b. **捕获时钟适配**：给 professional 捕获加 QPC 前馈（复用 `clock_bridge/rate.rs` 的 `RateClock` 思路接入 `RingSource`）或书面决定接受 occupancy-only 并记录理由。验证：`tests/common/clock_sim.rs` 仿真 + 双栈漂移对比测试。
   - c. **虚拟端点 observation**：`professional/backend/virtual_io.rs::prepare` push `ProfessionalStreamObservation`（对照 legacy `preparation.rs:204-211,330-337` 字段）。验证：`tests/virtual_boundary.rs` + UI 快照。
   - d. **每唤醒遥测**：决定补（`CallbackObservation` 接入 `Pump::cycle`）或放弃（记录在案）。验证：Trace 模式探针输出。
2. **切换默认路由**：`moiren-app/src/service/managed.rs` windows() 工厂恒返回 professional（f32/f64），删除 `settings != default` 分支。验证：`cargo test -p moiren-app --features gpui-test`、`settings_tests`、真实硬件冒烟（默认端点三种 role、默认 Shared 48k、VAD 全虚拟项目、进程环回）。
3. **观察体系收敛**（WA-4）：以 `ProfessionalObservationSnapshot` 为准，`service/model.rs:182-189` 与 `ui/state.rs` 双分支合一。验证：UI 快照 + 人工核对。
4. **中立层解耦**（WA-5）：`resample`/`servo` 移出 `professional/`；删除 `VirtualPreparedTransport::with_native_observations` 及 `virtual_owner/owner.rs` 的 `engine_backend` 观察 import。验证：全量 `cargo test -p moiren-windows-audio`。
5. **删除 legacy 栈**：`engine_backend/`（~2.9k 非测试行）、`capture/wasapi` 物理路径与 `capture.rs` 物理部分（进程环回 `CaptureSession` 保留——professional 依赖）、`render/`（~1.9k）、`shared_stream/wasapi.rs`+`selection.rs`+策略类型（`format.rs` 的 `catalog_support` 迁入 catalog 侧保留）、app 侧 `service/latency.rs:38-51` 的 `windows_backend_config`、`driver.rs:18-21` 拒绝文案与 SetSend legacy-rebuild 分支。合计可删 **~7.5k 非测试 LOC / ~44 文件**。验证：workspace build/test 全绿 + clippy -D warnings。
6. **关闭 D5**：随 legacy 删除，SetSend 走 managed 全量映射，决策记录改为 resolved-wontfix。
7. **文档收尾**：AGENTS.md/docs 中双后端表述、`negotiation.rs:9-10` 引用的 remediation 计划补终态说明。

## 5. 明确不建议合并项

1. **negotiation.rs 的 D4 参数化分歧**（`EnginePeriodicityPolicy`、`LegacySharedInit`）：professional exact-or-fail 与 legacy NOPERSIST/换算标志是各自既定诊断契约——已有显式参数与钉扎测试，保持。
2. **clock.rs vs clock_bridge/**：离线拟合诊断 vs RT 捕获自适应，数据流、约束、消费方全不同；合并只会制造耦合。
3. **professional/codec.rs vs vad/format.rs**：同为 LE PCM 解码，但前者服务 WASAPI 描述符（valid-bits/容器位宽/1..32 声道 → StereoFrame f64），后者是内核 ABI 冻结线格式（mono/stereo、精确长度契约 → f32），且被 `abi_conformance` 钉住。合并会把冻结 ABI 与协商逻辑耦死。
4. **legacy 渲染模型 vs professional Pump**：legacy 是 render owner 线程内直接驱动 Engine，professional 是 master worker 经 rtrb 环驱动。执行模型不同，不值得抽象统一——legacy 直接随退役删除。
5. **swdevice/probe/physical/exclusive_probe**（~1.8k 行，probe-tools 特性门控，`lib.rs:31-45`）：诊断工具，已复用 negotiation 的 exclusive 原语，保留。
6. **`tests/common/alloc_guard.rs` vs `src/render/wasapi/stream.rs` 内嵌 guard**：每二进制一个 `#[global_allocator]` 的机制约束，注释已说明。

## 6. 统计信号

- **legacy 栈**：~7590 非测试 LOC / 44 文件（engine_backend 2,959 + capture 1,010 + render 1,902 + shared_stream 782 + 模块根 966）；随退役可删约 7.5k。
- **professional 栈**：8,065 非测试 LOC / 21 文件（WASAPI 侧 ~4.8k + ASIO 3,252）。
- **VAD**：9,736 LOC / 19 文件（非测试 ~5.8k）；层次清晰（protocol→client→control_io→pcm→provider），未发现内部重叠。
- **两栈共享的基础设施**（退役后的存留内核）：negotiation 743、virtual_owner ~1,938、clock_bridge ~1,590、latency 555、process_loopback 552、session 370、owner 313、stats 300、capture_packet 103。
- **测试结构**：tests/ 5,909 LOC（20 个二进制 + common harness）、src 内嵌测试 12,665、examples 6,027；tests/ 的 spawn 位点从 ~104 → **0**。
- **双观察类型在 app 侧的消费点**：8 处非测试引用。

**结论**：本 crate 已从"平行双栈"演进为"共享内核 + 两套编排"；下一刀不在抽取而在第 4 节的 1→2→5（对齐→切换→删除），删除后 crate 非测试体量预计缩减约 25%。
