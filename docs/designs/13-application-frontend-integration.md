# 应用前端接入

本轮以 `FRONTEND.md` 和现有 gpui 组件库为基础，自行安排页面，不复制 Figma 页面。用户授权先补齐缺失后端。

## 设计与范围

深色音频工作站：canvas `#090b0f`、panel `#11151b`、raised `#1b2028`、border `#2a303a`、brand `#7549c6`、text `#f6f8fa`。使用嵌入 Inter，数值使用 Cascadia Mono。紫色用于操作与选择，输入、输出、候选和错误沿用公共语义色。

单层应用栏提供品牌、项目菜单、Graph/Mixer/Matrix/Devices、启停、Settings 和窗口操作。主体左对齐，工具属于各工作区，底部显示服务阶段及真实诊断。

```text
Logo  Project       Graph Mixer Matrix Devices      Start Settings Window
┌ Session / Library ┬ Graph / Mixer / Matrix / Devices ┬ Inspector ┐
│ 真实资源与操作     │ 主要工作面                       │ 选择详情    │
└──────────────────┴─────────────────────────────────┴───────────┘
Phase  elapsed  format  block  revision                 diagnostics
```

1024×720 收起侧栏为可切换抽屉，保留工作面；1440×900 展开。Mixer 横向滚动，Matrix 两轴滚动并保留表头。Settings 覆盖工作区，关闭恢复此前选择与相机。

## 数据边界

- Service 拥有真实 LogicalGraph；三个工作区只投影快照并提交事务，不维护第二份拓扑。
- 输入、输出选择是下次启动候选；活动会话身份来自服务，不因候选变化而变。
- 节点参数/连接提交记录 Requested、Accepted、Applied 和实际 revision/frame；回执前不能宣称活动。
- 实现后端支持范围内的拓扑编译、项目保存/加载、偏好保存和真实电平订阅。其他能力保留明确不可用说明。
- 不将未知电平、CPU、延迟或 XRUN 显示成零，不将边增益当成推子值。

## 实施与验收

1. 扩展共享 UI 状态与主题，拆分 shell 的 chrome、graph、mixer、matrix、devices、settings。
2. 并行实现 service 权威拓扑/编译、项目与偏好保存、真实源与输出的电平读取。
3. 接入全部导航、资源选择、启停、Inspector、参数、拓扑操作和文件操作，使用公共按钮/图标。
4. gpui headless 验收必须渲染真正的 shell，而不是测试专用占位页面；覆盖切页、Settings 返回、候选/活动身份、Matrix 和参数提交。
5. 执行 workspace check/test、gpui-test、fmt 和严格 Clippy。实际启动应用，检查 1440×900 与 1024×720 的各工作区并保存截图。

## 交付与验证结果

五个页面已接入公共 shell。Graph 的节点、曲线、Inspector 和 Minimap 取自同一服务快照；Mixer 的推子、Balance、Mute、Dim、Mono、Swap 提交真实参数；Matrix 直接编辑真实端口与边，内部网格保持直角无描边。Devices 展示真实设备、应用与活动时钟，Settings 保存密度并保留原工作区。

后端支持 Gain / Pan / Bus / Compressor 的候选图准备与启动。拓扑应用目前通过停止并重新准备会话完成，参数可在兼容活动节点上实时生效。载入项目的节点即使复用了 NodeId 数值，也不能控制此前仍在运行的项目。多个 IO 节点可作为候选保存，当前 native driver 仅启动单 Source、单 Sink 的双声道图。Compressor 使用默认设置；没有 publisher 的 CPU、端到端延迟与 XRUN 保持未知。

| 验证 | 当前结果 |
| --- | --- |
| `cargo build --locked --workspace` | 通过 |
| `cargo check --locked --workspace --all-targets` | 通过 |
| `cargo test --locked --workspace` | 696 passed；3 项原有硬件测试保持 ignored |
| `cargo test --locked -p moiren-app --features gpui-test` | 176 passed；1 项原有硬件测试保持 ignored |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test -- -D warnings` | 通过 |
| 原生 UI 导出 | 两种逻辑尺寸、五个页面共 10 张 PNG，均已目视检查 |

UI 验收使用真正的 StudioShell，覆盖四个工作区导航、Settings 返回与选择保留、紧凑抽屉、大矩阵的冻结表头，以及实际曲线点击后的路由 Inspector。新增回归还覆盖保存不可用设备身份、无 Session 模板清理旧选择、项目加载等待服务确认、稀疏文档节点 ID、请求拒绝保留原布局。电平测试使用真实音频窗口，检查隐藏、滚动、会话切换、旧缓存与 stale 数据。

截图由 GPUI 原生 `render_to_image` 导出，不依赖浏览器或 Figma。当前 Windows 自动化接口不可用，交互通过 headless 的真实 GPUI 事件测试验证，文件与参数后端另有独立回归。截图中的设备名、电平和阶段来自本机真实快照；它们不是固定示例数据。显示器缩放为 125%，因此逻辑尺寸与 PNG 物理像素不同。

| 工作区 | 1440×900 | 1024×720 |
| --- | --- | --- |
| Graph | [Comfortable](screenshots/application/comfortable-graph.png) | [Compact](screenshots/application/compact-graph.png) |
| Mixer | [Comfortable](screenshots/application/comfortable-mixer.png) | [Compact](screenshots/application/compact-mixer.png) |
| Matrix | [Comfortable](screenshots/application/comfortable-matrix.png) | [Compact](screenshots/application/compact-matrix.png) |
| Devices | [Comfortable](screenshots/application/comfortable-devices.png) | [Compact](screenshots/application/compact-devices.png) |
| Settings | [Comfortable](screenshots/application/comfortable-settings.png) | [Compact](screenshots/application/compact-settings.png) |
