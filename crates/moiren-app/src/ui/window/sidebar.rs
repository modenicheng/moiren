//! Global side panels. The inspector host on the right is shared by every
//! page at one fixed width; the node-library host on the left belongs to the
//! graph page. Both collapse to a protruding edge handle that flips its
//! chevron, and both animate their width with a spring so pages never see the
//! panel jump between widths.

use super::hot::{HotLayerView, HotSurface, MeterTracks};
use super::*;
use crate::ui::monitoring::DisplayLevel;
use crate::ui::shared_frame::SharedFrame;
use crate::ui::window::panel::{CachedPanel, PanelHost, PublishRef, cached_splice};

/// The inspector panel behind a [`PanelHost`]: rebuilt only when its inputs
/// changed — the published snapshot, the selection, the active editing
/// session, and the graph/library mode flags.
pub(in crate::ui::window) struct InspectorPanel;

#[derive(PartialEq)]
pub(in crate::ui::window) struct InspectorKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
    selected_node: Option<moiren_core::graph::NodeId>,
    selected_route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
    device_tab: devices::DeviceTab,
    selected_device: Option<String>,
    node_name_editing: Option<moiren_core::graph::NodeId>,
    editing_value: Option<moiren_core::graph::NodeId>,
    editing_balance: Option<moiren_core::graph::NodeId>,
    editing_node_gain: Option<moiren_core::graph::NodeId>,
    editing_node_pan: Option<moiren_core::graph::NodeId>,
    editing_send: Option<moiren_core::graph::EdgeId>,
    compact: bool,
    preferences: crate::project::AppPreferences,
}

pub(in crate::ui::window) type InspectorPanelHost = PanelHost<InspectorPanel>;

impl CachedPanel for InspectorPanel {
    type Owner = StudioShell;
    type Key = InspectorKey;

    fn key(&self, shell: &Self::Owner, cx: &App) -> Self::Key {
        let state = shell.state.read(cx);
        Self::Key {
            snapshot: PublishRef::new(std::sync::Arc::clone(&state.snapshot)),
            selected_node: shell.selected_node,
            selected_route: shell.selected_route,
            device_tab: shell.device_tab,
            selected_device: shell.selected_device.clone(),
            node_name_editing: shell.node_name_editing,
            editing_value: shell.mixer.editing_value,
            editing_balance: shell.mixer.editing_balance,
            editing_node_gain: shell.mixer.editing_node_gain,
            editing_node_pan: shell.mixer.editing_node_pan,
            editing_send: shell.mixer.editing_send,
            compact: shell.compact,
            preferences: shell.preferences.clone(),
        }
    }

    fn render(
        &self,
        shell: &mut Self::Owner,
        _window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement {
        page_inspector(shell, cx)
    }

    fn on_rebuild(&self, shell: &mut Self::Owner) {
        // The inspector's canvases register their meter tracks during this
        // same layout pass; stale placements must not survive a rebuild.
        shell.inspector_meters.reset();
    }

    fn observe_nested(
        &self,
        owner: &Entity<Self::Owner>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // Model entities nested in the panel — the inline rename input, the
        // mixer's editable-value inputs, and the export drafts — whose
        // notifications cannot invalidate this cached ancestor themselves.
        let inputs = owner.read_with(cx, |shell, _| {
            let mut inputs = vec![shell.node_name.clone()];
            inputs.extend(shell.mixer.value_inputs.values().cloned());
            inputs.extend(shell.export_controls.input_entities());
            inputs
        });
        inputs
            .into_iter()
            .map(|input| cx.observe(&input, |host, _, cx| host.notify_nested(cx)))
            .collect()
    }
}

/// The inspector panel's cached splice.
fn inspector_panel_for(shell: &StudioShell, _window: &Window) -> AnyElement {
    cached_splice(shell.inspector_panel.clone(), |entity| {
        entity
            .cached(StyleRefinement::default().size_full())
            .into_any_element()
    })
}

/// The node-library panels behind [`PanelHost`]s: the icon rail and the
/// detail sleeve, rebuilt only when their inputs changed.
pub(in crate::ui::window) struct LibraryPanel;

#[derive(PartialEq)]
pub(in crate::ui::window) struct LibraryKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
    pending_spawn: bool,
    compact: bool,
    preferences: crate::project::AppPreferences,
}

fn library_key(shell: &StudioShell, cx: &App) -> LibraryKey {
    LibraryKey {
        snapshot: PublishRef::new(std::sync::Arc::clone(&shell.state.read(cx).snapshot)),
        pending_spawn: shell.pending_node_spawn.is_some(),
        compact: shell.compact,
        preferences: shell.preferences.clone(),
    }
}

pub(in crate::ui::window) type LibraryPanelHost = PanelHost<LibraryPanel>;

