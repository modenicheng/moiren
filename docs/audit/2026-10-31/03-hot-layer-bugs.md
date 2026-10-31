# 子代理 C：两层渲染/交互机制潜在 bug 猎捕（hot layer + cached panels）

审计范围：design 17 合并后（main @ dc9b38a）`ui/window/hot.rs`、`ui/window/panel.rs`、`ui/shared_frame.rs`、`ui/window.rs`、`ui/window/desktop.rs`、mixer/graph 两个 meter_frame、tray_mixer、sidebar、chrome、matrix、settings、devices、error_strip、`ui/monitoring*`、`ui/widgets/{fader,meters}.rs`、graph 手势状态机与相关测试。只读审计，未改代码。已修的 8 轮实机 bug 不重报；同机制新位点已报。

---

## HC-1 (P1) 画布手势状态在切页/失活/异步加载路径无复位，卡死的 DRAG 需求把 poll 侧 meter 泵永久短路

- 位置
  - `crates/moiren-app/src/ui/window.rs:631`（`set_page` 只改 page/workspace/菜单标志，不清任何手势）
  - `crates/moiren-app/src/ui/window.rs:552-557`（`drag_active` = 四个手势 Option 之一非空）
  - `crates/moiren-app/src/ui/window/desktop.rs:468`
    ```rust
    let meters_changed = !shell.frames_wanted(cx) && shell.pump_meters(cx);
    ```
  - `crates/moiren-app/src/ui/window/graph.rs:89-103`（手势复位只有 `begin_pointer_session`/`on_cancel`/canvas 鼠标事件三处）
  - `crates/moiren-app/src/ui/window/project.rs:462-475`（异步项目加载完成会 `set_page`，同样不清手势）
- 证据 / 触发序列
  1. Graph 页按住**中键**平移画布（`pan_grab = Some`）；保持中键按住，用左键点 WorkspaceTab 切到 Matrix（`set_page` 不调用 `begin_pointer_session`，Matrix 页没有 canvas，中键抬起落在 Matrix 页背景上无人消费）→ `pan_grab` 永久为 `Some`。同样的路径：rubber-band 拖拽中异步项目加载完成并 `set_page`（`project.rs:463`）；Alt+Tab / Win+M 中途丢失 mouse-up 时手势同样卡住（回到 Graph 后需一次 canvas 上的鼠标移动才自愈，`graph.rs:276-303` 的 `!held` 分支是唯一自愈点，但它挂在 canvas 的 `on_mouse_move` 上，卸载后收不到）。
  2. 卡住后 `drag_active()` 恒真 → `frames_wanted()` 恒真 → `desktop.rs:468` 的短路使 **poll 侧 `pump_meters` 永远不再执行**；而 Matrix/Devices 页没有挂任何 hot layer（`window.rs:831-836` 只在 Graph/Mixer 页挂），于是两个泵（帧泵 + 控制泵）同时失效：主窗口所有 meter 冻结在最后一帧读数（连 legacy 输入/输出表也停在 `update_points` 的最后一次结果），tray flyout 因自带泵不受影响。
  3. 切回 Graph 页后：rubber band 仍按陈旧屏幕坐标画在热层（`graph/meter_frame.rs:186-213` 每次 paint 直接读 `shell.selection_drag`），帧循环以显示器速率空转（`pump` 返回 `frames_wanted`=true → `request_animation_frame`），直到用户在 canvas 上按下或移动鼠标。
- 影响：全窗口 meter 冻结（用户可见）；显示率帧循环空转烧 CPU（与第三轮实机"page that never stood down"同类但根因不同）；残留 rubber band 假象。
- 修复方向：`set_page`、`apply_project_presentation`、以及一个新增的窗口失活钩子（`on_window_activation(Deactivated)`）统一走 `on_cancel` 等价的手势清零 + `sync_drag_demand()`；或者 `frames_wanted` 给 DRAG 需求加"当前页仍挂 canvas"谓词，避免 poll 侧短路饥饿。

