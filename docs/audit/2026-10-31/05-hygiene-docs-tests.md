# 子代理 E：卫生、文档一致性、测试质量、CI 映射（2026-10-31，main @ dc9b38a）

范围：design 17 两层组件线（PR #53，`21d5b3b..dc9b38a`）合并后的卫生面。全部只读核查，未改代码、未跑 cargo。

总体结论：这条线的卫生水准明显高于仓库平均——新增文件（panel.rs / hot.rs / shared_frame.rs / error_strip.rs / ballistics.rs / 两个 meter_frame.rs）注释全部是约束/意图型，零 TODO/FIXME 残留；主要问题集中在**设计文档与代码的四处实质性漂移**和**两个测试覆盖缺口**（inspector 热层、step-4 面板缓存行为）。

---

## 一、注释卫生

结论：未发现"复述代码"型注释和 AI 风格冗余注释。抽查了全部重点新改文件（`ui/window/panel.rs`、`ui/window/hot.rs`、`ui/shared_frame.rs`、`ui/window/error_strip.rs`、`ui/window/chrome.rs`、`ui/monitoring.rs`、`ui/monitoring/ballistics.rs`、`ui/window/mixer/meter_frame.rs`、`ui/window/graph/meter_frame.rs`、`ui/window.rs`、`ui/window/tray_mixer.rs`、`ui/window/sidebar.rs`），并用 narration 模式（`// get/set/update/return/create/...`）全目录扫描无命中。注释普遍解释 why：如 `panel.rs:205-211`（key 捕获为何必须在通知效应里而非 render）、`monitoring.rs:434-441`（publication 为何不该唤醒界面）、`mixer/meter_frame.rs:341-347`（key 为何不能依赖 render 顺序的 shell 状态）。

### HY-1 (P3) chrome.rs 的"NOT cached"注释与同文件的 PanelHost 机制自相矛盾（术语混用）

- 位置：`crates/moiren-app/src/ui/window/chrome.rs:261-268`（注释）vs `chrome.rs:175-176, 182-259`（`ToolbarChrome = PanelHost<ToolbarPanel>`、`ToolbarKey`）。
- 证据：`toolbar_for_page` 内注释开头是"Chrome is deliberately NOT cached"，但同一文件定义并挂载的 `shell.toolbar_chrome` 正是一个带语义 key 的 `PanelHost`（render 闭包按 key 去重）。207b331（fix: keep the window drag regions outside the cached chrome）实际去掉的只是 `.cached()` splice，不是缓存机制本身；该 commit 信息里"chrome rebuilds per frame by necessity"指的是 prepaint/paint 重放（拖拽 hitbox 需每帧注册），不是 render 闭包。读者必须在"NOT cached"注释和 `ToolbarKey` 机制之间自行脑补出"cached splice ≠ keyed render"这个区分。
- 建议：注释改为明确两层语义，例如"the chrome keeps its keyed PanelHost (render dedup) but is deliberately NOT `.cached()`-spliced: …"。design 17 hazard 8 同步修（见 HY-6）。

### HY-2 (P3) 测试模块的"Explicit imports"避坑注释重复 4 份

- 位置：`chrome.rs:467`、`hot.rs:165`、`panel.rs:291`、`widgets/scroll_step.rs:61`。
- 证据：同一条"模块 glob 会用 gpui test 宏遮蔽内建 `#[test]`"的注释写了四遍。
- 建议：可接受（局部上下文重复），但若再新增第 5 份，应在 `ui/mod.rs` 或 window 模块文档记一次权威出处，其余引用。

其余注释卫生均为正面样例，无需改动。

---

## 二、TODO/FIXME 分诊

结论：**`crates/moiren-app` 全部源码、测试、examples 中 TODO/FIXME/HACK/XXX/WORKAROUND/TEMP 标记为零**（grep 全量核查）。没有可分诊条目，也没有应转 issue 的悬挂项。

