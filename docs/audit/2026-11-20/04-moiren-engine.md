# moiren-engine 架构审计（2026-11-20）

基线：main @ `d8610e05`。方法：主代理通读 crate 指南与 `host.rs`、`host/backend.rs`、`host/supervisor.rs`、`compiler.rs`、`runtime.rs`、`processor/mod.rs`、`threads.rs`，并抓取全部 107 个模块的 `//!` 文档行；只读审计。

## 职责摘要

实时执行、非实时编译与托管宿主；传输层在 `moiren-windows-audio`。`compiler` 把 `LogicalGraph` 降级为不可变 prepared plan（含 fused Bus 混合与基于生命周期的 buffer 规划），`EngineHost` 拥有编译/激活/参数路由/信号挂接的全部所有权，`runtime::Engine` 在音频线程线性执行 plan。f32/f64 双精度泛型。

## 内部分层（97 文件，六层）

| 层 | 模块 | 职责 |
|---|---|---|
| 宿主层 | `host.rs` + `host/`（supervisor、preparation、retirement、parameters、signals、snapshot、project、edits、backend） | `EngineHost` 唯一非 RT 生命周期 owner；supervisor 控制线程状态机；prepare/cleanup 双 worker lane；backend 契约 |
| 编译层 | `compiler.rs` + `compiler/`（bindings、plan、storage、send、update、observers） | 校验→降级→存储规划→参数通道→ExecutionPlan；兼容更新 `compile_update`；`compile_reference` 对拍 oracle |
| 运行时层 | `runtime.rs`、`runtime/swap.rs`、`runtime/mix.rs`、`buffer.rs`、`control.rs` | `Engine::render` 线性执行；plan 在渲染边界有界所有权转移；fused Send+Bus 内核；参数 SPSC 通道 |
| 处理器层 | `processor/`（builtin 13 内核 + plugin 适配） | `RtProcessor`/`RtObserver` 安全 DSP 契约；插件经 f32 staging 桥接 |
| 观测内容层 | `meter.rs`、`spectrum.rs`、`recorder.rs`、`signal/`、`rack.rs` | demand-gated 表头/频谱；录音走有界 SPSC + 文件线程；Rack 控制侧展开 |
| 基础设施 | `threads.rs`、`boundary.rs`（+bridge）、`node.rs`、`audio_config.rs`、`sample.rs` | 进程级线程注册表；设备无关流契约与时钟桥；IO 节点准备；f32/f64 |

## 图 1：crate 总览分层

```mermaid
flowchart TB
    subgraph HOST[宿主层 控制线程]
        EH[EngineHost 生命周期 owner]
        SUP[supervisor 状态机]
        PREP[preparation worker]
        RET[retirement worker]
        SIG[signals SignalHub]
    end
    subgraph COMP[编译层 非RT]
        CMP[compiler 降级与存储规划]
        UPD[compiler-update 兼容更新]
    end
    subgraph RT[运行时层]
        ENG[runtime Engine 渲染循环]
        SWAP[runtime-swap plan 切换]
        BUF[buffer BufferArena]
        CTL[control 参数通道]
    end
    subgraph PROC[处理器层]
        BUILTIN[builtin DSP 内核 13 种]
        PLUG[plugin 适配 f32 staging]
        RTP[RtProcessor 契约]
    end
    subgraph INFRA[基础设施]
        THR[threads 注册表]
        BRD[boundary 流契约与时钟桥]
        NODE[node IO 准备]
    end
    EH --> SUP --> PREP & RET
    SUP --> CMP & UPD
    CMP --> ENG
    ENG --> SWAP --> BUF & CTL
    ENG --> RTP
    BUILTIN & PLUG -->|实现| RTP
    SIG -.订阅分发.- RT
    THR -.登记.- HOST & RT
    BRD -.设备无关边界.- RT
```

## 图 2：编译流水线（控制侧，一次性）

```mermaid
flowchart LR
    subgraph IN[输入]
        LG[LogicalGraph 来自 moiren-core]
        NB[NodeBindings 来自 backend prepare]
        CFG[CompileConfig 预算与身份]
    end
    subgraph PIPE[compile_inner]
        V[graph-validate + 边语义检查 PreFader 与立体声 pan]
        PB[plan-build NodeKind 降级为 op 计划]
        SO[storage-optimize 生命周期 buffer 规划 原地化]
        ARENA[BufferArena 字节预算内分配]
        POPS[prepare_operations 绑定 arena IO]
        PCH[parameter_channel SPSC 表]
        XP[ExecutionPlan 含信号 PreparedSignalPlan]
    end
    OUT[CompiledGraph：Engine + ControlPort + CompiledBindings + Stats]
    LG & NB & CFG --> V --> PB --> SO --> ARENA --> POPS --> PCH --> XP --> OUT
    OUT -.对拍.- REF[compile_reference 每边一处理器 oracle 路径]
```

## 图 3：EngineHost 生命周期与更新语义

```mermaid
flowchart TB
    subgraph CLIENT[客户端 非 owner]
        EC[EngineControl 提交 edit-parameter]
        EO[EngineObserver 读快照与订阅]
    end
    subgraph SUPV[moiren-host-control 控制线程 supervisor]
        CMD[有界命令通道 wake Condvar]
        ST{状态机 Active-Candidate-preparing-blocked}
        RTR[parameters Router 路由到活动表]
        RESC[poll_resource 设备到达与缺席]
    end
    subgraph LANES[工作 lane 各自有界通道]
        PW[preparation worker 编译与 prepare]
        CW[retirement worker 退役回收]
    end
    subgraph NATIVE[原生 owner 由 backend 提供]
        BT[PreparedTransport-attach]
        BO[BackendOwner activate-status-timeline]
        SJ[stop_and_join 有界拆解]
    end
    EC --> CMD --> ST
    ST -->|兼容更新 块边界换 plan| RTR
    ST -->|不兼容 重建 owner 新 epoch| PW
    PW --> BT --> BO
    BO -->|timeline 帧对齐| RTR
    ST -->|退役 RetiredPlan 与 owner| CW
    CW --> SJ
    ST -->|发布快照 requested-accepted-applied| EO
```

