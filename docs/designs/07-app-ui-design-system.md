# Moiren 应用 UI 设计规格（4 页面 · 双尺寸）

> 状态：已废弃 — Slint 已于 2026-10-14 移除；当前 UI 设计与实现见 FRONTEND.md（gpui）。

> 本文是「把 Figma Graph 视觉语言落到 4 个应用页面」的实现基准：
> 颜色、字号、间距、组件、外壳几何、每页布局都给到可直接编码的数值。
>
> 取证来源（可复现）：
> - `designs/Moiren.simplified.fig` / `designs/Moiren.fig`（两份配色统计一致，23 色相同）
> - 页帧：`0:4` Control language、`0:281` Device manager、`0:687` Rack editor、
>   `0:1163` Matrix、`0:1662` Mixer、`0:2453` Graph
> - 命令：`bunx @open-pencil/cli analyze colors|typography|spacing <fig>`、
>   `export --node <id> -f jsx`，导出样本在 `.tmp/*.jsx`（`cl/devices/graph/mixer/matrix/rack`）
> - 注意：`export -f png` 在本机 headless 下因 canvaskit 路径问题失败；
>   视觉核对使用已存在的 `.tmp/design-review/figma-page.png` 与 `.tmp/design-review/figma-routing-canvas.png`。
>
> 本次交付只新增文档，不改动任何 `.slint` 源文件。

---

## 0. 结论摘要（先看这 8 条）

1. **深色为本设计的唯一基准**；Figma 未定义浅色，浅色只能作为推导值。
2. 中性色是"近黑 + 灰绿"，强调色是 **mint `#83DFC4`**（201 次使用，覆盖
   Selected / Running / Connected / 激活页签 / 品牌 mark 描边）。
3. 信号色只有 5 个角色：mint=broadcast/live、blue=monitoring、
   violet=music/bus、amber=attention/render、coral=overload/destructive。
4. 字体是 **Inter（界面）+ JetBrains Mono（数值）**；数值一律 mono，避免跳变抖动。
5. 间距以 **8px 为节奏**，但控件内部允许 2/3/4/5/6/7 这类微距（Figma 也这么用）。
6. 圆角分 4 档：控件 2、节点/内块 4、卡片/面板 6、旋钮 14；描边统一 1px。
7. 密度是"仪表盘密度"：按钮高 30、字段高 32、表行 66、面板内边距 16、工作区 20。
8. 双尺寸靠 **同一份描述 + 结构分支**实现（`Theme.compact`），不是两套重复布局。

---

## 1. 设计令牌

### 1.1 颜色（深色，Figma 实测）

| Token | 值 | Figma 用量 | 用途 |
| --- | --- | --- | --- |
| `bg-canvas` | `#111518` | 62 | 画布 / 窗口底 / 面板之间的空处 |
| `bg-chrome` | `#14191D` | 121 | 标题栏、工具栏、工作区工具栏、状态栏、表头 |
| `bg-panel` | `#191E22` | 299 | 卡片、面板、节点卡、设备行（隔行） |
| `bg-control` | `#21282D` | 86 | 中性按钮、输入框容器、激活页签、菜单 |
| `bg-control-hover` | `#2A3238` | — | 中性控件 hover（由 control 推导） |
| `bg-field` | `#14191D` | 121 | 字段/搜索/轨道/内嵌区块（与 chrome 同值，语义不同） |
| `border` | `#232B30` | 63 | 细分割线、hover 边界 |
| `border-strong` | `#30393F` | 302 | 卡片描边、表格描边、分隔线（主用） |
| `text` | `#E5ECEF` | 318 | 主文本、节点名、数值 |
| `text-muted` | `#A5B2BA` | 415 | 次要文本、参数名、未激活页签 |
| `text-faint` | `#6F818D` | 425 | eyebrow、单位、辅助说明、禁用 |
| `accent-live`（**主强调**） | `#83DFC4` | 201 | 激活/选中/运行中/已连接、主按钮、mark |
| `accent-live-soft` | `#223F39` | 26 | 强调底（激活页签、选中行、选中 chip） |
| `signal-device` | `#86B9EC` | 68 | 采集/monitoring/设备、channel strip 顶条 |
| `signal-music` | `#B1A0DF` | 34 | music/内部 bus 来源 |
| `signal-attention` | `#E7BC79` | 17 | 录制/离线/需注意 |
| `signal-overload` | `#EB8D87` | 4 | 过载、断开连接等破坏性动作 |
| `overlay` | `#000000` @ 58% | — | 菜单/弹层遮罩（叠加在画布） |

补充规则（来自 `0:4` 的 footer 与 specimen）：

- 语义标注原文：`Signal palette: mint / broadcast · blue / monitoring · violet / music ·
  amber / attention · coral / overload`。