impl CachedPanel for LibraryPanel {
    type Owner = StudioShell;
    type Key = LibraryKey;

    fn key(&self, shell: &Self::Owner, cx: &App) -> Self::Key {
        library_key(shell, cx)
    }

    fn render(
        &self,
        shell: &mut Self::Owner,
        _window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement {
        super::graph::library(shell, cx).into_any_element()
    }

    fn observe_nested(
        &self,
        owner: &Entity<Self::Owner>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        let search = owner.read_with(cx, |shell, _| shell.node_search.clone());
        vec![cx.observe(&search, |host, _, cx| host.notify_nested(cx))]
    }
}

/// The library's icon rail: same inputs as the detail sleeve, no search.
pub(in crate::ui::window) struct LibraryRailPanel;

pub(in crate::ui::window) type LibraryRailPanelHost = PanelHost<LibraryRailPanel>;

impl CachedPanel for LibraryRailPanel {
    type Owner = StudioShell;
    type Key = LibraryKey;

    fn key(&self, shell: &Self::Owner, cx: &App) -> Self::Key {
        library_key(shell, cx)
    }

    fn render(
        &self,
        shell: &mut Self::Owner,
        _window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement {
        super::graph::library_rail(shell, cx).into_any_element()
    }
}

/// The sidebar inspector's hot surface: live horizontal meter bars over the
/// inspector panel, painting from the tracks the inspector's canvases
/// register and the shared observations.
pub(super) struct InspectorMeters {
    shell: WeakEntity<StudioShell>,
    geometry: SharedFrame<MeterTracks>,
}

impl InspectorMeters {
    pub(super) fn new(shell: WeakEntity<StudioShell>, geometry: SharedFrame<MeterTracks>) -> Self {
        Self { shell, geometry }
    }
}

pub(super) type InspectorMeterLayer = HotLayerView<InspectorMeters>;
pub(super) type InspectorMeterFrame = SharedFrame<MeterTracks>;

impl HotSurface for InspectorMeters {
    type Frame = MeterTracks;

    fn pump(&mut self, _: &mut Window, _: &mut App) -> bool {
        // The inspector never sustains its own loop: the page's hot layers
        // own this window's frame demand and their animation frames carry
        // these bars; at rest the control-side poll redraws them.
        false
    }

    fn frame(&self) -> &SharedFrame<MeterTracks> {
        &self.geometry
    }

