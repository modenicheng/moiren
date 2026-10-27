# moiren-core + moiren-signal 审计报告

> 子区域：`crates/moiren-core`（8 文件 / ~2000 行）、`crates/moiren-signal`（31 文件 / ~9675 行）。
> 方法：core 全部 8 文件逐行读完；signal 全部 18 个 src 文件逐个读完，tests/ 结构化浏览 + 关键竞态测试核对；跨 crate 用 grep 对照 core protocol / engine host / app service 的类型引用关系；git log -S 确认共享层落地时间。全程只读。

## 1. 执行摘要

- **最重要的结论：10-19 的 Latest/Stream 双管线问题当日已被 commit `172f8c5` 修复**——slot/epoch/retire 机制已抽为共享层（`detach_sender`/`retire_sender`/`reclaim_retired_and_clear_adopted`/单一 `Demand` epoch）。残余平行仅存在于 dispatcher `step()` 尾部与 `subscribe_*` 骨架，且大部分有语义反证（见 CS-1/CS-2）。
- **Stream 零生产流量仍然成立，且 "awaiting first routed consumer" 文档标注未落地**（全 crate 与 docs/ 无任何标注文本）——这是唯一明确的 STILL-OPEN 承诺缺口。
- **moiren-core 存在两个生产零消费者的 pub API**：`validate_connection`（零外部调用者）与 `GraphIndex`（仅 example + 自测），共约 320 行，是最可直接转化为工单的删减项。
- **core 协议无三处平行定义**：engine 与 app 全部直接 import `moiren_core::protocol` 类型，分层是单向翻译（app `ParameterTarget` → engine `resolve_all` → core `ParameterKey`），健康。
- 端点层（transport/latest.rs vs stream.rs）有 6 对结构平行的小方法，其中 `check_open` 是逐字重复，`allow(dead_code)` 标注已失真（生产可达方法被静默）。

## 2. 既往已知项核对表

| 项目 | 状态 | 证据 |
|---|---|---|
| Latest/Stream 双管线互相拷贝并发不变式（2026-10-19） | **FIXED**（当日修复） | commit `172f8c5`（2026-10-19 20:20）"converge latest/stream scaffolding into shared helpers"。共享层：`storage.rs:227-259`（`detach_sender`/`retire_sender`，泛型 over S）、`registry.rs:32-78`（`retired_slot_snapshot`/`next_sender_slot`/`reclaim_retired_and_clear_adopted`，两条 subscribe 路径共用）、`demand.rs`（单一 epoch 机制）、`storage.rs:191-196`（`StreamSender::eligible`）。竞态守护测试 `tests/registry.rs:731` 存在且被源码注释引用 |
| Stream 零生产流量，保留但文档标注 "awaiting first routed consumer"（2026-10-23 结论） | **STILL-OPEN（标注未落地）** | 流量核实：生产代码零调用——engine 中唯一 src 命中 `runtime/swap.rs:314` 位于 `#[cfg(test)] mod signal_tests`（swap.rs:56）；app 唯一命中 `service/signals.rs:453` 在 `#[cfg(test)]`（line 243）；其余为 signal 自测、`engine/tests/runtime/signals.rs`、example。文档核实：grep `awaiting / first consumer / routed consumer / no production` 于 `crates/moiren-signal/` 与 `docs/` 均无命中 |
| recorder 专用 rtrb 合理保留（不重提） | **维持**（有一条补充证据） | `engine/src/recorder.rs:8` 不变。补充：`engine/src/meter.rs:56` 注释明确其 rtrb "drained on demand, with no signal-bus participation"——同模式的第二处独立队列，均与 signal crate 内 `storage.rs:264-284` 的 pool 用途不同，不构成重复 |
| core 协议与 engine 控制面 / app service 三处平行定义同一语义 | **无此事（OK）** | engine：`host/parameters.rs:1-2`、`control/channel.rs:3`、`control/view.rs:3`、`control/schema.rs:1` 直接 import；app：`service/parameters.rs:4-5`、`service/model.rs:2` 直接 import。分层为 app `ParameterTarget`（逻辑目标）→ engine `resolve_all`（host/parameters.rs:293-342）→ core `ParameterKey`（物理键）单向翻译。app `GraphEdit`（service/graph.rs:76-115）全部委托 core 原语（service/graph/edit.rs:102-141 调 `graph.connect/disconnect/set_send_params/add_input_port`），无平行拓扑实现 |

