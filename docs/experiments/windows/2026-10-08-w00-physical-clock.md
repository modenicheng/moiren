# W00：物理输入、静音输出与时钟

日期：2026-10-08。范围：Realtek 内置与 FreeDSP USB 耳麦的四个 Shared endpoint 同时运行；3 秒定位、60 秒观测、300 秒确认。四条流均正常启动、运行与 Stop。两路 capture 有非零信号；静音 render 不验收可听输出。全过程只保留统计和时间戳元数据。

## 条件与复现

- 本机 Windows 11 x64 build 26300、Intel Core Ultra 9 285H；Realtek 6.0.9802.1、Microsoft USB Audio 2.0 10.0.26100.9457。沿用同日的[环境记录](2026-10-08-environment.json)，增强 / spatial 和硬件 nominal SR 仍为 Unknown。
- QQMusic.exe PID 33356 持续通过 FreeDSP 播放；不切换默认设备，不改音量 / mute，不安装或调整驱动，不使用 Exclusive。Realtek 输出原本 muted、scalar 0，保持原样。
- [60 秒汇总](2026-10-08-physical-60s.json)、[300 秒汇总](2026-10-08-physical-300s.json)保留完整前后目录和 session 快照、clock 异常例子、间隔分布和 30 秒分段回归。原始全量 trace 位于 `target/w00/2026-10-08-physical-{60s,300s}.json`，不含 PCM。
- 四个 COM owner 各在独立 MTA 线程使用 native mix format、category Other、事件通知和 MMCSS Audio。先预填 SILENT render，再按 `GetBufferSize - GetCurrentPadding` 补静音。capture 完整获取和释放 packet；无输入到输出回路。

```powershell
# 复现前先重新 --list 并检查这些 opaque ID 仍指向目标设备。
$endpointIds = @(
    '{0.0.0.00000000}.{c20f8868-6ff2-476f-8734-a1e2812fa650}',
    '{0.0.0.00000000}.{dedcefcb-f139-4c50-8950-6925f8c60444}',
    '{0.0.1.00000000}.{0a18f197-0246-471d-9142-5ea67fe99450}',
    '{0.0.1.00000000}.{0e3bce79-16b7-4594-8ef2-5db33dfb70b6}'
)
$music = @(Get-CimInstance Win32_Process -Filter "Name = 'QQMusic.exe'")
if ($music.Count -ne 1) { throw 'Expected exactly one QQMusic process.' }
$probeArgs = @('--seconds', '300', '--observe-pid', $music[0].ProcessId)
foreach ($endpointId in $endpointIds) { $probeArgs += @('--endpoint', $endpointId) }
cargo run -p moiren-windows-audio --example w00_physical --locked -- @probeArgs |
    Set-Content -LiteralPath target/w00/physical.json -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw "Probe failed: $LASTEXITCODE" }
python crates/moiren-windows-audio/scripts/summarize_physical.py `
    target/w00/physical.json docs/experiments/windows/physical-summary.json
