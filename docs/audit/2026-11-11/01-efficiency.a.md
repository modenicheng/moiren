# 01 效率与渲染开销 — 初审 A（UI 渲染路径）

- 基线: main @ e2ed3d6a, 2026-11-11
- 范围: `crates/moiren-app/src/ui/**`（shell/panel·hot·chrome·sidebar·desktop·tray_mixer、pages/graph·mixer·matrix·devices·settings、monitoring、widgets/fader·meters、presentation、state）及直接喂 UI 的 notify/订阅路径（shell.rs pump_tick/pump_meters、pages/mixer.rs update_node_meters、service mailbox publish 侧）。
- 方法: 只读。先 `find+wc` 摸底（ui/ 131 文件 48.4k 行）；检索式 `\.cached\(`、`fn render|Render for`、`shape_text|shape_line`、`format!|to_string\(\)`（render 路径）、`cx\.notify\(\)`、`publish`、`clone\(\)`、`OBSERVATION_SAMPLE_CADENCE`、`request_animation|frames_wanted`；对照 docs/audit/2026-10-27/04-app.md、2026-10-31/03-hot-layer-bugs.md、2026-11-03/（01/02/04 分报告）、2026-11-09/03-app-ui.md 与 docs/designs/screenshots/mixer-render-isolation/README.md 逐项核现状。亲读深读文件：shell.rs、shell/{panel,hot,chrome,sidebar,desktop,tray_mixer}.rs、pages/{mixer,mixer/page,mixer.rs,matrix,devices,settings,graph,graph/page}.rs、pages/graph/{interactions,node_inspector(节选)}.rs、graph_canvas{,_view}.rs、graph_canvas_view/nodes.rs、graph_canvas/overlays.rs、widgets/{fader,meters,action_assets}.rs、monitoring.rs、state.rs、presentation.rs(节选)、service/mailbox.rs(publish 侧)。
- 对账口径：meter fps 审计实为 docs/designs/screenshots/mixer-render-isolation/（PR 世代随 10-27 轮），其"CPU draw p95 超 4.17ms 预算"基线问题已由 Mixer 渲染隔离 + per-strip 缓存关闭（该 README 稳态数据：draw p95 1.9–2.4ms，strip rebuilds=0）——下文不再作为开放项。

## 发现

### A-1 图画布手势热路径：每显示帧整页重建 + 全卡文本重 shaping + 世界全量重投影 [P1] [NEW]
- 位置: `crates/moiren-app/src/ui/pages/graph/page.rs:35-44,61-69`（ControlsKey 含 camera/positions/connection）、`pages/graph.rs:388-390`（page() 每次 render 调 `CanvasWorld::positioned`）、`pages/graph/interactions.rs:227-281`（node drag / pan 每次 mouse move `cx.notify()`）、`graph_canvas.rs:294-607`（positioned 每节点分配 ~10 个 String/Vec、每边 O(n) find、拓扑序、format!/t!、EQ/动态曲线卡 64 点采样 `curves::sample_curve_log_x`）、`graph_canvas_view/nodes.rs:15-53,73-105`（paint_label→shape_text 每卡 title/detail/knob 标签/值/chip 每次 paint 全部 shape；溢出标签截断走二分，最多 log2(n)+1 次 shape + `format!("{}…")`）。
- 证据: 拖卡、平移、缩放、拎线四类手势的输入都在 ControlsKey 里（camera、positions、connection、drag_guides 除外），每次指针上报（Windows 常见 125–1000Hz，gpui 折叠到显示帧）→ 键变化 → GraphPanel 整树重建：`CanvasWorld::positioned` 全量重投影（内容与手势无关的部分也重建），随后 `paint_canvas_layer` 重画网格/全部连线 Path/全部卡片，含每卡 2–6 次 `shape_line`。rubber-band 已经示范正确模式（带由热层 `paint_box_select_band` 画、选择离散提交，page.rs:263-293），同一手势族内唯独它不重建。
- 影响: 大图（30+ 节点）拖拽/平移期间每帧数百次堆分配 + 120+ 次文本 shaping + 64 点/卡的曲线函数求值；与 4.17ms/240Hz 预算同量级竞争，且全部是纯浪费（世界内容未变）。这是 UI 侧剩余的最大单点渲染开销。
- 建议方向: 两步解耦。①把 `CanvasWorld`（snapshot+已提交 positions+catalog 的函数）提升为 Arc 缓存在 shell 上，按指针等价复用，重建时只重投影布局；②把手势期偏移（node drag delta、拖拽吸附、连接虚线）从"烘焙进 positions/键"改为帧期增量，仿 rubber-band 走热层或 prepaint 增量应用；③给画布 paint 加文本 shaping 缓存（照抄 MixerMeters::ReadoutShapes 的"按文本键、封顶清空"模式，page.rs:761-807 已有范本）。
- 工作量: M–L

