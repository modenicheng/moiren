# 2026-10-31 UI 线只读审计（repo-steward，重点：design 17 两层 UI）

- **基线**：main @ `dc9b38a`（PR #53 合并点；design 17 实现线 `21d5b3b..dc9b38a`，34 文件 +4078/−1593）。工作区干净。
- **方法**：5 个并行只读子代理（死码 / 结构 / 热层 bug / 数据面 bug / 卫生·文档·测试·CI），遵循 repo-steward Full 级别规程；主代理对 6 条最重结论逐一亲证（下表 ✓ 列）。基线证据：`cargo clippy --workspace --all-targets -D warnings` 与 `gpui-test` feature 门均 **0 警告**、`cargo fmt --check` 全绿——本轮发现全部在 lint 能力之上。
- **重点**：按用户要求聚焦近期合并的 UI 代码（两层渲染 hot/cached、缓存键、meter 数据平面、per-strip 面板），同时全量清点死代码与结构债。
- **只读承诺**：审计未修改任何代码；本目录为唯一产出。

## 目录

| 文件 | 内容 |
|---|---|
| [01-dead-code.md](01-dead-code.md) | 死代码/死资产/仅测试引用面（design 17 删除侧干净；可回收 ≈209 行 + 21 svg） |
| [02-structure.md](02-structure.md) | ui/ 模块地图、重复比对、重组建议与"不要拆"清单（无 P1） |
| [03-hot-layer-bugs.md](03-hot-layer-bugs.md) | 热层/缓存/手势状态机 bug 猎捕（3×P1 + 11×P2 + 6 疑案 + 已核对无问题清单） |
| [04-state-data-bugs.md](04-state-data-bugs.md) | 订阅/门控/数值/节流/i18n bug 猎捕（2×P1 + 9×P2 + 7 待实测；既往坑均未复发） |
| [05-hygiene-docs-tests.md](05-hygiene-docs-tests.md) | 注释/TODO/文档漂移/测试覆盖/CI 映射/仓库卫生（4×P2 + P3；TODO 全仓为零） |
| [06-consolidation-plan.md](06-consolidation-plan.md) | **分期工单总方案**（批次/依赖/验证门）、实机疑案合并表、决策项 |

## 执行摘要

1. **design 17 本体质量高**：删除侧零孤儿（DOM meter 三件套/CachedChrome/VisibleSignal 等宣称删除全部落地）；注释全为约束/意图型；全 crate TODO/FIXME 为零；既往坑（FaderScale 逆映射、REST_DB、节流尾沿、revision 门、订阅宽限生命周期）逐一复核未复发。clippy/fmt 全绿。
2. **真正的风险集中在"键与族没跟上架构扩张"这一类**——五条 P1 全是它：
   - `PanelHost` 观察者的 `|| host.nested.is_empty()` 让无嵌套面板（Matrix/Settings/ErrorStrip/LibraryRail）**每次 notify 整树重建、键形同虚设**（HC-3，✓ 亲证），直接证伪 design 17 step-4 "计数器平"的验收声明；
   - 检查器缓存面板 `observe_nested` 与 `typing_focus` 谓词都**没跟上 design 17 之后的编辑器家族扩张**（SD-1 ✓、SD-2 ✓）：检查器里 Balance/Requested/Pan/压缩器编辑器键入不上屏（空闲会话全程盲打），Graph 页编辑器被 `keep_graph_focus` 每帧抢焦基本不可用，键入中 Ctrl+Z 会在文本下方 undo 整图；
   - `MatrixKey` 漏 density（HC-2 ✓，且亲证发现比报告更实锤：key 里的 `compact` 装的是窗口紧凑标志 `shell.compact`，渲染读的是主题 `shell.density`——**装错了概念**，不止漏字段）；
   - 画布手势状态无切页/失活复位（HC-1 ✓ 关键两环）：中键平移中切页 → `pan_grab` 卡死 → `frames_wanted` 恒真短路 poll 侧 meter 泵 → 新页无热层 → **主窗口 meter 全冻结** + 显示率帧循环空转。
