# 子代理:图画布与连线交互只读审计(2026-11-03)

审计范围:PR #93 feat/node-ui(合并点 3321062)与 PR #94 graph-autosave(9e3a7bc)之后的画布与连线交互面——`ui/pages/graph_canvas.rs` 及其 `overlays/curves/frame/knob/layout` 子模块、`ui/window/graph.rs`、`ui/window/graph/{page,detail,library,binding_menu}.rs`;交叉只看接口:`pages/graph_canvas_view*`(命中/绘制闭包)、`window/graph/node_inspector.rs`、`window/graph/tests.rs`。基线 `fix/spectrum-standard-curve`(main + 2 个频谱提交,工作区干净)。只读审计,未改任何代码;本文件为唯一产出。

发现合计:**3×P2 + 4×P3 + 2 疑案待实测**;既往项(HC-1/HC-9/HC-10 等)逐一复核无回归,其中 HC-10 的修复面(挖空)漏了 tooltip/橡皮带两个新位点(见 GC-3,新位点而非原项回归)。

| # | ID | 级别 | 标题 | 位置 | 已亲证 |
|---|---|---|---|---|---|
| 1 | GC-1 | P2 | Ctrl+Z/Y 不取消画布手势:拎线拖拽中 undo 后松手,向 service 提交陈旧 EdgeId → 假 fault 进错误条 | window.rs:944/978、graph.rs:644-677 | ✓ |
| 2 | GC-2 | P2 | Tone/Delay 旋钮拖拽按指针移动派发结构性编辑:BindSource 无 undo 合并键冲刷 100 深 undo 栈,每条编辑重准备引擎 + preparing 轮廓整段闪烁 | knob.rs:127、graph.rs:385-404、service/graph.rs:560/210-226 | ✓ |
| 3 | GC-3 | P2 | fault tooltip 与橡皮带不经 HUD 挖空,且 hover 离开画布不清零:tooltip 停在工具条/总览图上方不消失 | page.rs:177-187/193-214、graph.rs:343-363 | ✓ |
| 4 | GC-4 | P3 | 连线拖拽悬停不动时,卡片表/频谱冻结(meter 帧让位缓存画布,而缓存只在 key 变化时重画) | page.rs:261-263 | ✓ |
| 5 | GC-5 | P3 | `on_bg_down` 不走 `begin_pointer_session`:左拖中按中/右键 → pan 与 knob/连线/框选手势并存互踩 | graph.rs:454-490、444-450 | ✓ |
| 6 | GC-6 | P3 | detail 窗口三处小疵:死 `height` 计算、dynamics 图无裁剪可越出圆角图框、EQ 拖点移出绘图区即停(无窗口级捕获) | detail.rs:51-52/540-556/370-424 | ✓ |
| 7 | GC-7 | P3 | `node_positions` 幽灵条目(已删节点)永不清理:长编辑会话单调增长并随 autosave 指纹常驻比较 | library.rs:372、graph.rs:569-575 | ✓ |

---

## GC-1 (P2) Ctrl+Z/Y 不取消画布手势:拎线拖拽中 undo 后松手提交陈旧 EdgeId,产生用户没造成的 fault

- 位置
  - `crates/moiren-app/src/ui/window.rs:944-950`(`cancel_active_drags` 只遍历 `slider_slots`,不含任何画布手势)
  - `crates/moiren-app/src/ui/window.rs:978`(`undo_redo_graph` 只调这一个取消;画布五手势 connection/node/pan/selection/knob 均存活)
  - `crates/moiren-app/src/ui/window/graph.rs:636-677`(`on_port_up` 拎线分支:无论候选如何,`drag.detached` 为 Some 就派发 `Reconnect{edge,..}` 或 `Disconnect(edge)`)
  - `crates/moiren-app/src/service/graph/edit.rs:149-158`(Reconnect 第一步 `get_edge(edge).map_err(...)`);`service/graph.rs:643-660`(`record_error_with_subject` → `snapshot.faults.push` + `last_error`,且仅在下一次**成功**编辑时清除)
  - 对照:`delete_graph_selection`(graph.rs:198)在删除后主动清 `connection_drag` —— 同类"键盘编辑撞上拖拽"路径里 Delete 处理了,Undo 没处理。
