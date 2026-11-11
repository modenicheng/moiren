# 01 效率与渲染开销 — 独立复核

- 输入: 01-efficiency.a.md (A-1..A-8) + 01-efficiency.b.md (B-1..B-7)
- 复核基线: main @ e2ed3d6a（与初审相同），复核方法: 每条亲自打开被引位置核对代码与调用链；禁跑 cargo。
- 结论概览: 15 条全部成立，无误报；2 条标注需修正（A-3 的 NEW、B-1 的 KNOWN 依据），B-7 建议并入 B-3，B-2 视 B-1 落地降级。初审的行号引用普遍准确（个别 2–20 行漂移，见下）。

## 逐条判定

| ID | 初审判定 | 复核判定 | 依据（path:line + 一句话） |
|---|---|---|---|
| A-1 | P1 NEW | 成立（行号微漂移） | `pages/graph/page.rs:22-49` ControlsKey 确含 camera/positions/connection/preferences；`pages/graph.rs:393`（初审写 388-390）page() 每次 render 调 `CanvasWorld::positioned`；`interactions.rs:272,280` 拖卡/平移每次 move `cx.notify()`；`graph_canvas.rs:342-463`（format!/t!/to_string 每节点）、`580-581`（每边 O(n) find）；`graph_canvas_view/nodes.rs:15-54` shape_text 溢出走二分多次 shape + `format!("{}…")`，`73-105` paint_label 无缓存。rubber-band 例外属实（`page.rs:225,263`；`page.rs:301-306` 注释明确 box select 不重建）。 |
| A-2 | P2 KNOWN-OPEN | 成立 | `widgets/meters.rs:86-101` 每帧每通道构造 "L"/"R"/序号 label 后走无缓存 paint_text；`fader.rs:113-136` paint_text 每次 `text.to_owned()`+shape_line；`meters.rs:223-256` meter_spans 每次 `Vec::with_capacity(4)`。热层每帧调用链属实：`pages/mixer/page.rs:811,834-845` MixerMeters HotSurface::paint → `paint_mixer_meter_frame:867` → `:903` paint_channel_meter。ReadoutShapes 已修属实（`page.rs:762-807`）。HC-8 原文核对一致（2026-10-31/03-hot-layer-bugs.md:90）。 |
| A-3 | P3 NEW | 成立（NEW 标注需修正为"现象部分已知"） | 键族全部亲证：`shell/sidebar.rs:22,68`、`shell/chrome.rs:202,255`、`shell/tray_mixer.rs:186,204`、`pages/settings.rs:40,63`、`pages/devices.rs:31,54` 均以 `PublishRef<AppSnapshot>` 为键；`mailbox.rs:250-255` 注释确认发布仅在真实事件换 Arc；`service/mod.rs:385-388` elapsed 整秒量化 → 运行中 ≥1Hz 换新。**修正**：1Hz 重建现象本身已在 2026-11-03/03-inspector-keys.md:149 记载（"面板重建只随发布发生：运行中 ~1Hz（elapsed 量化）"，作为 IN-8 冻结读数的机理注脚），A-3 的新意在于把它作为"失效范围过大的开销"立项并覆盖全键族。 |
| A-4 | P3 NEW | 成立 | `shell/panel.rs:162-179` observer 在每次 owner notify 上捕获 key（先比较后 splice，捕获成本固定付出）亲证；`page.rs:52-75` ControlsKey::new 克隆 positions/preferences/desired/selected_nodes；`sidebar.rs:65-87` InspectorKey 克隆 observation/selected_device/preferences；`settings.rs:59-73`（初审写 59-73，实际 `:62` `global: settings.global.clone()`）克隆整个 Draft。 |
| A-5 | P3 NEW | 成立 | `pages/graph/page.rs:312` 每帧 `overlays.rects()`；`overlays.rs:27` rects 返回 Vec；`page.rs:328-341` covered 检查对每节点扫 `nodes[index+1..]`（O(n²) intersects）；`page.rs:368-372` 每节点 `carve()`（`overlays.rs:58` 返回 Vec）；`pages/mixer/page.rs:965-981` 悬停故障 strip 每帧 `faults_for_node` 克隆 cause/hint String 成 `Vec<CardFault>`。均在热层每帧路径，亲证。 |
| A-6 | P3 KNOWN-OPEN | 成立 | `graph_canvas_view/nodes.rs:519-652` paint_spectrum_body：3 条 dB hairline（551-560）+ 十进频率线逐条（566-583，默认跨度 ~27 条）+ 3 个 decade 标签 + FFT detail 四次 `paint_anchored_text`→shape_line；热层每帧每分析卡调用（`page.rs:375-392` 循环亲证）。SP-6 原文核对一致（2026-11-03/04-spectrum-monitoring.md:27,90，引 561-583 与现状吻合）。 |
| A-7 | P3 NEW | 成立 | `pages/matrix.rs:1-9` 模块头两条不缓存理由亲证；`:153` 每格 `format!("route-{ri}-{ci}")`、`:155` `t!`、`:147` db_text；全页随 publish 重建（页面不在 CachedPanel 内）。规模小、结构性平方级位点判断成立。 |
| A-8 | P3 NEW | 成立 | `pages/graph/node_inspector.rs:544-547` OBSERVATION_SAMPLE_CADENCE=200ms，注释自认 "every re-key rebuilds the cached inspector panel"；`pages/mixer.rs:99-135` sample_inspector_observation 到期改 `inspector.observation` 并 notify；`observation` 字段确在 InspectorKey（`sidebar.rs:76` 附近亲证）。与 11-03 IN-8 的关系：IN-8 记录的是修复前"观测不在键"的冻结缺陷，issue #118 修复后转为本条的开销问题——NEW 标注成立。 |
| B-1 | P2 KNOWN-OPEN | 成立（KNOWN 依据弱，见说明） | 主循环 `service/mod.rs:453-512`、`:489` 每轮无条件 `mailbox.publish(snapshot(&c))`；`:356-390` 每轮深拷贝 GraphSnapshot（`graph.rs:657-659` = `self.snapshot.clone()`）+ exports statuses/reconcile + 多项 clone；`model.rs:187-224` PartialEq 对 graph 按值比较且注释自认 "graph Arc is rebuilt per snapshot"；1ms 节奏亲证（`signals.rs:112-113` next_poll=+1ms、`mod.rs:496-508` park_timeout ≤ signal_wait）。**修正**：仓库内可追溯依据 X-3（10-27/05:51-54）只覆盖 meter 链路 3 拷贝/3 层轮询，不含"快照构建+全量比较"本身；KNOWN 判据主要依赖任务简报转述的 2026-10-29 审计（不在仓库）。按实质可视为 NEW（跨轮转述无法核实的部分除外）。 |
| B-2 | P2 NEW | 成立 | `driver.rs:156,184`（初审写 182，实际 184）update_vad_catalog 每 poll 两次；`:209` 每次 `context.exports.statuses(&context.graph.snapshot())` 深拷贝；`:215-233` BTreeSet 构建 + `owned != catalog.vad_endpoints` 比较；`:379` poll_exports 每轮深拷贝；`:548`、`:610`、`:618` 仅为读一个 revision 字段做整快照深拷贝，全部亲证。 |
| B-3 | P3 NEW | 成立 | `supervisor/snapshot.rs:5-80` 每轮无条件重建 state 值字段（`reason.clone()` :16、`fault.clone()`/`cleanup_error.clone()` :77-78）；`supervisor.rs:238` 每轮 pump、`:246-247` set_demand(has_demand()) + poll_interval() 内部再 has_demand()（`host/signals.rs:525-541` 各自 lock+扫描）；`signal/registry.rs:176-179` has_business_demand 遍历全部槽；`host.rs:57-61` max_topics=256、默认 poll 5ms。app 侧 `service/signals.rs:96-120` pump_if_due 与 wait_duration 各扫一次，亲证。 |
| B-4 | P3 NEW | 成立（前提微修） | `exports/runtime.rs:271-323` work() 仅以 adapter/in_flight/shutdown_complete/next_poll 门控，空 actions 也返回 Some(Work)；`:349` submit 后 next_poll=+20ms；`WindowsAdapter::execute`（:375）对空批仍无条件走 `export_observations()`（~:457）+ `export_name_observations()`（~:468）；`vad/provider.rs:129-146` 三把锁亲证；`driver.rs:425-430` 不检查 actions 为空即提交。**微修**：`managed.rs:225,254-257` adapter 恒为 Some 仅在 `WindowsVadProvider::discover()` 成功时成立（Windows 正常环境成立，非绝对）。 |
| B-5 | P3 NEW | 成立 | `bridge.rs:71-90` RingSink::write 逐帧 `tx.push` 且 channel(0)/channel(1) 循环内求值；`:234-268` RingSource::read、`:352-391` ProcessSource::read 逐帧 `channel_mut`；`engine_backend/owner.rs:493-497` Pump capture 逐帧 push；`asio/callback.rs:249-291` 逐帧 push/pop + 逐通道 ptr::copy；`buffer/view.rs:239-243,256-260` channel()/channel_mut() 每次 assert+stride。chunk API 先例亲证（`bridge.rs:202-210` read_chunk）。判级 P3 合理（无分配无锁，非瓶颈）。 |
| B-6 | P3 KNOWN-OPEN | 成立 | `meter.rs:18-20` ui_window_frames=sr/240；`meter/signal.rs:144-157` 每 240 帧发布一次；engine 泵 1ms（`signals.rs:525-533` idle.min(1ms)）；MeterBridge 1ms 逐窗口再拷贝（`managed/meters.rs:89-131`）；app 泵 1ms（`service/signals.rs:96-114`）；`meter/polling.rs:14-28` timeBeginPeriod(1) 按 demand 取放；LEVEL_STREAM_DEPTH=16（`service/signals.rs:19-21`）注释论证亲证。KNOWN 依据 X-2/X-3（10-27/05:47-54）+ 2026-11-09/06-signal-plugins.md（双总线已判为已解决分层形态）核对一致。 |
| B-7 | P3 NEW | 成立（建议并入 B-3） | `snapshot.rs:55-79` state 值字段每轮覆盖亲证；`host.rs:289-303` read()/snapshot() 每次 lock+clone 整快照亲证；消费方 `driver.rs:170`（每 poll read）+ `managed/meters.rs:89`（每 poll 再 read）亲证。与 B-3 同根，修 B-3 的 diff-before-write 后自然消解大半——维持初审判级并采纳其"并入 B-3"建议。 |