design 17 文档记录的 follow-up / hazard 现状核对（均已"做完"或如实标注）：

| 文档条目 | 声称 | 代码现状 | 结论 |
|---|---|---|---|
| flyout SharedFrame 断连（hazard 10 / follow-up 1） | 已修 | `window.rs:316-335` 每个 SharedFrame 只构造一次并 clone 给两侧；回归测试 `tray_mixer/tests.rs:39` | 一致 |
| flyout 裸默认标签 | 已修 | `tray_mixer.rs:340` 与 `mixer.rs:100` 同用 `crate::ui::state::display_label` | 一致 |
| fader 拖拽节流（follow-up 3） | 已修 | `panel.rs:71-127` NestedGate + `cb59d0e`；单测 `panel.rs:322-368` | 一致 |
| 二轮：per-strip panels / KEYED render / 通知效应捕获 | 已修 | `f7a99ba`；回归 `mixer_publish_rebuilds_only_the_changed_strip`（mixer/tests.rs:542） | 一致（除 HY-4 所指一条失实） |
| 三轮：capture-tap 站岗、readout 整形缓存 | 已修 | `window.rs:612-629 legacy_meters_shown`、`mixer/meter_frame.rs:519-532, 630-648`；回归 mixer/tests.rs:984 | 一致 |
| "Still true: chrome rebuilds on every publish" | 如实保留 | 与现状相符 | 一致 |

---

## 三、文档漂移（docs/designs/17-two-tier-ui-components.md vs 代码）

### HY-3 (P2) 文档声称 `UiState::poll` 已接管 observation 派生、`refresh_observation_nodes` 已被替换——代码里两者都不成立

- 位置：`docs/designs/17-two-tier-ui-components.md` §"The cache splits per item"（"…`UiState` now derives the observation source/sink pair itself (in `poll`…), replacing the render-ordered `refresh_observation_nodes`…"）。
- 证据：`ui/state.rs:273-308` 的 `poll` 完全不含 source/sink 派生（全文件无 "observation" 字样）；render 顺序的 `StudioShell::refresh_observation_nodes` 仍在（`ui/window.rs:969-999`）且仍从 render 调用（`window.rs:777`）。git 全历史核查：该派生从未进过 state.rs。实际落地的是另一个修补——strip key 改用快照侧谓词 `strip_probe_candidate`（`mixer/meter_frame.rs:348-363`，注释明确解释了 key 捕获时序问题）+ `observation_nodes` 变更守卫（`f7a99ba`）。
- 建议：改写该段为实际机制（"key 不再读 render 顺序的 shell 状态，改用快照侧谓词"），删掉"UiState 在 poll 里派生 / 替换了 refresh_observation_nodes"的失实表述。这是机器验收叙事段，读者会当作"已落地的机制"来对照代码。

### HY-4 (P2) 文档 References 称 X-1 "addressed"——engine/app 两套节点 level 目录双份实现原样仍在

- 位置：`docs/designs/17-two-tier-ui-components.md` §References（"X-1 (RT→UI level topic directory duplication, addressed alongside the shared NodeMeters)"）。
- 证据：2026-10-27 审计 05 报告定义的 X-1 = engine `SignalHub` 目录 vs app `SignalTopics` 目录双份。两者现在都还在：`crates/moiren-engine/src/host/signals.rs:255-256`（`levels`/`active_levels` BTreeMap，`prune()` :590）与 `crates/moiren-app/src/service/signals.rs:22,125-139`（`node_levels` + `prepare_node_levels`）。共享 `NodeMeters`（`ui/monitoring.rs`）删掉的是 **UI 侧第三份 ballistics 状态**（TrayMixer 私有副本），不触及 X-1 本体。
- 建议：References 行改为"UI 消费侧已随共享 NodeMeters 收敛；X-1（engine vs app 目录）仍开放"，把 X-1 放回跟踪清单而不是宣告已解决。

### HY-5 (P3) hazard 清单编号 1–8 之后直接是 10，缺 9

