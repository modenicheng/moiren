# 02 可维护性 — 初审 A（域内代码卫生）

- 基线: main @ e2ed3d6a, 2026-11-11
- 范围：域内代码卫生六项——重复代码（去重战役未覆盖的 NEW 重复）、死代码、函数级超长函数、可见性卫生、错误处理一致性、feature/cfg 门控。**不含**：跨边界一致性/测试/i18n（兄弟代理）、文件级 God 判定（11-09 已覆盖）、其他 11-11 报告。
- 方法（实际检索式与抽查文件）：
  - `wc -l` 排全仓 .rs；对 src/ 顶层非测试文件做 fn 间距扫描（`awk` 记录 fn 行号差 >200），人工复核 Top20（读码验证，剔除 vtable statics 插入造成的误报如 vst3 `new`/`instantiate`）。
  - `rg 'allow\(dead_code\)'`（22 处全查，逐处看注释与消费点）；`rg '\.unwrap\(\)|\.expect\(|panic!'` 按 `#[cfg(test)]` 边界切分后逐处判定；`pub fn` 全量枚举 + 交叉引用核查（windows-audio 32 个顶层 pub fn 与 app 75 个顶层 pub fn 逐名 rg）。
  - 6/12 行归一化滑动窗口全仓复制粘贴检测（awk 生成 59.7 万/109 万窗口行 + uniq -c），Top 重复块人工定位判定。
  - 依赖核查：`rg` 逐名验证 crossbeam-utils/resvg/windows-registry/libloading/raw-window-handle/tray-icon/embed-resource/windows-core。
  - cfg 门控抽查：`gpui-test` 24 处门控点对照 CI 两级测试步（runtime-foundation.yml:99,122）；gpui-kit 0.7.3 源码（~/.cargo/registry）核验 `pub mod test` 的 test-support 门。
  - 结构粗读 14 个文件：capture.rs、engine_backend.rs、edit.rs、preparation.rs、swap.rs（测试边界）、host/mod.rs、mixer/model.rs、tray_mixer.rs、lifecycle.rs、backend.rs、i18n_guards.rs、matrix.rs、graph_canvas.rs、vad/provider.rs。
  - 与 11-09 六分报告发现清单逐条对账；git log 核实去重战役 Wave1-5 合并状态（#154-#158 已并入，早于本基线）。

## 发现

### A-1 582 行"用完即删"的 .disabled 探针化石，引用已删除 API [P2] [NEW]
- 位置: crates/moiren-app/tests/meter_latency_probe.rs.disabled（582 行）
- 证据 / 影响：文件头自述 "TEMPORARY … run once against the default output device, report, then delete. Not part of the regression suite"（1-3 行），随 284b911e（wip 快照提交）入库存活至今；`.disabled` 后缀使其对 cargo 不可见，故无人发现它已**双重死亡**——第 24 行 `use moiren_app::tone::{ToneConfig, prepare_tone}` 引用的 `moiren_app::tone` 模块已在 83e6e343（"retire the legacy backend driver stack"）中删除，全仓无 `ToneConfig` 定义，改回 `.rs` 也编译不过。它仍被 git grep/未来审计当作"计量延迟基线"误导检索（本次审计即被其 14 处 `wait_for/wait_until` 引用干扰过）。文件内自述 "ChannelBallistics is pub(super)" 等事实也已过时。
- 建议方向：直接删除（git 历史保留全部内容）；若确需保留测量脚本形态，移入 `examples/` 挂 feature 并修复 import。顺手全仓 `find -name '*.disabled'` 确认无第二例（已核：仅此一个）。
- 工作量: S

