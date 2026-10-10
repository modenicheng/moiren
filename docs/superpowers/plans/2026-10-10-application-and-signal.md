# Application Runtime and Signal Bus Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付不阻塞 UI 的应用运行时，以及跨图连续、按需计算、RT 零分配的通用信号总线。

**Architecture:** UI 与 tray 共用主线程；AppService 串行管理应用状态、参数提交和有界 signal pump；输出 owner 同时运行 Engine；准备和阻塞回收交给最多两个普通 worker。参数领域留在 engine，观测放入独立 moiren-signal。

**Tech Stack:** Rust 2024、Slint 1.18.1、windows 0.62.2、rtrb =0.4.0、triple_buffer =9.0.0、std::thread / 有界通道；不引入 async runtime。

## Global Constraints

- RT 零分配、零扩容、零堆内存释放；不加锁、阻塞、日志、名称查找、Any/downcast、UI callback 或通用 executor 唤醒。
- UI + tray 共用 1 个主线程，AppService 1 个普通线程，每个独立输出时钟域 1 个输出 owner；普通任务 worker 最多 2 个。
- 参数保留 engine::control 的公共路径与原有时序语义；不创建参数线程、parameter crate 或 signal::command。
- Topic 单一有效发布绑定；同一 RT host 内兼容换图不清理 Latest、Stream、sequence、注册 generation 或业务订阅。
- 不覆盖当前用户的 Slint/UI、Cargo 与 main.rs 改动；执行前重新检查 git status，按职责合并。
- 用户已授权完成全部计划，并允许为接入调整前端；后端稳定、性能与职责边界优先。任务完成状态必须由当前代码和验证证据证明。

---

## 已确认设计与执行顺序

设计依据：[应用运行时](../specs/2026-10-10-application-runtime-design.md)、[信号总线](../specs/2026-10-10-rt-ui-signal-bus-design.md)。以 2026-10-10 工作区为基线；现有 GUI 原型由用户修改中，不能用旧 HEAD 覆盖。

| 顺序 | 子计划 | 独立交付与验收 |
| --- | --- | --- |
| A | [应用服务、会话和托盘](2026-10-10-application-runtime.md) | GUI 启停真实会话、真实目录、参数控制；隐藏到 tray 继续运行；启动/退出不阻塞 UI |
| B | [通用 signal crate](2026-10-10-signal-crate.md) | 不依赖 engine/Slint 的值及块 Latest/Stream、独立订阅、预算和有界 pump；独立测试及示例 |
| C | [Engine 与展示接入](2026-10-10-signal-engine-ui.md) | 任意输出端口观察、compressor 内部发布、跨图连续、真实 UI 电平及可见性需求 |

A 与 B 没有生产代码依赖，C 依赖两者。当前按 A → B → C 串行实施，使用 subagent-driven-development 逐项实现及独立审查；同一时刻只有一个实现任务写代码，避免交叉修改。

## 开工检查

- [x] 执行 `git status --short`、`git diff --stat`，记录原有修改。当前原有修改包括 Cargo.lock、moiren-app/Cargo.toml、main.rs、build.rs、ui/、designs/ 与 .tmp/。现有前端与计划保存为功能分支基线；designs/ 与 .tmp/ 保持原样。
- [x] 执行 `cargo test --locked -p moiren-engine --test runtime`（26 passed）和 `cargo check --locked -p moiren-app`（通过，有原型 build.rs/main.rs 警告），记录真实基线；不把历史测试当作新实现证据。
- [ ] 依次完成各子计划的任务，每个任务只提交明确列出的变更；Cargo.lock 仅纳入本任务的依赖更新。

## 全部交付的最终门槛

- [ ] `cargo test --locked --workspace`。
- [ ] `cargo clippy --locked --workspace --all-targets -- -D warnings`；区分原有警告与新增问题，不扩大本次产品修改来消除不相关警告。
- [ ] `cargo fmt --all -- --check`、`git diff --check`。
- [ ] 执行 signal 的自定义块示例与 engine 的 signal_internal_state 示例，记录输出与线程/分配证据。
- [ ] Windows 实机执行有限音量的显式设备监听与测试音；验证停止、隐藏/恢复、进程退出与显式退出。不同设备结果不得用模拟测试代替。
- [ ] 对 Slint 实际运行界面查看截图并操作；验收空目录、Starting/Stopping/Failed、真实电平、切页和滚动退订；截图存 `.tmp/`，不提交。
- [ ] 更新三份计划的任务勾选和证据；列出未执行的实机项。未经实机验证，不宣称硬件交付通过。

## 已处理的接口与范围选择

1. 参数模块拆分但不迁出 engine，保持既有请求/回复通路。
2. 服务承载 pump；无独立路由线程和 async runtime。
3. 持续运行用显式 UntilStopped；旧 CLI 仍只接受 1..=600 秒。
4. 准备好的输出等待 service 激活，过期完成直接回收；不短暂播放已取消的会话。
5. 单会话 GUI 先落地；多输出、录音、IPC、插件、自动重连只说明架构落点，不扩大本次实施范围。
6. 首批 UI 交付输入/输出逐声道电平；通用块与压缩量交付真实 API、测试和例子，不新增波形绘制页或压缩器产品页。

## 覆盖自检

| 关键约束 | 落地任务 |
| --- | --- |
| UI/tray/service/owner 分工、持续运行、退出 | A1–A5 |
| Latest/Stream、值/块、安全初始化、独立订阅 | B1–B3 |
| 零需求、新启用代次、Stream fence、分层 stats | B2–B4 |
| 显式注销、Busy/Closed/NoPublisher、预算保活 | B1、B4、B5 |
| stable host、scoped signals、候选不能发布、跨图连续 | B5、C1、C3 |
| 任意输出观察、参数分段、内部压缩量 | C1–C2 |
| 窗口/页面/viewport 需求及真实显示 | A5、C4 |
| RT 分配/释放、并发安全、真实运行证据 | B6、C3–C4、最终门槛 |
