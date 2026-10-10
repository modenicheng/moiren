# Generic Signal Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立任意 Copy 值与固定容量数据块的 Latest/Stream 总线，支持独立订阅、有界分发及同一稳定 RT host 的发布权限交接。

**Architecture:** 非 RT Bus 负责目录、注册/订阅与 pump；唯一 RtHost 拥有发布能力，prepared handle 只有类型和绑定身份，调用权限由不可复制的 scope 提供。Topic 资源与 graph 无关，使用 Latest 三槽或 Stream 固定池；每订阅者有独立存储。

**Tech Stack:** Rust 2024、rtrb =0.4.0、triple_buffer =9.0.0、thiserror 2、std 原子；独立 moiren-signal，不依赖 engine、core、Slint、Windows 或 Tokio。

## Global Constraints

- `T/H/E: Copy + Send + Sync + 'static`；不要求 Default，不接受拥有堆内存的 Vec/String 作为负载。
- RT 零分配、零扩容、零堆内存释放；不加锁、阻塞、日志、名称查找、Any/downcast 或 UI/executor 唤醒。
- Latest/Stream 模式注册时固定；只需 Latest 不分配或发布 Stream；需两种模式注册两个 Topic。
- 一个 Topic 一个有效 publisher binding；候选只有 prepared 权限，同一 RT host 块边界交接；不支持跨 RT host 迁移。
- Stream 满容量丢新消息，保留已入队项；读 guard 只占本订阅存储，不能阻塞其他订阅或 RT。
- Topic 显式注销；Busy 不改变状态；NoPublisher、Closed 与无新数据分别表达。
- 总线不创建线程，pump 由调用方驱动；默认应用预算 32 MiB、最多 128 Topic、每 Topic 最多 64 业务订阅、单份负载复制布局最大 1 MiB。上限均为非 RT 可配置，不能运行中扩容。
- Stream 默认 ingress/订阅深度均 8；全部资源准备侧分配并计入预算，包括 borrowed slot、暂存及元数据。

---

## 文件与公共类型

新 crate 目录为 `crates/moiren-signal/`。文件分工：

| 文件 | 职责 |
| --- | --- |
| src/lib.rs、types.rs、error.rs | 门面、四种 shape、ID/状态/配置/错误 |
| src/budget.rs、registry.rs、lifecycle.rs | 布局预算、名称/type 解析、引用和关闭 |
| src/demand.rs、sequence.rs | 需求/代次原子状态、发布序号和订阅 fence |
| src/storage.rs、transport/{mod,latest,stream}.rs | 安全初始化；原地 Latest/固定池 Stream |
| src/subscriber.rs、dispatcher.rs | 单值/块读取、独立存储、分层 stats、有界 fan-out |
| src/host/{mod,binding,scope,slot}.rs | 唯一 RT 权限、offRT 准备、scoped typed access、稳定槽 |
| tests/{registry,latest,stream,demand,lifecycle,host,allocation,concurrency}.rs | 契约测试与 RT/并发证据 |
| examples/custom_block.rs、README.md | 自定义头部/元素和线程使用示例 |

四种 shape 是 sealed marker：`LatestValue<T>`、`StreamValue<T>`、`LatestBlock<H,E>`、`StreamBlock<H,E>`，所有用户自定义 T/H/E 满足 Copy 等约束即可。`Topic<P>`、`PublisherHandle<P>`、`PreparedPublisher<P>`、`Subscriber<P>` 保留 P；名称查找只在非 RT 进行。

统一 ID 均为不可伪造的 opaque 类型：TopicId 包含目录身份/slot/注册 generation，HostId 与 BindingKey 独立；不得使用音频 graph revision 代替它们。PublicationMeta 为 `{ sequence: u64, activation_generation: u64, len: usize }`；业务头部不进通用元数据。shape 关联类型固定如下：

```rust
pub trait Shape: sealed::Sealed + Send + Sync + 'static {}
pub trait ValueShape: Shape { type Value: Copy + Send + Sync + 'static; }
pub trait BlockShape: Shape {
    type Header: Copy + Send + Sync + 'static;
    type Element: Copy + Send + Sync + 'static;
}
```

