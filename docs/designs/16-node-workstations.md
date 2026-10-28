# 节点工作台：参数、观测与布局

2026-10-27。依据当前 `feat/node-ui-workstations` 工作树的 gpui 卡片、Inspector 和 service 实现整理。本文记录参数清单、真实数据边界与逐节点验收方法。实现复用当前 gpui 主题及基础组件，不修改 Figma 母版；Figma 只核对指定的基础组件区域。

## 交互边界

- 拓扑与参数来自真实 `GraphSnapshot` / `GraphNodeState`，卡片和 Inspector 通过 `GraphEdit` 修改同一份图；UI 只保留输入、弹层、选择和布局状态。
- 配置展示候选状态，电平展示运行图。运行中 `candidate_revision` 与 `active_revision` 不同，卡片提示「编辑尚未生效；电平显示运行中的图」，不能把旋钮变化当作引擎已经应用。
- 卡片保留常用操作；Inspector 提供完整参数、资源绑定、通道条、端口路由、运行观测及删除。高级参数与路由默认展开，绑定、通道条和逐通道读数默认折叠。
- DSP 参数采用旋钮与数值输入，不统一堆叠横向滑杆。旋钮纵向拖动、Shift 微调、双击复位；数值在 Enter 或失焦时校验提交，未改动的格式化文本不回写量化值，错误输入保留并提示。
- `Level` 的含义依节点区分：Gain 卡片修改处理器增益，Source / Sink / Bus 修改通道条增益。物理输入与 Application 的数值是数字微调，不是设备硬件增益。

## 节点能力清单

`NodeKind` 共 15 种。Application 与 Tone 是 Source 的绑定模式，不是新增枚举；下表单列这两种用户入口。除表中次级内容外，所有节点均可进入通用 Inspector；通常有 Mute，Recorder / Meter / Spectrum 卡片不提供该快捷按钮。

| 类型 / 入口 | 用户参数 | 常用卡片交互 | Inspector 次级内容与边界 |
| --- | --- | --- | --- |
| Source（物理输入） | 输入端点、数字微调 | 输入设备 / Application 选择器、目录刷新、微调、Mute、真实竖表 | 清除 / 更换绑定；可改为 Tone；设备可用性以目录和服务状态为准 |
| Sink（输出） | 输出端点、通道条 Level | 输出设备选择、刷新、Level、Mute、真实竖表 | 清除 / 更换输出绑定、完整通道条、输入路由 |
| Gain | 处理器 Level | 增益数值与电平控制、Mute、图电平 | 精确增益、±1 dB、Unity、Silence；另有独立通道条增益 |
| Compressor | 输入增益、阈值、比率、Attack、Hold、Release、Knee、补偿增益、输出增益、干湿比 | 阈值 / 比率 / Attack / Release / 补偿 / 干湿六个主参数；拖动静态曲线改阈值与比率，Shift 微调 | 十参数按 Dynamics / Timing / Gain staging 分组；没有实时 GR 读数 |
| Eq | 单个 peaking band 的频率、增益、Q | 三参数旋钮与数值；拖动响应曲线改频率 / 增益，滚轮改 Q，Shift 微调 | 独立精确输入与同域校验；不是多段 EQ，也没有输入频谱叠加 |
| Gate | 阈值、衰减范围、Attack、Hold、Release | 五参数旋钮与数值、Mute、图电平 | 五参数独立精确输入与同域校验；linked downward expander，无实时门状态 / GR 可视化 |
| Limiter | 输入增益、阈值、Release | 三参数旋钮与数值、Mute、图电平 | 三参数独立精确输入；零延迟 linked peak limiter，不宣称 true-peak 或实时 GR |
| Delay | 延迟时间、反馈、Wet mix | 三参数旋钮与数值、Mute、图电平 | 三参数独立精确输入；时间上限来自已配置的 `max_delay_ms`，本页不调整其分配容量 |
| Bus | 输出 Level、动态输入端口 | 按实际输入列出来源与声道宽度，点击连接进入路由详情，增加输入 | 全部端口 / 连接、增加 / 删除末尾输入；删除已连接端口须先断开，不能凭空生成路由 |
| Pan（Balance） | 双声道平衡、Mono、Swap L/R | Balance 旋钮与数值、Mono / Swap、Mute | 精确平衡与 Left / Center / Right 步进；这是 stereo balance，不是任意多声道声像器 |
| ChannelMapper | 输出到输入映射，或有符号线性权重矩阵 | 4×4 分页精确权重，支持全部 256 通道；等宽时 Identity / Swap 1/2；自适应竖表 | 同一参数域，独立分页与输入草稿；有限负数和大于 1 的值均合法；0 表示该格无贡献 |
| Recorder | 会话内目标路径、WAV 编码、录音启停、队列容量 | 选文件、32-bit float / 24-bit PCM / 16-bit PCM 三选一、打开输出目录、开始 / 停止、状态 / 时长 / 剩余容量与错误 | 共享同一编码选择与队列容量 ±48,000 frames；不提供其他容器或独立采样率；详见录音约束 |
| Meter | 观测窗口帧数 | 128 / 480 / 2048 / 8192 选择；逐通道大号 held peak 与 RMS、真实订阅状态 | 2048 帧快捷设置、运行状态及逐通道读数；不是 LUFS / true-peak 仪表 |
| SpectrumAnalyzer | FFT 点数、hop 帧数、显示通道、dBFS 下限 | 真实单通道对数频谱、峰值频点、分辨率 / 窗长；FFT 与 hop 快捷选择、通道切换、−60 / −90 / −120 dBFS 显示下限、竖表 | 同一观测与快捷控制，另有完整引擎范围精确输入：FFT 16～8192 的 2 的幂，hop 1～FFT；候选配置与运行配置分开显示 |
| Rack | 路由内阶段顺序、bypass、阶段集合、当前阶段参数 | 阶段列表与当前阶段内联编辑；八类处理器的旋钮 / 精确输入或分页矩阵；旁通 / 移除 / 上下移动、按路由加入处理器 | 当前阶段完整参数，与卡片共享选择、独立输入实体；Compressor 十参数，Mapper 4×4 分页；不是插件宿主 |
| Application（Source 模式） | 应用绑定、数字微调 | 应用列表、刷新、微调、Mute、图电平 | 清除 / 更换来源；绑定使用 PID 与创建时间标识实例；选中条目不等于已经收到音频 |
| Tone（Source 模式） | 正弦频率、数字 Level | Frequency 旋钮与精确数值、Level、Mute、图电平 | 独立精确频率输入与 440 Hz 快捷绑定；不是波形选择器，仍依赖图启动与实际路由 |

