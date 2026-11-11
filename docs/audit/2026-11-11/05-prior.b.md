# 05 既往审计遗留处置 — 初审 B（2026-10-27 / 10-31 / 11-03 / 10-29）

- **基线**：main @ `e2ed3d6a`（PR #168 合并点，2026-11-10 22:40 +0800）。工作区除 `docs/audit/2026-11-09/`、`docs/audit/2026-11-11/` 两目录 untracked 外干净。
- **方法**：只读。逐条读取三轮已入库审计（2026-10-27 / 10-31 / 11-03）的 README 与分报告原文，按其记载的证据位点对当前 main 逐一亲证（Read/grep/git log）；未跑 cargo；未读 `docs/audit/2026-11-11/` 下其他代理的报告。判定值：已修（给提交）/未修（给当前证据）/部分修/误报/被重构消化（旧形态消失且新形态无此问题）。
- **范围修正（对派单的更正）**：派单称 2026-10-27 审计为"meter fps 与性能"，并要求核"Mixer 页每帧成本超 4.16ms 预算"与"层缓存被 owner 动画帧旁路"。**经逐文件核实，已入库的 2026-10-27 审计（README + 01~06）是一轮"功能与逻辑重叠"审计，不含任何 4.16ms 帧预算或层缓存旁路条目**（全目录 grep "4.16"/"fps"/"层缓存" 均无命中）。这两条描述与 10-31 审计 HC-3（owner 33ms notify 使缓存面板整树重建）的机制最接近，已按 HC-3 核销（见下）；"4.16ms"数字本身不可考（疑为 11-09/11-11 未入库轮次的记忆混入），登记为派单信息陈旧。当前 main 的帧预算机制现状：shell 持有显式帧需求仲裁（`shell.rs:699-712` `meters_in_motion`/`frame_demand`/`sync_drag_demand`）、面板带 `rebuild_count` 测试断言（panel.rs:150-157），本轮无法（也不允许 cargo）复测每帧毫秒数。

---

## 2026-10-27 状态表（功能与逻辑重叠审计，基线 927f05b）

### P0/P1

| ID | 标题 | 状态 | 证据 |
|---|---|---|---|
| WA-1 | 双后端能力不对称，路由由 `settings != default` 决定 | **已修** | `d67796a2` "delete the legacy W-K engine, render and shared-stream stacks"；`service/managed.rs:225-249` windows() 工厂对 f32/f64 均无条件构造 `ProfessionalBackend`，无 legacy 分支；engine_backend/ 现只容纳 professional 本体（owner/validation/virtual_io 等）；README:581 明注 "removed with the legacy stack in the 2026-10-27" |
| WA-2 | 同名 Tone 两栈幅度 1.0 vs 0.1 | **已修**（随退役消化） | legacy tone.rs 已删；唯一实现 `windows-audio/src/bridge.rs:331`（`sin() * 0.1`，幅度 0.1 胜出） |
| WA-3 | 进程环回准备逻辑双份 | **已修**（随退役消化） | 收敛为顶层 `process_loopback.rs`+`process_loopback/` 单模块，`moiren:process:` 解析单点 |
| APP-1 | legacy BackendDriver + 会话栈 ~1500 行 | **已修** | 全仓 grep `BackendDriver` 零命中（10-31 审计记载 83e6e34 已删）；service/ 无 driver.rs/parameters.rs |
| APP-2 | typing 谓词双写漂移、mixer 发送框漏覆盖 | **已修** | `ui/shell.rs:1033-1050` `typing_focus` 走 `text_input_entities()` 单次遍历（node_name+mixer 全族+compressor+export drafts），注释点名 audit SD-1/SD-2；`shell/tests.rs:1256`、`:1412` 回归测试 |
| EN-1 | 队列版 LevelMeter 支配/双测量 | **已修** | `engine/src/meter.rs` 现 56 行（原 ~380），头注释"measurement kernel lives in `signal`"；grep `LevelMeter\b/bind_meter/meter_reader` 零命中 |
| EN-2 | Recorder/Spectrum/Compressor 读出端无生产消费者 | **部分修** | Spectrum 已接线：`service/managed/meters.rs:159` 生产调用 `subscribe_spectrum`；Compressor reduction topic（engine signals.rs:489）仍无 app 订阅者；`record_wav`/`RecordingSession` 仍无 app 生产调用（仅 engine 内部），Recorder 节点仍在目录（node_catalog.rs:19）——创建后队列仍无人排空 |
| EN-3 | project.rs 三重 ParamSpec + 手写参数域 | **已修** | `engine/src/host/project.rs:164` 单一 `fn node_specs(config)->Vec<ParamSpec>`，配合 `specs_accept_initials`/`accepts_parameter`；`:260` 走 `storage_valid() && specs_accept_initials` |
| APP-5 | 托盘推子另立刻度 | **已修** | `ui/shell/tray_mixer.rs:33` 使用 `widgets::fader::fader`，SliderSlot/sync_faders 与主混音同源（model.rs） |
| APP-9 | service 层 65 处中文硬编码绕过 i18n | **已修** | service 层中文字面量 0 处（仅 tests.rs）；dispatch 错误改为类型化 `UiFailure`（state.rs:134）+ 绘制期翻译（chrome.rs:250）；`dispatch_error_cause` 返回英文键（shell/project.rs:702-712） |