sealed 模块只向 crate 内四种 marker 开放；LatestValue/StreamValue 实现 ValueShape，LatestBlock/StreamBlock 实现 BlockShape。`Topic::id(&self) -> TopicId` 可供非 RT 目录对照，不能转换成可写权限。

### Task B1: 四种注册接口、资源预算和安全初始化

**Files:**

- Create: `crates/moiren-signal/Cargo.toml`, `src/{lib,types,error,budget,storage,registry,lifecycle}.rs`, `tests/registry.rs`。
- Modify: 根 `Cargo.toml`, `Cargo.lock`。

**Interfaces:**

- `Bus::new(BusConfig) -> Result<Bus, RegisterError>`；BusConfig 为 `{ byte_budget, max_topics, max_subscribers_per_topic, default_stream_depth, max_payload_bytes }`，默认 32 MiB/128/64/8/1 MiB。max_subscribers_per_topic 必须 1..=64，与原子编码及固定上限一致。
- `register_latest_value<T>(&mut self, &str, T) -> Result<Topic<LatestValue<T>>, RegisterError>`。
- `register_stream_value<T>(&mut self, &str, T, usize) -> Result<Topic<StreamValue<T>>, RegisterError>`，最后参数是 ingress depth。
- `register_latest_block<H,E>(&mut self, &str, BlockInit<H,E>) -> Result<Topic<LatestBlock<H,E>>, RegisterError>`；Stream 多 ingress depth 参数。
- `BlockInit<H,E> { header: H, element: E, capacity: usize }`；另有 `register_latest_block_with<H,E>(&mut self, name: &str, capacity: usize, header_factory: impl FnMut() -> H, element_factory: impl FnMut() -> E) -> Result<Topic<LatestBlock<H,E>>, RegisterError>`；stream 对应 register_stream_block_with，增加 ingress depth 参数，返回 Topic<StreamBlock<H,E>>。工厂只在注册阶段调用，生成不可变安全初始化模板；订阅阶段复制模板初始化独立槽。模板也计入预算，不从正在被 RT 修改的槽复制初始内容。
- `resolve<P: Shape>(&self, &str) -> Result<Topic<P>, LookupError>`；LookupError 为 NotFound/TypeMismatch/ModeMismatch。
- `budget_used(&self) -> usize`；RegisterError 为 InvalidName/InvalidCapacity/SizeOverflow/BudgetExceeded/TopicLimit/DuplicateName/GenerationExhausted/PayloadTooLarge。注册 generation 与目录/host ID 用 checked 增长，耗尽不回绕；失败不提交新身份。
- 名称必须非空、不含 NUL，最多 256 UTF-8 字节；不强制业务路径层级。ZST 合法，仍计入元数据和实际槽布局。header+元素+meta 的单消息最大复制布局超 max_payload_bytes 时返回 PayloadTooLarge；该错误纳入 RegisterError，在任何分配/注册提交前检查。

- [ ] 添加无需 Default 的初始化与失败原子性测试：

```rust
use moiren_signal::{BlockInit, Bus, BusConfig, LatestBlock, LookupError};
use std::num::NonZeroU32;

#[test]
fn valid_nonzero_payload_can_register_without_default() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let topic = bus.register_latest_block("custom/status", BlockInit {
        header: NonZeroU32::new(7).unwrap(),
        element: NonZeroU32::new(3).unwrap(), capacity: 4,
    }).unwrap();
    let found = bus.resolve::<LatestBlock<NonZeroU32, NonZeroU32>>("custom/status").unwrap();
    assert_eq!(found.id(), topic.id());
    assert!(matches!(bus.resolve::<LatestBlock<u32, u32>>("custom/status"),
                     Err(LookupError::TypeMismatch)));
    assert!(bus.budget_used() > 0);
}
```

- [ ] `cargo test -p moiren-signal --test registry`，预期新 crate/API 尚不存在；添加 workspace member 和上述 dependencies，更新 lock 后后续使用 --locked。
- [ ] 实现 storage 布局预算，使用实际 slot struct 的 Layout，而不是仅 `len * size_of::<E>()`。所有乘法、加法和对齐均 checked；非 RT 分配结果保存在 staged allocation，全部成功后提交目录。值与块完整初始化，不使用 zeroed 创建任意 Copy 类型。

