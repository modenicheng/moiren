//! Global side panels. The inspector host on the right is shared by every
//! page at one fixed width; the node-library host on the left belongs to the
//! graph page. Both collapse to a protruding edge handle that flips its
//! chevron, and both animate their width with a spring so pages never see the
//! panel jump between widths.

use super::hot::{HotLayerView, HotSurface, MeterTracks};
use super::*;
use crate::ui::monitoring::DisplayLevel;
use crate::ui::shared_frame::SharedFrame;
use crate::ui::shell::panel::{CachedPanel, PanelHost, PublishRef};
use crate::ui::widgets::overlay_scrollbar::overlay_scrollbar;
use gpui_kit::base::ScrollbarAxis;

/// The inspector panel behind a [`PanelHost`]: rebuilt only when its inputs
/// changed — the published snapshot, the selection, the active page, the
/// active editing session, and the graph/library mode flags.
pub(crate) struct InspectorPanel;

#[derive(PartialEq)]
pub(crate) struct InspectorKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
    selected_node: Option<moiren_core::graph::NodeId>,
    selected_route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
    // `page_inspector` switches its whole tree on the active page (graph/mixer
    // node editors vs the matrix router vs the device inspector), so a page
    // switch must rebuild even when a selection rides along unchanged; keyed
    // without it the previous page's inspector froze until the next edit.
    page: Page,
    device_tab: devices::DeviceTab,
    selected_device: Option<String>,
    /// The device-override rows and the save footer render from the same
    /// settings-editor device draft as the Devices page, so the same edit
    /// revision (`Draft::revision`) keys them here — see `DevicesKey`'s
    /// field for why the revision stands in for the draft value.
    device_revision: u64,
    // TODO(#118 follow-up): the node-observation readouts (`node_observation`
    // in `graph/node_inspector.rs`) render from `node_meters`, which the
    // metering path updates without notifying the shell — keying them needs
    // a notify-path change, not just another key field, so they stay unkeyed
    // here for now.
    node_name_editing: Option<moiren_core::graph::NodeId>,
    editing_value: Option<moiren_core::graph::NodeId>,
    editing_balance: Option<moiren_core::graph::NodeId>,
    editing_node_gain: Option<moiren_core::graph::NodeId>,
    editing_node_pan: Option<moiren_core::graph::NodeId>,
    editing_send: Option<moiren_core::graph::EdgeId>,
    compact: bool,
    preferences: crate::project::AppPreferences,
    /// The panel scrolls inside the cached splice, whose paint bakes
    /// coordinates: the offsets must ride the key or a scroll freezes.
    /// `override_scroll` is the device overrides editor nested in the device
    /// inspector, scrolling apart from the panel itself.
    inspector_scroll: Pixels,
    override_scroll: Pixels,
}

pub(crate) type InspectorPanelHost = PanelHost<InspectorPanel>;

impl CachedPanel for InspectorPanel {
    type Owner = StudioShell;
    type Key = InspectorKey;

    fn key(&self, shell: &Self::Owner, cx: &App) -> Self::Key {
        let state = shell.state.read(cx);
        Self::Key {
            snapshot: PublishRef::new(std::sync::Arc::clone(&state.snapshot)),
            selected_node: shell.selection.node,
            selected_route: shell.selection.route,
            page: shell.presentation.page(),
            device_tab: shell.devices.device_tab,
            selected_device: shell.devices.selected_device.clone(),
            device_revision: shell.settings.editor.device.revision,
            node_name_editing: shell.inspector.node_name_editing,
            editing_value: shell.mixer.presentation.editing_value,
            editing_balance: shell.mixer.presentation.editing_balance,
            editing_node_gain: shell.mixer.presentation.editing_node_gain,
            editing_node_pan: shell.mixer.presentation.editing_node_pan,
            editing_send: shell.mixer.presentation.editing_send,
            compact: shell.chrome.compact,
            preferences: shell.preferences.values.clone(),
            inspector_scroll: shell.inspector.scroll.offset().y,
            override_scroll: shell.devices.device_override_scroll.offset().y,
        }
    }

