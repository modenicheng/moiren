# Moiren Realtime Engine Design

> 文件：`02-engine-design.md`  
> 状态：Draft v0.3  
> 适用范围：Moiren realtime audio engine、Graph Compiler、ExecutionPlan、buffer management、clock/boundary runtime

## 1. 设计目标

Moiren realtime engine 负责将用户编辑的 `LogicalGraph` 编译为可在实时线程中稳定执行的 `ExecutionPlan`。

这一层不负责表达 UI，也不维护第二份可编辑 Graph。它只关心：

- 处理顺序；
- buffer 分配、复用与生命周期；
- processor 调用；
- mixing；
- send 参数；
- format / sample-rate conversion；
- latency compensation；
- realtime parameter delivery；
- graph plan 切换；
- master clock 驱动；
- 多设备边界时钟适配。

总体路径：

```text
LogicalGraph
    │
    ▼
Graph Compiler
    │
    ├─ topology validation
    ├─ lowering
    ├─ latency analysis
    ├─ buffer liveness analysis
    ├─ in-place / alias analysis
    ├─ mixing lowering
    └─ schedule generation
    │
    ▼
ExecutionPlan
    │
    ▼
Graph RT Thread
```

核心原则：

```text
LogicalGraph 负责表达用户意图。
ExecutionPlan 负责高效执行。
Boundary 负责隔离设备和操作系统差异。
Realtime thread 不参与资源准备和图构建。
```

---

## 2. 与 Graph 层的边界

Realtime engine 建立在以下已经确定的 Graph 语义之上：

```text
Graph primitive:
    Node
    Port
    Edge

Input Port:
    0..1 incoming Edge

Output Port:
    0..N outgoing Edges

fan-out:
    Graph connection semantics

fan-in:
    explicit Node semantics
```

Bus 是 Node，不是设备。

Rack 是 Node，不是嵌套 Graph。

所有 outgoing Edge 地位相同，不存在特殊 Main Output。

Edge 可以携带轻量 Send Parameters：

```text
send gain
send pan
mute
tap = Pre | Post
```

Port 只表达逻辑音频流，不对应一块 realtime buffer。

---

## 3. Realtime thread 基本约束

Graph RT Thread 中禁止：

```text
dynamic allocation
free / destructor with unpredictable work
blocking mutex
blocking IPC
file I/O
device enumeration
graph mutation
plugin loading/unloading
complex logging
UI callback
unbounded queue operation
```

RT Thread 只执行已经准备好的计划：

```text
receive boundary demand
    ↓
select current ExecutionPlan
    ↓
consume realtime parameter events
    ↓
execute schedule
    ↓
push data to sinks / boundaries
```

第一阶段只采用一个 Graph RT Thread。

不同音频设备可以拥有独立 boundary thread，但这些线程不并行执行 Graph 本身。

暂不引入：

```text
parallel DAG scheduler
work stealing
RT worker pool
cross-core graph execution
```

---

# 4. Processing Sample Format

内部处理格式与设备 / 文件 PCM 格式分离。

当前设计：

```text
Processing Sample:
    f32 default
    f64 optional

I/O PCM:
    i16
    i24
    i32
    f32
    f64
```

整数 PCM 只存在于 boundary / file I/O。

Graph DSP 不使用 i16/i24 作为内部 processing precision，避免：

- 中间结果过早 clip；
- 反复 quantization；
- fixed-point headroom 复杂度；
- SIMD / DSP 实现复杂化。

概念接口：

```rust
pub trait ProcessingSample:
    Copy + Default + Send + Sync + 'static
{
}

impl ProcessingSample for f32 {}
impl ProcessingSample for f64 {}
```

典型运行形式：

```rust
Engine<f32>
Engine<f64>
```

第一阶段默认只需要完整实现 `f32` 路径，但数据结构不应阻止未来加入 `f64`。

---

# 5. AudioBuffer 模型

## 5.1 Storage 与 Block View 分离

`AudioBuffer` 不应同时承担：

```text
memory ownership
+
process() temporary view
```

建议拆成：

```text
BufferArena<S>
    owns memory

BufferSlotId
    identifies reusable storage

AudioBlockRef<'a, S>
AudioBlockMut<'a, S>
    temporary views during one process call
```