- 位置：`docs/designs/17-two-tier-ui-components.md` §"Invariants and hazard checklist"。
- 证据：207b331 把旧 hazard 8（SharedFrame 断连）改号为 10、新窗口控制条目占用 8，但没有重排其余条目，清单现为 1,2,3,4,5,6,7,8,10。
- 建议：重排为连续编号，或把 10 改为 9。

### HY-6 (P3) hazard 8 "toolbar and status bar are therefore live" 与 chrome 实际仍是 keyed PanelHost 不符（HY-1 的文档侧）

- 位置：`docs/designs/17-two-tier-ui-components.md` hazard 8 末句。
- 证据：`chrome.rs:175-176` 两个 bar 都是 `PanelHost`；去掉的只是 `.cached()` splice。文档自己的二轮段落又写着 "Chrome … keeps the owner path — its content follows publishes"，两处读起来互相矛盾（"live" vs "keeps the owner path"）。
- 建议：hazard 8 末句改为"the two bars are therefore not `.cached()`-spliced (drag regions need per-frame prepaint); their keyed PanelHosts still gate tree construction"。

### HY-7 (P3) Status 行的 commit 范围落后于实际实现

- 位置：`docs/designs/17-two-tier-ui-components.md:3-4`（"implemented (2026-10-31, commits 6c38f30..70ef4bc)"）。
- 证据：合并线 dc9b38a 内实现性提交一直延续到 `7fda056`（还有 5ec7849、cb59d0e、f7a99ba、e7fbfd9、207b331、9680e3a 等 14 个），且都被文档自己的 follow-up 章节引用；范围停在 step-4 的 70ef4bc 与内容不符。
- 建议：Status 行范围更新为 `6c38f30..dc9b38a` 或去掉范围只留合并点。

### HY-8 (P3) 设计草图的 trait 签名/字段清单与实现有多处小漂移

- 位置：`docs/designs/17-two-tier-ui-components.md` §Tier H 代码块（`wants_frames`、`pump(&mut self, shell, …)`、返回"whether anything display-bearing changed"）与 §Tier C 的 chrome key 描述（"(snapshot identity, page, compact, menu/focus flags)"）。
- 证据：实际 `HotSurface`（`hot.rs:56-76`）没有 `wants_frames`，`pump` 收 `(window, App)` 且返回"是否继续取显示帧（motion，非 visibility）"；实际 `ToolbarKey`（`chrome.rs:183-189`）是 snapshot/page/compact/`project_name`/`pending_request`，无 menu/focus 字段。文档已标 Status: implemented，草图会被当作现状读。
- 建议：在草图块前加一句"以下为设计期签名，落地形态见 hot.rs/panel.rs 及二轮修订"，或直接更新代码块。另有两处排版毛刺（"The clip indicator…" 段前双空行 + 悬挂缩进丢失，约 :170-171；Non-goals 的 RESOLVED 括号内断行破碎，约 :407-410）。

### HY-9 (P3) Deletions 段"NodeMeters promoted to an entity both windows observe"措辞失实

- 位置：`docs/designs/17-two-tier-ui-components.md` §Deletions。
- 证据：`NodeMeters` 是 `UiState` 上的普通字段（`state.rs:221`），不是独立 entity；两个窗口共享它是因为共享 `Entity<UiState>`（`tray_mixer.rs:133-135` 经 `state.update` 调用）。共享事实成立，"promoted to an entity"机制描述不对。
- 建议：改为"the flyout reads the shell's `UiState`-owned NodeMeters"。

### AGENTS.md 与 docs/plans 生命周期

- AGENTS.md 的 UI 结构描述（`src/ui/`、Tabler 图标 `assets/icons/`、`docs/` 架构笔记）**未过时**：不枚举具体文件，新增 hot.rs/panel.rs 等不产生漂移；图标目录与 LICENSE.tabler.txt 都在。
- `docs/plans`、`docs/superpowers/plans`：自 2026-10-27 以来**无新增计划文件**（`git log --diff-filter=A --since=2026-10-27` 为空）；superpowers/plans 现 56 个条目（46 md + 10 目录），与既往已知的归档问题（决策 D-6）规模一致，**无恶化**，不重报。

