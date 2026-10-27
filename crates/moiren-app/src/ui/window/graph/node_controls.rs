//! Native, task-specific controls over the graph's shared card geometry.
//! Only presentation state lives here; all edits go through the service.
use super::*;
use crate::service::{GraphEdit, GraphNodeState, InputSelection};
use crate::ui::pages::graph_canvas::{CanvasNode, CanvasWorld};
use crate::ui::widgets::fader::{SliderBehavior, hslider};
use crate::ui::widgets::meters::compact_horizontal_meter;
use crate::ui::window::mixer::{SliderEdit, SliderSlot, commit_closure, sync_slider_range};
use moiren_core::graph::{NodeId, NodeKind};
use std::{collections::BTreeMap, rc::Rc};

mod fields;
mod io;
mod parameter;
mod plots;
mod rack_editor;
mod recording;
mod rotary;
#[cfg(all(test, feature = "gpui-test"))]
mod tests;
mod utility;
use fields::{parameter_control, parameter_row};
use io::io_body;
use parameter::Parameter;
pub(super) use rack_editor::inspector as rack_inspector;
pub(super) use recording::format_controls as recording_format_controls;
pub(super) use utility::spectrum::inspector as spectrum_inspector;

#[derive(Default)]
pub(in crate::ui::window) struct NodeControls {
    pub(in crate::ui::window) heights: BTreeMap<NodeId, f32>,
    sliders: BTreeMap<(NodeId, Parameter), SliderSlot>,
    inputs: BTreeMap<(NodeId, Parameter), Entity<component::input::InputState>>,
    input_baselines: BTreeMap<(NodeId, Parameter), String>,
    errors: BTreeMap<(NodeId, Parameter), String>,
    pub(super) resource_popup: Option<NodeId>,
    application_picker: BTreeMap<NodeId, bool>,
    rack: rack_editor::RackEditorState,
    spectrum: BTreeMap<NodeId, utility::spectrum::SpectrumView>,
    spectrum_editors: utility::spectrum::editor::Editors,
}

impl NodeControls {
    pub(in crate::ui::window) fn owns_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
        self.inputs
            .values()
            .any(|input| input.read(cx).focus_handle(cx) == *focused)
            || self.rack.owns_focus(focused, cx)
            || self.spectrum_editors.owns_focus(focused, cx)
    }
}

