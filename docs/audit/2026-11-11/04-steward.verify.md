# 04 repo-steward 卫生 — 独立复核

- 输入: 04-steward.a.md（A-1..A-5 + A·B-1..A·B-7）+ 04-steward.b.md（B·B-1..B·B-12）。编号前缀区分两份初审：`A·B-n` = A 报告的 B 系列条目，`B·B-n` = B 报告条目。
- 基线复核：main @ e2ed3d6a 确认；工作区未跟踪项确仅 `docs/audit/2026-11-09/`、`docs/audit/2026-11-11/` 两目录。
- 方法：每条亲证——文档类逐个打开被引文件核对引文；CI 类逐断言读 `.github/workflows/runtime-foundation.yml` 全文、`.github/scripts/detect-changes.sh` 全文与 `detect-changes.test.sh` 全文；git 类逐个 `show`/`merge-base --is-ancestor`/`log -S`。未跑 cargo。未读本轮其他部分报告。

## 逐条判定

| ID | 初审判定 | 复核判定 | 依据（一句话） |
| --- | --- | --- | --- |
| A-1 | 11-09 审计 7 份未提交 [P2] | **成立** | `git status` 确仅 2 个未跟踪目录；`docs/audit/2026-11-09/` 恰 7 文件；38234b11 提交信息引用"audit fact F12"证实后续 PR 依赖其结论 |
| A-2 | VAD 文档暴露内网 SSH 账号/私钥路径 [P3] | **部分成立（修正）→ 升级 P2** | 暴露内容属实（handoff:107-112,174-175、validation:53-56：`<vad-user>@<lan-host>`、`<key-path>`、BatchMode=yes、EncodedCommand），无密钥材料入库；**但"仅 lan 远端、非公开仓库"前提为假**：`origin = git@github.com:modenicheng/moiren.git` 存在且 origin/main == e2ed3d6a、reflog 有多次 push——文档已上 GitHub 第三方托管 |
| A-3 | FRONTEND.md 指针滞后 + 日期不实 [P2] | **成立** | :5 "更新日期：2026-10-16" 与 :444 `window.rs`/`window/` 链接亲读属实；aa571041（11-08）改名 shell.rs/shell/ 确认死链；38234b11 仅改 §2 Devices 段未触及两处 |
| A-4 | 4 个截图目录无 README 无引用 [P3] | **成立（数字微修）** | 4 目录（library-cards/devices-role/managed-routing/mixer-strip）确无 README 且全文档树零引用；PNG 实数 9 张（2+3+2+2），非 ~11，不影响结论 |
| A-5 | "三 README 文档化已删栈" KNOWN-FIXED | **成立（KNOWN-FIXED 确认）** | 全历史仅 ea42434c 删过 1 个 README；17 份非审计 README 经 rg slint/electron/tauri/webview/wasm 零命中 |
| A·B-1 | AGENTS.md 漏 moiren-plugins 与 driver/tools/scripts [P2][KNOWN-OPEN] | **成立（证据细节修正）** | AGENTS.md:7-11 仅 5 crate（实际 6 成员）、:13 结构段无 driver(46 文件)/tools(14)/scripts(3)；KNOWN-OPEN 核实：2026-11-09 审计 README:44 即 W-2 原文；但"vad-soak 被 detect-changes.sh 引用"为假——实际被 `docs/vad-soak-validation.md:47`、`vad-handoff:184,298` 及自身测试引用 |
| A·B-2 | docs/plans/ 10 份过期计划 [P2] | **成立** | 10 文件清点属实；引文逐字核实（windows-integration:4"正式后端接入尚未开始"、gpui-migration:4"尚未开始任何迁移代码"、w00-passive:5"本切片已完成"）；basic-io 17/17 全勾、w00 两份 7/7 与 6/6 全勾、worklist 零未勾且 :58/:78 记录"有意保留"；f38cc98e"remove landed superpowers plans and stress artifacts"（136 文件 −38027 行）惯例属实 |
| A·B-3 | PRD"首版不开发 VAD"与已落地驱动栈矛盾 [P2] | **成立** | PRD:684 原文核实；头部自标 2026-10-27、§11.3 在 :708-709；driver/vad 46 文件 + AGENTS.md:10"VAD support"为现状；README:48 兜底句确实存在但 A 已正确论证其为进度兜底而非方向性陈述 |
| A·B-4 | TODO.md 状态行过期 [P3] | **成立** | :50-51"待推送评审"原文核实；TODO 末次提交 158d3247（11-08 11:53）早于 3218cb87 = Merge PR #108 from feat/canvas-context-menus-remerge（同日 17:20） |
| A·B-5 | 5 份 designs 无状态标注 [P3] | **成立** | 缺 `状态：` 者恰为 14/16/17/18/22 五份；注意 19 用半角冒号 `状态:Implemented`（易漏检）；designs/11 死链有其历史状态头（:4）覆盖 |
| A·B-6 | vad-* 5 份证据日志归置 [P3] | **成立** | 5 文件在 docs/ 顶层、docs/validation/ 仅 asio-v1/backend-nodes；STATUS.md:3-4 canonical 声明与 handoff 头部 Update 横幅亲读属实 |
| A·B-7 | 3 份 superpowers specs 死链 [P3] | **成立** | 恰 3 文件各 1 处 `../plans/` 死链（app-runtime:111、engine-managed-lifecycle:17、rt-ui-signal-bus:277）指向已删的 docs/superpowers/plans/；第 4 处命中（physical-capture:7）指向 `../../plans/` = docs/plans/ 为活链，A 正确排除 |
| B·B-1 | moiren-plugins 全门禁裸奔 [P1][KNOWN-OPEN] | **成立** | 六项断言逐一验证：(1) detect-changes.sh:52-59 ORDER/direct 仅 5 crate、:66-86 无 `crates/moiren-plugins/*` 分支落入 `*) full=true`；(2) push-main 硬编码清单 yml:48-50 无 plugins；(3) miri 矩阵 yml:206-220 仅 core/signal/audio/engine；(4) fmt 全局覆盖（yml:66 `fmt --all`）；(5) engine/Cargo.toml:11 依赖 plugins（仅作依赖被编译）；(6) 自测 test.sh 无 plugins 用例。plugins 7+2 个测试在任何路径不执行；AGENTS.md:34"push 全量"对 plugins 失实；KNOWN-OPEN W-1 与 2026-11-09 审计 README:35 对上 |
| B·B-2 | signal lib.rs Stream 注释反向失实 [P2][KNOWN-OPEN] | **成立（且比初审更重）** | lib.rs:6-9 原文核实；app signals.rs:26-27 `LevelTopic/SpectrumTopic = StreamBlock` + :141,146,151,188,244 生产发布属实；**另发现 engine `runtime/swap.rs:314` 也在 `register_stream_block`**——生产流量在两个 crate 都存在 |
| B·B-3 | clap abi.rs"原子计数器排水"注释失实 [P2] | **成立** | abi.rs:84-86 原文、mod.rs:86-88 三个空体 stub 亲读；crate 内无任何计数器/排水实现（vst3 的 AtomicU32 是 COM 引用计数，无关）；`git log -S 'atomic counter'` 唯一命中 d7f95567（诞生提交） |
| B·B-4 | managed-stress.ps1 孤儿 [P2] | **成立** | 默认参数指向不存在的 `managed_stress.exe`（全仓 examples 无此名）与已删的 `docs/superpowers/plans/2026-10-14-engine-stress`；除审计外全仓零引用；f38cc98e 已清 stress artifacts |
| B·B-5 | main 混入 wip/WIP 提交 [P2] | **部分成立（修正）** | 284b911e（实测 63 文件，非"30+"）+ 58d412a9 均在 main、作者 Lidozs55、违反 conventional 规范属实；**但"近 60 提交中 >25 文件的提交为 0"为假**：d5e51d11（26 文件）、10f3be7e（30 文件）同窗存在——两者为 conventional 机械令牌清扫，故"wip 快照族是唯一规范性违规"这一结论仍成立 |
| B·B-6 | 空提交重触发 CI [P3] | **成立** | be56cf50（11-10 00:25）零文件变更、主题自述"opened event lost to the proxy"；yml `pull_request:` 未显式 types（默认集含 synchronize/reopened），与初审判断一致 |
| B·B-7 | CI 注释引用已删路径 [P3] | **成立** | yml:175-176 注释引用 `docs/superpowers/plans/2026-10-19-repo-steward-remediation.md`；`docs/superpowers/` 下仅 specs/ |
| B·B-8 | .gitignore .worktrees/ 重复 [P3] | **成立** | :44-45 与 :60-61 两条同路径异措辞亲读确认 |
| B·B-9 | capture.rs:300 一行式 eprintln [P3] | **成立** | :300 单行循环体确认；:345/:416/:426 均为带标签结构化输出，反差属实 |
| B·B-10 | 工具链浮动 stable [P3] | **成立** | rust-toolchain.toml `channel = "stable"`；`git log --follow` 仅 ec861422 一次触碰；CI 四处 `rustup toolchain install stable`（yml:65/98/155/227） |
| B·B-11 | 两批截图缺所有权挂链 [P3] | **成立（与 A-4 部分重复）** | 两目录全文档树零引用核实；对照系（design 15:37,39、16:18）核实；系 A-4 四目录的子集，建议合并处置 |
| B·B-12 | mixer-io-prototype 无引用 [P3] | **部分成立（修正）** | designs 13/22 确零"prototype"提及，但 `crates/moiren-app/assets/icons/manifest.json:151` 明文引用该路径作为手绘图标出处——"零命中/所有权未声明"过；缺的只是 design 13/22 的文档锚点，建议不变 |
| 清理三档 | Safe/Review/Protected 分类 | **成立** | Safe 档亲证：`.worktrees/` 37 目录 vs worktree list 19 行（含主检出，即 18 注册）≈19 个未注册残留、`.tmp/` 234 项且两者均被 ignore；无 Protected 项误入 Safe/Review（driver/vad、Cargo.lock、rt-check、evidence json、审计历史均列 Protected 正确）；scripts/vad-soak.ps1 有活引用非孤儿 |