Rack 的新增菜单包含 Gain、Balance、EQ、Compressor、Gate、Limiter、Delay、Mapper 的默认预设。顺序以 `RackSettings.routes` 的槽引用为准，操作前校验声道宽度；不能旁通、移除或移动会破坏宽度链的阶段。用户看到顺序和处理器名称，不显示内部 `RackSlot.id` 或「slot N」提示。

### 参数单位与现存差异

以下是当前卡片输入边界，不代表所有 Inspector 入口已经统一。

| 参数组 | 当前边界 / 约定 |
| --- | --- |
| Level / Balance | Level 精确输入与既有 Inspector 一致：−120～+24 dB，另支持静音；滑动 / 旋钮行程仍为 −72～约 +24.08 dB（16×）；Balance 为 −100%～100% |
| Tone / EQ | Tone 卡片与 Inspector 精确输入共用既有 service 范围：0 < Hz < 24,000；旋钮舒适行程保留 20～20,000 Hz。EQ 频率 20～20,000 Hz、增益 −24～+24 dB、Q 为 0.1～20 |
| Compressor | 范围从 service 的 `CompressorParameter` / 引擎参数域取得；卡片干湿比用 0%～100%，Inspector 精确输入明确使用 0～1 |
| Gate / Limiter | Gate 阈值与 Range 为 −120～0 dB；Limiter 输入增益 −60～+60 dB、阈值 −60～0 dB；这些节点的时间参数为 0～5,000 ms |
| Delay | 时间 0～`max_delay_ms`；反馈 −99.9%～99.9%，Wet mix 0%～100%；service 保存线性比例，不把百分数原值直接写入 |
| Mapper | 卡片与 Inspector 均输入有限有符号线性权重，不限制为 −1～1、不自动归一化；真实输入 / 输出端口决定矩阵维度 |

通用通道条还包含 Fader、双声道 Balance、Mute、Solo、Dim 与声道模式；它与节点本体 DSP 参数分开。Identity 区可改名并显示类型 / 声道数，声道数不是这里的自由编辑字段。

## 可视化的真实来源

