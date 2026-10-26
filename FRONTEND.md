# Moiren 前端设计与协作要求

本文件面向后续修改 Moiren UI 的开发者和设计会话，集中记录公共设计要求、资源入口、数据语义与协作约束。更新日期：2026-10-16。

**2026-10-14 UI 框架迁移完成：Slint 已完全移除，桌面 UI 改为 gpui-kit 0.7.3（纯 Rust）。** 下文的 Slint 组件/断点描述是历史设计参考；现行实现见 `crates/moiren-app/src/ui/`（theme.rs 令牌、widgets/ 组件、pages/ 页面、window.rs 壳）。设计令牌与配色语义已 1:1 移植。Slint 时代的验收方式作废，headless 测试用 `cargo test -p moiren-app --features gpui-test`。

**2026-10-15 应用框架接入：Graph、Mixer、Matrix、Devices 和 Settings 共用 gpui 组件与服务快照。** 页面按工作站需求重新组织，不直接复制 Figma。服务拥有候选 LogicalGraph，支持真实拓扑事务、编译启动、Gain / Balance / Mono / Swap、输入与输出观测，以及项目和偏好保存。当前音频驱动的运行范围仍是单 Source、单 Sink 的双声道图；多设备候选可以编辑，应用时会明确报告运行限制。具体接线与验证见 [应用前端接入](docs/designs/13-application-frontend-integration.md)。

使用时按以下顺序判断：用户最新明确要求 → 本文件中的已确认要求 → 当前任务指定的 Figma 设计 → 对应设计文档 → 现有试验实现。仓库工程约束仍以 [AGENTS.md](AGENTS.md) 为准。设计示例、历史验收记录和助手先前的完成声明不能代替当前验证。

### 2026-10-16 范围更正与实现边界

- 所有 UI 隐藏 revision、候选/活动图版本、请求编号、内部 Session 编号和节点/端口 ID。用户只需要节点名称、音频参数、连接关系和必要的执行结果；内部确认机制保留在服务和状态模型中。
- 节点外观以 v0.1.2 的 Input `556:11071`、DSP `556:11072`、Output `648:12412`、Bus `650:6644` 为准：270 × 112 px、6 px 实色分类顶条、28 px 类型图标、12/10 px 双行标题、双声道细电平与 4 px 轨道、8 px 边缘端口。普通节点不画外描边，选中使用克制的灰色描边。优先复用本地导出的设计图标；显示真实参数和电平，不照抄设计中的示例数值。
- 画布支持从输入或输出端口开始拖线，显示预览，在相反方向的有效端口松手后提交真实拓扑事务。Escape、空白处或画布外松手取消；Backspace/Delete 删除选中节点及关联连线，也可删除选中连线。输入框获得焦点时沿用文本编辑键，不触发图删除。
- 点击连线后使用独立高亮，Inspector 展示该连线的起点、终点和发送参数。平移画布（空白处拖动、中键、Hand 工具、触控板滚动）保留节点/连线选择及 Inspector 的开关状态；空白处单击在松手后才清除选择。
- 节点库参考 v0.1.2 Graph 页 `3:4` 及 v0.1.0 `3:5604` 的主体和页脚，提供分类、搜索、点击/拖放创建、当前项目节点列表。Appbar 保留已有带图标的导航，不照搬新版纯文字版本。本地 Tabler 图标可替代手绘图标，不依赖临时远端资源。
- **Inspector 展示当前选中节点或连线，不负责整条链路。整体不采用 v0.1.2 `3:4` 的 Inspector 设计，至多局部参考。** 节点使用独立的低对比属性面板：名称、资源绑定、DSP 参数、Channel Strip 控制、端口和删除操作。连线复用 Graph/Matrix 的属性面板，无选择时显示空态。
- Mixer 视觉参考 v0.1.0 `100:339`（实际 frame `109:330`）及 Studio One 的竖向 Strip。每个真实节点有独立推子、Mute/Solo，声道按钮循环 `N CH → MONO → 各声道 ONLY → SWAP`，不再只控制链路中的第一个 Gain。通道条使用方角外框；Inputs、Sends 标题按钮贴合边框并使用方角；M/S 与 Mode/Dim 组成紧凑的田字格。Dim 激活只改变状态样式，不改写 `DIM` 文本。
- Channel Strip 是逻辑节点的显式处理配置，不插入伪造的图节点。`ONLY` 将所选声道复制到所有输出以便监听；`MONO` 为所有声道平均；`SWAP` 反转声道顺序。Solo 保留所选节点的上游和下游路径，静音平行分支，不改写用户的发送音量或发送静音配置。
- Sends 对应实际出边，音量独立可调，单个 PRE/POST 按钮切换取样点。有 Strip 时 PRE 位于节点 DSP 之后、Strip 推子之前，仍受 Mute/Solo 和声道模式影响；POST 位于推子之后。没有 Strip 的底层 Gain 仍可在其输入取样，其他未定义取样点不能冒充支持。
- Strip 标题可拖动排序；Bus/Output 默认停靠右侧并独立横向滚动。Strip 宽度及右停靠区宽度可拖动分隔条调整；顺序和宽度随项目保存，关闭到托盘再打开也保留。不改变逻辑拓扑顺序。
- 电平使用各节点独立的真实推子后 Sample Peak/RMS、三色分区（低于 −6 dBFS、−6 至 0 dBFS、0 dBFS 以上）和逐声道 Clip 保持，内部浮点信号保留正值余量；点击电平区域复位 Clip。可见 Strip 才订阅对应节点，滚出视口、隐藏和停止后释放；未绑定、未运行或候选尚未确认时显示未知，不复制别的节点读数。Analysis 暂无处理器，节点库明确禁用，不绘制假 FFT。
- Strip 和 PRE/POST 修改保留为候选，通过 Apply graph 在控制侧准备并等待引擎确认，不能把点击或 Accepted 当成 Applied。现有 Gain/Balance 和发送音量的实时控制仍沿用服务协议；它们与 Strip 推子是不同参数。
- 本机验证串行执行 Cargo，统一使用 `-j 2`，测试附加 `-- --test-threads=2`，避免过量并行导致降频。