Processor 不拥有 Graph buffer。

Processor 也不能保存任何 block view 或 sample pointer 跨越当前 `process()`。

需要跨 block 保存数据的 DSP，例如：

```text
Delay
Compressor envelope
Resampler
Reverb
Plugin internal state
```

必须自己拥有 persistent processor state。

---

## 5.2 内部采用 planar layout

内部统一采用 planar channel storage：

```text
channel 0:
L0 L1 L2 L3 ...

channel 1:
R0 R1 R2 R3 ...
```

而不是：

```text
L0 R0 L1 R1 L2 R2 ...
```

理由：

- DSP / SIMD 更容易处理单通道连续样本；
- 插件接口通常天然按 channel buffer 工作；
- 多声道处理时更容易建立 channel view；
- boundary 可以独立完成 interleave / deinterleave。

WASAPI / ASIO / file adapter 负责：

```text
device / file format
        ↓
format adapter
        ↓
Moiren planar processing buffer
```

设备布局和 PCM packing 不进入 Graph 核心。

---

## 5.3 Buffer metadata

概念结构：

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferSlotId(u32);

pub struct BufferSlotMeta {
    pub offset: usize,
    pub channels: u16,
    pub channel_stride: u32,
    pub capacity_frames: u32,
}

pub struct BufferArena<S: ProcessingSample> {
    storage: AlignedStorage<S>,
    slots: Box<[BufferSlotMeta]>,
}
```

其中：

```text
capacity_frames >= ExecutionPlan.max_block_frames
```

建议 storage 至少保证 64-byte alignment。

`sample_rate` 不需要重复存入每块 buffer。

同一 ExecutionPlan 内所有普通 processing buffer 默认属于同一个 Processing SR。

Channel layout 等静态信息优先放入：

```text
Port metadata
Compiler metadata
BufferSlot metadata
```

而不是每次 process callback 重复携带。

---

# 6. Buffer Pool → Buffer Planner + Buffer Arena

Moiren 不采用 RT 动态 buffer pool：

```text
RT:
    acquire()
    process()
    release()
```

即使 pool 已预分配，这种模型仍然让 realtime thread 承担：

```text
free-list management
runtime resource ownership
slot state transition
```

Moiren 已经存在 Graph Compiler，因此 buffer 生命周期可以静态规划。

正式方向：

```text
LogicalGraph
    ↓
lowered signal values
    ↓
liveness analysis
    ↓
Buffer Planner
    ↓
BufferSlot assignment
    ↓
ExecutionPlan + BufferArena
```

因此更合适的命名是：

```text
Buffer Pool      ×
Buffer Planner   ✓
Buffer Arena     ✓
```

---

## 6.1 Liveness

每个中间 signal value 拥有：

```text
producer
first use
last use
```

例如：

```text
A → B → D
 \→ C ─→ D
```

编译器可以建立：

```text
value A   live [0, 2]
value B   live [1, 3]
value C   live [2, 3]
value D   live [3, 4]
```

生命周期不重叠的 value 可以使用同一个物理 BufferSlot。

因此：

```text
logical signals != physical buffers
```

一个 Graph 可能具有几十个 Port / Edge / intermediate value，但只需要少量同时存活的 buffer。

---

# 7. Buffer Ownership 与 Lifetime

buffer ownership 只属于已经准备完成的 ExecutionPlan：

```text
Prepared ExecutionPlan
        │
        └── owns BufferArena
```

以下对象均不拥有 realtime buffer：

```text
LogicalGraph Node
Port
Edge
Processor
Send
Bus input
```

一次 render 中：

```text
BufferArena
    ↓
Executor resolves BufferSlot
    ↓
creates temporary AudioBlockRef / AudioBlockMut
    ↓
Processor::process()
```

一个 signal 的实际 buffer 生命周期由 compiler 决定：

```text
producer
    ↓
consumer 1
consumer 2
...
last consumer
    ↓
slot reusable
```

RT Thread 不执行引用计数，也不在运行时决定 buffer 是否可以释放。

---

# 8. Zero-copy Fan-out

Graph 已允许：

```text
Output Port → 0..N outgoing Edge
```

运行时 fan-out 不应复制 buffer。

例如：

```text
             ┌→ Headphones