impl StudioShell {
    pub(in crate::ui::window) fn sync_node_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let graph = self.state.read(cx).snapshot.graph.clone();
        self.node_controls
            .heights
            .retain(|id, _| graph.node(*id).is_some());
        self.node_controls
            .spectrum
            .retain(|id, _| graph.kind_of(*id) == Some(NodeKind::SpectrumAnalyzer));
        self.node_controls
            .sliders
            .retain(|(id, _), _| graph.node(*id).is_some());
        self.node_controls
            .inputs
            .retain(|(id, _), _| graph.node(*id).is_some());
        self.node_controls
            .input_baselines
            .retain(|(id, _), _| graph.node(*id).is_some());
        self.node_controls
            .errors
            .retain(|(id, _), _| graph.node(*id).is_some());
        let mut rows = Vec::new();
        for node in &graph.nodes {
            let Some(kind) = graph.kind_of(node.id) else {
                continue;
            };
            for parameter in Parameter::primary(kind, node) {
                let id = node.id;
                let key = (id, parameter);
                let value = parameter.value(kind, node);
                let text = format!("{value:.2}");
                self.node_controls
                    .input_baselines
                    .entry(key)
                    .or_insert_with(|| text.clone());
                let input = self.node_controls.inputs.entry(key).or_insert_with(|| {
                    let input = cx.new(|cx| {
                        component::input::InputState::new(window, cx).default_value(text.clone())
                    });
                    cx.subscribe(
                        &input,
                        move |shell, input, event: &component::input::InputEvent, cx| {
                            if !matches!(
                                event,
                                component::input::InputEvent::PressEnter { .. }
                                    | component::input::InputEvent::Blur
                            ) {
                                return;
                            }
                            let graph = shell.state.read(cx).snapshot.graph.clone();
                            let Some(node) = graph.node(id) else { return };
                            let Some(kind) = graph.kind_of(id) else {
                                return;
                            };
                            let text = input.read(cx).value().to_string();
                            // A knob can change the graph while this field keeps
                            // focus. Only submit edits to the displayed text, not
                            // its difference from the newest graph snapshot.
                            if shell.node_controls.input_baselines.get(&key) == Some(&text) {
                                shell.node_controls.errors.remove(&key);
                                cx.notify();
                                return;
                            }
                            let value = parameter::parse(&text, parameter.unit());
                            let edit = value.and_then(|v| parameter.edit(v, kind, node));
                            if let Some(edit) = edit {
                                shell.node_controls.errors.remove(&key);
                                shell.node_controls.input_baselines.insert(key, text);
                                if value != Some(parameter.value(kind, node)) {
                                    shell
                                        .state
                                        .update(cx, |s, _| s.dispatch(AppRequest::EditGraph(edit)));
                                }
                            } else {
                                let (min, max) = parameter.range(node);
                                shell.node_controls.errors.insert(
                                    key,
                                    format!("{min:.1} .. {max:.1} {}", parameter.unit()),
                                );
                            }
                            cx.notify();
                        },
                    )
                    .detach();
                    input
                });
                if !input.read(cx).focus_handle(cx).is_focused(window)
                    && !self.node_controls.errors.contains_key(&key)
                    && self
                        .node_controls
                        .input_baselines
                        .get(&key)
                        .map(String::as_str)
                        == Some(input.read(cx).value().as_str())
                {
                    // A render may precede Blur delivery; leave dirty text for
                    // the subscription to validate and commit before syncing.
                    if input.read(cx).value().as_str() != text {
                        input.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                    }
                    self.node_controls.input_baselines.insert(key, text);
                }
                rows.push((
                    key,
                    parameter.position(kind, node),
                    Box::new(move |position, graph: &crate::service::GraphSnapshot| {
                        let node = graph.node(id)?;
                        let kind = graph.kind_of(id)?;
                        parameter
                            .edit(parameter.value_at_position(position, node), kind, node)
                            .map(AppRequest::EditGraph)
                    }) as SliderEdit,
                ));
            }
        }
        sync_slider_range(
            window,
            cx,
            &self.state,
            &mut self.node_controls.sliders,
            rows,
            [0., 1., 0.001],
            |(id, p), ui, edit, slider| SliderBehavior {
                step: Rc::new(|v, dir| (v + dir as f32 * 0.001).clamp(0., 1.)),
                reset: graph
                    .node(*id)
                    .and_then(|n| graph.kind_of(*id).map(|k| p.default_position(k, n)))
                    .unwrap_or(0.),
                commit: commit_closure(&ui, &edit, &slider),
            },
        );
        rack_editor::sync(self, window, cx);
        utility::spectrum::editor::sync(self, window, cx);
    }
}