### A-2 Mixer 热层每帧每通道标签 shape_line + meter_spans 每通道 Vec [P2] [KNOWN-OPEN]
- 位置: `crates/moiren-app/src/ui/widgets/meters.rs:86-101`（"L"/"R"/"1".. 每帧 `paint_text`）、`widgets/fader.rs:115-136`（paint_text 无缓存 `shape_line`）、`widgets/meters.rs:225-256`（`meter_spans` 每调用 `Vec::with_capacity(4)`）。
- 证据: 2026-10-31 HC-8 指认两项；现状核对：读出 dB 文本已修（ReadoutShapes 文本键缓存，mixer/page.rs:753-807），但通道标签仍每帧走无缓存 `paint_text`，`meter_spans` 仍每通道每帧返回堆 Vec。16 条立体声 strip 在电平运动期间 = 每帧 32+ 次 shape_line（热层 canvas paint 每帧重跑，缓存面板救不了它）+ 每帧 ~32 次 4 元素 Vec 分配。
- 影响: 10-31 审计原判"shaping 主导每帧预算"的残余主项；240Hz 显示率下 ~7.7k 次 shape/s。
- 建议方向: 标签集合固定（L/R/1..N/"N ch"），做 per-(text,size) 小缓存或复用 ReadoutShapes；`meter_spans` 改返回栈上数组/写入调用方缓冲。
- 工作量: S

### A-3 PublishRef 整快照键族：不渲染 elapsed 的面板跟随 1Hz 时钟重建 [P3] [NEW]
- 位置: `shell/sidebar.rs:22,68,186,204`（InspectorKey/LibraryKey 以 `PublishRef<AppSnapshot>` 为键）、`shell/chrome.rs:201-226,254-270`（Toolbar/Statusbar 同法）、`pages/settings.rs:40,63`、`pages/devices.rs:31,54`、`shell/tray_mixer.rs:186,204`。
- 证据: service 侧 publish 仅在真实事件换 Arc（mailbox.rs:253-258 注释："requests, phase transitions, quantized clock ticks, tick-batched observation refreshes"），运行会话中量化时钟使快照 ≥1Hz 换新。指针判等键（PublishRef）于是让 inspector（node_inspector 全树）、library×2、toolbar、statusbar、settings/devices 页每秒全量重建，即便它们渲染的任何字段（节点参数/目录/设备行）都没变。对照：StripKey 已示范内容投影（`strip_content_eq`，mixer/page.rs:171-194）——"编辑 echo 只重建目标 strip"，但页面级键族没有同等待遇。编辑会话中每次 edit echo 也全量重建 inspector/library（graph 键族此时重建是应该的，inspector 不总是）。
- 影响: 稳态 1Hz × ~数百元素重建，绝对量小（~0.1ms/s 级）；但属"失效范围过大"类别，编辑密集会话放大，且每秒重建推翻"缓存面板键变化必有语义变化"的心智模型。
- 建议方向: inspector/library 键改内容投影（所选节点 entry + 编辑族标志 + preferences，快照 Arc 只留作故障帧投影的输入，比较走 node/edge 局部相等，StripKey 先例可直接抄）；toolbar/statusbar 体量小可不动。
- 工作量: M

### A-4 键捕获分配税：每次 shell notify 各面板克隆 positions/preferences/Draft [P3] [NEW]
- 位置: `pages/graph/page.rs:52-75`（ControlsKey::new 克隆 `positions: HashMap`、`preferences: AppPreferences`（含 override map、recent_projects Vec<PathBuf>）、`desired`、`selected_nodes`）、`shell/sidebar.rs:65-87`（InspectorKey 克隆 observation/selected_device/preferences）、`pages/settings.rs:59-73`（SettingsKey 克隆整个 `global: Draft`）。
- 证据: panel.rs:160-179 的 observer 在**每次** owner notify 上捕获键；Graph 页手势期间 notify 以显示帧率发生（与 A-1 同根放大）， Mixer 页拖拽经 publish 节流后每次 dispatch echo 一次。捕获本身不重建（键相等即 splice），但每次捕获固定付出 ~3–6 次中粒度克隆。
- 影响: 每帧 ~10 个面板 × 数次分配，绝对量小；拖拽期间与 A-1 叠加。键相等比较对 `ArcSnapshot<GraphSnapshot>` 在服务端每次 publish 换新 graph Arc（service/mod.rs:366 `Arc::new(graph)`）时退化为全值深度比较（每 publish 一次，图大时 ~μs 级，可接受）。
- 建议方向: 随 A-1/A-3 一起做：positions 只在拖拽键字段里存"被拖节点集合"（拖拽 delta 不进键）；preferences 改 Arc 化或按用到的字段投影（density/meters 已有先例）；SettingsKey 的 Draft 改 revision 计数（DevicesKey.device_revision 已示范）。
- 工作量: S–M

