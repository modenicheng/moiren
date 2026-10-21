# Moiren VAD 开发交接：2026-10-21

## 1. 接手时的结论

用户要求修复 VAD 审计问题并继续虚拟机稳定性测试，保留既有工作，
由子代理负责原子分批提交。当前任务尚未完成：**组合短测仍失败，
8 小时空闲／PCM 长测没有启动，也没有取得长期稳定性资格。**

已修复独立的并发关闸问题，增加有界时序记录，并完成真实目标机复测。
独立 PCM 短测通过；组合测试的 15 秒空闲和两段信号验证通过，
随后持续 PCM 出现一个 480 帧静音包，按原有标准判定失败。

下一步应先建立“单个 owner 周期跨期 → 两个边界重置／握手 →
麦克风读空”的确定性回归，定位完整因果链，再设计和修复恢复行为。
不要直接开始 8 小时测试，也不要降低静音、波形、新鲜度、覆盖率或截止时间标准。

详细验证台账：`docs/vad-validation-2026-10-21.md`。
驱动总体状态：`driver/vad/STATUS.md`，其中包含尚未提交的既有更新。

## 2. 仓库和必须保留的工作

- 工作目录：`D:\coding\moiren`，PowerShell，Rust 2024 workspace。
- 分支：`feat/vad-foundation`。
- 本交接文档创建前 HEAD：`c1d4ea5277614abdd7aea27a1ce50b620a1778cb`。
- 已完成的提交均未 push；未建立或合并 PR。
- 功能源码工作树干净。下面的既有文件必须保留，不要混入新提交：

```text
 M docs/superpowers/plans/2026-10-19-repo-steward-remediation.md
 M driver/README.md
 M driver/vad/STATUS.md
?? docs/vad-soak-validation.md
?? driver/moiren/
```

`driver/moiren/` 是用户独立的 KMDF 模板工程，仍未跟踪。
实际 ACX VAD 在 `driver/vad/`。不要批量 `git add driver/`、清理该模板、
回滚既有脏文件或把原始测试产物提交进仓库。

只 stage 明确负责的路径。继续把提交工作交给子代理；不要 amend 或 push，
除非后续指令明确要求。新鲜上下文先重读根 `AGENTS.md` 和当前 diff。

## 3. 已完成的提交

| 提交 | 内容和范围 |
| --- | --- |
| `bfe916ee6b15909b4e12023600426d6c2fe97394` | 两种 backend 的 virtual-only 边界也启用现有 clock-drift/FIR 适配，处理 native480 与 Graph256 的独立相位。 |
| `eca8906336285359321addbe9548e9f1328a8a1d` | 麦克风成功完成的延迟收集不再不断重设半周期相位；到期时至多提交一个新包，不追赶突发。 |
| `bf22a573ae1498c263cbe133e83914aafe34c4ca` | 首批失败证据和验证范围台账。 |
| `121118cf73f72a9bb016f35ad43ddec676cc7977` | 并发关闸的终结错误确认与单次状态查询，修改 `vad/client.rs`、`vad/control_io.rs`。 |
| `a6b16d1bb96b46b764eb69ccec16fc7f49525bb8` | owner 固定 64 条 gap tail，join 后输出；麦克风读空日志增加 worker 侧 QPC。 |
| `c1d4ea5277614abdd7aea27a1ce50b620a1778cb` | 汇总重复 PCM 失败、二进制哈希、时序证据与解释边界。 |

原始审计事项已有独立处理：

- `7ac7283`：生产源码抽取的 REMOVE／cleanup／destroy 回归证明当前源码会重试
  Removing 状态，并传播删除失败。审计描述中的旧缺陷在当前源码中不存在，
  不需要为此再改内核或安装新包。
- `93a9c0c`：忽略 VS／WDK 本地产物，保留用户模板工程。
- AU 控制 ACL 仍是标准用户、单 owner 的产品设计，不认证 Moiren 可执行文件，
  也不隔离互不信任的本地用户。不要把独占句柄说成进程认证。
  先前尝试的 limited S4U 任务实际仍拿到管理员 token，
  探测器拒绝启动，因此**标准用户目标机验收尚未通过**。

