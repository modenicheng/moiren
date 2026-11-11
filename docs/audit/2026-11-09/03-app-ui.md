# 2026-11-09 架构审计 — moiren-app UI 子树

基线: main @ 27aa57d7（工作区干净）

范围：`crates/moiren-app/src/ui/` 全部（96 个 .rs，46,930 行，其中专用测试文件约 13,224 行 + 内联 `#[cfg(test)]` 约 1,600 行，生产代码约 32k）。本轮按约不做事 bug 狩猎，聚焦架构与代码组织。

## 执行摘要（要点）

1. **总体判断：这是一套组织良好、文档密度异常高的 UI 子树。** 13 域组重组（2026-10-31）后边界清晰；缓存/热层双轨组件模型集中于 `shell/panel.rs` + `shell/hot.rs` + `shared_frame.rs` 三处，trait 契约与设计文档锚点（design 17）齐备，新开发者可从模块 doc 直接推断规则。
2. **最大的结构性问题不是 God 文件本身，而是「跨页共享内核藏进了页面模块」**（AU-1）：滑杆编辑机制活在 `pages/mixer/model.rs` 却被 graph（压缩器、检视器）与托盘 flyout 三方消费，且导入路径绕经 shell 预导出 glob（`crate::ui::shell::mixer::*`），形成 graph↔mixer 双向依赖且依赖图失真。
3. 七个千行文件中，五个判定为「内聚大、不必拆」（nodes.rs、mixer/page.rs、monitoring.rs、shell.rs、graph_canvas_view.rs 主干），两个「职责混杂、值得拆」（shell/project.rs、node_inspector.rs）——但拆法是搬出子模块而非削行数。
4. 五页结构「同型不同构」：都遵循 根模块(状态+re-export) + 子模块(渲染/模型) 的形状，但入口命名三种（mount+page / mount / page）、PageState 位置 4 处在页根、1 处在 page.rs（AU-4）。属低危一致性债。
5. i18n 组织为全仓标杆：单目录 `locales/zh-CN.yml`（1,040 键，键=英文源串）、`src/i18n_guards.rs` 三护栏（双向键台账、占位符一致、debug 格式绊线）测试期强制。Wave 5 后 UI 残留仅领域记号（`!`、`-inf`、`C`）与 gallery 标题。
6. theme 纪律完好：pages/shell/widgets 下裸 `hsla(` 为 **0 处**，SPACE_*/TEXT_*/BORDER 等 token 高频集中使用，PR#145-#153 的 token 波次无新增漂移。
7. 复用健康：四个弹出层（binding menu、context menu、settings 菜单、fault popover）全部走 `widgets/anchored_popover` + `occluding_page` 单源；ST-4（绑定清单双写）已由 `graph/binding_choices.rs` 单源解决。
8. 测试组织：约 129 个 gpui 无头验收测试 + 约 71 个纯模型测试，共享 harness 集中在 `shell/test_harness.rs`。三大平铺巨文件（graph 3,654 行/46 测试、mixer 3,132/33、shell 1,813/24）内部有清晰簇与本地共享 helper，可拆但不紧迫（AU-6）。
9. 与既往对账：2026-11-03 的 IN/GC/AS 系列为 bug 工单不在本轮范围；E-1（i18n 护栏）、E-2（ST-4）、E-3 曲线镜像等已可确认落地；X-1 与缓存复杂度为已知开放项，本轮对后者给出**正面复核**（集中度与文档优于预期）。

## 发现清单

