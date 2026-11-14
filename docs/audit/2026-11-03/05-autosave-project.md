# 子代理 05:自动保存/项目恢复(PR #94 graph-autosave)只读审计

审计基线:分支 `fix/spectrum-standard-curve` = main(3321062)+ 2 个频谱提交,工作区干净;审计对象为 PR #94(merge `9e3a7bc`,`de44a5a` + `9caf3cc`,18 文件 +1123/−41)。分区文件:`ui/window/{autosave.rs,project.rs,desktop.rs,test_harness.rs,tests.rs}`、`project/{startup.rs,storage.rs,tests.rs}`、`service/{startup.rs,managed.rs,managed/driver.rs,mod.rs(改动部分)}`、`ui/window/settings/persistence.rs`、locales 两文件。只读审计,未改任何代码;本文档为唯一产出。每条发现标注"已亲证"(file:line + 代码摘录 + 失败机理)或"疑案/待实测"。

---

## 发现表

| ID | 级别 | 标题 | 位置 | 状态 |
|---|---|---|---|---|
| AS-1 | P1 | 4 个 gpui-test shell 漏置 `autosave_path=None`:测试退出即把测试图写进开发者真实 autosave 文件 | devices/tests.rs:89、exports/tests.rs:321/420/612 | 已亲证(代码 + 框架层证实) |
| AS-2 | P1 | 打开工程不经编辑即退出:`remember_project_file` 只重置指纹不同步 autosave 文件 → 下次启动恢复陈旧 autosave,且 `project_path` 指向另一工程,Save 把旧工程内容覆写进该工程文件 | project.rs:549-560、window.rs:443-448、startup.rs:26-37 | 已亲证(代码推演,无反例) |
| AS-3 | P1 | 启动 resume 剥离离线 IO 的 journaled RemoveNode 让启动时捕获的指纹立刻变脏:剥离图 1 秒后被写回 autosave,离线设备节点跨会话永久丢失(undo 提示只在当前会话有效) | window.rs:449-451、driver.rs:313-326、autosave.rs:97-126 | 已亲证(推演;带时序竞争注记) |
| AS-4 | P2 | 退出 flush 受 gpui `SHUTDOWN_TIMEOUT=200ms` 硬约束,且偏好保存先 await、autosave 后 await 串行共享同一预算;超时只记 log 后照常退出,最后一次编辑可能静默不入盘 | window.rs:487-501、gpui-pre app.rs:79/1156-1206 | 已亲证(框架侧证实) |
| AS-5 | P2 | 损坏/版本不识的 autosave 静默丢弃(仅 eprintln):用户不可见,且随后第一次 autosave 写直接覆盖,恢复内容彻底消失;`restore_last_project` 默认 false,默认路径下连 last-project 兜底都没有 | storage.rs:110-123、model.rs:438 | 已亲证 |
| AS-6 | P2 | autosave 失败提示只进 `project_message`(仅工程菜单弹层内可见),持续失败时每 ~1s 重试并反复覆写该消息,正常运行不可见 | autosave.rs:169-188、project.rs:56-58 | 已亲证(渲染位点;重试频率为推演) |
| AS-7 | P2 | 测试覆盖缺口:防抖定时器真实路径(deadline 延展/超时唤醒)、写链"旧不覆新"顺序、损坏文件被覆盖、剥离×autosave 交互均零覆盖 | project/tests.rs、managed/tests.rs | 已亲证(覆盖面清点) |
| AS-8 | P3 | Page→WorkspaceDocument 映射出现第三份拷贝:新 `StudioShell::workspace_document` 与既有 `desktop::workspace` 并存 | project.rs:330-337、desktop.rs:497-504 | 已亲证 |
| AS-9 | P3 | `autosave_flush` 清 deadline 不清 timer 句柄:flush 后 shell 若继续存活,`poll_autosave` 因 `timer.is_some()` 永不再武装定时器(当前生产不可达) | autosave.rs:102-104/201 | 疑案(不可达路径) |
| AS-10 | P3 | 进程崩溃在 tmp 创建与 rename 之间会遗留 `autosave.moiren.json.PID.N.tmp` 孤儿,无清理机制 | storage.rs:201-227 | 已亲证 |
| AS-11 | P3 | 双实例并发运行共享同一 autosave 文件:原子写保证不损坏,但语义上互相覆盖恢复点(preferences 同病,非本 PR 引入) | storage.rs:97-108 | 已亲证(设计备注) |
| AS-12 | P3 | `restore_last_project` 偏好无任何 UI 入口,autosave 分支又无条件绕过它:一旦产生 autosave 该偏好永久失效,沦为近死配置 | model.rs:423/438、startup.rs:26-37 | 已亲证 |
| AS-13 | P3 | i18n-keys.tsv 机械扫描出 48 个代码零引用键(均先于本 PR 存在;新键不在其中) | locales/i18n-keys.tsv | 待实测(交 i18n 分区复核) |