### P2/P3

| ID | 标题 | 状态 | 证据 |
|---|---|---|---|
| APP-3 | 电平表分段 canvas/DOM 双实现 | **已修** | `widgets/meters.rs:151-225`：`paint_meter_fill`/`paint_meter_fill_horizontal` 均迭代共享 `meter_spans(level)` |
| APP-4 | 同一编辑构造模式 ×4 | **已修** | 唯一 `mixer_edit`（`pages/mixer/model.rs:596`），tray_mixer.rs:24 从 model 导入复用 |
| APP-6 | 按 NodeKind 找主节点散布 ~6 处 | **已修** | `service/graph.rs:127` `GraphSnapshot::node_of_kind`；UI 各处调用（shell.rs:1153/1156 等）；`primary_io_nodes` 复用（graph.rs:856+） |
| APP-7 | tone 会话判定双层 | **已修** | snapshot 携带 `tone_session`（service/mod.rs:384），UI 单点消费（monitoring.rs:233） |
| APP-8 | dB 步进/格式化碎片化 | **已修** | `presentation.rs:296-329` `db_text`/`db_text_bare`/`dbfs_text`/`step_db` 单源，matrix/mixer 共用 |
| APP-10 | i18n TSV 双事实源 | **已修** | locales/ 仅剩 zh-CN.yml，TSV 已删；双向护栏见 i18n_guards.rs |
| APP-11 | state.rs 内联 614 行测试 | **已修** | `ui/state/tests.rs` + `ui/state/transport_tests.rs` 独立文件 |
| APP-12 | library 详情/行装配重复（P3） | **未核**（启发式：未逐行比对） | pages/graph/library 相关文件经 1ae1ec74 重组 |
| APP-13 | i18n 漏网点 | **已修** | `node_inspector.rs:139` `t!("rename")` 已包；zh-CN 缺键被 guard 测试拦截（i18n_guards.rs:681-689） |
| EN-4 | process() 双传输样板 ×7 | **已修** | `linked_io!` 宏落地（gain.rs:86 等，process() 缩为 3 行） |
| EN-5 | kernel 构造 match 平行 + MAX_DELAY_MS 三处 | **部分修** | `DelaySettings::storage_valid()` 已单源（delay.rs:35-36），rack.rs:279、project.rs:260 改调；kernel 统一 prepare 入口未见 |
| EN-6 | Gain/Pan 双传输知识两处表示 | **已修** | `RtProcessor::transport_interchangeable`（runtime.rs:94）被 plan.rs:123 与 runtime 共用 |
| EN-7 | prune 三段式重复（P3） | **已修** | `signals.rs:729` 泛型 `prune_slots<T>` |
| EN-8 | Compressor validate_io 缺 MAX_CHANNELS | **已修** | compressor.rs:3 导入并使用 `validate_linked_io`（:219） |
| EN-9 | Sum/Bus 双名导出（P3） | **部分修** | `builtin/mod.rs:59` `pub use sum::Sum as Bus`——定义单源但双名仍在 |
| CS-3 | core `validate_connection` 死 API | **已修** | 全仓零命中（已删） |
| CS-4 | core `GraphIndex` 死 API | **已修** | 全仓零命中（已删） |
| CS-1/CS-2 | signal dispatcher/subscribe 骨架平行 | **未核**（推定未修） | dispatcher.rs/registry.rs 结构未见收敛提交 |
| CS-5..CS-8 | 端点层卫生等（P3） | **未核**（启发式） | — |
| X-1 | engine/app 双份节点 level topic 目录 + prune | **未修** | app `service/signals.rs:161-209` `node_levels` 目录+prune 仍在；engine `host/signals.rs:295` `owner/{gen}/node/...` 命名与 `:542` prune 仍在；11-03 审计"仍开放"维持 |
| X-2 | 双份 demand-aware 1ms 泵调度（P3） | **未修** | engine poll_interval 与 app `signals.rs:112`（`next_poll = now + 1ms`）、`mod.rs:496-507` 双策略仍在 |
| X-3 | meter RT→UI 三拷贝三泵（量化） | **未修** | `MeterBridge`（managed/meters.rs:35-41）与 `poll`（:88）原样；与 X-1 同解 |
| X-4 | windows-audio 双采样率路径（P3） | **未核** | `clock_bridge/` 仍在（src 列表可见），其消费者随 legacy 退役是否变化未逐行核 |
| X-5 | VAD 状态文档头条滞后 | **已修** | `docs/vad-validation-2026-10-21.md:5` 头部 "Update (2026-10-22): the eight-hour idle run subsequently passed"；`driver/vad/STATUS.md:8` 头条即 eight-hour PASS |
| X-6 | 依赖声明分散（P3） | **已修** | 根 Cargo.toml `[workspace.dependencies]` 集中 rtrb/serde/serde_json/windows(-core) 并附 pin 理由注释 |
| X-7 | stress 运行产物待决（P3） | **已修** | `f38cc98e` 移除 stress 产物；`find docs -name "*.stderr.log"` 零命中 |
| X-8 | FRONTEND.md 滞后（P3） | **已修**（删除消化） | docs/FRONTEND.md 已从仓库移除 |
| X-9 | designs/12 序号复用（P3） | **已修** | 仅剩 `12-unified-topology-backend-contract.md`（原 12-gpui 已改号 15） |
| X-10 | .worktrees 仅本地排除（P3） | **已修** | `.gitignore:44-45,60-61` 已含 `.worktrees/` |

