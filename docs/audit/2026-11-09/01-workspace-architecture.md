# 2026-11-09 架构审计 — 工作区与跨 crate

基线: main @ 27aa57d7（实测 `git rev-parse --short HEAD`，工作区干净）

范围：6 个 crate 的 Cargo.toml 全文亲读、依赖方向与 pub 面、workspace 一致性、根目录组织（driver/、tools/、scripts/、assets/）与 CI 覆盖、docs/ 健康度、AGENTS.md 漂移、既往「无需拆 crate」结论复核。只读审计：未改任何源文件、未运行 cargo。同轮已有并行子报告 `02-app-service.md`、`04-engine-core.md`、`06-signal-plugins.md`，本文只覆盖工作区/跨 crate 层面，已扫描三份报告避免重复上报。

## 执行摘要（≤10 条要点）

1. **实测依赖图是 3 层 DAG、无环、全部向下**：叶子层 core/signal/plugins → engine（core+signal+plugins）→ windows-audio（core+engine）→ app（core+engine+signal+windows-audio）。没有任何向上或横向违规依赖。
2. **「app 如何拿到 windows-audio」的答案**：`crates/moiren-app/Cargo.toml:57-59` 的 `[target."cfg(windows)".dependencies]`（非 [dependencies]、非 re-export、非 main.rs 直依赖），另有 `:53-55` 非 cfg 门控的 dev-dependencies（供 Linux 上跑可移植渲染测试，注释写明意图）。代码侧门控完全对齐：`src/service/mod.rs:22-23` `#[cfg(windows)] mod windows`、`:38-39` 条件导出 `WindowsBackend`，model.rs:225-229、managed.rs:127-243、exports/runtime.rs:354-529 逐项 cfg；非 Windows 构建 app 可编译，`start_service` 返回 `UnsupportedPlatform`（mod.rs:73-76）——刻意保住 Linux CI 的设计，接线干净。
3. **windows-audio → engine 的方向是正确的六边形架构**：engine 在 `crates/moiren-engine/src/host.rs` 定义 `EngineBackend`/`PreparedBackend`/`BackendError` port，windows-audio 的 `ProfessionalBackend` 实现它（`engine_backend.rs:21-31` import、`:217/:228` trait 方法签名）。平台 adapter 依赖引擎抽象 = 依赖倒置的正确形态；反向（engine 依赖 windows-audio）才错。单后端现状下不需要额外的抽象 crate。
4. **engine → plugins 同样正确且极薄**：plugins lib.rs:4-6 自述 "deliberately a leaf… The engine crate adapts"；engine 全部消费集中在 `src/processor/plugin.rs` 一个文件（PluginInstance/PluginParamEvent/spec 类型/PluginError/instantiate，共 6 处引用）。plugins 保持零 moiren 依赖。
5. **W-1（P1，本轮最重要发现）：CI surface 映射漏了 moiren-plugins**。`detect-changes.sh:52-58`（ORDER/direct）与 `:69-73`（crate 目录模式）均无 `crates/moiren-plugins/*`；它落入 `:84` 的 catch-all `full=true`，但 full 只把那 5 个包回填为 direct（`:88-92`）；push 到 main 的硬编码列表（`runtime-foundation.yml:48-50`）同样是 5 包。结果：**`cargo test -p moiren-plugins` 与 `cargo clippy -p moiren-plugins` 在任何 CI 触发路径下都不会执行**——plugins 的 6 个 FFI 集成测试（tests/plugin_ffi.rs）+ 2 个 vst3/abi 单测从不运行，clippy `-D warnings` 纪律对该 crate 失效。编译覆盖仍免费（engine 依赖它、`cargo fmt --all` 覆盖它），所以是盲区而非断裂。时间线：detect-changes.sh 定稿于 2026-10-26（e4ac768f，5 crate 时代），plugins 2026-11-03（d7f95567）才加入，映射从未更新。
6. **W-2（P2）：AGENTS.md 项目结构段与现实漂移**。`AGENTS.md:7-11` 只列 5 个 crate（漏 moiren-plugins；AGENTS.md 最后改于 2026-11-01 `284b911e`，早于 plugins 加入两天）；结构段对 driver/（C++ ACX/KMDF 驱动，独立 build.ps1 构建）、tools/（rt-check 独立 cargo workspace + audio-hack-lab 实验脚本）、scripts/（PS1 压测/soak）只字未提。且 `AGENTS.md:34` "pushes to main always run the full suite" 因 W-1 而失实。
7. **workspace 一致性整体优秀**：6/6 crate 均挂 `[lints] workspace = true`；所有跨 crate 共享依赖（thiserror/anyhow/flexi_logger/log/rtrb/serde/serde_json/windows/windows-core）100% `workspace = true` 声明——2026-10-27 审计建议的依赖集中化已落地（根 Cargo.toml:12-25，含 windows 家族 `=0.62.2` 单版本钉扎及其原因注释）；单用户依赖（gpui-kit/tray-icon/libloading/triple_buffer 等）留在 crate 本地是可辩护约定。feature 组织同样清晰：全部 feature 都是「门控非默认消费面」（probe-tools/vad-probe/gpui-test/ui-profiler/vad-integration-probe/component-gallery），各有注释 + `required-features` 钉住 example，CI 还显式 lint 门控面（runtime-foundation.yml:177-184）。
8. **pub API 面收敛度分层不一但均可接受**：signal 是精收 facade（lib.rs:25-43 只导出类型）；windows-audio 是 pub/pub(crate)/feature 三态治理的样板（lib.rs 全文可见 probe 模块全部 feature 门控、内部模块 pub(crate)）；engine 是「模块树即 API」风格（lib.rs 15 个裸 `pub mod`），未发现内部模块意外泄漏；app 的 `pub mod ui` 面大但它是栈顶无人消费。抽样：app 实际消费 windows-audio 约 11 个不同项（catalog_snapshot、asio、process_loopback、engine_backend 三项、vad 六项）——44k 行 crate 的公共面被用得很集中。
9. **X-1（engine/app 双 level 目录）仍开放且两侧都长大**：engine `src/host/signals.rs` 现为 802 行（2026-10-27 审计记录时 540），app `src/service/signals.rs` 608 行。2026-11-03 UI 审计裁定「维持独立决策」，本轮并行 06 报告则称其为「已解决的分层中继形态」——两结论存在张力，登记待裁决，不新开发现。
10. **「无需拆 crate」结论在 160k 行规模下维持**（详见对账节）。唯一理论拆分缝是 app 的 service/project 层（实测 service 是 gpui-free 的，两处 "gpui" 命中均为注释/feature 名），但 53 个 UI 文件消费 `crate::service`，缝宽且同 crate 模块纪律良好，现在拆只会搬代码不减耦合。

