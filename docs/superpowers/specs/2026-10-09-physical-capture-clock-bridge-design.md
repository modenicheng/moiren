# 单物理 Capture 与最小 Clock Bridge

日期：2026-10-09。基线：`c581bbf`。本切片实现 Windows 后端主线的首次 `Capture → Graph → Render`，UI 由用户在 Figma 独立设计。

## 决策与范围

沿用 [Windows 集成计划](../../plans/2026-10-08-windows-integration-plan.md) 的 W07 + W09 顺序。先实现单物理输入与单 Shared 输出，随后分别实施 Process Loopback、多输入/输出与恢复。直接复用 sample ring 的透传不能补偿独立设备漂移；将设备放进 Graph 会破坏现有 owner 边界。因此采用独立 Capture owner + backend Clock Bridge + `RtAudioSource<f32>`。

输出维持现有原生 48 kHz stereo f32 Render 主时钟。输入支持原生 44.1/48 kHz、mono/stereo f32；mono 明确复制到左右声道。拒绝其他 PCM、声道 mask 和采样率，不修改设备格式、系统音量、默认设备或其他应用 session。首版 SRC 使用连续相位线性插值，是功能基线，尚不满足专业音频重采样质量要求。

## 所有权与数据

Capture worker 自行初始化 MTA、打开指定 `eCapture` endpoint、协商 native mix format、创建事件与服务。`GetBuffer`/`ReleaseBuffer`、Start/Stop 及最终 COM Release 均留在该线程。所有退出路径停止 stream 后再释放 COM、最后 `CoUninitialize`。共享资源只有 stop kernel handle、固定容量 SPSC 和标量 atomic telemetry。

Bridge 在准备阶段分配完整 stereo frame ring。每帧附带 generation，producer 遇到 DATA_DISCONTINUITY、设备位置跳变或短写后改变 generation，reader 不跨 generation 插值。SILENT packet 写入真实零帧；非有限样本替换为零并计数。TIMESTAMP_ERROR 只使时间戳无效，仍保留有效 PCM。packet position 按 frame 理解，QPC 按 100 ns 理解，不用它们冒充已测出的硬件频率。

Reader 先等待 target fill，再以 render demand 拉取输入。启动/重新预填充时只保留 target fill 的最新缓存，单独累计被主动丢弃的旧帧；离线调用者可禁用此策略。比例为 `input_sr / 48000 × (1 + correction)`；fill 误差经低通与有界 PI 控制，限制 ±2000 ppm，积分抗饱和，保持相位连续。欠载清零未交付尾部并重新预填充；溢出丢新尾部，绝不等待另一个线程。generation 改变时清除插值历史并重新预填充。停止/重新绑定必须创建新桥，不能沿用旧设备缓存。

## 控制与状态

`CaptureOptions` 要求显式 endpoint ID 和 1..600 秒。`CaptureSession` 支持 stop/join，Drop 会停止并 join；启动握手在非 RT 控制侧反馈格式/初始化失败。Bridge observer 暴露累计采集、写入、丢帧、静音、非有限采样、discontinuity、timestamp error、预填充静音、欠载、重置、fill 与 correction。

`moiren-app monitor --list` 只枚举输入；`monitor --input <ID> --output <ID> [--seconds N] [--gain G] [--pan P]` 装配真实 `Source → Gain → Pan → Sink`，默认 gain 0.05。先准备 Capture 并得到 native format，再编译 Graph，最后启动输出；任何后续失败都停止/回收 Capture。通过 `CaptureSession::stop_signal()` 将共同 kernel stop event 传给 `start_render_with_stop`；两端在退出 streaming 时直接唤醒 peer，随后 Stop/释放 COM，控制侧 join 不承担停机通知时序。Capture 的时长从其 Start 起算，因此自然完成时 Render 可报告 Stopped 和略短的 elapsed，整体报告 Completed。报告将 capture/render 状态与 bridge 指标分开，capture 提前失败时整体仍为 Failed，禁止把余下静音误报为成功。

## 验收边界

自动测试覆盖 mono/stereo、静音 packet、错误长度、44.1→48 连续相位、不同 demand 分块、±ppm 与 packet jitter、容量/预算拒绝、欠载/溢出/断点恢复、producer 退出、RT 无分配释放，以及 COM fake 验证 lease 释放和 stop 优先级。实机捕获为显式 endpoint 的 opt-in 操作；枚举、纯测试和仿真不等于实机声音或长期稳定性验收。

2026-10-09 验收：FreeDSP 麦克风（48 kHz mono）→ FreeDSP 耳机（48 kHz stereo），gain 0.05、10 秒。采集 479,520 frames、999 packets；Render 474,816 frames、1,979 DSP blocks。Bridge 欠载、溢出、重置均为 0，两端 Stop 成功。用户确认“听感与延迟都还行”；该结果为主观确认，未测量端到端毫秒延迟。四组 44.1/48 kHz × ±1000 ppm、每组两小时加速模拟均无欠载/丢帧。workspace tests、严格 Clippy、fmt 与 diff 检查通过，Linux 交叉目标严格 Clippy 通过；仿真与交叉编译不等于长期硬件验收。

后续 Gate：30 分钟硬件 baseline；两小时跨钟压力；Process Loopback 的 PID/创建时间、迟到 activation、退出恢复；最后才开放多输入/输出和自动设备恢复。

API 依据：[GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)、[ReleaseBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-releasebuffer)、[Initialize](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize)。
