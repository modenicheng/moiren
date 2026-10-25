use super::*;
use crate::service::{GraphEdit, GraphNodeState, MixerState};
use controls::*;
use moiren_core::graph::{EdgeId, NodeId, NodeKind, SendParams, SendTap};
use rust_i18n::t;

mod model;
#[cfg(feature = "gpui-test")]
pub(super) mod performance;
mod routing;
pub(super) mod strip_controls;
use crate::ui::widgets::fader::{FaderScale, fader, ruler};
use component::menu::ContextMenuExt;
pub(super) use model::{MixerPresentation, SliderEdit, gain_db, linear_gain, sync_sliders};
use model::{StripDrag, reorder};
use routing::{inputs, sends};

impl StudioShell {
    pub(super) fn update_node_meters(&mut self, visible: bool, cx: &App) -> bool {
        let mut nodes = if visible {
            self.node_meter_points.borrow().clone()
        } else {
            Default::default()
        };
        if visible
            && self.inspector_open
            && let Some(selected) = self.selected_node
            && self
                .state
                .read(cx)
                .snapshot
                .graph
                .nodes
                .iter()
                .any(|node| node.id == selected)
        {
            nodes.insert(selected);
        }
        self.node_meters
            .update(&self.state.read(cx).snapshot, &nodes)
    }
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests;

pub(super) fn page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let graph = &state.snapshot.graph;
    let mut channels = div()
        .id("mixer-strips")
        .flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .track_scroll(&shell.mixer_scroll)
        .overflow_x_scroll()
        .overflow_y_scroll()
        .on_scroll_wheel({
            let this = cx.entity();
            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
        });
    let mut dock = div().flex().flex_none().min_w_full().h_full().justify_end();
    let dock_container = div()
        .id("mixer-dock")
        .flex()
        .flex_none()
        .w(px(shell.mixer.dock_width))
        .max_w(relative(0.55))
        .min_h_0()
        .border_l_1()
        .border_color(theme::BORDER_STRONG)
        .overflow_x_scroll()
        .overflow_y_scroll()
        .on_scroll_wheel({
            let this = cx.entity();
            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
        });
    for id in &shell.mixer.order {
        let Ok(node) = graph.graph.get_node(*id) else {
            continue;
        };
        let entry = graph.node(*id).expect("service metadata");
        let strip = strip(shell, entry, node.kind(), node.channels(), cx);
        if matches!(node.kind(), NodeKind::Bus | NodeKind::Sink) {
            dock = dock.child(strip);
        } else {
            channels = channels.child(strip);
        }
    }
    div()
        .id("mixer-page")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .on_mouse_move({
            let this = cx.entity();
            move |event, _, cx| {
                this.update(cx, |shell, cx| {
                    if event.pressed_button == Some(MouseButton::Left)
                        && let Some((docked, origin, width)) = shell.mixer.resize
                    {
                        let delta = f32::from(event.position.x) - origin;
                        if docked {
                            shell.mixer.dock_width = (width - delta).clamp(160., 640.);
                        } else {
                            shell.mixer.width = (width + delta).clamp(136., 280.);
                        }
                        cx.notify();
                    }
                });
            }
        })
        .on_mouse_up(MouseButton::Left, {
            let this = cx.entity();
            move |_, _, cx| this.update(cx, |shell, _| shell.mixer.resize = None)
        })
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(40.))
                .items_center()
                .justify_between()
                .px(px(16.))
                .child(heading(t!("Mixer")))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .when(
                            graph.active_revision != Some(graph.candidate_revision),
                            |row| row.child(badge(t!("Pending"), theme::WARNING)),
                        )
                        .child(apply(shell, cx))
                        .child(inspector_toggle("toggle-details", cx)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(channels.gpui_test_observe())
                .child(resize_handle(shell, true, "mixer-dock-resize", cx))
                .child(dock_container.child(dock).gpui_test_observe())
                .when(shell.inspector_open, |view| {
                    if shell.selected_node.is_none() && shell.selected_route.is_some() {
                        view.child(matrix::inspector(shell, cx))
                    } else {
                        view.child(graph::inspector(shell, cx))
                    }
                }),
        )
        .gpui_test_observe()
        .into_any_element()
}

fn resize_handle(
    shell: &StudioShell,
    docked: bool,
    id: impl Into<ElementId>,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let width = if docked {
        shell.mixer.dock_width
    } else {
        shell.mixer.width
    };
    let this = cx.entity();
    div()
        .id(id)
        .w(px(6.))
        .h_full()
        .flex_none()
        .bg(theme::SURFACE)
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            this.update(cx, |shell, _| {
                shell.mixer.resize = Some((docked, f32::from(event.position.x), width))
            });
        })
        .gpui_test_observe()
}

pub(super) fn node_kind_color(kind: NodeKind) -> Hsla {
    match kind {
        NodeKind::Source => theme::NODE_SOURCE,
        NodeKind::Sink => theme::NODE_SINK,
        NodeKind::Bus => theme::NODE_UTILITY,
        _ => theme::NODE_DSP,
    }
}