| 展示 | 数据来源 | 不应作出的推断 |
| --- | --- | --- |
| 节点柱、RMS、held peak、Clip | 引擎逐通道电平窗口，经 service 的 `signal_topics.node_levels` 和 `shell.node_meters` 形成带显示平滑 / 峰值保持的 `DisplayLevel` | 仅 Running、有对应 topic 且状态 Live 时呈现有效读数；Waiting / Stale / 无发布者 / 停止不是 0 dB 或静音测量结果 |
| Meter 窗口选择 | `MeterSettings.window_frames`；引擎节点观测使用该配置，其他隐式 UI 观测使用其默认窗口 | 候选窗口设置不证明运行图已切换；held peak 不是未经显示处理的瞬时采样 |
| EQ 响应曲线 | 当前 EQ 设置与图处理采样率计算的 peaking biquad 静态幅频响应 | 不是实时 FFT、输入频谱或测量到的声学响应 |
| Compressor 曲线 | 当前输入 / 补偿 / 输出增益、阈值、比率、Knee、干湿比计算的静态传输曲线 | 不模拟当前包络；Attack / Hold / Release 的动态过程与实时增益衰减（GR）未显示 |
| Spectrum 面板 | 引擎 Hann 窗、相干和归一化、单边峰值幅度，逐通道 channel-major FFT，经活动所有者与配置感知的 service topic、可见性订阅传递 | 显示 `20 log10` 幅度 / bin，不是 PSD、RMS 或 LUFS；曲线排除 DC，峰值读数可含 DC；压缩到像素保留最强频点，不伪造平均或冻结；停止、过期、无发布者时清空曲线 |
| LUFS / GR | 当前卡片无对应观测展示 | **LUFS 和实时 GR 未实现于此 UI**；不能把 RMS 改名为 LUFS，也不能从静态压缩曲线伪造 GR |
| Recorder 电平 | 同一套 post-fader 图观测，标签为 `Graph monitor · post-fader` | 不是录音器精确的 pre-strip 录入采样；亮柱不证明文件已经写入 |
| 录音状态、时间、丢帧 | service 的 `RecordingSnapshot`、WAV writer 已接收帧数 / 确认采样率、端点统计 | `written_frames` 不是 fsync 保证；丢帧计数是端点累计值，不是本文件独立累计 |

### 录音约束

- 当前支持 **WAV（RIFF，32-bit float / 24-bit PCM / 16-bit PCM）**，默认仍为 float32。编码在录制前通过卡片或 Inspector 选择；Starting / Recording / Stopping 时 UI 与 service 均拒绝修改。文件采样率与声道数来自启动时确认的图处理格式，不来自设备混音格式，也不由卡片另设。
- PCM 在文件 worker 中转换：最近值舍入，半 LSB 向远离零方向取整，超范围饱和，非有限值写为静音；不加抖动或噪声整形。浮点保留原有数字余量与位模式。PCM 使用 WAVEFORMATEXTENSIBLE，不为图声道虚构扬声器位置；奇数长度数据只在封尾时补齐 WORD，data 长度不含 padding。
- 切换目标路径保留编码；切换已完成文件的编码会建立新的 Idle 意图，清除旧时长 / 观测并重新预检，不能把旧文件重新标为另一种格式。连续快速点击以最后一次请求为准，不因渲染快照滞后丢弃选择。
- 目标路径由 service 在**本次会话内**保存，不写入项目文件。要求新的 `.wav` 路径，拒绝覆盖已有文件；文件选择器返回时若图版本或项目已改变，须重新选择。
- 录音启停独立于图的启停；必须先启动音频。关闭卡片 / Inspector 不停止 service 持有的录音；Stopping 表示正在完成文件，不允许重复开始；Canceled 明确表示创建文件前取消，不能显示“已保存”。
- `Proposed format` 仅是录音前基于候选配置的预估；`Checking disk space` 是异步预检状态，不能冒充已打开文件的确认格式。
- 剩余时长是已知上限的最小值：RIFF 容量与扣除预留后的磁盘容量。磁盘预留为 64 MiB 加两秒聚合写入量；磁盘未知时仅标注 RIFF 上限，全部未知时显示未知，不保证可录时长。

## 自适应卡片与竖表

