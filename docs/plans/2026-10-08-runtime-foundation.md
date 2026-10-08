# Moiren 实时基础实施计划：平铺 Buffer、Safe Processor IO 与参数控制链路

> 日期：2026-10-08  
> 基线：`main@006eb610ff4bf514366485b1c0227c9bc7bdda2c`  
> 本轮产物：可编译执行的 Rust 骨架、测试、离线示例与分阶段验收计划。不是完整音频产品。  
> 分支：`engine/flat-buffer-control-foundation`

## 1. 依据、变更范围与历史方案

依据 [Graph 设计](../designs/01-audio-graph.md)、[Engine Draft v0.3](../designs/02-engine-design.md)、[10 月 7 日实施评估](2026-10-07%20plan.md)、[Windows 接入计划](2026-10-08-windows-integration-plan.md)，以及本轮明确要求：改为一整个平铺音频 buffer，以集中 unsafe 提供易用 safe 接口。本文件负责本轮具体接口与实施状态；不另建一份平行的全项目架构说明。

| 既有讨论或尝试 | 本轮处理 |
| --- | --- |
| `Vec<&mut Slot>`、对外暴露 `&mut Vec<S>` | 不采用。所有权属于 Arena，RT 不得 resize，也不向 Processor 暴露内存拥有者。 |
| 每 Slot 一个 `Box<[S]>` | 本轮替换为一个 `Box<[S]>` 音频 slab，另存只读 SlotMeta。保留 planar 与 slot/view 分离。 |
| PR #1 的 safe 双 slot resolver、Separate Gain | 保留为历史独立尝试，不自动合并或关闭。本轮直接从最新 main 建分支，覆盖 N 路 IO、只读与原位配对。 |
| 对同一内存同时传 `&[S]` 与 `&mut [S]` 表示原位 | 禁止。原位接口只提供一个独占可变视图。 |
| Zig RT / Rust Control 等跨语言候选 | 不在本轮引入；当前骨架使用 Rust。跨语言并非已确认前置条件。 |
| Engine Draft 的 64-byte alignment 建议 | 暂缓。当前 Box 仅保证 `align_of::<S>()`；不能以 stride 或偶然地址宣称 64-byte 对齐。 |
| 参数绑定 UI、直接修改 DSP 对象 | 不采用。统一经 Control 校验、时间调度与 RT 参数表。 |
| 将 Bus / 每条内部路由映射为 Windows 设备 | 不采用。系统 I/O 通过显式边界节点；虚拟 Endpoint 是后续导出机制。 |

继续保持：唯一 LogicalGraph；Node/Port/Edge；Input 最多一条边、Output 支持 fan-out；Rack 为独立节点且内部有序；Bus 显式 fan-in；所有发送平权；Pre/Post-Fader 区分信号版本；一个 Graph RT 执行者、一个 Processing Timeline、可变 block；普通 DSP 使用 planar f32/f64。端口数量不等于音频分配数量。

## 2. 本轮交付与明确未交付项

| 领域 | 本轮可执行内容 | 尚未实现，不能据此验收 |
| --- | --- | --- |
| Buffer | 单音频 slab、检查尺寸与预算、范围别名验证、作用域借用、N 路 IO、原位配对 | 全图 liveness / 自动 slot 分配、64-byte 自定义分配、声道级别名复用 |
| Processor | Safe trait、IO/参数 prepare 验证、Gain、最小 Sum、独立 RtResources | Rack 插件宿主、Send matrix 融合、插件 ABI、PDC |
| 参数 | Float/Int/Bool/Enum、稳定业务键、prepare 绑定、SPSC、时间分段、Float ramp、Accepted/Applied | 多客户端调度服务、事务批次、未来事件取消、自动拖动合并 |
| IPC | 固定上限且版本化的请求/回复编解码，可接任意非 RT Read/Write transport | Named Pipe listener、连接认证/ACL、Hello/能力协商、订阅与重连 |
| 只读节点 | ReadPorts 专用 observer、sample peak/RMS、独立有界 telemetry 队列 | BS.1770/LUFS、true peak、校准、标准测试向量、频谱 worker |
| I/O | Source/Sink streaming trait、adapter、fake source、离线 sink 测试 | 正式 WASAPI/ASIO、capture/render bridge、SRC/drift、录音和虚拟设备 |
| 生命周期 | Schedule 与持久 DSP 实例分离；停止后回交拥有者析构 | 运行中 publish/retire、registry 扩容/删除、参数迁移和跨计划热切换 |

