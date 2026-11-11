# 03 产品/UX 功能缺口 · 初审 A

> 基线：`main @ e2ed3d6a`（2026-11-11）。只读审计，未运行 cargo，未修改任何产品文件。
> 范围：产品/UX 功能缺口（平台/引擎能力缺口由另一代理负责，本报告仅在交叉处引用）。

## 方法与信息源

1. `docs/Moiren-PRD-Roadmap.md` 全文逐项对照实现（§5 节点规划、§12 GUI、§15 路线图 M0–M5、§17 未决问题）。
2. `docs/TODO.md` 全部条目（4 条蒸馏待办 + 已完成项核对）。
3. `docs/designs/` 23 份设计的状态标注与"待实现"段落；重点 14/16/19/20/23，另查 04/11/12/17 的未落地段。
4. GitHub open issues（gh 经代理成功，共 15 个 open：79–84、86–91、105、150）。
5. 代码痕迹扫描：`todo!/unimplemented!`（产品代码 0 处）、占位文案、UI 禁用入口、设置页缺项。
6. 项目记忆线索逐条核实（录音接线、per-app 路由、VAD 呈现、插件批测、a11y、fork 补丁等）。

关键核实点均有代码证据（path:line）；"是否存在功能"均以亲查代码为准，不以文档自述为凭。

## 已确认"不存在缺口"的项（先排除，避免重复开发）

- **Routing Matrix 页已实现**：`ui/pages/matrix.rs`（有完整页面与测试，非占位）。
- **Mixer Sends 已实现**：`ui/pages/mixer/routing.rs`（send 行 + flush bar + 折叠）。
- **Undo/Redo 已实现**：`service/graph.rs:72-74`（journal）+ `ui/actions.rs:4`（`Undo/Redo` action）；design 12:120 的"仍需后续接入"已过时。
- **设备页职能重设计 A+B 已合并**（PR #167/#168，git log 可证），缺口只剩实机验收（见二）。
- **自动保存（PR #94）与终端 Ctrl+C（PR #103）已实现**：`project/storage.rs`、`project/startup.rs`、`console.rs`。
- **逻辑图 UI（右键菜单/整理/吸附/inspector）已按 TODO 2026-11-08 计划完成**。
- 产品代码无 `todo!()/unimplemented!`，无"敬请期待"类占位文案——缺口都是"没做"，不是"做了一半挂着"。

---

## 一、缺口清单（按产品支柱分组）

### 支柱 1：路由与图

**G1 [P1] Application Redirect / Takeover（应用声音接管）— 只做了一半**
- 现状：产品内应用输入只有被动捕获 `CaptureIntent::PassiveTap`（`crates/moiren-app/src/service/graph/preparation.rs:184`）；接管实验独立于产品（`crates/moiren-windows-audio/src/exclusive_probe.rs:2` 自述 "A positive result means process audio was observed, not selective takeover"）。PRD §5.1/M1 Gate 明确：方案未验证前不能宣称"重定向"。
- 已有基础：`tools/audio-hack-lab/REPORT.md` 已实测钉死管线抓取点位置并裁决"方案 A（端点主静音 + 渲染到另一端点）"可行、免管理员、可逆。
- 指导：把 lab 方案 A 产品化——入口 `moiren-windows-audio`（进程 loopback 模块 + `IAudioEndpointVolume` 静音/恢复状态机），应用层在 `InputSource::Application` 增加 `Redirect` intent 与"原端点已静音"的可解释状态；前置依赖：G15（#105 的取消/容错基建，接管失败必须能恢复原静音状态）；验收：防双声、静音状态可逆、OBS 并行捕获共存、崩溃后不遗留静音。

### 支柱 2：录音与制作

