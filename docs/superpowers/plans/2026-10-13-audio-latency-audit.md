# Moiren 音频内核时延审计与改进 Implementation Plan

> **For agentic workers:** 使用 `superpowers:executing-plans` 按任务逐项执行；步骤使用复选框追踪。本次交付范围为审计、实验与改进计划，音频实现改动由后续任务执行。

**Goal:** 在有测量依据和稳定性约束的前提下，压低 Moiren 的输入到输出时延、控制到声音时延及启动等待。

**Architecture:** 保留 Render master、可变处理帧数、离线准备与控制侧回收。先建立时延观测，再联合优化 WASAPI 周期、Capture Bridge 存量与 Render 排队；内核计算、数据拷贝及其他传输后端按独立切片推进。

**Tech Stack:** Rust 2024、WASAPI、IAudioClient3、MMCSS、rtrb、有界信号与现有 Compiler / Engine。

## Global Constraints

- 日期：2026-10-13；审计 HEAD：`197859c552930cd4d8ec8cb666beb38375463de3`。
- 审计对象：当前工作树的 Engine、Windows audio、应用 Monitor / Service；已有 UI 与 Cargo 改动未纳入本次修改。
- Prepare graphs, buffers, and signal bindings off the audio thread.
- Real-time processing must avoid heap allocation/deallocation and blocking; reclaim retired plans on the control side.
- Keep logical topology separate from prepared execution plans and platform transports.
- 采用 `rust-toolchain.toml` 指定的 stable；保持 Ubuntu / Windows 与默认 / Tree Borrows Miri 可移植测试。
- 时延策略优先放在 `moiren-windows-audio`；Engine 只接收必要的处理与时延契约；设备策略不进入 LogicalGraph。
- 配置目标、估计时延、历史观测、当前实测必须分开记录；任何档位都不得以爆音、丢帧或隐藏降级换取较低数字。

---

## 1. 审计结论

当前最大改进空间在设备周期和音频排队。现有 Engine 已按设备实际 demand 同步执行，`max_block_frames = 256` 是单次处理上限，不能解释成每经过一个节点就增加 5.33 ms。

优先处理四件事：

1. **默认 Capture Bridge 目标偏保守。** 固定保留 2048 个原生输入帧，在 48 kHz 下约 42.67 ms，在 44.1 kHz 下约 46.44 ms。正式物理输入与 Process Loopback 共用这个默认值，应用无法选择较低目标。
2. **Shared 低周期 API 尚未用于初始化。** Capture / Render 都调用旧 `IAudioClient::Initialize(..., 0, 0, ...)`。Render 查询 IAudioClient3 仅用于报告当前周期，没有请求较小周期。
3. **Render 每次补满整个 endpoint buffer。** 启动预填 capacity，稳态提交 `capacity - padding`。历史 FreeDSP capacity 为 1056 帧，约 22 ms；降低软件 block 不会自行降低这段排队。
4. **缺少端到端测量与运行中音频年龄上限。** 现有 last QPC 指向最近收到的有效包时间戳，即使包因 overflow 完全丢弃也会更新，不能代表正在输出的老帧。只在 prime 时裁剪，运行中大积压由慢速漂移控制或后续 reset 消化。

2048 帧目标与历史 1056 帧 capacity 的标称时间相加约 **64.67 ms**。这个数只是两项配置尺度的说明，**不是测得的输入到输出时延，也不是严格下限**：实际 FIFO 随包相位、需求分块和 servo 变化；设备、APO、ADC / DAC 等延迟也尚未计入。

本次另以现有 Clock Bridge API 做了 22 组离线实验。结果直接否定了“统一把目标改成 128 / 256 帧”的做法：设备仍以 480 帧交付时，256 帧目标持续短读；128 帧设备周期下，256 帧目标也会在负漂移与丢一包再补到的模拟条件下欠载。降低目标必须连同周期、抖动余量与 servo 一起设计。

## 2. 时延口径与数据路径

### 2.1 当前 Monitor 路径

```mermaid
flowchart LR
    ADC["设备 / ADC / 驱动 / APO"] --> Capture["Shared Capture：事件与包"]
    Capture --> Queue["跨线程 SPSC：原生输入帧"]
    Queue --> Bridge["ClockSource：插值 / 漂移控制"]
    Bridge --> Graph["Input → Send → Gain → Send → Pan → Send → Output"]
    Graph --> Output["同一 Render owner 内的软件输出桥"]
    Output --> Staging["interleaved staging"]
    Staging --> Native["WASAPI 排队 / Windows 引擎"]
    Native --> DAC["驱动 / DAC / 设备处理"]
```

Source / Sink 之间的 Send 来自当前 Compiler 的每边独立 buffer 策略。Graph 中这些操作在同一次调用中串行完成；软件输出桥写入后立即由同一 owner 读取，没有另起线程等待一个 block。

Process Loopback 的源头还包含目标应用自身的渲染与 Windows 转换，其时延上限不能仅靠 Moiren 的 buffer 设定保证。

### 2.2 分别衡量三类时延

| 口径 | 起点 → 终点 | 主要影响因素 |
| --- | --- | --- |
| 连续音频时延 | 已进入输入端的声音 → 输出端实际呈现 | Capture 周期、FIFO 年龄、DSP 算法、Render 排队、系统与硬件 |
| 控制到声音 | 用户 / MIDI 请求 → 首个受影响样本实际呈现 | 消息派发、事件调度、已预先算好的 Render 队列 |
| 启动 / 恢复 | Start 请求 → 首段有效声音 | 激活、COM / graph 准备、gate、prime、设备 Start |

Engine 时间线是已经处理的帧；它可以领先于实际听到的声音。控制 reply 的 effective frame 不是换能器呈现时间。

用于设计的预算表达式：

```text
L_total ≈ L_capture_delivery + L_bridge_age + L_algorithm
        + L_processing_and_scheduling + L_render_queue
        + L_remaining_system_and_hardware

bridge_age_ms = 原生输入队列中最老待用帧的年龄
queue_time_ms = queued_frames × 1000 / 对应队列的 sample_rate
period_ms = actual_period_frames × 1000 / 对应流的 sample_rate
```

各项边界必须互不重叠。若某测量已经包含驱动排队，不能再把同一 driver buffer 或 GetStreamLatency 叠加一次。容量是可容纳的最大帧数，实际已排队帧数才决定队列等待。

## 3. 代码证据与影响

链接指向审计时的代码；后续改动可能使行号变化。

