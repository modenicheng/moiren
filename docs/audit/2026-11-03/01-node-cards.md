# 节点卡片与目录审计 — moiren-app(子代理 01)

> 范围:PR #93 `feat/node-ui`(merge 3321062,55 文件 +8132/−548)的节点卡片面 — `crates/moiren-app/src/ui/pages/graph_canvas_view.rs`(784 行)、`ui/pages/graph_canvas_view/nodes.rs`(1223 行)、`ui/pages/node_catalog.rs`(222 行)、`ui/theme.rs`(token 清点)、`ui/widgets/action_assets.rs`(图标白名单)、`assets/icons/`、`docs/designs/18-node-cards-and-wiring.md`;交叉参照 `ui/pages/graph_canvas*`(投影层,另一代理主责)、`ui/window/graph/page.rs`(热层调用点)、`ui/window/graph/library.rs`(库,另一代理主责)。当前检出 `fix/spectrum-standard-curve` = main(3321062)+ 2 个频谱提交(ed93c58、e9387d2,均触及 nodes.rs),工作区干净。
> 方法:只读。全文通读分区文件与 `graph_canvas/` 全部子模块(camera/frame/knob/layout/overlays/wire/curves/tests);`git diff 3321062^1 3321062` 区分新增/存量;每个可疑点走 grep 全 crate 复核定义↔引用;theme 新 token 逐个计数读者;图标三方核对(白名单 ↔ 文件系统 ↔ 请求点)。未运行 cargo。发现 ID 前缀 NC。

## 总体结论

PR #93 的卡片重写整体质量高:状态机(轮廓分级)、dial pill 几何(带面积断言单测)、谱图绘制(除零/退化/停驻守卫齐全)、图标登记(双向护栏测试)都经得起推敲,且 2026-10-31 审计的 DC-3/DC-4/DC-8/DC-11 全部确认已修复。本轮发现集中在两类:**三份手工维护的 kind→tint 映射出现实漂移**(库行 vs 画布卡片颜色不一致,且 #93 又新增了第三份拷贝而非收敛),以及**拖拽幽灵卡片与注释声称的"落卡即所见"不符**(输入端口圆点全都不画)。既往复发过的"缓存键/谓词没跟上新族"未在缓存侧复发,但以"新增族时旁路映射没跟上"的同型形态出现在 tint 映射上(NC-1)。

## 发现汇总

| ID | 严重级 | 一句话 | 状态 |
|---|---|---|---|
| NC-1 | P1 | kind→tint 三份手写映射漂移:ChannelMapper 库行 NODE_DSP vs 卡片/幽灵 NODE_UTILITY | 已亲证 |
| NC-2 | P2 | 库拖拽幽灵卡不画输入端口圆点、无 binding chip/knob/FFT detail,与自述"exact default card"不符 | 已亲证 |
| NC-3 | P3 | overview 绘制闭包按节点重建整套 NodeLayout(含 Vec 分配),但只用了 card 矩形;节奏为每次面板重建 | 已亲证 |
| NC-4 | P3(P2 尺寸) | `NodeLayout::badge` 布局槽只算不画不打,是轮廓化修订前的残留 | 已亲证 |
| NC-5 | P3 | `NodeTemplate::kind()` 恒返回 `Some`,库的 `enabled` 判定恒真、`else` 分支不可达 | 已亲证 |
| NC-6 | P3 | `graph_canvas.rs` 矩阵预览 `let _ = cols;` 死局部(8 列截断算了不用) | 已亲证(跨区注记) |
| NC-7 | P3 | 设计文档 18 与实现漂移:spline→折线、1-2-5 网格→十进制网格未回写;Meter/Recorder 卡身与 B3 表不符 | 已亲证 |
| NC-8 | P3 | `CanvasCallbacks.on_canvas_mouse_move` 用 Box 而兄弟回调全 Rc,构造侧还要 `Rc::from` 转一手 | 已亲证 |

---

## NC-1 (P1) kind→tint 三份手写映射漂移:ChannelMapper 在库与画布上颜色不一致

- 位置(三份映射):
  1. `crates/moiren-app/src/ui/pages/node_catalog.rs:152-161` — `NodeTemplate::tint()`,`Self::Rack => theme::NODE_UTILITY, _ => theme::NODE_DSP`,**ChannelMapper 落入 `_` → NODE_DSP**(淡紫);
  2. `crates/moiren-app/src/ui/pages/graph_canvas.rs:377` — `NodeKind::ChannelMapper => ("route", theme::NODE_UTILITY)`,画布卡片 **NODE_UTILITY**(灰);
  3. `crates/moiren-app/src/ui/pages/graph_canvas_view.rs:338-352` — `node_drag_preview` 手写第三份 match,`NodeTemplate::Bus | NodeTemplate::ChannelMapper | NodeTemplate::Rack => theme::NODE_UTILITY`。
