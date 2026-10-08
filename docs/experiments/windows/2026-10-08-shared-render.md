# First Shared Render：Engine 到 FreeDSP 实际输出

日期：2026-10-08。Compiler 提交 `2fb754e` 已快进合入本地 main，随后直接在 main 实施单 WASAPI Shared 输出。用户显式选择“耳机（5- FreeDSP，48 kHz/stereo）”，授权 10 秒低幅度 440 Hz 测试；播放后回复“听到了”。首次 `Compiler → Engine → WASAPI → 物理输出` 可听链路通过该机器上的短时验收，M0.5 的真实输入与跨钟闭环仍未完成。

## 条件与复现

- 同日[环境记录](2026-10-08-environment.json)：Windows 11 x64 build 26300、Intel Core Ultra 9 285H、FreeDSP USB 耳机；Microsoft USB Audio 2.0 驱动 10.0.26100.9457。增强、spatial 和硬件实际采样率未另行验收。
- 只打开显式选定的输出 ID `{0.0.0.00000000}.{dedcefcb-f139-4c50-8950-6925f8c60444}`；调用前重新查询，确认 native mix 为 48 kHz、stereo、32-bit float。
- Compiler 准备 `Sine Source → Gain → Pan → Sink`，440 Hz、线性 gain 0.05、pan 0、两声道同相、Engine max block 256。只有一个 WASAPI owner 驱动 Engine；软件出口桥在该 owner 同步读写，不跨时钟。
- 不更改系统默认设备、endpoint/session 音量或 mute，不打开 capture，不写 PCM，不运行 Exclusive。只设置本流 category Other、NOPERSIST、自己的 ducking opt-out 和可选 MMCSS Audio。

```powershell
cargo run --locked -p moiren-app -- render --list
# 复现前重新确认此 ID；该命令会播放声音。
cargo run --locked -p moiren-app -- render --endpoint '{0.0.0.00000000}.{dedcefcb-f139-4c50-8950-6925f8c60444}' --seconds 10 --frequency 440 --gain 0.05 --pan 0
```

## 结果

[标量报告及状态对比](2026-10-08-shared-render-10s.json)不含音频样本。原始输出和只读目录保留在 `target/w00/2026-10-08-shared-render-{10s,before,after}.json`，未自动上传。

| 项目 | 本次结果 |
|---|---:|
| status / failure | completed / null |
| 请求 / 实际运行秒数 | 10 / 10.007983 |
| native mix / Processing format | 48 kHz / stereo / f32 |
| buffer capacity / actual engine period | 1,056 / 480 frames |
| 预填 PCM | 1,056 frames |
| 总提交 / 总处理 | 481,536 / 481,536 frames |
| DSP blocks / segments | 2,007 / 2,007 |
| audio wakes / timeout wakes | 1,001 / 0 |
| 每次可写需求 min / max | 480 / 480 frames |
| 零需求 / 空 padding 观测 | 0 / 0 |
| bridge shortfall | 0 frames |
| 非零提交 samples / peak | 963,070 / 0.05 |
| MMCSS / own-session ducking opt-out | 成功 / 成功 |
| Start / Stop | 成功 / 成功 |

总提交数包含预填、设备 padding 和停止边界，不能把 `10 × 48000` 当作严格提交计数目标。GetStreamLatency 返回 0，仅记录该 API 值，不据此推断端到端延迟。空 padding 为 0 也不单独证明没有全部类型的 underrun。用户确认听到，但没有评价主观音质、延迟、左右声道定位或所有播放帧是否到达换能器。

播放前后对比 8 个 endpoint、13 个原有 session：默认角色、endpoint 音量/mute 和可读取的原 session 音量/mute 均无变化。选定输出 scalar 保持 0.023705458，muted=false。PID 2672 的进程身份查询在两个 render endpoint 上前后均返回 0x80070005；相关 volume/mute 查询成功，错误原样保留。只观察快照差异，不恢复或归因外部设置，前后相等也不能排除中间瞬时影响。

## 软件验证与边界

Windows 本地 workspace 109 项测试通过，含一个 compile-fail doctest。严格 Clippy、fmt、all-targets check、软件 IO 应用、offline 与 compiled logical_graph 示例通过。新测试覆盖 tone 连续性/声像与控制队列、CLI 校验、零/可变 demand、bridge 首块/后续块失败统计、已有 Engine 时间线、无分配/释放、无 audio event 的 stop wake、原生格式拒绝、fake COM PCM 拷贝/取消及无麦克风设备列表。

独立审查修复两项问题：输出列表避免依赖 capture/default 查询；DSP counters 在每次成功 Engine render 后立即记录，失败也保留已完成工作且不计入 session 开始前的帧。审查复核通过。CI 配置覆盖 workspace，但本轮没有触发远程 CI；本机未运行 Linux/Miri。

本切片没有真实输入、SRC、跨钟填充控制、Plan Swap、失联恢复、GUI 或完整设备管理器。命令行通过有限时长正常 Stop；库 API 另提供独立 stop wake，尚无 CLI Ctrl+C 优雅停机。10 秒短测不替代 CPU 压力、反复启停、跨设备长时稳定性或 Takeover/OBS 共存验证。下一切片为单 capture 输入与最小 Clock Bridge/SRC 联合接入。

接口依据：[GetCurrentPadding](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getcurrentpadding)、[GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-getbuffer)、[ReleaseBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-releasebuffer)。
