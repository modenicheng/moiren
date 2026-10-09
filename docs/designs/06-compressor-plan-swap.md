# Compressor 与运行时 ExecutionPlan 切换

日期：2026-10-09。两项能力均位于 engine；编译、准备、失败清理和资源最终析构由控制侧完成，RT 在音频块边界移动拥有权。

## Compressor

`NodeKind::Compressor` 接受正数声道数，固定 input 0 / output 0，支持 f32 / f64、Separate / InPlace IO。`NodeBindings::bind_compressor(node, settings)` 可指定初始值；不绑定时使用以下默认值。底层 `Compressor::parameters(processor, settings)` 返回全部 10 个 ParamSpec，原草稿 ParameterId 保持不变。

| ID / 参数 | 单位 | 默认值 | 允许范围 |
| --- | --- | --- | --- |
| 0 INPUT_GAIN | dB | 0 | [-60, 60] |
| 1 THRESHOLD | dBFS | -18 | [-120, 24] |
| 2 RATIO | 比率 | 4 | [1, 100] |
| 3 ATTACK | ms | 10 | [0, 60000] |
| 4 RELEASE | ms | 100 | [0, 60000] |
| 5 HOLD | ms | 0 | [0, 60000] |
| 6 KNEE | dB | 6 | [0, 60] |
| 7 MAKEUP_GAIN | dB | 0 | [-60, 60] |
| 8 OUTPUTGAIN / OUTPUT_GAIN | dB | 0 | [-60, 60] |
| 9 MIX | 湿声比例 | 1 | [0, 1] |

全部参数为有限 Float，支持现有逐样本 ramp。prepare 检查整个 domain，防止后续自动化跨出 DSP 支持范围。