| 原则 | 当前实现与验证要求 |
| --- | --- |
| 内容决定高度 | 原生控件实测高度回写卡片布局，不再给所有种类统一固定 body 高度。绑定选择器、主参数、操作按钮优先可见；矩阵、阶段列表、多通道读数可局部滚动 |
| 按职责选布局 | I/O 将绑定放在整行，下方参数与竖表并排；当前 EQ / Compressor / Gate 使用侧表。Pan / Bus / Recorder / Meter 与非空 Rack 采用竖表；Gain / Tone / Limiter / Delay 和空 Rack 保持紧凑横表 |
| 侧表只拉伸 | `node_vertical_meter(levels, channels, clips)` 使用 `h_full`，外层 `absolute top_0 bottom_0` 随相邻内容拉伸；竖表不反过来决定卡片高度 |
| 共用宽度接口 | 外层与组件都用 `node_meter_width(channels)`：单 / 双声道 100 px，之后每声道加 38 px，外层最多占八声道的 328 px；更多声道在内部横向滚动，保留自然字宽及逐声道 Clip，不压缩成不可读的柱 |
| bounds 内分区 | 从实际 canvas bounds 划出顶部短 CLIP 标识及逐声道锁存条、有效 track、底部 L/R 或声道编号与 held peak 数值。Clip 不画到 bounds 外；外层点击重置锁存 |
| 一套坐标 | 刻度、marker、填充共用 `FaderScale` 和有效 track；按 track 高度筛刻度。短高 64～300 px、受限宽度和多声道由纯几何测试约束，柱宽不得为负或越界 |
| 可读数值 | 刻度 9 px，声道名 / 读数 10 px；held peak 一位小数，不在每格重复 dB 字样，缺失为「—」，真实静音为「−∞」 |
| 保持旧契约 | `compact_horizontal_meter` 每声道 Clip 位于最右；`compact_stereo_meter` Clip 位于顶部，默认外高仍为 182 px（176 px track 加间隔 / Clip），覆盖高度后按实际 bounds 绘制；Mixer 的 `channel_meter` 与 fader 共用 track 对齐不变 |
| 缩放与事件 | 小于 0.85 的语义缩放展示概览与 Edit node 入口；概览优先保证 Edit node 可达，空间充足才附加读数。参数操作不触发拖卡、平移、连线或 Delete 节点；重叠卡片的原生控件与背景共享堆叠顺序 |

多声道虽然有宽度计算与边界保护，仍须逐节点验证卡片总宽、内容换行与可读性；「不越界」不等于任意窄容器里仍能读清全部声道。

## 逐节点截图验证

采用「选一个节点 → 改布局 → 截卡片与 Inspector → 检查并记录 → 再处理下一个」的循环；批量合照仅作导航，不代替单节点验收。

| 验证对象 | 必须覆盖 |
| --- | --- |
| Source、Application、Tone、Sink | 长资源名、未绑定 / 无资源、停止 / 运行、候选未应用；单 / 双声道，数字微调与实际电平分离 |
| Gain、Pan、EQ、Compressor、Gate、Limiter、Delay | 主参数与单位不截断，旋钮 / 数值 / 曲线可操作，Inspector 可滚动；EQ 双「−12.0」分隔；静态图说明、Mute 与实际读数 |
| Mapper、Bus、Rack | 等宽 / 异宽、矩阵滚动、空输入 / 空阶段、有真实连接、非法宽度操作禁用；长列表不挤掉主操作，无内部 slot ID |
| Meter、Spectrum、Recorder | 无订阅 / 过期 / 运行读数；FFT 验证真实音调频点及配置切换；录音未选路径、预检、录制、停止收尾、失败与磁盘未知，不用模拟实时数据填图 |
| 每一个节点 | Comfortable / Compact 密度、正常 / 概览缩放、卡片与 Inspector；文本、焦点、tooltip、按钮可达性；适用时检查多声道与短高 |

截图目录为 `docs/designs/screenshots/node-workstations/`。逐节点捕获使用真实 gpui 渲染、真实引擎处理和受管电平订阅，音频 I/O 使用测试后端，不代表物理设备验证。每个节点分别捕获 1440×900 与 1024×720 下的卡片和 Inspector，并验证原生控件位于实测卡片边界内。扩展场景覆盖多输入 Bus、有阶段 Rack、录音准备/录制/封尾、50% 概览、32 声道滚动以及长资源名。录音场景先核对真实 WAV 文件头、帧数与文件大小，再删除测试 WAV，仅保留截图和清单证据。纯几何测试不代替真实字体与布局检查。

### 本轮截图证据

本轮在本地保留 23 份 JSON 捕获清单、98 张 PNG，清单引用的图片全部存在。按用户要求，这些生成物不进入 Git 暂存区或提交；仓库只保留捕获程序和文字验收记录。逐节点复核后再统一刷新共享控件改动；没有把一张全景图当作所有节点的验收。最后复核覆盖旋钮标签 / 输入 / 单位对齐、长设备名省略、录音长路径、多声道横向滚动、重叠节点与概览入口。