- 失败机理(已亲证,读通三处及调用方):库行图标瓦片底色/图标色取 `template.tint()`(library.rs:42、279),ChannelMapper 在 PROCESSING 组内是**灰色**,同组其余 7 个 DSP 种类为淡紫;从库拖出时幽灵卡铁轨与落地图标均为 **NODE_UTILITY 灰**;即同一种类在库与画布两处呈现不同类别色,且拖拽瞬间发生一次变色。`NODE_DSP = hsla(0.723, 0.781, 0.731)` 与 `NODE_UTILITY = TEXT_FAINT`(灰)肉眼可分。
- 溯源:#93 之前已有两份(目录 vs 画布,旧画布 `3321062^1:graph_canvas.rs:210` 同样 UTILITY),**#93 新增了第三份拷贝**(node_drag_preview 的 match,338-352),它按画布侧取值,等于用复制把漂移固化而不是收敛到单一来源;同一 PR 里 `icon()` 的同型漂移(Spectrum/Rack 图标)倒是修掉了(a33909e),tint 侧漏网。
- 建议:让 `NodeTemplate::tint()` 成为唯一权威 —— 修掉目录 `_` 臂(为 ChannelMapper 显式给 NODE_UTILITY,与画布对齐),`node_drag_preview` 直接调 `template.tint()` 删除手写 match;`graph_canvas.rs` 的 kind→tint 若难以复用目录(工作在 NodeKind 域),至少加交叉断言测试:对每个 `NodeTemplate`,`template.tint()` == 该 kind 卡片在 `positioned()` 里的 tint。

## NC-2 (P2) 库拖拽幽灵卡不画输入端口圆点;chip/knob/FFT detail 也不按"默认卡"呈现

- 位置:`crates/moiren-app/src/ui/pages/graph_canvas_view.rs:331-426`(`node_drag_preview`),关键在:
  - `:375 input_ports: Vec::new()` 而 `:354-358 input_count` 对 Output 和全部 DSP/ROUTING 模板给 1;
  - `:390 binding: None`、`:366-370 detail` 对 Spectrum 给 `"L/R"`、`knobs` 恒空。
- 失败机理(已亲证):`nodes.rs:1111-1116` 输入端口绘制循环 `layout.inputs.iter().copied().zip(&node.input_ports).zip(&layout.input_hints)` —— `layout.inputs` 由 `input_count` 生成(1 个锚点),`node.input_ports` 为空,zip 出 **0 次迭代**:除三个 source 模板外,**所有模板的幽灵卡都不画输入端口圆点与 "Nch" 提示**;Output 模板则一个端口都不画(`has_output=false` + 输入缺失),尽管 `:353` 注释明言 *"A fresh card's port shape: … the sink only inputs"* —— 意图就是 sink 幽灵要显示那个输入口。同理 Input/Output 幽灵画文本 detail 而非落卡上的 unbound binding chip(`binding: None` → `paint_node` 走 detail 分支),Tone 幽灵无频率旋钮,Spectrum 幽灵右上角 detail 是 "L/R" 而非落卡的 `FFT 2048`。与函数文档 *"the exact default card the drop will create… painted by the shared node painter"*(`:329-331`)直接矛盾。
- 用户可见度:每次从库拖拽(高频手势)期间,幽灵与落卡外观不一致;落卡本身正确,属瞬时外观缺陷,故 P2。
- 建议:预览节点按模板补 `input_ports: vec![CanvasPort { id: 占位, channels: 2 }]`(或让绘制循环以 `input_count` 为准、hint 文本退回 `node.channels`);Input/Output 补 unbound `BindingChip`、Tone 补 `CardKnob`、Spectrum detail 给 `t!("FFT %{fft}", fft = 2048)`;或最省事的替代 —— 弱化注释,明说幽灵是"通用壳卡"。顺带:若采纳 NC-1 建议,此处 tint match 一并删除。

## NC-3 (P3) overview 绘制闭包为每个节点重建完整 NodeLayout,却只用 card 矩形

