# Graph Compiler：优化 lowering 与参考实现

日期：2026-10-08；2026-10-09 补充 Compressor 和 Plan 发布；2026-10-25 增加 Bus 融合、last-use、原位处理和 slot 复用。`moiren_engine::compiler::compile()` 在控制侧将只读 `LogicalGraph`、外部 IO 绑定和初始参数准备为可运行引擎，默认执行优化 lowering。`compile_reference()` 保留每条 Edge 独立执行、每个信号独立存储的路径，用于结果比较与诊断。编译后的候选可通过 [Plan 发布接口](06-compressor-plan-swap.md) 发布；编译器本身不访问运行中的音频线程。

## 编译与绑定

```text
LogicalGraph + NodeBindings + CompileConfig
    → validate / stable topology
    → logical reads/writes / Send or fused Bus lowering
    → last-use / in-place / physical slot assignment
    → BufferArena / PreparedIo / RtResources / parameter_channel
    → ExecutionPlan / Engine + ControlPort + CompiledBindings
```

`NodeBindings<S>` 使用逻辑 NodeId。Source/Sink 必须显式绑定，即使节点孤立；`bind_source` / `bind_sink` 接受 `RtAudioSource` / `RtAudioSink`，`bind_io` 接受已经准备好的 Source/Sink processor，例如带 Boundary telemetry 的 InputNode/OutputNode。role、声道和端口由编译器与现有 prepare 验证。绑定不匹配、重复绑定、已删除节点均返回错误。

Gain/Pan 自动创建内置 processor，初始值默认为 1/0；`bind_gain` / `bind_pan` 可覆盖。Gain 节点沿用现有 `[0, 16]` 参数域，Pan 为 `[-1, 1]`。Compressor 默认使用 `CompressorSettings::default()`，`bind_compressor` 可覆盖全部 10 个初始值。Bus 不需要绑定。Source/Sink 的 backend 必须已经适配 Processing SR；Compiler 不打开设备、不实现 SRC，也不提供参数化自定义 IO processor 的参数 schema 注入。

`CompileConfig` 包含 EngineConfig、音频字节预算、plan revision、timeline epoch、控制队列容量和调度 horizon。返回的 `CompiledGraph` 包含 engine、control、逻辑绑定和统计。所有构造、编译、失败清理和停止后析构均在非 RT 侧完成；编译不会改变 LogicalGraph，也不会将 Engine 自动发布给运行中的音频线程。

## 信号与内存

lowering 首先记录每个操作的显式读取和写入，再由 `compiler/storage.rs` 规划物理 slot。所有 slot 位于同一个 planar slab，容量为 `max_block_frames`；最终访问仍经 `prepare_io` 检查，无新增 unsafe。

默认优化包括：

- Bus 的各路 Send gain/pan/mute 与求和合入一个 mix 操作，不再分配每条 Send 的音频中间结果。未连接 Bus 输入直接省略，空 Bus 和全部 mute 的 Bus 明确写出零。
- 若 Bus 输入绑定 observer，整个 Bus 回退为独立 Send + observer + Sum，让 observer 仍能读取 post-Send 的逐输入信号。
- 计算每个逻辑信号的最后读取位置。Gain/Pan 仅在输入的当前操作是最后使用者时原位执行；后续 fan-out、PreFader 和 observer 读取都延长相应信号的生命周期。
- 生命周期不重叠且声道数、容量相同的逻辑信号共享物理 slot。独立读写操作的复用要求旧生命周期结束位置严格小于新定义位置；同一操作内的原位别名只能来自前述证明。
- 未连接固定输入使用初始化为零且永不写入、永不复用的专用静音 slot；静音在跨 block 和参数分段时保持不变。

融合混音保留每条 Send 的系数顺序和精度：先计算 gain × balance，再缩放样本；每个 Send 结果及每次 Bus 加法都舍入到处理精度。各条逻辑 Edge 仍独立参与混音，即使共享来源。参数绑定在每个事件分段开始时读取一次，render 只更新预分配的临时字段。内核以 32 帧为单位使用有界栈上累加器，输出样本只写回一次；stereo 的 gain/pan ramp 每帧只插值一次，再应用到左右声道。已完成 ramp 的常量系数移至样本循环外。这些临时数组不引入跨调用排队或额外音频延迟。

`compile_reference()` 对包括默认参数在内的每条 Edge 执行 Send，保留独立输出 slot，并跳过原位和生命周期优化。顶层 Gain/Pan 链目前仍分别执行；原位仅减少存储，并不等于将多个处理器合并为一轮计算。

