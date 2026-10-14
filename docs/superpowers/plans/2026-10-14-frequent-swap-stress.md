# 频繁实际 Swap 压测

## 测量约定与执行计划

本轮延续现有托管引擎压测，不改生产调度或 UI。此前每秒 400 次提交被大量合并，不能当成每秒 400 次实际换图。

- 增加可选闭环模式：同一时间只有一个结构性编辑，Activated 回执、Host 活跃版本和实际 RT timeline 必须一致；计划版本必须改变，epoch 和 native owner 不变，frame 继续前进，才计一次 swap。
- 第一次编辑必须与初始配置不同，避免把 no-op 激活当作换图。逐次确认后才提交下一次，无法达到目标速率时报告实际吞吐，不制造积压或把合并计为完成。
- 先单测计数语义，再运行 Release、真实 WASAPI、全程末级静音的 20/50/100 次每秒及闭环饱和用例；加入同规模不换图基线。
- 保留原始逐秒进程资源、实际换图进度、延迟分位数和原生音频统计。监测 host fault、timeline stall、bridge shortfall、owner/epoch 变化、回执收敛及关闭回收。
- 在独立工作树验证并合并，保留主工作区正在改进的 UI 文件；只管理本轮启动的探针进程。

闭环吞吐包含控制侧确认轮询开销，不等于裸 RT swap 的理论上限；本机有限时长测试也不是长期无泄漏或所有驱动无 XRUN 的保证。

## 方法与复现

Release 探针使用真实默认 WASAPI Multimedia 输出，48 kHz / stereo，本轮实际共享周期为 480 帧（10 ms）。软件 tone → 工作负载处理链 → 独立 Gain(0) → 输出，全程静音、不录音、不改变系统默认设备。每四个工作负载处理器有一个 Compressor，其余为 Gain；表中处理器数不含 source、静音 guard 和 output 三个节点。

交替修改第一个 Gain 的静态初值 0.95 / 1.0，触发执行计划重新编译和替换，不是仅发送动态参数。另一个 Gain 同时接收 200 条/秒 NextBlock 参数命令，保持所有用例的参数负载一致。首个静态修改明确不同于初始值。

`--confirmed-swaps 1` 在每次 edit 回执 Activated 后，还要求 snapshot 的 graph/plan revision 与 RT timeline revision 相符、plan revision 严格增加、frame 前进、owner/epoch 不变。确认前不发下一次结构性编辑，禁止把 Superseded、no-op 或仅完成编译计入 swap。饱和档的 2,000/s 是调度上限，实际只按确认吞吐提交，不代表真的执行 2,000 次/s。

```powershell
cargo build --release --locked -p moiren-windows-audio --example managed_stress
.\scripts\managed-stress.ps1 -FrequentSwaps `
    -OutputDirectory .\docs\superpowers\plans\frequent-swap-rerun `
    -BaselineSeconds 30 -LoadSeconds 60 -SaturationSeconds 300
```

每秒采样独立探针进程资源，去掉前 5 秒及负载结束后的关闭/启停阶段。CPU 单核口径为进程累计 CPU 时间差 / 采样墙钟时间，整机口径再除以 16。激活延迟为提交回执返回后到控制侧观察到 RT 激活的时间，包含 2 ms 轮询；分位数以 1 ms 桶上界表示，不是音频回调的执行时间。

机器为 Intel Core Ultra 9 285H / 16 逻辑处理器。二进制 SHA-256：`242A2E32299DE8276C1A5D3E0C9D43EB82F2EDF7217F22749419B3EC4DC5B97F`。完整环境、原始进程样本、每秒实际 swap 进度、最终原生报告分别保存在 [frequent-swap](2026-10-14-frequent-swap/summary.json) 目录下。没有同时运行编译/测试来人为竞争本轮测量，但未锁定 CPU 频率、核心或其它系统任务。

## 实测结果

主测总负载窗口 510 秒，累计 **39,698 次实际 swap**，所有已提交结构性编辑均逐次激活，**0 次结构性编辑合并、0 次提交容量拒绝**。