不创建只有 `todo!()` 的后端或图编译模块。后续工作按实际依赖增加实现，而不是先扩一套空目录。

## 3. 线程、进程与数据流

```text
GUI / automation / future external client
          │ same-process command or local IPC
          ▼
Non-RT transport worker: framing / connection / authentication
          ▼
Single Control owner: schema / desired state / validation / scheduling
          │ bounded SPSC parameters                 ▲ applied replies
          ▼                                         │
Graph RT owner: ParameterRuntime + ExecutionPlan + RtResources
          │ scoped ProcessIo + ProcessParameters
          ▼
Source → Processor / Bus / Rack → Sink
   └──────── readonly observer → bounded telemetry → Control → UI
```

进程拆分仍为部署选择：同进程 GUI 可以绕过序列化，直接提交同一种领域命令；独立进程客户端经本地 IPC。两条路径最终进入同一个 Control 校验入口。不能让 UI 或 IPC worker 持有 `&mut Engine`、音频裸指针或处理器可变引用。

`rtrb` 只解决**同一进程内**预分配 SPSC 队列，不是跨进程共享内存协议。Named Pipe/socket 读取、解码、等待和写回复全部留在非 RT 线程。当前离线示例用内存流验证完整协议→队列→DSP→回复链，没有声称已经启动 IPC 服务。

设备 callback、文件 worker 与 Graph 之间使用各自拥有的 Boundary bridge。不能将 Graph slab 直接借给另一个长期运行的线程，也不能通过 IPC 发送该进程的地址。

## 4. Buffer 存储和寻址

```text
BufferArena<S>
 ├─ data: Box<[S]>             one audio allocation, fixed size
 └─ layout: Arc<Layout>        immutable metadata / identity only
      └─ SlotMeta[]: offset, len, channels, stride

slab: [slot0 ch0 | slot0 ch1 | slot1 ch0 | ...]
```

一个 Slot 是物理可复用区域，不是端口、逻辑信号或 ring buffer。信号版本由未来 Compiler 单独建模；不在 AudioBlock 中放 read/write 游标、时钟估计、IPC 句柄或引用计数。

地址公式：`base + slot.offset + channel * stride + window.start`。返回 channel slice 长度为 `window.end - window.start`，不是 capacity。当前 stride 等于 capacity；接口允许以后在 metadata 层增加 padding，但不向 DSP 暴露 padding。

`BufferArena::new(layouts, byte_budget)` 在非 RT 阶段检查正尺寸、乘加溢出、`isize::MAX` 范围、音频字节预算，并初始化全部样本。`byte_budget` 目前只覆盖音频 slab；服务端总预算还必须计入端口 metadata、Processor state、queues 与候选 Plan，不能宣称这是全进程内存限额。

prepare 后不修改布局、不重新分配、不增长。所有者移动不会改变 Box 中样本地址，但不能在活跃借用期间释放或替换 Box。没有 Pin/self-reference，也不缓存跨调用裸指针。若以后采用 64-byte aligned allocator，必须单独验证 Layout、初始化、dealloc 配对和两种 sample 类型；当前无需为 SIMD 预先扩大 unsafe 面积。

## 5. Unsafe 覆盖与安全证明

生产代码的自有 buffer unsafe 集中在 `buffer.rs`：私有 RawWindow 从一次获得的 slab base 生成经过验证的 slot slice。算术检查、布局构建、别名验证、声道切分、DSP、参数编解码、调度逻辑保持 safe Rust。并发队列复用 `rtrb`，不再自行实现另一套 unsafe ring。

这里必须区分三件事：内存安全、音频语义正确、实时预算。内存安全不能依赖插件承诺完整写出、没有 panic 或遵守性能约束；后两项需要独立验证。

