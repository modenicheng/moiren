//! Tray flyout window: quick fader access to the audio-relevant mixer
//! strips (sources, buses, sinks) without opening the main window.
//!
//! Faders reuse the main mixer's rows verbatim, so both share the identical
//! perceptual value space and edit path (see `mixer::model`): drag Changes
//! dispatch coalesced, the release's final value always lands, and the wheel
//! and double-click gestures edit immediately, as do mute/solo clicks.
//!
//! The strips are a cached panel ([`TrayPanel`]) and the live meter bars a
//! hot surface ([`FlyoutMeters`]) over them — the same two-tier split as the
//! main window's pages (design 17). Meter observations pump the
//! application's one `NodeMeters` table (shared with the main window
//! through `UiState`), so both windows see identical ballistics. Escape
//! closes the window.

use rust_i18n::t;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::controls::{button, heading, label};
use super::desktop::Desktop;
use super::hot::{HotLayerView, HotSurface, MeterTracks};
use super::mixer::{
    SliderSlot, fader_rows, mixer_edit, node_kind_color, reset_clip_on_click, sync_faders,
};
use super::panel::{CachedPanel, PanelHost, PublishRef};
use super::*;
use crate::service::{GraphNodeState, GraphSnapshot};
use crate::ui::monitoring::DisplayLevel;
use crate::ui::presentation::db_text;
use crate::ui::shared_frame::SharedFrame;
use crate::ui::widgets::fader::fader;
use crate::ui::widgets::meters::paint_meter_fill;
use moiren_core::graph::{NodeId, NodeKind};

const STRIP_WIDTH: f32 = 104.;
const FLYOUT_HEIGHT: f32 = 372.;
const METER_BAR_WIDTH: f32 = 7.;
const METER_HEIGHT: f32 = 176.;
/// The clip lamp sits above the bars (the loud end), the same convention as
/// the main mixer: 3 px lamp, 7 px gap.
const METER_LAMP_HEIGHT: f32 = 10.;

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
    faders: BTreeMap<NodeId, SliderSlot>,
    focus: FocusHandle,
    /// Where each strip's meter track sits, registered by the strips'
    /// canvases during a rebuild and read by the hot layer at paint time.
    geometry: SharedFrame<MeterTracks>,
    panel: Entity<TrayPanelHost>,
    meter_layer: Entity<FlyoutMeterLayer>,
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
        let this = cx.entity();
        let panel = cx.new(|cx| TrayPanelHost::new(TrayPanel, this.clone(), cx));
        // One geometry frame shared by both halves of the pipeline: the
        // strips' canvases publish into it and the hot surface reads it. Two
        // separate frames would leave the flyout without any meter bars.
        let geometry = SharedFrame::default();
        let meters = std::rc::Rc::new(std::cell::RefCell::new(FlyoutMeters::new(
            this.downgrade(),
            geometry.clone(),
        )));
        let meter_layer = cx.new(|_| FlyoutMeterLayer::new(meters));
        Self {
            state,
            desktop,
            faders: BTreeMap::new(),
            focus,
            geometry,
            panel,
            meter_layer,
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

    /// Advance the shared meter observations once; every rendered strip
    /// counts as visible because the flyout shows no other content. The
    /// desktop poll calls this while the flyout is at rest; the hot layer
    /// calls it per frame while motion lasts.
    pub(super) fn pump_meters(&mut self, cx: &mut Context<Self>) -> bool {
        self.state.update(cx, |state, _| {
            // Iterate the fader rows straight through: collecting the visible
            // set here would allocate on every pump tick (HC-14).
            state
                .node_meters
                .update(&state.snapshot, self.faders.keys().copied())
        })
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
    fn render(&mut self, _window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tray-mixer")
            .track_focus(&self.focus)
            .on_key_down(|event: &KeyDownEvent, window: &mut Window, _: &mut App| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            })
            .size_full()
            .relative()
            .child(
                self.panel
                    .clone()
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element(),
            )
            .child(self.meter_layer.clone())
    }
}

#[derive(PartialEq)]
pub(in crate::ui::window) struct TrayKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
}

/// The flyout's strip panel: faders, mute/solo, and readouts, rebuilt only
/// when the published snapshot changes.
pub(in crate::ui::window) struct TrayPanel;

pub(in crate::ui::window) type TrayPanelHost = PanelHost<TrayPanel>;

impl CachedPanel for TrayPanel {
    type Owner = TrayMixer;
    type Key = TrayKey;

    fn key(&self, owner: &Self::Owner, cx: &App) -> Self::Key {
        Self::Key {
            snapshot: PublishRef::new(Arc::clone(&owner.state.read(cx).snapshot)),
        }
    }

    fn render(
        &self,
        owner: &mut Self::Owner,
        window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement {
        owner.sync_faders(window, cx);
        let state = owner.state.read(cx);
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
            strips = strips.child(strip(owner, entry, node.kind(), cx));
        }

        div()
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
                    .child(open_button(cx)),
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
            .into_any_element()
    }

    fn on_rebuild(&self, owner: &mut Self::Owner) {
        // Geometry is registered by the strip canvases during this same
        // layout pass; stale placements must not survive a rebuild.
        owner.geometry.reset();
    }

    fn observe_nested(
        &self,
        owner: &Entity<Self::Owner>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // SliderState is a model entity, not a Render view in the dispatch
        // tree; its notifications cannot invalidate this cached ancestor by
        // themselves.
        let sliders = owner.read_with(cx, |owner, _| {
            owner
                .faders
                .values()
                .map(|slot| slot.state.clone())
                .collect::<Vec<_>>()
        });
        sliders
            .into_iter()
            .map(|slider| cx.observe(&slider, |host, _, cx| host.notify_nested(cx)))
            .collect()
    }
}