复现方式：构建 `cargo build --locked -p moiren-app --features gpui-test -j 2 --example ui_capture`，设置 `MOIREN_CAPTURE_NODES=1` 与 `MOIREN_CAPTURE_NODE=<节点筛选值>`，再执行生成的 `ui_capture` 示例并传入本地输出目录。筛选值包括 `input`、`gain`、`balance`、`eq`、`compressor`、`gate`、`limiter`、`delay`、`mapper`、`bus`、`rack`、`meter`、`spectrum`、`recorder`、`output`、`application`、`tone`，以及 `bus-expanded`、`rack-populated`、`recording-states`、`overview`、`meter32`、`input-long`。

以下为 Compact、100% 缩放时清单记录的逻辑像素，说明竖表由不同内容高度驱动，而非统一固定尺寸；电平表高度包括 Clip 与底部声道读数区，并非单独有效 track 的高度。

| 场景 | 卡片高度 | 电平表高度 |
| --- | ---: | ---: |
| 物理输入 | 300 | 144.8 |
| Balance | 244 | 175.2 |
| Bus：一个输入 | 253 | 184.8 |
| Bus：展开多个输入 | 417 | 348.8 |
| Rack：已有处理阶段 | 279 | 210.4 |
| EQ | 364 | 295.2 |
| Compressor | 478 | 409.6 |
| Recorder：未选文件 | 365 | 296.8 |
| Recorder：录制中 | 450 | 381.6 |
| Meter：32 声道 | 527 | 372.0 |

录音捕获通过真实 worker 写入并完成了 48 kHz、双声道 float32 WAV：196,128 帧、1,569,082 字节，文件头与长度校验通过。清单记录 `wav_header_verified: true` 和 `capture_wav_removed: true`；不保留测试音频，不将该测试后端结果表述为物理设备验证。

## 源码索引

| 职责 | 路径 |
| --- | --- |
| 15 种节点定义 | `crates/moiren-core/src/graph/model.rs` |
| 卡片、参数与交互 | `crates/moiren-app/src/ui/window/graph/node_controls.rs`；`node_controls/{fields,io,parameter,rotary,plots,recording}.rs`；`node_controls/utility/` |
| 次级编辑与诊断 | `crates/moiren-app/src/ui/window/graph/node_inspector.rs`；`node_inspector/{binding,parameters,routing,strip,diagnostics,sections}.rs`；`node_inspector/builtin/` |
| 图与录音服务 | `crates/moiren-app/src/service/graph/`；`service/recording.rs`、`service/recording/`、`service/managed/recordings.rs` |
| 实际电平与几何 | `crates/moiren-app/src/service/managed/meters.rs`；`crates/moiren-engine/src/host/signals.rs`；`crates/moiren-app/src/ui/monitoring/`；`ui/widgets/meters.rs`、`ui/widgets/meters/` |

后续新增能力须同步参数入口、订阅来源和单节点证据；没有真实数据源的观测继续明确标为不可用，不用示意动画替代。

## Recorder 格式补全（2026-10-28）

在 PR #23 的已验证节点基线上单独完成 Recorder，再继续其他节点。新增 `MOIREN_CAPTURE_NODE=recording-formats`：真实点击三种编码，各自在 Comfortable / Compact 下捕获准备和保存状态，共 12 张图；逐个完成六个真实短 WAV，遍历 RIFF chunks 校验编码、有效位数、帧数、字节数、padding 与非零有限采样，校验后删除本次测试文件。忙碌状态通过真实点击验证无请求派发，不依赖原生按钮未提供的 disabled 测试元数据。

本次重新运行 `recorder`、`recording-states`、`recording-formats`，共 22 张 PNG、3 份清单，均仅存在于本地 `docs/designs/screenshots/node-workstations/recorder-formats/`，不进入 Git。截图前将指针移到非交互标题栏区域，避免先前按钮的 tooltip 遮住正文。未选文件卡片高 415 px、竖表高 346.4 px；已配置路径与容量信息时卡片高 500 px、竖表高 431.2 px，均为内容实测，不是新增固定高度。布局校验覆盖每张图的卡片边界和竖表上下对齐。

验证：app GPUI 全套 488 passed / 0 failed / 3 ignored；最终捕获工具回归 17 passed；engine 全套 350 passed，8 个套件；默认及 GPUI-feature workspace strict Clippy、rustfmt 与 diff 空白检查通过。音频来自测试后端，不宣称物理设备录音验收。原有 98 张截图和更早的 workspace 测试结果仍属于前述基线，不混称为本次重新执行。