### A-2 后端准备错误是 String 错误链的第二个上游源：engine `BackendError` 三 String 变体 + 二次字符串化 [P2] [KNOWN-OPEN]（APP-1 已知方向的延伸面，11-09 范围未覆盖）
- 位置: crates/moiren-engine/src/host/backend.rs:87-95（`Unsupported(String)`/`Failed(String)`/`ResourceUnavailable(String)`）、crates/moiren-app/src/service/backend.rs:6-20（另一个同名 `BackendError { stage, reason: String }`）、crates/moiren-app/src/service/managed/lifecycle.rs:310（`.map_err(|error| host::BackendError::Failed(error.to_string()))` 二次字符串化）、crates/moiren-app/src/service/graph/preparation.rs:8-18（`PreparationFailure { node, cause: String }`）、产生点示例 crates/moiren-windows-audio/src/engine_backend.rs:336,341,391,422,500,505,515（约 15 处 `unsupported("英文句子")`）
- 证据 / 影响：11-09 APP-1 与 GraphFault 类型化战役针对的是 graph.rs 验证句（~40 句）；本发现指出**即使 GraphFault 全部类型化，fault 卡片仍会持续收到英文句子**：后端准备失败沿 `BackendError::Unsupported(String)` → `PreparationFailure.cause: String` → `record_error_with_subject` 进入同一 UI 渲染通道。这批后端句子不在 zh-CN.yml 目录、也不在 i18n_guards.rs:709 的 113 条 `DYNAMIC_KEY_EXCEPTIONS` 中（已核：无 "native driver"/"process loopback" 等条目）。另注意 service 层存在**两个同名 `BackendError`**（app 侧 struct 与 engine 侧 enum），与 WA-01 三 owner 同名是同一类命名卫生问题。
- 建议方向：把"错误类型化"的战役范围显式扩到 `BackendError`（枚举变体 + 参数化数据，Display 仅作日志）；`PreparationFailure.cause` 改持类型化载荷。此为方向登记不展开——i18n 归属兄弟代理，此处只主张**类型化**这一个侧面。
- 工作量: M

### A-3 `start_process_capture` 死 API：全仓零调用的 pub 再导出 [P3] [NEW]
- 位置: crates/moiren-windows-audio/src/process_loopback/stream.rs:29（定义）、process_loopback.rs:19（唯一引用点 = 自身再导出）
- 证据 / 影响：全仓 rg（含 examples/tests/docs）仅两处命中且均为上述定义与再导出；同组的 `prepare_process_capture_with_gate`、`start_process_capture_with_stop` 都有真实调用者（process_loopback/stream.rs:34,42,190），唯此一个是"停用即删"的退休残留。作为 pub API 它伪装成受支持的入口。
- 建议方向：删除；同文件顺检 `#[cfg(all(windows, feature = "probe-tools"))] pub(crate) use` 组（:16-17）内 `capture_format`/`process_loopback_stream_flags` 的消费点是否仍健康（已粗核在 probe 路径有消费，不动）。
- 工作量: S

### A-4 `run_ui_capture` 570 行单函数：CLI 解析 + service 启动 + gpui 运行 + 逐页捕获 + trace 落盘混排 [P3] [NEW]
- 位置: crates/moiren-app/src/ui/shell/capture.rs:257-827（570 行，区间内仅 1 个内嵌 fn）
- 证据 / 影响：设计评审截图/计量捕获工具（经 `ui/mod.rs:38` 与 `shell.rs:821` 双层再导出，入口 examples/ui_capture.rs:3）。fader-feel 战役刚两次扩它（315b0ea3 手势 sweep、5584c901 编辑回显延迟），是**活跃生长的基建函数**：`#[cfg(feature = "ui-profiler")]` 分支、meter_capture 分支、windows/非-windows 双 service 装配、每页遍历、JSON 写盘全部平铺在闭包内。当前无故障，但每加一种捕获就再长一段，阶段间共享的可变局部（directory、meter_capture、failed）越积越多。
- 建议方向：按方法论"提具名局部函数"分层（setup_service / per_page 捕获 / trace flush 三段），不跨文件、不造抽象；双层再导出顺手收敛为一处。
- 工作量: S

### A-5 `edit_candidate` 309 行单 match：每臂各自重复 validate→mutate→单源域重跑形状 [P3] [NEW]
- 位置: crates/moiren-app/src/service/graph/edit.rs:21-330
- 证据 / 影响：按 `GraphEdit` 变体一臂一逻辑，臂内重复出现"取拓扑行→validate_node_state→失败即整体丢弃 next"的三步形状（文件头注释已声明整体丢弃语义）。判定：**match 分发本身内聚**（穷尽臂是安全网，同 11-09 对 NodeKind 的裁决），不建议拆分分发；但臂间重复的 revalidation 形状可下沉为一个 `validate_edited_node` 已有函数的更多调用点（该 fn 已存在于 :9-19，仅部分臂用到）。风险低，纯收敛。
- 建议方向：把散落的重复校验序列对齐到已有 `validate_edited_node`/`validate_exports`；不做结构手术。
- 工作量: S

