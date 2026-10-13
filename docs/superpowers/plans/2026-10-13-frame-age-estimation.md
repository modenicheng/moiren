# T4：最老待用输入帧年龄估算

日期：2026-10-13。基于 T4 已提交的包与 Frame 时间关联、长窗口 Capture/QPC rate qualification。当前切片增加 owner-local 查询、可选的有界年龄恢复和 native QPC clock 注入，但不声称改变物理端到端时延。

## 定义

`ClockSource::oldest_frame_age(now_qpc_100ns)` 先沿用 `oldest_frame_stamp()` 找到最老实际待用 native frame，再按以下步骤估算：

```text
rate = queued Capture/QPC rate estimate, else nominal input sample rate
frame_offset_100ns = round(packet_offset_frames × 10_000_000 / rate)
estimated_frame_qpc = packet_qpc_100ns + frame_offset_100ns
estimated_age = now_qpc_100ns - estimated_frame_qpc
```

若已资格化的 capture rate 相对 nominal rate 为 `ppm`，计算使用 `rate = input_sample_rate × (1_000_000 + ppm) / 1_000_000`，全程用整数 / `u128` 中间值，按最近 100 ns 取整。未取得双窗口 estimate 时用 nominal rate，并由结果中的 `rate_correction_ppm: None` 保留该区别。返回值携带原 `QueuedFrameStamp`、输入率、观察 QPC、投影帧 QPC 和年龄；读取不会消费队列或改变 source / telemetry 状态。

空队列、未资格化 packet correlation、零观察 QPC、offset 超过已接收 prefix、无效的 rate 限制、时间加法溢出或观测早于估算帧时间，都会返回 `None`，不回绕、不把未知伪装成零年龄。估算年龄可以恰为零。

## 口径与边界

这是 QPC 公共时间域中、packet 首帧到最老待用 capture frame 的**估算**时间，不是设备交付延迟、DSP 延迟、Render 排队、呈现时间或物理闭环测量。已校准 rate 有助于缩小 packet 内帧偏移估算误差，但两窗口资格阈值及粗略斜率门限不是驱动时间戳准确度、packet 内速率稳定性或 offset 误差的认证上界；本接口不提供可信误差区间。上层只能把该值当作显式估算，不能据此声称最坏时延已被证明。

`now_qpc_100ns` 必须来自同一 QPC / 100 ns 域；callback 到达时刻不得替代 Capture packet stamp。Windows native capture 在准备阶段预备与 WASAPI packet 相同时间域的 QPC clock，并由 owner 传给 `ClockSource`；普通 `ClockBridgeConfig` 和 `with_prepared_input` 的 `age_limit_100ns` 默认仍为 `None`，因此 native 路径当前只具备测量基础，不会默认丢弃旧帧。显式设置年龄上限时，consumer 只在一次初始 queue snapshot 内按年龄换算有限丢弃量，重置 phase / servo 并执行 32 帧 fade；high-water recovery 优先，同一状态不重复恢复，producer 不会被 consumer 越权移动。`trim_on_prime=false` 的离线 FIFO 永远不启用 age discard。positionless virtual Process 源以及时间戳错误后的队列继续可正常 FIFO 消费，但年龄为 unavailable。

## 回归与验证

`clock_age` 的 11 项测试覆盖不消费的重复查询、44.1 / 48 kHz offset 舍入、current/next/ring 队列选择所继承的 phase 状态、部分接收前缀、完全丢包不重标旧帧、positionless 和无效时间回退、新 generation 预热、未来时刻及 QPC 加法 / 观测边界、已排队 ±1000 ppm rate estimate，以及年龄恢复的一次性快照、空 demand、离线保护和 high-water 优先级。

`clock_bridge` 的 allocation/deallocation 计数器覆盖年龄查询和 age recovery 路径。该测试在 Release 与桥接、packet timing、rate 套件一起运行；全 workspace `fmt --check`、`check --workspace --all-targets`、strict Clippy 和完整 workspace tests 基于只含精确 staged paths 的归档树运行。没有 Windows 设备实机测量，年龄上限尚未按设备 / driver / format / mode 完成硬件准入，也没有物理端到端验收。