fn strip(
    shell: &StudioShell,
    entry: &GraphNodeState,
    kind: NodeKind,
    channels: usize,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let id = entry.id;
    let selected = shell.selected_node == Some(id);
    let color = node_kind_color(kind);
    let title = div()
        .id(SharedString::from(format!("strip-header-{id:?}")))
        .flex()
        .flex_col()
        .gap(px(2.))
        .h(px(if shell.compact { 44. } else { 52. }))
        .flex_none()
        .px(px(10.))
        .justify_center()
        .border_t_1()
        .border_color(theme::BORDER)
        .cursor_pointer()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .overflow_hidden()
                .text_ellipsis()
                .child(crate::ui::state::display_label(
                    &shell.state.read(cx).snapshot,
                    entry,
                    kind,
                )),
        )
        .child(
            label(format!(
                "{} · {kind:?} · {channels} ch",
                shell
                    .mixer
                    .order
                    .iter()
                    .position(|node| *node == id)
                    .unwrap_or(0)
                    + 1
            ))
            .text_size(px(9.)),
        )
        .on_click({
            let this = cx.entity();
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.selected_node = Some(id);
                    shell.selected_route = None;
                    shell.inspector_open = true;
                    cx.notify();
                })
            }
        })
        .on_drag(id, move |_, _, _, cx| cx.new(|_| StripDrag))
        .tooltip(|window, cx| {
            component::tooltip::Tooltip::new(
                t!("Drag to reorder channels within the same mixer section").into_owned(),
            )
            .build(window, cx)
        })
        .gpui_test_observe();
    div()
        .id(SharedString::from(format!("strip-{id:?}")))
        .flex()
        .flex_none()
        .w(px(shell.mixer.width))
        .h_full()
        .min_h_0()
        .bg(if selected {
            theme::SURFACE_3
        } else {
            theme::SURFACE
        })
        .relative()
        .rounded_none()
        .border_l_1()
        .border_t_1()
        .border_b_1()
        .border_r_1()
        .border_color(theme::BORDER)
        .on_drop({
            let this = cx.entity();
            move |dragged: &NodeId, _, cx| {
                this.update(cx, |shell, cx| {
                    let graph = &shell.state.read(cx).snapshot.graph.graph;
                    let docked = |id| {
                        graph
                            .get_node(id)
                            .is_ok_and(|node| matches!(node.kind(), NodeKind::Bus | NodeKind::Sink))
                    };
                    if docked(*dragged) == docked(id) {
                        reorder(&mut shell.mixer.order, *dragged, id);
                    }
                    cx.notify();
                })
            }
        })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .pt(px(4.))
                .child(inputs(shell, entry, cx))
                .child(sends(shell, id, cx))
                .child(strip_controls::balance(shell, entry, channels, cx))
                .child(node_controls(entry, channels, cx))
                .child(
                    div()
                        .id(SharedString::from(format!("strip-readout-{id:?}")))
                        .flex_none()
                        .h(px(24.))
                        .text_center()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(12.))
                        .child(db(entry.mixer.gain))
                        .gpui_test_observe(),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("strip-levels-{id:?}")))
                        .relative()
                        .flex()
                        .flex_1()
                        .min_h(px(160.))
                        .justify_center()
                        .gap(px(6.))
                        .mt(px(14.))
                        .mb(px(32.))
                        .child(
                            div()
                                .id(SharedString::from(format!("strip-unity-{id:?}")))
                                .absolute()
                                .top(relative(FaderScale::position(1.)))
                                .h(px(1.))
                                .w_full()
                                .bg(theme::BORDER_STRONG)
                                .gpui_test_observe(),
                        )
                        .child(meter(shell, entry, kind, channels, cx))
                        .child(ruler())
                        .child(fader(&shell.mixer.faders[&id], entry.mixer.gain))
                        .gpui_test_observe(),
                )
                .child(title),
        )
        .child(resize_handle(
            shell,
            false,
            SharedString::from(format!("strip-resize-{id:?}")),
            cx,
        ))
        // Paint after the resize handle so its surface cannot shorten the bar.
        .child(
            div()
                .id(SharedString::from(format!("strip-color-{id:?}")))
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(4.))
                .bg(color.opacity(0.8))
                .gpui_test_observe(),
        )
        .gpui_test_observe()
}

pub(super) use strip_controls::node_controls;

fn mixer_edit(
    this: Entity<StudioShell>,
    id: NodeId,
    change: impl Fn(&mut MixerState) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        this.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                if let Some(node) = state.snapshot.graph.node(id) {
                    let mut requested = node.mixer;
                    change(&mut requested);
                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetMixer {
                        node: id,
                        state: requested,
                    }));
                }
            });
            cx.notify();
        })
    }
}

