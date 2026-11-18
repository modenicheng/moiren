# 架构约束与扩展点

本文管跨 crate 的架构规则；crate 内部的模块图与局部契约在 [crates/](crates/) 下。

## 三层分离

1. **逻辑拓扑**（`moiren-core`）：可编辑 `LogicalGraph`，非实时、传输无关。端口表达流，不表达通道号或实时 buffer 地址；只有图操作能修改模型。
2. **预备执行计划**（`moiren-engine::compiler` + `host`）：绑定、plan、prepared storage 在控制侧构建，在块边界原子切换；退役 plan 回控制侧回收。
3. **平台传输**（`moiren-windows-audio`）：WASAPI/ASIO/虚拟设备与原生 owner。平台类型不得泄漏进 core/engine 的公共契约。

设计背景见 `docs/designs/02-engine-design.md`、`04-logical-graph.md`、`05-graph-compiler.md`、`14-managed-routing-low-latency.md`、`16-render-architecture.md`。

## 实时规则

- 音频线程**不做**：堆分配/释放、阻塞等待、加锁、系统调用、COM 重建、跨 FFI 的 panic/unwind。
- 图、buffer、信号绑定、处理器状态、查询表都在控制侧准备；RT 只做有界拷贝与计算。
- **所有权交接**：候选/预备对象在控制侧构造；RT 接受预分配 owner；`RetiredPlan`、`RetiredHostUpdate`、stopped host 等必须回到控制侧再销毁。
- **信号发布**只通过当前操作授权的 `scope(&op)` + `RtHost`；scope 与 block guard 借用独占 host 访问，不能跨线程发送。
- **线程登记**用 `moiren_engine::threads`：父线程 `register_thread`，子线程只调 `ThreadGuard::enter`；新增 spawn 点必须登记。
- **平台边界**：COM 对象留在其 owner 线程；native teardown 必须有界，弃置路径不能无限 join。线程地图见 `docs/platform/windows-audio-threads.md`。
- **验证手段**：`tools/rt-check` 影子测试、`moiren-signal` 的 TLS 分配器测试、Linux CI 的 Miri（Windows 本地 Miri 有 std 分配器误报，见 [verification.md](verification.md)）。

## EngineHost 与更新语义

- `EngineHost` 拥有编译、激活、参数路由与信号自动挂接。客户端只提交 edit/parameter，不自行 compile、publish、restart、retire plan 或 join device worker。
- 参数与图更新是异步的：**请求 → 接受 → 应用** 三段，不能把提交当已生效。graph revision、plan revision、owner generation、resource generation 与 epoch 是彼此独立的身份，不能互相冒充。
- 兼容更新在块边界转移活动 IO 端点并保留 DSP 状态、ramp 与信号绑定；不兼容的 transport 变化重建 owner 并产生新 epoch，旧 epoch 自动化不迁移。
- 准备与回收分 lane；静态准备失败保留当前有效 plan，不进入重试死循环。缺少资源时保留期望项目，不把旧设备阻塞在新选择上。

## 新增 NodeKind（checklist）

`NodeKind` 是闭合枚举，穷尽 match 是编译期安全网；**不要**改成运行时注册表（2026-11-09 审计 E-1 裁决）。新增变体必须更新以下编译强制触点（2026-11-11 核对；树变动后复核）：

1. `moiren-core`：`NodeKind` 与 `initial_ports()`，在 `crates/moiren-core/src/graph/model.rs`。
2. `moiren-engine`：`Binding<S>`、`NodeBindings::bind_*` 与穷尽 `validate` match，在 `crates/moiren-engine/src/compiler/bindings.rs`。
3. `moiren-engine`：`NodeKind` lowering match，在 `crates/moiren-engine/src/compiler/plan.rs`。
4. `moiren-engine`：并行的 `NodeConfig` 枚举，在 `crates/moiren-engine/src/host/project.rs`。
5. `moiren-app`：`GraphNodeKind` 与双向 `From` 实现，在 `crates/moiren-app/src/project/graph.rs`。

以上只是**编译下限**。可用的节点还需要非编译强制接线：engine 的 processor/kernel 与 processor 测试；app 的节点 catalog、service preparation/builtin、export/persistence、locale 条目；需要观测时再加 render/hot-layer。Plugin 与 Recorder 是完整接线的参考——只加 enum 分支不算功能。

## 扩展点约定

- 新 processor：配置类型在 engine（必要时 core 提供逻辑配置），RT 路径只读已准备数据；参数更新走实时参数通道，不重建图。
- 新平台 transport：实现 engine 的 backend 契约（prepare/attach/activate/stop_and_join），不得把平台句柄类型带进 engine 公共 API。
- 新 feature gate：只门控"非默认消费面"；在 `Cargo.toml` 注释用途，示例用 `required-features`，CI 显式 lint 门控面。
- 新 crate：先读 [repository.md](repository.md) 的"新增 crate / 目录 / feature gate"清单，CI 映射与文档必须同 PR 更新。

## 文档优先级

架构意图以 `docs/designs/NN-*.md` 为准；实现与设计冲突时，先确认设计是否已被后续裁决取代，再改代码或补设计修订注记。历史审计与旧计划只作为证据，不作为当前架构承诺。
