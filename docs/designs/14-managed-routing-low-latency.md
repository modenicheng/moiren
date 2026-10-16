# 托管路由与低延迟

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

多个输入分别进行设备或进程捕获并在 Graph 中混合。进程输入仍为固定
PID + 创建时间的 PassiveTap；不抑制原应用声音。所有输入、输出共享取消信号，
必须全部确认 Start 才进入 Running；停止和任何准备失败都要回收全部 owner。

## 档位和数字含义

| 档位 | 请求 Shared 周期 | Render 库存目标 | 物理输入 jitter 假设 |
| --- | --- | --- | --- |
| Compatible | 设备默认 | 原生完整 capacity | 保留原兼容预算 |
| Balanced | 10 ms | 实际 period + 两份 period/2 向上取整的余量 | 一个实际 capture period |
| Low | 5 ms | 实际 period + 两份 period/4 向上取整的余量 | 半个实际 capture period，向上取整 |

两份 render 余量分别用于 wake margin 和 callback budget。实际周期必须来自
原生协商；请求 5 ms 不代表设备一定按 5 ms 工作。目标不能覆盖实际周期和声明
预算，或者超过设备 capacity 时，准备明确失败，不把目标截小后继续启动。

物理输入目标为：render 目标换算成原生输入帧数并向上取整，加一个实际 capture
period、jitter 假设和两帧插值 lookahead。队列容量另保留两个 capture period
及边界帧。所有计算在控制侧进行，使用有溢出检查的算术。

Follower 的输入队列还必须容纳 master 一次补充到目标库存时的完整 burst，
不能只按 master 的一个设备 period 预留。否则正常的分段输出也可能被误判为
库存过高并触发恢复。其 prime、高水位和 capacity 与 render 库存分别报告。

进程回环是虚拟流，没有实测物理设备周期；其输入预算不能伪装成已通过物理
输入准入。各输出及物理输入的实际队列、周期和 API fallback 独立报告。

设置保存的是期望档位。活动 owner 的档位单独显示；修改设置后通过 Apply
更新，新 owner 确认后才报告生效。旧偏好文件缺少档位时使用 Compatible。

队列帧数、漂移修正、准备阶段的预算和处理时间线都不是物理端到端时延。
物理输入到扬声器的延迟必须通过外部回采等测量确认。CPU 和 XRUN 的未知值
继续显示为未知，不能用静音、队列空或未报错替代测量。

## 验证入口

`routing_probe` 是有限时长的实机验证工具。每个输出都具有独立的零增益保护，
不保存 PCM，不改变默认设备或设备音量。没有指定输入时，混合两个软件音源。

```powershell
cargo run --locked -p moiren-windows-audio --example routing_probe -- --list
cargo run --locked -p moiren-windows-audio --example routing_probe -- `
    --output '<master endpoint ID>' --output '<follower endpoint ID>' `
    --seconds 10 --profile low
```

可重复传入 `--input '<capture endpoint ID>'` 检验多个物理输入。
工具执行一次真实兼容图修改，报告是否保留 owner / epoch、时间线是否推进、
关闭是否完成、原生报告及准备观测。物理端到端时延字段保持 `null`。

默认仍运行 5 秒；短测使用 `--seconds 1..60`，单会话长测显式使用
`--soak-seconds 60..1800`，两者互斥且不可重复。控制侧样本最多 1801 个，
失败后仍检查关闭并保留原始错误。`--trace` 额外保存固定 128 个最近异常窗口，
每个窗口含最多 8 次前序回调；覆盖数单独报告，不能解释为未发生过异常。

[连续运行资格记录](../superpowers/plans/2026-10-16-low-latency-qualification.md)
显示 FreeDSP + CABLE 在 Low 连续 600 秒内发生 120 次跟随积压恢复。
预算内 wake 间隔不保证长期消费量匹配；持续 native demand 缺口仍可能超过
2000 ppm 修正能力。该组合应选择 Balanced 后在相同 IO 上重新验证，必要时再
选择 Compatible。没有新增自动改档；healthy、零 overflow 或零 underrun
都不能替代无恢复和可听连续性验收。

普通软件测试与硬件执行证据分别记录在[本轮验收文档](../superpowers/plans/2026-10-16-managed-routing-low-latency-validation.md)中。音频格式边界仍为
Shared、双声道 graph、48 kHz 输出；SRC 仍是线性插值基线。
