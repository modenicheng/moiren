# 死代码 / 模块结构 / 测试卫生 / 仓库卫生横切审计（2026-11-03，repo-steward 子代理 F）

> 范围：全仓（重点 `crates/moiren-app/src/ui/`）+ 资产 + locales + docs。基线：分支 `fix/spectrum-standard-curve` = main（3321062，PR #93 feat/node-ui 合并点）+ 2 个频谱修复提交，工作区干净。焦点：PR #93（55 文件 +8467/−549）与 PR #94 graph-autosave（18 文件 +1123/−41）之后的横切质量。
> 方法：只读。死代码面 = 全工作区定义↔引用交叉扫描（脚本化：fn/const/enum variant/theme token/i18n 键四个维度，逐条 grep 复核）；模块结构 = 三处画布代码通读 + 依赖方向 grep + 逐对并排比对；测试卫生 = 六个大测试文件的 helper/装配样板特征 grep + `test_harness` 覆盖核对；仓库卫生 = `git status --porcelain --ignored` + 追踪文件类型/尺寸盘点 + `docs/designs/18` 逐条声明对代码核验。运行了 repo-steward 的 `repo_scan.py`（结论：no findings）与 `file_metrics.py`（结论已并入正文）。未运行 cargo。
> lint 免责：CI clippy `-D warnings` 全绿，故本报告只覆盖 lint 抓不到的面（pub 死面、仅测试引用、数据性死键、双写不变量、结构归属）。既往审计（2026-10-27 / 2026-10-31）已修项不重复报，只在对账表更新状态。

## 总体结论

1. **上轮（2026-10-31）的可回收清单已基本清偿且带护栏**：DC-1/DC-2 死函数已删、21 个死 svg 已删并新增**双向护栏测试**（`action_assets.rs:230` `every_icon_asset_has_a_request_path`，文件系统↔请求集合双向断言）、gallery 已 feature 门控、meter_frame→page 改名、MeterTrace 上移、TestObserve 中立化、display_label 三函数归位 presentation、`io_nodes` 严格版被 `primary_io_nodes` 单一实现取代、`OverlayRects::probe`（overlays.rs:41）以更好的形态吸收了 ST-8 的样板建议。代码级死面本轮扫描（fn/const/variant/token 四个维度）**全部为零**——包括 PR #93 新增的 33 个 `card::*` token与 45 个 theme token。
2. **本轮真正的债不在代码而在数据与双写不变量**，共三条 P2：
   - i18n 双向脱节（ST-1/ST-2）：43 个死键（键表+yml 各一份）横跨四次历史重构无人清，同时 PR #93 新增 4 个生产 `t!` 键**没有 zh-CN 翻译**（中文界面直接漏英文）——两个方向都没有护栏；
   - PR #93 的 `binding_menu.rs` 与 `node_inspector.rs` 手工双写同一份"绑定选择清单"（注释自承诺 same choices, same order，无测试护栏）；
   - `curves.rs` 逐行镜像 engine 两个私有 DSP 公式（peaking 系数、压缩 knee 曲线），注释自认 keep-in-sync，无护栏。
3. 三处画布代码是**一代实现的三层（模型/绘制/壳），不是新旧两代并存**；依赖方向 window→pages→widgets 单向干净（PR #93 之后仍无反向边）。`node_inspector.rs`（window/graph）与 `nodes.rs`（pages/graph_canvas_view）的分置讲得通。
4. 测试卫生良好：`test_harness`（134 行）被 11 个测试文件消费，装配样板已收敛；剩余重复只有 3 个近似的 poll-until 循环与 `assert_close` ×2（P3）。仓库卫生干净：零 TODO/FIXME、无未跟踪垃圾、stress 产物已删（f38cc98）。

---

## 一、死代码 / 可回收面

### ST-1 (P2) i18n 死键 43 个：四次历史重构的孤儿，键表与 yml 双份携带，无清理机制

- 位置：`crates/moiren-app/locales/i18n-keys.tsv` + `crates/moiren-app/locales/zh-CN.yml`（两文件 552 键一一对应，死键同步双份）
- 方法：提取 tsv 首列 552 键 → 对全工作区（含 moiren-engine 的 `title()/label()/description()` 表，排除 `t!(` 字面量与裸字符串两种引用形态，含 30 处变量键 `t!(x.title())`/`t!(label_text)`/`t!(fault.cause.as_str())` 的可达集）逐一核对；43 键在任何 .rs 中零出现。`git log -S` 定位孤儿化提交如下：