## 误报/降级说明

- **A-2 升级 P3 → P2（复核修正，唯一级别变更）**：初审以"git remote 仅 lan → D:/moiren，非公开仓库"为降级依据，该前提为假——本仓存在 `origin = git@github.com:modenicheng/moiren.git`，origin/main 与 e2ed3d6a 同步且 reflog 显示多次 `update by push`，两份 VAD 文档的内网拓扑（VM IP <lan-host>、专用账号 <vad-user>、私钥路径 + BatchMode=yes）已在 GitHub 托管。仓库当前可见性（private/public）无法本地判定，但"仅内网"缓解不成立、托管在第三方且可见性可翻转。维持恰当级别 P2 的理由：确认无任何密钥材料入库（`git ls-files` 无 .ssh/.pem/id_rsa 命中），暴露的是拓扑与账号名而非凭据本体。修复建议不变：两文档主机/账号改占位符，S 工作量。
- **B·B-5 证据修正但结论维持**：同窗口存在 26/30 文件的 conventional 提交，"违规仅此族"的支撑数据有误；但那两例是单一目的的机械令牌清扫，`wip:` 混合颗粒度快照（63 文件横跨 i18n/UI/服务/根文档）仍是唯一违反提交规范者，P2 流程建议（协作约定补"禁 wip 快照直入 main"）成立。
- **B·B-12 / A·B-1 证据面修正、建议不变**（见上表）。
- 无误报、无已修复条目。

