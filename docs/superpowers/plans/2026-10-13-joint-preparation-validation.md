# T3：两端原生准备与联合输入预算

日期：2026-10-13。此切片把物理 Capture/Render 的 native preparation 与控制侧 graph/bridge preparation 分开；T4 的低库存 servo 保护和 backlog 丢弃仍未实现。

## 变更

- Capture owner 在自己的 COM 线程完成 endpoint、format、Client3/legacy 初始化、实际 period 与 capacity 查询，返回纯 Rust `NativeCaptureInfo`；控制侧收到该信息后再创建带 `PreparedInputQueue` 的 Clock Bridge。
- Render owner 在自己的 COM 线程完成同样的 native 初始化并返回 `NativeRenderInfo`；控制侧完成 graph 编译与 `DemandRenderer` 后，经一次性 handoff 把 renderer 和已解析的 `PreparedRenderQueue` 返回给该 owner。
- 两端 COM 对象和 native event 始终由创建它们的 owner 持有。控制侧只传输 timing、bounded queue endpoints、prepared renderer 和 frame scalars；不跨线程传 COM interface。
- 共同 `ActivationGate` 仍在两个 owner 都完成 attachment 后才激活。共享 stop 先于 gate，handoff receiver 同时等待 stop 和 mailbox event；取消或任一准备错误都会唤醒并 join 另一端。
- `Compatible` 解析保留 capacity 8192、prime/servo 2048 与 Render full capacity；Custom 解析把输入 capacity、prime/servo、low/high water 安装进物理 Clock Bridge，并让 servo reference 独立于 prime threshold。
- Capture report 增加 `input_queue`，schema 升为 6；Render schema 保持 5。Process Loopback 保留独立初始化，不虚构 native period capability。

## 自动验证

初始联合准备切片在导出暂存树 `4a26579324242ee62f97d9a435c8535c17cefe56` 上通过 workspace check、test、fmt 与严格 Clippy。工作区另有 UI 与 Cargo.lock 草稿；直接检查工作区曾因 `crates/moiren-app/ui/components/dev-ready.slint` 的 `placeholder-text` 与重复 `border-color` 失败。这些草稿未纳入音频切片。

本轮生命周期补强的源代码暂存树 `150fabf984918938be9c6608480ab1ddaf514844` 已重新导出，并全部通过：

- `cargo check --locked --workspace --all-targets`
- `cargo test --locked --workspace`
- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets -- -D warnings`

日志保存于 `.tmp/audio-latency-audit/staged-joint-lifecycle-20261013-133923/`。handoff 与 fake native owner 测试依赖 Windows event，属于无硬件 Windows 测试，不应称为便携测试。它们复用实际两阶段 supervisor、attachment、报告收尾与 Engine 回收路径；fake 仅替换原生 client 准备和 streaming 边界。

Clock Bridge 回归先关闭 prime trim，在 fill=16、prime=8、servo reference=16 时验证 correction=0；在 fill=8 时允许 prime，且 correction 为负。该对照直接证明启动门槛与 servo reference 独立。配置还拒绝小于 modeled minimum 的 prime 或 servo reference，以及 config/metadata 不一致。

## 生命周期补强

- 断连回归首先观察到 sender 被丢弃后 mailbox 等待超时；sender Drop 现在唤醒 event，让 receiver 报告断连。
- Render attachment 异常回归在没有 unwind 保护时失败。Engine 现在保留在 unwind 作用域之外，native/staging/DSP panic 转成失败报告，COM 在 owner 上释放，Engine 返回 joiner 销毁。异常前后的 DropProbe 都确认销毁线程是控制侧。
- 原生 metadata 改为只读 accessor 返回副本，attachment 始终用 owner 实际准备信息验证预算。
- 两端原生 owner 的退出 guard 在正常退出、metadata channel 丢失或 panic 时发出共同 stop。回归覆盖准备失败、metadata 丢失、attachment 缺失/失败、attachment 前取消、gate 前取消、激活和激活后异常。
- stop 优先于 queued renderer 时，回收 renderer 的既有 timeline 不计入本 session 的 processed_frames；有界 mailbox 同时拒绝 late handoff。

定向测试包括 18 项 preparation 测试及 2 项 prepared bridge 测试。三个缺陷均有先失败后通过的回归证据；完整 workspace 检查在上述导出树上执行。

## 当前边界

本轮已执行并用复现脚本重跑 FreeDSP 矩阵，两轮各八组、共 16 组：Compatible 对照、联合 Custom 1024/480 的 Disabled/Summary/Trace、120 秒对照与联合长测，以及两路 CPU worker 负载；原生两阶段路径的 input_queue、共同 Stop 与音频计数已经核对，详见 `2026-10-13-joint-native-validation.md`。

此前 `2026-10-13-render-target-validation.md` 的报告仍只覆盖旧输入 2048，本轮报告才覆盖实际 1024 输入。新结果是最长单次 120 秒的候选验证；20–30 分钟漂移矩阵、T4 的运行保护/积压恢复和物理端到端测量仍未完成。
