# 04 repo-steward 卫生 — 初审 B（output-hygiene + legacy-cleanup + configuration + change-gate）

- **基线**：main @ `e2ed3d6a`（2026-11-10 22:40 +0800，Merge PR #168）。工作区仅 `docs/audit/2026-11-09|11-11/` 未跟踪（审计产物）。
- **方法**：只读。亲证手段：`git ls-files`/`git log -p -100`/`git log -G` diff 抽查、`rg` 全仓宏与注释扫描、`.github/` 全量精读、`git worktree list` 对账、逐文件 head 抽读。未跑 cargo。未读本轮（2026-11-11）其他代理报告；引用了既往周期（2026-10-27、2026-11-09）已归档审计用于 KNOWN 对账。
- **范围**：产出卫生（AI 冗余注释/调试残留/一次性产物）+ 遗留清理三档 + 机器可读配置 + 提交/PR 门禁。文档组织/漂移/plans 生命周期/AGENTS.md 归初审 A，本报告仅在作为证据时引用。

## 发现

### B-1 moiren-plugins 全门禁裸奔：CI 从不运行它的测试与 clippy [P1] [KNOWN-OPEN（W-1）]
- **位置**：`.github/scripts/detect-changes.sh:52-58`（ORDER/direct 仅 5 crate）、`:69-84`（无 `crates/moiren-plugins/*` 分支，落入 `*) full=true` 兜底）；`.github/workflows/runtime-foundation.yml:48-50`（push-to-main 硬编码 pkgs/lint_pkgs/miri_pkgs 亦无 plugins）；`:50`（miri 全量清单无 plugins）。
- **证据**：`Cargo.toml:7` plugins 是 workspace 成员；`crates/moiren-engine/Cargo.toml:11` engine 依赖 plugins。plugins 自有测试 `crates/moiren-plugins/tests/plugin_ffi.rs`（7 个 `#[test]`）+ `src/vst3/abi.rs`（2 个）。全 workflow 无 `--workspace` 测试/lint（仅 `fmt --all`，yml:66）。
- **影响**：plugins-only PR 会触发 full=true（把其余 5 个 crate 全测一遍——保守正确但浪费），但 `cargo test -p moiren-plugins`、`cargo clippy -p moiren-plugins`、miri 在任何路径（PR、push main）都永不执行；plugins 自身唯一 CI 覆盖是 fmt 与"作为 engine 依赖被编译"。FFI 正确性缺陷可无声存活。连带：`AGENTS.md:34` "pushes to main always run the full suite" 对 plugins 不成立；AGENTS.md crate 清单未列 moiren-plugins（归 A，此处作影响面证据）。
- **建议方向**：detect-changes.sh 增加 `moiren-plugins` 到 ORDER/direct；closure 方向为 plugins→engine→app（plugins 变更需拉 engine/app 重测）；push-main 与 full 分支的三份清单补 plugins；`detect-changes.test.sh` 补对应用例（当前自测无 plugins case）。miri 是否纳入单独决策（真插件 FFI 不可 miri，可先不加）。同步 AGENTS.md（A 界面）。工作量: S
- **验证门**：本地 `bash .github/scripts/detect-changes.test.sh` + 构造 plugins-only 文件清单跑 `--files` 断言 lint_pkgs 含 moiren-plugins。

### B-2 moiren-signal 门面注释反向失实：Stream 已承载生产流量 [P2] [KNOWN-OPEN]
- **位置**：`crates/moiren-signal/src/lib.rs:6-9`："The Stream transport is spec-complete but **carries no production traffic yet**: it is awaiting its first routed consumer…Latest carries all current publication paths."
- **证据**（亲证）：`crates/moiren-app/src/service/signals.rs:26-27` 定义 `LevelTopic/SpectrumTopic = StreamBlock<…>`，`:141,146,151,188,244` 生产 service 代码 `register_stream_block`，`:16-21` 注释解释为何电平/频谱窗口必须 FIFO 流式。全 app src 唯一 StreamBlock 生产落点即此文件。既往轨迹：2026-10-27 审计（01-core-signal.md:9,19）核实当时零生产流量并要求补标注 → 标注已落地；2026-11-02/03 起 app 在 Stream 上发布，2026-11-09 审计（06-signal-plugins.md:19）报 P2 文档漂移 S1；至 `e2ed3d6a` 未改。
- **影响**：crate 门面对新使用者撒谎——"Latest carries all current publication paths" 现在是假的；可能误导后续贡献者以为可随意改 Stream 语义。
- **建议方向**：改写为"Latest 与 Stream 都承载生产发布；Latest 用于超序覆盖型观测，Stream 用于需保序 FIFO 的窗口观测（电平/频谱）"。工作量: S

