# T4 第五切片：长窗口 Capture / QPC 速率前馈

日期：2026-10-13。基线：`6e70cf2`。本切片将与音频帧一致提交的包时间用于速率估计，并接入 prepared live input 的既有 FIFO servo。音频年龄限制、控制侧持续故障降档和新的实机准入仍待后续实现。

## 估计口径与资格检查

`QueuedFrameStamp::rate_correction_ppm` 是 `Option<i64>`，表示 Capture 原生帧 / QPC 的速率相对名义输入 rate 的偏差，单位为四舍五入后的 ppm。它沿用原始包首帧 position 与已转换为 100 ns 的 QPC，不能用 callback 到达时间代替。[S1]

先经过既有包区间连续性、generation、QPC 非零 / 单调和 1% packet-slope 粗检，再进入 `clock_bridge/rate.rs` 的 tracker：

- 每个窗口至少积累一秒 QPC 时间与八个有效 packet。
- 每个窗口的绝对 Capture / QPC 偏差不得超过 ±2000 ppm。
- 连续两个窗口的估计须在 100 ppm 内一致；按两个窗口的总 frames / 总 QPC interval 计算发布值。
- 不一致窗口撤销当前估计，需新的两个一致窗口才能再次发布。
- timestamp error、零 / 非单调 / 异常 QPC、generation 变化、position overflow 或完全丢包重置资格。空包 / 非法 payload 保持先前既有契约，不污染 tracker。

所有时间与位置先做整数差分，再以 i128 推导 ppm，避免把大的绝对 QPC 转成 f64 后丢失细小 interval 精度。窗口计数饱和，窗口合并用 checked addition；状态固定大小，无队列、重试或等待。

速率值与 packet stamp 一起存于对应音频 Frame。consumer 读取的是当前实际使用帧所携带的估计。新无效或完全丢弃的 packet 不会给旧队列中的帧换标签。Process / positionless source 的 tracking disabled 时始终没有估计。

这些阈值是保守的模型资格规则，不是对驱动 QPC 精度或设备时钟的认证。

## Servo 集成与边界

仅 `prepared_input.is_some() && trim_on_prime` 的 live 路径使用前馈。没有 prepared watermark 的 legacy 路径和 `trim_on_prime=false` 的有限离线策略继续使用 FIFO 控制；原始关联仍可被查询。

前馈、比例项与积分项的总和共用 `max_correction_ppm` 限幅，配置上限仍为 ±2000 ppm。anti-windup 以总修正判断饱和；低水位 guard 覆盖总修正并暂停积分学习。`BridgeSnapshot::correction_ppm` 继续记录实际总修正，测量 schema 未变。SRC phase、既有 reset / backlog recovery / fade 保持原有职责。

Capture / QPC 与 Capture / Render 是不同口径。Render 本身可能漂移，FIFO servo 必须继续学习 residual；不能把 input 的绝对 ppm 当成两端之间的最终比率。新增双端模型用 `(1 + capture_ppm / 1e6) / (1 + render_ppm / 1e6)` 推导正确的相对目标。

Frame 储存预算仍由 `capacity × size_of::<Frame>()` 自动核算，包含新增速率字段。所有分配发生于非 RT preparation。有效窗口更新、source read、stamp 查询和 observer snapshot 的回归测得 0 allocation / 0 deallocation。

## 回归与短矩阵结果

`clock_rate` 新增八项回归：长窗口预热、错误 / discontinuity 后重新预热、±5000 ppm 撤销、positionless fallback、实际 queued 前馈及总限幅 / guard / offline、旧排队估计不被新丢包污染、大绝对 QPC / 量化 jitter，以及窗口失配后的重新发布。`clock_bridge` 新增有效窗口更新的 RT allocation 回归。

RED 接口壳树 `47f4769122cd344e16e5e0d1e328ac9ad9431a2f` 中四项初始速率测试 2 failed / 2 passed；失败发生在窗口预热后仍没有估计。离线策略回归在树 `5be3c465d5d06fcfb5bebebc07cdd53982fbf868` 复现了不应启用前馈的错误，修正后通过。在隔离导出树中去掉 source 前馈、将一秒窗口缩为 10 ms，分别被实际行为断言拒绝。

