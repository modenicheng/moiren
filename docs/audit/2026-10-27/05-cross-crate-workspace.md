# 跨 crate 重叠 + 工作区审计报告

> 方法：只读审计 main @ 927f05b（917 个 tracked 文件，`git status` 完全干净）。读了 AGENTS.md 与 repo-steward SKILL 及三份 reference，运行 repo_scan.py，其余全部通过 Read/Grep + 只读 git 取证。五个重点怀疑对象逐一读了源码；既往已知项逐条对照当前 tracked 状态与代码。

## 1. 执行摘要

1. **信号系统的"分发/注册/订阅"没有三份实现**——moiren-signal 是唯一实现（`registry.rs:153-507` 的 subscribe/unregister/pump/register 族，`host/mod.rs` 的 RtHost）。真正的重叠在上一层：engine `SignalHub` 和 app `SignalService` 各自维护一条独立 Bus、各自的"节点→level topic 目录 + 增删 prune + demand-aware 1ms pump 调度"，再由 `MeterBridge` 手工拷贝接力（X-1/X-2/X-3）。meter 数据 RT→UI 共 3 次载荷拷贝、2 条总线各一次 pump、3 层轮询循环。
2. **VAD 栈基本排除"app 复刻 windows-audio"**：`vad_integration` 有意不用 `ControlClient/PcmOwner`（native.rs:85 注释），只取 `discover_present_interfaces` + 自写 WASAPI，这是验证 harness 应有的独立性。ABI 三重编码已收口：ABI.md 第 1 行声明 header authoritative，`abi_conformance.rs` 用 `include_str!` 解析头文件把 Rust 镜像钉死（scalar/IOCTL/尺寸/GUID 四类断言）。
3. **图校验无三层重写**：core 管结构不变量，engine `CompileError` 包装 `GraphError` 加编译期规则，app `validate_node_state`（graph.rs:423-427 注释自declare single source）+ builtin 验证直接构造 mini graph 委托 `EngineProject::validate`（builtin.rs:154-168），没有复制规则。
4. **唯一的平行算法实现同在 moiren-windows-audio 内**：`professional/resample.rs` 的 windowed-sinc `StereoResampler`（带 `set_correction_ppm`）与 `clock_bridge/source.rs` 的连续线性插值 + fill servo——README:45 自己承认这是"新旧两条路径"，收敛被 D3（legacy 退役需实机验证）阻塞。
5. **既往已知项大多已修复**：remediation plan 全部勾选且与代码一致（T4 spawn 点 104→54）、driver/moiren 已删、2.2GB src/ 构建树已迁、CI 脚本与 AGENTS 一致；仍开放的是 D2 运行产物（8 个 0 字节 stderr.log + ~481KB stress 输出，等用户决策）和 **VAD 状态文档碎片化**（STATUS.md 与 830 行台账的头条都滞后于 main 已含的 8h idle PASS 证据，主代理已亲证）。

## 2. 跨 crate 重叠矩阵