**小结**：可判定 34 项中已修 26、部分修 4（WA-6 未见、EN-2、EN-5、EN-9）、未修 3（X-1/X-2/X-3 信号双总线家族）、其余 P3 未核。**唯一成建制残留就是 X-1/X-2/X-3——"engine SignalHub 与 app SignalService 双 level 目录 + MeterBridge 接力"**，三轮审计连续点名未动。注：WA-6（role→ERole ×3）现剩 catalog.rs/wasapi.rs 两处，未见 negotiation 单源 helper，判部分修。

---

## 2026-10-31 状态表（UI 键/族审计，基线 dc9b38a）

### P1×5

| ID | 标题 | 状态 | 证据 |
|---|---|---|---|
| HC-3 | PanelHost `nested.is_empty()` 使无嵌套面板键控失效 | **已修** | `ui/shell/panel.rs:163-177` 观察者条件只剩 `key/locale` 比较与 `key: None` 首帧判据；注释明言"No empty-pool retry on top of that: a panel whose observe_nested legitimately yields nothing would take this branch on every notification and rebuild per notify"——旧子句已删并说明原因 |
| SD-1 | InspectorPanel `observe_nested` 漏 balance/gain/pan/压缩器/推子族 | **已修** | `shell/sidebar.rs:108-133` observe_nested 走 `text_input_entities()` 单次遍历 + inspector 自有 hslider 槽，注释点名"Missing any family leaves that editor blind-typing"；回归测试 `shell/tests.rs:1433+`（inspector_panel_rebuilds_when_a_nested_editor_moves） |
| SD-2 | `typing_focus` 漏新编辑器族（Ctrl+Z 撤图 + Graph 页抢焦） | **已修** | `shell.rs:1033-1041` typing_focus 以 `text_input_entities()` 单遍历实现（注释：a new editor family cannot land in one enumeration site and miss the other — the shared root cause of audit SD-1/SD-2）；`keep_graph_focus`（:1128-1141）经同一谓词 |
| HC-1 | 画布手势无切页/失活复位 → meter 泵永久短路 | **已修** | `shell.rs:747-751` `set_page` 首行 `cancel_active_gesture`（注释点名跨页卡死）；`shell.rs:540-549` `observe_window_activation` 失活即取消（注释点名 Alt+Tab 丢 mouse-up）；`pages/graph.rs:188-215` 统一取消体含 pan_grab/knob_drag 等 7 态 + `sync_drag_demand`；项目加载同钩（shell/project.rs 测试 :215） |
| HC-2 | MatrixKey 装 compact 概念、漏 density | **被重构消化** | Matrix 页不再缓存：`pages/matrix.rs:3-8` 头注释"The page renders uncached, on purpose"（旧 offset-key 缓存造成头/身错位，故整体转 live），行高现直接读 `shell.chrome.density`（:45）——冻结类失败在新形态下不存在 |

