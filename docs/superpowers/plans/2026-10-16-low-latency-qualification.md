# Low 档连续运行资格验证

日期：2026-10-16。承接[后端交接](2026-10-16-backend-low-latency-handoff.md)第 1 项。
本次补齐有限长测、异常窗口、需求缺口模型和实际设备结论，未修改档位队列或时钟伺服参数。

## 结论与适用范围

FreeDSP 输入 / 主输出 + CABLE 跟随输出的组合未通过 Low 无恢复资格验证：
同一个 owner 连续运行 600.004 秒，发生 120 次跟随积压恢复，主动丢弃 94134 帧，
约为每分钟 12 次恢复。主输出和物理输入没有积压恢复。各桥的 overflow drop 和
underrun 均为 0，但主动丢弃仍然意味着信号连续性未得到保证。

整段测试保持 owner / epoch、计划版本从 1 变为 2、时间线推进，最终 healthy、
`Closed` 且没有 cleanup error。这证明控制和回收正常，不能据此宣布音频连续。
工具的每路输出都有独立零增益保护，没有可听测试或外部回采。

本机后来只剩 Realtek 和 CABLE 可用，FreeDSP 已离线。旧 ID 的准备请求明确返回
`ResourceUnavailable`，正常关闭且没有活动 native 观测。因此后续 Realtek +
CABLE 的软件输入测试不能当作 FreeDSP 物理输入组合的直接对照。

Realtek 主输出 + CABLE 跟随输出、两路软件输入的 Balanced 单会话连续运行
600.005 秒，跟随积压恢复 / 主动丢弃 / overflow / underrun 均为 0，
保持 owner / epoch、计划 1 → 2、正常关闭。此记录通过了当前组合的 10 分钟
静音无桥恢复检查；仍未验证可听连续性、受控压力或 FreeDSP 输入的替代结果。

## 实际预算与观测

| 项目 | FreeDSP 主输出 / 输入 | CABLE 跟随输出 |
| --- | --- | --- |
| 实际 period | 336 帧，7 ms | 240 帧，5 ms |
| Render target | 504 帧 | 360 帧 |
| 输入桥 prime / high / capacity | 1010 / 1347 / 1683 | 866 / 1610 / 2114 |
| 最大 native wake 间隔 | 10.0207 ms | 8.5139 ms |
| 每次 native demand | 336 帧 | 0～360 帧 |
| empty padding / zero demand 次数 | 0 / 0 | 795 / 794 |
| 积压恢复 / 主动丢弃帧数 | 0 / 0 | 120 / 94134 |

长测开始于 09:43 左右，结束于 09:54。活动时间为 600.004 秒；进程总墙钟约
619.15 秒，包含准备、停止和 JSON 输出，不能作为音频时延。主输出累计提交
28802088 帧，跟随输出累计提交 28706880 帧。将各自提交帧数除以流运行时间，
得到约 48000.94 和 47842.44 帧/秒的粗略处理速率。这是 native demand 工作量
的观测，不是设备时钟漂移测量。

控制线程完成 29516 次健康轮询，保存 601 个 native 样本。系统 CPU 负载由
`Win32_Processor.LoadPercentage` 在外部脚本约每 11 秒读取一次，活动窗口内
54 个样本为 18%～85%，均值约 44.06%。这是有其他应用和构建活动时的环境负载，
不是受控压力实验，也不是音频 DSP CPU publisher；UI 的 CPU 未知值不变。

Balanced 对照记录的两个实际 period 都是 480 帧，render target 都是 960 帧。
主 / 跟随最大 wake 间隔为 10.932 / 13.118 ms；跟随输出仍有 437 次 empty padding
和 436 次 zero demand，但没有桥积压恢复。这些事件的含义必须和实际队列、
提交工作量一起判断，不能直接等同于硬件 XRUN。
其活动窗口内外部 CPU 样本为 32%～100%，均值约 68.52%，同样是环境负载记录。