计数:P0×0、P1×3、P2×4、P3×6。

---

## 详述

### AS-1 (P1) 4 个 gpui-test shell 漏置 `autosave_path=None`,测试退出把测试图写进开发者真实 autosave 文件

- 位置
  - `crates/moiren-app/src/ui/window/devices/tests.rs:88-94`
    ```rust
    let (view, cx) =
        cx.add_window_view(|window, cx| super::super::StudioShell::new(state, window, cx, None));
    ...
        view.update(cx, |shell, cx| {
            shell.preferences_path = None;          // 只挡了 preferences
            shell.presentation.page = ...Page::Devices;
    ```
  - `crates/moiren-app/src/ui/window/devices/exports/tests.rs:321-328`、`:419-428`、`:611-620`(同样模式:`preferences_path = None` + `presentation.page = Page::Devices`,其中 312 起的用例还点击创建/编辑真实 exports)。
  - `crates/moiren-app/src/ui/window.rs:366`(`StudioShell::new` 里 `autosave_path: crate::project::autosave_path().ok()` → Windows 上即 `%APPDATA%\Moiren\autosave.moiren.json`)。
- 机理(已亲证,三层证据):
  1. PR #94 给 `test_harness.rs:132`、`window/tests.rs:1479`、`capture.rs:161`、`window.rs:711` 四处 shell 补了 `autosave_path = None`,但漏掉了 devices 两个测试文件里的 4 处直接 `StudioShell::new`(全仓 `add_window_view` + `StudioShell` 共 6 处,4 处漏置;`grep add_window_view` 可复核)。
  2. 这 4 处测试都把 `presentation.page` 改成 `Devices`(exports/tests.rs:312 起的用例还通过点击把测试 exports 写进图,`candidate_revision` 递增),对照启动时捕获的指纹(`workspace` 字段是 Graph)必然判脏。
  3. 框架层证实退出必经 flush:`#[gpui_kit::test]` 宏在测试体结束后调用 `cx.quit()`(gpui-pre-macros-0.3.9/src/test.rs:172),`TestAppContext::quit` 执行 `app.shutdown()`(gpui-pre-0.3.9/src/app/test_context.rs:238-240),`shutdown` 逐个运行全部 `on_app_quit` 观察者(gpui-pre-0.3.9/src/app.rs:1156-1206)。`StudioShell::new` 注册的 quit 处理器(window.rs:487-501)调用 `autosave_flush` → 判脏 → 把测试图(测试 catalog 的假设备 + 测试 exports)原子写入开发者真实 autosave 文件。
- 影响:在任何开发机上执行 AGENTS.md 记载的 `cargo test --locked -p moiren-app --features gpui-test` 即摧毁该机真实恢复文档(用户上次会话的工作图被测试图覆盖,下次启动恢复出测试内容)。CI(GitHub Windows runner)的 APPDATA 是一次性的,无感。
- 修复方向:4 处补 `shell.autosave_path = None`;更稳的做法是把"测试 shell 必须无持久化"收进单一构造路径(`test_shell_body`),或让 `StudioShell::new` 在 `cfg(feature = "gpui-test")` 下默认 `autosave_path = None`,由生产路径显式打开。

