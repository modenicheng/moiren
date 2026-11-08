# 2026-11-03 UI 线只读审计(repo-steward,重点:PR #93 node-ui 与 PR #94 graph-autosave)

- **基线**:分支 `fix/spectrum-standard-curve` = main @ `3321062`(PR #93 node-ui 合并点,55 文件 +8132/−548)+ PR #94 graph-autosave(`9e3a7bc`)+ 2 个频谱提交(`ed93c58` RBW、`e9387d2` 对数网格)。工作区干净,审计零代码改动,本目录为唯一产出。
- **方法**:6 个并行只读子代理(节点卡片 / 画布连线 / 检查器缓存键 / 频谱监控 / 自动保存 / 死码结构测试),遵循 repo-steward Full 级别规程;主代理对 5 条 P1 与重点 P2 逐一亲证(下表 ✓ 列)。未跑 cargo(lint 基线取 CI 绿:clippy `-D warnings` + fmt 全绿,本轮发现全部在 lint 能力之上)。
- **既往对账基线**:2026-10-31 审计发现已由 PR #54-#78/#92 修复;本轮逐项复核未回退,同型失败在新族上的复发单独编号。
- **范围修正**:node_inspector.rs / compressor.rs 实为 PR #74 世代文件(#93 未改),但其上的新发现(IN-1 等)仍然成立。

## 目录

| 文件 | 内容 |
|---|---|
| [01-node-cards.md](01-node-cards.md) | 节点卡片/目录/图标/主题 token(P1×1 + P2×1 + P3×6) |
| [02-graph-canvas.md](02-graph-canvas.md) | 画布/连线拖拽/命中/HUD(P2×3 + P3×4 + 疑案×2) |
| [03-inspector-keys.md](03-inspector-keys.md) | 检查器/编辑器族/缓存键(P1×1 + P2×8 + P3×6,含表面/族全量清点) |
| [04-spectrum-monitoring.md](04-spectrum-monitoring.md) | 频谱数学/弹道/热层(P2×1 + P3×5;数学核心逐项手算亲证成立) |
| [05-autosave-project.md](05-autosave-project.md) | 自动保存/项目恢复(P1×3 + P2×4 + P3×6,含 18 项时序核对清单) |
| [06-structure-deadcode-tests.md](06-structure-deadcode-tests.md) | 死码/模块结构/测试卫生/仓库卫生(P2×4+;死码/死资产为零,可回收 i18n 台账 ≈255 行) |
| [07-remediation-plan.md](07-remediation-plan.md) | **分期修复工单总方案**(批次/依赖/验证门)、决策项 |

## 执行摘要

1. **频谱数学是好消息**:PR #93 分析卡与本分支两提交的数值核心——周期 Hann 幅度归一化、ENBW=1.5、音调 0 dBFS 校准、RBW 下限平顶/肩部滚降、对数十进网格 28 线锚点、弹道 28.95 dB/s——逐项手算亲证**全部成立**;RT 侧 observe 路径零分配;`meter_spans` 单源 5 表面未回退;FaderScale 逆映射坑未复发。外围只有热层每帧分配(SP-1)等工程债。
2. **真正的危险集中在 autosave 语义**(3×P1,全部指向用户数据):打开工程后陈旧 autosave 仍具恢复权威 + 一次 Save 即覆写用户工程文件(AS-2);启动剥离的离线节点图 1 秒后被写回 autosave,跨会话永久丢失(AS-3);4 处测试 shell 直建 `StudioShell` 漏置 `autosave_path=None`,验收套件会把测试图写进开发机真实 autosave(AS-1)。
3. **历史复发类第三次现身,换了形态**:两轮审计的"缓存键/谓词没跟上新族"本轮未在键上复发,但同型漂移落在映射表上——kind→tint 三份手写映射对 ChannelMapper 各执一词(NC-1);压缩器输入族的嵌套池捕获时序洞是 SD-1 同签名在新族上的复发(IN-1);#93 旋钮/EQ 拖拽没进 `cancel_active_drags` 单一游走,恰好违反 window.rs:926-932 注释亲口写明的不变量(IN-2/GC-2),拖拽中 Ctrl+Z 的后果从"覆写"到"冲刷 100 深 undo 栈"。
4. **台账性脱节首次量化**:i18n 双向失守——43 个死键(tsv+yml 双份)同时 4 个 #93 生产键缺 zh-CN 翻译(中文界面直接漏英文),两个方向均无护栏(ST-1/2);绑定清单在两处按同序手写 ~130 行平行代码,注释自承诺 "same choices, same order" 却无测试(ST-4);curves.rs 逐行镜像 engine 私有 DSP 公式 ~35 行(ST-5)。
5. **结构面健康**:死代码/死资产为零(上轮 12 项可回收全部清偿且多带护栏);三处画布代码(pages/graph_canvas* 与 window/graph/*)是三层单代实现**非新旧并存**;测试 harness 复用良好;X-1(engine/app 双 level 目录)仍开放且 spectrum 又添一实例。

## Top 优先级总表

| # | ID | 级别 | 标题 | 位置 | 已亲证 |
|---|---|---|---|---|---|
| 1 | AS-2 | P1 | 打开工程后陈旧 autosave 仍具恢复权威:下次启动恢复旧内容且 `project_path=P`,一次 Save 把旧内容覆写进用户工程文件 | project.rs:552-560 / window.rs:443-447 | ✓✓ |
| 2 | AS-3 | P1 | 启动 resume 的 journaled RemoveNode 剥离让指纹立即变脏,剥离图 1 秒后写回 autosave——离线设备节点跨会话永久丢失 | window.rs:449-451 | ✓(路径) |
| 3 | AS-1 | P1 | 4 处 gpui-test shell 直建 `StudioShell` 漏置 `autosave_path=None`(harness 的 `test_shell_body` 有,直建绕过),测试图写入真实 `%APPDATA%` autosave | devices/tests.rs:88 / exports/tests.rs:321,420,612 | ✓ |
| 4 | IN-1 | P1 | 压缩器 InputState 渲染期惰性创建,打破 panel.rs:172-175 嵌套池"实体先于 notify"前提:首次选中节点的第一段键入盲打(空闲会话无限期) | compressor.rs:61-81 / panel.rs:228-247 | ✓ |
| 5 | NC-1 | P1 | kind→tint 三份手写映射漂移:ChannelMapper 库行 NODE_DSP、画布卡与幽灵卡 NODE_UTILITY,拖拽瞬间变色(#93 修了 icon 同型漂移漏了 tint) | node_catalog.rs:159 / graph_canvas.rs:377 / graph_canvas_view.rs:341 | ✓✓ |
| 6 | GC-1 | P2 | 拎线拖拽中 Ctrl+Z 不取消手势(Delete 路径已清、Undo 路径漏),松手提交陈旧 EdgeId → get_edge 失败 → 假 fault 进错误条 | window.rs:944-978 | ✓ |
| 7 | GC-2 | P2 | 画布旋钮按指针移动派发结构性编辑且无 undo 合并键:每秒 ~30 条 journal 冲刷 100 深 undo 栈 + preparing 警告轮廓整段拖拽闪烁 | knob.rs:127 / graph.rs:385 | — |
| 8 | IN-3 | P2 | Graph 页 ControlsKey 缺 catalog/audio_settings.backend,#93 绑定 chips 与卡片标题却读 catalog:热插拔/Refresh sources 下图页冻结旧设备名(Mixer 页键含,两页不一致) | graph/page.rs:22-41 | — |
| 9 | SP-1 | P2 | 热层每帧为每张分析卡分配新 `Box<[f32]>`(paint 闭包内 `curve.points.into()`),与 c55a3ae "shell-owned scratch copy" 宣称相反 | graph/page.rs:362-377 | ✓ |
| 10 | ST-1/2 | P2 | i18n 双向脱节:43 死键(tsv+yml 双份)+ 4 个 #93 生产键无 zh-CN 翻译,双向零护栏(建议照 action_assets 模式加双向测试) | locales/ | — |

其余 P2/P3 与疑案(GC-3 tooltip 残留、GC-7 幽灵条目、IN-2、IN-7 详情窗零测试、AS-4 退出 200ms、AS-5 损坏静默丢弃、SP-2 通道整拷、ST-4/ST-5 双写镜像等)见各分报告;分期工单与决策项见 [07-remediation-plan.md](07-remediation-plan.md)。

## 既往已知项核对(2026-10-31 审计 → 本轮)

| 项 | 本轮状态 |
|---|---|
| HC-1 画布手势切页复位 | FIXED 未回退(#94 经 set_page 正确取消手势);HC-10 修复面漏新位点 → GC-3 |
| HC-2 MatrixKey / HC-3 PanelHost 键控 | FIXED 未回退 |
| SD-1 observe_nested 漏族 | 原族 FIXED;**同签名复发**于压缩器族(IN-1,非原项回归) |
| SD-2 typing_focus 漏族 / SD-7 拖拽中 undo | 未回退;**同型复发**于 #93 旋钮/EQ 拖拽(IN-2/GC-2) |
| SD-5 RMS 衰减注释矛盾 | FIXED 未复发(28.95 dB/s 亲证) |
| meter_spans 单源 / FaderScale 逆映射 | 均未回退 |
| 死 theme token | PR #93 新增 33 token 全存活,仍零死 token |
| 死图标资产 | 上轮 21 svg 已删+护栏;本轮白名单 28↔28 双向吻合 |
| component_gallery 423 行 | 已按上轮决策处理(见 06 对账表) |
| X-1 engine/app 双 level 目录 | **仍开放**,spectrum 又添一实例(HY-4 的"放回清单"维持) |
| designs/18 文档 | 结构对账通过(ST 区);4 条具体声明漂移(NC 区:badge 残留、网格制式、spline 描述等) |

## 统计信号

- ui/ 规模 40,955 行(含测试);审计对象:PR #93(55 文件 +8132/−548)+ PR #94(18 文件)+ 频谱 2 提交。
- 发现合计:**P1×5、P2×17+(ST 区以报告为准)、P3×27+、疑案/待实测若干**;主代理亲证 7 条(5×P1 + GC-1 + SP-1)。
- 可回收:i18n 台账 ≈255 行 + ~10 行 re-export + 4 条翻译;死代码/死资产为零。
- lint 基线:CI clippy/fmt 全绿,所有发现均为 lint 之上的语义/时序/台账层。