## 发现清单

| ID | 级别 | 标题 | 位置 | 证据摘要 | 建议+权衡 |
|---|---|---|---|---|---|
| W-1 | P1 | CI surface 映射漏 moiren-plugins，其自有测试与 clippy 永不运行 | `.github/scripts/detect-changes.sh:52-58,69-73,84,88-92,106-118`；`.github/workflows/runtime-foundation.yml:48-50` | ORDER 数组与 direct 映射均为 5 crate；`crates/moiren-plugins/*` 无模式匹配 → 落 catch-all full，但 full 只回填 5 包；push 硬编码列表同样 5 包；全 workflow 无任何 `-p moiren-plugins`。plugins 有 8 个测试（6 集成 + 2 单测）从不被 CI 执行；clippy 从不 lint 它。映射写于 10-26（e4ac768f），plugins 11-03（d7f95567）加入后未更新 | 一个小 PR：detect-changes.sh 加 ORDER/direct/目录模式/closure 注释四处 + workflow 48-50 三行硬编码加 moiren-plugins + detect-changes.test.sh 补断言（该自测 132 行、assert 结构现成）。风险≈0（纯增量映射，自测护住回归）。收益：补上整 crate 的测试/clippy 盲区 |
| W-2 | P2 | AGENTS.md 项目结构段漂移：漏 plugins crate 与 3 个根目录 | `AGENTS.md:7-13,34` | 结构段 5 个 crate bullet 无 moiren-plugins（11-03 加入，AGENTS.md 停在 11-01）；无 driver/、tools/、scripts/ 任何提及；:34 "pushes to main always run the full suite" 因 W-1 失实 | 修 W-1 时顺手更新：结构段补 plugins bullet 与根目录一段、:34 措辞对齐。成本几行文档；不修则新会话/新贡献者对仓库全貌认知有洞 |
| W-3 | P3 | rt-check 独立 workspace 重复拷贝 4 个版本 pin | `tools/rt-check/Cargo.toml:13-21` | log="0.4"、rtrb="=0.4.0"、thiserror="2.0"、windows="=0.62.2" 与根 workspace 数值一致但是拷贝；独立 workspace 有注释理由（target 目录隔离），root windows pin 的存在理由（单版本编译失败式防降级）在这里同样成立却只能靠人肉同步 | 维持现状为主（独立是刻意设计、版本漂移多为良性 semver 兼容）；若要护栏，最低成本是 push CI 里 diff 这 4 行与根声明——但有过度工程嫌疑，仅登记 |
| W-4 | P3 | docs/ 根部散置 5 个 vad 日期文件 + 双计划目录，与 docs/validation/ 并存 | `docs/vad-handoff-2026-10-21.md`、`vad-lifecycle-validation.md`、`vad-low-latency-goals-2026-10-28.md`、`vad-soak-validation.md`、`vad-validation-2026-10-21.md`；`docs/plans/`（9 件）与 `docs/superpowers/specs/`（11 件，10-18 冻结）并存 | TODO.md（11-08 活跃）+ designs/ 编号文档是 canonical 主线；根部 vad 散文件是历史地层；driver/README.md:15-18 引用其中两个并自述 superpowers/plans 已删 | repo-steward 式纯移动：根部 vad-*.md 收进 docs/validation/（git mv + 修 driver/README 等引用约 3 处）。低优先，等下次动 docs 时顺路；不建议专门起工单 |
| W-5 | P3 | engine lib.rs 全裸 `pub mod` 风格，无 facade 收敛 | `crates/moiren-engine/src/lib.rs:3-17` | 15 个 `pub mod` 全部公开、无 re-export 层；对比 signal/windows-audio 的精收风格不齐 | 风格选择而非缺陷：模块树即 API 在 Rust 常见，且未发现内部模块意外 pub 的实例；强行加 facade 会制造一次性 churn（所有 use 站点重写）。仅当某模块需要降级为 pub(crate) 时局部处理，不做整体改造 |