- 状态四态：Running=`#83DFC4`、Offline=`#E7BC79`、Clipping=`#EB8D87`、Disabled=`#6F818D`。
- 电平表：常态用所属通道的信号色；过载才转 coral。**不要用绿色表示"健康"**，
  本设计里"健康/在跑"= mint，"搬运信号"= blue。
- 浅色方案 Figma 未定义 → 见 §6/D3，默认以深色验收。

### 1.2 排版

家族：`Inter`（界面）/ `JetBrains Mono`（所有数值与机器可读字符串）。

| 角色 | 家族 | 字号/行高 | 字重 | 颜色 | 备注 |
| --- | --- | --- | --- | --- | --- |
| 页面标题（大图） | Inter | 20 / 28 | 600 | text | 每页工作区工具栏 |
| 页面标题（小图） | Inter | 16 / 22 | 600 | text | 同上 |
| 页面副标题 | Inter | 12 / 17（小图 11/15） | 400 | text-faint | "Live studio / …" |
| 卡片标题 | Inter | 12 / 17 | 600 | text | 卡片内一级标题 |
| Eyebrow / 分组标签 | Inter | 10 / 14 | 400 | text-faint | 全大写，`letter-spacing 0.6` |
| Inspector 段标题 | Inter | 11 / 15 | 600 | text | 全大写，`letter-spacing 0.6` |
| 表头 | Inter | 8 / 11 | 400 | text-faint | 全大写 |
| 正文 | Inter | 12 / 17 | 400 | text-muted | 参数名、说明 |
| 小字 | Inter | 11 / 15、10 / 14 | 400 | text-muted/faint | 行内辅助 |
| 微字 | Inter | 9 / 13、8 / 11 | 400 | text-faint | 单位、通道标签、快捷键 |
| 大数值 | Mono | 18 / 25 | 400 | mint 或 text | 健康卡值 |
| 数值（行内） | Mono | 12 / 17 | 400 | text | 属性值、字段值 |
| 指标 | Mono | 11 / 15、10 / 14 | 400 | text-muted | 引擎指标、连接标注 |
| 微指标 | Mono | 9 / 13、8 / 11 | 400 | text-faint | 峰值、刻度、脚注 |
| 按钮标签 | Inter | 12 / 17 | 500 | text 或 accent | 选中态用 accent |

实测佐证（`analyze typography`）：Inter 10/14 用了 149 次、Mono 10/14 用了 100 次、
Inter 9/13 用了 90 次、Mono 12/17 用了 84 次 —— 主体就是 9–12px 的高密度小字。

### 1.3 间距

- 节奏：**8px**；`4/8/12/16/20/24/32/48` 是主序列。
- 控件内部微距允许：`2 / 3 / 4 / 5 / 6 / 7`（Figma 中 7px 用了 91 次、3px 84 次）。
- 常用内边距（Figma 实测）：面板/卡片 `16`、列表行 `12`、输入 `10`、工作区 `20`、
  单元格 `14`、弹层 `6`。
- 垂直间距：卡片内部 `gap 10–14`；卡片之间 `block-gap`（大图 20 / 小图 16）。
- 外壳留白：`gutter` 大图 28 / 小图 20（仅用于工作区左右呼吸位）。

### 1.4 圆角 / 描边 / 控件几何

| 项 | 值 |
| --- | --- |
| 控件、字段、按钮、chip 容器圆角 | 2 |
| 菜单项圆角 | 3 |
| 节点卡、内嵌区块、页签、小卡 | 4 |
| 卡片、面板、菜单、健康卡 | 6 |
| 旋钮外圈 | 14（直径 42） |
| 药丸 | 999 |
| 描边 | 全部 1px（选中强调用 0.5px 的 accent 描边叠加 soft 底） |
| 标题栏高 | 32 |
| 工具栏高 | 62（大图）/ 56（小图） |
| 工作区工具栏高 | 48 |
| 状态栏高 | 28（小图 26） |
| 按钮高 | 30（首页主行动 52） |
| 字段高 | 32；选择器/搜索高 30 |
| 页签 | 30，容器 36（`padding 3`） |
| 表头高 / 表行高 | 34 / 66 |
| Inspector 段 | `padding-y 12 / padding-x 16`，`gap 10` |
| 横向电平条高 | 节点内 4；Inspector 5 |
| 竖向电平条 | 9 × 180，分度线 1.5px |
| 端口圆点 | 8（描边 1.5） |
| 节点卡 | 宽 156–215、高 124、圆角 4；类型标记 3×13 |
| 推子 | 45 × 205（轨道 3×189、手柄 31×14 r3） |
| 旋钮 | 42（区间环 38 描边 2、盘面 28、指针 2×8） |
| 状态点 | 6 |
| 窗口控制图标 | 12 |
| 列表图标 | 16；设备行图标 19；节点卡图标 24 |

### 1.5 动效

