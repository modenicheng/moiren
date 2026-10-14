# Handoff：Slint → gpui 迁移可行性评估

- **日期**：评估完成于当前会话（基于 `main` @ e2c5716，工作区含未提交改动）
- **状态**：可行性结论已出，**尚未开始任何迁移代码**。下一步是 1–2 周尖刀验证（见 §7）。
- **性质**：本文档是决策 handoff，不是迁移方案。所有 gpui 结论均基于源码级核实（zed `gpui-v0.2.2` tag + crates.io 元数据），来源见 §10。

---

## 1. 背景与动机

考虑放弃 Slint 换 gpui 的三个理由：

1. **不想引入 DSL**：Slint `.slint` 是独立语言，AI 辅助编写时常产出编译不过的语法；gpui 纯 Rust，编译器全程兜底。
2. **许可证不适配**：Slint Royalty-free 许可证与项目诉求不匹配；gpui 核心是 Apache-2.0。
3. **AI 写不明白 Slint**：部分成立——但注意 gpui 也有自己的 AI 风险（§5.4），不要高估收益。

**硬性约束**：Windows 触摸板捏合缩放（pinch-to-zoom）是**必须项**。Graph 画布的平移/缩放交互依赖它（`src/ui/graph_input.rs` + `src/ui/graph_input/windows.rs`）。

## 2. 迁移账本（现状盘点）

| 资产 | 规模 | gpui 迁移路径 |
|---|---|---|
| `.slint` 声明文件（17 个） | 4,324 行 | 全部重写为 Rust div/element 树 |
| Rust UI 胶水（`src/ui/`） | ~3,750 行 | 大部分重写；`bindings.rs` 的 45+ 个 `set_` 推送变为 Entity + `cx.notify()`（更简洁） |
| `native_acceptance.rs` + 软件渲染 headless 测试（~1,305 行） | `graph_input/tests.rs` 等 | **全部作废**，需基于 `#[gpui::test]` 重建（最大隐性成本） |
| `graph_input.rs` + `graph_input/windows.rs`（615 行） | winit 拦截 + Win32 触摸板桥 | 按 gpui Win32 后端重写；结论见 §6 |
| 33ms Timer 快照轮询（`window.rs:146`） | — | Entity + notify / 每帧 render 拉取，模式更自然 |
| 系统托盘（`ui/tray.slint`，SystemTrayIcon） | 16 行 | gpui **无 tray API**，需引 `tray-icon` crate 自行桥接 |

总量估算：**4–8 周专注工作**（UI 重写 ~5k 行 + 测试重建 ~2k 行 + 托盘/手势集成）。

## 3. 两块重点 UI 的可行性（用户重点关注）

### 3.1 Graph 画布 —— 可行，部分比 Slint 更顺手

- **网格/平移/缩放**：现在 Slint 用 `for column in ...` 循环实例化几十个 Rectangle 画网格线（`graph-canvas.slint:211-220`），写法勉强。gpui 用 `canvas()` paint 回调或自定义 Element + `paint_quad` 一次遍历完成，更短更快。
- **节点卡片**：div + taffy flexbox，与 Slint 布局语义 1:1 对应；拖拽用 `on_mouse_down/on_drag`。
- **MeterRegion 回传协议**（`signal-meter.slint` 的 `absolute-position` + `region-changed`，`main.slint:84-101`）：这是为弥补 Slint 能力发明的双向协议，gpui paint/prepaint 天然知道自身 bounds，**协议可整体删除**，可见性门控（"可见才订阅"）在 prepaint 里做。
- **连线（GraphWire）**：Slint `Path` 有 SVG 风格 bezier + stroke-width；**gpui 0.2.2 的 `Path` 只有 `move_to/line_to/curve_to/push_triangle`，`paint_path` 只填充，顶点结构（`PathVertex`）无 stroke 宽度**。1.6px 连线需自己把 cubic Bezier 采样 ~16 段、沿法线偏移生成 quad strip，约 40–60 行标准数学代码。
- **自定义 Element**（`request_layout/prepaint/paint` 生命周期）是迁移中技术含量最高部分；官方文档薄（自述"读 Zed 源码是最好学习方式"），但范例充足（30 个官方 example + Zed 源码）。