### A-6 windows-audio crate 根 API 面宽于实际外部消费：四组 pub 项仅 crate 内使用 [P3] [NEW]
- 位置与证据（逐名 rg 全仓交叉引用）：
  - `bind_microphone_sink`/`bind_playback_source`：唯一消费者是 crate 内 engine_backend/virtual_io.rs:72-75（virtual_owner.rs:25-26 再导出）；
  - `analyze_clock`/`analyze_device_clock`/`relative_ppm`：唯一消费者是 crate 内 physical/analysis.rs:5（clock.rs 其余命中为内部调用与内联测试）；
  - VAD 协议请求构造器 `capabilities_request`/`create_request`/`endpoint_request`/`gate_request`：唯一消费者是 crate 内 vad/client.rs 与 vad/tests.rs（vad.rs:51 根面再导出）；
  - `owned_snapshot`：以 `catalog_snapshot` 之名经 lib.rs:64 被 app 消费（service/windows.rs:11）——此项**正当 pub**，列出以示已核。
- 影响：这些是内核 RT/驱动内部件；pub 即承诺库 API 稳定，实际外部（app）从不接触。windows-audio 239 个 pub fn 对 40 个 pub(crate) fn 的比例显著高于 sibling crates（engine 260/95、signal 60/101）。
- 建议方向：随任一 windows-audio 卫生 PR（11-09 批次 3 的 WA-01 改名/WA-05/WA-08 同车）把这四组收窄为 pub(crate)；不动正当面（catalog_snapshot、backend 端口实现）。
- 工作量: M（面广但机械）

### A-7 长函数清单与判定（charter 第 3 项的完整盘点；>250 行者全部列出处与裁决）[P3] [NEW/KNOWN]
- 位置与证据（fn 间距扫描 + 人工读码复核）：
  - `run_ui_capture` 570 行（capture.rs:257）→ **A-4**。
  - `inspector` 438 / `builtin_controls` 372（node_inspector.rs:46,613）→ **KNOWN-OPEN AU-3**（11-09 已判"双巨函数值得拆"，现状未动）。
  - `prepare_with_native` 359（engine_backend.rs:229-586）→ **判定内聚不拆**：单条准备管线（update 短路→预算→虚拟 IO→分组→spawn→绑定→时钟变体），阶段顺序固定、每段有注释锚；见"明确不建议"。
  - `edit_candidate` 309（edit.rs:21）→ **A-5**。
  - `positioned` 315（graph_canvas.rs:294）、`graph_canvas_view` 311（graph_canvas_view.rs:513）、`render` 307（component_gallery.rs:153，feature 门控画廊页）、`page` 270（matrix.rs:31）、`new` 277（shell.rs:298）、`input_row` 234（mixer/routing.rs:244）→ **gpui 声明式渲染/装配惯例长**，单职责（建视图模型/画树/装配），不收。
  - `sync_mixer_controls` 255（mixer/model.rs:976）→ 单一职责（presentation 对 graph revision 的 reconcile 清扫），顺序固定，不收。
  - `configured_engine_project` 250（preparation.rs:46）→ 投影构造，内联 use 收敛后线性下降，不收。
  - 误报剔除：vst3/mod.rs `new`/`instantiate` 的"412/387 行"是 vtable statics 落在两 fn 之间所致；真实 fn 均为带 SAFETY 注释的线性 COM 序列。
- 工作量: —

## 正面登记