输入 gain 同时影响 detector 与湿声；全部声道的最大绝对值驱动一个联动 peak detector，统一施加 gain reduction。静态 knee 采用 [Giannoulis / Massberg / Reiss, JAES 2012，式 (4)](https://joshreiss.github.io/documents/2012/GiannoulisMassbergReiss-dynamicrangecompression-JAES2012.pdf) 的二次过渡，knee 为零时使用 hard knee。不提供外部 sidechain、lookahead 或独立声道压缩。

gain reduction 在 dB 域用一阶 attack / release 平滑，时间常数为指定 ms；零时间立即响应。目标 reduction 不小于当前值时执行 attack 并重新开始 hold；低于当前值时等待 `ceil(hold_ms * processing_sr / 1000)` 个样本再 release。hold 用整数样本计数，跨 block 与参数事件分段保持；attack 从不等待 hold。

dry 为原始输入；makeup 仅用于湿声，output gain 在混合之后。处理固定状态，不分配 scratch，不裁剪，也不保证任意浮点 headroom 与 gain 组合下输出有限。算法延迟为零。

## 准备、发布与回收

```text
Control: compile → PreparedPlan::new → [with_reuse] → publish
RT:      render(valid frames) → reserve retire → accept one pending
         → transfer state / swap → parameters → DSP → advance timeline
Control: poll_retired → reject_pending / drain old ControlPort → drop
```

在将 Engine 交给 render owner 前，调用 `engine.enable_plan_switching(retire_capacity)` 获得 PlanControlPort；pending 固定容量 1，retire 容量由调用方指定且非零。候选来自完整、未 render 的 Engine，可通过现有 compiler 准备；不能把已经开启切换或已退休的 Engine 当作候选。

`publish(candidate)` 要求 EngineConfig 的采样率、最大 block、事件预算完全相同，timeline epoch 相同，revision 严格大于本通道所有已发布 revision。即使候选被取消或拒绝，其 revision 也不会再使用。配置 / epoch 改变必须停止 callback 并重新准备。publish 失败的 `PublishFailure.plan` 返回原候选，可在控制侧重试或销毁；完整编译或准备失败不会触碰 active。

`Engine::render` 在帧数 / timeline 溢出检查之后、参数消费与 DSP 之前最多切换一次，保持 timeline 和 epoch。retire 队列满时不取候选，继续处理旧 active；无音频 demand 时不切换。`cancel_pending()` 请求取消最新发布的候选，已经提交的切换可能先完成；确认结果需读取 `RetireOutcome`。该接口不自动 coalesce 多次编辑。

成功切换返回旧 runtime 和 `Replaced { active_revision, frame }`，取消 / 过时复用返回候选和 `Rejected(reason)`。`PlanControlPort::active_revision()` 提供原子 revision 快照；取得该 revision 不代表当块 DSP 已完成。使用对应候选的 ControlPort 和 CompiledBindings 提交新参数，事件在激活后才消费。

## 显式复用与参数

在控制侧保存 `engine.plan_snapshot()`，或在发布前保存 `candidate.snapshot()`，供下一次准备使用；这些只包含不可变 metadata，不访问 RT 的可变 DSP state。`with_reuse(&basis, &[ProcessorReuse { old, new }])` 校验类型身份、role、latency、IO mode / 端口 / 声道及参数 ID / domain。映射不能重复或引用缺失实例。

调用方仍须保证静态配置兼容：同一 Rust 类型可能代表不同文件、设备或 bridge，类型检查不能证明它们配置相同。复用意味着保留旧实例；新准备的实例在回收包中销毁。IO adapter 的类型身份会穿透 compiler 的类型擦除 wrapper。

RT 按预计算索引移动 processor Box，并复制当前参数值与 FloatRamp 进度。候选的初始值不覆盖复用状态；要主动修改，向候选 ControlPort 提交参数事件。未复用的实例使用新准备状态与初始参数。active 快照身份与 basis 不同则整份候选退回 Control，active 不变，不部分迁移；即使停机后沿用原 registry，新 Engine 的参数表 / schedule 也不能冒充旧快照。

NodeId / EdgeId 属于逻辑图，ProcessorId 属于编译结果。跨计划必须从两份 bindings 获取映射；保留 edge 参数也需显式复用对应 Send processor。对 `DemandRenderer` 使用在线切换时，须复用现有输出 sink / writer 以保留其外部 reader，不能把新 bridge 隐式当成旧 bridge。

旧 ParameterRuntime 退休后，旧 ControlPort 拒绝新提交。已应用的 Applied 回复仍可读取；已接受但未应用的请求经 `RetiredPlan::reject_pending()` 产生 StaleRevision。返回值为尚未回复的请求数；回复队列满时需保留 RetiredPlan、排空旧 ControlPort 并重试，直至返回零再析构。参数 owner 已停止且 consumer 被释放时也拒绝新提交。

## 关闭与验证

PlanControlPort 丢失时，RT 保持 active，不接收剩余候选；已排队对象仍由 RT owner 的 queue endpoint 保持。外层停止设备 callback / render loop，将 Engine 交回非 RT 侧，再 drop 或 `into_parts`。启用切换后的 `into_parts` 同时在调用线程释放切换端点，因此也必须在停止后、非 RT 调用。不能在仍执行的 callback 中析构 Engine、PreparedPlan、RetiredPlan 或任一队列。

```sh
cargo run --locked -p moiren-engine --example plan_swap
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

离线示例在 frame 4 插入 Compressor，使 Gain / Sink 的 ProcessorId 变化；复用 Source / Gain / Sink，验证 8 样本 ramp 连续、timeline 连续及原输出桥不变。DSP 测试覆盖静态 knee、联动、attack / release、exact hold、mix、全部 10 项 ramp、窗口与 variable block；生命周期测试覆盖背压、取消、陈旧 basis、参数回复背压、停止和控制端丢失。线程局部 allocator 验证 f32 / f64 渲染与换图零分配 / 释放，独立线程测试确认 active / retired / pending 实例最终在控制线程析构。这些结论限于被测试路径，不代表自动迁移所有用户 DSP、无声 crossfade 或真实设备在线编辑已验收。
