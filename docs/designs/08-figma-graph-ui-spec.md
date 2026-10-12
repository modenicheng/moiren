# Moiren UI v0.1.0 — Figma "Graph" 设计规格

来源：Figma 文件 `Moiren-UI-v0.1.0`（node-id 0:1），两个 frame：

- `3:5604` — **完整布局** 1600×956（设计基准）
- `50:881` — **紧凑布局** 1157×644（自适应参考）

同一设计，所有面板宽度、控件尺寸完全一致；紧凑变体只是画布变窄、
节点列间距压缩。这意味着自适应靠**面板固定宽 + 画布 flex + 断点折叠**，
而不是等比缩放。

## 整体框架（三段式）

```
┌──────────────────────────────────────────────────────────────┐
│ Application toolbar · 44px 全宽                               │
│  [identity+project] [Graph|Mixer|Matrix 居中] [scene/engine/win] │
├────────────┬─────────────────────────────────┬───────────────┤
│ Node       │ Signal graph workspace (flex)   │ Connection    │
│ library    │  画布网格 + 节点 + 连线          │ inspector     │
│ 210px      │  顶部工具条 + 底部提示           │ 280px         │
├────────────┴─────────────────────────────────┴───────────────┤
│ Audio engine status · 28px 全宽                               │
└──────────────────────────────────────────────────────────────┘
```

工作区高度 = 窗口高 − 44 − 28（884px @956；572px @644+）。
左右面板有 2px 的 Panel resize handle（垂直居中，2×30px）→ 面板可调宽。

## 色板（无 style，直接节点取值）

| token | 值 | 用途 |
|---|---|---|
| window-bg | `#111518` | 画布/窗口底 |
| surface (chrome) | `#14191d` | 顶栏、状态栏、搜索框底、meter 底 |
| surface-2 (card) | `#191e22` | 面板、节点卡 |
| surface-3 (control) | `#21282d` | 按钮/动作框底 |
| surface-hover | `#2a3238` | （推断）控件悬停 |
| primary / violet | `#b1a0df` | 主强调：选中态、引擎运行、连线 |
| primary-deep | `#2b1f4a` | 选中按钮底、gain 徽章底 |
| signal-blue | `#86b9ec` | 物理输入/输出、部分内部总线 |
| signal-gold | `#e7bc79` | 录制/REC 相关 |
| danger | `#eb8d87` | 峰值段、REC 指示点 |
| border | `#30393f` | 面板/控件描边（主边框） |
| border-soft | `#232b30` | 分区线（推断自旧主题） |
| grid-line | `#1b2227` | 画布网格线 |
| grip/resize | `#49525a` | 拖拽把手 |
| text | `#e5ecef` | 主文字 |
| text-muted | `#a5b2ba` | 次级文字 |
| text-faint | `#6f818d` | 弱文字/占位符 |
| on-primary | `#14181c` | 紫底上的墨色文字 |

深色单主题（设计只有深色）。无绿色：运行=蓝紫，录制=金/红。

## 字体

- UI: **Inter**（Regular/Medium/SemiBold）
- 数值/机器信息: **JetBrains Mono**（10-11px）

字号阶梯（实测）：9 / 10 / 11 / 12 / 13 / 19px（检查器大标题）。
节点类型标签带 0.6px letter-spacing。Windows 上字体栈回落 Segoe UI / Cascadia Mono。

## 顶栏（44px，bg #14191d）