## HC-2 (P1) MatrixKey 漏 `density`：设置里改密度后矩阵页冻结在旧行高（#48 冻结类的新位点）

- 位置
  - `crates/moiren-app/src/ui/window/matrix.rs:16-21`（`MatrixKey { snapshot, selected_route, scroll, compact }`）
  - `crates/moiren-app/src/ui/window/matrix.rs:75-79`（渲染消费 `shell.density == Density::Compact ? 36. : 48.`）
  - `crates/moiren-app/src/ui/window/settings/transaction.rs:181`（`self.density = draft.density;` —— 密度是运行时可改的，保存即生效）
- 证据 / 触发序列：会话空闲（无 publish）→ 打开 Settings → Appearance → Density 改 Comfortable/Compact → Save。`shell.density` 变更并 notify，但 MatrixKey 的四个字段全没变（`compact` 是视口 <1200 的布局标志，不是 density），观察者判 key 相等 → 矩阵面板不重建 → 行高、行数视觉密度停在旧值，直到下一次 publish / 选路 / 滚动。ControlsKey、InspectorKey、StripKey 都带 `preferences: AppPreferences`（含 density），唯独 MatrixKey 没带任何 preferences。
- 影响：矩阵页显示与设置不一致（运行中会话因 ≥1Hz 的 elapsed publish 会在约 1 秒内自愈，空闲会话则一直错）。
- 修复方向：MatrixKey 增加 `density`（或整个 `preferences`，与其他页对齐）。

## HC-3 (P1) `PanelHost` 观察者的 `|| host.nested.is_empty()` 使无嵌套模型的面板键控彻底失效，退化为每次 notify 重建

- 位置：`crates/moiren-app/src/ui/window/panel.rs:214`
  ```rust
  if host.key.as_ref() != Some(&key) || host.locale != locale || host.nested.is_empty() {
      ...
      cx.notify();
  }
  ```
- 证据：`observe_nested` 返回空 Vec 的面板——`MatrixPanel`（matrix.rs 无覆写）、`SettingsPanel`（settings.rs 无覆写）、`ErrorStripPanel`（error_strip.rs 无覆写）、`LibraryRailPanel`（sidebar.rs:156-176 无覆写；注释自述"no search"）——其 `host.nested` 恒为空，`nested.is_empty()` 恒真，于是**宿主 shell 的每一次 notify 都 `cx.notify()`**，key 比较形同虚设。该子句本意是首次初始化（"订阅还没抓到"），但对合法无嵌套面板是永久条件。
- 触发序列 / 量化：Graph 页会话运行且有音频时，poll 里 node meter 的 raw levels 每个 33ms 都变（`monitoring.rs:440-447` `changed |= raw != raw`）→ `cx.notify()` 约 30Hz → 挂载中的 `LibraryRailPanel`（以及有故障时的 error strip）以 ~30Hz 全量重建；Matrix 页上每次 publish（运行中 ≥1Hz）+ 每次用户点击都重建整个 O(端口²) 网格（`matrix.rs:131-208`）。这正是 design 17 第 4 步验收"per-panel rebuild counters flat across poll notifies"所声称已消除的行为；`mixer_publish_rebuilds_only_the_changed_strip` 等测试只覆盖了有嵌套订阅的面板，测不到这批。
- 影响：机器可见的 CPU 回归（第三轮实机"烧一核"同象、不同因）；缓存对这批面板完全失效，后续任何"key 漏输入"类 bug 在这些面板上被重建频率掩盖而不易暴露。
- 修复方向：用一个显式 `nested_captured: bool`（或 `key: Option<...>` 已有的 None 判空足够承担首帧初始化语义）替代 `nested.is_empty()`；`host.key.is_none()` 已经是"首次"判据，`nested.is_empty()` 可以直接删。

