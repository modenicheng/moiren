# 2026-11-09 架构审计 — moiren-signal + moiren-plugins

基线: main @ 27aa57d7（工作区干净，实测 `git rev-parse --short HEAD`）

审计范围与体量（实测 `wc -l`）：

| crate | src | 集成测试 | 内联单测位置 |
|---|---|---|---|
| moiren-signal | 6,971 行 / 14 个模块文件 | 2,648 行 / 8 个文件（registry.rs 1,473 行 27 测试） | 14 个 src 文件含 `#[cfg(test)]`（dispatcher/tests.rs 704 行、transport/*/tests.rs 503+704 行为独立测试文件） |
| moiren-plugins | 4,455 行 / 9 个模块文件 | 1,915 行（plugin_ffi.rs 351 行 6 测试 + fixtures/gain_plugin.rs 1,564 行） | 仅 vst3/abi.rs 含 2 个 ABI 钉扎测试 |

本轮聚焦架构与代码组织、可维护性，非 bug 狩猎。所有结论先读代码后给出，每条附 file:line 证据。

## 执行摘要（≤10 条要点）

1. **两 crate 的分层纪律都是全仓库最好水平**。moiren-signal 把 unsafe 集中在唯一生产边界 `host/slot.rs`（模块自述 "the only production unsafe boundary"，slot.rs:1）；moiren-plugins 每个 abi.rs 只做 repr(C) 声明+unsafe 派发包装，mod.rs 做生命周期编排，三层分离清晰。
2. **Latest/Stream 双语义在类型系统上表达完整**：4 个 shape 类型（LatestValue/StreamValue/LatestBlock/StreamBlock）经 sealed trait 派发（types.rs:249-331），模式差异收敛于 `Mode::of` + `const STREAM`（types.rs:281-289），且 12 个 compile_fail 文档钉住不变性/变异性攻击面。
3. **RT 分配契约是机械化强制的**，不靠约定：`test_alloc.rs` 计数分配器（`#[global_allocator]`）+ `tests/allocation.rs` 用 `include!` 复用（test_alloc.rs:1-4 注明原因），`rt_publication_and_activation_paths_do_not_allocate`（tests/allocation.rs:26）覆盖全部发布路径。
4. **P2 文档漂移 S1**：signal lib.rs:6-8 声称 "Stream…carries no production traffic yet…Latest carries all current publication paths"，但 2026-11-02/03 起 app 生产路径已在 Stream 上发布（meters.rs:14/28 经 `PreparedPublisher<StreamBlock<..>>` + `scope.try_write`）。crate 门面文档对新使用者是错的。
5. **P2 文档漂移 P1**：clap/abi.rs:84-86 描述宿主回调 "land in an atomic counter the control side drains opportunistically"，实现却是纯 no-op（clap/mod.rs:86-88），描述了不存在的机制。
6. **plugins 三后端有真实且够用的公共层**：module.rs（同文件去重 + init/deinit 恰好一次）、spec.rs（描述符/参数）、instance.rs（RT 契约 trait）、lib.rs::instantiate 分派。不对称均来自格式本身（VST3 需要 COM 宿主对象），不是抽象缺口。
7. **vst3/abi.rs 的 `interface!` 宏**（abi.rs:193-241）把 COM vtable 派发写一次；vtable 槽位数测试（abi.rs:733-747）与 UID 字节序测试（abi.rs:714-724）有历史事故级注释（IPluginFactory2 槽位错位曾 segfault 真插件）。ABI 钉扎是教科书级的。
8. **依赖方向全部验证干净**：两 crate 均零 moiren 依赖（仅 thiserror/rtrb/triple_buffer=pinned/crossbeam-utils=pinned 与 thiserror/serde/libloading）；engine/app 只经 pub API 消费 signal（全部 use 站点核对），plugins 只被 engine 的 processor/plugin.rs 适配。唯一泄漏是 spec.rs:6 文档提及 engine 的 `ParamDomain`（无类型依赖，属适配器注释，可留）。
9. **「双信号总线」旧审计项确认为已解决的分层中继形态**：engine 总线（host/signals.rs:245，RT 观测）与 app 总线（service/signals.rs:77，UI 流）各司其职，app 经 engine 暴露的类型化订阅（ManagedLevelSubscription 等包装 `Subscriber<T>`）读取后在服务线程再发布（meters.rs:19-21 注释明确）。
10. **规模健康**：两 crate 模块粒度合适，无过碎模块；唯一偏粗文件 vst3/mod.rs（1,464 行）内部已有注释横幅划分出约 600 行宿主 COM 对象层，是唯一值得考虑的纯移动拆分（见 P4）。

## 发现清单

| ID | 级别 | 标题 | 位置 | 证据摘要 | 建议+权衡 |
|---|---|---|---|---|---|
| S1 | P2 | lib.rs 门面文档声称 Stream 无生产流量，已过时 | crates/moiren-signal/src/lib.rs:5-8 | 文档（2026-10-27 提交 60b0d2a6 写入）称 Stream "awaiting its first routed consumer…Latest carries all current publication paths"；但 app/src/service/signals.rs:141-151 注册 5 个 `register_stream_block` 生产主题，service/managed/meters.rs:14/28 以 `PreparedPublisher<StreamBlock<..>>` 在服务线程 `scope.try_write` 生产发布（2026-11-02 d5654f55、11-03 e8aa9ffe 引入） | 改写 lib.rs 首段为现状（Latest=RT 音频线程路径，Stream=服务线程/UI 中继路径）。权衡：纯文档改动零风险；不改则每个新使用者从门面就被误导 |
| P1 | P2 | CLAP 宿主回调文档描述了不存在的原子计数器机制 | crates/moiren-plugins/src/clap/abi.rs:84-86 vs clap/mod.rs:86-88 | abi.rs ClapHost 注释："restart/callback requests land in an atomic counter the control side drains opportunistically"；mod.rs 实现是 `host_request_restart(_host: *const ClapHost) {}` 等空函数，无任何计数器；mod.rs:3-6 与 78-79 的正确说法是 "accepted-and-ignored sinks / documented follow-up" | 把 abi.rs 注释改成与 mod.rs 一致的"合法 no-op，restart 处理是既定后续项"。权衡：仅注释；保留错误描述会让维护者以为 restart 有排水通道 |
| P2 | P3 | scan.rs 注释称 VST2 只在 VST2 命名目录内扫描，代码无此检查 | crates/moiren-plugins/src/scan.rs:154-168 | `format_of` 对任意裸 `.dll`/`.so` 返回 `PluginFormat::Vst2`（163-166），而 `scan_directory` 对非 vst3/vst bundle 目录一律递归（105-107）——扫描 `COMMONPROGRAMFILES\CLAP` 时其子目录里的杂散 dll 也会被当 VST2 探测 | 二选一：(a) 实现目录语境检查（父目录名含 VST2/VSTPlugins 才探测）；(b) 改注释为"任何裸共享库都按 VST2 探测，靠 magic 校验兜底"（vst2/mod.rs:129-134 确有 'VstP' magic 防护）。权衡：(a) 更精确但增加路径规则复杂度；(b) 诚实且现有 magic 检查已防误判，倾向 (b) |
| P3 | P3 | VST2 描述符以空 path 占位构造，靠调用方各自补写 | crates/moiren-plugins/src/vst2/mod.rs:221, 315, 533 | `open_effect` 构造 `PluginDescriptor { path: Path::new("").. }`；`instantiate`（315）与 `scan_descriptor`（533）事后 `descriptor.path = ...` 补写。漏补则得到静默错误描述符 | 让 `open_effect` 接收 path 参数一次填对，删除两处补写。权衡：一行签名改动，纯收敛 |
| P4 | P3 | vst3/mod.rs 1,464 行混合宿主 COM 对象层与实例编排 | crates/moiren-plugins/src/vst3/mod.rs:113-712 vs 714-1464 | 文件内已有两道注释横幅：`Host COM objects`（113 行起，约 600 行：HostApp、ComponentHandler、ParamChanges/Queue、BStream 双接口）与 `Instance`（714 行起）。宿主对象层是自含内聚单元，是 VST3 特有 | 可选：提取 `vst3/host_objects.rs`（纯移动）。权衡：符合"不因行数拆内聚文件"红线之例外——这里有真实内聚边界且 mod.rs 会降到 ~850 行与 clap 对称；不动也可接受，横幅已可导航 |
| S3 | P3 | 写守卫命名/入口对偶不自觉 | crates/moiren-signal/src/host/scope.rs:74-90 + lib.rs:37-38 | `try_write` 返回 shape 具体类型的守卫（经 sealed `BlockDispatch::Guard`），`try_write_uniform` 返回模式擦除的 `SignalBlockWriteGuard` 枚举；同时导出 `BlockWriteGuard`（latest 具体）与 `SignalBlockWriteGuard`，名称不体现 "shape-typed vs mode-erased" 差异 | 低成本改进：在两个方法与两个类型的文档互相指向并点明选择标准（编译期已知 Latest/Stream 用前者，泛型代码用后者）。权衡：重命名会波及 engine 调用点，文档交叉引用已够 |
| S4 | P3 | 订阅者上限 64 以字面量散布 4 个文件 | registry.rs:32-47, 118 / storage.rs:119,152 / demand.rs:4-6 (COUNT_MASK 0x7f) / dispatcher.rs 多处 `[Option<S>; 64]` | 64 槽位数组、u64 位图、demand 7 位计数域三处必须同步的编码，仅 registry.rs:118 的 `1..=64` 校验暗示关联 | 提一个 `pub(crate) const MAX_SUBSCRIBERS: usize = 64` 并在 demand.rs/storage.rs 引用。权衡：机械替换，风险≈0；收益是单一权威点 |
| S5 | P3 | budget.rs 对 triple_buffer 9.0.0 私有布局的显式建模是长期耦合点 | crates/moiren-signal/src/budget.rs:139-152 + Cargo.toml (`triple_buffer = "=9.0.0"`, `crossbeam-utils = "=0.8.23"`) | `latest_shared` 手工建模上游私有 `SharedState`（3×CachePadded<UnsafeCell<S>> + CachePadded<AtomicU8> + Arc 双计数）以保字节账目精确；已配分配器探针回归（tests/registry.rs:1186, 1319）且注释声明"依赖/工具链布局变更必须重跑" | 维持现状（钉版本+探针是正确管理），仅登记为已知脆弱点：升级 triple_buffer 时该函数与两处探针是必改清单。不建议抽象掉——精确账目正是该 crate 的价值主张 |
| P7 | P3 | has_editor/is_synth 在扫描与实例化两处语义不一致，仅部分文档化 | spec.rs:47-50 vs clap/mod.rs:258-259, 647-660 / vst3/mod.rs:1178-1179, 1453-1457 | 扫描侧 CLAP/VST3 的 has_editor 恒 false（clap:659, vst3:1456），CLAP 实例化才探测 `clap.gui`（258）；VST3 实例化也恒 false（1179）却具备探测条件。spec.rs 描述符注释只说明了声道数可为零的情况 | 在 `PluginDescriptor` 字段文档注明 "扫描值可能保守为 false；以实例描述符为准"（声道已有此约定，editor 同样适用）。权衡：文档一行；补 VST3 实例侧探测属功能增强，不属本轮 |
| P9 | P3 | 测试夹具在测试运行时 shell 出 rustc 编译 | crates/moiren-plugins/tests/plugin_ffi.rs:33-39 | `Command::new("rustc")` 现场把 fixtures/gain_plugin.rs 编成 cdylib 并复制三格式别名+bundle 布局（40-52） | 可接受（OnceLock 每测试二进制编译一次；CI 与本地同 toolchain）。仅登记：若未来 CI 缓存 rustc 不可用会整体跳不过。备选 build.rs 预编译会增加构建耦合，不建议改 |
| S8 | P3 | transport 模块内置 #[cfg(test)] 测试缝（endpoint/deliver_latest/deliver_next） | transport/latest.rs:14-28, 155-178 / transport/stream.rs:21-34, 273-287 | 测试缝允许 `test_writer` 绕过宿主直接驱动端点（断言 host_owner==0 防护，latest.rs:19-23），生产路径无此入口（slot.rs:133-138 `test_writer` 亦 cfg(test)） | 现状可接受：缝有断言防护且注释明确（"No production writer bypass exists"）。无需行动 |
| — | 观察 | engine 概念在 plugins 的唯一泄漏是文档级 | crates/moiren-plugins/src/spec.rs:6 | "map one-to-one onto the engine's `ParamDomain::Float`"——无类型/词汇依赖，是写给适配器作者的映射说明 | 可保留；若追求 crate 完全自述，可改为"映射到宿主侧的浮点自动化域" |

未发现 P1 级（需要立即处理的架构缺陷）事项。

## signal 模块地图

各模块一句话职责（附主入口）：

| 模块 | 职责 |
|---|---|
| `types.rs` (467) | 4 个 shape 类型族 + sealed 派发（`Sealed`/`ValueDispatch`/`BlockDispatch`）+ `value_shape!`/`block_shape!` 宏把模式差异编译期化；BusConfig/PublicationMeta/统计类型 |
| `registry.rs` (676) | `Bus`：目录、register/resolve/unregister、订阅编排（staged 预算→槽位选择→回收的顺序不变性注释极佳，49-66）、pump 轮转 |
| `storage.rs` (431) | 具体类型化存储：模板、ValueSlot/BlockSlot、Latest/StreamStorage、池（rtrb 双环）、sender 槽位回收原语（detach/retire） |
| `subscriber.rs` (391) | TopicState、SubscriptionLease、Latest/StreamReceiver、公开读 API（read_latest/next/copy_*） |
| `host/mod.rs` (802) | HostControl（准备侧）+ RtHost（唯一可移动 RT 属主）+ Table 激活事务 |
| `host/binding.rs` (434) | 发布者身份与所有权链：BindingKey/PreparedPublisher/PublisherHandle/ErasedBinding/OpSignalBindings |
| `host/scope.rs` (167) | `SignalScope`：当前操作表的独占发布许可；`SignalBlockWriteGuard` 模式擦除守卫 |
| `host/slot.rs` (214) | 稳定生产者单元（Cell/CellRoot/Pointer/Access）——**唯一生产 unsafe 边界**，4 处 unsafe 均有完整 SAFETY 论证 |
| `dispatcher.rs` (274+704 测试) | 非 RT pump：计费扇出（charge()）、退休槽回收、Stream 逐步腾空 |
| `transport/latest.rs` (566) | Latest 端点写语义 + `BlockWriteGuard`（丢弃即中止，不上ublish-on-drop） |
| `transport/stream.rs` (290+503 测试) | Stream 端点：池获取/归还/入队，`StreamBlockWriteGuard`；满队记 drop 不在 RT 析构 |
| `budget.rs` (269) | 字节账本（Charge RAII 退款）+ 全部布局模型（含上游 triple_buffer/rtrb 布局建模） |
| `demand.rs` (196) | 单原子字编码 订阅数(7b)+closed(1b)+generation；zero→positive 跳代际 |
| `lifecycle.rs` (219) | StateHandle/TopicLease/gate/close 语义、DirectoryId |
| `sequence.rs` (80) | 每写者序列分配 + 订阅栅栏（subscription_fence） |
| `error.rs` (141) | 9 个按操作族划分的错误枚举 + flatten_write_error（Unavailable→Outcome） |
| `test_alloc.rs` (102) | 计数分配器（单测二进制内生效；集成二进制 include! 复用） |

**数据流向**（发布，全部非 RT 准备→RT 执行→非 RT 分发）：

```
Bus::register_*(registry) ──创建──▶ storage::{Latest,Stream}Storage + host::slot::CellRoot(生产者)
HostControl::new_binding → prepare(bind_cell 取类型化指针) → PreparedPublisher
  → prepare_update(表) → op_bindings(操作子表)          [准备侧,非 RT]
RtHost::activate(校验 basis) ──换表──▶ RetiredHostUpdate(须送回非 RT 释放)
音频块: RtHost::begin_block(刷新 demand 缓存) → SignalScope{publish|try_write|publish_slice}
  → transport::{latest,stream} 端点写入生产者单元        [RT,零分配,机械验证]
Bus::pump(dispatcher) ──计费拷贝──▶ 各订阅者三缓冲/环     [非 RT]
Subscriber::read_latest / next / copy_*                   [消费侧,非 RT]
```

**可学习性评估**：读侧（subscribe/read）5 分钟可上手；写侧需要理解 9 个类型的准备链（见 S2），crate 内只有 compile_fail 反例文档、缺一个正向端到端示例。host/mod.rs:114-127 与 scope.rs:107-142 的守卫文档质量很高，但分散。这是 P3 级可改进点（加一段 lib.rs 文档示例或指向 docs/designs 的使用章节），非结构问题——链上每一步的类型名（Prepared*/Op*/Rt*/Scope）本身就在叙述权限模型。

