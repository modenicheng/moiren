# 05 既往审计遗留处置 — 独立复核

- **输入**: 05-prior.a.md（2026-11-09 轮 49 项）+ 05-prior.b.md（2026-10-27 / 10-31 / 11-03 / 10-29 四轮）
- **基线**: main @ `e2ed3d6a`（2026-11-11），与两份初审同基线。
- **方法**: 只读。全部 P1 级判定逐条亲证；全部"已修"判定逐条亲证（防假修）；"未修"判定按 60% 抽取（05-A P1/P2 16 条 16/16 亲证、P3 亲证 18 条；05-B 三轮已修判定抽查 45+ 条）。原始发现一律回读对应审计轮原文（11-03/05-autosave-project.md、10-27/05-cross-crate-workspace.md、11-09/README.md 等）后对当前代码逐行核。未跑 cargo。

---

## 重点战役裁决（AS-2/AS-3 / X 家族 / 修复率 0%）

### 战役 A：AS-2 / AS-3（autosave 数据损毁路径）— **裁定：两条 P1 均成立，05-B 判定正确**

对照 2026-11-03/05-autosave-project.md 原文（AS-2 :52-73、AS-3 :75-94），当前 main 逐行验证：

**AS-2（打开工程→不编辑→退出→下次启动恢复陈旧 autosave 且一次 Save 覆写工程文件）——全链原样：**

1. `crates/moiren-app/src/ui/shell/project.rs:975-988` `remember_project_file`：仅更新 `last_project_path` 偏好 + `:987` 重置 `autosave.stamp = AutosaveStamp::capture(self, cx)`；注释（:974）仍写 "the autosave fingerprint restarts because the state is already on disk"。**无任何 autosave 文件写入**——`save_autosave_at` 全仓仅 `shell/autosave.rs:170`（防抖写）与 `:223`（退出 flush）两处调用。
2. `shell/autosave.rs:115` poll 守卫与 `:138` settle 守卫均为 `if !autosave_matches(cx)` 才写：打开 P 后状态==P==新指纹，`autosave_matches` 恒真 → P 状态永不写入 autosave 文件；`:212` 退出 flush 同守卫直接返回 None → 磁盘 autosave 保持陈旧 Q。
3. `crates/moiren-app/src/project/startup.rs:23-29`：恢复时 autosave 无条件优先 last_project（注释自陈该设计），测试 `project/tests.rs:977` `startup_restores_the_autosave_over_the_last_saved_project` 明文钉住该优先级。
4. `shell.rs:510`：恢复后 `project_io.path = last_project_path`，不区分恢复来源；`shell/project.rs:789` `save_project` 的 `known = self.project_io.path`（:783 起）——一次 Save 直写该路径。

**用户后果**：会话 N 打开 P 不编辑退出 → 磁盘 autosave 仍为 Q → 会话 N+1 恢复出 Q 的内容、菜单显示 path=P → 单次 Save 把 Q 覆写进 `P.moiren.json`。数据损毁路径逐步推演成立，与 11-03 原始描述逐字对应。

**AS-3（启动 resume 剥离离线 IO 的 journaled 编辑弄脏指纹，1s 后剥离图写回 autosave）——全链原样：**

1. `shell.rs:512-514`：指纹在 `StudioShell::new`（首帧前）捕获，注释原文仍宣称 "the first edit, not the launch, owns the first autosave write"。
2. `service/managed/driver.rs:313-321` `resume_restored`：对每个离线 IO 执行 journaled `c.graph.edit(GraphEdit::RemoveNode(io.node))`（`:286` 设备目录到达时触发；service/graph.rs 的 journal 机制确认该编辑 bump `candidate_revision`）；故障提示（driver.rs:353-358）仍只给 "undo (Ctrl+Z)"——会话级，undo journal 不持久。
3. `AutosaveStamp::capture` 全仓恰 4 处（autosave.rs:161/193、shell.rs:514、project.rs:987）——**无 resume settle 后重盖、无宽限机制**；`AUTOSAVE_DEBOUNCE = 1s`（autosave.rs:15）；poll 由 shell.rs:618 控制泵持续驱动。
4. 9e3a7bc..e2ed3d6a 共 226 提交，触及 autosave.rs / project/ / driver.rs 的 16 个提交全部是 refactor/feature（模块搬移、ProjectMessage 类型化、session ledger 删除），**无一个触及 AS-2/AS-3 语义**；无新增场景测试（全仓 grep 无"打开工程后指纹与 autosave 文件一致性"覆盖）。

