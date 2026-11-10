# Design 23 — 设备页职能收敛：只做设备配置与监控

状态：已批准并实施（2026-11-10）。只读审计基于 `main` @ `de926079`，文中行号均以
该提交为准。

用户裁决（2026-11-10）：

1. 做 A（设备页）与 B（删除 UI 影子账本）。C（service 退役 `SessionSpec`）涉及核心
   模型，另案，§9 记录。
2. 状态栏路由标签直接删除。
3. 设备详情里的"使用者"可点击跳转到节点。
4. 虚拟设备不在输入/输出页签露面，只归"虚拟设备"页签。
5. Mixer 探针电平表与 legacy 监听表搁置，另案。
6. 监控归属采用主方案：观测自带端点身份（§3.7）。
7. "导出"页签改名"虚拟设备"，文案去掉"候选""意图"。
8. `session.limit` 丢弃：B 起不再兑现，类型删除归 C（§9.1）。

## 1. 问题

### 1.1 症状

设备页 Inspector 的"会话资源"面板里有一个黄色的"下次启动"徽标，设备行上有"下次启动
时使用 / 下次启动"按钮。用户问它是什么意思，并指出：设备页根本不该有切换按键——图里
有那么多输出节点，换的是哪个？

### 1.2 审计事实

| # | 事实 | 位置 | 验证 |
|---|---|---|---|
| F1 | "下次启动"是候选块的小节标题：其上是运行中的会话（`snapshot.desired`），其下是 UI 自己暂存的选择。"启动"指音频会话启动，不是应用重启 | `ui/pages/devices.rs:589-635` | 读码 |
| F2 | 设备行按钮只改 UI 选择，并对**图中第一个** Source/Sink 发绑定编辑。多 IO 图里这个目标是任意的 | `devices.rs:393-402`、`ui/state.rs:752-775`、`service/graph.rs:127-141` | 读码 |
| F3 | 绑定编辑是结构性编辑，会话运行时 200 ms 后自动热应用。面板上"选择其他资源不会影响正在运行的会话"是错误陈述 | `graph.rs:597-599`、`managed/driver.rs:99-116`、`managed.rs:91`、`managed/lifecycle.rs:143-184` | 读码 |
| F4 | 热应用分支不刷新 `desired`，于是行级"活动"徽标和 Inspector 的活动块在热切换后陈旧 | `lifecycle.rs:143-184`、`service/core.rs:42-56`、`devices.rs:262-270` | 读码 |
| F5 | Start 走 legacy 物理路径时，service 用 UI 选择回写主 Source/Sink 的绑定 | `state.rs:586-625`、`driver.rs:51-72`、`graph.rs:853-887` | 读码 |
| F6 | 清除主 Source 的绑定后 UI 选择不会清空，下次 Start 把旧绑定写回（幽灵回写） | `state.rs:692-700` 加 F5 | 推演，未实机复现 |
| F7 | 工程文件同时保存图和 `session`。加载后首次 Start，两者不一致时 `session` 胜出 | `ui/shell/project.rs:747`、`1035-1041` | 推演 |
| F8 | 导出页"绑定所选 IO"读取的是 Graph 页的节点选中状态 | `devices/exports.rs:485`、`523-527` | 读码 |
| F9 | 虚拟设备行的"绑定"按钮挑第一个声道数匹配的节点 | `devices.rs:14-40` | 读码 |
| F10 | 应用页签在 ASIO 下被禁用并声称"应用回环需要 WASAPI"。引擎只按输入节点自身约束接受应用音源，与输出后端无关；节点下拉在 ASIO 下照常列出应用 | `devices.rs:175`、`413-419`；`moiren-windows-audio/src/engine_backend.rs:316-332`；`graph/binding_choices.rs:135-154` | 读码，实机未验 |
| F11 | 流观测按节点归属、不含端点身份，并且始终反映当前生效的传输。时钟页按序号列流，不显示设备名 | `engine_backend/observations.rs:14-26`、`engine_backend/owner.rs:773`、`796`；`ui/presentation.rs:83-192` | 读码 |
| F12 | 图不可启动时 Start 按钮置灰，没有原因提示 | `ui/shell/chrome.rs:126-135` | 读码 |

