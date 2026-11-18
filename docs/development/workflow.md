# Worktree、分支、提交与合并

本文管"怎么把改动送进 main"；验证门本身见 [verification.md](verification.md)。

## 基本原则

- **先计划后实施**：中大型任务先写计划（范围、已采纳决策、阶段、每阶段验证门、风险与回退），计划定稿再动代码。
- **一任务一分支一 worktree**：agent/并行会话在独立 worktree 工作，主工作区留给集成、收尾和用户自己的 WIP。
- **提交与 push 需用户授权**：用户说"提交 / push / 做完一批就提交一批直接推 main"才执行；默认不要把改动自行推走。批量授权按阶段拆原子提交，提交后立即 push。
- **只 stage 自己动过的文件**：用户另有 WIP 时，除非明确说"所有改动统一提交"，不要把它们卷进提交。
- **提交前重查现场**：并行会话可能中途换分支/换目录；每次 `git add` 前重新确认 `git branch --show-current` 与 `git status`。
- **不碰 `.git/`**：包括 `.git/info/exclude`；worktree 目录用仓库既有惯例（`.worktrees/`），不要发明兄弟目录。

## 计划

- 个人计划放在 `.zcode/plans/`（本地、gitignored），常用结构：范围解读 → 决策/未答复问题表 → 实施阶段（每阶段验证门）→ 风险与回退 → 量化预期。
- 需要长期留存的决策写进 [docs/TODO.md](../TODO.md) 或设计文档；计划本身不作为仓库事实源。
- 计划里要写清"哪些验证由用户执行"（实机、soak）和"哪些由 CI 执行"，避免把未跑的验收当已完成。

## Worktree

```sh
git fetch origin
git worktree add .worktrees/<short-topic> -b <type>/<topic> origin/main
```

- 目录名用短主题（`audio-host`、`recorder`、`shape-style-key`），分支名用完整 conventional 名；不要用目录名反推分支规范。
- 本地 `.cargo/config.toml` 的 `[build] jobs = 4` 与 `target-*` 专用目录走 `.git/info/exclude`，不进版本库；worktree 之间可共享 `CARGO_TARGET_DIR` 复用依赖缓存。
- 删除 worktree 不删分支，提交不会丢；真正会丢的是**未跟踪文件**与**未推送分支的独有提交**。删除前：
  ```sh
  git -C .worktrees/<name> status --porcelain
  git cherry main <branch>        # patch-id 等价，判断内容是否已在
  git merge-base --is-ancestor <branch> main
  ```
- 长期战役可以保留 worktree；已合并/已决策放弃的线及时清理（`git worktree remove`），target 目录单独清理。
- 主工作区卡住时，用 `cd` 离开被钉住的目录再删；不要在 worktree 内对错误的 feature 分支做 merge。

## 分支命名

`<type>/<kebab-topic>`：

- 常用 `feat/`、`fix/`、`refactor/`、`perf/`、`test/`、`docs/`、`chore/`、`ci/`、`integrate/`。
- 实验/时间敏感线可带日期后缀（如 `feat/vad-virtual-playback-category-20261105`）。
- 合并后本地分支默认保留（便于回看/复用），远端分支随 PR 删除；定期用 `git branch -vv` 找 `gone` 分支并清理。

## 提交

Conventional Commits，英文祈使句，scope 用领域/ crate 名：

```
feat(engine): add compressor processor
fix(audio): bound native teardown so a wedged release cannot park Stop
refactor(ui): move the slider editing kernel to ui::editors
docs: add per-crate AGENTS.md guides
```

- 标题控制在约 72 字符内；squash 时 GitHub 会自动追加 `(#N)`。
- 正文写**为什么**、行为 bullet、`Tests:`/验证结果；一个提交一个逻辑切片，不要把无关文件混进来。
- 常用 scope：`app`、`ui`、`engine`、`audio`、`vad`、`windows-audio`、`signal`、`plugins`、`core`、`recorder`、`i18n`、`ci`。
- 不提交调试日志、一次性产物、未跟踪的个人文件；wip 快照式提交只在明确授权时使用。

## PR

- 小修可在用户明确授权时直推 main（`git push origin <branch>:main` 或 `git push origin main`），无需 PR；功能线/多文件改动走 PR。
- 标题沿用 conventional 风格；描述用中文，常用结构：背景/问题 → 行为变化（必要时表格） → 测试与验证 → 风险与已知边界；UI 改动附截图。
- 长期未收口的线可以开 draft（例如历史上的 #23/#26），但不要让 draft 无声腐烂：定期说明状态或关闭。
- 关联 issue：正文写 `Closes #N` / `Refs #N`，合并后确认 issue 已关闭。

### 堆叠 PR

- 顺序：合 base PR 时**先不删 base 分支** → 把子 PR base 显式改成 `main`（`gh pr edit <child> --base main`）→ 合子 PR → 最后删分支。
- squash 合并 base 后，子分支要 `git rebase --onto origin/main <old-base-tip> <child>` 去掉重复提交，否则 diff/CI 双倍。
- 每次合并后用 `git log --grep "<PR号>"` 确认提交真的到了目标分支；合错可用 `git fetch origin refs/pull/<N>/head:refs/heads/<rescue>` 抢救后重开 PR。

### 合并方式

当前采用 **squash merge**（保留 PR 号，main 历史线性）：

```sh
gh pr merge <N> --squash --delete-branch
```

合并后同步本地：

```sh
git switch main
git fetch origin            # 或 HTTPS 回退，见下
git merge --ff-only origin/main
```

确认 `git log --oneline -1` 指向合并提交、工作区干净，再删本地分支（`git branch -d <branch>`）。

## 跨机与网络

- 本仓库可能配置了本地辅助测试机、`lan` remote 或代理；具体主机、路径、ssh 别名、代理地址与并发限制属于本地环境信息，见未跟踪的 `docs/development/local/machines.md`（模板见 [local.example.md](local.example.md)）。
- 公开文档不写机器名、IP、ssh 别名、代理地址或本地绝对路径。
- 网络故障时只使用用户已有的通道（SSH/HTTPS 等），不要替用户切换/重启代理或改系统配置；恢复后从中断处继续。

## 清理债务

- 定期检查：3 个月未动的 worktree、已 `gone` 的本地分支、2 个以上长期 open 的 draft PR、无人认领的实验分支。
- 清理顺序永远是：先抢救未跟踪文件 → 确认独有提交已推送/已合并 → 删 worktree → 删分支 → 清 target。
- `target/` 与 `target-*` 在 gitignore 内，可安全删除；清理前确认没有正在跑的构建。
