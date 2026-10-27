# moiren-app 审计报告

> 子区域：`crates/moiren-app`（155 文件 / ~53300 行；src 125 文件 / 42,954 行 + tests 17 文件 / 4,602 行）。方法：逐文件深读 service/，结构化浏览+热点深读 ui/，grep 驱动定位重复模式，只读 git 核对 PR #6/#9/#10/#11/#13/#15 的实际落点。全程只读。

## 1. 执行摘要（最重要的重叠）

1. **APP-1 双驱动栈并存（最大重叠面）**：生产入口只有 `ManagedDriver`（`service/mod.rs:63-69`），而 legacy `BackendDriver`（832 行）+ `service/parameters.rs`（421 行）+ `service/windows.rs` 的会话机制（~450 行）+ `backend.rs` 四个 prepare 变体，合计 ~1500+ 行仅被 4 个测试目标构造（主代理已亲证：src 内构造点仅 `settings_tests.rs:89`）；`WindowsBackend` 在生产只用到 `catalog()`（`managed.rs:211`）。
2. **APP-3 电平表分段策略双实现**：`widgets/meters.rs` 内 canvas 版 `channel_meter` 与 DOM 版 `meter_spans` 各写一遍同一套 −6/0 dBFS 分色 + RMS 暗尾 + 峰值保持 marker 策略，靠注释约定同步。
3. **APP-2 "正在输入"焦点谓词双写且已漂移**：`window.rs:857-865` 与 `892-900` 两份近似谓词，一份含 export_controls、一份不含，且 mixer 发送增益输入框两份都没覆盖（主代理已亲证）。
4. **APP-5 托盘推子另立刻度**：`tray_mixer.rs:343-384` 对同一 `MixerState.gain` 用线性 dB taper + 手搓 Slider 装配，主混音用 `widgets/fader.rs` 的 FaderScale taper——同一数值两个窗口高度不一致。
5. **APP-9 服务层用户可见字符串绕过 i18n**：PR #6 只覆盖 UI（1392 处 `t!`），service 层 65 处中文硬编码直接流进 fault 列表，英文界面下提示全中文。

另：三个新功能怀疑点（检查器双实现、重建三头分发、工具栏重复命令）经查证**全部排除**——PR #9–#15 是干净的增量式融入，未引入平行实现（见核对表 a/b/e 行）。

## 2. 既往已知项核对表

| 项目 | 状态 | 证据 |
|---|---|---|
| InputSelection⇄SourceMode 映射 ×5 | **FIXED** | 仅剩 `ui/state.rs:313-332`（apply_source）与 `336-355`（current_selection）一对互逆函数；grep 全库 SourceMode 只在 state.rs 定义 + devices.rs:174/231 赋值 + 测试 |
| 节点校验逻辑 ×3 | **FIXED** | 单源 `service/graph.rs:427 validate_node_state`；调用方 edit.rs:13、preparation.rs:268、project/graph.rs:309，均不自述规则 |
| ServiceRuntime::spawn ×51→~104 | **FIXED/大幅改善** | 当前 34 处（全库 54），且 `spawn_service/initial_projection/finish_service/wait_until` 已抽入 `ui/window/test_harness.rs:29-85` 与 `tests/common/mod.rs`（后者是集成测试必需的镜像，doc 注释已说明原因） |
| state.rs 1271 行（A13 尾巴） | **基本解决** | 非测试部分 ~657 行（选择态+候选+transport 请求，职责单一）；658-1271 为内联 `mod tests`（614 行），残余尾巴仅为测试未迁出（→APP-11） |
| a) 全局检查器侧栏 = 两套检查器？ | **不成立（复用）** | `sidebar.rs:167-180 page_inspector` 分发到 `graph::inspector`（即 `node_inspector.rs:41` 同一实现）、`matrix::inspector`（连接/边主体）、`devices::inspector`；字段渲染共享 `controls.rs` 的 panel/detail/heading/button/edit 词汇 |
| b) 重建触发三头（journal/防抖/直重建）？ | **不成立（单闸门）** | Managed 侧所有触发（Start driver.rs:70、ApplyGraph :77、防抖到期 :157-163、undo/redo :121-127 同一防抖）都汇入 `self.apply` 布尔，唯一执行体 `lifecycle.rs:61 apply()`；legacy 侧唯一入口 `driver.rs:137 apply_candidate`。undo 与结构编辑共用 200ms 尾沿防抖，注释明确（managed/driver.rs:117-120） |
| c) 电平表/状态渲染重复 | **部分成立** | 数据通路单线（见第 4 节）；但**绘制策略**双实现（APP-3）；meter 资格判定另有两层重复（APP-6/APP-7） |
| d) i18n 接入不统一 | **部分成立** | UI 层均匀（`t!` 键=英文原文，`i18n.rs` 单一机制）；但 service 层 65 处中文硬编码（APP-9）、`node_inspector.rs:136 "rename"`、`presentation.rs:83 "Input"` 注释自认缺失（APP-13）、`locales/i18n-keys.tsv` 台账与 zh-CN.yml 双事实源（APP-10） |
| e) 画布工具栏重复命令分发 | **不成立** | PR #15（de76f56）是干净替换（文本按钮→图标按钮），无旧代码残留；全 UI 仅一个 context_menu（mixer.rs:461 表参数菜单），工具栏/滚轮缩放都直调 `camera` 方法（graph.rs:145/626），Fit 与画布自适应共用 `camera.adjusted=false` 单机制 |