**用户后果**：目录枚举晚于指纹捕获（ASIO/多设备慢枚举下几乎必现）时，剥离图 1 秒后写回 autosave，离线 IO 节点跨会话永久丢失；工程文件虽仍含节点，但被 startup.rs:27 的 autosave 优先级遮蔽。

**结论**：05-B 将 AS-2/AS-3 列为全审计最重结论，经复核**成立**。行号仅有数行漂移（remember_project_file 实为 :975-988；driver.rs 剥离 :313-321；shell.rs 指纹 :514）。

### 战役 B：X-1/X-2/X-3 双总线家族 — **裁定：X-1 已被 11-09 裁决性关闭（转述有误、依据成立、行动项未做）；X-2/X-3 仍开放**

1. **对象同一性**：10-27 原始条目（05-cross-crate-workspace.md:41-45）X-1 的诉求是「节点→level topic **目录与 prune 双份实现** + 两处 topic 命名格式各持一份」，建议 A=共享目录助手下沉、B=UI 直订 engine 订阅删第二条总线与 MeterBridge。**11-09 裁决记录（README.md:48-53）称"原诉求（消除类型镜像）已满足"——对原始诉求的转述不实**：10-27 条目从未提"类型镜像"；类型单源化是 11-03 spectrum 战役引入的新面（11-09/04 报告即以此口径对账）。11-09 主代理的"双目录=刻意分层中继，非重复实现"是一次**设计再裁决**（收编 11-03「维持独立决策」的开放状态），不是"原始诉求已被实现"。
2. **裁决的技术依据成立（亲证）**：类型层确已单源——`ui/monitoring.rs:8` 直接 `use moiren_engine::meter::signal::{AudioWindow, ChannelLevel}`，`ui/monitoring/spectrum.rs:12` 直接 `use moiren_engine::spectrum::SpectrumWindow`，全仓无本地镜像；双总线各有存在理由（engine `host/signals.rs:295` `owner/{gen}/node/...` 轮换命名 vs app `service/signals.rs:52` 稳定 `node_levels` 目录），`managed/meters.rs:1-3` 模块头与 `:19-22` 注释描述了中继形态。**"X-1 关闭"作为设计决策可接受、予维持。**
3. **两点残留**：(a) 裁决行动项（meters.rs 模块头加一行指向该裁决的锚点注释）**未执行**——05-A 登记正确，meters.rs:1-3 仍是原三行 doc；(b) 11-09 裁决只点名 X-1，**X-2/X-3 未被覆盖**：X-2 双 pump 调度（engine `meter/polling.rs:8` MeterPollingTimer + `host.rs:61` 默认 5ms vs app `service/signals.rs:112` 1ms）与 X-3 三拷贝三泵（`managed/meters.rs:35-40` MeterBridge + `:88` poll；spectrum 又添 `SpectrumSlot` 实例）代码实体原样，且 X-3 的"把拷贝/延迟预算写进设计文档（design 14）"行动项也未做——design 14 无任何 meter/MeterBridge 内容。
4. **05-B 口径修正**："三轮连续 open 未修"应改为：X-1 代码实体确实三轮未动（engine host/signals.rs 802 行 / app service/signals.rs 608 行，双侧持续生长），但已在 11-09 被裁决性关闭（决策而非修复）；**该家族还剩的真问题 = X-2/X-3 的结构收敛 + X-1 锚点注释（1 行）**。下一次审计不应再把 X-1 本体翻出来重审，但应检查锚点注释与 X-2/X-3。

