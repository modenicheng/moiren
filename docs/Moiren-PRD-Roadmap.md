# Moiren 需求文档与功能规划

> 状态：Draft / 当前讨论基线  
> 更新日期：2026-10-08
> 文档目标：统一产品需求、功能边界、架构约束与开发顺序。本文优先采用最近一次已收敛设计；尚未验证或仍有争议内容统一标为“待定”。

---

## 1. 项目概述

### 1.1 项目定位

**Moiren — A real-time audio graph for Windows.**

Moiren 是面向 Windows 11 设计的开源实时音频图系统。它不试图复刻 Voicemeeter，也不直接移植 JACK 或 PipeWire；核心思路是利用 Windows 原生音频能力，从零建立一套可视化、可解释、可自由连接的实时音频路由与处理系统。

Moiren 希望解决一个长期存在的问题：Windows 可以播放、录制、混音，也允许应用选择设备，但缺少一张真正面向用户的“全局音频图”。用户很难直接回答：

```text
某个声音从哪里产生？
        ↓
经过哪些处理？
        ↓
和哪些声音混合？
        ↓
最终送到哪里？
```

Moiren 将这些关系统一表示为 Graph：

```text
Application / Device Input
          ↓
       Routing
          ↓
     DSP / Rack
          ↓
         Bus
          ↓
   Physical / Virtual Output
```

### 1.2 核心愿景

长期目标可以概括为四句话：

- 所有声音都可以看见。
- 所有路径都可以解释。
- 所有连接都可以修改。
- 内部逻辑总线无需伪装成 Windows 虚拟声卡。

Moiren 不是“另一个音量混音器”，也不是“很多虚拟 Cable 的管理器”。它应成为 Windows 桌面上统一处理应用流、硬件输入输出、Bus、DSP 与按需虚拟 I/O 的实时音频图。

---

## 2. 产品目标与非目标

### 2.1 产品目标

1. **Graph first**  
   用户直接编辑音频拓扑。Mixer、Routing Matrix 等界面只是同一份 Graph 状态的不同投影。

2. **Bus is not a device**  
   `Game Bus`、`Voice Bus`、`Stream Bus` 等对象默认只存在于 Moiren 内部。只有用户明确执行“Expose to Windows”时，相关信号才成为系统 Endpoint。

3. **Windows-native**  
   围绕 WASAPI、MMDevice、Process Loopback、MMCSS、后续 ASIO / WDK 等 Windows 能力正面设计，而不是套一层 Unix 音频模型。

4. **Realtime first**  
   实时线程不承担动态分配、设备枚举、文件 I/O、阻塞同步、Graph 修改、复杂日志或 UI 工作。

5. **Explainable routing**  
   自动 SRC、格式适配、时钟同步等行为可以自动完成，但不能悄悄发生。用户应能看到发生了什么转换、为什么发生。

6. **Progressive complexity**  
   普通用户可以通过拖线、Bus、音量等直观概念完成日常路由；专业用户可以继续控制采样率、缓冲、独占模式、多通道、时钟、ASIO 等参数。

7. **Native desktop experience**  
   第一版就是完整 GUI 应用，而不是 CLI、SDK 或 Web App。目标包括现代 HiDPI UI、较低空闲内存、快速启动，不依赖 Electron、Chromium、WebView2 或本地 HTTP 服务。

### 2.2 明确非目标

当前阶段不计划：

- 替代 Windows Audio Service；
- 强制所有系统音频都经过 Moiren；
- 首版直接实现内核虚拟声卡驱动；
- 首版完整覆盖 DAW 级工作站功能；
- 为了语言统一而强行使用 Rust 编写 Windows Kernel Audio Driver；
- 将 JACK2 / PipeWire 整体移植到 Windows；
- 让 Rack 内部再变成一张可任意分支、合流、递归嵌套的 Graph。

---

## 3. 目标用户与核心场景

### 3.1 普通高级用户

主要包括游戏玩家、主播、录屏用户、Discord / Teams 用户、多设备用户以及希望细分系统声音的人。

