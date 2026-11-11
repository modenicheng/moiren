# 04 repo-steward 卫生 — 初审 A（repository-hygiene + docs-maintenance）

- 基线：main @ e2ed3d6a（2026-11-10 22:40，PR #168 合并）。工作区未跟踪项仅 2 个目录：`docs/audit/2026-11-09/`、`docs/audit/2026-11-11/`。
- 方法：本机 skill 取证脚本 `docs_inventory.py`（105 份 tracked 文档、6 处 broken-links）、`repo_scan.py`（tracked 文件无杂物信号）、`file_metrics.py`、`docs_drift.py`（无 `.repo-steward.toml` 映射，无输出——脚本输出存 `$TMP/stewardA/`，未写入仓库）；辅以 `git ls-files / log --diff-filter=D / rev-list --objects --all` 大文件普查、`rg` secrets/硬编码路径扫描、逐文件人工核对。未读 `docs/audit/2026-11-11/` 下其他代理报告。

## 发现

### A-1 已完成的 2026-11-09 审计报告（7 份）未提交，滞留工作区 2 天 [P2] [NEW]
- 位置：`docs/audit/2026-11-09/`（01-workspace-architecture.md … 06-signal-plugins.md + README.md，7 文件）
- 证据：`git status --porcelain` 全树仅 `?? docs/audit/2026-11-09/`、`?? docs/audit/2026-11-11/` 两条。仓库既有惯例是审计报告入库（tracked：`docs/audit/2026-10-27/`、`2026-10-31/`、`2026-11-03/` 三轮）。
- 影响：已完成且被后续 PR 引用结论的报告（如 F6/F12 事实已被 38234b11 提交信息引用）只存在于一台机器的工作区，未进版本库；对其他克隆/远端 `lan` 不可见，有丢失风险，也使后续审计无法引用。
- 建议方向：审计收口后随本轮修复一并提交（与 2026-11-11 本轮报告同批）。工作量: S

### A-2 两份 VAD 文档暴露内网测试机 SSH 账号与本机私钥路径 [P3] [NEW]
- 位置：`docs/vad-handoff-2026-10-21.md:107-112,174-175`（`<vad-user>@<lan-host>`、`<key-path>`、EncodedCommand 远程执行示例）；`docs/vad-validation-2026-10-21.md:53-56`（同 IP、同 key 路径）
- 证据：`rg 'BEGIN (RSA|...)|AKIA|ghp_|sk-'` 无命中；`git ls-files | rg '\.ssh|\.pem|id_rsa'` 为空——**未入库任何密钥材料**。`git remote -v` 仅 `lan → D:/moiren`（本机/局域网裸仓），非公开仓库，故降为 P3。
- 影响：一旦仓库未来公开或换远端，内网拓扑（测试 VM IP、专用账号名、key 文件名）随之泄露；BatchMode=yes 表明该 key 可无交互登录。
- 建议方向：两处文档把主机/账号改为占位符（如 `<test-vm>` / `<wdk-user>`），保留命令结构；不必改写历史。工作量: S

### A-3 FRONTEND.md（根，45KB/461 行）实现指针滞后 + 自报日期不实 [P2] [NEW]
- 位置：`FRONTEND.md:5`（"更新日期：2026-10-16"；"现行实现见 … window.rs 壳"）、`FRONTEND.md:444`（链接 `crates/moiren-app/src/ui/window.rs` 与 `src/ui/window/`）
- 证据：`window.rs` 已于 aa571041（2026-11-08 17:34）改名 `shell.rs`/`shell/`（`crates/moiren-app/src/ui/shell.rs`、`shell/` 实存）；最近一次 FRONTEND.md 提交 38234b11（2026-11-10 22:04）更新了 §2 内容但未触及这两处；`docs_inventory.py` 报 FRONTEND.md broken-links=2。
- 影响：FRONTEND.md 是 UI 协作契约（designs 07/12/15 均以它为现状指针），"现行实现见 window.rs" 会把新会话/新贡献者指到不存在的文件；"更新日期 2026-10-16" 与 11-10 实际修订矛盾，削弱其权威性。
- 建议方向：更新 `:444` 指针为 `shell.rs`/`shell/`，行 5 补一次修订日期（2026-11-10）；§5 里的 Slint 历史段落已有明确"历史设计参考"标注，不必清理。**归宿：保留在根目录**（定位是跨会话协作契约，20+ 处 designs 引用它，移动纯属churn）。工作量: S

