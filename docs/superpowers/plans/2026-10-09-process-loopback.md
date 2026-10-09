# Process Loopback Implementation Plan

> **For agentic workers:** Execute inline task-by-task in this session. The user authorized advancing the audio backend after accepting the physical-capture slice. Steps use checkbox syntax.

**Goal:** 交付指定应用进程树 → 现有 Graph → Shared Render 的正式闭环，可取消启动、识别目标退出，不错误绑定复用 PID。

**Architecture:** 将探针的 agile activation 提升为共享 owner 内部模块。物理 Capture 与 Process Loopback 共用 packet lease、stream loop、Clock Bridge 和 CaptureSession。控制层只传递 PID + creation time 的身份和标量状态。

**Tech Stack:** Rust、windows 0.62.2、现有 rtrb bridge 与 Compiler。

## Global Constraints

- 只支持 include-target-process-tree；显式 48 kHz stereo f32，Windows AUTOCONVERTPCM/SRC_DEFAULT_QUALITY。与物理 native capture 的格式策略分开。
- 目标 PID 和创建时间需同时匹配；不自动找同名进程，不改系统 endpoint/session 音量或默认输出。
- 目标不能包含本 monitor 进程，以免把自身输出捕回输入；启动前检查进程祖先链。
- callback 只发布 atomic completion；借用 BLOB 由 handler 持有至迟到完成，不在 callback 取出或跨线程传递音频 COM。
- activation 有 10 秒 timeout，可通过预先创建的 StopSignal 取消；退出、取消优先于 completion。stream wait 同时等待 stop、target 和 audio。
- CI 不打开硬件。实机使用独立低增益测试音进程，报告不保存 PCM；主观确认与量化延迟单独记录。
- 首阶段保持已提交的物理 capture 行为，后续多输入和自动恢复仍在 C/D 阶段。

## Task 1: 身份、取消与 activation

**Files:** Create `src/process_loopback.rs`, `src/process_loopback/{activation,identity,stream}.rs`, `tests/process_loopback.rs`; modify `src/lib.rs`, `src/owner.rs`, `src/probe/{activation,capture}.rs`, `src/session.rs`, crate Cargo.toml。

**Interfaces:** `inspect_process(pid) -> Result<ProcessIdentity, CaptureError>`；`ProcessLoopbackOptions { target, duration }`；`start_process_capture[_with_stop](...) -> Result<PreparedCapture, CaptureError>`；`StopSignal::new()`。

- [x] 测试 invalid PID/creation-time/duration、自身目标、身份匹配；确认缺接口导致失败，再实现纯数据契约。
- [x] 共享 activation 的 borrowed-BLOB/handler lifetime 测试，加 cancellation/exit/timeout precedence 和迟到 callback 测试。
- [x] owner 二次 OpenProcess 并比较创建时间；activation 前后检查 pinned handle；API 错误保留 stage/HRESULT。
- [x] 目标祖先链检查，拒绝捕获本 host；必要 Windows feature 仅按所用 API 增加。

## Task 2: 共用 Capture stream 与退出

**Files:** Modify `src/capture.rs`, `src/capture/wasapi.rs`, `src/capture/wasapi/stream.rs`; create `src/capture/wasapi/physical.rs`; reuse packet and clock modules。

- [x] 抽出物理准备与共享 stream 生命周期；保留声明/析构顺序、Stop 及 peer event 广播。
- [x] Process owner 用显式虚拟 stream format 初始化后进入同一 bridge；报告标明 source、process identity、Windows conversion。
- [x] wait 同时接受 target handle；目标退出有独立 TargetExited 状态，stop 和目标退出不依赖音频事件。fake/kernel 测试覆盖优先级与静默目标。
- [x] 跑目标 crate tests/严格 Clippy，物理 capture 与原 probe 仍可编译。

## Task 3: 应用接入与实机

**Files:** Modify `crates/moiren-app/src/monitor/{windows.rs}`, `src/monitor.rs`, `src/monitor_cli.rs`, `src/main.rs`, `tests/monitor_cli.rs`；更新两个 crate README。