| ID | 级别 | 标题 | 位置 | 证据摘要 | 建议+权衡 |
|---|---|---|---|---|---|
| AU-1 | P2 | 跨页共享编辑内核藏于 mixer 页，导入绕经 shell glob，graph↔mixer 双向依赖 | `pages/mixer/model.rs`（全文）；`pages/graph/compressor.rs:3`、`pages/graph/node_inspector.rs:6,215,222`；`pages/mixer.rs:1` | 滑杆机制（SliderSlot/SliderEdit/sync_*/start_value_edit/EditableValueFamily/gain_db_text）被 graph 两文件经 `crate::ui::shell::mixer::{...}` 引用（shell 只是 re-export 转运站）；反向 `mixer.rs:1` 引 graph 的 `observation_snapshot`/`OBSERVATION_SAMPLE_CADENCE`（该采样器亦被检视器用）。tray_mixer.rs:23 亦消费。mixer.rs:15 注释仍称旧路径 "window::graph::compressor" | 把滑杆/点击输入编辑族提为 `ui/editors/`（或 widgets/sliders），observation 采样提入 `monitoring/`（它本质是计量采样）。权衡：一次纯搬移 PR（diff 大但机械、测试 import 同步）；不搬则每个新 slider 表面（已 4 处）都续写误导路径 |
| AU-2 | P2 | shell/project.rs 四种职责混排 | `shell/project.rs`（1,062 行） | 单文件含：menu() 渲染（13-485，约 470 行单函数）、ProjectMessage/PersistedFailure 类型（486-583）、i18n 文案映射（584-686）、ProjectIo + StudioShell 工程 IO 逻辑（716-1062） | 拆 `project/{menu.rs, messages.rs}`（IO 留主文件），menu() 内部按 New/Open/Save 分节。权衡：小而机械；不拆的主要代价是 470 行菜单函数评审困难 |
| AU-3 | P2 | node_inspector.rs 双巨函数 + 字段 schema 混在渲染文件 | `pages/graph/node_inspector.rs:46-480`（inspector() 430 行）、`:609-981`（builtin_controls() 371 行）、`:1017-1131`（gate/limiter/delay 字段表） | 表驱动的 BuiltinFieldSpec schema、step 编辑器、观测采样（515-608）、inspector 视图装配、meter track 五块共存；观测采样还被 mixer 页跨页引用（见 AU-1） | 将 builtin 字段 schema + step 编辑器提为 `node_inspector/builtin_fields.rs`；inspector() 按 NodeKind 拆装配函数。权衡：表驱动设计本身良好，拆的是文件不是结构；观测采样随 AU-1 迁 monitoring |
| AU-4 | P3 | 五页入口与页状态组织不同构 | `pages/devices.rs:138,154`（mount+page）、`pages/graph/page.rs:145`（仅 mount）、`pages/mixer/page.rs:490`、`pages/matrix.rs:31`（仅 page）；PageState 位置：devices.rs:128 / mixer.rs:170 / matrix.rs:24 / settings.rs:90 在页根，graph/page.rs:127 在子模块 | 同一概念三种入口形状、状态两种落点；无行为差异，纯导航成本 | 定一条约定（推荐 graph 形状：根=状态类型+re-export，page.rs=mount+PageState+Panel）后逐页对齐。权衡：纯一致性收益、可随日常 PR 顺手做；单独迁移 PR 价值偏低 |
| AU-5 | P3 | dB/平衡文本格式化双源 | `pages/mixer/model.rs:228-306`（parse_gain_db/parse_balance/parse_pan/gain_db_text/balance_text） vs `presentation.rs`（db_text/dbfs_text/step_db，模块契约「纯文本投影」） | 两处各自维护 dB 文本真源；presentation.rs 的契约恰好容纳前者（纯函数、无 UI 状态） | 将解析/格式化族并入 presentation.rs。权衡：mixer 对 presentation 依赖加深一档，但消除格式漂移面；随 AU-1 一起做最省 |
| AU-6 | P3 | graph/tests.rs 3,654 行平铺 46 测试，六簇可辨识 | `pages/graph/tests.rs` | 簇：连线/键盘编辑(约10)、检视器(7)、相机缩放平移(6)、计量与缓存(7)、绑定菜单(4)、上下文菜单(3)、库(2)；顶部 8 个共享 helper（render/canvas_frame/add_node/wait_for_graph 等）已被全体复用 | 转 `tests/` 目录按簇分子模块（Rust file+dir 惯用法，helper 留 mod.rs）。权衡：纯组织性，cargo test 名称路径会变（CI/文档引用需同步）；收益是评审与定位 |
| AU-7 | P3 | 全壳计量裁决 update_node_meters 位于 mixer 页模块 | `pages/mixer.rs:24-136` | 注释自述跨 Graph/Mixer/Matrix/Devices 四页的可见性判定；调用方是 shell.rs:666 的 pump_meters —— 页面模块持有 shell 级职责 | 随 AU-1 移到 shell（与 pump 为邻）或 monitoring。权衡：搬移后 mixer.rs 根几乎只剩装配，职责更真 |
| AU-8 | P3 | 卫生项：过时注释与模块声明位置 | `pages/mixer.rs:15`（"window::graph::compressor" 旧路径）；`shell.rs:771-799`（模块声明在文件底部而非顶部）；`shell/panel.rs`、`shell/hot.rs` 服务全部页面却位于 shell/ 命名空间下 | panel.rs:1 doc 自称「Tier C of the two-tier component model（design 17）」但目录位置暗示 shell 专属 | 改注释与声明位置零成本随时做；panel/hot 若迁移（如 ui/tier/）需全 UI import 更新，建议仅在下次大动时顺带 |