### AS-2 (P1) 打开工程不经编辑即退出:指纹重置但 autosave 文件不同步 → 恢复陈旧状态 + Save 覆写无关工程文件

- 位置
  - `crates/moiren-app/src/ui/window/project.rs:549-560`
    ```rust
    fn remember_project_file(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.project_path.clone()
            && self.preferences.last_project_path.as_ref() != Some(&path) { ... }
        self.autosave_stamp = autosave::AutosaveStamp::capture(self, cx);
    }
    ```
    注释宣称 "the autosave fingerprint restarts because the state is already on disk"——对工程文件成立,对 autosave 文件不成立:没有任何 `save_autosave_at` 调用。
  - `crates/moiren-app/src/project/startup.rs:26-37`(恢复时 autosave 无条件优先于 last_project)。
  - `crates/moiren-app/src/ui/window.rs:443-448`(恢复后 `shell.project_path = shell.preferences.last_project_path.clone()`,不区分恢复来源)。
  - `crates/moiren-app/src/ui/window/project.rs:388`(`save_project` 的 `known = self.project_path`,Save 直写该路径)。
- 触发序列(已亲证,逐步推演):
  1. 会话 1:用户编辑工作图 Q(autosave 周期落盘为 Q 状态),退出。
  2. 会话 2:启动恢复 Q(autosave 优先)。用户通过 Open project 打开工程 P → `finish_project_load` → `remember_project_file`:`last_project_path=P`(偏好已存盘),指纹重置为 P 状态。此后 `autosave_matches == true`,已武装的防抖 settle 步在 `autosave.rs:121` 的 `if !shell.autosave_matches(cx)` 处被拦下,P 状态从不写入 autosave 文件。用户不编辑,退出:flush 因 matches 直接返回 `None`(autosave.rs:195-197)。磁盘 autosave 仍是 Q。
  3. 会话 3:启动恢复陈旧的 Q 文档(名称也是 Q 的名字),而 `project_path` 被置为 `last_project_path=P`(window.rs:447)。工程菜单里名称显示 Q、路径显示 P。用户按一次 Save(认为在保存当前工作)→ `save_project` 把 Q 内容写进 `P.moiren.json`——P 文件内容被无关工程覆盖,用户数据损毁。
- 变体(同根因):打开 P 的瞬间若上一次防抖写在途,该写仍会落盘旧文档(autosave.rs:148-167 不感知后续的项目切换),文件与指纹同样脱节。
- 影响:常见操作序列("打开看看、不动、退出")之后,下一次启动恢复错误的工程,且存在一次按键即覆写用户工程文件的数据丢失路径;名称/路径错配直接可见。
- 修复方向:`remember_project_file` 在重置指纹的同时把当前文档写入 autosave(或在指纹里区分"已在工程文件上"与"已在 autosave 上"两种已存状态);恢复路径按恢复来源设置 `project_path`(autosave 恢复时若 last_project 与恢复内容无来源关系,置 None 让 Save 走另存流程)。

### AS-3 (P1) 启动 resume 的剥离编辑让 autosave 立即变脏:离线设备节点被永久写入"已删除"的恢复文档

- 位置
  - `crates/moiren-app/src/ui/window.rs:449-451`
    ```rust
    // The restored startup state is already on disk (autosave or project);
    // the first edit, not the launch, owns the first autosave write.
    shell.autosave_stamp = autosave::AutosaveStamp::capture(&shell, cx);
    ```
  - `crates/moiren-app/src/service/managed/driver.rs:313-326`(`resume_restored` 对每个离线 IO 执行 `c.graph.edit(GraphEdit::RemoveNode(io.node))`——journaled 编辑,`candidate_revision +1`,见 graph.rs:682-685)。
  - `crates/moiren-app/src/ui/window/autosave.rs:97-126`(`poll_autosave` 由 desktop.rs:468 的 33ms 轮询驱动,判脏即武装防抖,1 秒后 `autosave_write_now`)。
