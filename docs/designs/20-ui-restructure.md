# Design 20 — UI 代码重构：结构重组与 StudioShell 瘦身

状态：已批准（2026-11-08）。本设计是纯重构的执行蓝图：**零行为变化**，
每一阶段合入前后以截图基线逐像素对比为准。

## 1. 问题

`ui/` 共 ~43.5k 行，三类根源（全部经代码亲证）：

1. **组织与内容不符**：`pages/` 里没有页面（只有 Graph canvas 模型与绘制），
   真正的页面全在 `window/`（占 ui/ 的 66%）；`app_view.rs` 只定义 actions 不是
   view；每页有两个同名 `page` 函数；36 个文件依赖 `use super::*` 扁平命名空间。
2. **StudioShell 上帝对象**：95 个字段（#106 合入后），55+ 个方法散布 13 个文件
   的 15+ 个 impl 块。新增一个字段需要同步 4 处：struct 定义、~260 行构造器、
   所属 Panel 的缓存键、`PresentationSnapshot`/autosave 指纹。手势取消清单、
   `text_input_entities`、`slider_slots` 等穷举式遍历靠手工维护——这是审计
   SD-1/SD-2 类缺陷（新编辑器族漏登一处枚举点）的共同根因，代码注释自己承认。
3. **重复模式**：overlay 关闭脚手架 ×4、NodeKind→tint/icon 映射 ×4、绑定选择
   列表 ×2（~130 行平行代码）、滚动容器三件套 ×9、测试/生产双 poll 循环。

2026-10-31 审计（02-structure.md）曾把 `StudioShell` 列入"不拆清单"，理由是拆
struct 会把字段公开化。当时约 60 字段；如今 95。本设计**有意识地反转该决定**，
方式不是公开散字段，而是按域收敛为拥有的子 struct——仓库里已有两个成功先例：

- `mixer: MixerPresentation`：1 个字段管 20 个子字段，一个模块一个 impl 块；
- `TrayMixer`：完全不持有 shell 也能工作（app-model 与窗口表现分离可行）。

## 2. 不变式（不推翻的东西）

- 两级组件模型与它的全部纪律：PanelHost 键差分、HotSurface 动画帧、
  SharedFrame 几何通道、缓存面板只能兄弟不能嵌套、键中禁入流逝时间/音频观测、
  `.cached()` 内容根必须 w_full/h_full（design 17 + PR #104 教训）。
- Poll 数据流：`Desktop::poll` 33ms → `UiState::poll` → `cx.notify`，无事件机制。
- `Entity<UiState>` 作为跨窗口共享的 app 模型（TrayMixer 已如此消费）。
- 服务层、引擎、signal 层零改动；gpui-kit/fork 不动。

## 3. 目标模块树

```
ui/
├── actions.rs            ← app_view.rs 改名（它只定义 Quit/Undo/Redo actions）
├── state.rs / presentation.rs / monitoring/ / widgets/ / theme.rs / …（不动）
├── shell/                ← window/ 改名，只留 shell 基建
│   ├── shell.rs          ← window.rs 改名：StudioShell ≈17 个域字段 + Render
│   ├── chrome.rs desktop.rs project.rs(菜单) fault_popover.rs autosave.rs
│   ├── panel.rs hot.rs controls.rs native.rs sidebar.rs tray_mixer.rs
│   ├── test_harness.rs tests.rs capture.rs performance.rs
├── pages/                ← 真正的页面
│   ├── mod.rs            （Page 枚举）
│   ├── graph/            ← window/graph* 全部 + pages/graph_canvas*、
│   │                        graph_canvas_view*、node_catalog.rs
│   │   （canvas/ library/ inspector/ detail/ binding_menu/ compressor/
│   │     interactions/ headline/ …保持子模块结构）
│   ├── mixer/            ← window/mixer*（model/page/routing/strip_controls）
│   ├── matrix/           ← window/matrix.rs
│   ├── devices/          ← window/devices*
│   └── settings/         ← window/settings*
```

规则：

