# Engine Managed Lifecycle Implementation Plan

> For agentic workers: implement one independently testable backend milestone at a time. Use superpowers:executing-plans for inline execution or superpowers:subagent-driven-development when independent implementers are available. Checkboxes track implementation, not completion of this planning document.

**Goal:** 引擎宿主打开后自动运行，图编辑自动更新，宿主释放时自动完成停止和回收。

**Architecture:** `moiren-engine` 的非 RT EngineHost 持有项目和协调器，自动管理资源取得、计划准备、块边界激活及非 RT 回收。`moiren-windows-audio` 实现平台资源契约。客户端提交图事务或参数修改并观察结果。

**Tech Stack:** Rust 2024、现有 stable 工具链、LogicalGraph、rtrb、std 线程与同步工具、现有 moiren-signal 和 WASAPI owner。

设计依据：[引擎托管生命周期与自动图更新](../specs/2026-10-13-engine-managed-lifecycle-design.md)。本文安排整个后端改动的顺序、接口责任与验收场景；具体 Rust API 签名随各里程碑的实现评审确定。

## 实施状态（2026-10-14）

T1–T7 的后端实现已交付；T8 的可移植示例与后端文档已交付，应用/UI 采用仍是独立后续集成。下文保留原始验收清单，逐项实现与测试证据、硬件验证限制见 [验收记录](2026-10-14-engine-managed-lifecycle-validation.md)，不把计划中的硬件故障场景当作已执行实验。

实际模块按职责细分为 `host/supervisor/{requests,updates,snapshot}.rs`，IO 复用回归位于 `tests/runtime/update.rs` 与 `tests/runtime/signals.rs`，而非原拟定的 `tests/host/reuse.rs`。所有权占位检查位于 `runtime/swap/reservation.rs`，一致时间线位于 `render/timeline.rs`。

## Global Constraints

- 准备图、buffer、绑定和设备的工作全部在非 RT 域完成。
- RT 不进行堆分配、析构计划、阻塞等待、设备枚举或图修改。
- `LogicalGraph` 是唯一拓扑权威，处理器和 IO 配置以稳定图 ID 关联。
- Engine 不依赖 windows-audio；平台适配器依赖 engine 定义的契约。
- 状态与接口依据后端语义确定，现有应用和界面结构不限制设计。
- 图版本、计划版本、owner generation、资源 generation 与时间线 epoch 分别维护。
- 普通准备或设备故障不删除期望项目；失效 IO 上的旧计划不能继续作为有效回退。
- 所有拥有的计划、队列端点、信号绑定和原生 session 返回非 RT 域后销毁。
- 保留低层离线执行入口及当前 RT 分配、并发、DSP 与 Miri 覆盖。
- 不新增依赖作为前提，不改动无关工作区内容，不自动创建分支或提交。

## 依赖与完成边界

```text
T1 项目与状态模型
  └─ T2 托管宿主与资源 owner
       ├─ T3 更新编译与稳定实例复用
       │    └─ T4 自动图更新与参数路由
       └─ T5 Windows 初始资源适配
            └─ T6 Windows 热更新与一致时间线（另依赖 T3/T4）
                 └─ T7 资源事件与恢复
                      └─ T8 无界面端到端验收与应用采用
```

T1/T2 首先形成可独立使用的宿主自动生命周期；T3/T4 在可移植后端上交付自动图更新；T5/T6 将契约落实到原生音频；T7 完成自动资源恢复。T8 接入调用方时不重新定义后端契约。

T7 完成即达到本轮后端交付边界。T8 的应用采用是独立后续集成；后端交付不等待正在演进的界面。

## T1：项目、编辑事务与状态模型

**Files**

- Create: `crates/moiren-engine/src/host.rs`
- Create: `crates/moiren-engine/src/host/project.rs`
- Create: `crates/moiren-engine/src/host/model.rs`
- Modify: `crates/moiren-engine/src/lib.rs`
- Create: `crates/moiren-engine/tests/host/main.rs`
- Create: `crates/moiren-engine/tests/host/project.rs`

**Interfaces**