- 证据 / 触发序列(已亲证,代码路径级)
  1. 在占用输入上按下左键拎起连线(`on_port_down`,graph.rs:582-619):`connection_drag = { port: 源输出, detached: Some(edge E, 输入 I) }`;画布把 `dst_port == I` 的线从投影里滤掉(graph.rs:219-226)。
  2. 拖拽中按 Ctrl+Z:画布焦点在 `graph_focus`,键处理只拦 backspace/delete/escape(graph_canvas_view.rs:570-577),ctrl-z 冒泡到窗口 action → `undo_redo_graph` → 只取消推子槽 → journal 回滚到 E 不存在的状态。`connection_drag` 原样存活(拖拽状态不在 undo 的取消范围)。
  3. 松手:候选按当前世界计算,`on_port_up` 无条件进入 detached 分支 → 落在输入上派发 `Reconnect { edge: E(已不存在), .. }`,落在空处派发 `Disconnect(E)`。
  4. service:`edit()` → `edit_subject`(get_edge 失败 → `GraphSubject::Graph`)→ `edit_candidate` 里 `get_edge` 报错 → `record_error_with_subject(Graph, "…edge…")` → **snapshot.faults 多一条 Graph 主题 fault、`last_error` 置位**,直到下一次成功编辑才清。错误条向用户显示一条并非其操作导致的失败;图本身无损(Reconnect 原子性保证,9f97285)。
  - 附带同因路径:拖拽旋钮中 Ctrl+Z——undo 后下一次指针移动仍按按下时锚定的 `start_value` 派发编辑(graph.rs:385-404 无 undo 感知),拖拽把刚撤销的值原样压回去,"undo 不生效"直至松手。detail 窗口 EQ 拖点对已被 undo 删掉的节点派发 `SetBuiltin`(detail.rs:340-364)同属"陈旧 id 编辑 → 假 fault"家族。
  - EdgeId 复用别名风险(`counts.edge` 随 undo 快照回滚,graph.rs:336-345)在单指针交互下无法构成(按住左键期间没有第二条连线创建路径),理论存在、现不可达,不另立条。
- 影响:拖拽 + undo 的组合操作后出现一条无法理解的错误(fault 主题为 Graph,画布无卡可挂,只在错误条出现);undo 被进行中的旋钮拖拽覆盖。概率中等(功率用户的自然手势),无数据损坏。
- 修复方向:`undo_redo_graph` 改调 `cancel_active_gesture`(或让 `cancel_active_drags` 纳入画布手势集),与 Delete 路径对齐;`on_port_up` 的 detached 分支在派发前用当前快照校验 `get_edge(picked.edge)` 仍指向 `picked.destination`,失配即静默丢弃。

## GC-2 (P2) Tone/Delay 旋钮拖拽按指针移动派发结构性编辑:undo 栈被冲刷 + 引擎按编辑重准备

- 位置
  - `crates/moiren-app/src/ui/pages/graph_canvas/knob.rs:127-132`(`ToneFrequency::edit` → `GraphEdit::BindSource`;`:133-142` DelayTime → `SetBuiltin`)
  - `crates/moiren-app/src/ui/window/graph.rs:385-404`(on_bg_move 的 knob 分支:每个 mouse-move 事件 `control.drag(...)` 后立即 `state.dispatch(EditGraph)`,无节流/DispatchGate)
  - `crates/moiren-app/src/service/graph.rs:547-566`(`edit_affects_prepared_structure`:`BindSource | SetBuiltin | BindOutput | ClearBinding => true`)
  - `crates/moiren-app/src/service/graph.rs:210-226` + `:236-259`(`undo_key`:`BindSource` 落入 `_ => None` → `record` 的合并条件 `key.is_some_and(...)` 永假 → **每次派发一条 undo 记录**,`UNDO_DEPTH=100` 溢出即逐出最旧)
  - `crates/moiren-app/src/ui/pages/graph_canvas.rs:390-392` + `graph_canvas_view/nodes.rs:986-990`(preparing 集合非空 → 卡片轮廓转 warning,design 18 B1)
- 证据 / 机理(已亲证)
  - Gain/Pan 旋钮走 `SetGain/SetPan` → 有 live 映射(结构位 false)+ undo 合并键;**Tone 走 BindSource(无合并键、结构性)、Delay 走 SetBuiltin(有合并键、结构性)**——四个画布旋钮里两个是结构性编辑。
  - 拖一个 Tone 旋钮:mouse-move 以显示率到达(dispatch 入队),service 每 poll(33ms)应用一条 → 每秒约 30 条 journal 记录(1 秒拖拽 ≈ 30 条,超过 undo 栈 1/3;两三个长拖即可把用户真实历史逐出),且每条都把该节点塞回 `preparing_nodes`、触发引擎重新准备——整个拖拽期间卡片轮廓停在"准备中"的 warning 色,音频源节点被反复拆建。
  - DelayTime:undo 有 500ms 滑窗合并(`builtin:{node}` 键,连续拖拽整体一步),但结构位仍 true → 重准备/轮廓闪烁同款。
