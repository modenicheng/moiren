# Moiren UI v0.2 — Figma 构建账本（2026-10-12）

> 本次会话在 `Moiren-UI-v0.1.0`（fileKey `axX8ZopKhTi9XfcfoLotiP`）中新建的全部内容。
> 原则：`Main` 页的 `3:5604`（Graph 完整布局）是唯一风格原型，只读；
> Application toolbar（`50:879`）与 Audio engine status（`50:880`）视为完成品，直接复用实例。
> 构建账本 JSON：`.tmp/figma-build/state.json`。

## 1. 页面清单（全部新建）

| 页面 | 内容 |
| --- | --- |
| `Moiren · Design System`（100:337） | 色板总览（21 token，Dark 值 + 解析 hex）、12 个文字样式样张、组件样例、使用规则说明卡 |
| `Moiren · Graph`（100:338） | 完整工作区 1600×956（见 §3） |
| `Moiren · Mixer`（100:339） | 8 条通道 strip：pan / fader+双电平表 / 2 sends / mute |
| `Moiren · Matrix`（100:340） | 8 源 × 4 目标发送网格；激活格 accent 底 + 增益值，空格 55% 透明 |
| `Moiren · Devices`（100:341） | 3 健康卡、筛选 chips、6 行设备表、选中详情两卡（通道分配 / 时钟稳定性） |
| `Moiren · Settings`（100:342） | 外观（System/Light/Dark 分段）、音频引擎 4 行只读、窗口托盘（开关 + danger）、关于 |
| `Moiren · Overlays`（100:343） | 右键菜单、设备离线横幅、空状态、Add node 弹窗（含 scrim）、编译失败行内错误 |
| `Moiren · Prototype Flow`（122:387） | Graph/Mixer/Matrix 三帧克隆 + 视图切换器 ON_CLICK 互跳（原型导航） |

## 2. 基础层（扩展现有集合，未新建重复集合）

- `Moiren · Theme` 增加 **Light** 模式（`100:0`）；Dark（`3:1`）保持默认。
  Light 18 值为推导（`accent/primary → #7054b3` 品牌紫），未做视觉验证，仅作模式占位。
- 新增语义变量：`surface/control-hover`、`text/on-accent`、`surface/scrim`；Primitives 补 `palette/2a3238`。
- 新增 **12 个文字样式** `Moiren/*`：Display 19 / Title 13 / Title SM 12 / Section 11(+0.6ls) /
  Button 12 Medium / Body 12 / Body SM 11 / Caption 10 / Micro 9；
  Value 12 / Value SM 10 / Value XS 9（JetBrains Mono）。

## 3. 新组件（Design System 页）

| 组件 | 变体 | 说明 |
| --- | --- | --- |
| `Moiren / Button`（扩展） | Default / **Selected** / **Hover** / **Pressed** / **Focused** / **Disabled** / **Danger** / **Ghost**（8 态） | 粗体为本次新增；几何对齐原 76×30 r4 |
| `Moiren / Library Item` | Default / Hover / Selected / **Disabled** | 186×54；Icon 为 INSTANCE_SWAP（`Icon#…`），Label/Detail 文本覆盖 |
| `Moiren / Icon` | 10 个 Tabler 3.49.0 字形 | mic/app/radio/sliders/merge/headphones/monitor/record/activity/search；stroke 1.8 |
| `Moiren / Section Header` | 单组件 | 全大写 11 SemiBold + 0.6ls + chevron |
| `Moiren / Search Field` | 单组件 | 186×32，placeholder + Ctrl K |
| `Moiren / Text Field` | Default / Focused / Error / Disabled | 240×32；Error=coral 边+字 |
| `Moiren / Select` | Default / Open（含下拉预览） | 240×30 |

## 4. Graph 页结构（基准 `3:5604` 的组件化重建）

- 复用 toolbar/statusbar 实例 + 现有 `Audio Node` 四变体（Input/Processor/Output/Recorder）。
- **节点库侧栏补全**：3 分类 × 9 条目全部为 Library Item 实例（图标 swap 到对应字形），
  折叠 chevron 已修正；底部拖拽提示卡。
- 画布：24px 网格（`border/grid`）、编辑工具条（Select 选中态/Pan/缩放）、
  12 节点实例（真实拓扑：Studio mic→Voice Rack→Voice→Stream→OBS/Recorder；
  Spotify→Music→Stream+Monitor；Discord→Monitor；Monitor→RME 耳机+分析）、
  12 条贝塞尔连线（按源端口类型着色：violet=内部总线/app、blue=物理设备、
  gold=Recorder、选中线=selected-wire）、gain 徽章（0.0 dB · POST）、REC 状态行。
- Connection Inspector：SIGNAL PATH（4 行 Property Row 实例，SPACE_BETWEEN 修正）、
  ROUTE CONTROLS（fader 7×414 + 双电平表 + 标题块）、CONNECTION NOTES + Locate 按钮。

## 5. 交互契约（已实现 / 待实现）

已实现：视图切换器三页互跳（Prototype Flow 页，ON_CLICK/NAVIGATE）。

待定（需用户确认后再做）：

1. **Device Manager / Settings 的导航入口**——toolbar 切换器只有 Graph/Mixer/Matrix 三签。
   文档 08 说 v0.1 只有 Graph 视图；文档 07 是左侧 4 项导航。两种外壳模型尚未拍板。