## 图 4：RT 渲染块路径（音频线程，零分配）

```mermaid
flowchart TB
    CB[音频回调进入 render-frames]
    APPP[apply_pending_plan 应用已发布的新 plan]
    BB[signal host begin_block]
    SEG{参数事件分段循环 max-events-per-block 有界}
    OP1[Processor 算子 arena-with-io + 参数视图 + 信号 scope]
    OP2[Mix 算子 fused Send+Bus]
    ADV[parameters-advance 与 timeline 推进]
    RR[RenderReport 返回 start-end-segments-events]
    CB --> APPP --> BB --> SEG
    SEG -->|段内逐 op| OP1 & OP2 --> ADV -->|下一段或结束| SEG
    SEG -->|完成| RR
    subgraph SWAPLANE[块间切换 由控制侧预发布]
        PCP[PlanControlPort publish]
        RETP[RetiredPlan 回控制侧销毁]
    end
    PCP -.渲染边界原子生效.- APPP
    RETP -.-> CW2[retirement worker 非 RT 回收]
```

## 图 5：处理器与参数通道组织

```mermaid
flowchart LR
    subgraph CTL[控制侧]
        PW2[参数 worker IPC 解码与合并]
        OWNER[ControlPort 唯一控制 owner]
        TABLE[RT 参数表 revision-epoch 绑定]
    end
    subgraph RTSIDE[实时侧]
        SPSC[有界 SPSC 队列]
        VIEW[ProcessParameters 每算子绑定视图]
        PROC[RtProcessor-process ctx-io-params-signals]
        OBS[RtObserver-observe Meter-Spectrum-Recorder]
    end
    subgraph KERNELS[内核清单 builtin]
        K1[Gain-Pan-Balance-Swap]
        K2[Bus-sum 融合混合]
        K3[EQ-Gate-Compressor-Limiter dynamics]
        K4[Delay-ChannelMapper-Strip]
        K5[PluginProcessor f32 桥接]
        K6[Rack 展开为既有内核链]
    end
    PW2 --> OWNER --> SPSC --> TABLE --> VIEW --> PROC
    PROC --> OBS
    KERNELS -.实现 RtProcessor.- PROC
```

## 架构观察

| # | 现象 | 证据 | 影响 |
|---|---|---|---|
| 1 | 三层分离与 RT 契约在类型层强制：plan/资源/参数表都有身份互检 | `runtime.rs:426-437`（`ForeignResources`/`ForeignParameters`）；`compiler.rs:204-273` 流水线 | 跨 plan 误用资源在编译期外的第一道口子被拒；「Rust 内存安全不保证实时安全」的补丁打得系统化（`processor/mod.rs:1`） |
| 2 | 身份体系五元分离落到具体字段 | `host.rs:158-169`（desired/active revision、plan revision、owner/resource generation）；`host/backend.rs:114-122`（PrepareRequest 携带三元身份）；`backend.rs:181-185` OwnerTimeline epoch+revision+frame | requested→accepted→applied 有真实数据结构支撑，UI 侧无需伪造；epoch 与 revision 不互冒充 |
| 3 | supervisor 单线程状态机 + 两个 worker lane，全部通道有界 | `host/supervisor.rs:88-91`（sync_channel(1) prepare、cleanup_capacity 清理）；主循环 `supervisor.rs:184-261` | 控制面无失控队列；「准备与回收分 lane」的规则在结构上不可违反 |
| 4 | 兼容/不兼容判定集中在两个纯函数 | `supervisor.rs:341-360` transport_compatible（engine+IO 集合）、`supervisor.rs:361-381` same_structure | 判定语义单点可审；strip 值变化走参数不重编译（`supervisor.rs:361-364` 注释） |
| 5 | 退役路径全程离 RT | `runtime.rs:494-497` retire_controls、`swap.rs:816-851` RetireOutcome/RetiredPlan、`supervisor.rs:305-333` Cleanup::Owner 整包移交 | 符合「退役对象回控制侧销毁」；#105 卡死端点的有界弃置语义在 `backend.rs:187-209` 有明确文档与 None 通道 |
| 6 | 观测类处理器的计算被刻意移出 RT | `spectrum.rs:1-4`（FFT 在读侧）；`meter/signal.rs:1`（demand-gated 精确窗口）；`recorder/session.rs:1-3`（文件线程） | RT 半边只留有界拷贝与发布；与「Meter 非 LUFS」的边界声明一致 |
| 7 | 线程注册表父侧登记 + enter 一次原子写 | `threads.rs:9-26`、`threads.rs:128-134`、`threads.rs:176-183` | RT 线程注册路径零锁零分配；app 性能面板行行可归因到 spawn 点 |
| 8 | `runtime/swap.rs` 是最大的单文件（43KB） | 文件清单排序第一 | plan 切换/复用/快照逻辑集中度偏高，属可接受的内聚，但新增切换语义时是主要审读面 |
| 9 | 编译器保留 reference 对拍路径 | `compiler.rs:159-167`、`compiler.rs:1-2` | 优化 lowering（fused/in-place）始终有正确性 oracle，测试策略成熟 |
