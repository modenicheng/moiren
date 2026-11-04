# ui/ 结构与可维护性审计（2026-10-31，main @ dc9b38a）

## 范围与方法

范围：`crates/moiren-app/src/ui/`（30,893 行，含测试与 gpui-test 专用代码；本轮重点为 design 17 "two-tier UI components"（PR #53，21d5b3b..dc9b38a）合并后的结构状态）。方法：只读；先读 `docs/designs/17-two-tier-ui-components.md` 理解意图分层，再通读两层架构核心（`window/hot.rs`、`window/panel.rs`、`shared_frame.rs`、`widgets/meters.rs`）、四个 HotSurface 实现（mixer/graph/tray/inspector）与全部 12 处 CachedPanel 接入，逐对并排比对疑似重复段；大文件按"职责数/耦合方向"评估而非行数。所有结论附 file:line 证据。未运行 cargo（主代理在跑）。

核对既往项：2026-10-27 审计 **APP-3（meter 分段策略双实现）已解决**——`meter_spans`（`widgets/meters.rs:220-251`）成为唯一定义源，全部 5 个表面经 `paint_meter_fill`/`paint_meter_fill_horizontal` + `paint_clip_lamp` 消费；DOM 版（`compact_stereo_meter`/`meter_bar`/`compact_horizontal_meter`）与 `VisibleSignal`/`PresentationDemand` grep 全空，设计 17 删除清单全部落地。TrayMixer 私有 `NodeMeters` 已并入 `UiState.node_meters`（`state.rs:221-224`），X-1 的 UI 半边随之收敛。

## 模块地图

| 路径 | 行数 | 职责一句话 |
| --- | --- | --- |
| `window.rs` | 1000 | StudioShell 组合根：~60 字段、实体装配（new 270 行）、根 Render、帧需求仲裁（frames_eligible/wanted）、meter 泵、焦点/undo |
| `window/hot.rs` | 181 | Tier H：HotSurface trait + HotLayerView 通用视图 + FrameDemand 仲裁器 + MeterTracks 共享几何表 |
| `window/panel.rs` | 369 | Tier C：CachedPanel trait + PanelHost + ArcSnapshot/PublishRef + NestedGate 节流 + cached_splice |
| `shared_frame.rs` | 115 | 缓存面板↔热层的带外几何通道（publish/read/reset/update） |
| `window/mixer/` | | 页面模块：`meter_frame.rs`(708)=页面装配（StripPanel/StripKey/缓存条带+MixerMeters 热面+页面布局）；`model.rs`(1138)=推子槽位/编辑器/分发节流；`strip_controls.rs`/`routing.rs`=条带控件与路由行；`performance.rs`=MeterTrace 计数器；`tests.rs`(2145)=两层验收 |
| `window/graph/` | | `meter_frame.rs`(302)=GraphPanel+ControlsKey+GraphMeters+橡皮筋绘制；`node_inspector.rs`(1340)=检查器树（表驱动）；`library.rs`=节点库；`compressor.rs`=压缩器滑杆 |
| `window/tray_mixer.rs` | 558 | 托盘 flyout 窗口：TrayPanel（缓存）+ FlyoutMeters（热面），复用 mixer::model 的行装配 |
| `window/sidebar.rs` | 461 | 右检查器/左库的宿主、弹簧动画、InspectorPanel/LibraryPanel(×2) 缓存、InspectorMeters 热面 |
| `window/chrome.rs` | 551 | 工具栏/状态栏（key 门控但非 cached_splice——拖拽区需每帧注册）+ Toolbar/Statusbar Panel |
| `window/matrix.rs`/`devices.rs`(+`devices/`)/`settings.rs`(+`settings/`) | 451/567(+623+179)/543(+363+261+191+92) | 各页内容 + 单键接入 PanelHost；settings 已按 model/fields/transaction/persistence 正确细分 |
| `window/error_strip.rs` | 187 | 故障条缓存面板（PublishRef 单键） |
| `window/desktop.rs`/`native.rs`/`project.rs`/`controls.rs`/`capture.rs`(+`capture/`) | 777/339/476/191/483(+139) | 桌面窗口生命周期与 33ms 生产轮询/Win32 杂项/项目菜单与装载/共享 DOM 控件底座/ui_capture 采集 |
| `pages/graph_canvas(.rs+4 子模块)` | 320+子 | 纯画布模型：CanvasWorld 投影、Camera、帧、布局、连线几何 |
| `pages/graph_canvas_view.rs`(+nodes) | 573+359 | 画布视图元素：canvas+输入层+overview+节点/表条绘制 |
| `pages/node_catalog.rs` | 279 | 库模板目录 |
| `widgets/` | | 共享底座：meters.rs(349)=唯一定义源+双取向填充绘制器；fader.rs=推子/刻度；actions/navigation/action_assets/scroll_step |
| `state.rs` | 753 | 设备行投影 + SelectionState + UiState（轮询/分发/传输验证）+ 显示标签；`state/` 仅测试 |
| `monitoring.rs`(+ballistics) | 1031(705 产码+326 测试) | 电平观测：MeterSubscriptions（legacy 源/汇）+ NodeMeters（受管节点）+ 共享 MeterSubscription + 弹道子模块 |
| `presentation.rs`/`theme.rs`(+`theme/`) | 294/4KB | 快照→文本投影；主题令牌 |
| 其余 | | `tray.rs` 托盘图标、`branding.rs`、`touchpad.rs`(Win)、`scroll_policy.rs`、`component_gallery.rs`(423，examples 专用)、`app_view.rs` 动作定义 |