Game Output ─┼→ Recorder
             └→ Stream Bus
```

编译后：

```text
Game output = BufferSlot #7

Headphones reads #7
Recorder   reads #7
Stream     reads #7
```

BufferSlot 在 last consumer 完成前保持 immutable。

不需要：

```text
reference counting
copy-on-write
runtime borrow tracking
per-edge buffer copy
```

Compiler 只需要知道：

```text
slot #7 last_use = ExecOp N
```

执行到最后一次读取后，后续 signal 才能复用该 slot。

---

# 9. In-place DSP

Processor 可以声明自身是否支持 in-place processing。

编译器只有在满足全部条件时才能令：

```text
output_slot = input_slot
```

至少包括：

```text
1. processor supports in-place
2. input/output format compatible
3. channel topology compatible
4. current processor is input value's last consumer
5. source buffer is writable internal storage
6. no conflicting alias between multiple outputs
```

例如：

```text
Game → EQ → Bus
```

如果 EQ 是 Game output 的最后一个 consumer：

```text
EQ input  = slot 3
EQ output = slot 3
```

合法。

但：

```text
         ┌→ Recorder
Game ────┤
         └→ EQ → Bus
```

若 Recorder 尚未读取 Game，则 EQ 不能覆盖 Game buffer。

如果 schedule 调整成：

```text
Recorder reads Game
EQ executes last
```

则 EQ 可以重新获得 in-place 条件。

第一阶段不要求为了增加 in-place 比例求全局最优 schedule。

规则：

> 先生成稳定拓扑执行顺序，再基于现有 schedule 执行 last-use alias analysis。

未来再考虑 schedule / buffer reuse 联合优化。

概念 capability：

```rust
pub enum InPlaceCapability {
    Never,
    Supported,
    Pairwise(Box<[InPlacePair]>),
}
```

---

# 10. Bus Mixing 与 Accumulation

Bus 编译后不需要为每个 Input Port 生成独立中间 buffer。

LogicalGraph：

```text
Game ─────→ Bus.Input0
Music ────→ Bus.Input1
Mic ──────→ Bus.Input2
```

ExecutionPlan 可以直接保存：

```rust
pub struct MixInput {
    pub source: BufferSlotId,
    pub gain: RtParamBinding,
    pub pan: RtParamBinding,
    // future: channel map / polarity / matrix
}

pub struct MixExec {
    pub output: BufferSlotId,
    pub inputs: Box<[MixInput]>,
}
```

执行：

```text
input0 -- transform --> dst
input1 -- transform + accumulate --> dst
input2 -- transform + accumulate --> dst
```

Send Gain / Pan / Mute 可以直接融合到 accumulation 中。

不需要：

```text
source
  ↓
temporary send buffer
  ↓
gain
  ↓
pan
  ↓
bus mix buffer
```

---

## 10.1 First-input overwrite optimization

Bus 不必总是：

```text
clear(dst)
accumulate(input0)
accumulate(input1)
...
```

可以：

```text
write transformed(input0) → dst
accumulate input1
accumulate input2
...
```

省去一次整块 clear。

进一步，如果 input0：

```text
last_use == this MixExec
```

并且 transform 支持 in-place，则可以直接：

```text
dst = input0 slot
apply send transform in-place
accumulate remaining inputs
```

因此：

```text
Buffer Planner
fan-out lifetime
in-place analysis
Bus lowering
```

应属于同一个 compiler pipeline。

---

# 11. Variable Block Engine

正式采用 Variable Block Engine。

核心约束：

```text
1 <= frames <= ExecutionPlan.max_block_frames
```

不要求：

```text
frames == 128
frames == 256
```

基础上下文：

```rust
pub struct ProcessContext {
    pub timeline_start: FrameTime,
    pub frames: u32,
    pub processing_sr: f64,
}
```

普通 processor：

```rust
pub trait RtProcessor<S: ProcessingSample> {
    fn process(
        &mut self,
        ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
    );
}
```

Master Boundary 一次要求多少 Processing Timeline frame，Graph 就处理多少，前提是不超过 `max_block_frames`。

典型情况：

```text
ASIO callback = 128
→ process(128)

