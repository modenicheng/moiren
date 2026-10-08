# 基础 Input / Output IO 节点

> 日期：2026-10-08。依据 [Graph 设计](01-audio-graph.md)、[Engine 设计](02-engine-design.md)及[实时基础实施计划](../plans/2026-10-08-runtime-foundation.md)。

## 范围与方案

本轮交付 engine 侧可执行的基础 IO 节点：控制侧配置、实时端口契约、有界音频桥、状态上报，以及第一版 `moiren-app` 应用入口。现有 `moiren-core` 和 `LogicalGraph` 草稿保持原样；本轮通过手工准备的 `ExecutionPlan` 调度节点。

三种实现方向：继续使用无配置的 Source/Sink adapter，改动最少但无法表达设备意图或观察故障；把 Windows 设备直接放进节点，短期可开设备但混合设备生命周期与 Graph 执行；使用纯数据配置与独立 streaming backend，符合现有责任边界，本轮选择第三种。

`moiren-engine::node` 保存 Input/Output 配置：多声道数量、物理/软件目标、应用 executable selector 与 PassiveTap/RoutedInput 意图，以及 Shared/Exclusive、期望 hardware SR、period、Master/Follower。PinnedEndpoint 保存 opaque endpoint ID 与可选 stable ID；FollowDefault 保存角色。配置不持有 COM、buffer 或 PID，不执行设备绑定。设置是期望值，backend 必须协商和验证；RoutedInput 不表示已完成应用输出接管。以后 Graph compiler 可将逻辑节点降为这些 prepared runtime，不要求 core 依赖 engine。

## 运行契约

`InputNode<S, T: RtAudioSource<S>>` 只有 output port 0，`OutputNode<S, T: RtAudioSink<S>>` 只有 input port 0；一个端口承载完整多声道流。prepare 检查配置与 backend 声道一致，拒绝错误端口、额外端口与原位 binding。运行时不访问配置字符串或重新查询声道。Output 只读，可与 observer 或其他 output 共用同一个信号 slot。

Source 返回的 `transferred_frames` 是有效音频前缀，其余尾部强制清零。超出请求帧数的报告归为无效，整个输入 block 清零。Sink 的短写丢弃未接受尾部，禁止阻塞重试；无效报告只能诊断，不能撤销 backend 已执行的写入。短读/短写至少计一次 XRUN；backend 的 `xruns` 表示本次调用的增量。时间 epoch 改变或 segment 不连续也记 discontinuity。

每次调用向独立 SPSC 发布带 epoch/start/end 的快照，包含规范化报告、缺失帧数、无效报告及累计传输、shortfall、XRUN、discontinuity、丢诊断数。满队列丢新快照；累计值保留，下一次成功上报可观察丢失数。reader 只排空读取开始时的队列长度，不能追逐 producer。快照与音频不互相施加背压。

## 音频桥

`audio_bridge<S>(channels, capacity_frames, byte_budget)` 在非 RT 阶段预分配 SPSC sample ring，返回独占 `AudioWriter<S>` / `AudioReader<S>`。写端同时支持 planar Graph block 与 interleaved worker slice，读端同样支持两种布局。实现满足 `RtAudioSink` / `RtAudioSource`，同一桥可按方向用于输入或输出。

容量和传输以完整 frame 计；使用 chunk 一次发布/提交完整多声道 frame，保持声道对齐。每次调用只使用开始时可用容量，不等待另一端；溢出丢新尾部，欠载补静音，报告接受/读取的帧数。空 worker 请求合法；不是声道整数倍的 slice 在访问 ring 前拒绝。对端退出后，writer 丢弃输入；reader 可排空已缓存数据，随后静音。不暴露 Graph 引用或外部指针。

桥只复制 ProcessingSample，保留浮点 headroom，不提供 PCM 量化、SRC、时间戳配对、adaptive resampling 或设备重启 reset。跨独立硬件时钟使用前，backend 必须实现对应适配；停止/格式改变/epoch 切换时需停止两端并重新准备桥，不能把旧缓存重新用于新设备。音频 byte_budget 只限制 sample storage，不包含 ring bookkeeping 或诊断队列。

worker 侧 write/read 报告由 worker owner 消费；桥不把另一端的 overflow/underrun 报告或设备 packet flags 自动转发到节点。正式设备接入时必须补齐 worker→Control 状态通道及 capture discontinuity 的传播，不能把 sample ring 当作完整的设备边界协议。

## 第一版 app

`moiren-app` 是独立的 headless binary + library。`OfflineApp` 拥有 engine、control、软件输入/输出桥及诊断 reader；`AppConfig` 定义 channels、Processing SR、max block 和 initial gain。应用准备阶段分配一个 slab、两个 bridge、参数和诊断队列；执行阶段按 `max_block_frames` 拆分 interleaved 输入，渲染并读出等长输出。shape 或 timeline 非法时在入队前拒绝，空请求不推进 timeline。

`set_gain` 使用现有 ControlPort，不直接修改 Gain；调用者可通过 `poll_applied` 读取确认，持续修改时应及时消费 Applied 队列。`stop(self)` 在同步处理结束后交回并销毁资源。单个 slab 和每个 audio bridge 分别受 8 MiB sample storage 上限约束；这不是全应用内存限额。

默认运行 `cargo run --locked -p moiren-app`，打印固定 stereo 样本的处理结果与 IO 快照；可用 `--gain 0..16` 改变初始增益。它提供真实 engine 调度和软件 IO 链路；设备选择配置尚不连接 WASAPI，app 不运行设备枚举、硬件播放或 GUI。

## 验证

覆盖配置拒绝、端口与声道拒绝、f32/f64 多声道与 wrap-around、欠载补静音、完整帧溢出、对端退出、错误报告、时间 discontinuity、队列背压和累计值；集成测试覆盖单 slab、fan-out、可变 block 与参数分段。使用线程局部计数 allocator 验证节点和桥的 render 路径无分配/释放。离线示例不打开设备，输出可检查的样本及边界快照。
