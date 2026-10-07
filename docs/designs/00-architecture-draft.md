# Moiren

**A real-time audio graph for Windows.**

Moiren 是一个面向 Windows 11 的开源实时音频路由与处理软件。

它以节点图为核心，将应用程序、物理音频设备、总线、效果器、录音器以及按需暴露的虚拟音频设备统一进同一张实时音频图。

Moiren 不试图复刻 Voicemeeter，也不是 JACK2 或 PipeWire 的简单 Windows 移植。

它希望重新思考 Windows 桌面音频路由这件事本身。

---

# 一、项目动机

Windows 音频生态长期存在一个明显缺口。

Windows 原生音频系统擅长完成基础任务：

- 应用播放声音；
- 每应用音量控制；
- 输入输出设备选择；
- 系统级混音；
- Shared Mode / Exclusive Mode；
- WASAPI 等基础 API。

但它几乎没有提供一个真正面向用户的全局音频图。

用户很难直观看到：

```text
某个应用产生的声音
        ↓
经过了什么处理
        ↓
进入哪个总线
        ↓
最终去了哪个设备
```

也很难自由建立：

```text
Game ───────────────→ Headphones
  └────→ Recorder

Browser ─→ EQ ─────→ Speakers

Mic ─→ Gate ─→ EQ ─┬→ Discord
                    └→ Monitor
```

Windows 自带功能更接近：

```text
Application
     ↓
Windows Audio Engine
     ↓
Endpoint
```

而不是：

```text
Node
 ├→ Node
 ├→ Bus
 └→ Effect
      ↓
     Sink
```

---

# 二、现有方案的问题

## Voicemeeter / VB-Audio

Voicemeeter 功能成熟，实际可用性高，也证明 Windows 上复杂软件音频路由完全可行。

但长期使用中存在明显问题：

1. 偶发稳定性问题；
2. UI 设计历史较久；
3. 高 DPI 与现代 Windows 体验一般；
4. Donationware 提示可能打断音频引擎工作；
5. 路由模型围绕固定 A/B Bus 展开；
6. 安装后系统里出现大量长期存在的虚拟输入输出；
7. 内部音频总线和 Windows Audio Endpoint 高度耦合；
8. 软件闭源，架构和社区扩展空间有限。

尤其第 6 点非常重要。

用户只是想创建：

```text
Game Bus
Voice Bus
Music Bus
Stream Bus
```

但 Windows 最终看到的却可能是：

```text
Virtual Input
Virtual AUX Input
VAIO3
Virtual Output
AUX Output
...
```

内部逻辑结构被强行映射成系统设备。

Moiren 希望从架构层彻底避免这一点。

---

## JACK2

JACK2 提供了非常优秀的音频系统理念：

```text
client
port
connection
graph
real-time processing
```

它允许多个应用以端口形式接入，并在运行时自由连接。

这套思想对 Moiren 影响很大。

但 JACK2 在 Windows 上有几个天然限制：

- 普通 Windows 应用不是 JACK client；
- 浏览器、游戏、Discord 等无法自然进入 JACK graph；
- Windows 桌面集成较弱；
- 用户体验更偏专业音频基础设施，而不是完整桌面应用。

因此 Moiren 会借鉴 JACK2 的思想，但不会直接使用 JACK2 源码，也不会要求 JACK Server。

长期可以考虑提供 JACK compatibility layer。

---

## PipeWire

PipeWire 更接近 Moiren 最终希望达到的体验：

```text
Application Stream
Device
Virtual Node
DSP
Bus
        ↓
统一 Graph
```

尤其值得借鉴：

- 节点模型；
- 动态端口；
- 动态连接；
- session-level routing；
- 应用音频流本身就是图节点；
- 虚拟节点无需等价于物理设备。

但 PipeWire 当前实现深度依赖 Linux/Unix 基础设施：

```text
fd
Unix socket
SCM_RIGHTS
memfd
eventfd
epoll
udev
BlueZ
DMA-BUF
```

完整移植 Windows 会变成大型平台重构工程。

所以 Moiren 不采用：

> PipeWire for Windows

这条路线。

而采用：

> Windows-native audio graph system

即保留其优秀理念，但围绕 Windows API 从零构建。

---

# 三、核心愿景

Moiren 希望让 Windows 上所有声音都变成可以观察和连接的对象。

核心目标可以概括为：

**所有声音都可以看见。**

