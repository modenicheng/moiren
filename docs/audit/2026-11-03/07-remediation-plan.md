# 2026-11-03 审计修复工单总方案

分期依据:严重级 + 用户数据风险 + 同文件聚集。每批一个 PR,均可独立合并;B/C 两批有共享文件(window.rs / graph.rs),注意 rebase 顺序。验证门对所有批次统一:

```
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked -p moiren-app --features gpui-test
cargo test --locked --workspace   # 批次收尾时
```

## 批次 A — autosave 数据安全(最优先,建议立即)

用户数据损毁路径,全部源自 PR #94。

| 项 | 内容 | 备注 |
|---|---|---|
| A-1 | AS-2:`remember_project_file` 后使 autosave 失去恢复权威(删除 autosave 文件,或立即以当前文档覆写) | 语义决策见 DEC-1 |
| A-2 | AS-3:启动 resume 剥离不得触发 autosave 写回(剥离完成后重捕获指纹;或改用非 journal 手段剥离) | 保住"首编辑拥有首写"的注释意图 |
| A-3 | AS-1:4 处测试 shell 补 `autosave_path=None`;根治 = 消灭直建 `StudioShell` 的旁路,统一走 `test_shell_body` | devices/tests.rs:88、exports/tests.rs:321/420/612 |
| A-4 | AS-7:补测试——防抖窗口内退出、损坏 autosave 回退、剥离-指纹交互、开工程不编辑退出再启动 | A-1/A-2 的回归网 |
| A-5 | AS-4/AS-5/AS-6(P2):退出 flush 与偏好写的 200ms 约束至少文档化;损坏文件的呈现与回退按 DEC-2;失败重试的可见面 | 可拆到 A' 小 PR |

实机验收:开工程 P→不编辑→退出→重启(应恢复 P 而非旧 autosave)→Save 不覆写;拔唯一麦→重启→设备回来后节点仍在;`%APPDATA%\Moiren\autosave.moiren.json` 跑完 gpui-test 套件后不变。

## 批次 B — 手势与 undo 交互(与 C 同级,文件聚集在 window.rs/graph.rs)

| 项 | 内容 | 备注 |
|---|---|---|
| B-1 | GC-1:`undo_redo_graph` 清 `connection_drag`(照 Delete 路径) | 一行级修复 |
| B-2 | IN-2+GC-2:#93 旋钮/EQ 详情拖拽纳入 `slider_slots` 单一游走(遵守 window.rs:926-932 已写明的不变量)+ BindSource 编辑加 undo 合并键 | 粒度决策见 DEC-3 |
| B-3 | GC-3:fault tooltip/橡皮带纳入 HUD 挖空 + 指针离开画布清零;顺带 GC-5 背景按下清双手势 | |
| B-4 | 实机手测:拎线中 Ctrl+Z、旋钮拖拽中 Ctrl+Z/Ctrl+Y、tooltip 悬停工具条 | B-1/B-2 各补一条手势回归测试 |

## 批次 C — 缓存键/族完整性与映射单源(历史复发类的根治)

| 项 | 内容 | 备注 |
|---|---|---|
| C-1 | IN-3:Graph 页 ControlsKey 补 `catalog` 与 `audio_settings.backend`,附重建计数器回归(呼应上轮 HY-11 的"面板可测"要求) | Mixer 页键已是正确参照 |
| C-2 | IN-1:压缩器 InputState 改为 update 路径预建(实体先于 notify),或给压缩器面板键加 editing 旗标 | 根治捕获时序,不是补丁 |
| C-3 | IN-7:详情窗/binding 菜单补交互测试(#93 三大新交互零覆盖) | |
| C-4 | NC-1:tint/icon 映射单源化——NodeTemplate/NodeKind 一处定义、库/画布/幽灵三消费方共享,附漂移回归测试(#93 已在 icon 上修过一次同型,证明逐处修不可持续) | NC-2(幽灵卡 input_ports 空)随单源化一并消解或单独修 |

## 批次 D — 热层与频谱外围(数学核心无需动)

| 项 | 内容 | 备注 |
|---|---|---|
| D-1 | SP-1:分析卡曲线 scratch 真落地(shell 持有复用 buffer / Rc 共享,兑现 c55a3ae 的宣称) | 热层 paint 闭包内零分配 |
| D-2 | SP-2:`project_spectrum` 按通道切片零拷,兑现"稳态零分配"注释 | |
| D-3 | SP-5 引注改 IEC 61260 base-two;SP-3/4/6 低优先随缘 | |

## 批次 E — 台账与镜像(全独立,可并行铺开)

| 项 | 内容 | 备注 |
|---|---|---|
| E-1 | ST-1/ST-2:i18n 双向护栏测试(照 action_assets.rs:230 模式)+ 清 43 死键 + 补 4 条 zh-CN 翻译 | 先落护栏再清,防止再犯 |
| E-2 | ST-4:绑定清单双写抽公共构造器(binding_menu.rs:59-187 与 node_inspector 同批 GraphEdit) | 消 ~130 行平行 |
| E-3 | ST-5:peaking/knee 纯函数搬 engine pub API,curves.rs 引擎真源 | 消 ~35 行镜像 |

## 批次 F — 清理与决策项(低风险收尾)

- NC-4:`NodeLayout.badge` 死槽删除 + designs/18 修订(4 条漂移:badge、网格制式、spline 描述、B2)。
- GC-6/GC-7/其余 P3(NC-3..8、GC-4、SP-3..6、AS-8..13、IN-4..6 等):见各分报告,按顺手原则随邻近批次带走。
- X-1(engine/app 双 level 目录):老开放项,spectrum 又添一实例——维持独立决策,不并入本方案。

## 决策项(阻塞对应条目)

| # | 问题 | 影响 |
|---|---|---|
| DEC-1 | autosave 与手动工程的权威语义:打开工程即作废 autosave?手动 Save 后删 autosave 还是保留为更新副本? | A-1 的设计 |
| DEC-2 | 损坏 autosave 的用户呈现:静默丢弃+滚动备份,还是显式提示? | A-5 |
| DEC-3 | 画布旋钮拖拽的 undo 粒度:per-gesture 单条 journal,还是时间窗合并? | B-2 |

## 建议顺序

A(数据安全)→ B+C(可两线并行,B 先因共享文件)→ E(独立随时)→ D → F。E 最早可立即开工,与 A 无冲突。
