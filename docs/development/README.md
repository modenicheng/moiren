# 开发与协作细则

本目录是 Moiren 的长期工程规则库。根 [AGENTS.md](../../AGENTS.md) 只保留项目摘要和最高优先级规则；本目录按主题存放完整、可独立维护的细则。

## 阅读优先级

发生冲突时按以下顺序判断：

1. 用户当前明确指令。
2. 根 [AGENTS.md](../../AGENTS.md) 的最高优先级规则。
3. [FRONTEND.md](../../FRONTEND.md) 中已确认的 UI/设计规则（仅 UI 任务）。
4. 本目录的对应主题文档。
5. `docs/designs/`、`docs/audit/`、`docs/validation/`、`docs/experiments/` 等记录（历史证据，不是当前事实）。
6. 代码、测试与 CI 的当前行为（最终事实源）。

过时文档不得覆盖代码；发现冲突时以代码/CI 为准，并顺手修正文档。

## 文档索引

| 文档 | 管什么 | 什么时候读 |
|---|---|---|
| [repository.md](repository.md) | workspace、crate 依赖、目录职责、新代码放哪 | 第一次进仓库；新增 crate、目录、模块 |
| [architecture.md](architecture.md) | 实时约束、逻辑图/plan/transport 分层、NodeKind checklist、扩展点 | 改引擎、图模型、节点、平台边界 |
| [code-style.md](code-style.md) | 命名、格式、unsafe/COM 注释、i18n 字符串、测试命名 | 写任何 Rust 代码 |
| [workflow.md](workflow.md) | worktree、分支、提交、PR、合并、跨机与网络 | 开始任务；提交；合并 |
| [verification.md](verification.md) | 本地/CI 验证门、功能门控命令、Miri、跨机环境与硬件验收 | 声明"已验证"之前 |
| [app-ui.md](app-ui.md) | app 服务/UI 的数据真相、i18n、gpui 约定、测试陷阱 | 改 `crates/moiren-app` |
| [documentation.md](documentation.md) | 文档体系、设计/审计/证据、语言与生命周期、本地记忆 | 写文档；做审计；更新 README |
| [collaboration.md](collaboration.md) | 通用协作、决策协议、审计与多代理编排、汇报方式；个人偏好见本地 | 与用户/子代理协作 |
| [local.example.md](local.example.md) | 个人偏好/机器配置模板（真实 `local/` 未跟踪） | 需要本机/测试机信息时 |
| [crates/README.md](crates/README.md) | 每个 crate 的模块图、局部契约、验证与文档入口 | 在某个 crate 里工作 |

## 维护规则

- **一处一规则**：同一规则只写在负责它的文档里，不在根 AGENTS 或别处复制。根 AGENTS 只放"违反就会造成不可接受后果"的规则。
- **可执行**：每篇文档尽量给出结论、命令、路径和例外；不写空泛原则。
- **同 PR 更新**：新增 crate、feature gate、NodeKind、CI surface 或工作流规则时，在同一个 PR 里更新对应文档与索引。
- **定期复核**：大型战役或审计结束后检查本目录的日期、命令和链接；用 `rg` 找引用，用脚本检查 Markdown 链接。
- **本地记忆不上库，个人环境也不上库**：`.zcode/`、`.superpowers/` 等个人代理记忆保持 gitignored；机器名、路径、ssh 别名、代理、个人偏好与硬件记录只放未跟踪的 `docs/development/local/`（模板 [local.example.md](local.example.md)），公开文档只留可共享规则。