- 触发序列(已亲证):
  1. 启动:恢复文档落图,`StudioShell::new` 在首帧前捕获指纹(revision = R)。
  2. 首个设备目录到达(WASAPI/ASIO 枚举,典型几十至几百 ms,晚于指纹捕获的机率高):`resume_restored` 剥离离线 IO 节点 → revision = R+1 → 快照发布 → UI 轮询吸收。
  3. `poll_autosave` 判脏 → 1 秒 settle → 把**已剥离**的图写回 autosave 文件。恢复文档从此不含该节点。
  4. 故障提示("Reconnect the device, then undo (Ctrl+Z)…",driver.rs:355-357)只在当前会话有效:undo journal 不持久化;用户若先退出再插回设备,下次启动恢复的图已没有该节点,插回也无路可走。显式保存过的工程文件仍含该节点,但恢复优先级被 autosave 压住(startup.rs:26-37)。
- 时序竞争注记:若目录枚举先于指纹捕获完成(快机器),指纹捕获的就是剥离后状态,本次启动不触发写;因此该问题是概率性的,慢枚举(ASIO 驱动、多设备)下几乎必现。
- 与设计宣称的出入:window.rs:449 的注释明确"第一次**编辑**才拥有第一次 autosave 写";剥离正是服务端发起的一次 journaled 编辑,违反了这一意图,把一个"临时降级"固化成了永久删除。
- 影响:用户拔线状态下的每次启动都会把对应 IO 节点从工作文档中永久清除(绑定、标签、位置全丢),且跨会话无恢复途径(undo 不持久,工程文件被 autosave 遮蔽)。
- 修复方向:resume 剥离不该弄脏用户文档——二选一:(a) 剥离用非 journaled 途径并让指纹把"剥离前恢复态"视为已存(但会失去 undo 提示的能力);或 (b) 保留 journaled 剥离,但指纹捕获延迟到 resume settle 之后,或指纹忽略"启动后 N 秒内仅由剥离引起的 revision 变化",并把剥离写进文档时在文档层留撤销记录。至少应在故障提示中说明"该删除会在退出后保留"。

### AS-4 (P2) 退出 flush 只有 200ms 总预算,且偏好保存串行占先用时

- 位置:`crates/moiren-app/src/ui/window.rs:487-501`(quit 处理器先 `pending.await`(偏好写)再 `autosave.await`,串行);gpui-pre-0.3.9/src/app.rs:79 `pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(200);`、app.rs:1199-1204 `block_with_timeout(SHUTDOWN_TIMEOUT, futures)` 超时仅 `log::error!("timed out waiting on app_will_quit")` 后继续退出。
- 机理(已亲证):quit flush 需先 await 链上在途写(每个写含 `sync_all` 落盘,storage.rs:219-220),再执行自己的临时文件 + fsync + rename;偏好写在同一 200ms 预算内先行消费。慢盘(HDD、BitLocker、杀软挂钩 FlushFileBuffers)下单次 fsync 即可吃满预算,链上还有多个写排队时更甚。超时后进程照常退出,最后一次编辑静默丢失(写是原子的,不会损坏,只是没落盘)。
- 影响:退出丢最后 <1s 编辑的概率窗口被 200ms 硬上限放大;丢失无任何用户可见信号(与防抖窗口退出 flush 的设计目标相悖)。
- 修复方向:autosave flush 与偏好写并行 await(`futures::join!`);或把 autosave 链的写合并为"最新文档一次写"(flush 已经构建最新文档,在途旧写可放弃——但当前链条无法取消在途写,可给 completion 加"已作废"标志让旧写跳过 rename);至少记录"退出保存未完成"的可观测信号。

