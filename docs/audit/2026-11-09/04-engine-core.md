# 2026-11-09 架构审计 — moiren-engine + moiren-core

基线: main @ 27aa57d7（实测 `git rev-parse --short HEAD`，工作区干净）

范围：`crates/moiren-core`（src 1,265 行 + tests 515 行）与 `crates/moiren-engine`（src 20,708 行，其中约 5,063 行为 src 内测试；tests/ 13,180 行 / 39 个文件）。只读审计，未修改任何源文件。

## 执行摘要（≤10 条要点）

1. **moiren-core 存续合理，建议维持独立**。它被 engine、windows-audio、app 三个 crate 直接依赖；`#![forbid(unsafe_code)]`（lib.rs:1）构成可验证的安全岛；依赖仅 thiserror；protocol.rs 的 wire 类型（ProcessorId/ParamValue/ReplyCode 等）被 engine 的 compiler/control/runtime 与 app/service 双方共用，是真实的跨层稳定契约，不是历史包袱。
2. **core↔engine 边界无双向概念泄漏**。core 不引用 engine/signal/plugins 任何类型；engine 侧对 core 的消费集中在 compiler、host、rack、runtime 四层，方向单一。唯一的"语义耦合"是 NodeKind 枚举本身（见 E-1）。
3. **RT 边界可识别性为"所有权转移 + 命名 + 文档 + 测试"四重保障，且无 unsafe Send/Sync**（全库 grep 0 命中）。Engine 按值穿过 RT 界（`attach(engine: Engine<S>)` 消费所有权，backend.rs:151-157；`stop_and_join` 经 OwnerReturn 归还，backend.rs:188-194），退休对象只经 RetiredPlan 队列回控制侧（swap.rs:820-821）。
4. **RT 禁分配是测试强制的，不是口头契约**：`tests/runtime/allocation.rs` 提供 TLS 计数分配器，compiler/runtime 共 10+ 个测试断言 `render()`（含热交换路径）零分配零释放（如 tests/runtime/swap.rs:69-70、108-110）。
5. **E-1（P2，最大维护性发现）：NodeKind 闭合枚举的"种类扇出"**。新增一种节点需同步修改至少 5 处、跨 2 个 crate：core 的 `NodeKind` + `initial_ports`、engine 的 `Binding` 枚举 + `bind_*` + `validate` match + `plan.rs` lowering match + host 的平行枚举 `NodeConfig`。仅 `NodeKind::Gain` 在 engine+app 出现 74 次。这是穷尽匹配安全网的代价，建议固化 checklist 而非开放注册。
6. **runtime/swap.rs 1160 行判定为内聚**：约 797 行实现（候选准备/发布/RT 四阶段提交/退休的完整 plan 交换生命周期，全部围绕 PlanSnapshot/Transfer/RtPlanPort 一个子系统）+ 363 行内联测试。不建议按行数拆分；仅测试放置位置违反仓库惯例（E-3）。
7. **Supervisor 是唯一的"宽结构"信号**：28 个字段（host/supervisor.rs:51-80）的状态机。属于控制侧生命周期状态机，可接受的结构性债务（E-2），未达 UI 层 95→14 那种失控程度；其余核心类型（Engine 7 字段、ExecutionPlan 9、LogicalGraph 3、EngineConfig 3）全部健康。
8. **test_support.rs 定位正确**：`#[cfg(test)] pub(crate) mod test_support`（lib.rs:18-19），仅 17 行，不编入生产 crate；与 tests/common/mod.rs 的平行 `pump` 是 Rust 集成测试可见性限制，双方均有文档互相指向（test_support.rs:1-5、tests/common/mod.rs:1-10）。
9. **X-1 对账：已解决**。app 现在直接 `use moiren_engine::meter::signal::{AudioWindow, ChannelLevel}`（app/src/ui/monitoring.rs:8）与 `SpectrumWindow`（app/src/ui/monitoring/spectrum.rs:12），不再镜像 level 类型；本轮未发现新的跨 crate 类型镜像实例。
10. 小毛病三件（P3）：模块声明风格双轨（18 处 .rs+dir vs 3 处 mod.rs）、engine 内的 winmm 平台钩子（meter/polling.rs）、host.rs 的 glob re-export。

## 发现清单