## plugins 后端对称性对照（clap/vst2/vst3）

| 关注点 | clap (mod 663 + abi 473) | vst2 (mod 537 + abi 124) | vst3 (mod 1464 + abi 748) |
|---|---|---|---|
| 模块加载 | module.rs acquire + entry.init/deinit 恰好一次（mod.rs:34-61） | module.rs acquire，无模块级 init（mod.rs:79） | module.rs acquire + InitDll/ExitDll（mod.rs:46-60） |
| 扫描 | 工厂遍历；声道 2/2 默认（mod.rs:653-658 注明需实例化） | open+close 全探测，报真实 pin 数（mod.rs:526-537） | factory2 元数据；2/2 默认（mod.rs:1451-1455） |
| 宿主对象 | 单个 Box<ClapHost>，host_data 自指（mod.rs:90-106） | audio_master 回调经 reserved1 找 HostContext（mod.rs:25-46） | 5 个 COM 对象 + 静态 vtable + owner 管理生命周期（mod.rs:113-712） |
| RT 参数应用 | 预分配 ClapInputEvents 队列（mod.rs:109-135, 505-509） | RT setParameter 直调，经典模式（mod.rs:400-414 有注释） | 字段直写 ParamChanges 容器，无 FFI 往返（mod.rs:1243-1257） |
| 状态保存 | clap.state 双向流（mod.rs:533-564） | chunk get/set + suspend 窗口（mod.rs:438-492） | IBStream 双段 blob + setComponentState（mod.rs:1290-1361） |
| 失败 unwind | 每步 destroy（mod.rs:229-279 注释逐条） | close_effect 各失败路径（mod.rs:173-301） | unwind 闭包逆序释放（mod.rs:869-889） |
| Drop 顺序 | stop→deactivate→destroy（mod.rs:606-619） | stop→mains-off→close→释放 context（mod.rs:495-522） | setProcessing(0)→setActive(0)→disconnect→terminate→release（mod.rs:1364-1396） |
| unsafe 注释 | 64 处 unsafe 均有 SAFETY（211 处 SAFETY 总数中） | 34 处 | 158 处 |

