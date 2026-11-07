use super::*;
use crate::service::GraphNodeState;
use crate::ui::presentation::db_text;
use controls::*;
use moiren_core::graph::{EdgeId, NodeId, NodeKind, SendParams, SendTap};
use rust_i18n::t;

mod model;
pub(super) mod page;
mod routing;
pub(super) mod strip_controls;
use crate::ui::widgets::fader::{FaderScale, fader, ruler};
// Slider-sync machinery is shared with the compressor parameter sliders
// (window::graph::compressor), so the slot type, edit type and the
// coalesced sync/commit entry points are visible across the window.
use model::StripDrag;
pub(super) use model::{
    EditableValueFamily, MixerPresentation, SliderEdit, SliderSlot, balance_text,
    commit_active_editor, commit_closure, fader_rows, gain_db_text, mixer_edit,
    reset_clip_on_click, start_value_edit, sync_faders, sync_slider_range,
};

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
        // Only the graph card paints a spectrum body: a Mixer strip or an
        // inspector selection must not keep an analyzer's FFT demanded.
        let spectra_visible = visible && self.presentation.page == Page::Graph;
        self.state.update(cx, |state, _| {
            state.node_meters.set_preferences(preferences);
            // Borrow the registered set straight through: the pump runs at
            // display and poll cadence, and cloning it here would allocate
            // on every one of those ticks (HC-14). The selected node rides
            // the same iterator instead of being inserted into the
            // registration set, which belongs to the canvases alone.
            if visible {
                let points = self.node_meter_points.borrow();
                let meters = state.node_meters.update(
                    &state.snapshot,
                    points.iter().copied().chain(inspector_selected),
                );
                let spectra = if spectra_visible {
                    state
                        .node_spectra
                        .update(&state.snapshot, points.iter().copied())
                } else {
                    state.node_spectra.update(&state.snapshot, [])
                };
                meters || spectra
            } else {
                // Nothing is registered and the inspector demand (above)
                // cannot exist without visibility: release everything.
                let meters = state.node_meters.update(&state.snapshot, []);
                let spectra = state.node_spectra.update(&state.snapshot, []);
                meters || spectra
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

/// The strip's cached control stack: everything below the live routing
/// sections, keyed by `StripKey`. Between key changes this subtree replays
/// its paint while the meter hot layer paints the live bars above it; the
/// routing rows and the strip selection live outside on the page's live
/// column exactly because they change or scroll at display rate.
fn strip_body(
    shell: &StudioShell,
    entry: &GraphNodeState,
    kind: NodeKind,
    channels: usize,
    cx: &Context<StudioShell>,
) -> Div {
    #[cfg(feature = "gpui-test")]
    {
        shell.trace.borrow_mut().strip_renders += 1;
    }
    let id = entry.id;
    // The strip's "!" badge rides the cached title row: the fault state is
    // a StripKey field. The danger frame, wash and banner anchor live on the
    // page's live column (see `strip_column`), which reads the same fault
    // projection every frame without a rebuild.
    let faulted = shell.state.read(cx).snapshot.graph.node_faulted(id);
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
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(crate::ui::presentation::display_label(entry)),
                )
                .when(faulted, |row| row.child(strip_fault_badge(id, cx))),
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
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(strip_controls::balance(shell, entry, channels, cx))
        .child(node_controls(shell, entry, channels, cx))
        .child({
            let editing = shell.mixer.editing_value == Some(id);
            editable_value(
                SharedString::from(format!("strip-readout-{id:?}")),
                db_text(entry.mixer.gain),
                editing,
                &shell.mixer.value_inputs[&id],
                start_value_edit(cx.entity(), EditableValueFamily::Fader, id, move |graph| {
                    graph
                        .node(id)
                        .map(|node| gain_db_text(node.mixer.gain))
                        .unwrap_or_default()
                }),
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
        .child(title)
}

pub(super) use strip_controls::node_controls;

/// The faulted strip's "!" badge at the title row's right edge — the graph
/// card badge's twin. Clicking it opens the fault popover on this strip's
/// fault group; the press is stopped here so the title row keeps its
/// select / double-click-to-inspect behavior for itself.
fn strip_fault_badge(id: NodeId, cx: &Context<StudioShell>) -> impl IntoElement {
    let this = cx.entity();
    div()
        .id(SharedString::from(format!("strip-fault-{id:?}")))
        .flex_none()
        .size(px(14.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::DANGER)
        .text_color(theme::controls::ON_ACCENT)
        .text_size(px(9.))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .child("!")
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
        .on_click(move |_, window, cx| {
            this.update(cx, |shell, cx| {
                // Anchor the popover on the badge: the click position sits
                // on it, so a badge-sized rect under the pointer is the
                // same hang point the graph badge uses.
                let position = window.mouse_position();
                let half = px(7.);
                shell.fault_popover = Some(super::fault_popover::FaultPopover {
                    node: Some(id),
                    anchor: Bounds {
                        origin: point(position.x - half, position.y - half),
                        size: size(px(14.), px(14.)),
                    },
                });
                cx.notify();
            })
        })
        .gpui_test_observe()
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
    // The key projects this same predicate (`page::meter_probe_eligible`);
    // compute it through the shared helper so the two cannot drift.
    let eligible = page::meter_probe_eligible(shell, &state.snapshot, entry, kind);
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
            let state = shell.state.clone();
            let id = entry.id;
            reset_clip_on_click(state, cx.entity(), id)
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
        .child(page::track(shell, entry.id, channels, eligible.then_some(input)))
        .child(div().id(SharedString::from(format!("meter-readout-{:?}", entry.id)))
            .absolute().bottom(px(-28.)).left(px(-10.)).w(px(48.)).h(px(12.)).line_height(px(12.))
            .text_center().font_family(theme::FONT_MONO).text_size(px(8.)).text_color(theme::TEXT_MUTED)
            .child(page::readout(shell, entry.id))
            .gpui_test_observe())
        .tooltip(|window, cx| component::tooltip::Tooltip::new(
            t!("Sample peak / RMS in dBFS. Shared scale with fader gain in dB. Click to reset clip.").into_owned()
        ).build(window, cx))
        .gpui_test_observe()
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