- [x] CLI `monitor --process <PID> --output <ID>`，与 `--input` 互斥；纯 parser 测试覆盖歧义、越界、重复、help。
- [x] 控制层 `ProcessMonitorOptions` / `start_process_monitor` 复用现有编译/Render 装配；TargetExited 不误报 Completed，失败仍优先。
- [x] 显式低增益独立测试音进程 → FreeDSP；检查音频非零、bridge 计数、停止；再让目标提前退出，确认两端及时收尾。
- [x] fmt、workspace tests、Windows/Linux 严格 Clippy、diff 检查；更新验收摘要，独立原子提交。

## 验收结果与限制

基线为第一阶段的 `f72b6fc`。自动测试包含 pending/cancel/timeout/late callback、BLOB 所有权、PID 创建时间、祖先链终止、自身捕获拒绝、无音频事件的目标退出、虚拟零位置回归、CLI 互斥及报告状态优先级。workspace tests、Windows/Linux 严格 Clippy、fmt 和 diff 检查通过；最终小改动重跑了受影响的 app/Windows-audio tests 与 workspace Clippy。普通 `cargo test` 不打开音频设备。

实机使用独立 `moiren-app render` 播放 440 Hz、gain 0.05 到 CABLE In 16ch 的 native 48 kHz stereo endpoint，Process monitor 以 gain 0.05 输出到既有 FreeDSP 耳机。没有修改 endpoint/session 音量或默认设备，没有保存 PCM。

| 场景 | 观察 |
| --- | --- |
| 指定进程，10 秒 | capture 475,680 frames / 991 packets；Render 提交 474,336 frames / 1,977 blocks，942,526 非零 samples；欠载/溢出/重置/断点均 0，两端 Stop 成功 |
| 目标 5 秒自然退出，monitor 请求 10 秒 | `target_exited`；Render 约 4.986 秒停止；欠载/溢出/重置 0，两端 Stop 成功 |
| 目标 gain 0，另一个进程同时播放 | 目标捕获 142,560 frames，监控输出非零 samples 为 0；未选进程输出 579,070 非零 samples，确认进程隔离 |
| pwsh 父进程 → 测试音子进程，预热 1 秒 | 捕获目标为 pwsh.exe，子进程音频有 705,110 非零 samples；父进程退出后两端 Stop 成功；欠载/重置 0 |
| 选择 monitor 的宿主祖先 | 在 activation/打开音频流前明确拒绝 FeedbackTarget |
| 物理输入回归，FreeDSP 10 秒 | capture 479,520 frames，948,672 非零输出 samples；欠载/溢出/重置 0，两端 Stop 成功 |

首轮实机发现虚拟 client 每个 packet 的 device position 都为 0；错误地套用物理位置连续性检测会导致 988 次伪断点、982 次重置和大量预填充静音。已用显式 `detect_position_gaps` 策略修复，保留原始位置诊断及 native discontinuity flags，并加入回归测试；物理输入默认策略不变。

另一次未预热的 pwsh/子进程冷启动出现 211 frames 短读（约 4.4 ms），发生在 producer 尚存活时；桥执行了 1 次重预填充，整个运行另计 4,544 frames 预填充静音，输出恢复。短读计数不等于完整可听间断长度。该异常保留，不能据此宣称冷启动/负载下无短读。预热后重复场景为 0 短读，但尚不能归因到具体 Windows/调度环节。C 阶段应覆盖应用启停、CPU 负载及长时间 jitter，再决定缓冲/SRC 策略。

Process Loopback 的实机非零音频/隔离/退出验收已通过；本轮未单独取得用户对应用捕获的主观声音确认，也未量化端到端毫秒延迟。activation timeout/迟到 callback 是无设备的 handler/等待器测试，并非人为延迟 Windows 内核。未完成长时间硬件压力、权限/隐私/拔插/睡眠故障恢复、第三方应用矩阵、多输入或自动重新绑定。原始 JSON 和 stderr 按仓库规则保留在忽略的 `target/w00` 中。

API 依据：[Microsoft sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)、[activation](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-activateaudiointerfaceasync)、[process parameters](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params)。