| 不变量 | 建立或维护位置 |
| --- | --- |
| 每个引用位于同一个活跃分配内，长度与指针算术不溢出，满足 S 对齐 | Arena 构造检查；RawWindow 只对同一 slab 内 SlotMeta 寻址 |
| 引用指向有效、已初始化的 S | 整个 slab 在 prepare 时以 `S::ZERO` 初始化；不向 safe DSP 暴露未初始化 slice |
| 所有写区域与其他读写区域不重叠，重复只读允许 | `prepare_io` 比较实际 slot 范围，不只比较符号 ID |
| 一次独占借用不能通过重复 get_mut 或重启 iterator 复制 | `&mut self` reborrow；iter_mut 只遍历一次已验证互斥的 bindings |
| 原位 input/output 不产生重叠共享/可变引用 | 一条 InPlace binding 对应一个 `AudioBlockMut`，未配对 sidechain 与 aux 仍独立 |
| 合法访问描述符不能套到另一 Arena 上 | PreparedIo 保留不可伪造的私有 layout Arc 身份；with_io 检查 `Arc::ptr_eq` |
| 引用不逃出处理作用域 | `with_io` 的高阶生命周期闭包；返回类型 R 与该作用域独立；compile-fail 测试 |
| 视图活跃期间不重新借用整个 slab | 每次 with_io 先取 base，之后只访问 disjoint slot slices，不再创建整块 `&mut [S]` |
| 不依赖 Drop 归还借用凭证 | 没有运行时借用计数；forget/unwind 测试；泄漏视图不会允许引用逃逸 |
| 多线程不能并发进入同一个执行器 | Engine 通过独占所有权和 `&mut self` 驱动；不共享可变 slab，不手写 unsafe Send/Sync |

`Arc<Layout>` 只共享 immutable metadata，RT 只比较指针，不 clone/drop Arc。不要把这误读为允许 `Arc<ExecutionPlan>` 内部随意改普通 Box 数据。当前不需要 UnsafeCell 存音频数据；以后并行 DAG 应另立证明，不能只加 unsafe Sync。

本轮借用粒度是整个 Slot。即使一次只处理几帧，构造 slice 仍独占完整 slot 范围；不允许另一个视图同时写同 Slot 的另一个声道或时间片。声道内部通过标准库 `chunks_exact_mut` 切分，无需额外 unsafe。

### 5.1 内存安全不等于 last-use 正确

`PreparedIo` 只证明**本次**引用集合不冲突，并验证来自哪一个 Arena；它不是完整 BufferPlanner 的证明对象。手工 OpSpec 仍可能过早覆写某个逻辑信号，得到错误音频。这不会因预初始化而变成未初始化内存 UB，但必须在接入 LogicalGraph 之前补齐 Compiler liveness。

未来 Planner 的最小顺序：lowering → 稳定拓扑顺序 → 独立 signal value/version → 消费者与 last_use → compatible slot assignment → in-place selection → prepare_io 二次验证。物理 slot 重用与逻辑 signal ID 分开；不得用 SlotId 充当 IPC 中的稳定身份。

同一来源接入 Bus 两次，两个只读端口可以引用同一个 Slot，但每次都必须参与求和。不能为减少输入数随意去重，也不能让其中一个输入原位覆写另一个仍需读取的副本。Pre-Fader observer/send 同样延长原始信号的 last_use。

### 5.2 输出初始化和异常

当前 Executor 在每个 process segment 前清零 Separate outputs，InPlace 保留已有样本。这样 partial-write safe Processor 至多产生静音尾部，不会播出此前逻辑信号的残留。优化为 first-input overwrite 或省略 clear 必须另有完整写覆盖依据，不能依赖普通 safe trait 中一个未经约束的标志去创建未初始化引用。

panic/插件故障策略还未实现。测试中捕获 unwind 只验证内存借用不会失效，不证明 RT 可以无代价恢复。正式 backend 必须在外层明确 fail-stop/静音和资源回收顺序；不能在 callback 中任意 unwind 并析构整套资源后继续假装播放正常。

## 6. Processor IO 与参数接口

本轮已有 API：

```rust
fn process(
    &mut self,
    ctx: &ProcessContext,
    io: ProcessIo<'_, S>,
    params: ProcessParameters<'_>,
);
```

`ProcessContext` 只承载 epoch、时间起点、本次 frames 和 Processing SR。音频 access 与参数 access 分开；Processor 不读取队列，不知道消息传输方式，也不持有 Arena。