本文区分 3 类信息：公共要求包括功能保留、组件复用、语义变量和最新范围更正；目标设计规格包括导航、框架与密度方案；当前资源快照包括色值、字体、尺寸和节点 ID。后两类为设计与实现提供基准，可以随新设计更新，不应因写入本文而变成不可调整的要求。

## 1. 必须先记住的要求

- 设计应专业、简约、现代、紧凑，适合长时间音频路由与监听工作。丰富功能通过清晰层级表达，不靠增加装饰或堆叠卡片。
- 所有页面共享主题变量、组件、图标和状态语义。可配置的颜色、圆角、padding、间距、尺寸、描边、字体与透明度应进入公共设计系统。
- 相关内容使用自动布局（Auto Layout）。保持随内容、密度和窗口尺寸变化的能力，不能只在截图尺寸下成立。
- **该用按钮组件就用按钮组件。** 复用组件实例，不能用普通 Frame、Rectangle 和文字手绘一个外观相似的按钮。
- 先在组件库中新建设计 Frame，完成零件、组件与整体试样的验证，再替换现有组件及页面实例。
- 用户指定了局部范围，就只修改该范围。不要把「某个表格去圆角、去边框」扩大为整个页面、组件库 Section 或全局主题的样式清理。
- **Matrix 只有内部表格采用直角、无边框。外部容器、筛选、按钮、搜索框、摘要与 Inspector 保留各自的圆角和边框。**
- Mixer 采用已授权旧设计的竖向通道条、细轨推子、刻度和电平表语言；保留 Mono、Swap、Dim 等功能。底部 3 张目的地卡片可以取消。
- 不把未知实时数据画成 0，不把候选配置标成活动配置，不把 Accepted 当成 Applied，不把原型控件表示成已接入运行时。
- Figma 设计任务与源码实现任务分开。只要求改设计时，不顺手修改 Rust 或 Slint 源码。
- 每个小组件与有意义的设计块都要实际导出并目视检查，发现问题先修复，再组装下一层。
- 永远不自己绘制 Icon ，要么使用 Figma 自带的， 要么去 Iconify 下载 Apache/MIT 协议的开源图标（优先 Material 系列）

## 2. 产品定位与共享模型

Moiren 是 Windows 原生实时音频路由与监听工作站。现行实现是 Rust + gpui-kit；设计通过原生组件、布局和轻量绘制实现，不以浏览器技术、Electron 或 WebView 为前提。

Graph、Mixer、Matrix 展示同一份音频图：

| 视图 | 主要职责 |
| --- | --- |
| Graph | 节点、端口、拓扑和信号流，解释连接关系 |
| Mixer | 通道或节点的参数、增益结构、电平与监听控制 |
| Matrix | 大规模输出侧到输入侧的连接关系、选中交点和路由详情 |
| Devices | 设备、应用音源、可用性、能力和时钟资源 |
| Settings | 应用偏好、快捷键、引擎选项与诊断 |

共享的是图对象身份、Session、资源、参数、请求与确认状态。各视图的相机、排序、筛选、折叠和滚动属于展示状态，不应产生另一份拓扑权威。不能将「共享 LogicalGraph」解释成「只有 Graph 页面可以编辑」。多视图统一编辑的具体端口、边和事务设计见第 10 节的评估文档。

「仿专业 DAW」指 Mixer 的布局、控制精度与信息密度。当前产品范围不包含时间线、编曲、波形编辑、MIDI、自动化轨道或插件市场；Mixer 以真实 Inputs 取代 Insert 区域。

统一视图保留 NodeId、PortId、EdgeId 身份：Mixer Strip 展示现有节点及其显式处理配置；Matrix 表达直接连接，不混同间接可达关系；多个 Sink 不合并成特殊 Master。端口承载完整多声道流，输入最多连接 1 条边，输出可扇出，连接受 DAG 与声道数校验约束。PreFader 的具体边界见本文件 2026-10-16 更正，不暗示隐式混音或声道数转换。

## 3. Figma 与参考资源

### 3.1 工作文件与允许的参考范围

主工作文件为 [Moiren UI v0.1.2](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2)，file key 为 `dnmEP4We8cGHc0GK33mOeg`。