**边界结论**：window/（壳与页面装配，知道 StudioShell）→ pages/（无壳的画布呈现）→ widgets/theme（共享底座）方向总体清晰，**不存在倒挂**：`window/graph.rs` 是控制器（组装 CanvasCallbacks），`pages/graph_canvas*` 是模型/视图，`window/graph/meter_frame.rs` 再包上两层机械。唯一的反向边见 ST-3。

## 发现清单

### ST-1 (P2) `meter_frame.rs` 名不副实：内容是"页面装配"，不是 meter frame
- 位置：`window/mixer/meter_frame.rs`（708 行）、`window/graph/meter_frame.rs`（302 行）
- 证据：mixer 侧 708 行中约 300 行与 meter 无关——`StripPanel`/`StripKey`/`strip_content_eq`（:59-217，缓存层键投影）、`page()`（:367-452，页面布局）；graph 侧含 `paint_box_select_band`（:186-213，橡皮筋选择带，纯手势绘制）与 `GraphPanel`/`ControlsKey`（:14-58）。文件头注释（两文件同文）自称"页面面板 + 显示率表层的装配"，`window.rs:832-833` 也以 `X::meter_frame::page(self, window)` 调用——调用的是"页"，住在"meter_frame"里。设计 17 之前该名准确（彼时只是覆盖 canvas），合并后职责膨胀未跟随改名。
- 建议：`mixer/meter_frame.rs` → `mixer/page.rs`，`graph/meter_frame.rs` → `graph/page.rs`（或 `panel.rs`，但与 `window/panel.rs` 易混，取 page.rs）。纯改名 + import/路径更新，`git log --follow` 保留历史。风险极低；收益是后来者按名找页面装配不再先读到 150 行键比较。

### ST-2 (P2) MeterTrace 统计全窗口却挂在 `MixerPresentation` 下
- 位置：字段 `mixer/model.rs:49-50`；定义 `mixer/performance.rs:10-24`；消费点 `window.rs:743`（shell_renders）、`chrome.rs:11,250`（toolbar/status_renders）、`graph/meter_frame.rs:78,151,157,281`（graph_renders/graph_frames/graph_meter_paints）
- 证据：`MeterTrace` 的 9 个计数字段中 6 个与 Mixer 无关（shell/toolbar/status/graph 系列），但所有页面都写 `shell.mixer.trace`——Graph 页的渲染计数写进"Mixer 呈现状态"，语义上跨了页。这是合并前的历史位置（mixer 最先有帧管线）未随架构上移。
- 建议：trace 移为 `StudioShell` 直属 `#[cfg(feature = "gpui-test")]` 字段，`performance.rs` 移到 `window/`（drop `mixer/` 前缀）。改动点 ~12 处机械替换 + 测试引用（`mixer/tests.rs`、`examples/ui_capture.rs`）。风险低；收益是性能台账的位置与其作用域一致，也为 Matrix/Devices 页将来挂计数器留对位置。

### ST-3 (P2) pages→window 唯一反向依赖：`TestObserve`
- 位置：`pages/graph_canvas_view.rs:16`（`use crate::ui::window::TestObserve;`），唯一调用点 :570；定义在 `window.rs:688-709`
- 证据：pages 层设计上不依赖壳（graph_canvas 全链无 StudioShell 引用，仅此一处为测试观测钩子向上伸手）。它使"pages 可整体被其他壳复用"的声称出现一个洞，也让 window.rs 又多一段与壳无关的代码。
- 建议：把 TestObserve（双 cfg 变体）移到中立模块（`ui/observe.rs` 或 `ui/widgets/`），window 与 pages 都向下引用。纯移动，风险低。