`ProcessIo` 分三种：ReadOnly、Separate、InPlace。InPlace 分支仍允许未配对只读 sidechain、额外独立 outputs 和多组 pair；不是把所有端口硬塞成单入单出。

```rust
match io {
    ProcessIo::Separate { inputs, mut outputs } => {
        let source = inputs.get(0).expect("validated input");
        let mut outputs = outputs.iter_mut();
        let (_, mut a) = outputs.next().expect("validated output A");
        let (_, mut b) = outputs.next().expect("validated output B");
        a.channel_mut(0).copy_from_slice(source.channel(0));
        b.channel_mut(0).copy_from_slice(source.channel(0));
    }
    ProcessIo::InPlace { inputs, mut pairs, .. } => {
        let mut main = pairs.get_mut(0, 0).expect("validated main pair");
        // 通过 main 读原值并覆写；不能另外取得同一内存的共享输入。
        // inputs 只含未配对、已证明不与 main 重叠的输入，例如 sidechain。
        let _ = (&inputs, &mut main);
    }
    ProcessIo::ReadOnly { inputs } => { let _ = inputs; }
}
```

上述为接口用法片段；完整可运行示例位于 `crates/moiren-engine/examples/offline.rs`。端口数字属于处理器 schema，与物理 SlotId 无关。prepare 验证数量、声道、模式和必需参数；处理时不查 LogicalGraph 或字符串名称。当前端口/参数 getter 在短小冻结表中查局部数字 ID；若 profiling 表明有开销，再生成更直接的 typed bindings，而不是现在声称零成本。

Gain 通过 `params.float(Gain::LEVEL)` 一次取得 Copy ramp；在每个声道使用相同 sample offset。参数 ramp 状态只在执行完一段图之后推进一次，不能按声道或 fan-out 消费次数推进。离散参数通过 `discrete` 读取；后续插件事件列表可与 ProcessParameters 共存，但不改变音频引用语义。

## 7. 参数下发：身份、时间、背压

### 7.1 三类身份不能混用

`ParameterKey { ProcessorId, ParameterId }` 是控制侧业务身份；ParameterRuntime 的 dense slot 是本实例私有寻址；`plan_revision` 与 `timeline_epoch` 分别防止旧计划命令和旧时间轴命令误入。当前每个 runtime session 固定 revision/epoch，不支持在原队列里悄悄换表。

ProcessorId 由后续 Control registry 分配；本轮是调用方提供 u64，并检查一个资源集合内不重复。尚未实现永久 ID 分配、跨项目 ID 迁移或 ABA 回收。未来不得在同一 revision/epoch 下复用删除对象的身份。

ParamSpec 当前定义值类型、范围和初值。Float 必须有限，Bool/Enum/Int 不接受 ramp。Gain 使用线性幅度，不是 dB；UI 单位、归一化映射、显示精度和持久化属于控制侧 descriptor 扩展。

实时数值不触发重编译。声道布局、端口增删、插件加载、Processing SR、设备绑定等结构更新不应伪装成一个 ParamValue：它们走 prepare/publish 工作流，原计划继续运行直至新资源可用。

### 7.2 调度语义

请求使用绝对 Processing Timeline frame，或显式 NextBlock 意图，不能由 UI 猜测当前 callback 内 offset。Control::submit 的 observed_frame 必须来自同 epoch 的可信 RT 快照；当前示例直接使用 Engine::timeline，真正跨进程 worker 的快照发布和连接状态仍待实现。

当前一个 time-ordered SPSC：同一时刻按入队顺序处理；早到未来事件留在队列，晚到事件在可执行时应用并回复 AppliedLate。落在 `[start,end)` 的事件才属于当前 block，恰在 end 的事件留到下一块。每块开始只快照一次可消费数量，并限制 max_events_per_block，生产者不能靠持续入队放大 callback 工作量。不同时间点通过子区间重新执行图；最多消费 E 个事件，最多形成 E+1 段。

Float ramp 在首个有效 sample 前进 1/N，N 个 sample 后达到目标；后续 callback 接续进度，重设目标从上次渲染后的值开始。当前所有 Processor 均按 variable block 处理。固定块插件未来需要局部 adapter，并把其额外延迟登记进 latency model。

