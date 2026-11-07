use rust_i18n::t;

use super::*;
use crate::service::{GraphEdit, InputSelection, RequestId};
use crate::ui::widgets::overlay_scrollbar::overlay_scrollbar;
use gpui_kit::base::ScrollbarAxis;
use pages::node_catalog::NodeTemplate;

#[derive(Clone)]
pub(in crate::ui::window) struct PendingNodeSpawn {
    request: RequestId,
    previous: Vec<moiren_core::graph::NodeId>,
    position: (f32, f32),
    template: NodeTemplate,
}

impl Render for NodeTemplate {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::ui::pages::graph_canvas_view::node_drag_preview(*self)
    }
}

pub(in crate::ui::window) fn library(
    shell: &StudioShell,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    use controls::*;
    let state = shell.state.read(cx);
    let query = shell.node_search.read(cx).value();
    let mut items = div().id("node-library-items").flex().flex_col().gap(px(4.));
    let mut group = "";
    let mut count = 0;
    for template in NodeTemplate::ALL
        .into_iter()
        .filter(|template| template.matches(&query))
    {
        count += 1;
        if group != template.group() {
            group = template.group();
            items = items.child(div().mt(px(14.)).mb(px(2.)).child(label(t!(group))));
        }
        let enabled = template.kind().is_some() && shell.pending_node_spawn.is_none();
        let this = cx.entity();
        let tint = template.tint();
        // One card per template: the tinted icon tile, title and description
        // share a single surface that is itself the click and drag target, so
        // hover, pointer and drop geometry all agree on the same bounds.
        let card = div()
            .id(SharedString::from(format!("add-{}", template.title())))
            .w_full()
            .flex()
            .items_center()
            .gap(px(theme::SPACE_2))
            .px(px(theme::SPACE_2))
            .py(px(6.))
            .rounded(px(theme::RADIUS_SM))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded(px(theme::RADIUS_SM))
                    .bg(if enabled {
                        theme::soft(tint)
                    } else {
                        theme::soft(theme::TEXT_FAINT)
                    })
                    .child(
                        component::Icon::default()
                            .path(format!("icons/{}.svg", template.icon()))
                            .size(px(15.))
                            .text_color(if enabled { tint } else { theme::TEXT_FAINT }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(theme::TEXT_SM))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if enabled {
                                theme::TEXT
                            } else {
                                theme::TEXT_FAINT
                            })
                            .child(t!(template.title())),
                    )
                    .child(
                        div()
                            .text_size(px(theme::TEXT_XS))
                            .text_color(theme::TEXT_FAINT)
                            .child(t!(template.description())),
                    ),
            )
            .when(enabled, |card| {
                card.cursor_pointer()
                    .hover(|style| style.bg(theme::SURFACE_3))
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.spawn_template_at_center(template, cx);
                        });
                    })
                    .on_drag(template, |template, _, _, cx| cx.new(|_| *template))
            })
            .gpui_test_observe();
        // The harness addresses drop origins by `library-{template:?}` while
        // the card answers to `add-{title}`; the wrapper carries the former
        // at exactly the card's bounds.
        items = items.child(
            div()
                .id(SharedString::from(format!("library-{template:?}")))
                .w_full()
                .child(card)
                .gpui_test_observe(),
        );
    }
    if count == 0 {
        items = items.child(label(t!("No matching nodes")));
    }
    let mut session = div()
        .id("session-node-list")
        .flex()
        .flex_col()
        .gap(px(4.))
        .mt(px(16.))
        .pt(px(12.))
        .border_t_1()
        .border_color(theme::BORDER)
        .child(heading(t!(
            "Session nodes · %{count}",
            count = state.snapshot.graph.nodes.len()
        )));
    for entry in &state.snapshot.graph.nodes {
        let id = entry.id;
        let selected = shell.selected_node == Some(id);
        let this = cx.entity();
        // The row wrapper carries the selected fact (`aria_selected`) for
        // tests and assistive tech; the button inside keeps the `locate-` id
        // it is addressed by. The selected variant below is pure paint, which
        // no element snapshot can see without this marker.
        session = session.child(
            div()
                .id(SharedString::from(format!("locate-row-{id:?}")))
                .w_full()
                .aria_selected(selected)
                .gpui_test_observe()
                .child(
                    button(
                        SharedString::from(format!("locate-{id:?}")),
                        crate::ui::presentation::display_label(entry),
                    )
                    .variant(if selected {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.selected_node = Some(id);
                            shell.selected_route = None;
                            // Locating selects; a double click also opens the
                            // inspector, matching canvas node behavior.
                            if shell.element_double_clicked(StudioShell::element_key("node", id)) {
                                shell.open_inspector();
                            }
                            cx.notify();
                        });
                    }),
                ),
        );
    }
    div()
        .id("session-io-panel")
        .flex()
        .flex_col()
        .flex_none()
        .h_full()
        .w(px(theme::LIBRARY_WIDTH))
        .bg(theme::SURFACE)
        .border_r_1()
        .border_color(theme::BORDER)
        .child(
            div()
                .p(px(12.))
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(heading(t!("Node library")))
                .child(
                    component::input::Input::new(&shell.node_search)
                        .id("node-search")
                        .bg(theme::SURFACE_3)
                        .text_color(theme::TEXT)
                        .border_color(theme::BORDER),
                ),
        )
        .child(
            // The bar overlay is a sibling of the scroll container, never a
            // child: the container translates its children by the offset at
            // prepaint, so an overlay inside it would drift away with the
            // content. The container scrolls inside the cached panel, so its
            // offset rides LibraryKey.
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .id("node-library-scroll")
                        .w_full()
                        .h_full()
                        .px(px(12.))
                        .track_scroll(&shell.library_scroll)
                        .overflow_y_scroll()
                        .on_scroll_wheel({
                            let this = cx.entity();
                            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
                        })
                        .child(items)
                        .child(session)
                        .gpui_test_observe(),
                )
                .child(overlay_scrollbar(
                    &shell.library_scroll,
                    "scrollbar-node-library-scroll",
                    ScrollbarAxis::Vertical,
                )),
        )
        .child(
            div()
                .p(px(12.))
                .flex_none()
                .border_t_1()
                .border_color(theme::BORDER)
                .child(label(t!("Drag a node onto the graph")))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(theme::TEXT_FAINT)
                        .child(t!("Or click to add at the canvas center.")),
                ),
        )
        .gpui_test_observe()
}

