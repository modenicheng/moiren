# Moiren 核心架构设计阶段性整理

## 1. 项目核心定位

Moiren 是面向 Windows 11 设计的开源实时音频图系统。

核心目标不是复刻 Voicemeeter，也不是将 JACK / PipeWire 直接移植到 Windows，而是在 Windows 原生音频能力之上建立一套可视化、可解释、可自由连接的实时音频路由系统。

项目长期坚持几个基本原则：

- Graph first：Graph 是底层模型，Mixer 只是不同视图。
- Bus is not a device：Bus 是内部图对象，Windows Endpoint 只是按需导出机制。
- Realtime first：架构首先服从实时音频约束。
- Native first：原生可执行程序、Windows API，不依赖浏览器运行时。
- Progressive complexity：普通用户看到简单模型，专业用户仍能控制采样率、时钟、ASIO、多通道等细节。
- Explainable routing：任何隐式转换、路由、处理都应能向用户解释。

原始愿景已经明确强调「所有声音都可以看见、所有路径都可以解释、所有连接都可以修改」，并将 Bus 与 Windows Endpoint 明确区分。 

---

# 2. Graph 总体模型

目前使用两层图模型，第一层直接面向用户，经过编译后的产物交给引擎执行。

正式方向收敛为：

```text
LogicalGraph
     │
     │ compile
     ▼
ExecutionPlan
```

其中可以存在临时 Compiler IR，但它只是编译实现细节，不属于项目公开模型，也不是另一张长期维护的 Graph。

`LogicalGraph`：

- 用户编辑；
- GUI 展示；
- 项目文件持久化；
- 表达用户真正想建立的拓扑。

`ExecutionPlan`：

- Graph Compiler 生成；
- 供实时线程执行；
- 不提供 Graph 编辑语义；
- 不要求继续维护 Node / Edge 图对象。

其内容大致包括：

```text
Processing schedule
Buffer bindings / Buffer plan
Route bindings
Realtime parameter bindings
Latency metadata
Format conversion information
```

此前已经明确：运行时需要编译结果，但没有必要为了实时执行再建立一份公开 Graph。

---

# 3. Graph Primitive

核心图只保留三个基本对象：

```text
Node
Port
Edge
```

原则上，一切用户能够在 Graph 中看到的实体都属于某种 Node。

例如：

```text
Application Source
Physical Input
Physical Output
Bus
Rack
Recorder
Meter
Analyzer
Virtual Endpoint
Utility
```

Bus 不成为独立 Graph primitive。

Rack 也不成为第二套嵌套图系统。

Graph 的主要复杂度应由 Node 类型和 capability 表达，而不是不断增加新的顶层图对象。

---

# 4. Port 模型

## 4.1 Port 表达音频流，而不是单声道

一个 Port 可以携带一个完整多声道流。

例如：

```text
Stereo
Mono
5.1
7.1
7.1.4
Discrete(N)
Custom(...)
```

默认不会把 stereo 展开成：

```text
L
R
```

也不会把 5.1 默认展开成六个 Graph Port。

需要拆分、重排或组合声道时，再使用明确的 channel mapping 行为。

这一设计已经作为 Graph 基础语义收敛。

---

## 4.2 Input Port 与 Edge 的关系

采用严格独立 Input Port 模型：

```text
Input Port
    ← 0..1 Edge

Output Port
    → 0..N Edges
```

因此：

```text
Game ─────→ Bus.Input0
Music ────→ Bus.Input1
Discord ──→ Bus.Input2
Mic ──────→ Bus.Input3
```

不是：

```text
Game ─┐
Music ├→ 一个特殊 MixingInput
Mic ──┘
```

这样可以保持几个非常简单的系统不变量：

```text
一个 Input Port 最多一个 upstream

一个 Edge 始终连接：
OutputPort → InputPort

Output Port 天然支持 fan-out

fan-in 由拥有多个 Input Port 的 Node 完成
```

独立 Input Port 主要增加少量 LogicalGraph metadata，不意味着实时线程必须给每个 Port 分配独立 audio buffer。

---

# 5. Fan-out 与 Fan-in

## Fan-out

