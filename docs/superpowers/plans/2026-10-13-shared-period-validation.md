# Shared 周期协商实现与实机验证

日期：2026-10-13。对应 [时延审计 T2](2026-10-13-audio-latency-audit.md#t2协商较小-shared-周期p1)。本轮实现设备周期协商，并接入 CLI、应用服务及物理输入监听路径。

## 实现与边界

- `SharedPeriodPolicy` 默认期望 5 ms；`None` 请求设备默认周期。按本流采样率换算帧，向 fundamental 的合法整数倍取整，限制在设备 min/max 的合法倍数范围内。
- 每次 owner 准备时先设置 ClientProperties，再查询本格式能力；Client3 初始化只传 EVENTCALLBACK 与本流 Session GUID。没有沿用旧初始化的 NOPERSIST 或自动转换 flags。
- 两端报告包含 desired、requested、actual、capabilities、初始化 API、fallback 原因，以及 Client3 失败阶段与 HRESULT。Render 旧的 `engine_period_frames` 复用同一次 actual 查询。Capture schema 为 5，Render schema 为 4。
- 仅缺少 Client3 接口或明确不支持 capability 查询时允许旧 Shared 初始化。设备失效、非法能力/策略及已尝试的 Client3 初始化失败直接报错。没有在同一 client 上重复 Initialize，也没有自动重试周期/格式占用冲突；将来增加此策略时必须重新 Activate。
- Process Loopback Capture 保留独立初始化路径；其 Shared 协商字段为空，不伪造物理端点能力。

## 原报告缺字段的根因

早期 `.tmp/audio-latency-audit/t2-summary.json` 完全没有 `shared_period` 字段。其 release 可执行文件编译时间为 10:44:40，早于 T2 源码修改。重新 `cargo build --locked --release -p moiren-windows-audio --example latency_probe` 后，字段和 Client3 结果均正常。没有发现 owner finish 覆盖报告的问题。

以下数据均来自重建后的 probe；每次解析额外确认 Capture schema 5、Render schema 4，避免把旧二进制结果当作当前实现。原失效的报告未纳入验收数据。

## 同设备对照

输入为 `麦克风 (5- FreeDSP)`（48 kHz、mono），输出为 `耳机 (5- FreeDSP)`（48 kHz、stereo）。Graph 为 Input → Send → Gain → Send → Pan → Send → Output，Engine block 为 256 帧，增益为 0。输入库存策略仍为原 2048 帧目标，输出仍使用 capacity 预填/补满策略。

设备两端能力均为 default 480、fundamental 1、min 336、max 480 帧。所有七组均为 Client3 成功，无 fallback，actual 等于合法 requested。期间没有刻意施加额外负载；这是短时功能对照，不代表满负载或长时稳定性验收。

| 期望周期 / 观测模式 | 时长 | 两端 actual 帧 | 两端 capacity 帧 | Capture callback p99 / max (µs) | Render callback p99 / max (µs) |
|---|---:|---:|---:|---:|---:|
| default / Summary | 5 s | 480 | 1056 | 38.3 / 62.0 | 102.3 / 107.6 |
| 1 ms / Summary | 5 s | 336 | 738 | 22.3 / 47.8 | 102.3 / 138.4 |
| 8 ms / Summary | 5 s | 384 | 844 | 31.9 / 96.5 | 102.3 / 120.1 |
| 5 ms / Disabled | 5 s | 336 | 738 | 未观测 | 未观测 |
| 5 ms / Summary | 5 s | 336 | 738 | 25.5 / 67.3 | 102.3 / 166.5 |
| 5 ms / Trace | 5 s | 336 | 738 | 25.5 / 44.5 | 102.3 / 183.7 |
| 5 ms / Summary | 30 s | 336 | 738 | 31.9 / 338.8 | 102.3 / 227.9 |

所有 p99 都是直方图上界，max 为观测最大值。Disabled 没有 QPC callback 数据，不能视为零成本实机测量。七组均为 `bridge.underrun_frames = 0`、`bridge.dropped_frames = 0`、`render.stats.empty_padding_wakes = 0`，无 bridge transfer shortfall 或 owner failure。默认周期组两端各记录 1 次 timeout wake，其余组为 0；owner 的 wait timeout 使用剩余运行时长，这与有限时长末尾唤醒相符，报告没有逐次 timeout 时间，不能断言其精确归因。初始 Capture discontinuity 保留为 1，不视为 steady glitch。两 owner 的 Completed/Stopped 差异来自共同 Stop 与有限时长的先后顺序。

Trace 两端均保留前 256 条记录，分别丢弃 Capture 457、Render 459 条后续诊断；音频统计继续累计，未随 trace 溢出丢帧。30 秒组 Render 4287 次 audio wake，观察到 p99 小于实际 7 ms 周期的 50%、max 小于 80%，只证明本次负载下满足该工程准入。

周期由 10 ms 降至 7 ms，capacity 从 1056 降至 738 帧；318 帧相当于 48 kHz 下 6.625 ms 的容量尺度变化。输入目标、启动库存与输出排队还没有联合降低。所有 measurement method 仍为 `unknown`，未测物理端到端时延，不能把周期、capacity 或输入库存尺度相加当作实测时延。

## 数据与复现

[results.csv](2026-10-13-shared-period/results.csv) 保存全部七组标量，目录中各同名 JSON 保留原始两端报告、桥统计和 bounded trace。

```powershell
cargo build --locked --release -p moiren-windows-audio --example latency_probe
cargo run --locked --release -p moiren-windows-audio --example latency_probe -- --list
cargo run --locked --release -p moiren-windows-audio --example latency_probe -- --input '<capture-id>' --output '<render-id>' --seconds 5 --mode summary --gain 0 --period-ms default
cargo run --locked --release -p moiren-windows-audio --example latency_probe -- --input '<capture-id>' --output '<render-id>' --seconds 5 --mode summary --gain 0 --period-ms 1
cargo run --locked --release -p moiren-windows-audio --example latency_probe -- --input '<capture-id>' --output '<render-id>' --seconds 5 --mode summary --gain 0 --period-ms 8
```

固定期望 5 ms 时，分别使用 `--mode disabled`、`summary`、`trace`；较长组使用 `--seconds 30 --mode summary`。每次明确指定 endpoint ID，默认增益为 0。

## 自动验证与后续验收

纯策略回归覆盖原生采样率、默认值、非整数周期、非对齐 bounds、u32 边界、无效参数和溢出。Fake Client3 回归覆盖查询先后、每次重查能力、缺接口/不支持查询的回退分类、设备失效不降级，以及初始化失败保留 request 且不重试。两端 owner 生命周期回归确认 request/actual 不会在 Stop 或 finish 时丢失；portable 测试检查 JSON 字段及 options 的启动前校验。

本次提交快照检查：workspace all-targets check、workspace tests、fmt check、strict Clippy。RT 分配/析构、trace 满载、Clock Bridge 漂移/抖动、Render packet lease 和共同 Stop 回归保持通过。硬件观测独立于自动测试。

仍需跨设备、其他应用占用/共存、20–30 分钟漂移与突发负载验收。下一阶段按实际周期设计输入 target、capacity 和 render target/prefill，重新跑 phase/jitter 模拟，不能把输入目标直接改成 128/256 帧。

## 官方接口依据

- [GetSharedModeEnginePeriod](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-getsharedmodeengineperiod)：能力与 ClientProperties/format 有关，周期必须满足 fundamental 与 min/max。
- [InitializeSharedAudioStream](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-initializesharedaudiostream)：Client3 支持的 stream flag 与初始化失败原因。
