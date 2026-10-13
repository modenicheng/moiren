# T3 第一切片：准备预算与 Render target 验证

日期：2026-10-13。此记录覆盖纯 Rust 联合预算解析与 Render target 的运行集成；两阶段 owner 准备、输入预算安装和低库存 servo 尚未完成，不代表整个 T3 已验收。

## 实现与准入

`RenderQueuePolicy::FullCapacity` 保留原行为。`Target` 在 native client 初始化、实际 period 与 capacity 查询后解析，要求目标至少覆盖实际周期、声明的迟醒余量与正值 callback 预算之和。超出 capacity、缺少实际 period、算术溢出或预算不足均报错，不通过 clamp 把失败伪装为成功。

启动 prefill 和之后的 refill 使用同一个目标；staging 仍按 native capacity 分配。`demand = max(target - padding, 0)`，零 demand 不推进 Engine timeline。原有 writable 统计继续记录 `capacity - padding`，实际提交需求由 latency histogram 单独记录。Render schema 升至 5，Capture schema 保持 5。

`prepare_latency_budget` 的 Custom 模式把最大输出需求向上取整转换到原生输入帧，再加入两帧插值前瞻、一个 Capture 实际周期和声明的输入 jitter。输入 capacity、prime、servo reference、low/high water 分开返回。在两端均为 48 kHz、336 帧周期、Render target 480、输入 jitter 96 时，模型最低输入目标为 914 帧：1024 通过，128/256/512/913 不通过。Capture 周期改为 480 后，1024 不通过同一预算。Compatible 保留 8192/2048 输入配置与 full-capacity Render，不声明模型 deadline 准入。

本切片没有把上述输入预算安装到 Clock Bridge。四组硬件实验与正式 app 均保持原输入配置，正式 app 也仍使用 `FullCapacity`。

## 本机观测

设备为 FreeDSP，明确指定 Capture 与 Render endpoint，两端均请求 5 ms，实际 period 均为 336 帧，采样率均为 48 kHz。Render native capacity 为 738 帧。所有实验 `gain=0`，未进行有声播放或物理回环。

Custom 实验声明 wake margin 96 帧（2 ms）、callback budget 48 帧（1 ms），Render 准入下界为 336 + 96 + 48 = 480 帧。以下 p99 是有界直方图的桶上界，不是逐样本精确分位数。

| 组别 | 时长 | target / prefill | callback p99 上界 | callback max | 最大 wake 间隔 | 回填前 padding |
|---|---:|---:|---:|---:|---:|---:|
| capacity-summary | 5 s | 738 / 738 | 89.5 μs | 108.5 μs | 7.4429 ms | 402 |
| target-480-summary | 5 s | 480 / 480 | 102.3 μs | 153.7 μs | 7.5084 ms | 144 |
| target-480-trace | 5 s | 480 / 480 | 127.9 μs | 152.7 μs | 7.3907 ms | 144 |
| target-480-summary-30s | 30 s | 480 / 480 | 102.3 μs | 191.6 μs | 7.4793 ms | 144 |

四组最大需求均为 336 帧；bridge underrun、bridge dropped frames、Render transfer shortfall、empty-padding wake、zero-demand wake 和两端 timeout wake 均为 0。Capture 初始 discontinuity 为 1，按原始报告保留。两端 owner 均成功 Stop，报告中 failure 为空。

Trace 保留前 256 条诊断，Capture 后续丢弃 457 条、Render 后续丢弃 459 条；音频计数继续累计，不能把诊断丢弃理解为音频丢帧。Summary 的 trace dropped 为 0。

本轮 Custom 最大 callback 为 0.1916 ms，最大 wake 间隔为 7.5084 ms，分别落在声明的 1 ms callback 与 7 + 2 ms wake 间隔预算内。此结论只适用于本轮低负载短时实验，不建立长时间或突发负载保证。

target/prefill 比 full capacity 减少 258 帧，在 48 kHz 下对应 5.375 ms 的排队尺度变化。`measurement_method` 仍为 `unknown`；没有物理 E2E 测量，不能把该数字表述为实测全路径时延收益。

另一次 target=479 的准备尝试以非零退出码拒绝，错误为 `render target cannot cover period, wake margin and callback budget`。拒绝发生在共同 activation 之前；两端清理路径由生命周期测试另外覆盖。

## 原始证据与复现

[results.csv](2026-10-13-render-target/results.csv) 保存四组标量；同目录同名 JSON 保留完整两端报告、桥统计和 bounded trace。[verify.ps1](2026-10-13-render-target/verify.ps1) 逐字段核对 CSV 与原始报告，也验证 schema、prefill、设备周期、错误计数与预期拒绝文字。

```powershell
cargo build --locked --release -p moiren-windows-audio --example latency_probe
& ./target/release/examples/latency_probe.exe --input '{0.0.1.00000000}.{0e3bce79-16b7-4594-8ef2-5db33dfb70b6}' --output '{0.0.0.00000000}.{dedcefcb-f139-4c50-8950-6925f8c60444}' --seconds 5 --gain 0 --mode summary --period-ms 5 --render-target-frames capacity
& ./target/release/examples/latency_probe.exe --input '{0.0.1.00000000}.{0e3bce79-16b7-4594-8ef2-5db33dfb70b6}' --output '{0.0.0.00000000}.{dedcefcb-f139-4c50-8950-6925f8c60444}' --seconds 30 --gain 0 --mode summary --period-ms 5 --render-target-frames 480 --wake-margin-frames 96 --callback-budget-frames 48
& ./docs/superpowers/plans/2026-10-13-render-target/verify.ps1
```

其余 Custom 组使用 `--seconds 5`，分别设置 `--mode summary` 和 `--mode trace`。拒绝组使用相同 Custom 参数并把 target 改为 479，原始错误保存在 [rejected-479.txt](2026-10-13-render-target/rejected-479.txt)。设备能力变化时不能照搬这些帧数，应重新查询并解析预算。

## 自动验证与剩余工作

预算回归覆盖 capacity 保持不变、目标回填、不规则 padding、不同采样率与周期、Compatible、缺少 actual、无效参数、预算不足和溢出。Render 回归覆盖较小 prefill、不规则需求、零需求 timeline 与 owner 准备失败/共同取消/Stop。

该切片按精确暂存树验证 workspace all-targets check、workspace tests、fmt check 与 strict Clippy，排除工作区中独立的 UI 草稿和 Cargo.lock 改动。硬件记录独立于 portable 测试。

剩余 T3：让两端 native owner 先返回纯 Rust timing，再由控制侧创建桥和图并安装联合预算，明确取消与失败清理；T4 后续处理低库存 servo 与积压恢复。还需跨设备、迟醒/不规则需求、20–30 分钟漂移和突发负载验收。