最终运行实现树：`8fd11c810df1d1c277668e9c0559c20ed50a9cf1`，导出目录 `.tmp/audio-latency-audit/staged-clock-rate-final-v2`。fmt、严格全工作区 / all-targets Clippy 和完整工作区测试通过：47 个套件共 541 passed、0 failed、4 ignored。release 聚焦测试也通过：clock_bridge 29、clock_timing 11、clock_rate 8、clock_control 7、clock_faults 2，共 57 项。

各 120 秒的 208 个模型组合全部通过：

| 矩阵 | 组合数 | 本切片结果 |
| --- | ---: | --- |
| 原低库存、相位、顺序、迟包控制 | 48 | short / underrun / drop / backlog recovery 均为 0；最大均值误差 71.607173 ppm |
| 新双端漂移及 timestamp-error fallback | 16 | 两种 rate × Capture ±1000 ppm × Render ±500 ppm × 有效 / 无效时间；全部零故障，最大相对均值误差 29.810817 ppm |
| 联合 padding / demand 故障与恢复 | 144 | 保留主动 Render pause 的预期故障；所有稳定阶段 short / xrun 为 0，最后一次恢复短读距 resume 不超过 1104 output frames |

联合故障矩阵的完整计数：81 short callbacks、48 source xruns、451584 Render missing frames、0 input underrun frames、104842 producer dropped frames、48 backlog recoveries、344160 intentional discarded frames、96 resets、137056 low-water guard frames。不能把主动停顿的非零故障从总数中隐去。

短矩阵 CSV 为 [control-120.csv](2026-10-13-clock-rate/control-120.csv)、[rate-120.csv](2026-10-13-clock-rate/rate-120.csv)、[faults-120.csv](2026-10-13-clock-rate/faults-120.csv)。旧控制 / 故障 verifier 可用 `-Csv` 指向新的 CSV；新的 [rate verifier](2026-10-13-clock-rate/verify.ps1) 检查双端坐标、duration、零故障、前馈 / fallback 与相对误差。

## 同一实现的 7200 秒扩展

`df344d3` 提交的运行实现与上述最终树的 `src` / `tests` 字节一致。对该实现重新执行了 64 个控制 / 双端漂移组合和 144 个联合故障组合，每个组合模拟 7200 秒，全部通过。没有复用此前 `0fc8b28` / `66a4a9a` 的长测结论。

| 矩阵 | 7200 秒结果 |
| --- | --- |
| 控制 48 组 | short / underrun / drop / backlog recovery 均为 0；最大均值误差 0.978078 ppm；guard 共 38592 frames |
| 双端漂移 / fallback 16 组 | 同样零故障；最大相对均值误差 0.184261 ppm；有效源保持 Capture ±1000 ppm 的 queued estimate，无效源保持 None |
| 联合故障 144 组 | 除 seconds 字段外，各坐标的全部字段与 120 秒 CSV 完全一致，包括 guard 计数；稳定阶段零 short / xrun，主动暂停的完整故障计数保留 |

长矩阵为 [control-7200.csv](2026-10-13-clock-rate/control-7200.csv)、[rate-7200.csv](2026-10-13-clock-rate/rate-7200.csv)、[faults-7200.csv](2026-10-13-clock-rate/faults-7200.csv)。三个 verifier 用对应 CSV 和 `-Seconds 7200` 都通过。新 rate verifier 的缺行、重复坐标、非零故障、隐藏有效 rate、NaN 和错误相对修正六份破坏样本均被拒绝。

长测命令从最终导出目录运行：

```powershell
$env:MOIREN_CONTROL_SIM_SECONDS = '7200'
$env:MOIREN_FAULT_SIM_SECONDS = '7200'
cargo test --locked --release -p moiren-windows-audio --test clock_control --test clock_faults -- --nocapture
```

完整日志 `clock-rate-final-v2-7200.log` 中 control 7 个测试耗时 226.19 秒，faults 2 个测试耗时 772.74 秒；这些是主机计算耗时，不是音频时延。模型仍采用瞬时 callback，不包含实机调度、系统负载或实际 DSP 耗时的认证。

日志位于 `.tmp/audio-latency-audit/`：`clock-rate-{check,fmt,clippy,workspace-tests}.log`、`clock-rate-final-v2-{120,7200}.log`、`clock-rate-red.log`、`clock-rate-offline-red.log`、`clock-rate-mutation-{feed,window}.log`。

本切片没有新的实机采集、播放、物理闭环或音频年龄测量，不声称已改善实测物理时延。T4 整体及 T5–T8 保持未完成状态。

## 参考

- [S1：IAudioCaptureClient::GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)，核读于 2026-10-13。
