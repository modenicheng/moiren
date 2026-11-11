# 02 可维护性 — 初审 B（跨边界一致性/测试组织/i18n 错误链路）

- 基线: main @ e2ed3d6a, 2026-11-11
- 范围：跨页/跨域依赖方向与 glob 转运、命名语义一致性（owner/RT 前缀）、测试组织（影子测试/摆放/平台门控/时序断言）、i18n 与错误链路剩余面（Wave1-5 并入后）、日志一致性、AGENTS.md 约定抽查。域内卫生（重复/死代码/长函数/可见性归并）归初审 A，未越界。
- 方法：纯读 + rg/git 证据（未跑 cargo）。先对账 2026-11-09 README Top 表（其 P1×1+P2×15 均按 KNOWN 对待），逐项核实现状；再对 11-09 以来 ~25 个合并 PR（#154-#168：i18n 五波、可见性三波、mixer I/O 重做、devices 页重做、fader feel）做增面扫描。i18n 部分实际比对了 `locales/zh-CN.yml`（1,537 行）、`src/i18n_guards.rs`（979 行）、`tests/i18n_ledger.rs`（725 行）与代码内 t! 站点。

## 发现

### B-1 resume 通知把用户节点名 format 进 i18n key，zh-CN 用户必然看到英文句子（cause+hint 双漏） [P1] [NEW]
- 位置：`crates/moiren-app/src/service/managed/driver.rs:343-357`（producer）；渲染面 4 处：`ui/shell/fault_popover.rs:72,117,134`、`ui/pages/graph/graph_canvas_view/nodes.rs:889`。
- 证据：恢复会话剥离离线设备节点时，`cause: format!("Resumed without {} {nodes} because {devices} offline: {names}", …)` 把 **用户可改的节点名** 拼进句子（且把单复数分支 `"node"/"nodes"`、`"its device is"/"their devices are"` 硬编码在 key 里）；hint 亦是自由英文句 `"Reconnect the device, then undo (Ctrl+Z) to restore the removed node."`。`fault_popover.rs:72` 以 `t!(fault.cause.as_str())` 查表——zh-CN.yml 中无 "Resumed without" 前缀任何条目（grep 0 hits），hint 同样不在 catalog。fallback 即英文原文（`i18n.rs` 有测试钉死此 fallback 行为）。`fault_popover.rs:60-62` 注释明确假设 "Fault causes … are English catalog keys"，但该约束无任何 guard 强制：动态 key 不进 DYNAMIC_KEY_EXCEPTIONS 也不进 catalog 时，双向账本两个方向都静默放行——这是 Wave1-5 guard 体系唯一的结构性盲区，而本例证明它已被踩中。
- 影响：设备离线恢复是常见运行路径；中文用户看到完整英文报错 + 英文 hint。且 key 含用户数据，结构上永远不可翻译——不是"补一条 catalog"能修的。
- 建议方向：payload 改结构化数据（剥离数 + 节点名列表），paint 层用带 `%{count}`/`%{names}` placeholder 的固定 key 翻译（沿 fault cause catalog 既有模式）；顺带在 i18n_guards 给"非字面 t! 站点必须登记其 key 来源"立一条可检查的清单（本例即反例）。
- 工作量: S-M