- 项目包含 LogicalGraph、NodeId 对应的静态处理配置、节点/边界对应的 IO 选择、期望实时参数和编译预算。
- 编辑凭据关联单个事务；结果区分接受、拒绝、被取代和激活确认，附带稳定图身份。
- 快照分别报告宿主状态、执行状态、期望图版本、已激活图版本和具体阻塞原因。
- 图事务一次验证整个候选项目；成功后原子提交，失败后不影响原项目。

- [ ] 建立项目模型与配置完整性检查：节点不存在、配置 kind 不匹配、IO 角色/声道不匹配分别返回结构化错误。
- [ ] 建立宿主、执行和编辑结果类型；所有单调计数器检查溢出，溢出时拒绝新工作且保留关闭能力。
- [ ] 实现非 RT 编辑事务；批次可以先创建节点再配置/连接，不允许发布部分完成的批次。
- [ ] 验证事务失败不改变原项目，删除节点同时移除其配套配置，边的实时参数变化不误判为拓扑更新。
- [ ] 验证未连接输入仍属于合法图，图无输出需求进入 Dormant 而非拓扑错误。

Run: `cargo test --locked -p moiren-engine --test host project`

Expected: 项目事务测试通过，现有 core Graph 语义保持一致。

## T2：托管宿主、自动运行与自动关闭

**Files**

- Modify: `crates/moiren-engine/src/host.rs`
- Create: `crates/moiren-engine/src/host/backend.rs`
- Create: `crates/moiren-engine/src/host/supervisor.rs`
- Create: `crates/moiren-engine/src/host/retirement.rs`
- Create: `crates/moiren-engine/tests/host/support.rs`
- Create: `crates/moiren-engine/tests/host/lifecycle.rs`
- Modify: `crates/moiren-engine/tests/host/main.rs`

**Interfaces**

- EngineHost 是唯一运行 owner；EngineControl/EngineObserver 不持有生命周期 owner。
- 平台契约分开表达资源准备、准备完成但未启动的 owner、启动确认、取消和拥有权返回。
- 资源状态用稳定资源身份与 generation 表达；准备工作接收取消令牌。
- 音频 owner 返回完整的 Engine/控制相关拥有权，错误返回也不能遗漏可回收对象。
- 准备通道与回收通道分离，派发拥有资源的工作前预留完成结果及回收容量。

- [ ] 建立不使用硬件的可控后端，支持准备阻塞、取消、启动确认、资源缺失、owner 返回和 worker panic。
- [ ] 实现 open 自动调度准备：无输出需求 Dormant，有输出但资源缺失 WaitingForResources，启动确认后 Running。
- [ ] 实现有界命令及完成处理；关闭使用独立的持久标志与唤醒，不能被已满命令队列阻挡。
- [ ] 实现 EngineHost Drop 自动取消与等待闭合；控制及观察句柄存活时依然能够 Closed。
- [ ] 验证关闭与准备完成竞态、激活竞态、准备失败和 panic；所有 Drop 探针必须出现在非 RT 的准备/回收域。
- [ ] 验证阻塞准备不阻挡已有 owner 的停止/回收；未返回的准备资源不得提前释放。
- [ ] 验证 Closed 只能在所有 owner、准备、退役、控制终结和回收工作排空后出现。

Run: `cargo test --locked -p moiren-engine --test host lifecycle`

Expected: 没有手动 start/stop/join 调用的用例完成整个生命周期，不遗留线程和拥有权。

## T3：更新编译、稳定边界与 DSP 复用

**Files**

- Create: `crates/moiren-engine/src/compiler/update.rs`
- Modify: `crates/moiren-engine/src/compiler.rs`
- Modify: `crates/moiren-engine/src/compiler/plan.rs`
- Modify: `crates/moiren-engine/src/compiler/bindings.rs`
- Modify: `crates/moiren-engine/src/runtime.rs`
- Modify: `crates/moiren-engine/src/runtime/swap.rs`
- Create: `crates/moiren-engine/tests/host/reuse.rs`
- Modify: `crates/moiren-engine/tests/host/main.rs`
- Modify: `crates/moiren-engine/tests/runtime/swap.rs`

**Interfaces**

- 输入是期望项目快照、已确认的 active metadata 和资源身份；输出是资源增量、复用槽位描述与 prepared update。
- 复用槽位描述没有对 live processor 的借用，取消候选只能回收候选新建资源。
- 实例匹配使用 NodeId/EdgeId 等稳定身份，并验证静态配置、IO、参数和信号 schema。
- 同一传输身份的 bridge 端点在 active/candidate 之间只有一个实际 owner。