### P2（抽样核）

| ID | 标题 | 状态 | 证据 |
|---|---|---|---|
| HC-4 | SettingsKey 缺 viewport/pending_request/project_message | **部分修** | `pages/settings.rs:39-51` Key 已含 `scroll`/`meters`/`density`/`pending_request`/`project_message`（滚动/请求/消息三漏已堵）；viewport 高度项未见（可能随布局改动失效，未逐行核） |
| HC-5 | legacy 探针 `meter_points:(bool,bool)` 后画者覆写共享槽 | **已修** | 标志机制已废：`shell.rs:693-700` `(input_shown, output_shown)` 改由 `metering.observation_nodes` 派生（`legacy_meters_shown`），paint 期不再写共享槽；注释点名"Subscribing it for a display nobody sees kept the whole page's frame loop armed forever" |
| HC-6 | SliderSlot.dragging 无卸载/失焦复位 | **部分修** | `pages/mixer/model.rs:155-159` `cancel_drag()`（dragging=false + cancelled + gate.discard）落地，undo/手势取消路径调用（shell.rs:1074-1080）；写回加围栏 `ff3c54f7`（fence on slot's own last request）；但"拖拽中视图卸载"的复位路径未见显式钩子 |
| HC-7 | readout_cache 永不清理 | **被重构消化** | mixer/meter_frame.rs 已整体删除（10-31 的 mixer/meter_frame 结构不存在），读数走 pages/mixer 新装配；泄漏等价物未逐一重查 |
| HC-8 | 热层每帧 shape 通道标签 + meter_spans 每帧堆分配 | **未修** | `widgets/fader.rs:115-135` `paint_text` 仍每次 `shape_line` 无缓存，meters.rs:92/104 标签仍走它；`meter_spans`（meters.rs:225）仍返回 `Vec<MeterSpan>` 每轨道每帧分配 |
| HC-9 | on_camera_fit 注册只增不减 | **未核** | 未逐行核 pages/graph 注册路径 |
| HC-11 + SD-4 | can_start 指纹折叠 profile/缺资源半边 | **被重构消化** | `can_start_transport`/`transport_request` 记忆化已不存在（grep 零命中）；38234b11 "delete the UI session ledger; start is start-by-graph" 改为按图启动，旧指纹机制消失 |
| HC-13 | gain_slider/pan_slider 死状态机 | **已修** | `7d576b38` 删除；grep 零命中 |
| HC-14 | 每泵克隆 node_meter_points/seen | **已修** | PR #67（2f1bb23b fix/meter-pump-efficiency）+ 0a4f604e |
| SD-3 | send −∞ 拼写集合与 parse_gain_db 分叉 | **已修** | `pages/mixer/model.rs:341-347` 统一 `parse_gain_db`（含 U+2212/infinity/大小写），:676 send 提交改走它 |
| SD-5 | RMS 积分器 ~29 dB/s 与注释 ~58 相反 | **已修**（按"承认下限"方案） | `monitoring/ballistics.rs:33,39`：`RMS_INTEGRATION=150ms` 保留，但新增显式自然衰减常数 `10·log10(e)/τ ≈ 28.95 dB/s` 并在 :118 注释说明 body 跟积分器而非峰值偏好——两处矛盾消除，行为按文档 B 方案固定 |
| SD-6 | 条带峰值读数全时段锁存 | **已修** | `23062ba8` "strip peak readouts follow the marker like the inspector" |
| SD-7 | 拖拽中 undo 被释放写回覆盖 | **已修** | `shell.rs:1096-1106` undo_redo_graph 先 `cancel_active_drags`（注释点名 audit SD-7） |
| SD-8 | legacy 采样 publication 即 notify | **已修** | `0a4f604e` "wake legacy meter polls on reading changes, not publications" |
| SD-9 | 检查器不可见页仍订阅选中节点 | **已修** | `shell.rs:698-705` `update_node_meters(visible)` 可见性门 + legacy_meters_shown 派生 |
| SD-10 | muted/dimmed 只写 false 残留 | **已修** | grep 零命中（状态已删） |
| SD-11 | i18n 断层（dispatch_error Debug 串等） | **已修** | 见 APP-9 证据；另 `57bb3d0e` 将 project_io 消息类型化为 ProjectMessage 绘制期翻译 |
| HY-3 | design 17 声称 poll 接管 observation 派生 | **已修** | `docs/designs/17-...md:358` 现表述"`refresh_observation_nodes` runs"——与代码一致（函数存在于 shell.rs:1139） |
| HY-4 | X-1 应放回开放清单 | **部分修** | X-1 本体仍开放（见 10-27 表）；"放回清单"动作本身由 11-03 审计复核确认执行 |
| HY-10 | inspector 热表面零测试 | **已修** | `shell/tests.rs:1490+` inspector_hot_layer_paints_the_registered_meter_tracks（含 census 断言） |
| HY-11 | step-4 七面板无重建计数器 | **已修** | `PanelHost::rebuild_count`（panel.rs:150-157，gpui-test 门）被 devices/graph/mixer 多处测试消费（devices/tests.rs:66/241、graph/tests.rs:3579、mixer/tests.rs:3130） |
| 死码/死资产（DC 区） | ~44 行零引用码、21 svg、gallery 423 行 | **已修** | 11-03 审计已复核清偿（白名单 28↔28）；本轮复核：gallery 已 feature 门（Cargo.toml:37-39 `component-gallery = []` + required-features；mod.rs:8-9 cfg 门）；icons/ 35 个文件由 action_assets 白名单映射 |