| 重叠对 | 涉及位置 | 重叠性质 | 可合并性评估 | 建议 |
|---|---|---|---|---|
| engine `SignalHub` 目录 vs app `SignalTopics` 节点级 level 目录 | `crates/moiren-engine/src/host/signals.rs:251-263`（levels/spectra/topics 三个 BTreeMap + `prune()` 608-659）vs `crates/moiren-app/src/service/signals.rs:97-173`（`node_levels` + `prepare_node_levels` 增删循环） | 两套"NodeId→LatestBlock<AudioWindow,ChannelLevel> topic 注册/注销/prune"生命周期，topic 命名规则各写一份（`owner/{gen}/node/…` vs `node/{id}/level`） | 部分。两条 Bus 存在的动机有注释支撑：engine 侧 topic 随 owner generation 轮换，app 侧 topic 对 UI 稳定（meters.rs:1-2）；但目录/prune 逻辑本身无 RT 语义，可下沉 | 把"节点 level topic 目录 + prune"收敛到一个 crate（建议 signal 或 engine 暴露泛型目录助手），保留双总线仅作稳定性边界；或长期让 UI 直接持 `ManagedLevelSubscription`（engine 已导出）+ generation 失效重订，消灭第二条总线 |
| 双份 demand-aware pump 调度 | engine `SignalHub::pump/poll_interval`（signals.rs:580-600，注释写明 1ms 预算依据）vs app `SignalService::pump_if_due/wait_duration`（signals.rs:70-94）；app 复用 engine 的 `MeterPollingTimer` | 同一条"有业务需求→1ms 轮询，无需求→idle"策略两处实现，参数（topic_checks 524 vs `max_topics*4`）各自推导 | 中。轮询参数语义不同，但策略可由 moiren-signal 提供统一调度类型 | 低优先：在 moiren-signal 加一个 pump 调度类型，两处替换；不紧急 |
| Meter RT→UI 通路（量化） | RT `SignalLevelMeter` 写 engine bus topic → engine supervisor 线程 pump（`host/supervisor.rs:238`）→ app service 线程 `MeterBridge::poll` 读订阅并显式拷贝再发布（`managed/meters.rs:110-124`）→ app bus `pump_if_due`（service/mod.rs:475）→ UI `VisibleSignal` 轮询（ui/signals.rs） | 3 次载荷拷贝（engine pump 投递、MeterBridge 元素拷贝、app pump 投递），2 个泵线程/循环 + UI 帧轮询共 3 层轮询；每节点窗口最多延迟约 2 个 1ms 泵周期 + 1 帧 | 与上一行同解：消除 MeterBridge 即减为 1 拷贝 1 泵 | 若双总线保留，至少在 meters.rs 或设计文档记录该量化路径（目前只有定性注释）；合并动作并入第一行 |
| `StereoResampler` vs `clock_bridge` 线性插值（同 crate 内） | `crates/moiren-windows-audio/src/professional/resample.rs:20-109`（sinc，`set_correction_ppm`）供 professional backend/bridge/owner + `virtual_owner/boundary.rs:251-298`；`clock_bridge/source.rs:1,24,112-152`（线性插值 + ratio/fill servo） | 两条"采样率变换 + ppm 修正"路径平行存在；README:45 明示新旧并存 | 受 D3 阻塞（legacy 退役需实机验证）；算法等级不同（sinc vs 线性），clock_bridge 的伺服/年龄/背压逻辑与传输耦合深 | 不动算法；D3 决策时一并裁定 legacy clock_bridge 路径退役，届时删除线性分支 |
| 计数分配器 guard ×3 | `moiren-signal/src/test_alloc.rs`（含 frees 线程归因）、`moiren-engine/tests/runtime/allocation.rs`、`moiren-windows-audio/tests/common/alloc_guard.rs`（crate 内 6 个 binary 经 `#[path]` 共享） | 三份相似 CountingAllocator | 不可合并：`#[global_allocator]` 按二进制存在，前两份文件均有注释解释该机制限制 | 接受现状（已是最优共享度） |
| serde / rtrb / serde_json 声明分散 | `Cargo.toml`（workspace 仅 4 项）vs engine/windows-audio/signal 各自 `rtrb = "=0.4.0"`、三处 `serde = {version="1.0", features=["derive"]}`、两处 `serde_json` | 同版本重复声明，无冲突 | 高（纯搬移） | 提升到 `[workspace.dependencies]`，顺带统一 windows 系 pin 风格（windows-audio `=0.62.2` vs app `0.62.2`）；lock 中 windows 0.57/0.58/0.61.3/0.62.2 并存来自 transitive（tray-icon/gpui-kit），不必处理 |

## 3. 既往已知项核对表