- 全部 `git mv` 保历史；移动后的文件把 `use super::*` 换成显式导入
  （PR1 允许机械的最小替换，PR2 起按域收紧可见性）。
- 每页两个 `page` 函数统一命名：挂载函数 `mount`，内容函数 `page`。
- element id、`gpui_test_observe` 标记、MeterTrace 计数器字段一律不改名——
  测试与截图基线依赖它们。

## 4. StudioShell 字段映射（95 → ~17）

| 新字段（类型） | 吸收的现有字段 |
|---|---|
| `state: Entity<UiState>` | 不变 |
| `chrome: ShellChrome` | presentation（page/visible/minimized）、project_menu、sidebar_open、inspector_open、compact、density、toolbar_chrome、status_chrome、fault_popover |
| `selection: Selection` | selected_node、selected_nodes、selected_route（主选中不变式收敛为方法） |
| `graph: GraphPageState` | camera、node_positions、node_drag、selection_drag、connection_drag、knob_drag、pan_grab、background_click、right_press、node_drag_snap、drag_guides、context_menu、graph_focus、graph_canvas_frame、graph_overlays、canvas_size、hovered_node、spectrum_scratch、binding_menu、pending_node_spawn、node_search、hand_tool、graph_controls、graph_meter_frame、detail_node、binding_menu_scroll |
| `mixer: MixerPageState` | mixer（MixerPresentation 原样内嵌）、mixer_geometry、mixer_scroll、dock_scroll、hovered_strip、mixer_frame |
| `matrix: MatrixPageState` | matrix_scroll |
| `devices: DevicesPageState` | device_tab、selected_device、export_controls、device_list_scroll、device_override_scroll、exports_scroll、devices_panel |
| `settings: SettingsPageState` | settings_open 并入 chrome 的 WorkspaceView 枚举；editor、value_menu、scroll、settings_panel |
| `inspector: InspectorState` | node_name、node_name_editing、compressor、inspector_meters、inspector_meter_layer、inspector_scroll、inspector_panel |
| `library: LibraryState` | library_scroll、library_rail_scroll、library_panel、library_rail_panel |
| `preferences: PreferenceStore` | preferences、preferences_path、preference_save、preference_save_revision、preferences_quitting |
| `autosave: Autosave` | autosave_path、autosave_stamp、autosave_deadline、autosave_timer、autosave_write |
| `project_io: ProjectIo` | project_path、project_name、project_message、project_busy、pending_project |
| `meters: MeteringInfra` | meters（MeterSubscriptions）、node_meter_points、meter_points、frame_demand、observation_nodes |
| `press: PressTracker` | last_element_press（跨页双击检测） |
| 原样保留 | desktop、window_state、touchpad(cfg windows)、test_page/trace(cfg gpui-test) |

分组规则：

- 组字段 `pub(in crate::ui)`；组的读写方法定义在拥有它的模块，不放 `impl
  StudioShell`（新代码零新增 shell impl 方法，存量方法随分组迁移为组方法）。
- 跨组访问白名单：`state`、`chrome`、`selection`、`preferences`、`meters`、
  `press`。页面组之间互访 = 设计违规。
- `WorkspaceView` 枚举取代 `settings_open + presentation.page` 两态：
  `Workspace(Page::Graph|Mixer|Matrix|Devices) | Settings`。
- 手工穷举清单收敛：`cancel_active_gesture`、`begin_pointer_session`、
  `set_page` 的取消列表、`text_input_entities`、`slider_slots` 改为各组自答
  （`graph.cancel_gesture()`、`graph.text_inputs()`…），shell 只做拼装。
  新增手势/编辑器字段漏登枚举的缺陷类别从此在编译器视野内。
- `PresentationSnapshot` capture/restore 改为按组进行（每组一个
  `capture()/restore()`）；autosave 指纹同理按组取材。新增字段的爆炸半径
  从"4 处同步"缩到"1 个模块内"。

## 5. PR 序列（每 PR 独立可合并、CI 全绿、零行为变化）

