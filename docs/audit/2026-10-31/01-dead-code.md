# 死代码与未用面审计 — moiren-app（子代理 A）

> 范围：`crates/moiren-app/src`（重点 `ui/`，其次 service 侧与 UI 相关部分）+ `crates/moiren-app/Cargo.toml` + `crates/moiren-app/assets/`，main @ dc9b38a（PR #53 design 17 "two-tier UI components" 合并后），工作区干净。方法：只读；对 `src/`（含 `#[cfg(test)]` 内联区、独立 tests/ 文件、examples/）做全量定义↔引用交叉扫描（脚本 + 逐条 grep 复核），三方核对图标（ActionAssets 白名单 ↔ assets 文件 ↔ 实际 path 请求），对照 `git diff c47035c..dc9b38a` 删除侧逐项 grep 验证无孤儿。未运行 cargo（主代理在跑 clippy）。既往审计 `docs/audit/2026-10-27/04-app.md` 的开放项状态更新见 DC-12。

## 总体结论

design 17 重构的删除侧是**干净的**：`signals.rs`（PresentationDemand/VisibleSignal/PresentationRect）、DOM 电平表（`compact_stereo_meter`/`meter_bar`/`compact_horizontal_meter`/`horizontal_span`）、`CachedChrome`、`ControlsKey` 双写、托盘私有 `NodeMeters`、`tests/signal_visibility.rs` 均已删尽，grep 零孤儿；`paint_meter_bars` 已成为 D1 统一绘制器（缓存画布与热层共用，`nodes.rs:158`）。本轮真正的死面集中在**更早的遗留**（core.rs 生命周期方法、图标资产）和**仅测试引用的 API 面**。Cargo.toml 无未用依赖/feature。既往记录的 17 个死 theme token 已全部复活或移除（DC-11）。

---

## DC-1 (P1) ServiceCore 三个生命周期方法零引用

- 位置：`crates/moiren-app/src/service/core.rs:122`（`start_failed`）、`:214`（`owner_completed`）、`:219`（`owner_stopping`）
- 证据：全工作区 grep（crates/ + examples/，含测试）：
  - `\.start_failed(` 0 命中；`start_failed` 全库仅 core.rs:122 定义行
  - `\.owner_completed(` / `\.owner_stopping(` 0 命中；各仅定义行
  - 引文：`pub fn owner_completed(&mut self, g: SessionGeneration) { if self.generation == g && self.owner == Some(g) && self.check_open().is_ok() { self.desired = None; } }`
  - 同 impl 的兄弟方法均有生产调用（`start`/`stop`/`begin_exit`/`finish_exit`→mod.rs:449/459、`activated`/`owner_reaped`→managed/driver.rs、`failed` 被 `start_failed` 包装）；唯这三个（加上 `start_failed` 本身）无人调用。属 83e6e34 "retire the legacy backend driver stack" 缩编后的残余：legacy 驱动曾是它们的唯一调用方。
- 建议：直接删除三者（含 `start_failed`，它只是 `self.failed(generation, error.to_string())` 的包装）。风险为零（无测试依赖）。约 17 行。

## DC-2 (P1) 测试后端的 TIMEOUT 常量死代码（连测试都不用）

- 位置：`crates/moiren-app/src/service/managed/test_backend.rs:24-28`
- 证据：`pub const TIMEOUT: Duration = if cfg!(miri) { Duration::from_secs(120) } else { Duration::from_secs(3) };` — moiren-app 全 crate grep 仅此 1 处（moiren-engine/tests 里的同名 TIMEOUT 是另一实体）。miri 分支说明它曾服务跨 crate 测试，现已无消费者。
- 建议：删除（5 行）。若保留是给未来测试用，也应加注释或删除，现状是纯死代码。

## DC-3 (P2) 图标白名单两条目登记但零请求（headphones/monitor，已嵌入二进制）