## 误报/降级说明（逐条展开需要修正的）

- **A-3（标注修正，维持 P3）**：初审标 NEW。复核发现 2026-11-03/03-inspector-keys.md:149 已在 IN-8 的机理段记载"面板重建只随发布发生：运行中 ~1Hz（elapsed 量化）"——即 inspector 随 1Hz 时钟重建的现象当时已被写下，只是作为"读数冻结"的机理注脚、未作为开销立项。建议标注改为 `NEW（开销视角）/ 现象 11-03 已记`。修复方向（内容投影、抄 StripKey 先例）不受影响。
- **B-1（KNOWN 依据修正，维持 P2）**：初审的 KNOWN-OPEN 判据自认主要来自任务简报转述的 2026-10-29 审计（不在仓库）；仓库内 X-3 只量化了 meter 链路，不含快照构建+比较。建议标注改为 `KNOWN（跨仓转述 10-29）/ 仓库内无对应条目，实质近 NEW`。技术内容（每轮深拷贝+全量比较、1ms 节奏、dedup 丢弃）全部亲证成立。
- **B-4（前提微修）**：`WindowsAdapter` 存在的前提是 `WindowsVadProvider::discover()` 成功（managed.rs:225），正常 Windows 环境成立但非无条件；空批循环本身（20ms 任务往返 + 三锁名称观察）完全属实。不改变判级与修复方向。
- **B-2（级别备注）**：维持 P2，但若 B-1 的令牌制落地，`pending_revision()` 等访问器仍需单独做（B-2 独立有效），只是"每 poll ≥4 次深拷贝"的主项会随 B-1 大幅缩小——排期时建议 B-2 跟随 B-1 同批。
- **B-7（并入建议）**：与 B-3 同根同修，维持 P3 但排期上不单独立项。
- 行号漂移登记（均不影响判定）：A-1 graph.rs:388-390→393；A-4 settings.rs 键克隆实际 :62；B-2 driver.rs:182→184；B-5 bridge.rs RingSource::read 实际 234-268。

