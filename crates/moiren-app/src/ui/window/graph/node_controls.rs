//! Native, task-specific controls over the graph's shared card geometry.
//! Only presentation state lives here; all edits go through the service.
use super::*;
use crate::service::{GraphEdit, GraphNodeState, InputSelection};
use crate::ui::pages::graph_canvas::{CanvasNode, CanvasWorld};
use crate::ui::widgets::fader::hslider;
use crate::ui::widgets::meters::compact_horizontal_meter;
use crate::ui::window::mixer::SliderSlot;
use moiren_core::graph::{NodeId, NodeKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

#[cfg(all(test, feature = "gpui-test"))]
mod delay_precision_tests;
mod fields;
#[cfg(all(test, feature = "gpui-test"))]
mod gate_precision_tests;
mod integer;
mod io;
#[cfg(all(test, feature = "gpui-test"))]
mod limiter_precision_tests;
mod mapper_editor;
mod parameter;
mod plots;
mod precision;
#[cfg(all(test, feature = "gpui-test"))]
mod precision_intent_tests;
mod rack_editor;
mod recording;
mod rotary;
#[cfg(all(test, feature = "gpui-test"))]
mod rotary_lifecycle_tests;
#[cfg(all(test, feature = "gpui-test"))]
mod tests;
#[cfg(all(test, feature = "gpui-test"))]
mod tone_tests;
mod utility;
use fields::{parameter_control, parameter_row};
pub(super) use integer::{Field as IntegerField, render as integer_inspector};
use io::io_body;
pub(super) use mapper_editor::inspector as mapper_inspector;
use parameter::Parameter;
pub(super) use precision::{
    delay_inspector, eq_inspector, gate_inspector, limiter_inspector, tone_inspector,
};
pub(super) use rack_editor::inspector as rack_inspector;
pub(super) use recording::format_controls as recording_format_controls;
pub(super) use utility::spectrum::inspector as spectrum_inspector;

#[derive(Default)]
pub(in crate::ui::window) struct NodeControls {
    pub(in crate::ui::window) heights: BTreeMap<NodeId, f32>,
    sliders: BTreeMap<(NodeId, Parameter), SliderSlot>,
    rotary_pending: BTreeMap<(NodeId, Parameter), rotary::Pending>,
    inputs: BTreeMap<(NodeId, Parameter), Entity<component::input::InputState>>,
    input_baselines: BTreeMap<(NodeId, Parameter), String>,
    input_observed: BTreeMap<(NodeId, Parameter), String>,
    input_edited: BTreeSet<(NodeId, Parameter)>,
    input_pending: BTreeMap<(NodeId, Parameter), crate::service::RequestId>,
    errors: BTreeMap<(NodeId, Parameter), String>,
    precision: precision::Editors,
    integer: integer::Editors,
    mapper: mapper_editor::Editors,
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
            || self.precision.owns_focus(focused, cx)
            || self.integer.owns_focus(focused, cx)
            || self.mapper.owns_focus(focused, cx)
    }
}