`CompileStats` 提供 operation/slot 数量、音频 slab 字节数，以及 fused operation、mix operation 和 in-place operation 数量。`audio_bytes` 与 `audio_byte_budget` 仅覆盖音频 slab，processor、参数、元数据、队列、DSP 内部历史和候选计划内存不计入。

5 节点 / 4 边 `logical_graph` 示例的 reference 为 9 个操作、8 个 slot；默认优化后为 7 个操作、3 个 slot，其中 1 个融合 mix、1 个原位操作。f64 stereo / 8 frames 的 slab 从 1024 字节减至 384 字节。

这些操作原本就在同一次 render 中同步完成，中间 slot 不会每经过一个节点额外延迟一个音频 block。这里减少的是循环、调度和内存流量；设备 period、异步桥队列和有状态 DSP 的算法延迟需分别测量。

## Edge 参数与能力边界

每条 Edge 生成内部 Send processor，不增加第二个可编辑 Graph 节点。gain 为有限非负线性幅度，允许大于 16；pan 使用现有 stereo balance 系数；mute 写出零样本。它们分别映射为 Float/Float/Bool 参数，通过原有 SPSC 和时间线更新，gain/pan 支持逐样本 ramp。

`compiled.bindings.edge(edge_id)` 返回 gain/pan/mute 的 ParameterKey，`node(node_id)` 返回 ProcessorId，可配合 Gain::LEVEL / Pan::POSITION。这些映射是当前计划的局部身份，下一次编译必须读取新映射并使用对应 revision/epoch。

- **PreFader：**Gain 节点默认读取自身输入；绑定 channel strip 的节点读取 strip 增益之前、已应用 mute/channel mode 的信号。其余节点返回 `CompileError::UnsupportedTap { edge }`。融合与存储规划都保留这些显式取样点。
- **非 stereo pan：**非零初始值返回 `UnsupportedPan { edge, channels }`，其运行时 pan 参数域固定为 0。mono / multichannel 的 gain/mute 仍受支持，无隐式 upmix/downmix。
- **逻辑编辑与 RT 更新：**`graph.set_send_params()` 修改逻辑快照；它不会自动向现有 Engine 下发消息。Control owner 同步 desired state 与 ParameterRequest；拓扑编辑后重新编译，再通过 `PreparedPlan` 和 `PlanControlPort::publish` 发布。复用节点时用两份 `CompiledBindings` 从相同 NodeId 得到旧 / 新 ProcessorId，不能假设局部编号相同。需要保留 Edge gain/pan/mute 时，调用方也应显式复用对应 Send processor；映射可由 edge keys 的 `gain.processor` 取得。

融合 Send 保留独立的逻辑单输入/单输出 schema，因此 Bus 增加其他输入不会使该 Send 的参数复用失效。Bus 的复用 schema 保留包含未连接输入的完整逻辑端口列表，observer 回退不会改变该身份。Gain/Pan 的复用 schema 同样独立于原位或独立存储模式，last-use 改变后仍可保留在途 ramp。

有限 gain 并不保证样本始终有限；编译器不会隐式插入限幅，需要时显式使用 Limiter。

## 验证

```sh
cargo run --locked -p moiren-engine --example logical_graph
cargo run --locked --release -p moiren-engine --example compiler_optimization
cargo test --locked -p moiren-engine --test compiler
cargo test --locked -p moiren-engine --test runtime
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

覆盖原示例的跨 block ramp、Edge ramp/mute、重复 Bus 来源、fan-out、多声道、空图/空 Bus/未连接输入、修改拓扑后重新编译、错误绑定/预算/参数域。24 张确定性随机 DAG 分别通过 reference/optimized 路径，以 f32/f64 运行四种 block 长度，并与独立样本求值比较。另有逐位等价测试覆盖动态 Send/节点参数 ramp、mute、稀疏输入、PreFader、fan-out 与重复来源，以及溢出顺序和 f32 逐次舍入。计划切换测试覆盖 Bus 增加输入后的 Send ramp，以及 PreFader 使 Gain 从原位切换为独立存储时的参数复用。完整编译链路的两种精度通过线程局部 allocator 计数验证 render 分配与释放均为 0；结论限于被测试的内置与软件桥路径。

`compiler_optimization` 在 release 模式比较编译后纯 render 的耗时，交替运行两种路径，报告五轮各 1000 次调用的中位数。追加 `-- --ramps` 可测量活跃 gain/pan ramp。该测量排除编译、设备和桥传输，不代表端到端音频延迟。首个可复现的 portable 记录见 [2026-10-25 Compiler 优化基线](../experiments/portable/2026-10-25-compiler-optimization.md)；它用于后续回归比较，不设跨机器纳秒硬阈值。