### 战役 C：11-09"修复率 0%" — **裁定：成立（超额抽查 8/8 全未修）**

从 27aa57d7..e2ed3d6a 声称未修的发现中抽查 8 条（要求 5 条），逐条亲证对应代码原样：

| 抽查 | 亲证结果 |
|---|---|
| W-1 | `.github/scripts/detect-changes.sh:53-62`（ORDER/direct 均 5 crate）、`:71-79` case 分支无 `crates/moiren-plugins/*`；workflow push 硬编码 5 包；全 .github 无 moiren-plugins —— 未修 |
| W-2 | AGENTS.md 全文无 moiren-plugins/driver/ 提及，:34 "pushes to main always run the full suite" 失实陈述在 —— 未修 |
| APP-1 | `service/graph.rs:562-576` edit_hint 仍 5 组英文子串匹配，`:55` `GraphFault.cause: String` —— 未修 |
| S1 | `moiren-signal/src/lib.rs:6-8` "carries no production traffic yet…Latest carries all current publication paths" 原文 —— 未修 |
| 06:P1 | `clap/abi.rs:84-86` "atomic counter the control side drains opportunistically" 原文；`clap/mod.rs:86-88` 三个空 no-op —— 未修 |
| AU-1 | `pages/graph/compressor.rs:3` 仍 `use crate::ui::shell::mixer::{...}`；`pages/mixer.rs:1` 反向 `use super::graph::{...}`；model.rs 1458 行 —— 未修 |
| WA-02 | `wasapi.rs` 恰 1231 行 —— 未修 |
| WA-05 | `asio_rt_tests.rs:1`、`engine_backend/recovery_host_tests.rs:1-2` 头部仍无"仅由 tools/rt-check 编译"注释 —— 未修 |

**计数修正**：区间实测 46 提交（含 merge；30 非 merge；12 first-parent）——05-A 的"40 提交"不准确（76 文件变更准确），不影响结论。提交清单复核确认全部属 #158-#168 平行功能线，无任何提交以 11-09 审计发现为对象。**修复率 0%（0/49）维持。**

---

## 逐条判定表

判定值：成立＝初审状态维持；改判＝初审状态需修正（详见"误报/改判说明"）。"推定"条目沿用初审的零触碰推定并注明。

### 2026-11-09 轮（初审 A，49 条 + 2 行动项）

