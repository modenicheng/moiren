# 子代理 C：检查器 / 编辑器族 / 缓存键分区只读审计（repo-steward）

审计基线：分支 `fix/spectrum-standard-curve` = main(3321062, PR #93 feat/node-ui) + 2 个频谱提交（ed93c58、e9387d2，仅触 spectrum.rs / nodes.rs 频谱轴与曲线、capture.rs、graph/tests.rs 频谱断言——属 04 分区，本轮仅浏览确认无检查器面改动）。工作区干净。

审计范围（分区文件 + 机制文件）：

- `crates/moiren-app/src/ui/window/graph/node_inspector.rs`（1338 行；PR #93 **未改**，最后触点 8f75701/PR #74 世代，`git diff 3321062^1 3321062 -- <路径>` 为空，任务描述中"新大文件"不确——它是 SD-1 时代的旧文件，但仍是本分区核心）
- `crates/moiren-app/src/ui/window/graph/compressor.rs`（223 行，同上非 #93 改动）
- `crates/moiren-app/src/ui/window/graph/detail.rs`（623 行，**#93 新增**：节点详情窗，根级 overlay）
- `crates/moiren-app/src/ui/window/graph/binding_menu.rs`（304 行，**#93 新增**：绑定 chip 菜单，根级 overlay）
- `crates/moiren-app/src/ui/window/sidebar.rs`（574；#93 改 65 行：InspectorKey 增 editing 族字段、observe_nested 单一游走化）
- `crates/moiren-app/src/ui/window.rs`（1038；#93 改 45 行：`text_input_entities` 单一游走、`slider_slots`/`cancel_active_drags`、knob_drag/detail_node/binding_menu 字段与根级 overlay 挂载）
- `crates/moiren-app/src/ui/state.rs`（783；#93 +5 行）、`window/graph.rs`（#93 +345：knob/chip/expand 手势）、`window/graph/page.rs`（ControlsKey + graph 热层 fault tooltip）、`window/panel.rs`（缓存机制本体）、`pages/graph_canvas.rs`（positioned 投影）、`pages/graph_canvas/knob.rs`（#93 新增，旋钮值数学）、`service/graph.rs` + `service/graph/edit.rs`（编辑校验/undo 合并）、`window/mixer/model.rs`（编辑器族共享机制）

方法：全静态亲证（file:line + 代码摘录 + 机理链），未跑 cargo。先复读 docs/audit/2026-10-31/04-state-data-bugs.md 的 SD-1（observe_nested 漏族）/SD-2（typing_focus 漏族）失败模式，重点核对 PR #93 新族（绑定 chips 菜单、旋钮拖拽、EQ 详情编辑器、故障 tooltip、新检查器面板）与既有枚举/缓存机制的契合。

**结论先行**：SD-1/SD-2 的枚举修复本身扎实——`text_input_entities()` 单一游走（window.rs:916-924）同时供给 `typing_focus`（:902-908）与 `InspectorPanel::observe_nested`（sidebar.rs:85-128），#93 新表面无文本输入，无新漏族。但找到了同一失败模式的**另一个入口**：嵌套池的**捕获时序**前提被压缩器族打破（IN-1，P1，首选压缩器节点键入盲打）；以及 SD-7 同型在 #93 旋钮/EQ 拖拽上的复发（IN-2）。键维度上 ControlsKey 仍缺 catalog（#93 的绑定 chips 把暴露面扩大了，IN-3）。

---

## 发现表

| ID | 级别 | 标题 | 位置 | 状态 |
|---|---|---|---|---|
| IN-1 | P1 | 压缩器输入实体"渲染期创建"违反嵌套池捕获时序前提：首次选中某压缩器节点后的第一段键入盲打（SD-1 同签名，新机制） | window/compressor.rs:61-65 / window.rs:763 / panel.rs:174-176 | 已亲证（静态机制链完整；附实机确认步骤） |
| IN-2 | P2 | 旋钮拖拽与 EQ 详情拖拽未纳入 undo 取消路径：拖拽中 Ctrl+Z 被下一次指针移动覆盖（SD-7 同型复发于 #93 新族） | window.rs:933-950 / graph.rs:385-404 / detail.rs:383-391 | 已亲证 |
| IN-3 | P2 | Graph 页 ControlsKey 缺 `catalog`（及 `audio_settings.backend`）：catalog-only 发布下图页绑定 chips 与默认标题冻结，Mixer 页键却含 catalog | graph/page.rs:22-41 / pages/graph_canvas.rs:435-492 / presentation.rs:286-312 | 已亲证（机制），影响面一半为 #93 前既有 |
| IN-4 | P2 | 绑定 chip 菜单与检查器 picker 硬编码 `AudioBackend::Wasapi`：ASIO 会话下列出 WASAPI 行、永不列 ASIO 驱动（#93 把既有缺陷复制到新表面） | binding_menu.rs:87-90,154-157 / node_inspector.rs:272-275,365-368 | 已亲证 |
| IN-5 | P2 | 旋钮/EQ 拖拽每个鼠标移动事件派发一条 GraphEdit，无滑杆族的 33ms 合并：控制侧编辑风暴（每条=快照整克隆+journal+publish） | graph.rs:385-404 / detail.rs:332-367 / mixer/model.rs:79-93 对照 | 已亲证 |
| IN-6 | P2 | 源/汇绑定选项列表在检查器与 chip 菜单两处手工双份维护（"same order" 靠人肉约定，无共享函数） | node_inspector.rs:214-402 / binding_menu.rs:59-187 | 已亲证 |
| IN-7 | P2 | 节点详情窗（EQ 拖拽编辑器、tint、dynamics 曲线、Esc/外点关闭）零测试覆盖——#93 三大新交互面中唯一无回归网者 | window/graph/tests.rs（全表无 node-detail 用例） | 已亲证 |
| IN-8 | P2 | 检查器 Runtime observation 数值（Status/Peak/RMS）不在 InspectorKey：两次发布之间冻结在重建值（pre-#93 行为，本轮记录在案） | node_inspector.rs:575-634 / sidebar.rs:18-38 | 已亲证 |
| IN-9 | P2 | `delete_graph_selection` 清 node_drag/connection_drag 但不清 knob_drag：删除后 DRAG 帧需求滞留，烧帧至指针回到画布产生一次无键移动 | graph.rs:198-199 / window.rs:571-577 | 已亲证 |
| IN-10 | P3 | detail.rs 三处死绑定（`let _ = height/close/shell;`）——迭代残留 | detail.rs:51-52,86,288 | 已亲证 |
| IN-11 | P3 | detail/binding 覆盖层各自的 Escape `on_key_down` 在焦点模型下是死路径（焦点恒在画布，Esc 实际走 canvas `on_cancel`）——防御性重复，注释未说明 | detail.rs:97-105 / binding_menu.rs:253-261 / graph.rs:137-139 | 已亲证 |
| IN-12 | P3 | 检查器节点类型标签直接 `format!("{:?}", node.kind())` 上屏：英文 Debug 枚举名（"SpectrumAnalyzer"…）绕过 t!（APP-9/SD-11 类断层） | node_inspector.rs:169 | 已亲证 |
| IN-13 | P3 | EQ 编辑器拖拽限定在绘图区 div 的 `on_mouse_move`，指针拖出绘图区（仍在对话框内）值冻结，回来才续动；无窗口级捕获 | detail.rs:379-391 | 已亲证 |
| IN-14 | P3 | 压缩器字段清空/乱输入 → `parse::<f64>().unwrap_or(NAN)` 派发 NAN → 服务端整批拒绝并 record_fault → 英文错误串上屏（无状态损坏，纯 UX/i18n） | compressor.rs:90-105 / service/graph/edit.rs:44-61 | 已亲证 |
| IN-15 | P3 | `hovered_node` 只在无键鼠标移动时重算：卡片被 fit/undo 挪走后 tooltip 指向陈旧节点，直到下一次指针移动 | graph.rs:339-363 / graph/page.rs:193-214 | 已亲证 |

统计：**P1×1，P2×8，P3×6**。

---

## 详述

### IN-1 (P1) 压缩器输入族的嵌套池捕获时序洞：首次选中后的键入盲打

这是 SD-1（"缓存检查器内键入不上屏"）的**同签名复发，但入口不是枚举漏项，而是池捕获与实体创建的时序**——SD-1 修复建立的设计前提被压缩器族的惰性创建打破。

机制前提（三段代码的对质）：

1. **池的捕获时点 = key 变化的通知效果期（先于渲染）**，panel.rs:228-247：

   ```rust
   let subscription = cx.observe(&owner, |host, owner, cx| {
       let key = host.panel.key(owner.read(cx), cx);
       ...
       if host.key.as_ref() != Some(&key) || host.locale != locale {
           host.key = Some(key);
           host.nested = host.panel.observe_nested(&owner, cx);   // <- 池在此捕获
   ```

   gpui 的 observer 在触发 notify 的 update 结束后的效果冲刷期运行，早于下一帧渲染。panel.rs:174-176 的注释把这一点写成了不变量：

   > "the pool's entities are created by the owner's update path **before it notifies**, so capture time always sees them."

2. **压缩器输入实体在渲染期创建，且只创建"当前选中的压缩器节点"的**，compressor.rs:61-77：

   ```rust
   let Some(node) = snapshot.graph.nodes().iter().find(|node| {
       node.kind() == NodeKind::Compressor && self.selected_node == Some(node.id())
   }) else { return; };
   ...
   let input = self.compressor.inputs.entry((id, parameter)).or_insert_with(|| ...)
   ```

   该函数只被 `StudioShell::render` 调用（window.rs:763，全仓唯一调用点）——即**晚于**观察者效果。实体按 `(node, parameter)` 惰性创建、跨选择保留（:56-60 只清死节点）。

3. **压缩器字段没有 editing 旗标**（不像 Fader/Balance/NodeGain/NodePan 族由 `editing_*` 入 InspectorKey）：点入字段只改焦点，不改任何键字段。

失败时序（静态亲证，逐事件推演）：

1. 选中压缩器节点 A（首次）：`on_select_node` → `selected_node = Some(A)` → `cx.notify()`（graph.rs:264-318）。
2. 效果冲刷：InspectorPanel 观察者 → key 变（selected_node）→ `observe_nested` 捕获池 → `text_input_entities()` 里 `compressor.inputs` **尚无 A 的实体**（它们在下一帧渲染才创建）。池对此后 A 的字段是盲的。
3. 帧渲染：`sync_compressor_controls` 创建 A 的 10 个输入实体；检查器面板重建，A 的字段**可见**。
4. 用户点入 Threshold 字段（仅焦点变化，无键变化、无 shell notify——Input 内部按下被拦截不冒泡到根提交监听），键入 "−30"：每次击键 notify 的是 InputState 实体——不在池中 → 无 `notify_nested` → 缓存面板不重建 → **字符不上屏**（SD-1 亲证过的同一显示机制：gpui-component `Input` 是 RenderOnce，缓存子树 replay 不重跑闭包）。
5. Enter → 提交订阅（挂在 shell 上，独立于池）正常派发 → 编辑产生 publish → PublishRef 变化 → key 变 → **池此刻才重新捕获**（A 的实体已存在）→ 此后 A 的键入恢复正常。

即：**每个压缩器节点的第一段编辑会话是盲打，直到第一次 Enter/任何键变化事件**。运行中的会话有 ~1Hz 的 elapsed 量化 publish 兜底（约 1 秒内字符批量上屏）；**空闲会话（编辑候选图的常见状态）无任何 publish，盲打窗口无限期**。其他文本族不受影响：value/balance/node_gain/node_pan 的行覆盖**全部节点**且打开编辑器必翻 `editing_*` 键字段（开编辑器那一刻池就重捕，实体早已存在）；`node_name`/`node_search`/export 草稿在 shell 构造时即存在。滑杆族（inspector_faders/compressor.sliders）同样存在该窗口，但拖拽 Change 以 33ms 合并派发、每次派发产生 publish 重建面板，节奏与 `NESTED_REBUILD_INTERVAL` 相同，视觉不可辨。

回归网缺口：`window/tests.rs:1230-1270` 的嵌套重建断言只测 **Balance** 族（真实点击打开编辑器后 set_value 断言 `inspector_renders` 递增）；`graph/tests.rs:582` 的压缩器测试直接操纵实体 + PressEnter，只断言服务端值，不触及上屏。压缩器族恰好是唯一无 editing 旗标兜底的族，恰无覆盖。

修复方向（任一）：(a) 压缩器字段引入与其它族一致的编辑旗标入 InspectorKey；(b) `sync_compressor_controls` 改在选中变化的 update 内创建实体（满足 panel.rs:174 前提）；(c) 池捕获兜底——面板 render 时比对"渲染到的实体集 vs 池集"，缺失则补订阅。配套补一条"首选压缩器节点键入 → inspector_renders 递增"的回归测试（照 window/tests.rs:1230 模板，去掉点击开编辑器一步）。

实机确认步骤（静态链已闭合，仍建议一条）：空闲会话（未 Start）→ 从库中生成 Compressor 节点（生成即选中）→ 点入任一字段键入 → 观察字符是否不可见，Enter 后整值出现。

### IN-2 (P2) 旋钮/EQ 拖拽未纳入 undo 取消路径——SD-7 同型复发于 #93 新族

- SD-7 修复（window.rs:942-950）把"撤销压倒进行中的拖拽"实现为取消**滑杆槽**：

  ```rust
  /// Cancels every in-flight slider drag so a journal step cannot be
  /// overwritten by its gesture (audit SD-7).
  fn cancel_active_drags(&self) {
      for slot in self.slider_slots() {          // faders/sends/inspector_faders/compressor.sliders
          if slot.dragging.get() { slot.cancel_drag(); }
      }
  }
  ```

  `undo_redo_graph`（:954-987）在派发 UndoGraph 前调用它。但 #93 的**卡片旋钮拖拽**（shell 级 `knob_drag`，graph.rs:749）和**详情窗 EQ 拖拽**（无状态、按 `pressed_button` 逐 move 派发，detail.rs:383-391）都不在 `slider_slots` 游走里，也没有其它取消挂点。
- 失败链（亲证）：拖旋钮中途按 Ctrl+Z——焦点在画布（非文本），`typing_focus` 不拦 → UndoGraph 派发，服务端 journal 弹出一步、UI 重建显示撤销值——但 `knob_drag` 仍在：**下一次指针移动**（graph.rs:385-404）以按压锚定值重算并再派发 `SetGain`，把撤销结果覆盖回去；且 journal `record`（service/graph.rs:245-258）在非合并记录时 `redo.clear()`——重做历史也被一并销毁。用户感知即 SD-7 原文："undo 时灵时不灵"。EQ 详情拖拽同理（拖拽中 Ctrl+Z 一样可达——详情窗不持有焦点，`keep_graph_focus` 维持画布焦点，按键绑定仍在根上生效）。
- 对照：撤销合并本身没问题——`undo_key`（service/graph.rs:210-236，`gain:{node}`/`builtin:{node}` 等）+ 500ms 窗口正确覆盖旋钮/EQ/压缩器/tint 的连发，undo 日志不会被拖拽灌爆；问题仅在"撤销后手势尾巴写回"这一环。
- 修复方向：`cancel_active_drags` 一并清 `self.knob_drag`（+ `sync_drag_demand`）；EQ 编辑器给拖拽会话一个 shell 侧状态（或由 detail overlay 在收到 Undo action 时复位）。补"旋钮拖拽中 Ctrl+Z 后指针再移动不再派发"的回归测试。

### IN-3 (P2) Graph 页 ControlsKey 缺 catalog / audio_settings.backend

- ControlsKey（page.rs:22-41）只装 `graph: ArcSnapshot<GraphSnapshot>`、generation、phase、desired、positions、camera、canvas、selected_nodes/route、connection、hand_tool、sidebar_open、compact、preferences。而图页渲染的 `CanvasWorld::positioned(&state.snapshot, ...)` 读取**整个 AppSnapshot**，其中 catalog 相关：
  - 绑定 chips（#93 新增）直接查 catalog 行名：graph_canvas.rs:435-492（`catalog.inputs`/`processes`/`outputs` 找 friendly name）；
  - 卡片标题经 `display_label` → `source_name`/`output_name` → `input_rows(&snapshot.catalog, snapshot.audio_settings.backend)`（presentation.rs:286-340）——同时消费 catalog **和后端**。
- AppSnapshot 的 graph Arc 每次发布重建、按键**内容**比较（model.rs:150-190 注释），所以 **catalog-only 发布**（设备热插拔/更名、VAD 目录变化——driver.rs:156-157/182 每次 poll 都跑 `update_vad_catalog`，:229 变化即改 catalog；以及 "Refresh sources" 显式请求）下 ControlsKey 相等 → 图页缓存不重建 → **chips 与默认标题停留在旧设备名**，直到任何图编辑/相机动作。对照组：Mixer 页键**含** `catalog: Arc<DeviceCatalog>`（mixer/page.rs:73,126），同一发布下 Mixer 条带刷新而图页卡片不刷新。
- 归因：标题一半（display_label）pre-#93 已然（`git show 3321062^1` 的 graph_canvas.rs:215 已调 display_label），chips 一半为 #93 新增暴露面。同型先例：SD-4（can_start_transport 指纹缺 catalog）。修复：ControlsKey 增 `catalog`（Arc，按 identity 比较）与 `audio_settings.backend`（或整个 audio_settings）。

### IN-4 (P2) 绑定面硬编码 Wasapi 后端（#93 复制既有缺陷到新表面）

- binding_menu.rs:87-90 / 154-157 与 node_inspector.rs:272-275 / 365-368 一致地写死：

  ```rust
  crate::ui::state::input_rows(&snapshot.catalog, moiren_engine::audio_config::AudioBackend::Wasapi)
  ```

  而 `input_rows/output_rows`（state.rs:71-95）按后端分流：Asio 时返回 `catalog.asio_drivers`（另一套行）。设置页可切 ASIO（settings.rs:518 有 Asio 分支）。后果：ASIO 会话下，chip 菜单与检查器 picker 列的是 WASAPI 端点（点了绑定不上 ASIO 通路），ASIO 驱动永不可选；而 `display_label`/`source_name` 是后端感知的（presentation.rs:276,326），两套语义已经分叉。检查器一半是既有缺陷；binding_menu 是 #93 按设计文档"the same choices the inspector lists, in the same order"忠实复制——把缺陷一起带了过来。修复：两处改传 `snapshot.audio_settings.backend`（与 IN-6 的列表合并一起做最划算）。

### IN-5 (P2) 旋钮/EQ 拖拽逐 move 派发，无 33ms 合并

- 滑杆族有显式合并：`FADER_DISPATCH_INTERVAL = 33ms`（mixer/model.rs:79-93，拖拽 Change 合并、释放落终值）。旋钮路径（graph.rs:385-404）与 EQ 详情拖拽（detail.rs:332-391）对**每个鼠标移动事件**直接 `state.dispatch(AppRequest::EditGraph(...))`。服务端每条 edit 都执行 `before = self.snapshot.clone()`（整图快照克隆）、journal 记录、发布（service/graph.rs:662-666）；每次发布又驱动全部 PanelHost 的 key 捕获。125-1000Hz 的指针事件率下这是纯控制侧风暴（无 RT 违规——都在控制线程），但对照滑杆族的设计意图属于明确的退化。undo 日志靠 500ms undo_key 合并兜住（见 IN-2），所以症状是 CPU/功耗与发布流量，不是撤销爆炸。修复：旋钮/EQ 拖拽复用 FADER_DISPATCH_INTERVAL 节流（尾沿落终值）。

### IN-6 (P2) 源/汇绑定选项双份手工维护

- node_inspector.rs:214-343（Source：Clear/Tone/物理输入/Playback exports/进程/Refresh）与 365-401（Sink）和 binding_menu.rs:59-187 的 `items()` 在两处各写一遍，顺序与内容靠注释约定（binding_menu.rs:58 "The same choices the inspector lists for the kind, in the same order"）而非共享构造。IN-4 的 Wasapi 硬编码能两处一致地错，恰证明双份会一起漂移。新增绑定种类（如未来 ASIO 感知后）必须记得改两处。修复：`items(snapshot, id) -> Vec<Item>` 移到共享模块（binding_menu 或 presentation），检查器与菜单同源渲染（检查器需要非 Ghost 变体与不同 id 前缀，Item 抽象已具备承载能力）。

### IN-7 (P2) 节点详情窗零测试覆盖

- `grep -n "detail"` 在 graph/tests.rs 仅命中库侧 detail sleeve（1775-1798，另一概念）；`node-detail-page`/`detail_node`/`node-tint-*`/`eq-editor` 无任何用例。#93 的三块新交互里，chip 菜单（tests.rs:2695）、旋钮（:334）、fault tooltip（:2436）、占用输入拽线重路由（:2501）都有测试，唯独详情窗（EQ 拖拽点、Q 滚轮、tint 派发 SetNodeTint、dynamics 曲线、Esc/外点关闭、`sync` 外的节点删除收尾）全裸奔。EQ 编辑器是 #93 唯二的连续值写回表面之一（另一是旋钮），其 clamp/对数映射/Q 保留逻辑（detail.rs:332-367）值得往返断言。建议至少：双击/按钮开窗 → 拖拽改频点 → 断言 SetBuiltin 落值与 clamp；Esc 关窗不残留。

### IN-8 (P2) 检查器观测读数冻结在两次发布之间（pre-#63 记录在案）

- `node_observation`（node_inspector.rs:575-634）读 `state.node_meters.status/channel_levels`（弹道层，随 33ms 泵推进），但这些不在 InspectorKey（sidebar.rs:18-38：snapshot PublishRef、选择、页、editing 族、compact、preferences）。面板重建只随发布发生：运行中 ~1Hz（elapsed 量化），空闲则零。结果：热层的电平条是活的，旁边 "Status/Peak n/RMS n" 数值冻结在重建时刻——两个相邻表面给出两个时刻的读数。条带侧同类问题曾以 SD-6 记录（数字读数 max-hold 语义）。非 #93 引入（node_inspector 未被 #93 触碰），按分区职责记录。修复：读数走热层/或键加"读数量化指纹"（如按 0.5dB 量化进键——SD-6 修复方向同款）。

### IN-9 (P2) delete_graph_selection 不清 knob_drag

- graph.rs:198-199 清 `connection_drag`/`node_drag`，遗漏同族的 `knob_drag`。场景：旋钮拖拽中（该节点已在选中集，或拖拽前已选中）按 Delete——节点删除后，后续移动有实时快照守卫（graph.rs:390-397 `graph.node(id)` 为 None 则不派发，亲证无死节点编辑），但 `drag_active()`（window.rs:571-577）持续为真 → `frames_wanted` 的手势短路持续成立 → vsync 帧循环空转烧帧。且 `on_bg_move` 只挂在画布元素上：指针移到检查器/工具栏等处不触发清除，烧帧持续到指针**回到画布**产生一次无键移动。对照 HC-1（手势滞留烧帧，P1）影响面小（无功能性冻结、自愈路径存在），定 P2。修复：delete_graph_selection 与 begin_pointer_session 对齐，一并清 `knob_drag` + `sync_drag_demand`。

### IN-10..IN-15 (P3) 摘述

- **IN-10** detail.rs:51-52 `let height = ...; let _ = height;`、:86 `let _ = close;`、:288 `let _ = shell;`——三个死绑定，迭代残留（`close` 在 :97 已被闭包实际使用，`let _` 纯多余）。顺手清。
- **IN-11** detail.rs:97-105 与 binding_menu.rs:253-261 的覆盖层 Escape 处理器：焦点恒在画布（keep_graph_focus + 覆盖层无聚焦子元素），键事件沿聚焦节点祖先冒泡，覆盖层是画布的**兄弟**，处理器不可达；真正的 Esc 通道是 canvas `on_cancel → cancel_active_gesture`（graph.rs:129-146 注释自述"Keyboard focus stays on the canvas while the page's popovers are open, so Escape lands here"）。作为纵深防御无害，但无注释说明其从属地位，读者会误以为这是主路径。
- **IN-12** node_inspector.rs:169 `label(format!("{:?}", node.kind()))`——类型行显示 Debug 枚举名（"Gain"/"SpectrumAnalyzer"/"ChannelMapper"），无 t!、无本地化（zh-CN 下英文）。与 SD-11/APP-9 类断层同族，一行改动（kind → 目录键）。
- **IN-13** detail.rs:379-391 EQ 拖拽挂在 `eq-editor-input`（inset_0 于绘图区）的 `on_mouse_move`：指针拖出绘图区但仍在对话框内时移动事件落在兄弟元素上，值冻结；指针回绘图区续动。覆盖层整体在 :125 拦截 on_mouse_move 只做 stop_propagation 不转发。手感毛刺，非数值错误（clamp 完备）。
- **IN-14** compressor.rs:90-105：清空字段按 Enter/Blur → `parse::<f64>().unwrap_or(NAN)` → 守卫 `value(parameter) == value` 对 NaN 恒 false → 派发 `SetCompressor{value: NaN}` → 服务端 `accepts` 校验拒绝（service/graph/compressor.rs:57-60 + graph.rs:590-592）→ `record_error_with_subject` 落 fault、英文 cause（"Compressor parameters are outside their valid range"）进错误条/tooltip。状态无损（快照整批丢弃），但"清空再输入"这一常见操作以一条不可读错误收场。修复：NAN/解析失败静默关编辑器（与 mixer 族 parse 失败语义对齐，model.rs:497-515）。
- **IN-15** `hovered_node` 仅在无键 move 时由 hit_test 重算（graph.rs:339-363）：fit/undo/框选改变布局后，静止指针下的 tooltip 归属不复核——可对已移走/换fault状态的卡片短暂错挂，直到下一次移动。窗口极短，P3。

---

## 编辑/输入/拖拽表面 × 机制覆盖 清点表

机制列：TF = typing_focus 覆盖；ON = 所在缓存面板 observe_nested 覆盖；KEY = 所在面板键覆盖其输入；UNDO = undo 期间被正确处置（取消或合并）。

| 表面 | 位置 | TF | ON | KEY | UNDO | 备注 |
|---|---|---|---|---|---|---|
| 库搜索 node_search | library sleeve（缓存：Library+Graph 双面板） | ✓(显式) | ✓（两面板都订） | ✓ | n/a | |
| 节点名重命名 node_name | 检查器（InspectorPanel） | ✓ | ✓（单一游走） | ✓ node_name_editing | ✓（Enter/Blur 提交，Ctrl+Z 被拦） | 测试 graph/tests.rs:488 |
| Fader 读数编辑器 value_inputs | 检查器/条带 | ✓ | ✓ | ✓ editing_value | ✓ | 开编辑器即翻键→池重捕（IN-1 兜底） |
| Balance 编辑器 balance_inputs | 检查器/条带（2ch） | ✓ | ✓ | ✓ editing_balance | ✓ | 回归网 window/tests.rs:1230 |
| Gain "Requested" node_gain_inputs | 检查器 | ✓ | ✓ | ✓ editing_node_gain | ✓ | 同 value 兜底 |
| Pan 编辑器 node_pan_inputs | 检查器 | ✓ | ✓ | ✓ editing_node_pan | ✓ | 同上 |
| Send 编辑器 send_inputs | 条带 | ✓ | ✓ | ✓ editing_send | ✓ | SD-3 已修（复用 parse_gain_db） |
| 压缩器文本 inputs | 检查器 | ✓ | ✓（枚举在池） | — 无旗标 | ✓（Enter 提交+合并） | **IN-1：捕获时序洞（首选盲打）** |
| 压缩器 hslider sliders | 检查器 | n/a | ✓ | n/a（值随发布） | ✓ | 33ms 合并，缺池窗口不可辨 |
| inspector_faders hslider | 检查器 | n/a | ✓ | n/a | ✓ | 同上 |
| 条带 faders/sends hslider | Mixer 页（非本面板） | n/a | （Mixer 分区） | ✓ | ✓ cancel_active_drags | |
| 导出草稿 export_controls | 设置/导出面 | ✓ | ✓ | ✓（其面板） | n/a | |
| **卡片旋钮拖拽/滚轮/双击（#93）** | 画布（GraphPanel 缓存外沿，热层/事件驱动） | n/a | n/a | ✓（值经 snapshot → graph 键） | **✗ IN-2/IN-9** | 值域往返有测试 knob.rs:207 |
| **EQ 详情拖拽/Q 滚轮/tint（#93）** | 根级 overlay（每帧重建） | n/a | n/a | n/a | **✗ IN-2** | **零测试 IN-7** |
| **绑定 chip 菜单（#93）** | 根级 overlay | n/a | n/a | n/a（按钮即时派发） | n/a | 生命周期收口✓；Wasapi 硬编码 IN-4 |
| 画布手势（选框/平移/节点/连线拖拽） | 画布 | n/a | n/a | ✓ ControlsKey（connection 在键） | ✓（切页/失活/Esc 复位，HC-1 修复在位） | |

面板键 vs 渲染读字段核对：InspectorKey 完整（含 5 个 editing 族、page、device_tab、双选择、compact、preferences；snapshot 走 PublishRef 恒新鲜）——除 IN-8 的观测读数。LibraryKey 完整。ControlsKey 缺 catalog/backend（IN-3），其余（positions/camera/canvas/selected×2/connection/detached/hand_tool/sidebar_open/compact/preferences）与 `graph::page` 渲染读逐项对上。

## 已核对无问题清单

1. **SD-1 修复本体**：`text_input_entities()`（window.rs:916-924，node_name + mixer 五族 + compressor.inputs + export）单源供给 typing_focus 与 InspectorPanel::observe_nested（sidebar.rs:100-127 含 inspector_faders/compressor.sliders 滑杆池）——枚举层面无漏族，#93 未新增文本输入表面。
2. **SD-2 修复本体**：typing_focus 拦 Ctrl+Z（undo_redo_graph:966-972）、keep_graph_focus 不再抢编辑器焦点（:993-1005，Graph 页实测有 graph/tests.rs:488 覆盖）。
3. **undo 合并**：undo_key（service/graph.rs:210-236）+ 500ms 窗口覆盖 SetGain/SetPan/SetBuiltin/SetCompressor/SetNodeTint/SetLabel/SetMixer——旋钮、EQ、tint、压缩器连发不会灌爆 journal。
4. **knob 值数学**（#93）：数字零增益 → −60 dB 地板往返稳定；log 域 clamp；gain floor 0.001 ≠ 真 mute 但 round-trip 自洽（knob.rs:95-114,118-144 + tests:207-266）。电平表未沾 FaderScale（meters.rs:124-136 守卫注释在位）。
5. **拖拽中节点消失守卫**：knob move 每次从实时快照读 entry（graph.rs:390-397），EQ 拖拽 Q 也读实时态（detail.rs:343-350）——无死节点编辑。
6. **绑定菜单/详情窗生命周期**：开（chip/expand）；关（菜单项、外点、Esc 经 canvas on_cancel、切页 set_page→cancel_active_gesture、窗口失活 observer、节点被删 sync/快照短路）；occlude 挡画布输入；菜单 `sync` 防止 overlay 活过锚点卡片（binding_menu.rs:32-44）。faults 在成功编辑/激活时整组清除（graph.rs:710-711），tooltip 每帧从 prepared frame 复核 `faulted`，无残留路径（IN-15 仅为指向陈旧）。
7. **文本提交语义**：根 mouse-down 提交活动编辑器（window.rs:838-847）；Blur=提交；空标签=取消重命名；parse 失败静默关编辑器（mixer 族）；编辑中 sync_value_inputs 停写不与快照写回打架（model.rs:486-492）。
8. **解析边界**：parse_gain_db 全拼写集 + 域过滤（model.rs:166-173 + tests:1124-1141）；parse_pan 的 U+2212 归一（:184-196）；tone ±10Hz 按钮 clamp 1..23999（node_inspector.rs:252-269）；压缩器滑杆 ln_1p 映射有往返测试（compressor.rs:189-199）。
9. **手势帧需求**：drag_active 含 knob_drag；frames_wanted 的手势子句限 Graph 页（HC-1 修复在位，测试 set_page_cancels…/window_deactivation…:1280/1400）。
10. **排序稳定性**：选中集 BTreeSet、投影按 topological_order、绑定列表按 catalog 行序——无面板项跳动源。
11. **既往项**（2026-10-31/2026-10-27 两目录 README 清单）：SD-3/SD-9/SD-10/HC-2(MatrixKey 载 density)/HC-3(no empty-pool retry) 逐一在代码中核实为已修且未回退；SD-7 滑杆半已修——**同型复发于旋钮/EQ 报 IN-2**；APP-9 i18n 断层仍在（IN-12/IN-14 为其在检查器面的活跃位点）。

## 与其他分区的边界

- 频谱两提交（ed93c58/e9387d2）：纯 spectrum.rs 投影数学与 nodes.rs 网格绘制，无检查器/键面改动——归 04 分区。
- 画布布局/hit test/曲线缩略（graph_canvas/layout.rs、curves.rs、frame.rs）：06/01 分区；本报告只在其与编辑器交互（hit test → knob/chip/expand 回调）层面核对。
- Mixer 页条带面板与 tray flyout：非本分区；仅在对照（catalog 在键、滑杆合并基线）时引用。