**明确不建单列的观察**：单文件目录（`graph_canvas_view/` 仅 nodes.rs、`shell/capture/`、`shell/performance/`、`matrix/`、`devices/exports/` 仅 tests.rs）是 Rust「file.rs+同名目录」惯用法与测试伴生布局的自然结果，不构成问题。

## UI 目录地图

| 模块 | 行数 | 职责一句话 |
|---|---|---|
| `mod.rs` | 46 | ui 门面：模块声明 + 对外 re-export（run/Page/widgets 动作组件） |
| `actions.rs` | 4 | 三个 gpui 全局动作（Quit/Undo/Redo） |
| `branding.rs` | 108 | 应用标识资产：托盘明暗主题 rgba 图标 + 窗口图标 |
| `component_gallery.rs` | 448 | feature 门控的动作组件原生评审面（不起音频，配套 example required-features） |
| `monitoring.rs` + `monitoring/` | 1242+1483 | 电平/频谱订阅与显示包络：legacy I/O 表 + 管理式 NodeMeters 共享生命周期校验；ballistics（动力学）、spectrum（ISO 266 网格）、table（订阅表 trait） |
| `observe.rs` | 27 | `gpui_test_observe` 中立测试钩子（页面不触达 shell） |
| `pages/` + 五页 | — | 工作区页面（见下节对照） |
| `presentation.rs` | 396 | AppSnapshot → 展示文本的纯函数投影（无状态、无派发） |
| `scroll_policy.rs` | 95 | 滚轮/触控板 → Pan/Zoom 的纯策略（Slint 时代移植） |
| `shared_frame.rs` | 115 | Tier H 几何通道：缓存面板 prepaint 发布、热层 paint 读取 |
| `shell.rs` + `shell/` | 1129+约5500 | 主窗口视图 StudioShell 与窗口机制：desktop（窗口生命周期/托盘 flyout）、chrome（工具栏+状态栏）、sidebar（检视器+库宿主）、panel（Tier C 缓存）、hot（Tier H 热层）、controls（共享积木）、project、autosave、preferences、fault_popover、native、capture/performance（gpui-test 专用）、tray_mixer、test_harness |
| `state.rs` + `state/` | 813+1072 | UiState：服务快照的 UI 侧投影存储 + 设备行查询；transport 候选指纹 |
| `theme.rs` + `theme/` | 283+100 | 全站 token（颜色/间距/字号/圆角/popover 度量）+ controls 别名 |
| `touchpad.rs` | 171 | Win32 精密触控板子类化桥（WM_POINTER 转发） |
| `tray.rs` | 172 | 系统托盘桥（tray-icon crate，事件→语义动作） |
| `widgets/` | 约2400 | 跨页组件：ActionButton/IconButton/导航按钮、fader（共享刻度）、meters（条形表）、overlay_scrollbar、scroll_step（滚轮分类）、anchored_popover + occluding_page（弹出层单源）、action_assets（图标/字体） |

