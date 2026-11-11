# 2026-11-09 全仓架构与代码组织只读审计

- **基线**：main @ `27aa57d7`（2026-11-09，工作区干净）。审计零代码改动，本目录为唯一产出（2026-11-11 已入库）。
- **方法**：6 个并行只读子代理按域分工（工作区/跨 crate、app 服务层、app UI、engine+core、windows-audio、signal+plugins），聚焦**架构与代码组织、可维护性**（非 bug 狩猎——11-03 已覆盖）；未跑 cargo，全部结论基于读码 + rg/git 证据。主代理对 4 条关键项亲证（下表 ✓ 列），并裁决了 01/04 两报告对 X-1 的口径矛盾。
- **既往对账基线**：2026-10-22 架构审计（"架构健全、无需拆 crate"）、2026-10-27 重叠审计（单栈化+依赖集中化已落地）、2026-10-29 repo-steward 全仓、2026-10-31 / 2026-11-03 UI 线两轮。本轮为 11-03 以来 ~140 提交（UI 重构战役 #144-#157、plugins crate 加入 d7f95567）后的首次全仓架构复核。

## 目录

| 文件 | 内容 |
|---|---|
| [01-workspace-architecture.md](01-workspace-architecture.md) | crate 依赖图实测、workspace 一致性、CI 覆盖、docs/ 根目录、AGENTS.md 漂移（P1×1 + P2×1 + P3×3） |
| [02-app-service.md](02-app-service.md) | service/project/i18n 组织、错误链路、测试摆放（P2×3 + P3×7） |
| [03-app-ui.md](03-app-ui.md) | ui/ 46.9k 行：域组结构、God 文件判定、跨页内核、五页一致性（P2×3 + P3×5） |
| [04-engine-core.md](04-engine-core.md) | core 存续判定、engine 分层、RT 边界可识别性、NodeKind 扇出（P2×2 + P3×4 + 结论×2） |
| [05-windows-audio.md](05-windows-audio.md) | 域分组评估、owner 命名、影子测试、unsafe/COM 现状（P2×4 + P3×5 + 正面登记×1） |
| [06-signal-plugins.md](06-signal-plugins.md) | signal 分层与契约、plugins 三后端对称性、FFI 三层（P2×2 + P3×9）。注意：其 finding ID "P1" 是编号非级别，实为 P2 级 |

## 执行摘要

