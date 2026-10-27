//! Tray flyout window: quick fader access to the audio-relevant mixer
//! strips (sources, buses, sinks) without opening the main window.
//!
//! Faders reuse the main mixer's rows verbatim, so both share the identical
//! perceptual value space and edit path (see `mixer::model`): drag Changes
//! dispatch coalesced, the release's final value always lands, and the wheel
//! and double-click gestures edit immediately, as do mute/solo clicks.
//! Escape closes the window.

use rust_i18n::t;
use std::collections::{BTreeMap, BTreeSet};

use super::controls::{button, heading, label};
use super::desktop::Desktop;
use super::mixer::{SliderSlot, fader_rows, node_kind_color, sync_faders};
use super::*;
use crate::service::{GraphNodeState, GraphSnapshot, MixerState};
use crate::ui::monitoring::NodeMeters;
use crate::ui::presentation::db_text;
use crate::ui::widgets::fader::fader;
use moiren_core::graph::{NodeId, NodeKind};

const STRIP_WIDTH: f32 = 104.;
const FLYOUT_HEIGHT: f32 = 372.;

#[cfg(all(test, feature = "gpui-test"))]
mod tests;

/// Strips shown in the flyout: nodes that carry audio in and out of the
/// graph. DSP shaping stays in the main mixer.
pub(super) fn strip_ids(snapshot: &GraphSnapshot) -> Vec<NodeId> {
    snapshot
        .nodes
        .iter()
        .filter(|node| {
            snapshot.graph.get_node(node.id).is_ok_and(|node| {
                matches!(
                    node.kind(),
                    NodeKind::Source | NodeKind::Bus | NodeKind::Sink
                )
            })
        })
        .map(|node| node.id)
        .collect()
}

/// Window size for a flyout showing `strips` entries.
pub(super) fn preferred_size(strips: usize) -> Size<Pixels> {
    size(
        px((strips as f32 * STRIP_WIDTH + 24.).clamp(280., 560.)),
        px(FLYOUT_HEIGHT),
    )
}

pub(super) struct TrayMixer {
    state: Entity<UiState>,
    desktop: Option<WeakEntity<Desktop>>,
    node_meters: NodeMeters,
    faders: BTreeMap<NodeId, SliderSlot>,
    focus: FocusHandle,
}

impl TrayMixer {
    pub(super) fn new(
        state: Entity<UiState>,
        desktop: Option<WeakEntity<Desktop>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        component::Theme::change(component::ThemeMode::Dark, Some(window), cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.on_release(|this, cx| this.release_desktop(cx)).detach();
        let mut node_meters = NodeMeters::new(state.read(cx).handle.clone());
        node_meters.set_preferences(
            crate::project::load_preferences()
                .unwrap_or_default()
                .meters,
        );
        Self {
            node_meters,
            state,
            desktop,
            faders: BTreeMap::new(),
            focus,
        }
    }

    /// Drop the desktop's flyout handles so a stale window never blocks the
    /// next toggle.
    fn release_desktop(&self, cx: &mut App) {
        if let Some(desktop) = self.desktop.clone() {
            let _ = desktop.update(cx, |desktop, _| {
                desktop.flyout = None;
                desktop.flyout_view = None;
            });
        }
    }

    /// Advance meter observations once; every rendered strip counts as
    /// visible because the flyout shows no other content.
    pub(super) fn pump_meters(&mut self, cx: &App) -> bool {
        let visible: std::collections::BTreeSet<NodeId> = self.faders.keys().copied().collect();
        self.node_meters
            .update(&self.state.read(cx).snapshot, &visible)
    }

    fn sync_faders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let graph = self.state.read(cx).snapshot.graph.clone();
        self.faders.retain(|id, _| graph.node(*id).is_some());
        // The shared rows cover every node; the flyout keeps only the
        // audio-bearing strips (sources, buses, sinks).
        let strips: BTreeSet<NodeId> = strip_ids(&graph).into_iter().collect();
        let mut rows = fader_rows(&graph);
        rows.retain(|(id, _, _)| strips.contains(id));
        sync_faders(window, cx, &self.state, &mut self.faders, rows);
    }
}