### 1.3 病根

"下次启动"只是症状。底下是单会话时代留下的模型：`SessionSpec` 描述**全局唯一的一对**
音源和输出。图早已支持多个各自绑定资源的 IO 节点，但这套模型还活着，UI 为了凑出一个
`SessionSpec`，维护了一份和图并行的影子账本：

- UI 侧：`UiState.selection`、`source_mode`、`tone_frequency`、`gain`、`pan`、
  `run_limit`、`max_block_frames`，以及 `candidate_config`、`candidate_spec`、
  `bind_candidate_resources`、`sync_graph_resources`、`restore_*`。
- 走带：`AppRequest::Start(spec)` 与 `ApplyGraph` 两条启动路径并存，UI 在本地克隆
  整张图预演 service 的校验来决定走哪条。
- service 侧：`configure_session`、`DesiredIntent::Session`、`snapshot.desired`。
- `desired` 的消费者：设备页、状态栏路由标签、Mixer 探针资格、legacy 监听表。

设备页的切换按钮是这份影子账本唯一的写入口。两份账本互相镜像、在 Start 时互相
回写，F4 到 F7 都出自这里。

### 1.4 节点侧已有完整入口

绑定在节点上早就能做全，删掉设备页的按钮不丢任何能力：

- `graph/binding_choices.rs` 是唯一的绑定选项表：测试音、物理输入、播放型虚拟设备、
  应用、刷新、物理输出、麦克风型虚拟设备、清除绑定。画布节点 chip 下拉
  （`binding_menu.rs`）和节点 Inspector（`node_inspector.rs:251`、`303`）共用它。
- Mixer 页选中条带后，Inspector 就是节点 Inspector（`ui/shell/sidebar.rs:678`）。
  条带自己的输入下拉是路由，不是设备绑定。
- `ClearBinding` 同时清掉节点自身绑定和指向它的虚拟设备绑定
  （`service/graph/edit.rs:96-109`），能替代导出页的"解绑"。
- 未绑定的 IO 节点在 chip 上显示"选择音源 / 选择输出"
  （`graph/graph_canvas.rs:228-229`）。从节点库新建的输入/输出节点本来就是未绑定，
  并自动打开 Inspector（`graph/library.rs:412-441`）。

## 2. 职责与不变式

设备页回答三个问题，并且只回答这三个：

1. 这台机器有哪些设备（物理和虚拟），现在在不在。
2. 每个设备打开时用什么流参数。
3. 每个设备此刻实际跑成什么样。

它不回答"哪个节点用哪个设备"（图的职责）和"会话是否在跑"（走带与状态栏的职责）。

硬规则：

- **R1** 设备页不发绑定类图编辑：`BindSource`、`BindOutput`、`BindExport`、
  `ClearBinding`。虚拟设备的生命周期与属性（`SetExports`、
  `SetExportAuthorizations`）属于设备配置，保留。
- **R2** 设备页不持有、不修改任何影响启动的状态。
- **R3** 设备页展示的运行事实以引擎观测为准。候选图只用来回答"谁绑定了它"，并照此
  标注。
- **R4** 绑定只有一个入口族：节点（画布 chip 下拉、节点 Inspector）。

不推翻的东西：图编辑自动应用与失败保持旧计划、报错就近显示（PR #100）、缓存面板与
键规则（design 17）、i18n 双向台账（design 21 §5）、首次启动的默认绑定与工程恢复
（PR #94）、覆盖配置编辑器与设置草稿机制（PR #99）。

## 3. 设计 A — 设备页

### 3.1 页签

`输入｜输出｜虚拟设备｜时钟`，默认仍是输出。

- 删除 `DeviceTab::Applications`。应用不是设备，在 Source 节点下拉里选。
- "导出"页签的用户可见名称改为"虚拟设备"。代码标识符（`exports.rs`、
  `DeviceTab::Exports`、元素 id）不动。