---

## 四、测试覆盖

### 已覆盖（headless，`gpui-test`，质量良好）

- **PanelHost keyed 缓存**：`mixer/tests.rs:243`（动画帧 strip_renders 平 + meter_paints 前进，符合文档不变量 6"paint 变 + 计数平，绝不只看计数"）、`:542`（publish 只重建被编辑的 strip，精确断言增量 1）、`graph/tests.rs:1331`（Graph 页同理）。
- **热层/帧停摆（stand-down）**：`graph/tests.rs:1463`（bar 下落跨 animation-only 帧）、`mixer/tests.rs:400`（静音停摆至新数据归来）、`:984`（capture-tap 有信号时页面仍站岗，三轮机器验收的回归）、`graph/tests.rs:674/825`（box-select 期间 meter 泵与手势帧）。
- **SharedFrame**：`shared_frame.rs:77-115` 单测 + hazard-10 回归 `tray_mixer/tests.rs:39`（registered>0 且 painted≥registered）。
- **meter 数据平面/订阅按需生命周期**：`graph/tests.rs:555`（选中 inspector 节点保持 level topic 需求、短暂不可视经宽限保留、取消选中后释放——`business_subscribers()` 断言）、`mixer/tests.rs:1065`（managed strip 时 legacy tap 必须未订阅）、`monitoring.rs`/`ballistics.rs` 的球统计学单测（含"rest 阈值钉住 FaderScale 锚点"的一致性测试）。
- **机制单测**：NestedGate（`panel.rs:322-368`）、FrameDemand（`hot.rs:163-180`）、ArcSnapshot/PublishRef（`panel.rs:298-320`）、Placement 裁剪（`mixer/meter_frame.rs:671-708`）。

### 测试覆盖缺口

#### HY-10 (P2) inspector 热表面（D3，`sidebar.rs` InspectorMeters）零测试——hazard-10 断连类缺陷在此表面无回归网

- 位置：`ui/window/sidebar.rs:181-235`（`InspectorMeters` / `InspectorMeterLayer` / `InspectorMeterFrame`）。
- 证据：全 crate 对 `InspectorMeters` 的引用只有装配点（`window.rs:186-187, 330-335`、`node_inspector.rs:1315` 注册几何），无任何测试触碰。对照同类：flyout 有 hazard-10 回归（tray_mixer/tests.rs:39），mixer/graph 有 paint 前进断言。该层 `pump` 恒返 false（搭页面帧循环的车，`sidebar.rs:198-203`），一旦装配点再犯"第二个同类型 SharedFrame"错误（编译通过、永远画空、无运行时检测——正是 hazard 10 描述的失败类），没有任何测试会失败。
- 建议：仿照 `tray_mixer_hot_layer_paints_the_strips_registered_meter_tracks` 补一条：打开 inspector、选中节点、断言 `inspector_meter_layer` 读到的 SharedFrame 有注册轨迹且 paint 被执行（可在 `InspectorMeters` 上加与 flyout 相同的 test-only 计数）。

#### HY-11 (P2) step-4 面板（matrix/devices/settings/inspector/library/library_rail/error_strip）无重建计数器、无缓存行为断言——文档 step-4 验收标准未兑现