    fn render(
        &self,
        shell: &mut Self::Owner,
        _window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement {
        #[cfg(feature = "gpui-test")]
        {
            shell.trace.borrow_mut().inspector_renders += 1;
        }
        page_inspector(shell, cx)
    }

    fn on_rebuild(&self, shell: &mut Self::Owner) {
        // The inspector's canvases register their meter tracks during this
        // same layout pass; stale placements must not survive a rebuild.
        shell.inspector.meters.reset();
    }

    fn observe_nested(
        &self,
        owner: &Entity<Self::Owner>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // Model entities nested in the panel whose notifications cannot
        // invalidate this cached ancestor themselves: every text-input family
        // the shell can render here (inline rename, the mixer's click-to-type
        // editors, the compressor fields, the export drafts) rides the shell's
        // single walk, and the inspector's own hslider slots — the channel
        // strip's fader row and the compressor parameter sliders — are
        // SliderStates only this panel renders, so they join the pool here.
        // Missing any family leaves that editor blind-typing inside the
        // cached subtree: its keystrokes update the model but no rebuild
        // carries them to the screen.
        let (inputs, sliders) = owner.read_with(cx, |shell, _| {
            let mut sliders = Vec::new();
            sliders.extend(
                shell
                    .mixer
                    .presentation
                    .inspector_faders
                    .values()
                    .map(|slot| slot.state.clone()),
            );
            sliders.extend(
                shell
                    .inspector
                    .compressor
                    .sliders
                    .values()
                    .map(|slot| slot.state.clone()),
            );
            (shell.text_input_entities(), sliders)
        });
        let mut subscriptions: Vec<Subscription> = inputs
            .into_iter()
            .map(|input| cx.observe(&input, |host, _, cx| host.notify_nested(cx)))
            .collect();
        subscriptions.extend(
            sliders
                .into_iter()
                .map(|slider| cx.observe(&slider, |host, _, cx| host.notify_nested(cx))),
        );
        subscriptions
    }
}

/// The inspector panel's cached splice, under the pinned bar overlay: the
/// overlay is a sibling of the splice, never inside it — a cached panel
/// replays paint without re-running closures, so a bar inside would freeze
/// and lose its drag target on cache hits.
fn inspector_panel_for(shell: &StudioShell, _window: &Window) -> AnyElement {
    div()
        .relative()
        .size_full()
        .child(
            shell
                .inspector
                .panel
                .clone()
                .cached(StyleRefinement::default().size_full())
                .into_any_element(),
        )
        .child(overlay_scrollbar(
            &shell.inspector.scroll,
            "scrollbar-inspector-panel",
            ScrollbarAxis::Vertical,
        ))
        .into_any_element()
}

/// The node-library panels behind [`PanelHost`]s: the icon rail and the
/// detail sleeve, rebuilt only when their inputs changed.
pub(crate) struct LibraryPanel;

#[derive(PartialEq)]
pub(crate) struct LibraryKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
    // The session-node list styles each row as selected from this field
    // (graph::library), so a selection change must rebuild the list even when
    // no publish rides along; keyed without it the selected style froze on
    // the previous row until the next snapshot. The rail renders no selection
    // and merely rides along — selection changes are discrete user actions.
    selected_node: Option<moiren_core::graph::NodeId>,
    pending_spawn: bool,
    compact: bool,
    preferences: crate::project::AppPreferences,
    /// The list and the rail scroll inside their cached splices: offsets
    /// ride the key (see `InspectorKey`'s scroll fields).
    scroll: Pixels,
    rail_scroll: Pixels,
}

fn library_key(shell: &StudioShell, cx: &App) -> LibraryKey {
    LibraryKey {
        snapshot: PublishRef::new(std::sync::Arc::clone(&shell.state.read(cx).snapshot)),
        selected_node: shell.selection.node,
        pending_spawn: shell.graph_view.pending_node_spawn.is_some(),
        compact: shell.chrome.compact,
        preferences: shell.preferences.values.clone(),
        scroll: shell.library.scroll.offset().y,
        rail_scroll: shell.library.rail_scroll.offset().y,
    }
}

pub(crate) type LibraryPanelHost = PanelHost<LibraryPanel>;

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
        let search = owner.read_with(cx, |shell, _| shell.graph_view.node_search.clone());
        vec![cx.observe(&search, |host, _, cx| host.notify_nested(cx))]
    }
}

/// The library's icon rail: same inputs as the detail sleeve, no search.
pub(crate) struct LibraryRailPanel;

pub(crate) type LibraryRailPanelHost = PanelHost<LibraryRailPanel>;

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
pub(crate) struct InspectorMeters {
    shell: WeakEntity<StudioShell>,
    geometry: SharedFrame<MeterTracks>,
    /// How many registered tracks this surface has painted. Test-visible
    /// census: a frame with registered tracks but zero paints means the
    /// layer is not drawing — the inspector's variant of the flyout's
    /// meter-less regression (design 17 hazard 9, audit HY-10).
    painted_tracks: std::cell::Cell<usize>,
}

impl InspectorMeters {
    pub(super) fn new(shell: WeakEntity<StudioShell>, geometry: SharedFrame<MeterTracks>) -> Self {
        Self {
            shell,
            geometry,
            painted_tracks: std::cell::Cell::new(0),
        }
    }

    /// Tracks the inspector's canvases registered for this surface. Empty
    /// means the assembly handed this surface a frame of its own instead of
    /// the panel's: it compiles, runs, and paints nothing forever.
    #[cfg(all(test, feature = "gpui-test"))]
    pub(crate) fn registered_tracks(&self) -> usize {
        self.geometry.read().map_or(0, |frame| frame.iter().count())
    }

    #[cfg(all(test, feature = "gpui-test"))]
    pub(crate) fn painted_tracks(&self) -> usize {
        self.painted_tracks.get()
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

    fn paint(
        &self,
        frame: &MeterTracks,
        _layer: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
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
                self.painted_tracks.set(self.painted_tracks.get() + 1);
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
            shell.chrome.inspector_open,
            theme::INSPECTOR_WIDTH,
            inspector_panel_for(shell, window),
        ))
        .child(shell.inspector.meter_layer.clone())
        .child(edge_handle(
            "inspector-toggle",
            shell.chrome.inspector_open,
            Side::Right,
            this,
            None,
        ))
}