**G2 [P1] 录音系接线（Recorder→WAV 全链路）— 引擎就绪，应用层零接线**
- 现状：引擎侧完备——`crates/moiren-engine/src/recorder.rs`（有界环形缓冲）、`recorder/wav.rs`（控制侧 WAV 落盘 + worker）、`host.rs:333` 提供 `recorder_reader()`；但 moiren-app 全库无一调用 `recorder_reader`/`RecordingSession`（grep 亲证 0 处）。UI 只有容量增减："Add 1 second"/"Remove 1 second"（`ui/pages/graph/node_inspector.rs:856-885`）。
- 指导：按 TODO.md 第 3 条补齐 UI 需求——录制启停（走带或节点 chip）、输出路径选择、编码选择、剩余容量提示；service 持有录制生命周期（与图启停独立，`stop 时 drain + finish` 语义已在 `wav.rs:19-21` 注释写明）。入口：`service/managed`（订阅 recorder 端点）+ `node_inspector.rs`。验收：录音期间改图不断流、停止后 WAV 头补写正确、64 MiB 上限行为可见。

**G3 [P1] Audio File Player 节点（BGM/音效）— 完全未动**
- 现状：`NodeKind` 无此变体（`moiren-core/src/graph/model.rs:45-74` 共 16 种，无 FilePlayer）；PRD §5.7/§12.6 已定规格（Playlist/Pad 双模式、RT 禁文件 I/O、待定 4 项）。
- 指导：属于 M2 排期项，工程量大，建议排在 G2/G8 之后；先行拍板 PRD §5.7 的 4 个待定项（解码格式/Pad 内存上限/工作线程池/快捷键范围，见三）。

**G4 [P2] RackPreset 保存与加载 — 无代码痕迹**
- 现状：PRD §12.3/§13.2 规划；`RackPreset`/`rack_preset` 在 app/core 均 0 匹配（grep 亲证）。
- 指导：数据模型照 PRD §13.2（只存 processor 类型/顺序/bypass/参数）；入口 `crates/moiren-core/src/graph/model.rs`（RackConfig）+ `project/`（序列化）。前置：无硬依赖，可与 G5 同期。

**G5 [P2] Rack 槽位重排序 UI — 有增删无排序**
- 现状：inspector 支持加槽/逐槽 bypass（`node_inspector.rs:932-946`：`next_slots.push(RackSlot{...})`），无任何 move/reorder 入口（grep 亲证）；PRD §12.3 要求"拖动改变顺序"。
- 指导：纯 UI+图编辑（重排 = 一次原子图编辑，走现有 undo journal）；入口 `ui/pages/graph/node_inspector.rs` rack 区块。验收：重排为单个 undo 步、实时参数按身份迁移不串位。

**G6 [P3] Scene / Profile 语义 — 未收敛未实现**
- 现状：PRD §13.3 自述"具体语义尚未收敛"；无代码。
- 指导：先拍板 Scene（实时参数/路由快照）与 Project 的边界再动手；不建议提前实现。

### 支柱 3：插件生态

**G7 [P1] 插件应用层四件套：扫描服务 / 浏览器 UI / inspector 参数面板 / 项目持久化**
- 现状：design 19 §5 明示"应用层为后续项"；亲证：`node_catalog.rs:49` "…and Plugin has none"（编辑器模板无 Plugin，用户无法创建插件节点）；`preparation.rs` 对 Plugin 有装配但对用户不可达。
- 指导：入口 `moiren-plugins/src/scan.rs`（目录扫描已实现）→ app 扫描服务（后台线程 + 缓存表）→ 节点目录/添加菜单加 Plugin → inspector 逐参数滑条（ParamSpec→Float 域已通）。前置：G9（扫描沙箱）至少要有进程外扫描的最小版，避免首次扫描拖垮 UI 进程。验收：添加 VST3/CLAP 插件节点、参数可调、保存/重载工程后插件状态恢复（序列化变体已有，`project/graph.rs` 含 `plugin`）。

**G8 [P2] 插件崩溃隔离 / 进程外扫描沙箱 / 真实插件批测**
- 现状：design 19 §5 列为 MVP 排除项（"第三方插件可直接拖垮进程"）；测试只有自绘 gain fixture（`moiren-plugins/tests/plugin_ffi.rs`），无真实插件批测；项目记忆遗留：等宽总线排除（30 个描述符）未建批测护栏 + 扫描必须进程外沙箱。
- 指导：分两步——(1) 扫描进程隔离（子进程扫描 + 结果 IPC，崩溃只丢目录）；(2) 批测脚本固化 30 描述符清单（含不等宽被拒样例），验收 = 全部报 `NoAudioPorts`（`moiren-plugins/src/spec.rs:93`）而非崩溃。

