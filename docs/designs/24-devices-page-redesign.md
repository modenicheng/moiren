# 设计 24:设备页整体重设计(间距、尺寸、信息架构)

状态:实现中 · 2026-11-19 · 分支 `feat/devices-page-redesign`(工作树 `.worktrees/devices-redesign`)

## 1. 背景与问题清单

设备页职能已在设计 23 定案并落地(#167/#168):只回答三个问题——有哪些设备、每台设备用什么流参数打开、每台设备此刻实际跑成什么样;绑定只在 Graph 的 IO 节点上编辑。本设计不改职能,重做呈现层:布局、间距、尺寸、信息组织。基线截图见 `target/capture-baseline/`(ui_capture 已扩展为每个设备 tab 各一张,见 §7)。

### 1.1 结构性缺陷

| # | 问题 | 根因 |
| --- | --- | --- |
| S1 | 缓存路径整页 hug 收缩:工具栏、列表、行全部按内容宽渲染(~500px),Refresh 按钮缩在 tab 旁,右侧大片死区;仅 Exports tab(不走缓存)正常满宽 | `devices-page` 根用 `flex_1` 承担尺寸,违反缓存面板根必须用确定尺寸(`w_full/h_full`)的规则(PR #104 教训);mixer/settings 根均已用 `size_full` |
| S2 | 行/卡片无阅读量上限:满宽后设备行会被拉到 ~1100px,不可读 | 列没有 measure;settings 已确立 680 阅读列(COLUMN_WIDTH) |
| S3 | 设备行垂直贴合堆叠,边框相接 | `device-list` 没有 gap |
| S4 | 弹性子元素被 flex_col 拉伸:inspector 的 badge 拉成整面板宽的紫带;导出卡里 Rename、默认资格按钮拉成整卡宽 | 直挂 flex_col 的行内元素没有包在行容器里 |
| S5 | inspector 两种状态两副面孔:无选中时是通栏 `panel()`(左描边、p-4);有选中时是 p-3 宿主里的浮动圆角卡 | 两条渲染路径的 chrome 不统一 |
| S6 | Clock 卡标题用 `·` 连接三段 | 违反已确立的"分隔符禁 `·` 用 `/`" |
| S7 | Exports tab 文字墙:页头一句 + 面板下三段连续说明 + 空态一句,帮助文案不在使用点旁边 | 帮助文本堆在列表头,没有按"是什么/状态语义/操作警告/按钮语义"分置 |
| S8 | 空态是一行裸 label | 无构图;graph inspector 已有 icon+文字的空态范式 |
| S9 | 创建面板里方向组(Playback/Microphone)与声道组(Mono/Stereo)两行无标签,四颗按钮连排难分辨 | 组没有标题 |

### 1.2 可用性/密度问题

- 激活 tab 用实心 Primary 按钮,比应用栏导航(文字+紫色下划线)重一档;页内类目切换是导航语义,应沿用下划线语言。
- 行内徽标(flex_wrap)跟在标题后换行,活动状态(In use/Unavailable)不齐、不扫读。
- 摘要行 `96 kHz / Shared / Device native / 512 frames` 是数值读数,用 UI 字体随数值更新抖动,违反"数值用等宽字体"。
- inspector 把配置(override 编辑)与监控(Users)平铺同一层级,没有分节节奏。
- Clock tab 只有一行标题+一行空话,空态不解释"启动音频后这里会出现什么"。

## 2. 目标与非目标

**目标**

1. 修复 §1.1 全部结构性缺陷;页框、列表、卡片、inspector 各元素有确定的、来自令牌标尺的间距与尺寸。
2. 信息架构按使用点组织:帮助文本放到它解释的控件旁;状态事实分组;配置与监控分节。
3. 复用既有组件(ActionButton、badge、detail、panel/panel_host、scroll_region、Input、stream_editor、footer、settings 行/卡语言);新增少量共享零件(空态、等宽 fact 行、页内 tab)。
4. 数据语义零变化:不新增假数据,不动绑定边界(设计 23 R1 源码扫描测试保持通过)。

**非目标**

- 不改 `settings::stream_editor`/`footer`/select 的内部实现。
- 不改 Graph/Mixer/Matrix 页(它们的 inspector 空态暂不动,只抽共享零件供设备页用)。
- 不引入新引擎数据;不做设备页路由控制;不做 Compact 抽屉改造。

## 3. 信息架构

页面保持四 tab:Inputs / Outputs / Virtual devices / Clock。

### 3.1 页框

```
┌────────────────────────────────────────────────────────────────┐
│ Inputs  Outputs  Virtual devices  Clock            [Refresh]  │ ← 48px 工具栏
│ └ tab:文字+紫色下划线(激活),Ghost 底;Refresh 右对齐        │
├────────────────────────────────────────────────────────────────┤
│ [ASIO 错误横幅(仅故障时,通栏 quiet danger)]                   │
├────────────────────────────────────────────────────────────────┤
│ ┌ 680px 阅读列(左锚定)──┐                      │ inspector 280 │
│ │ 列表/卡片滚动区        │                      │ (通栏 panel)  │
└────────────────────────────────────────────────────────────────┘
```

- 根:`size_full()` 确定尺寸,`flex_1` 兜底(S1;mixer 页根同款双保险)。
- 阅读列 680px(`theme::READING_COLUMN`,settings 的 COLUMN_WIDTH 同源收编)**左锚定**:本页右侧的 inspector 列随时开合,居中列会随宿主宽度变化整体平移(开 inspector 时整列表横跳,测试的定位点击也因此失准);左锚定后列表 x 恒定,空区留在右侧。`max_w_full` 在窄窗口收缩。
- 页内 tab 复用应用栏 WorkspaceTab 的视觉语言(文字 12px、激活 SEMIBOLD+2px FOCUS 下划线、Role::Tab/TabList),不复用其 appbar 尺寸变量。

### 3.2 Inputs / Outputs 列表行

```
┌──────────────────────────────────────────────────────────┐
│ 设备名(15 semibold,truncate)      [In use ×1] [Default] │ ← 状态徽标右对齐组
│ 96 kHz / Shared / Device native / 512 frames(mono 11)    │ ← 等宽摘要
└──────────────────────────────────────────────────────────┘
```

- 卡壳:SURFACE + HAIRLINE 边框 + RADIUS_MD(settings 卡语言);选中 FOCUS 边框,hover RAISED。
- 行内边距 px16/py12;标题→摘要 gap 6;列表 gap 8。
- 徽标语义分组右对齐,顺序:Unavailable / In use ×n / Bound ×n / Default / Override(状态事实在前,配置身份在后)。
- 点击行 = 打开详情(唯一交互,不变)。
- 空态:icon+标题+提示的居中空态块。

### 3.3 Inspector(统一 chrome)

两种状态同一副面孔:通栏 SURFACE + 左描边,内容 p-4 gap-12,分节用 hairline 分隔线;override 态底部保留草稿 footer(上描边)。

```
│ 设备名(15)                    │
│ Output configuration(caption) │
│ [Device override] 徽标行       │
│ ──────────────                │
│ Saved request      96 kHz / … │
│ [Use defaults] [Override]     │
│ (override 时)流参数编辑器      │
│ (就近说明)                    │
│ ──────────────                │
│ Users                         │
│ 节点名 [Running] [Locate]     │
│   └ Actual stream … 等宽 fact │
│ Active request …              │
```

- 徽标不再拉伸(S4);Users 行收进行容器。
- 空态(未选中):icon+一句话居中。

### 3.4 Virtual devices

- 页头:标题 + 两行定位说明(虚拟设备是什么、状态语义怎么读)。
- 创建卡(settings 卡语言):`Type [Playback|Microphone]`、`Channels [Mono|Stereo]` 两组带标签行 + 名称输入 + Create。
- 设备卡:标题(名称)+ 方向/声道 caption;标题行 Enabled/Disabled 徽标用品牌紫/中性灰(候选配置态,**不用**青色——青色保留给活流徽标);状态事实组(Endpoint/Gate/Binding);控制组(声道、改名[输入+按钮同行]、启用/停用+移除、默认资格)+ 组末就近警告;Users 分节。
- `export-default-help` 帮助块(测试锚定)保留,挂在页头说明组。

### 3.5 Clock

- 标题 + 空态(空时);处理时钟卡 + 每流一卡:标题=设备名,副行=`节点 / 角色`(caption,/分隔,S6),fact 行等宽值。

## 4. 视觉规格(令牌)

| 项 | 值 | 来源 |
| --- | --- | --- |
| 页根 | `w_full().h_full()` | PR #104 规则 |
| 页工具栏高 | 48(`PAGE_TOOLBAR_HEIGHT`) | 既有 |
| 阅读列宽 | 680px,水平居中 | settings `COLUMN_WIDTH` |
| 列表 gap | `SPACE_2`(8) | 间距标尺 |
| 卡片内边距 | px`SPACE_4` / py`SPACE_3` | 标尺组合 |
| 卡片圆角/边框 | `RADIUS_MD` + HAIRLINE 1px | settings 卡语言 |
| 选中边框 | FOCUS 1px | 既有 |
| 标题 | `TEXT_LG` 15 SEMIBOLD(`heading`) | 字阶 |
| 分节标题 | `TEXT_MD` 13 SEMIBOLD(settings `group_title` 语言) | 字阶 |
| 正文/说明 | `TEXT_SM` 12(`label`) | 字阶 |
| 徽标 | `TEXT_CAPTION` 11(badge 组件) | 组件 |
| 数值读数 | Cascadia Mono 11(`detail_mono` 新增) | 字阶 Mono/Metric |
| 分隔线 | `HAIRLINE_W` 1px / `HAIRLINE` 色 | 描边标尺 |
| 空态图 | 64px RAISED 圆角块 + 28px 图标(graph inspector 空态范式) | 既有范式 |

新字符串全部走 `t!` 并补 `locales/zh-CN.yml`;键留目录满足守卫。

## 5. 组件复用与新增

**复用**:`ActionButton`(`button()`)、`badge`、`detail`、`heading`/`label`、`panel`/`panel_host`、`scroll_region`、`component::input::Input`、`settings::{stream_editor, footer, stream_summary, fields::row}`、`WorkspaceTab` 的下划线语言(自绘同款,不复用其 appbar 尺寸)。

**新增(共享零件,进 `shell/controls.rs`)**:

- `empty_state(icon, message)` + `EMPTY_TILE`/`EMPTY_ICON` 几何常量:icon 块 + 说明的居中空态;设备页列表/inspector/Clock/Exports 空态统一用它,graph inspector 空态同源收编。
- `detail_mono(key, value)`:`detail` 的等宽值变体,用于流参数/时钟**数值**行( FRONTEND §5.4:mono 保留给数值读数;方向/角色等非数值元数据走 UI 字体 caption)。
- `hairline()`:水平 1px 分隔线。
- `page_tab(id, label, active, on_click)`:页内下划线 tab(Role::Tab + aria_selected),设备页四 tab 用;先落在 devices.rs,等第二个使用点出现再上移 widgets。

**合并重复**:overrides.rs 与 exports.rs 各有一份"Users 块"渲染,抽成 devices.rs 的 `users_section(snapshot, device, cx)` 单一来源。

## 6. 数据语义与规则遵循

- 不出现任何绑定编辑标识(源码扫描测试 `devices_page_sources_never_name_a_binding_edit` 保持绿)。
- requested → accepted → applied 语义不变:行徽标与详情仍区分候选绑定、引擎实测、保存请求与生效请求;未知仍显示 `—`。
- 测试锚定的 id 全部保留:`devices-page`、`device-list`、`device-row-{id}`、`devices-{inputs,outputs,exports,clock}`、`device-override`(按钮)、`device-override-host/inspector/scroll`、`device-user-{node:?}`、`reveal-{node:?}`、`devices-asio-error`、`export-{create*,row,name,rename,toggle,remove,default,mono,stereo,reveal,user*,users-empty*}-{id}`、`export-default-help`、`scrollbar-*`。
- 导出卡 meta 行去掉原始 endpoint id 展示(FRONTEND 2026-10-16:UI 隐藏内部 ID;端点名已是用户可见身份)。

## 7. 工具与验收

- `ui_capture` 扩展:每个设备 tab 各出一图(`{size}-devices-{inputs,exports,clock}.png`,Outputs 沿用 `{size}-devices.png`),comfortable+compact 双尺寸;已用其生成重设计前基线。
- 门(触面 = moiren-app):`cargo fmt --all -- --check`、`cargo clippy --locked -p moiren-app --all-targets -- -D warnings`、`cargo test --locked -p moiren-app --features gpui-test`;全仓 `cargo check --locked --workspace --all-targets` 兜底。
- 目检:重设计后 ui_capture 全 tab 双尺寸复核,与基线对照。
- 审计:开发完成后派无上下文子代理审计 diff+截图,迭代到无 Critical/Major。

## 8. 实现清单

| 文件 | 变更 |
| --- | --- |
| `ui/shell/controls.rs` | +`empty_state`、`detail_mono`、`hairline` |
| `ui/pages/devices.rs` | 根改确定尺寸;页内 tab 组件;列表列 680 居中+gap;行重排(徽标右组/等宽摘要);Clock tab 重构;空态;`users_section` 共享 |
| `ui/pages/devices/overrides.rs` | 统一通栏 inspector chrome;徽标/按钮行化;分节 hairline;Users 走共享块 |
| `ui/pages/devices/exports.rs` | 页头/创建卡/设备卡重排;标签化创建行;控件行化(S4);id 展示移除;帮助文本就位 |
| `locales/zh-CN.yml` | 新增键(如 "Type") |
| `ui/shell/capture.rs` | 设备页逐 tab 截图(已落地) |

## 9. 决策记录

- **阅读列左锚定而非居中**(实现期修订):本页右侧 inspector 随选中状态开合,居中列会在每次开合时整体平移——既是交互瑕疵(列表横跳)也让测试的定位点击失准(无头注册的 bounds 混代)。settings 可居中是因为其宿主宽度恒定。左锚定后列表 x 恒定,空区留在右侧。
- **测试折叠下断言走 scroll_to_control**(实现期修订):本机无头测试窗口布局视口被显示高度钳制(请求 1440×1600,实际 1920×1080),重设计后卡片加高,卡底控件落在折叠之下;页面本就可滚动(最小窗高 600px),测试按用户方式滚动到控件后再断言——与既有 compact 可达性测试同一先例。`scroll_to_control` 的滚轮目标同步从 `devices-page` 改为阅读列自身的滚容器(列左锚定后页面中心可能落在列外)。
- 激活 tab 用下划线而非实心 Primary:页内类目切换是导航语义,与应用栏导航同语言,减重。
- 行摘要用等宽字体:数值读数防抖,符合字阶 Mono/Metric 角色。
- Unavailable 徽标保持 WARNING 色:它是"需要行动"的状态,优先级高于身份徽标。
