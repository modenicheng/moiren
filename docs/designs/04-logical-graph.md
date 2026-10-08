# LogicalGraph 与 Bus / Pan

本轮实现控制侧拓扑模型及 engine 内置 processor。`LogicalGraph` 位于 core，不依赖 engine；音频执行继续使用手工准备的 `ExecutionPlan`，自动 lowering、Send DSP、Pre/Post tap、slot liveness 与在线换图属于后续 Graph Compiler / Control 工作。

## 拓扑契约

每个端口承载完整多声道流。`Source` 为 0 入 / 1 出，`Sink` 为 1 入 / 0 出，`Gain` 为 1 入 / 1 出；它们接受正数声道数。`Bus` 从 0 入 / 1 出开始，只允许动态增删输入，所有输入与输出声道相同。`Pan` 当前为 stereo balance，固定 2 声道、1 入 / 1 出。

节点、端口与边只能经图的编辑 API 修改；查询返回只读对象。`create_node(kind, channels)` 一次生成固定布局，创建失败时不消耗部分 ID。Logical IDs 是图内身份，与 `ProcessorId`、engine 的 `u16` 本地端口序号及 buffer slot 分开；复制整张图保持身份，用作独立编辑快照。节点的设备配置和持久项目文件编解码尚未接入该拓扑模型。

`connect(output, input, SendParams)` 只接受已有端口；检查方向、相同声道、输入占用、参数数值和 DAG。一个输入最多有一条入边，输出可任意扇出；同一源可连接同一 Bus 的多个独立输入，执行时对应多次求和。声道不匹配直接拒绝，没有隐式 upmix/downmix。

`topological_order()` 使用迭代 Kahn 算法，每次选择最早创建的可运行节点，计入孤立节点并正确计数节点之间的多条边。环路检测也用迭代遍历。`validate()` 在 Compiler 边界重新检查身份、布局、端口、参数和 DAG。遍历和编辑会分配内存，不能在 RT callback 中调用。

## 编辑与身份

```rust
use moiren_core::graph::{edit::connect_to_new_bus_input, *};

let mut graph = LogicalGraph::new();
let source = graph.create_node(NodeKind::Source, 2)?;
let bus = graph.create_node(NodeKind::Bus, 2)?;
let output = graph.get_node(source)?.outputs()[0].id();

// 底层操作：明确新增端口，再连接。
let input = graph.add_input_port(bus, 2)?;
let edge = graph.connect(output, input, SendParams::default())?;

// 编辑 helper：连接失败移除新端口，不留下孤立端口。
let second_edge = connect_to_new_bus_input(&mut graph, output, bus, SendParams::default())?;
```

`disconnect(edge)` 保留 Bus 输入，允许重新连接；`remove_input_port(bus, port)` 只移除未连接的 Bus 输入；`remove_node(node)` 同时移除关联边。删除后的 ID 不再使用，组合编辑失败已消耗的端口 ID 也不回收。此 helper 保证拓扑回滚，尚非完整 Undo/Redo 命令系统。Bus 输入上限为 65536，以适配当前 engine 的 `u16` 端口索引。

一个 Edge 直接保存 `SendParams`，不另外维护 Send ID 或节点 send 列表。默认 gain 为线性 1，pan 为 0，mute 为 false，tap 为 PostFader。gain 必须有限且非负，pan 必须有限并在 `[-1, 1]`；`set_send_params()` 不改变拓扑。该方法编辑逻辑意图，当前不会自动下发 RT 参数；后续 Control/Compiler 负责参数绑定与更新。首版连接仅比较声道数量，详细 speaker layout、SRC 与 conversion policy 尚未实现。

## Engine processor

`moiren_engine::processor::{Bus, Pan}` 均实现现有 `RtProcessor<S>`，支持 f32 / f64，报告零帧算法延迟。

`Bus` 以公开别名复用 `Sum`，没有额外 wrapper 或第二份混音代码。prepare 接受 N 路独立只读输入和 output 0，要求声道相同、Separate IO；N 可以为 0，相同 slot 可绑定多个只读输入。engine 在每个处理 segment 前清空独立输出，随后逐声道累计；空 Bus 输出静音、不自动归一化或裁剪。动态增删逻辑端口只发生在控制侧，运行时 IO 已固定。

`Pan` 当前是立体声线性 balance，不交叉混合左右声道。position 为 `[-1, 1]`，-1 最左、0 居中、1 最右，系数为：

```text
left_gain  = 1 - max(position, 0)
right_gain = 1 + min(position, 0)
```

居中时两个声道均保持 unity。`Pan::parameter(processor_id, initial)` 声明 POSITION 参数，可通过现有 `ParameterRequest` 指定生效帧和 ramp。prepare 检查 input/output 0 均为 stereo，并验证参数类型和允许范围；避免初始值合法但 schema 允许后续越界的情况。支持 Separate 与 InPlace IO，按 segment 的 sample offset 计算 ramp，不修改窗口之外的样本。既有 Gain 实现与参数契约保持原样。

原位 Pan 仍要求 buffer plan 保证最后消费者规则：需要 pre-pan 信号的 fan-out 必须先执行，或使用独立输出。本轮低级 executor 不证明整图 liveness。

## 运行与验证

```sh
cargo run --locked -p moiren-engine --example logical_graph
cargo test --locked -p moiren-core -p moiren-engine -p moiren-app
cargo clippy --locked -p moiren-core -p moiren-engine -p moiren-app --all-targets -- -D warnings
cargo fmt -p moiren-core -p moiren-engine -p moiren-app -- --check
```

例子建立两个 Source → Bus → Pan → Sink 的逻辑图，校验稳定排序，然后为此已知拓扑手工绑定 engine processors、三个 slots 和软件输出桥。它检查实际 stereo 样本及跨 block Pan ramp，不打开设备。

测试覆盖固定与动态布局、连接约束、重复源、环路、深链、删除/重连、ID 溢出与编辑回滚；DSP 覆盖空 Bus、重复只读输入、声道一致性、Pan 两种 IO 模式与两种精度、越界参数、有效窗口和跨 block ramp。完整链路通过线程局部 allocator 计数验证 render 无分配与释放，此结论仅适用于被测试的内置路径。