**所有路径都可以解释。**

**所有连接都可以修改。**

**所有内部总线都不需要伪装成 Windows 虚拟声卡。**

用户面对的不是大量设备名和历史术语，而是一张音频图。

例如：

```text
Applications             Processing              Buses              Outputs

Game ────────────────────────────────┬────→ Game Bus ─────→ Headphones
                                     │
Browser ─────→ EQ ─────→ Limiter ────┘

Mic ─────→ Gate ─────→ EQ ─────→ Voice Bus ─────→ Monitor
                                      │
                                      └────→ Virtual Mic ─→ Discord
```

---

# 四、最核心设计原则

## Bus 不是设备

这是整个项目最重要的架构原则。

> A bus is not a device.

用户创建：

```text
Voice Bus
Game Bus
Music Bus
Streaming Bus
Recording Bus
Sidechain Bus
```

这些应该只存在于 Moiren 内部。

它们不是 Windows Audio Endpoint。

Windows 系统音频设备列表不应该因此增加任何东西。

只有用户主动选择：

```text
Expose to Windows
```

某个节点或 Bus 才成为系统可见设备。

例如：

```text
Voice Bus
   │
   └─ Expose as Capture Endpoint
             ↓
       Moiren Voice
             ↓
           Discord
```

取消 expose 后，这个 endpoint 应该能够消失。

因此更准确的设计理念是：

> **Virtual endpoint is an export mechanism of the graph, not the graph itself.**

---

# 五、为什么这种设计在 Windows 上可行

现代 Windows 已经拥有比 Voicemeeter 最初设计时期更完整的 API。

Moiren 可以利用：

```text
WASAPI
MMDevice
Process Loopback
MMCSS
Software Device API
WaveRT
ASIO
```

尤其 Process Loopback 很关键。

它允许直接抓取某个应用的音频流，而无需先让应用播放到某个虚拟声卡。

因此可以直接构建：

```text
Chrome.exe ──────┐
Game.exe ────────┤
Spotify.exe ─────┼──→ Moiren Graph
Discord.exe ─────┘
```

而不是：

```text
Chrome
  ↓
Virtual Cable 1
  ↓
Router

Spotify
  ↓
Virtual Cable 2
  ↓
Router
```

只有当 Moiren 需要向外部应用提供一个输入设备时，才创建 Virtual Endpoint。

---

# 六、目标用户

Moiren 同时面向两类用户。

## 普通高级用户

包括：

- 游戏玩家；
- 主播；
- Discord / Teams 等语音用户；
- 录屏用户；
- 多音频设备用户；
- 音乐播放器用户。

他们可能只想实现：

```text
Game → Headphones
Music → Speakers
Discord → Headphones
Mic → EQ → Discord
Game + Mic → OBS
```

这类用户不应该被迫理解：

```text
WDM
KS
MME
ASIO buffer
Clock domain
A1
B2
VAIO
```

---

## 专业音频用户

包括：

- 音乐制作人；
- DAW 用户；
- 录音工程用户；
- ASIO 用户；
- 多通道音频用户。

他们需要：

```text
ASIO
Exclusive Mode
Sample Rate
Buffer Size
Clock Source
Multichannel
Channel Mapping
Low Latency
XRUN detection
Clock Drift Handling
Device Synchronization
```

Moiren 不能因为追求易用而牺牲这些能力。

因此 UI 应采用渐进式复杂度。

---

# 七、UI 设计方向

Moiren 从第一版开始就是完整 GUI 软件。

不是 CLI 工具。

不是 SDK。

不是 Web App。

明确不采用：

```text
Electron
Chromium
WebView2
Node.js
本地 HTTP Server
```

目标是：

```text
Native executable
Native Windows APIs
Modern HiDPI UI
Low idle memory
Fast startup
```

UI 主要包含几种视图。

## Graph View

核心视图。

类似模块化合成器 / node editor：

```text
[Game] ──────→ [Gain] ──────→ [Game Bus]
                                      │
                                      ↓
                               [Headphones]
```

用户可以直接拖拽连接。

---

## Mixer View

针对习惯传统调音台的用户。

每个节点或 Bus 可以呈现：

```text
Meter
Gain
Pan
Mute
Solo
Send
FX
```

这样既支持 Graph 思维，也保留专业音频用户熟悉的 Mixer 工作流。

---

## Routing Matrix

适合大规模路由。

