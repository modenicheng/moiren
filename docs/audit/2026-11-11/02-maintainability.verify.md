# 02 可维护性 — 独立复核

- 输入: 02-maintainability.a.md + 02-maintainability.b.md
- 基线: main @ e2ed3d6a（2026-11-11）。复核方法：每条亲自打开被引位置读码核对，并用 rg/git 交叉验证调用链（含 i18n catalog 与 DYNAMIC_KEY_EXCEPTIONS 反查、CI 脚本反查、git log 溯源）。未跑 cargo。

## 逐条判定

| ID | 初审判定 | 复核判定 | 依据（path:line + 一句话） |
|----|----------|----------|------------------------------|
| A-1 | .disabled 探针化石引用已删 API [P2][NEW] | 成立 | crates/moiren-app/tests/meter_latency_probe.rs.disabled:24 确引 `moiren_app::tone::{ToneConfig, prepare_tone}`；83e6e343 删除了 src/tone.rs（177 行），全仓 rg 无 ToneConfig 定义；主树唯一 .disabled 文件（find 证实，.worktrees/ 内为同名工作树拷贝非第二例）。582 行数吻合。 |
| A-2 | 后端 String 错误链第二上游 [P2][KNOWN-OPEN] | 成立 | engine `BackendError` 三 String 变体在 crates/moiren-engine/src/host/backend.rs:87-95 亲见；app 同名 struct 在 service/backend.rs:6-20；lifecycle.rs:310 `.map_err(…Failed(error.to_string()))` 与 preparation.rs:8-18 `PreparationFailure{node,cause:String}` 均实；engine_backend.rs 17 处 `unsupported("英文句")`（336-515 等），"native driver"/"process loopback" 在 zh-CN.yml 与 i18n_guards.rs:709 的 113 条 EXCEPTIONS 均零命中；fault 通道 graph.rs:680-716 → fault_popover.rs:72 `t!(fault.cause.as_str())` 全链通。 |
| A-3 | start_process_capture 死 API [P3][NEW] | 成立（死区比初审更大） | process_loopback/stream.rs:29 定义、process_loopback.rs:19 再导出，全仓无第三处引用（README.md:159 仅文档提及）。复核发现：`start_process_capture_with_stop`（:36）的生产调用者只有死掉的 `start_process_capture` 本体（:34）与一个 #[cfg(test)] 内联测试（:190，实为 Cancelled 分支测试），真正有生产消费者的只有 `prepare_process_capture_with_gate`（engine_backend.rs:679）——初审"同组两个都有真实调用者"的表述不准，删时应连同 with_stop 一并评估。 |
| A-4 | run_ui_capture 570 行 [P3][NEW] | 成立 | capture.rs:257 起、文件 827 行（恰 570 行）；双层再导出 ui/mod.rs:38 + shell.rs:821 实在；活跃生长属实：315b0ea3（fader 手势）、5584c901（编辑回显延迟）均扩此函数。入口实为 crates/moiren-app/examples/ui_capture.rs:3（初审写 "examples/…" 省略了 crate 前缀）。 |
| A-5 | edit_candidate 309 行臂间重复 [P3][NEW] | 成立（偏弱） | edit.rs:21 起、文件 330 行吻合；validate_edited_node 在 :6-19（初审写 :9-19 微偏）。复核数点：12 个臂已调用该 helper，剩余散落重复主要是 `.ok_or("The node does not exist")` 类行内序列——收敛空间真实但有限，P3 合理，维持"不拆分发"的裁决。 |
| A-6 | windows-audio 四组 crate 内专用 pub [P3][NEW] | 成立 | 逐组亲证：bind_* 定义 virtual_owner/provider.rs:79,108，消费者仅 virtual_owner.rs:25-26 与 engine_backend/virtual_io.rs:4,72,75；analyze_clock/analyze_device_clock/relative_ppm 消费仅 clock.rs 自身+physical/analysis.rs:5；四个 VAD 请求构造器消费仅 vad/client.rs（vad.rs:51 根面再导出）；catalog_snapshot（lib.rs:64 改名再导出）确被 app/service/windows.rs:11 消费、正当 pub。计数复核 241 pub / 40 pub(crate)，与初审 239/40 吻合。 |
| A-7 | 长函数清单与裁决 [P3] | 成立 | 抽查 9 处行号全中：inspector:46/builtin_controls:613、prepare_with_native:229、positioned:294、graph_canvas_view:513、render:153、matrix page:31、shell new:298、input_row:244、sync_mixer_controls:976、configured_engine_project:46。"内聚不拆/属 gpui 惯例"的逐项裁决与读码观感一致；AU-3 为 11-09 已知（挂起项），KNOWN 标注正确。 |
| B-1 | resume 通知拼用户节点名进 t! key，zh 必见英文 [P1][NEW] | 成立（P1 维持） | 全链亲证：driver.rs:351-357 `cause: format!("Resumed without {} {nodes} because {devices} offline: {names}")`，names 由 io.label 拼 `'{}' (input/output)`（:334-340，用户数据）；hint 亦自由英文句。"Resumed" 在 zh-CN.yml 与 i18n_guards.rs 均零命中；t! 回退即 key 原文（i18n.rs:71-72 测试钉死该行为）；渲染面 fault_popover.rs:72,121,138 与 nodes.rs:889 均 `t!(fault.cause.as_str())`；fault_popover.rs:60-62 注释自认假设无 guard。设备离线恢复为常见路径、结构上不可补译——P1 成立。 |
| B-2 | shell↔pages 双向 glob 桥 + 38 文件 use super::* [P2][NEW] | 成立 | shell.rs:12 `pub(crate) use super::pages::{devices,graph,matrix,mixer,settings}` 与 pages/mod.rs:12 `pub(crate) use crate::ui::shell::*`（:10-12 注释自证搬家便利）均亲见；`use super::*;` 计数精确复核：src 全仓 59 文件，剔 11 测试文件后生产 48，其中 UI 生产恰好 38（pages 26 + shell 12），五页面顶层全在内——初审数字精确。 |
| B-3 | 错误 prose 两家族并存 [P2][NEW] | 成立 | driver.rs:817-819 兜底臂 `E::Graph(_) \| E::InvalidProcessing \| E::Capacity => (…, error.to_string(), None)` 亲见；project.rs:136-157 Display 文本（"invalid processing configuration"/"project exceeds the host capacity"）在 zh-CN.yml 与 EXCEPTIONS 零命中；exports.rs:75+ 小写 prose 与 state.rs:113-116 "renders untranslated by design" 注释均实。 |
| B-4 | AU-1 滑杆内核原样 + 新增第三消费者 [P2][KNOWN-OPEN] | 部分成立（KNOWN-OPEN 与双向依赖属实；"11-09 后新增第三消费者"不实） | 内核仍在 mixer/model.rs（SliderSlot:137 等）、graph/compressor.rs:3-5 与 graph/node_inspector.rs:6 显式走 `crate::ui::shell::mixer::…`、反向 mixer.rs:1 与 mixer.rs:141 全部亲证属实。但 tray_mixer 并非 11-09 后新增：文件创建于 2026-10-23（ab3525a4 "add tray mixer flyout"），且 11-09 README Top 表 AU-1 行已写明"被 graph/托盘三方消费"。"mixer I/O 重做与 fader feel 加深内核地位"部分属实（model.rs 在 11-09 后有 66dd66ef/ff3c54f7/78bc6740/a50ee8a7/2049b7a6 一串 fader-feel 提交）。 |
| B-5 | WA-05 影子测试不可发现性原样 [P2][KNOWN-OPEN] | 成立 | asio_rt_tests.rs 47 行 #[path] 引真 asio/{abi,callback,sample}.rs 属实（拷贝漂移已消除）；`mod recovery_host_tests;` 全仓唯一声明点在 tools/rt-check/tests/professional_rt.rs:76，即仅 rt-check 编译；professional_rt.rs:33-90 的 negotiation/owner/engine_backend shim 逐条有注释（亲读）；两个影子文件头确无 "仅 tools/rt-check 编译" 归属声明（全仓仅 alloc_guard.rs:4 顺带提及），engine_backend.rs/lib.rs 无指针——11-09 建议未执行属实。 |
| B-6 | W-1/W-2 原样：plugins 无 CI 映射 + AGENTS.md 漂移 [P1][KNOWN-OPEN] | 成立（P1 维持） | detect-changes.sh ORDER(:52) 与 direct 表(:69-76) 均 5 crate、全文件无 "plugins"；runtime-foundation.yml 为唯一 workflow，push 全量步 pkgs/lint_pkgs 5 crate、miri_pkgs 4 crate（:48-50），无 plugins 步骤；crates/moiren-plugins 有 tests/plugin_ffi.rs+fixtures；AGENTS.md:7-13 结构段 5 crate、无 plugins/driver/tools/scripts。与 11-09 W-1/W-2 逐点对应，原样未修属实。 |
| B-7 | WA-01 owner×3 + clock×3 [P2][KNOWN-OPEN] | 成立 | 三个 owner.rs 亲见且语义确如初审细化：src/owner.rs=Win32 RAII（windows crate 句柄/MMCSS）、engine_backend/owner.rs=专业 RT worker 停靠/监督、virtual_owner/owner.rs=虚拟时钟传输；clock×3（clock.rs / latency/clock.rs / virtual_owner/clock.rs）同证。"src/owner.rs 才是真孤岛、另两个是同一契约的实现分叉"的细化合理。 |
| B-8 | settle_panels 固定 900ms×64 点 [P3][NEW] | 成立（计数修正：实际调用点 ~54） | test_harness.rs:48-51 `sleep(900ms)`+双 render_frame 亲见；wait_until/wait_for 轮询基建同在（:58-87）。计数复核：全仓 67 处引用 − 1 定义 − 2 再导出（ui/mod.rs、shell.rs）− 8 处 `use` 导入 = 54 个实际调用点（devices 4、graph 23、matrix 1、mixer 19、settings 1、shell 5、sidebar 1）；初审的 64 是把 use 行计入的引用数。≈48s 纯睡眠与动画常数隐式耦合的实质不变，P3 维持。 |
| B-9 | meter_latency_probe.rs.disabled 仍停 tests/ [P3][NEW] | 成立（与 A-1 同物，合并处理） | 同一文件同一事实；两份初审重复登记，修订清单按一条计。 |
| B-10 | 运行期失败走 eprintln! 绕过日志 [P3][NEW] | 成立（计数修正：运行期 12 处而非 11 处） | 逐点亲证：autosave.rs:173,224、persistence.rs:50、storage.rs:119、desktop.rs:218,376,484,489、shell.rs:99、chrome.rs:173、tray.rs:148、tray_mixer.rs:328 共 12 处（初审行文写 11、列了 12）；"Moiren <子系统>: {error}" 形状一致属实；main.rs:6/console.rs:39 为 logger 初始化前的合理豁免。复核补充：performance.rs:136 的 eprintln 是 ui-profiler 门控诊断输出（文件头 "absent from production builds"），与 capture.rs 打印同类，初审未列入是正确的排除。logging.rs flexi_logger 基建（文件+轮转）亲见。 |

