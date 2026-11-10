//! The strip's I/O block: one input row, the global Sends head and the send
//! list with its level bars. Routing rows retain graph edge/port identity,
//! including parallel connections.
//!
//! The block replaces the former two collapsible sections with the approved
//! prototype's single compact form: the input row answers "what feeds this
//! strip, and what can I do about it" (a checkbox dropdown behind the ghost
//! +/− button manages every input against the node's port cap), and the
//! sends live under a uniform-height list so every strip's cached body —
//! balance, fader, meter, title — starts at the same y and the meters line
//! up across the page.
use super::*;
use crate::service::GraphEdit;
use crate::ui::widgets::anchored_popover;
use crate::ui::widgets::fader::{SEND_BAR_H, send_bar};
use crate::ui::widgets::occluding_page::{claim_presses, occluding_page};
use crate::ui::widgets::overlay_scrollbar::{notify_owner, scroll_region};
use gpui_kit::base::ScrollbarAxis;
use gpui_kit::component::{Icon, Sizable, Size};

// --- I/O block geometry (the approved prototype's measurements) -------------
// No value here may depend on a strip's content: the block's height in each
// state is a page-wide constant. Rows truncate, the send list scrolls, and
// nothing grows.

/// The input row: 24 px of row plus its 1 px bottom rule.
const INPUT_ROW_H: f32 = 25.;
/// The Sends head, content-free so the collapsed block keeps its height.
const SENDS_HEAD_H: f32 = 22.;
/// One send row.
const SEND_ROW_H: f32 = 20.;
/// The send slot: row + flush bar; the bar's lower half doubles as the row
/// gap and the drag hit area, so slots stack with no extra margins.
const SEND_SLOT_H: f32 = SEND_ROW_H + SEND_BAR_H * 2.;
/// The send list's uniform row budget, shared by every strip; further sends
/// scroll inside the fixed height. The prototype's approved default.
const SEND_ROW_BUDGET: usize = 2;
/// The send list's fixed height: a 2 px top inset plus the budget.
const SEND_LIST_H: f32 = 2. + (SEND_ROW_BUDGET as f32) * SEND_SLOT_H;
/// The sends block's breathing room under the list / head.
const IO_BLOCK_PB: f32 = 4.;

/// The expanded I/O block's height: input row + head + list + bottom inset +
/// the block's 1 px bottom rule. `io` keeps this on every strip regardless
/// of how many inputs or sends it has.
// Referenced only by the headless suites, which compile as a separate
// target — the lib build itself never reads them.
#[cfg(feature = "gpui-test")]
#[allow(dead_code)]
pub(crate) const IO_EXPANDED_H: f32 = INPUT_ROW_H + SENDS_HEAD_H + SEND_LIST_H + IO_BLOCK_PB + 1.;
/// The globally collapsed I/O block: the send list folds away everywhere.
#[cfg(feature = "gpui-test")]
#[allow(dead_code)]
pub(crate) const IO_COLLAPSED_H: f32 = INPUT_ROW_H + SENDS_HEAD_H + IO_BLOCK_PB + 1.;

/// The input dropdown's box, measured against the strip's inner width (the
/// prototype hung it 4 px in from both strip edges).
const POPOVER_WIDTH: f32 = MixerPresentation::STRIP_WIDTH - 8.;
/// Row height inside the dropdown, for placing and scrolling.
const POPOVER_ROW_H: f32 = 24.;
/// The dropdown never grows past this; further candidates scroll.
const POPOVER_MAX_H: f32 = 260.;
/// The dropdown's head + padding, over the rows' height.
const POPOVER_CHROME_H: f32 = 30.;

/// A workstation glyph at the rows' small icon size. Icon resolves inherited
/// text color before painting, so a ghost button's color styles the glyph.
fn icon(path: &'static str) -> Icon {
    Icon::default()
        .path(path)
        .with_size(Size::Size(px(theme::ICON_SM)))
}