典型场景：

```text
Game ─────────────→ Headphones
  └───────────────→ Recorder

Music ────────────→ Speakers

Mic → Voice Rack → Voice Bus ─→ Monitor
                         └────→ Virtual Mic → Discord
```

这类用户不应先学习 WDM、KS、MME、Clock Domain、ASIO Buffer、A1/B2/VAIO 等历史术语才能开始使用。

### 3.2 专业音频用户

主要包括 DAW 用户、音乐制作人、录音用户、ASIO 用户与多通道设备用户。

长期需要覆盖：

- ASIO；
- Shared / Exclusive；
- Sample Rate / Buffer Size；
- 多通道与 Channel Mapping；
- 低延迟；
- XRUN / underrun / overrun 诊断；
- 设备占用状态；
- 硬件时钟与 drift；
- 多设备同步。

Moiren 不应粗暴抢占已由 DAW 使用的专业设备。设备状态至少要能表达 `Available / Owned by Moiren / Owned externally / Unavailable` 一类语义。

---

## 4. 产品核心模型

### 4.1 LogicalGraph 是唯一用户可编辑 Graph

当前正式模型：

```text
LogicalGraph
     │
     │ compile
     ▼
ExecutionPlan
```

`LogicalGraph` 用于：

- GUI 编辑；
- 项目持久化；
- 表达用户语义；
- 设备、Node、Port、Edge 等配置。

`ExecutionPlan` 用于：

- 实时调度；
- buffer binding；
- route binding；
- realtime parameter binding；
- latency metadata；
- format conversion metadata。

Compiler 可以使用临时 IR，但不长期维护第二张“ResolvedGraph / RealtimeGraph”。

### 4.2 Graph Primitive

核心 primitive 只保留：

```text
Node
Port
Edge
```

Bus、Rack、物理输入输出、应用输入、Recorder、Meter、Analyzer、Virtual Endpoint 等都属于不同 Node 类型，而不是新 Graph primitive。

### 4.3 Port 规则

一个 Port 表达一整条多声道音频流，而不是单独一个声道：

```text
Mono
Stereo
5.1
7.1
7.1.4
Discrete(N)
Custom(...)
```

核心不变量：

```text
Input Port  ← 0..1 Edge
Output Port → 0..N Edges
```

因此：

- fan-out 是 Graph 原生连接能力，不需要 Splitter Node；
- fan-in 由拥有多个 Input Port 的 Node 显式实现；
- 一个 Bus 可以动态增加多个独立 Input Port；
- 每条 Edge 永远连接一对明确 Port。

### 4.4 Graph Cycle

首阶段要求 LogicalGraph 为 DAG。

```text
LogicalGraph must be a DAG
```

Feedback 暂不通过放宽约束实现。以后如需反馈 DSP，应引入明确的 Delay Boundary、最小延迟与 feedback semantics。

---

## 5. Node 规划

### 5.1 Application Input Node

负责把某个 Windows 应用音频引入 Graph。

首阶段主要依赖 Process Loopback / WASAPI 相关能力。

至少需要区分两种产品语义：

1. **Capture / Monitor**：复制应用音频进入 Moiren，应用原本输出仍存在；
2. **Redirect / Takeover**：Moiren 接管应用声音，同时避免该应用继续直接输出到原设备。

第二种才是真正意义上的“路由应用声音”。当前仍有关键技术问题待解决：Process Loopback 本身偏向捕获，不天然等价于阻止原输出。与此同时，OBS 等其它软件可能并行捕获同一进程，Moiren 必须避免破坏其它捕获者所见信号或产生不可解释的双重路径。

因此，在 Redirect / Takeover 方案验证完成前，产品文案不能把普通 Process Loopback 宣称为完整重定向。

### 5.2 Physical Input Node

负责麦克风、Line In、音频接口等硬件输入。

首阶段：

- WASAPI Capture；
- 多声道描述；
- 设备格式适配；
- 输入 Meter；
- 设备失联与恢复。

后续：