WASAPI callback equivalent demand = 240
→ process(240)
```

当设备 sample rate 与 Processing SR 不同时，boundary 可产生变化后的 Graph frame demand，例如：

```text
261
262
261
262
...
```

Graph 本身仍能直接处理。

---

## 11.1 FixedBlockAdapter

某些 processor / plugin 如果只能处理固定块，则由 adapter 局部解决：

```text
Variable Graph Blocks
        ↓
FixedBlockAdapter
        ↓
fixed quantum processor
```

Adapter 可以拥有自己的：

```text
staging buffer
ring buffer
fixed-block accumulator
```

其额外 latency 必须进入 latency model。

不允许为了少数固定块 processor，将整个 Graph 强制改成固定 quantum。

---

# 12. ExecutionPlan

ExecutionPlan 不是第二张 Graph。

它更接近一组已经解析完成的 audio bytecode / linear operations。

建议：

```rust
pub struct ExecutionPlan<S: ProcessingSample> {
    pub generation: u64,

    pub processing_sr: f64,
    pub max_block_frames: u32,

    pub schedule: Box<[ExecOp]>,
    pub buffers: BufferArena<S>,

    pub latency: LatencyPlan,
}
```

基础操作：

```rust
pub enum ExecOp {
    Source(SourceExec),
    Process(ProcessExec),
    Mix(MixExec),
    Convert(ConvertExec),
    Delay(DelayExec),
    Sink(SinkExec),
}
```

具体类型仍可继续拆，但要求保持：

```text
no graph traversal in RT
no HashMap lookup in hot path
no dynamic port resolution in RT
no processor capability query in RT
```

运行逻辑应接近：

```rust
fn render<S: ProcessingSample>(
    plan: &mut ExecutionPlan<S>,
    ctx: &ProcessContext,
) {
    for op in plan.schedule.iter_mut() {
        execute(op, &mut plan.buffers, ctx);
    }
}
```

---

## 12.1 Lowering example

LogicalGraph：

```text
Game → Rack → Bus → Headphones
               ↑
Music ─────────┘
```

可能 lowering 为：

```text
0 Source(Game)      → slot 0
1 Process(Rack)     slot 0 → slot 1
2 Source(Music)     → slot 2
3 Mix(slot1, slot2) → slot 0
4 Sink(Headphones)  ← slot 0
```

这里：

```text
slot 0
```

在 Game 原始结果失效后已经复用。

---

# 13. Realtime Parameter Delivery

参数修改不触发 Graph recompilation。

包括：

```text
Gain
Pan
Mute
Send Gain
Send Pan
processor realtime parameters
```

参数事件统一绑定到 Processing Timeline：

```rust
pub struct ParamEvent {
    pub param: RtParamId,
    pub at: FrameTime,
    pub value: ParamValue,
}
```

不要使用：

```text
apply on next callback
```

作为唯一时间语义。

也不要让 control side 直接提交：

```text
offset_in_current_callback
```

因为 callback block size 可变。

---

## 13.1 Timeline-based event delivery

假设：

```text
current Processing Timeline = 14,381,920
```

用户更新 Gain，并希望事件发生在：

```text
frame 14,382,160
```

当前 render：

```text
start  = 14,382,000
frames = 256
```

RT 可转换成 processor-local offset：

```text
160
```

这样未来可支持 sample-accurate：

```text
automation
MIDI/control events
plugin parameter events
ramps
```

---

## 13.2 Queue topology

推荐：

```text
UI Threads
    ↓
Control Thread
    ↓
bounded SPSC Param Queue
    ↓
Graph RT Thread
```

不建议多个 UI producer 直接写 RT MPSC queue。

Control Thread 可以合并高频拖动事件：

```text
-1.001
-1.003
-1.008
-1.010
```

只向 RT 提交必要事件。

Queue 必须：

```text
bounded
preallocated
non-blocking on RT side
```

---

## 13.3 Stable realtime parameter IDs

参数 ID 不应依赖：

```text
ExecOp index
Node vector index
BufferSlot index
```

这些可能在 recompilation 后变化。

建议：

```rust
pub struct RtParamId {
    pub slot: u32,
    pub generation: u32,
}
```

或其它稳定 handle scheme。

参数绑定在 plan build 阶段解析完成。

Built-in Gain / Pan 等参数需要内部 smoothing / ramp，防止 zipper noise。

---

# 14. Graph Recompilation 与 Plan Swap

Topology change 走：

```text
UI
 ↓
