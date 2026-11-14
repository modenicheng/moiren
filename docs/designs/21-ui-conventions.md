# Design 21 — UI 约定与去硬编码规范

状态：草案（2026-11-09，基于三份全量审计：硬编码清单、布局弹性、i18n/逻辑）。
受众：之后**手工编辑本 UI 的人**。目标是"看得懂、改得动、改不坏"：
魔法数字归 token、布局弹性可预期、文本全部走 i18n、重复模式组件化。

## 1. Token 体系（theme.rs 为唯一来源）

现状：343 处字面 `px()` vs 64 处 token（84% 字面）。颜色已 100% token 化 ✓。

### 1.1 间距（补齐阶梯，~135 处字面可映射）

| Token | 值 | 已有 |
|---|---|---|
| `SPACE_0_5` | 2 | 新增 |
| `SPACE_1` | 4 | 新增（现在 22 处字面 4） |
| `SPACE_1_5` | 6 | 新增（24 处字面 6） |
| `SPACE_2/3/4` | 8/12/16 | 已有（采用率 23%，补齐） |
| `SPACE_6` | 24 | 新增 |

偏离栅格的值（3/7/10/14/28）按就近归档或语义命名。规则：**新代码禁止字面间距**。

### 1.2 文本（补齐字号+行高）

- 新增 `TEXT_MICRO=8`、`TEXT_TINY=9`、`TEXT_CAPTION=11`；沿用 XS/SM/MD/LG。
- 行高跟随字号 token（`TEXT_*_LH` 或 metrics 表内联）。
- `card::TITLE_SIZE`→`TEXT_SM`、`card::HINT_SIZE`→`TEXT_TINY` 改别名。

### 1.3 新系统

- 图标：`ICON_SM=14 / ICON_MD=16 / ICON_LG=18 / ICON_XL=24`（现仅 controls::ICON=16 一处）。
- 描边：`HAIRLINE_W=1`（配 divider helper）、`FOCUS_RING_W=2`、`ACCENT_BAR_W=2`。
- alpha：`ALPHA_SELECTION=0.10 / ALPHA_HALO=0.28 / ALPHA_DIM=0.55` + `halo()/dim()` helper（现 kind bar 0.8 vs 0.55 不一致，统一）。
- chrome 高度：`STATUSBAR_HEIGHT=28`（现两处字面）、`PAGE_TOOLBAR_HEIGHT=48`、`SECTION_HEADER_HEIGHT=28`、`HUD_INSET=12`。
- 窗口：`COMPACT_BREAKPOINT=1200`、`WINDOW_MIN=(940,600)`（现两处）、`RESTORE_MIN_VISIBLE=(100,40)`；`compact(a,b)` helper 收编 4 组内联 if-compact 对。
- popover 度量：`POPOVER_{VIEWPORT_MARGIN=8,ANCHOR_GAP=4,PAD,ITEM_GAP}` + 共享放置函数（见 §3）。

### 1.4 重复 token 清理

`TOOLBAR_HEIGHT`≡`controls::TOOLBAR`、`RADIUS_SM`≡`controls::RADIUS`、`NODE_DSP`≡`FOCUS`≡`PRIMARY`、`FONT_UI`≡`controls::FONT` 等改别名；`ON_ACCENT` 从 theme 重导出；`transparent_black()`×6 → `controls::TRANSPARENT`。

### 1.5 控件几何（留在控件旁，不进 theme）

fader 8 原语、meter 灯/标签偏移、balance 行几何、mixer 读数预留块（现 12 处字面散 4 文件，收敛为 `READOUT_*` 一组）、matrix 列宽、画布线宽/端口/命中常量、`graph_canvas/layout.rs` ~20 内联偏移。

## 2. 布局约定