- 位置:`crates/moiren-app/src/ui/pages/graph_canvas_view.rs:447-479`,`:464` `let layout = super::graph_canvas::NodeLayout::new(node);`
- 失败机理(已亲证,节奏核实过):`NodeLayout::new` 为每个节点构造全部字段并分配 5 个 Vec(inputs、input_hints、knobs、chip 映射、以及内部 collect),overview 只消费 `layout.card` 一个矩形。执行节奏为**每次面板重建**(页面在 `CachedPanel` 内,gpui-fast 回放缓存面板的 paint 不重跑闭包 —— page.rs:1-6 注释;ControlsKey 含 camera/connection/selection,拖线、平移缩放、选区时每次鼠标事件都整页重建),不是每帧;稳态只有表针动画时零成本。故为重建期的冗余分配 + 重复劳动(同一次重建里 `CanvasFrame::new` 已为每个节点做过一遍投影前布局)。
- 建议:overview 闭包里用 `Bounds { origin: point(node.x, node.y), size: size(node.width(), node.height()) }` 内联取卡矩形(4 行),删除对 `NodeLayout::new` 的依赖;或给 `CanvasNode` 加 `card_bounds()` 供两处共用。

## NC-4 (P3,若卡片头未来恢复状态字形则为 P2 尺寸) `NodeLayout::badge` 是"只算不画不打"的布局槽

- 位置:`crates/moiren-app/src/ui/pages/graph_canvas/layout.rs:90`(字段)、`:272-278`(构造)、`:317`(project);唯一"读者"是测试 `graph_canvas/tests.rs:361-368` 的矩形断言;`theme.rs:87-89` 的 `BADGE_SIZE` 注释仍描述 *"the badge is the error/loading marker"*。
- 失败机理(已亲证):设计 18 第一稿在卡片头放 error 字形 + loading chip;验收二轮修订(639e916,设计文档 139 行注记)把状态收敛到轮廓 + 故障 tooltip,`paint_node`(nodes.rs:1019-1052)只画 `layout.expand`,`hit_test`(frame.rs:112)也只测 expand —— badge 槽从此无消费者,但布局几何每个节点每次重建照算照投影。属修订未删净的残留死面(连带文档:设计文档 B2 段落 `docs/designs/18-node-cards-and-wiring.md:166-170` 仍写着 *"state glyphs at the right (error glyph, loading chip, expand button)"*)。
- 建议:删 `NodeLayout::badge` 字段与两处构造、tests.rs 对应断言;`BADGE_SIZE` 更名(如 `GLYPH_SIZE`)或保留并改注释只描述 expand。若卡片头将来确要恢复状态字形,应连消费者一起加回来而不是留空槽。

## NC-5 (P3) `NodeTemplate::kind()` 恒 `Some`,调用侧出现恒真判定与不可达分支

- 位置:`crates/moiren-app/src/ui/pages/node_catalog.rs:110-128`(`Some(match …)` 17 臂全列,无 None 路径);消费点 `ui/window/graph/library.rs:40` `let enabled = template.kind().is_some() && …`(恒真)与 `:315` `let Some(kind) = template.kind() else { … }`(不可达)。
- 失败机理(已亲证):枚举 Option 的 None 变体零构造 —— 正是本轮猎捕的"枚举变体零构造"形态,只是发生在返回类型上。`enabled` 实际只由 `pending_node_spawn.is_none()` 决定;`:315` 的 else 分支(注释意为"模板无真实 kind 时禁用")永远不会走到,给读者虚假的安全感。
- 建议:二选一 —— ①返回 `NodeKind` 直接量,调用点删掉恒真/不可达分支(测试 `spectrum_is_visible…`/`catalog_exposes…` 同步改);②若确要保留"目录可容纳无 kind 的装饰模板"的扩展点,加注释声明,并把 library.rs:40 的 `is_some()` 挪到 `:315` 一处使用,消除双处判定。

## NC-6 (P3,跨区注记) 矩阵预览的列截断值算了就扔

- 位置:`crates/moiren-app/src/ui/pages/graph_canvas.rs:528-532` — `let rows = preview.len().min(8); let cols = preview.first().map_or(0, |row| row.len().min(8)); … let _ = cols;`
- 失败机理(已亲证):`cols` 计算后由 `let _ = cols;` 显式丢弃,实际列截断只靠绘制侧 `paint_matrix_preview` 的 `.take(8)`(nodes.rs:731);`body_height` 只用 rows。行为正确(注释" capped at 8 × 8 "由绘制侧兜住),但死局部 + 双处截断语义易漂移。文件主责在另一代理,此处仅登记与卡片绘制相关的部分。
- 建议:删 `let cols…` 与 `let _ = cols;` 两行,或在 `card.matrix` 写入前统一截断(存进去的就是 ≤8×8),绘制侧 take 变纯防御。

## NC-7 (P3) 设计文档 18 与实现的漂移

