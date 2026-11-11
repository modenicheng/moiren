# 03 功能缺口 — 独立复核

- 输入: 03-feature-gaps.a.md + 03-feature-gaps.b.md
- 基线: main @ e2ed3d6a（2026-11-11）。复核者独立重开每条被引位置亲证（rg/读码/git log/gh），未运行 cargo，未修改两份初审。
- 判定分布：成立 36 · 部分成立(修正) 4（G4、G20、B-10、A 表计数类小误记入 G20 行外说明）· 误报 0 · 已存在 0 · 已知重复（双源合并）见交叉主题一节。

## 逐条判定（A 表 G1-G20/A1-A9 + B 表 B-1..B-13）

| ID | 初审判定 | 复核判定 | 依据（一句话） |
|---|---|---|---|
| G1 | 应用接管只做一半 [P1] | 成立 | `preparation.rs:184` 仅有 `CaptureIntent::PassiveTap`；`exclusive_probe.rs:2` 自述非选择性接管；`tools/audio-hack-lab/REPORT.md:31` 方案 A（loopback 捕获+端点主静音+渲到另一端点）已实测裁决；产品 crates 无 Redirect/Takeover 痕迹 |
| G2 | 录音系应用层零接线 [P1] | 成立（并入 M2） | moiren-app 全库 `recorder_reader`/`RecordingSession`/`record_wav` 0 命中；engine 侧 `host.rs:331`、`recorder.rs`（64 MiB 上限/accepted/read/dropped 计数）、`wav.rs`（fail-fast+finalize 补写+RIFF 4 GiB 封顶）完备；inspector 仅有 Add/Remove 1 second（node_inspector.rs:856-889） |
| G3 | Audio File Player 完全未动 [P1→排期] | 成立 | `moiren-core/src/graph/model.rs` NodeKind 恰 16 变体无 FilePlayer；PRD §5.7/§12.6 规格在、§17-16 明确"不为此设计插件通路" |
| G4 | RackPreset 无代码痕迹 [P2] | **部分成立(修正)** | `moiren-engine/src/rack.rs:98` 已有 `RackPreset{version,name,rack}` + serde + `valid_for` 校验（仅 engine 测试消费，app/core 0 命中）；缺口收敛为"应用层保存/加载/管理 UI + 工程持久化"，数据模型无需新建 |
| G5 | Rack 槽位无重排 [P2] | 成立 | inspector rack 区块仅 `next_slots.push`（node_inspector.rs:939），全文件无 move/reorder；PRD §12.3:772 要求"拖动改变顺序" |
| G6 | Scene/Profile 未收敛 [P3] | 成立 | PRD §13.3 自述"具体语义尚未收敛"；core/app 无 Scene 概念代码 |
| G7 | 插件应用层四件套缺失 [P1] | 成立（并入 M3） | `node_catalog.rs:49` "Plugin has none" 模板；`preparation.rs:256-259` Plugin 直接报错；app 内 `scan_directories`/`instantiate` 0 命中；序列化变体确实已备（project/graph.rs:73,94,117）；design 19 §5 明示应用层为后续项 |
| G8 | 插件无崩溃隔离/批测 [P2] | 成立（并入 M3） | `scan.rs:137-152` 进程内 collect_file；`examples/probe.rs:12-13` 作者自述无隔离；tests 仅自绘 gain fixture；design 19 §5 列崩溃隔离为排除项 |
| G9 | 插件宿主扩展缺 [P3] | 成立 | design 19 §5 整体排除；`vst3/mod.rs:211` restart=handler_noop_i32、:242 注释 "documented follow-up" |
| G10 | VAD 安装/生命周期未产品化 [P1] | 成立（范围扩至含默认设备引导，见 V-1） | `vad.rs` 模块头自述无 engine backend/installation/default-device policy；`git ls-files driver/` 仅 README 与 vad/.gitignore 被跟踪（驱动主体未跟踪）；devices 页 Exports 页签只管导出/绑定 |
| G11 | VAD ASIO 接口纯设计 [P2] | 成立 | PRD §11.3 五项关键问题全部标"待定"，无代码 |
| G12 | 端点呈现无分组拆分 [P2] | 成立 | `service/model.rs:123` vad_endpoints 映射完备；`devices.rs` 页签为 Inputs/Outputs/Virtual devices/Clock 单一虚拟页签，无虚拟/物理/应用分组 |
| G13 | per-app 路由未产品化 [P2] | 成立 | `AudioPolicyConfig` 仅存在于 `tools/audio-hack-lab/lab/src/main.rs:527-536`；产品 crates/docs 0 命中 |
| G14 | #105 四阶段未动 [P1] | 成立（并入 M1） | 六缺陷逐条亲证同 B-1（组串行 engine_backend.rs:440 `for group in groups`、recovery 仅 ASIO :426-439/:469-470）；#105 OPEN 且含 Stage 1-4 与 D1-D3 决策原文 |
| G15 | 故障文案 i18n 缺 [P2] | 成立 | #86 OPEN 亲读（dispatch_error Debug 串 + RegisterError）；`service/managed/lifecycle.rs:69-193` record_error 点存在（另有 driver.rs:44-66 多处，A 表入口不全但不影响结论）；`src/i18n_guards.rs` 台账+Debug 哨兵存在 |
| G16 | 界面语言仅 zh-CN [P2] | 成立 | `crates/moiren-app/locales/` 仅 zh-CN.yml；`settings/model.rs:517` set_language 在 |
| G17 | a11y 两笔遗留 [P3] | 成立 | #91 item1 在列；产品代码 `Role::Slider` 0 命中（问题在 fork/上游层，先实机定性正确） |
| G18 | 设置页缺项 [P3] | 成立 | `Section::{Audio,Interface,Diagnostics}`（settings/model.rs:50-57）；fields.rs 无主题项 |
| G19 | 托盘能力面窄 [P3] | 成立 | `ui/tray.rs:45-47` 仅 Show/Stop Session/Exit；`ui/shell/tray_mixer.rs` flyout 存在 |
| G20 | TODO 蒸馏微缺口 [P3] | **部分成立(修正)** | TODO.md 实为 5 条蒸馏待办（第 3 条"会话内 WAV 录制 UI"=G2，A 表并入正确但"四条"表述易误读）；第 4 条"观测真实性围栏"通用部分已于 2026-11-09（1ab83c85）在 `ui/monitoring/table.rs`+`monitoring.rs` 大半落地（generation 重置/Running 门/候选排除/宽限/Stopped·NotAvailable），A 表"未做"过时；余 ①②⑤ 维持微缺口 |
| A1 | 设备页重设计待实机验收 | 成立 | PR #167/#168 合并亲证（24173960/e2ed3d6a）；design 23 在（e8d55368） |
| A2 | 推子手感待实机验收 | 成立 | PR #165 合并（de926079）；死行防护提交 68840e15 存在 |
| A3 | 图自动保存待实机验收 | 成立 | PR #94 合并（9e3a7bcc） |
| A4 | 终端 Ctrl+C 待实机验收 | 成立 | PR #103 合并（119d45b7）；`crates/moiren-app/src/console.rs` 在 |
| A5 | fork 补丁待实机验收+文档欠更新 | 成立 | gh api 亲证 fork 7 提交中 5 个补丁：3 个 vsync + WM_MOVE 唤醒 + locale 回退；`docs/platform/gpui-fork.md` 仅记 3 个 vsync（第 4 条为免补丁节流说明）——欠 2 条 |
| A6 | #91 六项实机疑点 | 成立 | #91 OPEN 亲读，六项与 A 表一一对应 |
| A7 | #79 overlay 遮挡后 meters 冻结 | 成立 | #79 OPEN 亲读：visible 只写不读回、无 show 事件自愈 |
| A8 | minimal-io-latency 未资格化 | 成立 | `feat/minimal-io-latency` 存在（head 1b33b687@2026-11-04）且 `git merge-base --is-ancestor` 判定不在 main；CONCURN 清单在该分支 `docs/vad-low-latency-goals-2026-10-28.md`；另有 integrate/minimal-latency-main-20261108 分支（A 表未提，补充） |
| A9 | M0.5/M1 整体验收遗留 | 成立 | PRD §16 自述 "M0.5 整体验收仍未完成"（记录性凭据） |
| B-1 | #105 六缺陷全未修 [P1] | 成立（并入 M1，双源=G14） | 逐条亲证：无界握手+cancel 后 join（owner.rs:233-253 循环、spawn 内 `state.cancel(); worker.join()`）、组串行（engine_backend.rs:440）、BackendError 无归属字段（engine/src/host/backend.rs:86-98 仅 String 变体）、recovery 仅 ASIO（:426-439,:469-470）、spawn 只回传终态、catalog Activate/GetMixFormat/session 枚举无界（catalog.rs:126-184）且 service/windows.rs 同步调用；遗弃 worker 无登记（Worker 无 abandoned 态 :170-181，cleanup 无条件 join :705-708）；wasapi.rs open_inner 各 COM 调用均无 deadline |
| B-2 | 插件栈三层断链 [P1] | 成立（并入 M3，双源=G7/G8） | 三层亲证：进程内扫描无隔离（scan.rs+probe.rs 自述）；app→引擎断链（app 0 调用、preparation 报错、catalog 无模板，而 engine/host/project.rs:37 Plugin 变体与 processor/plugin.rs RT 适配完备）；C++ ABI 崩溃=进程级、catch_unwind 不覆盖（owner.rs:220-227 仅 Rust panic） |
| B-3 | 44.1-96k+立体声硬上限 [P2] | 成立 | `audio_config.rs:12` SAMPLE_RATES 四值白名单；`validation.rs:88-90` channels!=2 → "professional graph IO is stereo"、:103-108 引擎速率同白名单；`asio/buffer.rs:86-95` 同白名单且测试断言 192000 被拒（:146）；`codec.rs:84-121` 多通道仅编解码首两通道、其余 native.fill(0) 静音 |
| B-4 | 录制引擎完备但不可达；capture-only 被拒 [P2] | 成立（并入 M2，双源=G2） | moiren-app 0 调用亲证同 G2；`validation.rs:114-118` `outputs == 0` 即拒（"professional backend needs an output"），VirtualRealtime 仅"有输出但全虚拟"可达；RIFF 4 GiB 封顶亲证（wav.rs MAX_DATA_BYTES） |
| B-5 | WASAPI 断线无自动恢复/FollowDefault 不跟随 [P2] | 成立（并入 M1 Stage 2） | notification client 仅 OnDeviceStateChanged(非 ACTIVE)/OnRemoved latch 失效（wasapi.rs:192-260），OnDeviceAdded/OnDefaultDeviceChanged 为空 Ok(())；FollowDefault 在 prepare 时一次性解析（engine_backend.rs:372-374 注释亲证）；恢复机器（recovery.rs 单预算/supervisor/requests.rs）现成 |
| B-6 | ASIO：面板缺失/单驱动/粒度0/duplex 强约束 [P2] | 成立 | `asio/abi.rs:117` control_panel vtable 在、全仓调用 0（仅 capability_tests.rs:238 forbidden 与 tests.rs:282 stub）；>1 ASIO 组 `unsupported`（engine_backend.rs:416-425）；granularity 0 仅收 preferred（asio/buffer.rs:44-56 含 FL Studio 注释）；duplex 同率同缓冲否则整体失败（engine_backend.rs:145-174） |
| B-7 | 插件缺 MIDI/编辑器/传输/采样精确 [P2] | 成立 | `instance.rs:25-30` 事件仅 PluginParamEvent 且"apply at the first frame of the block"；moiren-plugins 全库无 midi/note 命中；vst3 has_editor:false 硬编码×2（:1179,:1456）；clap 仅探测 gui flag（:258）；等宽排除为文档化决策（spec.rs NoAudioPorts） |
| B-8 | Shared 精确 period 静默降级无复核 [P3] | 成立 | `wasapi.rs:663-674` shared_conversion → legacy Initialize（含 AUTOCONVERTPCM|SRC_DEFAULT_QUALITY，negotiation.rs:249-253）仅 warn 日志；period 精确性复核仅 ASIO（engine_backend.rs:508-517 `group.backend == AudioBackend::Asio` 门） |
| B-9 | 取消钩子 16 上限可耗尽 [P3] | 成立 | `engine/src/host/backend.rs:62-68` `hooks.len() < 16` 否则 Failed("cancellation hook capacity exhausted")；注册点三处亲证（engine_backend.rs:676、owner.rs:602/767、virtual_owner/owner.rs:179）；validation 允许 ≤64 IO 节点，>16 捕获/虚拟节点图理论上爆 |
| B-10 | ClockBridge 全套无生产接线 [P3] | **部分成立(修正)** | 生产 RingSource（owner.rs:523 构造）已集成 RateClock capture/QPC 前馈 + FollowerServo 反馈并 `set_correction_ppm` 驱动 SRC（bridge.rs:93,137-141,231-232）；CaptureIngress/CapturePacket/ClockSource/BridgeObserver 已用于 capture/wasapi*（preparation.rs:5、packet.rs:6、wasapi.rs:4）——"capture 端无速率自适应""全仓库仅测试消费"不成立；真正悬空的是 capture 为主时钟的 engine 源集成与 BridgeSnapshot/QueuedFrameAge 遥测对外暴露（0 生产消费者）；与 B-4 的互为因果保留但强度降级 |
| B-11 | 探测工具产品化参差/docs/platform 单薄 [P3] | 成立 | tools/rt-check 独立 harness、audio-hack-lab 一次性脚本、probe/exclusive_probe 仅 examples（lib.rs:45-52）、`docs/platform/` 仅 gpui-fork.md——均亲证 |
| B-12 | WASAPI driver_latency 恒 None [P3] | 成立 | `native.rs:59-61` 默认 None；`asio/stream.rs:433-437` 唯一 override；wasapi.rs 0 命中 |
| B-13 | （正面确认）panic/OOM 面受控 [—] | 成立 | spawn/cleanup catch_unwind（owner.rs:220-227 等）、supervisor.rs catch_unwind、recovery.rs:68-69 "deliberately no healthy-run timer" 均亲证；唯一泄漏面=遗弃线程堆叠（并入 B-1/D3） |

