# 子代理 4:频谱 / 监控 / 电平表(频谱新线 + 本分支 2 提交)

审计范围:`main @ 3321062`(PR #93 频谱线:`d5654f5 → 3c90ff7 → c5513ae → 92c7142 → 3e2d517 → 818257d`)+ 当前分支 `fix/spectrum-standard-curve`(`ed93c58` RBW 分辨率带宽、`e9387d2` 对数十进网格),工作区干净。分区文件逐一通读:

- `crates/moiren-app/src/ui/monitoring.rs`(1198 行)
- `crates/moiren-app/src/ui/monitoring/spectrum.rs`(872 行)
- `crates/moiren-app/src/ui/monitoring/ballistics.rs`(473 行)
- `crates/moiren-app/src/ui/widgets/meters.rs`(354 行)
- `crates/moiren-engine/src/spectrum.rs`(261 行)+ `spectrum/fft.rs` + `spectrum/tests.rs`
- 消费/泵送面:`ui/window.rs`(pump_meters / meters_in_motion)、`ui/window/mixer.rs`(update_node_meters)、`ui/window/graph.rs` + `graph/page.rs`(可见集注册、node_spectrum 读者)、`ui/pages/graph_canvas_view{.rs,_view/nodes.rs}`(热层绘制)、`service/managed/meters.rs`(d5654f5 中继)、`service/signals.rs`(主题注册)

方法:纯静态阅读 + 手工数值复算(每条数学断言算到底);未跑 cargo。所有发现标"已亲证"或注明疑案。

**总结论:频谱新线的数学核心(带功率/ENBW 归一化、RBW 下限、ISO 网格、对数十进网格、弹道)经逐项复算全部成立,未发现 P0/P1;发现 1×P2(热层每帧分配与提交信息宣称的"shell-owned scratch copy"不符)+ 5×P3。**

---

## 发现表

| # | ID | 级别 | 标题 | 位置 | 已亲证 |
|---|---|---|---|---|---|
| 1 | SP-1 | P2 | 热层每帧为每张分析卡分配新 `Box<[f32]>`,与 c5513ae 提交信息宣称的"shell-owned scratch copy 免每帧分配"相反;`NodeSpectrumReading` 文档注释亦失实 | `ui/window/graph/page.rs:372`、`ui/pages/graph_canvas_view.rs:205-209, 39-41` | ✓ |
| 2 | SP-2 | P3 | `project_spectrum` 每窗把各通道 bins 拷进新建 `Vec<Box<[f32]>>`,使 spectrum.rs:234-235"稳态窗口零分配"的注释失实(可用按通道切片免拷) | `ui/monitoring/spectrum.rs:164-170` | ✓ |
| 3 | SP-3 | P3 | 顶带越出 Nyquist 时,Nyquist bin 被赋以 ±half_bin 覆盖,带宽上侧 half bin 为幻影覆盖(量化 ≤0.04 dB 过读,仅顶带) | `ui/monitoring/spectrum.rs:173-188, 105-108` | ✓(数值) |
| 4 | SP-4 | P3 | 频谱订阅失败的 `Error` 状态在下一次 poll 被覆写为 `Waiting`,错误对用户不可见(复刻了电平路径 monitoring.rs:616-622 的既有模式) | `ui/monitoring/spectrum.rs:448-451` | ✓ |
| 5 | SP-5 | P3 | "ISO 266 base-2"为错误标准配对:ISO 266 优选频率是基 10(10^(3/10))系列;基 2 1/12 倍频程网格应引 IEC 61260 base-two。数值影响 ≤1.1% 频率(轴宽 0.15%),不可见 | `ui/monitoring/spectrum.rs:2-4, 67-70` | ✓(标准 + 数值) |
| 6 | SP-6 | P3 | 对数十进网格使每帧每卡竖向 hairline 路径数 13 → 31(默认跨度),全部在每动画帧重绘的热层里逐条 build/paint | `ui/pages/graph_canvas_view/nodes.rs:561-583` | ✓ |

无 P0(RT 侧全绿,见"已核对无问题清单"第 15 条)、无 P1(投影数学与刻度单源逐项复算成立)。

---

## 详述

### SP-1 (P2) 热层每帧分配曲线副本,与提交信息/注释宣称的设计相反

- 位置:`crates/moiren-app/src/ui/window/graph/page.rs:362-377`
  ```rust
  Rc::new(move |id, cx: &App| {
      let state = this.read(cx).state.read(cx);
      state.node_spectra.curve(id)
          .map(|curve| pages::graph_canvas_view::NodeSpectrumReading {
              points: curve.points.into(),   // &[f32] → Box<[f32]>:分配 + 拷贝
              ...
  ```
  调用点在 `graph_canvas_view.rs:205-209` 的 canvas paint 闭包内(`paint_canvas_layer`,每个可见分析卡每帧一次);gpui `canvas` 的 paint 每帧执行,动画期间(最高 240 Hz)每卡每帧分配 121×4 B = 484 B 的 Box。
- 三处宣称与代码不符:
  1. c5513ae 提交信息:"the hot layer repaints it every animation frame **from a shell-owned scratch copy** of the curve (…, **and a per-frame allocation would be a wakeup cost of its own**)"——c5513ae 落地版本即已是 `curve.points.into()`(经 `git show c5513ae:…/graph/page.rs` 亲证),scratch copy 从未存在;
  2. `graph_canvas_view.rs:39-41` 注释:"an owned copy **taken at rebuild time** … (**the hot layer reads that table directly**)"——拷贝发生在每次 paint 而非 rebuild,热层与缓存层走的是同一个分配闭包,并不"直接读表";
  3. 相邻的 `NodeMeterReading` 是 `Copy` 零分配,频谱读者是画布路径上唯一每帧分配点,与本仓库既有标准(HC-14:避免每 tick 克隆)不一致。
- 量级:240 Hz × 484 B ≈ 116 KB/s/卡,绝对量小;但按本仓库自己的口径这是热层分配债 + 文档/提交叙事失实(与 HY-3 同类)。
- 修复方向:`NodeSpectrumFn` 返回借用(读者闭包内即时投影到 `SpectrumCurve<'_>`),或 shell 持有 per-node scratch `Box<[f32]>` 复用;同步修正两处注释。

### SP-2 (P3) `project_spectrum` 每窗新建通道 Box 拷贝,"稳态零分配"注释失实

- 位置:`crates/moiren-app/src/ui/monitoring/spectrum.rs:164-170`
  ```rust
  let channel = |index: usize| -> Box<[f32]> {
      bins[index * bins_per_channel..][..bins_per_channel].iter().copied().collect()
  };
  let channels: Vec<Box<[f32]>> = (0..header.channels).map(channel).collect();
  ```
  每个到达窗口(默认 2048/hop1024 @48k ≈ 47 窗/s/卡)分配 1 个 Vec + channels 个 Box 并整份拷贝 bins(2ch×1025×4B ≈ 8.2 KB,≈385 KB/s/卡)。而同结构体里 `scratch`/`targets` 的存在理由正是注释 "A spare projection buffer **so steady-state windows never allocate**"(spectrum.rs:234-235)——通道拆分使这句对每窗不成立(每窗 1+N 次堆分配)。
- 无借用障碍:`bins: &[f32]` 与 `band_power` 的 `&[f32]` 参数可直接传子切片 `&bins[ch*bpc..][..bpc]`,零拷贝零分配。
- 严重级:UI 线程、量级小,定 P3;按 HC-14 的口径若上调亦可辩。

### SP-3 (P3) 顶带越过 Nyquist 时计入 Nyquist bin 的幻影覆盖

- 机理:`band_power` 把 bin i 视为覆盖 `[i·bin_hz − half_bin, i·bin_hz + half_bin]`(spectrum.rs:176-182)。最后一个 bin(Nyquist,中心 fs/2)的真实谱覆盖只有 `[fs/2 − half_bin, fs/2]`,代码却给它对称的 half_bin 上侧。当顶带中心被 `center <= nyquist` 过滤(spectrum.rs:93-97)保留且带边 `center·2^(1/24)` 越过 fs/2 时,越界部分按幻影覆盖计入权重。
- 数值例:fs = 41 000(fft 2048,bin 19.97 Hz):顶中心 20158.7 ≤ 20500 保留,带宽(含 RBW 下限)≈ [19584, 20750];Nyquist bin 覆盖被记作 [20490.1, 20510.0],其中 [20500, 20510] 为幻影 10 Hz;顶带总覆盖 ≈ 1166 Hz → 幻影占比 0.86% ≈ 0.037 dB 过读。仅影响最顶一个点、且需 Nyquist 落在顶带内;48 kHz 默认跨度(顶带 hi 20750 < 24000)完全不触发。
- 对照:低端对称情形(窗低于 bin 1 下缘、DC bin 排除)代码与文档明确按"clipped by the transform's usable span"少读;顶端未做对称钳制。修复一行:bin_hi 取 `min(bin_hi, nyquist)` 或限制 Nyquist bin 的 overlap 上界为 fs/2。P3(量化不可感知)。

### SP-4 (P3) 订阅失败后的 `Error` 状态被下一次 poll 覆写为 `Waiting`

- 位置:`spectrum.rs:448-451`
  ```rust
  let Some(subscriber) = self.subscriber.as_mut() else {
      self.status = MeterStatus::Waiting;
      return raw_changed;
  };
  ```
  `SpectrumSubscription::new` 中 `subscribe_topic` 失败时置 `MeterStatus::Error`(spectrum.rs:369-377),ticket/subscriber 均为 None;下一次 `poll` 只要主题本身未 Closed/NoPublisher,就走进上面临 else 分支,`Error` → `Waiting for a complete window`,且因既无 ticket 也无 subscriber,状态永远停在 Waiting——用户看到"等待窗口"而非订阅错误。与电平路径 `monitoring.rs:616-622` 逐字相同(既有模式在新代码里复制,非新引入的回退)。修复:Waiting 覆写前保留已有 Error(`self.status = self.status.clone().keep_error_or(Waiting)` 或仅当当前非 Error 时置 Waiting),两路径同修。

### SP-5 (P3) "ISO 266" 标准引注错位:基 2 网格配了基 10 的标准号

- 位置:`spectrum.rs:2-4`("projected onto an ISO 266 fractional-octave grid")、`:67-70`("ISO 266 base-2 centers at 1/12-octave spacing")、测试名 `twelfth_octave_grid_is_iso_266_anchored_at_one_kilohertz`。
- 机理:ISO 266(优选频率)的 1/3 倍频程系列是基 10 约定(步长 10^(3/10) = 1.9953,如 1000/1250/1600/2000);分数倍频程带的基 2 与基 10 双约定由 IEC 61260(及 ANSI/ASA S1.11)定义,基 2 即本代码的 `1000·2^(k/12)`。代码自身是自洽的基 2(锚 1 kHz、步 2^(1/12),与 RTA 传统一致),但把标准号写成了 ISO 266。
- 数值差异(若真按 ISO 266 基 10 取点):每步 10^(1/40)/2^(1/12) = 0.99981,±54 步到 20 Hz/20 kHz 处累计 ≈1.1% 频率偏移,轴宽占比 log2(1.011)/10 ≈ 0.16%——视觉与带电平均不可辨。纯文档级修正:引注改为 "IEC 61260 base-two fractional-octave grid"。

### SP-6 (P3) 十进网格的 hairline 路径数量翻倍且逐条 build/paint

- 位置:`nodes.rs:561-583`。旧 1-2-5 序列固定 10 条竖线;新十进网格在默认跨度(19.686..20158.7)画 base 10/100/1000/10000 各 mantissa 1..9 经区间过滤 = 8+9+9+2 = **28 条竖线**,加 3 条 dB 横线共 31 条,每条独立 `PathBuilder::stroke` + `paint_path`(`hairline`,:466-473)。这段代码在 `paint_spectrum_body` 里,由热层**每动画帧**对每张可见分析卡执行(网格本身是静态家具)。另:每十年循环固定跑 9 个 mantissa 再过滤,属无害小浪费。
- 修复方向(可选):网格(背景 + dB 线 + 十进线 + 标签)挪入缓存层一次构建,热层只画曲线;或单 Path 累积多段。P3(量级:每帧 ≤31 个小 path/卡)。

---

## 数学核对清单(逐项亲证)

1. **引擎窗归一化(周期 Hann,幅度谱)**:`coherent_scale = 2/Σw`,Σw(周期 Hann)= N/2 → 4/N;内部 bin 乘 2(单边折叠)、DC/Nyquist 不乘(spectrum.rs:149-156, 206-213)。实余弦 A·cos bin 居中时 `|X_m| = AN/4` → 峰 bin 读 A;裙边 bin m±1 因周期 Hann 的 W[±1] = N/4 读 A/2,其余为 0——与 PR #93 测试伪造的 0.25/0.5/0.25 精确一致(我先用连续核 G(1)=0.5 验证方向,再用周期 Hann 的三点谱精确复核;`reference_amplitudes` 的独立 O(N²) DFT 测试在 16..256 全尺寸锁定)。
2. **ENBW = 1.5 bin 精确**:`N·Σw²/(Σw)² = N·(3N/8)/((N/2)²) = 1.5`,与 `SpectrumParams::enbw_bins` 一致。
3. **音调校准经带功率**:bin 居中音调全裙在带内时 Σaₖ² = A² + 2(A/2)² = 1.5A²,除以 ENBW 1.5 → 恰为 A²,满幅正弦读 0 dBFS(测试容差 ±0.01 dB 成立)。宽带读数按"正弦等效 dBFS"约定自洽(宽带带内功率读 10·log10(2μ_band),与音调的幅度约定一致,非 +3 dB 错误)。
4. **bin→频率映射**:中心 `i·fs/N`(引擎文档与 UI `bin_hz` 一致),覆盖 ±half_bin;无 (i+0.5) 偏差。
5. **band_power 区间算术**:`first = floor((low+half)/bin_hz).max(1)` 恒等于"最小 i 使 i·bin_hz + half > low"(因 +half 与 −half 恰差 1 个 bin, floor 平移恒等);`bin_lo >= high` 提前 break;`overlap > 0` 严格剔除边界切触;low 为负时 f64→usize 饱和转换得 0 再 max(1) 排除 DC。相邻未加宽带精确铺贴无缝无重叠(band i 上边 = center_i·2^(1/24) = band i+1 下边),边界 bin 对其中一边 overlap 恰为 0,无双重计数。
6. **RBW 下限(ed93c58)**:带宽 `max(带宽, 1.5·bin_hz)` 围带中心对称加宽;以 2048@48k DC 注入复核:bin 1(23.4 Hz)幅度 1.0(DC 经 Hann 泄漏满幅,W[±1] 机理同上),点 0..6(中心 19.7..27.9,窗 [c−17.6, c+17.6] 全含 bin 1 覆盖 [11.7, 35.2])平顶 10·log10(1/1.5) = −1.76 dB;点 7/8/9 部分覆盖读 −1.77/−2.14/−2.54 dB 单调肩部;点 18(55.8 Hz)窗 [38.2, 73.4] 与 bin 1 无交 → REST_DB——与分支新测试断言逐点吻合。16 点 FFT 子分辨用例:峰点(2996.6 Hz)窗 [746.6, 5246.6],K = 1.0(bin1) + 0.249(bin2),读 −1.749 dB vs 期望 −1.761(Δ0.012 < 0.05)。低端不足 bin 时窗口被可用谱段截断 → 少读不虚读,与注释意图一致;唯一例外见 SP-3 顶端。
7. **FFT 尺寸无关性**:平坦 bins 1024 vs 2048(a 缩 1/√2)同密度带级差 < 0.3 dB(测试锁定;K·a²/ENBW 密度守恒)。
8. **ISO 网格**:k ∈ [−68, 52] 共 121 点,centers[68] = 1000 Hz,默认 low/high = 1000·2^(∓68/12)/2^(52/12) 精确落在格点上;K_EPSILON 1e-9 防 52.0000 浮点抖落到 51;Nyquist 钳制仅截顶端、低端不动(测试锁定慢速率)。基 2 vs 基 10 约定差异见 SP-5。
9. **对数十进网格(e9387d2)**:lo 19.686 → first_decade = 10;decades = floor(log10(2015.87)) + 1 = 4;base 10/100/1000/10000 × mantissa 1..9 经 [lo, hi] 闭区间过滤 → 20..90, 100..900, 1000..9000, 10000..20000;强线(mantissa 1)= 100/1k/10k 与标签一致。hi/first > 1 恒成立(hi > lo ≥ first_decade),decades ≥ 1 无退化;hi=20000 时 20000 线保留(闭区间)。lo=100 时 first_decade=100,mantissa 1 = 100 正常。
10. **dB 轴与静止门**:y_of 在 [REST_DB, 0] 线性,底 = REST_DB = floor_db = clamp 下界 = at_rest 阈值 = 绘制跳过条件,四者一致;线宽 1.5·scale 被 content mask 裁进 plot。
11. **频谱弹道/唤醒**:上升即时(max 钉住)、下降 30 dB/s 按墙钟(per-poll 间隔求和与泵频无关);窗间锯齿 ≈ 30·0.0213 = 0.64 dB @47 Hz 窗流,不可见;`advance` 在 points=None 时清 `decayed_at`——我追了一个候选 bug(恢复后首帧 7.5 dB 下跳)并被此行证伪;Stale(500ms)/换率/NoPublisher 都置 points=None 且同 poll 内清 decayed_at,恢复时 elapsed=0 无跳变。唤醒仅凭投影差 > 0.05 dB 或状态迁移;重复发布同曲线不唤醒(测试 24 次复发布断言不醒)。换采样率经 targets=None → `is_none_or` 为真 → 唤醒 ✓。
12. **电平弹道**:RMS 一阶积分 `fraction = 1 − exp(−window/τ)` 精确离散;自然衰减 10·log10(e)/0.150 = 28.953 dB/s;峰值条即落即衰、marker 保持后按偏好速率、windowless 帧全层保持(交付间隙不读作电平跌落);刷新率无关性有 60/120/144/240 Hz 测试锁定。
13. **刻度单源(对齐电平表)**:5 个表面——graph 卡(`nodes.rs:158/166` 经 `paint_meter_bars`)、mixer 条带(`mixer/page.rs:693 paint_channel_meter`)、托盘混音(`tray_mixer.rs:539`)、侧栏(`sidebar.rs:321`)——全部经 `paint_meter_fill(_horizontal)`/`meter_spans` 单源;新分析卡竖向表未私造刻度,底边与绘图区对齐属布局几何(92c7142/3e2d517,布局测试锁定)。`meter_height` 显式反转 FaderScale 尾段(meters.rs:116-133),`SCALE_TICK_DB` 与 `FaderScale::ANCHORS.last` 有测试钉住——FaderScale 逆映射历史坑未复发。
14. **中继与订阅面**:d5654f5 的 MeterBridge 中继在控制线程,形状失配窗跳过并等下次 sync 重注册;服务循环 park ≤10ms + 信号泵 1ms 到期,中继频率 >> 47 Hz 窗流,中间窗基本不丢(即使丢, Latest 语义 + 显示衰减使其不可见)。UI 侧 `valid_spectrum_window` 拒 NaN/∞/fs≤0/长度失配/非单调窗,投影无除零路径(bin_hz > 0、overlap ≤ 1、power ≥ 0、1e-18 下限后 clamp)。
15. **RT 约束**:`SignalSpectrumAnalyzer` 的 ring/窗/FFT 工作区/发布容量全部在控制线程预备;`observe` 路径零分配零阻塞(需求门关闭时零样本转换,`CONVERSIONS` 计数测试锁定);退役计划在控制侧回收。引擎侧本分支 diff 仅改默认值 2048/1024。

---

## 既往项复核(2026-10-31 / 2026-10-27)

| 项 | 本轮状态 |
|---|---|
| SD-5(RMS 自然衰减注释 ~58 实为 ~29 dB/s) | **已修且未复发**:ballistics.rs:35-39 `RMS_NATURAL_DECAY_DB_PER_S = 10·LOG10_E/0.15` ≈ 28.95,注释与代码一致,body 下落用积分器自身速率、与偏好解耦,`rms_body_falls_at_the_integrator_rate_under_every_falloff_preference` 对全部偏好档锁定(8259be7,早于 PR #93) |
| APP-3 / meter_spans 单源 | **未回退**:5 表面仍共享 painter(清单第 13 条),新分析卡表面亦走共享 painter |
| FaderScale 逆映射坑 | **未复发**(meters.rs:127-132 显式尾段反转 + ANCHORS 钉住测试) |
| REST_DB 停帧门 | **未复发且扩展一致**:频谱以同值 REST_DB 作 floor/静止阈/绘制跳过,`meters_in_motion` 并入 `node_spectra.at_rest()`(window.rs:585) |
| EN-2(engine 频谱读出端无消费者,2026-10-27 决策项) | **已解决**:d5654f5→3c90ff7 接线到应用信号总线与 UI 订阅 |

---

## 已核对无问题清单(其余猎捕方向)

- **窗归一化/幅度守恒**:见清单 1-3;DC/Nyquist 不折叠、内部 bin 折叠,引擎测试逐 bin 对独立 DFT 锁定。
- **带求和闭开区间/相邻带缝隙**:见清单 5,精确铺贴,边界零重叠计数。
- **RBW 下限"宁少读不虚读"**:低端被可用谱段截断(少读);唯一例外为 SP-3 顶端幻影覆盖(≤0.04 dB)。
- **log 域 0 频边界**:格点最低 19.686 Hz,x_of/log2 只作用于正频率;十进网格线最低 20 Hz,无 0 频路径。
- **fs=0/NaN/单 bin 带除零/−inf 钳制/曲线点数与带数一致/颜色域**:valid_spectrum_window 全拒;bin_hz > 0;`level_of` 的 1e-18 下限 + clamp(floor, ceiling);points.len == centers.len 由构造保证;填充/描边为固定 tint 透明度,无颜色映射域。
- **每帧重算带功率?**:否——投影每窗一次(47 Hz),衰减每泵一步 O(n),绘制每帧 O(n) 折线;SP-1/SP-2 为其中仅有的分配点。
- **spline 平滑每帧分配/低频 ramp 稳定性**:ed93c58 已删除 Catmull-Rom spline(改为折线,防过冲显示谱中不存在的电平,理由成立);92c7142 的低频 ramp 已被 RBW 下限一并取代,capture.rs 演示曲线同步删除平底段——两条死路径清理干净,无残留。
- **频谱订阅生命周期**:会话代重置、SUBSCRIPTION_GRACE 750ms 防抖、topic 身份变更重建、Mixer 页不保 FFT 需求(mixer.rs:54, 68-81)与电平路径对称。
- **`OctaveGrid` 退化输入**:fs/2 < 1k 时 nyquist 下限 1k;格点恒 ≥1;paint 侧 hi>lo 过滤 + 名义跨度回退。
- **serde 部分字段契约**:`[("fft_size", 64)]` 解码得 (64, 1024) 为 invalid 组合,由 `valid()` 在构造时拒绝——PR #93 前即同形((64, 512) 亦 invalid),非回归,测试明确钉住该契约。