### B-2 shell↔pages 双向 glob 桥 + 38 个生产文件 `use super::*`：跨页耦合零成本化（AU-1 的结构性使能器） [P2] [NEW]
- 位置：`ui/shell.rs:12`（`pub(crate) use super::pages::{devices, graph, matrix, mixer, settings}`）+ `ui/pages/mod.rs:12`（`pub(crate) use crate::ui::shell::*`）；38 个生产 UI 文件以 `use super::*;` 开头（5 个页面顶层文件全部在内）。
- 证据：pages 从 shell 拆出后留了双向脐带——shell 把五个页面命名空间整体 re-export，pages/mod.rs 又把 shell::*（含 gpui prelude、service 类型、t!、五个页面模块）整体 glob 回来；`pages/mod.rs:10-12` 注释自证这是搬家便利（"so page modules keep their `use super::*` glob after moving out of the shell module"）而非分层设计。后果：任何页面顶层文件无须一行 import 即可引用任何其它页面的全部符号，符号来源不可追踪；AU-1 类跨页内核消费（graph 页吃 mixer 内核）在此环境下是"最省事路径"。这解释了 AU-1 为何持续再生新消费者（见 B-4）。
- 影响：跨页依赖无摩擦增长；IDE "find usages" 与人工审阅都难判断某符号实际定义处。
- 建议方向：单方向化——保留 shell 消费 pages（shell.rs:12 有限列举尚可），去掉 pages/mod.rs:12 的 `use crate::ui::shell::*`，页面顶层改显式 import 清单（或至少 pages 不再隐式获得 gpui prelude/service 全集）；与 B-4（滑杆内核搬家）同车机械执行。
- 工作量: L（机械但面宽，可按页面分批）

### B-3 错误 prose 的两套家族并存：project_fault 兜底臂直通 engine Display 文本；ExportInvalid 自由 prose（均 zh 不可译） [P2] [NEW]
- 位置：`service/managed/driver.rs:818-819`（`E::Graph(_) | E::InvalidProcessing | E::Capacity => (…, error.to_string(), None)`）；`service/exports.rs:74-77` 起的 validate_exports 小写 prose（`"at most 64 virtual exports are supported"`）→ `ui/state.rs:113-116`（`UiFailure::ExportInvalid { reason: String }`，注释自述 "renders untranslated by design"）。
- 证据：engine `ProjectError` 的 thiserror Display（`moiren-engine/src/host/project.rs:136-157`）产出的 `"invalid processing configuration"`、`"project exceeds the host capacity"` 及 GraphError 文本，既不在 zh-CN.yml 也不在 DYNAMIC_KEY_EXCEPTIONS（两处 grep 0 hits），经 `t!(fault.cause.as_str())` 时英文直通。与 B-1 同根：cause 通道允许"非 catalog prose"静默流过。另外 validate_exports 的句式是小写句、GraphFault 家族是大写句——两套 prose 风格并存。
- 影响：兜底臂触发罕见（P2 而非 P1），但 ExportInvalid 是用户编辑导出失败的常规反馈路径，英文直出（虽然"by design"已文档化）。
- 建议方向：并入 APP-1 的 GraphFault 类型化战役一并收口（枚举化 InvalidProcessing/Capacity/Graph 三臂 + ExportInvalid 沿 MeterFault 模式类型化），统一 prose 家族。
- 工作量: M

### B-4 AU-1 跨页滑杆内核：原样保留，双向依赖未拆，且新增第三消费者 [P2] [KNOWN-OPEN]
- 位置：`ui/pages/mixer/model.rs`（SliderSlot/SliderDispatch/sync_slider_range 等内核仍在 mixer 页）；消费方 `ui/pages/graph/compressor.rs:3`、`ui/pages/graph/node_inspector.rs:6,219,226`、`ui/shell/tray_mixer.rs:23,28`（11-09 后新增）；反向 `ui/pages/mixer.rs:1`（`use super::graph::{OBSERVATION_SAMPLE_CADENCE, observation_snapshot}`）与 `ui/pages/mixer.rs:141`（re-export graph 的 `kind_tint`）。
- 证据：11-09 AU-1 指出的"内核藏于 mixer 页 + shell glob 转运 + graph↔mixer 双向"三项全部原样；11-09 后 mixer I/O 重做（#163）与 fader feel（#165）反而加深了 mixer/model.rs 的内核地位（ SliderSlot 扩展 inspector_faders 等）。graph 页两个新文件经 `crate::ui::shell::mixer::{…}` 显式走 shell 转运。
- 影响：同 11-09 判定；新增消费者使未来搬家成本继续上升。
- 建议方向：维持原议——滑杆编辑内核迁 `ui/editors/`（或 widgets 旁的共享编辑层），graph/mixer/tray 三方改为平等消费；与 B-2 同车。
- 工作量: M

