# 合并与整改总方案（2026-10-31 UI 线审计）

原则：行为正确性优先于结构美化；每批独立成 PR、可独立回滚；修复必须带回归测试（hazard 6 不变量：**断言 paint 输出变化 + 重建计数平**，绝不只断言计数）；涉及帧循环/热层的批次需 `MOIREN_CAPTURE_METERS=1` 实机抓帧证据。验证门按 AGENTS.md：`cargo test --locked -p moiren-app --features gpui-test` + `cargo clippy --locked --workspace --all-targets -- -D warnings` + fmt。

## 批次 1 — 行为 bug（P1，建议立即）

| 工单 | 内容 | 修复方向 | 测试 |
|---|---|---|---|
| W1 (HC-3) | `panel.rs:214` 删除 `\|\| host.nested.is_empty()`——`host.key.is_none()` 已承担首帧初始化语义 | 一行修复；Matrix/Settings/ErrorStrip/LibraryRail 恢复键控 | 无嵌套面板 key 不变时 render 计数平（需 W6 的计数器，可先在 Matrix 上验证） |
| W2 (SD-1+SD-2) | 同一根因的两面：编辑器家族扩张没跟上。`InspectorPanel::observe_nested` 补 `balance_inputs`/`node_gain_inputs`/`node_pan_inputs`/`inspector_faders` 槽/`compressor.inputs`/`compressor.sliders`；`typing_focus` 补 `value_inputs` + 上述四族（或改为"任何 InputState 实体持焦"判定） | sidebar.rs / window.rs；顺手消除 Graph 页 `keep_graph_focus` 抢焦 | 检查器编辑器键入即上屏（headless activate_window + 键入断言）；键入中 Ctrl+Z 不派发 UndoGraph；Graph 页编辑器打开不闪关 |
| W3 (HC-2) | `MatrixKey` 的 `compact: shell.compact` 是装错概念：渲染消费的是 `shell.density`。补 `preferences`（与其他页键对齐）或最少补 `density` | matrix.rs:29-36 | 改密度断言矩阵重建；顺带为"key 漏输入"类加一条模式回归 |
| W4 (HC-1) | 手势生命周期：`set_page`、`apply_project_presentation`、新增窗口失活钩子统一走 `on_cancel` 等价清零 + `sync_drag_demand()`；防御侧给 `frames_wanted` 的 DRAG 需求加"当前页仍挂 canvas"谓词，杜绝 poll 泵饥饿 | window.rs:631 / graph.rs:89-103 / desktop.rs:468 | 中键按住切页 → 手势清零、Matrix 页 meter 仍随 poll 更新；rubber band 不残留 |

W1 与 W2 无依赖可并行；W4 独立。四个都是小 diff，但 W2 需要仔细核对族清单（对照 strip 面板 `mixer/meter_frame.rs:263-313` 的完整集合）。

## 批次 2 — 防御测试网（P2，紧跟批次 1）

| 工单 | 内容 | 说明 |
|---|---|---|
| W5 (HY-10) | inspector 热表面回归：仿 `tray_mixer/tests.rs:39` 断言 registered→painted | hazard-10 失败类（编译通过、永远画空、无运行时检测）在该表面目前零防护 |
| W6 (HY-11) | step-4 七面板补 test-only render 计数 + key 完整性测试（无关 notify 计数平 / key 内输入重建） | 依赖 W1——键控失效不修，"计数平"断言根本不可达；这也是 #48 冻结类唯一系统性护栏 |
| W6 附带 (HY-12.3) | `ensure_subscription` 同 topic 重复 tick 不重建订阅的单测 | 3 行级，补齐 signal_visibility.rs 删除后的最后一块语义 |

## 批次 3 — 数据面语义修补（P2，可按条独立择机）