- ASIO input；
- 多设备 clock adaptation；
- 更复杂硬件通道路由。

### 5.3 BusNode

Bus 是内部混音节点，不是 Windows Endpoint。

职责：

- 动态 Input Ports；
- fan-in mixing；
- routing；
- Gain / Pan / Mute；
- outgoing Sends；
- Meter。

示例：

```text
Game ─────→ Input 0 ┐
Music ────→ Input 1 ├→ [Stream Bus] ─→ Output
Mic ──────→ Input 2 ┘
```

Bus 后续可以导出到 Windows，但“导出”是独立系统集成功能，不改变 Bus 本身语义。

### 5.4 RackNode

Rack 已收敛为一等 Node，而不是每个普通 Node 都自带 Processor Rack。

```text
Application → RackNode → BusNode → Output
```

Rack 内部维持线性 Processor Chain：

```text
Main Input
    ↓
Slot 1
    ↓
Slot 2
    ↓
...
    ↓
Slot N
    ↓
Main Output
```

要求：

- Slot 顺序就是执行顺序；
- 支持 Built-in DSP；
- 后续支持 CLAP / VST3；
- 支持 RackPreset；
- Rack 内不允许自由 split / merge / branch；
- 底层从第一版支持 N input / N output；
- 普通 UI 默认只展示 Main In / Main Out；
- 插件需要时再暴露 sidechain、aux、多输出或 surround buses。

### 5.5 Output Node

物理输出必须通过显式 Output Node 表达，不在任意 Node / Bus 属性里暗藏“输出设备”。

这样 Graph 可以直接回答“声音最终去了哪里”。

Output Node 需要逐步支持：

- 设备选择；
- WASAPI Shared / Exclusive；
- Hardware Sample Rate；
- Buffer / Period；
- 多声道；
- Clock Role；
- Device availability；
- 后续 ASIO output。

### 5.6 Utility / Recorder / Meter / Analyzer

规划中的辅助节点：

- Recorder；
- Meter；
- Spectrum Analyzer；
- Channel Mapper；
- Resampler / Format Adapter（多数情况下由 Compiler 隐式插入并在 UI 暴露诊断）；
- 后续其它 Utility。

---

## 6. Routing 与 Mixer 需求

### 6.1 Edge / Send

Edge 除 topology 外允许携带轻量 Send 参数：

```text
SendParameters
├─ tap: Pre-Fader | Post-Fader
├─ gain
├─ pan
└─ mute
```

默认：

```text
tap = Post-Fader
```

所有 outgoing Edge 地位相同，不存在特殊 `Main Output`。

对于具有 `Routable` 能力的 Node：

```text
Node processing result
        │
        ├── Pre-Fader Tap
        │
        ▼
       Gain
        ↓
       Pan
        │
        └── Post-Fader Tap
```

Send 参数调整属于实时参数变化，不应触发整图重新编译。

### 6.2 Mixer View

Mixer 保留，但定位为 **routing/control projection**，而不是第二套拓扑编辑器。

```text
Graph = canonical topology
Mixer = routing/control projection
```

Mixer 主要展示 Routable Node：

- Meter；
- Gain；
- Pan；
- Mute；
- Sends。

Mixer 不扫描 Graph 并尝试把 `Game → Rack → Bus` 自动折叠成传统 DAW channel strip；Rack 内容由 Rack Node / Inspector 编辑。

### 6.3 Routing Matrix

作为中后期 UI，为大规模路由提供紧凑视图。它必须编辑同一份 LogicalGraph / Edge 数据，不建立独立状态。

---

## 7. DSP 与格式处理

### 7.1 内置 DSP

建议按实际需求逐步加入：

- Gain；
- Pan；
- Mute；
- Channel Mixer / Mapper；
- EQ；
- Gate；
- Compressor；
- Limiter；
- Delay；
- Resampler；
- Meter；
- Spectrum Analyzer。

不要求 MVP 一次完成全部 DSP。早期优先做验证 Graph、buffer、实时参数与节点调度所需最小集合。

### 7.2 Processing Sample Format