## 交叉主题合并说明

双源重叠 4 组，合并为修订清单的三个"合并主项"（M1-M3）：

1. **M1 卡死端点容错（#105）** = G14（A）+ B-1（B）+ B-5（B，其 Stage 2 子项）。两份独立得出行号级一致的结论，可信度最高；D1-D3 拍板随行。B-5 的 WASAPI 恢复监视并入 Stage 2 一次实施（事件源+归属字段+FollowDefault 重解析）。
2. **M2 录音接线** = G2（A）+ B-4（B）+ TODO.md 蒸馏第 3 条（A 表 G20 内）。A 表给 UI/生命周期方案，B 表补两个 A 表未提的独立缺口：capture-only 会话被 validation 拒绝（`validation.rs:114-118`）与 RIFF 4 GiB/RF64 欠账；capture-only 需先拍板（联动 B-10）。
3. **M3 插件生态** = G7+G8+G9（A）+ B-2+B-7（B）。顺序共识：进程外扫描沙箱（B-2 第 1 层）→ 应用层接通四件套（G7/B-2 第 2 层）→ MIDI 合同与宿主扩展（G9/B-7，编辑器按 B 表"明确不建议"最后）。
4. **gpui fork 文档欠更新**（A5 与 B-11 各提一面）：A5 说补丁记录缺 2 条（亲证成立），B-11 说 docs/platform 仅 1 篇（亲证成立）——同一"平台文档化"主题，修订清单并为一项 P3。