```rust
fn checked_elements_bytes<E>(capacity: usize, slots: usize)
    -> Result<usize, RegisterError> {
    capacity.checked_mul(slots)
        .and_then(|n| n.checked_mul(std::mem::size_of::<E>()))
        .ok_or(RegisterError::SizeOverflow)
}
```

上面只计算元素区域；slot/header/queue/metadata 另用 Layout::array/extend/pad_to_align 计入。Latest 只准备其三槽与分发暂存，绝不能顺带建 Stream ring。注册失败不占名称、Topic slot、generation 或预算。

- [ ] 加容量溢出、超预算、重复名、模式不匹配、未注册 NotFound 和初始化非消息测试；核对按 factory 逐槽初始化，而不是复制未初始化占位。
- [ ] `cargo test --locked -p moiren-signal --test registry`；预期所有通过。显式暂存 workspace、lock 与新 crate 文件，提交 `feat: register typed signal topics with bounded storage`。

### Task B2: Latest 值/块、需求代次和显式提交

**Files:**

- Create: `crates/moiren-signal/src/demand.rs`, `sequence.rs`, `transport/mod.rs`, `transport/latest.rs`, `subscriber.rs`, `tests/latest.rs`, `tests/demand.rs`。
- Modify: `crates/moiren-signal/src/{lib,types,registry,storage}.rs`。

**Interfaces:**

- `DemandSnapshot { active: bool, generation: u64 }`；`scope.demand(&PublisherHandle<P>) -> Result<DemandSnapshot, PublishError>`；scope 在 B5 定义，B2 单元测试使用同一 private typed endpoint。
- 值 `scope.publish(&PublisherHandle<LatestValue<T>>, T) -> Result<PublishOutcome, PublishError>`。
- 块 `scope.try_write(&PublisherHandle<LatestBlock<H,E>>, max_len: usize) -> Result<BlockWriteGuard<'_,H,E>, WriteError>`；guard `header_mut() -> &mut H`、`elements_mut() -> &mut [E]`、`commit(self, len: usize) -> Result<PublishOutcome, PublishError>`；elements_mut 只暴露本次声明的 max_len，commit 要求 len <= max_len <= capacity。
- `PublishOutcome::{Published(PublicationMeta), NoDemand, DroppedFull { sequence: u64 }}`；PublishError 为 Closed/InvalidLength/SequenceExhausted/ForeignHost/InactiveBinding。try_write 无需求返回 `WriteUnavailable::NoDemand`，Stream 满返回 `WriteUnavailable::Full { sequence: u64 }`；WriteError 为 `Unavailable(WriteUnavailable)` 或 `Invalid(PublishError)`，不假造空 guard。
- `Bus::subscribe<P>(&mut self, &Topic<P>, SubscriptionOptions) -> Result<Subscriber<P>, SubscribeError>`；SubscriptionOptions `{ stream_depth: Option<usize> }`，Latest 指定 Stream depth 返回 InvalidOptions。SubscribeError 为 Closed/ForeignTopic/InvalidOptions/InvalidCapacity/BudgetExceeded/SubscriberLimit/DemandExhausted。
- 值订阅 `read_latest(&mut self) -> Option<ValueMessage<T>>`，ValueMessage `{ meta, value }`；块订阅 `read_latest(&mut self) -> Option<BlockReadGuard<'_,H,E>>`，guard `meta()`, `header() -> &H`, `elements() -> &[E]`。
- 读取区分首次/新版本，反复 read_latest 未更新返回 None；新 Latest 订阅在已有业务需求时复制当前完整快照作为首次消息。

- [ ] 在 private endpoint 单元测试里构造初始值 99，未发布时 read_latest 为 None；写值 1、2、3 后仅取得 3；块填 `[1,2]` 未 commit 后上一消息完整保留。非法 commit 长度不改变上一条消息或 sequence。
- [ ] 实现需求的单一原子编码：低 7 bit 为业务 subscriber count、bit 7 为 Closed、高 56 bit 为 activation generation。新增业务订阅在非 RT CAS 中，count=0 时 checked 增 generation；取消/Drop 用非 RT CAS 减 count。关闭只设置 Closed bit，保留 count 供现存 subscriber Drop，active 为 !closed && count>0。内部 dispatcher 不占 count；RT 仅 load Acquire。最多 64 个 subscriber 的校验在 CAS 增数前完成，工厂/预算失败不改变 demand。

