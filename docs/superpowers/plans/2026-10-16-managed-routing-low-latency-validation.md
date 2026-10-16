# 托管路由与低延迟验收记录

日期：2026-10-16。范围依据[实现计划](2026-10-15-managed-routing-low-latency.md)和[设计说明](../../designs/14-managed-routing-low-latency.md)。

续作的[连续运行资格验证](2026-10-16-low-latency-qualification.md)补充了单会话
600 秒 Low 长测、环境负载、异常窗口与可移植需求缺口模型。FreeDSP + CABLE
组合发生 120 次积压恢复，未通过 Low 无恢复资格；下述原始短测不能替代该结论。

## 交付行为

- 正式桌面服务使用 `EngineHost`。兼容图更新保留音频 owner / epoch；设备、IO 或档位变化由 host 管理重新准备和回收。编译或准备失败不冒充候选图已经活动。
- 所有 Source / Sink 都进入真实 `EngineProject`。主输出驱动唯一 Engine；其他输出通过各自预分配的时钟桥读取对应 Sink。所有输出的 writer 约束在热更新期间持续受保护。
- 每次 Apply 具有独立身份及参数 ACK 屏障。重复提交相同候选版本、延迟拓扑确认、加载另一份项目、取消和停止都不能消费新请求的确认或回退活动参数。
- Compatible / Balanced / Low 保存为用户偏好。期望档位与活动档位分开展示，下一次 Apply / Start 才采用新设置；退出和关闭重开窗口都保留有序偏好写入。
- 原生观测按 owner 和服务 generation 校验，展示各流实际周期、队列、初始化 fallback、时钟桥库存和故障恢复计数。停止或 owner 更换后清除旧数据。
- 准备失败会停止、join 并保留已协商原生 owner 的报告。音频线程的队列、图、缓冲和信号绑定提前准备，不引入堆分配、释放、阻塞或 COM 重建。

## 软件验证

本机为 Windows，使用仓库固定的 stable Rust 和 `--locked`。为避开用户正在运行的 `target/debug/moiren-app.exe`，Cargo 验证统一增加 `--target-dir .superpowers/managed-routing-target`；没有终止该应用。

| 命令 | 结果 |
| --- | --- |
| `cargo check --locked --workspace --all-targets` | 通过 |
| `cargo test --locked --workspace` | 通过：792 passed、0 failed、3 ignored |
| `cargo test --locked -p moiren-app --features gpui-test` | 通过：257 passed、2 ignored；与工作区测试存在重叠，不相加 |
| `cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo run --locked -p moiren-app --features gpui-test --example ui_capture -- <输出目录>` | 通过：原生渲染导出并检查设置页两种尺寸 |

重点回归包括：托管服务 25 项；相同候选 Apply 在音频暂停时等待自己的 ACK；旧拓扑和参数确认顺序；初始启动及未来拓扑确认；每路真实非零信号路由；所有 sink 的换图保护；零 RT 分配/释放；失败准备的 peer 回收与报告保留；档位无效预算；旧 generation 观测；实际 GPUI 点击；有序写盘、写盘失败反馈及 shutdown 后重新读取 Low 偏好。

多输出桥还覆盖 3 种相位 × 3 种漂移的 30 秒软件模型，含分段及延迟后的补充 burst。这些测试验证声明的模型，不替代设备长时间测试。Apply 成功测试接受协议允许的 `Applied` / `AppliedLate`；没有将普通调度下的及时完成冒充为时限保证。

最终静态审查已通过；发现的重复 Apply 身份、相同 revision 提前清除 pending、准备失败丢失报告和退出丢失偏好写入问题均已修复并有回归。此处未运行 Ubuntu、Miri 或远程 CI。

完整命令日志和逐项审查记录位于本地 `.superpowers/sdd/managed-low-latency/`，包括 `workspace-test-final.log`、`gpui-test-final.log`、`check-final.log`、`clippy-final.log` 和 `final-review.md`。

### 原子提交前复验

2026-10-16 用户要求原子提交后，从精确暂存树导出独立源码，在未包含 Exclusive 实验和 Graph 草稿的版本上重新执行：

| 检查 | 结果 |
| --- | --- |
| 工作区 all-target check | 通过 |
| 工作区完整测试 | 799 passed / 0 failed / 3 ignored |
| App `gpui-test` 测试 | 266 passed / 0 failed / 2 ignored；与工作区测试重叠 |
| `routing_probe` example 测试 | 14 passed / 0 failed |
| 全工作区 all-target 严格 Clippy，含 `moiren-app/gpui-test` | 通过 |
| fmt 与暂存 diff-check | 通过 |