### B-3 clap host 注释宣称不存在的"原子计数器排水"机制 [P2] [KNOWN-OPEN]
- **位置**：`crates/moiren-plugins/src/clap/abi.rs:84-86`："restart/callback requests land in an **atomic counter the control side drains opportunistically**."
- **证据**（亲证）：`crates/moiren-plugins/src/clap/mod.rs:86-88` 三个回调为空体 stub（`{}`），全 crate 无任何计数器/排水实现（`rg 'request_restart|request_process|request_callback'` 仅此 5 处 + abi 声明）。该注释自 crate 诞生提交 `d7f95567` 即如此（`git log -S` 唯一命中）。
- **影响**：描述了不存在的并发机制；读者会错误假设 restart 请求被缓冲并消费。
- **建议方向**：改为"get_extension 返回 null；request_* 目前为 no-op，请求被丢弃——若未来需要唤醒控制侧，在此落地"。工作量: S

### B-4 scripts/managed-stress.ps1 成孤儿脚本 [P2] [NEW]
- **位置**：`scripts/managed-stress.ps1:2`（默认 `$Executable = …\target\release\examples\managed_stress.exe`）、`:5`（默认输出目录 `docs/superpowers/plans/2026-10-14-engine-stress`）。
- **证据**（亲证）：全仓 `rg 'managed_stress'` 仅命中脚本自身；`crates/*/examples/` 无 managed_stress（windows-audio 现存 asio/exclusive/vad_*/w00_*，engine 现存 6 个，见各目录清单）；`docs/superpowers/plans/` 目录已不存在（`docs/superpowers/` 下仅 `specs/`）。脚本 2026-10-14 后未更新（mtime/`git log` 一致）。
- **影响**：照注释运行必失败；误导读者以为存在 managed_stress 探针与对应 plans 目录。
- **建议方向**：删除，或改指现存探针并更新输出目录约定。工作量: S

### B-5 main 历史混入 wip/WIP 混合颗粒度提交 [P2] [NEW]
- **位置**：`284b911e`（2026-11-01，"wip: snapshot uncommitted local work before merging origin/main"，30+ 文件横跨 AGENTS.md/README/locales/ui/service/graph）；`58d412a9`（同日，"Merge local WIP into upstream main (origin/main @ 6e4c5ee)"），作者 Lidozs55，均已在 main。
- **证据**（亲证）：`git show 284b911e --stat`；`git merge-base --is-ancestor` 确认在 main。近 60 提交中 >25 文件的提交为 0——违规仅此族。
- **影响**：违反 AGENTS.md 提交规范（feat/fix/refactor/test/chore）与颗粒度要求；`wip:` 快照把 i18n、UI、服务层、根文档混入单提交，回溯与 revert 困难。已发布历史不可重写。
- **建议方向**：不改历史；在协作约定（AGENTS.md PR 节，A 界面）补一条"禁止 wip 快照直入 main，本地暂存走分支+squash"。工作量: S

### B-6 CI 触发依赖代理事件，曾以空提交重触发 [P3] [NEW]
- **位置**：`be56cf50`（2026-11-10，空提交，"chore: retrigger CI (opened event lost to the proxy)"）。
- **证据**（亲证）：`git show be56cf50 --stat` 为零文件变更；提交主题自述原因。
- **影响**：门禁依赖 PR opened 事件可靠到达；丢失时需人工察觉并手动补触发，存在"无 CI 的 PR 合并"窗口。
- **建议方向**：流程面：事件再丢失时优先 close/reopen 或 rebase 触发 synchronize（保留可追溯），空提交作最后手段；如频发，考虑 `pull_request.types` 补 `synchronize`/`reopened` 显式声明（现状默认集已含，问题在代理侧）。工作量: S

### B-7 CI 注释引用已删除的文档路径 [P3] [NEW]
- **位置**：`.github/workflows/runtime-foundation.yml:176`："see docs/superpowers/plans/2026-10-19-repo-steward-remediation.md W8"。
- **证据**（亲证）：`ls docs/superpowers/` 仅 `specs/`；`docs/superpowers/plans/` 不存在（`f38cc98e` 已清理 landed plans）。
- **建议方向**：注释改为直接陈述事实（probe-tools feature-gated 的原因），删失效路径。工作量: S

### B-8 .gitignore 中 .worktrees/ 规则重复 [P3] [NEW]
- **位置**：`.gitignore:44-45` 与 `.gitignore:60-61`，同一路径 `.worktrees/` 两条注释两种措辞。
- **建议方向**：留一条（L44 的措辞更好）。工作量: S

