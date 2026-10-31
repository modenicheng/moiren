# 子代理 D：状态与数据平面潜在 bug 猎捕（订阅生命周期 / epoch 门控 / 数值 / 节流 memo / 停帧保活 / i18n / undo）

审计范围：main @ dc9b38a（design 17、PR #53 已合并），重点 `ui/state.rs`、`ui/state/*`、`ui/monitoring.rs`(+`ballistics.rs`)、`ui/shared_frame.rs`、`ui/window.rs`、`ui/window/mixer/{model,meter_frame,mixer,strip_controls,routing}.rs`、`ui/window/settings.rs`(+`model.rs`)、`ui/window/error_strip.rs`、`ui/presentation.rs`、`ui/window/project.rs`、`ui/window/sidebar.rs`、`ui/window/graph/node_inspector.rs`、`ui/window/tray_mixer.rs`、`ui/window/desktop.rs`，以及 app crate 内 service 侧接缝（`service/signals.rs`、`service/graph.rs`、`service/managed/meters.rs`）。只读，未跑 cargo（主代理在跑 clippy）。为验证"嵌套模型通知在缓存面板内的可见性"核查了外部依赖 gpui-component 0.7.3 的 `Input`（`impl RenderOnce`，`src/input/input.rs:535`，渲染期读 `Entity<InputState>`）。

核对既往项：e7fbfd9（Mixer 静音停帧）、1aeeb79（FaderScale 尾段逆映射，`fader.rs:28-68` 三组 round-trip 测试齐全，未发现破点）、3faa5b9（未聚焦保活，`frames_eligible` 无焦点子句 ✓）、cb59d0e 节流（`panel.rs:99-127`，尾沿单发、有复苏让位测试，**未发现尾帧丢失**——Arm 定时器到期 fire 且 owner 侧 key 重建不消耗 armed 态）、`poll_windows` 的 sequence fence / epoch / 单调帧校验（`monitoring.rs:683-703`）均按设计工作。与子代理 C 重复的位点（HC-3 `nested.is_empty()`、HC-11 latency_profile 布尔折叠、HC-13 死 gain/pan slider、HC-6 dragging 标志不复位）已剔除或仅交叉引用。

---

## SD-1 (P1) InspectorPanel 的 `observe_nested` 漏掉 balance / NodeGain / NodePan / 压缩器 / inspector 推子槽——缓存检查器内这些编辑器"盲打"

- 位置：`crates/moiren-app/src/ui/window/sidebar.rs:75-93`
  ```rust
  let mut inputs = vec![shell.node_name.clone()];
  inputs.extend(shell.mixer.value_inputs.values().cloned());
  inputs.extend(shell.export_controls.input_entities());
  ```
  而缓存的检查器树实际渲染（`graph/node_inspector.rs`）：Fader 读数编辑器（:172-184，已覆盖）、**inspector 推子 hslider（:185-189，`shell.mixer.inspector_faders[&id]`，SliderState 未订阅）**、**Balance 编辑器（:198-205 → `strip_controls.rs:304-332`，`balance_inputs` 未订阅）**、**Gain 节点 "Requested" 编辑器（:401-421，`node_gain_inputs` 未订阅）**、**Pan 节点编辑器（:456-476，`node_pan_inputs` 未订阅）**、**压缩器全部 `Input` 与 `hslider`（:502-531，`shell.compressor.inputs/sliders` 未订阅）**。对照条带面板 `mixer/meter_frame.rs:263-313`（faders + value + balance + 出边 send 滑杆/编辑器）可见检查器只是没跟上编辑器家族的扩张。
- 证据：design 17 的机制前提是"嵌套模型通知自身不能使缓存祖先失效，必须经 `observe_nested` 池转发"（`panel.rs:169-179` 注释、design 17 "The cache splits per item" 节）；gpui-component 的 `Input` 是 `RenderOnce`、渲染期读实体，位于缓存子树内时其文本只在面板重建时重画。
- 失败模式：检查器里点击 Balance/Requested/Pan/压缩器字段开始键入——按键只更新 `InputState` 并 notify 该实体，无人订阅 → 缓存面板不重建 → **打出的字符不上屏**，直到下一次无关 publish 重建（运行中约 1 次/秒的 elapsed 量化 publish；会话空闲时可能完全没有 publish，全程盲打）。Enter 提交仍正确（`commit_*` 直接读实体值），但用户是在看不见的文字上确认。inspector 推子/压缩器 hslider 拖动同理：thumb 只在每次 dispatch→publish 往返（≥33ms+服务延迟）后跳变，与条带推子"首次通知立即重建"的手感不一致。
- 修复方向：把 `balance_inputs`、`node_gain_inputs`、`node_pan_inputs`、`inspector_faders` 的 `slot.state`、`compressor.inputs`、`compressor.sliders` 的实体并入 `InspectorPanel::observe_nested` 池（经既有 `NestedGate` 节流）。严重度介于 P1/P0，因无法在此环境实跑 UI 定级 P1，见"待实测 1"。

