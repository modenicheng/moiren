# 05 既往审计遗留处置 — 初审 A（2026-11-09 审计全量现状）

- **基线**：main @ `e2ed3d6a`（2026-11-11）；**对照基线**：`27aa57d7`（2026-11-09 审计基线）。
- **区间工作量**：`git log --oneline 27aa57d7..e2ed3d6a` 实测 40 提交、76 文件变更，全部为 #158-#168 战役（UI 草稿反馈、可见性收紧 #159-#162、mixer I/O 重组 #163、i18n 术语 #164、推子手感 #165、设备页重建 #167、删 UI session ledger #168）。**没有任何提交以 11-09 审计发现为对象**。
- **方法**：P1×1 + P2×15 共 16 条全部亲证——打开当前 main 引用文件/行逐一核对（不看 diff 下结论，看当前代码）。P3×33 采用两级规则：引用文件在区间内被触碰的逐条亲证；未触碰的推定未修（标注"推定"，其中 5 条顺手用 ls/grep 升级为亲证）。行动项两项单独核实。
- **数法备注**：DYNAMIC_KEY_EXCEPTIONS 条目数用 `awk '/DYNAMIC_KEY_EXCEPTIONS/,/^];/' | grep -c '^\s*"'` 统一数法，两基线同法对比（11-09 报告的"116 条"是另一数法，不影响机制判定）。

## P1/P2 逐条状态表

**总判定：0/16 修复。** 无一"部分修"——各发现引用的机制、结构、文档全部原样，部分引用文件虽有触碰（graph.rs、mod.rs、mailbox.rs、mixer/model.rs、node_inspector.rs、shell/project.rs、recovery_host_tests.rs）但触碰内容（推子手感、endpoint identity、publish waker 等）与发现正交。