## 误报/降级说明

- **G4 降级（部分成立）**："无代码痕迹"只对 app/core 成立；`moiren-engine/src/rack.rs:98` 已有带 serde 与校验的 `RackPreset`。后续开发指导应改为"接线 engine 既有 RackPreset，而非新建数据模型"。
- **G20 修正**：TODO 蒸馏待办实为 5 条（第 3 条=录音 UI，即 G2）；第 4 条观测围栏的通用部分已于 2026-11-09 落地（1ab83c85 `ui/monitoring/table.rs`：generation 重置、Running 门、候选排除、宽限清理；`monitoring.rs:209-280` 停止/过期清空与 Stopped/NotAvailable 状态），A 表"频谱/电平通用围栏未做"过时，残余仅细节收尾。
- **B-10 降级（部分成立）**：clock_bridge 的速率自适应主链（RateClock 前馈+FollowerServo 反馈→SRC ppm 校正）已在生产 `bridge.rs` RingSource 接线；B 表"capture 端无速率自适应/仅测试消费"与代码不符。缺口收窄为 capture-主时钟 engine 源 + BridgeSnapshot/QueuedFrameAge 遥测暴露。
- **计数小误（不影响结论）**：open issues 实为 14 个（A 表"15 个"；枚举 ID 本身正确）；clock_* 测试文件 7 个（B 表"10 个"）。
- **引用行号微偏**（ substance 无误）：G14 的 recovery 字段实为 engine_backend.rs:469-470（A 表写 462）；G15 入口除 lifecycle.rs 外 driver.rs:44-66 亦有大量 record_error；B-9 的 `virtual_owner/owner.rs:179` 位于 `src/virtual_owner/`（非 engine_backend/ 下），路径易误读但存在。
- 无误报、无"已存在"级推翻：两份初审的"不存在/未接线"判定经全仓 rg（含宏调用面）复核全部可靠。