- 位置：`ui/window/mixer/performance.rs:11-24`（trace 只有 strip/toolbar/status/graph/shell 计数）；对照 `docs/designs/17-two-tier-ui-components.md` §Rollout step 4（"Acceptance: per-panel rebuild counters flat across poll notifies that do not touch their keys"）。
- 证据：grep 全 crate 无 matrix/devices/settings/library/inspector/error_strip 的 renders 计数；测试列表（window/tests.rs、graph/tests.rs、mixer/tests.rs、devices、settings 等）有大量对这些面板的**功能**测试（编辑、焦点、滚动），但没有一条断言"无关 notify 不重建它们"。PanelHost 通用机制经 strips/graph 验证，但每个面板的 **key 正确性**（漏输入 → 画面冻结，#48 失败类）没有逐面板回归网。
- 建议：至少给 inspector 与 library（侧栏常驻、输入最多）补 key 完整性测试：改一个无关输入（如 elapsed 推进）断言面板 render 计数平；再改一个 key 内输入断言重建。可用 `gpui_test_observe` 计数或给这些 PanelHost 加与 strip 同款的 test 计数。

#### HY-12 (P3) 被删 `tests/signal_visibility.rs`（163 行）的覆盖去向：随抽象正当消亡，一处小语义无后继

- 位置：删除提交 `70ef4bc`。
- 证据：该文件 6 个测试全部针对**同时被删除**的抽象（`PresentationDemand`/`PresentationRect`/`VisibleSignal`，`ui/signals.rs` 同提交删除），测试随实现消亡是正当的，不是缺口。仍有价值的三块语义现状：
  1. 可见性→订阅门控：由 `graph/tests.rs:555`（需求保持/宽限/释放）与 `mixer/tests.rs:984/1065`（legacy tap 门控）接替——已覆盖。
  2. hide/show 推进 demand generation、analysis 订阅者保活：属 moiren-signal crate 机制，crate 侧有 `tests/host.rs:406`（`compatible_activation_preserves_cached_demand_until_the_next_block`）等覆盖；app 侧集成测试随 VisibleSignal 移除，风险低（app 不再有 demand 包装层）。
  3. "持续可见的重复 tick 只入队一次订阅"（原 `persistent_visible_ticks_enqueue_only_one_subscription`）：现在的对应物是 `monitoring.rs:281-292` `ensure_subscription` 的"仅 topic 变化才重建订阅"，**无直接单测**（仅被宽限测试间接压到）。建议补一条 3 行单测：同 topic 重复 `update_points` 不重建订阅（可借 `input_subscribed` 之外的 test 钩或 topic 身份断言）。
- 结论：无 P1 级丢失；仅第 3 点算轻微缺口。

#### HY-13 (P3) ballistics 测试在两个模块各写一份，事实双权威

- 位置：`ui/monitoring.rs` tests（:712-1030，`ballistics_snap_the_peak…`、`ballistics_peak_bar_moves_immediately_without_hold`、`ballistics_marker_holds_then_falls…`、`rms_body_glides…` 等）vs `ui/monitoring/ballistics.rs` tests（:139-307，`bar_rises_instantly_while_the_marker_keeps_the_recent_maximum`、`first_window_snaps_every_layer_then_falls`、`rms_body_falls_at_the_selected_rate_in_silence` 等）。
- 证据：monitoring.rs:940 `ballistics_peak_bar_moves_immediately_without_hold` 与 ballistics.rs:262 `bar_rises_instantly_while_the_marker_keeps_the_recent_maximum` 断言同一行为（marker 定格 0.5、bar 按速率下落且被更静目标托底、更大窗瞬间抬升）；monitoring.rs:862 与 ballistics.rs:286 同为"首窗 snap"场景；两边各自定义 `const WINDOW = 4ms`。其余测试（rest 语义、clip hold、窗口校验）确属 monitoring 层职责，应保留。
- 建议：把纯 ChannelBallistics 行为的重复用例留在 ballistics.rs 一处，monitoring.rs tests 只保留跨层语义（rest/clip/校验/stereo 投影）。

#### HY-14 (P3) 测试内残留两处调试 eprintln