### ST-4 (P2) `display_label`/`source_name`/`output_name` 放在 state.rs，违背其自述边界
- 位置：定义 `state.rs:678-746`；state.rs 自己的文档（:210-213）写明"文本投影……在 `crate::ui::presentation`"；使用方 7 个文件（`mixer.rs:100`、`tray_mixer.rs:340`、`node_inspector.rs:96`、`mixer/routing.rs`、`graph/library.rs`、`pages/graph_canvas.rs`、`state.rs` 内部）
- 证据：这三个函数是快照→展示文本投影，与 `presentation.rs` 的 `phase_text`/`route_label`/`db_text` 同类（后者真的住在 presentation）。现状是同一类函数分居两处，且 state.rs 已 753 行。
- 建议：连同 `output_name` 一起移入 `presentation.rs`。纯移动 + import 更新；风险低。state.rs 剩下的（设备行投影、SelectionState、UiState、传输验证）保持一体（见"不要拆"清单）。

### ST-5 (P2) `cached_splice` 已退化为恒等函数（残留接缝）
- 位置：`window/panel.rs:282-287`；10 个调用点（error_strip/devices/settings/tray_mixer/sidebar×4/matrix/graph/mixer）
- 证据：函数体就是 `cached(entity)`——doc 自述 a11y live 分支被 gpui-fast 的 splice 语义取代后删除。10 个调用点的行为全部在各自的 `.cached(StyleRefinement…)` 闭包里，函数本身零行为。
- 建议：两个都站得住：(a) 内联删除（每个调用点少一层包装，名字不再暗示存在机械）；(b) 保留作为"缓存拼接唯一习语位 + hazard 8/a11y 教训的文档锚点"。倾向：下次触碰 panel.rs 时内联（把那段 doc 注释移到 PanelHost 的 Render impl 上），不单独开 PR。若未来 splice 需要重新分流（如非 a11y 的特殊路径），保留一个命名接缝有价值——这是判断点，不是硬伤。

### ST-6 (P3) MixerMeters::pump 与 GraphMeters::pump 逐行同构
- 位置：`mixer/meter_frame.rs:537-551` vs `graph/meter_frame.rs:145-163`
- 证据：并排比对——两段都是：upgrade shell → `shell.update` → `trace.begin(true)` → `Instant::now()` → `pump_meters(cx)` → `trace.add(elapsed)` → `frames_wanted(cx)`；唯一差异是 graph 侧额外 `graph_frames += 1` 与 `trace.finish(...)`（mixer 的 finish 在其 paint 尾部 :667）。约 12 行重复 ×2。
- 建议：可提 `fn pump_and_want(shell: &Entity<StudioShell>, cx: &mut App) -> bool` 收进 hot.rs 或 window.rs；省 ~10 行，代价是 trace 钩子需参数化（闭包或返回 elapsed）。收益边际，择机随 ST-1 改名一起做或不做。

### ST-7 (P3) reset_clip 点击闭包 ×3 与 mixer_edit ×2 近重复
- 位置：reset_clip：`mixer.rs:298-307`、`tray_mixer.rs:365-375`、`node_inspector.rs:1322-1329`；mixer_edit：`mixer.rs:262-275` vs `tray_mixer.rs:548-558`
- 证据：三个 reset_clip 闭包体都是 `owner.update(...→ state.node_meters.reset_clip(id); cx.notify())`，仅 owner 类型（StudioShell/TrayMixer）不同；两份 `mixer_edit` 逐行比对仅差 state 获取路径（`shell.state` vs `Entity<UiState>` 直持）。每份 ~10 行。
- 建议：提 `fn reset_clip_on_click(state: Entity<UiState>, id) -> impl Fn(...)` 与 `fn mixer_edit(state: Entity<UiState>, notify: Entity<impl …>, id, change)`——以 `Entity<UiState>` 为参即可覆盖三个 owner（mixer 侧传 `shell.state.clone()`）。放 `mixer/model.rs` 或 widgets。风险低；行为保持（notify 语义各 owner 已一致）。