### A-4 4 个截图目录（约 11 张 PNG）无 README、无任何文档引用 [P3] [NEW]
- 位置：`docs/designs/screenshots/{2026-10-30-library-cards,2026-11-10-devices-role,managed-routing-low-latency,mixer-strip-redesign}/`（各 2-3 张，共 ~11 张）
- 证据：仓库惯例是 screenshots 子目录配 README 说明生成方式（`screenshots/application/README.md` 记载 `ui_capture` 生成命令）；`rg "screenshots/<dir>" docs --glob '*.md'`（排除本轮审计）对这 4 个目录为 0 引用，`docs/designs/23-devices-page-role.md` 亦未引用 2026-11-10-devices-role。PNG 均 <250KB，非体积问题。
- 影响：轻微——证据快照与产生它的设计/验收文档脱钩，时间久了无法判断可否删除。
- 建议方向：给 4 个目录各补 1-2 行 README（来源页面/时间/对应 design），或在对应 design 文档补引用；不必删除。工作量: S

### A-5 既往线索"三 README 文档化已删栈"：现状无法复现，判 KNOWN-FIXED [P3] [KNOWN-FIXED]
- 证据：`git log --diff-filter=D -- '**/README.md'` 全历史仅删除过 1 个 README（`crates/moiren-app/ui/icons/README.md`，随 ea42434c Slint 移除删除）；现行 13 份 README（根、5 crate 中 4 份、asio、assets×4、driver×5、tools）经 `rg 'slint|electron|tauri|webview|wasm'` 扫描零命中。Slint 残留表述只存在于 designs 07/10/12，且均带"已废弃/历史记录"状态头（`docs/designs/07-app-ui-design-system.md:4` 等）。根 README:42-43 提及已删除的 superpowers 计划目录，但属刻意的历史说明。
- 结论：该线索所指状态在 10-14 迁移与 10-31 文档清退（f38cc98e）后已消除，无需行动。

### B-1 AGENTS.md 漏列 moiren-plugins 与 driver/、tools/、scripts/ [P2] [KNOWN-OPEN]（W-2 核实）
- 位置：`AGENTS.md:7-11`（crate 清单 5 项）、`AGENTS.md:13`（结构描述）
- 证据：根 `Cargo.toml` members 实为 6 个（含 `crates/moiren-plugins`，CLAP/VST3 宿主，2026-11-02 落地，见 `docs/designs/19-plugin-host-clap-vst.md:4`）；`driver/` 46 个 tracked 文件（ACX VAD 内核驱动，自带 5 份文档）、`tools/` 14 个（audio-hack-lab、rt-check）、`scripts/` 3 个（vad-soak PSScript，被 `.github/scripts/detect-changes.sh` 引用）均未在结构描述出现。README/引擎侧命令经抽查与 CI（`.github/workflows/runtime-foundation.yml`，Ubuntu/Windows/Miri）一致，无其他漂移。
- 影响：AGENTS.md 是代理/贡献者的第一入口；插件栈与驱动栈完全不可见，会导致改动这两处时跳过对应约束（驱动侧有签名/WDK/rt 约束，风险更高）。
- 建议方向：清单补 `moiren-plugins` 一行；结构段补 driver/tools/scripts 一句话（driver 的构建与测试入口指向 `driver/README.md:7-10`）。工作量: S

### B-2 docs/plans/ 10 份计划全部成为过期追踪文档 [P2] [KNOWN-OPEN]（10-22/10-29 线索核实）
- 位置：`docs/plans/`（10 份，2026-10-07 ~ 2026-10-24）
- 证据（逐份核对完成态）：
  - `2026-10-08-basic-io-nodes.md` 17/17 全勾、`2026-10-08-w00-physical-clock-plan.md` 7/7、`2026-10-08-w00-swdevice-plan.md` 6/6、`2026-10-08-w00-passive-probe-plan.md:5` 自标"本切片已完成"——均已执行完，未清退；
  - `2026-10-24-code-quality-worklist.md` 16 个条目中 13 done + 1 partial-done + 2"评估后有意保留"，**无剩余可执行项**；
  - `gpui-migration-feasibility-handoff.md:4` 仍称"**尚未开始任何迁移代码**"，而迁移于 2026-10-14 完成（ea42434c）；
  - `2026-10-08-windows-integration-plan.md:4` 仍称"正式后端接入尚未开始"、148 项 0 勾，而 `ProfessionalBackend`（WASAPI Shared/Exclusive + ASIO）已交付（`crates/moiren-windows-audio/README.md:3-6`）；
  - `2026-10-07 plan.md` 是立项评审快照，状态表（"moiren-core 仍是模板代码，尚无 LogicalGraph"）早已失效，7 处死链。
- 佐证惯例：仓库 2026-10-31 已用同一手法清退 landed 计划（f38cc98e "remove landed superpowers plans"，git 历史保留）。10-22 审计"tracking doc stale (8 items done)"所指向的状态至今未收口。
- 影响：约 370 行过期追踪与现状直接矛盾（最误导的是两份"尚未开始"），且与 designs/ 里的现行契约竞争"当前真相"。
- 建议方向：按 f38cc98e 惯例删除（git 历史即归档），或整体移入 `docs/audit/2026-10-31/` 式历史目录；worklist 中 A6/A7/A13、A8/A9 的"有意保留"结论若仍有价值，一句话蒸馏进 `driver/vad/STATUS.md` 或 TODO。工作量: M