**小结**：P1 5/5 解决（4 已修 + 1 被重构消化，无一原样残留）。P2 抽样 20 项：已修 14、部分修 3（HC-4/HC-6/HY-4）、未修 1（HC-8）、被重构消化 2（HC-7/HC-11+SD-4）。同型复发链（SD-1→IN-1）见 11-03 表。

---

## 2026-11-03 状态表（节点卡/画布/Inspector/频谱/autosave，基线 3321062+9e3a7bc）

### P1×5

| ID | 标题 | 状态 | 证据 |
|---|---|---|---|
| AS-1 | 4 处测试 shell 直建漏置 `autosave_path=None` | **已修** | 全部测试改走 harness：`shell.rs:795-809` `shell_for_tests` 强制 `preferences.path=None; autosave.path=None`（注释"Headless tests must never write the developer's real configuration"）；devices/exports/project 各 tests.rs 均经 `test_shell` 构造，grep 无直建绕过 |
| AS-2 | 打开工程不编辑即退出：陈旧 autosave 仍具恢复权威，一次 Save 覆写用户工程文件 | **未修** | 审计点名的三处原样：①`shell/project.rs:976-991` `remember_project_file` 仍只 `self.autosave.stamp = AutosaveStamp::capture(...)`、注释仍写"the autosave fingerprint restarts because the state is already on disk"，无任何 `save_autosave_at`（该函数全仓仅 autosave.rs:170/223 两处写文件）；②`project/startup.rs:27-28` 恢复仍无条件 autosave 优先于 last_project（测试 `project/tests.rs:978` 明文钉住该选择）；③`shell.rs:511` 恢复后 `project_io.path = last_project_path` 不区分恢复来源。审计的触发序列（开 P→不编辑→退出→下次启动恢复陈旧 Q 且 path=P→一次 Save 覆写 P）在当前代码上逐步推演依旧成立 |
| AS-3 | 启动 resume 剥离离线 IO 的 journaled RemoveNode 让指纹立即变脏，剥离图 1s 后写回 autosave | **未修** | `shell.rs:512-514` 指纹仍在 shell 构造期（首帧前）捕获；`service/managed/driver.rs:310-321` `resume_restored` 仍对每个离线 IO 执行 journaled `GraphEdit::RemoveNode`（candidate_revision+1）；`AutosaveStamp::capture` 全仓仅 4 处（构造期/写完成/开工程），无 resume settle 后重盖或宽限机制。目录晚于指纹到达（ASIO/多设备）时剥离图照写 autosave，离线节点跨会话永久丢失 |
| IN-1 | 压缩器 InputState 渲染期惰性创建打破"实体先于 notify"前提，首段键入盲打 | **部分修（需实测）** | 交互期创建的编辑器已恢复前提：balance/gain/pan 等"打开编辑器即改面板 key"（测试注释 `shell/tests.rs:1447-1450`），实体在同一次 update 内创建、先于 PanelHost 观察者效果运行。但压缩器族仍走渲染期对账：`shell.rs:841-862` `sync_presentation`（design 20 §5 render-time）调 `sync_compressor_controls`，而 `pages/graph/compressor.rs:76-79` 对新选节点仍 `entry().or_insert_with` 惰性建 InputState——选中通知触发的池捕获发生在渲染建实体**之前**，空闲会话下新选压缩器的输入实体不入池；grep 未找到压缩器族"新选节点键入即上屏"的回归测试。11-03 的失败窗口在压缩器族上疑似仍在，判部分修并列入实机待测 |
| NC-1 | kind→tint 三份手写映射漂移（ChannelMapper 各执一词） | **已修** | 单源 `pages/graph/node_catalog.rs:37` `kind_tint(kind)`；画布卡 `graph_canvas.rs:356` 调它；mixer 侧 `pages/mixer.rs:141` re-export 为 node_kind_color，tray_mixer.rs:346 同源——三表面一个函数 |