## SD-2 (P1) `typing_focus` 未纳入新点击输入编辑器族：Ctrl+Z 在键入中撤销图；Graph 页检查器编辑器被 `keep_graph_focus` 每帧抢焦点

- 位置：`crates/moiren-app/src/ui/window.rs:904-918`
  ```rust
  fn typing_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
      self.node_search... || self.node_name...
          || self.compressor.inputs.values().any(...)
          || self.mixer.send_inputs.values().any(...)
          || self.export_controls.owns_focus(focused, cx)
  }
  ```
  缺 `mixer.value_inputs`、`balance_inputs`、`node_gain_inputs`、`node_pan_inputs`（这些族在 e220fde/后续提交加入，`send_inputs` 与 `compressor.inputs` 加入时更新过本函数，新族没有）。
- 失败模式 A（undo 抢占）：`undo_redo_graph`（window.rs:922-949）明确以 `typing_focus` 为闸——"A focused text field owns ctrl-z; undoing the graph out from under a keystroke the user is typing would be a surprise"。在条带 dB 读数 / Balance / Requested / Pan 编辑器键入时按 Ctrl+Z：判定为非文本焦点 → `UndoGraph` 直接派发，图在编辑器下方被换掉；随后 Enter 的 commit 读的是"撤销后"的快照应用用户的数（`edit_mixer` 基于 live snapshot），用户输入与撤销结果混叠。
- 失败模式 B（Graph 页焦点偷取）：`keep_graph_focus`（window.rs:955-967）在 Graph 页每次根渲染运行：若聚焦句柄不属于 `typing_focus` 枚举集，`window.focus(&self.graph_focus)` 立即抢走。检查器（Graph 页也挂载，`sidebar.rs:448-461`）里打开任一上述编辑器 → `start_value_edit` 聚焦输入框 → 下一次根渲染（每个绘制帧都会重建根元素）即被抢焦 → 编辑器立刻 Blur → `sync_value_inputs` 的 Blur 订阅用预填文本 commit（预填 round-trip 为同值，等效 no-op 编辑）→ 编辑器闪开即关，**Graph 页检查器的点击输入编辑器基本不可用**（Mixer 页 `keep_graph_focus` 早退不受影响——现有测试 `mixer/tests.rs:1715+` 只测 Mixer 页，Graph 页无覆盖）。
- 修复方向：`typing_focus` 补齐四个族（或改为"任何 `InputState` 实体持有焦点"的判定）；`keep_graph_focus` 的文本焦点判定同步受益。

## SD-3 (P2) `commit_send_gain` 的 −∞ 拼写集合与 `parse_gain_db` 分叉（Unicode 减号输入静默无效）

- 位置：`crates/moiren-app/src/ui/window/mixer/model.rs:493-499`
  ```rust
  let gain = if matches!(text.trim(), "-inf" | "−∞" | "-∞") { Some(0.) } else { ... };
  ```
  对照统一入口 `parse_gain_db`（model.rs:137-144）：接受 `"-inf" | "−inf" | "-infinity" | "−infinity" | "-∞" | "−∞"` 且大小写不敏感。send 编辑器用自己的一套：缺 `"−inf"`（U+2212）、`"-infinity"`、大小写变体；`"-∞"` 重复字面量是死分支。
- 失败模式：应用自身显示的静音拼写是 U+2212（`db_text` → "−∞ dB"），用户照抄/输入 `"−inf"` 或 `"-INFINITY"` → parse 失败 → 编辑器关闭、什么都不改，无提示；同一输入在 fader 读数编辑器（走 `parse_gain_db`）却合法。两族编辑器行为不一致。
- 修复方向：send 提交复用 `parse_gain_db`（域 −120..=24 与其现有 `(-120., 24.)` 过滤一致），删除重复臂。

## SD-4 (P2) `can_start_transport` 指纹缺"资源可用性"与 `max_block_frames` 半边（HC-11 已报 profile 布尔折叠，此处为另一半）