- 位置：`crates/moiren-app/src/ui/widgets/action_assets.rs:87`（`"icons/headphones.svg"`）、`:96`（`"icons/monitor.svg"`）
- 证据：`ActionAssets::load` 白名单登记并 `include_bytes!` 嵌入，但全 src 无任何代码路径可请求这两个名字：`ActionIcon` 枚举 13 个变体（action_assets.rs:9-23）不含 headphones/monitor；画布节点图标集合为 `{figma-input, app, wave, figma-output, figma-bus, radio, activity, clock, route, figma-fader, input-port, output-port}`（`ui/pages/graph_canvas.rs:199-217` + `nodes.rs:348-352`）；节点库模板 `node_catalog.rs:130-142` 返回 `{mic, app, wave, radio, activity, clock, route, git-merge, speaker, sliders}`；sidebar 只用 chevron-left/right（sidebar.rs:391-403），检查器只用 zoom-scan（node_inspector.rs:82）。grep `headphones|monitor` 在 src 中除白名单外零命中（`monitor_input/monitor pair` 等为无关词）。
- 建议：删除两个 match 臂（各 1 行）并删文件；或留作 devices 页未来素材（当前无消费者）。它们每个 ~1.1KB 被 `include_bytes!` 带进每个发布二进制。附带收益：`action_assets.rs` 现有回归测试（:158-196）只覆盖"使用但未登记"方向，"登记但未用"方向无护栏——可在测试中反向断言白名单与调用集合一致。

## DC-4 (P2) assets/icons/ 19 个 svg 文件无任何代码路径可达

- 位置：`crates/moiren-app/assets/icons/`：`arrow-up-left, check, chevron, circle-dot, folder-open, info, moon, play, refresh, save, search, settings, square, stop, sun, system, win-close, win-max, win-min`（.svg）
- 证据：对 19 个文件名逐一 grep（含 `.svg` 后缀与裸名）全 crate（src/tests/examples）零命中。`"win-min"/"win-max"/"win-close"` 的命中（chrome.rs:148-161、native.rs:193-267）是元素 ID，图标实际经 `ActionIcon::Minimize/Maximize/Close` → `figma-actions/*.svg`（navigation.rs:21-27）；`icons/settings.svg` 未登记（工具栏用的是 `figma-actions/settings.svg`）。这些文件不在 `ActionAssets` 白名单内，而 `ActionAssets::load` 未命中一律回落 gpui-kit 自带 bundle（action_assets.rs:130-135），本地文件永不 serving。合计 24KB（21 个文件含 DC-3 的 2 个）。`manifest.json`（37 项 Tabler 台账）只是 README 引用的来源记录。
- 建议：删除 19+2 个文件并同步 manifest.json，或移入明确的 `assets/icons/unused/` 待用区。风险：纯仓库卫生；若设计侧计划近期启用（如 transport 的 play/stop），保留需注释说明。

## DC-5 (P2) component_gallery 无条件编入所有构建，仅 example 可达

- 位置：`crates/moiren-app/src/ui/component_gallery.rs`（423 行）+ `crates/moiren-app/src/ui/mod.rs:5`（`mod component_gallery;` 无 cfg 门）、`:20`（re-export）
- 证据：`run_component_preview` 的唯一调用方是 `examples/component_preview.rs:3`；`ComponentGallery` 类型只在 gallery 内部构造。主程序 `main.rs` 无任何入口分派到它。同时 `ui/mod.rs:22-26` 的 11 个 widget re-export（`ActionButton…WorkspaceTab`）在生产代码中全部经 `widgets::…` 直连路径使用（如 window.rs:16），`ui::X` 形态的消费者只有 gallery 自己的 `use super::{…}`（component_gallery.rs:7-9）——即这组 re-export 链是 gallery 专用的。gallery 是设计验收工具（"Native visual review"），代码本身活跃，但作为非 cfg 门模块它进入每个 release 二进制。
- 建议：给 `mod component_gallery;` 与 `pub use …run_component_preview;` 加 `#[cfg(any(test, feature = "gpui-test"))]` 或独立 feature（example `component_preview` 声明 `required-features`），re-export 链同批处理。保守方案：仅接受现状并在 mod 声明处注释"dev preview, linked into release by design"。若选择删除可回收 423+11 行（不建议——它是 Figma 验收的活工具）。

## DC-6 (P2) 仅测试引用的生产 API 面（12 处，~165 行）

定义在生产代码（无 cfg(test) 门）、除测试外无任何调用者的项。clippy 不报（均 `pub`）；逐条 grep 复核过定义文件内引用数=1（仅定义行）：