/// Left-hand node-library host for the graph page. The icon rail is the
/// collapsed form: it grows in from zero only while the detail sleeve is
/// closed, so the two springs swap one for the other in the same motion.
/// The inspector column's shell-owned state (design 20 §4): the rename
/// editor's input entity and target, the compressor parameter controls,
/// the meter-track frame and its hot layer, the panel's scroll, and the
/// cached InspectorPanel host.
pub(crate) struct InspectorState {
    pub(crate) node_name: Entity<component::input::InputState>,
    /// Node whose label is being edited inline in the inspector.
    pub(crate) node_name_editing: Option<moiren_core::graph::NodeId>,
    pub(crate) compressor: crate::ui::pages::graph::CompressorControls,
    pub(crate) meters: InspectorMeterFrame,
    pub(crate) meter_layer: Entity<InspectorMeterLayer>,
    pub(crate) scroll: ScrollHandle,
    pub(crate) panel: Entity<InspectorPanelHost>,
}

/// The node library's shell-owned state (design 20 §4): both hosts' scroll
/// containers and their cached panel hosts.
pub(crate) struct LibraryState {
    pub(crate) scroll: ScrollHandle,
    pub(crate) rail_scroll: ScrollHandle,
    pub(crate) panel: Entity<LibraryPanelHost>,
    pub(crate) rail_panel: Entity<LibraryRailPanelHost>,
}

pub(crate) fn library_host(
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
            !shell.chrome.sidebar_open,
            theme::LIBRARY_RAIL_WIDTH,
            deselecting(
                div()
                    .relative()
                    .size_full()
                    .child(
                        shell
                            .library
                            .rail_panel
                            .clone()
                            .cached(StyleRefinement::default().size_full())
                            .into_any_element(),
                    )
                    .child(overlay_scrollbar(
                        &shell.library.rail_scroll,
                        "scrollbar-library-rail",
                        ScrollbarAxis::Vertical,
                    ))
                    .into_any_element(),
                &this,
            ),
        ))
        .child(panel(
            "library-panel-width",
            shell.chrome.sidebar_open,
            theme::LIBRARY_WIDTH,
            deselecting(
                div()
                    .relative()
                    .size_full()
                    .child(
                        shell
                            .library
                            .panel
                            .clone()
                            .cached(StyleRefinement::default().size_full())
                            .into_any_element(),
                    )
                    .child(overlay_scrollbar(
                        &shell.library.scroll,
                        "scrollbar-node-library",
                        ScrollbarAxis::Vertical,
                    ))
                    .into_any_element(),
                &this,
            ),
        ))
        .child(edge_handle(
            "library-toggle",
            shell.chrome.sidebar_open,
            Side::Left,
            this,
            // The handle protrudes over the canvas inside the cached panel;
            // the graph meter frame paints around its registered rect.
            Some(
                shell
                    .graph_view
                    .overlays
                    .probe("library-handle")
                    .into_any_element(),
            ),
        ))
}

enum Side {
    Left,
    Right,
}

/// A press anywhere on the node library — a card about to spawn or be
/// dragged, the collapsed rail, the search — drops the canvas selection,
/// so the node that spawns is the only thing the drag that usually follows
/// can move. Capture phase: the cards' own click and drag handlers must not
/// be able to swallow the press first. The inspector deliberately has no
/// such wrapper: pressing it edits the selection, never clears it.
fn deselecting(content: AnyElement, this: &Entity<StudioShell>) -> AnyElement {
    let this = this.clone();
    div()
        .size_full()
        .capture_any_mouse_down(move |event, _, cx| {
            if event.button != MouseButton::Left {
                return;
            }
            this.update(cx, |shell, cx| {
                if shell.selection.node.is_some()
                    || !shell.selection.nodes.is_empty()
                    || shell.selection.route.is_some()
                {
                    shell.clear_canvas_selection();
                    cx.notify();
                }
            });
        })
        .child(content)
        .into_any_element()
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
    probe: Option<AnyElement>,
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
        .when_some(probe, |handle, probe| handle.child(probe))
        .child(
            component::Icon::default()
                .path(icon)
                .size(px(14.))
                .text_color(theme::PRIMARY),
        )
        .on_click(move |_, _, cx| {
            this.update(cx, |shell, cx| {
                match side {
                    Side::Left => shell.chrome.sidebar_open = !shell.chrome.sidebar_open,
                    Side::Right => {
                        shell.chrome.inspector_open = !shell.chrome.inspector_open;
                        if shell.chrome.compact {
                            shell.chrome.sidebar_open = false;
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
    match shell.presentation.page() {
        Page::Graph => {
            if shell.selection.node.is_none() && shell.selection.route.is_some() {
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

#[cfg(all(test, feature = "gpui-test"))]
mod tests;
