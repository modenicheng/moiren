# T4 第八切片：持续故障的控制侧回退

日期：2026-10-14。范围为应用服务的物理输入监听、便携控制协议、Render 淡入淡出及 WASAPI 提交确认。本轮没有开展新的实机采集、播放或物理端到端时延测量。

## 1. 启用方式与边界

`BackendDriver::new(...)` 保留现有默认行为。应用可通过 `with_monitor_latency(config, Some(policy))` 显式启用持续故障回退；传入 `None` 只选择准备策略，不自动重启。策略仅用于 Physical 输入，不改变 Tone 或 Process Loopback 的准备契约。

```rust
let driver = BackendDriver::new(Arc::new(WindowsBackend::default()))
    .with_monitor_latency(
        MonitorLatencyConfig::balanced(),
        Some(MonitorRecoveryPolicy::default()),
    )?;
let runtime = ServiceRuntime::spawn_with_driver(driver)?;
```

示例中的 Balanced 仍是实验配置，实际周期与联合预算不能满足时仍在准备阶段拒绝，不能将失败初始化隐藏为普通回退。此接口没有增加 CLI 或 UI 开关，也没有修改默认档位或授予硬件准入。

## 2. 故障窗口

`MonitorRecovery` 在控制侧读取累计计数。默认忽略启动后的前 1 秒，再要求连续 3 个 250 ms 观测窗口出现新的故障。原因保留 priming、underrun、producer drop、reset、high-water recovery、age recovery 和 discontinuity 的增量。正常静音和 low-water guard 不作为故障。

检测使用 Render 的 Engine timeline 作为处理进度；Bridge 的成功输出帧数不能代表持续输出 priming 静音的进度。producer 已结束时不再提供故障快照，正常结束由原会话生命周期处理。

重复快照不会重复消费计数。健康窗口、计数回退、时间回退和超过允许值的轮询间隔会清空连续窗口。一次观察不能补造多个未观察窗口；每个 detector 最多发出一次回退原因，固定数量的标量状态不会随运行时间增长。快照仍是独立 atomic 的近似观测，不是音频事务或呈现时间。

## 3. 有序替换

1. 原会话继续运行，独立的输出 envelope 在 240 个输出帧内淡出；48 kHz 下为 5 ms 的标称淡出长度。参数请求队列、用户增益和声像不用于内部静音。
2. Render 只有在 `ReleaseBuffer` 成功后确认淡出样本已提交。随后控制侧等待 native capacity 加一个 actual period 对应的排空时间。
3. 超过策略的淡出期限仍受控停止，并在原因中记录 `fade_timed_out`；故障 owner 不会无限阻塞服务轮询。这不承诺设备故障情况下仍能无缝淡出。
4. 服务推进 session generation，通知原 Capture / Render 停止，在 cleanup worker 上 join 两端并退役已接受的 controls。清理结果与 control ledger 未收完前不得重新准备。
5. 新会话使用 Compatible 队列及设备默认 Shared 周期，重新创建 native client、Bridge、graph、signal host 和 timeline epoch，启动时淡入。不存在 RT 环重分配、COM 重建或 graph 修改。

同一用户 Start 最多自动回退一次；Compatible 会话不再挂持续故障 detector。回退清理或重新准备失败会终结本次 intent，不进行无界重试。Stop、Exit 或新的 Start 可撤销回退；旧 generation 的失败不得误伤后续用户请求。

最新用户增益和声像写回 desired spec，在重建时保留。旧控制回复仍按原 revision / epoch 退役，新图获得新 epoch，plan-local revision 仍遵循现有编译契约。有限运行在触发替换时只保留剩余秒数并向上取整；这不是跨停启的精确墙钟期限，清理与准备间隙仍可能延长总墙钟耗时。

## 4. 可观察状态

`AppSnapshot.monitor_latency` 返回 requested 配置、selected 准备策略、原因增量、原/新 generation 和淡出超时标志。`selected` 表示将传给 native preparation 的策略，不能当作 actual period、测得时延或已准入的硬件档位。实际设备周期、队列预算及初始化失败仍使用原 native diagnostics。

T5 的 UI 档位选择、actual period / 队列显示和回退原因展示仍待完成。设备默认周期、FIFO 容量、排空估计和淡出帧数都不能当作物理端到端测量。

## 5. 回归与验收

- 故障 detector：启动宽限期、旧计数、每种原因、连续窗口、健康窗口、计数/时间回退、重复观察、停止输出和非法策略。
- 服务：正常淡出与排空、阻塞 join 时禁止提前准备、增益/声像保留、单次 Compatible 回退、Stop/Exit 撤销、淡出超时、cleanup/prepare 失败终结、用户 Start 覆盖回退以及旧失败隔离。
- Render：1/3/8/32 帧 demand 切分得到同样 envelope，零 demand 不推进 timeline，未提交不能确认完成，淡出后保持静音，成功路径分配/释放次数为 0。

本切片只完成可重复软件验收。T4 整体仍保留低库存控制进一步标定及硬件准入缺口；T5 的界面与物理测量任务仍未完成。

## 6. 本轮验证结果

以下检查于 2026-10-14 完成，命令均返回退出码 0：

- 当前工作区 `cargo test --locked --workspace`：51 组测试结果，共 584 项通过、0 项失败、4 项忽略；忽略项为需显式开启的原有 native window / audio 验收。本轮新增 17 项测试，覆盖 detector 8 项、服务回退 7 项、有限运行剩余时间 1 项和 Render envelope 1 项。
- 当前工作区 `cargo fmt --all -- --check` 与 `cargo clippy --locked --workspace --all-targets -- -D warnings` 均通过。
- 为排除未提交的 Cargo / UI 草稿对本提交的影响，将暂存树导出后，独立运行 `cargo check --locked --workspace --all-targets`、格式检查及严格 workspace Clippy，均通过。
- 导出暂存树中的 `cargo test --locked -p moiren-app --lib --test monitor_recovery --test service_recovery --test service_backend --test service --test signal_service`：71 项通过、0 项失败、4 项忽略。
- 导出暂存树中的 `cargo test --locked -p moiren-windows-audio --test render`：10 项全部通过，包含 envelope 成功路径的 0 分配 / 0 释放检查。

工作区全量测试与导出树聚焦测试分别记录，测试总数不混用。新增回退行为在导出树中完成独立验证；本轮仍没有新的实机采集、播放或物理时延结果。