| # | 项 | 位置 | 行数 | 引用者（全部为测试） |
|---|---|---|---|---|
| 1 | `SelectionState::input_index` | ui/state.rs:160 | 4 | ui/state/tests.rs |
| 2 | `output_index` | ui/state.rs:165 | 4 | ui/state/tests.rs、tests/ui_lifecycle.rs |
| 3 | `process_index` | ui/state.rs:170 | 6 | tests/ui_lifecycle.rs |
| 4 | `select_process` | ui/state.rs:176 | 4 | tests/ui_lifecycle.rs |
| 5 | `input_selection` | ui/state.rs:181 | ~8 | ui/state/tests.rs、tests/ui_lifecycle.rs |
| 6 | `latency_apply_text` | ui/presentation.rs:66-74 | 9 | presentation 内联测试（生产曾用于 settings 延迟页，现由 `control_feedback` 路径取代——后者 settings.rs:153 区生产在用） |
| 7 | `OfflineApp::set_gain` | lib.rs:230-254 | 25 | tests/offline.rs:41（main.rs --gain 走 `config.initial_gain`，见 main.rs:28-36，不经 set_gain） |
| 8 | `poll_applied` / `input_status` / `output_status` | lib.rs:256/259/262 | 9 | tests/offline.rs:42-46（`timeline()` 有内部调用 lib.rs:204，不算） |
| 9 | `GraphPreparation::engine_project` | service/graph/preparation.rs:6-19 | 14 | graph/tests.rs（生产全走 `configured_engine_project`，该函数被此死包装+测试引用） |
| 10 | `GraphPreparation::io_nodes` | service/graph/preparation.rs:288-313 | 26 | graph/tests.rs:645,882（旧严格版：单源单汇+立体声硬校验+英文错误串；生产已被 `primary_io_nodes`:241 取代，13 处调用） |
| 11 | `BuiltinNodeSettings::channel_mapper` | service/graph/builtin.rs:83-88 | 6 | builtin 内联测试（兄弟访问器 gain()/recorder() 等均生产在用） |
| 12 | `next_epoch` | timeline.rs:4-6 | 3 | timeline 内联测试（`allocate_epoch` 生产在用） |
| 13 | `ServiceCore::owner_started` | service/core.rs:183-193 | 11 | tests/service.rs:35-69、ui/window/tests.rs:718 |
| 14 | `ServiceJobs::try_defer` / `pending_jobs` | service/mod.rs:224-235/258-260 | 15 | tests/service_jobs.rs（兄弟 `try_reserve_cleanup`/`defer_reserved_cleanup` 生产在用） |
| 15 | `JobResult` | service/jobs.rs:216-219 | 4 | tests/service_jobs.rs:246-277 构造（生产侧经泛型推断使用同一类型时不显式点名——此项删除前需先迁移该测试，见"待核实"） |
| 16 | `ManagedHandle::with_f64_factory` | service/managed.rs:186-197 | 12 | managed/tests.rs:986（doc 自述"remains … for native observation and lifecycle tests"——有意保留的测试工厂） |

- 附：`ui/window/tray_mixer.rs:472-480` 的 `registered_tracks`/`painted_tracks` 与 `ui/monitoring.rs:124` 的 `input_subscribed` 同为测试专用，但已 `#[cfg(all(test, feature = "gpui-test"))]` 门控且注释说明用途（hazard-8 回归），**不算死面**，此处仅登记。
- 建议：分两类。①判定为产品 API 预留的（#7/#8 离线演示控制面、#16 测试工厂、#13 生命周期原语）加 doc 注释声明"test/embedder surface"；②其余（#1-6、#9-12、#14-15）连同其测试断言一并删或并入 `#[cfg(test)]`。注意 #9/#10 删除会顺带消掉 `io_nodes` 里两处英文硬编码错误串（关联上轮 APP-9）。风险：#15 的 `JobResult` 是 `CompletionQueue<JobResult<…>>` 泛型实参，src 生产代码若经别名使用则非死——已 grep 确认 src 内无其他显式出现，但泛型传播需在删除时以编译验证。

## DC-7 (P3) cached_splice 已退化为恒等函数