**G9 [P3] 插件宿主扩展（GUI 编辑器、bypass、sample-accurate、restart 请求等）**
- 现状：design 19 §5 整体排除；VST3 `restartComponent/request_restart` 目前"确认即忽略"。
- 指导：随 G7 落地后按用户反馈排期；CLAP GUI（`IPlugView` 类比）优先级高于 VST2。

### 支柱 4：虚拟设备与系统集成

**G10 [P1] VAD 驱动安装/生命周期产品化**
- 现状：`moiren-windows-audio/src/vad.rs` 模块头自述："This module does not provide an engine backend, installation, or default-device policy"；`driver/vad/` 存在但未跟踪（`docs/vad-handoff-2026-10-21.md:41`）；设备页虚拟设备页签只管"导出/绑定"（`ui/pages/devices.rs:134-136`），不负责驱动安装。
- 指导：补安装/升级/卸载流程与用户引导（检测驱动缺失 → 引导安装 → 授权/签名说明），以及"驱动在但端点未出现"的诊断文案；入口 `ui/pages/devices/exports.rs` + `vad/discovery.rs`。这是 VAD 从"能开发验证"到"能交付用户"的分水岭。

**G11 [P2] VAD ASIO 接口（PRD §11.3）— 纯设计，无代码**
- 现状：PRD 2026-10-27 增补的规划项，关键问题（IPC 协议/时钟归属/通道映射/单客户端语义/安装）全标"待定"。
- 指导：低延迟专业路径先行候选，但依赖 G10 稳定；先拍板 IPC 是否复用 VAD 共享内存协议。

**G12 [P2] OBS 式端点呈现拆分（虚拟/物理/应用音频）— 待拍板的呈现层缺口**
- 现状：VAD 端点识别链已完备（`service/model.rs:123` `vad_endpoints: BTreeMap<VirtualEndpointId, String>` 映射 + 设备页过滤 + PnP 四条件选择）；但设备页仍是单一"虚拟设备"页签（`devices.rs:136`），没有"虚拟/物理/应用音频"的分组呈现。
- 指导：待拍板"模板拆分 vs 下拉分组"（见三）；实现侧两者都只动设备页行模型（`devices.rs` + `service/model.rs` 行投影），无引擎改动。

**G13 [P2] per-app 路由（AudioPolicyConfig 隐藏 ABI）— 实验室成果未产品化**
- 现状：破解成果只存在于 `tools/audio-hack-lab/`（`lab/src/main.rs`、`REPORT.md`），产品 crates 与 docs/ 均无痕迹（grep 亲证）；也未拍板是否值得产品化。
- 指导：先决策（见三）；若产品化，能力面是"按应用强制路由策略"，与 G1 的 loopback 路线互补但风险高（未公开 ABI，系统更新可能失效），建议只作可选实验特性。

### 支柱 5：可靠性与诊断 UX

**G14 [P1] issue #105 四阶段实施（卡死端点容错）— 设计成稿，代码未动**
- 现状亲证六缺陷仍在：串行组开启（`engine_backend.rs:440` `for group in groups`）、无界握手（`engine_backend/owner.rs:239` `recv_timeout(20ms)` 循环无上限、cancel 走 join）、恢复监视只注册 ASIO（`engine_backend.rs:462` `recovery: (group.backend == AudioBackend::Asio)`）、失败无节点归属、无进度反馈、目录枚举无界。
- 指导：按 issue 内四阶段（绑死限时并行开启 → 归因+自动恢复 → 启动进度 UX → 降级启动）逐段交付，每段独立可发；前置拍板 D1/D2/D3（见三）。