| ID | 级别 | 标题 | 位置 | 证据摘要 | 建议 + 权衡 |
|---|---|---|---|---|---|
| E-1 | P2 | NodeKind 闭合枚举的跨 crate 扇出，新增节点成本高 | crates/moiren-core/src/graph/model.rs:45-102; crates/moiren-engine/src/compiler/bindings.rs:20-57,163-207; crates/moiren-engine/src/compiler/plan.rs:138-253; crates/moiren-engine/src/host/project.rs:20-30 | 一个节点种类有 5+ 个平行定义点：core `NodeKind` 枚举 + `initial_ports()`；engine `Binding<S>` 枚举（bind_* 方法约 30 个，bindings.rs:63-378）+ `validate` 穷尽 match + plan.rs lowering 穷尽 match；host 又有平行枚举 `NodeConfig`（project.rs:20 起，project.rs 内 58 处引用）。`NodeKind::Gain` 在 engine+app 共 74 处 | **维持闭合枚举**：穷尽 match 是漏配即编译失败的实时安全网，改为 trait/registry 开放注册会失去该保证并引入动态分发。建议在 docs 写「新增节点 checklist」把 5 个触点固化；若样板继续膨胀再考虑 declarative 宏生成 bind_*/validate。不为消除重复而引入运行时注册表 |
| E-2 | P2（边缘） | Supervisor 28 字段平铺状态机 | crates/moiren-engine/src/host/supervisor.rs:51-80 | 生命周期槽位（active/pending/preparing/blocked/fault/cleanup_error/cleanup_inflight）+ 外围（edits/router/structural_values/resource_nodes/版本计数 5 个 u64）平铺一个 struct；逻辑已按 requests/snapshot/updates 三个子文件拆分方法 | 可接受的结构性债务。可选：仅把 `edits`/`structural_values`/`resource_nodes` 收敛为一个 `PendingIntents` 子结构、版本五元组收敛为 `Revisions`。**不建议**强行合成 Phase enum——会让每次转移都解构重组，borrow 更糟；现状每个状态可独立读写、测试可直接构造。等下次因字段冲突出 bug 时再动 |
| E-3 | P3 | swap.rs 内联 363 行测试违反「tests.rs 同级文件」惯例 | crates/moiren-engine/src/runtime/swap.rs:56-418（`mod signal_tests`） | 仓库 9 个模块用同级 tests.rs（spectrum/tests.rs、node/tests.rs、rack/tests.rs、processor/tests.rs、recorder/tests.rs 等），唯 swap.rs 内联。swap.rs:88-90 注释解释的是「为何与 tests/common 重复」而非「为何内联」 | 移至 `src/runtime/swap/tests.rs`，swap.rs 内改 `#[cfg(test)] mod tests;`。纯机械移动，零语义变化，统一惯例 |
| E-4 | P3 | 模块声明风格双轨：mod.rs vs .rs+dir | crates/moiren-engine/src/processor/mod.rs; processor/builtin/mod.rs; signal/mod.rs | 18 处用 Rust 2018 风格 `.rs + 同名目录/`，3 处用旧式 `mod.rs`（processor、processor/builtin、signal——恰好是两个"纯目录"模块） | 低优先统一为 .rs+dir（`processor.rs` + `processor/`）。机械重命名，但会触碰全部 `use crate::processor::…` 路径的 git blame；收益仅一致性。可在下次大 PR 顺路做，不值得单独 PR |
| E-5 | P3 | engine 内的 Win32 平台钩子（winmm） | crates/moiren-engine/src/meter/polling.rs:41-46 | `#[link(name = "winmm")]` + timeBeginPeriod/timeEndPeriod 直接出现在平台中立层；已 cfg(windows) 隔离、SAFETY 注释齐全、仅控制线程调用（polling.rs:1-6 文档明示） | 单个、有正当理由（计量需求驱动的 1ms 定时器精度）且完全隔离的例外，可容忍。**若出现第二个平台钩子再迁移**到 windows-audio（通过 host 的 CancellationToken hook 或 trait 注入）；现在抽象是为一个实例造抽象 |
| E-6 | P3 | host.rs glob re-export 模糊符号来源 | crates/moiren-engine/src/host.rs:13-20 | `pub use backend::*; model::*; project::*; snapshot::*;`——`CancellationToken`、`BackendError`、`EngineProject` 等消费方一律写 `moiren_engine::host::X`，需跳进 4 个文件才知道定义处 | 保持现状：改具名导出需 churn app/windows-audio 数十处 use，收益小。新模块（signals、snapshot 已是具名）延续具名风格即可，存量 glob 不动 |
| C-1 | 结论 | core 存续合理（非发现，判定） | crates/moiren-core/src/lib.rs:1-5; protocol.rs:14-44 | 见执行摘要第 1 条 | 维持独立。合并进 engine 的唯一收益是少一个 crate，代价：丢失 `#![forbid(unsafe_code)]` 证明（engine 有 buffer/view.rs 原始指针 unsafe 与 E-5 的 winmm）、丢失最小编译单元、未来任何 out-of-process 编辑器/IPC 端点都要拖着整个 engine |
| C-2 | 结论 | 逻辑拓扑→编译→预备计划→RT 执行链路在模块结构上可读 | compiler.rs:151-281 → runtime.rs:169-367 → runtime.rs:511-593 → windows-audio engine_backend/owner.rs:513-553 | compile_inner 顺序完成 validate→bindings.validate→plan::build→storage::optimize→BufferArena→ExecutionPlan::prepare_compiled_inner→Engine::new，与 lib.rs 文档声明的分层一一对应 | 无需行动；这是对 2026-10-22「架构健全」结论的复核确认 |