## 五页结构一致性对照

| 页 | 入口 | 页状态位置 | 缓存策略 | 子模块 |
|---|---|---|---|---|
| Graph | `graph/page.rs::mount` | GraphViewState@page.rs + 手势/选区状态@graph.rs 根 | 页级 GraphPanel（ControlsKey 含 catalog/backend）+ GraphMeterLayer 热层 | 13 个子模块 + graph_canvas（纯投影）/graph_canvas_view（绘制+输入）两层 |
| Mixer | `mixer/page.rs::page` | MixerPageState@mixer.rs 根 | **per-strip** StripPanel（键驱动）+ MixerMeterLayer 热层 | model/page/routing/strip_controls |
| Matrix | `matrix.rs::page` | MatrixPageState@根 | **故意不缓存**（模块 doc 记载旧滚动缓存的两条失效理由） | 仅 tests |
| Devices | `devices.rs::mount→page` | DevicesPageState@根 | 页级 DevicesPanel | exports（含子目录 tests）、overrides |
| Settings | `settings.rs::mount→page` | SettingsPageState@根 | 页级 SettingsPanel（draft/key 文档化） | fields/menu/model/persistence/transaction |

评价：形状同型（根=状态+re-export，子目录=渲染/模型/测试），缓存策略差异均有文档化理由（per-strip vs 页级 vs 不缓存）。偏差集中在入口命名与 PageState 落点（AU-4）。Matrix 单文件页 458 行属合理规模，无需强拆。

**God 文件逐个判定**

| 文件 | 行数 | 判定 | 依据 |
|---|---|---|---|
| `pages/graph/tests.rs` | 3654 | 测试载体，见 AU-6 | — |
| `pages/mixer/tests.rs` | 3132 | 保留（测试载体，33 测试每个 50-150 行高密度验收） | 与 AU-6 同法可拆但更紧迫性低 |
| `shell/tests.rs` | 1813 | 保留 | 含 settings 套件（工作区级测试在 shell 合理） |
| `graph/node_inspector.rs` | 1311 | **拆**（AU-3） | 五块 concern；两巨函数 430/371 行 |
| `graph_canvas_view/nodes.rs` | 1292 | 内聚大，保留 | 纯画布绘制库一以贯之；`pub(in crate::ui)` 的四个 paint_* 是全 UI 画布工具（经 graph_canvas_view.rs re-export），可视为分层有意为之 |
| `mixer/model.rs` | 1276 | **搬迁**（AU-1/AU-5） | 滑杆机制=跨页内核；解析/格式化族并入 presentation |
| `monitoring.rs` | 1242 | 内聚大，保留 | 两族订阅+共享包络是其声明内聚点；已带 ballistics/spectrum/table 子模块；内联测试 458 行若嫌大可外移 monitoring/tests.rs |
| `shell.rs` | 1129 | 内聚大，保留 | 全部是 shell 核心（引导/构造/泵/Render/undo/焦点）；run()/run_app 可选下放 desktop.rs，收益小 |
| `shell/project.rs` | 1062 | **拆**（AU-2） | 四种 concern |
| `mixer/page.rs` | 1053 | 内聚大，保留 | 装配+StripPanel+热层+读出绘制，模块 doc 首段即解释分层 |

## 测试组织现状