| 组 | 键（tsv 行号） | 数量 | 孤儿化来源 |
|---|---|---|---|
| A. 设备监视/时钟面板旧文案 | L120-126、L394、L396-397、L411-412、L414-430（`%{prefix}: actual timing` 等 16 键） | 28 | 83e6e34 legacy 驱动退役 + b79224b 默认 WASAPI 路由重写设备详情页 |
| B. Graph 应用面旧状态 | L12 `Apply graph`、L225、L229、L233 | 4 | 47d00a5/aab8012 移除 Apply face（现行文案为 `Applying changes...`，presentation.rs:247-256） |
| C. 导出帮助旧文案 | L152、L153、L155（L154 `Default eligibility...` 仍活，经 `DEFAULT_ELIGIBILITY_HELP`） | 3 | 导出页帮助文案改版 |
| D. 混音条带旧标签 | L293 `Routing matrix`、L328、L332、L335-337（`L ONLY`/`R ONLY`/`CH %{channel} ONLY`） | 6 | 8529bf0 条带重设计（`MONO`/`SWAP`/`SENDS`/`PRE`/`POST` 仍活） |
| E. 改写后遗留错误变体 | L532（带 "— retry or apply later" 后缀）、L534 | 2 | 服务端错误串改写（现行 `parameters.rs:39` 只余 L533 措辞） |

- 佐证（反向）：错误串键（L499-550）**不是死键**——service 层英文原串经 `error_strip.rs:158` `t!(fault.cause.as_str())`/`:179` `t!(error.as_str())` 在显示时翻译；精度键（L463-468）经 `moiren-engine/src/audio_config.rs:45-84` 的 `title()` 表 + `presentation.rs:107/126` 变量键存活。
- 建议与护栏：删除 43 键（≈ tsv 43 行 + yml ≈172 行台账）；护栏复用 `action_assets.rs:230` 的既有模式（测试内 `read_dir`/`include_str!` 扫描源码 + locales，双向断言"键表键必有代码引用、`t!` 字面量键必有 yml 条目"），i18n.rs 现有测试只锚定个例（i18n.rs:62-92），两个方向都不设防——本条与 ST-2 是同一护栏的两个受益者。注意 tsv 第三列的 file:line 引用已随重构失效（APP-10 的双事实源问题仍在，本轮两文件相互一致但共同携带死键）。

### ST-2 (P2) PR #93 新增 4 个生产键无 zh-CN 翻译（中文界面漏英文）

- 位置与证据（全为生产调用点，`zh-CN.yml` 552 键中无对应条目）：
  - `t!("Bound")` — `window/devices.rs:298`（导出绑定按钮选中态；"Bind" 有翻译，"Bound" 没有）
  - `t!("Color")` — `window/graph/detail.rs:83`（详情窗色调区标题）
  - `t!("Full parameters live in the inspector")` — `window/graph/detail.rs:534`
  - `t!("Spectrum subscription: %{error}", error = …)` — `monitoring/spectrum.rs:372、441`（流入 `MeterStatus::Error`，检查器 `meter_status.label()` 直显，monitoring.rs:75-87）
- 判定：属 SD-11（service/UI i18n 断层）同族但方向相反的**新引入实例**：不是服务层绕过 t!，而是 UI 层新增键未落 locales。与 ST-1 合并一次清理（补 4 条 zh-CN + 删 43 死键 + 加双向护栏测试）。

### ST-3 (P3) `ui/mod.rs` 死 re-export：gallery 门控的残留半边 + 3 个零消费重导出