## 3. 发现清单

### APP-1｜P1｜平行实现/死代码
- **证据**：`crates/moiren-app/src/service/mod.rs:53-75`（start_service 只走 ManagedDriver）；`grep BackendDriver` 生产命中仅 `mod.rs:27` 再导出；构造点全在测试（`src/service/settings_tests.rs:89`、`tests/service_recovery.rs:243,254`、`tests/signal_service.rs:223`、`tests/service_backend.rs:291`）。legacy 会话栈：`service/driver.rs`（832）、`service/parameters.rs`（421）、`service/windows.rs:38-216,397-641`（PreparedAppSession/ActiveSession），`service/backend.rs:53-110` 四个 prepare 变体。`WindowsBackend` 生产仅 `catalog()`+`update_asio_catalog`（windows.rs:291-396）经 `managed.rs:211` 使用。
- **重叠详述**：三份 `AppRequest` match（`mod.rs:294 CoreDriver`、`driver.rs:390 BackendDriver`、`managed/driver.rs:5 ManagedDriver`）臂型相同、语义按后端分叉；legacy 的实时参数回写（driver.rs:460-528 手写 match 维护 desired_graph 副本）与 managed 的 `edit_parameter_values`+strip 同步是两套等价机制，legacy 版还手抄了一份参数→候选状态回写。
- **建议**：分两步。①把 `WindowsBackend` 的 catalog 部分拆出（供 ManagedDriver 注入），legacy 会话机制标注或收进测试专用；②逐套把 `tests/service_recovery.rs`、`tests/signal_service.rs`、`settings_tests.rs` 迁到 `ManagedDriver`+`managed/test_backend.rs`（该测试后端已存在），随后删 `BackendDriver`/`parameters.rs`/windows.rs 会话段与 backend.rs 多余 prepare 变体，`BackendAdapter` 收缩为 catalog 契约。预计净删 ~1200-1500 行。
- **风险与验证**：legacy 驱动承载 monitor 恢复/淡入 fallback（`recovery.rs`、`driver.rs:337-387`）语义，迁移时需保留等价覆盖；`cargo test --locked -p moiren-app` + `tests/service_recovery.rs` 全绿为准；删减分多个 PR。