## HC-4 (P2) Settings 弹层是缓存面板，但渲染消费 `viewport_size()`、`pending_request`、`project_message`，均不在 SettingsKey

- 位置
  - `crates/moiren-app/src/ui/window/settings.rs:28-33`（`SettingsKey { snapshot, section, global, device }`）
  - `crates/moiren-app/src/ui/window/settings.rs:118`（`let height = (viewport_size().height - 40.).max(260.)` → `.h(px(height.min(800.)))`）
  - `crates/moiren-app/src/ui/window/settings.rs:135-138`（点击外部关闭的 `inside_dialog` 判定在**事件时刻**用**新鲜** viewport 重算对话框矩形）
  - `crates/moiren-app/src/ui/window/settings.rs:491`（diagnostics 消费 `state.pending_request`）、`:504`（消费 `shell.project_message`）
- 证据 / 触发序列：打开 Settings → 缩放窗口（变小）：弹层高度冻结在旧值（内容被裁或留白）；此时点击弹层底部边缘，事件侧的 `inside_dialog` 用新视口算出的矩形与画面上旧高度的弹层错位——点在弹层可视区域内会被判为外部而关闭设置（或反之）。pending_request / project_message 变化同理不触发重建，Diagnostics 节的"控制反馈"行与项目消息行停在打开时刻。
- 影响：边界条件下（打开期间 resize / 恰有请求在途）错误关闭或错误显示。
- 修复方向：高度改由 live 层包裹（或 viewport 进 key）；`pending_request`、`project_message` 进 key。

## HC-5 (P2) legacy 探针标志 `meter_points:(bool,bool)` 是"后画者覆写"共享槽——同设备双 Source/Sink 条带时可见表被清成 false

- 位置：`crates/moiren-app/src/ui/window/mixer.rs:385-407`
  ```rust
  if input { flags.0 = visible; } else { flags.1 = visible; }
  points.set(flags);
  ```
- 证据：`meter_probe_eligible` 的判定是"条带 = 期望 spec 的 source/sink"（mixer/meter_frame.rs:319-339），并不保证唯一。若图中有两个 Source 节点绑定同一输入设备（或两个 Sink 绑定同一输出端点），两条带都挂 probe 画布；prepaint 按条带顺序执行，**最后注册的一条覆盖标志**。触发序列：添加第二个绑定相同设备的 Source → 滚动让其中一条完全离开视口（probe 画布仍 prepaint，`visible=false`）→ 标志被置 false → 留在视口内那条的 legacy meter 订阅被释放（`pump_meters` 里 `visible && input_shown` 为 false），可见的表变空/隐藏。这正是已知 SliderState 共享槽（e220fde）的同类机制，新位点。
- 影响：需要同设备双端点图（是否可构造取决于服务端校验，见疑案 S-6）；一旦成立即用户可见的表消失。
- 修复方向：标志改为计数/按 NodeId 记录注册者（`BTreeMap<NodeId, bool>` 折叠），或 probe 画布只置 true、由 `refresh_observation_nodes` 负责清零。

## HC-6 (P2) `SliderSlot.dragging` 无卸载/失焦复位：拖拽视图中途消失后，该推子的快照写回被永久抑制

- 位置：`crates/moiren-app/src/ui/window/mixer/model.rs:731`
  ```rust
  if !slot.dragging.get() && slot.state.read(cx).value().end() != value {
      slot.state.update(...set_value...)
  }
  ```
