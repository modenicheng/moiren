# 合并与优化总方案（Consolidation Plan）

> **执行状态（2026-10-29 更新）：P0–P4 已全部执行完毕并合并。** P0=PR #18（cccb42a）、P1=PR #19（b240b70）、P3+P4=PR #20（88d2774，含 legacy 栈删除）。遗留的用户决策项（WA-2/3/5/6 重写、D-2/D-3/D-5/D-6/D-7）见同目录各报告与 2026-10-27 会话记录，仍待拍板。

> 本文是本轮审计的落地工单总表：把五份分报告的全部建议按**依赖顺序**与**风险等级**编组为六个阶段，并单列需要用户拍板的决策项。所有工单默认行为保持（behavior-preserving），例外已标注；全部不触碰 RT 热路径（宏/trait 方法等均在控制侧或编译期）。
> 通用验证门：`cargo fmt --all -- --check`、`cargo clippy --locked --workspace --all-targets -- -D warnings`、受影响 crate 的 `cargo test --locked -p <crate>`；UI 工单加 `cargo test --locked -p moiren-app --features gpui-test`。

## 阶段总览

| 阶段 | 主题 | 规模 | 前置 |
|---|---|---|---|
| P0 | 零风险修正与文档对账 | 小，可 1-2 个 PR | 无 |
| P1 | 高价值合并（互相独立，可并行开发） | 中，~8 个 PR | 无 |
| P2 | meter/信号链路收敛（顺序执行） | 中 | EN-1 → X-1 |
| P3 | app legacy 驱动栈退役 | 大 | 建议在 P2 后 |
| P4 | 双后端退役 W-K（windows-audio） | 最大，需实机 | WA-1 四缺口 + 用户决策 |
| P5 | 收尾卫生与文档归档 | 小 | 各阶段完成后 |

---

## P0 — 零风险修正与文档对账（立即）

1. **APP-2 `typing_focus` 单一谓词**（正确性，最先做）：`ui/window.rs:857-865` 与 `892-900` 合并为一个谓词，全列 node_search / node_name / compressor.inputs / **mixer.send_inputs（新覆盖，修 Ctrl+Z 误触整图 undo 的洞）** / export_controls。加一条"发送输入框聚焦时 Ctrl+Z 不触发图 undo"的回归用例。
2. **WA-2 Tone 单一实现**：以 `professional/bridge.rs:131-162` 的泛型版为基底移入中立模块，legacy 路径改调它；幅度与频率上限二选一钉死（见决策 D-3），补频率/幅度/相位连续性单测。
3. **EN-8 Compressor `validate_linked_io` 统一**：换用共享校验（行为变化：>256ch Compressor 节点开始被拒，与 Gate/Limiter/EQ 一致——在 PR 描述里明示）。
4. **X-5 VAD 文档对账**（30 分钟）：`docs/vad-validation-2026-10-21.md` 头部 Result 改为指针；`driver/vad/STATUS.md` gates 更新为"8h idle 已过、music/16h 未过"；指定 STATUS.md 为运行状态唯一 owner。
5. **Stream 标注落地**：在 moiren-signal（lib.rs 或 stream.rs 模块 doc）标注 "Stream transport: awaiting first routed consumer (UI waveform)"——兑现 2026-10-23 的既定结论。
6. **X-6 依赖上收**：`rtrb`/`serde`/`serde_json` 等提升到 `[workspace.dependencies]`，统一 windows pin 风格（`=` 与否二选一）。
7. **顺手项**：CS-5 `check_open` 单源 + 摘除失真的 `allow(dead_code)`；EN-9 `Sum`/`Bus` 双名收敛；X-9 designs 序号 12 改号；X-8 FRONTEND.md 补 undo/侧栏/工具栏三节。
8. **D2 的一半**：删除 8 个 0 字节 `*.stderr.log`（零字节、零引用，安全类）；~481KB 明细留决策 D-5。

## P1 — 高价值合并（互相独立，可并行）

