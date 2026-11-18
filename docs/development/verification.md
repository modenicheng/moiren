# 验证与 CI

本文是"声明已完成"之前的检查单。原则：**说法要有可复现的证据；便携测试、CI、硬件验收分开记录。**

## 常用命令

```sh
cargo build --locked --workspace
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo run --locked -p moiren-app
cargo run --locked -p moiren-app -- --gain 0.25   # 离线音频演示，不打开设备
```

普通测试不打开音频设备；`moiren-app` 的 headless UI 验收需要 Windows：

```sh
cargo test --locked -p moiren-app --features gpui-test
```

## 功能门控与独立 harness

`cargo test --workspace` 只是便携基线，不是完整门禁。按改动面补跑：

| 面 | 命令 |
|---|---|
| engine 录音编码器 | `cargo test --locked -p moiren-engine --features mp3,flac --lib recorder`；`cargo test --locked -p moiren-engine --features mp3,flac --test host recording` |
| windows-audio VAD 探针 | `cargo test --locked -p moiren-windows-audio --features vad-probe --example vad_lifecycle --example vad_native --example vad_active_lifecycle` |
| windows-audio 探针示例 | `cargo build --locked -p moiren-windows-audio --examples --features probe-tools` |
| app 深度门控 | `cargo test --locked -p moiren-app --features gpui-test,vad-integration-probe`；`cargo test --locked -p moiren-app --features gpui-test,vad-integration-probe --example vad_integration` |
| app profiler 代码 | `cargo clippy --locked -p moiren-app --all-targets --features ui-profiler -- -D warnings` |
| rt-check 影子测试 | `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` |
| VAD driver 便携测试 | `bash driver/vad/test-portable.sh` |
| VAD driver 构建/测试 | Windows + Visual C++ + WDK 10.0.28000.0：`.\driver\vad\build.ps1`、`.\driver\vad\test.ps1` |

## CI 机制

- 单 workflow：`.github/workflows/runtime-foundation.yml`。
- PR 由 `.github/scripts/detect-changes.sh` 把 diff 映射到测试面（含 crate 依赖闭包）；push 到 main 恒全量。
- 门禁包含：fmt、Ubuntu/Windows 的 test 与 clippy、4 个 crate 的 Miri（stacked 与 Tree Borrows 两种）、rt-check、VAD driver 便携测试。
- `moiren-plugins` 与 `moiren-app` 不进 Miri：前者测试 spawn rustc 并加载真实 dylib，后者需要桌面栈。
- 注意：**crate 目录内的文档（含 `crates/*/AGENTS.md`）会触发该 crate 的测试面**；根级 docs 通常不触发。文档 PR 也可能跑全 crate 测试，这是映射的既有设计，不是异常。
- 未知路径与根 manifest/lock/toolchain/workflow 变化会保守回退到全量。

## 本地环境注意事项

- 重命令**串行执行**（一次一个），cargo 用 `-j 2`/`-j 4` 控制并发，避免降频；编译/测试并发时不要同时压满。
- 后台 Git Bash 可能没有 cargo PATH，用 `"$USERPROFILE/.cargo/bin/cargo.exe"` 全路径。
- `moiren-app.exe` 正在运行时 cargo 无法重链：可把 `target/debug/moiren-app.exe` 重命名为 `.running` 后跑测试，结束后删掉。
- Windows 本地 Miri 有 std 分配器 UB 误报（与改动无关），**Linux CI 才是 Miri 门槛**；本地失败先对照原行为是否复现。
- 时序脆弱测试的回归判定用"隔离单跑 + CI 是否复现 + 对照 main 同条件"，不要只凭一次全套失败就认回归。

## 远程测试机

- 第二台物理机通过 ssh 别名 `moiren-lan` 访问（仓库 `D:\moiren`，remote `lan`，`updateInstead`）。本机内存/降频压力大时，编译与全套测试优先迁到远端。
- 远端用户级环境变量已设 `CARGO_BUILD_JOBS=4`、`RUST_TEST_THREADS=4`；首次构建可临时用本机代理加速 crates.io，**不做永久代理配置**。
- 远端 gpui 构建若遇到 `%TEMP%` 路径含空格导致的 fxc 失败，先在 ssh 会话里设无空格 `TMP`/`TEMP`（如 `D:\rt-temp`）再跑。
- 长任务用后台 ssh 链：日志重定向 + 末尾写 `done.txt` 哨兵；本地轮询 `test result:` / `FAILED` 汇总，不要同步阻塞等待。
- 远端无活动显示器时 DWM 不合成，帧管线/性能类问题不能在该机复现；只做无回归旁证。

## 硬件与 soak

- 真机验收由用户按清单执行（设备切换、拔卡、卡死复现、插件扫描、VAD、ASIO、soak 等）；agent 不代替用户做硬件结论。
- 证据位置：`docs/validation/`（验收契约）、`docs/experiments/`（按日期的实验 JSON/日志）、`docs/designs/screenshots/`（UI 截图）、`docs/designs/evidence/`（soak/测量证据）。个人过夜 soak 数据可在 `.zcode/night-flake/`（本地、不上库）。
- 硬件验收与便携测试分开报告：不要说"测试全绿"而实际只跑了便携部分；也不要把一次真机结果外推成长期稳定性。

## 完成定义

一次改动可以报告"完成"，至少满足：

1. 改动面对应的 fmt/clippy/test 已通过；Miri/rt-check/feature gate 按本文件"功能门控与独立 harness"一节补跑。
2. CI 对本次 PR 的检查通过（或有明确证据说明为何某面不适用）。
3. 行为变化已更新对应文档/截图/设计状态（见 [documentation.md](documentation.md)）。
4. 需要实机验收的项在报告里明确标为"待用户执行"，并列出验收点。
5. 没有用旧截图/旧命令输出冒充本次证据。