- [ ] 先定义可取消的复用槽位与新实例包；保证候选准备不需要复制输入 reader 或输出 writer。
- [ ] 把低层 ProcessorId 生成与稳定图身份关联；不能凭编译顺序推断实例连续性。
- [ ] 为兼容 IO 实例实现激活边界的拥有权迁移，拒绝 schema 相同但后端资源身份不同的复用。
- [ ] 保留既有 with_reuse 检查与信号 activation 事务；拒绝和取消路径不动 active state。
- [ ] 验证计数 source、Compressor 历史、参数 ramp 和信号 publisher 的连续性。
- [ ] 验证回收队列满时延迟激活、pending 满时返回候选拥有权、stale basis 拒绝及取消竞态。
- [ ] 沿用分配跟踪，证明兼容更新的 RT prepare-free 激活路径无分配和析构。
- [ ] 在音频 owner 线程内执行分配跟踪；控制侧的正常准备分配不计入 RT 结果。

Run: `cargo test --locked -p moiren-engine --test host reuse`

Run: `cargo test --locked -p moiren-engine --test runtime swap`

Expected: 候选可在不复制 live IO 端点的条件下准备，块边界成功激活并保留兼容状态。

## T4：自动图更新与稳定参数目标

**Files**

- Modify: `crates/moiren-engine/src/host/supervisor.rs`
- Create: `crates/moiren-engine/src/host/parameters.rs`
- Create: `crates/moiren-engine/src/host/signals.rs`
- Modify: `crates/moiren-engine/src/host/model.rs`
- Modify: `crates/moiren-engine/src/host/retirement.rs`
- Create: `crates/moiren-engine/tests/host/updates.rs`
- Create: `crates/moiren-engine/tests/host/parameters.rs`
- Create: `crates/moiren-engine/tests/host/signals.rs`
- Modify: `crates/moiren-engine/tests/host/main.rs`

**Interfaces**

- 接受图事务自动使准备状态变更；没有单独 refresh 请求。
- GraphRevision 对应期望拓扑/静态配置；PlanRevision 对应发布尝试，二者可不同。
- 参数目标是稳定节点/边身份及参数语义；计划内 ParameterKey 由后端解析。
- 编辑凭据的激活结果和参数凭据的应用结果分别确认。
- 执行信号宿主和绑定由托管层创建、附加并退役，订阅消费不持有运行 owner。

- [ ] 实现最多一个准备任务、一个已发布候选和最新期望快照；编辑仍按接受顺序提交到项目。
- [ ] 在完成项到达与候选发布时检查 graph/owner/resource generation 和 active basis，自动回收过时结果。
- [ ] 实现接受 → 准备 → 发布 → 确认状态转换；编译完成不推进 active 图版本。
- [ ] 实现普通准备受阻时保留项目及仍有效的 active owner，阻塞原因附在更新结果上。
- [ ] 记录受阻版本和依赖 generation；同一条件在后续 poll 或状态读取时不重新派发准备。
- [ ] 实现参数路由，已有节点不重编译，新节点记录期望值并在激活后补齐准备期间的变化。
- [ ] 实现旧控制通道终结和稳定目标重新路由；显式 frame 调度不得跨 epoch 迁移。
- [ ] 自动管理 RtHost、prepared bindings 和激活权限；兼容热切换沿用宿主，更换 owner 后旧 publisher 失效。
- [ ] 验证连续编辑合并、被取代凭据、取消先于/晚于激活、删除参数目标和 queue backpressure。
- [ ] 验证每个已接受参数请求终结一次，换图和关闭不遗留等待凭据。
- [ ] 验证信号宿主无手动 attach，旧数据不冒充新 owner 的实时数据，订阅保留时关闭仍完成。

Run: `cargo test --locked -p moiren-engine --test host updates`

Run: `cargo test --locked -p moiren-engine --test host parameters`

Run: `cargo test --locked -p moiren-engine --test host signals`

Expected: 测试调用方只提交图/参数修改，后端自动生效且明确区分 desired 与 active。

## T5：Windows 初始资源与原生 owner 适配

**Files**

