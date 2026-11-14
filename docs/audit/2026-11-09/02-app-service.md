# 2026-11-09 架构审计 — moiren-app 服务层（非 UI）

基线: main @ 27aa57d7（工作区干净）
范围: `crates/moiren-app/src/` 下除 `ui/` 外全部 — `service/`（19 文件 ≈13.6k 行）、`project.rs + project/`、`i18n.rs`、`i18n_guards.rs`、`console.rs`、`logging.rs`、`timeline.rs`、`main.rs`、`lib.rs`。
性质: 只读架构/组织审计，不做 bug 狩猎（2026-11-03 已覆盖 autosave/project 语义 bug AS-1..AS-13）。

## 执行摘要（≤10 条要点）

1. service 层总体架构健康：单一服务线程独占 `ServiceContext`，UI 经 `AppHandle`（请求队列 + Arc 快照 ptr_eq 变化检测 + 订阅 ticket）单向消费，边界清晰、无共享可变状态泄漏。
2. 最大结构性债务是**错误链路三方逐字同步**（APP-1）：service 层错误主体仍是裸英文 String（即 i18n key），需与 `DYNAMIC_KEY_EXCEPTIONS`（116 条）和 `locales/zh-CN.yml` 三处逐字一致；`edit_hint` 还用子串匹配错误句配 hint，句子改字 hint 静默丢失。这是 i18n 类型化战役（Wave 5 后）尚未触及的主体。
3. `project ↔ service` 双向依赖：`project/startup.rs::into_context` 在持久化层组装 `ServiceContext`（运行时对象），职责放错边（APP-2）；一次小移动可解开。
4. `service/mod.rs`（561 行）是「声明 + 门面 + 核心实现」三合一（APP-3），与 `project.rs`（25 行纯声明）风格不一致；ServiceContext/ServiceRuntime/snapshot 可拆平级文件。
5. `.rs + 同名目录`模块风格在 graph/managed/exports/project 四处**使用一致**（疑点排除：无混用问题）；backend.rs、core.rs 等小散件无目录，属正常。
6. God 模块逐一判定：`graph.rs` 1005 行、`managed/driver.rs` 824 行均为「内聚大」，不建议按行数拆；仅 `graph.rs::edit_parameter_values`（编辑→引擎参数映射）是无共享状态的纯函数块，可顺手下沉（APP-8）。
7. 测试摆放三种风格并存（内联 mod tests / 子目录 tests.rs / 顶层 xxx_tests.rs），xxx_tests.rs 的技术理由（cfg(test) surface 须同 crate）存在但未成文（APP-6）；managed/tests.rs 的 `Rig` harness 与 test_backend.rs 复用良好，graph/tests 与 exports/runtime/tests 各自自持。
8. `timeline.rs` 是生产零调用的测试孤儿模块，唯一消费者 `service/graph/tests.rs:688`（APP-7）。
9. i18n 守卫（i18n_guards.rs 1084 行）设计质量高（自检、防失明下限、dormant 提示、rustfmt 免疫匹配），维护成本集中在 116 条动态 key 例外的手工同步——其消减路径正是 APP-1 的类型化。
10. 错误处理分层形态基本合理（入口 anyhow / 边界 thiserror / 领域内 String-即-key），但 String-即-key 依赖守卫兜底，是 APP-1 的根因而非独立问题。

## 发现清单