/// A routing row's click: the first click selects the edge (the Inspector
/// shows the route), a double click jumps to the Graph page with the same
/// edge selected — the connection becomes a highlighted wire there.
fn route_click(
    this: Entity<StudioShell>,
    ports: (moiren_core::graph::PortId, moiren_core::graph::PortId),
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |event, _, cx| {
        let jump = event.click_count() == 2;
        this.update(cx, |shell, cx| {
            let exists = shell
                .state
                .read(cx)
                .snapshot
                .graph
                .graph
                .edges()
                .iter()
                .any(|edge| (edge.src_port(), edge.dst_port()) == ports);
            if !exists {
                return;
            }
            shell.selection.node = None;
            shell.selection.route = Some(ports);
            if jump {
                shell.set_page(Page::Graph, cx);
            }
            shell.chrome.inspector_open = true;
            cx.notify();
        });
    }
}

pub(super) fn send_edit(
    this: Entity<StudioShell>,
    id: EdgeId,
    change: impl Fn(&mut SendParams) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        this.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                if let Ok(edge) = state.snapshot.graph.graph.get_edge(id) {
                    let mut params = *edge.params();
                    change(&mut params);
                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetSend {
                        edge: id,
                        params,
                    }));
                }
            });
            cx.notify();
        })
    }
}

/// Opens the strip's input dropdown: one dropdown at a time page-wide, hung
/// from the input row's registered rect.
fn open_input_popover(
    this: &Entity<StudioShell>,
    node: NodeId,
    anchor: &std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    cx: &mut App,
) {
    let anchor = anchor.get();
    this.update(cx, |shell, cx| {
        shell.mixer.presentation.input_popover = Some(InputPopover { node, anchor });
        cx.notify();
    });
}

/// Hover bookkeeping shared by the input and send rows: one notified field
/// per change drives the hover-revealed affordances (the dB readout, the
/// remove button) that a parent-hover selector cannot style. The strip id
/// rides along so the same edge's two rows (a source's send row and its
/// target's input row) reveal independently.
fn track_hover(
    this: &Entity<StudioShell>,
    key: (NodeId, EdgeId),
) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    let this = this.clone();
    move |hovered, _, cx| {
        this.update(cx, |shell, cx| {
            let next = hovered.then_some(key);
            if shell.mixer.presentation.hovered_route != next {
                shell.mixer.presentation.hovered_route = next;
                cx.notify();
            }
        })
    }
}

/// The selected rows' 2 px accent bar on the left edge; the soft accent wash
/// rides on the row itself, so the selected connection reads the same way on
/// both the input and the send row.
fn accent_bar() -> Div {
    div()
        .absolute()
        .left_0()
        .top_0()
        .bottom_0()
        .w(px(theme::ACCENT_BAR_W))
        .bg(theme::controls::PRIMARY)
}

/// An 18 px ghost icon button: no fill, no border, hover only. The rows'
/// trailing controls all share this column (the +/−, the send arrows, the
/// PRE/POST toggle), which is what keeps the row text on one baseline.
fn ghost_button(
    id: SharedString,
    glyph: impl IntoElement,
    tip: String,
    lit: bool,
    revealed: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .text_color(if lit {
            theme::controls::PRIMARY
        } else {
            theme::TEXT_FAINT
        })
        // A hover-revealed control (the single input's −) rests invisible
        // until its row's hover flips the flag.
        .opacity(if revealed { 1. } else { 0. })
        .cursor_pointer()
        .hover(|style| style.text_color(theme::TEXT).bg(theme::SURFACE_4))
        // The row underneath selects on click; the button owns its own press
        // so a +/− press never selects or jumps the row's edge.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(glyph)
        .tooltip(move |window, cx| component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
        .on_click(on_click)
        .gpui_test_observe()
        .into_any_element()
}

/// The strip's whole I/O block: the input row and the sends block, both
/// rendered live outside the strip's cached panel (they change at edit rate
/// and own a scroll container).
pub(super) fn io(
    shell: &StudioShell,
    entry: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("io-{:?}", entry.id)))
        .flex()
        .flex_col()
        .flex_none()
        .w_full()
        .child(input_row(shell, entry, cx))
        .child(sends_block(shell, entry.id, cx))
        .gpui_test_observe()
}