- 位置与条目(已亲证,逐条对照):
  1. **曲线渲染**:`docs/designs/18-node-cards-and-wiring.md:276-279` 声称 *"the curve is a Catmull-Rom spline … the FabFilter-style smoothness rather than a polyline"*;实现(nodes.rs:609-614)是**直线折线**,且注释明确记录了反向决策(分支提交 ed93c58:*spline handles overshoot at sharp peaks and would display levels the spectrum does not contain*)。文档未回写。
  2. **频率网格**:文档 `:248-251` 声称 1-2-5 系列(20, 50, 100, 200, 500…);实现(nodes.rs:561-583)是**十进制方格纸**(每个十进位一条强线 + 位内整数弱线,分支提交 e9387d2)。文档未回写;两者都发生在本分支的两个 fix 提交之后,属"修了代码忘了改设计"。
  3. **卡身种类**:文档 B3 表 `:189-196` 声称 Recorder 是 *"capacity/usage line"*、Meter 是 *"large per-channel bars + peak/RMS readout"*;实现两者都是普通 detail 行(`detail_string`,graph_canvas.rs:239-246)+ 标准表针条,无大表棒/读数。
  4. **卡片头字形**:见 NC-4,B2 段仍描述 error glyph + loading chip。
- 建议:文档加一节"post-merge revisions"或在对应段落补注(与文档 139 行已有的验收二轮注记同风格),把 1/2/4 三条对齐;第 3 条要么补实现要么改表格述为 detail line。设计文档是审计与后续 PR 的对账基准,漂移会放大 NC-1 这类"多份真值"问题。

## NC-8 (P3) `CanvasCallbacks` 一个 Box 字段混在 Rc 家族里

- 位置:`crates/moiren-app/src/ui/pages/graph_canvas_view.rs:111` `pub on_canvas_mouse_move: WindowAppFn<(Point<Pixels>, Option<MouseButton>)>`(Box)vs 其余回调全部 `Rc<…Callback>`;`:526-527` 构造侧 `Rc::from(on_canvas_mouse_move)` 转一手再克隆进 5 个鼠标处理器。
- 失败机理(已亲证):该回调被 5 个 handler 共享,必须 Rc 化;字段类型却声明为 Box,调用方构造时先 Box 再转 Rc,多一次指针拷贝与一层类型不一致(`on_pan` 也是 Box,但它单点使用,尚可)。
- 建议:`on_canvas_mouse_move` 字段类型改成 `Rc<WindowAppCallback<(Point<Pixels>, Option<MouseButton>)>>`,与兄弟一致,删掉 `Rc::from`。纯卫生,一行级。

---

## 既往项复核(docs/audit/2026-10-31 与 2026-10-27)

| 既往项 | 本轮状态 | 证据 |
|---|---|---|
| DC-3/DC-4 死图标(headphones/monitor 等 21 文件) | **FIXED,且加了双向护栏** | 文件已删;`action_assets.rs:229-329` `every_icon_asset_has_a_request_path` 现同时断言"请求可解析"与"文件必有请求点"(正是 DC-3 建议的反向断言);本轮三方核对 28↔28、figma-nodes 3↔3 全吻合 |
| DC-8 `CanvasNode::wide()`/`NODE_WIDE_WIDTH` | **FIXED** | 字段/常量/`#[allow(dead_code)]` 均已删,`width()` 现按 `spectrum` 分派 SPECTRUM_W/NODE_WIDTH;grep 零孤儿 |
| DC-11 theme 死 token | **未复发** | PR #93 新增 33 个 token(card 子模块 32 常量 + `node_tint()`)逐一 grep,全部 ≥1 生产读者(最少者:TITLE_WIDTH、DIAL_TRACK_W、DIAL_SWEEP_DEG、CURVE_BODY_H、MATRIX_GAP、PORT_Y、card::THUMB 各 1 处,均真实调用);被删旧 token(DECO_HEIGHT、FADER_Y、TRACK_WIDTH 等)全库零残留(tray_mixer 自有 METER_HEIGHT 为同名异体,存量) |
| 复发失败类"缓存键/谓词没跟上新族" | **缓存侧未复发;同型出现在映射侧** | 新增 OverlayRects 槽位 3 个(toolbar graph.rs:948、overview graph_canvas_view.rs:440、library-handle sidebar.rs:423)登记齐全,热层 carve 全覆盖;热层遮挡谓词 `covered`(page.rs:286-294)正确覆盖 meter+spectrum 两族;但 kind→tint 新增第三份拷贝未跟上 ChannelMapper 的既有取值(NC-1),与该失败类同根因:多份手写真值在扩族时各自演化 |

## 已核对无问题清单