### B-3 PRD 与现状直接矛盾："首版不开发 VAD" vs 已落地的 ACX VAD 驱动栈 [P2] [NEW]
- 位置：`docs/Moiren-PRD-Roadmap.md:684`（"首版不开发 VAD"）；现状：`driver/vad/`（46 文件，ACX/KMDF 驱动 + `driver/vad/STATUS.md` gate 记录）、`moiren-windows-audio` VAD 支持（AGENTS.md:10 自述"VAD support"）、`docs/vad-*` 5 份验证记录
- 证据：PRD 头部标"更新日期：2026-10-27（增补 …VAD ASIO 接口规划）"，但 10-27 之后 VAD 工作持续（`git log --since=2026-10-27 -- driver/vad` 有 792a3e22、9c34880d、edc63c3b、51dcd2b4 等）；§11.3（:709）把 VAD ASIO 作为规划讨论时未提 ACX VAD 驱动本身已在交付。
- 影响：PRD 是 README 明文链接的"产品与路线图"入口（README.md:48）；新读者读到"首版不开发 VAD"会误判 driver/ 目录为废弃实验。README:48 已有"早期进度段落以当前实现为准"的兜底，但这一条是明确的方向性陈述而非"进度段落"。
- 建议方向：最小补丁——在 §2 非目标或 §11.3 处加一行修订说明（VAD 内核驱动已作为独立工作线落地，gate 见 driver/vad/STATUS.md）；不重写 PRD 全文。工作量: S

### B-4 TODO.md 状态行过期：已完成分支仍标"待推送评审" [P3] [NEW]
- 位置：`docs/TODO.md:50-51`
- 证据："分支 `feat/canvas-context-menus` … 待推送评审"，实际已通过 PR #108 合并（3218cb87，2026-11-08 17:20）；TODO.md 最后一次提交是同日 11:53，晚于其成文。另 §"下一轮工作（设计已对齐，2026-11-08 已完成）" 4 项全 ✅，属已完成历史。
- 影响：小——但 TODO 是唯一的工作项追踪文档，混有决议记录/蒸馏待办/已完成段三种性质，"待推送评审"会让人去找一个已删除的分支。
- 建议方向：删去 :50-51 或改"已合并（#108）"；"下一轮工作"段移入决议记录区或删除。工作量: S

### B-5 23 份 designs 中 5 份无状态标注；无索引可查全局落地状态 [P3] [KNOWN-OPEN]（部分改善）
- 位置：`docs/designs/14-managed-routing-low-latency.md`、`16-render-architecture.md`、`17-two-tier-ui-components.md`、`18-node-cards-and-wiring.md`、`22-fader-feel-async-interaction.md`
- 证据：逐份抽 head 核对——18/23 份带 `状态：` 头（含 20"已批准 2026-11-08"、21"草案 2026-11-09"、23"已批准并实施 2026-11-10"，维护是活跃的）；上述 5 份无。14 被根 README:17 指为 canonical，但文档自身无状态行；18 是 TODO.md:14 确认的现行卡片路线基线，同样无标注。全仓无 designs 索引（rg 确认仅 README/PRD/TODO 散链 4-6 份）。
- 影响：判断"哪份已落地/部分/未动"只能逐文件打开；5 份缺标者恰含两份现行契约（14、18）。
- 建议方向：给 5 份各补一行 `状态：`（写法沿用既有 18/23 格式）。**不建议新建索引文件**（见"明确不建议"）。工作量: S

### B-6 vad-* 5 份证据日志散在 docs/ 顶层，与 docs/validation/ 定位重叠 [P3] [NEW]
- 位置：`docs/vad-handoff-2026-10-21.md`、`vad-validation-2026-10-21.md`、`vad-lifecycle-validation.md`、`vad-soak-validation.md`、`vad-low-latency-goals-2026-10-28.md`
- 证据：`driver/vad/STATUS.md:3-5` 自任"canonical owner of VAD run and gate status; the other `docs/vad-*.md` records are dated evidence logs"——所有权已收敛（好）；但归置上，同类验证记录 asio-v1/backend-nodes 在 `docs/validation/`，VAD 的 5 份在顶层。交叉引用经 docs_inventory 无死链（handoff 有 Update 横幅指向 STATUS，模式健康）。
- 影响：纯组织问题；顶层 docs/ 散件多 5 个，检索 VAD 现状需要跨两目录。
- 建议方向：低优先；若做，整批移 `docs/validation/vad/` 并修 3-4 处相对链接，勿逐份拆。工作量: S