**G15 [P2] 故障文案 i18n 化（issue #86）**
- 现状：dispatch_error / RegisterError fault 直接显示英文 Debug 串，中文 UI 穿帮（issue #86 自述）；#105 Stage 2/3 的结构化文案也依赖此约定。
- 指导：按 #86 惯例改 locale key + 结构化字段；入口 `service/managed/lifecycle.rs` 的 fault 记录点 + `i18n_guards.rs` 台账。

### 支柱 6：可及性与 i18n

**G16 [P2] 界面语言只有 zh-CN 一种翻译**
- 现状：`crates/moiren-app/locales/` 仅 `zh-CN.yml`（1529 键）；语言切换 UI 已备（`settings/model.rs:517` `set_language`），但无第二语言可切；另有 #86 的未翻译 fault 文案。
- 指导：若目标包含英文以外用户，补 en.yml 或社区翻译流程；否则维持现状并在 README 说明。i18n 台账守卫（`i18n_guards.rs`）已是加分基础。

**G17 [P3] a11y 两笔遗留**
- (a) #91 item1：Narrator + meters 的激活帧验证（实机欠账，见二）。
- (b) "a11y 下拖推子跳 -inf"疑案：预案为 fork 补丁（Click 回退跳过 `Role::Slider`）未落地；产品代码内无 `Role::Slider` 自定义（grep 亲证 0 处），问题应在上游 gpui-kit 渲染层，需先在实机复现定性再决定是否 fork。

### 支柱 7：设置、个性化与托盘

**G18 [P3] 设置页缺项**
- 现状：`Section::{Audio, Interface, Diagnostics}`（`settings/model.rs:51-56`）；音频面有模式/采样率/缓冲/编码/延迟档位，Interface 面只有语言+密度；无主题选择、无"Processing SR Auto/Manual"的独立命名（现以设备级"Follow processing"表达，PRD §8.1 语义未完整上浮）。
- 指导：低成本补主题开关与 Processing SR 显式说明；不建议扩设置面过大（与 design 20 瘦身方向相反）。

**G19 [P3] 托盘能力面窄**
- 现状：托盘菜单仅 Show/Stop Session/Exit（`ui/tray.rs:45-47`）；但有 TrayMixer flyout（`ui/shell/tray_mixer.rs`，带电平表）。缺：静音/音量快捷操作、场景切换、设备快速切换。
- 指导：在 flyout 内加主输出静音/音量即可覆盖高频需求；场景类操作等 G6 语义收敛后再说。

**G20 [P3] TODO.md 蒸馏待办四条（微缺口）**
- ① 卡上主控延长线（EQ 频率/增益上卡）；② inspector 数值编辑语义统一（Enter/失焦/非法提示，仅部分字段具备）；③ 观测真实性围栏（候选/运行配置区分、过期清空——设备页已按 design 23 做了节点侧，频谱/电平观测的通用围栏未做）；④ 压缩器静态曲线调参评估。
- 指导：均为小切口，可穿插在大项间隙；②④ 入口都在 `ui/pages/graph/node_inspector.rs`，③ 入口 `ui/observe.rs` + `ui/monitoring.rs`。

---

## 二、实机验收欠账清单（已实现、缺实机验证）

