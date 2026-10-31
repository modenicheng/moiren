use super::*;
use crate::service::{GraphNodeState, MixerState};
use crate::ui::presentation::db_text;
use controls::*;
use moiren_core::graph::{EdgeId, NodeId, NodeKind, SendParams, SendTap};
use rust_i18n::t;

pub(super) mod meter_frame;
mod model;
#[cfg(feature = "gpui-test")]
pub(super) mod performance;
mod routing;
pub(super) mod strip_controls;
use crate::ui::widgets::fader::{FaderScale, fader, ruler};
use component::menu::ContextMenuExt;
// Slider-sync machinery is shared with the compressor parameter sliders
// (window::graph::compressor), so the slot type, edit type and the
// coalesced sync/commit entry points are visible across the window.
pub(super) use model::{
    EditableValueFamily, MixerPresentation, SliderEdit, SliderSlot, balance_text,
    commit_active_editor, commit_closure, fader_rows, gain_db_text, start_value_edit, sync_faders,
    sync_slider_range,
};
use model::{StripDrag, reorder};
use routing::{inputs, sends};

impl StudioShell {
    pub(super) fn update_node_meters(&mut self, visible: bool, cx: &mut Context<Self>) -> bool {
        // The node inspector is the only inspector that renders meter
        // tracks, and it shows the selected node's own meters (sidebar's
        // `page_inspector`). The Matrix route view and the Devices page
        // mount no meter surface at all, so a selection carried across a
        // page switch must not keep the node's level topic demanded.
        // `visible` alone proves a Graph/Mixer frame gate or a graph
        // gesture, not that the current page's inspector paints the node.
        let inspector_selected = if visible
            && self.inspector_open
            && matches!(self.presentation.page, Page::Graph | Page::Mixer)
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
            Some(selected)
        } else {
            None
        };
        let preferences = self.preferences.meters;
        self.state.update(cx, |state, _| {
            state.node_meters.set_preferences(preferences);
            // Borrow the registered set straight through: the pump runs at
            // display and poll cadence, and cloning it here would allocate
            // on every one of those ticks (HC-14). The selected node rides
            // the same iterator instead of being inserted into the
            // registration set, which belongs to the canvases alone.
            if visible {
                let points = self.node_meter_points.borrow();
                state.node_meters.update(
                    &state.snapshot,
                    points.iter().copied().chain(inspector_selected),
                )
            } else {
                // Nothing is registered and the inspector demand (above)
                // cannot exist without visibility: release everything.
                state.node_meters.update(&state.snapshot, [])
            }
        })
    }
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests;

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
    #[cfg(feature = "gpui-test")]
    {
        shell.mixer.trace.borrow_mut().strip_renders += 1;
    }
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
                    // Single click selects the strip; a double click also
                    // opens the inspector.
                    if shell.element_double_clicked(StudioShell::element_key("node", id)) {
                        shell.open_inspector();
                    }
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
        .w(px(MixerPresentation::STRIP_WIDTH))
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
                .child({
                    let editing = shell.mixer.editing_value == Some(id);
                    editable_value(
                        SharedString::from(format!("strip-readout-{id:?}")),
                        db_text(entry.mixer.gain),
                        editing,
                        &shell.mixer.value_inputs[&id],
                        start_value_edit(
                            cx.entity(),
                            EditableValueFamily::Fader,
                            id,
                            move |graph| {
                                graph
                                    .node(id)
                                    .map(|node| gain_db_text(node.mixer.gain))
                                    .unwrap_or_default()
                            },
                        ),
                        px(12.),
                        TextAlign::Center,
                    )
                    .flex_none()
                    .h(px(24.))
                    .font_family(theme::FONT_MONO)
                    .gpui_test_observe()
                })
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
                        .child(fader(
                            &shell.mixer.faders[&id].state,
                            &shell.mixer.faders[&id].behavior,
                            cx,
                        ))
                        .gpui_test_observe(),
                )
                .child(title),
        )
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
            shell
                .state
                .update(cx, |state, _| state.edit_mixer(id, &change));
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
    let state = shell.state.read(cx);
    let input = kind == NodeKind::Source;
    // The key projects this same predicate (`meter_frame::meter_probe_eligible`);
    // compute it through the shared helper so the two cannot drift.
    let eligible = meter_frame::meter_probe_eligible(shell, &state.snapshot, entry, kind);
    let managed = state
        .snapshot
        .signal_topics
        .node_levels
        .contains_key(&entry.id);
    div()
        .h_full()
        .flex_none()
        .id(SharedString::from(format!("meter-reset-{:?}", entry.id)))
        .on_mouse_down(MouseButton::Left, {
            let this = cx.entity();
            let id = entry.id;
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.state.update(cx, |state, _| state.node_meters.reset_clip(id));
                    cx.notify();
                })
            }
        })
        .relative()
        // The managed-visibility canvas is mounted unconditionally and
        // registers only when its condition and the content mask hold: a
        // rebuild of the strip must refresh (or clear) exactly its own
        // registration, because per-strip panels no longer run a
        // whole-table reset over their siblings.
        .child({
            let points = shell.node_meter_points.clone();
            let id = entry.id;
            canvas(
                move |bounds, window, _| {
                    let intersection = bounds.intersect(&window.content_mask().bounds);
                    if managed
                        && intersection.size.width > px(0.)
                        && intersection.size.height > px(0.)
                    {
                        points.borrow_mut().insert(id);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        })
        .when(eligible, |view| {
            // The eligible strip owns the shared probe flag while mounted;
            // an eligibility or identity transition resets the flags in
            // `refresh_observation_nodes` and the new owners re-register on
            // their rebuild in the same layout pass.
            view.child(meter_probe(shell.meter_points.clone(), input))
        })
        .child(meter_frame::track(shell, entry.id, channels, eligible.then_some(input)))
        .child(div().id(SharedString::from(format!("meter-readout-{:?}", entry.id)))
            .absolute().bottom(px(-28.)).left(px(-10.)).w(px(48.)).h(px(12.)).line_height(px(12.))
            .text_center().font_family(theme::FONT_MONO).text_size(px(8.)).text_color(theme::TEXT_MUTED)
            .child(meter_frame::readout(shell, entry.id))
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
        })
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