- 影响:用户可见(轮廓异常闪烁、拖拽期间引擎无谓重建、undo 粒度/历史损坏);对照面:检查器里 Tone 只有 ±10 Hz 步进按钮(node_inspector.rs:254-266),离散点击不暴露此问题——画布旋钮是新的连续暴露面。
- 修复方向:Tone 频率给独立值编辑路径(或至少给 `BindSource` 在 Tone 情形下的 undo_key + 结构位豁免/参数化);画布旋钮拖拽套用 mixer 的 DispatchGate 尾沿合并;或拖拽期间对 preparing 轮廓做"自己编辑中"的抑制。

## GC-3 (P2) fault tooltip 与橡皮带不挖空 HUD 覆盖层;hover 离开画布不清零,tooltip 悬停在工具条上方不消失

- 位置
  - `crates/moiren-app/src/ui/window/graph/page.rs:177-187`(热层 paint 顺序:`paint_graph_meter_frame`(挖空)→ `paint_box_select_band`(不挖空)→ `paint_fault_tooltip`(不挖空))
  - `crates/moiren-app/src/ui/window/graph/page.rs:193-214` + `pages/graph_canvas_view/nodes.rs:808-881`(tooltip 只钳制在画布 bounds 内,不 `carve(overlays)`)
  - `crates/moiren-app/src/ui/window/graph.rs:340-363`(hover 只在画布 input div 的 mouse-move 里更新;指针移上工具条/总览图/库把手后事件不再到达,`hovered_node` 保持)
  - 结构性根因:热层是缓存页面的**后继兄弟**(page.rs:116-127),画在工具条/总览图/库把手之上;41c4d2d 给表条补了 `OverlayRects`+`carve`,tooltip/橡皮带两个同层绘制物没进挖空集合。
- 证据 / 触发序列(已亲证)
  1. hover 残留:悬停一张带 fault 的卡片(tooltip 显示)→ 不按键,把指针移到顶部的编辑工具条上——工具条的按钮 hitbox 接管 mouse-move,画布 `on_bg_move` 不再触发,`hovered_node` 不清零 → tooltip 原地不消失,且因热层画在工具条之上,tooltip 压在工具条图形上。总览图(右下)与库把手同理。
  2. 位置越界:故障卡停在画布左上/右下时,tooltip 本体(clamp 只对画布 bounds)可与工具条/总览图矩形重叠,从工具条"后面"画到"上面"。
  3. 橡皮带(条件性,见疑案 S-GC-1):框选拖拽期间指针越过工具条,若框架按 hit-test 路由 move,带子冻结在工具条上方且盖在工具条之上。
- 影响:视觉污染 + 交互困惑(tooltip 像被钉住);与 HC-10(已修)同机制的新位点。
- 修复方向:`paint_fault_tooltip`/`paint_box_select_band` 与表条一样按 `overlays.rects()` 挖空;画布 input div 挂 hover-leave(或对 hit_test 返回 None 时清 `hovered_node`)。

## GC-4 (P3) 连线拖拽悬停不动时卡片表/频谱冻结

- 位置:`crates/moiren-app/src/ui/window/graph/page.rs:261-263`(`if shell.connection_drag.is_some() { return; }` 跳过整个表帧,注释假设"连线拖拽每次移动都重建页面")。
- 机理(已亲证):`connection` 含 cursor,在 `ControlsKey` 里(page.rs:32-36)→ 指针移动时每步重建、paint 闭包重跑、表条新鲜;但**按住不动**时 key 不变 → 缓存面板"replays last frame's paint"(panel.rs:153),而热层又已让位 → 有音频时卡片表/频谱冻结在最后一次移动的读数,球状跳变在下次移动/松手时补齐。自愈型视觉瑕疵。
- 修复方向:跳过条件改为"仅当本帧页面确实重建"(如以 rebuild 计数/paint 代际为门),或连线拖拽也走热层并把连线 ghost 提到热层之上。

## GC-5 (P3) `on_bg_down` 不走 `begin_pointer_session`:左拖进行中按中/右键,双手势并存互踩