### ST-8 (P3) "prepaint 注册 canvas"样板 ×5
- 位置：`mixer/meter_frame.rs:467-487`（track）、`:491-507`（readout）、`tray_mixer.rs:433-444`（meter_track）、`node_inspector.rs:1330-1339`（inspector_meter_track）、`mixer.rs:315-333`（可见性注册）与 `:385-407`（probe 注册）
- 证据：同构段：`canvas(move |bounds, window, _| { <向 SharedFrame/flags 写入 bounds+content_mask> }, |_,_,_,_| {})` + 尺寸链。每份 12-20 行，差异只在写入目标与条件。
- 建议：可提 `registering_canvas(frame, move |g, bounds, mask| …)` 小helper 收 ~8 行/处。当前 6 处合计省 ~50 行；抽象压力中等，非必须。若做，放 shared_frame.rs 旁。

### ST-9 (P2→择机) `frames_eligible` 保留页白名单，与设计 17 的"eligibility follows mounting"目标相悖（已文档化偏差）
- 位置：`window.rs:530-537`（`matches!(page, Graph|Mixer)` + 自辩注释：残留 demand 会饿死控制侧 poll）；`sidebar.rs:198-203`（InspectorMeters::pump 恒 false）；设计文档 :141-143 承诺去掉该子句
- 证据：后果是检查器表在 Matrix/Devices 页只随 33ms poll 重绘（无显示率动画），且"哪页挂了热层"的知识由白名单人工维护——新增挂热层的页忘记改白名单会出现与注释所述对称的 bug（热层 pump 永不运行/或 demand 饿死 poll）。
- 建议：不急于改行为；最小动作是让挂载点登记自己（如 `FrameDemand` 增加 page 级注册，或 `frames_eligible` 改查"本窗口当前挂载的热层"）。收益是消除一条靠纪律维护的不变量；风险中等（牵涉帧循环，需 ui_capture 验证）。列为择机项，做 ST-1/ST-2 时不必捎带。

### ST-10 (P3) `ArcSnapshot` 单一使用者；与 StripKey 的手写比较并存
- 位置：`panel.rs:22-40`（定义）；唯一消费 `graph/meter_frame.rs:22`；mixer 走另一条路：手写 `PartialEq` + `strip_content_eq`（`mixer/meter_frame.rs:90-217`）
- 证据：两种"键比较"机制并存——graph 用 ArcSnapshot 的指针快路径 + 派生 PartialEq；mixer 因需"仅本条带相关内容"的投影比较而手写。语义确实不同，不算重复；但 ArcSnapshot 若长期只有一个使用者，其存在价值=为下一个整页键准备。可保留（20 行，自测齐全）；若做 ST-1 改名时顺手无成本，否则不动。

### ST-11 (P3) `FrameDemand` 的字符串源注册表只有两个源
- 位置：`hot.rs:24-50`（BTreeSet<&'static str> + `METER_MOTION`/`DRAG` 两常量，仅 set/any 消费）
- 证据：注册表形态（BTreeSet+字符串）暗示可枚举/可追踪的扩展点，实际无迭代方、无展示方；两个 `bool` 字段即可等价表达。30 行换来的泛化目前无压力。
- 建议：保留亦可（改动无收益）；若 ST-9 落地时重构此处，顺势简化为显式字段。不要为此单独开 PR。

### ST-12 (P3) `component_gallery.rs` 无条件编译进生产 lib
- 位置：`ui/mod.rs:5`（无 cfg 的 `mod component_gallery`）+ `mod.rs:20` pub use；唯一使用方 `examples/component_preview.rs:3`
- 证据：423 行演示代码常驻编译单元与文档面。建议 feature 门控（如 `gpui-test` 或专有 feature）或移入 examples 共享。外观级。

### ST-13 (P3) 生产轮询与测试轮询是两份实现
- 位置：生产 `desktop.rs:419-493`（Desktop::poll）；测试 `window.rs:494-523`（StudioShell::spawn_poll，cfg(gpui-test)）
- 证据：并排比对：两者都实现 33ms 循环 + `state.poll()` + Exited→quit + `finish_project_load` + 同一表达式 `!shell.frames_wanted(cx) && shell.pump_meters(cx)`（desktop.rs:468 vs window.rs:511），但 desktop 版还有托盘动作、iconic 抑制、flyout 泵——测试版是其简化复写。两份轮询漂移则验收测试所验非生产路径。
- 建议：测试轮询改为复用一个共享的"单步泵"helper（把 meters_changed 表达式与 finish_project_load 收进 StudioShell 方法，两处调用）。择机；风险低。