```

## 格式、输入与输出

| endpoint | 方向 / channels | Shared stream format | buffer | actual engine period | GetFrequency API units/s |
|---|---|---|---:|---:|---:|
| 扬声器 Realtek | render / 2 | 48 kHz interleaved f32 | 1,056 frames | 480 frames / 10 ms | 384,000 |
| 耳机 FreeDSP | render / 2 | 48 kHz interleaved f32 | 1,056 frames | 480 frames / 10 ms | 384,000 |
| 麦克风阵列 Realtek | capture / 4 | 48 kHz interleaved f32 | 1,056 frames | 480 frames / 10 ms | 768,000 |
| 麦克风 FreeDSP | capture / 1 | 48 kHz interleaved f32 | 1,056 frames | 480 frames / 10 ms | 192,000 |

这些是 mix / stream 数据；未将它们称为硬件实际格式。Realtek 四通道 channel mask 为 0，通道语义未验收。默认 / 最小 period 为 10 / 3 ms，本次实际 Shared engine period 为 10 ms。GetStreamLatency 均返回 0；这个 API 返回值不能证明真实输入输出延迟为 0。

300 秒结果：

| 项目 | Realtek capture | FreeDSP capture | 两路 render（各自） |
|---|---:|---:|---:|
| packets / wakes | 29,999 packets | 30,000 packets | 30,001 wakes |
| frames | 14,399,520 | 14,400,000 | 提交 14,400,480 + 预填 1,056 |
| RMS / peak | 0.00177383 / 0.172982 | 0.00502080 / 0.443916 | 全部 SILENT |
| 启动 / 后续 discontinuity | 1 / 0 | 1 / 0 | 不适用 |
| packet timestamp error / 回退 | 0 / 0 | 0 / 0 | 不适用 |
| packet position gap | 0 | 0 | 不适用 |
| 空 padding / 零需求 wakes | 不适用 | 不适用 | 0 / 0 |
| 每次可写 frames | 不适用 | 不适用 | min = max = 480 |
| API error / metadata dropped | 0 / 0 | 0 / 0 | 0 / 0 |

所有 capture packet 均为 480 frames，无 NaN / Inf。非零信号可以包含背景噪声，未录音试听，也没有验证通道映射、语言清晰度或声学路径。少一包与 render 多提交的边界帧包含启动 / 停机与 padding，不能仅按整 300 秒理论帧数判定丢帧。

到达间隔 median 约 10 ms，P99：Realtek render 10.2313 ms、FreeDSP render 10.3073 ms、Realtek capture 10.1978 ms、FreeDSP capture 10.1348 ms。到达抖动与设备时钟 drift 分开分析。300 秒过程中有日常工具 / 文档工作，末段还发生一次编译检查；这不是隔离空载基准。

## 时钟与时间戳异常

IAudioClock position 的单位必须与 GetFrequency 配对。capture packet 的 position 则是首帧位置，除以 stream SR；两者不能混用。本机频率数值与 `SR × block_align` 一致，但报告仍将 IAudioClock 原值标为 API units。[GetPosition](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock-getposition)、[GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)

两路 capture 的 IAudioClock 均出现：position 每个 wake 递增，QPC 每两个 wake 才更新；首个 QPC 为 0，HRESULT 仍为 S_OK。300 秒中 Realtek 有 14,999 次、FreeDSP 有 15,000 次相同 QPC 对应不同 position。position 本身没有回退。把读取放在 packet drain 前或后，两个 3 秒定位实验均复现；它们保存在 `target/w00/2026-10-08-physical-{smoke-fixed,clock-before-buffer}.json`。

这些不是 S_FALSE，也不是 capture packet 的 TIMESTAMP_ERROR。探针标记为 `inconsistent_qpc_reads`，保留原始元数据，并拒绝对这组 IAudioClock 配对做 rate fit；不把该问题误报为 position reset，也不套用任意时间偏移修补数据。

capture packet frame/QPC 配对正常推进，无间断、无错误标志，因此对比采用 `capture_packet_frames / sample_rate`；render 采用 `IAudioClock_position / GetFrequency`。FreeDSP capture 首帧位置有时非零，本次为 6,180,000；算法先减去各流原点，不要求启动位置为 0。

剔除每流首秒，再取共同 QPC 区间。300 秒共同 fit span 约 298.42 秒、每流约 29,843 个样本；诊断计数保留全程。独立 Python 回归与 Rust 整段结果核对差异小于 0.001 ppm。

| endpoint / comparison source | 60 秒相对 QPC ppm | 300 秒相对 QPC ppm | 300 秒 residual RMS |
|---|---:|---:|---:|
| Realtek render / IAudioClock | +0.0127 | −0.1512 | 35.41 µs |
| FreeDSP render / IAudioClock | −0.1961 | −0.1821 | 49.99 µs |
| Realtek capture / packet | +0.0108 | −0.1598 | 34.66 µs |
| FreeDSP capture / packet | −0.2102 | −0.1484 | 18.19 µs |

300 秒任意两流的整段相对差最大约 **0.03374 ppm**；两路输出相对差约 0.03092 ppm。30 秒分段各流对 QPC 的 ppm 范围合计约 −0.7414 至 +0.4172，显著大于整段两流差；不把微小整段差当成已知、稳定的晶振误差。GetFrequency 是常数 nominal 单位换算，并不会持续更新以反映漂移。[GetFrequency](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock-getfrequency)

本机 Shared/driver 暴露速率在该窗口内接近 QPC；这不证明物理设备共用晶振、不保证长期无漂移，也不能据此省略 W09 的 follower bridge。此次未连接 capture 与 render，未测 ring fill、自适应 SRC、输出间相位同步或真实端到端延迟。

## IAudioClock2 原始设备位置补测

300 秒确认后，同样四路另运行 60 秒，增加每 wake 的 `IAudioClock2::GetDevicePosition`。四路均可取得该接口，调用 HRESULT 全为 S_OK；采集与静音输出仍正常，前后设置无变化。[60 秒补测汇总](2026-10-08-physical-clock2-60s.json)，原始 trace 为 `target/w00/2026-10-08-physical-clock2-60s.json`。

| endpoint | 原始 position / QPC | 同一窗口估计 device frames/s | residual RMS |
|---|---|---:|---:|
| Realtek render | 有效推进，6,002 reads | 48,000.00751 | 3.429 device frames |
| FreeDSP render | 有效推进，6,001 reads | 48,000.01490 | 4.331 device frames |
| Realtek capture | 6,000 reads 的 position 和 QPC 全为 0 | 不拟合 | 不适用 |
| FreeDSP capture | 6,001 reads 的 position 和 QPC 全为 0 | 不拟合 | 不适用 |

原始设备帧数与客户端流位置分开记录；FreeDSP render 的 position 有较大起点，算法减原点后拟合。该 API 的单位明确为 device frames；设备 SR 可能不同于客户端 mix SR，因此只报告观测 frames/s，不凭 48 kHz mix format 计算硬件 nominal drift ppm。[GetDevicePosition](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock2-getdeviceposition)

capture 再次证明“接口取得 / S_OK”不足以认定 clock 数据可用。本机两种 capture 时钟接口都有数据限制，packet frame/QPC 仍可用。该补测不把不可用数据升格为一般性平台不支持。

## 探针修正与验证边界

首次 3 秒运行的输出成功，capture 在 SetDuckingPreference 上返回 `0x88890003`（AUDCLNT_E_WRONG_ENDPOINT_TYPE）。拆开错误阶段后确认 GetService / cast 成功，失败的是该方法。改为只调用探针自己的 render session；capture 保持 category Other 后运行成功。失败原始文件为 `target/w00/2026-10-08-capture-ducking-failure.json`，属于探针调用范围问题，不归为 capture 平台不支持。此偏好针对自己的 session，不能代替保护 QQMusic。[SetDuckingPreference](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-setduckingpreference)

长 trace 的 residual 原计算存在大数相减消减误差：300 秒合成 trace 的真实 0.5 µs 残差被算成约 5.08 µs。增加回归测试，并改用二次扫描直接累加 residual，避免为本机误报抖动。

验证：`cargo fmt -p moiren-windows-audio --check`、package all-target check、Clippy `-D warnings` 全部通过；20 个 package 测试通过。公共 Streaming / packet / MMCSS guards 提取后，另做 1 秒 QQMusic Process Loopback 回归，97 packets、94 个非零 packet、正常 Stop、无 API 错误，原始结果为 `target/w00/2026-10-08-process-regression-after-physical.json`。期间一次 `cargo test` 在旧 physical exe 仍运行时遇到 Windows LNK1104 文件占用；待流正常完成后重跑全部通过。

60 秒和 300 秒前后快照中，默认角色、四个物理 endpoint 的 volume / mute、QQMusic session 均无变化。QQMusic session scalar 0.4、muted false、Active，FreeDSP render scalar 0.023705458、muted false。相关快照读取无错误。未做持续听感监听，不能排除期间短暂可听影响。

## 本轮结论

- **已实测支持（本机 / 当前格式 / 5 分钟）：**四路并发 Shared 初始化、采集或静音提交、MMCSS、正常 Stop；native f32 输入统计与完整 packet release；Shared render padding demand；render IAudioClock 与 capture packet 时钟观测。两路 render 的 IAudioClock2 数据另有 60 秒有效实测。
- **需要指定条件：**按 flow 和来源处理时钟单位；本机 capture IAudioClock 的 position/QPC 配对不可用于 rate fit，IAudioClock2 全零，使用有效 packet 时间戳；非零启动位置要减原点；不要把 mix format、API latency 或近零相对 rate 等同物理事实。
- **未测试：**已知可听信号输出、输入到输出回路、零需求实际设备场景、格式 / period 更改、USB 拔插 / 失联恢复、30 分钟 / 2 小时稳定性、跨钟 ring / SRC、自适应 bridge、Exclusive 与 ASIO。
- **结果无法判定：**真实硬件晶振关系、声学路径 / 延迟、播放期间听感完全无影响。
- **已知不支持：**没有新增一般性平台结论。capture 上的 ducking 方法错误和不可用时间戳只登记本机调用 / 数据条件。