    fn paint(&self, frame: &MeterTracks, window: &mut Window, cx: &mut App) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        shell.update(cx, |shell, cx| {
            for (id, bounds, mask) in frame.iter() {
                let (channels, levels, clips) = {
                    let state = shell.state.read(cx);
                    (
                        state
                            .snapshot
                            .graph
                            .graph
                            .get_node(id)
                            .map(|node| node.channels())
                            .unwrap_or(1),
                        state.node_meters.levels(id),
                        state.node_meters.clips(id),
                    )
                };
                window.with_content_mask(Some(mask), |window| {
                    paint_inspector_meter(bounds, channels, levels, clips, window);
                });
            }
        });
    }
}

/// Slim horizontal bars per channel and a shared clip dot below, the
/// inspector's meter footprint; the shared segmentation math (design 17 D1)
/// comes from the meter painters.
fn paint_inspector_meter(
    bounds: Bounds<Pixels>,
    channels: usize,
    levels: Option<[DisplayLevel; 2]>,
    clips: [bool; 2],
    window: &mut Window,
) {
    let count = channels.clamp(1, 2);
    let (row_height, gap) = (px(4.), px(2.));
    // The clip lamp reserves the last 6 px of the row (3 px gap + 3 px
    // lamp), the same loud-end indicator every meter surface uses.
    let bar_width = bounds.size.width - px(6.);
    for channel in 0..count {
        let top = bounds.top() + (row_height + gap) * channel as f32;
        let track = Bounds::new(point(bounds.left(), top), size(bar_width, row_height));
        match levels.as_ref().and_then(|levels| levels.get(channel)) {
            Some(level) => crate::ui::widgets::meters::paint_meter_fill_horizontal(
                track,
                *level,
                theme::BORDER,
                window,
            ),
            None => window.paint_quad(fill(track, theme::BORDER)),
        }
        crate::ui::widgets::meters::paint_clip_lamp(
            track,
            Axis::Horizontal,
            clips.get(channel).copied().unwrap_or(false),
            window,
        );
    }
}

/// Panel motion: quick with a slight, damped overshoot and no wobble.
const PANEL_SPRING: SpringConfig = SpringConfig::new(320., 30., 1.);
const HANDLE_WIDTH: f32 = 18.;
const HANDLE_HEIGHT: f32 = 56.;

/// Right-hand inspector host: page content plus the collapse handle that
/// rides the panel's left edge.
pub(super) fn inspector_host(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let this = cx.entity();
    div()
        .id("inspector-host")
        .relative()
        .flex_none()
        .h_full()
        .child(panel(
            "inspector-panel-width",
            shell.inspector_open,
            theme::INSPECTOR_WIDTH,
            inspector_panel_for(shell, window),
        ))
        .child(shell.inspector_meter_layer.clone())
        .child(edge_handle(
            "inspector-toggle",
            shell.inspector_open,
            Side::Right,
            this,
        ))
}

/// Left-hand node-library host for the graph page. The icon rail is the
/// collapsed form: it grows in from zero only while the detail sleeve is
/// closed, so the two springs swap one for the other in the same motion.
pub(super) fn library_host(
    shell: &StudioShell,
    _window: &Window,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let this = cx.entity();
    div()
        .id("library-host")
        .relative()
        .flex()
        .flex_none()
        .h_full()
        .child(panel(
            "library-rail-width",
            !shell.sidebar_open,
            theme::LIBRARY_RAIL_WIDTH,
            cached_splice(shell.library_rail_panel.clone(), |entity| {
                entity
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element()
            }),
        ))
        .child(panel(
            "library-panel-width",
            shell.sidebar_open,
            theme::LIBRARY_WIDTH,
            cached_splice(shell.library_panel.clone(), |entity| {
                entity
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element()
            }),
        ))
        .child(edge_handle(
            "library-toggle",
            shell.sidebar_open,
            Side::Left,
            this,
        ))
}

enum Side {
    Left,
    Right,
}

/// The animated width sleeve. The inner content keeps its full fixed width so
/// text never reflows mid-animation; the sleeve clips it instead.
fn panel(
    spring_id: &'static str,
    open: bool,
    width: f32,
    content: AnyElement,
) -> SpringAnimationElement<Div> {
    div()
        .h_full()
        .overflow_hidden()
        .with_spring(
            spring_id,
            SpringAnimation::new(PANEL_SPRING).to(if open { width } else { 0. }),
            |el, width| el.w(px(width.max(0.))),
        )
        .child(div().w(px(width)).h_full().child(content))
}

/// The protruding tab attached to a panel edge. Collapsed it hugs the page
/// border and points toward the panel; expanded it rides the panel edge and
/// points the other way.
fn edge_handle(
    id: &'static str,
    open: bool,
    side: Side,
    this: Entity<StudioShell>,
) -> impl IntoElement {
    // A left panel's handle sits on its right edge and points with the panel;
    // a right panel's handle mirrors that on its left edge. The attached side
    // stays square; the protruding side rounds off.
    let (handle, icon) = match side {
        Side::Left => (
            div()
                .right(px(-(HANDLE_WIDTH + 2.)))
                .rounded_r(px(theme::RADIUS_MD)),
            if open {
                "icons/chevron-left.svg"
            } else {
                "icons/chevron-right.svg"
            },
        ),
        Side::Right => (
            div()
                .left(px(-(HANDLE_WIDTH + 2.)))
                .rounded_l(px(theme::RADIUS_MD)),
            if open {
                "icons/chevron-right.svg"
            } else {
                "icons/chevron-left.svg"
            },
        ),
    };
    handle
        .id(id)
        .absolute()
        .top(relative(0.5))
        .mt(px(-HANDLE_HEIGHT / 2.))
        .w(px(HANDLE_WIDTH))
        .h(px(HANDLE_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::SURFACE_3)
        .border_1()
        .border_color(theme::BORDER_STRONG)
        .text_color(theme::PRIMARY)
        .cursor_pointer()
        .hover(|style| style.bg(theme::SURFACE_4))
        .child(
            component::Icon::default()
                .path(icon)
                .size(px(14.))
                .text_color(theme::PRIMARY),
        )
        .on_click(move |_, _, cx| {
            this.update(cx, |shell, cx| {
                match side {
                    Side::Left => shell.sidebar_open = !shell.sidebar_open,
                    Side::Right => {
                        shell.inspector_open = !shell.inspector_open;
                        if shell.compact {
                            shell.sidebar_open = false;
                        }
                    }
                }
                cx.notify();
            });
        })
        .gpui_test_observe()
}

/// Inspector content follows the active page; the shell guarantees one
/// consistent host width across all of them.
fn page_inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    match shell.presentation.page {
        Page::Graph => {
            if shell.selected_node.is_none() && shell.selected_route.is_some() {
                matrix::inspector(shell, cx)
            } else {
                graph::inspector(shell, cx)
            }
        }
        Page::Mixer => graph::inspector(shell, cx),
        Page::Matrix => matrix::inspector(shell, cx),
        Page::Devices => devices::inspector(shell, cx),
    }
}