## 复核补充发现（V-1..）

无。复核中确认的若干小额子成本均已被 A 系列的重建路径框架覆盖，不单独立项：
- `pages/graph.rs:417-422`（每次重建 `selected_ids: Vec<String>` 逐节点 format!）与 `:423-432`（selected_wire O(E) find）→ 并入 A-1 的重建成本。
- `meter_spans` 堆分配同样作用于 Graph 画布热层（`paint_meter_fill` 单源数学，`graph_canvas_view/nodes.rs:126-186` 每帧每通道调用）→ 并入 A-2 的影响面（16 strip 之外再加图页每节点 2 通道）。
- Graph 页故障 tooltip（`page.rs:243-260`）走 PreparedFrame 预计算数据，无每帧 Vec 构建——与 mixer 侧 A-5 的 faults_for_node 不同，无需登记。

## 修订后优先级清单

### P1
| ID | 标题 | 位置 | 状态 |
|---|---|---|---|
| A-1 | 图画布手势热路径：每显示帧整页重建 + 全卡文本重 shaping + 世界全量重投影 | `pages/graph/page.rs:22-49`、`pages/graph.rs:393`、`interactions.rs:227-281`、`graph_canvas.rs:294-607`、`graph_canvas_view/nodes.rs:15-105` | 成立 [NEW]（重建节奏机理 11-03 NC-3 已记，成本聚合为本轮新立项） |