| 项目 | 状态 | 证据 |
|---|---|---|
| remediation plan 8 项漂移勾选（E13/E14/A11/A13/A14/W7/W8/W9） | **FIXED** | `docs/superpowers/plans/2026-10-19-repo-steward-remediation.md:234-270` 全部 `[x]`；抽查 W7：`tools/rt-check/` 存在且被 CI 引用（runtime-foundation.yml:104）；W8：probe examples 全部 `required-features` 门控 |
| T1/T4 开放、T4 恶化（spawn ~104） | **FIXED** | plan:207/222 均已 `[x]` 且附收敛明细；common harness 存在；`ServiceRuntime::spawn` 现存 54 处调用（原 ~104） |
| D2：stress 运行产物 + 8 个 0 字节 stderr.log | **STILL-OPEN**（待用户决策） | 8 个 0 字节 `docs/superpowers/plans/2026-10-14-{engine-stress,frequent-swap}/*.stderr.log` 仍 tracked；两目录 tracked 内容共 481KB；plan D2 明记 "awaiting user decision" |
| VAD 状态文档繁殖 6 份且矛盾 | **部分 FIXED** | 无 hard 矛盾残留：每节按日期分节。但 `docs/vad-validation-2026-10-21.md:1-6` 头部 "Result" 仍是 10-21 快照（"combined FAIL / Do not start the eight-hour runs"），同文件 :559-591 记录 10-22 "idle 8h passes…real stability evidence"（commit cdd44a2，已入 main）；`driver/vad/STATUS.md`:199/277 仍把 "eight-hour runs" 列为未合格（该文件 10-21 后未再更新） |
| docs/vad-soak-validation.md untracked 但被 tracked README 链接 | **FIXED** | `git ls-files docs` 含 vad-soak-validation.md；`driver/README.md:3-5,20` 链接无断链 |
| driver/moiren untracked 用户项目（10-22 决定删除） | **FIXED** | `ls driver/` 仅 `vad`；`git status --porcelain` 为空 |
| src/professional/rt-check/target 2.2GB 构建树在 src/ 内 | **FIXED** | 仓库根无 `src/`；工具在 `tools/rt-check/` |
| .gitignore 覆盖 target/、.vs/ | **FIXED** | .gitignore:3-4 `target`（无斜杠，匹配任意层级）、:38 `.vs/`，另有 .tmp/.v2c/.zcode/driver 构建输出 |
| detect-changes.sh 与 AGENTS.md 一致性 | **一致** | 脚本实现 crate 依赖闭包（detect-changes.sh:106-118）、push-to-main 全量（yml:46-54）、未知路径保守全量（:83-84），与 AGENTS 描述吻合；且带自测试（detect-changes.test.sh，CI 强制跑）。单 workflow，无重复 job |
| workflow 重复/可合并 matrix | **无问题** | jobs: changes/fmt/test(2OS)/clippy(2OS)/vad_driver_host/miri_suites(12 项，注释说明并行拆分理由)/miri 门；无冗余 |

## 4. 发现清单

### X-1 (P2) 节点级 level topic 目录与 prune 逻辑双份实现
- **证据**：`crates/moiren-engine/src/host/signals.rs:251-263,608-659` vs `crates/moiren-app/src/service/signals.rs:122-172`。两处 topic 命名格式各持一份（engine signals.rs:330-336 vs app signals.rs:150）。
- **详述**：两套目录的存在理由（generation 轮换 vs UI 稳定）成立，但目录/prune 是纯控制侧逻辑，规则（如"节点删除后注销 topic"）改动需改两处。
- **建议**：抽取共享的节点 topic 目录助手，或评估让 UI 直接订阅 engine 导出的 `ManagedLevelSubscription`，在 owner generation 变化时重订，删除第二条总线与 `MeterBridge`。
- **风险与验证**：改订阅路径会影响 UI 电平在图热切换时的连续性（现有测试 `analysis_receives_real_audio_windows_after_ui_hide_without_generation_reset` 覆盖此语义，需保持通过）。