### 3.2 物理设备行（输入、输出页签）

```text
┌───────────────────────────────────────────────────────────────┐
│ 耳机 (5- FreeDSP)                  默认  覆盖配置  使用中 ×2  │
│ 48 kHz · 24-bit · 480 帧 (10.0 ms) · 独占                     │
└───────────────────────────────────────────────────────────────┘
```

| 元素 | 规则 |
|---|---|
| 名称 | 设备名，单行省略 |
| 徽标 | `不可用`：有已保存覆盖但设备离线。`默认`：系统默认（ASIO 下不显示）。`覆盖配置`：有独立配置。`使用中 ×N`：该设备上有 N 条生效流。`已绑定 ×N`：有 N 个节点绑定但没有生效流。两者都为 0 时不显示 |
| 摘要行 | 有生效流时显示实际流；否则显示已保存请求（沿用 `settings::stream_summary`） |
| 交互 | 整行点击等于现有的 `inspect_device(id)`（`pages/settings/model.rs:595-609`），选中态沿用 FOCUS 边框。**行上没有任何其他按钮** |

行的来源是 `input_rows` / `output_rows`（它们已经排除 Moiren 自有的虚拟端点，
`state.rs:19-87`），加上已保存覆盖但离线的端点。`device_rows` 把虚拟设备回填进物理
页签的那段（`state.rs:89-132`）删除。

### 3.3 设备详情（Inspector）

选中一个设备后，Inspector 自上而下是：标题与方向、监控、配置、页脚。

**监控**

- 使用者列表。每行是节点标签、状态、`在图中定位`。状态由两个独立事实合成：

  | 候选图绑定到本设备 | 本设备上有该节点的生效流 | 显示 |
  |---|---|---|
  | 是 | 是 | 运行中，附流明细 |
  | 是 | 否 | 未运行（会话停止时），或等待生效 |
  | 否 | 是 | 绑定已变更，旧流仍在运行 |

- 流明细沿用现有格式与文案（`presentation.rs:112-187`）：实际流、后端与模式、请求
  缓冲、桥计数，ASIO 另有回调计数和驱动报告延迟。
- "运行中请求"（`active_audio_settings.effective_*`）只在本设备有生效流时显示。
- 没有使用者时显示一句"当前没有节点绑定此设备。"

**配置**

覆盖配置编辑器原样保留（`devices/overrides.rs:7-124` 与页脚）。删除其后按 `desired`
配对判定的"活动会话"段和两句说明（`overrides.rs:125-159`），其内容由上面的监控段
取代。

**未选中设备时**

"会话资源"面板（`devices.rs:589-635`）整个删除。输入、输出页签下显示"选择一个设备
查看其配置与监控。"；虚拟设备、时钟页签下各显示一句说明。

### 3.4 虚拟设备页签

| 处置 | 内容 |
|---|---|
| 保留 | 创建（方向、声道、名称）、重命名、改声道、启用与停用、移除、本机默认资格 |
| 保留 | 监控项：端点状态、门控、引擎确认 |
| 删除 | "绑定所选 IO"、"解绑"按钮，`binding_candidate`、`submit_export_binding`（`exports.rs:110-127`、`173-188`、`512-543`）。对 Graph 页选中状态的依赖随之消失 |
| 改为只读 | "候选图绑定"改为"使用者"：节点标签加 `在图中定位`；未绑定时提示到 Graph 的输入/输出节点上选择此设备 |
| 改文案 | 去掉"候选""意图"这类内部术语；帮助文字里"绑定"的指引改为指向节点 |

移除或停用一个正在使用的虚拟设备，行为不变：等同设备被拔出，由节点侧报错。

### 3.5 时钟页签

保留为跨设备总览，因为主时钟与从属桥的关系本来就跨设备。每条生效流一组，组标题是
`设备名 · 节点标签 · 角色`，明细与 §3.3 相同。首行的处理参数行保留。

### 3.6 跳转：`reveal_node`