/// Fixed-size native controls use semantic zoom instead of shrinking hit targets
/// into unreadable glyphs. Overview cards prioritize a reachable focus action.
pub(super) fn overlay(
    shell: &StudioShell,
    world: &CanvasWorld,
    cx: &Context<StudioShell>,
) -> Vec<(String, AnyElement)> {
    if (!shell.camera.adjusted && shell.canvas_size.is_none())
        || shell.hand_tool
        || shell.node_drag.is_some()
        || shell.connection_drag.is_some()
        || shell.pan_grab.is_some()
        || shell.selection_drag.is_some()
    {
        return Vec::new();
    }
    let snapshot = &shell.state.read(cx).snapshot;
    world
        .nodes
        .iter()
        .filter_map(|card| {
            let id = card.node_id?;
            let node = snapshot.graph.node(id)?;
            let rect = shell
                .camera
                .project_rect(pages::graph_canvas::NodeLayout::new(card).card);
            if let Some((w, h)) = shell.canvas_size
                && (rect.right() < 0. || rect.bottom() < 0. || rect.left() > w || rect.top() > h)
            {
                return None;
            }
            let compact = shell.camera.zoom < 0.85;
            let body = if compact {
                overview_body(shell, card, node, cx)
            } else {
                full_body(shell, card, node, cx)
            };
            let top = 58. * shell.camera.zoom;
            let owner = cx.entity().downgrade();
            let zoom = shell.camera.zoom;
            Some((
                card.id.clone(),
                div()
                    .id(format!("node-controls-{id:?}"))
                    .absolute()
                    .left(px(rect.origin.x + 10. * shell.camera.zoom))
                    .top(px(rect.origin.y + top))
                    .w(px((rect.size.width - 20. * shell.camera.zoom).max(1.)))
                    .when(compact, |view| {
                        view.h(px((rect.size.height - top - 10.).max(1.)))
                            .overflow_hidden()
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| {
                        if !cx.has_active_drag() {
                            cx.stop_propagation();
                        }
                    })
                    .on_key_down(|event, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "delete" | "backspace") {
                            cx.stop_propagation();
                        }
                    })
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .child(body)
                    .when(!compact, |view| {
                        view.child(
                            canvas(
                                |_, _, _| (),
                                move |bounds, _, _, cx| {
                                    // Measure an absolute sibling, not a stretch child:
                                    // the native content determines its own height.
                                    let height =
                                        (f32::from(bounds.size.height) / zoom + 68.).ceil();
                                    let owner = owner.clone();
                                    cx.defer(move |cx| {
                                        let _ = owner.update(cx, |shell, cx| {
                                            let old = shell
                                                .node_controls
                                                .heights
                                                .get(&id)
                                                .copied()
                                                .unwrap_or(0.);
                                            if (old - height).abs() >= 1. {
                                                shell.node_controls.heights.insert(id, height);
                                                cx.notify();
                                            }
                                        });
                                    });
                                },
                            )
                            .absolute()
                            .size_full(),
                        )
                    })
                    .gpui_test_observe()
                    .into_any_element(),
            ))
        })
        .collect()
}

