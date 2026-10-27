# 2026-10-27 全仓库只读审计（第三轮 repo-steward）

- **基线**：main @ `927f05b`（工作区干净，0 untracked）。
- **方法**：5 个并行只读子代理（core+signal / engine / windows-audio / app / 跨crate+工作区），遵循 repo-steward 技能 Full 级别规程；主代理对最重结论逐一抽样亲证（下表"已亲证"列）。
- **重点**：按用户要求，本轮不只查形式问题，聚焦**功能与逻辑重叠**（同一件事多处实现、语义重复、可合并的平行结构），并给出合并与优化方案。
- **只读承诺**：审计过程未修改任何代码；本目录为唯一产出。

## 目录

| 文件 | 内容 |
|---|---|
| [01-core-signal.md](01-core-signal.md) | moiren-core 与 moiren-signal 审计（Latest/Stream 收敛现状、死 API） |
| [02-engine.md](02-engine.md) | moiren-engine 审计（meter 双实现、ParamSpec 三重建表、读出端无消费者） |
| [03-windows-audio.md](03-windows-audio.md) | moiren-windows-audio 审计（双后端现状与退役步骤、Tone 分叉） |
| [04-app.md](04-app.md) | moiren-app 审计（legacy 驱动栈、meter 绘制双实现、i18n 断层） |
| [05-cross-crate-workspace.md](05-cross-crate-workspace.md) | 跨 crate 重叠矩阵、工作区卫生、docs/CI 一致性 |
| [06-consolidation-plan.md](06-consolidation-plan.md) | **合并与优化总方案**（分期工单、依赖顺序、验证门、决策项） |

## 执行摘要

1. **前两轮整改质量高**：既往 20 项已知问题中 16 项确认 FIXED（多数带单源化 + 钉扎测试），无一项被误报为修复。最近 5 个 PR（#9–#15：auto-apply、undo/redo、检查器侧栏、选择性 CI、工具栏图标）均为增量式融入既有单源结构，**未引入新的平行实现**。
2. **本轮最大的功能重叠不是新代码，而是三处"历史双轨"**：
   - **双后端**（windows-audio legacy vs professional，~7.5k vs ~8.1k 非测试行，路由仍由 `settings != default` 决定）——退役技术前置已全部就位，剩 4 个能力缺口（WA-1，已亲证）；
   - **双驱动栈**（app 内 legacy `BackendDriver` ~1500+ 行仅测试构造，生产只走 `ManagedDriver`）（APP-1，已亲证）；
   - **双电平表**（engine 内队列版 `LevelMeter` 被信号版全面支配，Meter 节点同一流测两次）（EN-1，已亲证）。
3. **信号系统没有三份实现**（坊间怀疑排除）：moiren-signal 是唯一 Bus/dispatcher 实现，engine 与 app 都是纯消费者；真正的重叠在上一层——engine `SignalHub` 与 app `SignalService` 各自维护"节点→level topic 目录 + prune + 1ms 泵"，meter 数据 RT→UI 共 3 次拷贝、3 层轮询（X-1/X-3）。
4. **漂移类正确性风险 3 处**：焦点谓词双写已漂移且 mixer 发送输入框漏覆盖（APP-2，已亲证）；同名 `moiren:tone:` 源两栈幅度 1.0 vs 0.1（WA-2，已亲证）；project.rs 三重 ParamSpec 表 + 手写参数域（EN-3，数值当前一致，属最易漏改的活重复）。
5. **文档事实源**：VAD 运行状态散布 5 处，两个权威文档头条仍写"8h 未通过"，与 main 已含的 8h idle PASS 证据相悖（X-5，已亲证）；Stream 零生产流量的"awaiting first routed consumer"标注承诺仍未落地。
6. **可直接回收的死面**：core 2 个死 pub API（~320 行）、engine 读出端无消费者 API（recorder WAV 全家 ~470 行 + 队列 meter ~380 行）、app legacy 驱动栈（~1.2–1.5k 行）、windows-audio legacy 栈（~7.5k 行，需实机验证后删）。

## Top 优先级总表