### 3.2 自绘组件（电平表/旋钮）—— gpui 强项

- **RMS/PEAK 条**：现实现就是"宽度=level 比例的 Rectangle + 1px peak 线"（`graph.slint` meter 段）。gpui `paint_quad` 直接画。每帧整树重建的模式对 30fps 电平表天然契合。
- **旋钮（DevReadyKnob）**：现在靠 `transform-rotation + transform-origin` 旋转 Rectangle 模拟（`dev-ready.slint:271`），hack。gpui 有 `TransformationMatrix::rotate` 和 path 弧线，写法更正经。
- 项目无重度依赖高级控件（无复杂表格/树/富文本），恰好避开 gpui 最弱区域。

## 4. 许可证核实结果（细节重要）

- **crates.io `gpui 0.2.2` 依赖树干净**：它依赖独立发布的 `gpui_sum_tree 0.2.2`（依赖仅 arrayvec/log/rayon），**不经过** zed 仓库 main 上 `sum_tree → ztracing(GPL-3.0)` 的污染链（该问题 = zed issue #55470，只影响 git 依赖 main 分支）。其余依赖 resvg/usvg(MPL-2.0)、taffy、lyon 等均可商用。
- **两条红线**：
  1. **禁止 git 依赖 zed main**（会静态链接 GPL-3.0 的 ztracing/zlog）；只允许 crates.io `=0.2.2` 或 gpui-kit 的 `gpui-pre` 快照分叉。
  2. **禁止使用 Zed 自家 `crates/ui` 组件库**（GPL-3.0 且未发布）。反正本项目靠自绘 + gpui-kit，不受影响。

## 5. 真实风险清单（按严重度）

1. **Headless 测试体系重建**（最大隐性成本）：现有验收测试建立在 Slint 软件渲染 + `MinimalSoftwareWindow` + 合成输入之上。gpui 需改 `#[gpui::test]`（确定性模拟输入）或 gpui-kit 的 headless 集成测试。约 1–2 周。
2. **版本断崖**：crates.io 卡在 0.2.2（2025-10 后无发布），zed main 已重构为 crate 家族（wgpu 渲染、AccessKit、taffy 0.10）。生态主力 **gpui-kit**（longbridge，Apache-2.0，75+ 组件，含带 IME 的文本输入、AccessKit、虚拟列表、UI 集成测试）绑定 `gpui-pre 0.3.9` 快照分叉。**建议直接押 gpui-kit 生态**：0.2.2 无文本输入、无无障碍，搜索框/数值输入不用手写（官方 input.rs 示例手写文本框要 746 行）。
3. **无障碍**：0.2.2 为零；main 的 AccessKit 已合入但未发布且 Zed 自己暂时 opt-out。现开了 Slint `accessibility` feature，若为产品需求只能走 gpui-kit（声称内建 AccessKit 角色）。
4. **AI 辅助风险**：gpui 的 Entity/Context API 形态独特，0.2.2 与 main 大幅分叉导致训练数据混杂，AI 会大量产出错误版本的 API——错误能被编译器抓住，但迭代轮次未必比 Slint 少。
5. **系统托盘**：见 §2，用 `tray-icon` 桥接，成熟但需自担集成。

## 6. Windows 捏合手势（必须项）—— 核实为可移植，非一票否决

**这是本评估最重要的源码级核实结论**：

