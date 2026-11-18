# Crate 指南索引

每个 crate 的局部规则（模块图、硬约束、验证命令、文档入口）。仓库级规则见 [../README.md](../README.md) 与 [AGENTS.md](../../../AGENTS.md)。

| Crate | 一句话职责 | 指南 |
|---|---|---|
| moiren-core | 可编辑 `LogicalGraph` 与控制协议（非实时、叶子） | [moiren-core.md](moiren-core.md) |
| moiren-signal | 有界 Latest/Stream 信号存储与 RT 发布所有权 | [moiren-signal.md](moiren-signal.md) |
| moiren-plugins | CLAP/VST2/VST3 ABI、扫描、实时插件实例（叶子） | [moiren-plugins.md](moiren-plugins.md) |
| moiren-engine | DSP、图编译、`EngineHost` 托管运行时 | [moiren-engine.md](moiren-engine.md) |
| moiren-windows-audio | WASAPI/ASIO 后端、引擎侧 IO、VAD、时钟、探针 | [moiren-windows-audio.md](moiren-windows-audio.md) |
| moiren-app | 应用服务、gpui 桌面 UI、CLI、持久化、i18n | [moiren-app.md](moiren-app.md) |