| # | 功能 | 验收场景 | 风险 |
|---|---|---|---|
| A1 | 设备页职能重设计 A+B（PR #167/#168，design 23 §6） | 六场景：运行中换输出设备；绑定打不开的设备（等待生效+旧流提示）；清主 Source 绑定后 Start 不回绑；拔唯一输出设备后 Start→插回自恢复；ASIO 输出+应用音源出声；旧工程（含 `session`）开→启→存→再开 | 启动语义变更（Start=按图启动）是行为级变化，出错影响所有日常流程 |
| A2 | 推子手感（PR #165，design 22） | 双击回默认、精确数值输入、手势取消、echo 延迟；含 input dropdown 死行防护（`68840e15`） | 手感类问题 headless 测不出，回归会直接伤日常操作 |
| A3 | 逻辑图自动保存（PR #94） | 三场景：正常重启恢复；唯一麦克风拔掉后的剥离与恢复；次要设备拔掉 | 剥离逻辑误伤=丢用户图 |
| A4 | 终端 Ctrl+C 优雅退出（PR #103，`console.rs`） | 会话运行中 Ctrl+C：释放设备、落盘、干净退出 | 退出不净=残留设备占用 |
| A5 | 设置页拖动帧率 bump（gpui-pre-windows fork 补丁） | 拖动设置滑条期间帧率保持流畅、松手回静止；fork 实有 5 提交而 `docs/platform/gpui-fork.md` 只记 3 个 vsync 补丁（另有 WM_MOVE 唤醒、locale 名回退），文档欠更新 | 平台层补丁，回归窗口异常 |
| A6 | issue #91 六项实机疑点 | a11y 激活帧；inspector spring 收起与缓存遮罩；iconic 恢复 SharedFrame 兄弟审计；失焦 dt 尖峰；IME 全角减号 `－inf`；同设备双端点（#83 前置） | 一轮实机过检即可清账，但每项都是 UI 硬伤候选 |
| A7 | issue #79 overlay 遮挡恢复 meters 冻结 | 长遮挡→恢复，visible 标记自愈（根因在平台层 show 事件丢失） | 电平表冻结损害"所有声音可见"承诺 |
| A8 | minimal-io-latency Stage 7 资格化（分支 `feat/minimal-io-latency` 未并 main） | 按 CONCURN 清单跑延迟资格化；分支 11-08 checkpoint 文档记录后搁置 | 低延迟是产品支柱；不资格化则该线成果悬空 |
| A9 | M0.5/M1 遗留整体验收（PRD §16） | CPU 压力、反复启停、失联恢复、跨钟长时运行无积压 | MVP 对外承诺的底线验收 |

## 三、待拍板决策清单

| # | 决策 | 选项 | 建议 |
|---|---|---|---|
| D1 | 卡死端点开启限时（#105 Stage 1） | 10s（提案）/ 5s | 10s：健康设备 ≤330ms 开启无成本，5s 更激进但更依赖自动重试 |
| D2 | WASAPI 失败自动剥离（#105 Stage 4） | 做（带通知+自动还原+兄弟守卫）/ 不做 / 仅自动启动路径做 | 对 WASAPI 做、对 ASIO 不做；手动 apply 需"临时移除、将还原"明示 |
| D3 | 弃开注册表（#105） | 加 per-endpoint 待开上限防泄漏 / 允许堆叠 | 加 guard（约 20 行），bounds 泄漏数 |
| P-1 | VAD 端点呈现拆分（G12） | 模板拆分 / 下拉分组 | 先下拉分组：改动小、保持图唯一真相源；数据积累后再评估拆分 |
| P-2 | per-app 路由产品化（G13） | 产品化为可选实验特性 / 封存 lab 成果 | 封存：隐藏 ABI 风险高，G1 的 loopback 路线先走 |
| P-3 | IME 全角减号 `－inf`（#91-5） | parse_gain_db 接受 / 明确拒绝并提示 | 拒绝+提示"用半角 -"：改动最小、语义清晰 |
| P-4 | 文件播放四待定项（PRD §5.7，G3 前置） | 解码格式范围；Pad 内存上限；线程池共享；快捷键注册范围 | WAV 先行、Pad 上限拍 64 MiB 同 Recorder 量级、共享工作线程池、快捷键先仅前台 |
| P-5 | hytao/外部播放器接入（PRD §17.16） | 通用音频端点 / 专用通路 | 维持原判断：通用端点先行，实测不达标再重估 |

## 收束建议（优先级序）

1. **G2 录音接线** + **G14 #105 实施**（含 D1–D3 拍板）：前者是引擎已备好却完全不可达的用户价值，后者防真机卡死事故，都是"已具备实现基础"的 P1。
2. **G1 Takeover 产品化**：lab 已验证方案 A，是产品差异化承诺（真正改路由而非监听）的最后一块硬骨头，依赖 G14 基建。
3. **G7 插件应用层**（先 G8 最小沙箱）：让已建成的三格式宿主对用户可见。
4. **实机验收欠账 A1–A4 优先清**：全部是已合并的大行为变更，越拖回归成本越高。