结论：不对称是**格式必要性**（VST3 的 COM 宿主对象、VST2 的 dispatcher 大开关）而非抽象缺口；三者在加载/扫描/实例/拆卸上的步骤对称性由 spec.rs 的统一描述符和 instance.rs 的统一契约保证。每处偏离主流宿主行为的决策（VST2 setChunk 忽略返回值 mod.rs:472-476、VST3 JUCE seek 兼容 mod.rs:592-594、ISizeableStream 服务 mod.rs:651-653）均有现场注释交代动机——这类"为什么"注释密度是该 crate 最突出的维护性资产。

## FFI 边界分层评估

每后端三层结构一致成立：

1. **raw 声明层**（`*/abi.rs`）：repr(C) 结构/常量/函数指针类型，来源与许可说明在文件头（vst3/abi.rs:1-11、clap/abi.rs:1-7、vst2/abi.rs:1-8，lib.rs:16-25 汇总许可立场）。
2. **派发包装层**（同文件）：vst3 用 `interface!` 宏统一 COM 派发并强制 FUnknown 三槽前缀（abi.rs:193-241）；clap 手写薄包装每槽一个 unsafe（abi.rs:294-473）；vst2 因单 dispatcher 函数指针无需包装层，调用点就地注释。每方法带 `# Safety` 契约。
3. **编排层**（`*/mod.rs`）：生命周期状态机 + 每调用点 SAFETY 论证（引用具体前提：插件状态、所有权存活）。