最初的独立重设计要求只读取 [Logo 参考节点](https://www.figma.com/design/axX8ZopKhTi9XfcfoLotiP/Moiren?node-id=20-613)，避免旧 UI 干扰。之后用户明确授权了特定旧设计的移植和参考。这个授权限于指定节点及理解其内容所需的子树，不代表可以扫描所有旧文件。

| 参考资源 | 范围与用途 |
| --- | --- |
| [Logo：20:613](https://www.figma.com/design/axX8ZopKhTi9XfcfoLotiP/Moiren-UI-v0.1.0?node-id=20-613) | 品牌来源；其余同文件内容没有自动获得参考授权 |
| [Graph v0.1.1：5:5](https://www.figma.com/design/U0TbolCFMgquGCzZhlCwiV/Moiren-UI-v0.1.1?node-id=5-5) | 已授权节点对比参考：保留有意义的颜色区分，解决内部 Port 占空间与几何风格不统一的问题 |
| [Graph v0.1.0：3:5661](https://www.figma.com/design/axX8ZopKhTi9XfcfoLotiP/Moiren-UI-v0.1.0?node-id=3-5661) | 已授权对比参考：改善信息不足、缺少图标与颜色区分缺乏语义的问题 |
| [Graph 手绘版本：5:977](https://www.figma.com/design/axX8ZopKhTi9XfcfoLotiP/Moiren-UI-v0.1.0?node-id=5-977) | 已授权参考 Meter 细节；节点整体仍需提升差异表达与易用性 |
| [用户修改的 Node Header：316:23](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=316-23) | 用户要求将该 Header 移植到其他新节点；后续任务先核对其当前内容与引用 |
| [旧 Matrix：27:15644](https://www.figma.com/design/7SkG0urUPleASo1WHZW2k9/Moiren?node-id=27-15644) | 已授权组件化移植、主题化并替换工作文件中的 Matrix |
| [旧 Mixer：27:14931](https://www.figma.com/design/7SkG0urUPleASo1WHZW2k9/Moiren?node-id=27-14931) | 已授权沿用布局与绘制语言，保留新设计的丰富控制功能 |
| [旧 Compact 布局：27:16749](https://www.figma.com/design/7SkG0urUPleASo1WHZW2k9/Moiren?node-id=27-16749) | 用户指定的紧凑排版参考，限于该节点与相应布局用途 |

本地品牌资产位于 [assets/logos/](assets/logos/)。直接复用 Moiren 标志及对应尺寸资源，不重画或以通用图标替代品牌。

### 3.2 文件结构

| 页面 | 内容 |
| --- | --- |
| `00 Foundations` | Logo、基础变量、文字样式与层级 |
| `01 Components` | 公共组件、变体、组件预览及 Slint 实现索引 |
| `02 Shell` | 应用框架 |
| `03 Graph` | Graph 页面 |
| `04 Mixer` | Mixer 页面 |
| `05 Matrix` | Matrix 页面 |
| `06 Devices` | Devices 页面 |
| `07 Settings` | Settings 页面 |
| `08 Prototype` | 交互原型、状态展示与 QA 组合 |

组件库按 `00 Icons`、`01 Navigation & Actions`、`02 Fields & Parameters`、`03 Feedback & Overlays`、`04 Shell & Panels`、`05 Monitoring`、`06 Graph`、`07 Mixer`、`08 Matrix`、`09 Devices & Diagnostics`、`10 Slint Implementation Index` 分区。

### 3.3 常用入口

下表是 2026-10-14 的只读核对结果。节点可能被移动、替换或删除；组件名称与所属页面用于重新定位，ID 不能当作永久 API。

| 资源 | 工作文件入口 |
| --- | --- |
| Logo 与品牌 | [00 Foundations / Brand：4:77](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=4-77) |
| 图标库 | [Library / 00 Icons：96:6034](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=96-6034) |
| 导航与按钮 | [Library / 01 Navigation & Actions：96:6035](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=96-6035) |
| 参数控件 | [Library / 02 Fields & Parameters：96:6036](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=96-6036) |
| Graph 组件 | [Library / 06 Graph：96:6040](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=96-6040) |
| Graph 大 / 小窗口 | [181:23](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=181-23) / [190:316](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=190-316) |
| Mixer v2 设计板 | [390:52](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=390-52) |
| Mixer v2 通道条 / Master | [411:12564](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=411-12564) / [408:23](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=408-23) |
| Mixer 大 / 小窗口 | [192:23](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=192-23) / [193:561](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=193-561) |
| Matrix 组件区 | [Library / 08 Matrix：96:6042](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=96-6042) |
| Matrix 内部路由表格 | [Matrix Routing Grid：356:23](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=356-23) |
| Matrix 总组件 | [Matrix Workspace：357:247](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=357-247) |
| Matrix 大 / 小窗口 | [195:23](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=195-23) / [196:251](https://www.figma.com/design/dnmEP4We8cGHc0GK33mOeg/Moiren-UI-v0.1.2?node-id=196-251) |

## 4. 公共布局与视觉语言

### 4.1 应用框架

- 顶部只有 1 层应用栏，目标高度为 44 px。左侧放 Logo、Project / Session 选择和保存状态；中部放工作区导航；右侧放引擎操作、Settings 和窗口按钮。
- Graph、Mixer、Matrix、Devices 是 4 个平级工作区。Settings 从窗口按钮左侧的齿轮进入，不是第 5 个平级页签。
- 导航使用文字与克制的紫色下划线，避免厚重按钮包围和额外导航栏。
- 底部 28 px 状态栏集中表达详细引擎信息，如格式、块大小、延迟、时钟、CPU、XRUN 和错误入口。顶部与底部不要重复堆放诊断信息。
- 页内工具放在对应内容区。页面控件不得再增加第 2 条全宽应用栏。
- Scene 选择放在 Project / Session 菜单中，不常驻占据应用栏。
- App Toolbar、Engine Status Bar 和窗口按钮在所有页面复用公共组件，不逐页重画。

### 4.2 密度与响应式

目标验收尺寸为 Comfortable 1440 × 900 和 Compact 1024 × 720。它们是同一套组件的变量模式与布局调整，不是 2 套独立组件库。

- 优先保证画布、通道条、表格等主要工作面。小窗口使用滚动、抽屉、折叠和文本优先级调整，不将所有控件等比缩小。
- 项目名称、设备名称应有约束宽度、换行或省略策略，不能裁掉一半字符。必要时通过 Tooltip 显示完整内容。
- Library、Inspector 和设备详情在 Compact 布局中使用抽屉；覆盖同一区域的抽屉避免同时遮挡主要内容。
- Mixer 通道区可横向滚动；Matrix 支持两轴滚动与冻结表头。
- 保持可用点击区域、键盘焦点和数值可读性。先隐藏低优先级说明，再考虑收起关键读数或状态。
- 世界坐标、窗口逻辑尺寸与截图物理像素分开处理；高 DPI 不应改变端口与连线的对应位置。

### 4.3 视觉层级

目标设计采用深色工作站：石墨与近黑工作面，紫色品牌交互，青色输入信号，蓝色输出，琥珀色候选与等待，红色错误与 Clip。Graph 节点另有分类色，见第 5 节。这些映射来自设计规格与当前变量；用户对 Graph 颜色的直接要求是区分有意义、统一且可读，没有规定整卡必须使用彩色背景，也没有规定连线按源端口着色或具体方向配色。

层级主要由亮度、间距、细边框与受控背景色建立。避免玻璃效果、大面积装饰渐变、过量发光和粗大胶囊控件。圆角与边框是否存在取决于组件职责；Matrix 的内部表格例外不能扩展为全局规则。

正文对比度达到 WCAG 2.1 AA；状态同时通过文本、图标或形状表达。动效短且服务于操作反馈与空间关系，不干扰电平、数值和长时间工作。

## 5. 主题、变量与排版

### 5.1 变量结构

Figma 的公共变量分为以下集合：

| 集合 | 用途 | 当前模式 |
| --- | --- | --- |
| `Moiren / Primitives` | 基础调色板，语义颜色的别名来源 | Value |
| `Moiren / Color` | 背景、文字、描边、状态、信号与节点分类 | Dark |
| `Moiren / Spacing` | padding 与间距 | Comfortable / Compact |
| `Moiren / Geometry` | 圆角与描边宽度 | Value |
| `Moiren / Size` | 控件、图标、工具栏与点击区域尺寸 | Comfortable / Compact |
| `Moiren / Alpha` | tint 透明度 | 单模式 |

当前 Figma 颜色集合只有 Dark，不能声称 Light 已完成。主题化要求组件引用语义变量并能统一调整；实现其他主题时同样遵守这些角色与对比度要求。

颜色从 Primitives 经语义别名进入组件。页面不要自行复制 HEX；Slint 重写应提供对应的 Theme token，并在公共组件中使用。下表 HEX 仅用于核对当前设计，不是要求到处写死。

### 5.2 当前颜色参考

| 角色 | Token 与 Dark 值 |
| --- | --- |
| 工作面 / 输入 | `bg/canvas`、`bg/input`：`#090b0f` |
| 工具栏 / 面板 / 浮层 | `bg/toolbar`、`bg/panel`、`bg/overlay`：`#11151b` |
| 弱面板 / 抬升控件 | `bg/panel-subtle`、`bg/raised`：`#1b2028` |
| 描边 | `border/subtle`：`#1b2028`；`border/default`：`#2a303a`；`border/strong`：`#3a424e`；`border/focus`：`#a985f0` |
| 文字 | `text/primary`：`#f6f8fa`；`text/secondary`：`#b4bcc8`；`text/muted`：`#8c96a6`；`text/disabled`：`#3a424e`；`text/on-accent`：`#ffffff` |
| 品牌 | `accent/primary`：`#7549c6`；`accent/hover`：`#8b64e1`；`accent/pressed`：`#6232a6`；`accent/muted`：`#35165f` |
| 信号 | `signal/input`：`#22d3ee`；`signal/output`：`#60a5fa`；`signal/active`：`#a985f0`；`signal/inactive`：`#3a424e` |
| 通用状态 | `status/info`：`#60a5fa`；`status/success`：`#22d3ee`；`status/warning`：`#fbbf24`；`status/danger`：`#f87171` |
| 业务状态 | `state/pending`：`#fbbf24`；`state/unavailable`：`#667080`；`state/prototype`：`#c6a9f7` |
| 电平 | `meter/safe`：`#22d3ee`；`meter/high`：`#fbbf24`；`meter/clip`：`#ef4444` |

Graph 分类有独立语义，不与 Selected、Pending 或实时电平混为一谈：

| 分类 | 主色 Token | 主色 | tint / chip 值 |
| --- | --- | --- | --- |
| Source | `node/source` | `#34d399` | `#065f46` / `#047857` |
| DSP | `node/dsp` | `#a985f0` | `#4a2281` / `#6232a6` |
| Sink | `node/sink` | `#fbbf24` | `#92400e` / `#b45309` |
| Analysis | `node/analysis` | `#22d3ee` | `#155e75` / `#0e7490` |
| Utility | `node/utility` | `#8c96a6` | `#3a424e` / `#667080` |

分类背景分别引用 `node/<category>/tint`、`node/<category>/chip`。透明度引用 `alpha/tint-10`、`alpha/tint-16`、`alpha/tint-25`、`alpha/tint-40`，分别为 0.10、0.16、0.25、0.40。不要用父级整体 opacity 代替完整的 Disabled / Unavailable 配色。

### 5.3 间距与尺寸

| Spacing Token | Comfortable / px | Compact / px |
| --- | --- | --- |
| `spacing/1` | 4 | 4 |
| `spacing/2` | 8 | 6 |
| `spacing/3` | 12 | 8 |
| `spacing/4` | 16 | 12 |
| `spacing/5` | 20 | 16 |
| `spacing/6` | 24 | 20 |
| `spacing/8` | 32 | 24 |
| `spacing/10` | 40 | 32 |
| `spacing/12` | 48 | 40 |

| Size Token | Comfortable / px | Compact / px |
| --- | --- | --- |
| `size/control-sm` | 28 | 28 |
| `size/control-md` | 32 | 30 |
| `size/control-lg` | 36 | 32 |
| `size/toolbar` | 44 | 44 |
| `size/statusbar` | 28 | 28 |
| `size/icon` | 16 | 16 |
| `size/panel-header` | 36 | 32 |
| `size/hit-target` | 32 | 32 |

Geometry：`radius/none = 0`、`radius/sm = 4`、`radius/md = 6`、`radius/lg = 8`、`radius/full = 999`；`stroke/thin = 1`、`stroke/focus = 2`，单位为 px。

这些是设计系统标尺，不意味着每个角色都使用同一圆角。新增几何参数先查可复用变量；没有合适角色时集中新增，不在多个零件内复制魔数。图形自身需要固定几何时，集中定义，并保持外层布局可适应内容。

### 5.4 字体与文字样式

当前 Figma 使用 Inter 与 Cascadia Mono。早期文档出现 Segoe UI Variable、JetBrains Mono 或旧配色，不能混合为新的统一规格。Windows 字体回退与实际渲染应在实现阶段验证。

| 样式 | 字体与字重 | 字号 / 行高，px |
| --- | --- | --- |
| `UI/Title` | Inter，600 | 20 / 26 |
| `UI/Heading` | Inter，600 | 15 / 20 |
| `UI/Body` | Inter，400 | 13 / 18 |
| `UI/Label` | Inter，500 | 12 / 16 |
| `UI/Label Small` | Inter，500 | 11 / 14 |
| `UI/Caption` | Inter，400 | 10 / 14 |
| `Mono/Metric` | Cascadia Mono，400 | 11 / 14 |
| `Mono/Metric Small` | Cascadia Mono，400 | 10 / 12 |
| `Mono/Value` | Cascadia Mono，600 | 12 / 16 |

数值使用等宽字体，避免电平、dB、Hz、revision 和 frame 更新时横向跳动。显示单位与正负号，并区分 `−∞`（实际静音或零幅度）与 `—`（未知）。移植旧设计时不机械保留过小字号；在 100% 下检查标签与刻度可读性。

## 6. 组件、图标与交互复用

### 6.1 公共组件

已有组件适用时使用实例和属性覆盖，保持与母版的关系。Figma 与 gpui 实现都应遵守这个原则。

| 需求 | Figma 组件 / 集合 | 当前 ID |
| --- | --- | --- |
| 主操作 / 危险操作 | `Button / Filled`，Primary / Danger | `49:238` |
| 次操作 / 轻操作 | `Button / Quiet`，Secondary / Ghost | `51:358` |
| 图标按钮 | `Icon Button / Standard` | `52:448` |
| 可选中图标按钮 | `Icon Button / Toggle` | `54:488` |
| 窗口操作 | `Window Button` | `56:512` |
| 文本 / 搜索 / 下拉 | `Text Field` / `Search Field` / `Select` | `64:806` / `64:929` / `65:864` |
| 数值与选择 | `Numeric Field` / `Checkbox` / `Switch` / `Segmented Control` | `68:1461` / `65:974` / `66:944` / `111:6447` |
| 框架 | `App Toolbar` / `Engine Status Bar` | `82:2929` / `82:3143` |
| 面板与抽屉 | `Dock Panel` / `Panel Header` / `Side Drawer` / `Bottom Drawer` | `81:2402` / `81:2250` / `81:2669` / `82:2670` |

标准按钮包括 Small / Medium / Large，以及 Default、Hover、Pressed、Focus、Disabled 等适用变体。选择状态、错误状态、Tooltip、Menu、Dialog、Toast、Empty / Loading / Error 等使用各自公共组件。

2026-10-14 首批源码实现：Filled / Quiet、Standard / Toggle Icon Button、Window Button、Workspace Tab 已移植为 gpui 公共组件，入口由 `moiren_app::ui` 导出。令牌位于 [theme/controls.rs](crates/moiren-app/src/ui/theme/controls.rs)，原生预览与状态边界见 [第一批 gpui 公共组件](docs/designs/12-gpui-ready-action-components.md)。其他组件仍按当前母版逐批实现。

- 「Show in Graph」使用 `Button / Quiet` 实例；「Apply on next start」使用 `Button / Filled` 实例。
- 替换时同时核对标签、图标、尺寸、对齐和回调。母版默认的 Plus 图标不能无条件保留到所有操作上；图标必须符合操作语义。
- 页面切换、浮层关闭和密度变化不能无故重置相机、选择、参数或滚动位置。
- 目标交互包括可见键盘焦点、Escape 关闭最上层浮层、参数键盘调整与重置；`Ctrl+1…4` 切换工作区，`Ctrl+,` 打开 Settings。关闭窗口、隐藏到托盘、显式退出应用的语义需清楚区分，不能让窗口关闭行为与音频生命周期脱节。重写时明确列出尚未实现的交互。

### 6.2 图标

先用 `Library / 00 Icons` 中的图标实例。节点类型、设备类型、监听操作与 Insert 等语义优先使用图标，不以 `IN`、`APP`、`FX`、`Σ` 等短文本充当图标。

图标不足时，用户允许使用 Iconify 或 Figma 内置图标。补充图标先成为库组件，再用于页面，统一基准尺寸、描边和变量配色。当前图标主要为 16 × 16 基准，节点和按钮中可按角色缩放。

常用图标：Input `42:69`、Output `43:70`、Application `44:71`、Clock `45:74`、Mute `36:60`、Dim `37:61`、Mono `38:63`、Swap `39:64`、Search `24:43`、Graph `9:25`、Effect `377:6057`。`Icon/Effect` 表达效果处理，适用于 Insert 等位置。

保留有意义的文字、数值和声道标识。按钮 `1` / `2`、`Mono` / `Swap` 等不因图标化而丢失功能解释。移除容器边框时尤其不能删除矢量图标自身的 strokes。

## 7. 各工作区的专项规则

### 7.1 Graph

- 保持简约专业，节点内部的参数、信号和状态具有清晰的信息密度。类别与状态各有表达，不复制旧卡片的视觉问题。
- 节点类型用图标与有意义的分类色区分。Source、DSP、Routing、Output、Analysis 等复用公共节点与端口组件。
- Port 不占用节点卡片内部正文区域；放在边缘并与连接端点对齐。
- 连线使用单线贝塞尔曲线，选中线具有独立高亮；静音和不可用保持可辨认，不展示内部图版本。
- Inspector 显示当前选中节点或连线的属性；连线展示起点、终点、声道数和发送参数，与 Matrix 共用连接属性面板。
- 相机与节点位置属于展示状态。Fit 使用实际节点边界；缩放保持锚点；节点拖动后端口和曲线同步，不改动拓扑。
- 支持 Select / Hand、Fit、缩放与 Minimap 的目标设计。拖入创建、端口连线、批量选择等涉及后端能力时，通过真实事务接入或明确标记原型。

当前公共 Graph 组件包含 `Graph Node / Source`、`Graph Node / DSP`、`Graph Node / Routing`、`Graph Node / Output`、`Graph Node / Analysis`、`Graph Port`、`Graph Cable`、`Canvas Toolbar`、`Minimap` 与 `Inspector Section`。

### 7.2 Mixer

2026-10-25 的实现以 Studio Pro 8 的信息布局为参考，沿用 Moiren 的扁平样式。纵向顺序为分类色条 → Inputs → Sends → 立体声 Balance → M/S → Mode/Dim → 增益读数 → 共用刻度的电平与推子 → 底部名称、类型与排序手柄。省略录制、反听、插件和自动化区域。

- Inputs 显示真实入边及目的端口，点击选择对应连接 Inspector；Source 的外部资源单独标记。Sends 显示所有真实出边，可启停、拖动电平、双击数值精确输入；最右侧 PRE/POST 始终可见。关闭发送保留增益，双击轨道复位至 0 dB。
- Inputs、Sends 的高度和折叠状态跨通道共享，列表内部滚动；不同路由数量不改变电平区域的对齐。标题按钮为贴边方角，顶部分类色条铺满通道宽度，不能被右侧宽度拖拽区遮短。默认宽 160 px，可在 136..280 px 调整。保留 Bus/Sink 右侧停靠、分隔调整、排序与持久化；适配 1440 × 900 和 1024 × 720。
- M/S 与 Mode/Dim 紧贴组成方角的田字形按钮组。立体声模式按 `ST → MONO → L → R → SWAP` 循环；Dim 为 −12 dB，激活后保留 `DIM` 文本。Balance 使用独立的中心定位控件，不复用通用数值 Slider；双击归中，仅立体声显示。该参数作用于 Strip，与显式 Balance 节点的 DSP 参数分别显示。
- 推子是平面长矩形手柄。纯 `FaderScale` 同时决定刻度、电平高度、手柄中心和拖动反算；主要锚点为 `+12 / +6 / 0 / −6 / −12 / −24 / −36 / −48 / −60 / −72 / −∞`，0 dB 位于距顶部 24% 处。低于 −72 dB 的有限值停留在有限下限并保留准确读数，最底端才写入零。推子单位为增益 dB，电平为 dBFS。
- 按实际声道数绘制 Sample Peak/RMS 与逐声道 Clip，不复制单声道读数。峰值即时上升，按幅度指数衰减，默认 20 dB/s、保持 500 ms；右键选择 `13.3 / 20 / 32 dB/s` 和 `Off / 0.5 / 1 / 2 s`，存入全局展示偏好。采用 Ardour 的 DAW 数字峰值下降速率惯例；Studio Pro 未公开精确下降算法，不声称完全复刻或满足 IEC PPM、True Peak、LUFS 标准。
- 原生动画帧驱动读取和衰减，跟随显示刷新率。隐式 UI 测量窗为 `max(1, floor(sample_rate / 240))`，显式 Meter 节点窗长保持原设置。存在订阅需求时，引擎泵和兼容桥分别使用 1 ms 轮询预算；无需求时恢复空闲节奏。峰值保持仅由实际收到的新窗续期，Latest 覆盖的未收到峰值无法恢复；较大音频回调会成批产出测量窗。
- 主 Mixer 的 Strip 参数继续通过 Apply graph 确认，候选修改显示 Pending。PRE 位于节点 DSP、声道模式、静音/独奏门控和 Balance 之后、增益与 Dim 之前；POST 位于增益与 Dim 之后。托盘沿用其原有自动应用策略。

实现细节、参考来源与验证要求见 [Mixer 推子改造计划](docs/superpowers/plans/2026-10-25-mixer-strip-redesign.md)。原 Figma v2 组件仅作为历史参考，不再定义当前通道顺序。

### 7.3 Matrix：范围必须准确

内部表格与外部 UI 分别遵守其规则：

| 区域 | 圆角 | 边框与层级 |
| --- | --- | --- |
| 路由格、表格内行列 Header、内部表格拼接 | 直角，`radius/none` | 无结构边框；靠填充亮度、文本、状态标记与选中背景区分 |
| 包围表格的页面或工作区容器 | 保留公共容器规格 | 保留适用的容器边框 |
| Filter Chip、Button、Search Field | 使用对应组件规格 | Default / Active / Focus 等状态保留 |
| Selection Summary、Routing Note、Inspector | 保留各自圆角 | 保留原本适用的边框与背景；不凭「恢复」给所有内部节点补同一描边 |
| 图标矢量 | 保留图形自身几何 | 图标的 strokes 不是容器边框，不能批量删除 |

不能遍历整个 `Library / 08 Matrix` 把所有 radius 和 strokes 清零；也不能在修复时给所有后代统一加圆角或边框。

- 路由表格的行、列与交点明确，冻结 Header，支持滚动与大矩阵。
- 已连接、未连接、自路由禁止、不可用等语义可识别。选中格在无边框风格下仍需清晰反馈。
- 连接值与源推子独立；单元格显示的 dB 不能冒充源的 Fader 值。
- 选择交点后通过 Inspector 解释连接身份、参数、映射和 Requested / Accepted / Applied。
- Filter / Search / Show in Graph / Apply 等动作复用组件，不能借紧凑样式取消标准交互状态。

当前表格 `Matrix Routing Grid` 为 `356:23`，Route Cell 集为 `352:33`；外部 Filter Chip `353:38`、Selection Summary `353:39`、Routing Note `353:47`、Filters `355:26`、Inspector Summary `93:5852`。此分组用于定位改动范围，不表示未来结构必须照搬。

### 7.4 Devices 与 Settings

Devices 区分 Inputs、Outputs、Applications、Clock；设备详情展示能力、身份、Default、Active、Next start、可用性和诊断。默认设备、活动设备与下一次 Start 的候选不是同一状态。设备消失后保留稳定身份与不可用原因；刷新失败保留最后一次成功数据。

Settings 负责 Audio Engine、Appearance、Shortcuts、Diagnostics 等应用偏好。关闭后回到原工作区并恢复选择、相机与面板状态。设备选择、路由与 Session 参数留在各自工作区，避免重复入口。

## 8. 数据、运行与工程契约

这些规则应在重写后继续满足，与现有页面的结构无关。

- 分开展示候选配置和当前 Active Session。运行 A 时选择下次启动的 B，不能把 A 的电平、身份或来源标签改成 B。
- 参数反馈包含 Requested → Accepted → Applied；拒绝或错误显示原因。Applied 由真实回执确认，可带 request、revision 与 effective frame，不能在本地拖动后立即假装生效。
- 无数据、无 publisher、等待完整窗口、不可用、停止、错误、Prototype 与真实读数分别展示。Unknown 使用 `—` 或状态说明。
- Peak、RMS、LUFS、相关度、CPU、延迟和 XRUN 各自需要真实来源。不能将已有 Source / Sink 观测伪装成每个处理节点的独立测量。
- 信号订阅跟随实际可见需求；切页、隐藏、最小化、滚动到完全不可见区域时释放相应 UI 订阅。恢复后按当前会话和订阅代际重新获取数据，防止旧缓存或旧 Session 读数串入。
- 页面局部错误在对应区域说明；不能因此清空其他仍有效的图、资源或读数。刷新失败保留最后成功结果，并标明其状态。
- UI 通过应用服务提交请求并消费快照、model 或信号。禁止在音频线程分配、回收或阻塞；图、缓冲区、信号绑定与旧执行计划的回收在控制侧完成。
- UI 展示状态与逻辑拓扑、执行计划、平台音频传输分离。绘制不拥有另一份业务状态。
- Slint model 通知、setter 和回调要遵守 Rust 借用与 UI 线程约束。实际资源身份、请求确认和订阅生命周期不能因重写被简化成不真实的占位逻辑。
- 设计中的示例数值可以用于明确标识的样例或 Preview，不能流入生产 UI 成为假观测。

## 9. 跨会话修改流程与验收

### 9.1 开始任务

1. 阅读本文件、[AGENTS.md](AGENTS.md) 和当前任务对应的设计入口。
2. 判断任务是 Figma 设计、组件库调整、源码实现还是技术验证。只做用户授权的阶段与范围。
3. 修改前查看目标、所属母版、变体和调用实例。源码任务先查工作区状态，保留用户已有改动。
4. 按节点与属性列出目标范围；需要修改全局 Theme 或共享母版时，先判断哪些页面会被影响。
5. 读取实时设计核对名称与 ID。禁止把临时状态账本、旧 node ID 或历史完成声明当作唯一依据。

### 9.2 设计与替换

1. 查找现有组件、图标、语义变量和文字样式，能复用就使用。
2. 在对应组件库分区先建新的设计 Frame，先形成整体结构，再制作与验证零件。
3. 为相关内容保留 Auto Layout，明确 FIXED / HUG / FILL、padding、间距和裁剪责任。
4. 配置适用变体与属性；组件说明包含用途、Slint 对应属性 / callback，以及 Runtime-ready / Prototype 边界。
5. 验证后替换实例，保存文字覆盖、图标、选择状态、尺寸、回调和内容语义。
6. 删除旧母版前确认引用已迁移，包括组件库 Review、页面、Prototype 和 QA 缩略图。仍有引用时保留并标明旧版，不制造孤立实例。
7. 整理设计板，保证完整内容不重叠、不越出画板、不因父级裁剪丢失按钮或脚注。

设计要求不能因为旧组件缺少变量或使用绝对定位而放宽。当前部分 Figma 组件仍是 `layoutMode=NONE`，部分数值未完整变量化；它们是后续对齐对象，不是已完成 Auto Layout 的证明。

### 9.3 视觉与结构检查

- 每个足够小的组件先截图检查，再检查组合和完整页面。
- 在 100% 下核对层级、对齐、密度、对比度、状态、长文本、数值更新和点击区域。
- 核对 1440 × 900 与 1024 × 720；多通道、较大 Matrix 与短窗口使用滚动，不挤坏控件。
- 检查 Selected、Pending、Unavailable、Disabled、Clip、空数据和错误状态。
- 查变量绑定与实际解析结果：背景、文字、描边、圆角、padding、间距和尺寸。只统计 SOLID 颜色不等于所有参数都已变量化。
- 查按钮、图标与其他可复用控件是否仍是实例，主组件是否存在，覆盖是否正确。
- 查所有裁剪祖先，而不只查有问题的子节点。子节点尺寸正确仍可能被默认 100 px 的父级 Frame 裁掉。
- 导出后实际查看图像。调用截图 API 成功或结构数据正确，不代表视觉验收完成。

### 9.4 Figma 操作经验

- 每次 `use_figma` 调用都显式切到目标页面；工具的 current page 可能重置。组合变体前确认节点与父级在同一页面。
- `getNodeByIdAsync` 允许返回 null；读取节点专属属性前按类型判断。TEXT 没有 `findAll`；COMPONENT 没有 `mainComponent`。
- 实例通过 `component.createInstance()` 创建。优先使用属性、变体或 swap；不要强行增删实例后代或改不可覆盖的相对位置。
- 修改文本前加载其实际字体并等待完成。字体 style 名从现有样式或字体列表读取，不猜测 `SemiBold` / `Semi Bold`。
- `setBoundVariableForPaint` 返回新 Paint，需要捕获并重新赋值；`boundVariables.color` 是带 `id` 的别名对象，不是数组。
- 复制 Paint 时显式保留别名和 opacity，再核对绑定与解析后的颜色。不要把历史「JSON 深拷贝必然破坏绑定」的诊断当成 API 规则。
- `resize()` 可能重置布局尺寸模式，先设几何再设最终 HUG / FILL / FIXED。文本换行与容器裁剪都要单独验证。
- 工具报错可能已经部分修改画布。先查受影响节点，再做定点恢复，不反复盲目重跑整段批处理。
- 若外部账本或文件操作被拒绝，不通过改路径、换工具或其他会话绕过权限。

### 9.5 源码验证

只写文档或只改 Figma 时，不需要编译整个 Rust workspace。实现阶段按变更范围验证，并提供真实应用截图；硬件音频验收与便携测试分开。

仓库常用命令：

```sh
cargo check --locked --workspace --all-targets
cargo build --locked --workspace
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo run --locked -p moiren-app
```

Slint 文件检查与原生 UI 验证可参考下面的历史接入文档，执行前核对当前工具、构建入口与测试用途。过去通过的命令与截图只证明当时的状态；涉及声源或实际设备的验收要明确其运行范围。

## 10. 资料索引与维护

### 10.1 设计资料

| 文档 | 使用方式 |
| --- | --- |
| [2026-10-12 UI 重设计规格](docs/superpowers/specs/2026-10-12-moiren-figma-ui-redesign-design.md) | 公共框架、工作区、状态、交互与密度设计；后续专项更正优先 |
| [2026-10-12 Figma 构建计划](docs/superpowers/plans/2026-10-12-moiren-figma-ui-redesign.md) | 历史实施顺序与验收目标，不当作当前完成状态 |
| [统一拓扑视图评估](docs/designs/11-unified-topology-views-evaluation.md) | Graph / Mixer / Matrix 共用图身份、节点 Slot、端口与多重边的架构建议；属于评估，不能描述为已实现功能 |
| [早期 UI 设计系统](docs/designs/07-app-ui-design-system.md) | 历史背景与参数来源，配色、字体与导航可能已被新要求取代 |
| [早期 Graph 规格](docs/designs/08-figma-graph-ui-spec.md) | 历史 Graph 组件和交互记录，按本文件与当前授权设计核对 |
| [Figma UI Kit 构建记录](docs/designs/09-figma-ui-kit-build-log.md) | 历史设计交付与实现索引 |
| [第一轮 UI 接入与验收](docs/designs/10-ui-round1-integration.md) | UI / 信号技术验证、数据语义和原生验收经验；不是重写时必须保留的布局 |

### 10.2 技术入口

当前源码入口如下；前文 Slint 组件与文件名只保留作历史设计背景。

| 路径 | 用途 |
| --- | --- |
| [src/ui/window.rs](crates/moiren-app/src/ui/window.rs) 与 [src/ui/window/](crates/moiren-app/src/ui/window/) | 公共 shell、各工作区、Settings、项目文件操作与 native capture |
| [src/ui/theme.rs](crates/moiren-app/src/ui/theme.rs) 与 [src/ui/theme/controls.rs](crates/moiren-app/src/ui/theme/controls.rs) | 共享语义令牌、字体与密度 |
| [src/ui/widgets/](crates/moiren-app/src/ui/widgets/) | 按钮、导航、图标与真实电平组件 |
| [src/ui/state.rs](crates/moiren-app/src/ui/state.rs) | 服务快照投影、候选资源、请求与确认反馈 |
| [src/ui/monitoring.rs](crates/moiren-app/src/ui/monitoring.rs) | 按窗口、页面、滚动可见性管理独立输入/输出订阅 |
| [src/service/graph.rs](crates/moiren-app/src/service/graph.rs) | 真实 LogicalGraph 权威、事务、候选与活动 revision |
| [src/project.rs](crates/moiren-app/src/project.rs) | 项目与偏好 JSON，原子保存、版本与结构校验 |
| [README](crates/moiren-app/README.md) | 运行、实际交互与验证命令 |

未接入的 CPU、端到端延迟、XRUN 等读数保持未知。Compressor 的全部 10 项参数在所选节点 Inspector 中提供精确输入和滑杆，按引擎参数域校验、持久化，并沿实时参数确认通道更新；时间与比率滑杆采用非线性映射以保留小值精度。切换拓扑通过 Apply / restart 准备新会话。载入项目不启动音频，且候选节点不能因 NodeId 数值复用而控制旧活动会话。

### 10.3 后续维护

- 新的公共要求、参考授权、状态语义或用户更正，直接更新对应章节，移除冲突表述；不要在文末无限追加聊天记录。
- 新增或替换公共组件时更新名称与入口，注明仍使用旧母版的范围。
- 尚未确认的方向标为方案或评估，不混入强制要求。Figma 设计状态与运行时能力分别记录。
- 临时截图 URL、用户机器的 Temp 路径、工具失败记录与批处理次数不作为跨会话依赖。
- 本文只整理公共前端约束。具体任务的计划、实验结果与验收证据进入对应设计或实现文档。