1. **总体架构在 160k 行规模下健康，"无需拆 crate" 结论维持**（01/04 两报告独立复核一致）。依赖图实测为 3 层 DAG 无环：core/signal/plugins（叶子）→ engine → windows-audio（实现 engine 的 EngineBackend port，六边形方向正确）→ app。app 经 `[target."cfg(windows)".dependencies]` 拿后端、代码侧 cfg 逐项对齐，Linux CI 可编译性是刻意保住的。唯一理论拆分缝（app 的 gpui-free service/project 层）实测缝宽 53 个 UI 消费文件，拆之只搬代码不减耦合。
2. **唯一的 P1 在 CI 而非代码**：W-1 detect-changes.sh 与 push workflow 均写于 5-crate 时代，moiren-plugins（11-03 加入）无任何路径映射——其 8 个测试与 clippy 在**任何 CI 触发路径下都不执行**，clippy -D warnings 纪律对该 crate 失效。修复约 20 行（脚本 4 处 + workflow 3 行 + 自测断言），风险≈0。
3. **分层纪律的两个标杆**：moiren-signal（unsafe 收敛在唯一生产边界 host/slot.rs、RT 零分配由计数分配器机械强制）与 moiren-plugins（每后端严格 abi 声明/派发包装/编排三层；gain_plugin 夹具是宿主 ABI 镜像的独立对照实现）。engine 的 RT 边界靠"所有权转移（全库零 unsafe Send/Sync）+ Rt 前缀 + 退休显式 API + 分配器测试"四重保障。
4. **既往大风险全部确认未回退**：VAD ABI 三重编码已消除（abi_conformance.rs include_str! 钉驱动头）、virtual-IO 重复已消除（bind_* 单源）、双后端/legacy 栈已删净、2026-10-27 依赖集中化已落地（共享依赖 100% workspace 化）、i18n 双向护栏（11-03 ST-1/2）已结构性解决。
5. **UI 子树组织好于预期**：13 域组边界清晰、缓存/热层集中于三文件且 trait 契约文档齐备、theme 零裸 hsla、四个弹出层单源。真正的结构问题是**跨页共享内核藏进页面模块**（AU-1）：滑杆编辑内核活在 pages/mixer/model.rs 却被 graph/托盘三方经 shell glob 转运消费，且 mixer↔graph 双向依赖。
6. **错误链路是 service 层最大债务**（APP-1）：错误主体仍是"英文句子即 key"，需产生点、DYNAMIC_KEY_EXCEPTIONS（116 条）、zh-CN.yml 三方逐字同步，edit_hint 还用子串匹配错误句——i18n 类型化战役（Wave 5 后）尚未触及的最大剩余面，消减路径即沿 MeterFault 模式类型化 GraphFault。
7. **NodeKind 闭合枚举扇出**（E-1）：新增节点种类需跨 2 crate 改 5+ 处（NodeKind/initial_ports/Binding+bind_*/validate/plan lowering/host NodeConfig）。两报告一致建议**维持闭合枚举**（穷尽 match 是 RT 安全网）+ 写 checklist 文档，不走开放注册。
8. **X-1 裁决（主代理）**：01 与 04 报告口径矛盾已亲证裁决——**类型层已单源**（app 直接 `use moiren_engine::meter::signal::{AudioWindow, ChannelLevel}`，无镜像；spectrum 同），**双目录是刻意的分层中继**（engine 总线管 RT 观测 → app 总线管 UI 流，service/managed/meters.rs:19-21 注释明确），非重复实现。建议：X-1 关闭，并在中继处加一行设计锚点注释，免未来审计再翻旧账。
9. **两处"文档描述不存在机制"的 P2**（06 报告，均已亲证）：signal lib.rs:5-8 门面仍称 "Stream 无生产流量"（11-02 起 app 生产路径已走 Stream）；clap/abi.rs:84-86 注释宣称宿主回调进"原子计数器排水"，实现是三个空函数（mod.rs:86-88）。都是新维护者最先读到的位置。
10. **God 文件基本是伪问题**：七个千行文件中五个判定"内聚大不必拆"（swap.rs、nodes.rs、monitoring.rs、shell.rs、mixer/page.rs），只有两个值得拆且拆法是搬子模块而非削行数（shell/project.rs 四职责混排、node_inspector.rs 双巨函数）。windows-audio 的 wasapi.rs 1231 行是"宽而浅"编排层，协商原语已单源。

## Top 优先级总表

| # | ID | 级别 | 标题 | 位置 | 已亲证 |
|---|---|---|---|---|---|
| 1 | W-1 | P1 | CI surface 映射漏 moiren-plugins：其 8 测试 + clippy 在任何 CI 路径下不执行 | detect-changes.sh:52-58 / runtime-foundation.yml:48-50 | ✓ |
| 2 | APP-1 | P2 | 错误"英文句子即 key"三方逐字同步，edit_hint 子串匹配脆弱；GraphFault 类型化是 i18n 战役最大剩余面 | service/graph.rs:562-576 / i18n_guards.rs:811 | ✓ |
| 3 | AU-1 | P2 | 跨页滑杆编辑内核藏于 mixer 页 + shell glob 转运 + graph↔mixer 双向依赖 | pages/mixer/model.rs / mixer.rs:1 / compressor.rs:3 | ✓ |
| 4 | 06:S1 | P2 | signal lib.rs 门面文档声称 Stream 无生产流量，与现实相反 | moiren-signal/src/lib.rs:5-8 | — |
| 5 | 06:P1 | P2 | clap 宿主回调注释描述不存在的"原子计数器排水"机制（实现为 no-op） | clap/abi.rs:84-86 vs mod.rs:86-88 | ✓ |
| 6 | WA-01 | P2 | 三个同名 "owner" 模块语义互不相干（src/owner.rs 实为 Win32 RAII 工具箱） | src/owner.rs / engine_backend/owner.rs / virtual_owner/owner.rs | — |
| 7 | WA-05 | P2 | 2,540 行真实测试只被 rt-check 影子编译，crate 内不可发现 | asio_rt_tests.rs / recovery_host_tests.rs | — |
| 8 | WA-08 | P2 | RT 边界识别靠逐模块头注释，缺集中线程模型地图 | lib.rs / docs/platform | — |
| 9 | E-1 | P2 | NodeKind 闭合枚举跨 crate 扇出：新增节点改 5+ 处，需 checklist 固化 | core graph/model.rs:45 + engine compiler/host | — |
| 10 | W-2 | P2 | AGENTS.md 结构段漂移：漏 plugins crate、driver/tools/scripts，"push 全量"因 W-1 失实 | AGENTS.md:7-13,34 | ✓ |