1. **windows-core "死依赖"线索证伪**：Cargo.toml:22-25 注释明确其为 `#[implement]` 宏展开所需并钉 "=0.62.2"，实测 moiren-windows-audio/Cargo.toml:65-67 同注释。既往线索关闭。
2. **#[allow(dead_code)] 22 处全部有据**：要么 `cfg_attr(not(test)/not(windows))` 跨平台/测试模式，要么带语义注释（如 signal/storage.rs:110 "consumed by the later transport/dispatcher tasks"——字段确被 dispatcher.rs:101-118 消费；routing.rs:48-49 gpui-test 无头套件专用；shell.rs:292-293 TouchpadRegistration 为 Drop 副作用保留）。零处"无声豁免"。
3. **可见性战役已落地且收口**：#160-#162（c63caf76/e4e09f12/ea3933f4）完成 ui 域 pub→pub(crate) 收窄；app 剩余 414 个 pub fn 抽查均为 bin 面（main.rs 用 5 项）或 tests/ 集成测试面（如 wait_for/wait_until 被 tests/common 消费），无裸漏。
4. **tray_mixer 对 mixer 页的复用是范本**：shell/tray_mixer.rs:4 明示 "Faders reuse the main mixer's rows verbatim"，经 widgets/fader.rs 与 presentation::db_text 单源（33,31 行）——AU-1/AU-5 的目标形态已有活例。
5. **生产路径 panic 卫生良好**：非测试 unwrap/expect 逐处判定后，全部为带消息的不变量断言（mutex poisoned、"sized with ballistics"、"grid built above"）或启动期失败（native.rs:154-156 字体/tray）；未发现会炸用户的数据路径 unwrap。错误处理整体纪律优于常规同规模项目。
6. **cfg 门控两级测试路径均闭环**：CI 主测试步无 feature 跑 `cargo test -p moiren-app`（runtime-foundation.yml:99），gpui 依赖测试在 devices/tests.rs 内逐项 `#[cfg(feature = "gpui-test")]` 门控（:50-500），有 gpui 的外层门控（settings/graph/mixer/matrix 等 6 处 `cfg(all(test, feature = "gpui-test"))`）与之互补——两条路都编译且各跑该跑的（对照 gpui-kit-0.7.3/src/lib.rs:122 `pub mod test` 的 test-support 门核验过）。
7. **feature 清单全部有读者**：gpui-test/ui-profiler/vad-integration-probe/component-gallery/vad-probe 均在 Cargo.toml 带意图注释、在 CI 有对应 lint/test 步。

## 明确不建议（防过度工程）

1. **不要拆 `prepare_with_native`（359 行）为多文件/状态机抽象**——它是单条准备管线，阶段固定；最多提局部函数。为行数拆会切断预算检查与 group 装配的共享局部（capacity、bindings、transport）。
2. **不要合并 WA-06/WA-07 的方向性镜像**（vad/provider.rs run_playback/run_microphone、virtual_owner/boundary builder 族）——11-09 已裁决"保留"，方向语义（reset/re-prime、fence 所有权）是专门注释的核心 RT 协议，本次复核维持。
3. **不要统一测试模块门控风格**——devices 的"外层 cfg(test)+内层逐项 gpui-test"与六页的"外层 cfg(all(test, gpui-test))"效果等价（已实证两条 CI 路径），统一化是纯 churn。
4. **不要清理 signal/storage.rs 与 cfg_attr 类 dead_code 豁免**——皆有消费点或平台语义，删注释/属性只会制造假信号。
5. **不要为 app 再发起一轮 pub→pub(crate) 战役**——sweep #160-#162 刚落地，剩余 pub 面是 bin/tests 的真实需要；新战役等有具体违规再点状处理。
6. **不要把 `edit_candidate` 拆成每变体一函数的模块**——穷尽 match 是验证安全网，拆散反而丢穷尽性检查。

## Top5

1. **A-2** 后端 String 错误链（BackendError 三 String 变体 + 二次字符串化 → fault 卡片）——GraphFault 战役后仍会漏英文句的最大剩余面，类型化范围应显式扩到它。P2, M。
2. **A-1** 582 行 .disabled 化石探针（引用已删 API）——删除即了结，S。
3. **A-6** windows-audio 四组 crate 内专用 pub API 收窄——随 windows-audio 卫生 PR 同车，M。
4. **A-4** run_ui_capture 570 行分层——活跃生长的基建函数，趁下一次扩展前提段，S。
5. **A-3** start_process_capture 死 API 删除——顺手项，S。