```rust
const COUNT_MASK: u64 = 0x7f;
const CLOSED_MASK: u64 = 0x80;
const MAX_GENERATION: u64 = u64::MAX >> 8;
fn next_subscription_word(word: u64) -> Result<u64, SubscribeError> {
    if word & CLOSED_MASK != 0 { return Err(SubscribeError::Closed); }
    let count = word & COUNT_MASK;
    let generation = word >> 8;
    let generation = if count == 0 {
        generation.checked_add(1)
            .filter(|g| *g <= MAX_GENERATION)
            .ok_or(SubscribeError::DemandExhausted)?
    } else { generation };
    if count >= 64 { return Err(SubscribeError::SubscriberLimit); }
    Ok((generation << 8) | (count + 1))
}
```

取消可以发生在订阅线程，原子编码保证零需求再恢复即使发生在两个 RT 块之间也增加代次。计数 CAS 重试只发生非 RT。RT block scope 缓存 DemandSnapshot，分段发布沿用同一代次；dispatcher/subscriber 仅交付当前有效代次的数据。

- [ ] Latest 使用 upstream `input_buffer_mut()` 和显式 `publish()`。总线 guard Drop 只结束借用，不调用 publish；块存储是固定 Box<[E]>，只修改既有 header/elements/len，不替换 Box。producer、dispatcher 与每个 subscriber 的 Latest 存储独立。
- [ ] sequence 由唯一 RT writer 的 checked u64 本地计数产生，分配时 store Release 到 attempt_sequence；原子时刻在 commit 和 transport publish 之前。NoDemand、非法长度、主动 abort 不递增；满容量申请在 B3 递增。首次已发布标记与初始 payload 分开。
- [ ] 追加两次 block 之间 subscribe→Drop→subscribe 测试，断言 generation 增加、旧缓存不可读，另一个订阅持续存在时 generation 不变。`cargo test --locked -p moiren-signal --test latest --test demand`；提交 `feat: add latest signals with demand generations`。

### Task B3: Stream 固定池、独立借用与新订阅 fence

**Files:**

- Create: `crates/moiren-signal/src/transport/stream.rs`, `tests/stream.rs`。
- Modify: `crates/moiren-signal/src/{registry,storage,subscriber,sequence,types}.rs`。

**Interfaces:**

- Stream 值 `scope.publish(&PublisherHandle<StreamValue<T>>, T)`；块 try_write/commit 与 B2 一致，增加 `publish_slice(handle, header: H, elements: &[E]) -> Result<PublishOutcome, PublishError>`。
- Stream 订阅 `next(&mut self) -> Option<ValueMessage<T>>` 或 `Option<BlockReadGuard<'_,H,E>>`。
- 块 `copy_next(&mut self, output: &mut [E]) -> Result<Option<BlockMessage<H>>, ReadError>`；BlockMessage `{ meta, header, len }`，ReadError 为 DestinationTooSmall { required: usize }；过小不消费该条消息。Latest 对应 copy_latest。
- Subscriber 的 depth 指尚未消费消息上限，已借出的块额外占一个预分配 loan slot。该 slot 显式计入预算；同一个 &mut Subscriber 最多一个 guard，不提供 clone 读取端。
- `Subscriber::status(&self) -> TopicStatus`；TopicStatus `{ lifecycle: Registered|Closed, publisher: Bound|NoPublisher }`，尚未发布由 read 返回 None 表达。