- 位置:`crates/moiren-app/src/ui/window/graph.rs:454-490`(背景按下直接设置手势,不先清场;`_` 分支只清 `background_click`/`selection_drag`);对照 `begin_pointer_session`(graph.rs:106-121)是其它所有按下路径的前置。
- 机理(已亲证):按住左键拖旋钮(knob_drag)时按下中键 → `pan_grab` 与 `knob_drag` 并存;on_bg_move 中 pan 分支(graph.rs:444-450)清 `node_drag` 但**不清 knob_drag/connection_drag** → 移动指针同时平移相机并持续派发旋钮值编辑。左键框选中按中键同理(pan 与橡皮带并存,世界矩形随相机漂移、选择集反复变)。所有手势在按钮全松后经 `!held` 分支复位,无持久卡死。
- 影响:罕见组合下的怪异交互(平移时旋钮值跟着变);无状态泄漏。
- 修复方向:`on_bg_down` 入口先 `begin_pointer_session`(非 hand-tool 时等价于清场),或 `_` 分支补清 knob/connection/node 拖拽。

## GC-6 (P3) detail 窗口三处小疵

- `crates/moiren-app/src/ui/window/graph/detail.rs:51-52`:算出 `height` 后 `let _ = height;` —— 死代码残留(真正的定位在 54-57 重算)。
- `detail.rs:540-556` + `:571-623`:dynamics 曲线容器只有 `.rounded().bg(...)`,无 `overflow_hidden`;`to_y` 把超出 ±60 dB 窗的输出(gate floor −80 dB、大 makeup)映射到图框外,线段画出圆角图框、压在留白上(EQ 编辑器有 `.overflow_hidden()`,detail.rs:374,dynamics 没有)。
- `detail.rs:379-391`:EQ 拖点的 `on_mouse_move` 只在输入 div 内触发,无窗口级捕获 —— 按住拖出绘图区(仍在对话框内)时拖点停在边缘不再跟随;对照 mixer 推子的 kit 捕获行为。
- 修复方向:删死变量;补 `overflow_hidden`;输入层用框架捕获(或至少对 dialog 全域挂 move)。

## GC-7 (P3) `node_positions` 幽灵条目永不清理

- 位置:写入点 `graph.rs:569-575`(`on_port_down` 给**所有**节点 `or_insert`)、`library.rs:372`(spawn)、`graph.rs:305`(选卡);全仓无按存活节点裁剪的路径(`sync_node_name` 只裁 `selected_nodes`,window.rs/node_inspector.rs:1273)。
- 机理(已亲证):任意一次端口拖拽即给全图节点落位;此后删节点留幽灵条目,长编辑会话单调增长;`AutosaveStamp.node_positions`(autosave.rs:33)带幽灵参与每次 poll 的相等比较;落盘侧被 `project_document` 按存活节点过滤,无正确性影响。
- 修复方向:`sync_node_name` 同址顺带 `node_positions.retain`。

---

## 疑案待实测

### S-GC-1 gpui 按下期间 mouse-move 的路由决定 GC-3 带冻结/EQ 拖点停滞的实机表现
headless(`gpui-test`)可仿真按下后把指针移出元素 bounds:确认 move 是按 hit-test 路由(画布停更 → GC-3 的带冻结/工具条停滞成立)还是按下捕获路由(则 GC-3 只剩 tooltip 残留与越界两 facet,GC-6 第三点减轻)。既往 8 轮多次出现"headless 绿/实机坏",结论以实机为准。

### S-GC-2 Tone 旋钮拖拽的实机观感(GC-2 量化)
headless 已能仿真旋钮拖拽(tests.rs `graph_knob_wheel_drag...`);补一个带 trace 的用例:拖 Tone 1 秒,统计 journal 增量(预期 ~30)、`preparing_nodes` 覆盖时长(预期 ≈ 全程)、引擎重准备次数,以确认 P2 定级与修复优先级。

---

## 既往项复核(2026-10-31 → 本轮,限本分区)

