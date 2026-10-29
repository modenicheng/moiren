# 第一轮 UI 接入：Graph、Mix 与应用骨架

> 状态：历史记录（第一轮 Slint UI 接入；Slint 已移除，见 FRONTEND.md）。

验收日期：2026-10-12。参考 Moiren UI v0.1.0 的 Graph page `100:338`（完整 frame `100:361`）、Mix frame `109:330`、节点卡片 `5:976`、toolbar 与 Inspector。用户允许修正 Figma 布局及重新设计节点卡片。

按职责提交：诊断500d86f、组件导入553c931、应用信号服务6ee64cd、真实epoch测试fixture8ed8978、Graph/Mix与骨架a5b2172、原生验收e0cc9f5。C4 Spec/Quality PASS；从21cba63开始的完整分支独立审查 APPROVE，四项发现全部关闭。未 push，用户07/08/09设计草稿保留。

## 布局约束

- 窗口分为 44 px toolbar、可伸展工作区和 28 px statusbar。工作区采用水平布局；中央内容消化剩余空间。
- 宽度达到 1280 px 时，210 px Library 和 280 px Inspector 同时停靠；1100–1279 px 收起 Inspector；940–1099 px 两侧都通过浮层打开。
- 浮层按钮预留独立位置，浮层打开后隐藏按钮；点击遮罩或 Escape 关闭浮层。短窗口的侧栏内容可以滚动。
- Library 分类与 Session I/O 按内容高度排列，额外空间交给滚动容器，避免将留白分配到设备选择区。
- Graph 与 Mix 保留各自组件实例，通过可见性切换；Graph 相机与共享 Gain/Pan 不因切页重置。Graph 表计订阅判断实际浮层是否显示，侧栏恢复停靠后不受旧浮层开关状态影响。
- 节点与连线使用 world 坐标。Fit 从实际节点边界计算缩放，并响应窗口宽高变化；用户平移、缩放后保留相机，直到再次 Fit。

## 节点卡片

普通卡片为 186 × 105 px，输出卡片为 226 × 105 px。卡片内部用 VerticalLayout 按内容分配尺寸：26 px 角色图标和可伸展的标题区构成头部，长设备名省略显示。角色由图标颜色表达，选择由描边表达；下方为信号或参数轨道和端口行。输入源图标按 Physical/Application/Tone 模式变化。Gain 与 Pan 卡片在头部显示当前参数，使用独立的细参数轨道及位置标记；Pan 的双极填充从中心刻度延伸。真实 peak/RMS 只由 Source 和 Sink 卡片显示，不将来源/汇端测量伪装为 Gain/Pan 的独立观测。

Rust 投影同时发布卡片尺寸及端口中心 `y = 83 px`，连线从同源坐标生成。移动节点只修改呈现位置，连线随端口同步移动。端口目前为展示；选择连线通过中点标记完成。Slint 的 GraphCardMetrics 统一定义 padding/header/gap/meter/routing 的纵向节奏为 16/32/10/9/12 px，仪表从 y = 58 px 开始；端口为 10 px，输入空心、输出实心。Source 与 Sink 的双行条分别绘制 RMS 长度和 peak 刻度，并只在真实 LatestBlock 读数到达后显示。

Fit 从卡片实际边界、画布留白和上/下层工具栏空间计算缩放，并将节点群组居中。滚轮缩放围绕光标，工具栏的独立减/加按钮围绕画布中心缩放，Select/Pan 分别显示工具选中状态；网格根据 viewport 生成并以正负 camera offset 的规范化余数定位，不受节点 world 边界限制。Source/Sink 卡片的真实仪表几何计入 Graph 页的可见订阅 demand。Mix channel strip 固定为 196 px 宽，Session 区伸展，滚动内容明确采用至少 528 px 宽、通道条至少 580 px 高；短窗口交由 ScrollView 滚动，避免压缩仪表及设备行。Mix 和 Inspector 的输入表计固定为 38 px 宽，与读数和状态并排。

## 数据与运行边界

第一轮固定会话链路是 `Source → Gain → Pan → Sink`，均为 stereo。`session_graph.rs` 提供共享构造器；`StudioState` 持有 `LogicalGraph` 并投影 typed NodeId/EdgeId，tone 和 monitor 使用相同构造器准备运行图。此轮未开放拓扑结构编辑，也未把画布模型当作运行时可编辑图。

设备来自真实 catalog；application 选择保留 PID 与 creation time 身份。Library 条目选择当前会话节点，源条目同时切换下一次 Start 的源模式。

Starting/Running 时，Graph、Mix 和 Inspector 的来源、去向、源类型图标及详情统一由当前 SessionSpec 投影。Session I/O 保留下一次 Start 的候选选择；运行 A 时改选 B 不会把 A 的电平标成 B。活动设备从 catalog 消失时显示其原始 ID 的 unavailable 提示；application 名称同时匹配 PID 与 creation time，避免 PID 重用后误归属。

Gain/Pan 在 idle 时修改候选参数，在 Running 时发送 `AppRequest::SetGainPan`，并展示真实 Accepted/Applied 回执。Inspector 中的控件明确作用于整个会话。所有 Slint setter 与 model notification 都在释放 `RefCell` borrow 后调用；保留同一 VecModel 并只更新变化的行。

Mix 展示一个真实 stereo session strip。默认 gain 为 0.01（−40 dB）。Start/Stop、generation、block size 和选定 output 均来自现有服务。关闭窗口保留原有 hide-to-tray 行为，Ctrl+Q 请求 service exit，并等待服务退役后结束事件循环。

## 验证