一处一责检查：COM 双接口偏移恢复（`BStreamObject::sizeable` 的 `offset_of!` 回退，vst3/mod.rs:519-530、654-663）只在 host 对象层出现一次；UID 字节序只在 `inline_uid` 一处定义（abi.rs:36-55）并有展开对照测试（714-724）。ABI 细节无第二实现点。测试侧 gain_plugin.rs 以**独立转录的第二实现**交叉验证宿主镜像（fixture 头注释 1-7 明言此目的），比任何单侧断言都强。

非显然 unsafe 注释覆盖：抽样的全部 SAFETY 块均说明前提而非复述操作；两处历史事故教训写进了测试注释（vst3/abi.rs:707-731 的 factory2 槽位与 UID 字节序）。

## 测试组织现状

**moiren-signal**：
- `tests/registry.rs`（1,473 行 / 27 测试）主题清晰成块：预算原子性与精确边界（922, 639）、分配器布局探针对账账目模型（1186, 1319——防 S5 耦合漂移的关键测试）、槽位退休竞态（731, 771）、注册验证/恐慌不提交（983）、ZST 边界（1088, 1450）。文件虽大但主题统一（"目录与目录资源的完整性"），暂不拆；若再增长可按 预算/竞态/布局探针 三分。
- 其余集成测试按面切分：host.rs（激活/退役线程归属，11 测试）、concurrency.rs、allocation.rs（RT 零分配）、stream/latest/demand/lifecycle 小文件。`tests/common/mod.rs` 与 `src/dispatcher/tests.rs` 各持一份 `pump_until_settled`，原因已注释（common/mod.rs:5-8：集成测试无法导入 cfg(test) 项）——已知语言限制的规范处理。
- 单测贴近行为内联（storage/sequence/demand/lifecycle/registry 各带内联模块），dispatcher 与 transport 有独立 tests.rs 文件——粒度选择合理。