正面确认（非发现，防未来误判）：6/6 `[lints] workspace = true`；共享依赖 100% workspace 化；`[patch.crates-io]` gpui-pre-windows fork 有文档指针（根 Cargo.toml:38-44 + docs/platform/gpui-fork.md）；app 对 windows-audio 的 dev+target 双声明是有意为之（Cargo.toml:53-55 注释）；fmt job（`cargo fmt --all`）覆盖全部 6 crate 包括 plugins。

## crate 依赖图（实测）

```
第一层（叶子，零 moiren 间依赖）
  moiren-core      (1.8k 行)  thiserror
  moiren-signal    (9.7k 行)  thiserror, rtrb, triple_buffer=9.0.0, crossbeam-utils=0.8.23
  moiren-plugins   (6.6k 行)  thiserror, serde, libloading        [2026-11-03 新增]

第二层
  moiren-engine (34.4k 行) → core + signal + plugins; rtrb, thiserror, serde
                             （plugins 消费仅 processor/plugin.rs 一个适配文件）

第三层
  moiren-windows-audio (44.0k 行) → core + engine; rtrb/anyhow/log/serde/serde_json/thiserror
                                    + [cfg(windows)] windows/windows-core
                                    + dev: signal
                                    （实现 engine::host::EngineBackend port）

第四层（栈顶）
  moiren-app (73.9k 行) → core + engine + signal
                          + [target.cfg(windows)] windows-audio + windows-registry/windows
                          + dev: windows-audio（非门控，Linux 测试用）
                          + gpui-kit, tray-icon, rust-i18n, raw-window-handle, flexi_logger
```

仓库外围（均不在根 workspace members，根 Cargo.toml:3-10 只有 6 crate）：
- `driver/vad/`：C++/ACX-KMDF 驱动，build.ps1 独立构建，README 声明"无任何 Rust/Engine/应用路由默认使用"；CI 经 `vad` surface 跑可移植契约测试（runtime-foundation.yml:185-195）。
- `tools/rt-check/`：刻意独立的小 workspace（自带 `[workspace]` 空表与 Cargo.lock），shadow-include 库源码做 RT 契约测试；CI 经 `rtcheck` surface（workflow:107-109），依赖闭包映射正确（detect-changes.sh:115-118）。
- `tools/audio-hack-lab/`：实验脚本集，CI 刻意 inert（detect-changes.sh:15 头注释明示）。
- `scripts/`：PS1 压测/soak 脚本，无显式映射 → 落 catch-all 触发全量（保守方向，无害）。
- `assets/logos/`：共享 logo，inert。

