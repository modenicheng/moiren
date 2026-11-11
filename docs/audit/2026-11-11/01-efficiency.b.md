# 01 效率与渲染开销 — 初审 B（引擎/音频/控制面运行时）

- 基线: main @ e2ed3d6a, 2026-11-11（工作区仅新增 audit 目录，无代码改动）
- 范围: moiren-engine（RT 执行、supervisor/host、meter/spectrum tap）、moiren-signal（host/slot 发布路径、Bus pump、dispatcher、Latest/Stream transport）、moiren-windows-audio（WASAPI/ASIO 数据面、engine_backend owner/Pump、bridge、VAD pcm_worker/provider、observations）、moiren-app/src/service/**（服务主循环、mailbox、managed driver 九阶段、MeterBridge、signals 泵、exports runtime）
- 不含: UI 渲染侧（另一代理）；docs/audit/2026-11-11/ 其他报告未读。

## 方法

实际检索式（rg）与抽查文件清单：

- 检索式: `thread::sleep|park_timeout|recv_timeout|wait_timeout|spin`（全范围）；`alloc|Vec::with|format!|Box::new`（signal host/scope/slot、engine runtime/swap）；`fn driver_latency|fn stats|fn check`（wasapi/asio/native）；`graph.snapshot\(\)|statuses|reconcile`（service 层）；`has_business_demand|pump_if_due|wait_duration`；`publish_snapshot|poll_interval`；`POLL_INTERVAL`（VAD）；`fn export_observations|fn poll`（provider）；`ui_window_frames`；`fn render`（runtime.rs）。
- 通读: service/mod.rs（572 行全文）、mailbox.rs、signals.rs、managed/driver.rs、managed/lifecycle.rs、managed/meters.rs、exports/reconcile.rs、exports/runtime.rs、jobs.rs、windows.rs、model.rs、graph.rs（snapshot/confirmed 段）。
- 通读: signal registry.rs 全文、dispatcher.rs（step 路径）、host/mod.rs 全文、meter/polling.rs、meter/signal.rs 全文、spectrum.rs 全文、host/signals.rs 全文、host/supervisor.rs、supervisor/snapshot.rs、runtime.rs（render 段）、host.rs（配置/observer）。
- 通读: windows-audio bridge.rs 全文、engine_backend/owner.rs 全文、asio/callback.rs 全文、asio/stream.rs（stats/latency/start-stop 段）、wasapi.rs（NativeStream 数据面 763-921 + EndpointNotifications）、engine_backend/observations.rs、vad/provider.rs（retire/observations）、vad/provider/pcm_worker.rs（wait 路径）、resample.rs（头部）、windows.rs、meter/polling.rs。
- 对照历史: docs/audit/2026-10-27/02-engine.md、05-cross-crate-workspace.md（X-1/X-2/X-3）、04-app.md；docs/audit/2026-11-09/02-app-service.md、04-engine-core.md、05-windows-audio.md、06-signal-plugins.md。任务简报提及的 2026-10-29 审计不在仓库中，以其转述为准。

## 发现

### B-1 服务循环每轮（~1ms 有 demand / ~10ms 会话运行中）全量构建 + 全量相等比较 AppSnapshot [P2] [KNOWN-OPEN]
- 位置: `crates/moiren-app/src/service/mod.rs:453-511`（主循环）、`mod.rs:356-390`（snapshot(&c) 每轮构建）、`mod.rs:489`（`mailbox.publish(snapshot(&c))` 无条件每轮执行）、`mailbox.rs:250-274`（publish 全量 `==` dedup）、`service/graph.rs:658-660`（`ServiceGraph::snapshot()` = `self.snapshot.clone()` 深拷贝）、`service/model.rs:189-224`（AppSnapshot::eq 中 `self.graph == other.graph` 按值比较 LogicalGraph）、`model.rs:161`（graph 为 `Arc<GraphSnapshot>`，但内容每轮重建）。
- 证据: GraphSnapshot 内含 `graph: LogicalGraph`（`crates/moiren-core/src/graph.rs:42-46`，`nodes: Vec<Node>`/`edges: Vec<Edge>` 按值）与 `nodes: Vec<GraphNodeState>`（`service/graph.rs:60-79`；GraphNodeState 含 `label: String`、`source: Option<InputSelection>` 内含 endpoint_id String，`graph.rs:20-35`）。主循环在有 meter demand 时以 1ms 节奏唤醒（`service/signals.rs:112` next_poll = now+1ms；`mod.rs:496-508` signal_wait ≤1ms），无 demand 但 owner 存在时 10ms。每轮：① `snapshot(&c)` 深拷贝整个 GraphSnapshot + `exports.statuses/reconcile`（O(exports×observations)，`exports/reconcile.rs:93-198`）+ `audio_settings.clone()`；② `mailbox.publish` 先构建后做全字段 PartialEq（含 LogicalGraph 逐节点/边比较）。model.rs:187-188 注释自认 "The graph Arc is rebuilt per snapshot, so it compares by value"。
- 影响: 会话运行期间服务线程每秒数百至一千次 O(graph) 深拷贝 + 全量比较 + 一批短命分配（String/Vec），绝大多数轮次结果"无变化"被丢弃。控制线程成本随图规模线性增长，纯浪费（不产生任何用户可见收益）。
- 建议方向: 用变更令牌替代"构建再比较"——ServiceGraph 已有 candidate/active/pending revision、faults 等天然令牌；catalog/exports/control 各维护脏标记或世代号；仅当任一令牌变化才构建 AppSnapshot。dedup 逻辑保留给"令牌变了但内容等价"的罕见场景（如同秒时钟量化已处理 elapsed）。
- 工作量: M
- KNOWN 判据: 任务简报转述 2026-10-29 审计"服务循环 1ms 全量快照"；可追溯的仓库内依据为 2026-10-27/05-cross-crate-workspace.md X-3（meter 链路 3 层轮询量化）与 X-1（双总线 pump），本轮核实快照构建/比较本身仍在原样执行，标 KNOWN-OPEN。

### B-2 driver poll 只读访问点每次深拷贝整个 GraphSnapshot；update_vad_catalog 每 poll 双调用 [P2] [NEW]
- 位置: `crates/moiren-app/src/service/managed/driver.rs:548`（`c.graph.snapshot().pending_revision` —— 运行中每 poll 一次深拷贝，仅为读一个 Option<u64>）、`driver.rs:610`、`driver.rs:618`（poll_edits 事件路径同型）、`driver.rs:209`（update_vad_catalog 内 `context.exports.statuses(&context.graph.snapshot())`）、`driver.rs:156` 与 `driver.rs:182`（update_vad_catalog 在同一次 poll 里被调用两次）、`driver.rs:379`（poll_exports 每轮 `c.graph.snapshot()`）。
- 证据: `ServiceGraph::snapshot()` 是深拷贝（见 B-1）。`driver.rs:154-183` 的 poll 序列中：update_vad_catalog（×2）+ poll_exports（×1）+ poll_execution 运行中分支（×1，line 548）= 每 poll ≥4 次 GraphSnapshot 深拷贝 + 2 次 statuses 构建 + 2 次 BTreeSet 构建 + 2 次 catalog `Arc` clone 后比较（`driver.rs:215-233`，`owned != context.catalog.vad_endpoints` 才换 Arc）。全部发生在 1ms（有 demand）或 10ms（运行中）节奏上。
- 影响: 与 B-1 同源但独立可修：即使发布侧改成令牌制，这些只读访问点仍每轮付出深拷贝。运行会话中每秒 ≥400 次（1ms 节奏）至 ≥40 次（10ms 节奏）纯读型深拷贝。
- 建议方向: ServiceGraph 暴露 `pending_revision()/candidate_revision()/active_revision()` 等轻量访问器；update_vad_catalog 合并为每 poll 一次（两处调用相隔十几行，第二次仅在 poll_exports 可能改变 exports 观察后有意义——可在 poll_exports 完成后单次调用或传入已有引用）；update_vad_catalog 接收 `&GraphSnapshot` 引用而非自建。
- 工作量: S

### B-3 engine supervisor 每轮重建 HostSnapshot 且 has_business_demand 每轮 3 次全目录扫描 [P3] [NEW]
- 位置: `crates/moiren-engine/src/host/supervisor/snapshot.rs:4-80`（每轮无条件 `snapshot.state = HostSnapshot{...}` 重建，含 `self.fault.clone()`、`reason.clone()`）、`supervisor.rs:238`（每轮 `self.shared.signals.pump()`）、`supervisor.rs:246-247`（`set_demand(has_demand())` + `poll_interval()` 内部再 `has_demand()`）、`host/signals.rs:525-541`（poll_interval/has_demand 各自 lock + 扫描）、`signal/registry.rs:176-181`（has_business_demand 遍历全部 256 槽）、`host.rs:57-61`（engine BusConfig max_topics=256，poll_interval 默认 5ms）。
- 证据: supervisor 每轮（demand 时 1ms、无 demand 5ms，host 存在期间永不静默）执行：`has_business_demand()` 扫描 ×2（set_demand 与 poll_interval 各一次）+ publish_snapshot 重建 HostSnapshot（Diagnostic/UpdateState clone，无变化时也重建——只有 desired/active 两个 Arc 有复用，state 值字段没有）。app 侧对称问题小一号：`service/signals.rs:96-120` pump_if_due 与 wait_duration 各调一次 `has_business_demand()`（131 槽）+ 两次 `Instant::now()`。
- 影响: 会话期间 host-control 线程 200-1000 次/秒唤醒，每轮若干小分配（fault/update clone）+ 最多 ~770 次/秒槽位原子扫描。单项都是微秒级，但都是常驻热路径上的可免成本；state 无 Arc 稳定性也让读者（EngineObserver::read，`host.rs:289-295` 每次 clone 整个 HostSnapshot 含 String）缓存失效更频繁。
- 建议方向: publish_snapshot 先 diff 再写（fault/update/timeline 不变时跳过重建，保持 state 稳定）；supervisor 每轮只调一次 has_demand 并把结果传给 set_demand 与 poll_interval（poll_interval 加带结果的重载）；app 侧 pump_if_due 返回下一截止时间，主循环直接用它，去掉 wait_duration 的第二次扫描。
- 工作量: S

### B-4 Windows 导出观察空批循环：适配器存在时每 20ms 一次原生查询 + 任务往返，即使没有任何导出 [P3] [NEW]
- 位置: `crates/moiren-app/src/service/exports/runtime.rs:272-326`（`work()` 无"计划为空"短路——仅以 adapter/in_flight/shutdown_complete/next_poll 门控，空 actions 也返回 Some(Work)）、`runtime.rs:328-351`（submit 后 next_poll=+20ms）、`runtime.rs:457-468`（WindowsAdapter::execute 空批仍调 `export_observations()` + `export_name_observations()`，后者需连取 3 把锁 `vad/provider.rs:129-146`）、`driver.rs:425-430`（poll_exports 每轮提交 work，不检查 actions 是否为空）、`service/managed.rs:254-257`（Windows 上 adapter 恒为 Some）。
- 证据: 运行会话或任何活动期间，服务线程 10ms 唤醒一次 → work() 每 20ms 产出一个空批 → prepare worker 执行（provider state lock + Vec 分配 + 名称观察三锁）→ 完成后 unpark 服务线程 → complete() 清理重建。全程无导出时该循环唯一产出是 `native_empty=true` 的排水证据（首轮即达成），此后每秒 ~50 次任务派发/完成往返持续到会话结束。
- 影响: 常驻后台搅动：50 次/秒 worker 任务 + 100 次/秒 mpsc/唤醒 + 每次若干分配。单次成本低（µs 级），但它是"UI 停了服务侧仍在空转"的一个具体残留点，且与 PR#37/40 的空闲静默目标相悖（仅限会话活动窗口内）。
- 建议方向: work() 增加"零期望导出 && 零在册观察 && 排水证据已建立（native_empty）"时返回 None 的短路；或把空批心跳降到 1s 级。注意保留首轮建立 native_empty 的路径与 shutdown 语义。
- 工作量: S

### B-5 RT 边界逐帧环操作与循环内重复求值 [P3] [NEW]
- 位置: `crates/moiren-windows-audio/src/bridge.rs:73-76`（RingSink::write 逐帧 `tx.push` 且 `input.channel(0)/channel(1)` 在循环内每帧重取）、`bridge.rs:234-267`（RingSource::read 逐帧 `output.channel_mut(0)/channel_mut(1)` 循环内求值）、`bridge.rs:356-391`（ProcessSource 同型）、`engine_backend/owner.rs:494-497`（Pump capture 逐帧 `tx.push`）、`asio/callback.rs:249-266, 273-290`（逐帧 `input.push`/`output.pop` + 逐通道 ptr::copy_nonoverlapping）、`engine_backend.rs` 下 Pump 渲染侧 `refill` 逐帧 pop/push（`bridge.rs:277-293`）。
- 证据: rtrb 提供 `write_chunk/read_chunk` 批量 API（仓库内 `bridge.rs:207` discard_backlog 已用 `read_chunk`），但生产/消费热路径全部走逐帧 push/pop；`AudioBlock::channel()`（`moiren-engine/src/buffer/view.rs:240-243, 258-267`）含 assert 与 stride 运算，在逐帧循环内重复执行。整链 capture→graph 与 graph→render 各 ~3-4 次拷贝（native buffer→解码暂存→环→图缓冲），均为值拷贝无分配，RT 分配纪律有测试兜底。
- 影响: 每帧多次多余函数调用/边界检查，48kHz 双通道约每秒 10 万帧 × 双向。绝对量小（无分配、无锁），属低风险改善而非瓶颈。
- 建议方向: 循环外提 channel(channel) 切片；rtrb 逐帧段改 chunk API（保持容量判定/丢弃语义不变）。不要动 ASIO callback 的逐帧 ptr::copy 结构——它与驱动双缓冲语义绑定，改动需重新过 rt-check。
- 工作量: S-M

### B-6 meter 链路常驻成本：240Hz×节点×3 拷贝×双 1ms 泵 + timeBeginPeriod(1) 系统级持有 [P3] [KNOWN-OPEN]
- 位置: `crates/moiren-engine/src/meter.rs:18-20`（ui_window_frames = sr/240 → 240 窗口/秒/端口）、`meter/signal.rs:144-157`（RT 每 200 帧发布一次 LatestBlock）、`host/supervisor.rs:238,246-247` + `host/signals.rs:514-534`（engine 总线 1ms 泵）、`service/managed/meters.rs:117-148`（MeterBridge 1ms 轮询、逐窗口再拷贝到 app 总线）、`service/signals.rs:96-114`（app 总线 1ms 泵）、`meter/polling.rs:14-28`（demand 期间 timeBeginPeriod(1) 全进程生效）、`service/signals.rs:19-24`（LEVEL_STREAM_DEPTH=16 ≈ 67ms 背压余量）。
- 证据: 表计可见时链路为：RT tap（240/s/端口，demand 门控，零订阅零成本——`meter/signal.rs` 测试锁定）→ engine supervisor 泵拷贝 1 → MeterBridge 序列去重后拷贝 2 → app 总线泵拷贝 3 → UI 订阅 FIFO。两条总线各自 1ms 泵 + 1ms 唤醒节拍（engine supervisor 与 app service），timeBeginPeriod(1) 在任一总线有 demand 期间持有。数据率实际只有 240 窗/秒/槽，1ms 泵对 16 槽 FIFO（67ms 余量）与 Latest 覆盖写都有数倍裕量。
- 影响: 表计可见时两条控制线程各 ~1000 唤醒/秒 + 全进程 1ms 定时器精度（笔记本功耗相关）。绝对 CPU 占用小（每轮 µs 级扫描+拷贝），但这是当前"空闲不静默"的最大单一来源。
- 建议方向: 泵周期与数据率对齐——level/spectrum 显示需求 60fps，4ms 泵周期即可（FIFO 16 槽覆盖 67ms，Latest 天然取最新），timeBeginPeriod 可随 4ms 周期不再必需或仅按需；把 1ms 常量改为随订阅类型/深度推导。若做 B-1 的令牌制，本项与 B-1 可同批评估。
- KNOWN 判据: 2026-10-27/05-cross-crate-workspace.md X-2/X-3 已量化该链路（3 拷贝 3 层轮询，"低优先"建议统一泵调度）；2026-11-09/06-signal-plugins.md 第 9 条已把双总线判定为"已解决的分层中继形态"（职责不同：engine=RT 观测、app=UI 流）。本条只保留"泵周期 vs 数据率"的运行时成本维度，标 KNOWN-OPEN。

### B-7 引擎 supervisor publish_snapshot 的 state 无 Arc 稳定性，读者每次全量 clone HostSnapshot [P3] [NEW]
- 位置: `crates/moiren-engine/src/host/supervisor/snapshot.rs:55-79`（每轮重建 state 值字段）、`host.rs:289-303`（`EngineObserver::read()/snapshot()` 每次 lock + clone 整个 ProjectSnapshot/HostSnapshot，含 fault/update 的 String）、消费方 `service/managed/driver.rs:170`（每 poll `owner.observer.read()`）、`meters.rs:89`（MeterBridge::poll 每轮再 read 一次）。
- 证据: desired/active 两字段有 Arc 复用（snapshot.rs:44-54），state 是值字段每次覆盖；服务线程每 poll 读 2 次（driver.poll 一处 + MeterBridge 一处），每次 clone 均分配 fault/update 内的 String（多数轮次为 None/Idle，分配为零；有 fault 时每读一次分配一次）。
- 影响: 小；与 B-3 同根（state 不稳定）。修 B-3 的 diff-before-write 后此项自然消解大半。
- 建议方向: 并入 B-3；或把 state 包成 `Arc<HostSnapshot>` 配 generation 令牌，读者按令牌取 Arc。
- 工作量: S（并入 B-3）/ M（独立做 Arc 化）

## KNOWN-FIXED 登记（一笔带过）

- PR#96 频谱 FFT 挪出 RT：`spectrum.rs:1-5` 模块文档与实现一致，RT 侧只维护历史环 + 拷贝窗口，FFT 在服务线程 MeterBridge（`meters.rs:149-189`），Latest 订阅 read_latest 天然背压（落后即跳窗，无积压）。
- 信号切换 epoch 泵阻塞 5c88800：`moiren-signal/src/dispatcher.rs:195-230` StreamStorage::step 退休槽分步排水（每步至多一个 owner），不阻塞泵。
- VAD park-probe F1 755831f：`vad/provider/pcm_worker.rs:390-443` 事件驱动 wait_activity（首个 ticket），无固定 10ms 空转；provider 的 10ms sleep 仅存在于生命周期收尾路径（retire_all 等瞬态操作）。
- PR#165 发布唤醒泵：`mailbox.rs:28-71` PublishSignal generation + waker take（服务线程零分配唤醒），已确认在位。
- PR#37/40 静音停帧与唤醒治理：UI 侧之外，服务线程全空闲（无 owner、无 demand、全排水）时真 `thread::park()` 无超时（`mod.rs:509-511`）；demand 驱动的 MeterPollingTimer 按需取放 timeBeginPeriod（`meter/polling.rs:14-28`）。残留空转点即 B-4（导出空批循环）与 B-6（表计 demand 期 1ms 节拍）。
- PR#26 minimal-io-latency 系列：协商原语单源于 negotiation.rs，wasapi.rs 为编排层（2026-11-09 WA-02 已判），本轮未见效率回归。

## 正面登记（明确不要动的）

1. RT 零分配纪律是机械强制的：moiren-signal `test_alloc.rs` 计数分配器覆盖全部发布路径；engine `tests/runtime/allocation.rs` TLS 分配器覆盖 render/热交换；windows-audio rt-check 影子测试覆盖 ASIO 回调与 recovery host。本轮抽查（host/scope/slot、runtime.rs render 段、bridge.rs、callback.rs）未发现任何绕过点。
2. ASIO 硬件回调（`asio/callback.rs`）：无驱动调用、无分配、无锁、无转换；SetEvent 直调 kernel32 避开 windows-rs Result 包装的潜在分配（callback.rs:20-25 注释明确）。这是全仓库 RT 纪律最好的文件。
3. WASAPI 数据面（`wasapi.rs:763-921`）：notifications.check 是单次原子读；无每唤醒延迟/位置查询（driver_latency 为 trait 默认 None，ASIO 侧为缓存值 `asio/stream.rs:433-439`）；capture/render 暂存全部预分配。GetCurrentPadding/GetNextPacketSize 为协议必需，频率已是每周期一次。
4. demand 门控贯穿 RT tap：SignalLevelMeter/SpectrumAnalyzer 在零订阅时零采样转换（`meter/signal.rs:285-306` 测试锁定 zero conversions）；发布窗口的 memcpy 也只在有 demand 时发生。
5. Latest/Stream 使用各得其所：RT→控制用 LatestBlock（覆盖写、只取最新），控制→UI 用 Stream FIFO（LEVEL_DEPTH 16 保 240Hz 窗口的峰值不丢，`signals.rs:16-19` 注释论证了为什么不用 superseding）——方向选择正确，勿"统一"成一种。
6. 目录/预算设计：Bus 目录固定槽位、预算原子记账、退休槽旋转回收（registry.rs/dispatcher.rs），无运行期增长路径。
7. Catalog/ASIO 探测按需一次性（用户 RefreshCatalog / 启动自动化），无周期性设备枚举或 COM 轮询（`service/windows.rs`、`jobs.rs` CatalogRefresher 单飞）。
8. mailbox 发布侧 dedup + Arc 稳定 + waker take（`mailbox.rs:250-274`）与 elapsed 秒级量化（`mod.rs:385-388` 注释明确动机）——发布去重机制本身是对的，问题只在 B-1 的"每轮构建再比较"。
9. RingSource 的跟随伺服 + 前馈时钟（`bridge.rs:146-231`）依赖等比 SRC 的连续修正能力，即使 native_rate==engine_rate 也不能旁路重采样器（ppm 修正即其职责）。
10. 服务线程 park/wake 拓扑：请求 unpark（AppHandle）、完成 unpark（CompletionPort）、信号泵 deadline park_timeout——无独立 timer 线程，唤醒源已合并。

## 明确不建议（防过度工程）

1. 不要合并双总线或消除 MeterBridge：2026-11-09 已判定分层中继为已解决形态（engine 总线 RT 观测 / app 总线 UI 流职责分离），合并会重新耦合 RT 观测生命周期与 UI 订阅动态。可做的只有泵周期参数对齐（B-6）。
2. 不要给 AppSnapshot 相等比较加哈希/校验和：B-1 的正解是令牌短路，哈希有碰撞面且仍要遍历。
3. 不要为 has_business_demand 建缓存或事件通知：131/256 槽原子扫描是微秒级，缓存引入失效协议反而更贵。只做"每轮调一次"的合并（B-3）。
4. 不要给 WASAPI shared render 的 GetCurrentPadding 做预测/缓存：它是 WASAPI 渲染仲裁的唯一权威，当前已是每周期一次，无冗余。
5. 不要把 ASIO callback 的逐帧双缓冲拷贝改成批量或 unsafe 重写（B-5 中明确豁免）：它与驱动 buffer-switch 语义和 rt-check 分配断言绑定，收益配不上风险。
6. 不要为导出观察加事件/回调式通知替代 20ms 轮询：PDO 状态没有可靠的事件源，B-4 的正解是空批短路而非机制替换。
7. 不要为 engine supervisor 引入按需睡眠（condvar 事件化）：5ms 空闲拍是准备/退休/命令排水的保底节奏，事件化会重引入 PR#165 之前的唤醒正确性难题。

## Top5

1. **B-1 服务循环每轮全量快照构建+全量比较** [P2, KNOWN-OPEN] —— 会话期间服务线程 100-1000 次/秒的 O(graph) 深拷贝与比较，纯浪费；令牌制发布是正解。
2. **B-2 driver poll 只读点深拷贝 + update_vad_catalog 双调用** [P2, NEW] —— 每 poll ≥4 次多余 GraphSnapshot 深拷贝，与 B-1 独立、改动量最小（S）、收益直接。
3. **B-6 meter 链路双 1ms 泵 + timeBeginPeriod(1) 系统级持有** [P3, KNOWN-OPEN] —— 当前"空闲不静默"的最大单一来源，泵周期对齐数据率（4ms 级）即可显著降耗。
4. **B-3 supervisor 每轮重建 HostSnapshot + 3×全目录扫描** [P3, NEW] —— host 存在期间 200-1000 唤醒/秒的常驻可免成本，diff-before-write + 单次 demand 查询。
5. **B-4 导出观察空批 20ms 循环** [P3, NEW] —— Windows 会话期间 50 次/秒无信息任务往返，空批短路一行级修复。
