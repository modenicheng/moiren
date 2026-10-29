# 第一批 gpui 公共组件

> 状态：现行参考（gpui 组件契约；总览见 FRONTEND.md）。

2026-10-14，基于 FRONTEND.md 和当前 Figma 母版实现。母版说明的 Runtime-ready 表示设计与控件契约可用于开发；页面运行时能力仍以 Rust 服务为准。

| Figma 母版 | Rust 组件 | 范围 |
| --- | --- | --- |
| Button / Filled，49:238 | ActionButton | Primary / Danger，3 个尺寸 |
| Button / Quiet，51:358 | ActionButton | Secondary / Ghost，3 个尺寸 |
| Icon Button / Standard，52:448 | IconButton | Neutral / Accent / Danger，Small / Medium |
| Icon Button / Toggle，54:488 | ToggleIconButton | 受控 Selected，保留独立指示点 |
| Window Button，56:512 | WindowControlButton | Minimize / Maximize / Close；动作由窗口所有者绑定 |
| Workspace Tab，59:542 | WorkspaceTab | 受控选择、下划线、可选图标、禁用状态 |

公共入口由 `moiren_app::ui` 导出。颜色、字体、几何与密度在 `src/ui/theme/controls.rs`，绘制规则在 `widgets/action_style.rs`。按钮与 Toggle 复用 gpui-kit 的 Base 控件，支持原生鼠标、Tab、Enter / Space，并使用各状态的完整配色。图标为原始 Figma SVG，Inter 字体内嵌。

主应用的工作区导航、窗口按钮和 Session 操作已使用新组件。Start / Stop 回调继续进入现有服务；没有输出设备时禁用启动。原 StudioButton 的禁用状态只改变外观，仍触发回调，现已修复并加入回归测试。候选请求与确认选择分别由调用方管理；Toggle 不自行确认引擎状态。

现有 Graph、参数控件和其他页面仍使用技术验证时的实现。本批没有实现字段、反馈浮层、Graph / Mixer / Matrix 母版，也没有改变音频图、设备策略或关闭窗口的生命周期策略。

## 原生预览

```sh
cargo run --locked -p moiren-app --example component_preview
cargo run --locked -p moiren-app --example component_preview -- --group=icons
cargo run --locked -p moiren-app --example component_preview -- --group=navigation
cargo run --locked -p moiren-app --example component_preview -- --small --compact
```

预览不启动音频服务。上方切换组件组和 Comfortable / Compact，下方可实际点击、用键盘激活、切换 Toggle 与工作区。尺寸不足时使用滚动查看完整状态表。

## 验证

Headless 测试覆盖 Disabled 回调、每次鼠标 / Enter / Space 激活只派发一次、Toggle 请求与确认分离、禁用控件不抢焦点、Workspace 选择语义、密度尺寸与长标签。视觉检查使用真实 Windows 窗口，区分 GPU 渲染与 headless 事件验证。

对应截图存放在 `docs/designs/screenshots/2026-10-14-actions/`。Windows 当前显示缩放为 125%，截图像素尺寸与逻辑窗口尺寸不同。

视觉证据：[Buttons](screenshots/2026-10-14-actions/buttons.png)、[Icons](screenshots/2026-10-14-actions/icons.png)、[Navigation](screenshots/2026-10-14-actions/navigation.png)、[1024 × 720 Compact](screenshots/2026-10-14-actions/compact-small.png)。小窗口将两列预览改为纵向排列，通过滚动查看后续控件。

已通过 workspace check、workspace tests（启用 gpui-test）、严格 Clippy 和 rustfmt；硬件专用 ignored 测试未运行。