### APP-2｜P1｜重复谓词已漂移（正确性风险）
- **证据**：`crates/moiren-app/src/ui/window.rs:857-865`（undo_redo_graph：node_search+node_name+compressor.inputs+**export_controls.owns_focus**）vs `:892-900`（keep_graph_focus：node_search+node_name+compressor.inputs，**无** export_controls）。且两份都**不含** `mixer.send_inputs`（`mixer/model.rs:180-221` 的发送增益文本框）。
- **重叠详述**：同一意图"当前焦点是否为文本输入"写了两遍；已经漂移一次（export_controls 只加在一处）。更实际的洞：在 Mixer 页发送增益输入框输入数字时按 Ctrl+Z，若 gpui-kit Input 不自行消费该键，将触发整图 undo 而非文本撤销。
- **建议**：抽 `StudioShell::typing_focus(&self, focused, cx) -> bool` 单一谓词（node_search、node_name、compressor.inputs、send_inputs、export_controls 全列），两个调用点共用。
- **风险与验证**：行为对齐后 keep_graph_focus 在 Graph 页语义不变；`cargo test -p moiren-app --features gpui-test` + 新增"发送输入框聚焦时 Ctrl+Z 不触发图 undo"用例。

### APP-3｜P1｜同策略双实现（视觉漂移风险）
- **证据**：`crates/moiren-app/src/ui/widgets/meters.rs:39-74`（`channel_meter` canvas 闭包内联推导 green/yellow/red 分段、RMS 暗尾、marker）vs `:127-195`（`meter_spans`/`meter_bar` 用 DOM 计算同一分段）。两处都以 `10f32.powf(-6./20.)`、`meter_height(1.)` 为界，注释靠 "Zone boundaries use the compact digital reference" 约定同步。
- **重叠详述**：`channel_meter` 的 (top,color) 循环+RMS 尾 = `meter_spans` 的逐 span 计算，仅最终绘制载体（paint_quad vs div）不同；分段数学完全可以共享。
- **建议**：canvas 版改为 `for span in meter_spans(level) { paint_quad(fill(track_from(span)...)) }`，删除内联推导；clip 点与 L/R 标签保留各自差异。
- **风险与验证**：纯绘制重构，`meters.rs` 既有单测 + gpui-test 截图用例（`window/tests.rs` mixer 表）覆盖。

### APP-4｜P2｜同一编辑构造模式 ×4
- **证据**：`crates/moiren-app/src/ui/window/mixer.rs:348-368 mixer_edit` 与 `tray_mixer.rs:387-406 mixer_edit`（函数体逐行等价，仅 notify 目标不同）；`mixer/strip_controls.rs:107-122 set_balance`（第三份"取 node.mixer→改一字段→dispatch SetMixer"）；`mixer/model.rs:156-166` 与 `tray_mixer.rs:112-127` 的 SliderEdit 闭包（第四份，gain 字段版）。
- **建议**：在 `UiState` 上加 `edit_mixer(&mut self, id, impl FnOnce(&mut MixerState))`，四处收敛到它；notify 由调用方自理。
- **风险与验证**：无行为差异；`cargo test -p moiren-app --features gpui-test`。

### APP-5｜P1｜同值双刻度 + 组件装配重复
- **证据**：`crates/moiren-app/src/ui/window/tray_mixer.rs:343-384`：自建 `fader`，thumb 位置 `(gain_db(gain)+60.)/84.` 线性 dB；对比主混音 `mixer.rs:320 fader()`（`widgets/fader.rs:125-173`，`FaderScale::position` 10 锚点 taper + 12dB 顶/−72dB 底）。两窗口同一 `MixerState.gain` 的推子高度不一致（如 −24 dB：主混音 0.67、托盘 0.43）。tray 的 Slider/Indicator/Track/Thumb 四件套装配 ~40 行与 widgets/fader.rs 高度相似。
- **建议**：给 `widgets/fader.rs::fader` 加宽度/高度参数（或 `fader_sized`），托盘改用它；副刻度统一 FaderScale。托盘滑条值域是 dB（[-60,24,0.1]）而主混音是归一化 [0,1]+FaderScale::gain，统一刻度建议托盘也改归一化域（`sync_slider_range` 已支持自定义 range，model.rs:76）。
- **风险与验证**：托盘拖动手感变化需人工确认；`tray_mixer/tests.rs` + FaderScale 往返单测。

