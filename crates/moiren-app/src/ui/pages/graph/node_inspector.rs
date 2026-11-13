//! The node inspector sidebar: the selected node's full column. This root
//! module assembles the panel — inline rename, headline visualization,
//! channel strip, ports, and the delete action — and owns the runtime
//! observation readout whose sampled snapshot (issue #118) keys the cached
//! panel. The per-`NodeKind` sections live in [`sections`]; the builtin
//! processor's controls and their table-driven field schema live in
//! [`builtin_controls`] and [`builtin_fields`].

use rust_i18n::t;

use super::*;
use crate::service::GraphEdit;
use crate::ui::editors::{
    EditableValueFamily, fader_readout_gain, gain_db_text, reset_clip_on_click, start_value_edit,
};
use crate::ui::presentation::{db_text, dbfs_text};
use crate::ui::widgets::fader::hslider;
use controls::*;
use moiren_core::graph::NodeKind;

mod builtin_controls;
mod builtin_fields;
mod sections;

pub(crate) fn inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let selected = snapshot
        .graph
        .graph
        .nodes()
        .iter()
        .find(|node| shell.selection.node == Some(node.id()));
    let mut view =
        panel("graph-inspector", &shell.inspector.scroll).child(heading(t!("Inspector")));
    let Some(node) = selected else {
        let view = view
            .child(
                div()
                    .id("inspector-empty")
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(theme::SPACE_4))
                    .child(
                        div()
                            .id("inspector-empty-art")
                            .flex()
                            .flex_none()
                            .w(px(64.))
                            .h(px(64.))
                            .items_center()
                            .justify_center()
                            .rounded(px(theme::RADIUS_LG))
                            .bg(theme::controls::RAISED)
                            .child(
                                component::Icon::default()
                                    .path("icons/zoom-scan.svg")
                                    .size(px(28.))
                                    .text_color(theme::TEXT_FAINT),
                            )
                            .gpui_test_observe(),
                    )
                    .child(label(t!("Select a node to inspect its properties.")))
                    .gpui_test_observe(),
            )
            .gpui_test_observe();
        return panel_host("graph-inspector", &shell.inspector.scroll, view).into_any_element();
    };
    let id = node.id();
    let entry = snapshot.graph.node(id).expect("service node metadata");
    let display = crate::ui::presentation::display_label(entry);
    let editing = shell.inspector.node_name_editing == Some(id);
    // One inline name control: a plain heading until clicked, then the editor
    // in the same place. There is no separate rename row.
    let name_control: AnyElement = if editing {
        div()
            .id("node-name-editor")
            .flex()
            .items_center()
            .gap(px(theme::SPACE_1))
            .child(
                component::input::Input::new(&shell.inspector.node_name)
                    .id("node-name")
                    .bg(theme::controls::RAISED)
                    .text_color(theme::TEXT)
                    .border_color(theme::BORDER),
            )
            .child(button("node-name-confirm", "✓").on_click({
                let this = cx.entity();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.confirm_node_rename(cx));
                }
            }))
            .child(button("node-name-cancel", "✗").on_click({
                let this = cx.entity();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.cancel_node_rename(cx));
                }
            }))
            .into_any_element()
    } else {
        div()
            .id("node-name-display")
            .flex()
            .items_center()
            .gap(px(theme::SPACE_1_5))
            .cursor_pointer()
            .child(heading(display.clone()))
            .child(
                div()
                    .id("node-name-edit-hint")
                    .text_size(px(theme::TEXT_XS))
                    .text_color(theme::TEXT_FAINT)
                    .child(t!("rename")),
            )
            .on_click({
                let this = cx.entity();
                let initial = display.clone();
                move |_, window, cx| {
                    this.update(cx, |shell, cx| {
                        shell.inspector.node_name_editing = Some(id);
                        shell
                            .inspector
                            .node_name
                            .update(cx, |input, cx| input.set_value(initial.clone(), window, cx));
                        cx.notify();
                    });
                    let input = this.read(cx).inspector.node_name.clone();
                    let handle = input.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                }
            })
            .gpui_test_observe()
            .into_any_element()
    };
    view = view
        .child(
            div()
                .pb(px(theme::SPACE_3))
                .border_b_1()
                .border_color(theme::BORDER)
                .child(name_control)
                .child(label(node_catalog::kind_title(node.kind()))),
        )
        .child(detail(&t!("Channels"), node.channels().to_string()));
    // The headline visualization (design 18 B4) leads the kind-specific
    // sections: the EQ's draggable control point and the dynamics family's
    // transfer curve render here.
    view = match node.kind() {
        NodeKind::Eq => view.child(headline::eq_section(
            id,
            entry,
            cx,
            headline::EQ_INSPECTOR_HEIGHT,
        )),
        NodeKind::Compressor | NodeKind::Gate | NodeKind::Limiter => view.child(
            headline::dynamics_section(node.kind(), entry, headline::DYNAMICS_INSPECTOR_HEIGHT),
        ),
        _ => view,
    };
    view = view
        .child(heading(t!("Channel strip")))
        .child(detail_editable(
            &t!("Fader"),
            SharedString::from(format!("inspector-fader-{id:?}")),
            db_text(fader_readout_gain(
                shell.mixer.presentation.inspector_faders.get(&id),
                entry.mixer.gain,
                cx,
            )),
            shell.mixer.presentation.editing_value == Some(id),
            &shell.mixer.presentation.value_inputs[&id],
            start_value_edit(cx.entity(), EditableValueFamily::Fader, id, move |graph| {
                graph
                    .node(id)
                    .map(|node| gain_db_text(node.mixer.gain))
                    .unwrap_or_default()
            }),
        ))
        .child(hslider(
            &shell.mixer.presentation.inspector_faders[&id].state,
            &shell.mixer.presentation.inspector_faders[&id].behavior,
            cx,
        ))
        // Live bars are the sidebar hot surface's job; this painting-free
        // canvas only registers where this node's meter tracks sit.
        .child(inspector_meter_track(
            shell,
            cx.entity(),
            id,
            node.channels(),
        ))
        .when(node.channels() == 2, |view| {
            view.child(crate::ui::shell::mixer::strip_controls::balance(
                shell,
                entry,
                node.channels(),
                cx,
            ))
        })
        .child(crate::ui::shell::mixer::node_controls(
            shell,
            entry,
            node.channels(),
            cx,
        ));
    if node.kind() == NodeKind::Source {
        // The class filter's stored half: which template created this node
        // (or what a restored project remembered). The live binding can
        // still override it — see `effective_source_class`.
        let stored = shell.graph_view.source_classes.get(&id).copied();
        view = sections::source_section(view, snapshot, entry, stored, id, cx);
    }
    if node.kind() == NodeKind::Sink {
        view = sections::sink_section(view, snapshot, entry, id, cx);
    }
    if node.kind() == NodeKind::Gain {
        view = sections::gain_section(view, shell, entry, id, cx);
    }
    if node.kind() == NodeKind::Pan {
        view = sections::pan_section(view, shell, entry, id, cx);
    }
    if node.kind() == NodeKind::Compressor {
        view = sections::compressor_section(view, shell, entry, id, cx);
    }
    view = view.child(builtin_controls::builtin_controls(
        id,
        node,
        &entry.builtin,
        snapshot,
        shell,
        cx,
    ));
    view = view.child(node_observation(shell, node, cx));
    view = view.child(heading(t!("Ports")));
    for (index, port) in node.inputs().iter().enumerate() {
        view = view.child(detail(
            &t!("Input %{index}", index = index + 1),
            t!("%{channels} ch", channels = port.channels()).into_owned(),
        ));
    }
    for (index, port) in node.outputs().iter().enumerate() {
        view = view.child(detail(
            &t!("Output %{index}", index = index + 1),
            t!("%{channels} ch", channels = port.channels()).into_owned(),
        ));
    }
    if node.kind() == NodeKind::Bus {
        view = view.child(
            button("add-bus-input", t!("Add bus input"))
                .on_click(edit(cx.entity(), GraphEdit::AddBusInput(id))),
        );
        if let Some(port) = node.inputs().last() {
            view = view.child(
                button("remove-bus-input", t!("Remove last input")).on_click(edit(
                    cx.entity(),
                    GraphEdit::RemoveBusInput {
                        node: id,
                        port: port.id(),
                    },
                )),
            );
        }
    }
    view.child(
        button("remove-node", t!("Delete node"))
            .variant(ButtonVariant::Danger)
            .on_click(edit(cx.entity(), GraphEdit::RemoveNode(id))),
    )
    .gpui_test_observe()
    .into_any_element()
}