当前较稳定方向：

```text
Processing:
    f32  (默认)
    f64  (预留 / 高级)

I/O PCM:
    i16
    i24
    i32
    f32
    f64
```

是否允许整数成为内部 processing precision 尚未正式定案；当前不建议让 i16 / i24 污染 DSP 核心。

### 7.3 Format Conversion

兼容转换允许自动执行，例如：

```text
44.1 kHz → SRC → 48 kHz
```

但必须可见：

- Graph Edge / Node 状态标记；
- Inspector 中显示输入、输出格式；
- 解释由谁插入何种 converter。

存在明显语义损失的 channel conversion，例如 `5.1 → stereo`，默认不应悄悄完成。用户必须明确选择 Downmix / Channel Mapper 策略。

---

## 8. Processing Sample Rate 与时钟

### 8.1 Processing Sample Rate

用户侧名称统一为：

```text
Processing Sample Rate
Processing SR
```

模式：

- `Auto`：跟随 Master Device；
- `Manual`：使用用户指定 SR，设备边界执行 SRC。

### 8.2 Clock 模型

当前候选方向：

```text
一个 Processing Timeline
一个 Processing SR
一个 Master Clock
多个 Device Boundary Clock
```

目标是让 Graph 内部保持统一时间轴，把多硬件晶振差异留在设备边界处理。

非 Master 设备需要通过 buffer fill monitoring、async SRC / drift correction 等方式吸收 ppm 级漂移。

此部分仍属未完全定案内容，尤其包括：

- Master 选择与切换；
- 输入设备 adaptation；
- follower 输出 adaptation；
- drift estimator；
- 设备断连后的 timeline 行为；
- Manual Processing SR 与 Master Clock 的精确定义。

---

## 9. 实时引擎约束

### 9.1 RT Thread 禁止项

实时路径原则上禁止：

- 动态内存分配 / free / resize；
- 阻塞 Mutex；
- 文件 I/O；
- 设备枚举；
- Graph mutation；
- 同步 IPC；
- UI callback；
- 复杂日志格式化；
- 不可预测长耗时操作。

### 9.2 Buffer 模型

当前已收敛方案：

```text
ExecutionPlan
└── BufferArena<S>
    ├── BufferSlot<S>
    │   └── Box<[S]>  // planar: ch0 | ch1 | ...
    ├── BufferSlot<S>
    └── ...
```

所有权：

```text
ExecutionPlan
    owns BufferArena
        owns BufferSlot[]
            owns audio memory

AudioBlock / AudioBlockMut
    only borrow
```

要求：

- BufferSlot 在 prepare 后尺寸冻结；
- RT 中 0 allocation / 0 free / 0 resize；
- 每个 Slot 使用 planar 连续布局；
- Port 不等于物理 Buffer；
- BufferPlanner 根据 logical signal lifetime 复用 Slot；
- fan-out 应尽量 zero-copy；
- 支持安全条件下 in-place processing；
- Send gain / pan 可以融合进 Bus accumulation；
- Ring Buffer 与普通 working BufferSlot 使用不同数据结构。

### 9.3 Processor I/O

Processor 不直接获得 `&mut BufferArena`。

目标结构：

```text
ExecutionPlan
      ↓
Executor
      ↓
ProcessIo
      ↓
Processor
```

多 Slot borrow 如确实需要 `unsafe`，应只存在于经过验证的 Buffer Resolver 小边界内。Processor、DSP 与绝大多数代码保持 safe Rust。

### 9.4 Block Size

仍待最终验证：

- 跟随设备 callback frame count；
- 固定 quantum；
- Variable Block Engine。

当前偏向 Variable Block：

```text
process(frames)
1 <= frames <= MAX_BLOCK_SIZE
```

但正式决定前必须同时验证 WASAPI Shared / Exclusive、ASIO、CLAP/VST3 block 约束、buffer planner 与 latency。

---

## 10. Windows 音频后端规划

### 10.1 WASAPI — P0

首阶段主要后端：