- 位置：`crates/moiren-app/src/ui/mod.rs:27-31`、`:33-36`
- 证据：
  - `pub use widgets::actions::{ActionButton, ButtonVariant, …}` / `navigation::{…}` / `action_assets::ActionIcon`（10 个名字）——`ui::X` 形态全 crate 零消费；唯一消费者是已 feature 门控的 `component_gallery.rs:9-11` 的 `use super::{…}`。DC-5 修复只给 `mod component_gallery`（mod.rs:9）与 `run_component_preview`（:26）加了 `#[cfg(feature = "component-gallery")]`，**没给配套的 re-export 链加**——feature 关闭时这 10 个 pub 名仍是无人用的公共 API（lint 不可见，pub 于 crate 根）。
  - `pub use pages::Page`（:33）、`state::SelectionState`（:35 半边）、`window::PageCell`（:36）——`ui::Page`/`ui::SelectionState`/`ui::PageCell` 全 crate 零命中（Page 实际经 `pages::Page` 用，PageCell 经 `super::PageCell` 用，见 window/tests.rs:1461）；`ui::UiState` 半边有 3 个消费者（project.rs:65、graph/tests.rs:10、tests/ui_acceptance.rs:21），保留。
- 建议：三行 widget re-export 加同款 cfg 门（或让 gallery 改用 widgets 直连路径）；删 `Page`/`SelectionState`/`PageCell` 三个死重导出。共 ~6 行，纯卫生。

### 仅测试引用面（对账，非新报）：仍开放 ~80 行，无恶化

上轮 DC-6 表中未修项现状逐一复核（调用方 grep 全为 tests）：`SelectionState::input_index/output_index/input_selection`（state.rs:209/215/234，消费者仅 state/tests.rs）、`process_index/select_process`（state.rs:221/228）、`latency_apply_text`（presentation.rs:66，state/tests.rs ×4）、`OfflineApp::set_gain/poll_applied`（lib.rs，tests/offline.rs）、`ServiceCore::owner_started`（core.rs:180，tests/service.rs + window/tests.rs:718）、`ServiceJobs::try_defer/pending_jobs`（service/mod.rs:235/272，jobs_tests）、`ManagedHandle::with_f64_factory`（managed.rs:195，doc 自述测试工厂）、`timeline::next_epoch`（timeline.rs:8，graph/tests.rs:688）。与上轮等价（上轮 ~165 行中约半数已随 #9/#10/#11 修复消失），无新增测试专用面（本轮 fn 扫描对 PR #93/#94 全部新函数零命中）。

---

## 二、模块结构：三处画布代码的归属地图

### 现状地图（PR #93 后，共 9,996 行，其中测试 3,684）

| 层 | 文件（行数） | 职责 | 依赖方向 |
|---|---|---|---|
| **模型**（pages/graph_canvas，纯投影/数学，无壳） | graph_canvas.rs(779)：`CanvasWorld/CanvasNode/CanvasWire/BindingChip/CardFault` 快照投影 + NodeKind→图标/详情；camera.rs(114)；layout.rs(356) 卡片度量；frame.rs(265) 命中测试帧（`HitTarget/DropTarget`）；wire.rs(116) 连线几何；knob.rs(292) 旋钮值数学；curves.rs(480，**pub**) EQ/压缩曲线数学；overlays.rs(167) HUD 矩形 + `probe`；tests.rs(909) | 快照 → 可绘制世界模型 | → theme/monitoring（无 window 依赖） |
| **绘制**（pages/graph_canvas_view） | graph_canvas_view.rs(784)：`CanvasCallbacks` + 网格/连线拖拽/overview + 输入层；nodes.rs(1223)：各 kind 卡片绘制（`paint_knob/paint_binding_chip/paint_matrix_preview/paint_rack_chain/paint_port`…） | 模型 → 像素 + 手势事件 → 回调 | → graph_canvas（`use super::super::graph_canvas`，nodes.rs:8） |
| **壳**（window/graph） | graph.rs(1029)：`impl StudioShell` 手势状态机 + `page()` 组装（:208-930）+ editing_toolbar；page.rs(396)：`ControlsKey`+`GraphPanel`（缓存）+`GraphMeters`（热层）+ 故障 tooltip/框选带绘制；library.rs(408)；node_inspector.rs(1338) 检查器树；detail.rs(623，新) 详情窗浮层；binding_menu.rs(304，新) 绑定下拉；compressor.rs(223)；tests.rs(2775) | 组装、派发 GraphEdit、缓存/热层机械 | → pages 两层（graph.rs:344/567/640、page.rs:10） |