**当前限制：**长未来事件不能任意提前入队，否则 FIFO 会阻挡更新更早的交互请求。代码设置 horizon 并明确返回 OutOfOrder，而非错误地排序或静默丢失。生产 Control 必须先维护可排序的非 RT 待提交集合和拖动合并；在短提交窗口发布。若产品要求已提交未来自动化与立即控制可以抢占，需要两条有序 lane 的有界合并，或可取消的预分配事件结构，另行测试后再扩展。单个 FIFO 不宣称已解决此问题。

### 7.3 Accepted、Applied 与满队列

请求队列满：返回 QueueFull，调用方保留尚未提交的 desired value，可重试/合并；不得声称已应用。协议层 request_id 用于关联回复，目前不提供去重或 exactly-once 保证。

Accepted：校验通过并进入本次 session 队列。Applied/AppliedLate：RT 已更新参数表，回复包含实际生效 frame。回复队列满时暂停应用后续参数，但继续渲染音频；绝不应用一个无法保留确认的事件。控制服务必须持续排空确认，与向慢客户端发送网络/管道回复分开，避免慢 UI 长期阻挡控制生效。

ACK、meter telemetry、结构更新结果不可共用同一种丢弃策略。遥测可以丢新快照并计数；参数 ACK 不静默丢弃；结构事务需要明确成功或失败。多参数原子批次、自动化取消、连接中断后的 Pending 状态与请求幂等尚未实现，后续不能把一组独立 Accepted 当作全有全无事务。

## 8. IPC 协议与服务安全边界

当前 `moiren-core::protocol` 使用显式 little-endian 字段编码，带 u32 body-length、u16 version、u16 opcode。参数请求 body 固定 64 bytes，回复 body 固定 40 bytes；长度超限在读取 body 前拒绝，不依据不可信长度分配内存。支持短读/分段到达、拼接帧、版本和 opcode 校验、非法数值及截断错误。任何 framing/version 错误应关闭连接，不猜测下一帧位置。

协议 v1 仅包含参数请求与回复。拓扑编辑、snapshot、Hello/能力协商、meter subscription 和 bulk preset 均未定义；未来增加明确 opcode/version 或独立受限通道，不能直接序列化 Rust enum 的内存布局，也不能破坏 v1 固定帧约束。测试中的 Vec 只用于非 RT 编解码，不出现在 render 内。

Windows transport 下一步采用本地 Named Pipe 的候选方案。服务端必须明确：当前用户/logon SID ACL、拒绝远程客户端、连接数和消息速率上限、长度限制、版本握手、读写取消、超时、断线重连。不能依赖默认 pipe ACL；不能让管道名字替代身份验证。首次连接先获取 active revision、epoch、时间快照和参数 schema；断线不自动停音频，重连重新同步 active state。

Control 是唯一 RT 参数队列 producer。多个 IPC worker 可以在非 RT 层串行交给 Control；禁止让多个 GUI/client 克隆 SPSC producer 直接写 RT。跨进程共享内存音频属于后续 data plane，必须另有所有权、跨进程同步、崩溃恢复协议，不复用普通 AudioBlock 生命周期或 rtrb 进程内地址。

## 9. 只读分析节点与零附加音频延迟

`RtObserver::observe(ctx, ReadPorts)` 从类型上不授予可写音频；Observer adapter 也拒绝任何输出 binding。它是信号消费者，没有新的音频信号结果。UI 如将 meter 绘制为串联节点，Compiler 应把其音频透传解释为同一逻辑信号的别名，而不是创建新 buffer 或新 quantum。

Observer 必须参与 liveness：Source → Pre Meter → In-place Gain → Post Meter 可在一个 Slot 内执行；若 Pre Meter 排在 Gain 之后，就会错误观察后级信号。为了零拷贝不得牺牲 Pre/Post 语义。

当前 LevelMeter 汇总跨声道 sample peak/RMS，在配置帧数后于 segment 边界发布一条统计消息。窗口决定显示更新时刻，不延迟音频路径；CPU 计算仍计入 RT deadline，不能承诺零耗时。队列满时丢新快照并累计丢失数；Reader::latest 只排空开始读取时的有限条数，取已排队消息中的最新项，不是覆盖式 latest-value mailbox。