### AS-5 (P2) 损坏/版本不识的 autosave 静默丢弃,且随后被覆盖,恢复内容彻底消失

- 位置:`crates/moiren-app/src/project/storage.rs:110-123`
  ```rust
  Err(error) => {
      eprintln!("Moiren autosave: {error}");
      None
  }
  ```
- 机理(已亲证):JSON 损坏、超过 8 MiB、版本不识(`UnsupportedVersion`,migrate 拒绝)统统只打 stderr,用户无感;回退分支正确(有测试 `startup_falls_back_to_the_last_project_without_a_usable_autosave`),但 `restore_last_project` 默认 false(model.rs:438),默认路径下直接空图启动。更重的是:启动后第一次 autosave 写会把损坏文件原子覆盖——用户上一次会话的工作状态被彻底销毁,连"翻一下旧文件"的机会都没有。
- 影响:异常断电/半写(理论上有原子写保护,但磁盘错误、外部工具改写、手误编辑都可能产生坏文件)后,用户丢失全部未保存工作且无提示、无残留。
- 修复方向:损坏时把原文件改名留档(`autosave.moiren.json.corrupt-<时间戳>`)并经 project_message/错误条告知;或至少在覆盖前检测"本次启动曾丢弃过 autosave"时提示一次。

### AS-6 (P2) autosave 失败的可见性:只进工程菜单弹层,持续失败每秒重试并反复覆写消息

- 位置:`crates/moiren-app/src/ui/window/autosave.rs:180-184`(失败 → `project_message`);渲染位点仅 `project.rs:56-58`(工程菜单弹层内)与 `settings.rs:504`(Settings 诊断区——缓存面板,消息变更不触发重建,即 2026-10-31 HC-4 既有问题的既有面,此处是新触发面而非复发)。
- 机理(已亲证渲染位点;重试频率为推演):写失败 → completion 回调置错误消息、不更新指纹 → 仍判脏 → 轮询重新武装 → 1 秒后重试。磁盘满/目录只读/杀软锁文件时,每秒一次失败,消息每秒覆写,而用户不打开工程菜单就永远看不到。
- 影响:autosave 长期失效完全不可见,与"恢复保障"的定位不符。
- 修复方向:连续 N 次失败升级为错误条(ErrorStrip)常驻;消息附带"最近一次成功保存时间"。

### AS-7 (P2) 测试覆盖缺口

已亲证(对 `project/tests.rs`、`ui/window/project.rs::tests`、`service/managed/tests.rs`、`service/startup.rs::tests` 全量清点):

| 缺口 | 现有最接近的测试 |
|---|---|
| 防抖定时器真实路径(deadline 被后续编辑延展、超时唤醒、`Ok(None)` 退出) | 无——`graph_edits_settle_into_the_autosave_file` 直接清 timer 后手动调 `autosave_write_now`(project.rs:273-278),定时器任务体(autosave.rs:105-125)零覆盖 |
| 写链"旧文档不覆盖新文档"顺序(两次快速写、在途写时再触发) | 无 |
| 打开工程后指纹重置与 autosave 文件一致性(AS-2 场景) | 无 |
| 剥离 × autosave 交互(AS-3 场景) | 无(service 测试只测剥离/undo/通知,UI 测试只测写入) |
| 损坏 autosave 被覆盖丢失(AS-5 后半) | 无(只测"损坏时回退") |
| 退出 flush 超时/失败路径 | 无(`quitting_flushes_...` 只测 happy path) |
| 恢复来源与 `project_path` 的对应(AS-2 后半) | 无 |

### AS-8 (P3) Page→WorkspaceDocument 映射第三份拷贝

`project.rs:330-337` 新增 `StudioShell::workspace_document`(注释自述 "shared by the document builder and the autosave fingerprint"),而既有共享函数 `desktop::workspace`(desktop.rs:497-504,被 window.rs:659、desktop.rs:527 使用)与其逐分支相同。新代码未复用旧单源,反而新增一份平行实现。修复:删除其一,`workspace_document` 委托 `desktop::workspace`(或反之)。已亲证。

