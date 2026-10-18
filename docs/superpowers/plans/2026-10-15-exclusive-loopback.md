# Exclusive render / process loopback experiment

**Goal:** 验证独占 render 端点期间，指定进程树的 Process Loopback 是否持续得到有效音频。

**Architecture:** 独立诊断入口；一个连续的进程回环 client 跨越 baseline、exclusive、released 三个窗口。独占静音 render 在自己的 COM owner 线程上运行；Start 成功才测量 exclusive，Stop + join + COM 释放完成才测量 released。每个窗口先持续读取并丢弃 500 ms，再统计 1..60 秒。输出统计 JSON，不保存 PCM。

**Tech Stack:** Rust 2024、现有 windows 0.62.2、现有 activation / packet lease / CaptureStats。

## 边界与方案选择

- 选择独立探针。直接改正式 render 会过早引入产品行为；普通 endpoint loopback 不能代替 process loopback 实验。
- 显式 PID 和端点 ID；固定进程 handle 与创建时间；不修改默认设备、音量、静音、独占优先权设置。
- 独占是端点级操作，影响端点上的所有共享应用。即便捕获成功，也不能宣称完成逐应用 Takeover。
- 独占协商先尝试设备 mix format，再尝试 48/44.1 kHz stereo PCM 16/24/32 与 float；逐项调用 IsFormatSupported。支持事件缓冲对齐重试，禁止悄悄退回 Shared。
- 独占送 SILENT 完整缓冲；线程停止有独立 kernel event，任何错误路径都停止并 join。流循环不分配，不写日志。
- baseline 无信号、目标退出、捕获失败、独占失败都不能算验证成功。500 ms settle 用于排除切换前残留；非零信号只报告观察，不证明完整性或物理端点静音。
- 实机与普通测试分开；先用独立低增益测试音，不以自动测试冒充第三方应用兼容性或主观听音验收。

## 实施步骤

- [x] `src/exclusive_probe.rs`：参数验证、阶段报告及证据分类，回归测试覆盖无基线、短残留、部分窗口、失败优先级。
- [x] `src/exclusive_probe/capture.rs`：复用 activation 和 packet lease，连续捕获、目标存活检查、逐窗口统计。
- [x] `src/exclusive_probe/render.rs`：独占格式协商、缓冲对齐、Start 握手、静音服务和可靠停止。
- [x] `examples/exclusive_probe.rs`：显式 `--pid` / `--endpoint` / `--seconds`，JSON 和有含义的退出状态；README 提供操作与限制。
- [x] 运行 crate tests、all-targets check、严格 Clippy、fmt；受控测试音在独占前后实测并记录量化结果。

## API 依据

- [Exclusive-Mode Streams](https://learn.microsoft.com/en-us/windows/win32/coreaudio/exclusive-mode-streams)：端点独占和共享抢占取决于用户设置；会阻断同一端点的其他声音。
- [Application Loopback sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)：进程树捕获不绑定端点；目标无 render stream 时得到静音。未承诺独占抢占期间可用。
- [IAudioClient::Initialize](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize)：独占 event 模式 duration = periodicity；对齐失败后查询帧数并重新 Activate。
- [IAudioSessionEvents::OnSessionDisconnected](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessionevents-onsessiondisconnected)：`DisconnectReasonExclusiveModeOverride` 会关闭共享会话所属 streams，已有 service 请求失效，后续调用可能返回 `AUDCLNT_E_DEVICE_INVALIDATED`。

## 2026-10-15 本机结果

使用独立 `moiren-app render` 440 Hz / gain 0.01；每个测量窗口 3 秒，另有 500 ms settle。只操作本轮创建的测试进程。原始 JSON / stderr / 重试脚本保存在忽略目录 `target/exclusive-loopback/`。

| 场景 | 结果 |
| --- | --- |
| CABLE In 16ch，mix + 48/44.1 kHz stereo PCM 16/24/32 和 float 格式集合 | baseline RMS 0.007071；所有尝试的独占格式均未获支持，`0x88890008`。只说明当前格式集合不匹配，不能推出驱动完全不支持独占；未抢占成功 |
| FreeDSP，直接捕获测试音 PID | baseline 143,808 非零 frames / RMS 0.007071；独占以 48 kHz stereo PCM16、480-frame buffer 成功 Start；原测试音在 `GetCurrentPadding` 得到 `0x88890004` 并退出，探针约 53 ms 后检测到退出、停止并释放独占。独占与 capture 均 Stop 成功。分类为 experiment_failed，没有把残留信号算成功 |
| FreeDSP，捕获保持存活的 pwsh 父进程及其测试音子进程；子进程失败后每 100 ms 加启动耗时重试 | 同一个 capture client 完整经历三阶段；baseline 144,144 signal frames，RMS 0.00707076；exclusive 134,400 captured frames / 280 packets，**0 signal frames / RMS 0 / peak 0**；released 144,144 signal frames，RMS 0.00707099 |

最后一组独占持续 3.507 秒，351 callbacks，提交 168,960 SILENT frames；独占与 capture Stop 均成功，探针 errors 为空。原共享流被 `0x88890004` 断开；独占期间重建 Shared 流连续返回 `0x8889000A`（device in use）；独占释放后重新打开成功。包未必带 SILENT flag：该次 exclusive 的 silent_packets 为 0，但 PCM 实际全零，因此必须检查采样值。

**结论：本机受控 Shared 音源未通过该方案。** 独占成功阻断原路径，同时使源应用的共享渲染流失效；进程仍存活、捕获仍产包也不代表有可用音频。释放独占后恢复信号，支持这是独占影响而不是捕获对象错误。暂不将其接入正式 Takeover。不能据此排除第三方应用、其他驱动或其他机制的特例；尚未做第三方应用矩阵、OBS 并行捕获、真实听音/物理回采、多设备或长时间验证。`endpoint_silence_verified` 保持 false。

最终格式协商保留非 `UNSUPPORTED_FORMAT` 的原始 HRESULT，不把设备/权限错误折叠成格式不支持。修改后以 1 秒 baseline 复测 CABLE In 16ch，仍返回 `0x88890008`，测试音正常完成。前后 catalog 比较确认默认设备、各端点 volume/mute 均未变化。

## 验证

- `cargo test --locked -p moiren-windows-audio`：完整 crate 测试通过，硬件测试继续 ignored；既有时钟矩阵包括约 240 秒的故障模拟和约 123 秒的启动模拟。
- `cargo test --locked -p moiren-windows-audio --lib exclusive_probe`：最终新增 7 项均通过，覆盖证据分类、参数前置验证、非有限采样和 stop/join 生命周期；不打开音频硬件。
- `cargo check --locked --workspace --all-targets`：通过。
- `cargo clippy --locked -p moiren-windows-audio --all-targets -- -D warnings`：通过。
- `cargo fmt --all -- --check`、`git diff --check`：通过。
- 构建 `exclusive_probe`、help 和非法参数退出码，以及上述三组独立硬件实验均已实际执行。未运行完整 workspace tests、Linux、Miri 或 CI；没有提交或发布。