## 复核补充发现（V-1..）

- **V-1 [并入 G10] VAD"成为默认设备"闭环只有资格位、无引导**：产品侧 `allow_default` 资格模型完备（`service/exports/reconcile.rs:33-37` Create/SetDefaultEligibility；`ui/pages/devices/exports.rs:207,565-575` 有开关与帮助文案，明示"On only permits eligibility; it does not set a Windows default"），但应用内无"引导用户到声音面板/一键设默认"的交付项；`vad.rs` 模块头自述不含 default-device policy。G10 修订后应把"默认设备策略/引导"与安装并列（非独立大项）。
- **V-2 [P3 微缺口] 托盘菜单不随运行中语言切换**：`ui/tray.rs:42-44` 注释自述菜单标签仅在启动时解析，中途切语言托盘保持启动语言直至重启。与 G16/G19 相邻但独立（切语言后需重启才全量生效的产品决策未文档化）。
- 除上述两项，未发现两份初审遗漏的值得单列的新缺口；界外线索（#87 stand-down、#81/#82/#83/#88/#89/#90/#150）属既往 issue 与 01/02 部分范围，不重复立项。

## 修订后优先级清单（缺口 P1/P2/P3 / 验收欠账 / 待拍板）

### 缺口（合并后 25 项）

**P1（4 项）**
1. **M1 #105 卡死端点容错四阶段**（双源 G14+B-1，含 B-5 并入 Stage 2；前置拍板 D1-D3）——入口 `engine_backend/owner.rs` spawn/握手、`engine_backend.rs` 组开启、`wasapi.rs` 通知客户端、`engine/src/host/backend.rs` BackendError。
2. **M2 录音接线**（双源 G2+B-4；含 capture-only 拍板联动）——入口 `service/managed`（订阅 recorder 端点+录制生命周期）、`ui/pages/graph/node_inspector.rs` recorder 区块、`validation.rs:114`。
3. **M3 插件生态接入**（双源 G7+G8+B-2；先进程外扫描沙箱，再扫描服务/浏览器/参数面板/持久化四件套）——入口 `moiren-plugins/src/scan.rs`、`service/graph/preparation.rs`、`ui/pages/graph/node_catalog.rs`。
4. **G1 应用接管产品化**（lab 方案 A 已验证；依赖 M1 的取消/恢复基建）——入口 `moiren-windows-audio`（进程 loopback+IAudioEndpointVolume 状态机）+ `InputSource::Application` 增 Redirect intent。