| ID | 级别 | 标题 | 位置 | 证据摘要 | 建议 + 权衡 |
|---|---|---|---|---|---|
| APP-1 | P2 | 错误「英文句子即 key」三方逐字同步，edit_hint 子串匹配脆弱 | `service/graph.rs:562-576`（edit_hint contains 匹配）、`service/graph.rs:622-656`（validate_node_state 全英文句子）、`service/graph/edit.rs`（同）、`service/managed/driver.rs:777-824`（project_fault）、`service/mod.rs:338-341`（硬编码句）；守卫侧 `i18n_guards.rs:811-931`（DYNAMIC_KEY_EXCEPTIONS 116 条）；UI 侧 `ui/shell/fault_popover.rs:72`、`ui/pages/graph/graph_canvas_view/nodes.rs:889`（`t!(fault.cause.as_str())`） | 同一验证错误句子必须三处逐字一致：产生点（graph/edit/driver）、DYNAMIC_KEY_EXCEPTIONS、zh-CN.yml。`edit_hint` 用 `error.contains("not bound to an input source")` 等子串配 hint——措辞微调即静默丢 hint，无任何测试/守卫兜底此耦合。i18n 类型化 Wave 5 已合 ProjectMessage/MeterFault/UiFailure，但 service 层最大错误源（graph 验证 ~40 句 + project_fault 12 句 + 零散 ~10 句）未动 | 沿 MeterFault 模式把 GraphFault 的 cause/hint 类型化为 enum + 参数，`GraphFault::cause_key()/hint_key()` 产出稳定 key；每合入一批即从 DYNAMIC_KEY_EXCEPTIONS 删除对应条目（守卫的 stale 检查会强制）。权衡：迁移面大（~60-70 句、UI 端 2 个渲染点 + fault equality 语义），建议按文件分波（graph.rs 验证句先行，收益最大且与 edit_hint 同点消亡）；project_fault 的 12 句是第二波 |
| APP-2 | P2 | project → service 反向依赖：持久化层组装运行时上下文 | `project/startup.rs:38-54`（`into_context` 构造 `crate::service::ServiceContext`，内联 set_authorizations + load_project 调用序列）、`project/model.rs:6`（use crate::service::{...}）；反向 `service/mod.rs:44,57`（ServiceStartError::Persistence、StartupDocuments::load） | 生产调用者只有 service/mod.rs:60/419；但 project/tests.rs 与 v2_tests.rs 在持久化层测组装行为——测试放错边是症状。概念上「可移植项目持久化」不应知道 ServiceContext 的构造细节（授权装载顺序、graph 替换语义） | 把 `into_context` 逻辑移为 `service` 侧自由函数（如 `service::context_from(startup: StartupDocuments)`，紧邻 start_service），`StartupDocuments` 保留纯数据；3 个测试文件跟随移动。权衡：pub API 表面不变（StartupDocuments 字段仍 pub），~40 行移动 + 测试归属调整；收益是依赖方向单一化（service → project），project 可独立理解为纯文档层 |
| APP-3 | P2 | service/mod.rs 三合一（声明+门面+核心实现），与 project.rs 风格分裂 | `service/mod.rs:41-77`（ServiceStartError+start_service）、`94-111`（RoundBudget）、`112-283`（ServiceContext+DeferredPermit+dispatch）、`285-344`（ServiceDriver+CoreDriver）、`345-379`（snapshot）、`380-517`（ServiceRuntime 含服务主循环）、`519-561`（内联测试）；对照 `project.rs`（25 行纯声明+re-export） | mod.rs 同时是模块根、公共门面、和服务层最核心编排（线程循环 + 状态聚合）的实现文件。`.rs+目录`模式要求 graph.rs/managed.rs/exports.rs 都是「类型+构造」薄文件（265-1005 行），而 mod.rs 自己 561 行实现，未遵循自家模式 | mod.rs 只留 mod 声明、re-export、start_service；ServiceContext/DeferredPermit/RoundBudget → `context.rs`，ServiceRuntime+snapshot+CoreDriver → `runtime.rs`（与 core.rs/backend.rs 平级）。权衡：纯移动无 API 变化（最终仍 `pub use`）；代价是两处 git blame 断层；「先读 mod.rs 就能读全服务」的现状优点会打折——可折中只拆 ServiceRuntime 主循环（约 180 行），ServiceContext 留 mod.rs |
| APP-4 | P3 | 8 个 glob re-export 抹平 service:: 命名空间，engine 类型无痕转手 | `service/mod.rs:24-39,78-81`：`pub use backend::*; exports::*; core::*; jobs::*; mailbox::*; model::*; moiren_engine::audio_config::*` 等 | 从 `use crate::service::AudioSettings` 看不出 AudioSettings 是 moiren_engine 的跨 crate 类型（mod.rs:34 转手）；符号来源 greppability 下降（搜定义两跳）。门面对 UI 消费（10+ 文件单一 use 路径）确有便利 | 保留门面但把 `moiren_engine::audio_config::*` 改为显式名列表（AudioSettings、AudioBackend 等实际使用的 ~6 个）+ 一行注释说明转手来源；其余 glob 可维持。权衡：显式列表需随 engine 增名维护；收益主要是可读性，不强求 |
| APP-5 | P3 | timeline.rs：生产零调用的测试孤儿模块挂在 src 顶层 | `timeline.rs:5-10`（注释自认 "nothing here has a production caller"）、`lib.rs:19-20`（#[cfg(test)] mod timeline）；唯一消费者 `service/graph/tests.rs:688`（timeline_epoch fixture） | 53 行、单一函数 next_epoch，为 graph 编译测试提供 epoch 源；顶层位置暗示它属于应用主干，实际只服务一个测试文件 | 移入 `service/graph/tests.rs` 内部 mod（或 `#[cfg(test)] mod support` 紧邻消费者）。权衡：一行移动；若未来多测试文件共用再回升，当前无此迹象 |
| APP-6 | P3 | 测试三种摆放并存，xxx_tests.rs 的技术理由未成文 | 内联：core.rs/mailbox.rs/signals.rs/startup.rs/latency.rs/i18n.rs/console.rs/mod.rs(snapshot_publish_tests)；子目录 tests.rs：graph/(31 test)、managed/(69)、exports/(12)、exports/runtime/(14)、project/(26)；顶层 xxx_tests.rs：service/jobs_tests.rs(2)、settings_tests.rs(2)、project/v2_tests.rs(6)、managed/driver/vad_catalog_tests.rs(2)；另有 `service/windows.rs:82` 内联 `mod catalog_tests`（第四种微风格：内联但 _tests 后缀） | jobs_tests.rs:1-3 注释给出理由（cfg(test) 专属 surface `try_defer`/`pending_jobs` 必须同 crate 单元测试），settings_tests 同理；但 v2_tests.rs/vad_catalog_tests.rs 只是主题拆分（v2 迁移、VAD 目录），无 cfg(test) surface 需求——同一命名承载两种语义。集成测试（tests/: service.rs、service_jobs.rs、signal_service.rs 等 8 个 + common/）与单元测试的分工（跨 crate 黑盒 vs 内部 surface）实际清晰 | 低成本方案：在 AGENTS.md 测试节加 3 行摆放规则（内联=同文件少量；tests.rs=模块级大量；xxx_tests.rs=仅当需要 cfg(test) 专属 surface，纯主题拆分别用）。v2_tests.rs 可并入 project/tests.rs（同主题）。权衡：规则成文比移动文件便宜；不建议为一致性做大规模搬迁 |
| APP-7 | P3 | ServiceMailbox::try_request 是测试专用兼容面 | `service/mailbox.rs:143-158`；生产主循环走 `try_dispatch`（mod.rs:449）；try_request 的调用只在 mailbox.rs 内部测试（6 处） | 测试名 `compatibility_reader_preserves_typed_subscription_for_dispatch` 自认兼容层；pub fn 无生产消费者 | 标 `#[cfg(test)]` 或直接删（测试改走 try_dispatch）。权衡：若视为 mailbox 公共 API 的对称性保留亦可，但应注释说明「测试兼容 reader」 |
| APP-8 | P3 | graph.rs 内两个可下沉纯函数块（其余判定内聚） | `service/graph.rs:406-468`（edit_parameter_values：GraphEdit→(ParameterTarget,ParamValue) 映射，消费方仅 managed/driver.rs:100,142 与 graph/preparation/tests）、`graph.rs:562-576`（edit_hint） | edit_parameter_values 注释自述"single canonical source for every driver"——它是 driver 侧适配映射，与 ServiceGraph 状态机（edit/undo/replace/preparing/running）无共享状态；edit_hint 属 APP-1 | 若做 APP-1 顺手把 edit_parameter_values 移到 graph/ 下新文件或 preparation.rs（它产出 DesiredParameter 同族）；不做 APP-1 则不动（当前 1005 行在可读范围）。判定依据：不为行数拆内聚文件——ServiceGraph 状态机本体（~600 行）是单一不变量（candidate/active/pending revision + journal），不应拆 |
| APP-9 | P3 | service 层内嵌渲染期翻译的展示访问器 | `service/graph/mixer.rs:48-66`（mode_label 直接调 rust_i18n::t!，注释自认"Only the UI thread calls this"）、`service/latency.rs:17-31`（title/description 英文句子，UI 经 t! 动态翻译，settings.rs:607/469 消费）、`driver.rs:349-358`（resume_notice 英文 cause/hint） | service 层三处为 UI paint 准备的文案出口，正是 DYNAMIC_KEY_EXCEPTIONS 中 ~30 条展示注册表条目的来源；与 APP-1 同根不同面 | 不单独动：APP-1 类型化推进到展示注册表（design 21 §3.4/#121 已有规划）时自然收敛为 key 常量。记录在案以免重复评估 |
| APP-10 | P3 | exports/runtime.rs 平台混杂：WindowsAdapter 内嵌于通用 Runtime 文件 | `service/exports/runtime.rs:355-540`（WindowsAdapter + native_can_open 等 Windows 专属）；`runtime.rs:13-14`（`#[cfg_attr(not(any(windows, test)), allow(dead_code))]` 压制非 Windows 告警） | Runtime 状态机（~340 行）平台无关，WindowsAdapter（~185 行）是 moiren_windows_audio 桥接；同文件导致非 Windows 构建需要 dead_code 豁免 | 可选：WindowsAdapter 移 exports/windows_adapter.rs（cfg(windows)），豁免删除。权衡：纯移动；exports/tests 与 runtime/tests 的 FakeAdapter 不受影响；收益小，排在 APP-1/2/3 后 |