标准响度计后续需要独立实现、校准和测试向量。本轮不把 RMS 改名成 LUFS，也不声称具有 true-peak、响度门限、频率加权或精确标准积分窗口。timeline/设备 discontinuity 后的窗口重置、持续流状态、NaN/Infinity 和极端幅度下的统计处理需列入分析器验收。

重型 FFT/可视化分析可复制限量降采样数据到专属预分配队列，或由专用 worker 分析。不能为了异步分析保存 slab 引用；不能让 UI 消費速度控制 Graph 进度。首次实现优先使用有界快照，不做每 sample 原子写入。

## 10. 硬件与软件音频 I/O 扩展

Graph 只面向 `RtAudioSource` / `RtAudioSink` 的同步 streaming 面；设备枚举、格式协商、打开/停止、COM 对象与 native buffer lease 留在 backend owner。Source 只写当前 AudioBlockMut；Sink 在调用内消费样本，或复制到自己预分配的存储。

| 输入/输出类型 | 进入 Graph 之前/之后的职责 |
| --- | --- |
| WASAPI physical capture / render | PCM 打包、de/interleave、packet lease、timestamps、Shared/Exclusive、event 调度 |
| 应用 process loopback | 进程身份、捕获范围、与原始播放及 Moiren 自身输出的关系；不等于 Takeover |
| ASIO | 驱动 callback/通道布局、设备拥有权；不强行争抢 DAW 占用 |
| Recorder / file playback | 文件读写与编码在 worker；Graph 仅与预分配 bridge 交互 |
| 虚拟 Endpoint / 软件客户端 | 明确系统可见 I/O 与内部 Bus 的差异；单独的 IPC data plane 与故障恢复 |

Source adapter 提前提供初始化静音，不足输入可留下静音尾部。Sink 不可写入或丢失设备时应计数并报告，不能阻塞。基础骨架最初的 BoundaryReport 只是返回契约；后续 [IO 节点设计](../designs/03-io-nodes.md)已补齐 engine InputNode/OutputNode 的有界状态快照、越界 transferred_frames 校验、timeline discontinuity、silence/xrun 策略，以及软件 sample ring 和首版离线 app。旧 adapter 仍不转发快照。正式设备接入仍需 worker→Control 状态通道、capture flags、epoch 重置、PCM、SRC/drift 与设备生命周期，不能只以 trait 或软件桥已存在宣告 backend 完成。

只有 master demand 驱动 Graph；follower 只消费各自 bridge，不重复执行全图。输入与输出均可能独立时钟，名义采样率相同不代表共钟。Processing SR 转换与实际 clock drift 分开建模；ring fill、时间戳、SRC ratio 和恢复属于 Boundary，不进入普通 AudioBlock。

现有 W00 记录只支持相应实验结论，不能推导正式后端已稳定。保持 [Windows 接入计划](2026-10-08-windows-integration-plan.md) 的阶段依赖：首次独立 capture→render 即需要最小跨钟桥，多输出不能先跳过 follower 验证。未经明确实验许可，不改默认设备、不 mute 原应用、不启动可听测试音。

## 11. 资源生命周期与未来 Plan 发布

当前 ExecutionPlan 持有 Arena 和已绑定操作；RtResources 持有持久 Processor state。prepare 和 Engine::new 检查资源/参数表身份，避免同样形状的新 registry 冒充原 registry。一次计划每个 Processor runtime 只安排一次，避免意外重复推进状态。当前有界参数分段会对同一个 Processor 处理连续子区间，这是正常时间推进，不是重复实例调用。

现阶段 Engine 不提供运行中换表、移除 Processor 或切换 epoch。`into_parts` 要求外层已停止 render，将拥有权交回非 RT 线程析构；代码不声称任意线程直接 drop Engine 都符合实时契约。

下一步 publish/retire 协议：Control 准备新计划→最多一个 pending→RT block boundary 接收→切换→旧计划通过 retire queue 回 Control。retire 无容量时维持旧 active plan，不能在 RT 随手 drop 新旧 Box。失败/取消候选在 Control 清理；关闭流程先撤回设备 callback，再移交全部资源，最后销毁 queues。