## 与既往审计对账

| 既往项 | 本轮状态 | 证据 |
|---|---|---|
| 2026-10-22「架构健全、无需拆 crate」 | **维持**（独立复核，见下节） | 本报告全部分层证据；并行 04-engine-core.md:76 同样维持 |
| 2026-10-27 重叠审计 → 依赖集中化建议（serde/rtrb/serde_json/windows 提升 workspace.dependencies） | **已落地** | 根 Cargo.toml:12-25 现有 9 项集中声明，含 windows 家族单版本钉扎 + 原因注释 |
| 2026-10-27「workspace 仅 4 项」的时代 | 已翻页 | 同上；各 crate manifest 亲读确认共享依赖 100% `workspace = true` |
| 2026-11-03 X-1「engine/app 双 level 目录」 | **仍开放，双侧长大**（engine host/signals.rs 540→802 行；app service/signals.rs 608 行） | 11-03 审计裁定「维持独立决策」；本轮并行 06 报告改称「已解决的分层中继形态」——**两结论有张力，建议主代理/用户裁决一次**，本文不重复展开（06 报告已按其口径覆盖） |
| AGENTS.md 结构段漂移（任务简报疑点） | **证实**，即 W-2 | 见发现清单 |
| 「app 的 [dependencies] 只列 core/engine/signal，windows-audio 从哪来」（任务疑点） | **已解**：target-gated + dev 双声明，代码侧 cfg 对齐 | 见执行摘要第 2 条 |
| 2026-10-22 审计报告本体 | docs/audit/ 下**不存在** 2026-10-22 目录（最早为 2026-10-27）；该结论以口头基线形式被引用（04-engine-core.md:76） | `ls docs/audit` 实测：2026-10-27、10-31、11-03、11-09 |

**「无需拆 crate」独立复核（160k 行规模）**：维持。论据：
1. 3 层 DAG 每层职责单一且方向正确（port/adapter 已实证），无循环或横向依赖压力。
2. 大 crate 内部模块化在承压：windows-audio 44k 内 vad/ 11k、engine_backend/ 5.2k、asio/ 4.5k 各自成模块；app 73.9k 中 ui/ 46.9k（pages 27.4k/shell 9.2k/widgets 2.7k）、service/ 14.4k、project/ 2.9k。没有跨模块的「上帝文件」或双向纠缠信号。
3. 唯一自然拆分缝是 app 的 headless 核（service+project，实测 gpui-free），但 53 个 UI 文件消费 `crate::service`、且 2026-11-03 UI 审计确认模块纪律良好——拆出只会搬代码、不减耦合、还引入 feature/发布编排成本。触发拆分的合理信号（service 需要独立复用方、UI 与服务发布节奏分叉）均未出现。
4. 反向风险也小：6 crate 已是合适数目，再拆的边际收益为负。

## 维护性建议 Top 4（含权衡）

1. **修 W-1（CI 映射补 moiren-plugins）**——最高优先。成本：一个 ~20 行 PR（脚本 4 处 + workflow 3 行 + 自测断言），自带 detect-changes.test.sh 回归护栏；风险≈0。不修的代价是整 crate 的 clippy `-D warnings` 纪律与 8 个测试永久静默。
2. **同步修 W-2（AGENTS.md 结构段）**——与 1 同 PR 或紧随。成本几行；收益是任务简报这类「文档与现实对不上」的疑点不再产生。
3. **X-1 裁决**：11-03「维持独立」与 06 报告「已解决的中继形态」两个口径需要统一（要么在 06 报告口径下关闭 X-1，要么维持开放并在 README/设计文档记一笔中继形态的量化路径）。零代码成本，纯决策。
4. **W-4 顺路收敛 docs 根部 vad 散文件**——repo-steward 式纯移动（git mv 进 docs/validation/ + 修 ~3 处引用），不专门起工单，搭下次 docs 变更的车。

明确**不建议**：为 app 73.9k 拆 crate（论据见对账节）；给 engine 加 facade（W-5）；为 rt-check 版本同步加 CI 检查（W-3，过度工程）。