- **W7 (SD-3)**：send 编辑器提交复用 `parse_gain_db`，删除分叉的 −∞ 拼写集与死臂 `"−∞"`。低风险直接做。
- **W8 (HC-11+SD-4)**：`can_start_transport` 指纹统一整理：latency_profile 用判别值（非布尔折叠）、补 selection 身份 / `max_block_frames` / catalog 规模三元组。两报告同域合并一项。
- **W9 (SD-7)**：拖拽中 undo——`undo_redo_graph` 检测任一 `dragging` 时先取消活动拖拽或拒绝；否则 Release 覆盖撤销结果。与 HC-6（见疑案 R3）相邻，先做本条（行为明确）。
- **W10 (SD-8)**：legacy 采样 `if publication { return true }` 改用 NodeMeters 同款判据（status/电平变化才唤醒），消除订阅存续期 33ms 恒 notify。第三轮"烧核"同类的轮询率残留。
- **W11 (HC-9)**：`on_camera_fit` 注册改用户相机投影或加视口剔除——放大态下离屏节点不再持续订阅/推进弹道。
- **W12 (HC-7)**：`readout_cache` 按 `frame.tracks` 键集裁剪（on_rebuild 同步清理），堵唯一漏网的几何/文本缓存泄漏。
- **W13 (HC-14)**：`node_meter_points.borrow().clone()` 与 `seen.clone()` 移出显示率路径（借用穿透或 retain 二次遍历）。
- **W14 (SD-9)**：`update_node_meters` 订阅条件收紧到"该页检查器确实渲染 meter"。
- 需决策后做：SD-5（RMS 衰减，DEC-2）、SD-6（Peak 读数语义，DEC-3）、SD-10+HC-13（死状态机，DEC-6/批次 4）。

## 批次 4 — 死代码与资产清理（低风险机械）

- **W15**：零引用直接删——DC-1（ServiceCore 三方法，17 行）、DC-2（TIMEOUT，5 行）、DC-8（`wide()`+`NODE_WIDE_WIDTH`，~10 行）。
- **W16**：DC-6 分诊——判定产品预留的加 doc 注释（#7/#8/#13/#16），其余（#1-6、#9-12、#14-15）连测试断言一并删或 `#[cfg(test)]` 化；#15 JobResult 删除前编译验证泛型传播。#9/#10 顺带消两处英文硬编码（APP-9 残余）。
- **W17**：资产——DC-3（白名单 headphones/monitor 两臂 + 文件）、DC-4（19 个零引用 svg）**先过 DEC-4 设计 roadmap 确认再删**；白名单测试补"登记↔使用双向一致"断言。
- **W18 (DC-5=ST-12)**：component_gallery cfg 门控或 example required-features——过 DEC-5。

## 批次 5 — 结构机械重组（纯移动/改名，无行为变化）

- **W19 (ST-1+ST-2)**：`mixer/meter_frame.rs`、`graph/meter_frame.rs` 改名 `page.rs`（git mv 保历史）；`MeterTrace` 上移 `StudioShell`、`performance.rs` 移 `window/`。一次消除两处最大"名/位置≠职责"，也为 W6 的各页计数器留对位置。
- **W20 (ST-3+ST-4)**：`TestObserve` 移中立模块（消除 pages→window 唯一反向边）；`display_label`/`source_name`/`output_name` 移 presentation.rs（恢复 state.rs 自述边界）。
- **W21 (ST-7)**：`reset_clip` ×3 / `mixer_edit` ×2 收敛为以 `Entity<UiState>` 为参的 helper。
- **W22 (ST-5=DC-7)**：`cached_splice` 内联（注释移 PanelHost 文档）——下次触碰 panel.rs 时捎带，不单独开 PR。
- 明确**不做**（02 报告"不要拆"清单）：monitoring.rs、window.rs/StudioShell、state.rs 主体、node_inspector.rs、三个 tests.rs、两个几何寄存器、PanelHost/HotSurface/SharedFrame/NestedGate。

## 批次 6 — 文档对齐（半小时级）

- **W23 (HY-3)**：design 17 "The cache splits per item" 段改写为实际机制（快照侧谓词 `strip_probe_candidate`），删除 "UiState 在 poll 派生 / 替换 refresh_observation_nodes" 失实表述。
- **W24 (HY-4)**：References 行改为"UI 消费侧已收敛；X-1 仍开放"，X-1 放回跟踪清单。
- **W25**：HY-5（hazard 编号缺 9）、HY-6+HY-1（chrome "NOT cached" 术语与 keyed PanelHost 矛盾）、HY-7（Status commit 范围）、HY-8（设计草图加"设计期签名"声明）、HY-9（NodeMeters "promoted to entity" 措辞）。
- **W26 (HY-14)**：删 mixer/tests.rs 两处调试 eprintln。

## 实机疑案合并表（一次实机 pass 可全部覆盖）