### P2（抽样核）

| ID | 标题 | 状态 | 证据 |
|---|---|---|---|
| GC-1 | 拎线拖拽中 Ctrl+Z 不取消手势，松手提交陈旧 EdgeId → 假 fault | **未修** | `shell.rs:1084-1115` undo_redo_graph 只 `cancel_active_drags`（滑杆槽），不含 `connection_drag` 等画布手势（drag_active 的五态在 :659-665）；`pages/graph/interactions.rs:605-660` `port_up` 松手仍直接以拖拽起点的 `picked.edge` 派发 Reconnect/Disconnect，无 revision/存在性校验 |
| GC-2 / IN-2 | 旋钮每指针移动派发结构编辑、无 undo 合并键 → 冲刷 100 深 undo 栈 | **已修** | `service/graph.rs:239-284` 落地 undo 合并：`undo_key()`（:242）+"How long successive same-key edits stay merged"（:261）+ `journal.record(before, edit.undo_key())`（:760）；旋钮拖拽值锚定按压值防反馈环（interactions.rs:182-184 注释）。IN-2 的"未进 cancel 游走"半边：undo 只取消滑杆槽，画布 knob_drag 不在其列——journal 已不被冲刷，残余风险降级为部分修 |
| IN-3 | Graph 页 ControlsKey 缺 catalog/backend，热插拔下图页冻结旧设备名 | **已修** | `pages/graph/page.rs:22-31` ControlsKey 现含 `catalog: ArcSnapshot<DeviceCatalog>` 与 `backend: AudioBackend`，注释逐字解释原因（"a catalog-only republish must rebuild…"） |
| SP-1 | 热层每帧为每张分析卡分配新 `Box<[f32]>` | **已修** | `pages/graph/page.rs:359-366` paint 闭包改写 shell 自有 `spectrum_scratch` 缓冲并借 `&[f32]` 画（`SpectrumCurve { points: &shell.graph_view.spectrum_scratch, …}`）；:411 注释"copied out at rebuild time … so the cached paint owns what it draws" |
| ST-1/2 | i18n 双向脱节（43 死键 + 4 生产键缺 zh-CN），双向零护栏 | **已修** | `i18n_guards.rs`（design 21 §5）双向护栏：缺译检测（:681-689）、孤儿键检测（:835+）+ `tests/i18n_ledger.rs`；TSV 台账已删（locales/ 仅 zh-CN.yml） |
| ST-4 | 绑定清单两处手写 ~130 行平行 | **已修** | `pages/graph/binding_choices.rs:1-7`："The one ordered binding choice list … they previously kept two ~130-line copies in step by comment alone"（issue #119 / design 21 §3.4） |
| ST-5 | curves.rs 逐行镜像 engine 私有 DSP 公式 | **部分修** | curves.rs 已删除；投影数学迁至 `monitoring/spectrum.rs` 并重写为"engine 归一化幅度 → ISO 266 分数倍频程 → dBFS"的显示层投影，带 ENBW=1.5 与文献引用（:32,132-139）——镜像消失，但投影公式仍属 app 侧实现（职责上可辩护） |
| AS-4 | 退出 flush 200ms 总预算串行 | **未核**（推定未修） | shell.rs 退出路径未见预算调整 |
| AS-5 | 损坏 autosave 静默丢弃 | **部分修** | `project/tests.rs:1027-1029` 钉住"损坏 autosave 回退 last project"；用户可见提示未见 |
| GC-3 / GC-7 / IN-7 / SP-2 | tooltip 残留 / 幽灵条目 / 详情窗零测试 / 通道整拷（P2/P3） | **未核**（启发式） | 画布代码随后续重构多轮改动，未逐一比对 |

