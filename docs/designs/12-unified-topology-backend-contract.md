# 三视图共用的后端读取与编辑契约

> 状态：历史设计（「Slint 尚未迁移」的表述已过时；当前为 gpui，见 FRONTEND.md）。

日期：2026-10-14。对应[三视图评估](11-unified-topology-views-evaluation.md)第 5.4、7、8、9.4 节；本轮只实现 core/engine 基础接口，桌面应用与 Slint 尚未迁移。

## 一次读取项目和运行状态

`EngineObserver::read()` 返回 `ProjectSnapshot`，在一个锁下取得期望项目、活动项目和匹配的 HostSnapshot。`desired`、`active` 是只读 `Arc<EngineProject>`；未改变的项目复用 Arc，实时参数改变也会更新期望项目副本，不推进 GraphRevision。

```rust
use moiren_core::graph::{GraphError, GraphIndex, NodeId};
use moiren_engine::host::EngineObserver;

fn inspect(observer: &EngineObserver, node: NodeId) -> Result<(), GraphError> {
    let view = observer.read();
    let index = GraphIndex::new(&view.desired.graph);
    for edge in index.outgoing(node)? {
        println!("{:?}: {:?} -> {:?}", edge.id(), edge.src_port(), edge.dst_port());
    }
    println!("desired {}, active {:?}", view.state.desired_revision, view.state.active_revision);
    for parameter in view.desired.parameter_values() {
        println!("{:?}: {:?}", parameter.target, parameter.value);
    }
    Ok(())
}
```

保留的 `snapshot()` 和 `project()` 与旧调用兼容，但分开调用可能读到不同发布。需要版本和内容对应时使用 `read()`。

活动项目仅在 owner/计划确认后出现，准备中的更新不会提前替换它。Closed 的最终快照清除活动项目与活动版本，仍保留期望项目用于诊断。保留 Arc 不持有 IO、运行实例或宿主生命周期。

`active` 描述确认计划准备时的配置，不能作为实际实时参数读数。ParameterReceipt 仍是应用确认的依据；仅观察图版本相等或 EditOutcome::Activated 不能推断全部参数已应用。

## 返回创建对象的身份

`edit` 保留 unit-result 调用方式。`edit_with_result` 允许事务返回 NodeId、PortId、EdgeId 或调用方定义的 Send 结果；`EditReceipt<T>` 的 `outcome()`、`wait()` 保持原生命周期语义。

编辑返回值先与候选一起暂存，只有通过整个项目验证、宿主容量检查与版本分配才发布。`result()` 对 Clone 结果返回副本；Queued、Rejected、Conflict 没有结果。结果在 Accepted 后可读，之后即使准备被 Superseded 或宿主关闭仍保留；它证明项目接受，不证明音频激活。结果的分配和复制都在非 RT 域。

```rust
use moiren_core::graph::{EdgeId, NodeId, NodeKind, PortId, SendParams, edit::connect_to_new_bus_input};
use moiren_engine::host::{EditReceipt, EngineControl, EngineObserver, HostError, NodeConfig};

fn add_bus_route(
    control: &EngineControl,
    observer: &EngineObserver,
    source: PortId,
) -> Result<EditReceipt<(NodeId, PortId, EdgeId)>, HostError> {
    let basis = observer.read().edit_basis();
    control.edit_if_current(basis, move |project| {
        let channels = project.graph.get_port(source)?.channels();
        let bus = project.graph.create_node(NodeKind::Bus, channels)?;
        project.nodes.insert(bus, NodeConfig::Bus);
        let edge = connect_to_new_bus_input(&mut project.graph, source, bus, SendParams::default())?;
        let input = project.graph.get_edge(edge)?.dst_port();
        Ok((bus, input, edge))
    })
}
```

节点配置和拓扑在同一个事务里验证。非法回调、panic 或超出宿主容量不会泄露候选对象身份。直接操作 EngineProject 时，使用 `edit_with_result` 返回经验证的值；原 `edit` 保持单位返回值签名与只返回错误时的类型推断。

成功项目先一致发布，再发布 Accepted/Activated；基于凭据唤醒的读者可以读取已接受的项目。Accepted 不保证读者恰好读到该中间版本，更晚的合法编辑仍可先行发布。

## 冲突处理

