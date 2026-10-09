# Compressor 与运行时 Plan 切换

用户已要求继续实现 compressor，随后实现运行时 ExecutionPlan 切换。本设计沿用 runtime foundation 的块边界 publish / swap / retire 契约。

Compressor 保留草稿的 10 个 ParameterId，增益、threshold、knee 使用 dB，attack / release / hold 使用 ms，ratio 至少为 1，mix 为 [0, 1]。使用前馈 peak detector，以全部输入声道的最大绝对值联动；二次 soft knee 计算目标 gain reduction，随后在 dB 域执行 attack / hold / release，一阶时间常数为指定 ms。零 attack / release 立即响应，hold 仅延迟 release。输入增益在 detector 和湿声路径前；makeup 仅用于湿声，output gain 用于混合后。dry 为原始输入。支持 f32 / f64、Separate / InPlace、逐样本自动化和跨块持久状态，无延迟、无 RT 分配。新增 LogicalGraph Compressor 类型及 compiler 默认 / 显式参数绑定。

计划切换采用完整预备 runtime 所有权包，含 ExecutionPlan、RtResources、ParameterRuntime。单 pending SPSC 与有界 retire SPSC 均在控制侧创建；Engine::render 在参数消费和 DSP 前执行一次切换，维持 timeline。相同 EngineConfig 和 epoch、递增 revision 为热切换条件；改变采样率、块容量、epoch 需要停止和重新准备。publish 失败返回候选，不吞掉所有权；retire 满时不接收候选。控制侧丢失或音频停止时，RT 不析构候选或旧计划；停止后将 Engine 移回控制侧销毁所有队列。

控制侧持有不可变 PlanSnapshot，可在 RT 运行中检查候选兼容性，不读取可变 DSP。显式 ProcessorReuse 映射旧、新 ProcessorId，校验类型、role、端口及参数 domain，拒绝重复映射；RT 只交换预计算索引中的 processor Box，复制参数状态以保持当前值和 ramp 进度。调用方负责保证同类型 processor 的静态配置兼容；IO backend 复用由调用方显式选择。替换实例随旧 runtime 回收。候选基于过时快照身份时退回 retire，保持旧 active；仅比较 registry 身份不足以证明参数表或 schedule 仍匹配。

每份参数通道带共享 retired 标记；旧通道拒绝新提交，已接受但未应用事件在控制侧回收旧 runtime 时返回 StaleRevision，保留已有 Applied 回复及背压语义。候选自己的事件只在激活后应用。

不提供 crossfade、lookahead、外部 sidechain、设备重建或自动 epoch 切换。Windows demand renderer 可使用启用切换的 Engine；其输出 bridge 必须通过显式复用保持身份。离线例子与测试演示 compiler 换图和回收，不启动可听设备。

测试覆盖静态 knee、联动、时间常数、hold、mix、自动化、IO/schema 拒绝、variable block 等价；切换覆盖边界、timeline、revision、背压、取消/关闭、状态与 ramp 迁移、陈旧参数、析构线程和 f32 / f64 的零 RT 分配/释放。

静态曲线依据 [Giannoulis / Massberg / Reiss, JAES 2012](https://joshreiss.github.io/documents/2012/GiannoulisMassbergReiss-dynamicrangecompression-JAES2012.pdf)。时间常数与 hold / mix 是本项目上述明确约定。