fn meter(
    shell: &StudioShell,
    entry: &GraphNodeState,
    kind: NodeKind,
    channels: usize,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    #[cfg(feature = "gpui-test")]
    let began = std::time::Instant::now();
    let state = shell.state.read(cx);
    let input = kind == NodeKind::Source;
    let eligible = matches!(kind, NodeKind::Source | NodeKind::Sink)
        && shell
            .observation_nodes
            .is_some_and(|(generation, source, sink)| {
                generation == state.snapshot.generation
                    && entry.id == if input { source } else { sink }
            })
        && state.snapshot.desired.as_ref().is_some_and(|active| {
            if input {
                entry.source.as_ref() == Some(&active.source)
            } else {
                entry.output_endpoint_id.as_ref() == Some(&active.output_endpoint_id)
            }
        });
    let levels = eligible
        .then_some(if input {
            shell.meters.display_input()
        } else {
            shell.meters.display_output()
        })
        .flatten();
    let levels = shell.node_meters.levels(entry.id).or(levels);
    let clips = shell.node_meters.channel_clips(entry.id);
    let managed = state
        .snapshot
        .signal_topics
        .node_levels
        .contains_key(&entry.id);
    let raw = shell.node_meters.raw_levels(entry.id);
    let readout = raw.map(|levels| levels.iter().map(|level| level.peak).fold(0f32, f32::max));
    let view = div()
        .h_full()
        .flex_none()
        .id(SharedString::from(format!("meter-reset-{:?}", entry.id)))
        .on_mouse_down(MouseButton::Left, {
            let this = cx.entity();
            let id = entry.id;
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.node_meters.reset_clip(id);
                    cx.notify();
                })
            }
        })
        .relative()
        .when(managed, |view| {
            let points = shell.node_meter_points.clone();
            let id = entry.id;
            view.child(
                canvas(
                    move |bounds, window, _| {
                        let intersection = bounds.intersect(&window.content_mask().bounds);
                        if intersection.size.width > px(0.) && intersection.size.height > px(0.) {
                            points.borrow_mut().insert(id);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
        })
        .when(eligible, |view| {
            view.child(meter_probe(shell.meter_points.clone(), input))
        })
        .child(crate::ui::widgets::meters::channel_meter(
            shell.node_meters.channel_levels(entry.id).or_else(|| levels.as_ref().map(|levels| levels.as_slice())), channels, clips,
        ))
        .child(div().id(SharedString::from(format!("meter-readout-{:?}", entry.id)))
            .absolute().bottom(px(-28.)).left(px(-10.)).w(px(48.)).h(px(12.)).line_height(px(12.))
            .text_center().font_family(theme::FONT_MONO).text_size(px(8.)).text_color(theme::TEXT_MUTED)
            .child(readout.map(|peak| if peak == 0. { "−∞".into() }
                else { format!("{:.1}", 20. * peak.log10()) }).unwrap_or_else(|| "—".into()))
            .gpui_test_observe())
        .tooltip(|window, cx| component::tooltip::Tooltip::new(
            t!("Sample peak / RMS in dBFS. Shared scale with fader gain in dB. Click to reset clip; right-click for response.").into_owned()
        ).build(window, cx))
        .gpui_test_observe()
        .context_menu({
            let this = cx.entity();
            let settings = shell.preferences.meters;
            move |mut menu, _, _| {
                use crate::project::{MeterFalloff, PeakHold};
                use component::menu::PopupMenuItem;
                menu = menu.label(t!("Peak fall-off · dB/s"));
                for falloff in MeterFalloff::ALL {
                    let this = this.clone();
                    menu = menu.item(PopupMenuItem::new(format!("{} dB/s", falloff.db_per_second()))
                        .checked(settings.falloff == falloff)
                        .on_click(move |_, _, cx| this.update(cx, |shell, cx| {
                            shell.preferences.meters.falloff = falloff;
                            shell.save_preferences(cx);
                            cx.notify();
                        })));
                }
                menu = menu.separator().label(t!("Peak hold"));
                for hold in PeakHold::ALL {
                    let this = this.clone();
                    menu = menu.item(PopupMenuItem::new(if hold == PeakHold::Off { t!("Off").into_owned() }
                        else { format!("{} s", hold.duration().as_secs_f32()) })
                        .checked(settings.peak_hold == hold)
                        .on_click(move |_, _, cx| this.update(cx, |shell, cx| {
                            shell.preferences.meters.peak_hold = hold;
                            shell.save_preferences(cx);
                            cx.notify();
                        })));
                }
                menu
            }
        });
    #[cfg(feature = "gpui-test")]
    shell.mixer.trace.borrow_mut().add(began.elapsed());
    view
}

fn meter_probe(
    points: std::rc::Rc<std::cell::Cell<(bool, bool)>>,
    input: bool,
) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            let intersection = bounds.intersect(&window.content_mask().bounds);
            let visible = intersection.size.width > px(0.) && intersection.size.height > px(0.);
            let mut flags = points.get();
            if input {
                flags.0 = visible;
            } else {
                flags.1 = visible;
            }
            points.set(flags);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