## 模块分层地图（职责一句话 + 依赖方向）

依赖方向均单向向下；`→` 表示"依赖"。

| 模块 | 行数(src) | 职责 | 依赖 |
|---|---|---|---|
| moiren-core::graph | 1,008 | 非 RT 可编辑拓扑（节点/端口/边/环检测/拓扑序），封装字段、只经方法变异 | 仅 thiserror |
| moiren-core::protocol | 298 | 版本化定长控制帧编解码（参数请求/回复），纯 wire 契约 | 仅 thiserror |
| engine::sample | ~30 | ProcessingSample 特征：处理精度与设备 PCM 打包解耦 | 无 |
| engine::audio_config | 231 | 持久化的音频意图配置，独立于平台发现/协商 | core |
| engine::buffer(+view) | 613 | 单一拥有的平面缓冲区 + 作用域 IO 视图（view.rs 含受控 unsafe 指针视图） | sample |
| engine::boundary(+bridge) | 419 | 设备无关流契约 + 定容 SPSC 音频桥；无 COM/驱动缓冲穿越 | buffer, processor |
| engine::control(+5) | ~700 | 参数管线：IPC worker→控制 owner→有界 SPSC→RT 表→回复（rtrb）；FloatRamp 参数斜坡 | core::protocol, signal(仅 moiren-signal?) 否——仅 core |
| engine::processor(+builtin,plugin) | ~2,900 | RtProcessor/RtObserver 安全 DSP 契约 + 全部内置内核 + CLAP/VST 适配 | buffer, control, sample, moiren-signal, moiren-plugins（**仅 plugin.rs 一个文件**） |
| engine::signal(dir) | ~250 | 引擎侧信号发布能力的编译期绑定（plan/process） | moiren-signal |
| engine::compiler(+7) | ~2,000 | 非 RT 降载：拓扑→调度→生存期缓冲规划→fused mix→优化→增量 update | core, buffer, control, runtime, processor, rack, boundary, node, recorder, spectrum |
| engine::runtime(+mix,swap) | 1,923 | 线性执行器：ExecutionPlan 预备、Engine::render RT 循环、plan 热交换四阶段提交 | buffer, control, processor, sample, moiren-signal, core(ProcessorId) |
| engine::node(+2) | ~600 | 预备的 IO 端点（Input/OutputNode、虚拟端点）；预备/销毁皆非 RT | buffer, boundary, processor |
| engine::rack(+3) | ~1,130 | 控制侧有序处理器链配置，编译期展开（含 routing/prepare） | core, processor |
| engine::meter(+2) | ~560 | Meter 节点设置 + 需求门控 peak/RMS 内核 + Win32 计时精度 | processor, moiren-signal |
| engine::spectrum(+fft) | ~580 | 需求门控精确窗口分析器；FFT 已在音频线程外（e8aa9ffe，2026-11-03 PR#96 确认在案） | processor, moiren-signal |
| engine::recorder(+wav×4) | ~1,700 | 有界录音传输：observer 只转换发布完整块，WAV worker 独立线程 | buffer, processor |
| engine::host(+10) | ~4,100 | 非 RT 托管宿主：EngineHost/Supervisor 状态机、项目模型、准备/退休 worker、信号中枢 | 以上全部 + core |

链路可读性（C-2）：`LogicalGraph`(core) → `compiler::compile*` 产出 `CompiledGraph{Engine, ControlPort, CompiledBindings}` → `Engine` 连同所有权移入平台 transport（windows-audio 的 PreparedTransport::attach）→ `Engine::render` 在音频线程跑 → `RetiredPlan`/`OwnerReturn`/`into_parts` 把对象送回控制侧销毁。每一跳的模块边界与 AGENTS.md「逻辑拓扑/预备计划/平台传输分离」逐条对得上。

## RT 边界可识别性评估

结论：**结构可识别、纪律测试强制，但无编译器级标记**——对该代码库规模而言这是合理的设计选择。

四层证据：

