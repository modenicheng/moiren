use super::*;
use crate::service::{GraphEdit, InputSelection, RequestId};
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
        div()
            .w(px(theme::NODE_WIDTH))
            .p(px(12.))
            .rounded(px(theme::RADIUS_MD))
            .bg(theme::SURFACE_3)
            .border_1()
            .border_color(theme::BORDER)
            .text_color(theme::TEXT)
            .child(self.title())
    }
}

pub(super) fn library(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
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
            items = items.child(div().mt(px(14.)).mb(px(4.)).child(label(group)));
        }
        let enabled = template.kind().is_some() && shell.pending_node_spawn.is_none();
        let this = cx.entity();
        let row = div()
            .id(SharedString::from(format!("library-{template:?}")))
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(8.))
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        button(
                            SharedString::from(format!("add-{}", template.title())),
                            template.title(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .disabled(!enabled)
                        .on_click(move |_, _, cx| {
                            this.update(cx, |shell, cx| {
                                let center = shell
                                    .canvas_size
                                    .map(|(width, height)| (width / 2., height / 2.))
                                    .unwrap_or((320., 220.));
                                shell.spawn_node(
                                    template,
                                    shell.camera.canvas_to_world(center),
                                    cx,
                                );
                            });
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(theme::TEXT_FAINT)
                            .child(template.description()),
                    ),
            )
            .when(enabled, |row| {
                row.cursor_move()
                    .on_drag(template, |template, _, _, cx| cx.new(|_| *template))
            })
            .gpui_test_observe();
        items = items.child(row);
    }
    if count == 0 {
        items = items.child(label("No matching nodes"));
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
        .child(heading(format!(
            "Session nodes · {}",
            state.snapshot.graph.nodes.len()
        )));
    for entry in &state.snapshot.graph.nodes {
        let id = entry.id;
        let selected = shell.selected_node == Some(id);
        let this = cx.entity();
        session = session.child(
            button(
                SharedString::from(format!("locate-{id:?}")),
                entry.label.clone(),
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
                    shell.inspector_open = true;
                    if shell.compact {
                        shell.sidebar_open = false;
                    }
                    cx.notify();
                });
            }),
        );
    }
    div()
        .id("session-io-panel")
        .flex()
        .flex_col()
        .flex_none()
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
                .child(heading("Node library"))
                .child(
                    component::input::Input::new(&shell.node_search)
                        .id("node-search")
                        .bg(theme::SURFACE_3)
                        .text_color(theme::TEXT)
                        .border_color(theme::BORDER),
                ),
        )
        .child(
            div()
                .id("node-library-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(12.))
                .child(items)
                .child(session),
        )
        .child(
            div()
                .p(px(12.))
                .flex_none()
                .border_t_1()
                .border_color(theme::BORDER)
                .child(label("Drag a node onto the graph"))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(theme::TEXT_FAINT)
                        .child("Or click to add at the canvas center."),
                ),
        )
        .gpui_test_observe()
}

impl StudioShell {
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
        self.selected_node = Some(id);
        self.selected_route = None;
        self.inspector_open = true;
        if self.compact {
            self.sidebar_open = false;
        }
        let source = match pending.template {
            NodeTemplate::Tone => Some(InputSelection::Tone { frequency_hz: 440. }),
            _ => None,
        };
        self.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::SetLabel {
                node: id,
                label: pending.template.title().into(),
            }));
            if let Some(source) = source {
                state.dispatch(AppRequest::EditGraph(GraphEdit::BindSource {
                    node: id,
                    source,
                }));
            }
        });
    }
}