### A-5 Graph 热层每帧分配与小额 O(n²) [P3] [NEW]
- 位置: `pages/graph/page.rs:312,331-341,370`（每帧 `overlays.rects()` 分配 Vec、covered 检查对每节点扫全部后继节点 O(n²) 矩形相交、每节点 `carve()` 建 Vec 与中间 Vec）、`pages/mixer/page.rs:965-980`（悬停故障 strip 时每帧 `faults_for_node`→`Vec<CardFault>` 克隆 cause/hint String）。
- 证据: 热层 paint 每显示帧执行；overlays 通常 3 个、`carve` 每节点每帧 1+N 次 Vec 分配；covered 检查 n²/2 次 intersects（30 节点=435 次/帧，量级尚可）。
- 影响: UI 线程每帧几十次小分配；与仓库自己的"热层零稳态分配"口径（HC-14/SP-1 修复标准）不一致。
- 建议方向: `rects()` 结果与 carve 输出复用 scratch 缓冲（spectrum_scratch 先例 page.rs:356-369）；covered 检查可预计算"卡片遮挡对"随 CanvasFrame 缓存；banner fault 列表改借用投影。
- 工作量: S

### A-6 频谱体每帧重建网格与轴标签（31 hairline path + 4 次 shape_text/卡）[P3] [KNOWN-OPEN]
- 位置: `pages/graph/graph_canvas_view/nodes.rs:519-651`（paint_spectrum_body：3 dB 线 + 至 27 条十进频率线逐条 `PathBuilder` build/paint，"100/1k/10k"+FFT detail 四次 `paint_anchored_text`→`shape_line`），热层每帧每分析卡执行（graph/page.rs:374-392）。
- 证据: 2026-11-03 SP-6 原判（十进网格把 hairline 数 13→31）未修；热层与缓存层共用此 painter，故每动画帧全量重画。
- 影响: 每分析卡每帧 ~31 个小 path + 4 次 shaping；多分析卡 + 高刷下可感。
- 建议方向: 网格/标签/轴搬进缓存层（仅曲线随帧变化——与 SP-6 原建议一致），或单 Path 累积多段。
- 工作量: S–M

### A-7 Matrix 页每 publish 全量 O(端口²) 重建 [P3] [NEW]
- 位置: `pages/matrix.rs:31-203`（有意不缓存，模块头 1-9 记载两条失效理由；每 rebuild 全格 Button、每格 `format!("route-{ri}-{ci}")`、`t!` 无障碍标签、`db_text`、`name()` label clone）。
- 证据: 触发频率 = publish 率（运行会话 ≥1Hz）+ 每次点击；10×10 端口即 100 Button/次，`t!` 逐格查表。规模小时无感，图增长后每次 publish 的重建成本线性于端口数平方。
- 影响: 当前规模轻微；属结构性"无虚拟化列表"类别里唯一平方级位点。
- 建议方向: 不必恢复滚动缓存（模块头理由成立）；可把网格体做成行级 `div().id(row)` 复用或仅键格内容变化的 memo；或低优先搁置到端口数实际增长。
- 工作量: M

### A-8 观测读出 200ms 采样重建整个 inspector 面板 [P3] [NEW]
- 位置: `pages/graph/node_inspector.rs:545-547`（OBSERVATION_SAMPLE_CADENCE=200ms）、`pages/mixer.rs:99-135`（sample_inspector_observation 到期即改 `inspector.observation`→InspectorKey 变化→面板全量重建）。
- 证据: 选中任一带 meter 轨的节点且会话运行时，Peak/RMS 两个文本每 200ms 使整个 inspector 树（含 builtin_controls 全套字段）重建一次（issue #118 的修复选择了"进键"路线）。注释自认"every re-key rebuilds the panel"。
- 影响: 5Hz × inspector 全树；与 A-3 同一类（细粒度变化触发整面板），但频率更高。
- 建议方向: 观测块抽为 inspector 内的独立子缓存面板（或文本走热层/局部 splice），键只含 observation 快照；字段主体键保留低频语义变化。
- 工作量: M