Output Port 可以直接连接多个目标：

```text
             ┌→ Headphones
Game Output ─┼→ Recorder
             └→ Stream Bus
```

无需 Splitter Node。

因此：

```text
fan-out = Graph connection semantics
```

## Fan-in

普通 Input Port 不接受多个 Edge。

混合必须显式发生在能够处理多个独立 Input Port 的节点中，例如 Bus。

因此：

```text
fan-in = Node semantics
```

这样用户能够明确看到哪里真正发生了求和，而不是在一个“万能 Input”里隐式完成。

---

# 6. Edge 与 Send

Edge 不只是简单的 topology 引用。

为了兼容传统 Mixer 工作流，每条发送允许携带轻量 routing 参数：

```text
SendParameters
├─ tap
├─ gain
├─ pan
└─ mute
```

当前 tap：

```text
Pre-Fader
Post-Fader
```

默认：

```text
Post-Fader
```

这些参数属于 routing/send 语义，不需要额外创建：

```text
GainNode
PanNode
MuteNode
```

例如：

```text
Game ── -6 dB / L20 ──→ Stream Bus
```

Graph UI 可以将这些参数画在 Edge 上。

Mixer 则可以把相同参数显示成传统 Send 控件。

Send 参数变化属于实时参数变化，不属于 topology change，因此不应该触发整图重新编译。 

---

# 7. Node Gain / Pan 与 Pre/Post Send

对于具有 routing/channel-strip 能力的 Node，当前信号概念为：

```text
Node processing result
        │
        ├──── Pre-Fader Tap
        │
        ▼
      Gain
        ↓
       Pan
        │
        └──── Post-Fader Tap
```

其中：

```text
Pre-Fader
= Node Gain/Pan 之前

Post-Fader
= Node Gain/Pan 之后
```

因此 Node Gain/Pan 用于批量调整这个 Node 的整体结果。

例如想整体降低 Game：

```text
Game Gain = -6 dB
```

而不是分别调整：

```text
Headphones Send
OBS Send
Recorder Send
...
```

只有当用户想单独修改某一条发送时，才修改 Edge 上的 Send Parameters。

所有输出 Edge 仍然平权：

```text
不存在 Main Output
```

这一点已经明确放弃传统 DAW “主输出 + Sends”的底层模型。

---

# 8. RackNode

处理器链不再属于每个普通 Node 内部。

目前决定增加一种独立节点：

```text
RackNode
```

例如：

```text
ApplicationSource
       ↓
    RackNode
       ↓
     BusNode
       ↓
     Output
```

RackNode 可以承载：

```text
Built-in DSP
VST3
CLAP
未来其它 Processor
```

这样普通 Graph 不需要将：

```text
EQ → Compressor → Limiter → ...
```

全部摊成大量独立 Node。

RackNode 已确定作为一等 Node，而不是每个 Node 都携带 ProcessorRack。

---

# 9. Rack 内部模型

Rack 内部不是一张自由 Graph。

它保持线性 Processor Chain：

```text
Main Input
    ↓
Slot 1
    ↓
Slot 2
    ↓
Slot 3
    ↓
Slot N
    ↓
Main Output
```

Slot 顺序就是执行顺序。

用户拖动：

```text
EQ
Compressor
```

变成：

```text
Compressor
EQ
```

只会改变 Rack 内部处理顺序，不改变 LogicalGraph topology。

Rack 内部不支持自由：

```text
split
merge
branch
任意连接
```

如果需要复杂 routing，就回到顶层 Graph。

这样可以避免：

```text
Graph
    └─ Rack
          └─ another Graph
```

这种递归式复杂结构。

---

# 10. Rack 多输入、多输出

RackNode 从架构第一版就支持：

```text
N input ports
N output ports
```

普通场景 UI 默认只展示：

```text
Main Input
Main Output
```

插件声明额外 bus 时，再动态暴露：

```text
Sidechain Input
Aux Input
Aux Output
Multi Output
Surround I/O
```

例如：

```text
Music ─────────────→ Rack.Sidechain
Mic ───────────────→ Rack.Main
```

这样能够自然支持：