当前 Realtek + CABLE 组合随后运行 Low 60 秒，实际 period 为 480 / 240 帧，
render target 为 720 / 360 帧，记录 23 次恢复、主动丢弃 23789 帧，没有
overflow / underrun。最终诊断修复二进制各做一次 Low / Balanced 15 秒短测，
两次都没有桥恢复；这两段短测只复核新二进制的正常路径，不能推翻此前长测
或说明队列行为已得到改善。本次没有修改队列 / 伺服参数。

## 恢复原因的证据与模型

此前 30 秒基线复现 5 次恢复，丢弃 3923 帧；新增 trace 后的另一段 30 秒记录
复现 9 次恢复，丢弃 7253 帧。不能只依据某一次短测的恢复率判断改善或退化。

异常窗口保留恢复前 8 次回调和恢复当次的 QPC、padding、demand、桥库存、
修正 ppm 及恢复计数。600 秒记录最后保留的 18 个恢复窗口，其所有上下文
wake 间隔均不超过 6.3803 ms，低于跟随 render target 对应的 7.5 ms。
恢复时高库存真实存在；并非每次都由当次 wake 超时触发。

窗口中反复出现 padding 为 360 时 demand 为 0，随后 demand 为 360 的序列。
两次正常回调应消费 `240 + 240 = 480` 帧，而 `0 + 360 = 360` 少消费 120 帧；
这种缺口累计后会推高桥库存。有限异常窗口和近似 scalar 快照不能证明具体
WASAPI 驱动内部原因，也不能把 empty padding 解释成硬件 XRUN。

`tests/output_followers.rs` 的
`persistent_native_demand_deficit_is_not_hidden_by_healthy_wake_intervals`
将这个条件转为可重放的 30 秒软件模型，使用真实 Low 准备预算和异步 336 / 240
帧生产 / 消费节奏：

- 正常 `240 + 240`：不恢复。
- 总消费量相同的 `120 + 360`：不恢复。
- 每 0.6 秒出现 `0 + 360`：持续消费缺口超过 2000 ppm 修正上限，明确积压恢复。

三组都要求无 overflow / underrun，修正不超过 2000 ppm，RT 不分配或释放内存。
这验证了持续需求缺口会触发有界恢复，未声称逐回调重放完整驱动行为。原有
3 相位 × 3 漂移模型继续覆盖预算内分段和补充 burst。

## 工具与诊断

`routing_probe` 默认仍为 5 秒，`--seconds` 限制为 1..60；显式
`--soak-seconds` 接受 60..1800，不能重复或与 `--seconds` 混用。
长测使用一个持续的 host，不是循环短会话。

探针约每 20 ms 检查 Running、owner、epoch、fault 和时间线；每秒保存 native
样本，最多 1801 个，保留终止样本槽位。观察到失败后仍关闭 host 并输出诊断，
原始运行错误优先于后续关闭错误。轮询只能检测已发布的状态，不能排除两次
快照之间的事件。

`--trace` 额外开启固定 128 个最近异常窗口，每个窗口最多 8 次前序回调。
窗口覆盖数单独报告，不能把未保留的事件当作没有发生。此次 Low 长测保留
128 个窗口，覆盖 740 个较早窗口，其中最后保留 18 个恢复窗口；累计桥计数
和全部 601 个控制侧样本仍保留。原有首 256 次回调 trace 的丢弃计数独立存在。

故障路径经过两项审查修复：诊断 recorder 由报告持有，在 native 回调 panic 后
仍保留已记录窗口，最终交给 joiner 释放；渲染、GetBuffer 或 ReleaseBuffer
返回错误时，先记录当次可用的 wake / padding / demand 和桥计数，再保留原错误。
失败回调带 `failed` 标记，提交帧数为 0；panic 当次不会伪装成已完成的回调。

无需改变系统设置的复现入口：

```powershell
cargo run --locked -p moiren-windows-audio `
    --target-dir .superpowers/managed-routing-target --example routing_probe -- --list