- 证据：`dragging` 只在 `SliderEvent::Change` 置 true、`Release` 置 false（`model.rs:685-700`）。若拖拽中该 slider 视图卸载（切页带走条带、窗口最小化、或 Alt+Tab 后 mouse-up 落在别的窗口且 kit 不补发 Release——kit 行为见疑案 S-3），`Release` 永不到达，`dragging` 卡 true：此后所有 publish 的写回都被跳过，主窗口该推子_thumb 停在拖拽值，flyout/inspector 移动同一增益时主窗口不跟随，直到用户再拖一次该推子。条带推子、send、inspector 推子、compressor、tray 全部经 `sync_slider_range` 共用此机制。
- 影响：边界条件（拖拽中失活/卸载）后 UI 显示与服务状态脱节。
- 修复方向：strip 面板 `on_rebuild`/页面卸载路径复位对应槽的 `dragging`；或给 `dragging` 记时间戳，超过一个 dispatch 间隔无 Change 即视为失联。

## HC-7 (P2) `MixerMeters::readout_cache` 永不清理已删节点条目

- 位置：`crates/moiren-app/src/ui/window/mixer/meter_frame.rs:519`（`BTreeMap<NodeId,(String,Option<ShapedLine>)>`）与 `:633-648`（只 `entry(id).or_default()` / 覆写）。
- 证据：节点删除后条目永留（含一条 shaped line）；长时间编辑会话中随节点增删单调增长。同类：`MeterGeometry.tracks/readouts` 由 `StripPanel::on_rebuild` 按本条带清理（`:256-259`），无泄漏——readout_cache 是唯一漏网。
- 影响：缓慢内存增长 + 陈旧节点 id 复用时（NodeId 若复用）一帧旧文本。
- 修复方向：paint 前按 `frame.tracks` 键集裁剪，或 `on_rebuild` 同步清理。

## HC-8 (P2) 热层每帧重新 shape 通道标签，且 `meter_spans` 每条轨道每帧堆分配

- 位置：`crates/moiren-app/src/ui/widgets/meters.rs:81-96`（`paint_text` → `shape_line` 每次调用都新建 ShapedLine）；`meters.rs:220-251`（`meter_spans` 返回 `Vec<MeterSpan>`，`paint_meter_fill` 每通道每帧一次）。
- 证据：第二轮实机修复把峰值读数改成了"文本变了才 reshape"（meter_frame.rs:630-648 注释明说 shaping 主导了每帧预算），但通道标签 "L"/"R"/"1".. 仍走无缓存 `paint_text`；一个 16 轨 Mixer 在 60fps 下即 16×通道数次 shape_line/帧。
- 影响：显示率下 UI 线程预算回归（无错误像素）。
- 修复方向：标签（固定小集合）做 per-(label,size) 缓存；`meter_spans` 改栈上 `[MeterSpan;4]` 或复用缓冲。

## HC-9 (P2) `on_camera_fit` 用"拟合相机"注册 node_meter_points，且只增不减

- 位置：`crates/moiren-app/src/ui/window/graph.rs:402-419`（`fitted.project_rect(card).intersects(viewport)` → `node_meter_points.extend(...)`）+ `graph/meter_frame.rs:83-88`（on_rebuild 才清表）。
- 证据：注册用的是 `CanvasFrame::new` 里算出的**自动 fit 相机**（`graph_canvas_view.rs:339-343` 传入 `on_camera_fit(&frame.camera, ...)`），fit 视图包含全部节点；用户放大后离屏节点仍持续注册订阅，Graph 页停留期间永不释放（仅页切换清理，`window.rs:782-785`）。结果：放大到只看 1 个节点时，全部节点的窗口仍被排空、ballistics 仍在推进，`at_rest` 要等所有离屏节点也衰减到底——"该站 down 不 down"的轻度复发（比第三轮的"永不 rest"温和：静音最终仍会 rest）。
- 影响：放大态下帧循环/泵做了不可见功；离屏有信号时页面不站 down。
- 修复方向：注册改用 `shell.camera`（用户相机）投影，或注册时同时做视口剔除。

## HC-10 (P2) Graph 页 meter 覆盖层可能画到 library 探出把手上方