**判定：不存在新旧两代并存。** PR #93 在既有三层内原位重写（graph_canvas.rs 650 行改动、nodes.rs 1205 行改动是重写不是并存）；旧 `meter_frame.rs` 已改名 `page.rs`（ST-1 上轮建议落地）；`figma-nodes/` 四个旧卡片 svg 已删且 README 已改写（assets/figma-nodes/README.md 自述"Every kind icon now comes from assets/icons/"，与 nodes.rs:942/1047 仅剩 port/more 引用一致）。依赖方向 grep：pages 两层对 `crate::ui::window` 零引用（TestObserve 已中立化到 ui/observe.rs，graph_canvas_view.rs:15）。

### ST-4 (P2) 绑定选择清单手工双写：binding_menu vs node_inspector（PR #93 新引入）

- 位置：`window/graph/binding_menu.rs:59-187`（`items()`）vs `window/graph/node_inspector.rs:221-300+`（Source 节）、Sink 节对应段
- 证据：两处按**相同顺序**遍历相同数据源构造相同 `GraphEdit`：Tone(440) → `state::input_rows`(WASAPI) → Playback 导出 → processes（Source，binding_menu.rs:78-137 vs node_inspector.rs:234-305）；`output_rows` → Microphone 导出（Sink，binding_menu.rs:154-182 vs inspector 对应段）。binding_menu.rs:58 注释自承诺"The same choices the inspector lists for the kind, in the same order"——**这条不变量靠手工维护，无任何测试**（graph/tests.rs 的 `graph_binding_chip_opens_the_endpoint_menu_and_binds_from_it` 只测菜单开合与绑定，不测两清单一致）。新增一种 InputSelection（如未来的虚拟设备）需要同时改两处，漏一处即两表面行为分叉。约 130 行平行逻辑。
- 建议：提取共享枚举器（如 `fn source_choices(snapshot, id) -> Vec<Choice>`，Choice = {id,label,edit,current}，放 state.rs 或 presentation.rs 旁），两个表面各自决定渲染形态（inspector 按钮/菜单行 + inspector 独有的 tone ±10 Hz 微调保留在 inspector 侧）。低风险纯收敛；若不做，至少加一条两清单长度/顺序一致性的回归测试。
- 反面（为什么不是"合并两个表面"）：菜单是 canvas 就近重绑、检查器是全参数面板，交互形态与挂载点（shell root 浮层 vs 缓存面板）都不同，合并 UI 是过度设计；要收敛的是**数据**不是表面。

### ST-5 (P2) curves.rs 逐行镜像 engine 两个私有 DSP 公式，无同步护栏（PR #93 新引入）

- 位置：`pages/graph_canvas/curves.rs:48-64` `peaking_coefficients` ↔ `moiren-engine/src/processor/builtin/eq.rs:82-99` `Eq::coefficients`（15 行逐行同，含相同 clamp）；`curves.rs:176-188` `reduction_db` ↔ `compressor.rs:160-171` `Compressor::target_reduction`（Giannoulis Eq.(4) 逐行同）。两处注释均自认"mirrored from … (private to the engine crate); keep the two in sync"（curves.rs:46-47、:170-172）。
- 风险：镜像漂移时缩略图/详情窗曲线对实际 DSP 撒谎，且无测试能发现（curves.rs 测试只验数学性质：峰值/对称/平直，不锚定与 engine 一致）。
- 建议（该搬）：把这两个**纯函数**（无 RT 约束、无状态）改为 engine pub API（如 `moiren_engine::processor::builtin::{peaking_coefficients, target_reduction}` 或一个 `analysis` 小模块），app 删镜像。收益：消掉两条手工同步不变量 + ~35 行；代价：engine 公共面 +2 个纯函数，可忽略。替代（不搬的最低动作）：在 curves.rs 测试里用 `Eq::coefficients` 输出对拍——但它私有，对拍不可行，故搬出优于对拍。
- 反面（为什么曲线**绘制**留在 UI 层是对的）：采样/画点逻辑（`sample_curve_log_x`）是绘制关注点，不该下沉；只共享数学内核。

### node_inspector.rs(1338, window/graph) vs nodes.rs(1223, pages/graph_canvas_view) 的归属判定：**成立，不动**