```text
                  Outputs
              HP   SPK   OBS   Rec

Game          ●     ○     ●     ●
Browser       ●     ●     ○     ○
Mic           ●     ○     ●     ●
Discord       ●     ○     ○     ○
```

---

## Effect Rack

节点可以挂载效果链：

```text
Mic
 ↓
Noise Gate
 ↓
EQ
 ↓
Compressor
 ↓
Limiter
 ↓
Voice Bus
```

---

## Device Manager

统一管理：

```text
WASAPI
ASIO
MME
DirectSound
KS
Virtual Endpoints
```

普通用户看到简化模式。

高级用户可以展开：

```text
Sample Rate
Buffer Size
Shared / Exclusive
Clock
Channels
Latency
```

---

# 八、音频节点模型

整个 Moiren 都围绕：

```text
Node
Port
Edge
Graph
```

构建。

节点可能是：

```text
Application
Physical Input
Physical Output
Bus
Mixer
Effect
Recorder
Meter
Analyzer
Virtual Endpoint
Plugin
```

每个节点拥有若干：

```text
Input Port
Output Port
Control Port
```

节点之间通过 Edge 连接。

---

# 九、逻辑 Graph 与实时 Graph 分离

GUI 操作的 Graph 可以是动态、灵活的。

例如：

```text
Node
Edge
Port
HashMap
Vec
Dynamic State
```

但实时线程不能直接使用这一套结构。

因此 Moiren 应采用：

```text
LogicalGraph
     ↓
Graph Compiler
     ↓
CompiledGraph
```

CompiledGraph 包含：

```text
固定 Processing Schedule
预分配 Audio Buffers
节点执行顺序
端口映射
Channel Mapping
Latency Metadata
```

实时线程只执行：

```text
for node in schedule:
    process(node)
```

而不是每次 callback 重新遍历复杂动态图。

---

# 十、Graph 更新机制

用户修改图时：

```text
GUI
 ↓
Control Thread
 ↓
修改 Logical Graph
 ↓
Compile New Graph
 ↓
准备全部资源
 ↓
Atomic Swap
 ↓
Realtime Thread 使用新图
```

Realtime Thread 不参与 graph 构建。

这样避免：

```text
audio callback
   ↓
lock graph
   ↓
重新分配
   ↓
XRUN
```

---

# 十一、实时线程设计原则

Realtime Audio Thread 必须严格限制操作。

禁止：

```text
动态内存分配
阻塞 Mutex
文件 IO
设备枚举
同步 IPC
UI callback
复杂日志 formatting
不可预测系统调用
```

应尽量保证：

```text
read audio
 ↓
process
 ↓
mix
 ↓
write audio
```

所有 buffer 预分配。

所有处理计划预生成。

---

# 十二、线程模型

初步可以拆成：

```text
UI Thread
    │
    ▼
Control Thread
    │
    ├─ Device Manager
    ├─ Session Manager
    ├─ Graph Compiler
    └─ Persistence
          │
          ▼
      Atomic Graph
          │
          ▼
Realtime Threads
```

设备枚举、COM 初始化、ASIO 控制、文件保存等均不得进入 RT 路径。

---

# 十三、Windows 音频后端

## WASAPI

第一阶段主要后端。

支持：

```text
Capture
Render
Shared Mode
Exclusive Mode
Event-driven buffering
Loopback
Process Loopback
```

它也是普通 Windows 桌面应用接入的基础。

---

## ASIO

长期属于核心能力，而不是附加功能。

需要考虑：

```text
ASIO Driver enumeration
Multiple channels
Clock
Buffer size
Direct monitoring
DAW coexistence
Exclusive usage
```

尤其需要处理：

> 某个 ASIO 设备当前是否已经被 DAW 独占。

Moiren 不应粗暴抢占。

应该允许：

```text
DAW controls ASIO
Moiren stays away
```

或者后续通过兼容层实现更复杂共存。

---

## MME / DirectSound / KS

这些主要用于兼容性。

Moiren 内部 engine 不应围绕这些 API 设计。

它们属于 backend adapter。

---

# 十四、设备时钟问题

这是项目从早期就必须考虑的一层。

如果用户同时连接：

```text
USB DAC A
USB Interface B
HDMI Audio C
```

它们拥有不同硬件时钟。

即使都声称：

```text
48000 Hz
```

实际可能是：

```text
47999.1
48001.3
48000.4
```