fn open_button(cx: &Context<TrayMixer>) -> impl IntoElement {
    let this = cx.entity();
    button("tray-mixer-open", t!("Open"))
        .variant(ButtonVariant::Ghost)
        .on_click(move |_, _, cx| {
            this.update(cx, |flyout, cx| {
                if let Some(desktop) = flyout.desktop.clone() {
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
                        // The main mixer's display name: an export binding,
                        // the bound input device or output endpoint — never
                        // the bare default label ("Input"/"Output").
                        .child(crate::ui::presentation::display_label(
                            &flyout.state.read(cx).snapshot,
                            entry,
                            kind,
                        )),
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
                .child(meter_track(flyout, id))
                .on_mouse_down(
                    MouseButton::Left,
                    reset_clip_on_click(flyout.state.clone(), cx.entity(), id),
                )
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

/// The strip's meter footprint: a painting-free canvas that registers the
/// track's bounds for the hot layer, sized like the DOM meter it replaced.
fn meter_track(flyout: &TrayMixer, id: NodeId) -> impl IntoElement {
    let geometry = flyout.geometry.clone();
    canvas(
        move |bounds, window, _| {
            geometry.update(|geometry| geometry.register(id, bounds, window.content_mask()));
        },
        |_, _, _, _| {},
    )
    .w(px(METER_BAR_WIDTH * 2. + 2.))
    .h(px(METER_HEIGHT + METER_LAMP_HEIGHT))
    .flex_none()
}

/// The flyout's hot surface: live bars over the cached strips, painting from
/// the geometry the strips registered and the shared observations.
pub(in crate::ui::window) struct FlyoutMeters {
    flyout: WeakEntity<TrayMixer>,
    geometry: SharedFrame<MeterTracks>,
    /// How many registered tracks this surface has painted. Test-visible
    /// census: a frame with registered tracks but zero paints means the
    /// layer is not drawing (the regression that muted the flyout).
    painted_tracks: std::cell::Cell<usize>,
}

impl FlyoutMeters {
    pub(in crate::ui::window) fn new(
        flyout: WeakEntity<TrayMixer>,
        geometry: SharedFrame<MeterTracks>,
    ) -> Self {
        Self {
            flyout,
            geometry,
            painted_tracks: std::cell::Cell::new(0),
        }
    }

    /// Tracks the sibling panel registered for this surface. Empty means the
    /// two halves of the pipeline drifted apart and the flyout has no meters.
    #[cfg(all(test, feature = "gpui-test"))]
    pub(in crate::ui::window) fn registered_tracks(&self) -> usize {
        self.geometry.read().map_or(0, |frame| frame.iter().count())
    }

    #[cfg(all(test, feature = "gpui-test"))]
    pub(in crate::ui::window) fn painted_tracks(&self) -> usize {
        self.painted_tracks.get()
    }
}

pub(in crate::ui::window) type FlyoutMeterLayer = HotLayerView<FlyoutMeters>;

impl HotSurface for FlyoutMeters {
    type Frame = MeterTracks;

    fn pump(&mut self, _window: &mut Window, cx: &mut App) -> bool {
        let Some(flyout) = self.flyout.upgrade() else {
            return false;
        };
        flyout.update(cx, |flyout, cx| {
            flyout.pump_meters(cx);
            let state = flyout.state.read(cx);
            state.snapshot.phase == SessionPhase::Running && !state.node_meters.at_rest()
        })
    }

    fn frame(&self) -> &SharedFrame<MeterTracks> {
        &self.geometry
    }

    fn paint(&self, frame: &MeterTracks, window: &mut Window, cx: &mut App) {
        let Some(flyout) = self.flyout.upgrade() else {
            return;
        };
        flyout.update(cx, |flyout, cx| {
            for (id, bounds, mask) in frame.iter() {
                let (levels, clips) = {
                    let state = flyout.state.read(cx);
                    (state.node_meters.levels(id), state.node_meters.clips(id))
                };
                window.with_content_mask(Some(mask), |window| {
                    paint_flyout_meter(bounds, levels, clips, window);
                });
                self.painted_tracks.set(self.painted_tracks.get() + 1);
            }
        });
    }
}

/// Two slim bars with a clip lamp above each — the flyout's meter
/// footprint, painting the same `meter_spans` segmentation as every other
/// surface (design 17 D1) under the main mixer's clip convention: the lamp
/// sits at the loud end of the track and lights while a clip latches;
/// clicking the strip resets it.
fn paint_flyout_meter(
    bounds: Bounds<Pixels>,
    levels: Option<[DisplayLevel; 2]>,
    clips: [bool; 2],
    window: &mut Window,
) {
    let bar_width = px(METER_BAR_WIDTH);
    for (channel, clip) in clips.iter().enumerate() {
        let left = bounds.left() + px(channel as f32 * (METER_BAR_WIDTH + 2.));
        let track = Bounds::new(
            point(left, bounds.top() + px(METER_LAMP_HEIGHT)),
            size(bar_width, px(METER_HEIGHT)),
        );
        match levels.as_ref().and_then(|levels| levels.get(channel)) {
            Some(level) => paint_meter_fill(track, *level, theme::BORDER, window),
            None => window.paint_quad(fill(track, theme::BORDER)),
        }
        crate::ui::widgets::meters::paint_clip_lamp(track, Axis::Vertical, *clip, window);
    }
}