1. **HWND 可得**：gpui `Window` 公开实现 `HasWindowHandle`（`window.rs:4845`，走 `platform_window.window_handle()`），docs.rs 公开 API 含 `raw_window_handle`。现有 `SetWindowSubclass` 桥接只依赖 HWND。
2. **gpui 的 wnd_proc 对手势更友好**：gpui Windows 后端 `handle_msg`（`events.rs:34-112`）**没有 WM_POINTER 分支**，未知消息直接 `DefWindowProcW`——系统手势识别天然保留。对比 winit 0.30 会**消费** WM_POINTER、逼我们 subclass 抢先放行，**gpui 下连拦截都不需要**，桥接代码更简单。
3. **MK_CONTROL 守卫可照搬**：gpui `handle_mouse_wheel_msg` 从键盘追踪取 modifiers，检测不到捏合合成的 Ctrl。现有线程局部 `WheelModifierGuard` 模式（subclass 设 flag → DefSubclassProc → gpui 同步派发 → 监听器读 `wheel_control()` → Guard 恢复）在 gpui 下同样有效，链路同步、无时序变化。
4. **缩放锚点**：`mouse_position()` / `layout_bounds()` / `scale_factor()` 均为公开 API。
5. **已知降级点**：现 winit 路径 1 的 `PinchGesture`（DirectManipulation 合成、真连续手势）在 gpui 无对应，只剩路径 2（捏合→Ctrl+wheel 合成 + `exponential_zoom_factor` 指数平滑）。高频小增量 line delta 主观手感应接近，但混合多指手势（平移+缩放同时）精度不如 DirectManipulation。**尖刀期须实测手感**；不达标则升级为在 subclass 的 HWND 上自做 DirectManipulation（COM，约 300–500 行 unsafe，团队已证明有能力驾驭——现有 `windows.rs` 就是 180 行手势桥接）。

## 7. 下一步：尖刀验证（1–2 周，出 go/no-go）

依赖锁定二选一：`gpui = "=0.2.2"`（crates.io）或 gpui-kit + `gpui-pre =0.3.9`（推荐，理由见 §5.2）。**写进 Cargo 政策：禁止 zed git 依赖（§4 红线）。**

验收项（按用户重点排序）：

1. **GraphCanvas Element**：网格 + 滚轮 pan/zoom + quad-strip 贝塞尔连线 + 节点拖拽（§3.1）。
2. **电平表 + 旋钮**：30fps 数据源驱动，验证 notify/重绘路径（§3.2）。
3. **捏合移植**（go/no-go 关键）：`open_window` 回调里 `RegisterTouchpadThread`（user32 序号 2688）+ `window_handle()` 取 HWND 装 subclass + MK_CONTROL 守卫 + 滚轮监听器读 flag；触摸板实测手感；不达标再评估 DirectManipulation 工作量。
4. **配套验证**：`tray-icon` 集成、gpui-kit Input + 中文 IME（Windows）、一条 `#[gpui::test]` headless 用例。

若尖刀期 DirectManipulation 集成被判定必须且受阻，是唯一可能一票否决的技术点——其余均有成熟路径。

## 8. 总体结论

**可行性：中高偏正面。** UI 构成（自绘仪表 + 图画布 + 少量表单）落在 gpui 强项；捏合（原唯一一票否决候选）核实为可移植且比 Slint 下更简单。剩余风险集中在测试重建（确定性成本）与版本断崖（押 gpui-kit 缓解）。

## 9. 决策需要但本次未覆盖

- Slint 侧的长期收益保留方案（若最终 no-go）：无。
- 电平表 30fps 以外的高帧率需求（gpui 每帧重绘 vs Slint 局部重绘的功耗对比）：未测，尖刀项 2 可顺带记录。
- 多显示器 DPI 变化下的表现：未验证。

## 10. 证据来源

- zed `gpui-v0.2.2` tag 源码：`src/window.rs`（`HasWindowHandle for Window` L4845）、`src/scene.rs`（`Path`/`PathVertex` 无 stroke，L680–880）、`src/interactive.rs`（`ScrollWheelEvent` L363）、`src/platform/windows/events.rs`（`handle_msg` L34–112，无 WM_POINTER；`handle_mouse_wheel_msg` L530+，Lines delta + 键盘 modifiers）、`src/platform/windows/window.rs`（wnd_proc L1180+）、`src/platform.rs`（`get_raw_handle`）、`src/elements/canvas.rs`。
- crates.io 元数据：`gpui 0.2.2` 依赖列表（`gpui_sum_tree 0.2.2` 干净依赖）、gpui-kit 0.7.x 绑定 `gpui-pre =0.3.9`。
- zed issue #55470（GPL 污染仅 git main 路径）；Zed Windows GA 博客（D3D11 一等公民）。
- 第三方生态报告 GoldStrikeArch/rust-gui-desktop-ecosystem-state `03-gpui.md`（版本断崖、AccessKit 时间线、gpui-kit 生态位）。
- 本仓库：`crates/moiren-app/ui/**`、`crates/moiren-app/src/ui/**`（行数、机制定位）。