### B-9 ui_capture 捕获 harness 内一行式无条件 eprintln [P3] [NEW]
- **位置**：`crates/moiren-app/src/ui/shell/capture.rs:300`：`for display in &displays { eprintln!("Capture display {:?}: {:?}"…); }`（单行循环体，随每次 capture 运行无条件转储显示器边界，与 :345/:416 的结构化输出风格不一致）。
- **影响**：非生产 RT 路径（capture harness，`run_ui_capture` 仅外部驱动调用），属调试残留风格而非功能问题。
- **建议方向**：并入下次 capture 相关改动顺手收敛为结构化输出或删除。工作量: S

### B-10 工具链浮动 "stable"，无版本基线 [P3] [NEW]
- **位置**：`rust-toolchain.toml`（`channel = "stable"`，`git log --follow` 显示 `ec861422` 创建后从未改动）；CI 侧 `rustup toolchain install stable` + `cargo +stable`（yml:65/98/155/227），两端机制一致。
- **影响**：本地与 CI 同步浮动，但每次 stable 升级可无声改变 rustfmt/clippy 行为，`-D warnings` 门禁可能在无代码变更时变红（task 背景：1.99 基线曾被打破）。部分缓解已在位：`Cargo.toml` workspace.lints 将 future_incompatible/unused 设 deny，注释明言"lint 漂移不能只靠 CI 拦"。
- **建议方向**：可选把 channel 锁到具体版本（如 `1.4x`）并按季度手动升级、升级提交顺带清 lint；维持浮动也可接受（团队已习惯即时清理）。不加 lint 超集配置。工作量: S