**计数**：初审 17 条（A-1..A-7、B-1..B-10，其中 A-1 与 B-9 同物）→ 合并 16 个独立发现。复核判定：成立 15（其中 4 条带计数/细节修正：A-3、A-4、B-8、B-10）、部分成立 1（B-4）、误报 0、已修复 0、已知重复 0（KNOWN-OPEN 标注经与 11-09 README 对账全部准确：A-2↔APP-1 延伸、A-7↔AU-3、B-4↔AU-1、B-5↔WA-05、B-6↔W-1/W-2、B-7↔WA-01）。

## 误报/降级说明

- **无降级、无误报**。两份初审的位置引用真实性与推理链经亲证全部成立，P1×2 的级别均站得住（B-1：常见路径 + 结构性不可译 + 四个渲染点全走 `t!(cause.as_str())`；B-6：plugins 的测试与 clippy 在任何触发路径下不执行）。
- **B-4 细节修正**："11-09 后新增第三消费者（tray_mixer）"不实——tray_mixer 创建于 2026-10-23（ab3525a4），11-09 README 已将托盘计入 AU-1 消费方。KNOWN-OPEN 主体（内核原样、双向依赖、内核地位加深）不受影响，维持 P2。
- **A-3 死区扩大（强化）**：`start_process_capture_with_stop` 的生产调用者仅剩死掉的 `start_process_capture` 与一个内联测试；删除时应一并评估两者，同文件 README.md:159 的文档段同步清理。
- **A-5 弱化说明**：12 个臂已调用 `validate_edited_node`（fn 实在 :6），剩余重复有限——维持 P3 但按"顺手收敛"理解，不值得单开 PR。
- **B-8 计数修正**：64 为引用数（含 use 导入/再导出/定义），实际调用点 54；结论量级（~48s 纯睡眠）不变。
- **B-10 计数修正**：运行期 12 处（初审写 11 但列了 12）；performance.rs:136 为 profiler 专用输出，排除正确。
- **对 A 报"正面登记 #3"的更正**：`wait_for`/`wait_until` 并非"被 tests/common 消费"——tests/common/mod.rs:38-64 是一套**镜像实现**，其头注释（:5-7）说明集成测试以默认 feature 构建、无法导入 pub(crate) 的 `#[cfg]` harness，属有意为之的文档化重复；该注释还指向已不存在的旧路径 `ui::window::test_harness`（现为 ui/shell/test_harness.rs），登记为 V-1。