- **弹性纪律**（现状已好，保持）：列内容 `flex_1()+min_w_0()`；伸缩容器 `min_h_0()`；控件 `flex_none`，弹性只来自容器。
- **文本截断**：承载用户/设备文本的标签一律 `min_w_0 + .truncate()`（或 `text_ellipsis`+`whitespace_nowrap`）。补齐清单：mixer 条标题（缺 nowrap）、routing 行、设备/进程名标题、matrix 表头/单元格、`detail()` 值列（#132 已改弹性换行）、工具栏页签、状态栏 elapsed/format。
- **页签弹性化**：`WorkspaceTab` 去掉固定 112px 宽，改 `min_w` + 标签 truncate——工具栏唯一不可缩元素。
- **绝对定位**：锚定浮层合法；**布局性绝对定位**只有 mixer 读数一处（§1.5 收敛后仍保留，但魔法数归组）。
- **compact 断点**：只在**进入** compact 时收起面板（恢复不强制展开）；断点/min 尺寸进 theme 常量。
- **margin vs gap**：容器内间距一律 `gap()`；现余 14 处 margin 中仅"居中半宽偏移"类合法。

## 3. 组件化（去重批次，对应 #123）

1. `anchored_popover`（锚定+视口钳制+翻转；蓝本 fault_popover.rs:146-158）+ `occluding_page`（全屏遮挡关闭件，6 处复制收敛；统一 outside-press 语义为子树拦截式）。
2. `scroll_region(id, handle, axis, child)`：`relative 包裹 + 滚动容器 + 兄弟 overlay 滚动条`三件套（12 处手写收敛；滚动轮通知已收敛为 `notify_owner`）。
3. 检查器 `section(title, body)` 包装（node_inspector ~15 处 heading 栈）与 `inspector_frame()`（三检查器共用壳）。
4. NodeKind 显示名注册表（承接 #131 的 kind_tint/kind_icon，补 `kind_title`，治 #121 的 `:?` 直出）。

## 4. i18n 约定（对应 #120/#121 + 审计深挖）

- **键+参数存储，渲染期翻译**：状态/错误消息存键与参数（或类型化枚举），显示时才 `t!`——换语言后无冻结文案。涉及 project 状态、settings 草稿消息、meter 错误态、dispatch_error（现三类混存）。
- **默认节点标签**：新节点默认名不存英文数据；存"默认"标记，渲染时用目录标题翻译（顺带消除 default_label 与目录标题的 9 处命名分叉）。
- **guard 测试**（仿 action_assets 白名单模式）：字面 `t!` 键↔yml 双向对账 + 占位符三方一致 + 注册表枚举完整性 + 源码检查（label/detail/heading 参数禁 `{:?}`）。
- **TSV 台账**：由 yml 生成或删除（已漂移：Arrange 缺失）；yml 去冗余 `en:` 行；build.rs 加 `rerun-if-changed=locales`。
- **直出清单**（#121 扩大到 11 处）：NodeKind/ExportDirection/StreamMode 显示名走注册表；错误消息暂以 format 收敛形状，键化随键+参数改造。

## 5. PR 序列（与 issue 修复合流）

| PR | 内容 | 对应 |
|---|---|---|
| P1 | 缓存键补全：ControlsKey+catalog/backend、Inspector/DevicesKey+草稿 PublishRef、观测读数入键 | #117 #118 |
| P2 | i18n 台账清理 + guard 测试 + rerun-if-changed | #120 |
| P3 | NodeKind kind_title + `:?` 直出收敛（注册表承接） | #121 |
| P4 | 绑定选择列表共享 + Wasapi 参数化 | #119 |
| P5 | overlay×6 组件化（anchored_popover + occluding_page） | #123a |
| P6 | scroll_region 三件套组件化 | #123b |
| P7 | 双 poll 合一（spawn_poll 复用 Desktop::pump 表达式） | #123c |
| P8 | monitoring 三表合一（泛型 SubscriptionTable） | #124（骨架已备） |
| P9+ | token 铺设（§1 分 3-4 个 PR：间距/文本/新系统/清理别名）、截断补齐（§2）、页签弹性、compact 断点治理 | 本设计 |

每 PR 门禁沿用：fmt/clippy/529 测试/29 截图 diff=变化清单（视觉改动逐处向用户报备）。

## 6. 不做

- 不改既有视觉基调（token 值=现值的命名化；alpha 统一>0.03 的差异除外，逐处报备）。
- 不引入 rem/预处理器风格 helper（保持 px+token 双层）。
- 引擎/服务层错误键化（172 条）单独立项，不在本设计内。
