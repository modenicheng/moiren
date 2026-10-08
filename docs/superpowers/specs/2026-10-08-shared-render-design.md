# First WASAPI Shared Render

依据已接受的 M0.5 路线和用户“继续；后续直接在 main 工作，先合并”的指令，本轮在 `main@2fb754e` 实施。第一步 Compiler 已快进合并；原 Compressor 草稿属于用户编辑，不擅自补完或删除。

## 范围与选择

选择单输出 owner 直接驱动唯一 Engine，先固定已实测的 native mix 48 kHz / stereo / interleaved f32。另一方案是独立 Graph 线程与 demand 队列，会增加同步/延迟；第三种方案是先实现完整设备管理器、格式协商和 SRC，会延后首次真实 PCM 输出。本轮采用第一方案，其他格式明确拒绝，后续再扩展。

设备由用户显式选择 opaque endpoint ID，不猜设备名或自动跟随默认设备。调用方负责在控制侧用 Compiler 准备 `Sine Source → Gain → Pan → Output`；测试信号默认为 440 Hz、线性 gain 0.05，两声道同相信号。无 DC 测试信号，不自动修改 endpoint/session 音量或系统默认设备。本轮用户已选择 FreeDSP 耳机并授权 10 秒测试；播放结束后确认“听到了”，结果见[实机记录](../../experiments/windows/2026-10-08-shared-render.md)。

## 数据与所有权

`moiren-windows-audio::render::DemandRenderer` 是可跨平台测试的无设备数据路径，拥有 Engine<f32> 和对应软件输出桥 reader。检查 Processing SR 为 48000、两个声道和桥容量，按 Engine 的 max block 拆分设备需求；每块 render 后立即读取同线程桥到调用方 staging slice。零需求不调用 DSP；不完整 bridge transfer 初始化缺失区并返回错误。此桥仅是同 owner 的 planar/interleaved 出口，不宣称跨设备时钟适配。

Windows `render::wasapi` owner 负责 COM、IAudioClient、audio event、render service、MMCSS 和 driver buffer lease。stream 初始化前校验实际 mix format，并预分配实际 buffer capacity 大小的 staging。预填真正 PCM 后 Start；每次 audio wake 读取 padding，计算 `capacity - padding`，在 driver lease 前完成 DSP/staging；GetBuffer(N) 后只做有界 copy，ReleaseBuffer(N) 配对并在同线程执行。错误 lease 通过 guard 取消，旧 samples 不会被提交。

`RenderSession` 提供 start/request_stop/join，stop 为共享标准 OwnedHandle 包装的独立 manual-reset event。等待集合优先包含 stop，再包含 audio event；超时最多 100 ms，并有 1..600 秒运行上限。无音频事件或零 demand 时仍可停止。COM 创建、调用及最终 Release 均在 owner；正常/错误 Stop 后离开 streaming 阶段再析构 Engine、queues、services、events 和 apartment。Drop 也发送 stop 并 join，避免后台 worker 漏出生命周期。

成功音频循环不分配、不日志格式化或写盘，只记录有界标量统计：primed/submitted/processed frames、DSP blocks、audio/timeout/zero-demand wakes、empty padding observations、bridge shortfall、实际 capacity/period/latency、MMCSS 和 Stop 结果。空 padding 只是一项诊断，不直接称为已证明的 underrun。Windows/Engine 错误保留 stage 和 HRESULT/typed error，在 Stop 后转换为可序列化报告。

## 入口与验收

`moiren-app render --list` 列可选 render endpoints，`moiren-app render --endpoint <ID> --seconds <1..600> [--frequency <Hz>] [--gain <0..1>] [--pan <-1..1>]` 播放明确选择的测试链路；原离线入口保持可用。JSON 报告在 owner 退出、资源释放后输出。

纯测试覆盖 0/小/超过 max block demand、可变 block 的正弦连续性、Gain/Pan 参数、桥短缺、格式/端点/期限/对齐错误、stop event 在没有 audio event 时仍唤醒和块渲染分配/释放计数。全 workspace 检查与既有离线例子通过。用户选择设备后跑 10 秒实机 smoke，用户确认听到声音；可再按授权进行更长运行，不把只提交成功记为听感通过。

本轮不接入 capture/process loopback、SRC/clock bridge、热插拔重建、Plan Swap、GUI 或全设备管理；不将单次 smoke 记为日常可用稳定性验收。

官方契约：[Initialize](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize)、[GetCurrentPadding](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getcurrentpadding)、[GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-getbuffer)、[ReleaseBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-releasebuffer)。