### B-7 3 份 superpowers specs 链接到已删除的 plans 目录 [P3] [NEW]
- 位置：`docs/superpowers/specs/2026-10-10-application-runtime-design.md`、`2026-10-10-rt-ui-signal-bus-design.md`、`2026-10-13-engine-managed-lifecycle-design.md`（各 1 处 `../plans/…` 死链，共 3；docs_inventory JSON 佐证）
- 证据：`docs/superpowers/plans/` 整目录于 f38cc98e（2026-10-31）删除，specs 未同步。其余 broken links：designs/11 ×4（文内自标"部分文件已随重构删除"，可接受）、plans/2026-10-07 ×7（随 B-2 消解）、`docs/audit/2026-11-03/05-autosave-project.md` ×1（历史审计引用旧路径，不动历史报告）。
- 建议方向：3 处死链改为"见 git 历史 f38cc98e 前 plans/…"或直接删除链接；其余随 A-3/B-2 消解。工作量: S

## 正面登记
- **临时文档清退机制真实在运转**：f38cc98e（10-31）删除 24 份 landed superpowers plans；designs 状态头 11-08~11-10 仍在活跃维护（20/21/23 三份新标）。B-2 只是该机制未覆盖到 docs/plans/ 旧存量。
- **canonical-owner + 更新横幅模式**：`driver/vad/STATUS.md` 自任 gate 状态唯一 owner，过期文档（vad-handoff）以 Update 横幅指向它而非篡改历史；根 README:17,21,30 用"canonical 描述见 X"避免复制细节。
- **.gitignore 精修且有理由注释**：`target*/`（含变体 target dir）、audio-hack-lab 日志（附"结论在 REPORT.md"说明）、node-workstations 截图、driver 构建产物；本机专属项走 `.git/info/exclude`（`.cargo/config.toml` 文件内自注"intentionally excluded"）。`.claude/settings.local.json` 精确忽略而非整目录。
- **历史干净**：`git rev-list --objects --all` 全历史 >300KB 的 blob 只有 Inter.ttf（vendor 字体，fonts/README 说明）；无 bak/orig/log/tmp/swap 入库；`repo_scan.py` tracked 文件零杂物信号。
- **实验与产线分离**：tools/audio-hack-lab 独立 crate 不入 workspace、日志忽略、REPORT.md 留结论；被 `.github/scripts/detect-changes.sh` 引用属有意保留。
- **screenshots 有再生路径**：application/README.md 记录 `ui_capture` 生成命令，与 `crates/moiren-app/examples/ui_capture.rs` 对应。

## 明确不建议
- **不建议新建 docs/designs 索引文件**：状态头 + README/PRD 散链已覆盖 23 份中的 18+；索引本身会成为第二漂移面（skill 原则：不为本次审计造索引）。
- **不建议为对称性给 moiren-core / moiren-plugins 补 README**：AGENTS.md 一行 + designs/19 已描述插件栈；core 的契约有 designs/04 现行标注。避免"每 crate 一 README"仪式化。
- **不建议移动 FRONTEND.md 进 docs/**：它是根级协作契约，designs 7+ 处引用其根路径；移动收益为零、churn 为正。修指针即可（A-3）。
- **不建议删除 4 个无引用截图目录**：均为 <250KB 带日期证据快照，补 README 比删除更符合"硬件验证与记录分离"惯例。
- **不建议为 .ruff_cache/、.codex-work/ 加 ignore 规则**：前者是偶发本地缓存（当前未被 status 显示仅因目录暂空/内容为空），后者为空目录；等真正出现噪音再收紧（skill：只对真实生成的本地产物加规则）。
- **不建议现在创建 .repo-steward.toml**：docs_drift 无映射可用，但仓库已用 canonical-owner 横幅人工控漂移且近期见效；等出现"同一文档反复漂移"再上机器映射。
- **不建议改写历史审计/设计文档正文**（含 designs/11 的 Slint 时代路径、2026-11-03 审计死链）：它们是带日期的记录，状态头已标明历史属性；只修"当前真相"文档的链接。

## Top5
1. **B-2 清退 docs/plans/ 10 份过期计划**（两份"尚未开始"直接误导；沿用 f38cc98e 惯例）——M
2. **B-1 AGENTS.md 补 moiren-plugins + driver/tools/scripts**（KNOWN-OPEN W-2，第一入口不可见两大栈）——S
3. **B-3 PRD "首版不开发 VAD" 修订说明**（与已落地驱动栈直接矛盾）——S
4. **A-3 FRONTEND.md 指针更新（window.rs→shell.rs）+ 修订日期**（UI 协作契约的"现行实现"入口失效）——S
5. **A-1 提交 2026-11-09 审计报告**（已完成工作滞留未跟踪区 2 天）——S