### B-5 WA-05 影子测试：形态改善（#[path] 引真模块），但 crate 内不可发现性原样 [P2] [KNOWN-OPEN]
- 位置：`crates/moiren-windows-audio/src/asio_rt_tests.rs`（47 行）、`src/engine_backend/recovery_host_tests.rs`（1,287 行）、`tools/rt-check/tests/professional_rt.rs`（1,085 行）；CI 触发 `.github/workflows/runtime-foundation.yml:109`（仅 rtcheck surface）。
- 证据：改善面——asio_rt_tests.rs 已从整份拷贝瘦身为 `#[path]` 引用真 `asio/{abi,callback,sample}.rs` 的 47 行 harness（拷贝漂移风险消除）；professional_rt.rs 的模块 shim（engine_backend 镜像、owner 镜像、negotiation 常量镜像）逐条有注释说明缘由。未改善面——约 2,400 行真实 RT 测试仍只被 rt-check 编译；`asio_rt_tests.rs`/`recovery_host_tests.rs` 文件头注释均未声明"仅 tools/rt-check 编译"（crate 内 grep rt-check 仅 `tests/common/alloc_guard.rs:4` 一处顺带提及），lib.rs/engine_backend.rs 无指针。11-09 建议（归属注释）未执行。
- 影响：维护者改 `asio/callback.rs` 或 `engine_backend/recovery.rs` 的私有路径时无从知晓有一整套测试依赖之；rt-check 不跑的 PR（大多数）下这些测试处于"不存在"状态。
- 建议方向：维持 11-09 建议——给两个影子文件头加归属注释（"compiled only by tools/rt-check: …"，含触发条件），并在 engine_backend.rs 模块声明处加一行指针；S 工作量即可闭合可发现性缺口。
- 工作量: S

### B-6 W-1/W-2 原样：moiren-plugins 仍无任何 CI 映射；AGENTS.md 结构段仍漂移 [P1] [KNOWN-OPEN]
- 位置：`.github/scripts/detect-changes.sh`（ORDER 与 direct 表均 5 crate，`grep -c moiren-plugins` = 0）；`.github/workflows/runtime-foundation.yml`（无 plugins 步骤；`:50` miri_pkgs 硬编码 4 crate 亦无 plugins）；`AGENTS.md:7-13`（结构段 5 crate，漏 `crates/moiren-plugins`、`driver/`、`tools/`、`scripts/`）。
- 证据：11-09 唯一 P1（W-1）+ W-2 至今未修；touch `crates/moiren-plugins/**` 的 PR 其 8 个测试与 clippy 在任何触发路径下不执行。
- 影响：同 11-09 判定；本域内顺带亲证，修复方案以 11-09 01 报告为准（~25 行）。
- 工作量: S

### B-7 WA-01 owner×3 语义孤岛原样（一处细化），clock×3 为同族低危实例 [P2] [KNOWN-OPEN]
- 位置：`crates/moiren-windows-audio/src/owner.rs` / `src/engine_backend/owner.rs` / `src/virtual_owner/owner.rs`。
- 证据：三模块仍在且语义互不相干。细化一处：后两者实为同一 `BackendOwner` 契约的两个实现（professional WASAPI transport vs virtual clock transport），语义上是"实现分叉"而非"无关撞名"；真正的语义孤岛是 `src/owner.rs`（Win32 RAII 工具箱：OwnedHandle/PacketLease/Mmcss/Process，被 probe、exclusive_probe 等多方消费）。同族新增观察：crate 内 `clock.rs` ×3（`src/clock.rs` 遥测 ClockPoint、`latency/clock.rs` QpcClock 测量、`virtual_owner/clock.rs` ClockTick 调度）——但后两者被域目录 scoping，撞名风险显著低于 owner 组。
- 影响：同 11-09；`use crate::owner::…`（如 exclusive_probe/capture.rs、probe/capture.rs）与 `super::owner` 在同一 crate 内指涉完全不同的物，是误读高发点。
- 建议方向：维持原议（`src/owner.rs` 改名如 `win32_raii`）；clock×3 顺手可在各自模块头加一行语义注释即可，不必改名。
- 工作量: S