新增 `StudioShell::reveal_node(node, cx)`，供"使用者"条目调用：

1. 切到 Graph 页。
2. 单选该节点（现有 `select_single_node`，`pages/graph.rs:261`）。
3. 打开 Inspector。
4. 相机保持缩放。节点不在视口内时平移到视口中心；画布尺寸未知时跳过（Graph 首帧的
   自适应会兜底，`graph_canvas/frame.rs:55`）。

节点已不存在时不做任何事。跳转是导航，不写图，不违反 R1。

### 3.7 监控数据的归属

设备页要回答"这个设备上跑着哪些流"。F11 说明现有观测做不到精确：观测只带节点号，
而候选图可能已经改了绑定。新绑定被拒、旧计划继续运行时，按节点归属会把旧设备的流
记到新设备名下，而且会一直错到用户修正为止。

**主方案：观测自带端点身份。**

```rust
// moiren-windows-audio/src/engine_backend/observations.rs
pub enum StreamEndpoint {
    /// MMDevice 端点 id 或 ASIO 驱动 id，与设备目录行的 id 相同。
    Physical(String),
    Application { executable: String },
    Virtual(VirtualEndpointId),
}

pub struct ProfessionalStreamObservation {
    pub node: NodeId,
    pub endpoint: StreamEndpoint, // 新增
    // 其余字段不变
}
```

流的登记点有四处：物理输入与输出（`engine_backend.rs:520`、`536`，端点 id 就是
`Group.id`）、应用回环（`engine_backend.rs:686`）、虚拟 IO
（`engine_backend/virtual_io.rs:78`）。登记时端点身份都在作用域内。这是控制侧的
元数据，不进实时路径。

app 侧新增一组纯函数（建议放 `ui/pages/devices/monitor.rs`）：

- `bound_nodes(graph, device)`：候选图里绑定到该设备的节点。
- `live_streams(snapshot, device)`：端点身份等于该设备的生效流，沿用
  `fresh_observation` 的新鲜度守卫（`presentation.rs:14-22`）。
- 两者按节点合并成 §3.3 的三态。

`professional_observations` 只在 Windows 上存在，非 Windows 构建按现有惯例给空桩
（`presentation.rs:49-54`）。

**回退方案（已否决，仅记录）：** 按候选绑定归属，节点处于 `preparing_nodes`、带节点级
fault 或图有 `pending_revision` 时隐藏流数值。缺陷是图级 fault 的拒绝场景识别
不出来，仍会张冠李戴。

成本：观测 1 Hz 刷新，设备页在会话运行时每秒重建一次，与现在的时钟页相同。

### 3.8 删除清单

| 文件 | 删除 |
|---|---|
| `ui/pages/devices.rs` | `export_bind_candidate`（14-40）；行上的选择与绑定按钮及其点击逻辑（242-270 的 VAD 分支、329-407）；应用页签分支（412-469）；"会话资源"面板（589-635）；`DeviceTab::Applications` |
| `ui/pages/devices/overrides.rs` | `desired` 配对段与两句说明（125-159） |
| `ui/pages/devices/exports.rs` | `binding_candidate`、`submit_export_binding`、绑定与解绑按钮 |
| `ui/state.rs` | `device_rows`（89-132）；`bind_candidate_resources`（752-775，唯一调用者是设备页） |
| `ui/presentation.rs` | `native_timing_rows` 改为按流投影并带设备名、节点名；`source_name`、`output_name` 若无其他引用则删 |

### 3.9 职责守卫

仿照 i18n 守卫加一条源码扫描测试：`src/ui/pages/devices/` 下出现 `BindSource`、
`BindOutput`、`BindExport`、`ClearBinding` 任一标识符即失败。R1 由测试守住，不靠
约定。

## 4. 设计 B — 删除影子账本

### 4.1 `UiState`