Figma 未定义动效。沿用工程现值，不要放大：`fast 120ms`（hover/背景）、
`normal 180ms`（面板展开）、`slow 280ms`（页面切换）。只对 `background`、
`border-color`、`height` 做过渡；**电平条不做动画**（实时值直通）。

### 1.6 Slint 令牌映射

`theme.slint` 的 `Theme` global 保持"唯一真源"，本次需要补齐/改名如下：

| 现有 token | 目标 |
| --- | --- |
| `window-bg` | `#111518`（已是） |
| `surface`/`titlebar`/`sidebar` | 统一到 `#14191D`（chrome） |
| `surface-2` | `#191E22`（panel） |
| `surface-3` | `#21282D`（control） |
| `surface-4` | `#2A3238`（control-hover） |
| `border` / `border-strong` | `#232B30` / `#30393F` |
| `text` / `text-muted` / `text-faint` | `#E5ECEF` / `#A5B2BA` / `#6F818D` |
| 新增 `accent-live` / `accent-live-soft` | `#83DFC4` / `#223F39` |
| 新增 `signal-device` / `signal-music` / `signal-attention` / `signal-overload` | `#86B9EC` / `#B1A0DF` / `#E7BC79` / `#EB8D87` |
| `radius-sm/md/lg/xl/pill` | 收敛为 `radius-control 2` / `radius-node 4` / `radius-card 6` / `radius-dial 14` / `radius-pill 999` |
| 新增几何 token | `titlebar-height 32`、`toolbar-height 62/56`、`workspacetoolbar-height 48`、`statusbar-height 28/26`、`control-height 30`、`field-height 32`、`table-row-height 66`、`table-head-height 34` |
| `font-ui` / `font-mono` | `"Inter"` / `"JetBrains Mono"`（见 D2 的回退） |

> 保留"一个 token 承载两档值 + 页面只按 `Theme.compact` 分支结构"的现有写法，
> 这样 4 个页面各只有一份描述。

---

## 2. 应用外壳

Figma 帧是 1600×1000（32 标题栏 + 62 工具栏 + 878 面板 + 28 状态栏）。
应用目标是 1600×956 与 1180×760，所以外壳按下面的算术压缩。

### 2.1 大图 1600×956

```
┌───────────────────────────────────────────────────────────────┐ 32  标题栏  #14191D
│ ◇moiren  Live studio.mrn              –   □   ×               │
├───────────────────────────────────────────────────────────────┤ 62  工具栏  #191E22
│ ▸ Live studio          [ 场景 01 Broadcast ]   ● Engine   Stop │
├──────────┬────────────────────────────────────────────────────┤
│ 导航      │  工作区工具栏 48  #14191D                           │ 834
│ 232      │ ──────────────────────────────────────────────────  │
│ #191E22  │                                                     │
│          │  页面内容（padding 20，block-gap 20）                │
│ 监控路由  │                                                     │
│ 测试音    │                                                     │
│ 音频设备  │                                                     │
│ 设置      │                                                     │
├──────────┴────────────────────────────────────────────────────┤ 28  状态栏  #14191D
│ ● Audio stable  48 kHz / 24-bit  128 samples · 2.67 ms  Clock… │
└───────────────────────────────────────────────────────────────┘
```

算术：`956 − 32 − 62 − 28 = 834`（body）；`1600 − 232 − 1 = 1367`（内容区）；
`1367 − 2×20 = 1327`（可用宽）。两列时 `1327 − 20 = 1307` → 配置列 784 / 实时列 523。

### 2.2 小图 1180×760

```
┌───────────────────────────────────────────────┐ 32  标题栏
│ ◇moiren  Live studio.mrn      –  □  ×        │
├───────────────────────────────────────────────┤ 56  工具栏
│ ▸ Live studio      [01 Broadcast ▾]  ●  Stop  │
├────────┬──────────────────────────────────────┤
│ 导航    │  工作区工具栏 48                      │ 646
│ 208    │ ───────────────────────────────────  │
│        │  页面内容（padding 16，block-gap 16）  │
├────────┴──────────────────────────────────────┤ 26  状态栏
│ ● Audio stable  48 kHz / 24-bit  128 samples  │
└───────────────────────────────────────────────┘
```

算术：`760 − 32 − 56 − 26 = 646`；`1180 − 208 − 1 = 971`；`971 − 2×16 = 939`。
**小图所有页面都用单列**，需要"右侧实时面板"的页面把该面板改为内容末尾的一段。

### 2.3 外壳组件与测量

**标题栏（32）** `#14191D`，`padding-left 16 / padding-right 18`

