# W00 物理输入 / 静音输出 / 时钟观测

**Goal：**验证 Realtek 内置与 FreeDSP USB 的 Shared capture、Shared render demand、clock position / QPC 能力，以及设备之间的相对速率。

**Architecture：**复用已有 COM / HANDLE / packet guards，增加 `physical` 探针和纯 `clock` 分析。每个显式 endpoint 在自己的 MTA owner 线程持有全部 COM 对象，主线程仅接收纯数据。输入使用实际 mix format（本机四路均为 f32）；输出填充静音帧。packet / clock / demand 元数据在 streaming 前预分配，停机后写 JSON。

**约束：**继续 W00；QQMusic 原播放设置不变；不修改 default / volume / mute，不安装或配置 driver，不使用 Exclusive，不创建输入到输出回路，不保存 PCM。stream category 明确设 Other。SetDuckingPreference 只作用于探针自己的 render session，capture 上的该方法实测返回端点类型错误；它不能修改或保护 QQMusic。静音 render 只证明 buffer / clock 工作，不证明可听信号到达扬声器。

**文件：**`crates/moiren-windows-audio/src/clock.rs`（frequency / QPC 换算与回归）、`src/physical.rs`（端点 owner 与报告）、`src/owner.rs` / `src/probe.rs`（公共 guards）、`examples/w00_physical.rs`（显式 endpoint CLI）、README、实验记录。

- [x] 纯 clock 测试覆盖 byte / frame 单位、±ppm、静止 position、S_FALSE 排除、reset / 回退、相同 QPC 窗口和相对比值；增加长 trace 微秒残差回归测试，修正消减误差。
- [x] 独立 endpoint 解析 flow、native f32 格式、buffer、default / actual engine period、latency、clock frequency；保持 Unknown 与错误阶段。
- [x] capture 完整获取与 Release，记录 SILENT / discontinuity / timestamp error；render 预填充，按 buffer minus padding 请求，零需求跳过，记录 submitted / padding / demand。
- [x] 每次 wake 调用 IAudioClock::GetPosition，保留原始 HRESULT、position units 和 100 ns QPC；停机后在相同 QPC 窗口回归 position/frequency 对 QPC，不用 wake timing 推导物理 drift。
- [x] 将 capture 的有效 frame / QPC packet clock 与异常 IAudioClock 观测分开；可选读取 IAudioClock2 的原始 device frames，以 frames/s 汇报，不猜硬件 nominal SR；另做 60 秒实测。
- [x] 枚举并显式选定四个物理 endpoint；先 3 秒 smoke，再同时观测 60 秒，最后做 5 分钟确认。采集前后对比 QQMusic 的 session 与 endpoint 设置。
- [x] package format / check / tests / Clippy；保存原始 trace 到 `target/w00/`，本地实验汇总写入 `docs/experiments/windows/`。本轮不宣称 2 小时稳定性、可听输出验收或已完成 W09 自适应 bridge。

时钟报告应同时给出 fit span、sample count、相对 QPC 的 ppm、fit residual 和设备间 ppm。IAudioClock position 标为 API units，按 GetFrequency 换算；capture packet 与 IAudioClock2 的 position 是各自契约下的 frames，分开记录。跨钟结果只是本机 Windows/driver 暴露时钟的观测，不直接断言晶振共用关系。

结果：[物理设备与时钟实测记录](../experiments/windows/2026-10-08-w00-physical-clock.md)。capture IAudioClock 的 QPC 隔次重复、IAudioClock2 position/QPC 全零，均拒绝拟合；采用有效 packet frame/QPC。300 秒四流整段最大相对差约 0.03374 ppm，30 秒分段波动更大，不能据此省略 follower bridge。20 个 package 测试及静态检查通过。