**未立项（评估后排除）**：
- 「mod.rs 与 graph.rs+graph/ 混用？」——不成立：`.rs + 同名目录`在 graph/managed/exports/project 四处一致使用（mod.rs 只声明子模块）；backend/core/mailbox/jobs/model/signals/startup/latency/windows 为无目录散件，均 <610 行且单一职责，无需目录。
- `managed/driver.rs` 824 行——内聚大：一个 poll 循环的阶段序列（completions/catalog/observations/execution/edits/parameters/meters/run-limit/exports），共享 owner/edits/parameters 状态，lifecycle/meters/parameters 已合理外分；无职责混杂证据。
- `ServiceContext` 17 字段聚合——设计意图（服务线程独占状态包），各字段有独立 owner 模块管理自身不变量，非 God object。
- CoreDriver 与 ManagedDriver 双 AppRequest 分发器（mod.rs:295-344 vs driver.rs:5-153）——编译器强制两处 match 同步，fallback driver 是非 Windows/测试必需；仅 mod.rs:339 硬编码句随 APP-1 消亡。

## 模块地图（service/ 各模块一句话职责 + 耦合方向）

| 模块 | 职责 | 依赖方向（→ = 依赖） |
|---|---|---|
| `mod.rs` (561) | 模块根+门面；ServiceContext 状态聚合、ServiceRuntime 服务主循环、snapshot 投影、CoreDriver fallback、start_service 入口 | → 全部子模块、→ project（StartupDocuments/PersistenceError/ProjectDocument） |
| `backend.rs` (24) | CatalogBackend trait + BackendError（目录枚举契约） | → model（DeviceCatalog） |
| `core.rs` (328) | ServiceCore 会话生命周期状态机（desired intent / owner generation / phase 三分） | → model |
| `mailbox.rs` (334) | UI↔service 通道：AppHandle 请求入队（校验前置）+ 快照发布（Arc dedup）+ 订阅投递 | → model、→ signals（SubscriptionRequest） |
| `jobs.rs` (273) | JobPool 双 lane 工作池、CompletionQueue/Port/Permit（完成存储预订）、CatalogRefresher | → backend、→ model |
| `model.rs` (230) | 共享数据模型：SessionSpec/AppRequest/AppSnapshot/DeviceCatalog/ControlSummary | → moiren_core/engine、→ project（ProjectDocument）、→ windows_audio（ProfessionalSessionObservations，cfg） |
| `signals.rs` (608) | SignalService：应用信号总线（topic 注册/注销按图演进、订阅执行、1ms pump 预算） | → moiren_signal/engine meter、spectrum |
| `startup.rs` (372) | 启动意图推导：strippable_offline_io（离线 IO 判定）、default_session_for_backend（默认会话） | → model、→ graph（GraphSnapshot） |
| `latency.rs` (84) | AudioLatencyProfile 保存意图 + 缓冲映射 + 展示文案（APP-9） | 无内部依赖 |
| `windows.rs` (128) | WindowsBackend：WASAPI/ASIO/进程目录枚举 → DeviceCatalog | → backend、→ model、→ moiren_windows_audio |
| `graph.rs` (1005) + `graph/` | 可编辑拓扑状态机：ServiceGraph（候选/激活/待定 revision + undo journal）、GraphEdit 全集、验证、GraphFault；edit.rs 应用编辑、preparation.rs 构造 EngineProject、mixer/compressor/builtin 节点状态域 | → moiren_core/engine、→ exports（validate_exports） |
| `managed.rs` (265) + `managed/` | 桌面引擎宿主适配：ManagedDriver 类型+构造（Owner/PendingEdit/PendingParameter）；driver.rs 请求分发+九阶段 poll；lifecycle.rs retire/apply/open_host；meters.rs MeterBridge（engine→应用 topic 中继含频谱变换）；parameters.rs 实时参数提交/ACK 屏障 | → core、graph、exports::runtime、signals、jobs、startup、model；→ moiren_engine::host |
| `exports.rs` (187) + `exports/` | VAD 虚拟端点：意图/机器本地授权/观察校验（exports.rs）、对账计划（reconcile.rs）、原生生命周期 Runtime+Adapter（runtime.rs，含 WindowsAdapter） | → moiren_core、graph（LogicalGraph 校验）；runtime → jobs、meters 无关 |
| `project.rs` (25) + `project/` | 可移植持久化：文档模型/迁移（model.rs）、图文档（graph.rs）、导出文档（exports.rs）、原子存储+PersistenceError（storage.rs）、启动装载（startup.rs，含 APP-2 的 into_context）、窗口态（window.rs）、表计展示偏好（meter.rs） | → service（SessionSpec/GraphSnapshot 等，APP-2 反向点）、→ moiren_core/engine |
| crate 顶层 | i18n.rs（语言枚举+切换，113 行运行时）/ i18n_guards.rs（测试期三守卫，1084 行）/ console.rs（Win32 控制台事件→优雅退出桥）/ logging.rs（flexi_logger 封装）/ timeline.rs（测试孤儿，APP-5）/ lib.rs（OfflineApp 离线嵌入 API）/ main.rs（CLI 入口） | console→ui::run（仅主线程调用）；lib.rs OfflineApp 独立于 service |