/// The input row answers "what feeds this strip, and what can I do about it":
///  - a node with no input ports (sources) shows a dead 无输入, no affordance;
///  - an empty node that can take input shows 无输入 and an always-visible +
///    opening the checkbox dropdown;
///  - a multi-input node keeps the + plus an `n/max` chip, and the dropdown
///    manages every input;
///  - a single-input node swaps + for − once occupied, which fades in while
///    the pointer is anywhere over the row, not only over the icon.
fn input_row(
    shell: &StudioShell,
    entry: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let graph = &shell.state.read(cx).snapshot.graph;
    let id = entry.id;
    // The node's own input ports are the cap: each port is single-connection,
    // so "n/max" is how many of them are wired.
    let cap = graph
        .graph
        .get_node(id)
        .map(|node| node.inputs().len())
        .unwrap_or(0);
    let ins: Vec<_> = graph
        .graph
        .edges()
        .iter()
        .filter(|edge| edge.dst() == id)
        .collect();
    let this = cx.entity();
    // The row's window rect, registered at paint: the dropdown hangs from it.
    let row_bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));

    if cap == 0 {
        // A source cannot take input; the row states that instead of hinting
        // at an operation that does not exist.
        return div()
            .id(SharedString::from(format!("input-none-{id:?}")))
            .flex()
            .items_center()
            .gap(px(theme::SPACE_1))
            .h(px(INPUT_ROW_H))
            .pl(px(theme::SPACE_2))
            .pr(px(theme::SPACE_1))
            .border_b_1()
            .border_color(theme::BORDER)
            .text_size(px(theme::TEXT_XS))
            .text_color(theme::TEXT_FAINT)
            .child(icon("icons/arrow-left.svg"))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(t!("No inputs").to_string()),
            )
            .gpui_test_observe();
    }

    // The edge the row stands for: the selected route when it feeds this
    // strip, otherwise the first input.
    let edge = ins
        .iter()
        .find(|edge| shell.selection.route == Some((edge.src_port(), edge.dst_port())))
        .or_else(|| ins.first())
        .copied();

    let selected =
        edge.is_some_and(|edge| shell.selection.route == Some((edge.src_port(), edge.dst_port())));
    let muted = edge.is_some_and(|edge| edge.params().mute);
    let name = edge
        .and_then(|edge| graph.node(edge.src()))
        .map(crate::ui::presentation::display_label)
        .unwrap_or_else(|| t!("No inputs").into_owned());

    let multi = cap > 1;
    let manage_tip = t!(
        "Manage inputs (%{used} of %{max})",
        used = ins.len(),
        max = cap
    )
    .into_owned();
    let chip = (multi && !ins.is_empty()).then(|| {
        let this = this.clone();
        let bounds = row_bounds.clone();
        let tip = manage_tip.clone();
        div()
            .id(SharedString::from(format!("input-cap-{id:?}")))
            .flex_none()
            .h(px(14.))
            .px(px(3.))
            .rounded(px(2.))
            .font_family(theme::FONT_MONO)
            .text_size(px(theme::TEXT_TINY))
            .line_height(px(14.))
            .text_color(theme::TEXT_FAINT)
            .cursor_pointer()
            .hover(|style| style.text_color(theme::TEXT).bg(theme::SURFACE_4))
            .child(format!("{}/{}", ins.len(), cap))
            .tooltip(move |window, cx| {
                component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| open_input_popover(&this, id, &bounds, cx))
            .gpui_test_observe()
    });
    let button = if ins.is_empty() || multi {
        let tip = if ins.is_empty() {
            t!("Add input (up to %{max})", max = cap).into_owned()
        } else {
            manage_tip
        };
        let this = this.clone();
        let bounds = row_bounds.clone();
        ghost_button(
            SharedString::from(format!("input-add-{id:?}")),
            icon("icons/plus.svg"),
            tip,
            false,
            true,
            move |_, _, cx| open_input_popover(&this, id, &bounds, cx),
        )
    } else {
        // The single input's remove control: invisible until the pointer is
        // over the row, so the row reads as a plain label at rest.
        let edge_id = edge.expect("single-input row with its edge").id();
        let revealed = shell.mixer.presentation.hovered_route == Some((id, edge_id));
        let ports = edge.map(|edge| (edge.src_port(), edge.dst_port()));
        let this = this.clone();
        ghost_button(
            SharedString::from(format!("input-remove-{id:?}")),
            icon("icons/minus.svg"),
            t!("Remove this input").into_owned(),
            false,
            revealed,
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    // Deselect before the dispatch: the route is known here,
                    // gone by the time the publish lands.
                    if shell.selection.route == ports {
                        shell.selection.route = None;
                    }
                    shell.state.update(cx, |state, _| {
                        state.dispatch(AppRequest::EditGraph(GraphEdit::Disconnect(edge_id)));
                    });
                    shell.mixer.presentation.input_popover = None;
                    cx.notify();
                })
            },
        )
    };

    let row_id = edge
        .map(|edge| format!("input-{:?}", edge.id()))
        .unwrap_or_else(|| format!("input-empty-{id:?}"));
    let mut row = div()
        .id(SharedString::from(row_id))
        .relative()
        .flex()
        .items_center()
        .gap(px(theme::SPACE_1))
        .h(px(INPUT_ROW_H))
        .pl(px(theme::SPACE_2))
        .pr(px(theme::SPACE_1))
        .border_b_1()
        .border_color(theme::BORDER)
        .text_size(px(theme::TEXT_XS))
        .text_color(if muted {
            theme::TEXT_FAINT
        } else {
            theme::TEXT_MUTED
        })
        .hover(|style| style.bg(theme::controls::RAISED))
        .child(icon("icons/arrow-left.svg"))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(edge.is_none(), |name| name.text_color(theme::TEXT_FAINT))
                .child(name),
        );
    if let Some(chip) = chip {
        row = row.child(chip);
    }
    row = row.child(button);
    if let Some(edge) = edge {
        // The ports' display positions, off the nodes' own port lists.
        let input_port = graph
            .graph
            .get_node(id)
            .ok()
            .and_then(|node| {
                node.inputs()
                    .iter()
                    .position(|port| port.id() == edge.dst_port())
            })
            .unwrap_or(0)
            + 1;
        let source_port = graph
            .graph
            .get_node(edge.src())
            .ok()
            .and_then(|node| {
                node.outputs()
                    .iter()
                    .position(|port| port.id() == edge.src_port())
            })
            .unwrap_or(0)
            + 1;
        let edge_id = edge.id();
        row = row
            .when(selected, |row| {
                row.bg(theme::soft(theme::controls::PRIMARY))
                    .child(accent_bar())
            })
            .tooltip({
                let text = t!(
                    "%{name} · out %{source_port} → %{target} · in %{port}",
                    name = graph
                        .node(edge.src())
                        .map(crate::ui::presentation::display_label)
                        .unwrap_or_else(|| t!("Unknown").into_owned()),
                    source_port = source_port,
                    target = entry.label,
                    port = input_port,
                )
                .into_owned();
                move |window, cx| component::tooltip::Tooltip::new(text.clone()).build(window, cx)
            })
            .on_hover(track_hover(&this, (id, edge_id)))
            .on_click(route_click(this, (edge.src_port(), edge.dst_port())));
    }
    row.child(
        canvas(move |bounds, _, _| row_bounds.set(bounds), |_, _, _, _| {})
            .absolute()
            .inset_0(),
    )
    .gpui_test_observe()
}