| 编号 / 优先级 | 代码证据 | 判断与改进方向 |
| --- | --- | --- |
| F1 / P1 | [ClockBridgeConfig 默认值](../../../crates/moiren-windows-audio/src/clock_bridge.rs#L35)、[正式流构造](../../../crates/moiren-windows-audio/src/capture/wasapi/stream.rs#L50) | capacity 8192、target 2048，按原生输入帧计量。优先加入按实际周期和抖动预算解析的策略；capacity 不等于必须填满。 |
| F2 / P1 | [Render 初始化](../../../crates/moiren-windows-audio/src/render/wasapi/stream.rs#L117)、[Capture 初始化](../../../crates/moiren-windows-audio/src/capture/wasapi/physical.rs#L53) | 两端旧 Shared 初始化。加入 capability query、合法 period 选择和实际结果报告；不能把 GetDevicePeriod 的旧 minimum 当成 Shared 最小周期。 |
| F3 / P1 | [启动预填](../../../crates/moiren-windows-audio/src/render/wasapi/stream.rs#L154)、[逐 wake 补满](../../../crates/moiren-windows-audio/src/render/wasapi/stream.rs#L190) | prefill 与稳态目标均由容量决定。新增独立的 render target；回填量受可用空间与下一次 deadline 约束。 |
| F4 / P1 | [prime 裁剪](../../../crates/moiren-windows-audio/src/clock_bridge/source.rs#L94)、[overflow 丢新尾](../../../crates/moiren-windows-audio/src/clock_bridge/ingress.rs#L92) | 启动能去除旧帧，运行中没有音频年龄上限。实时监听需要 consumer 控制的有界追赶策略；离线录音保留 FIFO 语义。 |
| F5 / P1 | [250 ms 滤波与 PI](../../../crates/moiren-windows-audio/src/clock_bridge/source.rs#L61) | 限幅 ±2000 ppm 有利于稳定，但处理突发积压慢。区分慢速 drift 与快速 backlog recovery；低 target 下重新校准队列采样相位与启动状态。 |
| F6 / P0 | [Bridge 标量](../../../crates/moiren-windows-audio/src/clock_bridge/telemetry.rs#L8)、[Render 报告](../../../crates/moiren-windows-audio/src/render.rs#L220) | 没有 FIFO 年龄分位数、wake jitter、DSP WCET、render padding 分布、呈现时间与端到端相关性。先建立测量。 |
| F7 / P2 | [每边独立 Send / slot](../../../crates/moiren-engine/src/compiler/plan.rs#L103)、[每 op 清输出](../../../crates/moiren-engine/src/runtime.rs#L384) | 数据复制、清零和多次扫描影响 CPU 余量；是小周期稳定性的障碍，不是按节点叠加的固定音频时延。 |
| F8 / P2 | [同步输出桥](../../../crates/moiren-windows-audio/src/render.rs#L179)、[staging 拷贝到 native](../../../crates/moiren-windows-audio/src/render/wasapi/packet.rs#L1) | 同 owner 仍经过 planar → SPSC → interleaved staging → native。可减少复制，保持失败初始化及借用契约。 |
| F9 / P2 | [逐帧插值读取](../../../crates/moiren-windows-audio/src/clock_bridge/source.rs#L121)、[额外输出统计扫描](../../../crates/moiren-windows-audio/src/render/wasapi/stream.rs#L83) | chunk 读取、统计合并与降低诊断扫描成本值得基准测试。已有 signal meter 是按需求激活，继续保持。 |
| F10 / P2 | [RT 处理 / 切事件](../../../crates/moiren-engine/src/runtime.rs#L348)、[控制服务等待](../../../crates/moiren-app/src/service/mod.rs#L267)、[NextBlock 请求](../../../crates/moiren-app/src/service/parameters.rs#L48) | 64 个事件可切出较多全图 segments；服务 park 最长 10 ms，但请求可 unpark，不能固定计入每次控制延迟。需要测控制派发与呈现。 |
| F11 / P2 | [latency metadata](../../../crates/moiren-engine/src/processor/mod.rs#L52)、[swap 兼容检查](../../../crates/moiren-engine/src/runtime/swap.rs#L590) | Processor latency 已有字段，Compiler 尚未传播、报告和补偿各路径。当前 Gain / Pan / Bus / Compressor 没有排队 lookahead；attack / release 是响应常数，不等于音频传输延迟。 |
| F12 / P2 | [Process Loopback 初始化](../../../crates/moiren-windows-audio/src/process_loopback/stream.rs#L99)、[原生 MMCSS](../../../crates/moiren-windows-audio/src/capture/wasapi/stream.rs#L84) | Process input 请求 Windows 转成 48 kHz stereo，同时仍走 ClockSource；可评估转换与调度，但不能直接套用物理 endpoint 的 period 能力。MMCSS Audio 已接入，无需把“加入 MMCSS”重复列作未实现功能。 |

已经具备的有利基础：事件驱动、单一 Render master、可变 demand、离线分配、完整帧 SPSC、overflow / underrun 可见、停止事件、Graph 与设备解耦、signal demand gating、无分配 / 释放回归测试。

## 4. 历史数据与本轮验证

### 4.1 历史实机：2026-10-08 / 2026-10-09

以下取自本机已有验收记录，不是本轮重新测量。原记录位于忽略的 Windows 实验目录，关键数字复制在本报告中，避免依赖未提交文件才能理解结论。

| 场景 | 历史观测 | 可得结论 |
| --- | --- | --- |
| FreeDSP Shared 输出，2026-10-08，10 秒 | actual period 480 帧；capacity 1056 帧；每 wake demand 480 帧；GetStreamLatency = 0 | 设备当时以约 10 ms 周期运行；0 API 值不能证明零端到端时延。 |
| FreeDSP 物理输入 → 输出，2026-10-09，10 秒 | bridge final / min / max fill = 1911 / 1218 / 2166 帧；underrun / dropped / resets = 0 / 0 / 0；初始裁剪 3712 帧 | 48 kHz 下这些观测约为 39.81 / 25.38 / 45.13 ms 的 FIFO 存量尺度，已是显著软件时延。不能由独立 scalar snapshot 得到每个输出样本的精确年龄。 |
| 同一物理闭环的启动 | Capture 比 Render 先运行约 134 ms；后续共同停止 | 主观“启动慢”需要和连续音频时延分开。当前 gate 改善了准备顺序，历史启动数字不能直接套到当前。 |
| 既有模拟时钟长测 | 44.1 / 48 kHz × ±1000 ppm，各 7200 秒模拟音频时长；使用默认 target 2048 | 证明原保守配置在该模拟下稳定，不能外推到 128 / 256 帧目标或真实系统调度。 |

### 4.2 本轮自动化

2026-10-13 在当前工作树运行：

```powershell
cargo test --locked -p moiren-engine -p moiren-windows-audio
cargo build --locked -p moiren-windows-audio
```

均成功。覆盖现有 runtime、compiler、control、signal、plan swap、RT allocation / deallocation、packet lease、停止、variable demand 与 Clock Bridge 回归；既有 drift 测试按默认的每组 120 秒模拟运行。本轮没有开启麦克风、播放声音或开展新的实机环回测量。

### 4.3 本轮 22 组目标填充实验

源码：[simulation.rs](2026-10-13-audio-latency-audit/simulation.rs)。完整结果：[results.csv](2026-10-13-audio-latency-audit/results.csv)。

实验直接调用现有 `capture_bridge` / `ClockSource`：48 kHz、120 秒模拟时长、capacity 8192、默认 PI / ±2000 ppm、max segment 256。采集先于同一时刻的 render；jitter 表示每第 7 个采集事件延后到下一个事件一并交付。包含 480 / 128 帧同周期、480 capture / 128 render 混合周期、±1000 ppm。

| Capture / Render 周期 | target | 抖动 | 漂移 ppm | short callbacks | underrun frames |
| --- | ---: | --- | ---: | ---: | ---: |
| 480 / 480 | 2048 | 无 | 0 | 0 | 0 |
| 480 / 480 | 1024 | 一包延后 | -1000 | 0 | 0 |
| 480 / 480 | 1024 | 一包延后 | +1000 | 0 | 0 |
| 480 / 480 | 512 | 一包延后 | 0 | 1714 | 383936 |
| 480 / 480 | 512 | 无 | -1000 | 191 | 96 |
| 480 / 480 | 256 | 无 | 0 | 11999 | 12000 |
| 128 / 128 | 256 | 一包延后 | 0 | 0 | 0 |
| 128 / 128 | 256 | 一包延后 | -1000 | 5759 | 5091 |
| 128 / 128 | 512 | 一包延后 | -1000 | 0 | 0 |
| 128 / 128 | 512 | 一包延后 | +1000 | 0 | 0 |
| 480 / 128 | 256 | 无 | 0 | 20998 | 1535872 |
| 480 / 128 | 512 | 无 | 0 | 0 | 0 |

short callback 是出现过有效音频后、一次请求未完整转移的回调数，包含之后的 re-prime 静音。underrun frames 是现有 Bridge 的欠载尾帧统计，两者口径不同；不能把 short callback 数当成欠载事件数。所有 22 组 dropped frames 为 0。

结论：

- 原 480 帧周期下，1024 帧是有依据的**下一轮实验候选**，标称目标降低 21.33 ms；仍需覆盖启动预填 1056 帧、不同事件相位、真实系统调度和更长时长。
- 512 帧目标在理想零漂移下能运行，但负漂移或抖动时失败，不能作为当前默认替换。
- 小 Render block 不足以弥补大 Capture period；输入端周期必须一并缩短。
- 128 帧同周期下，512 帧目标通过本轮正负漂移与抖动模拟，是较保守的小周期实验起点；256 帧需要改进 servo / 安全余量后再尝试。
- 模拟没有 WASAPI 初始全容量预填、线程迟醒、DPC / ISR、APO 或硬件延迟，也不构成音质、实机 deadline 或端到端时延验收。

Windows 复现命令（输出目录位于已忽略的 `.tmp`）：

```powershell
cargo build --locked -p moiren-windows-audio
if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed.' }
New-Item -ItemType Directory -Force -Path .tmp/audio-latency-audit | Out-Null
rustc --edition=2024 -O docs/superpowers/plans/2026-10-13-audio-latency-audit/simulation.rs --extern moiren_windows_audio=target/debug/libmoiren_windows_audio.rlib -L dependency=target/debug/deps -o .tmp/audio-latency-audit/simulation.exe
if ($LASTEXITCODE -ne 0) { throw 'Simulation compilation failed.' }
& ./.tmp/audio-latency-audit/simulation.exe | Set-Content -Encoding utf8 .tmp/audio-latency-audit/reproduced.csv
if ($LASTEXITCODE -ne 0) { throw 'Simulation failed.' }
$expected = Get-Content -LiteralPath docs/superpowers/plans/2026-10-13-audio-latency-audit/results.csv
$actual = Get-Content -LiteralPath .tmp/audio-latency-audit/reproduced.csv
if ($expected.Count -ne 23 -or $actual.Count -ne 23) { throw 'Expected 22 simulation cases plus the CSV header.' }
$differences = Compare-Object -ReferenceObject $expected -DifferenceObject $actual -SyncWindow 0
if ($differences) { $differences; throw 'Simulation results differ from the saved CSV.' }
Write-Output 'All 22 simulation cases match the saved CSV in row order.'
```

提交前复核：上述 Engine / Windows audio 测试与构建再次通过，22 组模拟结果逐行对应保存的 CSV；实验源码通过 `rustfmt --edition 2024 --check`。这次复核同样未启动音频设备。

## 5. 尽可能压低时延的措施清单

“直接”表示减少算法或排队时间；“间接”表示增加 deadline 余量，允许后续使用更小周期。优先级 P0 为测量与准入，P1 为直接高收益，P2 为优化 / 配套，P3 为扩展后端或专用环境。

### 5.1 设备周期与排队

| 编号 | 优先级 / 收益 | 措施与适用边界 |
| --- | --- | --- |
| 1 | P0 / 测量 | 同时报告两端 default / fundamental / min / max / requested / actual period、capacity、prefill 和实际 padding 分布。未知能力保持 unknown。 |
| 2 | P1 / 直接 | Render 使用 IAudioClient3::GetSharedModeEnginePeriod + InitializeSharedAudioStream 选择较小合法 Shared 周期。 |
| 3 | P1 / 直接 | Physical Capture 同样使用小 Shared 周期；仅改输出端不够。分别查询各自格式、category 与 options 下的能力。[S1–S3] |
| 4 | P1 / 直接 | 解除 target 2048 的固定值，按两端周期、最大实际 demand、包相位、抖动与插值余量解析输入目标。 |
| 5 | P1 / 直接 | 解耦 SPSC capacity 与 target：容量保留突发余量，但稳态不主动填满。 |
| 6 | P1 / 直接 | Render 按独立 target padding 回填，避免每次填满 capacity；保留足够的下一次 wake / DSP deadline 余量。[S4] |
| 7 | P1 / 直接 | 启动仅预填已解析的 render target，并保证 Capture priming 足以支持该需求；不要先塞入一大段静音或不足输入的 PCM。 |
| 8 | P1 / 直接 | 给实时监听加入最大音频年龄与 high-water mark；在 consumer 一侧批量丢弃过旧帧，统计明确区分 intentional skip 与 overflow。 |
| 9 | P1 / 直接 | backlog recovery 使用有界快速追赶 / 重定位，drift servo 只修正长期速率差；不能用持续极端 SRC 倍率追赶几十毫秒积压。 |
| 10 | P2 / 稳定 | 支持 Compatible / Balanced / Low / Custom 策略与明确 fallback 原因。标定得到的稳定档位才允许成为设备配置。 |

### 5.2 跨钟与 SRC

| 编号 | 优先级 / 收益 | 措施与适用边界 |
| --- | --- | --- |
| 11 | P1 / 稳定 | 重新定义低目标下 servo 的采样参考：测 wake 前后队列锯齿及分段消费，控制平均占用，同时保住最小安全存量。 |
| 12 | P1 / 稳定 | 以 capture position / QPC 长窗口估算真实速率作为 feed-forward，FIFO PI 做较慢微调；TIMESTAMP_ERROR / 零位置源不能走无效估计。[S5] |
| 13 | P1 / 稳定 | prime / reset 初始化 filtered fill、积分与比率；区分刚启动、稳定、欠载恢复。两帧插值余量与完整 device demand 分别计算。 |
| 14 | P2 / 间接 | ClockSource 评估 rtrb chunk 两段读取，减少逐帧 pop 的环操作；批次与 generation 边界保持正确。 |
| 15 | P2 / 直接 | 已验证共同采样时钟且同采样率时，允许无 ASRC 的直通或更浅缓冲。相同设备名称 / nominal sample rate 不构成共钟证据。 |
| 16 | P2 / 直接 | 让 processing sample rate 可匹配 master 的 native rate，减少不必要 SRC；48 / 44.1 kHz 分开预算。更高采样率只在 CPU / transport 允许时实验。 |
| 17 | P2 / 音质约束 | 未来更高质量 SRC 用明确的 group delay 报告，允许 causal 低延迟与高质量档位选择；更长 FIR 会消耗时延预算。 |
| 18 | P2 / 稳定 | 多设备按 clock domain 放置必要的 ASRC；同域避免串联两个 bridge，跨域保留速率适配与最小 jitter buffer。 |

### 5.3 内核计算与内存路径

| 编号 | 优先级 / 收益 | 措施与适用边界 |
| --- | --- | --- |
| 19 | P0 / 测量 | 测 capture wake→packet release、render wake→ReleaseBuffer、Engine block / segment 的 p50 / p95 / p99 / max，分别归因等待和计算。 |
| 20 | P2 / 间接 | 根据实际 period 测 64 / 128 / 256 / 480 / 512 的处理上限；保留 variable frames。block 越小调用与调度开销越高。 |
| 21 | P2 / 间接 | 对无 fan-out、无 observer 冲突的边做 liveness / in-place 规划，减少每边 Send 和 arena 存量。 |
| 22 | P2 / 间接 | 边 Send 的 identity 快路径保留可自动化参数语义；不能因编译时 gain=1 就永久删除以后可变化的参数。 |
| 23 | P2 / 间接 | 同 owner 输出用准备好的直接 Sink / block adapter 替代 SPSC，先写 staging，最后一次交给 native。跨线程输出仍用桥。 |
| 24 | P2 / 间接 | 在 lease / failure 契约证明后，比较直接写 WASAPI native buffer 与 staging 方案；GetBuffer→ReleaseBuffer 的持有时间必须受 deadline 约束。 |
| 25 | P2 / 间接 | 去除经过证明的重复清零，合并 interleave 与输出统计；完整覆盖标记、短写静音和任意安全 processor 的行为仍需回归。 |
| 26 | P2 / 间接 | 对 Gain / Pan / Sum / SRC 热点评估 SIMD、连续内存和状态布局；有依据地减少虚调用或合并操作。数值与自动化语义保持。 |
| 27 | P2 / 间接 | 发布版测性能；准备阶段触碰 arena / stacks / tables，避免首个小周期发生 page fault / lazy initialization；大 graph 尤其需要。 |
| 28 | P2 / 间接 | 诊断原子按 block / packet 汇总，减少共享 cache line 竞争；可选 profiling 关闭时低开销，observer 不可向音频施加 backpressure。 |
| 29 | P2 / 间接 | 检查 Compressor 静音尾部的 subnormal 成本；仅在数值验收后设置并恢复 worker 的 FTZ / DAZ，避免改动 UI / control 线程状态。 |
| 30 | P2 / 间接 | 限制极端自动化造成的 segment / 全图重跑成本；UI 覆盖式控制可在 control 侧合并，sample-accurate 事件保持顺序和回执。 |

### 5.4 调度、启动与控制

| 编号 | 优先级 / 收益 | 措施与适用边界 |
| --- | --- | --- |
| 31 | P2 / 稳定 | 现有 MMCSS Audio 与 Pro Audio、AvSetMmThreadPriority 做受控比较，报告注册 / priority 失败；不修改全局 MMCSS 注册表。[S6] |
| 32 | P2 / 稳定 | 将首次 DSP / prefill 纳入适当 MMCSS 区间；音频 owner 不执行设备枚举、UI、JSON 序列化或日志输出。 |
| 33 | P2 / 稳定 | Capture drain 设置 packet / frame 工作预算，每次及时 ReleaseBuffer；超限报告 backlog 并按 age 策略追赶，避免不停追生产者。 |
| 34 | P2 / 直接 | 缓存可复用的 graph 资源与设备能力，准备并行进行，最后重新核实设备 / actual period；改善 Start→首个有效音频，避免准备时间混入 steady latency。 |
| 35 | P2 / 直接 | 同时测 request enqueue、service dispatch、control submit、effective frame 与 expected presentation；基于 unpark 唤醒处理关键请求。 |
| 36 | P2 / 直接 | 保持按样本 / 下一块的控制；Render 队列越深，已经算好的旧参数样本越多，先降低音频预先计算的距离再讨论 UI polling。 |
| 37 | P2 / 稳定 | 小周期降级通过 control 侧有序停启、fade 与重新准备实现；连续延迟降低不在 RT 上重新分配环或重建 COM。 |

### 5.5 DSP、系统与其他后端

| 编号 | 优先级 / 收益 | 措施与适用边界 |
| --- | --- | --- |
| 38 | P2 / 直接 | 对每输出路径报告 processor / SRC / plugin latency；实时监听优先零 lookahead、causal / minimum-phase 算法。 |
| 39 | P2 / 对齐 | Bus 等混音点只为必要对齐添加补偿；实时监听可显式选择较低总时延。sidechain 不能机械按普通混音输入规则处理。 |
| 40 | P2 / 条件直接 | 显式提供 RAW / processing mode 选项，与普通模式 A/B；RAW 仍可能有 always-on APO / driver / hardware 处理，不保证全部绕过。[S7] |
| 41 | P3 / 直接 | 独立实现 WASAPI Exclusive event 后端，支持设备原生 PCM、格式与 buffer 对齐协商；它的事件缓冲规则不能照搬 Shared padding target。[S8] |
| 42 | P3 / 直接 | 对具备原生 ASIO 的专业设备实现 ASIO driver callback 后端，评估同一驱动全双工、64 / 128 帧。收益取决于具体驱动与硬件实测。 |
| 43 | P3 / 条件直接 | Process Loopback 记录自身可用能力、实际包间隔与 Windows 转换；不要假设 IAudioClient3 小周期初始化及旧 flags 都可用。 |
| 44 | P3 / 条件直接 | 后续虚拟 endpoint / IPC 的每级 buffering 分别预算：共享内存 SPSC、event 通知、单一主时钟，避免重复 block batch。 |
| 45 | P3 / 稳定 | 用 WPR / WPA / ETW 分析迟醒、DPC / ISR、page fault 与 CPU 迁移；仅有归因证据时测试 affinity / power profile / USB 拓扑。 |
| 46 | P3 / 条件直接 | USB wired / native low-buffer 设备、设备 DSP、Bluetooth / 无线 transport 分开测；Moiren 只能减少其自身排队。硬件 direct monitor 可更低，但会绕过 Graph 的处理效果。 |

## 6. 预算、候选档位与收益

### 6.1 帧到时间

| 帧数 | 48 kHz | 44.1 kHz |
| ---: | ---: | ---: |
| 64 | 1.33 ms | 1.45 ms |
| 128 | 2.67 ms | 2.90 ms |
| 256 | 5.33 ms | 5.80 ms |
| 480 | 10.00 ms | 10.88 ms |
| 512 | 10.67 ms | 11.61 ms |
| 1024 | 21.33 ms | 23.22 ms |
| 2048 | 42.67 ms | 46.44 ms |
| 8192 | 170.67 ms | 185.76 ms |

Capture target 用输入 rate；Render target 用输出 rate；不能把 44.1 kHz 的帧直接当作 48 kHz 帧。

### 6.2 首轮实验档位

| 档位 | 设备周期 | Bridge / Render 策略 | 准入与目标 |
| --- | --- | --- | --- |
| Compatible | 旧 Shared 默认 | 保留现有 target / full-capacity 策略，补观测 | 保持已有兼容性，建立真实 baseline。 |
| Balanced | 先保持 actual 480 / 480，之后尝试合法 3–5 ms | 首测 target 1024；Render target 至少覆盖一完整周期与实测 jitter / WCET 余量 | 本轮模拟提供 target 1024 的候选依据；先争取减少至少 15 ms 的实测 median，稳定性不能退化。 |
| Low | 两端实际周期均可到约 2–3 ms | 首测 input target 512，再在改进 servo 后尝试 256；输出约 1–2 periods 加余量 | 支持设备上实测物理 input→output 的 p95 ≤ 20 ms 为目标；若硬件 / APO 达不到，报告分项与最佳稳定值。 |
| Custom / 专业 | 经过协商的 64 / 128 等帧，或 Exclusive / ASIO | 按实际时钟域和 deadline 确定目标，不能固定复制 Shared 配置 | p95 ≤ 10 ms 作为条件目标；需具体硬件确认，不能作为通用承诺。 |

Balanced / Low 的周期值是意图，实际请求必须是 fundamental 的整数倍且在 min–max 范围内。[S2] 配置未通过准入前不替换默认策略。

仅输入目标从 2048 → 1024 / 512 / 256，48 kHz 下的标称目标差分别为 21.33 / 32.00 / 37.33 ms。Render target 若能从历史 1056 → 576 帧，标称排队上限再减少 10.00 ms；576 帧只是旧 480 帧周期下的实验候选，96 帧余量是否够用必须由 jitter / WCET 验证。这些差值不能相加后直接宣布端到端降低了相同时间。

### 6.3 解析规则

设 P 是输出实际 period，C 是最大输入包交付间隔对应的原生输入帧，D 是最大 render demand（输出帧）；J 是实测调度 / 包 jitter 对应的原生输入帧预算，H 是插值需要的输入前瞻。

- input target 必须支持 startup demand 和 steady demand，并覆盖跨线程事件相位、输入批量交付空窗、J、H。
- 可将 `ceil(D × input_rate / output_rate) + C + J + H` 作为保守起点，再通过 trace / phase sweep 缩减。
- Render target 起点为 `P + ceil((J_wake + T_callback_budget) × output_rate)`，其中两个时间预算用秒，受 capacity 和驱动约束限制；达不到安全预算就提高 period / fallback，不能简单 clamp 后宣称安全。
- input target 的 queue sampling reference 必须定义清楚。当前各 segment 前采样使均值与 wake 前占用不同；priming threshold、servo 平衡点、low-water 和 capacity 不宜共用一个裸数。
- capacity 至少覆盖 target、最大 packet burst 与恢复策略所需余量；active high-water / age limit 控制排队上界。

## 7. 分阶段实施计划

这些切片分别有独立验收。T1–T5 是直接降时延的主线，T6–T8 可按测量结果排期；Exclusive / ASIO 分别提交独立设计与实现计划。

### T1：建立时延观测与基线（P0）

**文件职责：** 新增 `crates/moiren-windows-audio/src/latency.rs` 作为模块入口、`src/latency/telemetry.rs` 与 `tests/latency.rs`；在 `src/lib.rs` 导出模块；修改 `render.rs`、`capture.rs`、`clock_bridge/telemetry.rs`、两端 stream、`moiren-app/src/monitor/windows.rs`。新增 `examples/latency_probe.rs` 负责显式选择设备后的测量与标量输出。

**接口：** 每 owner 使用准备好的固定 histogram / trace ring，记录 `wake_qpc_100ns`、`demand_frames`、`padding_before_frames`、`submitted_frames`、`callback_duration_100ns`。控制侧导出 p50 / p95 / p99 / max 与 trace dropped，不从 RT 生成 JSON。以 schema 版本明确新增字段。

- [ ] 写 histogram 边界、counter overflow、禁用 profiling、trace 满载和独立 snapshot 的回归；测试 trace 满只丢诊断，不影响完整音频转移。
- [ ] 两端 owner 内采时间，render 每次 ReleaseBuffer 后记录完成；控制侧合并结果。用同一 QPC 时间单位，避免把 raw QPC tick 当作 100 ns。[S5, S9]
- [ ] 增加测量方法标签：估计 / 软件环回 / 物理线缆 / 外部双通道；来源未知保持 unknown。GetStreamLatency 保留原始值，不当作真实 E2E。[S10]
- [ ] `cargo test --locked -p moiren-windows-audio --test latency`、现有 render / clock_bridge / RT 无分配回归全部通过。
- [ ] 建立同 graph、同设备、同增益与相同系统负载的 baseline；gate 与 Stop 期间不计入 steady latency。提交建议：`feat(audio): add bounded latency telemetry`。

**验收：** 能说明时延主要来自哪一层；开启观测不引入分配、锁或 audio backpressure，观测开关的 callback 成本分别报告。

### T2：协商较小 Shared 周期（P1）

**文件职责：** 扩展 T1 的 `src/latency.rs` 加入纯策略，新增 `src/shared_stream.rs`（owner 内 COM 初始化）；修改 `lib.rs`、`render/wasapi/stream.rs`、`capture/wasapi/physical.rs`、两个 options / report。设备枚举仅补能力展示，不在 RT 查询。

**接口：** `SharedPeriodCaps { default_frames, fundamental_frames, min_frames, max_frames }`；策略输入用期望时间，owner 按本流 rate 换算帧；输出包含 requested、actual、初始化 API 与 fallback 原因。period 选择是纯函数，COM 在原 owner 上执行。

- [ ] 写 min / max、非整数倍、上溢、无 Client3、属性切换后重查能力、初始化失败的 fake COM 测试。
- [ ] 为给定 sample rate / category / options 查询 GetSharedModeEnginePeriod；期望帧向合法 fundamental 倍数取整并检查范围。[S2]
- [ ] 调用 InitializeSharedAudioStream。**新 API 文档明确列出的 stream flag 是 EVENTCALLBACK，不能把旧 NOPERSIST / AUTOCONVERTPCM / SRC_DEFAULT_QUALITY 组合照搬。** Session GUID 与 session flags 按新契约分别处理。[S3]
- [ ] 已占用、格式 / 周期冲突、缺少接口可按策略显式选择兼容路径；无效策略与设备失效分别报错，禁止把配置 bug 隐藏成普通降级。重试时重新 Activate client，保留各次原因。
- [ ] 初始化后核实 actual period / format / capacity；请求值与结果不同时报告事实。Process Loopback 仍用其独立已验证路径。
- [ ] 执行 windows-audio 单测与 all-targets check；硬件比较默认 / 可用最小 / 中间周期，确认 UI 与其他应用共存。提交建议：`feat(audio): negotiate shared engine periods`。

**验收：** 两端可在支持硬件上进入较小实际周期；fallback 清晰，不错误引用旧 GetDevicePeriod minimum。

### T3：联合解析输入目标与渲染目标（P1）

**文件职责：** 修改 `latency.rs`、`capture/wasapi/stream.rs`、`render.rs`、`render/wasapi/stream.rs`、`moiren-app/src/monitor/windows.rs`、`service/model.rs` 与 `service/windows.rs`；测试放 `tests/latency.rs`、`tests/render.rs` 及 app 现有 session 测试。

**接口：** `PreparedLatencyBudget` 包含两端实际 rate / period、input prime / servo / low-water / high-water 目标、render target 与 policy。它在激活之前一次性解析，启动后的音频 owner 读取不可变配置。

- [ ] 增加两阶段准备：两端 owner 先持有 native client 并返回纯 Rust capability / actual 信息；控制侧解析预算、建立桥与 graph，再把 renderer 交给 Render owner；所有 COM 仍留在其创建线程。旧准备顺序在 capture.source 构造时尚无 render actual period，不能靠缓存猜测。
- [ ] 将 Bridge 默认参数变成显式准备参数，分别区分 capacity、prime threshold、servo reference、最小余量和 backlog 上限。
- [ ] 以纯函数计算 target 回填量，并让启动预填采用同一 target。一个可直接测试的策略核心如下；policy 解析阶段另验证 target 是否覆盖安全 deadline：

```rust
fn target_demand(capacity: u32, padding: u32, target: u32) -> Option<u32> {
    if capacity == 0 || padding > capacity || target == 0 || target > capacity {
        return None;
    }
    Some(target.saturating_sub(padding).min(capacity - padding))
}

#[test]
fn target_refill_does_not_fill_native_capacity() {
    assert_eq!(target_demand(1056, 96, 576), Some(480));
    assert_eq!(target_demand(1056, 600, 576), Some(0));
    assert_eq!(target_demand(1056, 1057, 576), None);
}
```

- [ ] 覆盖 legacy full prefill、降低 prefill、零 demand、不规则 demand、capacity 变化、两端不同周期、预算不足、共享 gate 取消 / Stop。任何零 demand 都不推进 Engine timeline。
- [ ] 先验证 Balanced target 1024，再测试 Low；迟醒时按可写与年龄预算追赶，记录实际 padding，不能让小 target 的未提交空间造成无声欠载。
- [ ] 执行 `cargo test --locked -p moiren-windows-audio --test render --test latency` 和 app session 测试。提交建议：`feat(audio): prepare joint input and output latency budgets`。

**验收：** 能独立改变两个目标，不必缩小 arena / SPSC capacity；原 Compatible 行为保留，所有初始化失败都清理 owner。

### T4：修正低存量 servo 并限制运行中积压（P1）

实施进度：运行中 high-water 的有界 consumer 丢弃、phase / servo reset、32 帧淡入与独立恢复计数已通过第一切片验证，见 [运行中高水位恢复记录](2026-10-13-backlog-recovery-validation.md)。年龄限制、低库存 servo 与完整矩阵仍待实现，以下整体任务暂不勾选完成。

**文件职责：** 修改 `clock_bridge.rs`、`clock_bridge/source.rs`、`clock_bridge/ingress.rs`、`clock_bridge/telemetry.rs`；新增 `clock_bridge/timing.rs` 存放包时间与年龄推导；扩展 `tests/clock_bridge.rs`。

**接口：** 输入 packet stamp 必须关联 generation、原生首帧位置、QPC 与已接收的连续帧区间；consumer 能知道最老实际待用帧的年龄。元数据使用预分配、有界结构，音频与 stamp 提交关系必须一致；满载时显式失效，不使用最新包时间冒充旧音频时间。

- [ ] 将本轮失败组合纳入 regression，另增加 44.1 kHz、render-first / capture-first、相位 sweep、连续迟包、startup 预填与恢复；新策略必须完整通过 ±1000 ppm，不能靠允许短读通过。
- [ ] 对有可信位置的源估算 rate / QPC 斜率并筛掉异常，给 servo feed-forward。Process source 零位置 / timestamp error 回退到只用 FIFO 的模式。
- [ ] 重新校准低目标的 fill reference / startup integral，使同一设备不同 Engine segment 切分下 drift 收敛相近，并对最低库存作有界保护；不能把零 drift 的 FIFO 锯齿解释成硬件漂移。
- [ ] 达到 high-water / age limit 时，consumer 在一次初始队列快照内丢弃有限数量旧帧，重置相位 / servo，进行短 fade；禁止 producer 越权移动 consumer。跨 generation 禁止插值，intentional skip 与 dropped / discontinuity 独立统计。
- [ ] 同一状态下不会不停 reset；故障持续时通过 control 降档。离线 `trim_on_prime=false` 场景保留完整录音，不启用 age-based discard。
- [ ] 执行 Clock Bridge 与 RT allocation 测试；新增低目标矩阵每组合先模拟 120 秒，合格后扩展至 7200 秒。提交建议：`fix(audio): stabilize low-fill clock adaptation and bound backlog`。

**验收：** 新目标在全部矩阵下无稳定阶段 underrun；暂停 render 后恢复不会持续输出陈旧音频，追赶量和 fade 成本有界。

### T5：标定、暴露档位与实机准入（P1）

**文件职责：** 修改 `service/model.rs`、`service/windows.rs`、`monitor_cli.rs`、现有 UI bindings / device 设置页面及 README；测试扩展 app CLI / service。界面遵循现有 Slint theme，UI 变化附截图。

**接口：** SessionSpec 增加明确 policy；服务 snapshot 返回 requested / actual profile、fallback reason 与测量状态。低延迟能力缺失时显示实际兼容档位。

- [ ] CLI 加 profile / 期望周期 / 显式 target 配置及单位验证；非法或无法满足的 Custom 配置在启动前拒绝。
- [ ] UI 选择档位，显示 actual period、估计队列和已测物理时延；不把 nominal 256 frame blocks 标成总时延。
- [ ] 按第 8 节的负载 / 时长 / 格式矩阵执行候选准入，配置只记录同 endpoint、driver / format / mode 下通过的档位；设备属性变化使标定失效。
- [ ] 对用户选择的档位允许受控回退，运行中调整依照控制侧 fade / Stop / prepare / Start；展示回退原因。
- [ ] 运行 app CLI / session 测试、workspace check / fmt / Clippy；由设备实测结果决定默认是否从 Compatible 升级。提交建议：`feat(app): expose measured audio latency profiles`。

**验收：** 配置确实影响 backend actual 行为；低数字伴随可核实的测量及稳定性结果。

### T6：热点驱动的数据路径优化（P2）

**文件职责：** 先新增 `moiren-engine/examples/render_cost.rs` 的可重复基准；仅热点确认后修改 `compiler/plan.rs`、`buffer/view.rs`、`runtime.rs`、`render.rs`、`render/wasapi/packet.rs` 或 `clock_bridge/source.rs`。Compiler 规划与 Windows staging 各自独立审查。

- [ ] 基准覆盖简单链、fan-out、多输入 Bus、Compressor、observer、64 事件压力，输出 block / callback p99 / max，而不仅 frames/s。
- [ ] 一次只实现一项：Send identity 快路径、slot 生命周期复用、允许的 in-place、直接同 owner Sink、批量 SRC 或统计合并；每项保留前后成本和数值结果。
- [ ] 清零仅在覆盖契约明确且任意 safe processor 仍安全时减少；保留 window 外样本、失败尾部静音、fan-out、sidechain 和 DSP state reuse 测试。
- [ ] 每项后运行 engine / windows-audio 相关回归与无分配测试；通过当前 CI 的两种 Miri，再比较小周期硬件余量。

**验收：** 改进 worst callback 成本且不改变逻辑图语义。成本收益不足则不增加新复杂度；性能优化本身不声称直接减少一个 block 的时延。

### T7：路径时延模型与低延迟 DSP 策略（P2）

**文件职责：** 新增 `moiren-engine/src/compiler/latency.rs`，修改 `processor/mod.rs`、`compiler.rs`、`compiler/plan.rs`、现有 CompiledBindings / report；测试放 `tests/compiler/latency.rs` 并在现有 test main 中注册。

- [ ] 在编译时传播每个输出的算法时延；Source 的动态 transport 年龄另外报告，不能把 drift queue 的瞬时值作为固定 PDC。
- [ ] 用零 / 128 / 256 帧 mock delay 测单链、fan-out、Bus、多输出与独立 sidechain，检查报告路径值。
- [ ] 普通 Bus 求和的显式补偿在控制侧准备 Delay storage；实时监听允许选择不补偿的低延迟模式并展示路径差。sidechain 对齐由 processor 契约决定。
- [ ] 对未来 FFT、linear-phase、oversampling、plugin lookahead / latency change 给出独立 budget；模式改变先准备并换计划，RT 不调整 storage。
- [ ] 运行 compiler、plan swap、allocation 与 DSP 数值回归；当前零延迟内建链不得因为加入模型而多出补偿 buffer。

**验收：** 用户能找到真正增加算法延迟的节点；补偿的额外延迟可解释且可按路由选择。

### T8：专用低延迟后端与系统诊断（P3）

**文件职责：** WASAPI Exclusive 新增 `moiren-windows-audio/src/exclusive/`；ASIO 是否新 crate 由驱动边界决定；都保留现有 Engine boundary。专用 probes 与测量材料放 `examples/` 和 `docs/experiments/`。

- [ ] 为 Exclusive 独立实现格式支持检查、PCM adapter、event buffer 对齐、设备占用 / 失效与 Stop；不复用 Shared 的局部 target 回填算法。[S8]
- [ ] ASIO 首先证明同驱动全双工 callback、block size、sample position、跨线程控制与关闭 / 回收，再接 Graph；不用额外 WASAPI Render queue。
- [ ] 每个后端与最佳 Shared 档位做同条件物理 loopback A/B；无法支持的 format / mode 清楚报告。
- [ ] 仅对实测迟醒抓 WPR / WPA / ETW trace，定位 DPC / ISR / 电源 / CPU 迁移后再进行单变量系统实验。每次记录是否降低 jitter 与实际 E2E。

**验收：** 扩展后端带来可重复收益且保持 RT 契约。该任务不阻塞 Shared P1 主线。

## 8. 验收与测量方案

### 8.1 测量方法

1. **软件年龄估计：** 捕获包首帧 QPC → 正在消费帧 → Render 预计呈现。携带匹配 generation / frame range 的 stamp；GetPosition 必须按其 GetFrequency 解释，不能默认 device position 单位就是 sample frames。[S5, S9]
2. **物理线缆探针：** 已知脉冲 / MLS 经 Render→线缆→Capture，在控制侧对齐分析。报告探针到底包含哪些 queue / graph；只测 Render→Capture 不等价于整条监听 route。
3. **真正 input→output 监听：** 外部发生器送入受测输入，外部同步双通道记录参考输入与监听输出，以相关峰差计算。共用测量时钟抵消测量通道共同延迟；分别校验设备路径与接线。
4. **软件 Process Loopback：** 单独报告 source-specific 延迟和限制，不能替代 ADC / DAC / 硬件测量。

每次报告采样率、格式、模式、actual period、两端目标 / 实际占用、graph / algorithm delay、负载、测试时长、有效样本数、排除的启动 / 停机区间。p99 样本不足时不生成看似精确的分位数。

### 8.2 候选档位的准入矩阵

| 维度 | 测试 |
| --- | --- |
| 流 / 时钟 | 同设备输入输出；两个独立设备；48 kHz mono / stereo 与 44.1→48 kHz；Process input 独立验收 |
| 调度 | 两种事件先后、相位 sweep、不规则包大小、单包及连续迟包、UI 拖动、CPU / GPU / 磁盘压力 |
| 漂移 | 0、±100、±1000 ppm；检查 ppm 收敛、fill 下界和上界，不只检查最后一个值 |
| 状态转换 | 全容量 / 小目标 prime、反复 Start / Stop、控制高压、捕获 discontinuity、停止后恢复、设备失效 |
| Graph | 简单链；Compressor；fan-out / Bus；meter on / off；事件 segment 压力 |
| 时长 | 每候选空闲 / 压力各 10 分钟 × 3 次；每时钟 / rate 关键组合 7200 秒模拟；最终设备长时稳态 ≥ 2 小时 |

验收条件：

- 稳态 bridge underrun、overflow、非预期 intentional skip 为 0；所有 native discontinuity 与时间戳异常如实报告。预期故障注入另列，不混进稳定通过结果。
- 输出没有可测 gap / 错样或相关失败；`empty_padding_wakes = 0` 只能作辅助证据。
- callback p99 建议 ≤ actual period 的 50%，max ≤ 80%，作为首轮保守工程准入；这不是 Windows API 的固定阈值。另按 padding 和实际 packet deadline 验证没有迟交。
- 物理 E2E p50 / p95 / p99 / max 均报告；Balanced 与同条件 baseline 对比，Low / 专业档位按第 6 节条件目标验收。
- RT 无 allocation / deallocation、无阻塞，诊断队列满不会导致音频丢帧；退役与最终析构发生在 control / cleanup。
- 对低目标配置重新验证 capture 先停 / render 先停的共同 Stop，避免把生命周期尾部欠载误归因为漂移。

### 8.3 实现阶段通用自动检查

```powershell
cargo test --locked -p moiren-engine -p moiren-windows-audio
cargo test --locked -p moiren-app
cargo check --locked --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

提交只包含该切片相关路径。CI 按现有 Ubuntu / Windows 与 nightly Miri 模式执行，硬件报告单独保存，不以 portable test 代替实机验收。

## 9. 不宜优先采取的做法

- 只改 `max_block_frames`，Shared period 与排队 target 不变：不能直接消除 2048 帧输入库存或 1056 帧 native 队列。
- 只缩小 capacity：可能变成丢帧，实际目标和年龄策略却未改变。
- 将 target 一刀切为 128 / 256：本轮实验已证实多个失败场景。
- 无证据关闭跨钟适配、仅因输入输出都写着 48 kHz 就视为共钟：长期会积压或欠载。
- 为追赶 backlog 大幅提高 SRC correction：可能造成音高 / 时间扭曲，且 ±2000 ppm 下仅约 96 帧/s 的净修正量不能快速消化突发库存。
- 普遍使用 busy spin、Realtime priority、全局 timer resolution、注册表调优或固定 CPU affinity：先测调度归因，避免用系统副作用替代 buffer 策略。
- 把 Compressor attack / release、meter 的 1536 帧窗口或 UI 刷新周期记成音频路径固定 delay：它们分别影响控制响应 / 显示 / 计算，当前不会把样本排队到窗口结束再输出。
- 把 bus PDC 当作降时延功能：PDC 是对齐，会给较快路径增加延迟，应服务于明确路由需求。
- 把 GetStreamLatency = 0、短时主观“还行”或无 RT 分配当作低端到端时延证明。

## 10. 官方接口依据

2026-10-13 核查 Microsoft 官方文档；本文的设备表现仍以具体机器测量为准。

- **S1:** [Low Latency Audio](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio)：Windows 的低周期机制、APO / driver / hardware 分项与硬件依赖。
- **S2:** [GetSharedModeEnginePeriod](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-getsharedmodeengineperiod)：period 与格式 / ClientProperties 相关；fundamental 整数倍，受 min–max 约束。
- **S3:** [InitializeSharedAudioStream](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-initializesharedaudiostream)：新 Shared 初始化、支持的 flags 与周期 / 格式占用错误。
- **S4:** [GetCurrentPadding](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getcurrentpadding)：Shared render 排队帧数与最大可写空间。
- **S5:** [IAudioCaptureClient::GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)：包首帧 device position / QPC、100 ns 单位、packet lease 与释放周期要求。
- **S6:** [Multimedia Class Scheduler Service](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service)：任务注册和多媒体线程调度。
- **S7:** [AUDCLNT_STREAMOPTIONS](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/ne-audioclient-audclnt_streamoptions)：RAW 仍保留 endpoint-specific always-on、driver 与 hardware 处理。
- **S8:** [Exclusive-Mode Streams](https://learn.microsoft.com/en-us/windows/win32/coreaudio/exclusive-mode-streams)：独占流、设备权限、event buffering 与格式支持。
- **S9:** [IAudioClock::GetPosition](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock-getposition)：位置、频率与 QPC 时间的对应关系。
- **S10:** [GetStreamLatency](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getstreamlatency)：初始化后不变的流最大 latency API 值，不能替代本应用全路径测量。
