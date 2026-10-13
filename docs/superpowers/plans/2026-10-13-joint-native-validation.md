# T3：联合 1024/480 预算的 FreeDSP 实机验证

日期：2026-10-13，Asia/Shanghai。实现来源为 `d2849852eb478fff78c5cb706a1fe83271b1621c`；release probe 从已验证源代码树 `150fabf984918938be9c6608480ab1ddaf514844` 构建。原始资料、二进制 SHA-256、设备枚举、CSV 与复现脚本在 `2026-10-13-joint-native/`。

## 结论与边界

物理 Capture/Render 的两阶段准备路径已在 FreeDSP 上执行，并由复现脚本再次执行完整矩阵。两轮共 16 组运行，Custom 输入 prime/servo=1024、Render target/prefill=480 的十组运行均无 Bridge underrun、音频丢帧、Render transfer shortfall 或 empty-padding wake；两个 owner 均正常 Stop。Compatible 的六组对照同样满足这些音频计数要求。

这是单设备、48 kHz、最长单次 120 秒的候选配置验证。`measurement_method` 始终为 `unknown`，gain=0。本轮没有物理回环、音频年龄或呈现时间测量，不能据此宣布端到端时延降低、听感无爆音或长期准入完成。

## 实际准备信息

本轮重新枚举后，FreeDSP Capture 为 48 kHz mono，Render 为 48 kHz stereo。Capture 适配后向 Engine 提供 48 kHz stereo；两端 endpoint ID 与 format 支持结果保存在 `environment.json`。

- Device default：两端 actual period=480，Render capacity=1056，Compatible 输入 prime/servo=2048，Render target=1056。
- 请求 period=5 ms：两端 actual period=336，Render capacity=738。
- 同周期 Compatible：输入 capacity=8192、prime/servo=2048，Render target/prefill=738。
- 联合 Custom：输入 capacity=8192、prime/servo=1024、low-water=482、high-water=7856、modeled minimum=914；Render capacity=738、target/prefill=480、deadline budget=480。

Custom 的声明预算为输入 jitter=96 帧、Render wake margin=96 帧、callback budget=48 帧。它们是准备时的模型输入，尚未通过长期 WCET 与调度尾部标定。

## 首轮运行

| Case | 时长 | 两端 period | 输入 prime | 输出 target | 观测 padding | 最小 segment 库存 |
|---|---:|---:|---:|---:|---:|---:|
| compatible-default-summary | 5 s | 480 | 2048 | 1056 | 576 | 1793 |
| compatible-small-summary | 5 s | 336 | 2048 | 738 | 402 | 1793 |
| joint-disabled | 5 s | 336 | 1024 | 480 | 未记录 | 769 |
| joint-summary | 5 s | 336 | 1024 | 480 | 144 | 769 |
| joint-trace | 5 s | 336 | 1024 | 480 | 144 | 440 |
| compatible-small-summary-120s | 120 s | 336 | 2048 | 738 | 402 | 1773 |
| joint-summary-120s | 120 s | 336 | 1024 | 480 | 144 | 769 |
| joint-load-summary-120s | 120 s | 336 | 1024 | 480 | 144 | 769 |

Summary/Trace 中的 padding 最小值与最大值相同，steady demand=336；Disabled 不记录这些 callback 直方图。所有原始报告使用 envelope schema=2、Capture schema=6、Render schema=5，并实际携带 `input_queue`，不同于此前输入仍为 2048 的 Render-only 报告。

120 秒同周期 Compatible/Custom 的 callback histogram p99 上界均为 89.5 µs，观测最大值分别为 218.6/253.2 µs；两个 CPU worker 负载下的 Custom 最大值为 199.8 µs。相应 wake interval 最大值为 9.9343/9.9336/9.9987 ms。这些观测最大值不能作为 WCET 保证。

每轮负载由两个独立 CPU worker 构成，每个执行约 120 秒的整数运算；系统有 16 个逻辑处理器。工作进程正常退出，时间与 checksum 在各轮 `cpu-load-results.json`。这是有限并发 CPU 负载，未覆盖整机饱和、DPC/ISR 尾部、磁盘或 GPU 压力。

Trace 的 Capture/Render bounded trace 各丢弃 458 条诊断记录，音频丢帧均为 0。`joint-summary` 短测两端各有一次 timeout wake；其 completed 状态、近 5 秒 elapsed 与有限时长结束等待一致。当前 schema 未记录每次 timeout 的发生时间，因此不逐次归因，也不把 timeout 计数直接当作音频欠载。其他七组 timeout 计数为 0。