- `cargo test --locked --workspace`：452 项测试及文档测试通过；4 项原生验收按需独立运行并通过。包含节点移动与连线投影、互斥选择、参数合法性、订阅取消、跨会话 epoch 和旧 Latest 缓存过滤。兼容请求读取的回归测试使用同一 ServiceContext 注册和派发 Topic，并在 publish 后检查 consumed request ID。
- `cargo clippy --locked --workspace --all-targets -- -D warnings`：通过。
- `cargo fmt --all -- --check` 与 `git diff --check`：通过。
- `slint-viewer --check`：main 与 dev-preview 均通过。
- 尺寸切换回归：940 px 打开 Library 浮层后，扩至 1600 px 停靠，Graph output region 恢复 enabled；再缩至 940 px，实际浮层显示时所有业务订阅为 0；关闭浮层后 tone generation 从 1 升至 2，subscriber count 恢复为 1，并收到新 RMS/peak。
- `cargo build --locked -p moiren-app --features slint/mcp`：通过，并用于实际原生 UI 验收。
- 本次重设计实际复验 1600 × 956 和 940 × 600 logical px，系统缩放为 1.25；最小窗口截图为 1175 × 750 physical px。检查停靠/浮层、Fit、独立减/加缩放、Pan 工具选中、节点拖动以及 Mix 内容滚动。此前 1180 × 760、连线选择、Library 定位与切页保留相机的原生证据仍保留。
- 原生 Gain 节点拖动实测移动 `(64, 90)` logical px，端口与连线同步；Inspector 显示选中节点。缩放减/加实测为 100% → 80% → 100%。
- 440 Hz tone 实际启动至 Running；在 Mix 将 Gain 从 −40 dB 调至约 −34 dB，并调整 Pan 至 R 55%，收到 `Applied · request 24 · revision 1 · frame 49536`。Graph 输出卡片与 Mix 显示真实双声道 RMS/peak；tone 没有独立 input publisher，输入读数明确显示不可用。随后 Stop 回到 idle，tone subscriber count 归零，Ctrl+Q 正常退出进程及 MCP listener。

本次截图及原始证据在 `crates/moiren-app/.tmp/`：`ui-round4-graph-final.png`、`ui-round4-graph-drag.png`、`ui-round4-graph-940.png`、`ui-round4-pan-tool.png`、`ui-round4-mix-live.png`、`ui-round4-mix-940.png`、`ui-round4-mix-940-scrolled.png`、`ui-round4-native-evidence.json`、`ui-round4-workspace-tests.log`、`ui-round4-clippy.log`。此前 ui-round1 和 c4-round3 的记录保留在同目录。

尺寸切换的修复验收保存在 `ui-round4-visibility-regression.json`、`ui-round4-regression-redocked-live.png` 和 `ui-round4-graph-940-live.png` 中。

最终复验日志为 `final-reviewed-workspace-tests.log`、`final-reviewed-workspace-clippy.log` 与 `final-linux-epoch-check.log`；Linux 为跨目标编译检查。4 项独立原生 fixture 覆盖候选 B 与活动 A 的标签归属、equal peak/RMS 的绘制层级、空 catalog/错误重试/全部七阶段，以及真实默认输出上的 hide/restore/Stop/restart/Quit。低软件 gain 0.005 的实际输出 peak/RMS 约为 −46/−49 dB；独立分析在 UI 隐藏期间 sequence 5→15、generation 保持 1，重启前后完整窗口均为 frames 0–1536，但 epoch 为 1→2。退出至 owner/service/worker join 为 239 ms，无错误。证据分别位于 `.tmp/native-peak-marker/`、`.tmp/native-acceptance/` 和 `.tmp/native-audio-acceptance/`；合成 catalog 与峰值图层场景已与实际硬件数据分开标识。

## 实时电平接入

- AppService 注册稳定的 `monitor/input/output/level`、`monitor/output/level` 和 `tone/output/level` Topic；Graph 与 Mix 只在实际可见的输出仪表区域保持业务订阅。
- 引擎在 Source 输出和 Pan 输出安装 1536-frame、逐声道 peak/RMS 观察器；发布只发生在有订阅且窗口完整时，不影响音频样本或处理时序。
- Slint 的仪表区域通过 `absolute-position`、ScrollView 的 `content-x/content-y` 与 `visible-width/visible-height` 计算；同时裁剪 body、viewport 和窗口边界，部分相交保持订阅，完全移出才退订。
- 隐藏、切页、Stop 和最小化会释放订阅并清本地读数；恢复时重新建立 demand generation。无 publisher、等待完整窗口、关闭 topic 与真实读数分别展示，不用假零值冒充实时数据。
- 新 tone/monitor 会话在准备线程取得进程内唯一、检查溢出的 timeline epoch，保留 Topic 和订阅的同时仍能区分重叠的 frame 范围。UI 每次绑定会话时记录 Topic 的尝试序列下界，只接受之后产生的完整窗口；独立分析订阅持续存在时，重启也不会把前一会话的 Latest 缓存标成当前电平。裁剪暂停保留该会话下界，窗口恢复重新建立下界。
- 已核对 `c4-round3-native-evidence.json` 的实机记录：最小化时 input/output subscriber count 均为 0，原生恢复后 generation 分别从 5/4 升至 6/5，并收到新读数；hide 后再次归零，实际 tray 通知恢复后 generation 升至 7/6。Graph/Mix 切页、横向完全裁剪、参数 Applied 回执及恢复截图亦保存在同目录。

## 后续范围

节点新增/删除、拖端口连线、拓扑提交、项目与场景持久化、多通道 Mix、Matrix 仍未接入。CPU/XRUN 仍显示未知。当前第一轮交付不等于整个 Application + Signals 目标完成。