- node_inspector 构建检查器树并经 `StudioShell` 派发 `GraphEdit`（node_inspector.rs:1326 `reset_clip_on_click(shell.state.clone(), this, id)` 等大量 shell 实体交互）——必须有壳，window/graph 是正确位置；上轮"不要拆清单"的表驱动 `BuiltinFieldSpec` 判断仍成立（:1038-1153 表 + :1209 `t!(label_text)`）。
- nodes.rs 是纯绘制（接收 `CanvasCallbacks` 回调上抛手势，不触 shell），pages 是正确位置。
- 两文件都大但各含单一职责的 per-kind 分派；按 kind 拆文件只会复制装配段（上轮结论维持）。
- 观察项（不立新案）：`NodeKind::` 分派现散布 6+ 生产文件（graph_canvas.rs:36、node_catalog.rs:26、state.rs:10、capture.rs:9、node_inspector.rs:7、detail.rs:6）——新增节点 kind 需触 6 处 + locales。这是三层架构的固有代价且各处关注点不同（图标/模板/行投影/验收/字段/详情），**不建议**引入 kind→trait 注册表之类的统一抽象（可 grep 性变差、为假想的 kind 增长付复杂度）；设计 19 若再增 kind，把"新增 kind 检查单"写进设计文档即可。

### 该合 / 该搬 / 该拆 / 不要动（宁少勿多）

- **该收敛**：ST-4 绑定清单数据（唯一实质 P2 结构项）；ST-5 DSP 镜像公式搬 engine。
- **该搬**：无（TestObserve/display_label 等上轮已搬完；autosave.rs(211) 挂 window/ 正确——它就是 `impl StudioShell` 的域模块，window.rs:130/451、project.rs:559 消费）。
- **该拆**：无 P1/P2 级候选。`window/graph.rs::page()`（:208-930，约 720 行）是长函数，但它是顺序 DOM 组装（world 投影 → canvas view → toolbar → overlay 挂载），行数来自域规模；机械拆段只会制造传参噪音。若未来再增浮层（第三个 root overlay），可届时把"浮层挂载段"抽出，现在不动。
- **不要动**：node_inspector.rs(1338)、nodes.rs(1223)、monitoring/spectrum.rs(872，投影数学 ~200 行 + 订阅表 ~340 行 + 测试 ~300 行，与 monitoring.rs 同构——上轮"两张表共享生命周期校验"的不拆理由同样适用)、graph/tests.rs(2775)/mixer/tests.rs(2511)（按 harness 聚集的验收测试，上轮同判）、window.rs(1038，组合根)。
- 结构残留开放项（上轮遗留，现状未变）：ST-6 泵同构（mixer/page.rs:611 vs graph/page.rs:152，~12 行 ×2）；ST-9 `frames_eligible` 页白名单（window.rs:544-551，注释自辩保留）；ST-13 生产/测试双轮询（desktop.rs:465-476 vs window.rs:507+，PR #94 在两处同步加了 `poll_autosave`——双写维护成本的又一实证，也证明两份目前未漂移）。均择机，不单独立案。

---

## 三、测试卫生

### 现状：harness 已建且覆盖良好，剩余重复为 P3

- `ui/window/test_harness.rs`（134 行，`#[cfg(feature = "gpui-test")]`）提供 `spawn_service/initial_projection/finish_service/settle_panels/wait_until/wait_for/shell_with_state/test_shell`，被 11 个测试文件 + native.rs/project.rs 内联测试消费（grep `test_harness` 11 文件）。`test_shell` 自带环境隔离（`preferences_path = None` + **PR #94 补的 `autosave_path = None`**，test_harness.rs:125-134——新功能的测试隔离跟上了，好信号）。
- ui/ 测试总量：`#[gpui_kit::test]` 117 个（14 文件）+ 内联 `#[test]` 117 个；专用测试文件 11,250 行 / ui 总 40,955 行（27.5%）。规模较上轮（30,893）+10,062 行，主要来自 PR #93（graph/tests.rs 836 行、graph_canvas/tests.rs +392、service 侧 graph/tests.rs +285/managed/tests.rs +173）。
- 抽查测试名与断言意图（PR #93 新增 10 个 graph 测试 + binding chip 测试逐行读）：命名与断言一致，质量高（例：`graph_binding_chip_opens_the_endpoint_menu_and_binds_from_it` 实测菜单开/Escape 关/重开绑定/空目录关闭四段）。一处登记：该测试对"无输出设备目录"做了 `if let Some(row) … else` 环境分支（graph/tests.rs 尾段）——绑定断言在有真实输出设备的机器上才跑，CI 无设备时静默降级为只测关闭路径。可接受（硬件测试与可移植测试分离是 AGENTS 约定），但绑定路径若只在开发机跑过，值得在测试注释里写明。