## 4. 必须保留的实现契约

- 薄驱动负责 ACX 端点和有界 PCM；Graph、DSP、SRC 和配置留在用户态。
  同一 Graph 只有一个执行 owner，物理主时钟丢失不能静默切换为虚拟主时钟。
- 没有客户端、静音、零读写、等待超时、空／满队列均不能被当成设备死亡；
  不恢复“三秒无 PCM 即故障”或 follower 等内容启动。
- `PreparedVirtualPlayback`／`PreparedVirtualMicrophone` 携带 `reset_epoch`、
  `producer_quiesced_epoch`、`consumer_quiesced_epoch`，
  边界使用 `.with_reset_epoch(request).with_epoch_handshake(p, c)`。
  producer／consumer 双向确认完成前不能恢复 fresh pumping。
- 播放 reset 先退休旧原生请求，再发布 producer ack；
  麦克风 reset 等 Graph producer ack 后才能 discard，再发布 consumer ack。
  不允许旧 epoch 的迟到发布跨越 reset。
- 麦克风最多一个 outstanding WRITE，四个预分配槽轮转。
  活跃 worker 等 oldest event，最多 10 ms；timeout 正常，不取消请求，
  不额外叠加固定 10 ms sleep。
- IO 序号属于控制 file session，替换 `PcmOwner` 不能从 1 重新开始。
- `121118c` 只处理精确的 `ERROR_REVISION_MISMATCH`：
  非阻塞 completion 检查也返回同一终结错误，才允许该 control view 继续 QUERY。
  随后的单次 QUERY 必须证明同一端点／流、gate epoch 严格推进且已关闭。
  不重发 SET_GATE，不接受重新打开的 gate 或新流。
- 其他 completion 错误、超时或不确定状态仍 poison；
  未完成请求的 Box、缓冲区、event、OVERLAPPED 保留到终结 drain。
- RT 路径不能分配、释放、阻塞或格式化日志。
  当前 clock trace 只写固定数组，日志由 control joiner 输出。
- 不能通过增加队列、保留陈旧内容、补播全部错过周期或放宽波形判据掩盖失败。
  当前 carrier residency／端到端延迟尚未完成预算验收。

## 5. 虚拟机连接和进程生命周期

已验证的目标：

| 项目 | 值 |
| --- | --- |
| VMware 显示名称 | `Windows 11 x64` |
| Guest hostname | `DRIVER-TEST` |
| Windows build | `10.0.26200` |
| SSH | `wdkremoteuser@192.168.111.128` |
| 本机私钥路径 | `C:\Users\cheng\.ssh\moiren-test` |
| Guest 证据目录 | `C:\MoirenVadTest` |

```powershell
ssh -i C:\Users\cheng\.ssh\moiren-test -o BatchMode=yes -o ConnectTimeout=10 wdkremoteuser@192.168.111.128
```

用户曾提供 `.1` 地址；前次验证该地址不接受这把 key。
`.128` 才是已实际测试的 guest，下一轮仍先核对 hostname，不能仅凭地址操作。

用户明确要求连接后阻止休眠。此前已确认 AC/DC standby timeout 都是 0，
hibernation 已关闭／不可用。新一轮先复核电源设置和 probe 进程。
最后一次目标机核验为 **2026-10-21 15:46:23 +08:00**：
没有 `vad*`／`moiren*` 进程，present-only PnP 只看到正常的
`ROOT\MEDIA\0000` 控制总线，没有 present 的 `MOIREN\*` 子设备。
这是历史快照，不能代替下一次启动前检查，也不是 100 轮幽灵端点验收。

本轮没有安装驱动或修改系统默认音频设备；继续这条用户态调查时保持该范围。

远程复杂命令使用 UTF-16LE base64 的 `powershell -EncodedCommand`，
避免嵌套 SSH 引号。短测采用 **task 生命周期、前台 SSH wrapper**：