| 元素 | 规格 |
| --- | --- |
| 品牌 mark | 20×16，mint 描边 1.3px（用 `assets/logos/` 的 mark 变体） |
| "moiren" | Inter 12/17 600，`text` |
| 文档名 | Inter 10/14，`text-faint`（"Live studio.mrn"） |
| 拖拽区 | 中间 `WindowMoveArea` 弹性填充 |
| 窗口控制 | 3 个 12px 图标，间距 30，图标色 `text-muted`；关闭 hover 底 `signal-overload` |

**工具栏（62 / 56）** `#191E22`，下描边 1px `border-strong`，`padding-x 20`，`gap 22`

| 区域 | 规格 |
| --- | --- |
| 左：项目 | folder 图标 16 + 名称 Inter 13/18 600 + 副行 Inter 10/14 faint + chevron 12 + save 15 |
| 中：场景选择 | 容器 36、`padding 3`、r6、底 `#14191D`；内含激活按钮 122×30（r2、底 `#21282D`、描边 `border-strong`）+ 2 个 10px 纯文本场景名 |
| 右：引擎传输 | 状态点 6 + 阶段文本 Mono 11/15（运行时 mint）+ Stop 按钮 70×30 |
| 小图差异 | 项目副行隐藏；名称 12/600；场景收缩为单个 30 高选择器；阶段文本 Mono 10/14；Stop 64×28 |

**导航栏（232 / 208）** `#191E22`，右描边 1px `border-strong`

| 元素 | 规格 |
| --- | --- |
| 头部 48 | "导航" Inter 12/17 600 + 收起图标 14。`padding-x 16` |
| 分组标签 | Inter 9/13 600 全大写 `text-faint`，其下 gap 13 |
| 导航项 | 高 58（小图 54），`padding-x 12`，`gap 10`，r4。图标 16 + 主标签 Inter 11/15 + 副标签 Inter 9/13 faint |
| 选中态 | 底 `#223F39`、主标签 `text`、图标 mint、左侧 3px mint 标记（r1） |
| hover | 底 `#21282D` |

导航项内容（4 项，副行即"这一页干什么"）：

| 页面 | 图标（Tabler） | 主标签 | 副标签 |
| --- | --- | --- | --- |
| 0 监控路由 | `route` | 监控路由 | 输入到输出的实时转送 |
| 1 测试音 | `wave` | 测试音 | 正弦信号与校准 |
| 2 音频设备 | `devices` | 音频设备 | 硬件与端点目录 |
| 3 设置 | `settings` | 设置 | 应用偏好 |

**工作区工具栏（48）** `#14191D`，下描边 1px `border-strong`，`padding-x 20`

| 区域 | 规格 |
| --- | --- |
| 左 | 页面名 Inter 12/17 600 + 分隔 + 上下文 Inter 10/14 `text-faint` |
| 右 | 页级状态 Mono 10/14 `text-muted` + 页级动作按钮（30 高） |

**状态栏（28 / 26）** `#14191D`，`padding-x 16`

| 区域 | 规格 |
| --- | --- |
| 左指标组 | 状态点 6 + "Audio stable" Mono 10/14 mint + 采样率/位深、缓冲/延迟、时钟源 三项 Mono 10/14 `text-muted`，`gap 23` |
| 右性能组 | Mono 10/14 `text-faint`；小图隐藏 |
| 未知值 | 一律 `—`（em dash）。不给"假数据" |

---

## 3. 可复用组件清单

| Figma 组件 | Slint 组件 | 关键属性 | 现状 |
| --- | --- | --- | --- |
| Window title bar | `TitleBar` | brand、doc、窗口回调 | 新增（拆出） |
| View selector / View tab | `AppTab` + `AppTabBar` | `active`、`label` | 现有 `Segmented` 可改名复用 |
| Action（按态） | `AppButton` | `tone: accent \| neutral \| danger \| ghost`、`icon`、`enabled` | 已有，tone 需对齐 |
| Field（值+单位） | `AppField` | `value`(mono)、`unit`、`enabled` | 新增 |
| Search field | `AppSearch` | `placeholder`、`shortcut` | 新增 |
| Table heading / Device row | `AppTableHead` / `AppTableRow` | `columns`、`selected` | 新增 |
| Audio node card | `RouteNodeCard` | `icon`、`name`、`type`、`key-param`、`in/out-channels`、`level` | 现有 `RouteNode` 升级 |
| Port | `PortDot` | `connected`、`accent` | 新增 |
| Cable / connection | `RouteCable` | `selected`、`muted`、`accent` | 新增（`Path`） |
| Level meter（横向） | `LevelMeter` | `left/right/peak/available` | 已有 |
| Level meter（竖向） | `StripMeter` | `level`、`peak`、`accent` | 现有 `MeterChannel` 抽公开 |
| Fader | `AppFader` | `value`、`db-scale`、`bipolar` | 新增（纵向） |
| Rotary parameter | `AppDial` | `value`、`label`、`readout`、`bipolar` | 新增 |
| Property row | `KeyValue` | `label`、`value`、`value-color` | 已有 |
| Inspector section（可折叠） | `InspectorSection` | `title`、`collapsed` | 新增 |
| Status（点+文本） | `StatusChip` | `tone`、`label`、`dot` | 现有 `Badge` 增加 tone |
| Menu / Menu item | `AppMenu` / `AppMenuItem` | `items`、`destructive` | 新增 |
| Card | `Card` | `raised` | 已有 |
| Section label / eyebrow | `SectionLabel` | `text`、`uppercase` | 已有 |
| Stat tile | `StatTile` | `label`、`value`、`unit`、`accent` | 已有 |
| Health tile（大数值卡） | `HealthTile` | `eyebrow`、`value`、`sub`、`accent` | 新增 |

