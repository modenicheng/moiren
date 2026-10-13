# T4 第一切片：运行中高水位恢复

日期：2026-10-13。基线提交：`2f8d0cc`。本切片落实审计 T4 中的运行中高水位恢复、generation 安全与短淡入；低库存 servo、包时间 feed-forward、音频年龄限制与完整低目标矩阵仍待实现。

## 行为与所有权

- 仅在已开始插值、存在 `prepared_input` 且 `trim_on_prime=true` 时启用。非空 demand 的库存达到或超过 `high_water_frames` 即触发；空 demand 不消费样本或触发恢复。
- consumer 从一次初始库存快照计算丢弃量，恢复到独立的 `servo_reference_frames`，不使用 `prime_frames` 作为运行目标。快照同时包含 ring 与已持有的 current/next；后续 producer 写入不会增加本次丢弃量。
- 只有 consumer 移动读游标。ring 丢弃通过一次有界 chunk commit 完成；不追赶 producer、不分配、不释放堆对象、不阻塞。
- 丢弃量为 1 时只移除 current，保留已从 ring 取出的 next。无条件清空这两个帧会多丢一帧，破坏精确目标。
- 每次恢复重置 phase、filtered fill、integral 与 correction，重新安装保留的插值帧。新流在 32 个成功输出帧内按 `1/32 ... 32/32` 淡入，状态跨 Engine segment 保持。
- generation 检查仍在插值之前执行，禁止跨 generation 插值。真实 generation 边界或 starvation 会按原有规则 reset/reprime，并取消未完成的恢复淡入。
- 意图明确的 skip 返回 `BoundaryReport.discontinuity=true`，自身不增加 xrun。`backlog_discarded_frames` 与 `backlog_recoveries` 独立记录；producer `dropped_frames`、native `discontinuities`、`prime_discarded_frames` 与 underrun 仍按各自原因统计。
- `trim_on_prime=false` 同时关闭启动与运行中裁剪，保留离线录音。没有 prepared watermark 的配置保留原有运行行为。恢复后的 fill scalar 与 min/max 使用裁剪后的 segment 入口库存，不能当作裁剪前峰值或最老音频年龄。

## 回归证据

最初 7 项测试在实现前的 RED 阶段为 1 通过、6 失败，失败集中在缺失恢复标记、旧音频未裁剪与恢复后 generation 边界。实现后这 7 项及新增 6 项测试均通过；Clock Bridge 总计 26 项通过。

| 覆盖 | 验证结果 |
|---|---|
| 256 帧库存恢复到 64 | 精确丢弃 192，输出从 frame 192 开始，32 帧淡入；下一次输出 frame 224，不重复 reset |
| high water 的包含边界 | fill=127 连续，fill=128 恢复 |
| 独立 prime / servo | prime=64、servo=96 时丢弃 160，恢复到 96；已有负 correction 清为 0 |
| 仅丢弃一个帧 | 保留 next，输出 frame 1 开始；没有额外跳过帧 |
| Engine 分段 | 32 与 1/3/5/7/16 的输出完全一致，恢复标记仅出现在首段 |
| 44.1 kHz | 清除旧 fractional phase，新相位按 nominal ratio 连续推进，淡入样本数值通过 |
| ring wrap | 跨 ring 尾部的丢弃保留正确前缀边界，无 producer drop |
| generation 边界 | 首个 pair 跨代时输出零帧并返回 xrun；补齐新 generation 后仅输出新样本 |
| 空 demand / 离线 / legacy | 空 demand 推迟恢复；离线与无 prepared watermark 保持原样本顺序 |
| RT 契约与报告 | ingress、恢复、32 帧淡入与 snapshot 的分配/释放计数均为 0；新增 scalar 的 JSON 值通过核对 |

既有 120 秒漂移回归也通过：44.1/48 kHz 各覆盖 ±1000 ppm 与迟包，默认目标 2048 的 underrun 和 producer dropped 均为 0。这四组使用原有默认配置，**不是** prepared 1024 低库存矩阵的准入结果。

## 提交树验证

工作区已有 UI、Cargo.toml 与 Cargo.lock 草稿，本切片仅暂存相关路径并导出检查。经过验证的源代码树为 `8b1d35ebbfea753784c8f93730eb31d3bde3f482`，完整日志在 `.tmp/audio-latency-audit/staged-backlog-recovery-20261013-144214/logs/`。

全部命令退出码为 0：

```text
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

workspace 共 510 项通过、0 项失败、4 项忽略，包含 doc-tests。此源代码树包含 7 个实现/README 路径；随后加入本记录及审计中的进度链接，未改变已验证源代码。

## 报告与剩余准入

新增 bridge scalar 后，`latency_probe` envelope schema 升为 3，应用 monitor schema 升为 4；Capture/Render schema 仍为 6/5。历史联合实机数据保留其原始 schema 与源代码 provenance。

本切片未进行新的实机采集/播放；此前 T3 的 16 组 FreeDSP 结果均来自本切片之前的代码，不能作为运行恢复或淡入的硬件验证。这里也未测量物理端到端、音频年龄或 presentation latency。

后续仍需：低库存 controller 保护与零漂移 FIFO 锯齿校准、可信包时间与 rate feed-forward、最老帧年龄限制、持续故障的控制侧降档、prepared 低目标的相位/迟包/漂移矩阵、7200 秒模拟及 20–30 分钟硬件准入，以及 render 暂停后恢复的实机验证。审计 T4 整体保持未完成。