### P2
| ID | 标题 | 位置 | 状态 |
|---|---|---|---|
| B-1 | 服务循环每轮全量构建 + 全量比较 AppSnapshot（1ms/10ms 节奏） | `service/mod.rs:453-512,489,356-390`、`service/graph.rs:657-659`、`model.rs:187-224` | 成立 [KNOWN 依据弱，实质近 NEW] |
| B-2 | driver poll 只读访问点每 poll ≥4 次 GraphSnapshot 深拷贝 + update_vad_catalog 双调用 | `managed/driver.rs:156,184,209,379,548,610,618` | 成立 [NEW]（建议与 B-1 同批） |
| A-2 | Mixer 热层每帧每通道标签 shape_line + meter_spans 每通道堆 Vec（Graph 热层同源） | `widgets/meters.rs:86-113,223-256`、`widgets/fader.rs:113-136` | 成立 [KNOWN-OPEN, HC-8 残余] |

### P3
| ID | 标题 | 位置 | 状态 |
|---|---|---|---|
| A-3 | 整快照 PublishRef 键族使 inspector/library/toolbar/statusbar/settings/devices/tray 跟随 1Hz 时钟重建 | `shell/sidebar.rs:22,68`、`chrome.rs:202,255`、`tray_mixer.rs:186,204`、`settings.rs:40,63`、`devices.rs:31,54` | 成立 [标注修正：现象 11-03 已记] |
| A-8 | 观测读出 200ms 采样重建整个 inspector 面板 | `node_inspector.rs:544-547`、`pages/mixer.rs:99-135` | 成立 [NEW] |
| B-6 | meter 链路常驻成本：双 1ms 泵 + timeBeginPeriod(1) 系统级持有 | `meter.rs:18-20`、`host/signals.rs:525-533`、`managed/meters.rs:89-131`、`service/signals.rs:96-114`、`meter/polling.rs:14-28` | 成立 [KNOWN-OPEN, X-2/X-3] |
| B-3 | engine supervisor 每轮重建 HostSnapshot + 每轮 2-3 次全目录 demand 扫描 | `supervisor/snapshot.rs:5-80`、`supervisor.rs:238,246-247`、`signal/registry.rs:176-179`、`service/signals.rs:96-120` | 成立 [NEW]（B-7 并入） |
| B-4 | Windows 导出观察空批循环：每 20ms 原生查询 + 任务往返 | `exports/runtime.rs:271-323,349,375,457,468`、`driver.rs:425-430` | 成立 [NEW，前提微修] |
| A-6 | 频谱体每帧重建网格与轴标签（~31 hairline + 4 次 shaping/卡） | `graph_canvas_view/nodes.rs:519-652` | 成立 [KNOWN-OPEN, SP-6] |
| A-5 | Graph 热层每帧分配与小额 O(n²)（rects/carve/covered + mixer 故障横幅克隆） | `pages/graph/page.rs:312,328-341,368-372`、`pages/mixer/page.rs:965-981` | 成立 [NEW] |
| A-7 | Matrix 页每 publish 全量 O(端口²) 重建（有意不缓存） | `pages/matrix.rs:1-9,153-155` | 成立 [NEW] |
| B-5 | RT 边界逐帧环操作与循环内重复求值（chunk API 可用） | `bridge.rs:71-90,234-268,352-391`、`owner.rs:493-497`、`asio/callback.rs:249-291` | 成立 [NEW] |
| B-7 | supervisor state 无 Arc 稳定性，读者每次全量 clone HostSnapshot | `snapshot.rs:55-79`、`host.rs:289-303`、`driver.rs:170`、`meters.rs:89` | 成立 [NEW，并入 B-3] |

### 排期备注（Top3 输入）
1. **A-1**（P1）：UI 侧最大单点杠杆；修法按初审三步（世界 Arc 化复用 → 手势偏移增量帧期化 → 文本 shaping 缓存照抄 ReadoutShapes）。
2. **B-1 + B-2 同批**（P2）：令牌制发布 + 轻量 revision 访问器，一次改动消除服务线程主浪费项。
3. **A-2**（P2，工作量 S）：HC-8 最后残余，per-(text,size) 小缓存 + meter_spans 栈上数组，顺带覆盖 Graph 热层同源调用。