- wrapper 检查没有其他 probe／应用，拒绝重复证据目录，并校验 exe SHA256。
- `Start-Process -PassThru -WindowStyle Hidden`，stdout／stderr 分开落盘。
- 保留真实进程句柄，等待正常退出；超时只终止自己启动并持有句柄的进程。
- 保存 `launch.json`、`exit.json` 和 `probe/report.json`，结束后核验退出／清理。
- 先前通过 SSH detach 的进程曾消失，不能假设后台启动能可靠跑完 8 小时。
  长测还需要可靠、可检查及可停止的执行机制。

## 6. 二进制、日志和短测模板

当前已核验的带诊断二进制：

```text
source_commit:
a6b16d1bb96b46b764eb69ccec16fc7f49525bb8

guest_executable:
C:\MoirenVadTest\vad_integration-70FFCA82.exe

SHA256:
70FFCA82BFC3AE1B139FEA590F0D83EE7ACA8D9995DE97E478F685516F5165A5
```

`c1d4ea5` 只有台账更新。新改代码后必须重新构建、记录源版本并核对哈希，
不能把旧二进制重新标记为新提交。

短测 wrapper 在 guest 的 `C:\MoirenVadTest\run-native-short.ps1`；
本机副本在被忽略的 `driver/vad/build/tests/run-native-short.ps1`。
该 wrapper 未跟踪，不能假设另一个 checkout 会自动带有它。

以下模板可复现现有二进制；`-Combined` 运行 15s idle + 65s PCM，
去掉该开关运行独立 PCM 短测。每次都使用新的证据目录。

```powershell
$remoteScript = @'
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
& 'C:\MoirenVadTest\run-native-short.ps1' `
    -Executable 'C:\MoirenVadTest\vad_integration-70FFCA82.exe' `
    -ExpectedSha256 '70FFCA82BFC3AE1B139FEA590F0D83EE7ACA8D9995DE97E478F685516F5165A5' `
    -SourceCommit 'a6b16d1bb96b46b764eb69ccec16fc7f49525bb8' `
    -EvidenceName ('handoff-combined-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')) `
    -Combined
'@
$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($remoteScript))
ssh -i C:\Users\cheng\.ssh\moiren-test -o BatchMode=yes -o ConnectTimeout=10 `
    wdkremoteuser@192.168.111.128 powershell -NoProfile -EncodedCommand $encoded
```

独立短测内部参数为 `--exercise --pcm --timeout-seconds=120`；
组合为 `--exercise --soak-idle-seconds=15 --soak-pcm-seconds=65 --timeout-seconds=260`。
外层最多等 280 秒。不要把超时后的不确定结果当成重新启动的授权。

诊断 filter 是 **`info,moiren::audio::vad=debug`**。
`moiren_windows_audio::vad=trace` 不会启用这些自定义 target 记录。
`scripts/vad-soak.ps1` 会强制 `RUST_LOG=info`，不能假设该 runner 会输出 gap 诊断。
不要同时运行 App integration probe 和另外一个拥有控制会话的原生工具。

本机原始证据保存在被忽略的：

```text
driver/vad/build/tests/20261021-soak-followup/
```

其中有 14 个 run 目录、before／after PnP JSON 和旧候选 patch。
保留这些证据，但不提交 exe、原始日志或候选 patch。
旧试验二进制的来源和失败分类见详细台账；不要将它们重新解释为已提交版本。

## 7. 最新目标机结果

下面的名称都带 `20261021-` 前缀，guest／本机副本目录同名。

| 目录后缀 | 结果 |
| --- | --- |
| `121118c-pcm-short1` | PASS；两段各 48,000 帧完全匹配；Stop 时 capture 保持 RUN，随后验证 48,000 帧静音；service joined／control released。 |
| `121118c-combined1` | 15.068s idle 和两段信号通过；持续 PCM 在 14.828s 失败：711,360 matching + 480 silent。 |
| `a6b16d1-pcm-short1` | PASS；两段精确信号及保持 RUN 的 Stop 静音验证通过。 |
| `a6b16d1-combined1` | 15.120s idle 和两段信号通过；持续 PCM 在 16.148s 失败：774,720 matching + 480 silent。 |
| `a6b16d1-combined2` | 15.061s idle 和两段信号通过；持续 PCM 在 12.512s 失败：600,000 matching + 480 silent。 |