- [ ] 为深度 2 的 private ingress 写测试：发布 10、20 成功，30 返回 DroppedFull 且前两条顺序不变；消费后发布 40，其 sequence 与 20 之间存在缺口，Topic drops=1。try_write 成功后放弃不产生缺口，满槽申请则产生一次 drop/sequence。
- [ ] 实现安全的所有权循环：`free slot ring → RT writer guard → ready ring → dispatcher-owned slot → free ring`。ring 使用 rtrb；移动的是拥有有效初始化存储的 slot owner，不 clone/resize/free payload。abort 把 slot 放回 writer 本地 spare 或已准备 free ring，不能触发容量失败后析构 payload。
- [ ] Subscriber 建独立 ready/free ring 及 loan slot；dispatcher 只复制有效 header/elements 并保留原 meta。不把 RT ingress slot 直接借给业务 subscriber；一个 subscriber 满时只增加自己的 drops，继续其他订阅。
- [ ] 新 Stream 订阅在存储准备成功后 load Acquire attempt_sequence 为 fence，再正式增加需求；只交付 generation 匹配且 sequence > fence 的消息。fence 的订阅边界采用 B2 的 commit 分配时刻，不采用 pump 读取时刻。并发测试用 barrier 控制 commit 序号分配与 enqueue 两个阶段，验证边界的明确顺序。
- [ ] 追加三个独立 subscriber 的不同深度测试；一个持有 guard，另两个仍能接收。小目标 slice 不消费、借用与复制内容一致、注销后的 guard 可读且不提前释放槽。`cargo test --locked -p moiren-signal --test stream`；提交 `feat: add bounded stream signals and independent read loans`。

### Task B4: 有界 pump、生命周期和分层统计

**Files:**