- 位置：`crates/moiren-app/src/ui/state.rs:541-560`。指纹 = (candidate_revision, sample_rate, block_frames, precision==F64, latency_profile==default)。但被验证的 `transport_request`（:502-539）在非虚拟路径还消费：`candidate_spec()` → `self.selection`（output_id / 当前源身份）对 **catalog 行可用性**的校验（:463-487），以及 `spec.max_block_frames`（= `self.max_block_frames`，可与 snapshot 的 block_frames 暂时不同，:509-528 用的是 spec 值）。
- 失败模式：`RefreshCatalog` / 设备热插拔只换 catalog、不 bump candidate_revision → 工具栏 Start 的可用态停在旧值：输出设备被拔后按钮仍可点（点击时 `toggle_transport` 走**未记忆化**的 `transport_request` 现算，失败并报错——数据无损）；反过来设备恢复可用后按钮可能保持禁用直到任何一次图编辑。Devices 页选行会立即 `bind_candidate_resources()`（devices.rs:359-365）派发 BindSource/BindOutput → publish 带 revision bump → 自愈，故窗口期仅为一次 publish 往返；纯 catalog 变化是唯一的持久路径。
- 修复方向：指纹加入 `selection.output_id/input 身份`、`self.max_block_frames`，或以 catalog 的（inputs.len, outputs.len, processes.len）三元组进入指纹；与 HC-11 一并整理。

## SD-5 (P2) RMS 积分器自然衰减实为 ≈29 dB/s，注释声称 ~58 dB/s——Fast(40 dB/s) 档下 RMS body 的可见下落被积分器钳住、不跟偏好

- 位置：`crates/moiren-app/src/ui/monitoring/ballistics.rs:14-17`
  ```rust
  /// One-pole mean-square integration across ingested windows. The natural
  /// decay in silence is ~58 dB/s, above every fall-off preference, ...
  pub(super) const RMS_INTEGRATION: Duration = Duration::from_millis(150);
  ```
  实算：mean_square 以 exp(−t/τ) 衰减，dB 速率 = 10·log10(e)/τ = 10·0.4343/0.150 ≈ **28.9 dB/s**（`monitoring.rs:981` 的测试注释也写 "~29 dB per second at 150 ms"，两处自相矛盾）。而 `MeterFalloff::Fast = 40 dB/s`（monitoring.rs:722 测试）。
- 失败模式：`advance` 中 `body_db = integrated.max(self.body_db - release)`（ballistics.rs:84-85）取较高者=较慢者：静音持续时若偏好快于 28.9 dB/s（Fast=40），积分器反而更慢，RMS body 只以 ~29 dB/s 下落，**用户选择的 40 dB/s 对 RMS body 不生效**（bar/marker 仍按 40 走，两层速度分裂），且 `at_rest`（body_db ≤ −96 dB）比偏好暗示的更晚到达，停帧延迟。现有测试 `rms_body_falls_at_the_selected_rate_in_silence` 用 Moderate（20 dB/s < 28.9）恰好绕开该边界。
- 修复方向：要么把 `RMS_INTEGRATION` 降到 ~75 ms（自然衰减 ≈58 dB/s，与注释一致且高于全部档位），要么在注释与测试中承认 body 有 29 dB/s 下限并给 Fast 档加回归测试。

## SD-6 (P2) 条带数字峰值读数是"全时段最大值"锁存：不衰减、点击重置也不清，与检查器 Peak（marker 语义）不一致

- 位置：`crates/moiren-app/src/ui/monitoring.rs:603-611`（保留电平的 peak 只增不减：`level.peak = level.peak.max(element.peak)`，仅当 500ms 无交付（STALE）/ NoPublisher / 重订阅才置 None）+ `ui/window/mixer/meter_frame.rs:612-628`（读数画 `raw_levels` 的 peak）。clip 重置点击只 `reset_clip`（mixer.rs:298-307），不清读数。
- 失败模式：运行中的会话里一次 0 dBFS 瞬态后持续静音：bar/marker 按弹道衰减归零、页面 stand-down，但条带下方数字读数**钉在整个订阅生命期的最大值（如 "0.0"）**，直到该节点停发 500ms 或图重建；同一节点在检查器 "Peak n" 显示的却是 marker（保持 0.5s 后按偏好下落，node_inspector.rs:613-627）——两处"Peak"语义不同。该行为在 21d5b3b（PR #53 基线）即存在（`git show 21d5b3b` 同样用 raw_levels），非本 PR 回归，按"同型位点"上报。
- 修复方向：读数改用 `DisplayLevel.marker`（代价：下落期每帧重 shape 字符串——可按 0.5dB 量化后仍命中缓存），或给读数加与 clip 相同的点击重置、并在 tooltip 说明 max-hold 语义。