**moiren-plugins**：
- `plugin_ffi.rs`（6 测试）覆盖：三格式扫描发现、每格式 生命周期+process+state 全链路、错误 id 拒绝（278）、factory2 元数据（293）、模块 init 去重（326）。
- `fixtures/gain_plugin.rs`（1,564 行）模拟一个同时导出三格式入口的增益插件：真实参数表、TEST_LATENCY=3、状态流/chunk、VST3 双接口流与连接点。它不是玩具——是对宿主 ABI 镜像的独立对照实现。
- **是否需要第二个夹具**：当前唯一。缺口在"行为不规矩的插件"（请求宿主扩展、声道不等宽、无参数、queryInterface 怪异返回）——这些由 abi 钉扎测试与 vst3 宿主对象的真实插件回归（c81791c8 "harden against real plugins"）部分覆盖。建议：在 gain_plugin 内加开关常量（如不等宽声道、空参数表变体）复用同一 cdylib，而非新增独立夹具文件；维护成本接近零，收益是 instantiate 拒绝路径（NoAudioPorts 等）可被端到端触发。
- 内联 `#[cfg(test)]` 分布：signal 14 文件（合理，贴近不变性）、plugins 仅 vst3/abi.rs 2 个 ABI 钉扎测试（正确位置——被钉的就是 ABI 本身）。