| 工单 | 动作 | 主要验证 |
|---|---|---|
| **EN-3 ParamSpec 单源** | project.rs 加私有 `node_specs(config, width_hint)`，`validate`/`validate_parameter`/`parameter_values` 三处 + Strip/Edge 手写域全部改走 kernel 表；删 450-461 等手写臂 | tests/host/parameters.rs、tests/host/project.rs |
| **EN-5 kernel prepare 入口** | 各固定 schema 内核加 `X::prepare(channels, settings, sr, id) -> Result<(Self, Vec<ParamSpec>)>`；rack/prepare.rs 与 compiler/plan.rs match 臂收敛为一行；`MAX_DELAY_MS` 三处 → `DelaySettings::storage_valid()` 一处 | tests/rack、tests/compiler |
| **EN-6 `transport_interchangeable`** | `RtProcessor` 默认 false，Gain/Pan 覆写 true；plan.rs:378 与 swap.rs:547-551 都改查该方法 | tests/runtime/swap.rs reuse 用例、tests/compiler/optimization.rs |
| **EN-4 `linked_io!` 宏** | 7 个 process() 双臂样板各缩至 3-5 行（保持单态化）；若团队偏好显式 match，降级为"新内核必须用宏"约定 | tests/processor/tests/contracts.rs |
| **WA-3 进程环回单源** | "selector 解析 + 身份校验 + 准备 + observation"抽到 `process_loopback` 单一入口，两栈调用 | tests/process_loopback.rs |
| **WA-6 role→ERole helper** | `negotiation.rs` 加映射 helper，三处（professional/wasapi、engine_backend/resources、catalog）改调 | catalog 测试 |
| **WA-5 中立层归位**（纯移动） | `resample.rs`/`servo.rs` 移出 `professional/`（顶层或并入 virtual_owner） | 全量 windows-audio 测试 |
| **EN-7 SignalHub 内部收敛** | prune 抽 `unregister_stale` 泛型 helper；`Managed*Subscription` 三胞胎合并为 `ManagedSubscription<T>`（留别名）；错误映射 `From<HostError>` | tests/host/signals.rs |
| **CS-1/CS-2 signal 第二批收敛** | `LatestSender::eligible`；retired 前奏 helper；`reserve_subscription_bytes`（20 行 ×2）；不变式 doc 集中 | signal 全套 + tests/registry.rs 竞态守护 |
| **CS-3/CS-4 core 死 API 删除** | 删 `validate_connection` 与 `GraphIndex`（~320 行），routing_probe 改线性过滤 | `cargo check --workspace --all-targets` |
| **APP-3 meter 分段数学单源** | canvas 版 `channel_meter` 改消费 `meter_spans`；删内联推导 | meters.rs 单测 + gpui-test 截图 |
| **APP-4 `edit_mixer` 收敛** | `UiState::edit_mixer(id, FnOnce(&mut MixerState))`，四处（两份 mixer_edit、set_balance、SliderEdit 闭包）收敛 | gpui-test |
| **APP-5 托盘推子统一** | `widgets/fader.rs::fader` 加尺寸参数，托盘改用；刻度统一 FaderScale（托盘值域改归一化） | tray_mixer/tests.rs + 人工手感确认 |
| **APP-6 `node_of_kind` 查询** | `GraphSnapshot` 加按 kind 查询，~6 处扫描收敛 | gpui-test 全套 |
| **APP-7 tone 会话判定单源** | MeterBridge 把 topic 接线结果发布进快照（或 `is_tone_session`），UI/monitoring 改读 | tests/signal_visibility.rs |
| **APP-8 dB 格式化/步进收敛** | `db_text`/`dbfs_text`/`step_db` 三件套；同步快照断言里的 −∞/−inf 拼写 | window/tests.rs |
| **APP-9 + APP-13 i18n 补面** | service 层 65 处中文消息迁英文键 + zh-CN 目录（edit_hint/project_fault/validate_node_state 优先）；补 rename/Input/SourceMode 键 | graph/tests、managed/tests 文本断言同批更新 |
| **APP-11 state.rs 测试外迁** | 内联 614 行 mod tests → `ui/state/tests.rs` | 无行为变化 |
| **APP-10 i18n 台账** | 翻译完成后删 `i18n-keys.tsv`（或先去 file:line 列） | 无代码依赖 |

