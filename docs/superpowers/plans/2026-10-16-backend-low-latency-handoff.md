# Backend / Low Latency Handoff

更新时间：2026-10-16。工作目录 `D:\coding\moiren`，PowerShell，分支 `feat/ui`。
续作开始 HEAD 为 `6003f22`，提交整理前为 `12fc6a0`，期间新增提交来自并行前端任务。

用户要求直接推进后端至低延迟，并已确认并行前端修改属于预期，允许继续后端。
核心接入已完成；续作补齐连续 Low 长测、需求缺口模型和有界异常诊断。
FreeDSP + CABLE 组合未通过 Low 无恢复资格，不能将控制层 healthy 当作音频连续。

## 先读这些文件

1. [验收记录](2026-10-16-managed-routing-low-latency-validation.md)：最新测试结果、实机数值、已知限制和截图。
2. [设计说明](../../designs/14-managed-routing-low-latency.md)：所有权、主 / 跟随输出、档位预算和观测含义。
3. [实现计划](2026-10-15-managed-routing-low-latency.md)及[设计规格](../specs/2026-10-15-managed-routing-low-latency-design.md)。
4. [实机证据 JSON](../../designs/evidence/2026-10-16-managed-routing-low-latency.json)：从真实运行提取的字段，包含原始日志 SHA-256；从被忽略的实验目录原样复制到可跟踪目录。
5. [连续运行资格验证](2026-10-16-low-latency-qualification.md)及[续作证据](../../designs/evidence/2026-10-16-low-latency-soak.json)：600 秒长测、需求缺口模型、环境负载和当前硬件限制。此 JSON 位于可纳入 Git 的目录；原 `docs/experiments/` 被其自身 `.gitignore` 忽略。

本地 `.superpowers/sdd/managed-low-latency/` 为被 Git 忽略的完整证据目录。不要删除，其中有原始硬件 JSON、命令日志、审查记录和原始用户文件哈希。不要依赖 `target/` 下的历史报告，那里曾在环境刷新后消失。

## 已完成，后续直接沿用

- **正式服务接入 EngineHost**：`crates/moiren-app/src/service/managed.rs` 及 `managed/`。生产 `start_service()` 使用 ManagedDriver，旧 BackendDriver API 保留兼容用途。
- **真实多输入 / 多输出**：`service/graph/preparation.rs` 翻译全部 IO、DSP 和参数；首个稳定 Sink 为 master，其他为 follower。多个输入可在图中混合。重复资源、格式及进程 PID + 创建时间校验均保留。
- **单 Engine + 多 native owner**：`moiren-windows-audio/src/engine_backend/` 和 `render/source.rs`。主输出执行主图，跟随输出读取各自 Sink 的有界时钟桥；所有 Start ACK、失败取消、peer join 和报告回收统一处理。
- **保护所有 Sink**：`moiren-engine/src/runtime/swap/` 和 compiler plan API；兼容换图持续保护所有输出 writer，已有零 RT 分配 / 释放回归。
- **三档延迟预算**：`engine_backend/profile.rs`、`service/latency.rs`。按实际协商周期准入；Low 请求 5 ms，设备可协商到更长。Follower 余量包含 master 完整目标库存补充 burst，不能退回仅按单个 period 计算。
- **真实诊断**：`engine_backend/observations.rs`、`ui/state.rs` 和 `ui/window/settings.rs`。按 owner / generation 防止旧数据，显示周期、队列、fallback、桥库存、欠载、丢帧和恢复。CPU / 物理端到端时延 / 硬件 XRUN 未测量时保持未知。
- **设置与持久化**：三档真实动作、候选与活动档位分离、旧文件 Compatible 默认值、后台有序写入、退出 drain、关闭重开保留写入链、失败反馈均完成。
- **有限静音实机工具**：`crates/moiren-windows-audio/examples/routing_probe.rs`。独立零增益输出，真实兼容计划 1 → 2，检查 owner / epoch、时间线、healthy 和 Closed，不保存 PCM、不改默认设备或音量。
- **连续长测与异常窗口**：探针显式支持 `--soak-seconds 60..1800`，与 `--seconds 1..60` 互斥；控制侧最多保存 1801 个样本。`--trace` 记录固定 128 个最近 native 异常窗口及最多 8 次前序回调，覆盖数保持可见。Render report schema 为 6。
- **截图工具**：`ui/window/capture.rs` 已补齐与正式入口一致的偏好恢复；设置页 1440×900 / 1024×720 截图保存在 `docs/designs/screenshots/managed-routing-low-latency/`。