```text
main input + sidechain
mono → stereo
multi-output plugin
surround plugin
```

而不需要未来重新修改 Port 模型。

---

# 11. Rack Preset

Rack 很适合直接承担 preset 能力。

例如：

```text
Broadcast Voice

1. Gate
2. EQ
3. Compressor
4. De-esser
```

Preset 保存：

```text
Processor 类型
顺序
启用 / bypass 状态
Processor state / parameters
```

不保存 Graph routing。

因此：

```text
RackPreset
```

是处理链 preset，而不是 Scene 或整个 Graph preset。

---

# 12. Mixer View 当前定位

Mixer 暂时保留，但不再尝试模拟完整 DAW channel strip。

最重要规则：

> Mixer 不从 Graph 上自动搜索、折叠 Processor Chain。

例如：

```text
Game → Rack → Voice Bus → Headphones
```

Mixer 不需要分析：

```text
Game → Rack
```

然后猜测 Rack 是否应该“属于 Game channel”。

否则存在：

```text
Game → EQ ─┬→ Bus A
           └→ Compressor → Bus B
```

时根本无法稳定建立双向映射。

因此：

```text
Graph = canonical topology
Mixer = routing/control projection
```

Mixer 主要展示适合快速批量控制的 Node：

```text
Gain
Pan
Mute
Sends
Meter
```

Rack 内容则通过 Rack Node 本身或 Inspector 编辑。

这样 Mixer 修改不会大规模重构 Graph。此前已经明确不应从 Graph processor chain 反向推导 Mixer。

---

# 13. Node Capability

目前更合适的方向不是“所有 Node 都具有相同控件”，而是 capability 模型。

例如概念上：

```text
Routable
├─ Gain
├─ Pan
├─ Mute
└─ Sends

ProcessorHost
└─ Processor Slots
```

典型节点：

```text
ApplicationSource
    Routable

BusNode
    Routable
    Mixing

RackNode
    ProcessorHost

OutputNode
    Device Boundary
    optional output controls

Meter / Analyzer
    Utility
```

具体 Rust trait / enum 形式尚未正式确定，但架构上不应该要求每个 Node 都同时拥有 Gain、Pan、Rack、Send 等所有能力。

---

# 14. BusNode

Bus 是一种 Node，不是 Windows Audio Endpoint。

主要职责：

```text
dynamic Input Ports
mixing
routing
Gain/Pan
Sends
```

例如：

```text
Game ─────→ Input 0 ┐
Music ────→ Input 1 ├→ [Stream Bus] ──→ Output
Mic ──────→ Input 2 ┘
```
Bus 本身仍然不是设备。原始项目设计已经将此作为核心原则。

# 14.5 系统 IO 节点

这一节是之前我们没有讨论过的。

## In

捕获相关的至少两种：直接接管应用输入（然后把它输出流掐掉），后期需支持虚拟设备输入和统一桌面VAD输入

除此之外还有音频设备的输入（后续考虑加入midi支持，但这是很久之后的计划，可能v3都不一定搞）

## Out

这个节点统一处理输出，而不是在任意节点/Bus节点中设置输出到哪里。应该显式通过UI/数据明确我的音频到底是从哪里到哪里了

需要支持各类音频硬件设置，不限于时钟问题、采样问题、独占等等。

---

# 15. Buffer 与 Port 的关系

当前已经确定一个重要方向：

> LogicalGraph Port 不等价于 realtime audio buffer。

例如 Bus 有：

```text
Input0
Input1
Input2
Input3
```

编译后完全可以只成为：

```text
BusExec
├─ source buffer #3
├─ source buffer #8
├─ source buffer #12
└─ source buffer #17
```

实时混合：

```text
clear(output)

mix(output, source3)
mix(output, source8)
mix(output, source12)
mix(output, source17)
```

无需执行：

```text
source
 ↓ copy
InputPort buffer
 ↓ copy
mix buffer
```

因此大量 Input Port 不意味着大量 buffer allocation。

真正影响 audio buffer 数量的是“同时仍然存活的信号结果”，而不是 LogicalGraph Port 数。

---

# 16. Send DSP 可以融合进 mixing

