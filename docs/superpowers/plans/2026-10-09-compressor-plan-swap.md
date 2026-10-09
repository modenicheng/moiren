# Compressor 与 Plan 切换实施计划

**Goal:** 完成可编译、可自动化的 compressor，以及控制侧 prepare / publish、RT swap、控制侧 retire 的在线换图。

**Architecture:** Compressor 是持久 RtProcessor；完整候选 runtime 经单 pending SPSC 发布。兼容复用与参数迁移使用控制侧快照和预计算映射，RT 只移动所有权与复制数值。

**Tech Stack:** Rust 2024、已有 rtrb 0.4.0、workspace Cargo 测试。

## 全局约束

- 沿用 compressor 草稿 ParameterId 0..9；不丢弃用户草稿需求。
- f32 / f64；RT 不分配、释放、锁、日志、设备调用。
- 固定 EngineConfig 和 timeline epoch；revision 严格递增，timeline 保持。
- 一份 pending，retire 满时保持 active；publish 失败返回候选。
- 所有新队列创建及最终销毁在控制侧；停止后移回 Engine。
- 不打开实际音频设备，不改变系统音频设置。

## Task 1 — Compressor DSP 与 Compiler

- [x] 在 processor/tests/compressor.rs 先添加静态曲线、联动、hold、时间常数、mix 和分块等价测试，执行 `cargo test --locked -p moiren-engine --lib compressor` 确认草稿缺失接口导致失败。
- [x] 完成 processor/builtin/compressor.rs、导出 Compressor / CompressorSettings。参数 schema 校验覆盖未来自动化域，而不只验证初始值。
- [x] 添加 NodeKind::Compressor、NodeBindings::bind_compressor、compiler lowering 和端到端控制参数测试。
- [x] 执行 engine/core 测试并审查数值与 IO 契约。

## Task 2 — Runtime publish / swap / retire

- [x] tests/runtime/swap.rs 先验证块边界、timeline 连续、pending / retire 背压、配置拒绝、控制侧回收和状态迁移；执行 `cargo test --locked -p moiren-engine --test runtime swap` 验证缺失接口失败。
- [x] runtime/swap.rs 提供 PlanSnapshot、PreparedPlan、ProcessorReuse、PlanControlPort、RetiredPlan；Engine 启用队列，render 边界消费一次。
- [x] control.rs 添加 retired 标记和兼容参数状态迁移；控制侧回收旧请求，旧通道不再接收新请求。
- [x] 使用既有 allocator 计数，验证切换与背压零分配/释放；覆盖独立控制 / 音频线程退出。

## Task 3 — 集成与验证

- [x] examples/plan_swap.rs 演示 compiler 编辑图、候选发布、显式 processor 复用及控制侧回收。
- [x] 更新 README、compiler 契约与运行时实施记录，明确静态配置兼容、输出 bridge 复用与停机限制。
- [x] `cargo test --locked --workspace`、`cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check` 和离线示例通过。
- [x] 独立审查 compressor 与整份并发生命周期改动，修复实质问题后记录结果。

## 验证记录

- 工作于既有 main 工作区，保留 compressor 草稿的全部参数编号；没有启动真实设备或更改系统音频设置。
- Compressor DSP 11 项测试，包括全部参数逐样本自动化；compiler 的 f32 / f64 Compressor render 检查分配 / 释放均为 0。
- Runtime 15 项换图 / 原位分配测试，其中双精度 Compressor envelope 迁移可在候选生效时继续 release 并接收 makeup 事件。
- 生命周期测试使用独立 render 线程，验证 active / retired / pending 最终析构线程以及控制端消失时的零 RT 释放。
- RED / GREEN 回归修复：hold 浮点计时多等待一个样本；参数 owner 停止后仍接受提交；停机后复用相同 registry 时旧快照误匹配新参数表。最终比较完整 active 快照身份。
- `cargo test --locked --workspace --quiet`：141 项测试全部通过，含 1 项 compile-fail doctest。
- `cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`git diff --check` 通过。
- `offline`、`logical_graph`、`plan_swap` 三个离线例子通过；换图例子在 frame 4 从 revision 1 切换为 2，timeline 到 8，保留原 Gain ramp 与输出桥。
- 独立只读审查通过，未发现 Critical / Important 问题；唯一 Minor 为原位 Compressor 的分配计数缺口，已补双精度、跨 block / segment 自动化的零分配 / 释放测试并通过最终 workspace 检查。