各组 native discontinuity 总计为 1、Bridge resets 为 0。min-fill 按 Engine segment 入口统计；Trace 的 440 帧不能直接与以完整 Render target/demand 推导的 482 帧 low-water 当作同一剩余需求比较。T4 仍需补齐剩余 demand、相位与库存保护的联合验证。

相同 actual period 下，输入 target 从 2048 到 1024 的名义库存尺度差为 21.333 ms；Render target 从 738 到 480 的差为 5.375 ms，steady padding 从 402 到 144。这些尺度差不相加当作实测总时延改善。

## 完整脚本回放

`run.ps1` 已实际执行相同八组运行与两次拒绝，随后由 `verify.ps1` 验证。第二轮 raw JSON、CSV、负载与拒绝记录完整归档在 `2026-10-13-joint-native/replay/`；二进制 SHA-256、source commit 与 source tree 与首轮一致。

第二轮八组的音频欠载、丢帧、transfer shortfall 与 empty-padding wake 同样均为 0；Custom padding 仍为 144。Trace 最小 segment 库存为 435，两端各丢弃 458 条诊断记录。120 秒 Custom Summary 的观测最大 callback 为 361.8 µs，有限 CPU 负载下为 210.7 µs。

回放中 Compatible 120 秒 Summary 两端各有一次 timeout，Custom 120 秒 Summary 的 Render 有一次 timeout，其余六组为 0；这些计数逐项保留在 CSV，未用重新采集覆盖首轮数据。各轮都只有有限时长近结束的 aggregate 状态证据，不提供逐次 timeout 的时间归因。

## 准备阶段拒绝

- 5 ms 请求、Render target=479：actual period=336，deadline=336+96+48=480，报 `render target cannot cover period, wake margin and callback budget`。
- Device default、Render target=624：actual period=480，Render deadline 恰为 624，但输入 minimum=624+2+480+96=1202 超过固定 Custom target=1024，报 `input target cannot cover render demand, capture gap, jitter and lookahead`。

两次 probe 均在 1 秒以内返回失败，没有进入五秒 streaming；exit code、elapsed 与错误文本分别在 `rejections.json` 和对应 `.txt`。owner 取消、join 和 Engine 回收的无硬件回归见 `2026-10-13-joint-preparation-validation.md`。

## 验证与复现

两轮各八组 raw JSON 的每个 CSV 字段、native format/period/capacity、预算 metadata、激活/Stop、音频失败计数、两个 CPU worker 和两次准备拒绝均已通过 `verify.ps1`。CSV 使用 invariant culture 保留浮点 round-trip 精度。

```powershell
pwsh -NoProfile -File docs/superpowers/plans/2026-10-13-joint-native/verify.ps1
pwsh -NoProfile -File docs/superpowers/plans/2026-10-13-joint-native/verify.ps1 `
  -EvidenceDirectory docs/superpowers/plans/2026-10-13-joint-native/replay
```

重新采集时，先从记录的源码版本编译 release `latency_probe`，然后将结果写入新的空目录。以下使用本次已枚举的 endpoint，设备改变后应重新枚举并选择：

```powershell
pwsh -NoProfile -File docs/superpowers/plans/2026-10-13-joint-native/run.ps1 `
  -ProbePath .tmp/audio-latency-audit/target-joint-preparation/release/examples/latency_probe.exe `
  -OutputDirectory .tmp/audio-latency-audit/joint-native-new-run `
  -CaptureEndpointId '{0.0.1.00000000}.{0e3bce79-16b7-4594-8ef2-5db33dfb70b6}' `
  -RenderEndpointId '{0.0.0.00000000}.{dedcefcb-f139-4c50-8950-6925f8c60444}' `
  -SourceCommit d2849852eb478fff78c5cb706a1fe83271b1621c `
  -SourceTree 150fabf984918938be9c6608480ab1ddaf514844
```

runner 按相同配置执行八组采集和两次拒绝，并重新生成、核对新目录中的 CSV。对已有报告执行 `verify.ps1` 只读验证；`-UpdateCsv` 才会重建 CSV。重新采集的数值可能随调度变化，不要求它与保存的 CSV 数值相同。

## 后续准入

本切片补齐两阶段原生路径的候选实机证据。仍需 T4 的低库存 servo、generation-safe backlog 恢复与 RT 分配回归，以及 44.1 kHz、事件相位/顺序、连续迟包、±1000 ppm 长模拟和 20–30 分钟实机漂移/负载矩阵。low/high water 目前只是已校验准备元数据，尚不触发运行保护或积压恢复。
