# First Graph Compiler

用户提供的进度评估和随后对核对、实施、更新文档三项工作的 `All` 回复作为本轮范围依据。基线为 `main@adc725d`，本轮在 `feat/graph-compiler` 工作。

## 决策

采用保守、无槽位复用的参考编译器。另一种方案是立即分析 last-use 并使用原位处理，内存更省，但增加 fan-out 覆写风险；第三种方案是只生成节点排序，仍要求调用方装配边和 IO，不能满足本轮验收。

`moiren-engine::compiler` 在非 RT 侧消费 LogicalGraph 的只读快照和 NodeBindings，生成单 slab、PreparedIo、内置及边处理器、参数表、ExecutionPlan 和可运行 Engine。core 保持唯一可编辑拓扑，不引入第二张持久 Graph。Source/Sink 通过调用方提供的软件 source/sink 或已准备好的 IO processor 绑定；Gain 默认 1、Pan 默认 0，可显式设置初始值。所有绑定必须属于图中正确类型的节点，重复绑定直接拒绝。

每个节点输出、每个 Edge 的发送结果和每个未连接输入各有独立槽位。信号不会被原位覆写，不进行 slot liveness 优化。未连接输入读取初始化后永不写入的专用静音槽位；空 Bus 由现有 Sum 内核输出静音。每条边在其目标节点前执行独立 Send 内核，重复来源仍按独立输入累计。

Send 支持 PostFader、有限非负线性 gain、stereo balance pan、mute；给 ControlPort 返回 EdgeId 对应的三个参数键。静音明确写零；gain 不沿用 Gain 节点的 16 倍上限。非 stereo 非零 pan 返回携带 EdgeId 的错误，后续参数表也限制该边 pan 为 0。PreFader 暂返回携带 EdgeId 的 UnsupportedTap：当前节点没有文档所需的独立 channel strip 与两个取样点，不能把 PreFader 悄悄视作 PostFader。该边界已向用户提出可选语义澄清；若用户指定其他规则，更新此契约后再实施对应部分。

返回 CompiledGraph 包含 Engine、ControlPort、NodeId→ProcessorId、EdgeId→参数键以及操作/槽位/音频字节统计。参数键只属于当前 plan revision，不宣称跨热切换稳定。编译错误发生在非 RT 侧，不能改动调用方图，也不会发布半份计划。音频 slab 继续由 BufferArena 检查字节预算和尺寸，预算不包含 processor/queue/metadata。

## 验收

- logical_graph 示例不再自行构造 BufferSlotId、PreparedIo 或 OpSpec，输出与原先 8 帧声像 ramp 一致。
- 覆盖 Source/Sink/Gain/Bus/Pan、fan-out、重复 Bus 来源、空图/空 Bus/未连接输入、可变 block、两种精度及错误绑定/格式/预算。
- 用确定性随机 DAG 的独立样本求值与实际输出比较；通过线程局部 allocator 验证编译所得完整链路 render 无分配和释放。
- workspace 测试、严格 Clippy、fmt、离线例子通过；CI 增加 Windows crate 的编译和无硬件测试，不运行设备探针。
- 更新 PRD 和 Windows 接入顺序：Compiler → Shared Render → Capture/Loopback + 最小 Clock Bridge → Plan Swap → GUI → 多输出与稳定性；Takeover 为独立并行 Gate。

不将本轮 Compiler 交付记为真实音频、优化 BufferPlanner、Pre/Post channel strip 或运行中换图验收通过。