fn full_body(
    shell: &StudioShell,
    card: &CanvasNode,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    if matches!(card.kind, NodeKind::Source | NodeKind::Sink)
        && !matches!(node.source, Some(InputSelection::Tone { .. }))
    {
        return io_body(shell, card, node, cx);
    }
    let snapshot = &shell.state.read(cx).snapshot;
    let mut body = div().flex().flex_col().gap(px(6.)).w_full();
    if snapshot.phase == SessionPhase::Running
        && snapshot.graph.active_revision != Some(snapshot.graph.candidate_revision)
    {
        body = body
            .child(note("Edits pending; meters show the running graph").text_color(theme::WARNING));
    }
    if let Some(plot) = plots::response(
        card.kind,
        node,
        snapshot.audio_settings.processing.sample_rate as f64,
        cx,
    ) {
        body = body.child(plot);
    }
    let parameters = Parameter::primary(card.kind, node);
    let tone = matches!(node.source, Some(InputSelection::Tone { .. }));
    let mut rotary_controls = div().flex().flex_wrap().gap(px(6.)).w_full();
    let mut has_rotary = false;
    for parameter in parameters {
        if parameter == Parameter::Level && !tone {
            body = body.child(parameter_row(shell, node, parameter, cx));
        } else {
            has_rotary = true;
            rotary_controls =
                rotary_controls.child(parameter_control(shell, node, parameter, true, cx));
        }
    }
    if has_rotary {
        body = body.child(rotary_controls);
    }
    match card.kind {
        NodeKind::Source => {
            body = body.child(note(
                if matches!(node.source, Some(InputSelection::Tone { .. })) {
                    "Sine generator · digital level"
                } else {
                    "Digital trim · post-fader meter"
                },
            ))
        }
        NodeKind::Pan => body = body.child(controls::stereo_controls(node, cx)),
        NodeKind::Recorder => body = body.child(recording::content(shell, node.id, cx)),
        _ => {}
    }
    if let Some(content) = utility::content(shell, card, node, cx) {
        body = body.child(content);
    }
    let this = cx.entity();
    let id = node.id;
    let mut actions = div().flex().items_center().gap(px(6.));
    if !matches!(
        card.kind,
        NodeKind::Recorder | NodeKind::Meter | NodeKind::SpectrumAnalyzer
    ) {
        let mut muted = node.mixer;
        muted.muted = !muted.muted;
        actions = actions.child(
            controls::button(format!("card-mute-{id:?}"), t!("Mute"))
                .variant(if node.mixer.muted {
                    ButtonVariant::Danger
                } else {
                    ButtonVariant::Secondary
                })
                .on_click(controls::edit(
                    this.clone(),
                    GraphEdit::SetMixer {
                        node: id,
                        state: muted,
                    },
                )),
        );
    }
    let side_meter = matches!(
        card.kind,
        NodeKind::Eq
            | NodeKind::Compressor
            | NodeKind::Gate
            | NodeKind::Pan
            | NodeKind::Bus
            | NodeKind::Recorder
            | NodeKind::SpectrumAnalyzer
    ) || (card.kind == NodeKind::Rack
        && node
            .builtin
            .rack()
            .is_some_and(|rack| !rack.slots.is_empty()));
    let content = div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .w_full()
        .child(body.id(format!("card-content-{id:?}")))
        .when(!side_meter && card.kind != NodeKind::Meter, |view| {
            view.child(div().flex_none().child(meter(shell, node, card, cx)))
        })
        .child(
            actions.flex_none().child(
                controls::button(format!("card-details-{id:?}"), t!("Details"))
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.selected_node = Some(id);
                            shell.selected_nodes.clear();
                            shell.selected_nodes.insert(id);
                            shell.selected_route = None;
                            shell.open_inspector();
                            cx.notify();
                        })
                    }),
            ),
        );
    if side_meter {
        let channels = snapshot
            .graph
            .graph
            .get_node(id)
            .map_or(2, |n| n.channels());
        let meter_width = crate::ui::widgets::meters::node_meter_width(channels);
        div()
            .relative()
            .w_full()
            .pr(px(meter_width + 16.))
            .child(content)
            .child(
                div()
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .w(px(meter_width))
                    .child(vertical_meter(shell, node, cx)),
            )
            .into_any_element()
    } else {
        content.into_any_element()
    }
}

fn overview_body(
    shell: &StudioShell,
    card: &CanvasNode,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let this = cx.entity();
    let id = node.id;
    let x = card.x;
    let y = card.y;
    let body_height = (card.height() - 58.) * shell.camera.zoom - 10.;
    let show_summary = body_height >= 100. && card.width() * shell.camera.zoom >= 160.;
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .w_full()
        .child(
            controls::button(format!("card-focus-{id:?}"), t!("Edit node"))
                .full_width()
                .on_click(move |_, _, cx| {
                    this.update(cx, |s, cx| {
                        // Lock the target's presentation position before a
                        // taller neighbor's first measurement reflows its column.
                        s.node_positions.insert(id, (x, y));
                        s.camera = Camera {
                            pan_x: 32. - x,
                            pan_y: 78. - y,
                            zoom: 1.,
                            adjusted: true,
                        };
                        s.selected_node = Some(id);
                        s.selected_nodes.clear();
                        s.selected_nodes.insert(id);
                        s.selected_route = None;
                        cx.notify();
                    })
                }),
        )
        .when(show_summary, |mut body| {
            let rows = ((body_height - 58.) / 20.).clamp(0., 3.) as usize;
            for parameter in Parameter::primary(card.kind, node).into_iter().take(rows) {
                body = body.child(detail_line(
                    parameter.label(),
                    &format!(
                        "{:.1} {}",
                        parameter.value(card.kind, node),
                        parameter.unit()
                    ),
                ));
            }
            let snapshot = &shell.state.read(cx).snapshot;
            let levels = (snapshot.phase == SessionPhase::Running
                && snapshot.signal_topics.node_levels.contains_key(&id)
                && shell.node_meters.status(id) == crate::ui::monitoring::MeterStatus::Live)
                .then(|| shell.node_meters.channel_levels(id))
                .flatten();
            let channels = snapshot
                .graph
                .graph
                .get_node(id)
                .map_or(2, |node| node.channels());
            // Overview stays an honest compact readout, not another tiny editor.
            body.child(compact_horizontal_meter(
                levels,
                channels,
                if levels.is_some() {
                    shell.node_meters.channel_clips(id)
                } else {
                    &[]
                },
            ))
        })
        .into_any_element()
}