### B-8 settle_panels 固定 900ms 睡眠 × 64 个调用点：约 1 分钟纯睡眠进 UI 测试套件，且与动画时长隐式耦合 [P3] [NEW]
- 位置：`crates/moiren-app/src/ui/shell/test_harness.rs:41-51`（定义）；调用点 64 处（mixer 20、graph 24、devices 8、shell 6、其余若干）。
- 证据：`settle_panels` 用固定 `sleep(900ms)` 等弹簧动画落地，注释说明了动机但机制是纯计时。两个问题：(1) 64 × 0.9s ≈ 58s 纯睡眠，是 gpui-test 套件的最大单项固定开销；(2) 900ms 与弹簧动画的实际时长是两个无关联的常数——动画调长超过 900ms 时 64 处测量会悄悄进入动画中段（假绿而非报错）。同文件已有 `wait_until`/`wait_for` 带超时轮询基建（test_harness.rs:53-82）。
- 影响：CI 时间 + 一处动画常数改动可致大面积静默误测。时序断言本身不脆弱（等待恒大于需求），故 P3。
- 建议方向：以弹簧完成信号或确定性动画时钟替代固定睡眠；最低限度把 900ms 与动画配置对齐并注释联动关系。
- 工作量: M

### B-9 meter_latency_probe.rs.disabled：自declared"用完即删"的临时探针仍停在 tests/ [P3] [NEW]
- 位置：`crates/moiren-app/tests/meter_latency_probe.rs.disabled`。
- 证据：文件头自述 "TEMPORARY … run once against the default output device, report, then delete. Not part of the regression suite"；改名 .disabled 后事实上不参与编译，成为纯入库死件（git 历史可随时找回）。
- 影响：低；但它是 tests/ 目录里唯一"永不编译"的文件，破坏 tests/ 目录的可预期性。
- 建议方向：删除。
- 工作量: S

### B-10 运行期失败走 eprintln! 绕过诊断日志文件（GUI 进程 stderr 通常丢弃） [P3] [NEW]
- 位置：运行期 11 处：`ui/shell/autosave.rs:173,224`、`ui/pages/settings/persistence.rs:50`、`project/storage.rs:119`（load 失败路径）、`ui/shell/desktop.rs:218,376,484,489`、`ui/shell.rs:99`、`ui/shell/chrome.rs:173`、`ui/tray.rs:148`、`ui/shell/tray_mixer.rs:328`；boot 前 3 处（`main.rs:6`、`console.rs:39` 等）合理。
- 证据：app 有完整 flexi_logger 基建（文件 + 轮转 + stdout 镜像，`logging.rs`），且 main 在一切之前初始化。上述运行期失败（自动保存失败、偏好保存失败、窗口状态恢复失败）本应进诊断文件供用户上报，却走 `eprintln!("Moiren X: {error}")`——Windows GUI 子系统进程 stderr 无处可去，也不经 logger 格式化/轮转。14 处 eprintln 形状完全一致（"Moiren <子系统>: {error}" 前缀），说明是既有惯例而非散逸——问题在惯例本身与日志基建冲突。service/windows-audio 生产路径已统一 `log::…!(target: "moiren::…")`。
- 影响：可诊断性缺口：自动保存失败在用户侧完全不可见。
- 建议方向：运行期失败改 `log::warn!(target: "moiren::ui"/"moiren::project", …)`；保留 logger 初始化失败前的 eprintln（别无选择）。约定一句话写进 AGENTS.md 或 logging.rs 模块头。
- 工作量: S

## 正面登记