| ID | 初审状态 | 复核状态 | 依据（path:line + 一句话） |
|---|---|---|---|
| W-1 (P1) | 未修 | 成立（亲证） | detect-changes.sh:53-79 无 moiren-plugins；workflow:48-49 硬编码 5 包 |
| W-2 | 未修 | 成立（亲证） | AGENTS.md 无 plugins/driver；:34 失实陈述在 |
| APP-1 | 未修 | 成立（亲证） | service/graph.rs:562-576 子串匹配 + :55 cause String |
| APP-2 | 未修 | 成立（亲证） | project/startup.rs:44-48 into_context 仍构造 ServiceContext |
| APP-3 | 未修 | 成立（亲证） | service/mod.rs:112/306/395 三同居一文件（572 行） |
| AU-1 | 未修 | 成立（亲证） | compressor.rs:3 → shell::mixer；mixer.rs:1 → graph；model.rs 1458 行 |
| AU-2 | 未修 | 成立（亲证） | shell/project.rs 1055 行；menu() :13 起单函数；ProjectMessage×32 同居 |
| AU-3 | 未修 | 成立（行数亲证） | node_inspector.rs 1315 行，结构未动 |
| E-1 | 未修（checklist 未写） | 成立（亲证） | "checklist" 仅命中旧审计与 design 17；无新增节点文档 |
| E-2 | 未修（按建议冻结） | 成立（亲证） | supervisor.rs:51-81 Supervisor 恰 28 字段平铺 |
| WA-01 | 未修 | 成立（亲证） | 三个 owner.rs（src/、engine_backend/、virtual_owner/）原位 |
| WA-02 | 未修 | 成立（亲证） | wasapi.rs 1231 行 |
| WA-05 | 未修 | 成立（亲证） | 两影子测试文件头无归属注释 |
| WA-08 | 未修 | 成立（亲证） | lib.rs 一句话文档；docs/platform/ 仅 gpui-fork.md |
| S1 | 未修 | 成立（亲证） | signal lib.rs:6-8 原文 |
| 06:P1 | 未修 | 成立（亲证） | clap/abi.rs:84-86 + clap/mod.rs:86-88 |
| P3×33 | 0/33（17 亲证+16 推定） | 成立（18 条亲证 + 15 条推定维持） | 亲证：W-3（rt-check 4 pin）、W-4（vad×5+双计划目录）、APP-4（mod.rs glob）、APP-5（app/src/timeline.rs:1 + lib.rs:20 挂载）、APP-7（mailbox.rs:214 无 cfg(test)）、APP-8（graph.rs:406）、APP-9（graph/mixer.rs:46-52 t!）、APP-10（exports/runtime.rs 540 行）、AU-4（devices.rs:110 mount+:126 page 双形状等三形状原样）、AU-5（model.rs:345 parse 家族与 presentation.rs:296-329 双源）、AU-6（graph/tests.rs 3654 行）、AU-7（pages/mixer.rs:26 impl Shell）、AU-8（mixer.rs:14-16 旧路径注释）、E-3（swap.rs:57 内联 mod）、WA-03（negotiation.rs:169/:195 + provider.rs:77 原文）、06:P2（scan.rs:154-157 注释 vs format_of :157 无目录检查；扫描根含 VST_PLUGIN_PATH 等通用目录，矛盾实际存在）、06:P4（vst3/mod.rs 1464 行）、S4（demand.rs:83 `limit.min(64)` 字面量在）；其余 15 条推定未修维持 |
| 行动项：X-1 锚点注释 | 未加 | 成立（亲证） | meters.rs:1-3 原三行 doc，无裁决锚点 |
| 行动项：12 项"明确不建议" | 零违反 | 成立（抽查） | members 6 crate；gallery feature 门（Cargo.toml:39/:51）；mixer 四子模块；graph.rs 未拆 |

### 2026-10-27 轮（初审 B，功能与逻辑重叠审计）