## 已修复面登记（KNOWN-FIXED，一句话依据）
- PR#37 静止停帧：`FrameDemand`/`at_rest`/REST_DB（hot.rs:24-50、monitoring.rs:31-40、shell.rs:647-656），空闲会话零动画帧。
- PR#40 快照 churn：elapsed 量化整秒（service/mod.rs:385-388）+ publish 内容去重（mailbox.rs:253-258）+ iconic 短路（shell.rs:871-878）。
- PR#96/PR#163/订阅宽度：FFT 挪出音频线程；legacy 探针仅在"probe 可见且无 managed topic"订阅（shell.rs:728-745 `legacy_meters_shown`）；NodeSpectra 仅 Graph 页需求（pages/mixer.rs:53-79）——未发现订阅过宽新位点。
- PR#165/PR#98：滑杆 wheel/双击 commit 直写不回灌（fader.rs:179-217）；overlay 滚动条均为缓存面板兄弟（sidebar.rs:172-176、mixer/page.rs:535-550、controls.rs:187-200）。
- PR#104 缓存根尺寸：strip_body 根 `w_full().h_full()` 且注释立规（pages/mixer.rs:161-166,267-273）；settings/devices/toolbar 根均 `size_full`。
- PR#16/17/HC-1/HC-2：手势切页/失活复位（shell.rs:545-549,747-762；frames_wanted 页面谓词 shell.rs:647-656）。
- HC-3 PanelHost 空池常真：已修，`nested.is_empty()` 子句已删且注释立规（panel.rs:169-178）。
- HC-14 每泵克隆：注册集借用穿越（pages/mixer.rs:62-71）、ClipHold 无克隆 observe（monitoring.rs:377-396）、flyout 同法（tray_mixer.rs:140-148）。
- HC-7 readout 缓存无界：读出缓存改文本键天然无 per-node 条目（mixer/page.rs:753-779）。
- SP-1 热层曲线每帧分配：`spectrum_scratch` 复用缓冲（graph/page.rs:352-369），`NodeSpectrumReading` 仅 rebuild 期拷贝（graph/page.rs:413-427）。

## 正面登记（明确不要动的）
- PanelHost 键捕获在观察者 effect 内、render 零 owner 读（panel.rs:160-179,205-223）——这是"每 publish 不炸全缓存"的根基，勿移回 render。
- PublishRef/ArcSnapshot 双判等策略（panel.rs:17-64）：服务端内容去重快照用指针、可复用 Arc 的图快照用值+指针捷径，语义各自正确。
- `strip_content_eq`（mixer/page.rs:171-194）：忽略会话簿记、只比渲染输入，是整个 per-strip 缓存成立的条件。
- ReadoutShapes 文本键 shaping 缓存及其封顶策略（mixer/page.rs:753-807）：A-1/A-2 修画布文本时应照抄此模式，而不是 per-track 缓存（该文件注释已论证 per-track 在峰值下落期会全表 reshape）。
- 热层三处 surface 的 pump/需求仲裁（hot.rs + MixerMeters/GraphMeters/FlyoutMeters）：单一写者、静止交还控制泵，结构正确。
- 图标资产：路径全字面量（action_assets.rs:53-75）、内嵌字节、白名单双向测试；每帧 asset 路径解析不存在。

## 明确不建议（防过度工程）
- 不要把 toolbar/statusbar 移入 `.cached()` splice：拖拽区 hitbox 每帧重注册是硬约束（chrome.rs:307-323 注释已记,title bar 曾因此失灵）。
- 不要给 Matrix 页恢复"scroll 偏移入键"的缓存：模块头两条失效理由成立（matrix.rs:1-9）；A-7 的方向是行级 memo，不是整页缓存。
- 不要合并 canvas/DOM 电平表绘制载体：10-27 APP-3 已裁决（合并分段数学而非载体，且 meter_spans 单源已完成）；托盘静态双条用 DOM 是对的。
- 不要为设备列表/节点库（~14 模板+会话节点）上虚拟化：当前规模无收益；唯一候选是 Matrix（A-7）且非必需。
- 不要给 meter 标签引入 per-track 缓存或把 ruler 挪出缓存面板：ruler 在 StripPanel 内 rebuild-only（fader.rs:74-113），标签按文本键即可。
- 残留 ~12% vsync 唤醒属上游 zed#66221，不立项。

## Top5
1. **A-1**（P1）画布手势热路径整页重建/全卡重 shaping/世界重投影——大图交互流畅度的最大杠杆。
2. **A-2**（P2）Mixer 热层每帧标签 shaping + meter_spans 堆分配——HC-8 的最后残余，工作量 S。
3. **A-3**（P3）整快照 PublishRef 键族使 inspector/library 跟随时钟每秒重建——照 StripKey 先例做内容投影。
4. **A-8**（P3）200ms 观测采样重建整 inspector——观测块独立子面板。
5. **A-6**（P3，KNOWN-OPEN SP-6）频谱网格/轴标签搬出热层——多分析卡高刷下立竿见影。