1. **i18n 账本 guard 体系是全仓最严的机械防线**（#154-#158 后闭环）：双向键账本（`i18n_guards.rs:562+`，字面 t! ↔ zh-CN.yml 双向，DYNAMIC_KEY_EXCEPTIONS 113 条带"fulfilled 即删"的自清规则）、placeholder 集合一致（两处独立实现）、**Debug 直出 tripwire 的 allowlist 已清零**（`1f66a6ec`，此后任何 `{:?}` 进 UI 文本直接 fail 而非进 allowlist）、en 值必须逐字镜像 key（`tests/i18n_ledger.rs:267-269`）、catalog 单文件 `{en:, zh-CN:}` 双字段结构使"zh/en 键集合漂移"结构性不可能。术语恒等规则落实：ST/MONO/SWAP 等以身份条目（zh-CN == en）钉在 catalog（zh-CN.yml:1250-1255），#164 专门修正 strip/channel-mode 术语回退英文。
2. **crate 依赖方向实测干净**：core/signal/plugins 叶子 → engine(+plugins) → windows-audio → app 单向 DAG；service/project 生产代码零 `crate::ui` 引用（唯一命中在测试文件 `service/settings_tests.rs:62`，可接受的测试便利）。
3. **平台门控纪律**：硬件验收 `#![cfg(all(windows, target_pointer_width = "64"))]` crate 级钉死（`tests/asio_acceptance.rs:2`）；Linux CI 显式只跑非 UI 面，UI headless 测试明确 Windows-only 且有注释说明（runtime-foundation.yml:120-125）；Miri job 存在且覆盖 4 个核心 crate（runtime-foundation.yml:196-238）——AGENTS.md 的 CI 声明除 plugins 缺口外属实。
4. **服务层测试时序模式正确**：`service/managed/tests.rs` 全部使用 deadline + 轮询（`assert!(Instant::now() < deadline)` 循环），无裸 sleep 等待条件；`ui/shell/test_harness.rs` 的 wait_until/wait_for 带 `#[track_caller]` 定位与超时报文。测试名全部描述性 snake_case（如 `real_asio_callbacks_and_diagnostic_snapshots_do_not_allocate_or_free`）。
5. **RT 前缀纪律无违例**：全仓 `Rt*` 类型仅 3 个（RtHost/RtResources/RtPlanPort），逐一核实均在 RT 路径。
6. **日志 target 规范**：service 与 windows-audio 生产日志统一 `target: "moiren::…"` 命名（如 `moiren::audio::service`），flexi_logger 单点初始化。

## 明确不建议

1. **不要**为"可发现性"把 rt-check 影子测试搬进 crate 的 tests/——它们必须在库外 shadow-compile 才能测私有 RT 路径；修法是 B-5 的归属注释，不是搬家。
2. **不要**把 zh-CN.yml 拆成 en.yml + zh-CN.yml 双文件——现单文件双字段 + "en 镜像 key"断言（i18n_ledger.rs:267）是漂移的结构性保险，双文件反而回到需要账本追赶的形态。
3. **不要**一次性清掉 38 个 `use super::*`——纯机械全量替换会把 shell 簇自身的局部内聚也打散；只随 B-2 的单方向化按页面分批处理。
4. **不要**为 B-1 单开"动态 key 静态校验"新分支——沿 APP-1 类型化收口（数据与句子分离）即同时消灭该盲区的成因，独立校验器是重复投资。
5. **不要**给 clock×3 / capture×3 做改名工程——域目录 scoping 已足够，模块头一行语义注释是上限。

## Top5

| # | 项 | 级别 | 一句话 |
|---|---|---|---|
| 1 | B-1 | P1 | resume 通知把用户节点名 format 进 t! key，zh 用户必见英文（cause+hint 双漏，结构上不可翻译）——i18n guard 体系被实证踩中的盲区 |
| 2 | B-6 | P1 | W-1/W-2 原样未修：moiren-plugins 的测试与 clippy 在任何 CI 路径下不执行 |
| 3 | B-2 | P2 | shell↔pages 双向 glob 桥（38 文件 `use super::*`）使跨页耦合零成本，AU-1 的结构性使能器 |
| 4 | B-4 | P2 | AU-1 滑杆内核原样 + 双向依赖未拆 + 新增第三消费者（tray_mixer） |
| 5 | B-3 | P2 | 错误 prose 两家族并存（project_fault 兜底臂 + ExportInvalid），并入 GraphFault 类型化一并收口 |