**P2（10 项）**
5. B-3 采样率/多通道硬上限（先拍板目标矩阵）。
6. G10 VAD 安装/生命周期产品化（扩含 V-1 默认设备引导）。
7. G11 VAD ASIO 接口（依赖 G10 稳定）。
8. G12 端点呈现拆分（待拍板 P-1）。
9. G13 per-app 路由（待拍板 P-2 后定生死）。
10. G15 故障文案 i18n（#86；入口 lifecycle.rs+driver.rs+台账）。
11. G16 第二界面语言（en 或社区流程）。
12. G4 RackPreset 应用层接线（数据模型已在 engine）。
13. G5 Rack 槽位重排 UI。
14. G9+B-7 插件宿主扩展：MIDI 事件通道、传输信息、bypass/编辑器（编辑器按 B 表最后）。

**P3（11 项）**
15. G6 Scene/Profile（先拍板语义）。
16. G17 a11y 两笔遗留（先实机定性）。
17. G18 设置页补主题/Processing SR 说明。
18. G19 托盘静音/音量快捷操作（场景类等 G6）。
19. G20 余项：卡上主控延长线、inspector 数值编辑语义统一、压缩器静态曲线（③观测围栏仅收尾）。
20. B-8 Shared 精确 period 事后复核/显式标记。
21. B-9 取消钩子 16 上限分槽化。
22. B-10（修正后）capture-主时钟 engine 源 + 遥测暴露（联动 M2 capture-only 拍板）。
23. B-11 探测工具产品化 + A5 平台文档欠更新（fork 补 2 条 + docs/platform 总览）。
24. B-12 WASAPI driver_latency（GetStreamLatency/padding 推导）。
25. V-2 托盘语言随切换（微项，可并入 G16/G19 实施）。