### AS-9 (P3/疑案) flush 后 `autosave_timer` 句柄残留,后续防抖永久失灵(当前不可达)

`autosave_flush`(autosave.rs:201)只清 `autosave_deadline`,不清 `autosave_timer`;定时器任务醒来读到 `Ok(None)` 后返回(autosave.rs:110),但 shell 里的 `autosave_timer` 仍是 `Some(已结束的任务)`。此后 `poll_autosave` 在 autosave.rs:102-104 因 `timer.is_some()` 直接返回,永不再武装新定时器——之后所有编辑都不再自动保存。当前唯一调用方是 `on_app_quit`,之后进程即退出,故不可达;但这是为未来"取消退出/多阶段关机"埋的雷。修复:flush 里同置 `autosave_timer = None`(或定时器任务退出前自清)。疑案/待实测(不可达路径)。

### AS-10 (P3) 崩溃遗留 tmp 孤儿文件

`write_json`(storage.rs:201-227)失败路径删除临时文件,但进程在 create 与 rename 之间崩溃时 `autosave.moiren.json.<pid>.<n>.tmp` 永久留在 `%APPDATA%/Moiren/`,无启动清扫。反复崩溃(或 AS-4 的超时中断)会累积。已亲证(代码路径);影响仅为目录卫生。修复:启动时清理本进程名匹配的陈旧 tmp(pid 已死即可删,或按 mtime)。

### AS-11 (P3) 双实例共享同一 autosave 恢复点

`autosave_path()` 固定于配置目录(storage.rs:97-108),两个实例并行运行时原子写保证不损坏,但恢复点互相覆盖,后退出者胜;恢复时也可能拿到另一实例的工作图。preferences.json 同病(既有),非本 PR 引入,随多实例支持一并决策。已亲证(设计备注)。

### AS-12 (P3) `restore_last_project` 沦为近死配置

字段(model.rs:423)默认 false(model.rs:438),无任何 UI 写入点(全仓引用仅 startup.rs:29 分支、model.rs 定义、project.rs:550 注释);autosave 分支无条件优先(startup.rs:26-29),意味着只要产生过一次 autosave,该偏好对启动恢复永久无效,唯一残存作用是"无 autosave 文件时是否恢复 last project"。要么给它 UI 并把 autosave 恢复纳入同一开关,要么删字段。已亲证。

### AS-13 (P3/待实测) i18n-keys.tsv 机械扫描 48 个零引用键

对 tsv 全部 551 键做 `grep -rF` 全 src 扫描,48 键无精确匹配(如 "Apply graph"、"L ONLY"、"%{prefix}: actual timing" 等)。抽查证实部分是 service 层运行时 `format!` 拼串(mixer.rs:49 拼 "CH {} ONLY",不经 t!,即既往 SD-11 的 service i18n 断层面),部分疑似真死键("Apply graph" 全仓无引用)。**均先于 PR #94 存在**;PR 新增键 "Could not auto-save: %{error}" 有引用(autosave.rs:182)且双语齐全(zh-CN.yml:296-298)。本条仅记录,交 i18n 分区复核。

---

## 时序核对清单(按猎捕方向逐项裁决)