/// Collapsed library form: one icon per template in a constant rail beside
/// the detail sleeve. Clicking spawns at the canvas center and dragging pulls
/// the template onto the canvas, exactly like a full row.
pub(in crate::ui::window) fn library_rail(
    shell: &StudioShell,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let this = cx.entity();
    let mut rail = div()
        .id("library-rail")
        .w_full()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(2.))
        .py(px(10.))
        .track_scroll(&shell.library_rail_scroll)
        .overflow_y_scroll()
        .on_scroll_wheel({
            let this = this.clone();
            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
        });
    for template in NodeTemplate::ALL {
        let enabled = shell.pending_node_spawn.is_none();
        let this = this.clone();
        rail = rail.child(
            div()
                .id(SharedString::from(format!("rail-{template:?}")))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(32.))
                .h(px(32.))
                .rounded(px(theme::RADIUS_SM))
                .child(
                    component::Icon::default()
                        .path(format!("icons/{}.svg", template.icon()))
                        .size(px(18.))
                        .text_color(if enabled {
                            template.tint().opacity(0.7)
                        } else {
                            theme::TEXT_FAINT
                        }),
                )
                .when(enabled, |icon| {
                    icon.cursor_pointer()
                        .hover(|style| style.bg(theme::SURFACE_4))
                        .on_click(move |_, _, cx| {
                            this.update(cx, |shell, cx| {
                                shell.spawn_template_at_center(template, cx);
                            });
                        })
                        .on_drag(template, |template, _, _, cx| cx.new(|_| *template))
                })
                .gpui_test_observe(),
        );
    }
    // The rail's outer chrome (width, right border) lives on the relative
    // host so the bar overlay can pin there as the scroll container's
    // sibling; the offset rides LibraryKey inside the cached panel.
    div()
        .relative()
        .flex_none()
        .w(px(theme::LIBRARY_RAIL_WIDTH))
        .h_full()
        .bg(theme::SURFACE)
        .border_r_1()
        .border_color(theme::HAIRLINE)
        .child(rail.gpui_test_observe())
        .child(overlay_scrollbar(
            &shell.library_rail_scroll,
            "scrollbar-library-rail-scroll",
            ScrollbarAxis::Vertical,
        ))
}

impl StudioShell {
    fn spawn_template_at_center(&mut self, template: NodeTemplate, cx: &mut Context<Self>) {
        let center = self
            .canvas_size
            .map(|(width, height)| (width / 2., height / 2.))
            .unwrap_or((320., 220.));
        self.spawn_node(template, self.camera.canvas_to_world(center), cx);
    }

    pub(super) fn spawn_node(
        &mut self,
        template: NodeTemplate,
        position: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = template.kind() else {
            return;
        };
        if self.pending_node_spawn.is_some() || !position.0.is_finite() || !position.1.is_finite() {
            return;
        }
        let previous = self
            .state
            .read(cx)
            .snapshot
            .graph
            .nodes
            .iter()
            .map(|entry| entry.id)
            .collect();
        if let Ok(request) = self.state.update(cx, |state, _| {
            state.try_dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind,
                channels: 2,
            }))
        }) {
            self.camera.adjusted = true;
            self.pending_node_spawn = Some(PendingNodeSpawn {
                request,
                previous,
                position,
                template,
            });
        }
        cx.notify();
    }

    pub(in crate::ui::window) fn finish_node_spawn(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = &self.pending_node_spawn else {
            return;
        };
        let snapshot = self.state.read(cx).snapshot.clone();
        if snapshot.consumed_request_id < pending.request {
            return;
        }
        let pending = self
            .pending_node_spawn
            .take()
            .expect("checked pending creation");
        if snapshot.graph.last_error.is_some() {
            return;
        }
        let created: Vec<_> = snapshot
            .graph
            .nodes
            .iter()
            .filter(|entry| !pending.previous.contains(&entry.id))
            .collect();
        let [entry] = created.as_slice() else {
            return;
        };
        let id = entry.id;
        self.node_positions.insert(id, pending.position);
        // The spawned node is the whole selection: anything selected before
        // must not ride along with the drag that usually follows a spawn.
        self.selected_nodes.clear();
        self.selected_nodes.insert(id);
        self.selected_node = Some(id);
        self.selected_route = None;
        self.open_inspector();
        let source = match pending.template {
            NodeTemplate::Tone => Some(InputSelection::Tone { frequency_hz: 440. }),
            _ => None,
        };
        // Input/Output templates keep the default label so their card name
        // follows the bound device; other kinds pin the template title.
        let follows_binding = matches!(
            pending.template,
            NodeTemplate::Input
                | NodeTemplate::Application
                | NodeTemplate::Tone
                | NodeTemplate::Output
        );
        self.state.update(cx, |state, _| {
            if !follows_binding {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetLabel {
                    node: id,
                    label: pending.template.title().into(),
                }));
            }
            if let Some(source) = source {
                state.dispatch(AppRequest::EditGraph(GraphEdit::BindSource {
                    node: id,
                    source,
                }));
            }
        });
    }
}