长时间运行一定发生 drift。

因此 Moiren 后期必须拥有：

```text
Clock Domain
Master Clock
Adaptive Resampling
Drift Estimation
Buffer Fill Monitoring
```

否则多设备路由迟早爆音或积累延迟。

---

# 十五、DSP 与效果链

内置效果第一阶段可以包括：

```text
Gain
Pan
Mute
Channel Mixer
EQ
Gate
Compressor
Limiter
Delay
Resampler
Meter
Spectrum Analyzer
```

这些组件需要支持实时安全处理。

长期支持：

```text
CLAP
VST3
```

其中 CLAP 很值得优先考虑，因为接口较现代，也更适合开源生态。

---

# 十六、虚拟音频设备

Moiren 第一阶段不需要开发驱动。

第一版可以做到：

```text
Application
Mic
Line In
   ↓
Graph
   ↓
Physical Outputs
```

已经具备大量价值。

等核心稳定后，再实现：

```text
Graph Bus
   ↓
Expose to Windows
   ↓
Virtual Playback / Capture Endpoint
```

---

# 十七、Virtual Audio Driver

未来如果实现 VAD，建议：

```text
Moiren.exe
    Rust

MoirenVAD.sys
    C / C++ + WDK
```

不强求全项目 Rust。

Windows Kernel Audio Driver 生态目前仍以：

```text
WDK
WDM
WaveRT
SysVAD
```

为主。

为了语言统一强行使用 Rust 没有必要。

驱动职责应该尽量薄：

```text
Windows Audio Endpoint
        ↕
Shared Memory / IPC
        ↕
Moiren Engine
```

实际 graph、DSP、routing 都继续运行在用户态。

---

# 十八、动态 Endpoint

长期目标不是一次安装十几个固定虚拟声卡。

而是：

```text
用户点击：
Expose Voice Bus

Windows 出现：
Moiren Voice
```

然后：

```text
Disable Exposure

Moiren Voice 消失
```

这样 Windows 音频设备列表保持干净。

---

# 十九、技术栈

当前方向：

```text
Rust
```

用于：

- Audio Engine；
- Graph；
- DSP；
- Windows API；
- Device Management；
- Control Plane；
- GUI Application。

Windows API 主要考虑通过：

```text
windows crate
```

直接访问 COM / WASAPI / MMDevice 等接口。

早期可以适度使用：

```text
wasapi crate
```

快速验证，但核心架构不要绑死第三方封装。

---

# 二十、GUI 技术

当前倾向：

```text
Slint
```

原因：

- native executable；
- 不依赖 WebView；
- 没有 Chromium；
- 较低运行时开销；
- 适合自绘音频界面；
- Rust 集成自然；
- HiDPI 支持较好；
- 适合 Meter、Fader、Graph、Node、Cable 等自定义 UI。

Moiren 并不要求所有控件都是传统 Win32 HWND。

所谓原生主要指：

```text
Native binary
Native APIs
No browser runtime
Low memory overhead
Proper DPI support
```

---

# 二十一、项目模块设计

初步目录可以是：

```text
moiren/
├── crates/
│   ├── moiren-core/
│   ├── moiren-graph/
│   ├── moiren-engine/
│   ├── moiren-dsp/
│   ├── moiren-wasapi/
│   ├── moiren-asio/
│   ├── moiren-platform-windows/
│   └── moiren-app/
│
├── ui/
│
├── driver/
│   └── moiren-vad/
│
└── docs/
```

但初期不要为了“架构漂亮”拆太细。

更现实的第一阶段可以只有：

```text
crates/
├── core
├── engine
├── windows-audio
└── app
```

等职责真正稳定后再拆。

---

# 二十二、第一版 MVP

MVP 不应该一开始挑战虚拟声卡驱动。

第一阶段只解决：

```text
Windows Applications
Microphone
Physical Inputs
        ↓
Moiren Graph
        ↓
Physical Outputs
```

核心功能：

```text
WASAPI Capture
WASAPI Render
Process Loopback
Graph View
Node Routing
Gain
Basic Mixer
Basic EQ
Meters
Multiple Outputs
Scenes / Profiles
```

这已经足够形成一个真正可用产品。

---

# 二十三、第二阶段

核心 graph 稳定以后加入：

```text
ASIO
Advanced Mixer
Effect Rack
More DSP
Recording
Latency Compensation
Clock Domains
Adaptive Resampling
```