左：Logo(28×28，隐藏占位) + "Moiren"(12 SB, #a5b2ba) + 分隔 + 项目组
（folder-open 16 + "Live studio"(13 SB) + chevron-down 12 + save 15）。
中：视图切换器（Graph 选中 76×30 紫描边、Mixer/Matrix 默认 76/72×30）。
右：SCENE 标签(10px) + 场景按钮(122×30: chevron 16 + "01 Broadcast" 12 M) +
引擎状态点(6px 紫椭圆) + "Engine running"(JBM 11 紫) + Stop 按钮(70×30: square 16 + Stop) +
窗口控制（minus 12 / square 10 / x 12，42px 间距中心）。

按钮规格：底 #21282d，描边 #30393f，圆角 4，高 30；选中态描边 #b1a0df。
窗口控制：无底色，hover 变化（设计留白，实现自定），close hover 红。

2026-10-12 顶栏实现修正：控件拆入 `toolbar-controls.slint`，按钮视觉高度仍为 30px，
点击热区覆盖 44px 行高。操作按钮图标为 18px，Start/Stop 最小宽度统一为 80px；
窗口按钮热区统一为 42×44px，图标为 16px（最大化为 14px），按钮之间不留间隙。
内容间距为 8px，控制组间距为 16px；窗口控制组贴齐右边框，关闭按钮悬停背景覆盖到窗口边缘。

## 节点库（210px 面板，bg #191e22，右描边）

内边距 12px（内容宽 186）。

- 标题行 "Node library"(13 SB) + 折叠 chevron 16
- 搜索框 186×32（底 #14191d 描边 #30393f r4）：search 14 + "Search nodes" 11 + "Ctrl K" 10
- 分类（SectionHeading 11 SB uppercase #a5b2ba + chevron 12）：
  - SOURCES: Physical input(mic), Application audio(app-window), Virtual input(radio)
  - PROCESSING: Effect rack(sliders-horizontal), Mix bus(git-merge)
  - DESTINATIONS: Physical output(headphones), Virtual endpoint(monitor), Recorder(circle-dot), Signal monitor(activity)
- LibraryItem 186×54：icon 16 + 两行（名 11 / 副标 9 faint）+ 第三行 detail 10 faint + 右侧 grip dots（2×3 圆点 1.5px）
- 拖拽提示卡 186×67 r4 底 #14191d："Drag a node onto the graph" 11 + "Or right-click the canvas…" 10
- 分组间 18px 间隙；分类可折叠

## 信号图画布（flex，bg #111518）

- 网格：24px 间距，线 #1b2227（0.5px 视觉 → 1px 实现）
- 编辑工具条（画布内浮层左上，y=18）：Select(选中 80×30 紫底深 #2b1f4a+紫描边) / Pan 54×30 / 缩放控件 112×30（"− 100% +"）+ 2 个 icon 按钮
- 右上提示 "Drag nodes to arrange · Scroll to zoom" 10px faint
- GraphNode（两种宽度：源/处理节点 156×124；终点节点 215×124；r6，底 #191e22 描边 #30393f）：
  - 名字 12 SB（左 padding 22 留类型标条）
  - Type marker：3×13 r1 色条（节点类型色）
  - NodeType 标签 9px + 0.6 letterspacing（类型色）
  - Detail 10px JBM（如 "RME · Analog 1 · mono"）
  - 端口标签行：in "1 ch"/"L / R"/"Σ 2"，out 同理，9px faint；左入右出
  - 端口圆点 8px：出=类型色实心+节点底描边；入=节点底+类型色描边（左右居中突出 4px）
  - Value 9px JBM（dBFS 读数，右下）
  - Level meter：4px 高，底 #14191d，前景类型色，宽 89（156 节点）/148（215 节点）
  - 特例：Recorder 节点附加 REC 行（红点 5px + "REC 00:12:48 · 211 MiB" JBM 9 金色）
- 连接线：贝塞尔（实现定），颜色跟随源端口类型色；中点 gain 徽章
  85×20 r4（底 #2b1f4a 描边紫，"0.0 dB · POST" JBM 9 紫）
- 底部 footer（画布内左下 y=-42）："Drag ports to connect · Shift to select multiple · Right-click for actions" 10 faint；右下 "Voice → Stream selected" 10 紫（选中态）

## 连接检查器（280px 面板，bg #191e22，左描边）

- 头 48px：sliders icon 16 + "Inspector" 12 SB
- SIGNAL PATH 分区（读力行 17px 间距 29）：Source / Destination / Channel mapping / Route state — Key 12 #a5b2ba 左，Value 12 JBM #e5ecef 右
- ROUTE CONTROLS：fader（导轨 7×414 #30393f，thumb 20×36 r4 白）+ 双通道电平表（7×414 底 #30393f；前景 #b1a0df，峰值段 2px #eb8d87）+ 标题块（"CONNECTION · SELECTED" 10 SB 紫 / "Voice → Stream" 19 SB / "Stereo route · 2 channels" 11 muted）
- CONNECTION NOTES：描述文字 11 muted 两行 + "Locate connection" 按钮 142×30
- 分区间 1px #30393f 分割线，内边距 16px（内容宽 248）
- 底部 spacer；面板内容超高时滚动（实现决定）

## 状态栏（28px，bg #14191d）

左：状态点 6px 紫 + "Audio stable" JBM 10 紫 + "48 kHz / 24-bit" + "128 samples · 2.67 ms" + "Clock: RME Babyface Pro FS"（JBM 10 #a5b2ba，间距不均 ~29/23/149…实现用 24px 芷间隔）。
右：CPU 8.4% / XRUN 0（JBM 10 #a5b2ba）+ 分隔线 + "v0.1.0"（JBM 10 紫）。

## 图标（Tabler 3.49.0 / Iconify，stroke currentColor, 24 viewBox）

已有：app, check, chevron-down, clock, devices, info-circle, microphone, moon,
plus, refresh, route, search, settings, adjustments-horizontal, volume,
player-stop, sun, device-desktop, wave-sine, x, maximize, minus。

需补充：folder-open, save, radio, git-merge, headphones, monitor,
circle-dot, activity, arrow-up-left(↖ select), hand-move(pan),
zoom-scan / arrows-move(缩放)。

## 交互契约（设计暗示）

- 视图切换 Graph/Mixer/Matrix（v0.1 只实现 Graph，其余占位）
- 节点库条目拖入画布；右键画布菜单（后续）
- 端口拖拽连线；Shift 多选；滚轮缩放
- 连线选中 → 检查器显示该路由；"Locate connection" 高亮连线
- SCENE 下拉、Stop 引擎、窗口三键、面板 resize handle
- Ctrl+K 聚焦搜索