| ID | 初审状态 | 复核状态 | 依据 |
|---|---|---|---|
| WA-1 | 已修 | 成立（亲证） | managed.rs:231/243 f32/f64 无条件 ProfessionalBackend，无 legacy 分支 |
| WA-2 | 已修（随退役消化） | 成立（亲证） | bridge.rs:331 `sin() * 0.1` 唯一实现 |
| WA-3 | 已修（随退役消化） | 成立（亲证） | process_loopback.rs + process_loopback/ 单模块 |
| APP-1 | 已修 | 成立（亲证） | 全仓 BackendDriver 零命中 |
| APP-2 | 已修 | 成立（亲证） | shell.rs:1033-1047 typing_focus 走 text_input_entities |
| EN-1 | 已修 | 成立（亲证） | engine/meter.rs 56 行；LevelMeter/bind_meter 零命中 |
| EN-2 | 部分修 | 成立（亲证） | meters.rs:159 subscribe_spectrum 在；compressor reduction 与 RecordingSession/record_wav app 侧零命中 |
| EN-3 | 已修 | 成立（亲证） | host/project.rs:164 单一 node_specs；:260 走 storage_valid&&specs_accept_initials |
| EN-4 | 已修 | 成立（亲证） | linked_io! 7 处使用 |
| EN-5 | 部分修 | 成立（亲证） | delay.rs:35 storage_valid 单源被 rack.rs:279/project.rs:260 复用；统一 prepare 入口无 |
| EN-6 | 已修 | 成立（亲证） | transport_interchangeable 单定义（bindings.rs:393）被 plan.rs:123 复用 |
| EN-7 | 已修 | 成立（亲证） | host/signals.rs:729 泛型 prune_slots |
| EN-8 | 已修 | 成立（亲证） | processor/builtin/compressor.rs:219 validate_linked_io |
| EN-9 | 部分修 | 成立（亲证） | processor/builtin/mod.rs:59 `pub use sum::Sum as Bus` 双名仍在 |
| APP-3 | 已修 | 成立（亲证） | meters.rs meter_spans 共享迭代（paint 双向） |
| APP-4 | 已修 | 成立（亲证） | mixer_edit 唯一定义 model.rs:596 |
| APP-5 | 已修 | 成立（亲证） | tray_mixer.rs:33 widgets::fader::fader |
| APP-6 | 已修 | 成立（亲证） | service/graph.rs:127 node_of_kind |
| APP-7 | 已修 | 成立（亲证） | mod.rs:384 tone_session 入 snapshot；monitoring.rs:233 消费 |
| APP-8 | 已修 | 成立（亲证） | presentation.rs:296-329 db_text 族单源 |
| APP-9 | 已修 | 成立（亲证） | service 层（非测试）中文字面量零文件；UiFailure 类型化（state.rs:105） |
| APP-10 | 已修 | 成立（亲证） | locales/ 仅 zh-CN.yml |
| APP-11 | 已修 | 成立（亲证） | state/tests.rs + transport_tests.rs 独立 |
| APP-13 | 已修 | 成立（亲证） | node_inspector.rs:139 t!("rename") |
| CS-3 / CS-4 | 已修 | 成立（亲证） | validate_connection / GraphIndex 全仓零命中 |
| HC-13 | 已修 | 成立（亲证） | gain_slider/pan_slider 零命中 |
| WA-6（小结注） | 部分修（剩两处） | **改判：已修（收敛）** | 当前仅 wasapi.rs:109-111 一处 role→ERole 映射（resources.rs/catalog.rs 映射随 legacy 退役消失） |
| X-1 | 未修 | **改判口径：裁决性关闭（11-09）** | 见战役 B；代码实体未动（host/signals.rs:295/:542 + app signals.rs:52/:161-208） |
| X-2 | 未修 | 成立（亲证） | engine meter/polling.rs:8 + host.rs:61（5ms）vs app signals.rs:112（1ms）双调度 |
| X-3 | 未修 | 成立（亲证） | meters.rs:35-40/:88 MeterBridge+poll 原样；design 14 无拷贝/延迟预算记载 |
| X-5 | 已修 | 成立（亲证） | driver/vad/STATUS.md:5-9 头条即 eight-hour idle PASS；vad-validation 头部有 10-22 Update |
| X-6 | 已修 | 成立（亲证） | 根 Cargo.toml [workspace.dependencies] 集中 + pin 理由注释 |
| X-7 | 已修 | 成立（亲证） | find docs 无 *.stderr.log |
| X-8 | 已修（删除消化） | 成立（亲证） | docs/FRONTEND.md 不存在 |
| X-9 | 已修 | 成立（亲证） | designs/ 仅 12-unified-topology；gpui 已改 15 |
| X-10 | 已修 | 成立（亲证） | .gitignore:45/:61 .worktrees/ |
| X-4、APP-12、CS-1/2、CS-5..8 | 未核/启发式 | 维持未核 | 未逐行比对（X-4 clock_bridge/ 仍在，与"未核"一致） |

### 2026-10-31 轮（初审 B，UI 键/族审计）

