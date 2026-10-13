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

`cargo check --locked --target x86_64-pc-windows-msvc -p moiren-windows-audio --all-targets` 通过。`cargo check --locked --workspace --all-targets` 通过后，当前工作区 UI 草稿新增的 Slint 文件导致后续完整 check 失败；该错误来自 `crates/moiren-app/ui/components/dev-ready.slint` 的 `placeholder-text` 与重复 `border-color`，不属于本切片，且文件未纳入提交。

工作区完整测试在该 UI 草稿生成前已通过：应用单元测试 37 passed/4 ignored，Monitor 集成测试 6 passed；Windows audio 的 Clock Bridge、latency budget、Render、Shared Stream 与 workspace 测试全部通过。两阶段新增 handoff 的 Windows 编译检查通过，便携 handoff 测试覆盖 stop 优先、单次取值、关闭后回收 queued ownership 和拒绝 late handoff。

Clock Bridge 新增回归确认 prepared capacity、prime、servo reference 与 high/low water 不被混淆，并拒绝 metadata 与实际 config 不一致的输入。

## 当前边界

尚未在本轮重新执行硬件采集；上一轮 `docs/superpowers/plans/2026-10-13-render-target-validation.md` 的 FreeDSP 证据仍只覆盖 Render target 480、输入旧配置。新的 physical Monitor/probe 路径可用 Custom `input_target=1024`，但需要重新跑硬件和 20–30 分钟漂移矩阵后才能宣称联合输入库存稳定。