## 复核补充发现（V-1..）

- **V-1 `crates/moiren-app/tests/meter_latency_probe.rs.disabled`（582 行）死文件入库 [P3] [NEW]**
  - 自述"TEMPORARY metering-latency probe — run once ... then delete. Not part of the regression suite"（:1-2），随 B·B-5 的 wip 快照 284b911e 入库；`.disabled` 后缀使 cargo/CI 完全不可见，全仓除审计外零引用（`rg meter_latency_probe` 仅命中自身）。
  - 影响方向：582 行一次性探针以不可编译形态占据 tests/ 目录，既无运行价值也无归档说明，时间久后无人能判断去留。
  - 建议方向：删除（git 历史可寻回）；若测量结论仍有价值，把结论蒸馏进相应 design 或 STATUS 文档后再删。工作量: S。
- （除 V-1 外无其他补充：experiments/portable、experiments/windows、plugins/examples/probe.rs、vad-soak.ps1 均经引用扫描确认有主，不列。）

## 修订后优先级清单（P1/P2/P3）

**P1（1 项）**
| # | 条目 | 来源 | 动作 | 量 |
| --- | --- | --- | --- | --- |
| 1 | moiren-plugins CI 门禁裸奔（KNOWN-OPEN W-1）：detect-changes.sh 映射/ORDER + push-main 三清单 + 自测补 plugins 用例；miri 可暂缓 | B·B-1 | 补映射与清单（closure 方向 plugins→engine→app） | S |