- 位置：`crates/moiren-app/src/ui/window/panel.rs:282-287`
- 证据：`pub(in crate::ui::window) fn cached_splice<E: 'static>(entity: Entity<E>, cached: impl FnOnce(Entity<E>) -> AnyElement) -> AnyElement { cached(entity) }` — a11y live 分支移除后（commit 6d7e5f0，设计文档 hazard 注记）函数体只剩透传；10 个调用点（devices.rs:117、matrix.rs:54、error_strip.rs:53、settings.rs:66、sidebar.rs:98/325/335、tray_mixer.rs:162、graph/meter_frame.rs:104、mixer/meter_frame.rs:411）。
- 建议：两选一：内联为直接 `cached(entity)` 调用（每处少一层包装，删 6 行函数+10 处 import）；或保留作为"splice 语义 + a11y 历史教训"的文档锚点（当前注释承担了这个角色）。倾向前者，把注释移到 `PanelHost` 文档。

## DC-8 (P3) CanvasNode::wide() 死构建器 + NODE_WIDE_WIDTH 与 NODE_WIDTH 同值

- 位置：`crates/moiren-app/src/ui/pages/graph_canvas.rs:315-319`（`#[allow(dead_code)] fn wide()`）、`ui/theme.rs:61`（`NODE_WIDE_WIDTH: f32 = 270.;` 与 `:60` `NODE_WIDTH: f32 = 270.` 完全同值）
- 证据：`wide` 字段全部构造点为 `wide: false`（graph_canvas.rs:72、graph_canvas_view.rs:198）；`width()` 的 `if self.wide` 分支（:44-46）即便调用也返回同值。两处 `#[allow(dead_code)]` 之一（另一处 window.rs:214 touchpad 字段为 Drop 保留，注释充分，**合理**）。
- 建议：删 `wide()` 构建器与 `wide` 字段及 `NODE_WIDE_WIDTH`（`width()` 直接返回 `NODE_WIDTH`），并移除该 `#[allow(dead_code)]`。约 10 行。风险：若宽卡是设计路线图项，保留字段但应把 allow 换成注释说明。

## DC-9 (P3) paint_channel_meter 内联重复 channel_meter_width 的宽度数学

- 位置：`crates/moiren-app/src/ui/widgets/meters.rs:62-66` vs `:13-19`
- 证据：`let count = channels.max(1) as f32; let gap = (8./count).min(3.); let width = (28./count).clamp(2.,10.).min((40. - gap*(count-1.))/count);` 与 `channel_meter_width` 逐行相同；调用方 meter_frame.rs:464 用函数版求宽度、:602 用内联版画条，两处必须保持一致却无共享。
- 建议：`paint_channel_meter` 改为调用 `channel_meter_width` 求总宽再按 count 拆分（需返回 (width,gap) 或提供伴生函数）。约 5 行收敛。

## DC-10 (P3) graph page() 未使用的 _window 形参

- 位置：`crates/moiren-app/src/ui/window/graph/meter_frame.rs:103` `pub fn page(shell: &StudioShell, _window: &Window) -> AnyElement`；调用点 window.rs:832 传 window 仅为与 mixer 版 `page(self, window, cx)` 对齐。
- 建议：删参或保留以维持两 page 签名对称；一行级，择机。

## DC-11 (P3, 状态更新) 既往 17 个死 theme token：现状零死 token

- 证据：逐 token grep（`theme::X` / `t::X` / `controls::X` / `card::X` 四种引用形态）覆盖 theme.rs 全部 35 个顶层 token + spacing/geometry 常量、theme/controls.rs 全部 30 个 token 与 Density/ControlSize/Metrics、card 子模块 14 个 token——**全部至少 1 个生产调用点**（此前 2026-10-27 记录的死 token 已被 design 17 前后的页面重建消化）。低频但存活：OVERLAY_SCRIM（settings.rs:153）、HAIRLINE（library.rs:248）、GRIP（nodes.rs:209）、SURFACE_4（sidebar.rs:422、library.rs:275）。
- 注：2026-10-27 归档中未检索到"17 个死 token"的原始清单（grep "token" 无命中，05-cross-crate 的"17"是另一上下文）；本结论基于当前全量 token 清点，若主代理持有原清单可直接对名复核。

## DC-12 (状态更新) 既往审计 04-app.md 开放项在本轮重构后的状态