| ID | 初审状态 | 复核状态 | 依据 |
|---|---|---|---|
| HC-3 | 已修 | 成立（亲证） | panel.rs:163-177 观察者仅 key/locale 比较；"No empty-pool retry" 注释在 |
| SD-1 | 已修 | 成立（亲证） | sidebar.rs:108+ observe_nested 单遍历 + "Missing any family leaves that editor blind-typing" 注释 |
| SD-2 | 已修 | 成立（亲证） | 同 APP-2 证据（shell.rs:1033-1047） |
| HC-1 | 已修 | 成立（亲证） | shell.rs:751 set_page 首行 cancel_active_gesture；:545-547 失活取消 |
| HC-2 | 被重构消化 | 成立（未逐行重验，头注释口径一致） | matrix 页转 live 形态在 |
| HC-4 | 部分修 | 成立（亲证） | settings.rs:39-51 Key 含 scroll/meters/density/pending_request/project_message；无 viewport 高度项 |
| HC-5 | 已修 | 成立（亲证） | shell.rs:701/:732 legacy_meters_shown 派生 |
| HC-6 | 部分修 | 成立（亲证） | model.rs:155 cancel_drag 落地；卸载复位路径未见 |
| HC-8 | 未修 | 成立（亲证） | fader.rs:115-135 paint_text 每帧 shape_line；meters.rs:225 meter_spans 返回 Vec |
| HC-13/HC-14 | 已修 | 成立（亲证/提交在） | 零命中；2f1bb23b/0a4f604e 在 git log |
| SD-3 | 已修 | 成立（亲证） | model.rs:345 parse_gain_db 统一（U+2212/∞/大小写） |
| SD-5 | 已修（按 B 方案） | 成立（亲证） | ballistics.rs:30-39 RMS_INTEGRATION=150ms + 10·log10(e)/τ≈28.95 常数化 |
| SD-6/SD-7/SD-8/SD-9/SD-10/SD-11 | 已修 | 成立（亲证/提交在） | 23062ba8 在；shell.rs:1104-1106 cancel_active_drags（SD-7 注释）；:707 update_node_meters(visible)；muted/dimmed 零命中；UiFailure 类型化 |
| HY-3 | 已修 | 成立（亲证） | design 17 :358 "render-time refresh_observation_nodes runs" 与代码一致 |
| HY-10 | 已修 | 成立（亲证） | shell/tests.rs:1514 inspector_hot_layer_paints_the_registered_meter_tracks |
| HY-11 | 已修 | 成立（亲证） | panel.rs:155 rebuild_count；devices(4)/graph(1)/mixer(2) 处消费 |
| DC 区（死码/gallery/icons） | 已修 | 成立（亲证） | Cargo.toml:39/:51 component-gallery 门；assets/icons 35 文件 |
| HY-4 | 部分修 | 成立（口径并入战役 B） | X-1 已于 11-09 裁决关闭 |

### 2026-11-03 轮（初审 B，节点卡/画布/Inspector/频谱/autosave）

| ID | 初审状态 | 复核状态 | 依据 |
|---|---|---|---|
| AS-1 | **已修** | **改判：部分修（仍具 P1 危害）** | harness 存在且 devices 已迁移，但 exports/tests.rs 三处直建未置 autosave.path=None——详见改判说明 1 |
| AS-2 | 未修 | 成立（亲证，P1） | 见战役 A |
| AS-3 | 未修 | 成立（亲证，P1） | 见战役 A |
| IN-1 | 部分修（需实测） | 成立（亲证代码面） | compressor.rs:76-79 `entry().or_insert_with` 渲染期惰性建 InputState 原样；无新选节点键入回归测试 |
| NC-1 | 已修 | 成立（亲证） | node_catalog.rs:31 kind_tint 单源；graph_canvas.rs:356、mixer.rs:141 re-export、tray_mixer.rs:24 同源 |
| GC-1 | 未修 | 成立（亲证） | interactions.rs:605+ port_up 直接以拖拽起点 picked.edge 派发 Reconnect/Disconnect，无 revision/存在性校验；shell.rs:1104-1106 undo 只 cancel_active_drags（滑杆槽），不含 connection_drag |
| GC-2 / IN-2 | 已修（IN-2 半边部分） | 成立（亲证） | service/graph.rs:239-254 undo_key + :257 UNDO_COALESCE_WINDOW=500ms + :258 UNDO_DEPTH=100 + record 合并 |
| IN-3 | 已修 | 成立（亲证） | graph/page.rs:22-31 ControlsKey 含 catalog+backend，注释解释原因 |
| SP-1 | 已修 | 成立（亲证） | graph/page.rs:359-366 paint 闭包写 shell.spectrum_scratch |
| ST-1/ST-2 | 已修 | 成立（亲证） | src/i18n_guards.rs + tests/i18n_ledger.rs:672/:682 双向护栏测试 |
| ST-4 | 已修 | 成立（亲证） | binding_choices.rs:1-8 注释自述合并两份 ~130 行拷贝 |
| ST-5 | 部分修 | 成立（亲证） | curves.rs 已删；monitoring/spectrum.rs ISO 266/ENBW 投影（app 侧实现） |
| AS-4 | 未核（推定未修） | 维持 | 退出预算未见改动 |
| AS-5 | 部分修 | 成立（亲证） | storage.rs:111-121 损坏仍 eprintln+None；回退 last_project 有测试（project/tests.rs:1003-1029）；用户可见提示无 |
| GC-3/GC-7/IN-7/SP-2 | 未核 | 维持 | 启发式范围 |