| ID | 级别 | 标题 | 状态 | 证据（当前 main 亲证） |
|---|---|---|---|---|
| W-1 | P1 | CI surface 映射漏 moiren-plugins：其 8 测试 + clippy 在任何 CI 路径下不执行 | **未修** | `.github/scripts/detect-changes.sh:53`（ORDER）与 `:56-62`（direct）仍是 5 crate，`case` 分支 `:71-79` 无 `crates/moiren-plugins/*` 模式；`.github/workflows/runtime-foundation.yml:48-49` push 硬编码列表仍 5 包；全 workflow 无任何 `-p moiren-plugins`。区间内两文件均未被触碰 |
| W-2 | P2 | AGENTS.md 结构段漂移：漏 plugins crate 与 driver/tools/scripts | **未修** | `AGENTS.md` 区间内零提交（不在 76 文件清单）；当前结构段仍列 5 crate 无 moiren-plugins，无 driver/、tools/、scripts/ 提及；:34 仍写 "pushes to main always run the full suite"（因 W-1 仍失实） |
| APP-1 | P2 | 错误"英文句子即 key"三方逐字同步，edit_hint 子串匹配脆弱 | **未修** | `service/graph.rs:562-576` `edit_hint` 仍以 `error.contains("not bound to an input source")` 等 5 组子串配 hint；`service/graph.rs:53-57` `GraphFault.cause` 仍是 `String`（未类型化）；DYNAMIC_KEY_EXCEPTIONS 同法计数 139→136（区间内 #164 微降 3 条，机制原样）；UI 侧 `t!(fault.cause.as_str())` 渲染点未变 |
| APP-2 | P2 | project → service 反向依赖：持久化层组装运行时上下文 | **未修** | `project/startup.rs:44` `into_context` 仍构造 `crate::service::ServiceContext`（:48 `ServiceContext::with_settings`）；project/startup.rs 未被触碰 |
| APP-3 | P2 | service/mod.rs 三合一（声明+门面+核心实现） | **未修** | `service/mod.rs` 561→572 行；`ServiceContext`(:112)、`CoreDriver`(:306)、`ServiceRuntime`(:395)、start_service(:52) 仍同居一文件，未拆 context.rs/runtime.rs |
| AU-1 | P2 | 跨页滑杆编辑内核藏于 mixer 页 + shell glob 转运 + graph↔mixer 双向依赖 | **未修** | `pages/graph/compressor.rs:3` 仍 `use crate::ui::shell::mixer::{SliderDispatch, ...}`（转运经 `shell.rs:12` `pub(crate) use super::pages::{..., mixer, ...}`）；`pages/mixer.rs:1` 仍反向 `use super::graph::{OBSERVATION_SAMPLE_CADENCE, observation_snapshot}`；内核仍在 `pages/mixer/model.rs`（1276→**1458 行**，#165 推子手感使其继续增肥）。双向依赖与误导路径全保留 |
| AU-2 | P2 | shell/project.rs 四种职责混排 | **未修** | `shell/project.rs` 1062→1055 行；menu() ~470 行单函数、ProjectMessage/PersistedFailure 类型、i18n 文案映射（:591/:636）、StudioShell IO（:729 起）仍同居，未拆 project/{menu,messages}.rs |
| AU-3 | P2 | node_inspector.rs 双巨函数 + 字段 schema 混在渲染文件 | **未修** | `pages/graph/node_inspector.rs` 1311→1315 行；`inspector()`(:46 起) 与 `builtin_controls()`(:613 起) 两巨函数 intact；BuiltinFieldSpec(:34) 仍在渲染文件，无 builtin_fields.rs 子模块 |
| E-1 | P2 | NodeKind 闭合枚举跨 crate 扇出，需 checklist 固化 | **未修（checklist 未写）** | docs/ 与 AGENTS.md 检索无"新增节点 checklist"文档（"checklist" 仅命中旧审计与 design 17 UI 主题）；NodeKind 结构未动。**注意：建议的前半（维持闭合枚举）被遵守，后半（写 checklist）未做**。区间内 core/graph.rs、engine/node/config.rs 的改动是 DeviceMode::title 展示名（i18n 工作），与扇出无关 |
| E-2 | P2(边缘) | Supervisor 28 字段平铺状态机 | **未修（按建议冻结）** | `host/supervisor.rs` Supervisor 仍 28 字段平铺（config…parameter_intent，:52-81 区域逐字段核对）；建议本就是"冻结不修"，符合预期 |
| WA-01 | P2 | 三个同名 "owner" 模块语义互不相干 | **未修** | `src/owner.rs`、`engine_backend/owner.rs`、`virtual_owner/owner.rs` 三者原位（ls 实测）；crate 区间内仅 engine_backend 四文件被触碰，与改名无关 |
| WA-02 | P2 | wasapi.rs 1231 行可目录化 | **未修** | `src/wasapi.rs` 仍 1231 行（区间内零触碰，wc 实测与 11-09 报告行数完全一致） |
| WA-05 | P2 | rt-check 影子测试文件在 crate 内不可发现 | **未修** | `src/asio_rt_tests.rs:1`、`src/engine_backend/recovery_host_tests.rs:1-2` 头部均无"仅由 tools/rt-check 编译"归属注释；crate 内引用仍为零、仅 `tools/rt-check/tests/professional_rt.rs:32,75` 影子挂载。**recovery_host_tests.rs 在区间内被 2f705697 触碰 3 行（endpoint identity），是"顺手加注释"的最佳时机，未加**；engine_backend.rs:50 `mod virtual_tests` 命名与内容不符也原样 |
| WA-08 | P2 | RT 边界识别靠逐模块头注释，缺集中线程模型地图 | **未修** | `src/lib.rs:1` 仍一句话文档；`docs/platform/` 仍只有 gpui-fork.md，无线程/RT 边界表。lib.rs 区间内零触碰 |
| S1 | P2 | signal lib.rs 门面声称 Stream 无生产流量（与现实相反） | **未修** | `moiren-signal/src/lib.rs:6-8` 原文 "The Stream transport is spec-complete but carries no production traffic yet…Latest carries all current publication paths"；moiren-signal 全 crate 区间内零触碰 |
| 06:P1 | P2 | clap 宿主回调注释描述不存在的"原子计数器排水"机制 | **未修** | `moiren-plugins/src/clap/abi.rs:84-86` 原文 "restart/callback requests land in an atomic counter the control side drains opportunistically"；`clap/mod.rs:86-88` 仍是三个空 no-op 函数。moiren-plugins 全 crate 区间内零触碰 |

## P3 处置统计与逐条简表

**统计：0/33 修复。** 其中逐条亲证 17 条（引用文件被触碰或顺手读码确认）、推定未修 16 条（引用文件区间内零触碰）。修复率 0%。