**按钮四态（精确取值）**

| tone | 底 | 描边 | 文本/图标 |
| --- | --- | --- | --- |
| accent（选中） | `#223F39` | `#83DFC4` 0.5px | `#83DFC4` |
| neutral | `#21282D` | `#30393F` 1px | `#E5ECEF`（图标 `#A5B2BA`） |
| danger | `#21282D` | `#30393F` 1px | `#EB8D87` |
| ghost（无底） | 透明 | 透明 | `#A5B2BA` |
| disabled | `#14191D` | `#30393F` | `#6F818D` |

---

## 4. 页面设计

### 4.0 通用页面骨架（4 页共用）

```
工作区工具栏(48)  ← 页面名 + 上下文 + 页级动作
─────────────────────────────────────────────
页面内容 padding 20/16, gap 20/16
  大图：内容列 + 实时/详情列（gap 20）
  小图：单列堆叠
```

每页都要覆盖的状态：`idle / starting / running / stopping / failed / exiting / exited`
（4 页读同一个 `Backend.phase-text`，颜色 mint=运行、amber=启动/停止中、coral=失败）。

### 4.1 监控路由（`pages/monitor.slint`）

**核心内容元素**

| # | 元素 | 内容 | 数据来源 |
| --- | --- | --- | --- |
| 1 | 页头 | "监控路由" + "Live studio / 实时监听与转发" | 静态 |
| 2 | 阶段状态 | 点 + `Backend.phase-text` | `Backend.phase` |
| 3 | 路由图卡 | 源节点卡 → 连线（中点标注"增益 x% · 声像 L/R"）→ 目标节点卡 | `source-mode`/`input-index`/`process-index`/`output-index`/`gain`/`pan` |
| 4 | 来源卡 | Segmented「物理设备 / 应用程序」+ 选择器 | `Backend.input-options` / `process-options` |
| 5 | 目标卡 | 输出选择器 + 端点格式行（采样率/通道/格式支持） | `Backend.outputs[i]` |
| 6 | 处理卡 | 增益推子 + 声像旋钮（Balance，居中）+ 时长 + 循环开关 | `gain`、`pan`、`duration`、`loop-forever` |
| 7 | 实时卡（右列） | 峰值/RMS（Mono 12）+ 电平条 5px + 2×2 统计 + 主按钮 52 | 电平在 C4 前显示 `—` |
| 8 | 统计项 | 已运行 / 延迟 / 欠载(xrun) / 时钟漂移 | `Backend.elapsed` 等，未知 `—` |

**大图 1600×956（两列 784 / 523）**

```
┌ 工作区工具栏 ────────────────────────────────┐
│ 监控路由   Live studio / 实时监听与转发   ● 运行中 │
├──────────────────────────────┬───────────────┤
│ 路由图卡                       │ 实时监视        │
│  ┌────┐   ╭─增益·声像─╮  ┌────┐ │ ● 运行中        │
│  │源卡│───┴──────────┴──│目标│ │ 峰值 −12.3 dBFS │
│  └────┘                  └────┘ │ ▬▬▬▬▬▬▬▬░░░░░  │
│ 来源卡   [物理设备|应用] ▾…    │ RMS  −21.6 dBFS │
│ 目标卡   ▾ Speakers …          │ 已运行 00:12:48 │
│ 处理卡   增益 ▬▬▬●▬ / 声像 ◎   │ 延迟  0.00 ms   │
│          时长 60s / 循环 ●      │ 欠载 — / 漂移 — │
│                                │ [ 开始监控 52 ]  │
└──────────────────────────────┴───────────────┘
```

**小图 1180×760（单列）**

```
工作区工具栏(48)
路由图卡（源 → 目标，横向压缩到 939 内）
来源卡（Segmented + 选择器）
目标卡（选择器）
处理卡（增益推子 + 声像旋钮 + 时长 + 循环）
实时卡（阶段 + 峰值/RMS + 电平条 + 2×2 统计 + 主按钮）
```

结构分支：`if Theme.compact: 单列 else: 两列`；
旋钮在大图右列与会话同级、在小图紧跟推子。

**Slint 骨架**