| # | ID | 级别 | 标题 | 区域 | 已亲证 |
|---|---|---|---|---|---|
| 1 | WA-1 | P0 | 双后端能力不对称：切默认路由前 4 缺口（latency 档位/捕获时钟/虚拟观察/唤醒遥测） | windows-audio | ✓ 路由条件 |
| 2 | APP-2 | P1 | "正在输入"焦点谓词双写已漂移，mixer 发送输入框两份都没覆盖（Ctrl+Z 会误触整图 undo） | app | ✓ |
| 3 | EN-1 | P1 | 队列版 LevelMeter 被信号版支配；Meter 节点双重测量；meter_reader 零生产调用 | engine | ✓ 双实现注释 |
| 4 | EN-3 | P1 | project.rs 三重 ParamSpec 构建 + Pan/Strip/Edge 手写参数域（最高漂移风险） | engine | — |
| 5 | APP-1 | P1 | legacy BackendDriver + 会话栈 ~1500 行仅测试构造；生产仅用其 catalog() | app | ✓ |
| 6 | WA-2 | P1 | 同名 Tone 软件源两栈幅度 1.0 vs 0.1、频率上限不同 | windows-audio | ✓ |
| 7 | EN-2 | P1* | Recorder/Spectrum/Compressor 读出端无生产消费者（*决策项：接线或裁剪） | engine | — |
| 8 | APP-9 | P1 | service 层 65 处中文硬编码绕过 i18n，英文界面下 fault 提示全中文 | app | — |
| 9 | X-1 | P2* | 节点 level topic 目录 + prune 双份（engine SignalHub vs app SignalService） | 跨 crate | — |
| 10 | APP-3 | P1 | 电平表分段策略 canvas/DOM 双实现（−6/0 dBFS 分色数学两份） | app | — |

其余 P2/P3 项见各分报告与 [06-consolidation-plan.md](06-consolidation-plan.md) 完整清单。

## 既往已知项核对汇总

| 区域 | 核对项数 | FIXED | 开放/部分 |
|---|---|---|---|
| core+signal | 4 | 1（Latest/Stream 收敛，当日即修） | Stream 文档标注未落地；core 协议无平行定义（排除） |
| engine | 6 | 3（smoothing 单源、delay 预检、swap 收敛） | process 双臂残余（EN-4）；参数结构 match 平行（EN-3/5）；signals.rs 分层判定为正确（排除） |
| windows-audio | 8 | 6（M1/MMCSS、M2/virtual-IO、H1/ABI 测试、重试、alloc guard、T4 spawn 104→0） | 双后端本体（WA-1）；D5（降级 wontfix 候选） |
| app | 7 | 4（映射×5→1 对、校验×3→1、spawn 104→54、A13） | 监测双绘制（APP-3）；i18n 断层（APP-9）；检查器/重建分发/工具栏三项怀疑均排除 |
| 工作区 | 9 | 7（plan 勾选、T4、moiren 删除、2.2GB 树、.gitignore、CI 一致、soak 文档入库） | D2 stress 产物待决；VAD 状态头条滞后（X-5） |

## 统计信号

- 规模：5 crate / ~156k 行 Rust（app 53.4k、windows-audio 56.5k、engine 34.0k、signal 9.7k、core 2.0k）。
- windows-audio：legacy ~7.6k vs professional ~8.1k（含 ASIO 3.3k）非测试行；退役后预计 crate 缩 ~25%。
- 测试 spawn 位点：windows-audio tests/ 104→0；全库 ServiceRuntime::spawn 104→54（app 34）。
- i18n：UI 层 `t!` 1392 处全覆盖 vs service 层中文硬编码 65 处——断层正好落在 service/UI 边界。
- 可回收死面合计（含需实机验证的 legacy 栈）：约 **10k+ 行实现 + 相应测试**。

## 待用户决策项（详见 06 方案 §5）

1. EN-2：Recorder/Spectrum/Compressor 读出端——产品要接线（录音面板/频谱画布）还是本轮裁剪？
2. W-K：默认路径切 professional 的时间表（4 个能力缺口 + 实机 soak）。
3. WA-2：Tone 统一幅度取 0.1 还是 1.0（会改变其中一条路径的可听行为）。
4. D2：8 个 0 字节 stderr.log（可安全删）与 ~481KB stress 明细（保留 summary 删明细是折中）。
5. X-1：信号双总线收敛路线（A：目录助手下沉；B：UI 直订 engine 侧）。
6. docs/superpowers/plans/ 55 份已完成计划的归档策略。
