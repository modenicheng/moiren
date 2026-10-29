# Moiren App

运行 `cargo run --locked -p moiren-app` 打开原生 gpui 桌面应用。

- Graph、Mixer、Matrix、Devices 共用服务端的 LogicalGraph。Settings 从齿轮进入，返回时保留工作区与选择。
- 在 Devices 选择下次启动的物理输入、应用音源与输出；选择不会替换当前活动会话。Graph 的 Session 面板也可选测试音。
- Graph 添加、删除节点，Inspector 调参数与 Bus 输入；在 Matrix 选择端口交点并连接、断开或编辑路由增益。图编辑自动应用：兼容更新经 EngineHost 在块边界生效并保留原音频 owner，界面区分候选与引擎确认的活动 revision。设备或 IO 集合变化可以重建会话。
- Node library 现在可创建 EQ、Gate、Limiter、Delay、Channel Mapper、Recorder、Meter、Spectrum Analyzer 和 Rack；Inspector 通过服务端候选图编辑真实配置。观察节点的 recording、metering 和 spectrum 资源由托管 host 自动准备，编辑期间保持 candidate 状态。
- Mixer 的推子、Balance、Mute、Dim、Mono、Swap 提交真实音频请求，反馈区分 Requested、Accepted、Applied。Peak、RMS 仅显示对应节点的真实观测。
- 项目菜单可打开、保存完整候选图和布局；加载不启动音频。密度和延迟档位偏好保存在用户配置目录，项目缺失设备仍保留稳定身份。

Windows 桌面驱动支持多输入、多输出的双声道图，以及 Gain、Pan、Bus、Compressor。每个 IO 节点独立绑定资源；首个输出为 master，其余输出使用独立时钟桥。Settings 可选择 Compatible / Balanced / Low 延迟档位，保存后下次启动生效（见[托管路由与低延迟](../../docs/designs/14-managed-routing-low-latency.md)），并查看各原生流的实际周期与时钟统计；CPU、端到端延迟和 XRUN 没有对应观测时显示 `—`。

Graph 节点位置属于展示状态；拖动节点不会改变拓扑。`Fit` 使用实际节点边界，滚轮以光标为锚缩放，触控板双指滚动与捏合分别平移、缩放，也可用中键拖动或 Hand 工具平移。小窗口默认收起 Session / Inspector，使用页内按钮打开；Mixer 可横向滚动，Matrix 表头随滚动保持可见。

前端验证：

```powershell
cargo test --locked -p moiren-app --features gpui-test
cargo run --locked -p moiren-app --features gpui-test --example ui_capture
```

`ui_capture` 使用真正的 shell 和 GPUI 原生渲染器，在 1440×900、1024×720 逻辑尺寸分别导出五个页面，然后退出。图像保存在 `docs/designs/screenshots/application/`；物理像素随显示缩放比例变化。

headless 应用负责 engine 的装配、控制和生命周期，默认运行软件 IO 全链路：

```text
interleaved input → AudioWriter → InputNode → Gain → OutputNode → AudioReader → interleaved output
```

在仓库根目录运行：

```sh
cargo run --locked -p moiren-app
cargo run --locked -p moiren-app -- --gain 0.25
cargo test --locked -p moiren-app
```

默认演示 stereo f32、48 kHz Processing SR、可变 block 和一个 planar slab；输入和输出由独立预分配 sample ring 隔离。程序打印输入、处理后样本和边界诊断，不打开音频设备。

library 的 `OfflineApp::process_interleaved` 接受等长、完整 frame 的输入/输出 slices，超过最大 block 自动拆分；空请求不推进时间。`set_gain` 经控制队列下发，`poll_applied` 返回运行确认；持续更新参数时需消费确认队列。`input_status` / `output_status` 返回有界队列中最新可用快照，队列满时的新快照可能已丢弃。

`OfflineApp` 是同步离线 owner。`stop` 在处理结束后释放 engine 与桥。每块 slab/bridge 的 sample storage 上限为 8 MiB，队列及 metadata 不计入该上限。

## 窗口与托盘

Windows 标题栏的 X 和 Alt+F4 会销毁桌面窗口、视图及 UI 仪表订阅，不是最小化；后台音频和托盘继续运行。点击托盘或选择 `Show Moiren` 会新建窗口，并恢复当前工作区、图布局和候选项目展示状态。最小化按钮仍只最小化；托盘的 `Exit` 才退出进程并回收音频服务。托盘创建失败时，关闭最后一个窗口会退出，避免留下不可访问的后台进程。

窗口正常位置、大小、最大化状态和工作区独立保存在 `%APPDATA%\Moiren\window.json`，关闭和退出时原子写入；不会把最小化状态当成下次启动状态。恢复时使用当前显示器工作区校正位置和尺寸。此文件不保存或自动加载音频项目，也不改变 `preferences.json` 的兼容性。

标题栏 Logo 和工作区标签两侧的空白区域可以原生拖动窗口、双击最大化或还原；项目菜单、工作区标签和操作按钮不参与拖动。最大化按钮依据 Windows 当前状态在最大化与还原之间切换，保留正常窗口的恢复位置。

托盘使用已有透明 Logo：Windows 系统浅色主题为紫色，深色主题为米白，主题切换后最多约一秒更新，与应用 UI 的主题偏好无关。应用和 EXE 图标固定使用紫色 Logo 与米白圆角背景；构建时从 `assets/logos/` 的 SVG 生成 16–256 像素多尺寸 ICO，无需运行时 SVG 解码。原始品牌 SVG 保持不变。