```slint
export component MonitorPage inherits VerticalLayout {
    spacing: 0;                       // 工具栏与内容各自管间距
    WorkspaceToolbar { title: "监控路由"; context: "Live studio / 实时监听与转发";
        StatusChip { tone: Backend.active ? accent : faint; label: Backend.phase-text; } }
    ScrollView { VerticalLayout {
        padding: Theme.compact ? Theme.space-4 : 20px;
        spacing: Theme.block-gap;
        if Theme.compact {
            VerticalLayout { spacing: Theme.block-gap;
                RouteCanvas { } SourceCard { } OutputCard { } ProcessCard { } LiveCard { } }
        } else {
            HorizontalLayout { spacing: 20px;
                VerticalLayout { horizontal-stretch: 3; spacing: 20px;
                    RouteCanvas { } SourceCard { } OutputCard { } ProcessCard { } }
                LiveCard { horizontal-stretch: 2; } }
        }
    } }
}
```

### 4.2 测试音（`pages/render.slint`）

**核心内容元素**

| # | 元素 | 内容 | 数据来源 |
| --- | --- | --- | --- |
| 1 | 页头 | "测试音" + "Live studio / 正弦信号与校准" | 静态 |
| 2 | 阶段状态 | 同 4.1 | `Backend.phase` |
| 3 | 目标输出卡 | 输出选择器 + 端点格式行 | `Backend.outputs` |
| 4 | 频率卡 | 大数值字段（Mono，值+`Hz`）+ 4 个预设 chip（100 / 440 / 1k / 2k）+ 对数滑杆（带刻度） | `frequency` |
| 5 | 电平卡 | 增益推子（dB 刻度 +6/0/−12/−24/−∞）+ 声像旋钮（Balance，"Center" 可双击复位） | `gain`、`pan` |
| 6 | 时长卡 | chip 10s / 30s / 60s / ∞ + 循环开关 | `duration` |
| 7 | 实时卡（右列） | 输出电平（L/R）+ 主按钮 52（播放/停止）+ 提示"预留安全电平" | 电平 `—` 直到 C4 |

**大图（两列 784 / 523）**

```
┌ 工作区工具栏 ─────────────────────────────────┐
│ 测试音   Live studio / 正弦信号与校准    ● 空闲  │
├──────────────────────────────┬────────────────┤
│ 目标输出  ▾ Speakers …         │ 输出电平        │
│ 频率      220 Hz               │  L ▬▬▬▬░░░░     │
│           [100][440][1k][2k]   │  R ▬▬▬▬░░░░     │
│           ▬▬▬●▬▬▬▬▬ (对数)      │ 频率 220 Hz     │
│ 电平      增益 ▬▬▬● / ◎ Center │ 增益 −3.0 dB    │
│ 时长      [10s][30s][60s][∞] ●  │ [ 播放测试音 52 ]│
└──────────────────────────────┴────────────────┘
```

**小图**：同序单列；右列的"输出电平"变成第 6 张卡，主按钮贴底全宽。

### 4.3 音频设备（`pages/devices.slint`）——直接对照 Figma `0:281`

**核心内容元素**

| # | 元素 | 内容 | 数据来源 |
| --- | --- | --- | --- |
| 1 | 工作区工具栏 | "音频设备" + "Live studio / Audio hardware & endpoints" + `Last scan HH:MM:SS` + "重新扫描" 按钮（refresh 图标 16） | `Backend.refresh-catalog()` |
| 2 | 健康行（3 卡，高 99） | 引擎时钟（18/600 mint + "Locked at 48,000 Hz"）/ 处理缓冲（"128 samples" + "2.67 ms per audio block"）/ 设备健康（"n connected / m total" + "0 dropouts · 0 clock errors"） | 会话统计；未知 `—` |
| 3 | 筛选行 | 胶囊：`All` / `Physical · n` / `Virtual · n`（激活 = accent 态）+ 右侧"搜索设备…" | `Backend.inputs/outputs` 派生 |
| 4 | 设备表 | 表头 34 + 行 66，列宽 DEVICE 390 / STATUS 132 / MODE 100 / RATE 100 / CHANNELS 144 / BUFFER 106 | `Backend.inputs` + `outputs` |
| 5 | 设备行 | 图标 19（mic/speaker/headphones/radio/monitor） + 名称 Inter 12/17 500 + 副行 Inter 9/13 faint（"USB · Driver 1.253"）+ 状态点 6 + 状态文本 + 4 个 Mono 10/14 字段 | `AudioEndpoint` |
| 6 | 选中详情（两卡） | 通道分配（绑定行：`INPUT · Analog 1 → Studio mic`，40 高，r4，底 `#14191D`）+ 时钟稳定性 | 后续功能；本轮可只读/占位 |