### 必须保留的竞态修复

ManagedDriver 每次 Apply 的身份独立于候选 revision 和 host revision。`ApplyConfirmation { id, revision, host_revision }`、`PendingEdit.apply_id`、`Owner.latest_apply_id` 和 `Owner.active_apply_id` 共同约束确认。

`active_apply_id` 防止周期状态投影使用上一次同 revision 的确认，提前清除这一次 Apply 的 pending；只有本次参数 ACK 屏障完成后才更新映射。初始映射 id 为 0，未来拓扑的确认也有回归。不要简化为只比较 `(candidate_revision, host_revision)` 或单独比较 pending revision。

回归 `unchanged_apply_stays_pending_until_its_own_realtime_ack` 在暂停音频时重复 Apply 已活动候选，必须等待自己的 ACK。相关测试允许协议合法的 `AppliedLate` 成功；不要在不测试时限的场景重新强制只接受 `Applied`。

## 剩余工作，按优先级继续

### 1. Low 的实机稳定性资格验证

**已完成单会话 600 秒静音长测及需求缺口模型；FreeDSP + CABLE 未通过 Low 无恢复资格。**
受控压力和可听连续性仍未验证。保留先前短测和本次长测：

- FreeDSP master / capture 为 336 帧，即 7 ms；CABLE follower 为 240 帧，即 5 ms。
- Low render 目标 504 / 360 帧；capture prime / capacity 为 1010 / 1683；follower prime / high / capacity 为 866 / 1610 / 2114。
- 30 秒记录：follower 积压恢复 1 次，主动丢弃 777 帧。
- 最终 10 秒记录：follower 积压恢复 2 次，主动丢弃 1562 帧。两次 Low 都没有 bridge drop / underrun，均 healthy / Closed；该统计仍不足以证明无音频中断。
- 最终 Compatible / Balanced 各 3 秒重跑，均无积压恢复 / bridge drop / underrun，真实计划 1 → 2 且保留 owner / epoch。
- 本次 Low 连续 600.004 秒：120 次恢复、主动丢弃 94134 帧，约 12 次/分钟；各桥 overflow / underrun 为 0，owner / epoch 保持，真实计划 1 → 2，healthy / Closed、无 cleanup error。
- 控制侧保存 601 个 native 样本；活动窗口内外部系统 CPU 样本为 18%～85%，均值 44.06%。这只是环境负载，不是音频 DSP CPU 指标或受控压力。
- 最后保留的 18 个恢复窗口中，前 8 次及当次 wake 间隔均不超过 6.3803 ms。0 / 360 帧需求替代正常 240 / 240 会积累消费缺口，不能只按 wake 超时解释恢复。
- 可移植 30 秒模型比较 240 / 240、120 / 360 与 0 / 360：前两组不恢复，最后一组持续缺口超过 2000 ppm 修正能力时明确恢复。保持无 overflow / underrun、零 RT 分配 / 释放；没有放大队列或隐藏计数。
- Realtek 主输出 + CABLE 跟随输出、两路软件输入的 Balanced 连续 600.005 秒：恢复 / 主动丢弃 / overflow / underrun 均为 0，计划 1 → 2、同 owner / epoch、正常关闭。两个实际 period 都是 480 帧、render target 都是 960 帧。它不是 FreeDSP 物理输入组合的直接对照。
- Realtek + CABLE 的 Low 60 秒记录为 23 次恢复、丢弃 23789 帧；修复诊断后的最终 Low / Balanced 各 15 秒短测没有恢复。短测差异不构成队列行为改善或 Low 长期资格证明。

建议下一 agent：