| 项 | 本轮状态 |
|---|---|
| HC-1 画布手势无切页/失活复位 | **FIXED 未复发**:`set_page` → `cancel_active_gesture`(window.rs:653-661)、窗口失活观察器(window.rs:474-481)、`frames_wanted` 页面门(window.rs:558-567);新手势(connection/knob)均已纳入 `cancel_active_gesture` 与 `drag_active`(window.rs:571-577);测试 `set_page_cancels_canvas_gestures_and_restores_the_poll_pump`、`window_deactivation_cancels_canvas_gestures` 在册 |
| HC-9 fit 相机注册 meter 需求只增不减 | **FIXED**:`on_camera_fit` 改为按本帧实际相机"替换式"剔除(graph.rs:500-543);测试 `zoomed_graph_camera_releases_offscreen_meter_demand_and_fit_restores_it` |
| HC-10 表条画上库把手 | **FIXED**(OverlayRects+carve,41c4d2d;测试 `graph_hud_overlays_register_their_rects_for_the_meter_frame`),但同层 tooltip/橡皮带未进挖空集合 → **GC-3 为同机制新位点,非原项回归** |
| S-2 缓存面板内滚动容器(含 node-library-scroll) | 状态不变:library 滚动仍在缓存 LibraryPanel 内、无 track_scroll/offset-in-key 处理,既往疑案身份保留,不另立新条 |
| SD-2 `typing_focus`/`keep_graph_focus` 抢焦 | 本分区复核:keep_graph_focus 已尊重 typing_focus(window.rs:993-1005);画布 keydown 只拦 backspace/delete/escape,无新编辑器家族漏网 |
| PR #94 对画布状态触碰 | `apply_project_presentation` 经 `set_page` 取消手势、清 `pending_node_spawn`、恢复 positions/camera(project.rs:562-622);autosave 指纹含 camera/positions(test 覆盖 pan-only 落盘);测试 shell 禁用 autosave 路径,无实盘写入 |

## 已核对、未见问题的高风险机制(供后续审计跳过)

- **ControlsKey 完整性**(page.rs:22-64):页面内容实际读取的每个输入都在键内——graph(ArcSnapshot,值等价兜底)、generation/phase/desired、positions、camera、canvas、selected_nodes/selected_route、connection(port+cursor+detached)、hand_tool、sidebar_open、compact、preferences;locale 由宿主承担;`node_search` 经 `observe_nested`;`pending_node_spawn` 与 `selected_node` 只被嵌套的 LibraryPanel/LibraryRailPanel 消费且 LibraryKey 已含(sidebar.rs:151-165,含注释);`hovered_node`/meter 读数按设计走热层。"键漏字段/装错概念"两轮复发类在本键上未发现第三例。
- **拎线改道状态机**(5fe4b28 + 9f97285):拾取读实时 service 边;投影滤掉被拎线使 Displacing/Cycles 判定在"减去该边的世界"上计算(正确);落回原口恢复原边、落他口单原子 Reconnect(失败半程不动原边,edit.rs:149-171)、落空断开、Esc 恢复;两端方向、占用位移、环拒绝、body 落点均有测试在册(tests.rs 74-333、2501)。
- **命中测试 z 序与半径**:节点逆序(后画者先测)+ 单卡内 port→expand→knob→chip→card 优先级,与绘制次序(线在下、卡按序、后卡盖上)一致;端口绘制半径 4×zoom 与 8px 命中半径在 zoom≤2 恰好相等、低 zoom 放宽属注释明示的权衡;线命中容差按屏幕像素且有分数 zoom 测试(`wire_hits_keep_a_screen_pixel_tolerance_at_fractional_zoom`、`hit_testing_respects_node_z_order_and_clips_offscreen_cards`)。
- **数值边界**:camera 全部 is_finite/clamp 守卫 + 非有限投毒/巨大图 fit/退化连线测试在册;`carve` 纯几何测试(不重叠、恰好覆盖、吞没);knob 0..1 归一/log 域往返测试;曲线采样器退化样本数测试;`paint_grid` 的 rem_euclid 步进无除零(zoom∈[0.25,2])。
- **binding_menu**(291dee5):节点删除即 `sync_binding_menu` 摘除(render 期,window.rs:758);kind 不可变故 `items()` 不会在有节点时返回 None;outside-click 的 inside 判定矩形与布局用同一组 left/top/width/height(实际高度被 `max_h(height)` 封顶,不会出现"可见行被判外部");`.occlude()` 阻断画布下层事件,菜单打开期间不可能并发拖动节点使锚点失效;Esc 经画布焦点路径关闭。
- **spawn/选中语义**(bcd1259):`finish_node_spawn` 使新节点成为全部选中;library 的 capture 期 `deselecting` 包装先清画布选中;`pending_node_spawn` 防重入、非有限落点拒收;两测试在册(`node_library_native_drop...`、`library_press_drops...`)。
- **框选**:选择在**事件期**应用(graph.rs:405-430,注释点明热层 paint notify 不可达的历史坑)、带子由热层逐帧画;`apply_box_select` 的命中对 `!drag.moved` 有门槛;`sync_node_name` 每 render 裁掉已删节点的选中项。
- **undo 合并**:SetGain/SetPan/SetBuiltin/SetLabel/SetNodeTint 均有键并在 500ms 滑窗合并(GC-2 的 BindSource 是唯一漏网,已单列)。