- 位置：`crates/moiren-app/src/ui/window/graph/meter_frame.rs:229-277`（覆盖层 mask 只裁到 `frame.bounds` = canvas host 矩形）；`sidebar.rs:385-394`（library 折叠把手 `.right(px(-20))` 探进 canvas 区域）；`graph.rs:572-593`（页面 row-reversed 使把手画在 canvas 之上，但 `graph_meter_frame` 作为 controls 的**后继兄弟**画在包括把手在内的整页之上）。
- 证据：把节点拖到画布最左沿，其 meter 落在把手覆盖的 20px 内：#50 修复只处理"更晚画的节点卡片遮挡"，未处理把手这一非节点遮挡者 → 覆盖层条形/夹灯画在把手之上。
- 影响：视觉瑕疵（条压在把手图形上）。
- 修复方向：`covered` 判定并入把手矩形，或覆盖层整体右移让出把手带。

## HC-11 (P2) `can_start_transport` 指纹把 latency_profile 折叠成"是否默认"布尔

- 位置：`crates/moiren-app/src/ui/state.rs:541-551`（fingerprint 末项 `latency_profile == default`）。
- 证据：三个档位（Compatible/Balanced/Low，settings.rs:388-394），从非默认 A 切到非默认 B 时指纹不变，`configured_engine_project` 若对两档验证结论不同，工具栏 Start 按钮的可用性沿用陈旧结果，直到 revision/采样率/缓冲变化。
- 影响：边界条件下 Start 状态错（多一次点击或误禁用）。
- 修复方向：指纹放进 profile 的判别值（如 `as u8`）。

## HC-12 (P2) flyout：单声道节点画一根死条；窗口宽度在条带数变化后不更新，横向滚动位于缓存面板内且未做 scroll-under-cache 处理

- 位置：`crates/moiren-app/src/ui/window/tray_mixer.rs:526-545`（恒画两根：`for (channel, clip) in clips.iter().enumerate()`，mono 第二根恒空条）；`desktop.rs:250-251`（窗口尺寸只在打开时按 `strip_ids().len()` 计算，之后不随图变化调整）；`tray_mixer.rs:204-210`（`overflow_x_scroll` 在缓存的 TrayPanel 内，无 track_scroll/notify/offset-in-key，对照 design 17 hazard 7）。
- 证据 / 触发序列：flyout 开着时在主窗口加节点超过约 5 条 → 条带溢出但窗口不变宽 → 需要横向滚动，而该滚动容器在缓存子树内未做处理（实机表现见疑案 S-2）。mono 节点（如 Tone? 由通道数决定）第二根条恒为 BORDER 空条 + 灰灯，被用户读作"右声道无声"。
- 影响：误读 + 溢出不可达（若 S-2 证实滚动失效）。
- 修复方向：按 `channels.min(2)` 画条；窗口宽度跟随 publish 调整；滚动随 S-2 结论处理。

## HC-13 (P2) `shell.gain_slider` / `pan_slider` 是成套但从未渲染的死状态机

- 位置：`crates/moiren-app/src/ui/window.rs:238-283`（创建 + 订阅 Change→写 state→request_parameters）与 `:790-808`（每次渲染做值同步 `set_value`）。全仓库无任何视图渲染这两个实体（grep 无第二个消费点）。
- 影响：无直接错误；每帧空同步、订阅悬挂，且`if value != gain` 的浮点直等写回一旦未来接上视图就是 HC-6 同类（无 dragging 守卫）的坑。建议删除或接上视图时补守卫。

## HC-14 (P2) 每泵克隆：`update_node_meters` 克隆 `node_meter_points`，`NodeMeters::update_at` 克隆 `seen` 表

- 位置：`crates/moiren-app/src/ui/window/mixer.rs:30`（`self.node_meter_points.borrow().clone()`）；`crates/moiren-app/src/ui/monitoring.rs:410`（`for (id, _) in self.seen.clone()`）。
- 证据：两条都在显示率帧路径上（每帧每窗口各一次，BTreeSet/BTreeMap O(n) 分配）。
- 影响：每帧堆分配（UI 线程，非 RT 违规，但与"显示率预算"目标相悖）。
- 修复方向：`seen` 遍历改为索引快照或 `retain`+二次遍历；可见集用 `&BTreeSet` 借用穿过 `state.update`（或 Rc）。