## SD-7 (P2) 拖拽中 undo 被释放写回静默覆盖（写回抑制 + Release 无条件派发的组合）

- 位置：`crates/moiren-app/src/ui/window/mixer/model.rs:727-734`（拖拽中抑制快照写回）与 :699-715（Release 无条件派发指针终值，除非 wheel/双击已越权）。`undo_redo_graph`（window.rs:922-949）不检查 `dragging`。
- 失败模式：拖推子途中按 Ctrl+Z → 服务发布撤销后的增益 → UI 写回被抑制（thumb 停在指针处，视觉上撤销"没生效"）→ 松手时 Release 把指针终值派发出去，**撤销被整体覆盖**。注释自认"any one stale write-back self-corrects"，但用户可感知为"undo 时灵时不灵"。与 HC-6（dragging 标志永不复位）同域不同点：HC-6 是抑制泄漏，本条是撤销语义被释放路径吞掉。
- 修复方向：`undo_redo_graph`（及一切图级编辑派发点）检测 `dragging` 任一为真时拒绝或先取消活动拖拽；或 Release 前比对"上次派发值 vs 当前快照值"，发现快照被外部（undo）改写则不覆盖。

## SD-8 (P2) legacy 采样的唤醒政策与 NodeMeters 相反：`publication` 即返回 true，订阅存续期间 33ms 轮询恒 notify

- 位置：`crates/moiren-app/src/ui/monitoring.rs:247-249`
  ```rust
  if publication { return true; }
  ```
  对照 NodeMeters 的显式政策（monitoring.rs:434-441）："A publication alone must not wake the interface: a running stream publishes silence windows too"。`update_points_at` 属 legacy 源/汇探针（Mixer 条带在"probe 挂载且该节点无受管 topic"时才订阅，e7fbfd9 之后面很窄），但一旦处于该状态：静音窗口仍以发布节奏到达 → 每次 33ms 轮询 `pump_meters` 返回 true → `cx.notify()` → 根元素每 33ms 全量重建（缓存面板 replay，代价小但非零），**页面永不真正安静**——正是第三轮实机修的"烧核"类残留，只是从显示帧率降到了轮询率。
- 修复方向：legacy 路径复用 NodeMeters 的判据（status 变化或 raw 电平变化才返回 true），删除 publication 短路。

## SD-9 (P2) `update_node_meters` 在检查器不显示 meter 的页面仍持续订阅选中节点（240Hz 窗口照抽）

- 位置：`crates/moiren-app/src/ui/window/mixer.rs:28-53`：`inspector_open && selected_node` 时无条件 `nodes.insert(selected)`。但检查器在 Matrix 路由视图（`matrix::inspector`，仅按钮与文本）与 Devices 页（设备检查器）不画任何 meter（inspector 几何只由 `node_inspector.rs:1309-1340` 注册）。seen 表由 33ms 轮询每 tick 重插，750ms 宽限永不到期。
- 失败模式：会话运行 + 选中一个有信号的节点 + 切到 Matrix/Devices 页并开检查器 → 该节点订阅保持、每秒 240 个窗口被抽出并推进弹道，无任何表面消费（frames_eligible 为 false 不烧帧，只是无谓泵送与分配）。轻微，P2。
- 修复方向：insert 条件收紧为"当前页检查器确实渲染该节点的 meter"（Graph/Mixer 页的 graph::inspector），或由 inspector 几何注册表（`inspector_meters`）驱动。

## SD-10 (P2) `UiState.muted/dimmed` 是只写 false 的残留，其 poll 增益同步抑制条件一旦被驱动即吞服务端增益

- 位置：`crates/moiren-app/src/ui/state.rs:286-288`
  ```rust
  if before.0 != after.0 && !self.muted && !self.dimmed { self.gain = after.0; }
  ```
  全仓 grep：`self.muted`/`self.dimmed` 只在 `restore_*` 里被置 false，无任何置 true 的路径（条带的 M/DIM 按钮编辑的是 `MixerState`，另一码事）；`effective_gain`/`candidate_config` 却仍消费它们。