Rack 阶段参数内联编辑、实时频谱、Tone / Mapper 参数域一致性与 Inspector 完整精确输入仍是后续任务；此补全不代表整个节点重设计目标完成。


## Rack 阶段工作站（2026-10-28）

Rack 采用可滚动的双列阶段选择器，一次只展开当前阶段，不把整个处理链展开为旋钮墙。卡片与 Inspector 共享稳定槽位选择，但输入实体、草稿、错误与拖动所有权分离；选择阶段不编辑图，也不自动打开 Inspector。

- Gain、Balance、EQ、Compressor、Gate、Limiter、Delay 均有卡片旋钮和精确数值输入。Compressor 卡片显示六项常用参数，Inspector 以双列精确输入补全十项；Balance 的 Mono / Swap 与阶段旁通、路由内上下移动、移除直接可达。
- Mapper 使用 4×4 分页带符号权重矩阵，可访问全部 256 通道，不把合法负数或大于 1 的值限制成概率。Identity / Swap 只在真实宽度允许时启用；旁通 Mapper 编辑的是配置宽度，不是旁通后的有效宽度。
- `EditRack` 只提交字段级意图，由 service 合并最新候选状态；实际输入输出端口参与校验。服务所有的 epoch 防止替换、删除重建、undo / redo、加载后旧控件回调修改新阶段。旁通 / Mono / Swap 使用服务端 toggle 语义，快速连续点击不会因渲染快照滞后丢失第二次操作。
- 无效草稿与错误可跨阶段 / 分页保留，导航时提交有效修改；清洁的隐藏输入、拖动状态与失效 epoch 及时回收。保留草稿不抢占当前键盘焦点，隐藏 Inspector 的错误也不锁死卡片导航。
- Rack 宽度按已配置阶段的信息预算计算，切换阶段不移动连接口；高度来自当前原生内容测量。空 Rack 使用紧凑横表，所有非空 Rack 使用贴合实际 body 高度的竖表。共享旋钮输入修正了单行高度调用和数值水平中心；Overview 如会覆盖节点则隐藏，不能挡住电平表读数。

本轮按 `rack`、`rack-populated`、`rack-editing` 分别验证：空 Rack / 四阶段 Rack 各四张，八类阶段卡片各两种密度，Compressor / Mapper Inspector 各两种密度，共 28 张 PNG、3 份 JSON。生成物仅保存在本地 `docs/designs/screenshots/node-workstations/rack-editing/`，不进入 Git。`rack-editing` 验证真实选择按钮、共享选择、相机不移动、原生输入数量、控件边界与 Overview 无遮挡；其清单明确不声称原生捕获程序执行了参数输入，精确输入及手势由 GPUI 接受测试单独覆盖。

Compact 下空 Rack 高 228 px；四阶段 Rack 选 Gain 高 392 px、竖表 324 px；八阶段 Rack 选 Gain / EQ / Limiter / Delay 高 428 px、竖表 360 px，选 Compressor 高 562 px、竖表 493.6 px，选 Mapper 高 468 px、竖表 400 px。不同高度均来自内容，不是统一卡片高度。

限制仍然明确：Rack 参数沿现有 prepared-structure / 自动 Apply 路径，不冒充无重建实时调参；没有每阶段 GR 或 FFT 观测，不绘制假表。实时频谱、Tone / 独立 Mapper 参数域一致性和其他节点 Inspector 的完整精确输入仍属于后续工作。

验收结果：app GPUI 全套 540 passed / 0 failed / 3 ignored；Rack 定向回归 64 passed；Overview 模块调整后定向回归 1 passed。默认与 gpui-test feature 的 workspace / all-targets Clippy 均以 `-D warnings` 通过，rustfmt 与 diff 空白检查通过。共享输入组件额外重新捕获 Input、Gain、EQ、Compressor 的卡片与 Inspector、两种密度，共 16 张 PNG / 四份清单，保存在本地 `rack-editing/shared-fields/`，同样未入库。

## Spectrum 实时工作站（2026-10-28）

此增量替代前文“实时频谱尚不可用”的阶段性记录，其他未完成项不因此消失。卡片与 Inspector 直接显示引擎已激活的 Hann FFT；UI 不另算 FFT，也不生成假曲线。数值为单边 coherent-Hann 校正后的每 bin 峰值幅度，经 `20*log10` 转成 dBFS，不是 RMS、PSD 或 LUFS；观察位置为节点 strip 之后。