- Create: `crates/moiren-windows-audio/src/engine_backend.rs`
- Create: `crates/moiren-windows-audio/src/engine_backend/preparation.rs`
- Create: `crates/moiren-windows-audio/src/engine_backend/owner.rs`
- Modify: `crates/moiren-windows-audio/src/lib.rs`
- Modify: `crates/moiren-windows-audio/src/render/wasapi.rs`
- Modify: `crates/moiren-windows-audio/src/render/wasapi/preparation.rs`
- Create: `crates/moiren-windows-audio/tests/engine_backend.rs`

**Interfaces**

- 适配器实现 T2 的 engine 资源契约，负责设备解析、格式/period/队列协商和可取消准备。
- 原生对象留在 COM owner，跨线程包装只携带控制令牌、诊断及可移交的纯 Rust runtime。
- Capture/render 使用共同的激活资格，原生 Start 确认后才报告 Running。

- [ ] 基于已有 capture/render preparation 与 ActivationGate 实现适配器，避免新建第二套原生 stream 实现。
- [ ] 把必要的 capture/render 联动停止、join 和 runtime 返回责任下沉到 Windows owner 包装。
- [ ] 保留目前共享 period、队列延迟预算、时钟桥与诊断的已验证行为。
- [ ] 验证 prepare 返回失败或调用方取消时，两侧 owner 都被停止并交回拥有权。
- [ ] 验证 render 失败后仍返回 renderer，capture 失败不能报告成功的静音会话。
- [ ] 用假原生 owner 完成平台包装的可控回归；真实设备启动另列硬件验收。

Run: `cargo test --locked -p moiren-windows-audio --test engine_backend`

Expected: Windows 适配器遵循 engine 的拥有权与激活契约，不依赖 app 的 SessionSpec。

## T6：Windows 图热更新与一致时间线

**Files**

- Modify: `crates/moiren-windows-audio/src/render.rs`
- Modify: `crates/moiren-windows-audio/src/render/wasapi/packet.rs`
- Modify: `crates/moiren-windows-audio/src/engine_backend/owner.rs`
- Modify: `crates/moiren-windows-audio/tests/render.rs`
- Modify: `crates/moiren-windows-audio/tests/engine_backend.rs`

**Interfaces**

- 同一传输 generation 的 DemandRenderer 消费同一输出 bridge，兼容图修改只更换执行计划。
- 一致时间线包含 owner generation、epoch、revision、frame；观测使用有界读取并允许暂不可用。
- 配置或传输不兼容时，通过自动 owner 更换获得新 epoch，旧 owner 返回前不释放其资源。

- [ ] 添加原生 renderer 热更新回归：准备两个兼容图，保持一个输出消费者，修改后得到新计划样本。
- [ ] 满足稳定 bridge 的拥有权测试后，替换现有 PlanSwitchingUnsupported 限制。
- [ ] 更新时间线发布和观测：切换后 revision 更新，读端不返回跨版本拼接的 frame/epoch。
- [ ] 验证队列短缺、被取消候选和回收容量不足均不造成重复消费、失配输出桥或 RT 析构。
- [ ] 实现不兼容配置的自动取消 → 回收 → 新 owner 启动，明确记录音频间隙与新 epoch。
- [ ] 硬件验证相同输出设备的图修改不调用原生 Stop/Start；换设备后自动完成新的启动。

Run: `cargo test --locked -p moiren-windows-audio --test render`

Run: `cargo test --locked -p moiren-windows-audio --test engine_backend`

Expected: 热更新实际进入原生输出，时间线 revision 正确，不只在离线 Engine 上通过。

## T7：资源事件、等待与自动恢复

**Files**

- Modify: `crates/moiren-engine/src/host/backend.rs`
- Modify: `crates/moiren-engine/src/host/supervisor.rs`
- Modify: `crates/moiren-engine/src/host/model.rs`
- Modify: `crates/moiren-windows-audio/src/engine_backend/preparation.rs`
- Create: `crates/moiren-engine/tests/host/resources.rs`
- Modify: `crates/moiren-engine/tests/host/main.rs`
- Modify: `crates/moiren-windows-audio/tests/engine_backend.rs`

**Interfaces**