fn vertical_meter(
    shell: &StudioShell,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let snapshot = &shell.state.read(cx).snapshot;
    let id = node.id;
    let channels = snapshot
        .graph
        .graph
        .get_node(id)
        .map_or(2, |n| n.channels());
    let levels = (snapshot.phase == SessionPhase::Running
        && snapshot.signal_topics.node_levels.contains_key(&id)
        && shell.node_meters.status(id) == crate::ui::monitoring::MeterStatus::Live)
        .then(|| shell.node_meters.channel_levels(id))
        .flatten();
    let this = cx.entity();
    div()
        .id(format!("card-meter-{id:?}"))
        .size_full()
        .tooltip(|window, cx| {
            component::tooltip::Tooltip::new(t!("Graph monitor · post-fader").to_string())
                .build(window, cx)
        })
        .child(crate::ui::widgets::meters::node_vertical_meter(
            levels,
            channels,
            if levels.is_some() {
                shell.node_meters.channel_clips(id)
            } else {
                &[]
            },
        ))
        .on_click(move |_, _, cx| {
            this.update(cx, |s, cx| {
                s.node_meters.reset_clip(id);
                cx.notify();
            })
        })
        .gpui_test_observe()
        .into_any_element()
}

fn meter(
    shell: &StudioShell,
    node: &GraphNodeState,
    card: &CanvasNode,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let id = node.id;
    let this = cx.entity();
    let snapshot = &shell.state.read(cx).snapshot;
    let channels = snapshot
        .graph
        .graph
        .get_node(id)
        .map_or(2, |n| n.channels());
    let levels = (snapshot.phase == SessionPhase::Running
        && snapshot.signal_topics.node_levels.contains_key(&id)
        && shell.node_meters.status(id) == crate::ui::monitoring::MeterStatus::Live)
        .then(|| shell.node_meters.channel_levels(id))
        .flatten();
    let peak = levels.map(|l| l.iter().map(|v| v.marker).fold(0f32, f32::max));
    let text = peak.map_or_else(
        || t!("No live signal").to_string(),
        |p| {
            if p <= 0. {
                "-inf dBFS".into()
            } else {
                format!("{:.1} dBFS", 20. * p.log10())
            }
        },
    );
    div()
        .id(format!("card-meter-{id:?}"))
        .flex()
        .flex_col()
        .gap(px(5.))
        .py(px(5.))
        .w_full()
        .child(
            div()
                .flex()
                .justify_between()
                .child(note(if card.kind == NodeKind::Recorder {
                    "Graph monitor · post-fader"
                } else {
                    "Post-fader · Peak / RMS"
                }))
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(10.))
                        .child(text),
                ),
        )
        .child(compact_horizontal_meter(
            levels,
            channels,
            if levels.is_some() {
                shell.node_meters.channel_clips(id)
            } else {
                &[]
            },
        ))
        .on_click(move |_, _, cx| {
            this.update(cx, |s, cx| {
                s.node_meters.reset_clip(id);
                cx.notify();
            })
        })
        .gpui_test_observe()
        .into_any_element()
}

fn note(text: impl Into<SharedString>) -> Div {
    let text = text.into();
    div()
        .text_size(px(11.))
        .line_height(px(14.))
        .text_color(theme::TEXT_MUTED)
        .child(t!(text.as_ref()).to_string())
}
fn detail_line(key: &str, value: &str) -> Div {
    div()
        .flex()
        .justify_between()
        .gap(px(8.))
        .child(note(key.to_owned()))
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .text_size(px(11.))
                .child(value.to_owned()),
        )
}