1. **所有权转移（最强机制）**：全库无 `unsafe impl Send/Sync`（engine+core grep 0 命中）。Engine 跨界即 move：`PreparedTransport::attach(self: Box<Self>, engine: Engine<S>, …)`（backend.rs:151-158）消费所有权，`BackendOwner::stop_and_join(self: Box<Self>) -> OwnerReturn<S>`（backend.rs:188）连错误都必须保留 Engine 归还（backend.rs:179 文档「Must preserve Engine ownership on every native error and caught RT panic」）。RT 侧不可能意外与控制侧共享 Engine——类型系统禁止。
2. **命名约定**：RT 侧类型一律 `Rt` 前缀——RtProcessor、RtObserver（processor/mod.rs:48,90）、RtResources（runtime.rs:100）、RtPlanPort（swap.rs:846）、RtAudioSource/RtAudioSink（boundary.rs:35,41）、moiren-signal 的 RtHost。约 50 处 doc 注释含 non-RT/realtime/audio thread 字样；关键契约写在 trait 文档上（processor/mod.rs:75-77「No allocation, locks, syscalls, logging, IPC or unbounded work」）。
3. **退休路径显式建模**：RetiredPlan 文档「Drop only on the control thread」（swap.rs:820-821）；`Engine::into_parts`（runtime.rs:596-603）与 `PreparedPlan::retire_controls`（swap.rs:645-647）把销毁义务写成 API 而非约定。
4. **测试强制**：`tests/runtime/allocation.rs` 的 TLS 计数分配器在 10+ 处断言 `render()` 零分配，**包括热交换激活路径**（tests/runtime/swap.rs:69-70、108-110、150-152 等七处）与 fused mix、优化调度（tests/compiler/optimization.rs:137,299）。

薄弱点：`Engine::render` 本身没有类型/属性级"RT-only"标记（无法阻止新调用方在错误线程调用——但 ownership 转移已把调用方限制为 transport owner）；unsafe 只存在于 buffer/view.rs 的受控视图与 polling.rs 的 winmm，均有 SAFETY 注释与 `deny(unsafe_op_in_unsafe_fn)`（lib.rs:1）。

## 与既往审计对账

| 既往项 | 本轮状态 |
|---|---|
| 2026-10-22「架构健全、无需拆 crate」 | 复核维持（C-2）。moiren-signal 独立、engine 内部分层清晰，无新裂仓压力 |
| 2026-11-03 X-1：engine/meter 与 app ui 的 level 类型镜像 | **已解决**：app/src/ui/monitoring.rs:8 与 app/src/service/signals.rs:2 直接复用 `moiren_engine::meter::signal::{AudioWindow, ChannelLevel}`；ui/monitoring/ballistics.rs 经 `super::` 转发同一类型，无本地副本 |
| X-1 追问：spectrum 是否新增镜像实例 | **无**。app/src/ui/monitoring/spectrum.rs:12 直接 `use moiren_engine::spectrum::SpectrumWindow`；app 侧 `SpectrumParams` 是纯显示参数（分频带/锚点/dBFS 界），不复制引擎数据结构 |
| 频谱 FFT 挪出音频线程（2026-11-03 PR#96） | 在案：git e8aa9ffe `feat(engine): move the spectrum FFT off the audio thread`；spectrum.rs:1-4 文档与 fft.rs 的 Radix2Fft 工作区结构一致 |
| 2026-11-03 后新增的重量级变更 | plugins crate 集成（d7f95567）——engine→plugins 耦合被限制在单文件 processor/plugin.rs（285 行，grep 全部 9 处命中均在此），缝口干净 |

## 维护性建议 Top 5（含权衡）

1. **为「新增节点种类」写一份 checklist 文档（对应 E-1）**。五个触点（core NodeKind+initial_ports、engine Binding+bind_*+validate、plan.rs lowering、host NodeConfig、app catalog）目前只靠穷尽 match 的编译错误逐个暴露。文档成本半天，把隐性流程显性化。**不要**为消除样板改成运行时注册表——穷尽检查的编译期安全性对 RT 系统价值更高。
2. **把 swap.rs 的 signal_tests 移到 runtime/swap/tests.rs（E-3）**，与其余 9 个模块的测试放置惯例对齐。纯机械移动。顺手可把 swap.rs 顶部错误枚举与实现之间不再隔 363 行测试，可读性净赚。
3. **冻结而非修复 Supervisor 宽度（E-2）**：下次自然触碰 supervisor.rs 时，把 `edits`/`structural_values`/`resource_nodes` 收敛为一个子结构即可，不单独开 PR。强行政合成状态机会让转移代码更难读。
4. **模块风格统一（E-4）与 host glob 导出（E-6）均挂起**：收益是一致性，成本是 git blame 稀释与全库 use churn。设定规矩「新模块一律 .rs+dir、新导出一律具名」，存量等顺路。
5. **守住两道已经做对的缝，别让它们退化**：engine→plugins 只许 processor/plugin.rs 一个文件引用 moiren-plugins；engine→平台只许 meter/polling.rs 一个 winmm 钩子（E-5）。出现第二个平台钩子时再抽注入点，现在抽是为单实例造抽象。