- Capture；
- Render；
- Shared Mode；
- Event-driven；
- Process Loopback；
- 普通 Loopback；
- 后续 Exclusive Mode。

WASAPI 是 MVP 接入普通 Windows 应用与物理设备的基础。

### 10.2 ASIO — P1 / P2

ASIO 是长期核心能力，不只是“兼容插件”。

需要处理：

- Driver enumeration；
- 多通道；
- Buffer Size；
- Clock；
- 设备独占；
- DAW coexistence；
- 设备已被外部程序使用时的可解释状态。

### 10.3 MME / DirectSound / KS — 兼容层

不作为 Engine 核心模型基础。确有需求时作为 backend adapter 或兼容入口加入。

---

## 11. 虚拟音频设备与系统集成

### 11.1 核心原则

Virtual Endpoint 是 Graph 的导出机制，不是 Graph 本身。

```text
Voice Bus
   ↓
Expose to Windows
   ↓
Moiren Voice
   ↓
Discord / OBS / Game
```

取消 expose 后，对应 Endpoint 应可以消失，避免长期堆积十几个固定虚拟设备。

### 11.2 开发顺序

首版不开发 VAD。

核心稳定后再进入：

- Virtual Audio Driver；
- Dynamic Endpoint；
- Virtual Capture Endpoint / Virtual Microphone；
- Virtual Playback Endpoint；
- Bus / Node 按需导出；
- 统一桌面级虚拟入口。

驱动建议保持“薄”：

```text
Windows Audio Endpoint
        ↕
Shared Memory / IPC
        ↕
Moiren Engine
```

Graph、DSP、routing 继续运行在用户态。

实现语言可以采用 C / C++ + WDK，不要求内核层使用 Rust。

---

## 12. GUI 功能规划

### 12.1 Graph View — P0

核心界面，需要支持：

- 创建 / 删除 Node；
- 拖拽连接 Port；
- fan-out；
- Bus 动态输入；
- 选中 Edge 修改 Send 参数；
- Node Inspector；
- 转换 / 错误 / 设备异常状态可视化；
- 基本 Meter；
- 保存与恢复 Graph。

Graph 始终是 canonical topology。

### 12.2 Mixer View — P1

用于快速批量调节：

- Meter；
- Gain；
- Pan；
- Mute；
- Sends。

不承担 Rack 链自动折叠，不成为另一份状态。

### 12.3 Rack Editor — P1

用于：

- 添加 / 删除 Processor；
- 拖动改变顺序；
- Bypass；
- 参数编辑；
- Sidechain / Aux buses；
- RackPreset 保存与加载。

### 12.4 Device Manager — P1

统一查看和配置：

- WASAPI devices；
- 后续 ASIO；
- Shared / Exclusive；
- Sample Rate；
- Buffer / Period；
- Channel Layout；
- Clock Role；
- Latency；
- Device ownership / availability；
- 后续 Virtual Endpoints。

### 12.5 Routing Matrix — P2

面向复杂场景提供大规模连接总览与快速编辑。

---

## 13. 状态、配置与持久化

### 13.1 Project / Graph State

需要持久化：

- Node 与 Port；
- Edge；
- Send parameters；
- Bus 配置；
- Rack 与 Processor state；
- Output / Input 设备绑定；
- Processing SR；
- UI 布局中真正影响工作流的必要状态。

### 13.2 RackPreset

只保存：

- Processor 类型；
- 顺序；
- enabled / bypass；
- processor state / parameters。

默认不保存：

- Graph routing；
- Send destinations；
- 硬件设备绑定。

### 13.3 Scene / Profile

早期产品规划中已列入需求，但具体语义尚未收敛。

后续需要区分：

- `RackPreset`：处理链；
- `Scene`：一组实时参数 / routing 状态；
- `Profile / Project`：完整 Graph 与设备配置。

在数据模型明确前，不应让这三个概念互相覆盖。

---

## 14. 非功能需求

### 14.1 性能

目标：