impl Render for TrayMixer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_faders(window, cx);
        // Frame-driven meters: keep the vsync loop running while the flyout
        // is alive; the desktop poll covers snapshot changes.
        self.pump_meters(cx);
        window.request_animation_frame();

        let state = self.state.read(cx);
        let graph = &state.snapshot.graph;
        let ids = strip_ids(graph);
        let empty = ids.is_empty();

        let mut strips = div()
            .id("tray-mixer-strips")
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_x_scroll();
        for id in ids {
            let (Ok(node), Some(entry)) = (graph.graph.get_node(id), graph.node(id)) else {
                continue;
            };
            strips = strips.child(strip(self, entry, node.kind(), cx));
        }

        div()
            .id("tray-mixer")
            .track_focus(&self.focus)
            .on_key_down(|event: &KeyDownEvent, window: &mut Window, _: &mut App| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            })
            .size_full()
            .flex()
            .flex_col()
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::BORDER_STRONG)
            .bg(theme::SURFACE)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .h(px(36.))
                    .border_b_1()
                    .border_color(theme::BORDER)
                    .child(heading(t!("Mixer")))
                    .child(div().flex_1())
                    .child(open_button(self, cx)),
            )
            .child(if empty {
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .px(px(16.))
                    .child(label(t!("No strips · open Moiren to build a graph")))
                    .into_any_element()
            } else {
                strips.into_any_element()
            })
    }
}

fn open_button(flyout: &TrayMixer, cx: &Context<TrayMixer>) -> impl IntoElement {
    let this = cx.entity();
    let desktop = flyout.desktop.clone();
    button("tray-mixer-open", t!("Open"))
        .variant(ButtonVariant::Ghost)
        .on_click(move |_, _, cx| {
            this.update(cx, |_, cx| {
                if let Some(desktop) = desktop.clone() {
                    let _ = desktop.update(cx, |desktop, cx| {
                        if let Err(error) = desktop.show_window(cx) {
                            eprintln!("Moiren show window: {error}");
                        }
                    });
                }
                cx.notify();
            });
        })
}

fn strip(
    flyout: &TrayMixer,
    entry: &GraphNodeState,
    kind: NodeKind,
    cx: &Context<TrayMixer>,
) -> impl IntoElement {
    let id = entry.id;
    let color = node_kind_color(kind);
    let levels = flyout.node_meters.levels(id);
    let clips = flyout.node_meters.clips(id);
    div()
        .id(SharedString::from(format!("tray-strip-{id:?}")))
        .flex()
        .flex_col()
        .flex_none()
        .w(px(STRIP_WIDTH))
        .min_h_0()
        .bg(theme::SURFACE)
        .border_r_1()
        .border_color(theme::BORDER)
        .child(div().h(px(3.)).flex_none().bg(color.opacity(0.55)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_none()
                .min_w_0()
                .px(px(8.))
                .py(px(8.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(theme::TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::TEXT)
                        .child(entry.label.clone()),
                )
                .child(label(format!("{kind:?}"))),
        )
        .child(
            div()
                .id(SharedString::from(format!("tray-strip-levels-{id:?}")))
                .flex()
                .flex_none()
                .items_end()
                .justify_center()
                .gap(px(8.))
                .py(px(6.))
                // The shared fader fills its container; the flyout fixes the
                // track height the main mixer takes from the strip.
                .child(div().h(px(196.)).flex_none().child(fader(
                    &flyout.faders[&id].state,
                    &flyout.faders[&id].behavior,
                    cx,
                )))
                .child(crate::ui::widgets::meters::compact_stereo_meter(
                    levels, clips,
                ))
                .on_mouse_down(MouseButton::Left, {
                    let this = cx.entity();
                    move |_, _, cx| {
                        this.update(cx, |flyout, cx| {
                            flyout.node_meters.reset_clip(id);
                            cx.notify();
                        });
                    }
                })
                .gpui_test_observe(),
        )
        .child(
            div()
                .id(SharedString::from(format!("tray-strip-controls-{id:?}")))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .px(px(8.))
                .pb(px(6.))
                .child(
                    button(SharedString::from(format!("tray-strip-mute-{id:?}")), "M")
                        .variant(if entry.mixer.muted {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(mixer_edit(flyout.state.clone(), cx.entity(), id, |state| {
                            state.muted = !state.muted
                        })),
                )
                .child(
                    button(SharedString::from(format!("tray-strip-solo-{id:?}")), "S")
                        .variant(if entry.mixer.solo {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(mixer_edit(flyout.state.clone(), cx.entity(), id, |state| {
                            state.solo = !state.solo
                        })),
                )
                .gpui_test_observe(),
        )
        .child(
            div()
                .id(SharedString::from(format!("tray-strip-readout-{id:?}")))
                .flex()
                .flex_none()
                .justify_center()
                .pb(px(10.))
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(11.))
                        .text_color(theme::TEXT_MUTED)
                        .child(db_text(entry.mixer.gain)),
                )
                .gpui_test_observe(),
        )
        .gpui_test_observe()
}

/// Strip edit; it applies itself through the live parameter path.
fn mixer_edit(
    state: Entity<UiState>,
    this: Entity<TrayMixer>,
    id: NodeId,
    change: impl Fn(&mut MixerState) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        state.update(cx, |state, _| state.edit_mixer(id, &change));
        this.update(cx, |_, cx| cx.notify());
    }
}