**P2（10 项）**
| # | 条目 | 来源 | 动作 | 量 |
| --- | --- | --- | --- | --- |
| 2 | VAD 文档暴露内网 SSH 账号/私钥路径（**升级**，origin 已是 GitHub 托管；无密钥入库） | A-2 | 两文档改占位符 | S |
| 3 | AGENTS.md 漏 moiren-plugins + driver/tools/scripts（KNOWN-OPEN W-2，建议与 #1 同车） | A·B-1 | 补清单与结构段 | S |
| 4 | 清退 docs/plans/ 10 份过期计划（两份"尚未开始"直接误导；循 f38cc98e 惯例） | A·B-2 | 删除或移历史目录，蒸馏"有意保留"结论 | M |
| 5 | PRD:684"首版不开发 VAD"与已落地驱动栈矛盾 | A·B-3 | §2/§11.3 补修订说明 | S |
| 6 | FRONTEND.md:444 window.rs 死链 + :5 日期不实 | A-3 | 指针改 shell.rs/shell/，补修订日期 | S |
| 7 | signal lib.rs:6-9 Stream 门面注释反向失实（engine+app 双生产落点） | B·B-2 | 改写门面段 | S |
| 8 | clap abi.rs:84-86"原子计数器排水"描述不存在的机制 | B·B-3 | 改写为 no-op 说明 | S |
| 9 | scripts/managed-stress.ps1 孤儿（指向不存在的 example 与已删目录） | B·B-4 | 删除或改指现存探针 | S |
| 10 | main 混入 wip/WIP 混合颗粒度提交（63 文件快照）——协作约定补禁令 | B·B-5 | AGENTS.md PR 节补条目 | S |
| 11 | 提交 2026-11-09 审计报告 7 份（连同本轮报告收口） | A-1 | 随本轮收口一并入库 | S |

**P3（12 项）**
| # | 条目 | 来源 | 动作 | 量 |
| --- | --- | --- | --- | --- |
| 12 | 4 个截图目录无 README 无引用（合并 A-4 + B·B-11：library-cards/devices-role/managed-routing/mixer-strip 共 9 PNG） | A-4, B·B-11 | 各补 1-2 行 README 或 design 挂链 | S |
| 13 | TODO.md:50-51"待推送评审"过期（分支已随 #108 合并） | A·B-4 | 删改该行 | S |
| 14 | 5 份 designs 缺状态头（14/16/17/18/22；19 为半角冒号勿误判） | A·B-5 | 各补一行 `状态：` | S |
| 15 | vad-* 5 份证据日志归置（owner 已收敛，纯组织问题，低优先） | A·B-6 | 可选整批移 docs/validation/vad/ | S |
| 16 | 3 份 superpowers specs `../plans/` 死链 | A·B-7 | 删链或改指 git 历史 | S |
| 17 | tests/meter_latency_probe.rs.disabled 582 行死文件（复核新增） | V-1 | 删除，结论先蒸馏 | S |
| 18 | CI 空提交重触发（代理事件丢失） | B·B-6 | 流程面约定，优先 close/reopen | S |
| 19 | yml:176 注释引用已删 plans 路径 | B·B-7 | 改为直接陈述 | S |
| 20 | .gitignore .worktrees/ 规则重复（:44-45 与 :60-61） | B·B-8 | 留一条 | S |
| 21 | capture.rs:300 一行式无条件 eprintln | B·B-9 | 下次顺手收敛 | S |
| 22 | 工具链浮动 stable（可选，团队已有 workspace lints 缓解） | B·B-10 | 可选锁版本，维持现状亦可 | S |
| 23 | mixer-io-prototype 缺 design 13/22 锚点（icons manifest:151 已引，仅补文档锚） | B·B-12 | design 13 或 22 补一句 | S |

**Top3（修订后）**：① P1 plugins CI 门禁（与 #3 AGENTS.md 同车，一个 PR ~25 行）；② A-2 SSH 暴露占位符化（升级项，S）；③ docs/plans/ 清退（漂移质量最大、两处方向性误导）。