- 低延迟；
- 低 jitter；
- RT thread 零动态分配；
- 充分利用 zero-copy / in-place；
- 多个 Port 不线性增加无意义 buffer；
- 默认处理精度 `f32`；
- 多声道和多设备场景保持可预测资源开销。

首轮性能指标暂不写死具体毫秒或内存数字，应先建立 benchmark 与 profiler 基线。

### 14.2 稳定性

至少需要覆盖：

- 设备热插拔；
- 应用退出 / 重启；
- 默认设备变化；
- Endpoint 不可用；
- Exclusive 设备被占用；
- Processor 失败；
- Graph compile 失败时继续运行旧 ExecutionPlan；
- 后续插件 crash isolation。

### 14.3 可诊断性

用户和开发者应能看到：

- Graph compile error；
- cycle；
- format mismatch；
- 自动 SRC；
- channel mismatch；
- device unavailable；
- xruns / underruns / overruns；
- latency；
- clock drift；
- 当前 active ExecutionPlan / revision（开发模式）。

---

## 15. 分阶段功能路线图

> 这里按依赖关系排，不强行绑定具体发布日期。

### M0 — 实时引擎骨架

目标：先证明核心 Graph 可以安全编译并稳定执行。

必须完成：

- `ProcessingSample` 基础抽象；
- `AudioBlock` / `AudioBlockMut`；
- `BufferSlot` / `BufferArena`；
- BufferPlanner；
- logical signal lifetime；
- fan-out zero-copy；
- in-place 判定；
- Bus accumulation；
- Send gain / pan 融合；
- `LogicalGraph → ExecutionPlan`；
- DAG / cycle detection；
- Executor / ProcessIo；
- RT parameter delivery；
- Plan prepare / swap / retire；
- fake source / processor / sink 测试；
- benchmark 与基本 RT diagnostics。

出口条件：不依赖真实 Windows 设备，也能稳定运行一张非平凡 Graph，并验证 buffer 生命周期、fan-out、mixing 和 plan swap。

### M0.5 — First Real Audio Path

目标：首次同时验证自动 Graph 编译、实时执行与真实 Windows 音频路径。本阶段先用 headless 入口完成设备选择和音频验收，完整 GUI 属于 M1。

交付顺序：

1. Graph Compiler 参考实现：Source/Sink/Gain/Bus/Pan、PostFader send 参数、自动独立槽位和 IO 绑定；已落地，优化 BufferPlanner 与 channel-strip PreFader 分别后续验收。
2. Test signal → Engine → 用户显式选择的单 WASAPI Shared 输出：首版 native 48 kHz/stereo/f32 已落地，FreeDSP 10 秒测试获用户试听确认；纯测试覆盖可变 demand、数据路径分配计数和停止唤醒。CPU 压力、反复启停和失联恢复另行验收，见 [Shared Render 记录](experiments/windows/2026-10-08-shared-render.md)。
3. Physical Capture / Process Loopback → Graph → 单输出；先单输入再混音。首次跨独立 capture/render 时同时实现有界 bridge、SRC 和填充量控制，不能等到多输出才处理时钟。
4. 运行中 Plan publish/swap/retire：固定 EngineConfig / epoch 的 engine API 已落地，控制侧准备、RT 块边界切换、非 RT 回收，并支持显式 DSP / 参数 ramp 迁移，见 [Compressor 与 Plan 切换](designs/06-compressor-plan-swap.md)。实际输出应用的换图入口和设备在线编辑另行接入；master 音频事件停止时仍须由 control wake 推进停机，已有 Shared stop 路径覆盖无音频事件的停止唤醒。

Takeover 可行性是并行 P0 Gate：验证原始输出抑制、session mute/volume 对捕获的影响、进程树隔离、OBS 共存和恢复。未通过时交付 Capture / Monitor，保留原始播放；不为实验自动修改默认设备或其他应用设置。

出口条件：通过 Compiler 建立真实输入 → Bus/基本处理 → 指定输出，可实时改 Gain/Pan/Mute，跨钟长时运行无持续积压，停止后释放资源。漂亮 GUI、虚拟设备、插件和 Named Pipe 不作为该阶段前置条件；进程隔离仍是后续部署选择。

