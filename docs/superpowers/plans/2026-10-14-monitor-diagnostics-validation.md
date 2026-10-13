# 物理 Monitor 的服务观测链

日期：2026-10-14。接续持续故障回退，补齐 T5 所需的实际周期与队列信息。范围为 Physical Monitor 的准备信息保存、服务快照和可重复软件验收。

## 1. 设计与实施顺序

现有 `AppSnapshot.monitor_latency.selected` 仅代表准备策略，实际两端周期与联合预算在 native preparation 中生成。直接用 selected 重建显示值会遗漏请求与结果的差异；从终止后的报告取得这些值又无法支持运行中的界面。

本轮选择保存准备时返回的纯 Rust 标量，并复用已有 Bridge / Engine observer。另一种方式是在音频线程新增 UI 统计通道，会增加热路径工作；当前已有信息足够完成本切片，因此无须新增 RT 写入、查询 COM 或改变音频排队。

1. 建立平台无关的 `MonitorPreparedDiagnostics` 与 `MonitorObservation`。保留两端 rate、requested / actual period、native capacity、初始化 API / fallback，以及解析后的输入和 Render 预算。
2. 在两端 native preparation 已返回且联合预算已解析后保存诊断。Capture / Render COM 对象仍由原 owner 持有；Process Loopback 不借用 Physical 的周期信息。
3. 通过 `ActiveSession::monitor_observation` 与 `AppSnapshot.monitor_diagnostics` 发布纯 Rust 快照。快照带 owner generation，且只在当前 Physical 会话为 Running 时发布。Starting、Stop、Exit、失败和替换期间清空旧诊断；替换运行后使用新 owner 的准备结果。
4. 先验证缺失发布的失败回归，再运行聚焦测试、workspace check / fmt / Clippy，并在导出的暂存树中独立验证。完成后按本切片原子提交。

## 2. 数字含义

`actual_period_frames: None` 保持未知，不能拿 requested period、Engine block 或设备默认能力代填。帧数换算使用该流自己的 sample rate，向上取整到纳秒；非法 rate 或换算上溢返回 `None`。

输入的 prime threshold、servo reference、low / high water 与 capacity 分别保留；Render 的 target、capacity 和 optional deadline budget 也分别保留。`input_fill_frames` 是已有 Bridge 的近似库存采样，其 duration 仅是库存尺度；processed frame 来自包含 priming / 静音的 Engine timeline。独立 atomic 读数不构成音频事务。

本接口没有物理 E2E 或呈现年龄字段，也没有把两端库存相加当作总时延。UI 档位选择、实际周期与原因展示、硬件准入及物理端到端测量仍待后续集成。本轮不修改默认 profile，不启动新的实机采集或播放。

## 3. 软件验收范围

- 实际周期独立于请求值、两端不同 rate、未知周期、零值和无效 rate。
- 空输入库存、非整数纳秒换算、上溢；库存、输入目标和 Render capacity 保持独立。
- native 标量转换保留解析预算、初始化 API / fallback 和未知 actual period。
- 双 owner 尚未 Started 时不发布；运行中库存更新与已持有快照隔离。
- Stop 清空旧数据，即使 join 仍阻塞；自动回退后标注新 generation 并换用 Compatible 预算。
- backend 未提供实际诊断时保留 `None`，selected 策略不能制造诊断。
- 默认驱动无需 profile override 即可发布实际诊断；Tone / Process 即使错误 backend 返回 Physical 诊断也不对外发布。

## 4. 本轮验证结果

2026-10-14 完成，以下命令均返回退出码 0：

- `cargo test --locked -p moiren-app --lib --tests`：16 个 test binary，137 项通过、0 项失败、4 项忽略；包含 3 项诊断专用测试和 12 项服务回退测试。首次快照的非阻塞读取使用既有等待辅助，避免把服务线程启动竞争误判为诊断缺失。
- `cargo test --locked -p moiren-windows-audio --test render --test latency --test clock_bridge`：54 项通过、0 项失败。
- `cargo test --locked --workspace`：54 个 test binary，646 项通过、0 项失败、6 项忽略，包含既有长时钟压力矩阵。忽略项仍是需显式开启的原有 native window / audio 验收。
- `cargo check --locked --workspace --all-targets`、`cargo check --locked --target x86_64-pc-windows-msvc --workspace --all-targets`、`cargo fmt --all -- --check` 和 `cargo clippy --locked --workspace --all-targets -- -D warnings` 均通过。

Windows 目标只完成条件编译检查，没有在本轮启动物理设备。上述通过结果证明的是服务快照接线、预算标量转换和可重复软件路径；不构成实际设备周期、物理端到端时延或硬件准入结论。