Send Gain / Pan 不需要创建中间 buffer。

例如：

```text
Game
 └─ -6 dB / L20
        ↓
       Bus
```

ExecutionPlan 可以直接生成：

```text
mix_accumulate(
    bus_output,
    game_buffer,
    gain,
    pan,
)
```

而不是：

```text
Game
 ↓
temporary buffer
 ↓
Gain
 ↓
Pan
 ↓
Bus
```

这样可以同时保留清晰 Graph 模型和低开销实时执行。

---

# 17. Format Conversion

兼容转换允许自动进行。

例如：

```text
44.1 kHz
    ↓
SRC
    ↓
48 kHz
```

但所有隐式转换必须向用户可见。

Graph Edge 或 Inspector 至少需要能够说明：

```text
Automatic conversion
44.1 kHz → 48 kHz
```

不允许“自动转换但完全隐藏”。

这里 UI 设计需要再行商议。预期可能就是把输入节点标记一个高亮色，hover后有对应提示）

对于存在明确语义损失的 channel conversion，例如：

```text
5.1 → stereo
```

默认禁止隐式完成。

用户必须明确选择 Downmix / Channel Mapper 策略。

这保持“所有路径都可以解释”原则。

---

# 18. Processing Sample Rate

正式 UI/架构名称：

```text
Processing Sample Rate
```

简称：

```text
Processing SR
```

不使用 “Graph sample rate” 这种对用户不够直观的名称。

提供：

```text
Auto
Manual
```

Auto：

```text
Processing SR = Master Device SR
```

例如：

```text
Master Device: 96 kHz
Processing SR: 96 kHz
```

Manual：

```text
Processing SR: 48 kHz
Output Device: 96 kHz
```

则输出边界执行：

```text
48 → 96 kHz
```

并在 UI 中明确显示。

---

# 19. Processing Sample Format

这部分尚未完全定案。

目前一致部分：

```text
默认 f32
```

并希望底层抽象 sample type，为高级用户预留其它处理精度。

讨论过：

```text
f32
f64
```

作为内部 Processing Sample。

同时：

```text
i16
i24
i32
f32
f64
```

可以作为设备 / 文件等 PCM I/O format。

但是否允许 `i16/i24` 成为真正内部 processing precision，目前尚未最终确认。

当前较保守候选方案是：

```text
Processing:
    f32
    f64

I/O:
    i16
    i24
    i32
    f32
    f64
```

理由包括浮点 DSP headroom、避免反复 quantization 以及 SIMD 实现便利性。相关讨论尚需继续。

---

# 20. Graph Cycle

第一阶段明确：

```text
LogicalGraph must be a DAG
```

不支持 feedback cycle。

Graph Compiler 在提交 topology change 时执行 cycle detection。

未来如果确实需要 feedback DSP，再单独设计：

```text
Delay boundary
Feedback semantics
Minimum delay
```

不会通过放宽 DAG 约束直接加入。

---

# 21. 当前运行时设计方向

目前已经接受：

```text
LogicalGraph
    ↓
Graph Compiler
    ↓
ExecutionPlan
```

实时线程不遍历用户编辑结构，不进行：

```text
动态内存分配
Graph mutation
设备枚举
阻塞锁
复杂日志
同步 UI 操作
```

原始项目文档也已经明确提出 LogicalGraph 与编译后实时结构分离，以及通过 Atomic Swap 更新实时处理计划。

但以下内容仍需下一阶段继续设计：

```text
Buffer ownership
Buffer reuse
Buffer lifetime
fan-out zero-copy
in-place processing
Bus accumulation
Execution scheduling
RT parameter delivery
Graph swap
latency calculation
```

---

# 22. Block Size / Processing Quantum

这部分尚未正式定案。

已经讨论三种方向：

```text
A. 直接跟随设备 callback frame count

B. 固定 processing quantum，例如 128 / 256 frames

C. Variable Block Engine
```

当前比较有吸引力的是 C：

```text
process(frames)
1 <= frames <= MAX_BLOCK_SIZE
```

由 master device 当前需求驱动本次处理长度。

但这只是当前候选方案，还没有完成验证，也没有正式作为架构约束。