**行状态色**：Connected=mint、Available=`text-muted`、Offline=amber、Disabled=`text-faint`；
选中行底 `#223F39` + 左 3px mint 标记。所有普通行都是同一个底色 `#191E22`，
靠 1px `border-strong` 描边分隔（Figma 不用斑马纹）；排序/筛选后也不要引入斑马纹。

**大图**：健康行 3 等分（可用宽 1327，gap 12 → 每卡 ≈434）；筛选行两端对齐；
表格宽 1327；详情行两卡 632 / 剩余（gap 16）。

**小图**：健康行保留 3 卡但改为 2+1 折行（或纵向堆叠，副行仍显示）；
表格**隐藏 MODE / BUFFER 两列**，列宽 DEVICE 420 / STATUS 150 / RATE 130 / CHANNELS 剩余；
详情卡改为纵向堆叠。

**空态**：目录为空时表格区显示一行 `text-muted` 说明（"暂无可用设备"），
不要渲染一行空白行，不要索引不存在的第 0 行。

### 4.4 设置（`pages/settings.slint`）

**核心内容元素**

| # | 元素 | 内容 | 数据来源 |
| --- | --- | --- | --- |
| 1 | 工作区工具栏 | "设置" + "Live studio / 应用偏好" | 静态 |
| 2 | 外观卡 | 主题 Segmented「系统 / 浅色 / 深色」（`Theme.preference`）+ 说明行 | `Theme.preference` |
| 3 | 音频卡 | 默认输入 / 默认输出选择器 + 只读格式行（采样率、通道、格式支持）+ 刷新入口 | 目录 + 未来持久化 |
| 4 | 窗口与托盘卡 | "关闭窗口后继续运行" 开关 + "退出 Moiren"（danger 文本按钮） | 托盘策略；`exit-app` |
| 5 | 关于卡 | 品牌 wordmark（深色用 off-white 变体、浅色用紫色变体，保持原始宽高比）+ 版本 Mono 10/14 + 版权行 | `assets/logos/` |

每张卡都是 `InspectorSection` 结构：段标题（Inter 11/15 600 全大写 + chevron 12）
+ 内容区（底 `#14191D`、`padding 12`）+ 段间 1px `border-strong`。

**大图**：单列，内容最大宽 720，左对齐并留 `gutter 28`。
**小图**：单列，最大宽 939，卡片内边距 16。

---

## 5. 图标与品牌资产

| 用途 | Tabler 图标 | 尺寸 | 颜色 |
| --- | --- | --- | --- |
| 导航：监控路由 / 测试音 / 音频设备 / 设置 | `route` / `wave` / `devices` / `settings` | 16 | 选中 mint，其他 `text-muted` |
| 来源：物理设备 / 应用 / 虚拟输入 | `mic` / `app-window` / `radio` | 16 | `text-muted` |
| 目标：物理输出 / 虚拟端点 / 录制 | `headphones` / `monitor` / `circle-dot` | 16 | `text-muted` |
| 处理：效果机架 / 混音总线 | `sliders-horizontal` / `git-merge` | 16 | 强调项 mint |
| 分析 | `activity` | 16 | `text-muted` |
| 设备行 | `audio-lines`/`speaker`/`mic`/`headphones`/`monitor`/`radio` | 19 | 选中 mint，其他 `text-muted` |
| 工具栏 | `folder-open`、`save`、`chevron-down` | 15–16 | `text-muted` |
| 窗口控制 | `minus` / `square` / `x` | 12 | `text-muted`（关闭 hover coral） |
| 页面动作 | `refresh-cw` / `search` / `list-filter` / `grid-2x2` / `arrow-right` | 16 | `text-muted` |
| 节点卡 | `mic` / `sliders-horizontal` / `git-merge` / `speaker` | 24 | mint |

规则（沿用已确认的策略）：

- 所有功能图标来自 **Iconify / Tabler Icons 3.49.0（MIT）**，24×24 viewBox、
  `stroke-width 1.5`、`currentColor`；禁止手绘 SVG 路径。
  新增图标要同步更新 `ui/icons/manifest.json` 与 `LICENSE.tabler.txt`。
- 品牌资产只用 `assets/logos/` 的 5 个用户提供文件（Figma Logo 页 node 19:176、
  direction 2C「斜向穿梭 · 紫色」）。**不要**把它们标成 MIT/Iconify。
- 深色界面用 off-white 变体（`mark-dark` / `wordmark-dark`），
  浅色界面用紫色变体（`mark-light` / `wordmark-light`）；图标用的是
  `moiren-2c-icon-64.svg`（软件/托盘图标）。
- 旧 `ui/icons/logo.svg` 不再使用。

---

## 6. 待决策与风险