---

## 疑案待实测

### S-1 遮挡恢复后可见性标记卡死（已知疑案）——根因假说与验证

- 事实链：`presentation.visible` 的唯一写者是 gpui 的 `observe_window_visibility` 回调（`window.rs:466-474`，且 `minimized = !visible` 与其同源）和 `prepare_close` 的 `hide_to_tray`；`frames_eligible`（`window.rs:530-537`）以此为门。render 里另读 `window.is_visible()`（`window.rs:771`）但**从不写回** `presentation.visible`——两套可见性可以永久分歧。最小化路径有自愈（`desktop.rs:458-464` 轮询 `window_is_iconic` 并在 restore 转换时强制一次 notify），可见性路径没有对应轮询。
- 根因假说：Windows 平台在窗口被完全遮挡/切换（DWM cloaking 或快速 alt-tab 的 show 事件合并）时把 gpui 的 visibility 事件置 false，而恢复侧的 true 事件在快速转换中丢失/合并；应用层没有从 `window.is_visible()`（或原生 `IsWindowVisible`）自愈的路径，`presentation.visible` 永久 false → `frames_eligible` false → 帧循环不再武装、poll 侧 `pump_meters` 以 `visible=false` 空转 → 表全部停摆。拖拽旁路（`pump_meters` 的 `|| drag_active()`）只能救进行中的手势。
- 验证：`MOIREN_CAPTURE_METERS=1` + `ui-profiler` 跑 Mixer 页音频；用另一最大化窗口完全覆盖 Moiren ≥2s 后快速 alt-tab 回来；观察 `meter_paints` 是否停止、`presentation.visible`（临时日志）是否为 false；对照组为慢速恢复（正常 WM_SHOWWINDOW 路径应恢复）。
- 修复方向：render（或 desktop poll，仿 `main_window_minimized` 的 iconic 自愈）里做 `if window.is_visible() && !self.presentation.visible { self.presentation.visible = true; self.presentation.minimized = false; cx.notify(); }`；也可在 Windows 下轮询 `IsWindowVisible`/`IsIconic` 双信号。

### S-2 缓存面板内"未处理"的滚动容器实机行为（hazard 7 不变量 vs headless 测试的矛盾）

design 17 hazard 7 明言缓存面板内滚动容器必须做 track_scroll+wheel-notify+offset-in-key，且 Exports 页因该处理仍有残留问题而**故意保持 live**。但以下未处理位点仍然存在：settings 弹层 `settings-scroll`（settings.rs:222-233）、Devices 非 Exports 页 `device-list`（devices.rs:155-162）、node library `node-library-scroll`（library.rs:202-206）、flyout 横向条带（tray_mixer.rs:204-210）。反证：`expanded_library_bounds_its_scroll_area_and_scrolls_to_lower_templates`（tests.rs:1103）在 headless 里滚动成功。两者矛盾未决：headless 的 splice 行为可能与实机不同（前 8 轮已多次出现 headless 绿/实机坏）。验证：实机逐一滚轮滚动上述四处，确认 (a) 滚动是否生效、(b) 滚动后缓存内按钮/行的点击命中是否仍准（hitbox 是否随偏移重放）、(c) 滚动后 mixer 几何类注册（若有）是否刷新。任一不成立即按 hazard 7 补处理或转 live。

### S-3 gpui-kit Slider 在视图卸载/窗口失活时是否补发 Release（HC-6 的前置事实）