| PR | 内容 | 风险点 |
|---|---|---|
| PR0 | 本设计文档 | 无 |
| PR1 | 目录重组（§3 树，git mv + 显式导入 + mount/page 命名） | 纯机械；截图基线必须 29/29 |
| PR2 | matrix 域分组（最小试点，验证方法论） | MatrixPanel 无缓存 |
| PR3 | devices + settings 域分组 | DevicesKey/SettingsKey 路径、settings_open→WorkspaceView |
| PR4 | preferences/autosave/project_io 持久化组 | PresentationSnapshot、autosave 指纹 |
| PR5 | inspector + library 组 | InspectorKey 16 输入、编辑器族枚举 |
| PR6 | mixer 组 | StripKey 手写 PartialEq、DispatchGate |
| PR7a/7b | graph 组（最大，拆两半：交互态 / 投影与面板） | ControlsKey、SharedFrame、OverlayRects |
| PR8 | chrome + selection 组 | ToolbarKey/StatusbarKey、双击检测 |
| PR9 | meters + 生命周期组 + constructor 收尾 | pump_meters、frame_demand、observation_nodes |
| PR10 | render 头部 10 个 sync 钩子聚合为 `sync_presentation()` 单入口（只聚合不改时机；"sync 移出 render"另立决策） | IN-1 类时序敏感 |
| PR11+ | 去重：overlay 关闭组件化、NodeKind 注册表、绑定列表共享、`scroll_area()` 三件套、合并测试 poll 双胞胎 | 各自独立小 PR |

## 6. 截图验证（本重构的行为红线）

工具：`cargo build --locked -p moiren-app --features gpui-test --example
ui_capture` → `target/debug/examples/ui_capture.exe <outdir>`，默认模式一次
产出 29 张 PNG（4 页 × comfortable/compact、设置 3 区、设备覆写、图谱 5 种
缩放 + 故障提示 + 绑定菜单 + 频谱、4 种检查器、节点家族）。

已在本机验证**同 commit 连跑两次 29/29 逐像素一致**（2026-11-08，
commit 3218cb8，见 `D:\coding\moiren-shots\BASELINE-MANIFEST.txt`）。因此：

- 基线：`D:\coding\moiren-shots\baseline-a\`（29 张，仓库外存放防误提交）。
- 每个重构 PR 在本地跑一遍采集到 `moiren-shots\<pr-label>\`，与基线做
  逐像素 diff（`python diff_shots.py baseline-a <candidate>`，零容差）。
  29/29 一致 = 该 PR 零 UI 变化，可交付合并；任何不一致必须解释或返工。
- 环境指纹（同机、同显示器配置、preferences.json sha256）随 manifest 记录；
  diff 出现全量噪声时先核对指纹是否漂移（用户在实机 App 里改了偏好）。
- CI 不跑截图（GPU/显示环境不可控）；截图门是本地交付标准，diff 结果贴进
  PR 描述。
- MeterTrace 计数器断言（strip_renders/graph_renders/inspector_renders）与
  473 个 gpui 测试在 CI 侧作为第二道护栏。

## 7. 明确不做 / 后续方向

- 不在重构 PR 里修既有缺陷。已确认待修（转 issue）：ControlsKey 缺
  catalog/backend 输入（IN-3）、InspectorKey 缺设备草稿字段与观测读数
  （IN-8 族）、DevicesKey 缺 settings 草稿、绑定菜单/检查器双份列表硬编码
  Wasapi（ST-4/IN-4）、43 个 i18n 死键、`:?` 调试格式直出界面 3 处。
- 后续独立方向：monitoring 三表泛型合一；entity-per-page（TrayMixer 模式
  推广，把页面从 shell 观察者变成独立 entity）；sync 钩子彻底移出 render；
  i18n 键双文件单一化。

## 8. 与既往决策的关系

- 反转 2026-10-31 审计 02-structure.md 的"StudioShell 不拆"决定（60→95 字段，
  且方案从"公开字段"变为"域分组 + 组内方法"）。
- 采纳同审计 06-consolidation-plan 的 ST-3/ST-4/ST-6 相关项为 PR11+。
- design 17 的全部不变式原样有效；本文不修改两级组件模型的任何规则。
