# W00 被动捕获探针实施计划

> 执行方式：当前会话按任务顺序实现与验证，使用 executing-plans 工作流。
> 用户已确认：仅 W00；内置与 USB 耳麦可用；QQMusic 正常听歌时不受干预；首轮只保留统计。
> 状态：本切片已完成。10 项测试、package check / Clippy / format、目录查询、1 秒 smoke、60 秒实测和无效 PID 拒绝均通过。W00 全部验收尚未完成。

**Goal：**独立运行设备 / 会话枚举与 60 秒 QQMusic Process Loopback 探针，取得真实数据和明确的未测范围。

**Architecture：**新增 `moiren-windows-audio`，不依赖 core / engine。纯统计模块接收 f32 数据；单个 COM owner 负责目录快照、异步 activation、Shared capture、停止和释放；completion callback 只写原子完成标志。音频包只在 GetBuffer / ReleaseBuffer 期间读取。预分配 packet metadata，停机后一次写 JSON。

**Tech Stack：**Rust 2024、固定 `windows = 0.62.2`、MMDevice / Audio Session / Process Loopback、serde JSON。

## 约束与验收

- 保持 QQMusic 原有播放；探针不调用音量 / mute setter、不修改默认设备、不打开 render / Exclusive stream、不捕获物理麦克风。
- 首轮协商 48 kHz / stereo / interleaved f32 capture，开启 Windows 自动格式转换；这是捕获 stream format，不是物理硬件格式。
- 持有目标进程查询 handle，记录 PID + 创建时间；目标退出就结束，不跟踪复用 PID。
- 保存 packet 帧数、flags、device position、QPC（100 ns）、到达时间、RMS / peak；不保存 PCM 或 WAV。
- 无音频不等于不支持；报告区分成功捕获信号、无信号、目标退出、API 失败。
- 首包 discontinuity 与后续 discontinuity 分开；TIMESTAMP_ERROR 包不参与时间戳单调性判断。
- 采集前后对比默认 render 角色、endpoint 音量 / mute、目标 session 音量 / mute；差异只报告，不自动恢复或修改。
- 60 秒仅验证基础捕获，不宣称进程隔离、听感无卡顿、Takeover、跨钟、恢复或长期稳定性已经通过。

## 任务 1：可验证的统计

**文件：**`Cargo.toml`、`Cargo.lock`、`crates/moiren-windows-audio/Cargo.toml`、`crates/moiren-windows-audio/src/lib.rs`、`crates/moiren-windows-audio/src/stats.rs`。

**接口：**`analyze_f32(bytes, frames, channels, silent) -> Result<PacketMetrics, &'static str>`；`CaptureStats::observe` / `summary`。

- [x] 添加独立 workspace member 和测试：stereo frame / sample 区分、SILENT 无 payload、payload 长度验证、NaN / Inf、silence 加权 RMS、初始 discontinuity、错误时间戳过滤和时间戳回退。
- [x] 运行 `cargo test -p moiren-windows-audio --lib`，确认缺少统计实现导致失败。
- [x] 实现无分配统计，运行相同测试通过。

## 任务 2：只读目录与捕获 owner

**文件：**`crates/moiren-windows-audio/src/probe.rs`、`crates/moiren-windows-audio/src/catalog.rs`、`crates/moiren-windows-audio/src/owner.rs`、`crates/moiren-windows-audio/examples/w00_probe.rs`。

**接口：**`probe::run(pid: Option<u32>, seconds: u32) -> windows::core::Result<ProbeReport>`；无 PID 只枚举，显式 PID 才捕获。

- [x] 添加 COM / task memory / PROPVARIANT / HANDLE guards；所有接口与 WASAPI services 留在 owner 线程。
- [x] 枚举 active capture / render endpoint、名称、mix format、period、默认角色、音量与 session；单项读取失败保存 HRESULT，不把未知写成零。
- [x] activation callback 实现 IAgileObject；参数所有权覆盖迟到 completion；等待有 10 秒截止。
- [x] event-driven Shared Process Loopback；预分配 metadata；按 SILENT flags 处理 packet；每个有效 packet 完整 Release；deadline 与目标退出检查不依赖音频事件。
- [x] 停止并释放后重新枚举，生成统计与前后对照 JSON；CLI 限制 `--seconds` 在 1–600。
- [x] `cargo fmt -p moiren-windows-audio --check`、`cargo check -p moiren-windows-audio --all-targets --locked`、`cargo clippy -p moiren-windows-audio --all-targets --locked -- -D warnings`。
- [x] 修复 bindings 自动析构借用 BLOB 的错误释放；补两个 ownership / agile callback 回归测试。最终短测成功，无效 PID 返回 `api_failed`、`0x80070057` 和 exit code 1。

## 任务 3：实机证据

**文件：**`crates/moiren-windows-audio/README.md`、`docs/experiments/windows/`、原 Windows 计划的 W00 状态。

- [x] 先执行 `cargo run -p moiren-windows-audio --example w00_probe --locked -- --list`，检查目录与 session。
- [x] 读取当前 QQMusic PID，执行 `cargo run -p moiren-windows-audio --example w00_probe --locked -- --pid <当前PID> --seconds 60`，停机后保存 JSON。
- [x] 记录 OS build、设备 / 驱动、requested capture format、实际 buffer、packet 统计、停止结果和前后状态；没有证据的设置记为 Unknown。
- [x] 对结果按证据定性，保留未测项；记录复现命令及后续对照实验。原始 packet trace 保存在已忽略的 `target/w00/`，本地实验汇总保存在 `docs/experiments/windows/`。

官方依据：[Application Loopback Sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)、[GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)、[ActivateAudioInterfaceAsync](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-activateaudiointerfaceasync)。
