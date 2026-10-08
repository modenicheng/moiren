# Graph Compiler：无槽位复用参考实现

日期：2026-10-08。`moiren_engine::compiler::compile()` 在控制侧将只读 `LogicalGraph`、外部 IO 绑定和初始参数准备为可运行引擎。该实现是后续优化 BufferPlanner 的正确性参照，尚未实现槽位复用、原位选择、Pre/Post channel strip 或运行中换图。

## 编译与绑定

```text
LogicalGraph + NodeBindings + CompileConfig
    → validate / stable topology
    → per-edge Send lowering / separate slot assignment
    → BufferArena / PreparedIo / RtResources / parameter_channel
    → ExecutionPlan / Engine + ControlPort + CompiledBindings
```

`NodeBindings<S>` 使用逻辑 NodeId。Source/Sink 必须显式绑定，即使节点孤立；`bind_source` / `bind_sink` 接受 `RtAudioSource` / `RtAudioSink`，`bind_io` 接受已经准备好的 Source/Sink processor，例如带 Boundary telemetry 的 InputNode/OutputNode。role、声道和端口由编译器与现有 prepare 验证。绑定不匹配、重复绑定、已删除节点均返回错误。

Gain/Pan 自动创建内置 processor，初始值默认为 1/0；`bind_gain` / `bind_pan` 可覆盖。Gain 节点沿用现有 `[0, 16]` 参数域，Pan 为 `[-1, 1]`。Bus 不需要绑定。Source/Sink 的 backend 必须已经适配 Processing SR；Compiler 不打开设备、不实现 SRC，也不提供参数化自定义 IO processor 的参数 schema 注入。

`CompileConfig` 包含 EngineConfig、音频字节预算、plan revision、timeline epoch、控制队列容量和调度 horizon。返回的 `CompiledGraph` 包含 engine、control、逻辑绑定和统计。所有构造、编译、失败清理和停止后析构均在非 RT 侧完成；编译不会改变 LogicalGraph，也不会将 Engine 自动发布给运行中的音频线程。

## 信号与内存

每个节点输出和每条 Edge 发送结果分配独立槽位。每个未连接输入读取专用、初始化为零且永不写入的槽位；空 Bus 使用既有 Sum 与输出清零契约。槽位存在于同一个 planar slab，全部使用 `max_block_frames` 容量。所有访问通过既有 `prepare_io` 验证，没有新增 unsafe。

目标节点的输入先执行对应 Send，再执行节点；重复来源连接同一 Bus 的两个输入仍分别求和。fan-out 各路读相同的节点输出，发送处理写各自槽位。参考实现为包括默认参数在内的每条 Edge 执行 Send，增加 copy 和内存开销；后续优化可消除恒等发送、融合 Sum、计算 last-use 和选择原位，但须与参考结果交叉验证。

`CompileStats.audio_bytes` 与 `audio_byte_budget` 仅覆盖音频 slab，processor、metadata、queues 和候选计划的额外内存未计入。5 节点 / 4 边示例使用 9 个操作、8 个槽位，f64 stereo / 8 frames 时音频 slab 为 1024 字节。

## Edge 参数与能力边界

每条 PostFader Edge 生成内部 Send processor，不增加第二个可编辑 Graph 节点。gain 为有限非负线性幅度，允许大于 16；pan 使用现有 stereo balance 系数；mute 写出零样本。它们分别映射为 Float/Float/Bool 参数，通过原有 SPSC 和时间线更新，gain/pan 支持逐样本 ramp。

`compiled.bindings.edge(edge_id)` 返回 gain/pan/mute 的 ParameterKey，`node(node_id)` 返回 ProcessorId，可配合 Gain::LEVEL / Pan::POSITION。这些映射是当前计划的局部身份，下一次编译必须读取新映射并使用对应 revision/epoch。

- **PreFader：**返回 `CompileError::UnsupportedTap { edge }`。旧 Graph 设计中的取样点位于同一节点 channel strip 的 Gain/Pan 之前；当前节点尚无该 strip，不能将独立 Gain 节点输入擅自定义为所有节点的 PreFader。
- **非 stereo pan：**非零初始值返回 `UnsupportedPan { edge, channels }`，其运行时 pan 参数域固定为 0。mono / multichannel 的 gain/mute 仍受支持，无隐式 upmix/downmix。
- **逻辑编辑与 RT 更新：**`graph.set_send_params()` 修改逻辑快照；它不会自动向现有 Engine 下发消息。Control owner 同步 desired state 与 ParameterRequest；拓扑编辑后重新编译，当前仍需停机后更换引擎。

有限 gain 并不保证样本始终有限；参考实现不裁剪、不提供 limiter，不新增独立的安全音量策略。

## 验证

```sh
cargo run --locked -p moiren-engine --example logical_graph
cargo test --locked -p moiren-engine --test compiler
cargo test --locked -p moiren-engine --test runtime
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

覆盖原示例的跨 block ramp、Edge ramp/mute、重复 Bus 来源、fan-out、多声道、空图/空 Bus/未连接输入、修改拓扑后重新编译、错误绑定/预算/参数域。24 张确定性随机 DAG 分别以 f32/f64 运行四种 block 长度，并与独立样本求值比较。完整编译链路的两种精度通过线程局部 allocator 计数验证 render 分配与释放均为 0；结论限于被测试的内置与软件桥路径。
