# 仓库结构与放置规则

## 定位

Moiren 是 Rust 2024 workspace，面向 Windows 实时音频：可编辑 `LogicalGraph` → 编译器产出 prepared plan → `EngineHost` 托管生命周期 → WASAPI/ASIO 等平台 transport → gpui-kit 桌面 UI。

## 目录职责

| 路径 | 职责 | 细则 |
|---|---|---|
| `crates/moiren-core` | 可编辑 `LogicalGraph` 拓扑与控制协议；非实时、无平台依赖 | [moiren-core](crates/moiren-core.md) |
| `crates/moiren-engine` | DSP 处理器、图编译、buffer、`EngineHost` 托管运行时 | [moiren-engine](crates/moiren-engine.md) |
| `crates/moiren-signal` | 有界 Latest/Stream 信号存储与发布所有权 | [moiren-signal](crates/moiren-signal.md) |
| `crates/moiren-plugins` | CLAP/VST2/VST3 ABI、扫描、实时实例（叶子 crate） | [moiren-plugins](crates/moiren-plugins.md) |
| `crates/moiren-windows-audio` | 专业音频后端：WASAPI Shared/Exclusive、ASIO、引擎侧 capture/render、VAD、时钟适配、探针 | [moiren-windows-audio](crates/moiren-windows-audio.md) |
| `crates/moiren-app` | 应用服务、CLI、gpui 桌面 UI、项目持久化与 i18n | [moiren-app](crates/moiren-app.md) |
| `driver/vad` | 独立的 C++ ACX/KMDF 虚拟音频驱动（x64，VS + WDK） | `driver/README.md`、`driver/vad/STATUS.md` |
| `tools/rt-check` | 独立 Cargo workspace，用 `#[path]` 影子编译原生 RT 契约测试 | `tools/rt-check` |
| `tools/audio-hack-lab` | 实验脚本与结论报告；日志不上库 | `tools/audio-hack-lab` |
| `scripts/` | 支持脚本（如 `vad-soak.ps1`） | — |
| `assets/logos/` | 共享品牌 SVG | `assets/logos/README.md` |
| `docs/` | 设计、审计、验证、平台说明与开发规则 | [documentation.md](documentation.md) |

## crate 依赖方向

```
core ─┐
signal┼─→ engine ─→ windows-audio ─→ app
plugins┘
```

- `moiren-core` 是叶子，仅依赖 `thiserror`；不得引入 engine、平台或 UI 依赖。
- `moiren-signal`、`moiren-plugins` 也是叶子；`plugins` 不得依赖任何 `moiren-*` crate，由 engine 适配。
- `moiren-windows-audio` 不依赖 app；`moiren-app` 是栈顶，可依赖所有库 crate。
- 新增跨 crate 依赖前，先确认方向没有反向、没有把平台细节带进 core/engine。

## 代码放置

- 每个 crate 用 `src/` 放源码、`tests/` 放集成测试、`examples/` 放可运行示例（可选）。
- 模块按职责拆分，避免 God 文件；控制面与实时面、平台层与逻辑层分开。
- 桌面 UI 只放在 `crates/moiren-app/src/ui/`；Tabler 图标在 `crates/moiren-app/assets/icons/`；UI 不使用 markup 语言。
- 新增模块先找已负责该领域的父模块；不要为单实例造抽象，也不要把内聚文件按行数硬拆。

## 新增 crate / 目录 / feature gate

必须同 PR 完成：

1. `Cargo.toml` workspace `members`、共享依赖与 `[lints] workspace = true`。
2. `.github/scripts/detect-changes.sh` 的 `ORDER`、`direct`、目录 pattern 与依赖闭包，以及 `detect-changes.test.sh` 的断言。
3. `.github/workflows/runtime-foundation.yml` 的 push-main 全量清单、必要的 Miri/feature 步骤。
4. 根 `AGENTS.md` 的 crate 列表（如仍是摘要级）、本目录的索引与本文件、对应 crate 指南。
5. crate 的 `README.md`（面向使用者）与 `docs/development/crates/<name>.md`（面向修改者）。

feature gate 的既有约定：每个 feature 都是"非默认消费面"的门控（探针、headless 测试、profiler、编码器），在 `Cargo.toml` 注释用途，示例用 `required-features` 钉住，CI 显式 lint 门控面。

## 文档入口

完整文档体系见 [documentation.md](documentation.md)；第一次接触项目建议顺序：[README.md](../../README.md) → [AGENTS.md](../../AGENTS.md) → 本目录 → 对应 crate 指南。