- 失败模式：当前无（恒 false）。属脆弱残留：未来任何一处把 `muted=true`（例如恢复旧 mute 快捷键）接入后，静音期间服务端增益变更（undo、另一窗口 flyout 编辑、项目载入）将**静默不回写** `self.gain`，解除静音后用陈旧值写回，覆盖真实状态——正是该条件当年想保护的场景的反面。
- 修复方向：删除 `muted/dimmed` 与该抑制条件（若 legacy 传输控件已死，见 HC-13 的 gain/pan slider），或补一条"unmute 时从快照重读 gain"的对称路径。

## SD-11 (P2) i18n 断层现状：`dispatch_error` 走 Debug 串、服务侧 RegisterError 类 fault cause 无翻译键（其余新错误路径已核对全覆盖）

- 位置：
  - `crates/moiren-app/src/ui/state.rs:341-342`：`self.dispatch_error = Some(format!("{error:?}"))` —— `DispatchError` 的 Debug 串（如 `QueueFull`）直接进错误条，error_strip 靠 `t!(error.as_str())` 透传（error_strip.rs:168-177 注释自认"plain prose; a catalog miss renders the source text unchanged"）。
  - `crates/moiren-app/src/service/signals.rs:143-145,166`（`error.to_string()` 的 `RegisterError`）→ `managed/meters.rs` 的 `?` → `record_error` 成为 `fault.cause` → 错误条 `t!(fault.cause.as_str())` 透传，zh-CN 无键则英文原文上屏。
  - 已核对无断层的新路径：`transport_request` 两条 t!（state.rs:516,537）、`MeterStatus::Error`（monitoring.rs:535,572）、`MeterStatus` 全部 label 键与 `Unavailable input/output`、`Test tone · %{frequency} Hz`、`%{name} (PID %{pid})` 均在 `locales/zh-CN.yml` 命中；settings 的 merge/draft 错误（settings/model.rs:136-141 等）全部 t!。
- 失败模式：中文界面下这两类错误以英文 Debug/Display 原文展示（APP-9 断层现状，非新回归，但仍是绕过 t! 的活跃位点）。
- 修复方向：dispatch_error 改为枚举 → 键映射；service 侧 fault cause 约定目录键并为 RegisterError 变体补键。

---

## 待实测（本轮静态无法定论）

1. **SD-1 实机严重度**：Graph/Mixer 页检查器打开 Gain 节点 "Requested"（或压缩器字段）键入，确认字符是否不可见/约 1s 批量上屏；顺带确认 gpui-fast `cached()` 对"渲染期嵌套实体读"确实不注册依赖（若实际注册，SD-1 降级为手感问题）。
2. **SD-2B 实机确认**：Graph 页检查器点击 dB 读数，确认编辑器是否被 `keep_graph_focus` 在下一帧抢焦并以预填值立即 Blur-commit（打开即关）。
3. **DispatchGate 尾值丢失**：拖拽中 alt-tab/窗口失焦导致无 `Release` 事件的路径（kit 是否兜底发 Release）：最后 ≤33ms 的拖拽值是否永远未派发、thumb 在下次 publish 被写回拽回（与 HC-6 的 stuck-dragging 是近邻场景）。
4. **−96 dBFS 停帧阈值 vs 实机底噪**：典型输入设备空闲本噪（实测多在 −70~−90 dBFS，远高于 REST_DB=−96）——接真实麦克风空闲时 Mixer 页是否实际永不 stand-down（数据确实在动，行为"诚实"但常驻帧循环）；顺带在 24-bit 静音通道（数字零）确认 stand-down 与唤醒（喂 −80dB 信号应唤醒）符合 e7fbfd9 回归。
5. **失焦/被遮挡保活下的 dt 尖峰**（PR#46 路径）：compositor 节流导致 `advance` 的 dt 长尖峰时 marker/bar 的 `rate*dt` 与 hold anchor（`held_at + peak_hold`）行为是否符合 `decay_is_independent_of_refresh_rate` 测试的外推。
6. **iconic→restore 的 SharedFrame 陈旧几何**：长时间最小化吸收多个 publish 后恢复（`restored` 单次 notify），确认各条带/画布重建时 `on_rebuild` 的自清理没有把未重建兄弟的注册误删（mixer 几何是"缺失才 reset + 各自 remove"，理论安全，值得实机对拍注册表与可见 track 数）。
7. **全角负号/IME**：zh 输入法下 dB 编辑器键入全角"－inf"或 "−"（U+2212）变体（SD-3 的用户侧入口）与 `parse_gain_db` 的实际接受集合。