三次组合失败均为正常报告退出，无强制 kill／外层超时，
service 已 join、control session 已释放，没有 cleanup error。
零 unexpected frame／零 capture discontinuity 不能抵消 silent frame 的失败。

两轮完整 tail 的关键数值：

| 证据 | combined1 | combined2 |
| --- | --- | --- |
| owner total／retained | 11／11 | 9／9 |
| 最近的前置 owner QPC | 536807677760 | 538208597695 |
| stage／skipped periods | Wake／1 | Wake／1 |
| microphone 0/480 读后的 QPC | 536807713944 | 538208656281 |
| 记录的 counter frequency | 10,000,000 | 10,000,000 |
| 两个采样的间隔 | 3.6184 ms | 5.8586 ms |

`Wake` 只表示 pre-render 检查发现跨期，不意味着已证明 timer 唤醒延迟。
owner 采样在 reset 增量之前，microphone 采样在 read 之后，中间可以被抢占。
这两个间隔不是 stall duration，也不能单凭时间邻近证明某个 reset 清空了该 read。
尾部没有截断，但尚缺少逐边界 reset requester／epoch 的直接关联。

两次都没有 expired WRITE 日志。combined1 的驱动麦克风 silence 保持启动值 6；
combined2 在失败前保持 5，随后在 teardown 的 paused／gate3 状态观察到 6，
该后置计数变化不能当作原发 kernel starvation 证据。

旧 run 曾出现驱动 silence 计数增长、没有 carrier short 的另一类故障。
它与本次用户态 0/480 编成零包的签名不同，不能合并成一个已证明的根因。

## 8. 首要代码入口和下一步

| 路径 | 关注点 |
| --- | --- |
| `crates/moiren-windows-audio/src/virtual_owner/owner.rs` | `run` 在 wake／after-render 任一 skipped period 后调用 `reset_timeline`，重置全部注册边界。 |
| `crates/moiren-windows-audio/src/virtual_owner/owner/trace.rs` | 固定 64 条 tail 和 join 后日志；不是所有 reset requester 的追踪器。 |
| `crates/moiren-windows-audio/src/virtual_owner/boundary.rs` | playback discard／FIR reprime 和 microphone producer 握手、背压重置。 |
| `crates/moiren-windows-audio/src/vad/provider/pcm_worker.rs` | `observe`、两方向 `quiesce_*`、单请求 microphone cadence 和 reset 后重新编码。 |
| `crates/moiren-windows-audio/src/vad/pcm_bridge.rs` | `encode_packet` 的 carrier read／补零及 QPC 诊断。 |
| `crates/moiren-windows-audio/src/vad/client.rs` | `finish_close` 的有界状态确认。 |
| `crates/moiren-windows-audio/src/vad/control_io.rs` | terminal completion、poison、pending Box 保活和 blocking drain。 |
| `crates/moiren-app/examples/vad_integration/` | 原生 pair、连续 PCM 质量／覆盖检查、报告和清理。 |

当前源码中，一个 Graph256 周期在 48 kHz 约为 5.333 ms。
即使只 skip 一周期，owner 也请求所有边界 reset。
playback 的 4800 帧 carrier 在 reset 后需要重新 priming，
当前阈值 2544 帧，约六个 480 帧发布。小跨期可能放大成较长恢复空窗。
microphone carrier 为什么在恢复后过早读空，仍需把握手与实际发布顺序复现出来。

建议下一步按顺序执行：

1. 用可控假时钟、独立推进的 native480 tick、真实 prepared owner 和两个带 p/c
   fence 的边界，建立 Graph256／48 kHz 的连续非零基线。
2. 只延迟一次 owner wake，使 `skipped_periods == 1`。
   保持 stream／gate 身份不变、worker observation gap 低于现有 reset 阈值，
   排除额外的 worker-origin reset。