## P2 — meter/信号链路收敛（顺序执行）

依赖链：EN-1 →（决策 D-4）→ X-1/X-3。

1. **EN-1 删除队列版 LevelMeter 全链**（先行，独立成立）：
   - 删 `bind_meter`、`LevelMeter`/`MeterReader`（meter.rs ~380 行）、`PreparedReaders.meters`、`SignalHub.meters`、`ManagedMeterReader`、`EngineObserver::meter_reader`；
   - Meter 节点语义不变（PassThrough + 隐式 `SignalLevelMeter` 已按其 window_frames 发布），`meter_reader` 调用者迁 `subscribe_level`；
   - 验证：`cargo test -p moiren-engine --test host`；app 端 meters.rs 已在信号路径，无 UI 变化。
2. **X-1/X-3 双总线收敛**（路线见决策 D-4）：
   - 路线 A（保守）：节点 level topic 目录 + prune 抽成共享助手（engine 或 signal 侧），engine/app 各自调用；双总线保留作稳定性边界；在 meters.rs 或设计文档记录 3 拷贝/3 层轮询的量化路径。
   - 路线 B（彻底）：UI 直接订阅 engine 导出的 `ManagedLevelSubscription`，owner generation 变化时重订，删除 app 侧第二条总线与 `MeterBridge`——meter 通路从 3 拷贝 3 轮询降为 1 拷贝 1 泵 + UI 帧。
   - 验证门：`analysis_receives_real_audio_windows_after_ui_hide_without_generation_reset` 必须保持通过（图热切换时 UI 电平连续性语义）。
3. **X-2 pump 调度统一**（可选收尾）：moiren-signal 提供统一 demand-aware 调度类型，engine/app 替换。

## P3 — app legacy 驱动栈退役（APP-1）

分两个 PR，迁移先行、删除在后：

1. **PR-迁移**：`tests/service_recovery.rs`、`tests/signal_service.rs`、`settings_tests.rs` 从 `BackendDriver` 迁到 `ManagedDriver` + `managed/test_backend.rs`（已存在）；monitor 恢复/淡入 fallback 语义（recovery.rs、driver.rs:337-387）需等价覆盖后放行。
2. **PR-删除**：删 `BackendDriver`（driver.rs 832 行）、`service/parameters.rs`（421）、`service/windows.rs` 会话段（~450）、`backend.rs` 多余 prepare 变体；`WindowsBackend` 的 catalog 职责拆出注入 `ManagedDriver`，`BackendAdapter` 收缩为 catalog 契约。预计净删 ~1200-1500 行。
3. 连带：EN-2 若选裁剪（决策 D-1），app 侧 `service/signal_preparation.rs`（自建 bus + 自 compile 的 legacy 挂表路径，tone/monitor/session_graph 仍走它）在迁移完成后可删。
4. 验证门：`cargo test --locked -p moiren-app` 全绿（含 gpui-test 套件）。

## P4 — 双后端退役 W-K（WA-1 → WA-4 → 删除）

严格按 03 报告第 4 节七步执行，摘要：

1. **能力对齐**（4 缺口，切默认的阻塞项）：latency profile 映射（决策 D-2 选方案 A/B）；professional 捕获 QPC 前馈（或书面接受 occupancy-only）；虚拟端点 `ProfessionalStreamObservation` 补齐；每唤醒遥测补/弃（记录在案）。
2. **切默认路由**：`service/managed.rs` windows() 恒 professional，删 `settings != default` 分支。硬件冒烟清单：默认端点三 role、Shared 48k、VAD 全虚拟项目、进程环回。
3. **观察体系收敛**（WA-4）：以 professional 形状为准，app `model.rs`/`ui/state.rs` 双分支合一。
4. **中立层解耦**（若 P1 未做）：resample/servo 移出 professional/。
5. **删除 legacy 栈**：~7.5k 非测试行 / ~44 文件（清单见 03 §4.5）。crate 预计缩 ~25%。
6. **关闭 D5**：SetSend 决策改 resolved-wontfix。
7. **文档收尾**：AGENTS.md/negotiation.rs 引用的 remediation 计划补终态。
8. 连带收益：X-4（clock_bridge 线性插值分支随 legacy 删除）、WA-7（clock 家族文档）、D-2/D-5 文档项。