### 2026-10-29 轮（可考 5 项）

| 条目 | 初审状态 | 复核状态 | 依据 |
|---|---|---|---|
| 服务循环 1ms 全量快照 | 仍开（结构未变，1ms demand-gated） | 成立（亲证） | service/mod.rs:448 附近每迭代无条件 `mailbox.publish(snapshot(&c))` + park_timeout；snapshot() 全量重建 |
| Graph 页无缓存 | 已修（被重构消化） | 成立 | ControlsKey 两层缓存 + rebuild_count（见 HY-11） |
| 三 README 文档化已删栈 | 已修 | 成立 | windows-audio README 单栈叙述；FRONTEND.md 已删 |
| theme 17 死 token | 已修 | 成立 | 10-31 全量清点 + 护栏测试防回潮 |
| windows-core 死依赖 | 误报（当时即误判） | 成立 | Cargo.toml 注释说明 #[implement] 展开引用 |

流程发现（10-29 报告丢失、11-09 untracked）维持成立：docs/audit/ 无 2026-10-29 目录；`git status` 确认 docs/audit/2026-11-09/ 与 2026-11-11/ 均 untracked。

---

## 误报/改判说明

1. **AS-1（2026-11-03, P1）：05-B 判"已修" → 改判"部分修（未竟，仍具 P1 危害）"。** 本次复核唯一 P1 级改判。证据链：
   - `shell.rs:798-808` `shell_for_tests` 确实强制 `preferences.path = None; autosave.path = None`（注释 "Headless tests must never write the developer's real configuration"），`test_harness.rs:124-136` test_shell_body 同；devices/tests.rs 经 `test_shell(cx)` 构造（:323），shell/tests.rs:1707 直建后双置空，capture.rs:325 置空——以上安全。
   - 但 `pages/devices/exports/tests.rs:235/:340/:561` 三处**直建** `cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None))`，随后只置 `shell.preferences.path = None`，**未置 `shell.autosave.path = None`**。`StudioShell::new` 第 4 参是 `desktop: Option<WeakEntity<Desktop>>`（shell.rs:298-302）而非持久化开关；`shell.rs:429` 仍无条件 `path: crate::project::autosave_path().ok()` 从真实配置目录派生。
   - 危害链完整：三个测试经 `.edit(GraphEdit::SetExports)`（:92/:118）与 `dispatch(AppRequest::EditGraph)`（:410/:466）及点击 export-create 产生 journaled 编辑 → candidate_revision 变脏；`shell.rs:554-560` `on_app_quit` → `autosave_flush`（autosave.rs:210-227）判脏即原子写。在开发机执行 AGENTS.md 记载的 `cargo test --locked -p moiren-app --features gpui-test` 会把测试图写入开发者真实 `%APPDATA%\Moiren\autosave.moiren.json`——正是 11-03 AS-1 原始描述的后果。05-B 的"grep 无直建绕过"与事实不符（`rg "StudioShell::new"` 即见三处）。