Control Thread
 ↓
LogicalGraph mutation
 ↓
Graph Compiler
 ↓
prepare resources
 ↓
new ExecutionPlan
 ↓
publish
 ↓
RT swaps at block boundary
```

RT Thread 不参与：

```text
compile
allocate
plugin initialization
resource destruction
```

---

## 14.1 Publish / Retire protocol

建议采用简单 RCU 风格协议：

```text
Control Thread                         Graph RT Thread

compile new plan
allocate arena
prepare processors

       ───── publish ───────────────→

                                      render boundary
                                      swap current plan

       ←──── retire old plan ───────

destroy old plan
free arena
release resources
```

可以使用两个固定容量 SPSC queue：

```text
publish: Control → RT
retire:  RT → Control
```

RT 不做最终资源销毁。

尤其避免 RT 上发生：

```text
Arc final drop
Box deallocation
plugin destructor
COM release with unpredictable work
```

---

## 14.2 Pending plan coalescing

连续编辑：

```text
A
B
C
```

不应该产生三份长期排队的 ExecutionPlan。

第一阶段建议限制：

```text
最多一个 pending plan
```

Control Thread 始终保留：

```text
latest desired LogicalGraph
```

如果已有 plan 等待 swap，则中间版本可以合并。

完成 swap / retire handshake 后，再编译最新 topology。

后续如果编译开销明显，可以进一步引入取消、版本跳过或增量编译。

---

# 15. Processor Runtime Lifetime

Graph recompilation 不应自动清空所有 DSP state。

例如：

```text
用户只是新增一条 Edge
```

不应该导致：

```text
Compressor envelope reset
Reverb tail reset
Plugin state reset
```

因此 processor runtime 与 ExecutionPlan 需要适度分离。

概念：

```text
RtResourceRegistry

ProcessorId 42
    └─ persistent Compressor runtime
```

ExecutionPlan 保存已经解析完成的 realtime handle。

```text
old plan ─┐
          ├→ Processor runtime #42
new plan ─┘
```

RT 一次只执行 active plan，因此同一个 runtime 不会由两个 Graph Plan 同时调用。

删除 processor 时：

```text
new plan no longer references runtime
        ↓
old plan retires
        ↓
Control Thread destroys runtime
```

具体 ownership 类型需要在 Rust contract 阶段继续确定，但 realtime 析构不能发生在 Graph RT Thread。

---

# 16. Latency Model

所有内部 latency 统一以：

```text
Processing Timeline frames
```

表达。

第一阶段至少支持整数 frame latency。

建议类型一开始给未来 fractional delay 留空间：

```rust
pub struct Latency {
    pub whole_frames: u32,
    pub fractional_q32: u32,
}
```

Processor / adapter 可以声明：

```text
intrinsic latency
```

例如：

```text
Gain                 0
basic EQ             usually 0
lookahead limiter    N
linear phase EQ      N
FixedBlockAdapter    N
Resampler            filter delay
plugin               plugin reported latency
```

---

## 16.1 DAG latency propagation

Compiler 沿 DAG 传播 arrival latency。

概念：

```text
arrival(output)
    = max / transformed input arrival
    + processor intrinsic latency
    + adapter latency