- 卡片提供对数频率图、实际 Nyquist 上限、声道切换、峰值 bin、频率分辨率 / 窗时长、显示下限、FFT 与 hop 预设。Inspector 额外提供 FFT / hop 整数精确输入；字段级请求合并最新候选状态，连续修改不会用旧快照覆盖其他字段。
- 图卡片与可见 Inspector 驱动订阅；仅 Mixer 可见不维持 Spectrum 订阅。隐藏即清除显示，释放有 750 ms 宽限；重新出现须等到新发布，不能将保留帧显示为实时。session / owner / topic / settings 及 source-frame 边界约束陈旧数据。
- 引擎泵按完整 payload 预留预算，FFT8192 在 64 / 256 声道实测可交付；最大一次复制为 4,195,400 字节，配置预算 4,195,456 字节。level host 保持 131 slots，service directory 为 263；不宣称可同时观察全部 4096 节点。
- 卡片宽度由图控件与声道表预算组成，高度由原生内容决定。Spectrum 使用贴合 body 的竖表；停止、等待、过期分别呈现，不把旧曲线当作运行数据。

最终原生捕获 `MOIREN_CAPTURE_NODE=spectrum-live` 完成 16 张 PNG / 一份 JSON：两种密度、卡片与 Inspector、实时 / 配置生效 / 第二声道 / 停止状态。937.5 Hz、48 kHz、峰值幅度 0.1 的真实引擎测试音产生预期 −20 dBFS 峰值。所有竖表 bounds 验证为 true，精确输入未越界；Compact Inspector 与停止卡片已重新目视复核。生成物只在本地 `docs/designs/screenshots/node-workstations/spectrum-live/`，不入库；不代表物理设备验收。

验证：app GPUI 全套 577 passed / 0 failed / 3 ignored；engine 351 passed、八个套件；默认及 gpui-test workspace/all-targets strict Clippy、fmt 和 diff 检查通过。回归覆盖无效输入恢复、尚未收到快照时的 FFT/hop 连续修改、快速切回已选预设、隐藏/重启/换形后的数据边界。Tone / 独立 Mapper 一致性与其余 Inspector 精确输入仍待完成。

## Tone 精确输入与参数基础（2026-10-28）

Tone 的卡片与 Inspector 精确频率统一使用 service 的既有合法范围；旋钮的舒适行程不等于输入域。输入显示保留可往返浮点文本，极小正频率与小数不会在未编辑失焦时量化回写。Level 数值输入支持与既有 Inspector 相同的 −120～+24 dB 和静音；保留手势端点 16×，不改变 Mixer fader 几何。Delay 的 ±99.9% 转换修正了浮点端点越过 ±0.999 的问题，不钳制非法输入。

Inspector 使用独立输入实体、基线、错误草稿与待消费请求。卡片同样按真实适用参数回收控件；输入实体身份校验阻止 Tone→物理输入→Tone 后的旧 Blur 修改新绑定。连续输入 1000→440 时不因旧快照仍为 440 而丢失最后请求；队列 Busy 不提前更新基线，保留原文供重试。三项审查回归均先复现失败再修复通过。

`MOIREN_CAPTURE_NODE=tone` 最终保存六张本地 PNG / 一份 JSON，两种密度各含卡片、默认 Inspector、展开绑定后的精确输入。六帧旋钮标题 / 数字 / 单位 bounds 对齐验证通过，两帧精确输入边界通过，并已目视检查 Compact 展开 Inspector 与 Comfortable 卡片；全部明确为停止状态，不宣称实时 Tone 观测。截图先于最终非视觉队列修复，该修复由原生输入接受测试验证；生成物不入库。

本增量定向验证：18 项 Tone 相关测试、12 项字段级 service 意图测试、5 项 meter freshness 测试，以及 43 项 engine recorder 相关测试通过。Meter freshness 由已接收窗口持续时间推导，真实 topic 交付测试验证长窗口不提前过期、短窗口恢复后收紧期限。Recorder 提供无分配容量检查，候选验证与工厂共用 64 MiB f32 队列约束，256 声道上限为 65,536 帧。以上不是新一轮完整 workspace 验证，整体验收仍待剩余节点编辑完成。

## 独立 Mapper 精确工作站（2026-10-28）