| 删除 | 位置 |
|---|---|
| 字段 `selection`、`source_mode`、`gain`、`pan`、`tone_frequency`、`max_block_frames`、`run_limit`、`transport_check` | `state.rs:308-337` |
| 类型 `SelectionState`、`SourceMode`、`TransportCheckFingerprint` | `167-251`、`133-149`、`253-275` |
| `apply_source`、`current_selection`、`candidate_config`、`candidate_spec`、`candidate_preparation` | `490-579` |
| `transport_request`、`can_start_transport`、`transport_fingerprint`、`selected_resources_available` | `586-690` |
| `sync_graph_resources`、`restore_graph_candidate`、`restore_candidate`、`graph_resources`、`primary_parameters` | `692-802` |
| `request_parameters`（没有生产调用者） | `464-488` |
| `UiFailure::SourceUnavailable`、`UiFailure::GraphNotReady` | `284-303`，及 `presentation.rs:324-334` |

`poll()` 只剩替换快照和复位 `pending_request`。`restore_preferences` 与
`apply_project_presentation`（`project.rs:1005`）去掉对 `max_block_frames` 的镜像
赋值。`input_rows`、`output_rows`、`backend_offers_*` 保留。

### 4.2 走带：Start 就是按图启动

```rust
pub fn toggle_transport(&mut self) {
    match self.snapshot.phase {
        SessionPhase::Running | SessionPhase::Starting => self.dispatch(AppRequest::Stop),
        SessionPhase::Idle | SessionPhase::Failed => self.dispatch(AppRequest::ApplyGraph),
        _ => {}
    }
}
```

- 工具栏 Start 的禁用条件去掉 `!can_start_transport()`（`chrome.rs:134`），只剩请求
  挂起和 Stopping / Exiting / Exited 三个相位。
- **启动校验只在 service 一处。** UI 不再克隆图预演。校验失败走 service 现有的失败
  分支（`lifecycle.rs:79-99`）：能定位到节点的错误落在节点卡片和通道条上，其余进
  状态栏摘要，相位变为 Failed。fault 组在下一次成功编辑时清空（`graph.rs:723-724`），
  用户在节点上修好绑定后红框即消失，再点 Start 即可。
- 这同时解决 F12：原因显示在出问题的节点上，不再是一个没有解释的灰按钮。

### 4.3 工程文档与自动保存

- 保存不再写 `session` 字段（`project.rs:747`）。图里每个节点的绑定、增益、声像
  本来就完整保存。
- 加载不再恢复影子账本（`project.rs:1035-1041`）。
- 自动保存的指纹去掉 `session` 分量（`ui/shell/autosave.rs:49`、`96`）。
- 旧文档兼容：
  - 只有 `session` 没有图的旧文档，service 已经会把它转成默认图的绑定
    （`service/mod.rs:198-203`），不变。
  - 没有 `processing` 字段、块大小存在 `session.max_block_frames` 里的旧文档：这项
    迁移以前靠 UI 影子账本在 Start 时带过去。改由 `load_project` 接管——
    `processing` 缺失且 `session` 存在时，用它的块大小。
  - 新版本写出的文件没有 `session`；旧版本读到后会从图恢复，双向兼容。
- `SessionDocument` 类型和读取路径保留，属于 C 的边界。

### 4.4 状态栏路由标签

删除 `route_label`、`statusbar-route` 元素及其三条单测（`chrome.rs:367`、
`491` 起、`599-660`）。`shell/tests.rs:1654-1671` 的用例改名并去掉对应断言。

### 4.5 行为变化

| 变化 | 之前 | 之后 |
|---|---|---|
| Start 不再回写绑定 | Start 用 UI 选择覆盖主 Source/Sink 的绑定 | 图里的绑定就是启动时用的绑定。F6、F7 消失 |
| Start 不再置灰 | 图不可启动时按钮置灰，无原因 | 按钮可点，service 校验，节点上就近报错 |
| 主 IO 的设备离线 | 主设备不在目录中时不能 Start | 与其他 IO 节点一致：可以 Start，会话等待资源，设备回来后由 host 自行恢复 |
| 旧文档的 `session.limit` | 文档里的限时会带到 Start | 不再兑现。UI 从不产出限时，只影响手写或早期演示文档 |
| 状态栏 | 显示一对"音源 → 输出" | 不显示 |