/// The inspector's runtime-observation readout, sampled for the cache key
/// (issue #118). The status transitions are discrete and re-key immediately;
/// the Peak/RMS texts move with the ballistics at the meter frame rate, so
/// they re-key on the sampling cadence instead — keying the raw display
/// levels would rebuild the whole cached inspector panel per display frame.
/// The snapshot stores the formatted texts because those are exactly what a
/// viewer can see: two readings that spell the same text never wake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservationSnapshot {
    pub(crate) status: crate::ui::monitoring::MeterStatus,
    pub(crate) texts: Vec<String>,
}

pub(crate) fn observation_snapshot(
    meters: &crate::ui::monitoring::NodeMeters,
    node: moiren_core::graph::NodeId,
) -> ObservationSnapshot {
    let texts = meters
        .channel_levels(node)
        .map(|levels| {
            levels
                .iter()
                .flat_map(|level| [dbfs_text(level.marker), dbfs_text(level.rms)])
                .collect()
        })
        .unwrap_or_default();
    ObservationSnapshot {
        status: meters.status(node),
        texts,
    }
}

/// How often the observation texts are re-sampled into the inspector key.
/// Text readouts need no more than a few refreshes a second, and every
/// re-key rebuilds the cached inspector panel.
pub(crate) const OBSERVATION_SAMPLE_CADENCE: std::time::Duration =
    std::time::Duration::from_millis(200);

