# T4 第二切片：固定输出帧控制与低水位保护

日期：2026-10-13。基线：`fbfbb60`。本切片消除 Engine segment 对 FIFO servo 的采样偏置，增加有界低水位保护与独立计数，并建立按实际 native frame 周期运行的确定性模型。包时间 feed-forward、音频年龄恢复和完整硬件准入仍待完成。

## 根因与控制修正

旧控制器按每次 `read_to` 的入口库存与 demand 更新。即使 dt 按帧数换算，采样位置仍受分段影响：相同零漂移输入、480 帧 demand 和 1024 目标，在 30 秒模型中的末尾库存为 543 帧；改为 256/224 segment 后为 669 帧，相差 126 帧，回归首先失败。

现在每 32 个成功输出帧采样一次 FIFO，持有的 step 与剩余帧数跨 segment 保存。积分与滤波使用固定帧量；prime/recovery/真实 reset 重置控制相位。480、256、64、1 帧切分的输出推进与控制轨迹因此一致。公开的 `fill_frames` / min / max 仍在 segment 入口取样，它们本身可随切分变化，不能拿这些 scalar 代替实际样本推进量作该回归。

另一个 RED 回归发现：发生过积压的滤波器在实际库存低于 low water 时仍输出正 correction（约 +1.755 ppm），会加快消耗低库存。达到 prepared low water 后，新策略临时使用配置中的负 correction limit，冻结 integral；补齐输入后在下一个固定 quantum 解除。它始终受既有 0..2000 ppm 限制，不扩大拉伸量、不产生替代样本，真实 starvation 仍返回短读、discontinuity、xrun 与完整 underrun 计数。

`low_water_guard_frames` 只统计实际成功输出的保护帧，跨 segment 统计一致；恢复、欠载 reset 会清理保护状态。相关 RT 回归确认 ingress、controller/guard、输出与 snapshot 没有 heap allocation/deallocation。启动 trim 测试现在与同样本、同库存、无积压的 source 对照；首帧 correction 为 0，后续自然库存变化产生的 correction 与对照完全相同。

## 预算与模型边界

原审计中的小目标不是新的可用档位：480 周期、Render target 624、jitter 96 要求输入至少 1202 帧，拒绝 512；128 周期、Render target 272、jitter 128 要求至少 530，拒绝 256。拒绝回归明确要求 `InsufficientInputBudget`，不靠容许短读或放大 SRC 拉伸通过。

新模型将 native packet 大小固定，按 native rate × `(1 + drift)` 计算实际到达间隔，独立调度 Render。它比旧模型的固定交付时刻、可变 packet frame 数提供了不同的时钟相位覆盖；原始 22 组结果仍属于原实现，未改写为新模型结果。

本矩阵在激活前实际调用 `prepare_latency_budget`，然后使用解析出的 `PreparedInputQueue` 构造 source：

| 维度 | 值 |
|---|---|
| 输入率 / native packet period | 44.1 kHz / 309 帧；48 kHz / 336 帧 |
| 输出率 / Render period | 48 kHz / 336 帧 |
| Render target / wake margin / callback budget | 480 / 96 / 48 帧 |
| 输入 target / capacity | 1024 / 8192 帧 |
| 输入 modeled minimum / low water | 44.1 kHz：841 / 443；48 kHz：914 / 482 |
| 漂移 | -1000、0、+1000 ppm |
| 相位 | 0、84、168、252 个输出帧 |
| 同时事件顺序 | capture-first 与 render-first |
| 迟包 | 每第七个 packet 延后 96 个输出帧；stamp 仍对应原生 packet 首帧 |
| 最大 Engine segment | 256 帧 |

44.1 kHz / 309 是模型输入，不声称本机设备支持该 Shared period。模型将 callback 视为即时执行，未模拟真实 callback CPU 成本、DPC、Render 调度抖动或驱动行为。

## 120 秒矩阵与回归

48 个组合各执行 120 秒，全部满足：稳定阶段 short callbacks=0、underrun=0、producer dropped=0、backlog recovery=0。保护计数合计 43200 个输出帧，完整保留在 CSV 中，没有把保护或欠载归零隐藏。

瞬时 `correction_ppm` 包含 FIFO 锯齿与控制响应，不能要求每个末尾采样点都等于硬件 drift。本回归另用已知 ramp 的原生样本位置变化除以后半程成功输出帧数，计算后 60 秒实际应用的平均 correction；最大绝对误差约 71.42 ppm，全部小于 300 ppm。这个量是模型内样本推进率，不是物理端到端或音频年龄测量。