| # | 疑案 | 关联 | 验证要点 |
|---|---|---|---|
| R1 | 遮挡恢复可见性卡死（根因假说：`presentation.visible` 只被平台事件写、render 读 `is_visible()` 不写回） | S-1 | 全遮挡 ≥2s 快速 alt-tab 回；看 meter_paints 停滞与 visible 标志；修复方向=仿 iconic 自愈轮询 |
| R2 | 缓存面板内 4 处未处理滚动容器实机行为（hazard 7 vs headless 绿的矛盾） | S-2 | settings 弹层/Devices 列表/node library/flyout 横滚：滚动生效？点击命中准？ |
| R3 | kit Slider 卸载/失活是否补发 Release | S-3=SD 待实测 3 | 决定 HC-6 定级（P1 化或降防御项）与拖拽尾值丢失 |
| R4 | SD-1/SD-2 实机严重度（盲打/闪关是否如推演） | SD 待实测 1/2 | 批次 1 前置确认，可直接实机复现 |
| R5 | −96dB 阈值 vs 实机底噪（真实麦克风空闲是否永不 stand-down） | SD 待实测 4 | 行为"诚实"但常驻帧循环，可能需产品判断 |
| R6 | a11y 激活瞬间缓存命中帧读值时效 | S-4 | Narrator + Tab 遍历推子值 |
| R7 | inspector 弹簧收展动画期缓存 mask 时效 | S-5 | 收展 300ms 内条形是否越界/空条不画 |
| R8 | 同设备双 Source/Sink 可否构造 | S-6 | HC-5 前置 |
| R9 | 失焦保活 dt 尖峰下 marker/hold 行为 | SD 待实测 5 | compositor 节流长尖峰 |
| R10 | iconic 恢复后 SharedFrame 兄弟注册互删 | SD 待实测 6 | 长时间最小化恢复对拍注册表 |
| R11 | IME 全角负号/−inf 变体接受集 | SD 待实测 7 | SD-3 用户入口 |

## 待用户决策项

1. **DEC-1（批次 1 范围）**：W1–W4 是否作为一个修复 PR 线立即推进（建议：W1+W3 一个 PR，W2 一个 PR，W4 一个 PR）。
2. **DEC-2（SD-5）**：RMS 积分器降到 ~75ms（改手感，自然衰减 ≈58 dB/s 覆盖全部档位）还是承认 29 dB/s 下限（改注释+给 Fast 档补测试）？
3. **DEC-3（SD-6）**：条带数字峰值读数改 marker 语义（与检查器一致，代价是下落期重 shape，可量化缓存）还是保留 max-hold + 加点击重置？
4. **DEC-4（DC-4/DC-3）**：21 个零引用 svg 直接删，还是设计侧（Figma transport 等）有近期启用计划？
5. **DEC-5（DC-5）**：component_gallery 门控方式——cfg feature、example required-features、还是维持现状加注释？
6. **DEC-6（HC-13+SD-10）**：`gain_slider`/`pan_slider`/`muted`/`dimmed` 死状态机——删除（legacy 传输控件残余）还是计划接视图（接则必须补 dragging 守卫与 unmute 重读路径）？
7. **DEC-7（ST-9，择机）**：`frames_eligible` 页白名单改挂载驱动（消除靠纪律维护的不变量）——牵帧循环，需 ui_capture 验证，是否列入近期？
8. **既有遗留**：~481KB stress 明细（原 D-5）、docs/plans 归档（原 D-6）、X-1 双目录收敛路线（放回清单后需重选 A/B 路线）。

## 建议顺序

批次 1（W1–W4）→ 实机 pass（R4 先行，R1/R2/R3 高价值）→ 批次 2（W5/W6）→ 批次 3 按 W7→W10→W8→W9→W11..W14 择机 → 批次 4/5/6 可穿插在等待实机/CI 的间隙。所有 UI 行为类 PR 附 `ui_capture` 证据（AGENTS.md 要求）。

## 落地记录（2026-10-31，当日整改完成）

DEC-1..DEC-6、DEC-8 已由用户拍板，六批次当日全部落地 main；未完成项已开 issue（#79–#91）。