其余 P2（APP-2/3、AU-2/3、E-2、WA-02）与 33 条 P3 见各分报告。分报告各自给出"明确不建议做"清单（防为行数拆内聚文件、防为单实例造抽象），合计 12 项。

## X-1 裁决记录（engine/app 双 level 目录，2026-10-31 起）

- 04 报告：**类型已单源**——`app/src/ui/monitoring.rs:8` 直接 `use moiren_engine::meter::signal::{AudioWindow, ChannelLevel}`，spectrum.rs:12 同；无本地镜像，亦无新镜像实例。
- 01 报告：**双侧目录仍在长大**（engine host/signals.rs 802 行 / app service/signals.rs 608 行）。
- 06 报告给出统一解释：这是**分层中继**——engine 总线（RT 观测发布）与 app 总线（UI 流订阅）各司其职，app 经 engine 类型化订阅在服务线程再发布（meters.rs:19-21）。
- **主代理裁决**：X-1 关闭。原诉求（消除类型镜像）已满足；双目录是中继架构的刻意形态，非重复实现。行动项：在中继处（service/managed/meters.rs 模块头）加一行锚点注释指向该结论，防复发误判。

## 建议批次（供排期参考，非工单）

1. **立即**：W-1+W-2 一个 PR（~25 行，CI 映射 + AGENTS.md 同步，detect-changes.test.sh 补断言）；两处 P2 文档漂移（06:S1、06:P1）顺手同车。
2. **高杠杆重构**：APP-1 GraphFault 类型化（沿 MeterFault 模式分波：graph.rs 验证句先行）；AU-1+AU-5+AU-7 合并一次机械搬移（滑杆内核 → ui/editors/，observation 采样 → monitoring/）。
3. **中优先**：AU-2 拆 shell/project.rs、WA-01 owner 改名、WA-05 影子文件归属注释、WA-08 RT 地图（三件可合成一个 windows-audio 卫生 PR）。
4. **文档化决策**：E-1 新增节点 checklist、X-1 锚点注释、APP-6 测试摆放规则入 AGENTS.md。
5. **挂起（等顺风车）**：AU-3 node_inspector 拆分、E-3 swap 测试外移、WA-02 wasapi 目录化、P4 vst3 host_objects 提取、W-4 docs 收敛。

## 统计信号

- 规模：全仓 159.5k 行 Rust；app 73.9k（其中 ui/ 46.9k）、windows-audio 44.0k、engine 34.4k、signal 9.7k、plugins 6.6k、core 1.8k。
- 发现合计：**P1×1、P2×15、P3×33**、正面登记×3（WA-10 probe-tools 门控、C-1 core 存续、C-2 编译链路可读）；主代理亲证 5 项（W-1、APP-1 关键证据、AU-1、06:P1、X-1 裁决）。
- 架构健康面：依赖图无环且方向正确、共享依赖 100% workspace 化、6/6 crate lints 挂接、unsafe 全部收敛有 SAFETY、RT 零分配/ABI 钉扎均测试强制、既往审计大项零回退。
- 未跑 cargo（纯读审计）；lint 基线取 CI 绿。