## 维护性建议 Top 5（含权衡）

1. **修正两处 P2 文档漂移（S1、P1）**——signal lib.rs 门面与 clap/abi.rs 宿主注释。二者都是新维护者最先读到的位置，且都描述了与现实相反的行为。成本：两段纯文档；不改的代价：对 Stream 生产地位与 restart 处理状态的错误心智模型会持续复制。
2. **VST2 path 占位符收敛（P3）**——`open_effect` 收 path 一次填对。收益：消除"调用方忘记补写"这一静默错误类别；成本：一行签名。
3. **transport/model 一致性小整顿（S3+S4）**——写守卫双入口加交叉文档；64 上限提为命名常量。都是零行为变更的防漂移加固；不建议重命名守卫（波及 engine 调用点，收益不抵）。
4. **可选：提取 vst3/host_objects.rs（P4）**——纯移动约 600 行，使三后端 mod.rs 体量对称（663/537/~850）。仅当下次有人实质修改宿主对象时顺手做；专程做一次移动提交的收益不足以证明其扰动（git blame 断链）。
5. **夹具变体开关而非第二夹具**——在 gain_plugin.rs 内以编译期常量/环境开关生成"难缠插件"行为（不等宽、零参数），补 instantiate 拒绝路径与退化参数表的端到端覆盖。权衡：夹具已 1,564 行，再加需保持三格式对称的手工同步；变体控制在少数几个分支内可控，独立新夹具则会复制 1,500 行 ABI 转录——明确反对后者。

明确**不建议**做的：拆 registry.rs / types.rs（内聚于"目录完整性"/"shape 语义"）、为 vst2 造 dispatch 包装层（无 vtable 可包）、抽象三后端的 instantiate 状态机（格式差异正是领域知识，强行模板化会把 per-format 生命周期顺序藏进回调）。