| # | 猎捕项 | 裁决 |
|---|---|---|
| 1a | 防抖 1s 窗口内退出丢最后一次保存 | **不丢**:flush 直接以当前状态构建最新文档写入(autosave.rs:193-210),有测试(`quitting_flushes_a_dirty_document_into_the_autosave_file`,连纯视角变(camera pan)都覆盖);但受 AS-4 的 200ms 预算约束 |
| 1b | 串行写链在途时再触发保存 | **无竞态**:每次写在 UI 线程按序 spawn 并 `await_completion(previous)`(autosave.rs:148-152),旧文档不可能压过新文档;完成回调有 `AutosaveStamp::capture == stamp` 守卫防陈旧指纹回写(autosave.rs:171-179);quit flush 与 settle 步并发时两侧内容相同(同帧状态),原子 rename 不损坏 |
| 1c | 退出 flush 超时与失败路径 | **AS-4**:200ms 硬预算 + 偏好写先行串行;失败仅 eprintln(autosave.rs:206-208) |
| 1d | 写文件原子性/半写自愈 | **原子**:tmp(pid+计数器,create_new 防踩)+ sync_all + rename(storage.rs:194-227);损坏自愈回退 last_project 有测试;但损坏时静默+随后覆盖 = **AS-5**;崩溃 tmp 孤儿 = AS-10 |
| 2 | 指纹覆盖 | **完整**(详见"已核对无问题"第 5 条),无漏字段;真正的缺陷在指纹重置时机(AS-2)与系统自身编辑(AS-3) |
| 3a | 恢复优先级分支 | 正确:autosave > last_project,missing/unusable 均回退(startup.rs:26-37 + 测试) |
| 3b | 恢复失败呈现 | **AS-5**:静默;工程文件分支错误仍会 `?` 顶穿启动(既往行为,非回归) |
| 3c | with_automatic_resume 直接 apply 的失败路径 | 取消语义完备(LoadProject/Start/ApplyGraph/Stop 均 `cancel_startup_automation`,driver.rs:30-98;有测试);apply 失败走既有 preparation_rejected 故障面;唯 resume_notice 若 owner 永不出现则永不发布(挂起到 Stop 取消,无害) |
| 3d | 剥离在撤销栈上用户可见/Ctrl+Z | **是设计**:hint 明说 undo 恢复,service 测试验证 undo 找回节点;但跨会话固化成删除 = **AS-3** |
| 3e | 唯一 IO 离线留 WaitingForResources | 正确:`offline.retain(online>0)`(startup.rs),apply 仍发出由 host 等设备,有测试 `resume_never_strips_the_last_source_...` |
| 4 | faults 时序 | 等待是**无限期**的(直到 owner settled 或显式请求取消,driver.rs:366-377);窗口(session)永未激活则通知不发布(挂起);去重:Option::take 一次 + record_fault 按 subject+cause 去重(graph.rs:651-660);confirmed 的 faults.clear 只在 candidate==revision 时发生,不会吃掉已发布通知(graph.rs:922-935,测试断言周期确认不清除) |
| 5 | i18n 新键 | 新键齐全且有引用;死键 48 个均为既往(AS-13) |
| 6 | test_harness autosave_path=None | **AS-1**:harness/主 tests/capture 均置,devices 两文件 4 处漏置 |
| 7a | 路径不存在/无权限 | autosave_path() 失败(无 APPDATA)时静默无自动保存(window.rs:366 `.ok()`);写失败面见 AS-6 |
| 7b | 时钟回拨 | **免疫**:防抖全用 `Instant`(QPC 单调)+ `saturating_duration_since`(autosave.rs:101/108) |
| 7c | 多窗口写冲突 | 单 shell 持有 autosave;tray flyout 是独立 TrayMixer 非 StudioShell(desktop.rs:479-493),无多写者;双实例 = AS-11 |
| 7d | 手动另存后 autosave 收敛 | 编辑后保存会收敛(内容一致);打开-不编辑-退出不收敛 = **AS-2** |

## 已核对无问题清单