不变：首次启动的默认绑定与自动启动、工程与自动保存恢复、图编辑自动应用、Stop 语义。

### 4.6 B 之后的 `desired`

`snapshot.desired` 仍然发布，启动时由 `managed_session_spec` 从图生成
（`lifecycle.rs:186-204`），语义与现在相同。它的消费者只剩搁置项：Mixer 探针资格、
legacy 监听表，以及 Graph 控件缓存键里的一个字段。F4 的陈旧问题留给 C。

## 5. PR 序列

| PR | 内容 | 触及 | 风险点 |
|---|---|---|---|
| PR0 | 本设计文档 | — | — |
| PR1 | A：§3 全部，含观测端点身份、`reveal_node`、职责守卫 | `moiren-windows-audio`、`moiren-app` | 面板键要覆盖新读取的输入；设备页截图基线按预期变化 |
| PR2 | B：§4 全部，含 `load_project` 的块大小迁移、§8 文档同步 | `moiren-app` | 启动语义变化，需实机验收 |

先 A 后 B。A 合入后影子账本失去写入口，只剩镜像图的作用，B 再删它时不需要任何
过渡代码。两个 PR 之间 F6、F7 依然存在，与现状相同，建议连续合入。

每个 PR 的门槛：`cargo fmt --all -- --check`、
`cargo clippy --locked --workspace --all-targets -- -D warnings`、
`cargo test --locked --workspace`、
`cargo test --locked -p moiren-app --features gpui-test`，加 ui_capture 截图审查。

## 6. 测试计划

### PR1

| 处置 | 用例 |
|---|---|
| 删 | `devices/tests.rs`：`owned_vad_rows_bind_in_opposite_app_directions_without_starting_audio`、`quick_bind_skips_channel_mismatches`、`quick_bind_prefers_an_unoccupied_match_and_falls_back_to_steal` |
| 删 | `exports/tests.rs`：`binding_requires_matching_direction_shape_and_exclusive_graph_io` |
| 改 | `exports/tests.rs`：`export_controls_create_rename_bind_unbind_toggle_and_remove` 去掉绑定、解绑步骤；`shape_edits_reject_incompatible_binding_then_allow_unbind_and_rebind` 的绑定改由图编辑直接构造 |
| 改 | `state/transport_tests.rs` 里两条 `device_rows` 用例改为断言"物理页签不含自有虚拟端点" |
| 留 | 设备面板的两条缓存用例与覆盖草稿用例（后者直接调用 `inspect_device`，不受行交互变化影响） |
| 增 | 监控投影的纯函数单测：三态合并；被虚拟设备认领的节点不算物理设备的使用者；观测过期时无生效流 |
| 增 | headless：设备页四个页签都没有选择或绑定按钮；点击设备行打开详情；"在图中定位"切页、选中节点、打开 Inspector；虚拟设备页没有绑定按钮 |
| 增 | headless：ASIO 后端下 Source 节点的下拉仍列出应用行（固化 F10 的结论） |
| 增 | 职责守卫（§3.9） |
| 增 | `moiren-windows-audio`：四个登记点产出的观测都带正确的端点身份 |

### PR2

