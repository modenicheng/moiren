# T4 第七切片：低库存启动与预填回归

日期：2026-10-14。基线：`9831800`。本切片只增加低库存启动模型、可复核 CSV 与验证器，未改变音频运行实现。

## 覆盖范围

新增 `clock_startup.rs` 对 Capture / Render owner 的启动顺序、实际周期、首个有效音频和 Render 预填分别建模：

- 输入采样率为 44.1 kHz / 48 kHz；输出为 48 kHz。
- Render 周期为 336 / 480 输出帧，对应 309 / 336 或 441 / 480 输入帧。
- 漂移为 -1000 / 0 / +1000 ppm；336 周期相位为 0 / 84 / 168 / 335，480 周期使用 479 帧边界相位。
- owner 启动顺序为同时、Capture-first、Render-first；错开启动使用 12 个 Render 周期。
- Capture 每 17 个事件中的前 8 个增加 96 输出帧迟包，验证启动后仍无持续短读。
- Engine segment 使用 256 帧；独立回归另使用 1 / 64 / 256 帧。
- 另行模拟 legacy full-capacity Render prefill，检查零 demand 不推进 Bridge 的输出、prime discard 或 Engine timeline 计数。

## 预算结论

本模型仍以 1056 帧 native capacity 作为历史设备尺度，但使用准备阶段的联合预算计算目标：336 帧 Render 周期的 admitted 输入目标为 1024，480 帧周期需要 1280。对原审计中的 128 / 256 / 480 / 512 / 1024 小目标组合，预算检查均返回 `InsufficientInputBudget`；因此不能把 1024 帧目标的 480 周期结果描述成已准入的低库存候选。

这与现有 480 帧周期、1024 帧目标失败模拟一致，也解释了为什么降低输入库存必须与设备周期、迟包余量和 Render 目标联合协商。该结论是准备预算和离线模型结论，不是实机 Shared period 支持或端到端物理时延测量。

## 120 秒结果

`results-120.csv` 保存 90 个唯一组合，`verify.ps1` 检查列、坐标、周期换算、启动顺序、计数和 correction 限制。全部组合满足：

| 指标 | 结果 |
|---|---:|
| startup matrix cases | 90 |
| startup partial callbacks | 0 |
| short callbacks / source xruns | 0 / 0 |
| modeled Render missing frames | 0 |
| input underrun / producer dropped / reset | 0 / 0 / 0 |
| 最大双方启动后首帧等待 | 2496 output frames（480 周期模型，约 52 ms） |
| prime discarded frames | 114459 |
| priming frames | 221136 |
| low-water guard frames | 332224 |

Capture-first 的积压通过一次启动裁剪消化；Render-first 保留初始 prime 等待。两者都没有把 startup 等待或 intentional trim 伪装成稳定阶段故障。

## 7200 秒复核

同一 release 测试执行 90 个组合各 7200 秒，全部通过。独立验证器输出与 120 秒相同的计数和首帧边界；除 `seconds` 与长窗口末尾 `last_correction_ppm` 外，其余 CSV 字段逐行一致。该测试是加速离线模型，不是 7200 秒实机采集 / 播放。

```powershell
$env:MOIREN_STARTUP_SIM_SECONDS = '7200'
cargo test --locked --release -p moiren-windows-audio --test clock_startup clock_startup_joint_queue_matrix_handles_staggered_owners -- --nocapture
./docs/superpowers/plans/2026-10-13-clock-startup/verify.ps1 `
  -Csv ./docs/superpowers/plans/2026-10-13-clock-startup/results-7200.csv -Seconds 7200
```

验证器负例也已复现并拒绝：缺少一行、重复坐标、非零 startup partial callback、隐藏 Capture-first 裁剪、非法 phase、非 finite correction、非法计数字符串。

## 提交树验证

源代码暂存树：`9a2b46f029e012044a47fe969096ee358016c547`。工作区已有 UI、Cargo 与 lifecycle 草稿未纳入本切片。该暂存树通过：

```text
cargo check --locked --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

聚焦测试 `clock_startup` 为 3 passed；完整 workspace 测试仍包含既有 clock fault 长测。未进行新的实机采集 / 播放、物理端到端测量或控制侧持续故障降档，因此 T4 整体与 T5 硬件准入仍未完成。