3. **既往审计对账**：APP-1/2/3/5 FIXED（APP-2 的旧家族确实修了，但 SD-2 是同失败模式在新家族上的复发，非原项回归）；APP-9 断层仍在但面已收窄（SD-11）；17 个死 theme token 现为**零死 token**；**X-1（engine/app 双份节点 level 目录）被 design 17 文档误标 "addressed"，实际原样仍在，应放回开放清单**（HY-4）。
4. **文档四处实质漂移**（HY-3 最重：声称 `UiState::poll` 接管 observation 派生并删除 `refresh_observation_nodes`——代码两者都不成立）；测试两处覆盖缺口：inspector 热表面零测试（正是 hazard-10"断连画空"失败类）、step-4 七面板无重建计数器（#48 类冻结对它们不可测）。
5. **可回收面**：零引用死代码 ≈44 行、仅测试引用 ≈165 行、死图标资产 21 个 svg/24KB（~2.2KB 已嵌入每个二进制）；component_gallery 423 行无门控常驻所有构建。结构债无 P1，两处"名不副实"（meter_frame.rs 实为页面装配、MeterTrace 挂错归属）最值得先修。

## Top 优先级总表

| # | ID | 级别 | 标题 | 区域 | 已亲证 |
|---|---|---|---|---|---|
| 1 | HC-3 | P1 | PanelHost `nested.is_empty()` 使 4 个无嵌套面板键控失效，~30Hz 整树重建 | panel.rs:214 | ✓ |
| 2 | SD-1 | P1 | 检查器 `observe_nested` 漏 balance/gain/pan/压缩器/inspector 推子族：缓存内键入不上屏 | sidebar.rs:75-93 | ✓ |
| 3 | SD-2 | P1 | `typing_focus` 漏新编辑器族：Ctrl+Z 撤销编辑中的图 + Graph 页焦点每帧被抢 | window.rs:904-918 | ✓ |
| 4 | HC-1 | P1 | 画布手势无切页/失活复位：卡死后双泵失效，主窗口 meter 全冻结 | window.rs:631 / desktop.rs:468 | ✓ 两环 |
| 5 | HC-2 | P1 | MatrixKey 装的是窗口 compact 而渲染读主题 density：改密度后矩阵页冻结旧行高 | matrix.rs:29-36 | ✓✓ |
| 6 | HY-11 | P2 | step-4 七面板无重建计数器/缓存断言，step-4 验收标准未兑现（#48 类不可测） | tests | — |
| 7 | HY-10 | P2 | inspector 热表面零测试（hazard-10 断连失败类无回归网） | sidebar.rs:181-235 | — |
| 8 | HY-3 | P2 | design 17 声称 poll 接管 observation 派生——代码不成立，验收叙事失实 | docs/designs/17 | — |
| 9 | HC-13+SD-10 | P2 | 死状态机：`gain_slider`/`pan_slider` 从未渲染、`muted/dimmed` 只写 false 的残留抑制条件 | window.rs:238-283 / state.rs:286 | — |
| 10 | SD-5 | P2 | RMS 积分器自然衰减 ≈29 dB/s 与注释 ~58 相反：Fast(40) 档下 RMS body 不跟偏好 | ballistics.rs:14-17 | — |

其余 P2/P3（HC-4..14、SD-3..11、ST-1..13、DC-3..10、HY-1..14）见各分报告；分期工单与决策项见 [06-consolidation-plan.md](06-consolidation-plan.md)。

## 既往已知项核对（2026-10-27 审计 → 本轮）

| 项 | 本轮状态 |
|---|---|
| APP-1 legacy 驱动栈 | **FIXED**（83e6e34 已删，零残留） |
| APP-2 typing 谓词漂移 | 旧家族 FIXED；**同型复发**于新族（SD-2，非原项回归） |
| APP-3 meter 分段双实现 | **FIXED**（`meter_spans` 单源，5 表面共享 painter） |
| APP-5 / APP-6 | FIXED / 部分 FIXED（node_of_kind 13 处） |
| APP-9 service 层 i18n 断层 | 仍开放、面收窄（SD-11：仅 dispatch_error Debug 串 + RegisterError cause） |
| 17 个死 theme token | **清零**（79 token 全量清点全部存活） |
| X-1 engine/app 双 level 目录 | **仍开放**（design 17 文档误标 addressed，HY-4 要求放回清单） |
| D2 stress 产物 | stderr.log 半项已解决；~481KB stress 明细仍 tracked 待决 |

## 统计信号

- ui/ 规模 30,893 行（含测试）；design 17 线 34 文件 +4078/−1593。
- 发现合计：**7×P1**（5 行为类 + 2 死码类）、~28×P2、~24×P3、13 条实机疑案/待实测（06 合并表）。
- 可回收：≈209 行代码 + 21 个 svg；gallery 门控/删除决策上限另 +434 行。
- lint 基线：clippy（workspace all-targets、gpui-test 门）与 fmt 全绿——所有发现均为 lint 之上的逻辑/结构/文档层。
