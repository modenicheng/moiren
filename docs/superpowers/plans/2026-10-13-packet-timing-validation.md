# T4 第四切片：音频帧与包时间的一致关联

日期：2026-10-13。基线：`e88a14d`。本切片实现有界时间关联，为后续速率估计和音频年龄限制提供原始数据。没有启用 rate feed-forward、age discard 或控制侧降档，不声称降低了实测端到端时延。

## 接口与提交关系

`ClockSource::oldest_frame_stamp()` 返回 `Option<QueuedFrameStamp>`。它在 source owner 上读取最老待用的原生帧；返回值同时包含 generation、原包首帧 position / QPC、实际接收 prefix 长度和该帧在原包中的 offset。它不读取独立更新的最新包标量，也不消耗音频。

微软的 `GetBuffer` 契约把 position 与 QPC 关联到包的首帧，QPC 已转换为 100 ns 单位，不能替换为 callback 到达时间。[S1] 实现保留这个原始 anchor 和原生帧 offset，没有用名义采样率把它们合成看似精确的逐帧时间。

`PacketStamp` 是小型 Copy 数据，包含非零 QPC；stamp 与 offset、samples、generation 一起写入同一个 `Frame`，通过现有 `write_chunk_uninit(...).fill_from_iter(...)` 提交。没有第二个元数据队列、单独的元数据满载或跨队列提交竞态。部分 overflow 只给已接收 prefix 附加真实长度；完全丢弃的新 packet 不改变队列中旧帧的关联。

startup trim、运行中 consumer discard 和环回绕移动的是完整 Frame，时间关联随样本保留。插值把 phase 推进延迟至下次输出迭代，因此查询按 phase 选择 current、next 或 ring front，覆盖 44.1 kHz 的分数相位及 48 kHz / 正 correction 下的双帧推进。返回 stamp 只说明待用帧的关联，不保证已有足够 lookahead 或 source 已完成 prime。

## 资格检查与回退

只在启用 position clock 且同一 generation 中出现两个连续、可信度足够的 packet 后附加 stamp。首次 packet、generation 更换和原生 discontinuity 先建立新 anchor，不反向修改已入环的旧帧。

以下情况产生 `None`，后续重新收集两个正常 packet：

- `TIMESTAMP_ERROR`、零 QPC、重复 / 回退 QPC。
- position 的 packet 区间溢出或不连续。
- QPC 与原生帧变化不符合名义 rate 的粗略范围。
- 没有接收任何帧，或 position-clock tracking 被禁用。

packet-slope 粗检阈值为名义关系的 1%，用有界整数运算比较差值；此值容纳单帧包的 100 ns 量化和 ±1000 ppm 模型 drift。它不是漂移估计、硬件时钟认证或 SRC 拉伸额度，既有 ±2000 ppm correction 限制保持不变。异常时间只撤销该 packet 的关联，音频仍走原有 FIFO 控制；原有 native discontinuity / position-gap / generation 语义不变。

空 packet 和非法 payload 在进入 tracker 前返回，不能污染旧 anchor。关闭 `detect_position_gaps` 的虚拟 / Process source 始终回退为无时间关联的 FIFO 模式。`BridgeSnapshot::last_qpc_100ns` 继续保持最新接收的标量语义，包括 overflow 完全丢弃的 packet；不能把它与此 API 混作音频年龄。

## 内存与 RT 契约

所有 slot 在 `capture_bridge` 准备期间分配，新增 stamp 不含 heap owner 或析构工作。现有 `capacity_frames × size_of::<Frame>()` 预算检查自动计入样本与时间关联的完整储存；只够旧 16 字节 sample/generation slot 的预算现在会被拒绝。不能只按 samples 大小计算新的 ring 预算。

查询只使用 owner-local held frame 或一次 SPSC peek，没有重试环、等待或独立发布原子事务。有效 packet 的 tracker 更新、音频读取、stamp 查询与 observer snapshot 通过新增 allocation/deallocation 回归，计数均为零。

## 回归与验证范围

先添加返回 None 的接口壳与八项核心行为回归。RED 中 7 项失败、1 项虚拟回退通过；失败包括旧 frame 不具备 queued stamp 和旧 slot 预算被错误接受。实现后八项通过，再补充分数 / 双帧推进、量化单帧正负 drift，以及已 warm 的高位置区间溢出回归。

`clock_timing` 共 11 项测试；`clock_bridge` 新增一项有效关联路径的 RT allocation 回归。覆盖实际接收 prefix 与完全丢弃新包、startup / high-water discard、generation 重新 warm、错误 QPC、空 / 非法 payload、虚拟回退、storage budget 和插值 lookahead。

验证从导出的暂存树运行，排除工作区已有 UI / Cargo 草稿。最终源树：`741109642f9da244029e0df6d9d8f2be49a6c91e`，目录：`.tmp/audio-latency-audit/staged-clock-timing-final-v2`。此前矩阵的 7200 秒记录属于 `0fc8b28`；不能把它描述为新时间关联实现的长测结果。

验证结果与源树归属：

- 初版完整实现树 `a23a6b6ff2a23d70644676c356d16cba3e55bcd4` 的全工作区 check、fmt、严格 all-targets Clippy 与完整测试通过；46 个测试套件共 531 passed、0 failed、4 ignored。
- 最终树仅强化了 native position overflow 测试：先证明第二包已具有有效关联，再断言第三包的区间溢出撤销关联。两个树的 `crates/moiren-windows-audio/src` 字节一致。
- 最终树的 release `clock_timing` 11 项与 `clock_bridge` 28 项再次全部通过；fmt 与严格全工作区 / all-targets Clippy 再次通过。
- 完整测试输出中的 144 个 `fault_case` 与 48 个 `clock_case`，逐字段与各自归档的 120 秒 CSV 完全一致。时间关联加入后，没有改变这些模型中的采样、控制或故障计数。

日志在 `.tmp/audio-latency-audit/`：`clock-timing-check.log`、`clock-timing-workspace-tests.log`、`clock-timing-focused-v2.log`、`clock-timing-fmt-v2.log`、`clock-timing-clippy-v2.log`。RED 证据为 `clock-timing-red.log`，接口壳测试树为 `f204ff86e8f8789f2bfc3071eed9ec5c2789f936`。

没有新的实机采集、播放、物理闭环或音频年龄测量。完整 T4 仍待实现可信长窗口 rate feed-forward、音频年龄限制 / 恢复、持续故障控制侧降档，以及新的实机准入。

## 参考

- [S1：IAudioCaptureClient::GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)，核读于 2026-10-13。