### 频谱数学（README 摘要第 1 条的"好消息"）

PR #93/#95/#96 的数值核心经 11-03 手算亲证成立；本轮复核其载体仍在且未回退：`monitoring/spectrum.rs`（投影/ENBW/衰减）、`monitoring/ballistics.rs`（28.95 dB/s 常数化）、`meter_spans` 五表面单源。SP-1 的每帧分配已如上修复。

**小结**：P1 5 项：已修 2（AS-1、NC-1）、部分修 1（IN-1，压缩器族存疑）、**未修 2（AS-2、AS-3——正是两条指向用户数据损失的 autosave 语义洞）**。P2 抽样 9 项：已修 6、部分修 2、未修 1（GC-1）、未核若干。

---

## 2026-10-29 可考子集（repo-steward，报告未入库）

### 报告去向核实

- `find docs` 无任何 `*2026-10-29*`/steward 报告文件；`git log --all --grep steward -i` 仅命中 2026-10-19 时代的 remediation 清单提交（aa19314d/16bac7e5/8383d283）。**结论：2026-10-29 那轮（46 项）成果未入库、不可逐条追溯——审计成果丢失风险，登记为流程发现。**
- 旁证加重该风险：**`docs/audit/2026-11-09/`（6 个报告文件）同样是 untracked**（git status `??`）。仓库已连续出现两轮审计产物只存在于工作区、未提交入库的情况。建议：审计目录随审计结束立即提交；`docs/audit/README` 或索引文件登记各轮基线 commit。

### 五项可考条目逐条核现状

| # | 条目（外部记录措辞） | 状态 | 证据 |
|---|---|---|---|
| 1 | "服务循环 1ms 全量快照" | **仍开（结构未变，但 1ms 为需求门控）** | `service/mod.rs:447-510` 主循环每次迭代无条件的 `mailbox.publish(snapshot(&c))`；`snapshot()`（mod.rs:356-394）每次重建 AppSnapshot——含 `c.graph.snapshot()`（graph.rs:658-660，结构体克隆：nodes/graph/exports 表）与 `exports.statuses+reconcile`；循环节奏 `park_timeout(min(signal_wait,10ms))`，信号泵到期时为 1ms（signals.rs:112）。缓解面：泵与 1ms 唤醒是 demand-aware（无业务订阅时 park 至 10ms 或 park 无限）；但"每迭代全量重建+发布快照"的本体未变 |
| 2 | "Graph 页无缓存" | **已修（被重构消化）** | design 17/20 两层渲染落地：Graph 页 `ControlsKey` 缓存面板（pages/graph/page.rs:22-31，含 catalog/backend 键）+ 热层画布 + `rebuild_count` 测试 |
| 3 | "三 README 文档化已删栈" | **已修** | `crates/moiren-windows-audio/README.md` 已改写为 professional 单栈叙述，:581 明注 legacy 栈已于 2026-10-27 删除；docs/FRONTEND.md 已删；根 README 无已删栈描述 |
| 4 | "theme 17 死 token" | **已修** | 10-31 审计已全量清点 79 token 零死（其"既往对账"节）；11-03 复核 PR #93 新增 33 token 全存活；当前 `ui/theme.rs` 92 个 pub const 且别名关系有注释（:68/:197/:244，即 #153 色别名收敛后的形态）；i18n/icon 另有双向护栏测试防回潮 |
| 5 | "windows-core 死依赖" | **误报（当时即误判，现已文档化）** | `crates/moiren-windows-audio/Cargo.toml:65-67` 注释明言："windows-core is not referenced by handwritten code: the #[implement] macro's expansion references `::windows_core` directly"；`#[implement]` 实际用于 wasapi.rs:158（IMMNotificationClient）与 process_loopback/activation.rs:36；workspace 表还给出精确 pin 理由（:20-23） |