持久 DSP registry 需要与计划 swap 同步增删，但不让 Control 在 RT 运行中读取/复制可变 DSP state。复用已有实例应通过 RT 所有权下的 slot/handle 迁移；替换实例在非 RT prepare，旧实例退出所有可达计划后延迟回收。参数表也要迁移当前值与 ramp 进度，并对不兼容 schema 和旧 epoch 请求明确回复。

crossfade、warm-up 和 state transfer 仍是后续策略，不是本轮既成设计。crossfade 同时渲染两个计划时不能对同一状态实例执行两次；必须明确分离、复制或一次计算后 fan-out，并计入额外计算与延迟预算。

## 12. 分阶段实施和出口条件

### F0 — 可运行基础（本 PR）

- [x] 单 slab + 私有 metadata；公开构造检查预算/溢出，音频格式 f32/f64。
- [x] 作用域 safe IO；N 路只读/独立写/原位配对；重复只读与 foreign proof 防护。
- [x] 参数 descriptor、typed codec、SPSC、时间分段、ramp、Accepted/Applied 与背压。
- [x] Observer 与 sample peak/RMS；独立 telemetry；fake Source/Sink 和离线例子。
- [x] ExecutionPlan / RtResources 分离，prepare 验证 IO/schema/绑定身份。
- [x] Linux/Windows 测试、Clippy、fmt 与 Miri 工作流；真实检查结果以 PR 对应提交为准。

出口：命令全部通过、离线示例不打开设备；没有把上述 scaffold 冒充正式 Graph/IPC/backend；生产 unsafe 集中审计。

### F1 — Compiler 与 slot reuse（依赖 F0）

- [ ] 从唯一 LogicalGraph 降为 signal values，稳定 topo schedule；计入 observer、Pre/Post、sidechain、重复 Bus 输入。
- [ ] 实现 last-use 与静态 slot assignment；首轮只整 Slot 复用，格式/容量不兼容不共享。
- [ ] in-place 只选 last consumer，保留未配对端口；禁用优化时行为不变。
- [ ] 对照无复用 safe 参考执行器，用随机 DAG 验证 fan-out、重复 source、空 Bus、Rack 顺序、channel topology 和 variable block 输出。

出口：优化开关前后逐样本或规定数值容差内一致；故意破坏 alias/liveness 时 prepare/编译拒绝，不能仅靠音频听感。

### F2 — Control owner 与真正 IPC（依赖 F0，与 F1 可并行）

- [ ] 连接/version/capability/session 协商；用户级 Named Pipe ACL、local-only、断线取消和速率限制。
- [ ] 发布可信 active revision、epoch、timeline 与 schema 快照；desired/queued/applied 三态区分。
- [ ] 非 RT 排序、lookahead、拖动 coalescing；明确未来自动化与交互请求冲突规则；不合并有语义的离散事件。
- [ ] Pending 请求表有容量/超时；ACK 排空不受单个慢客户端阻塞；补齐去重或明确 at-least-once 重试语义。
- [ ] 结构命令与实时数值命令分路；事务批次要全有全无，不能逐项排队冒充事务。

出口：两个独立进程真实 roundtrip，拆帧/拼帧/错版本/恶意长度/慢读/断线/重连全部有测试；堵塞 IPC 不阻塞音频；所有拒绝或延期可解释。

### F3 — 在线 publish/retire 与 DSP 状态（依赖 F1/F2）

- [ ] 有界 publish/retire 与 pending coalescing；无 RT 最终析构，满队列不交换所有权。
- [ ] 增删 Processor 和参数 schema 的事务、持久状态复用、ramp 迁移与 epoch 切换。
- [ ] 故障、取消、关闭和 device loss 期间回收所有资源；callback 不再访问 retired plan。

出口：高频编辑与参数洪泛下持续渲染，旧计划/实例析构线程可审计；旧命令不修改新对象；效果器状态不因无关路由修改复位。

### F4 — 第一个真实 Boundary 闭环（对接 W01/W06/W09）