**验证门**：parity 集成测试（同图两栈 observation 对比）+ `tests/common/clock_sim.rs` 仿真 + 实机 soak（这是 D3 一贯前提，不可省）。

## P5 — 收尾卫生

- D-5 落地后的 stress 目录处置；`docs/superpowers/plans/` 55 份完成计划的归档（决策 D-6）。
- `.worktrees/` 入 .gitignore（X-10，一行）。
- 下次全量审计复查本文档各工单的完成度（本文档随后可归档进 plans/）。

---

## 决策项（需用户拍板）

| # | 决策 | 选项与默认建议 |
|---|---|---|
| D-1 | **EN-2 读出端去留**：Recorder WAV 全家（~470 行实现 + ~700 行测试）、`subscribe_spectrum`、`subscribe_compressor` 均无生产消费者，但 app 节点目录能创建这三类节点（Recorder 队列无人排空、dropped_frames 静默增长）。 | (a) 接线：产品规划里有录音面板/频谱画布/压缩表 → 全保留并列 app 欠账；(b) 裁剪：删 WAV worker 与 compressor per-node topic 准备，保 PassThrough 拓扑占位。**默认建议 (b)**，等 UI 排期再复活（git 里随时可找回）。 |
| D-2 | **latency profile 在 professional 的表达**（WA-1 缺口 1）。 | A：档位→2 的幂 buffer_frames（Balanced→512、Low→256），改 app 映射；B：放宽 professional 的 2 的幂校验接受 480/240。**默认建议 A**（不动协商校验契约）。 |
| D-3 | **Tone 统一幅度**（WA-2）：legacy 1.0 vs professional 0.1。 | 统一 **0.1**（防满幅削波、与 professional 现行为一致），PR 说明 legacy 路径可听变化；或保留差异并在两处加"幅度不同"的显式注释（不推荐，正是本轮要消除的分叉）。 |
| D-4 | **信号双总线收敛路线**（X-1）。 | A：目录助手下沉（保守，双总线保留）；B：UI 直订 engine 侧 `ManagedLevelSubscription` + generation 重订，删 `MeterBridge`（彻底，少 1 总线 1 拷贝 1 泵）。**建议 B**，但以"图热切换电平连续性"测试通过为放行条件；若 B 实施中发现 gpui 订阅生命周期纠缠超预期，回退 A。 |
| D-5 | **D2 stress 产物**：8 个 0 字节 stderr.log + ~481KB json/png 明细。 | stderr.log 直接删（P0 已列）；明细**建议保留 summary.json 删其余**（折中），或整目录删除。 |
| D-6 | **plans/ 归档策略**：55 份已完成计划。 | 整体移入 `docs/plans/archive/`（保留检索）或删除（git 历史可找回）。二选一即可，非紧急。 |
| D-7 | **W-K 时间表**：4 缺口对齐 + 切默认 + 实机 soak 的排期。 | 由用户按硬件可用性定；建议 P0-P3 先行，W-K 作为独立里程碑（改动面最大、需要 soak 窗口）。 |

## 量化预期（全部落地后）

- 删除/回收实现代码：legacy 双后端栈 ~7.5k + app legacy 驱动栈 ~1.5k + engine 读出端 ~1.1k（D-1 选 b）+ core 死 API ~0.3k + 零散（Sum/Bus、check_open、i18n 台账等）≈ **10.5k+ 行**（不含对应测试）。
- windows-audio crate 非测试体量 −25%；app service 层单驱动；engine 每图每块只测一次电平。
- 漂移高危点清零：参数域、焦点谓词、Tone 幅度、meter 分段、Gain/Pan 双传输判定、role→ERole 映射全部单源。
- 事实源唯一化：VAD 运行状态归 STATUS.md；i18n 键归 zh-CN.yml；依赖版本归 workspace 表。