impl StudioShell {
    pub(in crate::ui::window) fn sync_node_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let graph = self.state.read(cx).snapshot.graph.clone();
        let consumed = self.state.read(cx).snapshot.consumed_request_id;
        // A surviving NodeId does not imply the same controls: rebinding Tone
        // to a device retires its frequency draft, focus owner and callbacks.
        let applicable: BTreeSet<_> = graph
            .nodes
            .iter()
            .filter_map(|node| graph.kind_of(node.id).map(|kind| (node, kind)))
            .flat_map(|(node, kind)| {
                Parameter::primary(kind, node)
                    .into_iter()
                    .map(move |parameter| (node.id, parameter))
            })
            .collect();
        self.node_controls
            .heights
            .retain(|id, _| graph.node(*id).is_some());
        self.node_controls
            .spectrum
            .retain(|id, _| graph.kind_of(*id) == Some(NodeKind::SpectrumAnalyzer));
        self.node_controls
            .sliders
            .retain(|key, _| applicable.contains(key));
        self.node_controls
            .inputs
            .retain(|key, _| applicable.contains(key));
        self.node_controls
            .input_baselines
            .retain(|key, _| applicable.contains(key));
        self.node_controls
            .input_observed
            .retain(|key, _| applicable.contains(key));
        self.node_controls
            .input_edited
            .retain(|key| applicable.contains(key));
        self.node_controls
            .input_pending
            .retain(|key, request| applicable.contains(key) && *request > consumed);
        self.node_controls
            .errors
            .retain(|key, _| applicable.contains(key));
        for node in &graph.nodes {
            let Some(kind) = graph.kind_of(node.id) else {
                continue;
            };
            for parameter in Parameter::primary(kind, node) {
                let id = node.id;
                let key = (id, parameter);
                let value = parameter.value(kind, node);
                let text = if parameter == Parameter::Frequency {
                    value.to_string()
                } else {
                    format!("{value:.2}")
                };
                self.node_controls
                    .input_baselines
                    .entry(key)
                    .or_insert_with(|| text.clone());
                self.node_controls
                    .input_observed
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
                                component::input::InputEvent::Change
                                    | component::input::InputEvent::PressEnter { .. }
                                    | component::input::InputEvent::Blur
                            ) {
                                return;
                            }
                            // A queued load can reuse IDs before this old surface
                            // disappears. It must not receive outgoing drafts.
                            if shell.pending_project.is_some() {
                                return;
                            }
                            // A queued Blur may outlive an input removed by
                            // rebinding. Never let it act on the new entity at
                            // the same (node, parameter) key.
                            if shell.node_controls.inputs.get(&key).map(Entity::entity_id)
                                != Some(input.entity_id())
                            {
                                return;
                            }
                            let graph = shell.state.read(cx).snapshot.graph.clone();
                            let Some(node) = graph.node(id) else { return };
                            let Some(kind) = graph.kind_of(id) else {
                                return;
                            };
                            if !Parameter::primary(kind, node).contains(&parameter) {
                                return;
                            }
                            let text = input.read(cx).value().to_string();
                            if matches!(event, component::input::InputEvent::Change) {
                                // Change can follow a no-op replacement. Only a
                                // text transition carries new intent; deleting
                                // then retyping a baseline still counts as edited.
                                if shell.node_controls.input_observed.get(&key) != Some(&text) {
                                    shell.node_controls.input_observed.insert(key, text);
                                    shell.node_controls.input_edited.insert(key);
                                }
                                return;
                            }
                            // Untouched Enter/Blur must not submit rounded text
                            // or undo another surface's newer pending request.
                            if !shell.node_controls.input_edited.contains(&key)
                                && shell.node_controls.input_baselines.get(&key) == Some(&text)
                            {
                                shell.node_controls.errors.remove(&key);
                                cx.notify();
                                return;
                            }
                            let value = parameter::parse(&text, parameter.unit());
                            let edit = value.and_then(|v| parameter.edit_precision(v, kind, node));
                            if let (Some(value), Some(edit)) = (value, edit) {
                                // Changed text is ordered intent, even when it
                                // equals an older snapshot ahead of publication.
                                match shell.state.update(cx, |state, _| {
                                    state.try_dispatch(AppRequest::EditGraph(edit))
                                }) {
                                    Ok(request) => {
                                        shell.node_controls.errors.remove(&key);
                                        shell
                                            .node_controls
                                            .input_baselines
                                            .insert(key, text.clone());
                                        shell.node_controls.input_observed.insert(key, text);
                                        shell.node_controls.input_edited.remove(&key);
                                        shell.node_controls.input_pending.insert(key, request);
                                        rotary::remember_input_admission(
                                            shell, id, parameter, value, request, cx,
                                        );
                                    }
                                    Err(_) => {
                                        // Busy leaves the draft dirty and retryable;
                                        // neither Enter nor Blur may mark it clean.
                                        shell.node_controls.errors.insert(
                                            key,
                                            t!("Edit could not be sent; try again").to_string(),
                                        );
                                    }
                                }
                            } else {
                                shell
                                    .node_controls
                                    .errors
                                    .insert(key, parameter.precision_error(node));
                            }
                            cx.notify();
                        },
                    )
                    .detach();
                    input
                });
                if !input.read(cx).focus_handle(cx).is_focused(window)
                    && !self.node_controls.input_edited.contains(&key)
                    && !self.node_controls.errors.contains_key(&key)
                    && !self.node_controls.input_pending.contains_key(&key)
                    && self
                        .node_controls
                        .input_baselines
                        .get(&key)
                        .map(String::as_str)
                        == Some(input.read(cx).value().as_str())
                {
                    // Preserve dirty text until Blur commits, and committed
                    // text until the service publishes its consumed request.
                    if input.read(cx).value().as_str() != text {
                        input.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                    }
                    self.node_controls.input_baselines.insert(key, text.clone());
                    self.node_controls.input_observed.insert(key, text);
                }
            }
        }
        rotary::sync(self, window, cx);
        rack_editor::sync(self, window, cx);
        utility::spectrum::editor::sync(self, window, cx);
        precision::sync(self, window, cx);
        integer::sync(self, window, cx);
        mapper_editor::sync(self, window, cx);
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
            | NodeKind::ChannelMapper
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
        let meter_width = if card.kind == NodeKind::ChannelMapper {
            crate::ui::pages::graph_canvas::mapper_meter_width(channels)
        } else {
            crate::ui::widgets::meters::node_meter_width(channels)
        };
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
        .child(
            crate::ui::widgets::meters::node_vertical_meter(
                levels,
                channels,
                if levels.is_some() {
                    shell.node_meters.channel_clips(id)
                } else {
                    &[]
                },
            )
            .when(
                snapshot.graph.kind_of(id) == Some(NodeKind::ChannelMapper),
                |meter| {
                    // Bound only the viewport; all channels retain readable columns
                    // inside the meter's existing horizontal scroll surface.
                    meter.w(px(crate::ui::pages::graph_canvas::mapper_meter_width(
                        channels,
                    )))
                },
            ),
        )
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