状态与数据流总结：service 线程独占全部可变状态；UI 只持有 `AppHandle`（try_request/try_snapshot/subscribe_topic），快照消费用 `Arc::ptr_eq` 变化检测（ui/state.rs:376-379），发布侧 dedup 保 Arc 稳定（mailbox.rs:179-202）。订阅经 mailbox 排队到服务线程执行（signals::execute），ticket 异步交付。除 APP-2 的 into_context 外，project 与 service 的切割干净（project 只做 document ↔ GraphSnapshot 转换与磁盘 IO）。

## 测试组织现状盘点

**单元测试（src 内，共 ~190 个 #[test]）**
| 摆放 | 文件（数量） | 形态 |
|---|---|---|
| 内联 mod tests | core(3)、mailbox(5)、signals(8)、startup(6)、latency(2)、i18n(3)、console(1)、timeline(2)、mod.rs snapshot_publish_tests(2)、windows catalog_tests(2)、project/meter(部分) | 与实现同文件，少量聚焦 |
| 子目录 tests.rs | graph/tests(31)、managed/tests(69)、exports/tests(12)、exports/runtime/tests(14)、project/tests(26) | 模块级大文件（已知大文件 graph 1562 / managed 2972 / project 1040） |
| 顶层 xxx_tests.rs | jobs_tests(2)、settings_tests(2)、project/v2_tests(6)、managed/driver/vad_catalog_tests(2) | jobs/settings 有 cfg(test) surface 技术理由（jobs_tests.rs:1-3 注释）；v2/vad 为纯主题拆分 |