- Create: `crates/moiren-signal/src/dispatcher.rs`, `tests/lifecycle.rs`。
- Modify: `crates/moiren-signal/src/{registry,lifecycle,budget,subscriber,types}.rs`。

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy)]
pub struct PumpBudget {
    pub topic_checks: usize,
    pub deliveries: usize,
    pub copy_bytes: usize,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct PumpReport {
    pub topic_checks: usize,
    pub deliveries: usize,
    pub copy_bytes: usize,
    pub has_pending: bool,
    pub required_copy_bytes: usize,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct TopicStats { pub attempts: u64, pub ingress_drops: u64 }
#[derive(Debug, Clone, Copy, Default)]
pub struct SubscriberStats { pub delivered: u64, pub overflow_drops: u64 }
```

- `Bus::pump(&mut self, PumpBudget) -> PumpReport`，`has_business_demand(&self) -> bool`。
- `Bus::unregister<P>(&mut self, &Topic<P>) -> Result<(), UnregisterError>`；UnregisterError 为 Busy/Closed/ForeignTopic。
- `Topic::stats()`/`Subscriber::stats()` 返回独立复制计数；Subscriber 状态可在无后续消息时查询。计数饱和不回绕，不将 Latest 跳版本计成 Stream ingress loss。

- [ ] 构造 3 个 subscriber，预算只允许 ingress→暂存一次和一个 subscriber 复制；连续 pump 验证总 copy_bytes 不超限、fan-out 用保存游标逐轮完成；held guard 不占 RT pool。为超本轮 budget 的单消息检查 required_copy_bytes，调整到足额后能前进。
- [ ] 用完整成本扣减函数实施每一步预算，禁止把“一个 ingress”当成全部 fan-out 成本：

```rust
fn charge(report: &mut PumpReport, budget: PumpBudget, bytes: usize) -> bool {
    let Some(total) = report.copy_bytes.checked_add(bytes) else { return false; };
    if report.deliveries >= budget.deliveries || total > budget.copy_bytes {
        report.has_pending = true;
        report.required_copy_bytes = report.required_copy_bytes.max(bytes);
        return false;
    }
    report.deliveries += 1;
    report.copy_bytes = total;
    true
}
```

header+有效元素+transport meta 的实际复制布局都计入 bytes。metadata-only 的 Topic 检查也受 topic_checks 上限。按 Topic round-robin，不让一个长期活跃 Stream 吃掉全部循环；一轮 fan-out 只包含取出时存在且符合 fence/generation 的订阅，随后新订阅不补历史 Stream。

- [ ] 非 RT unregister 与新 lease/订阅建立通过同一 registry lifecycle gate 串行协调。publisher Bound 或 graph prepared/active/retired lease 非零返回 Busy，失败无变化；成功去掉名称映射并设置 Closed bit，使 active 立即为 false，但保留实际 subscriber count 直到 Drop，避免取消下溢。老 subscriber/guard 保留可安全读的独立存储；可读状态为 Closed；新同名注册使用新 generation，旧 handle 永远不解析成新 Topic。
- [ ] 所有注册、订阅与闭合后 loan 存储持续计入 budget，真实最后引用释放时才退账。仅退订不注销 Topic、不释放其 ingress。NoPublisher 保留旧 Latest/既有 Stream，重新 bind sequence 连续；若业务 demand 曾归零则新 generation 过滤优先。
- [ ] 测试预算不足订阅不增加 demand、Busy 不关闭、注销旧 guard 安全、同名重注册隔离、无 publisher 不丢旧数据、最后一条 Full 的 stats 可读取。`cargo test --locked -p moiren-signal --test lifecycle --test latest --test stream`；提交 `feat: bound signal fanout and explicit topic lifecycle`。

### Task B5: 唯一 stable host、typed handle 与调用权限

**Files:**

- Create: `crates/moiren-signal/src/host/mod.rs`, `binding.rs`, `scope.rs`, `slot.rs`, `tests/host.rs`。
- Modify: `crates/moiren-signal/src/{lib,registry,lifecycle,types}.rs`。

**Interfaces:**

- `Bus::create_host(&mut self, max_bindings: usize) -> Result<(HostControl, RtHost), HostError>`；max_bindings 固定，应用使用 128。HostError 为 InvalidCapacity/BudgetExceeded/PublisherConflict/Closed/ForeignTopic/HostCapacity。
- `HostControl::new_binding(&self) -> Result<BindingKey, HostError>` 分配不回绕的绑定身份，耗尽返回 BindingExhausted；`prepare<P>(&self, &Topic<P>, BindingKey) -> Result<PreparedPublisher<P>, HostError>`；prepared 的 `handle(&self) -> PublisherHandle<P>` 仅访问标识，不授予写权限。HostControl 可非 RT clone，RtHost 不可 Clone、不实现 Sync，可移动到一个 RT owner。
- `HostControl::prepare_update(&self, &[ErasedBinding]) -> Result<PreparedHostUpdate, HostError>`；ErasedBinding 由 PreparedPublisher 的非 RT `erase()` 产生，包含 Topic/host/type/schema/owner 身份及 plan lease。`PreparedHostUpdate::op_bindings(&self, BindingKey) -> Result<OpSignalBindings, HostError>` 在非 RT 生成每操作表，未知 key 返回 UnknownBinding；多 Topic 属于同一 op 时允许使用同一个 BindingKey。重复 Topic 的两 owner 仍拒绝。
- `RtHost::activation<'a>(&'a mut self, &'a mut PreparedHostUpdate) -> Result<ActivationTransaction<'a>, ActivateError>`；transaction 只借用 host 与 update，不能 Clone/Send，放弃时不移动或释放资源；`commit(self) -> RetiredHostUpdate` 为验证完成后的无失败提交。独占借用防止验证与提交间插入另一次 activation。ActivateError 为 ForeignHost/Closed/StaleBasis/HostCapacity/PublisherConflict。
- `RtHost::activate(&mut self, PreparedHostUpdate) -> Result<RetiredHostUpdate, ActivateFailure>` 是上述 transaction 的便利入口；ActivateFailure 持有错误与完整未消费 update，不在 RT drop；RetiredHostUpdate 也必须进入 offRT retire。当前有效绑定表只允许每 Topic 一 owner。
- `RtHost::scope<'a>(&'a mut self, &'a OpSignalBindings) -> SignalScope<'a>`；OpSignalBindings 是 prepare_update 产生的已校验每操作表，只能用于当前 active update。SignalScope 不可 Clone/Send/Sync，使用 PhantomData<Rc<()>>；方法见 B2/B3，借用 guard 绑定 scope 的短借用。
- `RtHost::begin_block(&mut self)` 缓存 active slots 的 demand 快照，不读取墙钟；`PublisherHandle<P>` 可 Copy，但无独立 publish 方法。
- `SignalScope::publish` 按 shape 的 `ValueShape` trait 分派，`try_write`/publish_slice 按 `BlockShape`；trait 由 crate sealed，仅上述四 shape 实现，用户只提供 T/H/E。

- [ ] 添加权限测试：第二个 host 绑定同 Topic 返回 PublisherConflict；candidate 的 handle 在旧 scope 返回 InactiveBinding；activate 后旧 handle 失效，新 handle 发布仍沿用原 sequence、generation、实际槽地址。两份 candidate 都存在时取消其中一份不修改当前 active binding。
- [ ] 实施稳定 endpoint：目录/host 控制根共同保活预分配 publisher cell，唯一 RtHost 的 `&mut` scope 才能取出对应 typed writer。所有 Any/TypeId 解析和 erased→typed pointer 绑定在非 RT 完成；RT 只比较 host/binding 身份并访问已经验证的 slot。unsafe 限于 slot.rs，逐项注释唯一 mutable alias、地址稳定和 plan/host lease 的存活依据。
- [ ] 已有 Topic update 只换权限表，不移动/重建 producer；新 Topic 的 cell 在非 RT 创建，包在 PreparedHostUpdate 中，激活只占用预分配的空 host slot。slot 回收随 RetiredHostUpdate 走 offRT；不能在 RT Vec::push、resize、克隆 producer 或 drop 替换出的 Box/Arc。
- [ ] PublisherConflict 保护从 host 准备到解绑/退场；prepared/active/retired graph lease 都令注销 Busy。无活动 binding 的 host slot可以保留 endpoint，但它不能授予旧计划权限；注销 Closed 后该 slot 不可再次 prepare，实际存储在安全退场后回收并持续计费。
- [ ] 引用释放和资源回收控制根必须保活直到所有 RT host 与 retired update 已移回非 RT。测试主动退出 Bus 控制用户、保留后台 cleanup 根，RT 继续完成 publish/activate，不出现最后 Arc 析构；随后 cleanup drop 记录线程 ID。host 完全停止并在非 RT 释放后允许新 host 重新绑定注册 Topic；这属于无并发 writer 的重新绑定，沿用原 writer 存储/sequence，不提供活动 host 之间的迁移。遗留 HostControl 无法在 host 已释放后重新授予权限。
- [ ] 用公开接口建立基本 host 测试，确保上述 API 能组合使用：

```rust
use moiren_signal::{Bus, BusConfig, PumpBudget, SubscriptionOptions};

#[test]
fn typed_handle_requires_active_scope() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let topic = bus.register_latest_value("status/value", 0.0_f32).unwrap();
    let mut subscriber = bus.subscribe(&topic, SubscriptionOptions::default()).unwrap();
    let (control, mut host) = bus.create_host(4).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    let update = control.prepare_update(&[publisher.erase()]).unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap()); // initial activation is off RT here
    host.begin_block();
    {
        let mut scope = host.scope(&op);
        scope.publish(&publisher.handle(), 0.75).unwrap();
    }
    bus.pump(PumpBudget { topic_checks: 4, deliveries: 4, copy_bytes: 4096 });
    assert_eq!(subscriber.read_latest().unwrap().value, 0.75);
}
```
- [ ] 加 compile-fail doctest：SignalScope 不能 clone/发送至 thread、BlockWriteGuard 不能逃出作用域；分别执行 `cargo test --locked -p moiren-signal --test host` 和 `cargo test --locked -p moiren-signal --doc`。提交 `feat: authorize typed signal publication with stable RT hosts`。

### Task B6: RT 分配、并发安全和独立示例

**Files:**

- Create: `crates/moiren-signal/tests/allocation.rs`, `tests/concurrency.rs`, `examples/custom_block.rs`, `README.md`。
- Modify: `crates/moiren-signal/src/host/slot.rs` 中审计后确有必要的安全说明；其余修复仅针对本 crate。

**Interfaces:** 只消费 B1–B5 已定义公开 API；不添加音频/Slint 类型或不受约束的 byte payload 逃生口。

- [ ] 用 TLS allocator 计数包围仅 RT 调用段：值/块 publish、Latest 覆盖、Stream Full、guard abort、generation 改变、activate 成功/拒绝、UI subscriber Drop 后的 RT。assert alloc=0、realloc=0、dealloc=0；准备、pump、subscription 和最终回收在计数段之外。new Topic activation 也必须纳入。
- [ ] 压力测试一个生产线程 + pump 线程 + 两个独立订阅线程，自定义 `{ sequence_echo, inverse_echo }` 成对变化，逐条断言来自同一发布；一个读 guard 通过 barrier 保留时另一个 subscriber 继续。线程 join 后核对 ingress/各 subscriber 分层 loss 与原始 sequence，不用固定时间等待保证通过。
- [ ] 为 safe 初始化/guard/pointer 访问跑 Miri：`cargo +nightly miri test -p moiren-signal --test host --test latest --test stream`。首次环境缺 nightly/Miri 时记录环境问题，先完成正常测试；不得把未执行说成已通过。并发 unsafe 内部若需要模型检查，以该 crate 的实际原子协议添加最小模型，不为整个软件引入新的 runtime。
- [ ] 示例在独立 producer 线程发布 `StreamBlock<FrameHeader, f32>`，另注册一个 LatestValue<f32>；有限运行并把 host/绑定所有权返回给非 RT join 调用方。使用以下完整示例，不让 publisher/表在 producer 退出时析构：

```rust
use moiren_signal::{BlockInit, Bus, BusConfig, PumpBudget, SubscriptionOptions};