### APP-6｜P2｜"按 NodeKind 找主节点"扫描散布 ~6 处
- **证据**：`ui/state.rs:279-296`（找 Gain/Pan）、`state.rs:554-594`（找 Source/Sink/Gain/Pan）、`ui/window.rs:917-933`（找 Source/Sink）、`service/managed/meters.rs:69-78`（找 Source/Sink）、`ui/window/mixer.rs:380-394`（meter 资格）、`ui/window/graph/library.rs:118-123`（取 kind 兜底）。service 侧已有 `GraphPreparation::primary_io_nodes/gain_pan_nodes`（`service/graph/preparation.rs:227-319`），UI 没有对应单源。
- **建议**：在 `GraphSnapshot` 上加 `node_of_kind(kind) -> Option<(&Node, &GraphNodeState)>` 之类的查询，UI 各处改调它；mixer meter 资格判定同时复用它。
- **风险与验证**：机械替换；gpui-test 全套。

### APP-7｜P2｜tone 会话判定双层
- **证据**：UI `ui/monitoring.rs:175-181` 以 `InputSelection::Tone` 决定 monitor 对；service `service/managed/meters.rs:79-92` 以 host 侧 `InputConfig{source: Software}` 检测 tone 决定节点→topic 接线。同一业务规则（"本会话是纯 tone"）两个输入源、两处实现。
- **建议**：由 MeterBridge 把解析结果（哪三个 topic 对应当前 source/sink/tone）作为事实发布进 `SignalTopics`/快照，UI 直接取；或在快照上发布 `is_tone_session` 布尔。
- **风险与验证**：小改动；`tests/signal_visibility.rs`、monitoring.rs 内嵌测试覆盖。

### APP-8｜P2｜dB 步进/格式化碎片化
- **证据**：±1 dB 步进数学重复两处（`ui/window/matrix.rs:299-318` 路由增益 vs `graph/node_inspector.rs:322-343` 节点增益，均 `10f64.powf(±1./20.)`，后者多 0 值/16 上限特判）；dB 文本格式三套：`controls.rs:71 db()`（"+x.x dB/−∞ dB"）、`node_inspector.rs:510 level_db()`（"+x.x dBFS/−inf dBFS"，小写 inf 不一致）、`mixer.rs:454`（"{:.1}" 裸数值）。
- **建议**：收敛为 `db_text(f64)` 与 `dbfs_text(f32)` 两个格式化器 + `step_db(gain, delta_db, min, max)` 一个步进函数（放 widgets 或 presentation.rs）。
- **风险与验证**：−inf/−∞ 拼写统一会改快照文本，需同步 `state.rs:1147`、`window/tests.rs` 断言字符串。

### APP-9｜P1（漂移/产品一致性）｜i18n 层间断层
- **证据**：service 层含中文字符串字面量 65 处（非测试），全部直达用户：`service/graph.rs:376-390 edit_hint`、`graph.rs:434-464 validate_node_state` 消息、`graph/edit.rs`（"节点不存在"等）、`graph/preparation.rs:100-152`（"输入节点尚未绑定音源"等）、`managed/driver.rs:662-702 project_fault` 的中文 cause/hint、`mod.rs:333`、`driver.rs:556`。渲染端 `ui/window.rs:729-744` 用 `t!("Error: %{cause}")` 包裹这些中文 cause——英文界面下框架文案英文、内容中文。`rust_i18n` 目录是 crate 级嵌入（lib.rs:6），service 同 crate 完全可以 `t!`。
- **建议**：短期把 `edit_hint`/`project_fault`/`validate_node_state` 的消息表迁移为英文键+zh-CN 目录（与 UI 同法）；长期把 fault cause 改为枚举/错误码，由 UI 层做本地化映射（`GraphFault.cause: String` 改结构化是更彻底的单源方案）。
- **风险与验证**：服务测试断言了中文文本（graph/tests.rs、managed/tests.rs 多处），需同批更新；`tests/ui_acceptance.rs` 的 fault 列表用例。

### APP-10｜P3｜双事实源台账
- **证据**：`crates/moiren-app/locales/i18n-keys.tsv`（494 行，含 key→用途→file:line 引用→中文说明）与 `locales/zh-CN.yml`（1484 行，真目录）并存；TSV 的 file:line 列随重构必然失效（de76f56/2e81018 都在改它）。i18n.rs 测试只锚定 yml。
- **建议**：TSV 定位是翻译工作清单，翻译完成后删除；若保留，去掉 file:line 列。无代码依赖，纯仓库卫生。