/// The Sends block: the uniform head (label left, global fold chevron right)
/// and, when not globally collapsed, the fixed-height send list.
fn sends_block(shell: &StudioShell, id: NodeId, cx: &Context<StudioShell>) -> Div {
    let collapsed = shell.mixer.presentation.sends_collapsed;
    let this = cx.entity();
    let head = div()
        .id(SharedString::from(format!("sends-head-{id:?}")))
        .flex()
        .items_center()
        .justify_between()
        .h(px(SENDS_HEAD_H))
        // 10 px left sits the label on the rows' text inset; 4 px right puts
        // the chevron's stroke on the ghost buttons' icon column.
        .pl(px(10.))
        .pr(px(theme::SPACE_1))
        .text_color(theme::TEXT_FAINT)
        .cursor_pointer()
        .hover(|style| style.text_color(theme::TEXT))
        .child(
            div()
                .text_size(px(theme::TEXT_CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .child(t!("SENDS").to_string()),
        )
        .child(
            div()
                .size(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .child(if collapsed {
                    icon("icons/chevron-right.svg")
                } else {
                    icon("icons/chevron-down.svg")
                }),
        )
        .tooltip(|window, cx| {
            component::tooltip::Tooltip::new(
                t!("Collapse or expand the sends of every strip").into_owned(),
            )
            .build(window, cx)
        })
        .on_click(move |_, _, cx| {
            this.update(cx, |shell, cx| {
                shell.mixer.presentation.sends_collapsed =
                    !shell.mixer.presentation.sends_collapsed;
                cx.notify();
            })
        })
        .gpui_test_observe();
    let block = div()
        .flex()
        .flex_col()
        .flex_none()
        .pb(px(IO_BLOCK_PB))
        .border_b_1()
        .border_color(theme::BORDER)
        .child(head);
    if collapsed {
        return block;
    }
    let scrolls = &shell.mixer.presentation.row_scrolls[&id];
    let this = cx.entity();
    let graph = &shell.state.read(cx).snapshot.graph;
    let mut rows = div()
        .id(SharedString::from(format!("send-rows-{id:?}")))
        .flex()
        .flex_col()
        .w_full()
        .h_full()
        .pt(px(2.))
        .track_scroll(&scrolls.send)
        .restrict_scroll_to_axis()
        .overflow_y_scroll()
        .on_scroll_wheel(notify_owner(this));
    let mut count = 0;
    for edge in graph.graph.edges().iter().filter(|edge| edge.src() == id) {
        count += 1;
        rows = rows.child(send_slot(shell, edge, cx));
    }
    if count == 0 {
        rows = rows.child(
            label(t!("No sends"))
                .pl(px(10.))
                .pt(px(theme::SPACE_0_5))
                .text_size(px(theme::TEXT_XS)),
        );
    }
    block.child(
        scroll_region(
            SharedString::from(format!("scrollbar-send-rows-{id:?}")),
            &scrolls.send,
            ScrollbarAxis::Vertical,
            rows,
        )
        // The page-wide constant: empty strips reserve the same list height,
        // which is what keeps every later row of every strip aligned.
        .flex_none()
        .h(px(SEND_LIST_H)),
    )
}

/// One send: the row (arrow-as-mute, target, hover-revealed dB, PRE/POST)
/// with the level bar flush under it.
fn send_slot(
    shell: &StudioShell,
    edge: &moiren_core::graph::Edge,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let id = edge.id();
    let params = *edge.params();
    let pre = params.tap == SendTap::PreFader;
    let strip_id = edge.src();
    let target = edge.dst();
    let selected = shell.selection.route == Some((edge.src_port(), edge.dst_port()));
    let this = cx.entity();
    let graph = &shell.state.read(cx).snapshot.graph;
    let value = crate::ui::presentation::db_text_bare(params.gain);
    let editing = shell.mixer.presentation.editing_send == Some(id);
    let revealed = shell.mixer.presentation.hovered_route == Some((strip_id, id));

    // The row's leading arrow is the send's enable/mute toggle: lit = sending,
    // dimmed = muted. Same 18 px ghost hit box as the PRE/POST button.
    let (arrow_color, arrow_opacity) = if params.mute {
        (theme::TEXT_FAINT, 0.55)
    } else {
        (theme::controls::PRIMARY, 1.)
    };
    let arrow_tip = if params.mute {
        t!("Send muted — click to enable; the gain is preserved").into_owned()
    } else {
        t!("Send enabled — click to mute; the gain is preserved").into_owned()
    };
    let mute = send_edit(this.clone(), id, |params| params.mute = !params.mute);
    let tap = send_edit(this.clone(), id, |params| {
        params.tap = if params.tap == SendTap::PreFader {
            SendTap::PostFader
        } else {
            SendTap::PreFader
        };
    });

    let mut row = div()
        .id(SharedString::from(format!("send-row-{id:?}")))
        .flex()
        .items_center()
        .gap(px(theme::SPACE_1))
        .h(px(SEND_ROW_H))
        .pl(px(theme::SPACE_2))
        .pr(px(theme::SPACE_1_5))
        .text_size(px(theme::TEXT_XS))
        .text_color(if params.mute {
            theme::TEXT_FAINT
        } else {
            theme::TEXT_MUTED
        })
        .cursor_pointer()
        .hover(|style| style.bg(theme::controls::RAISED))
        .on_hover(track_hover(&this, (strip_id, id)))
        .child(
            div()
                .id(SharedString::from(format!("send-mute-{id:?}")))
                .flex_none()
                .size(px(18.))
                .ml(px(-2.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .text_color(arrow_color)
                .opacity(arrow_opacity)
                .cursor_pointer()
                .hover(|style| style.bg(theme::SURFACE_4).opacity(1.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(icon("icons/arrow-right.svg"))
                .tooltip(move |window, cx| {
                    component::tooltip::Tooltip::new(arrow_tip.clone()).build(window, cx)
                })
                .on_click(mute)
                .gpui_test_observe(),
        )
        .child(
            div().flex_1().min_w_0().truncate().child(
                graph
                    .node(target)
                    .map(crate::ui::presentation::display_label)
                    .unwrap_or_default(),
            ),
        );
    // The dB readout: hover-revealed (the row's width belongs to the target
    // name at rest), click-to-type through the shared underline editor.
    if editing {
        row = row.child(
            div()
                .id(SharedString::from(format!("send-value-{id:?}")))
                .flex_none()
                .w(px(30.))
                .font_family(theme::FONT_MONO)
                .child(underline_editor(
                    &shell.mixer.presentation.send_inputs[&id],
                    px(9.),
                    TextAlign::Right,
                )),
        );
    } else {
        row = row.child(
            div()
                .id(SharedString::from(format!("send-value-{id:?}")))
                .flex_none()
                .w(px(30.))
                .text_right()
                .font_family(theme::FONT_MONO)
                .text_size(px(theme::TEXT_TINY))
                .text_color(if selected {
                    theme::TEXT_MUTED
                } else {
                    theme::TEXT_FAINT
                })
                .opacity(if revealed { 1. } else { 0. })
                .cursor_pointer()
                .child(value)
                .tooltip(|window, cx| {
                    component::tooltip::Tooltip::new(
                        t!("Click to enter the send level (−60 … +24 dB)").into_owned(),
                    )
                    .build(window, cx)
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(start_send_edit(this.clone(), id))
                .gpui_test_observe(),
        );
    }
    row = row
        .child(
            ghost_button(
                SharedString::from(format!("send-pre-{id:?}")),
                icon("icons/fader-scale.svg"),
                t!("PRE: after channel mode, balance and mute; before gain and Dim. POST: after gain and Dim.")
                    .into_owned(),
                pre,
                true,
                tap,
            ),
        );

    div()
        .flex()
        .flex_col()
        .flex_none()
        .child(
            row.when(selected, |row| {
                row.bg(theme::soft(theme::controls::PRIMARY))
                    .child(accent_bar())
            })
            .on_click(route_click(this, (edge.src_port(), edge.dst_port())))
            .gpui_test_observe(),
        )
        .child(
            div()
                .id(SharedString::from(format!("send-track-{id:?}")))
                // Flush under the row: no gap above the bar. The bar itself
                // carries the 4 px below as hit area.
                .child(send_bar(
                    &shell.mixer.presentation.sends[&id].state,
                    &shell.mixer.presentation.sends[&id].behavior,
                    params.mute,
                    cx,
                ))
                .gpui_test_observe(),
        )
        .into_any_element()
}

/// Click-to-type on the send readout: the editor's text was already synced
/// (the sync stands down only while `editing_send` names this edge), so
/// opening is focus + flag.
fn start_send_edit(
    this: Entity<StudioShell>,
    id: EdgeId,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, window, cx| {
        this.update(cx, |shell, cx| {
            if let Some(input) = shell.mixer.presentation.send_inputs.get(&id) {
                input.read(cx).focus_handle(cx).focus(window, cx);
                shell.mixer.presentation.editing_send = Some(id);
                cx.notify();
            }
        });
    }
}

// --- The input dropdown ------------------------------------------------------

/// The strip's open input dropdown, mounted over the whole page: an ordinary
/// form multi-select. One checkbox per candidate node — checked = connected,
/// unchecked = not — toggling connects/disconnects at once and the list
/// stays open for the next pick; once the node's input cap is reached every
/// unchecked row is visibly unavailable. Outside press or Escape closes it,
/// like every shell-root popover.
pub(crate) fn input_popover_overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let this = cx.entity();
    let Some(pop) = shell.mixer.presentation.input_popover else {
        return div().into_any_element();
    };
    let graph = shell.state.read(cx).snapshot.graph.clone();
    let Ok(node) = graph.graph.get_node(pop.node) else {
        return div().into_any_element();
    };
    let cap = node.inputs().len();
    let ins: Vec<_> = graph
        .graph
        .edges()
        .iter()
        .filter(|edge| edge.dst() == pop.node)
        .collect();
    let full = ins.len() >= cap;
    let mut count = 0usize;
    let mut rows: Vec<AnyElement> = Vec::new();
    for candidate in &shell.mixer.presentation.order {
        if *candidate == pop.node {
            continue;
        }
        // A node without outputs cannot feed anything: not a candidate, so
        // the list only ever offers legal connections.
        let Ok(candidate_node) = graph.graph.get_node(*candidate) else {
            continue;
        };
        if candidate_node.outputs().is_empty() {
            continue;
        }
        count += 1;
        rows.push(pop_row(&graph, pop.node, cap, full, *candidate, cx).into_any_element());
    }
    let height = (count as f32 * POPOVER_ROW_H + POPOVER_CHROME_H).min(POPOVER_MAX_H);
    let placed =
        anchored_popover::under_anchor(pop.anchor, POPOVER_WIDTH, height, window.viewport_size());
    let close = this.clone();
    let head = div()
        .flex()
        .items_center()
        .justify_between()
        .pl(px(10.))
        .pr(px(10.))
        .pt(px(theme::SPACE_1))
        .pb(px(theme::SPACE_1))
        .text_size(px(theme::TEXT_XS))
        .text_color(theme::TEXT_FAINT)
        .child(t!("Choose inputs").to_string())
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .text_size(px(theme::TEXT_TINY))
                .text_color(if full {
                    theme::WARNING
                } else {
                    theme::TEXT_FAINT
                })
                .child(format!("{}/{}", ins.len(), cap)),
        );
    occluding_page(
        "mixer-input-popover-page",
        true,
        move |cx| {
            close.update(cx, |shell, cx| {
                shell.mixer.presentation.input_popover = None;
                cx.notify();
            })
        },
        claim_presses(
            div()
                .id(SharedString::from(format!("input-popover-{:?}", pop.node)))
                .absolute()
                .left(px(placed.left))
                .top(px(placed.top))
                .w(px(POPOVER_WIDTH))
                .max_h(px(height))
                .flex()
                .flex_col()
                .rounded(px(theme::RADIUS_MD))
                .bg(theme::controls::RAISED)
                .border_1()
                .border_color(theme::BORDER)
                .overflow_hidden()
                .child(head)
                .child(
                    scroll_region(
                        "scrollbar-mixer-input-popover",
                        &shell.mixer.presentation.input_popover_scroll,
                        ScrollbarAxis::Vertical,
                        div()
                            .id("input-popover-scroll")
                            .max_h(px(height))
                            .flex()
                            .flex_col()
                            .track_scroll(&shell.mixer.presentation.input_popover_scroll)
                            .restrict_scroll_to_axis()
                            .overflow_y_scroll()
                            .children(rows),
                    )
                    .max_h(px(height)),
                )
                .gpui_test_observe(),
        ),
    )
}

/// One dropdown row: checkbox + candidate name + the connected input port.
/// A row's click is its checkbox's click — the checkbox paints the state,
/// the row is its hit area, exactly like a form's label-wrapped control.
fn pop_row(
    graph: &crate::service::GraphSnapshot,
    node: NodeId,
    cap: usize,
    full: bool,
    candidate: NodeId,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let this = cx.entity();
    let existing: Vec<_> = graph
        .graph
        .edges()
        .iter()
        .filter(|edge| edge.src() == candidate && edge.dst() == node)
        .collect();
    let connected = !existing.is_empty();
    let disabled = !connected && full;
    let name = graph
        .node(candidate)
        .map(crate::ui::presentation::display_label)
        .unwrap_or_default();
    // The connected input's display position, for the trailing port label.
    let port = existing.first().and_then(|edge| {
        graph
            .graph
            .get_node(node)
            .ok()
            .and_then(|target| {
                target
                    .inputs()
                    .iter()
                    .position(|port| port.id() == edge.dst_port())
            })
            .map(|at| at + 1)
    });
    let tip = if connected {
        t!("Connected — uncheck to disconnect")
    } else if disabled {
        t!(
            "Input limit reached (%{max}) — release one first",
            max = cap
        )
    } else {
        t!("Check to connect to the next free input")
    };
    let tip = tip.into_owned();
    let row = div()
        .id(SharedString::from(format!("input-pop-row-{candidate:?}")))
        .flex()
        .items_center()
        .gap(px(theme::SPACE_1_5))
        .h(px(POPOVER_ROW_H))
        .px(px(10.))
        .text_size(px(theme::TEXT_XS))
        .text_color(if connected {
            theme::TEXT
        } else {
            theme::TEXT_MUTED
        })
        .when(disabled, |row| row.opacity(0.4))
        .when(!disabled, |row| {
            row.cursor_pointer()
                .hover(|style| style.bg(theme::SURFACE_4))
        })
        .child(
            // The checkbox: accent-filled with the check glyph when wired,
            // an empty bordered box when not.
            div()
                .flex_none()
                .size(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(2.))
                .border_1()
                .border_color(if connected {
                    theme::controls::PRIMARY
                } else {
                    theme::BORDER_STRONG
                })
                .bg(if connected {
                    theme::controls::PRIMARY
                } else {
                    theme::WINDOW_BG
                })
                .text_color(theme::WINDOW_BG)
                .when(connected, |box_| {
                    box_.child(
                        Icon::default()
                            .path("icons/check.svg")
                            .with_size(Size::Size(px(10.))),
                    )
                }),
        )
        .child(div().flex_1().min_w_0().truncate().child(name))
        .when_some(port, |row, port| {
            row.child(
                div()
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(theme::TEXT_TINY))
                    .text_color(theme::TEXT_FAINT)
                    .child(t!("In %{port}", port = port).to_string()),
            )
        })
        .tooltip(move |window, cx| component::tooltip::Tooltip::new(tip.clone()).build(window, cx));
    if disabled {
        // At the cap the unchecked row is display-only: the click handler is
        // not attached at all, so the row neither fires nor lies about it.
        row.gpui_test_observe()
    } else {
        row.on_click(move |_, _, cx| toggle_connect(&this, node, candidate, cx))
            .gpui_test_observe()
    }
}

/// One checkbox toggle on the dropdown: connect to the lowest free input
/// port, or disconnect every edge this candidate feeds the node with. The
/// list stays open for the next pick.
fn toggle_connect(this: &Entity<StudioShell>, node: NodeId, candidate: NodeId, cx: &mut App) {
    this.update(cx, |shell, cx| {
        let graph = shell.state.read(cx).snapshot.graph.clone();
        let removed: Vec<_> = graph
            .graph
            .edges()
            .iter()
            .filter(|edge| edge.src() == candidate && edge.dst() == node)
            .map(|edge| ((edge.src_port(), edge.dst_port()), edge.id()))
            .collect();
        if !removed.is_empty() {
            let routes: Vec<_> = removed.iter().map(|(ports, _)| *ports).collect();
            for (_, id) in removed {
                shell.state.update(cx, |state, _| {
                    state.dispatch(AppRequest::EditGraph(GraphEdit::Disconnect(id)));
                });
            }
            if shell
                .selection
                .route
                .is_some_and(|route| routes.contains(&route))
            {
                shell.selection.route = None;
            }
        } else {
            // Connect to the node's lowest free input port from the
            // candidate's first output; each input port is
            // single-connection, so a free port is one no edge lands on.
            let used: std::collections::HashSet<_> = graph
                .graph
                .edges()
                .iter()
                .filter(|edge| edge.dst() == node)
                .map(|edge| edge.dst_port())
                .collect();
            let free = graph.graph.get_node(node).ok().and_then(|target| {
                target
                    .inputs()
                    .iter()
                    .map(|port| port.id())
                    .find(|port| !used.contains(port))
            });
            let source = graph
                .graph
                .get_node(candidate)
                .ok()
                .and_then(|node| node.outputs().first().map(|port| port.id()));
            if let (Some(source), Some(destination)) = (source, free) {
                shell.state.update(cx, |state, _| {
                    state.dispatch(AppRequest::EditGraph(GraphEdit::Connect {
                        source,
                        destination,
                    }));
                });
            }
        }
        // The dropdown stays open (the live-apply multi-select); the next
        // render re-reads the graph and repaints the checkboxes.
        cx.notify();
    });
}
