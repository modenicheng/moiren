# 基础 IO 节点与首版 App 实施计划

**Goal:** 实现 [IO 节点设计](../designs/03-io-nodes.md)定义的 engine InputNode/OutputNode、有界桥，并从 `moiren-app` 跑通输入→Gain→输出。

**Architecture:** engine 保存 IO 配置与 prepared runtime，独立 SPSC 复制音频并传递诊断；app 负责准备、控制、同步离线执行及非 RT 收尾。core 使用现有 protocol，不新增模型。

**Tech Stack:** Rust 2024、已有 thiserror、anyhow 与 rtrb 0.4.0；不新增外部依赖。

## 约束

- 生产代码不新增 unsafe；Graph slab 的借用仍由 buffer 模块管理。
- 一个端口承载多声道，输入节点只有 output 0，输出节点只有 input 0。
- RT 不分配、释放、加锁、打开设备、读写文件或等待队列。
- 保留工作区已有 graph.rs 与 lib.rs 改动，不扩展 Graph compiler 或 Windows W00 探针。

## Task 1：Engine 配置与校验

文件：`crates/moiren-engine/src/node/config.rs`、`src/node/config/tests.rs`。

- [x] 先写有效 Pinned/FollowDefault/Software/Application 配置及空 selector、零 channels/SR/period 拒绝测试；确认模块缺失失败。
- [x] 实现 InputConfig/OutputConfig、EndpointSelector、DeviceOptions、InputSource/OutputTarget 及 `validate() -> Result<(), IoConfigError>`；ClockRole/CaptureIntent 保持在 boundary。
- [x] `cargo test --locked -p moiren-engine --lib node` 验证通过。

## Task 2：运行节点与诊断

文件：`crates/moiren-engine/src/node.rs`、`src/node/tests.rs`、`src/boundary.rs`、`src/lib.rs`。

- [x] 先写 prefix、非法 transferred_frames、错误端口、时间断点与有界诊断测试；确认缺少节点失败。
- [x] 实现 `InputNode::<S,T>::new(config, source, capacity)`、`OutputNode::<S,T>::new(config, sink, capacity)`，返回节点与 BoundaryReader；沿用 RtProcessor 的 prepare 与 process 契约。
- [x] 共享 Source/Sink 验证及报告规范化；短输入尾部清零，诊断累计 saturating counters，reader 有限排空。
- [x] 节点测试通过，旧 SourceAdapter/SinkAdapter import 和构造方式兼容。

## Task 3：音频桥与实时集成

文件：`crates/moiren-engine/src/boundary/bridge.rs`、`tests/io.rs`、`tests/runtime.rs`。

- [x] 先写 stereo f32/f64 roundtrip、ring wrap、容量溢出、欠载与对端退出测试；确认未实现桥失败。
- [x] 实现 `audio_bridge<S>(channels, capacity_frames, byte_budget) -> Result<(AudioWriter<S>, AudioReader<S>), BridgeError>`；完整 frame chunk 发布/消费，检查零尺寸、乘加溢出及预算。
- [x] 实现 worker `write_interleaved(&[S])` / `read_interleaved(&mut [S])` 和 Graph RtAudioSink/RtAudioSource，拒绝错位 slice，保留浮点幅度。
- [x] 集成验证单 slab、两个 output fan-out、参数分段、variable block 与满队列；计数 allocator 确认 render 及 worker sample transfer 无分配/释放。
- [x] 独立线程 producer/consumer 验证声道对齐。

## Task 4：首版 App

文件：`crates/moiren-app/{Cargo.toml,src/lib.rs,src/main.rs,tests/offline.rs,README.md}`、workspace manifest/lockfile、根 README、CI。

- [x] 先写应用 IO roundtrip、跨 block ramp、输入拒绝与空请求测试；确认新 API 缺失失败。
- [x] 实现 `OfflineApp::new(AppConfig)`、`process_interleaved(&[f32], &mut [f32]) -> Result<ProcessReport, AppError>`，装配 Input→Gain→Output，shape 检查后拆分处理。
- [x] 实现 `set_gain(f64,u32)`、`poll_applied()`、`input_status()`、`output_status()`、`stop(self)`，控制请求经有界队列提交；提供 headless CLI。
- [x] 更新文档和 CI，加入 app 测试与运行命令。
- [x] 完整执行测试、Clippy、fmt、`cargo run --locked -p moiren-app` 与既有 offline 示例；检查结果和既有阻断见下方记录。

## 实施记录

基线：既有 core/engine 测试通过；严格 Clippy 被已有 graph 草稿的 dead_code 与 new_without_default 拒绝。本轮不修改该草稿；新 engine/app 另外执行 `cargo clippy --locked -p moiren-engine -p moiren-app --all-targets --no-deps -- -D warnings` 与对应 fmt。

最终本地验证（Windows，2026-10-08）：

| 命令 / 检查 | 结果 |
|---|---|
| `cargo test --locked -p moiren-core -p moiren-engine -p moiren-app` | 40 个 unit/integration tests + 1 个 compile-fail doc test 通过；本轮新增 17 项 |
| `cargo clippy --locked -p moiren-engine -p moiren-app --all-targets --no-deps -- -D warnings` | 通过；core 依赖仍产生基线 warnings |
| `cargo fmt -p moiren-engine -p moiren-app -- --check` | 通过 |
| `cargo run --locked -p moiren-app` | 4 帧 stereo 输出正确乘以 0.5，Input/Output 无 shortfall、XRUN 或 discontinuity |
| `cargo run --locked -p moiren-app -- --gain 0.25` | 输出正确乘以 0.25 |
| `cargo run --locked -p moiren-engine --example offline` | 旧示例正常，参数在 frame 32 Applied，render 分为两个 segment |
| 包含 core 的严格 Clippy / fmt | 仍被原有 graph.rs 的 dead_code / new_without_default，以及 graph.rs / lib.rs 格式问题阻断；与本轮新增代码无关 |
| `git diff --check` | 通过 |

Linux/Windows 双平台及两种 Miri 模式已在 CI 中加入 app；本地未执行 Linux 或 Miri，不能把工作流配置视为已完成的检查结果。正式设备后端尚未接入。

提交前另将仅包含本轮暂存内容的 tree 导出到隔离目录，重新执行 core/engine/app 的 41 项测试、包含 core 的严格 Clippy 与 fmt，全部通过。上表中的基线阻断属于原工作区保留的 Graph 草稿，该草稿不包含在本轮提交中。
