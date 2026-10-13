# T4 第三切片：连续迟包、Render 迟醒与停顿恢复模型

日期：2026-10-13。基线：`66a4a9a`。本切片只扩展回归与实验记录，音频运行实现不变。与第二切片的固定 Render demand 模型相比，本模型使用真实 `PreparedRenderQueue::demand(padding)` 联合调度输入与输出队列；覆盖连续迟包、顺序批量交付、连续迟醒及队列耗尽后的恢复。

## 模型与准入

测试代码：[clock_faults.rs](../../../crates/moiren-windows-audio/tests/clock_faults.rs)。每个坐标先调用 `prepare_latency_budget`，使用成功准备的输入与输出预算。输入 native packet 大小固定，实际间隔随 native rate 与 drift 改变；时间戳仍对应 packet 的首帧，交付变晚不修改 stamp。

Render queue 从 480 帧启动静音开始。模型按输出时钟经过的帧数消耗 padding，迟醒后根据剩余 padding 回填到 target；不足的 native queue 帧数累计为 `render_missing_frames`。这些值是模型结果，不是 WASAPI 实机报告。callback 视为瞬时完成；预算里的 48 帧 callback reserve 不代表已模拟真实 CPU、DPC 或驱动执行时间。

| 共同维度 | 值 |
|---|---|
| 输入率 / native capture period | 44.1 kHz / 309 帧；48 kHz / 336 帧 |
| 输出率 / Render period / capacity / target | 48 kHz / 336 / 738 / 480 帧 |
| input capacity / 最大 Engine segment | 8192 / 256 帧 |
| Render wake margin / callback budget | 96 / 48 帧 |
| drift | -1000、0、+1000 ppm |
| 输入相位 | 0、84、168、252 个输出帧 |
| 同时事件顺序 | capture-first 与 render-first |
| 连续迟交付 / 迟醒 | 每 17 个事件中连续 8 个事件使用延后量 |

44.1 kHz / 309 帧是模型假设，不声称设备支持此 Shared period。较晚 packet 阻挡后续交付时，按原生顺序在同一时刻批量交付，不越过旧包，也不把旧包时间写成批量到达时刻。

| 故障模式 | Capture 延后量（输出帧） | Render 延后量（输出帧） | input target | modeled minimum（44.1 / 48 kHz） |
|---|---:|---:|---:|---:|
| `burst` | 96 | 96 | 1024 | 841 / 914 |
| `capture_stall` | 1024 | 96 | 2048 | 1693 / 1842 |
| `render_pause` | 96 | 96，并在第 20 秒停顿 9600 帧 | 1024 | 841 / 914，9600 帧停顿超出准入假设 |

每模式包含 48 个坐标，共 144 个。额外拒绝回归要求：申报 Capture 延后 1024 输出帧时，两个输入率下的 1024 target 都返回 `InsufficientInputBudget`；2048 target 可以准备。不能把 `burst` 的 1024 预算用于宣称更大的 Capture stall 已获准入。

## 验收与故障可见性

`burst` 与 `capture_stall` 要求整段稳定运行无 short callback、source xrun、输入 underrun、producer dropped、Render native queue 缺帧或积压恢复。

`render_pause` 故意耗尽输出队列并让 Capture 填满 SPSC，要求故障如实可见：正数 native queue 缺帧、正数 producer dropped、恰好一次 backlog recovery，并保留 generation mismatch 引起的 xrun、short callback 与 reset。运行中丢弃量受初始队列快照及 reference 限制，不能追赶后续 producer 写入。

恢复观察窗从 Render 恢复时刻开始，截止于随后 10 个原生周期，即 3360 个输出帧（70 ms）。窗内最多 11 次模型回填；此窗外不得新增 short callback / source xrun。窗末与运行结束的 reset、输入 underrun、backlog recovery、producer dropped 累计数必须相等。CSV 同时记录全程故障总数与窗外故障数；测试没有把故障期间的累计总数归零，也没有改动运行代码来抑制报告。

70 ms 是本模型的验收上限，不能当成实测恢复时间或音频年龄界限。低于 high water 的陈旧音频、可信时间戳关联、age limit、feed-forward 与持续故障降档仍不在本切片实现范围内。