- [ ] 先 fake source→单 master Shared render，再 physical/process capture→独立 bridge→render。
- [ ] 端点 owner、lease、PCM conversion、variable demand、ring 与最小 drift correction。
- [ ] BoundaryReport 接入统计；输入短缺静音、输出积压、discontinuity、设备重连、clock/SR 变化有明确策略。
- [ ] 实机记录延迟和 XRUN；多输出在 follower bridge 稳定后加入，不提前接 ASIO/driver 复杂度。

出口：遵守现有 Windows 实验操作边界，测试条件、实际格式和稳定性时长可复现；Capture/Takeover 分别验收。

### F5 — 完整分析器、插件和性能优化

- [ ] 标准响度/true-peak 测试向量、分声道/窗口语义、epoch reset；重分析 worker 的有界复制链路。
- [ ] 固定块 adapter 与插件 reported latency 接入 PDC；FFI 输入输出 raw pointer 契约单独审计。
- [ ] profile 后决定 64-byte alignment、clear elision、first-input overwrite、Send matrix fusion。
- [ ] arena-aware Miri、sanitizer/fuzz、并发队列 stress 和真实 deadline 测量持续纳入变更门槛。

这些工作不能为了“零拷贝”跳过初始化、别名或时间语义证明。

## 13. 代码归属和验证方法

| 文件 | 单一职责 |
| --- | --- |
| `moiren-core/src/protocol.rs` | wire DTO、显式字段编码与 framing；无 unsafe、无 RT 内存地址 |
| `moiren-engine/src/buffer.rs` | slab、access prepare、唯一引用构造边界、safe block/port view |
| `moiren-engine/src/control.rs` | 参数 schema/表、控制到 RT 队列、时间和 ACK、ramp |
| `moiren-engine/src/processor/mod.rs` | safe Processor/Observer 契约；兼容导出 Gain、Sum |
| `moiren-engine/src/processor/builtin/` | 内置 Gain、Sum DSP 实现 |
| `moiren-engine/src/runtime.rs` | prepare、runtime identity、线性执行与参数分段 |
| `moiren-engine/src/meter.rs` | 只读 sample peak/RMS 和有界遥测 |
| `moiren-engine/src/boundary.rs` | backend 无关 streaming 面与 fake source |
| `moiren-engine/tests/`、`examples/offline.rs` | 借用/控制/实时分配回归与可运行示例 |

基础检查：

```sh
cargo test --locked -p moiren-core -p moiren-engine
cargo clippy --locked -p moiren-core -p moiren-engine --all-targets -- -D warnings
cargo fmt -p moiren-core -p moiren-engine -- --check
cargo run --locked -p moiren-engine --example offline
rustup toolchain install nightly --profile minimal --component miri
cargo +nightly miri test --locked -p moiren-core -p moiren-engine
```

CI 另以 `MIRIFLAGS=-Zmiri-tree-borrows` 重跑。测试覆盖两种浮点、作用域逃逸 compile-fail、多输出同时存活、重复只读、非法 alias、foreign Arena/参数表、move/forget/unwind、边界时间、跨 block ramp、事件预算、命令/确认/遥测满队列与线程移交。内置链路的 render 用 thread-local allocator 计数验证零分配与零释放；此测试不推广到所有第三方 Processor。

Miri 通过是特定执行路径证据，不是一般性 soundness 证明；CI 不代替真机 deadline、听感、硬件时钟或音频质量测试。命令只覆盖 core/engine，未宣称重新验收 Windows 实验 crate。新增队列依赖由 Cargo 生成 lockfile；最终 CI 仅有只读权限，不自动改仓库。

## 14. 实现依据

下列公开资料用于约束底层实现，不替代项目自身架构决策：

- [Rust slice::from_raw_parts](https://doc.rust-lang.org/std/slice/fn.from_raw_parts.html) 与 [from_raw_parts_mut](https://doc.rust-lang.org/std/slice/fn.from_raw_parts_mut.html)：分配范围、对齐、初始化、生命周期和别名要求。
- [rtrb](https://github.com/mgeier/rtrb) 与 [0.4.0 API](https://docs.rs/rtrb/0.4.0/rtrb/)：进程内 realtime-oriented SPSC；本轮不另写 unsafe queue。
- [Miri](https://github.com/rust-lang/miri)：动态未定义行为检查及其边界。
- [Microsoft Named Pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)：服务端访问权限设计依据；实际 transport 留给 F2。