本次包含当前 HEAD 前端所依赖的 Mixer / Compressor / 逐节点电平后端接口及相关回归。两份实机证据 JSON 均位于 `docs/designs/evidence/`，原始短测证据从被忽略的实验目录逐字节复制。Exclusive 的 5 个源文件 / 计划 SHA-256 与原始记录一致，共享 README / `lib.rs` 的 Exclusive 内容保持未暂存。日志位于本地 `.superpowers/sdd/managed-low-latency/atomic-commit-20261016/`。本次未重跑硬件、Ubuntu、Miri 或远程 CI，也未推送。

## 原生硬件验证

使用 `routing_probe`，每路输出都经过独立零增益保护。工具不保存 PCM，不改变默认设备、endpoint 音量或 mute。以下原始记录的主输出为 FreeDSP，跟随输出为 CABLE In 16ch；物理输入为 FreeDSP mono，统一处理 / 输出为 48 kHz stereo。FreeDSP 在后续 13:59 的设备清单中已离线。

| 记录 | 输入和时长 | 主 / 跟随实际周期 | Render 目标帧数 | 桥 drop / underrun | 跟随积压恢复 |
| --- | --- | --- | --- | --- | --- |
| Compatible 最终重跑 | 两路软件音源，3 秒 | 480 / 480 帧，即 10 / 10 ms | 1056 / 1056 | 0 / 0 | 0 |
| Balanced 最终重跑 | 两路软件音源，3 秒 | 480 / 480 帧，即 10 / 10 ms | 960 / 960 | 0 / 0 | 0 |
| Low 较长记录 | 一路物理输入，30 秒 | 336 / 240 帧，即 7 / 5 ms | 504 / 360 | 0 / 0 | 1 次，主动丢弃 777 帧旧积压 |
| Low 最终复核 | 一路物理输入，10 秒 | 336 / 240 帧，即 7 / 5 ms | 504 / 360 | 0 / 0 | 2 次，主动丢弃 1562 帧旧积压 |

四次记录均为 healthy、保留 owner / epoch 且时间线推进，并正常到达 `Closed`。最终三种档位的探针还要求真实计划版本从 1 增至 2，确认执行了兼容换图；较早的 Low 30 秒记录未输出计划版本字段。没有报告 fault 或 cleanup error。

Low 物理 capture 实际周期也是 336 帧，输入 prime / capacity 为 1010 / 1683；follower 桥 prime / high water / capacity 为 866 / 1610 / 2114。Follower 预算已包含 master 补充目标库存的完整 burst，修复了仅按一个设备周期留余量导致的假恢复。

Low 10 秒记录中 follower 的 `empty_padding_wakes` 和 `zero_demand_wakes` 均为 15；它们不是硬件 XRUN 计数。桥的 `dropped_frames = 0` 不包括单独统计的主动积压丢弃，不能据此宣称音频完全连续。设备 / 驱动 / 调度影响下仍出现恢复，当前证据不足以承诺长期无恢复或无 XRUN。

实测字段与原始日志 SHA-256 保存在[硬件证据 JSON](../../designs/evidence/2026-10-16-managed-routing-low-latency.json)，从被忽略的实验目录原样复制以随提交保留。请求 5 ms 被 FreeDSP 协商为 7 ms，符合设备能力下限。96 kHz CABLE 输出被现有格式校验明确拒绝；没有自动更改设备格式。

## 界面证据

原生截图保留真实请求 / 活动档位及未知值。截图工具现在与正式启动一样恢复用户偏好；导出不会写入偏好文件。以下截图证明布局和状态展示，设备时序证据以前述 probe JSON 为准。

- [1440 × 900 设置页](../../designs/screenshots/managed-routing-low-latency/comfortable-settings.png)
- [1024 × 720 设置页](../../designs/screenshots/managed-routing-low-latency/compact-settings.png)

## 保留边界

物理端到端时延仍为 `null`，没有外部回采测量；DSP CPU 和硬件 XRUN 保持未知。多输入已由可移植测试覆盖，实机闭环只使用一路物理输入；多个物理输入及多个进程捕获并行仍需针对实际设备验证。续作已补充 10 分钟静音环境负载记录，但没有覆盖受控压力、运行中拔插、睡眠恢复或所有驱动组合。

正式 native 路径保持 WASAPI Shared、双声道图、48 kHz 输出和线性 SRC。Exclusive / Takeover、ASIO、多通道、插件及专业 SRC 没有被纳入此次完成声明。

开始时已有的 Exclusive 探针、子模块和计划共 5 个文件逐一核对 SHA-256 均未变化；README 与 `lib.rs` 中原有的 Exclusive 新增内容也保持。交接与续作验证阶段没有提交、暂存、推送或切换分支。用户随后要求原子提交；当前 HEAD 的前端已引用的 Mixer / Compressor / 逐节点电平后端支撑随托管路由一起整理，Exclusive 实验和 Graph 草稿不纳入。