### X-2 (P3) demand-aware 1ms pump 调度策略两处实现
- **证据**：`signals.rs:591-600` vs `crates/moiren-app/src/service/signals.rs:70-94`。app 侧已复用 engine 的 `MeterPollingTimer`，但策略主体仍是两份。
- **建议**：低优先；若动，在 moiren-signal 提供统一调度类型。风险：轮询参数与各自目录容量绑定，机械合并会引入耦合。

### X-3 (P2, 量化陈述) meter RT→UI 通路成本
- **证据**：`managed/meters.rs:93-125`（每次服务循环 `begin_block` + 逐 slot `read_latest` → `try_write` 显式元素拷贝 → `commit`）；`host/supervisor.rs:238-247`（engine 泵）；`service/mod.rs:475`（app 泵）。
- **详述**：RT 写 → engine dispatcher 拷贝 → MeterBridge 拷贝 → app dispatcher 拷贝 → UI `read_latest`；轮询 3 层。设计意图有注释，且 `business_subscribers()==0` 时整段跳过（meters.rs:96），无浪费型轮询；问题只在结构复杂度与两处目录（X-1）。
- **建议**：随 X-1 一并处置；若保留双总线，把该通路的拷贝/延迟预算写进设计文档（docs/designs/14 或 host README）。

### X-4 (P2) windows-audio 内两条采样率适配路径
- **证据**：见矩阵第 4 行。README.md:45 自认并存。
- **建议**：不合并算法；D3 落定时同步删除 legacy clock_bridge 插值分支。提前合并会破坏 D3 的回退能力。

### X-5 (P2) VAD 运行状态无 canonical owner，两个权威文档头条滞后
- **证据**：`docs/vad-validation-2026-10-21.md:1-6` 头部 Result 仍为 10-21 的 "FAIL / 不要开始 8h"；同文件 :559-591 记录 10-22 idle 8h PASS + music 相位环境性失败的最终 verdict（commit cdd44a2 已在 main）；`driver/vad/STATUS.md:199,277` 仍列 "eight-hour idle/audio runs" 未合格。当前 VAD 状态散布 5 处：vad-handoff、vad-lifecycle-validation、vad-soak-validation、vad-validation-2026-10-21、STATUS.md。
- **详述**：不是硬矛盾（各节日期定界），但任何只读头部的人会得出"8h 未通过"的错误结论——正是上一轮审计发现的矛盾模式残留。
- **建议**：a) `vad-validation-2026-10-21.md` 头部 Result 段改为一句指针"最新结论见文末 2026-10-2x 节"；b) STATUS.md 的 gates 列表更新为"8h idle 已过、music/16h 未过"；c) 长期指定 STATUS.md 为唯一运行状态 owner，台账类文档只记证据细节并链接。
- **风险与验证**：纯文档改动；验证 = 对照 cdd44a2 提交信息。

### X-6 (P3) 依赖声明集中度
- **证据**：见矩阵第 6 行。
- **建议**：搬入 workspace 表并统一 pin 纪律；lock 中 4 个 windows 版本来自 transitive，无需处理。

### X-7 (P3) D2 运行产物仍待决策
- **证据**：8 个 0 字节 `*.stderr.log` + 两 stress 目录 481KB tracked。plan D2 明记 awaiting user。属用户决策项，不自行处置。

### X-8 (P3) FRONTEND.md 滞后于其后的 UI 合入
- **证据**：FRONTEND.md:4 更新日期 2026-10-16；此后 main 合入 undo/redo（PR#10）、全局 Inspector 侧栏（PR#11）、canvas 工具栏图标（PR#15），文中均未提及。README 引用与命令全部可解析，无漂移。

### X-9 (P3) docs/designs 序号 12 复用
- **证据**：`12-gpui-ready-action-components.md` 与 `12-unified-topology-backend-contract.md` 并存（前者已改号为 15）。