1. **写链串行化**(autosave.rs:53-72/132-189):completion 经 `Arc<Mutex>` + Waker 交接,写按 spawn 序串行,旧不覆新;`unwrap_or_else(|p| p.into_inner())` 防毒锁。
2. **原子写**(storage.rs:183-228):临时文件 `create_new`(pid + 原子计数器命名,跨进程不踩)+ `sync_all` + 同目录 `rename`;失败清理临时文件,旧目标文件保持完整。
3. **防抖退避语义**(autosave.rs:97-126):轮询每次延展 deadline,持续编辑不写盘;定时器任务读 deadline 增量睡眠,deadline 被清(写权转移)即自退。
4. **指纹字段逐一比对**(autosave.rs:22-51/77-93):revision 覆盖图/节点状态/exports/撤销(SetExports、undo/redo、replace、configure_session 全部 bump candidate_revision,graph.rs:682/753/801/841);processing、session(candidate_config)、camera、workspace、mixer_order、node_positions、name 与 `project_document`(project.rs:339-377)输出字段一一对应,无漏字段。
5. **恢复优先级与损坏回退**(startup.rs:21-42):autosave 缺失/不可用回退 last_project;三个分支均有 project/tests.rs 测试。
6. **resume 不重绑不静音**(driver.rs:308-362 + managed/tests.rs `restored_project_resumes_with_its_own_bindings_once_the_catalog_arrives`):保留恢复的绑定与 0.7 增益,candidate_revision 不变(设备齐全时零编辑);显式请求优先取消(有测试);catalog 失败保留 automatic_resume 等待重试(driver.rs:284-287 仅在 Ok 分支清)。
7. **剥离判定**(service/startup.rs `strippable_offline_io`):export 绑定/Tone/Process/owned VAD 端点不算离线;未绑定节点不算在线兄弟;仅同类存在在线节点时剥离;三个测试覆盖(剥离、唯一不剥离、双离线不剥离、tone 兄弟、owned 端点)。
8. **剥离通知时序**(driver.rs:363-377):等 owner 且 settled(pending none + active==candidate)后 record_fault,避开 `confirmed` 的 faults.clear 窗口;一次性 + 去重;测试断言周期确认不清除。
9. **单写者**:全仓仅 StudioShell 持有 autosave 写路径;tray mixer 是独立组件;`capture.rs` 捕获运行已置 None(capture.rs:159-162)。
10. **布局校验不会卡死 autosave**:UI zoom 被夹在 [0.25,2.0](camera.rs:54-113) ⊂ 校验区间 [f32::EPSILON,4.0];camera/坐标全走有限值检查;mixer.order 唯一性由 push 前 contains/retain/reorder 维护(model.rs:813/875-876/1006-1015)——`project_document` 不会因布局非法持续失败。
11. **i18n 新键**:"Could not auto-save: %{error}" tsv(i18n-keys.tsv:99)与 zh-CN.yml(:296-298,en+zh-CN)齐全,代码引用于 autosave.rs:182。
12. **偏好静默写**(settings/persistence.rs:12-31/77-97):`save_preferences_quiet` 分支只压制用户反馈,链式序列化与 revision 守卫未动,无回退。

## 既往审计对账

- 2026-10-27 / 2026-10-31 两目录无任何 autosave/项目恢复条目(该功能区 2026-11-02 才随 PR #94 落地),本分区无"复发/回退"可报。
- 交集仅一处:AS-6 的消息可见性踩在 2026-10-31 HC-4(project_message 不进 SettingsKey、只在弹层渲染)的既有面上——是同一既有限制的新触发面,按嘱不以复发论处,已交叉引用。
- SD-11(service 层 i18n 断层)在 AS-13 抽查中再次可见(mixer.rs:49 拼串),维持开放状态,不重复报。

## 结论摘要

PR #94 的写链工程(原子写、串行链、防抖、指纹)本体扎实,恢复优先级与 resume 降级路径测试到位;真正的风险集中在**指纹与磁盘文件的同步时机**上:AS-2(打开-不编辑-退出的陈旧恢复 + Save 覆写工程)与 AS-3(启动剥离被固化进恢复文档)都是"指纹说已存、文件说没有/反之"的时序裂缝,AS-1 则是本 PR 直接引入的测试卫生回归(4 处漏置,已可摧毁开发机真实恢复文档)。建议三条 P1 优先修复并按 AS-7 补测试。