后续需要结合：

```text
WASAPI shared/exclusive
ASIO
VST3/CLAP block requirements
buffer planner
latency
```

一起决定。

---

# 23. Clock 模型

这里也尚未最终定案。

已经明确的一点：

> 不希望在用户可见的一张 Graph 内建立多个互相独立的 Graph Clock Domain。

当前候选模型是：

```text
一个 Processing Timeline
一个 Processing SR
一个 Master Clock
多个 device boundary clock
```

例如：

```text
USB Mic
 device clock A
     ↓
clock adaptation
     ↓
Processing Timeline
     ↓
Main DAC
 master clock

Processing Timeline
     ↓
clock adaptation
     ↓
HDMI
 device clock B
```

也就是说不同物理设备可以拥有独立 hardware clock，但这种差异尽可能在设备边界解决，而不是把 Graph 本身切成多个执行岛。

Output/Input Node 后续需要明确设备参数，例如：

```text
hardware sample rate
shared / exclusive
buffer / period
clock role
```

但 Master/Follower、async SRC、drift correction 具体机制还没有最终拍板。

---

# 24. UI 与数据结构分离原则

一个贯穿目前设计的原则：

> 数据结构不需要直接等于 UI 形状。

例如 Bus 数据层：

```text
Input0
Input1
Input2
Input3
```

Graph UI 可以只画：

```text
Game ─────┐
Music ────┤
Mic ──────┤ Bus ───→
Discord ──┘
```

无需把四个 socket 永久占满 Node 左侧。

同理 Send 参数属于 Edge 数据，但 UI 可以通过：

```text
edge overlay
inspector
mixer send controls
```

多种形式编辑。

Rack 内部可能存在许多 slot，但 Graph 只显示一个 RackNode。

这让底层保持严格，界面保持简洁。

---

# 25. 当前 Graph 核心不变量

截至目前，可以比较确定地写下：

1. LogicalGraph 是唯一用户可编辑 Graph。
2. ExecutionPlan 是编译结果，不是第二张 Graph。
3. Graph primitive 只有 Node / Port / Edge。
4. Bus 是 Node，不是设备。
5. Rack 是 Node，不是嵌套 Graph。
6. Port 携带多声道音频流，而不是单个 channel。
7. Input Port 最多一个 incoming Edge。
8. Output Port 可以拥有任意多个 outgoing Edges。
9. fan-out 属于 Graph 连接能力。
10. fan-in 由多 Input Port Node 显式实现。
11. 所有 outgoing Edge 地位相同，没有 Main Output。
12. Edge 可以携带 Send Gain / Pan / Mute / Pre-Post 参数。
13. Send 参数变化不属于 topology change。
14. Node 可通过统一 Gain/Pan 调整自身整体 routing result。
15. Rack 内 Processor 严格按 slot 顺序执行。
16. Rack 从底层支持 N input / N output。
17. Rack 内部不允许自由 routing。
18. Mixer 不反向推导 Graph processor chain。
19. 自动格式转换必须对用户可见。
20. 有损 channel conversion 需要显式用户意图。
21. 第一阶段 Graph 必须是 DAG。
22. Processing SR 支持 Auto / Manual。
23. Auto Processing SR 跟随 Master Device。
24. Port 数量不决定 realtime audio buffer 数量。
25. ExecutionPlan 可以融合 Send DSP、mixing 和 buffer binding。

---

# 26. 下一阶段待解决问题

下一轮建议从实时引擎核心开始，而不是继续扩 UI。

优先顺序：

```text
1. AudioBuffer 数据结构
2. Buffer Pool
3. Buffer ownership / lifetime
4. fan-out 如何真正 zero-copy
5. in-place DSP 的条件
6. Bus mixing 与 accumulation
7. Variable block 是否最终采用
8. ExecutionPlan 调度方式
9. 参数更新如何进入 RT thread
10. Graph recompilation / atomic swap
11. latency model
12. Processing SR 与 Master Clock 最终关系
13. 多设备 drift / resampling
```

完成这些以后，Moiren 的核心 realtime engine 架构基本就能落到代码接口层。