| ID | 标题（略） | 状态 | 证据 |
|---|---|---|---|
| W-3 | rt-check 重复 4 个版本 pin | 亲证未修 | `tools/rt-check/Cargo.toml:13-21` log 0.4 / rtrb =0.4.0 / thiserror 2.0 / windows =0.62.2 原样 |
| W-4 | docs 根部 5 个 vad 散文件 + 双计划目录 | 亲证未修 | `docs/vad-*.md` ×5 原位；`docs/plans/`（10 件）与 `docs/superpowers/specs/` 仍并存 |
| W-5 | engine lib.rs 全裸 pub mod | 推定未修 | `moiren-engine/src/lib.rs` 区间内零触碰 |
| APP-4 | 8 个 glob re-export 抹平 service:: 命名空间 | 亲证未修 | `service/mod.rs:34` 仍 `pub use moiren_engine::audio_config::*;`（mod.rs 被触碰但 glob 未收敛） |
| APP-5 | timeline.rs 测试孤儿模块 | 亲证未修 | `src/timeline.rs` 原位，`lib.rs:20` `mod timeline` 挂载未变 |
| APP-6 | 测试摆放规则未成文 | 推定未修 | AGENTS.md 零触碰；v2_tests.rs / vad_catalog_tests.rs 未动 |
| APP-7 | ServiceMailbox::try_request 测试专用兼容面 | 亲证未修 | `service/mailbox.rs:214` `pub fn try_request` 无 `#[cfg(test)]`；且 `ui/state/transport_tests.rs:32` 新增一个测试消费点（消费面略扩大） |
| APP-8 | edit_parameter_values 可下沉纯函数块 | 亲证未修 | `service/graph.rs:406` 原位（graph.rs 被触碰但该函数未动） |
| APP-9 | service 层内嵌渲染期翻译访问器 | 亲证未修 | `service/graph/mixer.rs:46-50` mode_label 仍直接调 `rust_i18n::t!` |
| APP-10 | WindowsAdapter 内嵌通用 Runtime 文件 | 亲证未修 | `service/exports/runtime.rs` 540 行；WindowsAdapter(:355)、native_can_open(:358)、两处 `cfg_attr(not(any(windows,test)), allow(dead_code))`(:14,:19) 原样（文件被触碰但未拆） |
| AU-4 | 五页入口与页状态不同构 | 亲证未修 | 入口三形状原样：devices.rs:110 mount+page:126、settings.rs:98/109、mixer/page.rs:489 page、matrix.rs:31 page、graph/page.rs:145 mount。#167 重建设备页后仍保留 mount+page 形状 |
| AU-5 | dB/平衡文本格式化双源 | 亲证未修 | `mixer/model.rs:345-401` parse_gain_db/parse_balance/parse_pan/gain_db_text 原位；presentation.rs 仍持另一套 |
| AU-6 | graph/tests.rs 3654 行平铺 | 亲证未修 | wc 实测仍 3654 行，未目录化 |
| AU-7 | 全壳 update_node_meters 在 mixer 页 | 亲证未修 | `pages/mixer.rs:26` `impl StudioShell::update_node_meters` 原位 |
| AU-8 | 过时注释与声明位置 | 亲证未修 | `mixer.rs:14-16` "window::graph::compressor" 旧路径注释仍在；shell.rs 模块声明仍在文件底部(:815-840)；panel.rs/hot.rs 仍在 shell/ 下 |
| E-3 | swap.rs 内联 363 行测试 | 亲证未修 | `runtime/swap.rs:57` `mod signal_tests` 仍内联 |
| E-4 | mod.rs vs .rs+dir 双轨 | 推定未修 | processor/mod.rs 等零触碰 |
| E-5 | engine 内 winmm 平台钩子 | 推定未修 | meter/polling.rs 零触碰（单实例例外，建议本就是不动） |
| E-6 | host.rs glob re-export | 推定未修 | host.rs 零触碰 |
| WA-03 | 双栈时代 stale 注释 3 处 | 亲证未修 | `negotiation.rs:169` "Both stacks only pass…"（SAFETY 契约内，优先级最高那处）、`:195` "The legacy stack additionally treats…"、`virtual_owner/provider.rs:77` "Both native backends…" 全部原文在 |
| WA-04 | bridge.rs 顶层命名含糊 | 推定未修 | bridge.rs 零触碰 |
| WA-06 | run_playback/run_microphone 镜像 | 推定未修 | vad/provider.rs 零触碰（建议本就是保留） |
| WA-07 | virtual builder 镜像 | 推定未修 | virtual_owner/boundary.rs 零触碰（同上） |
| WA-09 | lib.rs 模块文档与 crate 面不符 | 亲证未修 | `src/lib.rs:1` 原文（随 WA-08 一并读取） |
| 06:P2 | scan.rs 注释称 VST2 只在 VST2 目录扫描，实现无此检查 | 亲证未修 | `scan.rs:154-156` 注释原文 vs `format_of`(:158-168) 对任意裸 dll/so 返回 Vst2，矛盾原样 |
| 06:P3 | VST2 描述符空 path 占位 | 推定未修 | vst2/mod.rs 零触碰 |
| 06:P4 | vst3/mod.rs 1464 行宿主对象层 | 推定未修 | vst3/mod.rs 零触碰（建议本就是等顺风车） |
| S3 | 写守卫命名/入口对偶不自觉 | 推定未修 | host/scope.rs 零触碰 |
| S4 | 订阅者上限 64 字面量散布 | 推定未修 | registry.rs/storage.rs/demand.rs 零触碰 |
| S5 | budget.rs 对 triple_buffer 私有布局建模 | 推定未修 | budget.rs 零触碰（登记性发现） |
| 06:P7 | has_editor/is_synth 两处语义不一致 | 推定未修 | spec.rs 零触碰 |
| 06:P9 | 测试夹具运行时 shell rustc | 推定未修 | tests/plugin_ffi.rs 零触碰（登记性发现） |
| 06:S8 | transport 内置 cfg(test) 测试缝 | 推定未修 | transport/latest.rs、stream.rs 零触碰（判定为可接受现状） |

