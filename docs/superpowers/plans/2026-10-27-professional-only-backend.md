# Professional-only backend（2026-10-27）

> 落地 `docs/audit/2026-10-27/06-consolidation-plan.md` 的 P3（APP-1 app legacy 驱动栈退役）+ P4（双后端退役 W-K）的代码部分。分支 `feat/backend-professional-only`（worktree `.worktrees/professional-only`）。实机冒烟/soak 由用户执行后合并。

## 结果

- **单后端**：`ManagedDriver::windows()` 恒返 `ProfessionalBackend`（f32/f64）；legacy `engine_backend`/`render`/`shared_stream` 栈、app 侧 `BackendDriver` 会话栈全部删除（本分支累计净删 ~2.1 万行）。
- **目录布局**：`professional/` 嵌套消失。后端实现占 `engine_backend/`（backend/owner/observations/recovery/validation/virtual_io），`wasapi.rs`、`asio.rs`+`asio/`、`bridge.rs` 与中立层 `resample/servo/codec/native/format` 均在 crate 根。
- **观察体系**（WA-4）：app 只消费 professional 形状；虚拟端点也会发布行（含纯虚拟项目）。

## 能力对齐（WA-1 四缺口的处置）

1. **latency profile（D-2 方案 A）**：Balanced→512、Low→256 帧设备 buffer 请求（无显式 buffer 时注入 `DeviceOptions.period_frames`；显式请求与 per-device override 恒优先）；Compatible=设备默认。档位选择器对所有配置可见，profile 变更恒触发重启。
2. **捕获时钟适配**：已补齐。Pump 每唤醒发布 (累计帧数, QPC) 到 seqlock 写时钟；RingSource 以 `clock_bridge::RateClock` 双窗一致（≥1s、≥8 包、≤100ppm 符合度）输出前馈 ppm，与 occupancy 伺服求和；带积压回收（水位=3×伺服基准，弃至基准 + 32 帧淡入）、停顿回收（250ms 写时钟超时）与间隙回收（≥4 周期内交付低于名义一半）。补给改为按块 `input_frames_needed` 有界搬运，ring 保持权威积压。rt-check 零分配断言覆盖。
3. **虚拟端点观察**：已补齐。VirtualRealtime 路径的 transport 恒转 follower session，activate 发布、status 标记 running。
4. **每唤醒遥测**：**放弃并记录**。professional 保留 `NativeStreamStats` 累计计数器与 `driver_latency`（seqlock）；per-wake QPC/需求/填充直方图随 legacy 删除（LatencyRecorder 仅存进程环回 Summary 模式）。理由：诊断收益不抵 RT 热路径加记录点；需要时按 `latency/telemetry.rs` 现有内核复活。

## 语义取代记录

- monitor 恢复/淡入 fallback（recovery.rs、driver.rs 旧 337-387）：由 professional `RecoveryMonitor`（单次有界恢复）+ engine `WaitingForResources`/失败保持旧图不断信号取代。`BackendDriver` 在生产本就不可达（auto-apply #9 后仅测试构造），`monitor_diagnostics` 生产恒 None。
- Tone 幅度（D-3）：统一为 professional 的 0.1。
- SetSend（D-5）：随 legacy 重建分支删除，resolved-wontfix。
- managed 自动启动不重试不可用 catalog（与 legacy 1s 重试的分歧）：保留 managed 语义。

## 行为变化（用户可见）

- CLI `monitor`/`render`/`tone` 子命令删除（legacy render/capture 栈上的工具；桌面 UI 与 `--gain` 离线演示保留）。
- 设置页延迟档位对所有配置可见；Balanced/Low 语义=设备 buffer 请求（非 legacy 队列/恢复策略）。
- 诊断面板 legacy 行（shared period 请求、queue 水位、boundary probes）移除；设备页 Clock 标签展示 professional 实际流时序行。

## 测试覆盖变化

- 移植进 `src/service/managed/tests.rs`：catalog 行/失败保行、自动启动一次性消费默认会话、零信号需求下的控制/退役。
- 随语义退役：`tests/service_recovery.rs`（12）、`tests/service_backend.rs`（24）中 Parameters 账本/饱和/timeline 专属用例（ACK 屏障等价物已在 managed 套件）、monitoring.rs 六个 tone/monitor fixture 用例（MeterBridge 交付与 backend-free 可见性谓词保留）。
- 新增：bridge 单测（前馈收敛、积压/间隙/淡入）、虚拟观察发布、档位→buffer 映射。

## 实机冒烟清单（合并前，用户执行）

1. 默认端点三 role（console/multimedia/communications）Shared 48k 起声
2. 延迟档位 Compatible/Balanced/Low 切换：诊断面板显示实际 period；档位变更重启生效
3. 显式设置各一：采样率 96k、Exclusive、显式 buffer、ASIO（如有）
4. VAD 全虚拟项目：虚拟播放+虚拟麦克风诊断行出现
5. 进程环回
6. 物理捕获长时漂移（≥30min，对比切换前电平表/听感）
7. 拔设备/换默认端点：RecoveryMonitor 恢复或干净失败（不断信号）
8. tone 会话
9. 过夜 soak（主机安静）

## 后续（未在本分支）

- P0/P1/P2 工单见整合计划（用户另有 worktree 在做 chore/p0-zero-risk-hygiene、refactor/p1-app）。
- `professional` 类型名（`ProfessionalBackend`/`Professional*Observation`）保留以控制 diff；可作后续纯重命名。