- 位置：`ui/window/mixer/tests.rs:587`（`eprintln!("DELTA first={} second={}", …)`）、`:918-921`（`eprintln!("RESIZE-REBUILDS {}", …)`）。
- 证据：非 `capture.rs`/`performance.rs` 那类有意的 trace 输出，是 `mixer_publish_rebuilds_only_the_changed_strip` / `mixer_fader_small_drag_after_resize…` 调试残留，测试输出噪声。
- 建议：删除，或若对诊断有用改为 `#[cfg(feature = "gpui-test")]` 的 trace 计数。

---

## 五、CI 映射（.github/scripts/detect-changes.sh）

结论：**无缺口**。

- 映射粒度是 crate 目录前缀：`crates/moiren-app/*) direct[moiren-app]=true`。本线全部新路径——`src/ui/window/hot.rs`、`src/ui/window/panel.rs`、`src/ui/shared_frame.rs`、`src/ui/window/error_strip.rs`、`src/ui/monitoring/ballistics.rs`、`src/ui/window/{graph,mixer}/meter_frame.rs` 重写、删除的 `tests/signal_visibility.rs`——都落在 `moiren-app` surface：进 `pkgs`（test）与 `lint_pkgs`（clippy），按设计排除出 `miri_pkgs`（app 不进 Miri 矩阵）。
- 未识别路径保守回落 `full=true`（全量），兜底方向正确。
- 附带核对：`.github/scripts/detect-changes.sh` 头部注释对 surfaces 的描述与 `.github/workflows/runtime-foundation.yml` 的消费一致（2026-10-27 审计已核，本次抽查未见变动）。

---

## 六、仓库卫生快查

- `git status --porcelain`：仅 `?? docs/audit/2026-10-31/`（本次审计自身产物目录，含已在写的 02-structure.md）。工作区干净，无未跟踪杂物。
- 根目录：三个 `target-ingress-*` 构建目录（10-29）已被 .gitignore 覆盖（不出现在 status），非垃圾；`FRONTEND.md`/`README.md` 均为 tracked 正式文档。无新增垃圾。
- **既往 D2 现状**：8 个 0 字节 `*.stderr.log` **已删除**（`git ls-files | grep -c stderr.log` = 0）——该半项已解决；`docs/superpowers/plans/2026-10-14-{engine-stress,frequent-swap}/` 的 ~481KB stress 明细（json/png/jsonl）**仍 tracked**，维持"待用户决策（D-5）"状态，无恶化。
- `docs/` 一次性产物：顶层 vad-*.md 四份为既往已知 X-5 碎片化问题，非新增；本次审计目录 2026-10-31 属预期产物。
- docs/plans 归档问题（既往已知 55 份）：无新增、无恶化（见 HY-9 附注）。

---

## 汇总

| 节 | 条目 | P1 | P2 | P3 |
|---|---|---|---|---|
| 注释卫生 | HY-1, HY-2 | 0 | 0 | 2 |
| TODO 分诊 | （零标记；follow-up 表全部一致） | 0 | 0 | 0 |
| 文档漂移 | HY-3..HY-9 | 0 | 2 | 4 |
| 测试覆盖 | HY-10..HY-14 | 0 | 2 | 3 |
| CI 映射 | 无发现 | 0 | 0 | 0 |
| 仓库卫生 | 无新发现（D2 半项已解决） | 0 | 0 | 0 |

最重要的三条：

1. **HY-10 (P2)**：inspector 热表面（D3）零测试——hazard-10"SharedFrame 断连永远画空且无运行时检测"的失败类在这个表面没有任何回归网，应补一条 registered→painted 断言。
2. **HY-3 (P2)**：design 17 声称 `UiState::poll` 已接管 observation 派生并替换 `refresh_observation_nodes`——代码里两者都不成立（`window.rs:777/969` 仍在 render 期运行），机器验收叙事与实现脱节。
3. **HY-11 (P2)**：step-4 七个面板（matrix/devices/settings/inspector/library/library_rail/error_strip）无重建计数器、无缓存行为断言，文档写明的 step-4 验收标准（"per-panel rebuild counters flat"）未兑现，#48 类 key 遗漏冻结对它们不可测。