## 复核补充发现（V-1..）

- **V-1 tests/common 镜像 harness 的"以谁为准"指针已断 [P3]**
  - 位置: crates/moiren-app/tests/common/mod.rs:5-7
  - 头注释写 deadline 语义镜像 "`#[cfg(test)]` harness in `ui::window::test_harness`"，但该 harness 早已随页面拆家迁至 `crates/moiren-app/src/ui/shell/test_harness.rs`（shell.rs:833-837）。wait_until/wait_for/POLL_INTERVAL 双实现本身是文档化的有意重复（不动），但指针指向不存在的模块路径，后续改动 `test_harness.rs` 的轮询/超时语义时，维护者沿注释找不到镜像源——A 报正面登记 #3 的方向性误述即源于此断针。修法：一行注释改路径，S。

（除此之外未登记更多 V：TODO/FIXME 零存量的复核为阴性；`Page::title`、edit_hint 五句、precision/format 等非字面 t! 站点均确认在 DYNAMIC_KEY_EXCEPTIONS + zh-CN.yml 双向登记；除 B-1 外无第二个自由 prose 进 cause/hint 通道的生产者。宁缺毋滥。）

## 修订后优先级清单（P1/P2/P3）

| 级别 | ID | 标题 | 位置 | 状态一句话 |
|------|----|------|------|------------|
| P1 | B-1 | resume 通知把用户节点名 format 进 t! key，zh-CN 必见英文（cause+hint 双漏，结构不可译） | service/managed/driver.rs:351-357 → ui/shell/fault_popover.rs:72,121,138、ui/pages/graph/graph_canvas_view/nodes.rs:889 | 成立；payload 结构化 + 固定 key 带 placeholder，随 GraphFault 类型化收口 |
| P1 | B-6 | moiren-plugins 在任何 CI 路径下无测试/clippy；AGENTS.md 结构段漂移 | .github/scripts/detect-changes.sh:52-76、.github/workflows/runtime-foundation.yml:48-50、AGENTS.md:7-13 | 成立（11-09 W-1/W-2 原样）；~25 行修复，风险≈0 |
| P2 | A-2 | 后端准备错误 String 链（BackendError 三 String 变体 + app 同名 struct + 二次字符串化）直达 fault 卡片 | engine/host/backend.rs:87-95、app service/backend.rs:6-20、managed/lifecycle.rs:310、graph/preparation.rs:8-18、engine_backend.rs 17 处 | 成立；类型化范围应显式扩到 BackendError |
| P2 | B-2 | shell↔pages 双向 glob 桥 + 38 个生产 UI 文件 `use super::*`，跨页耦合零成本化 | ui/shell.rs:12、ui/pages/mod.rs:12 | 成立；单方向化，按页面分批 |
| P2 | B-3 | project_fault 兜底臂直通 engine Display 文本 + ExportInvalid 自由 prose（zh 不可译） | managed/driver.rs:817-819、service/exports.rs:75+、ui/state.rs:113-116 | 成立；并入 GraphFault 类型化一并收口 |
| P2 | B-4 | AU-1 跨页滑杆内核原样 + graph↔mixer 双向依赖未拆（"新增第三消费者"细节不实） | ui/pages/mixer/model.rs、graph/compressor.rs:3、graph/node_inspector.rs:6、mixer.rs:1,141 | 部分成立（KNOWN-OPEN 主体属实）；内核迁 ui/editors/，与 B-2 同车 |
| P2 | B-5 | WA-05 影子测试 crate 内不可发现（归属注释未加） | src/asio_rt_tests.rs、src/engine_backend/recovery_host_tests.rs、tools/rt-check/tests/professional_rt.rs:76 | 成立；两文件头 + engine_backend.rs 声明处一行指针即闭合 |
| P2 | B-7 | owner×3 语义孤岛（src/owner.rs 为真孤岛）+ clock×3 低危同族 | src/owner.rs / engine_backend/owner.rs / virtual_owner/owner.rs | 成立；src/owner.rs 改名 win32_raii，clock 加模块头注释即可 |
| P3 | A-1（=B-9） | 582 行 .disabled 探针化石，引用已删除的 tone 模块 | crates/moiren-app/tests/meter_latency_probe.rs.disabled:24 | 成立；直接删除 |
| P3 | A-3 | start_process_capture 死 API（复核：with_stop 亦无生产调用者，死区更大） | process_loopback/stream.rs:29,36、process_loopback.rs:19、README.md:159 | 成立（修正）；两者一并评估删除 |
| P3 | A-4 | run_ui_capture 570 行混排（CLI 解析/service 装配/捕获/落盘），活跃生长 | ui/shell/capture.rs:257-827 | 成立；提具名局部函数分段，双层再导出收敛 |
| P3 | A-5 | edit_candidate 309 行臂间 revalidation 形状残余重复（12 臂已用 helper，收敛空间有限） | service/graph/edit.rs:21-330 | 成立（弱）；对齐到 validate_edited_node，不拆分发 |
| P3 | A-6 | windows-audio 四组 crate 内专用 pub API 收窄（bind_*、analyze_*、VAD 构造器；catalog_snapshot 除外） | virtual_owner/provider.rs:79,108、clock.rs:49,66,174、vad/protocol.rs:648+ | 成立；随 windows-audio 卫生 PR 同车 |
| P3 | A-7 | 长函数盘点：仅 AU-3（inspector/builtin_controls）为 KNOWN-OPEN 待拆，其余裁决（内聚不拆/gpui 惯例）复核维持 | node_inspector.rs:46,613 等（清单见初审） | 成立；维持原裁决，防过度工程条目有效 |
| P3 | B-8 | settle_panels 固定 900ms × 54 个调用点（初审 64 为引用数），与动画时长隐式耦合 | ui/shell/test_harness.rs:48-51 | 成立（计数修正）；以弹簧完成信号或对齐常数替代 |
| P3 | B-10 | 运行期失败 12 处 eprintln! 绕过 flexi_logger（GUI stderr 丢弃，自动保存失败不可见） | autosave.rs:173,224、persistence.rs:50、storage.rs:119、desktop.rs:218,376,484,489、shell.rs:99、chrome.rs:173、tray.rs:148、tray_mixer.rs:328 | 成立（计数修正）；改 log::warn!(target:"moiren::…")，约定写进 logging.rs 头 |
| P3 | V-1 | tests/common 镜像 harness 注释指向已不存在的 ui::window::test_harness | crates/moiren-app/tests/common/mod.rs:5-7 | 新发现；一行注释修指针 |