结果：`2026-10-13-clock-control/results-120.csv`。其中浮点字段使用 Rust round-trip display，保存与验证日志一致的值。`verify.ps1` 检查全部列、48 个唯一坐标、周期/目标/时长、音频计数、guard 范围、finite 值、correction 限制及平均 rate 误差。

定向覆盖包括 6 项 `clock_control` 测试与 27 项 `clock_bridge` 测试：分段一致性、两项预算拒绝、48 个 admitted 场景、低水位 stale correction、补齐后的固定 quantum 解除、真实 starvation 报告，以及无分配、SRC、generation 和有界恢复。既有默认 2048 的四组 120 秒漂移/迟包回归也继续通过。

## 提交树验证与复现

源代码暂存树：`7603754d8ee6901b083658f12db56b968d09880b`。日志：`.tmp/audio-latency-audit/staged-clock-control-20261013-154645/logs/`。该树的全部 workspace check、test、fmt、严格 Clippy 已通过：517 项测试通过、0 项失败、4 项忽略，包含 doc-tests。之后仅补充本记录、精度完整的 CSV、校验脚本和审计进度链接；已验证的 crate 文件没有改变。已有 UI 与 Cargo 草稿未纳入检查或提交。

在该导出树执行：

```powershell
cargo test --locked -p moiren-windows-audio --test clock_control -- --nocapture
```

从测试日志中保留 `clock_case,` 行并移除该前缀，使用 CSV 的同一列顺序保存。核对归档：

```powershell
./docs/superpowers/plans/2026-10-13-clock-control/verify.ps1
```

长模拟通过 `MOIREN_CONTROL_SIM_SECONDS=7200` 启用，仍执行完整 48 个组合与同样的严格音频断言。

## 7200 秒完整矩阵

控制修正提交 `6db4ad58542b46f65de982e95a24a42dc8093974` 后，使用与该提交 crate 内容完全相同的上述导出树，在 release profile 执行完整 48 个组合，每组合 7200 秒。命令退出码为 0，测试运行时间为 156.22 秒；它是加速离线模拟，不是 96 小时的实机监听。

```powershell
$env:MOIREN_CONTROL_SIM_SECONDS = '7200'
cargo test --locked --release -p moiren-windows-audio --test clock_control clock_control_prepared_low_inventory_handles_rate_phase_order_and_lateness -- --nocapture
```

48 组 short callbacks、underrun、producer dropped、backlog recovery 均为 0。后 3600 秒实际应用平均 correction 的最大误差约 0.979 ppm；所有 case 的最后 correction 仍在 ±2000 ppm 内。输入位置由 f32 ramp 读取，在长测末尾的单点库存估计可有数十帧量化误差，不能作为精确音频年龄；长窗口的平均 rate 分析与严格音频计数分别保留。

长测 guard 合计仍为 43200 帧，每个坐标的累计值均与 120 秒一致。同一 release 二进制随后重跑完整 120 秒矩阵，48 行的每一个 CSV 字段均与归档的 debug 记录完全一致；在本确定性模型的相同前缀之后，没有新增保护帧。

长测结果：`2026-10-13-clock-control/results-7200.csv`。日志分别保存于 `.tmp/audio-latency-audit/clock-control-7200-6db4ad5.log` 与 `clock-control-release-120-6db4ad5.log`。两个 CSV 都通过独立校验；长测复核命令为：

```powershell
./docs/superpowers/plans/2026-10-13-clock-control/verify.ps1 -Csv ./docs/superpowers/plans/2026-10-13-clock-control/results-7200.csv -Seconds 7200
```

## 当前边界

Probe envelope schema 升为 4，应用 monitor schema 升为 5；Capture/Render schema 保持 6/5。本切片未进行新的实机采集或播放，T3 的 FreeDSP 数据不能作为这次控制器修改的硬件结果。

T4 整体仍未完成：连续迟包/Render 迟醒与积压后恢复的组合模型、可信 stamp/rate feed-forward、最老实际待用帧的年龄、持续故障的控制侧降档，以及 20–30 分钟硬件准入和物理测量仍需继续。本次 7200 秒矩阵覆盖上表给定的相位、周期与单次周期性迟包，不代表所有故障矩阵已经通过。