| 场景 | 处理器 | 时长 | 实际 swap / 实际每秒 | CPU 单核 / 整机 | Private 峰值 | Working Set 峰值 | 激活 p99 / 最大 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 不换图基线 | 120 | 30 s | 0 / 0 | 9.88% / 0.62% | 4.03 MiB | 13.70 MiB | — |
| 目标 20/s | 64 | 60 s | 1,199 / 19.98 | 6.75% / 0.42% | 5.45 MiB | 14.80 MiB | 15 / 21.86 ms |
| 目标 50/s | 120 | 60 s | 2,999 / 49.97 | 16.86% / 1.05% | 6.85 MiB | 16.32 MiB | 22 / 23.16 ms |
| 目标 100/s | 120 | 60 s | 5,879 / 97.94 | 24.95% / 1.56% | 6.80 MiB | 16.16 MiB | 17 / 27.72 ms |
| 闭环饱和 | 120 | 300 s | 29,621 / 98.73 | 27.58% / 1.72% | 7.29 MiB | 16.71 MiB | 17 / 71.36 ms |

20/s 档节点数较少，不能把它低于 120 节点基线的 CPU 解释为 swap 降低开销。120 节点基线与其它三个 120 节点档才是同规模对比；CPU 仍受频率、调度和参数合并比例影响。

### 五分钟饱和档

- 计划 revision 从 1 到 29,622；29,621 条 edit 回执全部 Activated，全部由 RT timeline 复核。owner 与 epoch 始终为 1，desired-active 差值最大为 1，没有通过反复重启音频伪装 swap。
- 299 条逐秒进度记录；相邻进度窗口实际速率约 75.12–100.96/s，证明在运行期间持续换图，而非最后排空时一次性计数。整体 98.73/s 接近本次 10 ms native 周期对应的每秒 100 次唤醒，但这是该后端、图和闭环确认策略的测量，不是引擎理论上限。
- 音频 callback p99 上界 1.229 ms、最大 1.787 ms，低于本次 10 ms 周期。最长 native wake 间隔约 11.869 ms；最长控制侧时间线观测间隙 62 ms、单次激活确认 71.362 ms。后两项包含观察线程/协调器调度，不能当作 callback 执行耗时。
- Private bytes 从稳定采样首点 6.55 MiB 到末点 7.12 MiB，变化 +0.56 MiB；峰值 7.29 MiB。每 30 秒窗口均值约 6.55–6.91 MiB，后半段仍有回落而不是随 swap 数线性累积；不足以证明长期完全无泄漏。
- 句柄 187–189、线程 6–12，未随 swap 次数持续增加。负载结束后追加 10 次打开/关闭全部到达 Closed，该档最长关闭 36 ms；五档全部主会话也正常关闭，总计 15 次关闭验证，最慢 39 ms。
- 59,999 条并发参数命令中 3,807 Applied、56,192 Superseded，全部终结。参数保持最新值合并语义；**结构性编辑零合并不等于每条参数命令都执行**。持续换图时参数 Applied 吞吐明显下降，是本轮可见的行为边界。

五档均无 host fault、cleanup error、native failure、bridge shortfall、empty-padding wake、timeout wake 或非零音频输出。零 shortfall 等计数只覆盖现有观测项，不等同于证明驱动在任意情况下绝无 XRUN。

## 验证与边界

- 新增/相关探针单测 6 项通过：确认计数、no-op/重复回执、缺失/旧 timeline、owner/epoch 改变、时间线倒退、延迟分位数、参数范围与静音 guard。
- 引擎 host 25 项、runtime 56 项通过，保留 swap 的 RT 零分配/零析构、退休队列背压及控制侧回收覆盖。
- 工作区 `cargo check --locked --workspace --all-targets`、严格 Clippy、fmt 检查通过。没有为本次探针变更宣称重跑完整硬件矩阵或 Miri。
- 所有探针进程均已退出。只改测量探针、脚本和证据，不改生产引擎调度，也不改正在演进的 UI。该结果覆盖真实输出的频繁计划替换，不覆盖物理输入桥、设备拔插、多设备漂移或小时级 soak。
