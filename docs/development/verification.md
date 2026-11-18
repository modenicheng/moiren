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

## 本地/跨机环境

个人机器、辅助测试机、代理、ssh 别名、线程数、TEMP 绕法、本机 cargo/Miri 坑属于本地信息，见未跟踪的 `docs/development/local/`（模板见 [local.example.md](local.example.md)）。不存在时按本文件门禁在本机串行执行，并先向用户确认；不要假设某台机器一定可用。

通用要求：重命令一次一个、降低并发避免降频；Windows 本地 Miri 的 std 分配器误报不能当回归，以 Linux CI 为准；时序脆弱测试用"隔离单跑 + CI 是否复现 + 对照 main 同条件"判定。

## 硬件与 soak

- 真机验收由用户按清单执行（设备切换、拔卡、卡死复现、插件扫描、VAD、ASIO、soak 等）；agent 不代替用户做硬件结论。
- 证据位置：`docs/validation/`（验收契约）、`docs/experiments/`（按日期的实验 JSON/日志）、`docs/designs/screenshots/`（UI 截图）、`docs/designs/evidence/`（soak/测量证据）；个人过夜 soak 数据放本地未跟踪目录。
- 硬件验收与便携测试分开报告：不要说"测试全绿"而实际只跑了便携部分；也不要把一次真机结果外推成长期稳定性。

## 完成定义

一次改动可以报告"完成"，至少满足：

1. 改动面对应的 fmt/clippy/test 已通过；Miri/rt-check/feature gate 按本文件"功能门控与独立 harness"一节补跑。
2. CI 对本次 PR 的检查通过（或有明确证据说明为何某面不适用）。
3. 行为变化已更新对应文档/截图/设计状态（见 [documentation.md](documentation.md)）。
4. 需要实机验收的项在报告里明确标为"待用户执行"，并列出验收点。
5. 没有用旧截图/旧命令输出冒充本次证据。