3. 在 wake 记录后约 3.6184 ms 及邻近次序调度麦克风消费，
   记录 request epoch、两方 ack、carrier occupancy 和第一个 post-reset encode。
   覆盖两个边界收到同一 owner reset batch，以及迟到生产／消费的交错。
4. 保留零静音、无旧代次穿越、有界队列、唯一 Graph owner 的断言。
   先得到能复现目标机签名的 RED，不能把测试改成“允许一个静音包”。
5. 根据已证明的失败路径明确有界 freshness／recovery 契约，再实施最小修复。
   不直接删除 reset、延长超时、改半周期常量、加队列或照搬 MMCSS 试验。
6. 主机回归和独立审查通过后，提交、构建并记录 SHA256。
   依次跑独立 PCM 短测、组合短测及重复验证；仍失败则继续调查。
7. 只有前置关口可重复通过、长测进程管理可靠后，才进入 8 小时空闲／PCM、
   100 轮生命周期、Verifier、电源转换和发布资格验证。

补充调查边界：管理 QUERY 和 PCM 提交仍共用根 sequential queue、
passive work item、Admin lock。内核 admission delay 尚未测量；
async WRITE 返回耗时不能代替内核入队时间。若拆分 dispatch，
必须单独复审 cancellation／rundown，并重新验证目标驱动。
Rust 的 thread sleep 已确认使用高分辨率 timer，不能再归咎于固定 15.6 ms Sleep。

## 9. 已有验证和可复用命令

最新库测试为 **390 passed、2 intentionally ignored**；VAD 子集 106 passed；
owner 7 passed；RT 检查 79 passed；驱动六组 host suites 和 soak runner 回归通过。
严格 workspace Clippy（包含两种 probe feature）、fmt、diff check 通过。
release 构建成功，重新构建的哈希与实际 guest exe 一致。

完整 `cargo test --locked --workspace` 在 phase 修复和最终 cadence 版本通过。
后加的并发 close／clock trace 由上述 focused suites 覆盖，
不要把旧 workspace 全量结果重新标记为最新源码全量结果。
本轮未执行 Miri、Verifier、8h、100 轮、电源转换或正式签名验收。

```powershell
cargo test --locked -p moiren-windows-audio --lib vad::
cargo test --locked -p moiren-windows-audio --lib virtual_owner::owner::
cargo test --locked -p moiren-windows-audio --lib
cargo test --locked --manifest-path tools/rt-check/Cargo.toml
pwsh -NoProfile -File driver/vad/test.ps1
pwsh -NoProfile -File scripts/tests/vad-soak.tests.ps1
cargo clippy --locked --workspace --all-targets --features moiren-windows-audio/vad-probe,moiren-app/vad-integration-probe -- -D warnings
cargo fmt --all -- --check
git diff --check
cargo build --locked --release -p moiren-app --features vad-integration-probe --example vad_integration
Get-FileHash -LiteralPath target/release/examples/vad_integration.exe -Algorithm SHA256
```

本机检查日志在被忽略的 `driver/vad/build/tests/`：
`workspace-final.log`、`library-clock-trace.log` 等。
全量时钟模拟可能耗时数分钟；使用可回读的 session，不因等待模糊而重复启动。
普通 `cargo test` 不能安装驱动或自行打开真实设备。

## 10. 子代理与接手提醒

当前会话中可复用的代理，是否仍可访问须由接手环境确认：

- Lovelace：`01a15cdf-3719-72c1-8d65-ca51dcdc230d`，
  负责按主代理明确批准的路径做原子提交，最近三个提交均由其完成。
- Godel：`01a15cba-658f-78c0-848e-75cf3e562d24`，
  完成只读复审，指出“已完成的 revision mismatch 被 poison”问题，
  并给出单周期、双边界回归建议。
- Bacon：`01a15cba-6709-7b82-8976-9ad9155c8d8e`，
  实现并验证 `control_io.rs` 的终结确认，完成时钟记录独立复审。

没有尚在运行的测试或待完成的代理写入任务。
接手后重新分配文件所有权，再并行开发；保留已完成的 p/c handshake、
槽轮转、cadence 和 close 修复，不从旧候选或已撤销的 pacing 试验重新开始。