| 处置 | 用例 |
|---|---|
| 删 | `state/tests.rs` 中围绕选择与候选的用例：`asio_candidates_and_selected_rows_use_the_same_driver_in_both_directions`、`vanished_asio_driver_keeps_saved_identity_without_wasapi_fallback`、`asio_output_accepts_tone_and_live_process_without_substituting_physical_input`、`defaults_and_refresh_preserve_identity_and_reject_vanished_selection`、`exact_process_identity_survives_reorder_but_not_pid_reuse`、`restored_audio_settings_sync_block_size_without_starting_audio` 里的镜像断言、`service_resource_binding_changes_update_the_shared_session_projection`、`unavailable_resources_remain_in_saved_candidate_configuration`、`graph_only_template_clears_previous_project_resource_selection`、`running_projection_edits_candidate_nodes_instead_of_legacy_session_pair` |
| 改 | `requested_backend_controls_names_and_row_validation_across_switches` 改为直接测行函数 |
| 删 | `state/transport_tests.rs` 中区分两条启动路径和指纹的用例，文件收敛为下面的新增项 |
| 增 | Idle、Failed 下点 Start 发 `ApplyGraph`；Running、Starting 下发 `Stop` |
| 增 | service 回归（对应 F6）：清除主 Source 绑定后 `ApplyGraph`，节点保持未绑定并得到节点级 fault，相位为 Failed；修好绑定后 fault 清空 |
| 增 | 目录里没有主输出设备时 Start 仍然发出请求 |
| 增 | `load_project`：有图、有 `session`、无 `processing` 的旧文档，块大小取自 `session` |
| 改 | 工程序列化用例：新保存的文档不含 `session`；含 `session` 的旧文档仍能加载 |
| 改 | 自动保存指纹用例去掉 `session` 分量 |

### 实机验收（两个 PR 合并后）

1. 运行中在节点 chip 上换输出设备：声音切过去，设备页两个设备的"使用中"徽标与流
   明细跟着变。
2. 把节点绑到一个打不开的设备：旧设备继续出声，新设备下显示"等待生效"，旧设备下
   显示"绑定已变更，旧流仍在运行"，节点上有报错。
3. 清除主 Source 的绑定后点 Start：不会被悄悄绑回去，节点上报未绑定。
4. 拔掉唯一的输出设备后点 Start：进入等待；插回后自行恢复。
5. ASIO 输出加应用音源：能出声。
6. 旧工程文件（含 `session`）打开、Start、保存、再打开。

## 7. i18n 台账

双向台账守卫（`src/i18n_guards.rs`）要求删除调用点时同步删目录键，新增文案同步
加键。以下清单以守卫的最终结果为准。

删除（确认没有其他字面引用后）：

- `Next start`、`Use next start`、`Bind`、`Bound`、`Applications`
- `No capturable applications`、`Start an application that plays audio, then refresh. Protected applications may be unavailable.`
- `Application loopback requires WASAPI. Choose WASAPI in Settings to use application sources.`
- `PID %{pid} · created %{created}`
- `Session resources`、`Active session`、`No active session`、`Source mode`、`Select available resources`、`Not selected`
- `Selecting another resource leaves the running session unchanged. Stop and start to switch devices.`
- `Configure never changes routing; choosing Use next start on a device row updates the candidate graph.`
- `This device is not bound to the active source/output pair.`
- `Choose an available source and output before starting.`、`Graph is not ready: %{error}`
- 导出页里含"候选""意图"的若干句，随 §3.4 的改文案替换

`SourceMode::title` 产出的三个动态键随类型删除，从 `DYNAMIC_KEY_EXCEPTIONS`
（`i18n_guards.rs:709`）及其在 `tests/i18n_ledger.rs` 的镜像表中清理仍无人产出的项。

新增（建议文案）：`虚拟设备`、`使用中 ×%{count}`、`已绑定 ×%{count}`、`使用者`、
`在图中定位`、`未运行`、`等待生效`、`绑定已变更，旧流仍在运行`、
`当前没有节点绑定此设备。`、`选择一个设备查看其配置与监控。`，以及虚拟设备页指向
节点的绑定指引。

设置页的"下次启动 · 引擎已停止"与保存提示不在本次范围，见 §9.3。

## 8. 文档同步（随 PR2）

- `crates/moiren-app/README.md` 第 6 行：改为"Devices 配置各设备的流参数并查看
  设备与流的状态；输入、输出与应用音源在节点上绑定"。
- `FRONTEND.md` 第 55 行与第 337 行：去掉 Applications 页签与 "Next start" 状态，
  按 §2 的职责重写。
- `docs/designs/13-application-frontend-integration.md` 已归档，不改；其"输入、输出
  选择是下次启动候选"一条由本设计取代。

## 9. 另案记录