```

在 fan-in / mix point：

```text
Input A = 128 frames
Input B = 512 frames
Input C =   0 frames
```

为了同步：

```text
A +384
B   +0
C +512
```

然后再执行 Bus mix。

---

## 16.2 PDC 属于 ExecutionPlan

LogicalGraph：

```text
A ──┐
B ──┼→ Bus
C ──┘
```

ExecutionPlan 可以生成：

```text
A → CompDelay(384) ─┐
B ──────────────────┼→ Mix
C → CompDelay(512) ─┘
```

Compensation Delay 不需要出现在用户 Graph 中。

但 Inspector / diagnostics 必须允许解释：

```text
automatic latency compensation: +512 frames
```

以延续 explainable routing 原则。

---

## 16.3 第一阶段补偿边界

第一阶段自动 PDC 只要求：

```text
对同一 fan-in / mix point 的路径进行对齐
```

不同物理输出之间是否互相等待属于：

```text
Output Synchronization
```

这是独立策略。

默认不要为了对齐高延迟 HDMI 输出，让低延迟耳机无条件增加相同延迟。

---

# 17. Processing SR 与 Master Clock

必须区分两个概念。

## 17.1 Processing Sample Rate

Processing SR 定义：

```text
Processing Timeline 中
1 second = Processing SR frames
```

例如：

```text
48 kHz
→ 48,000 Processing Timeline frames / second
```

它是 Graph 内 DSP 时间坐标。

---

## 17.2 Master Clock Source

Master Clock Source 决定：

```text
现实时间推进速度
什么时候需要继续 render
```

它通常来自主输出设备，但不要求一定是 Output。

例如纯录音场景：

```text
USB Mic → Recorder
```

USB Mic 完全可以成为 Master Clock Source。

因此底层概念使用：

```text
Master Clock Source
```

UI 在普通输出场景仍可显示：

```text
Master Device
```

---

## 17.3 Processing SR Auto / Manual

Auto：

```text
Processing SR := Master Boundary nominal sample rate
```

典型情况：

```text
Master DAC = 48 kHz
Processing SR = 48 kHz
```

Manual：

```text
Master DAC = 44.1 kHz
Processing SR = 48 kHz
```

仍然合法。

此时：

```text
48 kHz Processing Timeline
        ↓
fixed-ratio SRC
        ↓
44.1 kHz Master Device
```

Master Device 仍然是时间权威。

---

# 18. SRC 与 Drift Correction 分离

必须明确区分：

```text
Sample Rate Conversion
```

与：

```text
Clock Drift Correction
```

情况 A：

```text
Processing SR 48k
Master device 44.1k
```

需要 fixed-ratio SRC：

```text
48000 / 44100
```

但不需要独立时钟 drift correction，因为 Master Device 本身就是时间权威。

情况 B：

```text
Master DAC 48k
Follower HDMI 48k
```

两个 nominal SR 相同，但硬件晶振独立。

此时需要 async clock adaptation / drift correction。

---

# 19. 多设备 Boundary Clock 模型

Graph 内只有：

```text
one Processing Timeline
one Processing SR
one Master Clock Source
```

不同设备可以拥有独立 hardware clock，但差异全部在 boundary 处理。

总体模型：

```text
                         Graph RT Thread
                       Processing Timeline
                              │
             ┌────────────────┼────────────────┐
             │                │                │
             ▼                ▼                ▼
        Master DAC        HDMI follower    USB follower
```

Follower boundary 不推动 Graph timeline。

---

## 19.1 Master Boundary

Master Boundary 负责决定 Graph render demand。

概念：

```text
device callback/event
        ↓
how much real time is requested
        ↓
convert to required Processing Timeline frames
        ↓
Graph RT render(frames)
```

当 Processing SR 与 Master Device SR 相同：

```text
1 device frame ≈ 1 graph frame
```

不同时则通过 phase accumulator / SRC demand 产生 variable graph block size。

---

## 19.2 Follower Output

Follower Output：

```text
Graph RT Thread
      │
      ▼
SPSC Audio Ring
      │
      ▼
Async SRC / Clock Adapter
      │
      ▼
Follower Device RT Thread
```

必须存在跨线程 buffering，因为硬件 callback 不同步。

---

## 19.3 Follower Input

Follower Input：

```text
Follower Capture Thread
      │
      ▼
SPSC Audio Ring
      │
      ▼
Async SRC / Clock Adapter
      │
      ▼
Graph RT Thread
```

因此：

```text
Input device clocks
Output device clocks
```

统一视为 Boundary Clock 问题。

Graph 内普通 Node 不感知这些硬件时钟。

---

# 20. Drift Estimation 与 Adaptive Resampling

Follower bridge 通过 audio ring fill level 估计相对 clock drift。

目标：

```text
ring fill ≈ target fill
```

例如：

```text
capacity = 4096 frames
target   = 2048 frames
```

如果 follower 消耗过快：

```text
fill < target
```

则微调 async resampler ratio。

如果 follower 消耗过慢：

```text
fill > target
```

则反方向调整。

概念控制器：

```text
error = current_fill - target_fill