## 复现与归档

默认 120 秒：

```powershell
cargo test --locked --release -p moiren-windows-audio --test clock_faults -- --nocapture
./docs/superpowers/plans/2026-10-13-clock-faults/verify.ps1
```

结果保存为 `2026-10-13-clock-faults/results-120.csv`。独立 verifier 检查列名、144 个唯一坐标、三模式各 48 个坐标、输入 / 输出预算参数、非负计数、零稳定故障、停顿故障计数和恢复界限。CSV 按原始 `fault_case,` 日志逐列保存。

144 个组合各运行 120 个模拟秒，全部通过。前两个模式的 96 个组合全程所有音频故障计数为零；48 个 `render_pause` 组合的非零计数完整保留如下：

| 120 秒矩阵合计 | 结果 |
|---|---:|
| short callbacks / source xruns | 81 / 48 |
| 稳定区 short callbacks / source xruns | 0 / 0 |
| modeled Render native queue 缺帧 | 451584 帧（每次停顿 9408 帧） |
| 输入 underrun | 0 帧 |
| producer dropped | 104898 帧 |
| backlog recovery / intentional discard | 48 次 / 344160 帧 |
| reset | 96 次（每个停顿一次 backlog reset 与一次 generation reset） |
| 最晚 short callback 相对恢复时刻 | 1104 输出帧（模型中 23 ms） |

全部 144 个组合的 low-water guard 合计 165152 成功输出帧，其中包括持续迟包模式；该数没有混入 underrun 或被隐藏。恢复窗末与运行结束的错误 / 丢弃 / reset 计数相同，未出现后续循环 reset。

验证使用导出的暂存树，排除工作区已有的 UI / Cargo 草稿。源树：`8f3c6961e87a27528b3caf8f359d1c5ffd6691c2`，目录：`.tmp/audio-latency-audit/staged-clock-faults-20261013-final`。运行实现与基线一致，只增加 `clock_faults.rs` 测试。

此树通过 `cargo check --locked --workspace --all-targets`、`cargo test --locked --workspace -- --nocapture`（519 通过、0 失败、4 忽略，含 doc-tests）、`cargo fmt --all -- --check` 和严格 workspace / all-targets Clippy。日志为 `.tmp/audio-latency-audit/clock-faults-{check,workspace-tests,fmt,clippy}.log`。原有分配、并发、图与 DSP 回归也在此次 workspace 测试中运行。

## 两小时完整矩阵复核

同一已验证源树的全部 144 个组合又各运行 7200 个模拟秒，全部通过。与 120 秒 CSV 逐行比较，除 `seconds` 外所有字段完全一致，包括 low-water guard 计数；没有新增短读、generation xrun、输入 underrun、producer dropped、backlog recovery 或 reset。这是给定场景的加速软件模型验证，不是两小时实机采集 / 播放。

```powershell
$env:MOIREN_FAULT_SIM_SECONDS = '7200'
cargo test --locked --release -p moiren-windows-audio --test clock_faults clock_faults_joint_queue_handles_bursts_stalls_and_pause_recovery -- --nocapture
./docs/superpowers/plans/2026-10-13-clock-faults/verify.ps1 -Csv ./docs/superpowers/plans/2026-10-13-clock-faults/results-7200.csv -Seconds 7200
```

矩阵加速执行耗时 486.66 秒，CSV 为 `2026-10-13-clock-faults/results-7200.csv`。随后按 release 模式重放完整 120 秒矩阵，耗时 7.81 秒，144 行全部字段与归档的 debug CSV 完全一致。源树的 `crates/` 内容与提交 `0fc8b28` 相同；日志分别为 `.tmp/audio-latency-audit/clock-faults-7200-0fc8b28.log` 与 `clock-faults-release-120-0fc8b28.log`。

两个 CSV 均通过独立 verifier。另使用临时副本验证了五类负例：缺少一行、坐标重复、非零稳定区短读、把实际 producer dropped 改成零、非法计数字符串，全部按预期拒绝。

本切片未进行新的实机采集或播放。完整 T4 仍未完成，下一切片需要把 packet stamp 与已接收帧 / generation 一致关联，再实现可信速率估计、最老帧音频年龄及持续故障的控制侧处理。