| 批次 | 工单 | PR |
|---|---|---|
| 1 行为 bug | W1+W3 缓存键（HC-3/HC-2） | #62 |
| 1 行为 bug | W2 编辑器族单一来源（SD-1/SD-2） | #63 |
| 1 行为 bug | W4 手势生命周期（HC-1） | #61 |
| 2 测试网 | W5 inspector 热表面回归（HY-10） | #68 |
| 2 测试网 | W6 七面板键完整性 + W26 eprintln（HY-11/HY-14） | #72 |
| 2 测试网 | W6b 键缺口修复（InspectorKey 页 / ErrorStripKey dispatch_error / LibraryKey selected_node） | #76 |
| 3 数据面 | W7 send 解析统一（SD-3） | #54 |
| 3 数据面 | W8 传输指纹补全（HC-11+SD-4） | #59 |
| 3 数据面 | W9 拖拽中 undo（SD-7） | #69 |
| 3 数据面 | W10 legacy 泵唤醒政策（SD-8） | #67 |
| 3 数据面 | W11 相机注册幂等（HC-9） | #71 |
| 3 数据面 | W12 readout 泄漏 + W13/W14 泵克隆与页面条件（HC-7/HC-14/SD-9） | #66、#70 |
| 3 数据面 | SD-5 RMS 弹道定案路线 (a)（DEC-2） | #56 |
| 3 数据面 | SD-6 峰值读数 marker 语义（DEC-3） | #66 |
| 4 死码 | W15+W16 死 API 与分诊（DEC-6 核查） | #64 |
| 4 死码 | gain/pan 滑条状态机与 muted/dimmed（DEC-6） | #65 |
| 4 死码 | W17 图标资产（DEC-4） | #60 |
| 4 死码 | W18 gallery 门控（DEC-5） | #58 |
| 5 结构 | W19 meter_frame→page 改名 + MeterTrace 上移（ST-1/ST-2） | #75 |
| 5 结构 | W20 TestObserve 与显示投影归属（ST-3/ST-4） | #73 |
| 5 结构 | W21 clip-reset/mixer-edit helper（ST-7） | #74 |
| 5 结构 | W22 cached_splice 内联（ST-5/DC-7） | #78（替代 base 分支被删的 #77） |
| 6 文档 | W23–W25 design 17 对齐 + HY-1 chrome 注释 | #55 |
| 6 文档 | W26 调试 eprintln | #72 |
| 既有遗留 | D5 stress 产物 + D6 已完成计划删除（56 条目，DEC-8） | #57 |

未完成项 → issue：R1 可见性自愈 #79；R2+S-2 滚动-under-cache 实机 #80；R3+S-3 dragging 复位（kit Release 核查前置）#81；HC-4 settings 键 #82；HC-5+S-6 探针共享槽 #83；HC-8 标签整形 #84；HC-10 覆盖层压把手 #85；SD-11 i18n 断层 #86；R5 −96dB 阈值 #87；ST-9 帧资格白名单（DEC-7 暂缓）#88；X-1 双目录（待定）#89；flaky managed ack-barrier #90；其余实机疑案清单 #91。

### 审计误报与偏差修正（整改中发现）

- **HC-9 字面机制不复现**（#71 实证）：fit 相机仅在初始帧采用（`CanvasFrame::new` 只在 `!camera.adjusted` 时 fit），用户缩放后 `frame.camera` 即用户相机且 `ControlsKey` 含 camera——"fit 注册只增不减"的前提不成立。落地的加固是注册改整表替换 + 全链路回归测试。
- **DC-6 两处计数误差**（#64 实证）：`timeline` 模块本身已是 `#[cfg(test)]`（`allocate_epoch` 并无生产调用者，审计原表称"生产在用"不实）；`next_epoch` 同理仅补注释。其余分诊照计划。
- **DC-5 的 11 个 widget re-export 并非 gallery 独占**（#58 实证）：`widgets/action_tests.rs`（gpui-test 门）消费其中 9 项，故保留 re-export 链。
- **ST-4 消费者计数 7→8**（#73 实证）：devices.rs 的具名导入漏计。
- **ST-5 文档锚点落点**：按 PanelHost 类型级文档（#78）而非 Render impl，效力等同。
- **W6 测试网发现三个此前未知真冻结**（#72 证据测试 + #76 修复）：InspectorKey 漏 `presentation.page`、ErrorStripKey 漏 `state.dispatch_error`、LibraryKey 漏 `selected_node`——同 HC-2 类，审计静态扫描未覆盖到。