- 资源事件携带稳定身份与新 generation，协调器只重新评估受影响依赖。
- 默认设备选择与显式设备身份分别保存；进程选择包括身份验证，PID 复用不是恢复。
- 适配器对缺少可靠通知的可恢复条件提供有界、可取消探测；同一资源至多一个 outstanding 探测。
- 更新受阻和 WaitingForResources 均保留期望项目；静态图/配置错误不安排资源探测。

- [ ] 实现资源 generation 变化使旧准备结果失效，资源恢复自动触发准备，无需新的启动请求。
- [ ] 实现显式设备等待原身份、默认设备跟随当前默认身份的不同规则。
- [ ] 实现输出需求消失时自动释放 owner，再次增加需求时自动取得 owner。
- [ ] 实现受限 availability 探测：配置间隔下限和并发上限，退出时取消，不在 RT 运行。
- [ ] 使用可控时钟验证通知合并、通知缺失的探测恢复、probe 并发上限和重复事件不形成重启循环。
- [ ] 验证设备拔插、占用解除和进程退出后保留项目，阻塞原因准确；失效 IO 的旧计划被回收。
- [ ] 执行独立硬件验收，记录真实设备、资源事件和恢复路径，不把假后端的通过当作硬件通过。

Run: `cargo test --locked -p moiren-engine --test host resources`

Run: `cargo test --locked -p moiren-windows-audio --test engine_backend`

Expected: 依赖条件改变后后端自行推进，同一静态错误没有周期性编译或 stream 重启。

## T8：无界面验收、文档与调用方采用

**Files**

- Create: `crates/moiren-engine/examples/managed_graph.rs`
- Modify: `crates/moiren-app/src/lib.rs`
- Modify: `crates/moiren-app/src/main.rs`
- Modify: `crates/moiren-app/src/service/mod.rs`
- Modify: `crates/moiren-app/src/service/driver.rs`
- Modify: `crates/moiren-app/src/service/backend.rs`
- Modify: `crates/moiren-app/src/service/windows.rs`
- Modify: `crates/moiren-app/src/service/model.rs`
- Modify: `crates/moiren-app/src/service/parameters.rs`
- Modify: `crates/moiren-app/src/service/signal_preparation.rs`
- Modify: `crates/moiren-app/tests/service_backend.rs`
- Modify: `crates/moiren-app/tests/service.rs`
- Modify: `crates/moiren-app/README.md`
- Modify: `crates/moiren-windows-audio/README.md`

**Interfaces**

- 无界面例子只打开宿主、提交图/参数修改并读取凭据，然后让 owner 自然离开作用域。
- 应用 Service 将应用命令映射到托管控制入口，消费状态快照；删除重复的音频 owner 生命周期协调。
- CLI 的运行时长属于应用任务策略，到期释放宿主；引擎不持有 UI 或演示时长概念。

- [ ] 交付可移植的 managed_graph 示例，验证打开 → 编辑 → 自动激活 → owner Drop 的完整路径。
- [ ] 将应用端已有音频 owner 控制移交托管宿主；保留应用邮箱、项目保存和信号订阅等应用职责。
- [ ] 更新受影响的应用测试，验收统一后端行为；没有界面测试依赖才是后端阶段完成。
- [ ] 文档说明自动生命周期、资源等待、active 确认以及保留的低层离线 API。
- [ ] 执行下述最终检查；修改了相关代码后再扩展已有 allocation 和 Miri 验证。

```text
cargo test --locked -p moiren-engine
cargo test --locked -p moiren-windows-audio
cargo run --locked -p moiren-engine --example managed_graph
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Expected: 后端可移植用例通过，workspace 兼容现有调用方。原生设备恢复与热更新的硬件证据单独记录；CI 保留 Ubuntu/Windows 和 nightly Miri 默认/Tree Borrows 检查。

## 评审关口

- T2 后检查自动关闭与拥有权：仍要求调用方手动 join/retire 即未达标。
- T4 后检查更新语义：编译完成提前声称 active、旧版本覆盖新版本或参数终结缺失即未达标。
- T6 后检查原生链路：桥接替换失配、时间线 revision 固定或仅删拒绝检查即未达标。
- T7 后检查恢复：故障清空项目、PID 复用被误识别为恢复或静态错误无限重建即未达标。
- T8 后检查边界：必须打开某个界面才能验证宿主生命周期即未达标。