## 3. 发现清单（按严重度）

### CS-1｜dispatcher 两个 `step()` 的 retired 回收前奏与投递尾部平行（残余，非回归）
- **级别**：P2　**类别**：同一状态机骨架两份
- **证据**：`crates/moiren-signal/src/dispatcher.rs:80-97`（Latest retired 前奏）vs `dispatcher.rs:173-199`（Stream retired 前奏）；`dispatcher.rs:120-147`（Latest 投递尾）vs `dispatcher.rs:219-249`（Stream 投递尾）。
- **详述**：重复在骨架与顺序："`retired_receivers & occupied` → charge(0) → `fanout.retired_slot` → `retire_sender` → recipients==0 收尾"与"trailing_zeros 选收件人 → lease.upgrade → eligibility → charge → 清位 → copy+publish/enqueue → `increment(delivered)`"。差异：Stream 有 `retire_one()` 分阶段排水与 `retired_last` 交替调度（175-200）；Latest 的 eligibility 谓词是内联四条件（127-129），Stream 已提取为 `StreamSender::eligible`（storage.rs:191-196，含 `== self.generation` 第五条件）。
- **建议**：(1) 给 `LatestSender` 加 `eligible(&self, meta, &DemandSnapshot) -> bool`（放 storage.rs `StreamSender::eligible` 旁，注释指明 Latest 用 `demand.generation` 而非 sender 侧 generation 字段的原因——latest 订阅在 read 侧二次过滤，subscriber.rs:234）；(2) retired 前奏提取为 `Fanout` 上的 helper（接收 "post-detach 收尾" 闭包）。到此为止，不合并整个 step。
- **不建议深合并的反证**：lossless（`dispatch: Option<S>` staging、full-drop 计量在 charge 之前判 `ready.is_full()`，226-231）vs latest-wins（值语义 `dispatch: S`、NoDemand 时仅推进 meta，109-111）的状态差异贯穿每一步；强行参数化单机会把两种语义编织进一层间接。
- **验证**：`src/dispatcher/tests.rs` 全部 15 测；`tests/registry.rs:307`（每付费步至多排水一个 owner）、`:615`（大 retired pool 不延迟同 fanout 其他收件人）。

### CS-2｜`subscribe_latest` / `subscribe_stream` 剩余骨架平行
- **级别**：P2　**类别**：同一准备序列两份
- **证据**：`crates/moiren-signal/src/registry.rs:250-313` vs `registry.rs:325-396`。
- **详述**：共同 10 步序列两份：`demand.check_add` → retired 快照 → 选槽 → `budget.check` → `try_reserve` → 二次 downcast → `SubscriptionLease::staged` → `demand.add` → `counted=true` → `reclaim_retired_and_clear_adopted` → install sender → `occupied |= bit`。真差异只有两段：Latest 的播种（289-296，copy_from+publish+seeded_sequence）与 Stream 的 fence/generation/retirement（360-373）。核心竞态不变式（"先 reserve 后 reclaim、先清 completion 位后安装 replacement"）在两个函数体里各隐含一份，靠注释互相提醒——这是漂移风险所在。
- **建议**：最小合并：提取 `fn reserve_subscription_bytes(&self, bytes) -> Result<Charge>`（check+try_reserve 对，259-278 与 337-357 重复的 20 行）；install+reclaim+occupy 收尾可写成一个按字段闭包的泛型 helper，但若闭包签名超过 4 个参数则放弃（收益/复杂度比）。保守版只做第一项 + 把顺序不变式集中写进 `reclaim_retired_and_clear_adopted` 的 doc（现已写一半，registry.rs:53-59）。
- **风险与验证**：保序是硬约束；守护测试 `tests/registry.rs:731`（retiring receiver 保持槽位直到 drop 完成）、`:771`、`:1379`（rejected subscription 零分配）。