- **两层门控**（有意设计，test_harness.rs:1-9 文档化）：`cfg(test)` 纯模型套件（graph_canvas 27、state 17、transport 12、settings/model 7、devices 2、exports 6 + panel/hot/shared_frame/ballistics 内联）跑默认 `cargo test`；`cfg(all(test, feature = "gpui-test"))` 无头窗口套件（graph 46、mixer 33、shell 24、sidebar 6、tray_mixer 4、devices 4、action_tests 4、settings 3、exports 3、matrix 2，合计约 129）跑 `--features gpui-test`。
- **共享 harness 单源**：`shell/test_harness.rs` 提供 spawn_service/test_shell/shell_with_state/settle_panels/wait_until/wait_for；`ui/mod.rs:42-45` 按 feature 精确 re-export。graph/tests.rs 顶部另有 8 个页内 helper（render/canvas_frame/add_node 等）被 46 个测试复用，setup 复用率良好。
- **断言密度高**：验收测试普遍「手势序列 + 几何断言 + 重建计数（design 17 hazard 6 的 rebuild_count 配对 paint 输出）」，如 mixer_publish_rebuilds_only_the_changed_strip。
- 拆分评估（AU-6）：graph/tests.rs 六簇可辨识、helper 已共享，转 tests/ 目录是低风险纯组织动作；mixer/shell 的 tests.rs 簇数更少（mixer 约 4 簇、shell 约 4 簇），收益递减，建议只动 graph。

## 与既往审计对账

| 既往项 | 本轮状态 |
|---|---|
| 2026-11-03 IN/GC/AS/ST 工单 | bug 类不在本轮范围；抽查可见 B-1/C-1 类修复痕迹（undo 清 drag、ControlsKey 含 catalog/backend —— graph/page.rs:30-40 注释明确） |
| E-1 i18n 双向护栏 | **已落地**：`src/i18n_guards.rs` 三护栏（键台账双向、占位符一致、debug 绊线 + allowlist） |
| E-2 / ST-4 绑定清单双写 | **已解决**：`graph/binding_choices.rs` 单源，binding_menu.rs 与 node_inspector.rs 共同消费 |
| PR#141-#157（i18n Wave4/5、token、WorkspaceView、anchored_popover） | 全部在基线内；MeterFault/UiFailure/ProjectMessage 已类型化并 paint 期翻译 |
| X-1 engine/app 双 level 目录 | 不在本子树 |
| 缓存/热层复杂度（已知开放项） | **正面复核**：三文件集中、trait 契约文档（CachedPanel::Key 的「时间与音频观测不得入键」纪律写在 panel.rs:143-146）、design 17/20/21 锚点贯穿；复杂度本身未降低，但可发现性优于「已知事实」的预期 |

## 维护性建议 Top 5（含权衡）

1. **抽跨页编辑内核（AU-1+AU-5+AU-7 合并执行）**：滑杆机制/点击输入族 → `ui/editors/`，observation 采样与全壳 update_node_meters → `monitoring/` 或 shell，解析/格式化族并入 presentation.rs。一次机械搬移 PR 消除 graph↔mixer 双向依赖与 `shell::mixer` 误导路径。权衡：diff 大、测试 import 需同步；不做的成本是每个新 slider 表面继续跨页穿透。
2. **拆 shell/project.rs（AU-2）**：menu.rs + messages.rs。最小成本解开 470 行菜单函数的评审困难。
3. **拆 node_inspector 的字段 schema（AU-3）**：builtin_fields.rs 子模块 + inspector() 按 kind 分函数。表驱动结构不动，只动文件边界。
4. **定五页组织约定（AU-4）**：以 graph 形状为范本，逐页对齐入口名与 PageState 落点；随日常触页 PR 顺手迁移，不单开战役。
5. **graph/tests.rs 目录化（AU-6）**：六簇子模块 + 共享 helper 的 mod.rs。纯组织，CI 引用路径需一次同步。

**明确不建议**：为行数拆 nodes.rs / monitoring.rs / shell.rs / mixer/page.rs（内聚成立）；强拆 matrix 单文件页；迁移 shell/panel|hot 到新命名空间（AU-8 仅记录，收益不抵 import 搅动）。