### 9.1 C：service 退役 `SessionSpec`

B 之后 UI 不再产出 `SessionSpec`，service 里它只剩内部用途。范围索引：

| 项 | 位置 |
|---|---|
| `AppRequest::Start(SessionSpec)` 及其处理 | `service/model.rs`、`managed/driver.rs:51-72`、`service/mailbox.rs:100`、`service/mod.rs:312` |
| `DesiredIntent::Session` 与 `desired()` | `service/core.rs:13-26`、`247` |
| `AppSnapshot.desired` | `service/mod.rs:367` |
| `session_spec`、`managed_session_spec` | `service/graph.rs:890-938` |
| `configure_session`（B 之后的调用者：自动启动的默认绑定、旧文档迁移） | `graph.rs:853-887`、`driver.rs:276`、`mod.rs:200` |
| `SetGainPan`、`update_gain_pan`、`pending_gain_pan`（无 UI 调用者） | `driver.rs:128-151`、`mod.rs:318`、`core.rs:114-121` |
| `RunLimit` 全链：枚举、`SessionSpec.limit`、`poll_run_limit`（`driver.rs:760-773`）、`RunLimitDocument`（`project/model.rs:286-305`）、`DesiredIntent` 的 limit 字段。2026-11-10 全仓清查：`Seconds` 无任何生产入口，仅测试与手写文档；B 起不再兑现，C 连类型删除 | `service/model.rs`、`core.rs:84-86` |
| `SessionDocument` | `project/model.rs:161-216` |
| "主节点"概念 `primary_node_of` 的其余使用者 | `graph.rs:136-141` 起 |

带入 C 的已知缺陷：F4（热应用不刷新 `desired`）。B 之后它的影响面只剩 §9.2 的两个
显示面。

### 9.2 Mixer 探针电平表与 legacy 监听表（搁置）

两者都用 `desired` 的那一对来判定资格：`pages/mixer/page.rs:285-331`、
`ui/monitoring.rs:200-245`，相关的还有 `tone_session` 和三个监听 topic。启动 C 之前
要先回答：探针表提供了什么节点电平表没有的东西，以及它与 2026-10-27 重叠审计里
"双 meter"一条的关系。

### 9.3 其他观察

- 设置页状态行"下次启动 · 引擎已停止"（`presentation.rs:270-274`）语义为真但措辞
  含糊；保存提示说"下次启动时生效"，旁边的"重启音频"按钮却能立刻生效
  （`pages/settings/model.rs:229`）。
- 虚拟设备的意图存在工程的图里。打开另一个工程会创建或退役 Windows 端点。设备的
  存在与否取决于打开哪个工程，这一点值得单独评估。
- 会话等待资源时相位显示为 Failed，同时 host 仍在等待并会自行恢复。B 让这个状态
  更容易被用户遇到，它是否需要独立的相位值得评估。
- Mixer 条带上没有设备绑定入口，要经 Inspector。可以考虑在条带 I/O 块加 chip。

## 10. 风险与待确认

风险：

- **面板键漏项。** 设备详情新读取的输入（观测、图节点绑定）都随快照进键；实现时逐项
  核对 `DevicesKey` 与 Inspector 的键。2026-10-31 审计的五个 P1 全是这一类。
- **启动语义变化。** §4.5 的三项变化以 §6 的实机清单验收。
- **后端观测字段。** PR1 触及 `moiren-windows-audio`，要确认 CI 的依赖闭包与
  rt-check 影子工程不受影响。
- **截图基线。** 设备页的截图按预期变化，其余应保持不变；若基线里有运行态画面，
  PR2 之后状态栏少一个标签。

已拍板（2026-11-10，全部确认）：监控归属采用主方案（§3.7）；"导出"改名"虚拟设备"
并去掉"候选""意图"措辞；`session.limit` 丢弃（B 不兑现、C 删类型）；**Start 按钮不再
因图无效而置灰**（§4.2，按推荐方案：可点、service 校验、错误落在节点上就近显示）。
设计定稿。