### CS-3｜moiren-core 死 pub API：`validate_connection` 零外部调用者
- **级别**：P2　**类别**：dead code（导出无调用者）
- **证据**：`crates/moiren-core/src/graph.rs:355-363` 定义；全仓库唯一调用在自身测试 `graph/query.rs:240,249`。doc 自述 "Advisory non-RT preflight…for a client"，但 app 的连接路径 `service/graph/edit.rs:111-118` 直接调 `graph.connect` 并以 `map_err(|e| e.to_string())` 回显错误，从未 preflight。
- **详述**：为假想的 UI 预检保留的 API（repo-steward 反模式：无压力抽象）。它复用 `connection_nodes`（与 `connect` 共享校验，这点是好的），但"共享"不构成保留理由——`validate()` 已同样复用 `check_edge`。
- **建议**：删除 `validate_connection` + query.rs 测试中的 preflight 断言块（保留 connect 等价性断言改写为直连 connect）。`connection_nodes`/`check_edge` 保留（`connect` 与 `validate` 仍用）。
- **风险与验证**：无生产行为影响；`cargo test -p moiren-core`。
- **附带**：`edit.rs:7` `connect_to_new_bus_input` 仅被 2 个 example + 1 个 engine 测试使用，无生产调用者。example 是真实资产，暂列观察。

### CS-4｜`GraphIndex` 生产零消费者
- **级别**：P2　**类别**：dead code（导出无生产调用者）
- **证据**：`crates/moiren-core/src/graph/query.rs:9-104`（278 行含测试）。全仓库调用：query.rs 自测 + `windows-audio/examples/routing_probe.rs:546`。engine 编译器用 `graph.topological_order()`（compiler/plan.rs:250），app/engine 无任何 GraphIndex 引用。
- **详述**：为"客户端展示用稀疏邻接索引"设计，但没有客户端接它——UI 邻接展示走 app 侧 `GraphSnapshot` 自己的结构。与 CS-3 同源：预想消费者未出现。
- **建议**：倾向删除 query.rs（连同 `pub use query::GraphIndex`），routing_probe example 改用 `graph.edges()` 线性过滤；若认为 routing_probe 是长期核心 example，降级标注并在下次审计复核。
- **风险与验证**：无生产影响；`cargo check --workspace --all-targets` + example 重编译。

### CS-5｜端点层小规模重复与 `allow(dead_code)` 标注失真
- **级别**：P3　**类别**：卫生 + 真实 dead-code 信号被屏蔽
- **证据**：
  - `check_open` 逐字重复：`transport/latest.rs:48-54` vs `transport/stream.rs:52-58`；
  - cfg(test) `endpoint()` 构造器重复：`latest.rs:15-29` vs `stream.rs:22-35`；
  - `LatestEndpoint::publish`（latest.rs:57-58）与 `StreamEndpoint::publish`（stream.rs:133）带 `#[cfg_attr(not(test), allow(dead_code))]`，但它们生产可达——`types.rs:378/380` 经 `value_shape!` 宏的 `publish_access` 调用。`try_write`（latest.rs:132-133 / stream.rs:175-176）同批标注需逐一复核。这些标注会静默未来的真实 dead-code 警告；
  - `StreamEndpoint::publish_slice`（stream.rs:188-201）是 `SignalScope::publish_slice`（scope.rs:91-104）的 Stream-only 测试专用平行实现。
- **建议**：`check_open` 移到 `TopicState`（subscriber.rs，与 `demand.is_closed` 同居）；核对并摘除生产可达方法上的 `allow(dead_code)`；`publish_slice` 加测试 seam 声明或删除。
- **验证**：`cargo test -p moiren-signal`（transport 双 tests 模块覆盖）。

### CS-6｜身份计数器分配两份实现
- **级别**：P3　**类别**：同一算法两处
- **证据**：`host/mod.rs:29-35` `identity()`（AcqRel/Acquire，错误 `HostError::BindingExhausted`）vs `lifecycle.rs:122-128` `checked_identity()`（Relaxed，错误 `RegisterError::GenerationExhausted`）。lifecycle.rs:120-121 注释自认 "matching the `try_update` form used for host identities"。
- **建议**：可合一为 `fn next_id(counter: &AtomicU64, ord: Ordering) -> Result<u64, ()>`，两处包装。**反证**：两处 ordering 各有内存语义注释，合并需保留 ordering 参数，抽象净收益接近零。列为可选，不建议单独开工单。