`EditBasis` 包含进程内唯一 HostId 和期望 GraphRevision。`edit_if_current` 在协调器执行时比较它们，冲突返回 `EditOutcome::Conflict { expected, actual }`，完全不运行编辑回调。同一批命令里的两个相同结构版本编辑也会依次验证；前一个改变结构后，后一个冲突。

HostId 区分不同宿主中的相同数字版本，不是持久文件里的 project identity。应用保存和跨文件选择仍需要自己的项目身份。

收到 Conflict 后重新读取项目，重建编辑预览，再提交明确的新操作。不要把整张旧图作为重试内容自动覆盖。

版本检查针对结构、静态节点配置与 IO/processing 配置；实时参数不推进 GraphRevision。因此对单条边占用、参数值或其他更细条件有要求时，在事务回调中检查当前候选。`edit_with_result` 没有版本检查，适合已明确采用当前项目语义的操作。

批量结构编辑可以在一个回调内执行，任一步失败即回滚整个候选。实时参数仍是独立 ParameterReceipt，不承诺批次同帧原子应用。

## 邻接查询和连接预检

`GraphIndex::new(&LogicalGraph)` 在控制侧构造只读稀疏索引；Rust 借用保证引用期间图不能被修改。存储按节点、输入端口与边增长，不创建稠密矩阵。

| 方法 | 语义 |
| --- | --- |
| `incoming(node)` | 节点全部直接入边，保留图中 Edge 顺序 |
| `outgoing(node)` | 节点全部直接出边，保留图中 Edge 顺序 |
| `between(source, destination)` | 节点对的全部直接边；不计算可达关系 |
| `input_edge(port)` | 完整图的输入占用；未连接输入返回 None |

有效孤立节点返回空边列表；未知节点返回 NodeNotFound。输出端口作为输入查询返回 PortDirectionMismatch，未知端口返回 PortNotFound。同源到 Bus 的多个输入保持独立 EdgeId，不聚合发送参数。

Matrix 的行列筛选可以在调用方选择轴后使用这些查询，但不能先删去隐藏节点再校验。索引中的占用不受页面筛选影响。更新快照后重建索引；保留旧索引只代表保留旧的只读视图。

`LogicalGraph::validate_connection(output, input, params)` 与 `connect` 使用同一内部校验：对象存在、方向、声道数量、发送参数、输入占用、完整图 DAG 与 Edge ID 容量。它不修改拓扑、不预留输入、不消耗 ID；实际事务仍须重新调用 `connect`。这只是逻辑合法性检查，不检查设备可用性、编译器功能、PreFader 支持或全部宿主资源预算。

## 期望参数口径

公开 `EngineProject::parameter_values()`，复用宿主既有的取值规则：先读取节点配置与 Edge 默认值，再按稳定 ParameterTarget 使用显式期望参数覆盖。返回有效期望值，不是运行中瞬时值，也不发送控制命令。

这能避免未来 Graph 标签、Mixer 推子、Matrix 发送值分别读取不同来源。函数会分配，限定在非 RT 的读取、投影、准备和控制逻辑中使用。

## 验证与剩余集成

针对性测试覆盖一致发布、保留活动项目、并发结构修改、Arc 复用、实时及按帧参数更新、真实创建身份、拒绝结果、同批冲突、跨宿主冲突、Superseded 结果保留、容量/关闭、并行 Bus 路由、直接与间接连接、隐藏来源的输入占用以及完整图环路校验。

本轮在独立 worktree 与独立 Cargo target 中验证，包含同期的 `942ee2c` 压力验证提交，保留主工作区的未提交 UI 改动：

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked --workspace` | 666 passed，0 failed，8 ignored，含文档测试；托管宿主测试 38 项 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `git diff --check` | 通过 |
| `cargo run --locked -p moiren-engine --example managed_graph` | 确认自动更新至活动图/计划版本 2，宿主释放后自动关闭回收 |
| 独立只读审查与兼容性修复复核 | 无剩余实质问题；error-only unit edit 的推断回归已覆盖 |

这些是本地软件验证；本轮没有另行执行硬件验收或远程 CI。

应用层的统一命令历史、对象元数据、Undo/Redo 身份恢复、活动/期望差异投影和三视图筛选仍需后续接入。前端在设计完成后再迁移；本轮不新增节点级 mute/solo、隐式 channel strip 或 Rack，也不扩大当前音频设备能力。