---

# 二十四、第三阶段

再进入系统级集成：

```text
Virtual Audio Driver
Dynamic Endpoint
Expose Bus to Windows
Virtual Microphone
Virtual Playback Device
```

这样才能实现：

```text
Moiren Bus → Discord
Moiren Bus → OBS
Moiren Bus → Game
```

而且不需要永久堆积大量虚拟设备。

---

# 二十五、长期功能

长期可以考虑：

```text
CLAP
VST3
JACK Compatibility
MIDI
Network Audio
Remote Control
Automation
Snapshots
Scene Switching
Per-game Profiles
Plugin Sandbox
Crash Isolation
```

---

# 二十六、与 Windows Audio 的关系

Moiren 不打算替代 Windows Audio Service。

它更像位于其上层和侧面的高级 routing layer。

普通应用仍然可以使用：

```text
WASAPI
MME
DirectSound
```

Moiren 负责：

```text
捕获
重新路由
处理
混音
输出
```

需要时通过 VAD 向 Windows 暴露新的 endpoint。

这样可以尽量避免与 Windows 内部音频架构正面冲突。

---

# 二十七、与 DAW 的关系

Moiren 不应该假定：

> 所有声音都必须经过 Moiren。

专业音频环境尤其如此。

例如用户启动 Studio One：

```text
Studio One
    ↓
ASIO
    ↓
Audio Interface
```

这是完全合理的。

Moiren 应该允许设备处于：

```text
Available
Owned by Moiren
Owned externally
Unavailable
```

而不是粗暴抢占。

长期可以考虑：

```text
ASIO bridge
JACK compatibility
Shared professional graph
```

但这些属于高级能力。

---

# 二十八、项目定位

Moiren 不是：

```text
another volume mixer
another virtual cable
another Voicemeeter skin
another Equalizer APO frontend
another JACK frontend
```

它更接近：

> 一个面向 Windows 桌面的实时音频图系统。

它同时拥有：

```text
Routing
Mixing
DSP
Application Capture
Professional Audio
Dynamic Virtual I/O
```

---

# 二十九、品牌与名称

项目正式名称：

# Moiren

名字来自对“线、编织、路径、命运之线”这些意象的抽象。

它对应项目核心视觉：

```text
声音不是设备列表中的条目。

声音是一条流动的线。

节点决定它如何：
分岔
交汇
处理
延伸
抵达
```

因此 Moiren 的视觉设计可以围绕：

```text
Thread
Path
Intersection
Flow
Node
Weave
```

展开。

无需使用传统：

```text
音符
耳机
扬声器
波形
```

作为主要品牌符号。

---

# 三十、推荐副标题

主副标题：

**Moiren — A real-time audio graph for Windows.**

更面向用户：

**Route, mix and process every sound on Windows.**

更偏项目愿景：

**A modern open audio graph for Windows.**

---

# 三十一、项目原则

Moiren 可以长期坚持以下原则：

1. **Graph first**
   
   Mixer 是 Graph 的一种视图，不是底层模型。

2. **Bus is not a device**
   
   Windows Endpoint 只是 Graph 的导出接口。

3. **Realtime first**
   
   所有架构首先服从实时音频要求。

4. **Native first**
   
   不依赖浏览器运行时。

5. **Open first**
   
   核心能力不依赖商业虚拟声卡或闭源路由组件。

6. **Windows native**
   
   不照搬 Linux 音频体系，而是围绕 Windows API 正面设计。

7. **Progressive complexity**
   
   普通用户无需理解专业术语，专业用户又能控制全部底层参数。

8. **No permanent device spam**
   
   虚拟设备只在真正需要时存在。

9. **Professional compatibility**
   
   ASIO、DAW、Exclusive Mode、硬件时钟从早期就纳入架构考虑。

10. **Explainable routing**
    
    用户永远应该能够知道某条声音从哪里来、经过什么、最终去了哪里。

---

# 三十二、一句话总结

Moiren 是一个为 Windows 11 从零设计的开源实时音频图系统。

它希望将应用程序、游戏、麦克风、专业音频设备、DAW、效果器、总线与按需虚拟设备统一到同一个可视化节点图中，同时保持低延迟、低资源占用、现代原生 UI，以及对 Windows 专业音频生态的完整兼容能力。

最终目标不是再做一个虚拟调音台。

而是让 Windows 上的声音真正成为一张可以自由连接的图。