### M1 — Windows 可用路由 MVP

目标：形成第一版真正可操作桌面应用。

必须完成：

- WASAPI physical capture；
- WASAPI render；
- Process Loopback 应用捕获；
- Application / Physical Input / Bus / Output Node；
- Graph View；
- 基本 Node Inspector；
- Gain / Pan / Mute；
- Meter；
- 多输出；
- Processing SR 基础设置；
- 兼容 sample-rate conversion；
- Project save / load；
- 设备断连与错误提示。

关键 Gate：

- 明确 Application Capture 与真正 Redirect / Takeover 的边界；
- 如果首版产品承诺“把某应用从默认输出改路由到其它地方”，必须先解决原始 render 抑制以及与 OBS 等第三方并行捕获共存问题。

### M2 — Mixer、Rack 与基础 DSP

目标：从“能路由”进入“能长期使用”。

规划：

- Mixer View；
- RackNode 编辑器；
- RackPreset；
- EQ；
- Gate；
- Compressor；
- Limiter；
- Delay；
- Channel Mapper；
- Meter / Analyzer 增强；
- Recorder；
- Scene / Profile 语义定稿；
- Routing Matrix（可按需求延后）。

### M3 — 专业音频与多设备

目标：解决真正专业或复杂硬件场景。

规划：

- WASAPI Exclusive；
- ASIO；
- 多通道；
- latency model / compensation；
- master clock；
- multi-device boundary clock；
- drift estimation；
- async resampling；
- xruns / timing diagnostics；
- DAW / external ownership 状态处理。

### M4 — Windows 虚拟 I/O

目标：让 Moiren Graph 可以按需向任意 Windows 应用提供设备接口。

规划：

- MoirenVAD；
- Dynamic Endpoint；
- Virtual Microphone；
- Virtual Playback Endpoint；
- Bus / Node `Expose to Windows`；
- endpoint 生命周期与权限；
- driver ↔ user-mode shared memory / IPC；
- 安装、升级、卸载与驱动签名流程。

### M5 — 插件与生态

规划：

- CLAP；
- VST3；
- plugin latency integration；
- fixed-block adapter；
- plugin sandbox / crash isolation；
- automation；
- snapshots；
- per-game profiles；
- remote control；
- network audio；
- JACK compatibility。

### 长期 / 暂不排期

- MIDI；
- 更复杂 surround workflow；
- feedback graph；
- network-distributed graph；
- 其它平台。

MIDI 当前优先级很低，不应影响早期音频引擎与 Windows 路由设计。

---

## 16. 当前最高优先级开发清单

2026-10-08 的首版 Compiler 已合入 `main@2fb754e`，补齐自动计划准备，详见 [Compiler 契约](designs/05-graph-compiler.md)。其后在 main 接入首条 `Test signal → Engine → WASAPI Shared` 实际输出，FreeDSP 10 秒实机测试获用户试听确认。W00 的 Process Loopback 与物理 capture/静音 render 仍作为独立实验；真实设备/应用输入尚未接入 Graph，M0.5 整体验收仍未完成。

推荐顺序：

1. 保留 Compiler 和单 Shared 输出的正确性基线；扩展格式、压力/重复启停与故障测试随后续切片推进。
2. Physical Capture / Process Loopback 与最小 Clock Bridge/SRC 联合完成稳定闭环。
3. Plan Swap 与资源生命周期、失联停机协议。
4. 最小 Slint Graph GUI、绑定、编辑、状态与项目保存。
5. 多输出、恢复与长期稳定性，形成日常可用 MVP。
6. Mixer/Rack/EQ 等操作与 DSP，再按需求推进 ASIO、VAD、CLAP/VST3。

Takeover 实验始终并行，不能以捕获成功代替重定向验收。槽位复用优化不阻塞首次真实音频；先保留无复用 Compiler 作为差分参照。单进程控制通道先服务上述闭环，Named Pipe 在明确跨进程需求后实施。