- **新旧并存判定**:`graph_canvas.rs`(纯投影:CanvasNode/CanvasWorld/候选判定)+ `graph_canvas_view*.rs`(绘制/输入)是分层关系而非两代卡片代码;全库唯一卡片绘制器是 `nodes.rs::paint_node`,无旧绘制路径残留(2026-10-31 时代的 DOM/D1 绘制器早已统一到此处)。
- **命中区 vs 绘制区**:端口命中半径 8px(canvas 坐标,frame.rs:95/178)≥ 绘制圆点半径 `4*scale`(zoom 上限 2,camera.rs:56/69 全路径 clamp 0.25..=2,fit 下限 EPSILON),低 zoom 放大命中区符合注释意图;expand/knob/chip/card 的命中矩形与绘制矩形同源(`NodeLayout` 同一 bounds),无错位。
- **状态机优先级与阈值**:`paint_node`(nodes.rs:986-994)faulted(DANGER/OUTLINE_W)> preparing(WARNING/OUTLINE_REST_W)> selected(PRIMARY/OUTLINE_W)> rest(GRIP/OUTLINE_REST_W),与设计 18 B1 的"error > loading > selected"及 1px/1.5px 分级一致;状态不改卡身(旋钮/表条照画)符合 B1 修订。
- **除零/NaN/退化守卫**:`capsule_path` 长度 <1e-3 bail、`arc_capsule_path` 弧 <0.5° bail 且值弧外部另有 `to-from > 0.002` 守卫(≈0.54°,两阈值余量 0.04°,若改 DIAL_SWEEP_DEG 需同改,已在 NC-8 外记录为微注);谱图 `width/height <= 0` 早退、span 过滤 `hi > lo` 否则回落 `SPECTRUM_NOMINAL_SPAN_HZ(19.686, 20158.7)`、点数 <2 或全体 ≤ REST_DB 视为停驻;`paint_grid` 的 step 因 zoom 全路径 clamp 恒 ≥24;`shape_text` 二分切分在 char_indices 边界上,无 panic 路径,超宽落省略号。
- **文本截断**:标题(TITLE_WIDTH)、chip 标签、旋钮 label/value、故障 tooltip 行(MAX_LINE=360)全部经 `shape_text` 截断;tooltip 左缘 clamp 到画布、底部空间不足翻转到卡上方(nodes.rs:860-872)。
- **热层调用安全**:`paint_fault_tooltip` 的 index 由热层按 `hovered_node` 在**当前帧** world 里 `position()` 现查(page.rs:202-209),无陈旧索引越界;悬停由 mouse_move 命中驱动(graph.rs:350-363),与卡片绘制同源几何;热层对 HUD 覆盖物 carve(page.rs:322-349)、对被后画卡片遮挡的 meter/spectrum 跳过(`covered` 谓词),连线拖拽期间让位缓存画布(page.rs:261-263)。
- **clip lamp 几何**:横向表条预留 `3*scale`(nodes.rs:143)与 lamp 位置 `track.right()+3`(meters.rs:47-50)一致;纵向 lamp 在表条顶端上方 10px,落在 header 右侧空白区(x 坐标避开 expand/title 区),在卡片 content mask 内。
- **每帧成本疑点排除**(重要):缓存面板"回放 paint 不重跑闭包"(page.rs:1-6),故 `paint_canvas_layer`/`paint_spectrum_body`/`paint_label` 的 shaping 与 PathBuilder 只在面板重建时执行;每动画帧只有热层(paint_meter_bars/paint_spectrum_body 活体/tooltip),谱图每帧 4 次 shape_text(3 刻度 + detail)是设计内成本。
- **i18n**:`FFT %{fft}` 键随 #93 同步到 zh-CN.yml 与 i18n-keys.tsv;NodeTemplate 全部 title/description 键在 zh-CN.yml 中齐备;`%{count} stage(s)` 单复数分支有注释说明。
- **测试覆盖**:节点卡片面带 layout/hit/candidate 单测(graph_canvas/tests.rs,909 行)、dial/胶囊几何面积断言单测(nodes.rs:1189-1222)、knob 数学单测(knob.rs)、目录搜索/kind 映射单测(node_catalog.rs)、图标双向护栏(action_assets.rs)。

## 统计

- P0:0;P1:1(NC-1);P2:1(NC-2);P3:6(NC-3..NC-8)。
- 立即可回收:NC-1 收敛三份 tint 映射(净删 ~15 行)、NC-4 死布局槽(~10 行)、NC-5 恒真分支(视方案 −10/+0)、NC-6 死局部(2 行)、NC-8 类型统一(1 行)。
- 文档对账:NC-7 四条漂移待回写设计文档 18。