取 headless shell，`simulate_mouse_down` 在某 send 推子上 → 切页（`set_page(Page::Graph)`）→ `simulate_mouse_up`：观察 `SliderEvent::Release` 是否仍到达该 slot（kit 是否用窗口级监听）。若不到达，HC-6 升级为实机可复现 P1；若到达，HC-6 降为纯防御项。

### S-4 a11y 激活瞬间与缓存命中帧

现行机制：`cached_splice` 恒等函数（panel.rs:282-287），依赖 gpui-fast 的 splice 以 `a11y::stale` 自闸——激活后重建直至 a11y 节点录得，之后从缓存记录重放。代码层未发现卡死窗口（locale 变化经观察者强制重建；面板卸载则不渲染）。剩余风险在框架：Narrator 开启瞬间 + 恰逢 meter-only 帧（根重绘但面板 splice）时，屏幕阅读器读到的节点值可能是一帧旧的 ballistics 之外的静态值。验证：开 Narrator → Tab 遍历 Mixer 条带读推子值，同时 `MOIREN_CAPTURE_METERS` 抓帧比对；再在拖拽推子期间重复（此时重建被 NestedGate 节流至 33ms，读到的值最多滞后一个间隔——属预期，确认无"永不更新"即可）。

### S-5 弹簧收展 inspector 时缓存几何 mask 的时效

`panel()`（sidebar.rs:356-371）动画的是外层 sleeve 宽度，内层内容定宽；inspector 的 meter 轨道 canvas 在**缓存面板**内注册 `bounds+content_mask`（node_inspector.rs:1315-1337），而收展不触发面板重建（InspectorKey 无 inspector_open）。动画期间热层用注册时的旧 mask 画条，可能出现条画出收拢中的 sleeve 外/收拢后残留一瞬。验证：音频运行时反复点 inspector 折叠把手，`MOIREN_CAPTURE_METERS` 抓帧看动画 300ms 内条形是否越界；另测"会话停止（无 publish）时关再开 inspector"，确认轨道空条（BORDER 底）是否出现——若 mask 冻结在零宽则连空条都不画。

### S-6 同设备双 Source/Sink 是否可构造（HC-5 的前置）

在图上添加第二个 Output 节点并绑定与主 Sink 相同的端点（或第二个同设备 Source），确认服务端 `GraphSnapshot` 校验是否放行；放行则 HC-5 成立并可按其触发序列复现"可见条带 legacy 表消失"。

---

### 附：核对过、未见问题的高风险机制（供后续审计跳过）

- 峰值保持 marker 衰减：`advance` 的 `anchor = held_at+hold` 且 `.max(previous)`（ballistics.rs:77-83）保证过期后按帧增量线性衰减，不累计绝对时长；测试只覆盖单步但数学上正确。
- `NestedGate`（panel.rs:98-127）：Arm 单发、fire 被 resume 的 leading rebuild 正确压掉、armed 状态必被消费。
- `DispatchGate`（mixer/model.rs:69-100）：首拍即发、尾拍合并、release 清零，均有测试。
- `SharedFrame::update` 的读者持有复制语义与 Empty no-op（shared_frame.rs:50-75）符合"按条带累积、整表重置会洗掉兄弟"的设计。
- 推子槽共享：strip（`mixer.faders`）/inspector（`inspector_faders`）/tray（flyout 自有）/compressor（仅 inspector 一处）/sends（仅 strip 一处，routing.rs:342）各一视图一槽，e220fde 机制无新违例。
- 相机数学（camera.rs）：zoom/pan 全部 `is_finite`/`max(1.)` 守卫，无除零。
- chrome 工具栏/状态栏为 live splice（chrome.rs:261-286 注释 + 无 `.cached()`），`window_control_area(Drag)` 仅存在于 live chrome——已知坑无缓存内复发。
- Mixer resize 几何：`mixer_fader_small_drag_after_resize_tracks_the_new_geometry` 表明框架对分配边界变化的缓存视图会重跑 prepaint（slider bounds 更新），meter 几何注册同路径。