### X-10 (P3, 记录在案) .worktrees/ 排除仅本地生效
- **证据**：10 个活动 git worktree，靠 `.git/info/exclude:7` 忽略而非 tracked .gitignore。可考虑入 .gitignore（一行 `/\.worktrees/`），避免他人 clone 后 worktree 噪声。

## 5. 明确不建议合并项

1. **`vad_integration` 的自写 WASAPI/身份解析**（examples/vad_integration/native.rs）：harness 刻意不复用生产路径（"No ControlClient/PcmOwner here"，native.rs:85）——验证器复用被验证代码会掩盖系统性缺陷；且 feature-gated，默认不编译。
2. **ABI header + Rust 镜像双表示**：header 是 C 侧权威、ABI.md 是人类文档（第 1 行声明权威归属）、protocol.rs 是 Rust 消费者——已由 `abi_conformance.rs` 的 include_str 解析钉死，结构上无法再收敛。
3. **moiren-signal 本体**：全工作区唯一的 Bus/dispatcher/host 实现，engine/app 均为纯消费者，无重叠可消。
4. **app builtin 节点验证**：已通过构造 mini graph 委托 `EngineProject::validate`（builtin.rs:154-168），是正确的"无重复"设计，不要为省一次编译再抄规则。
5. **计数分配器三份**：`#[global_allocator]` 每测试二进制一份的机制约束，文件内注释已说明；windows-audio 内部 `#[path]` 共享已是上限。
6. **`clock.rs` vs `clock_bridge.rs`**：离线时钟拟合诊断 vs 在线捕获适配，职责正交，非重叠（真正的平行对是 X-4 的 resample vs clock_bridge 插值）。
7. **meter/recorder 的 `ReaderSlot` 双子结构**（engine signals.rs:732-749 共享 `reader_handle`）：同文件已抽取共享实现，属已收敛状态。

## 6. 工作区卫生清单

| 路径 | tracked? | 问题 | 处置建议 |
|---|---|---|---|
| docs/superpowers/plans/2026-10-14-{engine-stress,frequent-swap}/*.stderr.log（8 个，0 字节） | 是 | 空文件无证据价值 | 删除（安全：零字节） |
| 同上目录 *.json/*.jsonl/*.png（~481KB） | 是 | D2 待决：运行产物 vs 计划证据 | 待用户决策（保留 summary.json 删明细是折中方案） |
| tools/audio-hack-lab/*.log（27 个，共 34KB） | 是 | 实验输出，REPORT.md 的证据 | 保留（量小且有 REPORT 承接）；detect-changes.sh:81 已列为 CI 豁免 |
| docs/ 顶层 4 份 vad-*.md | 是 | 状态事实分散（X-5） | 保留文件、更新头条 + 指定 STATUS.md 为状态 owner |
| docs/superpowers/plans/（55 项） | 是 | 完成计划未归档/未删（repo-steward 原则：完成即退场） | 待用户决策批量归档策略；非紧急 |
| driver/（仅 vad） | 是 | 无；driver/moiren 已删 | 保留 |
| tools/、scripts/ | 是 | 各有明确职责（rt-check、soak/stress 脚本 + 脚本自测试） | 保留 |
| .gitignore / .git/info/exclude | — | tracked 规则充分；.worktrees 仅本地排除 | 可选：.worktrees 入 .gitignore（X-10） |
| docs/designs/12-×2 | 是 | 序号冲突 | 下次顺手改号 |
| 工作树 | — | `git status` 0 条；无 2.2GB src 树 | 无需动作 |

**总体判断**：上一轮 remediation 的执行质量高（勾选与代码一一对应，T4 spawn 104→54 实测属实）；本轮真正的结构性主题只有一个——"节点 level 信号的双总线 + 桥接"（X-1/X-2/X-3），以及一个流程性主题——VAD 运行状态的事实唯一 owner（X-5）。两者都不是缺陷，前者是可规划的结构收敛点，后者是一次 30 分钟的文档修正。