1. 读取本次资格记录及 raw trace，保留上述缺口模型和原有 3 相位 × 3 漂移的 30 秒预算内模型。不要通过清零、隐藏计数或单纯放大队列获得表面通过。
2. FreeDSP 在 13:59 续作时已离线，旧 ID 的准备请求明确失败并正常关闭；当前只有 Realtek / CABLE 输出可用。重新接入后，在同一 FreeDSP 输入 / 双输出组合上长测 Balanced，再视结果验证 Compatible。
3. 当前策略是用户选择 Balanced 后 Apply 并确认新 owner 的活动档位，再复测；没有新增自动降档或修改偏好。不同设备 / 输入的结果不能冒充同组合对照。
4. 如果继续追查具体驱动原因，必须获得 native padding / 设备时钟的更完整相关证据。当前有限窗口只支持需求缺口机制，不能断言具体 WASAPI 驱动内部原因。
5. 受控压力、可听连续性和外部回采分别验收。物理时延、DSP CPU 和硬件 XRUN 仍然未知。

完成标准：有可复现证据说明恢复原因，预算内行为有回归，长测结论和限制写入验收文档；物理时延不得从队列帧数推导为实测值。

### 2. 扩大硬件覆盖

**尚未完成：多个真实物理输入同时运行、多个进程捕获并行及混合 IO 的实机验收。** 多 IO 的编译、路由、时钟与失败处理已有可移植回归；当前实机只覆盖两路软件输入或一路物理输入 → 两路输出。

继续验证不同可用设备和进程组合，检查各节点真实路由与资源身份；补充准备期间取消、设备失效 / 拔插、停止后重开、运行中切档、睡眠恢复的实际 owner 回收与 UI 状态。错误必须可见，不得保留旧 native 观测。

进程捕获没有实测物理周期，仍采用保守输入预算；不要把它标注成已完成物理输入低延迟准入。正式格式限制见下一节。

### 3. 实际性能和物理时延测量

**尚未实现 / 测量：真实 CPU 指标 publisher、可信硬件 XRUN 来源、外部回采端到端时延。** 当前 UI 正确显示未知。若继续此方向，应先定义数据源、测量范围和误差，再接入 UI；桥欠载、empty padding 和 zero demand 不能替代硬件 XRUN。

### 4. 交付整理

- 本机完整测试已结束并通过，详见下一节；新改动只重跑相关检查，避免无意义重复整套慢时钟矩阵。
- 未执行 Ubuntu / nightly Miri / 远程 CI。后续提交或 PR 时运行仓库要求的 CI，CI 通过后再按用户要求合并。
- 提交整理时 index 为空。当前 HEAD 的前端已经引用 Mixer / Compressor 状态和逐节点电平接口，因此后端提交需要包含这些支撑实现，不能只提交续作的探针与诊断。仅按精确路径整理依赖闭包，保留 Exclusive 实验和 Graph 草稿。**禁止 `git add .`、回退整个共享文件或覆盖其他任务。**
- 用户随后要求原子提交；本次授权不包含推送、PR 或合并。

## 当前验证状态

以下均已实际运行，退出码为 0；两套测试有重叠，数量不要相加：

| 检查 | 结果 |
| --- | --- |
| 工作区 `cargo check --locked --workspace --all-targets` | 通过 |
| 工作区 `cargo test --locked --workspace` | 792 passed / 0 failed / 3 ignored |
| `cargo test --locked -p moiren-app --features gpui-test` | 257 passed / 0 failed / 2 ignored |
| 工作区严格 Clippy，包含 `moiren-app/gpui-test` 和全部 targets | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `git diff --check` | 通过；仅显示仓库已有的 CRLF 转 LF 提示 |
| 原生 `ui_capture` | 成功，设置页两种尺寸已目视检查 |
| 最终静态审查 | Approved，见本地 `final-review.md` |

上表是交接时的完整工作区基线，不是续作之后整套重跑。续作新增 native 诊断、
长测探针和需求缺口模型，相关最新检查及审查以连续运行资格记录为准；
没有修改生产服务、图拓扑或队列 / 伺服策略。

续作最终相关测试 192 项通过；工作区 all-target check、包含 GPUI 测试特性的
严格 Clippy、fmt 和 diff-check 通过。诊断的 unwind / 失败回调保留问题已修复，
8 个新增回归均完成 RED / GREEN；独立续作审查 SPEC PASS / QUALITY APPROVE，
详见本地 `continuation-20261016/continuation-final-review.md`。两段 600 秒长测和
最终短测均已结束，没有需要接着轮询的本任务进程。没有重跑完整工作区测试、
Ubuntu、nightly Miri 或远程 CI。