### B-11 两批设计截图缺所有权挂链 [P3] [NEW]
- **位置**：`docs/designs/screenshots/2026-10-30-library-cards/`（2 PNG）、`docs/designs/screenshots/2026-11-10-devices-role/`（3 PNG，`0084e0ae` 加入）。
- **证据**（亲证）：对照系——`2026-10-14-actions` 由 design 15 挂链（15:37,39）；`mixer-render-isolation` 有自有 README 且 design 16:18 引用；`application/` 与 `application-release/` 由 `docs/designs/screenshots/application/README.md`（"…and mixer cadence measurements"）与 `crates/moiren-app/README.md:23` 共同持有；designs/*.md 对上述两批目录零引用。
- **影响**：设计证据失去文档锚点后即成死资产候选。
- **建议方向**：在 design 20（library-cards 世代）与 design 23（devices-role）各补一行证据链接；不删图。工作量: S

### B-12 docs/experiments/mixer-io-prototype/ 无文档引用 [P3] [NEW]
- **位置**：`docs/experiments/mixer-io-prototype/`（HTML/CSS/JS 交互原型 + tabler 图标）。
- **证据**（亲证）：非 audit 文档树内 `rg 'mixer-io-prototype'` 零命中；designs 13/22 无 "prototype" 引用。
- **影响**：它是 `a17270b3` "rebuild the mixer strip I/O block per the **approved prototype**" 的批准源物——有历史价值但所有权未声明。
- **建议方向**：在 design 13 或 22 补一句"批准原型见 docs/experiments/mixer-io-prototype/"；不删。工作量: S

## 清理候选三档分类（Safe / Review / Protected）

**Safe（可再生/确认冗余）**
- 本地面（不入仓库史）：`.worktrees/` 有 37 个目录但 `git worktree list` 仅注册 18 个 → 约 19 个未注册残留，可 `git worktree prune` + 删目录；`.tmp/`（约 170 个会话产物：png/json/log/pid/mixer-*-20261026 等，已被 ignore）可整目录清空。均不影响仓库。
- 仓库内 Safe 档基本为空：worktree 清理战役（`f38cc98e`、`83775596` 等）已扫清可再生垃圾——跟踪树无 .bak/.orig/.log/.zip/dump（`git ls-files` 模式扫描零命中，仅两个假阳性：`output-port.svg` 是图标、`physical/analysis.rs` 是源码）。

**Review（旧脚本/无主归档/证据物——动手前先补所有权或确认）**
- `scripts/managed-stress.ps1`（B-4 孤儿；删或修）。
- `docs/designs/screenshots/{2026-10-30-library-cards,2026-11-10-devices-role}/`（B-11 挂链）。
- `docs/experiments/mixer-io-prototype/`（B-12 挂链）。
- `crates/moiren-engine/examples/signal_internal_state.rs`（全仓无文档引用的自省演示；自含可运行，保留或挂入 engine README）。
- `tools/audio-hack-lab/`（实验档案：REPORT.md 为结论 owner，run 脚本可复现，log 已被 .gitignore:22-24 定向忽略，`pdb/` 为本地未跟踪——保留）。
- `docs/plans/` 9 份 2026-10-07/08 早期计划 + `2026-10-24-code-quality-worklist.md`（自述"file:line may drift"；生命周期归 A 界面，本审不提删）。
- `docs/vad-*.md` 5 份日期戳证据日志（owner = `driver/vad/STATUS.md` 头部明言"the other docs/vad-*.md records are dated evidence logs"——保留；移动/归档属 A）。

**Protected（迁移/pin/门禁依赖/再生路径不明的产物）**
- `driver/vad/**`（ACX 驱动栈：MS-PL pin 于 PROVENANCE.md、STATUS.md 持 gate 结论；硬件验证面，勿动）。
- `Cargo.lock`（`--locked` 门禁依赖）、`rust-toolchain.toml`、`.gitattributes`（EOL 契约）、`.gitignore`。
- `tools/rt-check/`（CI rtcheck 面的 RT 契约夹具）。
- `docs/designs/evidence/*.json`（low-latency soak 测量证据）。
- `docs/audit/2026-10-27|10-31|11-03/`（有 README+生命周期标记的审计惯例产物，`485e1fef` 统一标记过）。
- `assets/logos/`（`crates/moiren-app/build.rs` rerun-if-changed 消费）。
- `.cargo/config.toml`（有意不入库：`.git/info/exclude` 排除且文件头自述热/并发限值——本地配置的正确姿势）。

## 正面登记
- 零 `dbg!/todo!/unimplemented!`；crates 内 TODO/FIXME/HACK 计数为 0（`rg -c` 全空）。
- 生产路径 eprintln 全部是统一模式的错误上报（"Moiren tray unavailable: {error}" 等 17 处）或明确标注的 capture/profiler harness 输出；`crates/moiren-windows-audio/tests/common/mod.rs:22` 对 println 原子性的注释是 why 级注释的样板。
- AI 冗余输出有 CI 级防线：i18n debug-format 泄漏 guard tests（`bad2ded3`）、display-name 清零 allowlist（`1f66a6ec`）——既往"调试 eprintln 入库"事故（`818257df` 回修）后已制度化。
- `detect-changes.sh` 自带自测且每次 CI 必跑（yml:56-57）；未知路径保守全跑（sh:82-84）。
- `Cargo.toml` workspace.lints deny 对齐 CI `-D windows`，显式注释防本地/CI 漂移；`windows =0.62.2` 精确 pin 带 API 理由。
- `.gitignore` 分区注释全部带理由（"AI Slops" 段、audio-hack-lab log、driver VS 输出）。
- 提交规范遵循率高：近 60 非合并提交 100% conventional（仅 B-5 两例在其外，60-100 窗口）。
- 提交信息可读性佳（原因式 subject，如 "wake the control pump when the service publishes"）。

## 明确不建议
- **不删** `docs/vad-*.md` 证据日志——`driver/vad/STATUS.md` 显式声明它们是 linked evidence logs。
- **不删** `docs/audit/` 历史周期——仓库已建立审计 README+生命周期标记惯例。
- **不删** `tools/audio-hack-lab/` 与其 REPORT.md——结论 owner 明确，log 再生路径已被 ignore 规则覆盖。
- **不加** rustfmt.toml / clippy.toml / .editorconfig / CODEOWNERS / codecov / dependabot：现配置面刻意最小且维护良好（默认 rustfmt + workspace lints + .gitattributes EOL 已覆盖全部现实压力点）；.editorconfig 与 .gitattributes 职能重复；单人协作 CODEOWNERS 无压力。
- **不把 miri 强加给 moiren-plugins**（真插件 FFI 宿主不可 miri；先补 test+clippy 门禁即达目的）。
- **不清** `docs/designs/screenshots/application/`——ui_capture 默认输出位且有 README owner。
- **不重写** `284b911e`/`58d412a9` 已发布历史。

## Top5
1. **B-1** moiren-plugins 门禁裸奔（P1，KNOWN-OPEN W-1）——补映射+清单+自测，S。
2. **B-2** signal lib.rs Stream 门面注释反向失实（P2 KNOWN-OPEN）——改写门面段，S。
3. **B-3** clap abi.rs "原子计数器排水" 注释失实（P2 KNOWN-OPEN）——改写注释，S。
4. **B-4** managed-stress.ps1 孤儿脚本（P2）——删除或改指现存探针，S。
5. **B-5** main 上的 wip/WIP 混合提交（P2 流程）——协作约定补条目，防再犯，S。