2. Inspector 与节点选中状态的联动原型（同页 overlay）。

## 5a. 收尾轮（同日续）

- **Overlays 补全**：Engine starting 横幅（dashed spinner + "Preparing devices · 2 of 3"）
  与 Scanning skeleton（3 行 shimmer + "Scanning audio devices…"）落地并截图验证。
- **节点几何修正**：Audio Node 实例 226→186 宽（列距 206 内不再重叠）；12 条连线按
  端口圆点实测位置（out=右缘、in=左缘、y+83）全量重建。
- **Bus 节点组件** `Moiren / Audio Node · Bus`（135:387，DS 页）：Processor 派生 detach，
  双端口（in L/R / out Σ 2）+ "INTERNAL BUS" 类型标（9px + 0.6ls，accent/primary 绑定）。
  Voice/Music/Stream/Monitor 四个内部总线换用该组件——语义上总线必有入和出，
  原 Input 变体只有输出口是错的。母版 5:975 的 MIN 约束输出口圆点问题由此绕开（未动用户原始组件）。
- **紧凑布局** `Moiren · Graph · Compact 1180×760`（125:689，Graph 页 y=1056）：
  采用参考帧 50:881 的策略——面板宽不变、画布 970 宽 + clipsContent（内容 1110 裁剪=可滚动）、
  检查器折叠、toolbar 实例内隐藏 Scene selector。
- **Prototype Flow 刷新**：Graph 克隆画布替换为修正后的节点/连线。
- 全页截图终验：Graph（全尺寸+紧凑）/Mixer/Matrix/Devices/Settings/Overlays/DS/Prototype 均通过。

## 5b. v1 紧凑外壳轮（用户指定参考：`7SkG0urUPleASo1WHZW2k9` 帧 27:15644 / 27:16749）

用户喜欢第一版的紧凑排版，按其模式改造：

- **`Moiren / View Navigation` 组件**（144:330，DS 页）：210 宽侧栏，
  "View navigation" 头 + 折叠图标 + Graph/Mixer/Matrix/Devices/Settings 五项
  （选中项 accent/surface 底 + accent 描边）+ 底部 SCENE 徽章卡。
  —— 同时解决了 Devices/Settings 没有导航入口的遗留问题。
- **Matrix 页完全重建**（v1 密度）：工作区工具条带（标题+面包屑+统计+动作）、
  8 源 × 9 目标密集网格（单元格 90×49：激活=accent 底 + 增益 JBM + POST 微标，
  空格=面板底 + ·）、图例行、紫色选中路由条（Voice → Stream + Show in Graph）、
  两张语义脚注卡、右侧 ROUTE · SELECTED CELL 检查器（含 Mute/Remove route 动作）。
- **Mixer / Settings 页**插入导航实例，内容右移 210；Mixer 8 条 strip 压缩到
  130 宽（间距 137）装进 1110 内容区。
- **Devices 页按用户三点要求重建**：
  1. **行内 icon**：6 行设备表每行 16px Icon 实例（Monitor/Microphone/Headphones/Radio），
     离线行整体降透明度；
  2. **时钟漂移可视化**：CLOCK STABILITY · LAST 60 S 卡——±1 ppm 色带 + 零线 +
     60 点漂移折线（accent 描边）+ 末端当前值圆点 + ppm/时间轴标；
  3. **设备移除**：REMOVE DEVICE 红描边卡——后果说明（删除节点与经由路由、
     旧 plan 继续播放直至重编译）+ 受影响清单（2 节点 · 3 路由）+ Remove device danger 按钮 +
     "routes are silenced first" 提示；Inspector 亦含 Remove device… 项。
- **Prototype Flow 刷新**：Mixer/Matrix 克隆重建（自带导航），三帧切换器重新接线
  （NAVIGATE 拒绝自导航——每帧跳另外两帧）。

### 已知取舍

- Graph 页（全尺寸+紧凑）保留节点库侧栏、不加导航列：库侧栏 210 宽与导航同位，
  v1 的 Graph 视图身份由 toolbar 切换器承担。
- Mixer strip 130 宽较原 176 窄，send 行增益文字换行到最小可用宽度；如需恢复
  全宽可去掉导航列或减少 strip 数。

- 连线/标记色跟随**源端口**节点类型；健康/运行 = mint 家族；**不用绿色**。
- 面板宽固定：库 210 / Inspector 280 / resize handle 2×30；画布 flex。
- 8px 节奏 + 控件内 2–7px 微距；圆角 2/4/6；全部 1px 描边。
- 状态覆盖：Default/Hover/Pressed/Focused/Selected/Disabled/Error（Button 8 态、
  Text Field 4 态、Select 2 态、Library Item 4 态）。

## 6. 语义约定（写入 Design System 页说明卡）

- 连线/标记色跟随**源端口**节点类型；健康/运行 = mint 家族；**不用绿色**。
- 面板宽固定：库 210 / Inspector 280 / resize handle 2×30；画布 flex。
- 8px 节奏 + 控件内 2–7px 微距；圆角 2/4/6；全部 1px 描边。
- 状态覆盖：Default/Hover/Pressed/Focused/Selected/Disabled/Error（Button 8 态、
  Text Field 4 态、Select 2 态、Library Item 4 态）。
