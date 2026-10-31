# 托管路由与低延迟

> **2026-10-27 更新**：本文原描述的 legacy 队列/库存档位模型与 `routing_probe`
> 验证工具已随单栈化（PR #20）废弃。档位语义已按下文重写为设备 buffer 请求；
> 变更背景见 professional-only 后端计划（2026-10-27；计划文档已于 2026-10-31
> 从仓库移除，见 git 历史：`git log -- docs/superpowers/plans`）。
> 本文是延迟档位语义的 canonical 文档；README 中的档位描述以本文为准。

## 生命周期与路由

桌面服务提交真实 `LogicalGraph` 对应的 `EngineProject`，由 `EngineHost`
统一管理编译、块边界更新、资源等待和控制侧回收。候选图与引擎确认的活动图
继续使用不同版本；修改候选图并不代表音频已经应用。更换设备、改变 IO 集合
等不兼容更新可以重建 transport 并推进 epoch。

每个 Source / Sink 独立绑定资源。第一个输出作为 master，其余输出为 follower；
原生适配器检查 master 唯一性和重复端点。主输出的设备 demand 驱动唯一的
Engine，其余输出通过预分配的时钟桥读取各自 Sink 的音频，不重复执行主图。
Follower 的库存、漂移修正、欠载、丢帧与重置单独观察。兼容换图必须保留所有
活动输出的唯一 writer，不能只保护 master 的 sink。

多个输入分别进行设备或进程捕获并在 Graph 中混合。进程输入使用可持久化的
executable 选择器（`InputSource::Application`，`PassiveTap` 不抑制原应用声音）；
激活时解析为带创建时间校验的活跃进程身份，不静默换绑同名新进程。所有输入、
输出共享取消信号，必须全部确认 Start 才进入 Running；停止和任何准备失败都要
回收全部 owner。

## 档位和数字含义

延迟档位保存的是**设备 buffer 请求**，不是 legacy 队列/库存/恢复策略：

| 档位 | 设备 buffer 请求（无显式配置时） |
| --- | --- |
| Compatible | 设备默认周期 |
| Balanced | 约 10 ms（512 帧 @ 48 kHz） |
| Low | 约 5 ms（256 帧 @ 48 kHz） |

档位映射注入 `DeviceOptions.period_frames`；显式的 per-device 或全局 buffer
请求恒优先于档位映射。native 准入只接受 2 的幂的帧数，映射因此固定在上述
预设上。档位变更在下次启动时生效（保存即刻、重启应用），诊断面板显示实际
协商 period。请求 10 ms 不代表设备实际按 10 ms 工作；实际周期、初始化
fallback 来自原生观测，请求值不是测量值。

## 验证入口

`routing_probe` 已随 legacy 栈删除（2026-10-27）。现存实机探针为
`asio_probe` / `exclusive_probe`（能力与占用诊断）和 W00 系列
（`w00_probe` / `w00_physical` / `w00_swdevice`，见
[windows-audio README](../../crates/moiren-windows-audio/README.md)）。
托管路径的连续运行资格结论保留在 2026-10-16 低延迟资格验证记录中（计划文档
已于 2026-10-31 从仓库移除，见 git 历史：`git log -- docs/superpowers/plans`）：
FreeDSP + CABLE 在 Low 连续 600 秒内发生 120 次跟随积压恢复（legacy 队列
模型下测得）；预算内 wake 间隔不保证长期消费量匹配。该组合应选择 Balanced
后在相同 IO 上重新验证，必要时再选择 Compatible。没有自动改档；healthy、
零 overflow 或零 underrun 都不能替代无恢复和可听连续性验收。

普通软件测试与硬件执行证据分别记录在 2026-10-16 托管路由验收文档中（计划文档
已于 2026-10-31 从仓库移除，见 git 历史）。音频格式边界仍为
Shared、双声道 graph、48 kHz 输出（canonical 描述见
[windows-audio README](../../crates/moiren-windows-audio/README.md)）；专业
路径使用预计算 windowed-sinc SRC，旧 Clock Bridge 路径保留线性插值基线。