| 项目 | 新状态 | 证据 |
|---|---|---|
| APP-1 legacy 驱动栈 ~1500 行 | **FIXED** | commit 83e6e34 "retire the legacy backend driver stack"：`service/driver.rs`、`service/parameters.rs` 已删（git log --diff-filter=D 确认），windows.rs 缩至 128 行，`BackendDriver/PreparedAppSession/ActiveSession` 全库零命中 |
| APP-2 typing 谓词双写漂移 | **FIXED** | 单一 `StudioShell::typing_focus`（window.rs:904-918）覆盖 node_search+node_name+compressor.inputs+**send_inputs**+**export_controls**，undo_redo_graph 与 keep_graph_focus 共用 |
| APP-3 meter 分段双实现 | **FIXED** | `meter_spans`（meters.rs:220，注释"single source"）唯一分段源；DOM 变体已删；`paint_meter_fill(_horizontal)` 两取向共用（D1） |
| APP-5 托盘独立刻度 | **FIXED** | tray_mixer.rs:23,30 复用 `mixer::{fader_rows,sync_faders}` 与 `widgets::fader::fader` |
| APP-6 按 kind 找主节点散布 | **部分 FIXED** | `GraphSnapshot::node_of_kind`（service/graph.rs:84）已存在，13 处调用；未收敛的调用点（window.rs:969 区等）已在用它 |
| APP-11 state.rs 内联测试 | 未变 | 仍为内联 `#[cfg(test)]`（state.rs:748+），非死代码，仅组织问题 |

## 附：Cargo.toml 与 feature 核对（无发现）

- 全部 `[dependencies]` 均有使用（anyhow 22 文件、flexi_logger 2、serde_json 14、thiserror 5、raw-window-handle 3、tray-icon→ui/tray.rs、windows-registry→ui/branding.rs、rust-i18n 29）；build-dependencies resvg/embed-resource 均在 build.rs。
- 三个 feature（gpui-test/ui-profiler/vad-integration-probe）分别被 tests、ui/window/capture.rs+mixer/performance.rs（`#[cfg(feature="ui-profiler")]`）、examples/vad_integration 消费；两个 example 的 required-features 声明与内容一致。无未用依赖、无未用 feature。

## 统计

- **完全死代码（零引用，可立即删）**：DC-1（17 行）+ DC-2（5 行）+ DC-8（~10 行）+ DC-7（6 行，若内联）+ DC-9（5 行收敛）+ DC-10（1 行）≈ **44 行**
- **死资产**：DC-3+DC-4 = 21 个 svg 文件 / 24KB（其中 ~2.2KB 因 include_bytes 已嵌入每个二进制）+ manifest.json 条目
- **仅测试引用小计**：DC-6 表 16 项 ≈ **165 行**（其中 UI 侧 #1-8 ≈ 69 行）
- **可择机回收**：DC-5 若 cfg 门控 component_gallery 则 release 二进制减重 ~423 行源（源码保留）；若删除则 +434 行
- 合计立即可回收 ≈ 209 行代码 + 21 个资产文件；连同 gallery 门控/删除决策上限 ≈ 640 行。

## 待核实

1. **JobResult（DC-6 #15）**：作为 `CompletionQueue<JobResult<…>>` 的泛型实参可能在 src 生产代码中以别名/推断形式出现——grep 未发现显式出现，但删除前需编译验证（tests/service_jobs.rs:246 是唯一显式标注点）。
2. **"17 个死 theme token" 原始清单**：2026-10-27 归档（01-06 号文件）内未检索到该清单本体；本轮以全量 token（79 个）逐一验证替代。若主代理有原始 token 名单，可对名复核——当前无任何死 token。
3. **OfflineApp 测试面（DC-6 #7/#8）**：lib.rs 是库目标，`set_gain/poll_applied/input_status/output_status` 可能被定位为嵌入方 API；但当前 crate 无外部依赖者（workspace 内无反向依赖），按"仅测试引用"归类。若产品计划保留 CLI 之外的控制面用途，应加 doc 注释后移出本表。
4. **icons/{play,stop,square,check,…} 19 文件（DC-4）**：不排除设计侧（Figma transport 控件）近期启用计划；删除前建议与设计台账（docs/designs/07、assets/icons/README）确认无 roadmap 引用——代码侧确定零引用。