correction =
    Kp * error
  + Ki * integral(error)

ratio = nominal_ratio * (1 + correction)
```

正常 correction 应限制在很小范围，例如 ppm 级。

具体 Kp / Ki、滤波、最大 slew rate 尚未定案。

---

## 20.1 Discontinuity recovery

以下情况不能只靠 drift controller 慢慢恢复：

```text
device restart
large timing discontinuity
ring underrun
ring overflow
system suspend/resume
endpoint invalidation
```

应执行显式 bridge reset：

```text
re-center ring
reset drift estimator
reset SRC phase/state as required
short fade / ramp
report discontinuity / XRUN
```

不要让 PI controller 为恢复大错误而产生极端 resampling ratio。

---

# 21. Boundary Responsibilities

系统 I/O Node 是 Graph 与 OS / device runtime 之间的明确边界。

Boundary 负责：

```text
device callback integration
PCM format conversion
interleave / deinterleave
fixed sample-rate conversion
clock adaptation
cross-thread ring buffering
hardware period handling
shared / exclusive mode
ASIO buffer handling
XRUN / discontinuity reporting
```

普通 DSP Graph 不直接处理：

```text
WASAPI packet shape
ASIO callback ownership
hardware clock drift
PCM bit depth
interleaving
```

这样可以保证 core engine 不绑定某个 Windows backend。

---

# 22. Compiler Pipeline

建议 Graph Compiler 最终形成如下阶段：

```text
LogicalGraph
    │
    ├─ 1. validate DAG / ports / formats
    │
    ├─ 2. lower Node semantics
    │      Rack → Process operations
    │      Bus  → Mix operations
    │      Sends → Mix transforms
    │
    ├─ 3. insert format / SR adapters
    │
    ├─ 4. resolve processor capabilities
    │      in-place
    │      latency
    │      max block
    │      port topology
    │
    ├─ 5. calculate path latency
    │
    ├─ 6. insert compensation delays
    │
    ├─ 7. generate topological schedule
    │
    ├─ 8. signal liveness analysis
    │
    ├─ 9. BufferSlot allocation / reuse
    │
    ├─10. in-place alias analysis
    │
    ├─11. bind realtime parameters
    │
    ├─12. allocate BufferArena
    │
    └─13. emit ExecutionPlan