fn node_observation(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let state = shell.state.read(cx);
    let running = state.snapshot.phase == crate::service::SessionPhase::Running;
    let managed = state
        .snapshot
        .signal_topics
        .node_levels
        .contains_key(&node.id());
    let eligible = running && managed;
    let status = if eligible {
        shell.state.read(cx).node_meters.status(node.id()).text()
    } else if managed {
        t!("Not running").to_string()
    } else {
        t!("Unavailable for this node").to_string()
    };
    let mut view = div()
        .id("node-observation")
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_1_5))
        .child(heading(t!("Runtime observation")))
        .child(
            div()
                .id("node-observation-status")
                .child(detail(&t!("Status"), status))
                .gpui_test_observe(),
        );
    if !running && managed {
        view = view.child(label(t!("Start audio to observe this node.")));
    } else if !managed {
        view = view.child(label(t!(
            "This node has no managed level topic in the current service projection."
        )));
    }
    if let Some(levels) = eligible
        .then(|| shell.state.read(cx).node_meters.channel_levels(node.id()))
        .flatten()
    {
        let mut readings = div()
            .id("node-observation-levels")
            .flex()
            .flex_col()
            .gpui_test_observe();
        for (channel, level) in levels.iter().enumerate() {
            readings = readings
                .child(detail(
                    &t!("Peak %{index}", index = channel + 1),
                    dbfs_text(level.marker),
                ))
                .child(detail(
                    &t!("RMS %{index}", index = channel + 1),
                    dbfs_text(level.rms),
                ));
        }
        view = view.child(readings);
    }
    panel_host("graph-inspector", &shell.inspector.scroll, view).into_any_element()
}

impl StudioShell {
    /// Per-render graph-page housekeeping: exit a name edit whose node is no
    /// longer selected, drop selection members the graph no longer has, and
    /// keep the primary selection inside the canvas multi-selection.
    pub(crate) fn sync_node_name(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.inspector.node_name_editing
            && self.selection.node != Some(id)
        {
            self.inspector.node_name_editing = None;
        }
        let graph = self.state.read(cx).snapshot.graph.clone();
        self.selection.nodes.retain(|id| graph.node(*id).is_some());
        // Class entries are spawn/project intent; retired nodes release
        // theirs like the selection does.
        self.graph_view
            .source_classes
            .retain(|id, _| graph.node(*id).is_some());
        if let Some(id) = self.selection.node {
            if graph.node(id).is_some() {
                self.selection.nodes.insert(id);
            } else {
                self.selection.node = None;
            }
        }
    }

    /// Commits the inline name edit: dispatches SetLabel unless the value is
    /// empty (treated as cancel). Enter, ✓ and focus loss land here.
    pub(crate) fn confirm_node_rename(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.inspector.node_name_editing.take() else {
            return;
        };
        let label = self.inspector.node_name.read(cx).value().trim().to_string();
        if !label.is_empty() {
            self.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetLabel {
                    node: id,
                    label,
                }))
            });
        }
        cx.notify();
    }

    /// Discards the inline name edit without touching the stored label.
    fn cancel_node_rename(&mut self, cx: &mut Context<Self>) {
        if self.inspector.node_name_editing.take().is_some() {
            cx.notify();
        }
    }
}

/// The inspector's meter footprint: a painting-free canvas that registers
/// where this node's meter tracks sit, for the sidebar hot surface to paint
/// live bars at. Sized like the DOM meter it replaced.
fn inspector_meter_track(
    shell: &StudioShell,
    this: Entity<StudioShell>,
    id: moiren_core::graph::NodeId,
    channels: usize,
) -> impl IntoElement {
    let geometry = shell.inspector.meters.clone();
    // Clicking the meter resets the clip latch, like every meter surface.
    div()
        .id(SharedString::from(format!("inspector-meter-{id:?}")))
        .w_full()
        .h(px(if channels >= 2 { 15. } else { 9. }))
        .flex_none()
        .on_mouse_down(
            MouseButton::Left,
            reset_clip_on_click(shell.state.clone(), this, id),
        )
        .child(
            canvas(
                move |bounds, window, _| {
                    geometry
                        .update(|geometry| geometry.register(id, bounds, window.content_mask()));
                },
                |_, _, _, _| {},
            )
            .size_full(),
        )
}