卡片与 Inspector 使用独立输入实体和 4×4 分页，不再把合法权重限制在 ±1，也不因显示小块而重建完整矩阵。单格请求由 service 合并最新候选值，Identity / Swap 使用真实端口宽度；切页提交有效草稿一次，非法草稿保留但隐藏字段不占用键盘。成功加载、撤销/重做和布局变化会退休旧编辑器，预设/分页回调另有 owner lifetime 校验。Inspector 折叠使用实际共享展开状态，不在渲染阶段外查询 GPUI keyed state。

十二项原生接受测试通过。原生 32 声道场景完成两种密度、两种编辑表面的首/末页与预设，共 16 张截图；通过真正聚焦输入、Ctrl+A、输入与 Enter 将最后一格写为 −3.125，并验证完整矩阵及 active revision。另捕获四张双声道卡片/Inspector。Compact 双声道卡片为 440×256，32 声道为 516×382 逻辑像素；竖表随内容分别为 187.2 / 313.6 高。所有截图与 JSON 仅本地保留，未入 Git。

当前横向滚动保留自然通道列宽，末通道完整可见，但标尺会随内容移出且左边缘可能露出半个前导读数；这是已识别的后续打磨项，不隐瞒为完成。专用 16 帧均验证竖表边界；通用四帧只执行原有 bounds 检查，遗漏的竖表分类将在共用捕获验证器中修正后重拍。Mapper 交互与捕获源码由 `b5a05e6` / `741ad9d` 原子提交并推送至 PR #23。

## EQ / Gate 精确输入与视觉复核（2026-10-28）

EQ 三字段和 Gate 五字段在 Inspector 使用独立输入实体，复用卡片参数范围和单位。字段级意图合并最新候选状态，连续修改不再用旧设置覆盖相邻参数。EQ 曲线拖动原子提交频率 / 增益，保留最新 Q，并沿用现有 500ms 合并窗口形成 band 撤销项；Q 滚轮对最新候选值累乘，精确输入后立即滚动也不会跳回旧快照。待加载项目时阻止旧输入和 EQ plot 请求。旋钮端点返回精确范围端值，不放宽非法文本的校验。

EQ 14 项、Gate 7 项原生/参数测试通过；EQ 审查新增的七项中六项先失败后修复，一项已由卡片加载栅栏通过。18 项 service 字段测试、27 项 Rack 回归及 18 项 Tone 回归通过。每个节点分别捕获两种密度卡片 / Inspector 共四帧并目视复核；EQ 高 364、竖表高 295.2，Gate 高 324、竖表高 256 逻辑像素。Gate 截图发现 ms 被拆行，改用单行等宽单位后重新捕获通过，保持数值中心与旋钮对齐。

通用捕获器现在覆盖全部实际侧竖表节点，检查表高与 body 一致；边界仅允许 0.5px 的浮点/像素舍入，回归明确拒绝更大越界。EQ / Gate / stereo Mapper 的全部竖表检查均为 true。Mapper 额外 16 帧已重拍：固定标尺与 CLIP 标题不随通道滚动，末页不再露出被截断的第 28 声道读数，29～32 通道完整可读；自然通道宽度和滚动几何不变。纯几何三项回归通过，未更改 Mixer / compact meter。

以上由 `2511a8e` / `1b77844`（EQ）、`79718ba`（固定标尺）、`2d642e4`（Gate）、`c5921de`（单位单行）和 `91c40a8`（捕获验收）分别提交。截图 / JSON 仍未入库。Limiter / Delay、Meter / Recorder 精确输入与最终整体验证仍在进行，不把这些分段结果当作完整 workspace 终验。


## Limiter / Delay 精确参数验收（2026-10-28）

Limiter 和 Delay 的三个 Inspector 字段均与卡片分离输入实体，沿用相同参数域。字段级请求合并最新候选值，不覆盖其他参数；Delay 保留已有 `max_delay_ms`，反馈 ±99.9% 精确映射到引擎 ±0.999，Wet mix 仍以百分比显示。两个节点各五项定向测试通过，各自完成两种密度的卡片 / Inspector 四帧原生捕获，并分别目视检查紧凑 Inspector 与舒适卡片。短卡片保留紧凑横表，不为竖表而增加空白高度。

代码分别以 `7005ad3` / `8d9dbd9` 原子提交并推送。八张 PNG 与两份 JSON 仅保存在本地 `limiter-precision/`、`delay-precision/` 子目录。后续审查发现共享旋钮在快照未发布期间的连续滚轮、加载后的旧回调，以及跨表面重输原值的意图处理仍有缺陷；这些按独立回归修复，不把局部参数测试和截图作为整体交互完成证明。