### ST-6 (P3) 三个近似 poll-until 循环 + `assert_close` ×2

- 位置：`mixer/tests.rs:10-19` `poll()`（轮询 `pending_request` + 每轮 render）、`graph/tests.rs:59-71` `wait_for_graph()`（轮询 `GraphSnapshot` 谓词 + 成功后 render）、`devices/exports/tests.rs:245-265` `wait_for()`（轮询快照谓词 + `!pending_request` + 成功 render）。三者都是 `wait_until` 之上的薄封装，差异只在谓词与 render 时机，~30 行平行；`assert_close` 在 `graph_canvas/tests.rs:26` 与 `graph/tests.rs:55` 各一份（4 行）。
- 建议：harness 加一个 `poll_snapshot_until(render_each_step: bool, predicate)` 即可收编三处（`assert_close` 可一并进 harness）。收益小，择机随下次动 harness 时做；不值得单独 PR。

---

## 四、仓库卫生

- **工作区干净**：`git status --porcelain` 仅 `?? docs/audit/2026-11-03/`（本审计自身）。ignored 项全部合理（target 族、`.tmp/`、`.worktrees/`、`.ruff_cache/` 等 22 项，与 .gitignore 逐条对应）。追踪文件无构建产物：类型分布 rs 472/md 91/svg 42/png 28/json 22；最大追踪文件 Inter.ttf 788KB（字体，正当）与 designs/evidence 292KB（验证证据，正当）。
- **既往 D2 stress 产物：FIXED** — `docs/superpowers/plans/2026-10-14-*` 已随 f38cc98 "remove landed superpowers plans and stress artifacts" 删除，`git ls-files docs/superpowers/plans` 为空。
- **docs/designs/18 对账：通过**。逐条核验：`GraphEdit::ConnectReplacing`（graph.rs:145 + 测试 graph/tests.rs:747/793/999）、`Reconnect`（graph.rs:154/512）、`GraphSnapshot::preparing_nodes`（graph.rs:78/693）、`DropTarget`（frame.rs）、detail 窗（detail.rs，"Full parameters live in the inspector" 与 B4"inspector 长尾参数"承诺一致）、宽 4:3 频谱卡（theme.rs `card::SPECTRUM_W/BODY_H` 注释复述同一设计）、图标经 ActionAssets 白名单（phase 11，白名单测试覆盖）、capture 验收态（capture.rs:506-594 selected/faulted 状态注入，phase 12）。未发现设计声明与实现相悖处。
- **rt-check 改动（PR #93 内 2148e9f/85b044e）：意图正当，登记一处无护栏镜像**。意图：negotiation.rs（COM 相关）是 Windows-only，非 Windows 构建下 RT owner 的影子测试拿不到它导出的协商限值，故在 `tools/rt-check/tests/professional_rt.rs:43-54` 镜像 4 个纯数字常量（MIN/MAX_SAMPLE_RATE_HZ、MAX_BUFFER_FRAMES、MAX_CHANNELS）并导出 `TaskMemory`，保持 owner 的实时路径全平台受测。一致性：四个值当前与 `moiren-windows-audio/src/negotiation.rs:41-44` 完全一致（逐一比对）。风险：纯文本镜像无同步护栏（跨平台编译不到真源，无法用编译断言），值是物理限值、漂移概率低——P3 登记，若 negotiation 限值将来变更，需记得两处同改（可在 negotiation.rs 常量注释里点名 rt-check 镜像位置）。
- **AGENTS.md / README**：与现状一致（icons 路径、feature 命令、`component-gallery` example 的 required-features 与 Cargo.toml:50-51 相符）。全 crate TODO/FIXME 仍为零。

---

## 五、既往项对账表（2026-10-31 → 本轮）