**集成测试（tests/）**：service.rs、service_jobs.rs、signal_service.rs、automatic_startup.rs、offline.rs、i18n_ledger.rs、ui_acceptance.rs、ui_lifecycle.rs + common/ 共享。与单元测试分工（跨 crate 黑盒 vs 内部 surface）实际清晰；i18n_ledger.rs 与 i18n_guards.rs 的 DYNAMIC_KEY_EXCEPTIONS 有镜像关系（i18n_guards.rs:813 注释指认 ALLOWED_RUNTIME_KEYS）。

**helper 复用**：managed/tests.rs 的 `Rig`（driver+context+pool+FakeBackend 组合）+ test_backend.rs（#[path] 挂载，gpui-test feature 复用）是良好实践；graph/tests.rs 的 render/ids/routes、exports/runtime/tests.rs 的 FakeAdapter 各自自持。小重复：Tone 440 SessionSpec 字面 fixture 在 core.rs:293-302 与 settings_tests.rs:5-14 各一份（两处，不值得立项）。

## 与既往审计对账

| 既往 | 内容 | 本轮关系 |
|---|---|---|
| 2026-11-03 AS-1..AS-13 | autosave/project 语义 bug（指纹、覆盖、剥离×autosave 等） | 不重复；本轮 project/ 只看组织。AS 系列涉及的行为面（startup resume、storage 原子写）本轮仅作模块地图，未复核 bug 修复状态 |
| 2026-11-03 ST-1（i18n 死键 43，i18n-keys.tsv 时代） | 死键清理 | PR#156 守卫三件套（bad2ded3…7cf1e313）合入后，死键问题已由 `every_zh_cn_key_is_referenced_by_a_literal_or_exempted` 结构性强制（orphaned/stale 双向检查）——此项视为已解决；残余成本是 116 条动态例外手工同步，即本报告 APP-1 |
| 2026-11-03 ST-6（UI poll-until 重复） | 测试卫生 | UI 侧，不在本范围 |
| 已知大文件（graph/tests 1562、managed/tests 2972） | — | 复核确认行数（1562/2972/1040）；判定为模块级测试集中，配合 Rig/test_backend 尚可维护，不因行数立项 |