### APP-11｜P3｜A13 残尾
- **证据**：`ui/state.rs:658-1271` 内联 `mod tests` 614 行（transport_tests.rs 已外置先例在同目录）。
- **建议**：迁到 `ui/state/tests.rs`。机械移动，无行为变化。

### APP-12｜P3｜小规模并行装配
- **证据**：`ui/window/graph/library.rs:42-91`（详情行：icon+t!title+点击 spawn+on_drag）vs `:219-254`（rail：同一 icon+点击 spawn+on_drag），模板数据本体已单源（`pages/node_catalog.rs NodeTemplate::ALL`）。~25 行装配重复。
- **建议**：抽 `template_entry(template, enabled, this)` 共享三件套，两处只差排版。收益有限，下次改 library 时顺手做。

### APP-13｜P3｜i18n 漏网点
- **证据**：`ui/window/graph/node_inspector.rs:136` `.child("rename")` 硬编码；`ui/presentation.rs:83` `"Input".to_string()` 带注释自认不在键目录；`state.rs:39-46 SourceMode::title()` 的三个键需确认（"Physical input" 在 yml 未见独立键，与 node_catalog.rs:57 同名标题可复用）。
- **建议**：补 2-3 个键并包 `t!`；与 APP-9 同批处理。

## 4. 明确不建议合并项

- **MeterSubscriptions vs NodeMeters（monitoring.rs:71/290）**：二者共享 `MeterSubscription` 核心（449-594），差异是真实边界——legacy 双通道固定订阅 vs 按节点可见性+宽限期驱动的需求管理，合并会搅乱需求语义。
- **meter 数据通路两次拷贝（engine→MeterBridge→stable topic→UI owned）**：分别跨越 service 线程与 UI 线程边界，符合 AGENTS "RT 之外、有界、所有权清晰"约束；层数已是下限（结构性收敛见跨 crate 报告 X-1）。
- **matrix::inspector vs node_inspector**：主体不同（边/路由 vs 节点），字段集不同；已共享 controls.rs 词汇，再抽公共框架不值。
- **两驱动 `ServiceDriver::request` 的 match 臂相似**：逐臂语义按后端分叉（SetMixer 在 legacy 强制重建、在 managed 走 strip 参数；undo 的防抖 vs 立即 apply 源于两套重建机制）。抽"公共预处理"会把分叉藏进钩子。正确路线是 APP-1 的缩减而非抽象。
- **`ui/window/test_harness.rs` 与 `tests/common/mod.rs` 镜像**：集成测试构建于 default features，`#[cfg(test)]` 项不可导入，doc 注释已解释；两份 ~60 行是构建系统硬约束。
- **project/model.rs 的 SourceDocument⇄InputSelection 与 state.rs 的 SourceMode⇄InputSelection**：分别是持久化格式与 UI 选择态，两层各自已是单源，跨层强行合一会耦合磁盘格式与 UI。
- **channel_meter（canvas）与 meter_bar（DOM）并存本身**：主混音逐帧动画需要 canvas 单表面（注释明言避免每帧重建布局树），托盘静态双条用 DOM；应合并的是分段数学（APP-3），不是绘制载体。

## 5. 统计信号

- 规模：src 125 文件 / 42,954 行 + tests 17 文件 / 4,602 行；390 个测试函数；测试占比约 35-40%。
- 顶级文件大多"名不副实"：monitoring.rs 1308 行中 ~690 为测试；node_inspector.rs 1188 行为单一职责按 kind 分段，表驱动 BuiltinFieldSpec 已压掉大部分重复，不建议机械拆分。
- `t!` 调用 1392 处（UI 全覆盖）vs service 中文硬编码 65 处——i18n 完成度的断层正好落在 service/UI 边界。
- 近 5 个 PR（#9-#15）在 moiren-app 净增 ~1,600 行，均为增量式融入既有单源（NodeTemplate、validate_node_state、单一 apply 闸门），**本轮重叠主力仍是更早的 legacy 驱动栈与 meter 双绘制，而非 PR #6-#15**。