| 既往项 | 本轮状态 | 证据 |
|---|---|---|
| DC-1 ServiceCore 三生命周期方法 | **FIXED** | `start_failed/owner_completed/owner_stopping` 全库零命中（已删） |
| DC-2 test_backend TIMEOUT | **FIXED** | 文件内零命中 |
| DC-3+DC-4 死图标 21 svg | **FIXED+护栏** | 文件已删；`action_assets.rs:229-329` 双向回归测试（资产↔请求互查） |
| DC-5/ST-12 gallery 无门控 | **FIXED（残留 ST-3）** | `#[cfg(feature = "component-gallery")]`（mod.rs:9/26）+ example required-features；re-export 链未同批门控 |
| DC-6 #9 `engine_project` | **FIXED** | preparation.rs:7-17 已 `#[cfg(test)]` + doc 声明 |
| DC-6 #10 严格版 `io_nodes` | **FIXED** | 仅存 `primary_io_nodes`（preparation.rs:243），生产 5 处 |
| DC-6 #11 `channel_mapper` | **复活为生产** | graph_canvas.rs:502 PR #93 起真实消费 |
| DC-6 其余（selection 三件/latency_apply_text/offline 面/owner_started/try_defer/JobResult/with_f64_factory/next_epoch） | **仍开放 ~80 行** | 见"仅测试引用面"；JobResult 生产版已删（仅 tests/service_jobs.rs:231 本地定义） |
| DC-7/ST-5(上) cached_splice | **FIXED** | 零命中，10 调用点已内联 |
| DC-8 `wide()`/NODE_WIDE_WIDTH | **FIXED** | wide 成为真实卡宽语义（card::SPECTRUM_W），NODE_WIDE_WIDTH 删除 |
| DC-11 死 theme token | **维持零** | theme 45 + card 33（新）+ controls 32 全部有生产引用（脚本+手工双重核验） |
| ST-1(上) meter_frame 名不副实 | **FIXED** | 两处均已改名 page.rs |
| ST-2(上) MeterTrace 归属 | **FIXED** | window.rs:186 直属字段，performance 挂 window（:735） |
| ST-3(上) TestObserve 反向边 | **FIXED** | ui/observe.rs 中立模块 |
| ST-4(上) display_label 等居 state.rs | **FIXED** | presentation.rs:275-318 |
| ST-7(上) reset_clip/mixer_edit 闭包 | **FIXED** | `reset_clip_on_click` 共享 helper（mixer.rs:317、tray_mixer.rs:373、node_inspector.rs:1326） |
| ST-8(上) prepaint 注册样板 ×6 | **FIXED（更好形态）** | `OverlayRects::probe`（overlays.rs:41-53）单一注册原语，样板仅剩 fader.rs:210 一处 |
| ST-9 frames_eligible 白名单 | 仍开放（自辩注释保留） | window.rs:540-551 |
| ST-13 双轮询 | 仍开放；PR #94 双侧同步扩（未漂移） | desktop.rs:467/472 vs window.rs:523/525 |
| X-1 engine/app 双 level 目录 | **仍开放，面+1** | engine host/signals.rs:540 `prune` 与 app service/signals.rs 双目录仍在；PR #93 又按同模式新增 **spectrum** topic 目录（engine signals.rs:334-354 vs app signals.rs:49/213-255）——未恶化但同一债多了一份实例 |
| APP-10 tsv/yml 双事实源 | 仍开放 | 552 键两侧一致，但共同携带 43 死键（ST-1），file:line 列已失效 |
| D2 stress 产物 481KB | **FIXED** | f38cc98 删除 |

## 统计

- **死代码（代码级）**：0 行（fn/const/variant/token 四维扫描全零；含 PR #93 全部新增面）。
- **死数据/资产**：i18n 死键 43 个（tsv 43 行 + yml ≈172 行台账）；死 svg 0 个（护栏生效）。
- **缺失翻译**：4 个生产键（devices.rs:298、detail.rs:83/534、spectrum.rs:372/441）。
- **双写不变量（无护栏）**：绑定清单（~130 行平行）、DSP 镜像公式（~35 行）、rt-check 常量（4 个）、双轮询（PR #94 已实证同改两处）。
- **仅测试引用生产面**：~80 行（较上轮 ~165 收缩，均为已知项）。
- **立即可回收合计**：≈255 行台账 + ~10 行 re-export + 4 条翻译补录；若做 ST-4/ST-5 收敛再 -165 行平行逻辑。
- ui/ 规模 40,955 行（+10,062 / +33% vs 上轮），画布三处合计 9,996 行（测试 3,684）。