## 维护性建议 Top 5（含权衡）

1. **GraphFault 类型化（APP-1，最高杠杆）**：沿已合的 MeterFault/ProjectMessage 模式，把 graph 验证错误句（validate_node_state/edit_candidate ~40 句）类型化为 enum+参数，cause/hint 出口产出稳定 key。一箭三雕：消 edit_hint 子串脆弱匹配、砍 DYNAMIC_KEY_EXCEPTIONS 大头（116→~50）、覆盖 APP-9 的展示字符串路径。权衡：迁移面大、fault equality 语义需保（GraphSnapshot PartialEq 参与 dedup），按文件分波（graph.rs 先、project_fault 次之）可控制风险。
2. **into_context 移到 service 侧（APP-2，最小移动）**：解开 project→service 的唯一实质反向依赖，project 纯化为文档层。~40 行移动 + 2 个测试文件改归属；无 API 破坏。
3. **mod.rs 减薄（APP-3，可折中）**：至少把 ServiceRuntime 主循环（~180 行）拆出；ServiceContext 是否随拆看团队对「mod.rs=目录」的偏好。纯移动、pub use 兜底、无行为变化；代价是 blame 断层与「一个文件读完服务」的便利打折。
4. **测试摆放规则成文（APP-6，三行文档）**：AGENTS.md 写明内联/tests.rs/xxx_tests.rs 的判据（后者仅限 cfg(test) surface），v2_tests.rs 顺手并入 project/tests.rs。零代码风险，止住第四种微风格（windows.rs 的内联 catalog_tests）继续繁殖。
5. **零碎归位（APP-5+APP-7，一次性小 PR）**：timeline.rs 移入 graph/tests、ServiceMailbox::try_request 标 cfg(test) 或删。合计 <60 行变更，消除两个「为什么在这」的困惑点。

审阅但不动：managed/driver.rs（内聚大）、ServiceContext 聚合（设计意图）、glob 门面（APP-4 仅建议 audio_config 转手显式化）、exports/runtime.rs 平台拆分（APP-10 排末位）。