### CS-7｜共享 `Fanout` 携带模式专属字段
- **级别**：P3　**类别**：trait/struct 边界不清（轻度）
- **证据**：`dispatcher.rs:11-18`：`active` 仅 LatestStorage 使用（71,77-145），`retired_last` 仅 StreamStorage 使用（175-200）；`recipients`/`retire_cursor` 双方共用。
- **建议**：把 `active` 移入 `LatestStorage`、`retired_last` 移入 `StreamStorage`，或在 `Fanout` doc 注明字段归属。可与 CS-1 顺手一起做。

### CS-8｜core 编辑路径每次 connect 全量重建拓扑（记录项，非重复）
- **级别**：P3　**类别**：性能 note
- **证据**：`graph.rs:365-388` `connection_nodes` → `would_cycle` → `topology()`（528-548，每次重建 BTreeMap + 邻接表）；`find_port` 线性扫全部节点端口（80-91）。测试注释自认超线性（tests/graph.rs:450-453）。app 侧 4096 节点上限（service/graph/edit.rs:64）下可控。
- **建议**：本轮不动。仅登记；若未来图编辑成为交互瓶颈，优先缓存 indices。

## 4. 明确不建议合并项（防过度抽象清单）

1. **Latest/Stream 传输核心**（storage.rs:96-161 两个 Storage、两个 endpoint/guard）：latest-wins 三缓冲 vs lossless 所有权移动池是真语义边界；RT 发布路径全单态（sealed trait 静态分发），抽象成统一机制必然引入模式分支或 dyn。10-19 已抽出能共享的全部。
2. **三套 per-API 错误枚举**（`RegisterError`/`SubscribeError`/`HostError`/`UnregisterError` 各含 `Closed`/`ForeignTopic` 变体）：每个 API 面的失败域不同，合成大枚举会放宽匹配面。变体重名是领域巧合。
3. **三层 block guard**（`BlockWriteGuard` / `StreamBlockWriteGuard` / `SignalBlockWriteGuard`）：enum 包装是零成本静态分发；两个底层 guard 的 abort 语义不同。改 trait object 会在 RT 路径引入动态分发。
4. **`LatestEndpoint` 的 latest 播种逻辑**（registry.rs:289-296）：是 latest-wins 语义本体（晚订阅者拿完整快照），不是 Stream 侧的缺失对称物。
5. **app `valid_gain_pan` vs core `SendParams::validate`**：gain 域不同（UI 0..=1 vs 传输层 ≥0 无上限），语义分层非重复；可在 app 侧加一行注释指明收紧原因。
6. **recorder / meter 各自的 rtrb**：维持既有结论；meter.rs:56 "drained on demand, with no signal-bus participation" 是支持保留的新注脚。
7. **cfg(test) 投递探针 `deliver_latest` / `deliver_next`**：私有传输层的简化镜像，测试 seam，保留。
8. **core `check_edge` 单边契约 + `GraphIndex::input_edge` 占用查询**：同一不变式的写侧与读侧，已是正确的单一权威结构。

## 5. 统计信号

- **moiren-core**：8 文件 / 2000 行（src 1485 + tests 515）。死 pub API 2 个（~320 行）。
- **moiren-signal**：31 文件 / 9675 行（src 含内嵌测试约 6930，tests/ 2648）。最大 `host/mod.rs` 802（其中约 295 行为内嵌测试）；`tests/registry.rs` 1473 行 / 44 测试函数。
- **重复模式计数**：端点平行方法对 6 对；dispatcher step 平行段 2 处；subscribe 骨架 2 处；身份分配 2 处；逐字重复函数 1 处；生产可达却标 `allow(dead_code)` 的方法 ≥4 个。
- **可转工单**：CS-1/CS-2（signal 收敛第二批）、CS-3/CS-4（core 删减 ~320 行）、CS-5（端点卫生）；CS-6/CS-7 顺手项；CS-8 仅登记。
