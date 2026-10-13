# 引擎托管生命周期验收记录

日期：2026-10-14。独立工作树分支：`feat/engine-managed-lifecycle`；合并目标：原工作区的 `feat/application-runtime-signals`。不修改现有 UI，不提交其并行改动。

## 后端交付

| 里程碑 | 实现与证据 |
| --- | --- |
| T1 项目模型 | `EngineProject` 持有真实 `LogicalGraph`、节点配置与期望参数；完整事务验证、失败回滚、容量限制及无输出 Dormant，由 `tests/host/project.rs`、`lifecycle.rs` 验证 |
| T2 生命周期 | `EngineHost::open` 自动准备与启动；唯一宿主 Drop 自动取消、停止、join 和非 RT 回收，控制/观察句柄不延长运行；独立准备/回收通道及关闭竞态由 `lifecycle.rs`、`updates.rs` 验证 |
| T3 复用 | IO 端点以元数据占位、不复制 SPSC reader/writer，精确 active basis 与 sink 迁移守卫；source/sink、Compressor 历史、节点/边 ramp、信号权限及 RT 无堆操作由 `tests/runtime/{swap,update,signals}.rs` 验证 |
| T4 自动更新 | 编辑后自动准备、合并连续期望、丢弃过时完成项、块边界确认后推进 active；稳定目标参数路由与回执终结、自动信号绑定由 `tests/host/{updates,parameters,signals}.rs` 验证 |
| T5 原生 owner | Windows adapter 不依赖 app；capture/render 共用激活 gate，attach 不启动，协调器确认后 activate；失败/panic 仍返回 Engine，由假原生 owner 与 renderer handoff 测试验证 |
| T6 热更新 | 同一 owner 保留输出消费者，保护 sink 精确复用；revision/epoch/frame 一致有界观测；原生兼容更新和不兼容配置自动换 owner 由静音硬件测试验证 |
| T7 资源恢复 | 配置关联的 generation 事件、显式设备与默认角色区分、PID + creation identity、有界可取消串行 probe；失效 owner 回收、恢复、新选择不受旧事件阻挡由 `tests/host/resources.rs`、适配器 resource 测试验证 |
| T8 后端部分 | `examples/managed_graph.rs` 不使用 UI，打开 → 参数修改 → 插入 Gain → 自动激活 → 自动关闭；engine/Windows README 说明公共 API 与低层兼容入口 |

应用 Service/UI 采用不在本轮后端交付范围。保留低层离线 `Engine`/compile/plan ports，不要求普通托管客户端手动管理它们。

## 自动验证

独立工作树使用自己的 `CARGO_TARGET_DIR`，避免并行工作树共享构建产物。最终验证在纳入目标分支已提交的时钟/延迟改动之后执行：

```text
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --locked --workspace
cargo run --locked -p moiren-engine --example managed_graph
```

以上命令均在合入目标 `860f096` 后以退出码 0 完成；两项原生静音用例同样通过。远程 CI 未触发。工作流新增 `managed_graph` 示例检查，既有 workspace 和双模式 Miri 检查继续保留。

宿主测试 25 项，runtime 测试 56 项。新增回归包括：退役 candidate 不得再发布、复用失败不得丢失已接受控制请求、关闭返回 pending candidate、输出槽位迁移后的复用保护、配置 A→B 后忽略 A 的旧资源事件。

Linux-target Miri 命令（默认及 `-Zmiri-tree-borrows` 分别执行）：

```text
cargo +nightly miri test --target x86_64-unknown-linux-gnu --locked -p moiren-engine --test host
cargo +nightly miri test --target x86_64-unknown-linux-gnu --locked -p moiren-engine --test runtime update
```

`update` 过滤包含 14 项 IO 复用与相关信号回归。完整 core Miri 尝试因既有 65,536-port 大图用例耗时过长而主动中止；不声称完整 workspace Miri 或远程 CI 已通过。Windows-target Miri 的 `System` 对齐释放报告在未修改的既有 swap 分配跟踪测试上也能复现，因此使用 CI 所采用的 Linux 目标验证新增行为，不修改无关分配器。

宿主 25 项及上述 update 14 项在两种模式下均通过。进一步扩大到 runtime 全量时，默认模式为 55 通过、1 失败：未修改的 `meter_and_compressor_demand_transitions_keep_full_dsp_and_zero_heap_work` 把浮点结果逐位比较，Miri 中结果相差约 1e-16。在原工作区未合入本分支的基线代码上独立复现同一失败，未修改这个无关测试。Tree Borrows 扩大检查时显式排除此已确认的基线失败，不把这次排除后的运行称为完整 Miri 通过。

验证期间发现并修复了测试自身的快照时序假设：owner 已停止不代表协调器已发布 Closing；测试等待快照确认。既有未修改的 app `signal_service` 首次全工作区运行出现一次即时快照为空，独立复跑及随后完整运行通过，未对应用测试做范围外修改。

## 静音硬件验收

```text
cargo test --locked -p moiren-windows-audio --test engine_backend -- --ignored --test-threads=1 --nocapture
```

实际执行两个独立硬件用例，音频图全程输出 gain 为 0；不更改系统默认设备，不保存音频：

- 默认 Multimedia 输出：启动 `owner=1/epoch=1/plan=1`，插入节点后仍为 `owner=1/epoch=1`，推进到 `plan=2`；将项目选择改为 Console 后自动成为 `owner=2/epoch=2/plan=3`。Drop 后 `Closed`，`fault` 与 `cleanup_error` 均为空。
- 默认 Console 物理输入 → Gain(0) → 默认 Multimedia 输出：capture/render 一起达到 Running，时间线持续推进；Drop 后两侧自动结束，关闭无故障或回收错误。

这些用例验证真实 WASAPI 自动启动、热更新和拥有权闭合；Console 与 Multimedia 可能解析为同一个物理设备，所以不把角色修改称为实际设备拔插或切换硬件。设备拔插、独占占用解除、真实进程退出/重启和不同物理设备切换未在本次操作中执行；对应状态与资源逻辑有可控回归，但不冒充硬件证据。

Windows 初始适配保持现有传输能力：单个共享 48 kHz stereo 输出、多个跟随输入（支持的物理/进程捕获格式经已有 clock bridge 适配）及 software tone。通用宿主契约不受该平台首版能力或 UI 限制；不支持的传输配置明确报错并保留项目。