cargo run --locked -p moiren-windows-audio `
    --target-dir .superpowers/managed-routing-target --example routing_probe -- `
    --output '<current master ID>' --output '<current follower ID>' `
    --input '<current supported capture ID>' --soak-seconds 600 --profile low --trace
```

必须先确认 ID 仍可用；省略 `--input` 会混合两路软件音源。当前 Realtek capture
为 48 kHz 四声道、CABLE capture 为 96 kHz，均不满足现有物理输入格式限制。
本次没有更改默认端点、设备格式、音量或 mute。

## 降档与剩余验收

Low 的适用条件包括实际协商周期、队列准入，以及目标 IO 组合在实际负载下的
连续运行结果。仅通过准备或显示 healthy 不构成无恢复资格。

本次 FreeDSP + CABLE 组合应先选择 Balanced，再 Apply 并等待新 owner 的实际
档位确认，在同一设备组合上重跑连续测试；如果仍有恢复，再选择 Compatible。
这是手动选择和重新验证策略，未新增自动改档或自动写入偏好。

FreeDSP 离线后，尚不能完成该组合的 Balanced / Compatible 长测对照。
目前可确认的是 Realtek + CABLE 配合软件输入的 Balanced 记录，而非所有输入
及设备组合都已获得资格。
受控 CPU / IO 压力、运行中设备失效或拔插、睡眠恢复、多个物理输入和多个
进程捕获并行仍需独立验证。可听连续性、物理端到端时延及硬件 XRUN 也没有
测量，不能由本次静音队列记录推导。

最终二进制还明确拒绝了当前 Realtek 四声道物理输入；错误和取消后的 prepared
render 报告被保留，诊断 Trace 状态没有丢失，host 正常到达 `Closed`，活动
native 观测为空。该记录验证准备失败，不是运行中拔插验收。

## 续作软件验证

以下检查针对最终续作源码，使用隔离产物目录
`.superpowers/managed-routing-target`，没有终止用户应用：

| 检查 | 结果 |
| --- | --- |
| Windows audio 库、latency / clock_bridge / render / follower / budget / diagnostics 测试及 routing_probe 示例测试 | 192 passed，0 failed |
| 工作区 `cargo check --locked --workspace --all-targets` | 通过 |
| 工作区 all-target 严格 Clippy，包含 `moiren-app/gpui-test` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `git diff --check` | 通过，仅有现有 CRLF 转 LF 提示 |
| 最终 `routing_probe` 构建及两段 15 秒正常路径 / 四声道输入失败路径 | 符合预期 |
| 独立续作审查 | SPEC PASS / QUALITY APPROVE，0 个未解决发现 |

诊断审查的 8 个回归均先在原实现上失败，再在修复后通过。包含终止渲染错误、
GetBuffer / ReleaseBuffer 错误、非空 padding 的提交错误、prepared / direct owner
panic、取消 / Start 失败、准备前失败，以及同一诊断 Vec 在 native worker 上
未释放、在 joiner 上恰好释放一次的指针级验证。固定窗口及 `finish()` 的正常
路径保持零分配 / 释放断言。

全工作区测试、Ubuntu、nightly Miri 和远程 CI 没有在本次续作中重跑。
原验收中的 792 项工作区 / 257 项 GPUI 完整测试属于交接基线，不与本次 192 项
相加。续作不涉及前端修改；用户并行编辑与暂存的前端文件保持原样。

## 证据保存

可纳入 Git 的[精选证据 JSON](../../designs/evidence/2026-10-16-low-latency-soak.json)
包含原始日志 SHA-256、所用二进制 SHA-256、实际预算、累计计数、分钟样本、
恢复窗口及环境负载摘要。它位于 `docs/designs/evidence/`，避免现有
`docs/experiments/.gitignore` 的整体忽略规则。

原始约 6.7 MB Low JSON、601 个样本、stderr 进度、外部负载记录、诊断审查和
命令日志保存在 `.superpowers/sdd/managed-low-latency/continuation-20261016/`。
长测使用失败 / unwind 诊断保留修复之前的二进制；后续修复不改变队列或伺服，
该版本差异必须和最终短测证据一起保留。