```

Compiler 可以内部使用临时 IR。

临时 IR 不是公开 Graph 类型，也不需要持久化。

---

# 23. 初步 Rust Contract

下面是一版接口方向，不视为最终 API，但后续实现应尽量围绕这些责任边界收敛。

```rust
pub trait ProcessingSample:
    Copy + Default + Send + Sync + 'static
{
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferSlotId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameTime(pub u64);

pub struct ProcessContext {
    pub timeline_start: FrameTime,
    pub frames: u32,
    pub processing_sr: f64,
}

pub struct ExecutionPlan<S: ProcessingSample> {
    pub generation: u64,
    pub processing_sr: f64,
    pub max_block_frames: u32,

    pub schedule: Box<[ExecOp]>,
    pub buffers: BufferArena<S>,
    pub latency: LatencyPlan,
}

pub enum ExecOp {
    Source(SourceExec),
    Process(ProcessExec),
    Mix(MixExec),
    Convert(ConvertExec),
    Delay(DelayExec),
    Sink(SinkExec),
}

pub struct MixInput {
    pub source: BufferSlotId,
    pub gain: RtParamBinding,
    pub pan: RtParamBinding,
}

pub struct MixExec {
    pub output: BufferSlotId,
    pub inputs: Box<[MixInput]>,
}

pub trait RtProcessor<S: ProcessingSample> {
    fn process(
        &mut self,
        ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
    );
}
```

Processor metadata 在 prepare / compile 阶段查询：

```text
port topology
sample format support
in-place capability
latency
block-size constraints
```

RT 不重复查询这些 capability。

---

# 24. 当前已经可以视为定案的 realtime engine 原则

```text
1. Graph RT Thread 不使用 LogicalGraph 直接执行。

2. ExecutionPlan 是线性编译结果，不是第二张 Graph。

3. 内部 processing format 默认 f32，并预留 f64。

4. 内部 audio storage 使用 planar layout。

5. Audio storage 由 ExecutionPlan / BufferArena 拥有。

6. Port / Edge / Processor 不拥有 realtime buffer。

7. 不使用 RT dynamic Buffer Pool；buffer reuse 由 Buffer Planner 静态决定。

8. BufferSlot 生命周期由 signal liveness 决定。

9. fan-out 默认 zero-copy，多 consumer 读取同一 slot。

10. in-place processing 由 compiler 根据 capability + last-use 静态决定。

11. Bus mixing 直接从 upstream slot accumulate，不创建 per-input 中间 buffer。

12. Send Gain/Pan/Mute 可以融合到 mix accumulation。

13. 采用 Variable Block Engine。

14. 固定 quantum processor 由局部 FixedBlockAdapter 兼容。

15. 第一阶段只使用一个 Graph RT Thread。

16. realtime 参数通过 Control Thread → bounded SPSC → RT Thread。

17. realtime 参数使用 Processing Timeline FrameTime 表达时间。

18. topology change 编译新 ExecutionPlan，并在 block boundary swap。

19. old plan 在 RT 外销毁。

20. processor persistent state 不应因普通 graph recompilation 自动重置。

21. latency 以 Processing Timeline frames 表达。

22. fan-in point 执行自动 latency compensation。

23. Processing SR 与 Master Clock Source 是两个独立概念。

24. Auto Processing SR 跟随 Master Boundary nominal SR。

25. Manual Processing SR 可以与 Master Device SR 不同。

26. fixed-ratio SRC 与 independent-clock drift correction 分开处理。

27. Graph 内只有一个 Processing Timeline。

28. 多设备硬件时钟差异全部在 boundary bridge 处理。

29. follower device 不推动 Graph timeline。

30. follower boundary 使用 ring buffer + adaptive SRC 吸收 drift。
```

---

# 25. 尚未锁死的实现细节

以下已经有方向，但还不应写成不可更改架构约束：

```text
1. BufferArena 具体 storage representation
2. BufferSlot alignment / padding policy
3. planar channel stride 具体布局
4. Buffer Planner 使用 linear-scan 还是其它 allocator
5. ExecutionPlan ExecOp 是否继续细分
6. Processor runtime registry 的 Rust ownership 形式
7. Param Queue 具体实现
8. ParamEvent overflow / coalescing policy
9. plan publish / retire queue 具体类型
10. pending plan 的编译取消策略
11. latency fractional representation 是否第一版直接实现
12. FixedBlockAdapter 内部 staging 策略
13. master boundary frame-demand phase accumulator
14. async resampler 算法
15. drift estimator filter
16. PI controller 参数与 slew limit
17. follower ring target fill 与容量策略
18. discontinuity fade 长度
19. Output Synchronization 是否以及何时加入
20. Graph RT Thread 的 MMCSS profile / Windows priority policy
```

这些属于后续接口实现和 benchmark 阶段继续收敛的内容。

---

# 26. 下一步

目前核心 realtime architecture 已经可以进入代码接口设计。

建议实现顺序：

```text
1. ProcessingSample
2. BufferSlotId / BufferSlotMeta
3. BufferArena
4. AudioBlockRef / AudioBlockMut
5. Compiler-side SignalValue / liveness
6. BufferPlanner
7. ExecOp / ExecutionPlan
8. simple Source → Gain → Sink prototype
9. zero-copy fan-out
10. in-place analysis
11. MixExec
12. variable block validation
13. realtime ParamQueue
14. PlanExchange
15. latency propagation / CompDelay
16. boundary clock bridge
17. drift / ASRC
```

第一阶段测试 Graph 可以刻意保持极小：

```text
Source A ─→ Gain ─┬→ Sink A
                  └→ Bus ─→ Sink B
Source B ──────────┘
```

用它验证：

```text
variable block
buffer reuse
fan-out zero-copy
last-use
in-place
mix accumulation
param update
plan swap
```

这些成立后，再接入真正 WASAPI boundary。