**结论**：5 项中 3 项已修（Graph 缓存、README、theme token）、1 项误报（windows-core）、1 项仍开但形态收窄（服务循环全量快照——1ms 已 demand-gated，全量重建/发布本体未动）。**另登记两项流程发现：10-29 报告本身丢失；11-09 轮报告未入库。**

---

## 结论

### 各轮修复率（以本轮亲证/可判定项计）

| 轮次 | 已修 | 部分修 | 未修 | 被重构消化 | 未核(P3 启发式) | 修复率 |
|---|---|---|---|---|---|---|
| 2026-10-27（重叠/死面） | 26 | 4 | 3 | （WA-2/3/4 计入已修·随退役消化） | 9 | ~76%（可判定 33 项中） |
| 2026-10-31（UI 键/族） | 19（P1×4 + P2×15） | 3 | 1（HC-8） | 3（HC-2、HC-7、HC-11/SD-4） | 少量 P2/P3 | P1 5/5；P2 ~83% |
| 2026-11-03（autosave 等） | 8（P1×2 + P2×6） | 3（IN-1、ST-5、AS-5） | 3（AS-2、AS-3、GC-1） | 1（ST-5 半边） | 若干 | P1 2/5 —— **四轮中唯一 P1 成建制未清的轮次** |
| 2026-10-29（可考 5 项） | 3 | 0 | 1（服务快照，收窄） | — | — | 1 项误报 |

**被重构消化是主导机制之一**：#109-#128 目录重组、#144-#168 的 design 17/20/21 战役消化了 10-31 的 2 条 P1 与多条 P2（旧形态消失且新形态有注释与测试锚定），且消化处普遍**优于**原修复建议（如 Matrix 页转 live、start-by-graph 删除指纹记忆化）。

### 建议立即处理（按风险排序）

1. **AS-2（P1，用户数据损毁）**：打开工程→不编辑→退出→下次启动恢复陈旧 autosave 且 path 指向该工程→一次 Save 覆写工程文件。修复方向审计已给（`remember_project_file` 重置指纹时同步写 autosave，或恢复路径按来源置 `project_path=None`）；三处位点至今原样（shell/project.rs:976-991、project/startup.rs:27-28、shell.rs:511）。
2. **AS-3（P1，跨会话数据丢失）**：resume 剥离离线 IO 的 journaled RemoveNode 弄脏指纹，1 秒后把"已删除节点"的图写回 autosave，undo 不持久、无恢复途径。driver.rs:314-321 与 shell.rs:512-514 的时序洞未动。
3. **X-1/X-3（P2，三轮连续 open 的结构债）**：engine/app 双节点 level 目录 + MeterBridge 三拷贝三泵，11-03 时 spectrum 又添一实例；建议按 10-27 方案 A（目录助手下沉）或 B（UI 直订 engine 订阅）立项，避免第四次审计再点名。

次优先：GC-1（拎线中 Ctrl+Z 提交陈旧 EdgeId 的假 fault，undo 不取消画布手势）、HC-8（热层标签每帧 shape_line + meter_spans 每帧 Vec）、IN-1（压缩器新选节点首段键入盲打，需实机验证）、以及流程项——**把 untracked 的 docs/audit/2026-11-09（与本次 2026-11-11）尽快提交入库，并确立"审计产出当轮入库"的约定**，避免 10-29 式成果丢失再次发生。
