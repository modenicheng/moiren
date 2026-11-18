# app 服务与 UI 规则

适用 crate：[moiren-app](crates/moiren-app.md)。UI 的设计/视觉规则以 [FRONTEND.md](../../FRONTEND.md) 为准；本文管工程与数据语义。

## 分层与数据流

- **服务层**（`src/service/`）拥有候选 `LogicalGraph`、`EngineHost` 驱动、jobs、快照与持久化。UI 只发 `AppRequest`、消费快照/model/信号，不直接改图、不编译、不操作引擎线程。
- **项目层**（`src/project/`）是持久化文档模型（版本、加载/保存、autosave、window/preferences）；加载项目不启动音频。
- **UI 层**（`src/ui/`）只做投影与交互；`service/` 生产代码不 import `crate::ui`（测试用 `crate::ui::finish_service` 关服务）。
- 结构编辑（增删节点/连线/绑定/路由）防抖后重建并原子切换；失败保留旧图继续运行。参数编辑优先走实时参数通道（下一音频块生效，零重编译）。

## 真相与呈现

- **候选图是唯一路由真相**；运行事实以引擎观测为准，配置事实以已保存请求为准，两个并排展示，不互相冒充。
- 参数反馈是 **Requested → Accepted → Applied**；Applied 必须有真实回执，不能本地拖动后就显示生效。
- 没有数据/没有 publisher/等待窗口/不可用/停止/错误分别展示；Unknown 用 `—` 或状态说明，**永远不画假读数**。
- 候选配置不能标成活动配置；Accepted 不能当 Applied；原型控件不能装作已接入运行时。
- 观测订阅跟随真实可见需求：切页、隐藏、最小化、滚出视口时释放；恢复后按当前会话/订阅代际重新取数，防止旧缓存串入。
- 页面局部错误在局部说明，不清空其他有效区域；刷新失败保留最后成功结果并标状态。

## UI 性能约定

- 高频重绘走热层/缓存/按需订阅；不要让每帧重建整页、全量重 shaping 或深拷贝快照。
- 缓存视图的内容根用 `w_full`/`h_full` 承担尺寸，不要用 `flex_1`（缓存内容按 layout root 布局，grow 无效）。
- 可见性变化要释放/重建订阅；未运行、未绑定或候选未确认时显示 Unknown，不复制别的节点读数。

## i18n

- 所有用户可见文本走 `t!`；键集以 `crates/moiren-app/locales/zh-CN.yml` 为唯一事实源，`tests/i18n_ledger.rs` + `src/i18n_guards.rs` 守卫漂移。
- 完整句子汉化；音频术语简称（M/S、ST、MONO、SWAP、L/R、n CH、DIM 等）所有语言保持英文。
- 不要用 `format!("{:?}")` 的结果直接进 UI 文本；动态 key 只允许在 `DYNAMIC_KEY_EXCEPTIONS` 中有据登记，优先改成结构化数据 + 画时 `t!`。

## gpui-kit 约定

- UI 用 gpui-kit（Rust，无 markup），组件遵循 `RenderOnce` + `#[derive(IntoElement)]`，颜色/间距/圆角/字体走 `src/ui/theme/` token，不散落魔法数字。
- 复用现有 `widgets/` 组件与 Tabler 图标（`assets/icons/`）；新增图标要进 `ActionAssets` 白名单并有回归测试。
- 窗口、托盘、标题栏拖拽等平台行为遵循 `crates/moiren-app/README.md` 的现状语义；关闭窗口不等于退出，托盘 Exit 才回收音频服务。
- UI 截图用 `ui_capture` 生成到 `docs/designs/screenshots/application/`；UI PR 附真实截图，不用手绘或旧图。

## 测试与环境

- 默认测试不开音频设备；`--features gpui-test` 的 headless UI 验收只在 Windows CI 跑。
- 测试构造 shell 必须清空 `autosave.path` / `preferences.path`（共享 harness 已做），否则可能写入开发者真实 `%APPDATA%\Moiren`。
- 不要断言与渲染次数强耦合的脆弱指标；时序断言用 deadline/轮询，不用裸 sleep。
- 已知的测试坑与修复记录见审计报告与本地记忆，不在本文件重复。

## feature 与示例

| feature / 目标 | 用途 |
|---|---|
| `gpui-test` | headless UI 验收（Windows） |
| `vad-integration-probe` | VAD 服务集成探针（`--example vad_integration`） |
| `ui-profiler` | 原生 UI profiler 抓取路径（含 `gpui-test`） |
| `component-gallery` | 设计评审组件画廊（`--example component_preview`） |

示例：`ui_capture`（截图）、`vad_integration`（VAD 集成）、`component_preview`（组件画廊）。具体命令见 [verification.md](verification.md)。