随后按用户原子提交要求，精确暂存并导出独立源码重新验证：工作区完整测试
799 passed / 0 failed / 3 ignored，GPUI 测试 266 passed / 0 failed / 2 ignored，
探针 14 项通过；all-target check、含 GPUI 特性的严格 Clippy、fmt 和暂存
diff-check 均通过。此轮覆盖当前 HEAD 的后端依赖，取代上述未重跑完整测试的
续作时点状态；未重跑硬件或远程 CI，详见验收记录的“原子提交前复验”。

用户正在运行的 `target/debug/moiren-app.exe` 曾导致 Cargo 覆盖失败，因此本轮命令都使用隔离产物目录。**不要终止用户应用。** 后续可复用：

```powershell
cargo check --locked --workspace --all-targets --target-dir .superpowers/managed-routing-target
cargo test --locked --workspace --target-dir .superpowers/managed-routing-target
cargo test --locked -p moiren-app --features gpui-test --target-dir .superpowers/managed-routing-target
cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test --target-dir .superpowers/managed-routing-target -- -D warnings
cargo fmt --all -- --check
```

时钟 control / faults / startup 矩阵单个测试可能耗时数分钟，日志出现 “running for over 60 seconds” 属于当前已知执行时长，先确认进程和日志再判断故障。

## 复现实机命令

先枚举，确认设备仍可用；opaque ID 可能因设备重新安装而变化：

```powershell
cargo run --locked -p moiren-windows-audio --target-dir .superpowers/managed-routing-target --example routing_probe -- --list
cargo run --locked -p moiren-windows-audio --target-dir .superpowers/managed-routing-target --example routing_probe -- `
    --output '{0.0.0.00000000}.{dedcefcb-f139-4c50-8950-6925f8c60444}' `
    --output '{0.0.0.00000000}.{93f962ad-26f6-4485-8a34-e1c7b27f44d5}' `
    --input '{0.0.1.00000000}.{0e3bce79-16b7-4594-8ef2-5db33dfb70b6}' `
    --seconds 30 --profile low
```

首个输出为 master；可重复 `--input` / `--output`，最多各 16 个，省略 input 使用两路软件音源。当前工具始终静音。不调整系统默认设备、格式、音量或独占策略。一个 96 kHz CABLE endpoint 已被格式校验拒绝，此拒绝属于当前能力边界。

## 共享工作区保护

- 原任务开始 HEAD 为 `a1e1228`，续作开始为 `6003f22`，并行前端任务随后推进至 `ec39967`。不要把整个 HEAD 差异当成本任务改动。
- Graph canvas、节点目录 / 端口、Mixer / channel strip、Compressor 面板、逐节点 meter 和 `FRONTEND.md` 有并行变更。服务、host、UI state、window 等存在共享文件，后续修改以当前源码为准。Compressor UI 已被其他任务补齐，不要依据旧文档恢复默认参数限制。
- 用户原有 Exclusive 实验必须保留：`examples/exclusive_probe.rs`、`src/exclusive_probe.rs`、`src/exclusive_probe/{render,capture}.rs`、`docs/superpowers/plans/2026-10-15-exclusive-loopback.md`，以及 Windows audio README / `lib.rs` 的 Exclusive 内容。5 个文件最终 SHA-256 全部与开始时相同，README / lib 新增内容也逐行核对保留。
- 原始哈希 / diff 为 `.superpowers/sdd/managed-low-latency/exclusive-before.json` / `exclusive-before.diff`，最终核对为 `exclusive-final.json`。这些文件是对照依据，不能被新的 baseline 覆盖。
- 严格遵守仓库 RT 约束：图和 buffers 在控制侧准备；RT 不分配、不释放、不阻塞、不做 COM；退休计划在控制侧回收。逻辑拓扑、准备计划和平台 transport 保持分离。

## 范围外长期后端能力

正式 Exclusive / 逐应用 Takeover、ASIO、多通道、虚拟音频驱动、专业 SRC、多输出物理延迟补偿、CLAP / VST3、Named Pipe / 远程控制，以及 EQ / Gate / Limiter / Delay / Recorder、PreFader、标准 LUFS / True Peak 仍属后续。先完成上述 Low 验证与硬件覆盖，再根据用户安排选择扩展方向。