### ST-14（核对项，非行动）"one meter painter"之后的残留平行度——已到合理下限
- 位置与并排比对结论：
  - 分段数学 1 份：`meters.rs:220-251`（meter_spans）✅。
  - 填充绘制器 2 个取向：`paint_meter_fill`（:151-178）vs `paint_meter_fill_horizontal`（:182-203）——同一循环换轴，注释自认互为旋转；参数化成 Axis 会引入分支，现状可接受，不合并。
  - clip lamp 1 份：`paint_clip_lamp`（:27-51）✅。
  - 表面足迹布局 4 份（真差异）：`paint_channel_meter`（meters.rs:53-109，竖向+L/R 标签+通道宽公式）、`paint_meter_bars`（`pages/graph_canvas_view/nodes.rs:158-189`，横向+世界坐标+缩放）、`paint_flyout_meter`（`tray_mixer.rs:526-545`，固定 7px×2）、`paint_inspector_meter`（`sidebar.rs:240-271`，4px 行×2）。并排看 flyout 与 inspector：两者的"有值→painter / None→背景 quad / lamp"三行式相同（tray_mixer.rs:539-544 vs sidebar.rs:255-269），仅 Bounds 推导与取向不同；mixer 的 None 分支不画背景（透明）。可再抽一个 `paint_track_or_empty(track, axis, level, bg, window)`（省 ~6 行/处、统一 None 语义），但收益小，列为可选。几何推导部分是真差异，**不应**继续合并。
  - HotSurface 骨架 4 份 impl（设计如此）：pump 重复见 ST-6；paint 各面 8 行升级样板，不值得抽。
- 结论：设计 17 声称的"单一 meter 绘制器"在数学与载体层面成立，剩余为表面皮肤，属合理差异。

## 重组优先顺序

1. **ST-1 + ST-2（同一 PR）**：`meter_frame.rs`×2 改名 `page.rs`；MeterTrace 上移 StudioShell、performance.rs 移 `window/`。全部机械改动，一次消除最大的两处"位置/名字≠职责"。
2. **ST-3 + ST-4（同一 PR）**：TestObserve 中立化；display_label/source_name/output_name 移 presentation.rs。两个小的归属修正，恢复自述边界。
3. **ST-7**：reset_clip / mixer_edit 小 helper 收敛（顺手级）。
4. 择机：ST-9（帧资格与挂载统一，需 ui_capture 验证）、ST-13（轮询单步泵共享）、ST-5（cached_splice 内联）。
5. 外观/不动：ST-6、ST-8、ST-10、ST-11、ST-12。

## 不要拆清单（内聚大文件，附理由）

- **`monitoring.rs`（1031 = 705 产码 + 326 测试）**：两张表（legacy `MeterSubscriptions` 源/汇探针 + 受管 `NodeMeters`）共享 `MeterSubscription`、生命周期校验与显示包络，弹道已独立为 `monitoring/ballistics.rs`。两张表并存是设计 17 的明确保留（capture tap 需要它）；拆文件只会把共享私有项变 pub。
- **`window.rs`（1000 行）**：StudioShell 是组合根，~60 字段对应 12 个缓存面板 + 4 个热层 + 图/混音/项目/偏好各域状态，各域行为已通过 `impl StudioShell` 分散在 mixer.rs/graph.rs/project.rs/node_inspector.rs 等页面模块。拆 struct 会强制把这些 impl 块的域字段公开化。最多做字段分组注释；不为行数拆。
- **`state.rs` 主体（移走 ST-4 的三个函数后 ~700 行）**：设备行投影 ↔ SelectionState ↔ candidate_spec/transport_request 是一条紧耦合链（候选规格验证直接消费设备行与选择身份），拆开制造循环引用。`state/` 目录只放测试是既有约定，无需改。
- **`graph/node_inspector.rs`（1340）**：`inspector()` 长但单职责（按 kind 组装检查器树），表驱动 `BuiltinFieldSpec`（:32-40, :1038-1153）已消除字段级重复；2026-10-27 审计同判。按 kind 拆文件会复制编辑器装配段。
- **`window/tests.rs`(1172)、`mixer/tests.rs`(2145)、`graph/tests.rs`(1563)**：验收测试按 harness 聚集、按测试名导航；拆分收益为零、干扰 `cargo test` 过滤习惯。
- **`MeterGeometry`（mixer）与 `MeterTracks`（hot.rs）两个几何寄存器**：值类型不同（mixer 需 per-track channels+legacy_input+readouts 表；共享表只有 bounds+mask）。统一需参数化，无调用方收益，反而模糊 flyout/inspector 的"单表"约定。
- **PanelHost / HotSurface / SharedFrame / NestedGate**：分别 12 / 4 / 3 / 1 个实现或使用方——前三个泛型压力充分；NestedGate 单使用者但为可独立单测的纯时序逻辑（panel.rs:322-368），是正当抽取，非过早抽象。