| # | 事项 | 证据 | 建议 |
| --- | --- | --- | --- |
| **D1** | **主强调色**是 mint 还是紫？ | Figma 全部 6 个帧里 mint 都是主导强调（Graph 42 次、Rack 43 次、Mixer 33 次、Devices 22 次），且 Selected/Running/Connected/激活页签/品牌 mark 都用它；`0:4` footer 明确写 "mint / broadcast"。但品牌方向 2C 的 logos 是紫色（浅色 mark `#7054B3`），且在建的 `theme.slint` 把 primary 改成了紫 `#b1a0df` 并声明"无绿色"。 | **以 mint `#83DFC4` 作为主强调**（= 手调 Graph 基准），紫色降级为 `signal-music` 角色；深色下品牌 mark 是 off-white，不冲突。若最终选择紫色，只需改 `Theme.primary` 一族 + `accent-live*` 两行，页面无需改。**需要用户拍板。** |
| **D2** | 字体可得性 | Figma 用 Inter + JetBrains Mono。Slint 1.18 的 `font-family` 是单一族名，从系统字体库解析；`Renderer::register_font_from_memory/path` 在默认渲染器（femtovg/skia）里是默认实现，直接返回 `Err("This renderer does not support registering custom fonts.")`。 | 两个选项：(a) 机器上安装 Inter + JetBrains Mono；(b) 回退到 Windows 自带 `Segoe UI` + `Consolas`（或 `Cascadia Mono`，不保证存在）。**保留 `font-ui`/`font-mono` 两个 token，两档集中切换**，并在 UI 验收里记录实际生效族名。 |
| **D3** | 浅色主题 | Figma 只有深色。 | 交付以深色为准；浅色作为推导值保留在 `Theme` 里，但要标"未经设计验证"。若不需要浅色，可把主题选择收敛为"跟随系统 / 深色"。 |
| **D4** | 尺寸差 | Figma 帧 1600×1000，应用目标 1600×956 / 1180×760。 | 已按 §2 压缩外壳（32/62/28 与 32/56/26），面板区从 878 → 834 / 646。若坚持 Figma 原高，需要同步把应用窗口改成 1600×1000。 |
| **D5** | 页面切换放哪 | Figma 用工具栏 "View selector" 切视图，没有左侧导航；应用现状是左侧 4 项导航。 | 建议保留左侧导航（4 项带副标签），工具栏只放项目/场景/传输，避免两套导航重复。备选：工具栏放页签 + 去掉导航栏。 |
| **D6** | 设备详情卡 | Figma 的"通道分配 / 时钟稳定性"依赖尚未实现的绑定能力。 | 本轮按只读卡实现；没有数据时整卡显示 `—` 与说明，不做假数据。 |
| **R1** | 并发写盘 | 生成本文档期间，`crates/moiren-app/ui/theme.slint` 在同一工作区被另一个实现者改写（尺寸 5088 → 9176 字节，时间戳 02:49:40）。 | 本文档不含代码改动，与并发实现不冲突。落地时请先把 `theme.slint` 的令牌对齐到 §1.6，再改页面，避免两套数值同时存在。 |
| **R2** | 视觉核对方式 | `openpencil export -f png` 在本机 headless 失败（canvaskit wasm 路径被拼成 `D:\C:\Windows\...`）。 | 用 `-f jsx` 导出做结构/数值核对（已导出到 `.tmp/{cl,devices,graph,mixer,matrix,rack}.jsx`），用既有 `.tmp/design-review/figma-page.png` 做视觉对照；改完页面后用 Slint MCP 截图对比。 |

---

## 附：复现命令

```powershell
# 文档概览与层级
bunx @open-pencil/cli info designs\Moiren.simplified.fig
bunx @open-pencil/cli tree designs\Moiren.simplified.fig --depth 6 | Out-File .tmp\tree6.txt

# 令牌提取
bunx @open-pencil/cli analyze colors     designs\Moiren.simplified.fig
bunx @open-pencil/cli analyze typography designs\Moiren.simplified.fig
bunx @open-pencil/cli analyze spacing    designs\Moiren.simplified.fig --grid 8

# 逐帧结构化导出（含精确坐标/尺寸/颜色/文本）
bunx @open-pencil/cli export designs\Moiren.simplified.fig --node 0:4    -f jsx -o .tmp\cl.jsx
bunx @open-pencil/cli export designs\Moiren.simplified.fig --node 0:281  -f jsx -o .tmp\devices.jsx
bunx @open-pencil/cli export designs\Moiren.simplified.fig --node 0:2453 -f jsx -o .tmp\graph.jsx
bunx @open-pencil/cli export designs\Moiren.simplified.fig --node 0:1662 -f jsx -o .tmp\mixer.jsx
bunx @open-pencil/cli export designs\Moiren.simplified.fig --node 0:1163 -f jsx -o .tmp\matrix.jsx
bunx @open-pencil/cli export designs\Moiren.simplified.fig --node 0:687  -f jsx -o .tmp\rack.jsx
```