### 验收欠账（实机，A1-A9 全部维持）

A1 设备页六场景（PR #167/#168）· A2 推子手感（#165）· A3 自动保存三场景（#94）· A4 Ctrl+C 退出（#103）· A5 fork 补丁帧率（文档欠更新已亲证）· A6 #91 六疑点 · A7 #79 遮挡恢复 · A8 minimal-io-latency 资格化（补充：另有 integrate/minimal-latency-main-20261108 整合分支待处置）· A9 M0.5/M1 整体验收。优先序维持 A1-A4。

### 待拍板（维持 8 项）

D1 开启限时 10s/5s · D2 WASAPI 自动剥离范围 · D3 遗弃 worker 上限防泄漏 · P-1 端点呈现拆分方式 · P-2 per-app 路由产品化与否 · P-3 IME 全角减号取舍 · P-4 文件播放四待定项 · P-5 hytao 接入路径。（D1-D3 原文在 #105 内，P-3 原文在 #91 item5，P-4 在 PRD §5.7，P-5 在 PRD §17-16，均亲证在案。）

### 收束建议（复核后不变，微调）

1. M2 录音接线 + M1 #105（先拍 D1-D3）——两者均有完整实现基础，P1 中最接近可交付。
2. G1 接管产品化（依赖 M1）。
3. M3 插件生态（先沙箱后四件套）。
4. A1-A4 实机欠账优先清账（均为已合并的行为级变更）。