## 行动项核实（X-1 锚点注释 / 明确不建议 12 项）

**X-1 锚点注释：未加。** `service/managed/meters.rs:1-3` 模块头仍是原三行 doc（"Copy bounded latest windows between the lifecycle-owned host directory and the stable application topics…"），没有一行指向 11-09 X-1 裁决（"双 level 目录是刻意分层中继，非重复实现"）的设计锚点。meters.rs 区间内零触碰。零成本行动项，建议随任一 service 侧 PR 顺路补上。

**"明确不建议"清单：12 项零违反。** 逐项核对（防止这波功能 PR 拆了不该拆的/造了不该造的抽象）：

1. 不为 app service/project 拆 crate —— 遵守（workspace members 仍 6 crate，根 Cargo.toml 实测）。
2. 不给 engine 加 facade —— 遵守（engine/src/lib.rs 零触碰）。
3. 不为 rt-check 版本同步加 CI 检查 —— 遵守（workflow 零触碰）。
4. 不为行数拆 managed/driver.rs —— 遵守（零触碰）。
5. 不拆 ServiceGraph 状态机本体 / ServiceContext —— 遵守（graph.rs/mod.rs 被触碰但结构未拆）。
6. 不为行数拆 nodes.rs —— 遵守（零触碰）。
7. 不为行数拆 monitoring.rs —— 遵守（零触碰）。
8. 不为行数拆 shell.rs —— 遵守（被可见性收紧 PR #161 触碰，但子模块集合不变，未拆）。
9. 不为行数拆 mixer/page.rs、不强拆 matrix 单文件页 —— 遵守（mixer 子模块仍 model/page/routing/strip_controls 四件；matrix.rs 零触碰）。
10. 不迁移 shell/panel|hot 到新命名空间 —— 遵守（仍在 shell/ 下）。
11. 不为 NodeKind 改运行时注册表、不强合成 Supervisor Phase enum —— 遵守（闭合枚举原样；Supervisor 仍平铺 28 字段）。
12. windows-audio 不强行收敛域目录、不合并 run_playback/run_microphone 与 virtual builder 镜像、不移 rt-check 影子文件 —— 遵守；signal/plugins 不拆 registry.rs/types.rs、不为 vst2 造包装层、不抽象 instantiate 状态机、不加第二夹具文件 —— 遵守（moiren-signal 与 moiren-plugins 全 crate 零触碰，结构性遵守）。

**反向信号（登记）**：#164（strip 术语保持英文）使 DYNAMIC_KEY_EXCEPTIONS 同法计数微降 3 条（139→136），是区间内唯一向 APP-1 方向的（微小）改善；但类型化本身零进展，且 #165 的展示键工作正是在这套"动态 key 例外"机制上继续施工。

## 结论

- **修复率：0/49（0%）。** P1×1 + P2×15 = 16 条全部未修；P3×33 全部未修（17 条亲证 + 16 条推定）。11-09 以来约一周的工作（#158-#168）全部是平行功能线，无一条审计发现被顺手处理；三处"最佳顺手时机"（recovery_host_tests.rs 被触碰未加注释、i18n_guards.rs 被触碰未动类型化、devices.rs 重建未对齐五页约定）均错过。
- **冻结纪律良好**：12 项"明确不建议"零违反——这波改动没有制造新的架构债，只是没有偿还旧债。
- **建议立即处理（按序）**：
  1. **W-1 + W-2 一个 PR**（~25 行：detect-changes.sh 四处 + workflow 两行 + AGENTS.md 结构段，含自测断言）——P1 是整 crate 测试与 clippy 盲区，拖一天多一天静默，成本几乎为零。
  2. **S1 + 06:P1 同车捎带**（两段纯文档改写，且都是新维护者最先读到的位置）。
  3. **APP-1 GraphFault 类型化**——最大单项债务且仍在被依赖施工（#164/#165 都在动态 key 机制上加码），拖得越久迁移面越大；按 11-09 建议从 graph.rs 验证句先行。
- **次优先**（顺风车项）：X-1 锚点注释（1 行）、WA-05 影子文件头注释（2 行）、AU-8 过时注释（3 行）——合计 <10 行，可搭任意 app/windows-audio PR。
- **对后续排期的含义**：11-09 报告"建议批次"1（立即批）至今未启动，其余批次（高杠杆重构/中优先/文档化/挂起）状态不变，可直接沿用其排期建议，无需重审。