CI 常规矩阵覆盖整个 workspace 的编译、测试、Clippy 与格式检查，包括 Windows crate；Miri 继续覆盖 core/engine/app。CI 不打开音频设备。公开发行前仍需用户确定 LICENSE，并完成相应实机验收；本地通过不等于远程 CI 已执行。

---

## 17. 当前未决问题

### P0：会阻塞 MVP

1. **Application Redirect / Takeover**  
   如何在捕获某进程后避免它继续直接输出到原 Endpoint？如何与 OBS 等其它 Process Loopback 捕获者并存？

2. **Variable Block 的设备接入验证**
   Engine 已采用可变 block；后续验证 WASAPI demand 拆分、SRC 状态连续及插件固定 block 适配。

3. **ExecutionPlan swap / retire**  
   固定配置版本已采用单 pending SPSC、块边界交换和有界 retire，满队列继续旧计划，控制侧最终析构。跨 epoch / Processing SR 切换、crossfade 和实际设备编辑仍需后续协议。

4. **参数控制与计划迁移**
   参数 SPSC、ramp 与 ACK、Compiler 节点 / 边参数键已落地；换图可按显式旧 / 新 ProcessorId 映射迁移兼容参数当前值和 ramp。调用方由 NodeId / EdgeId 维护业务身份；多客户端调度与自动迁移策略仍待实现。

5. **Latency model**  
   Processor、SRC、设备边界、后续插件怎样统一报告并补偿延迟？

### P1：会影响专业能力

6. Processing SR 与 Master Clock 的最终关系；
7. 多输出 async SRC / drift correction 的深化（首次独立输入输出的最小实现属于 P0）；
8. ASIO 与外部 DAW 共存策略；
9. 多通道 channel layout / mapping 细节；
10. Scene / Profile / Project 的数据边界。

### P2：后续系统集成

11. Dynamic Virtual Endpoint 技术路线；
12. VAD 与用户态共享内存协议；
13. 插件 sandbox；
14. CLAP / VST3 优先顺序；
15. Network Audio / Remote Control。

---

## 18. MVP 功能边界建议

第一版公开 MVP 建议只承诺：

```text
Windows Application / Physical Input
                ↓
          Moiren Graph
                ↓
          Physical Output
```

并包含：

- 可视化 Graph；
- 应用与设备输入；
- Bus；
- 显式 Output Node；
- 基本 Gain / Pan / Mute；
- Meter；
- fan-out / fan-in；
- 多输出；
- 基本格式转换；
- 保存 / 加载。

不在首个 MVP 中承诺：

- 内核 VAD；
- 动态虚拟设备；
- ASIO 完整支持；
- VST3 / CLAP；
- 完整 Recording Studio；
- MIDI；
- Network Audio；
- feedback routing。

如果 Application Redirect 技术仍未解决，MVP 应明确把对应功能称为 **Application Capture / Monitor**，而不是“完全接管并重定向”。

---

## 19. 核心验收原则

任何新功能进入稳定版本前，都应至少满足以下原则：

1. **路由可解释**：用户能指出声音来源、路径、处理与去向。
2. **Graph 唯一真相源**：其它视图不能建立第二份拓扑状态。
3. **RT 安全**：实时路径无动态分配、无阻塞 I/O、无不可控同步。
4. **错误可恢复**：Graph compile 失败不能直接摧毁当前正常播放。
5. **隐式转换可见**：自动完成不等于偷偷完成。
6. **设备不是 Bus**：内部结构不污染 Windows Endpoint 列表。
7. **复杂度渐进**：简单任务不要求理解专业音频术语。
8. **专业能力不被阉割**：高级模式仍可控制格式、通道、时钟、缓冲与独占行为。

---

## 20. 一句话项目定义

**Moiren 是一个为 Windows 11 从零设计的开源实时音频图系统：它将应用音频、硬件 I/O、Bus、DSP、Mixer 与按需虚拟 Endpoint 统一进一张可视化 Graph，同时以实时安全、低延迟、可解释路由和专业音频兼容为底层约束。**