#[derive(Debug, Clone, Copy)]
struct FrameHeader { frame_start: u64, sample_rate: u32 }

fn main() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let state = bus.register_latest_value("example/latest", 0.0_f32).unwrap();
    let blocks = bus.register_stream_block("example/frames", BlockInit {
        header: FrameHeader { frame_start: 0, sample_rate: 48_000 },
        element: 0.0_f32, capacity: 2,
    }, 8).unwrap();
    let mut state_reader = bus.subscribe(&state, SubscriptionOptions::default()).unwrap();
    let mut block_reader = bus.subscribe(&blocks, SubscriptionOptions::default()).unwrap();
    let (control, mut host) = bus.create_host(4).unwrap();
    let key = control.new_binding().unwrap();
    let state_publisher = control.prepare(&state, key).unwrap();
    let block_publisher = control.prepare(&blocks, key).unwrap();
    let update = control.prepare_update(&[
        state_publisher.erase(), block_publisher.erase(),
    ]).unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    let state_handle = state_publisher.handle();
    let block_handle = block_publisher.handle();
    let producer = std::thread::spawn(move || {
        for block in 0..4_u64 {
            host.begin_block();
            let mut scope = host.scope(&op);
            scope.publish(&state_handle, block as f32).unwrap();
            let mut write = scope.try_write(&block_handle, 2).unwrap();
            *write.header_mut() = FrameHeader {
                frame_start: block * 2, sample_rate: 48_000,
            };
            write.elements_mut().copy_from_slice(&[block as f32, block as f32 + 0.5]);
            write.commit(2).unwrap();
        }
        (host, op)
    });
    let (host, op) = producer.join().unwrap();
    for _ in 0..8 {
        let report = bus.pump(PumpBudget {
            topic_checks: 128, deliveries: 128, copy_bytes: 262_144,
        });
        if !report.has_pending { break; }
    }
    let latest = state_reader.read_latest().unwrap();
    println!("latest={} sequence={}", latest.value, latest.meta.sequence);
    let mut output = [0.0_f32; 2];
    let mut received = 0;
    while let Some(message) = block_reader.copy_next(&mut output).unwrap() {
        received += 1;
        println!("frame={} sr={} sequence={} values={:?}",
            message.header.frame_start, message.header.sample_rate,
            message.meta.sequence, &output[..message.len]);
    }
    assert_eq!(received, 4);
    println!("topic={:?} subscriber={:?}", blocks.stats(), block_reader.stats());
    drop((host, op, state_publisher, block_publisher));
}
```

示例按 PumpReport.has_pending 在固定最大 8 轮内推进，断言 4 条皆收到。producer 已 join，消费队列总量有界，不能把该有限示例改成追逐活动 producer 的无界 drain。
- [ ] `cargo test --locked -p moiren-signal`、`cargo clippy --locked -p moiren-signal --all-targets -- -D warnings`、`cargo run --locked -p moiren-signal --example custom_block`；预期测试通过、示例正常退出。提交 `test: verify signal RT and concurrency contracts`。