2. **X-1 状态口径（2026-10-27）**：05-B "未修（三轮 open）" → 改 "裁决性关闭（11-09，决策非修复；转述原诉求有误但技术依据成立；锚点注释行动项未做）"。X-2/X-3 维持未修，且仍是该家族剩余真问题。
3. **WA-6（2026-10-27 P2 注）**：05-B "现剩 catalog.rs/wasapi.rs 两处，判部分修" → 当前 main 仅剩 wasapi.rs:109-111 单点映射，改判已修（随 legacy 退役收敛，非按建议加 helper）。
4. **06:P2（11-09）措辞澄清**：当前注释（scan.rs:154-157）已含 "in such a context" 对冲措辞，比 05-A 转述显得温和；但扫描根含 VST_PLUGIN_PATH、ProgramFiles/VSTPlugins 等非 VST2 命名目录，且 format_of 无目录检查——发现本身成立，维持未修。区间内 scan.rs 零触碰。
5. **计数修正**：05-A "40 提交" → 实测 46（含 merge）/30（非 merge）/12（first-parent）；"76 文件" 准确。两份初审的个别行号有数行漂移（NC-1 :31 非 :37；UiFailure :105 非 :134；HY-10 :1514；i18n 护栏现为 src/i18n_guards.rs + tests/i18n_ledger.rs 布局），均不影响判定。

## 复核补充发现

- **V-1（并入 AS-1 改判，不单独立项）**：`pages/devices/exports/tests.rs:235/:340/:561` 直建 shell 未置 `autosave.path = None`——修复为三处各加一行或改走 `test_shell`；建议同时把"测试 shell 必须无持久化"收敛为唯一构造路径（11-03 已给过同方向建议）。
- 无其他新增——抽查范围内未发现两份初审漏报的实质问题。

## 修订后统计与建议

**修复率（修订后）**：

| 轮次 | 修订后判定 |
|---|---|
| 2026-11-09（49 项） | **0/49（0%）维持**（8 条超额抽查全未修；46 提交全是平行功能线） |
| 2026-10-27（可判定 34） | 已修 27（+WA-6）、部分修 3（EN-2/EN-5/EN-9）、X-1 裁决性关闭、未修 2（X-2/X-3） |
| 2026-10-31（P1×5+P2 抽 20） | P1 5/5 维持；P2 已修 14、部分修 3、未修 1（HC-8）、消化 2 |
| 2026-11-03（P1×5+P2 抽 9） | P1：**已修 1（NC-1）、部分修 2（AS-1、IN-1）、未修 2（AS-2、AS-3）**——较初审的 2/5 再降，仍是四轮中唯一 P1 成建制未清的轮次；P2 已修 6、部分修 2、未修 1（GC-1） |
| 2026-10-29（可考 5） | 3 已修、1 误报、1 收窄未修 |

**建议立即处理（修订后排序）**：

1. **AS-2（P1，用户数据损毁）**——修复方向不变（remember_project_file 重置指纹时同步写 autosave，或恢复路径按来源置 project_path=None）；三处位点原样（shell/project.rs:975-988、startup.rs:23-29、shell.rs:510/:789）。
2. **AS-3（P1，跨会话数据丢失）**——指纹捕获延迟到 resume settle 之后或忽略剥离引起的 revision 变化；driver.rs:313-321 与 shell.rs:512-514 时序洞未动。
3. **AS-1 残留 + W-1 同批一个 PR**——exports/tests.rs 三处补 `autosave.path = None`（或改走 test_shell，共 ~3 行）是全审计最便宜的两条 P1 之一；与 W-1（detect-changes.sh 四处 + workflow 两行 + AGENTS.md，~25 行）合一个 PR 当天可清。
4. **次优先**：APP-1 GraphFault 类型化（仍在被 #164/#165 加码施工）；X-2/X-3 结构收敛立项（避免第四次点名）；X-1 锚点注释（1 行）、HC-8、GC-1 顺风车。
5. **流程**：docs/audit/2026-11-09/ 与 2026-11-11/ 仍 untracked——审计产出当轮入库的约定仍未建立，两轮成果仍处丢失风险中。
