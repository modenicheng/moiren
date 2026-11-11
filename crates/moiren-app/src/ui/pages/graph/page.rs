//! The Graph page assembly: cached page content behind a [`PanelHost`] and
//! the display-rate meter layer behind a [`HotLayerView`]: the meter canvas
//! is a sibling, never a descendant of the cached subtree — gpui-fast
//! replays a cached panel's paint without re-running its closures, so live
//! bars must be painted by the hot layer at the frame the canvas prepaint
//! published.
use super::*;
use crate::ui::pages::graph::graph_canvas::CanvasFrame;
use crate::ui::shell::hot::{HotLayerView, HotSurface};
use crate::ui::shell::panel::{ArcSnapshot, CachedPanel, PanelHost};
use std::rc::Rc;

/// The Graph page panel behind [`PanelHost`]: the canvas world, library, and
/// editing toolbar, rebuilt only when their semantic key changes.
pub(crate) struct GraphPanel;

/// Only inputs the page content consumes belong in this key. Elapsed time
/// and audio observations must not invalidate it: the shell root still
/// rebuilds on every drawn frame, and those rebuilds must splice this cache
/// instead of re-projecting the graph.
///
/// Gesture in-flights ride the key by identity, never by cursor: a card
/// drag's live offset and a connection drag's cursor change every pointer
/// report, and keying them would rebuild the whole page per display frame
/// (audit A-1). The dragged entities leave the cached canvas at drag start
/// (this key flips) and paint through the hot layer until the drag ends;
/// the offset itself is read at paint time.
#[derive(PartialEq)]
pub(crate) struct ControlsKey {
    graph: ArcSnapshot<crate::service::GraphSnapshot>,
    // The canvas world projects the cards' binding chips from the catalog
    // (`CanvasWorld::positioned`), so a catalog-only republish must rebuild
    // the page even while the graph compares equal; the ArcSnapshot keeps
    // the usual publish on the pointer fast path.
    catalog: ArcSnapshot<crate::service::DeviceCatalog>,
    // Which rows a catalog offers (WASAPI endpoints vs ASIO drivers) flips
    // with the backend discriminator, a small Copy enum — keyed directly.
    backend: moiren_engine::audio_config::AudioBackend,
    generation: crate::service::SessionGeneration,
    phase: SessionPhase,
    desired: Option<crate::service::SessionSpec>,
    positions: std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    camera: Camera,
    canvas: Option<(f32, f32)>,
    selected_nodes: Vec<moiren_core::graph::NodeId>,
    selected_route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
    // The connection drag's identity: which port it pulls from and — when
    // it picked an edge up off its input — that input. The cursor moves per
    // frame and paints through the hot layer instead.
    connection_drag: Option<(
        crate::ui::pages::graph::graph_canvas::PortTarget,
        Option<moiren_core::graph::PortId>,
    )>,
    // A card drag is in flight: the cached canvas hides the dragged
    // selection and its wires for the gesture's whole duration (they paint
    // through the hot layer at the live offset), so the flag must flip the
    // key exactly at drag start and end.
    node_drag: bool,
    // Snap alignment guides: their world coordinate changes only when the
    // snap target does, so each transition costs one rebuild — not one per
    // pointer report.
    guides: crate::ui::pages::graph::graph_canvas::DragGuides,
    hand_tool: bool,
    sidebar_open: bool,
    compact: bool,
    preferences: crate::project::AppPreferences,
}

impl ControlsKey {
    fn new(shell: &StudioShell, cx: &App) -> Self {
        let snapshot = &shell.state.read(cx).snapshot;
        Self {
            graph: ArcSnapshot::new(snapshot.graph.clone()),
            catalog: ArcSnapshot::new(snapshot.catalog.clone()),
            backend: snapshot.audio_settings.backend,
            generation: snapshot.generation,
            phase: snapshot.phase,
            desired: snapshot.desired.clone(),
            positions: shell.graph_view.node_positions.clone(),
            camera: shell.graph_view.camera,
            canvas: shell.graph_view.canvas_size,
            selected_nodes: shell.selection.nodes.iter().copied().collect(),
            selected_route: shell.selection.route,
            connection_drag: shell
                .graph_gestures
                .connection_drag
                .map(|drag| (drag.port, drag.detached.map(|picked| picked.destination))),
            node_drag: shell.graph_gestures.node_drag.is_some(),
            guides: shell.graph_gestures.drag_guides.unwrap_or_default(),
            hand_tool: shell.graph_gestures.hand_tool,
            sidebar_open: shell.chrome.sidebar_open,
            compact: shell.chrome.compact,
            preferences: shell.preferences.values.clone(),
        }
    }
}

pub(crate) type GraphPanelHost = PanelHost<GraphPanel>;

impl CachedPanel for GraphPanel {
    type Owner = StudioShell;
    type Key = ControlsKey;

    fn key(&self, shell: &StudioShell, cx: &App) -> Self::Key {
        ControlsKey::new(shell, cx)
    }

    fn render(
        &self,
        shell: &mut StudioShell,
        window: &mut Window,
        cx: &mut Context<StudioShell>,
    ) -> AnyElement {
        #[cfg(feature = "gpui-test")]
        {
            shell.trace.borrow_mut().graph_renders += 1;
        }
        super::page(shell, window, cx)
    }

    fn on_rebuild(&self, shell: &mut StudioShell) {
        // Fresh visibility demand: the canvas prepaint re-registers the
        // nodes its fitted camera shows during this same layout pass.
        shell.metering.meter_points.set((false, false));
        shell.metering.node_meter_points.borrow_mut().clear();
    }

    fn observe_nested(
        &self,
        shell: &Entity<StudioShell>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // The library's search input is a model entity nested in this cached
        // tree; its notifications cannot invalidate the cached ancestor by
        // themselves, so typing must re-render through an explicit observer.
        let search = shell.read_with(cx, |shell, _| shell.graph_view.node_search.clone());
        vec![cx.observe(&search, |host, _, cx| host.notify_nested(cx))]
    }
}

/// The shell's cached canvas world (audit A-1): `CanvasWorld::positioned`
/// re-projects every card — a dozen heap strings per node, a topology walk,
/// per-edge O(n) lookups, per-card curve sampling — and rebuilds used to pay
/// it on every gesture frame even though pan, zoom and drags change nothing
/// the world contains. The cache holds the last projected world together
/// with everything that could have changed it; a rebuild whose inputs all
/// compare equal reuses the `Arc` and re-projects only the cheap camera
/// layout. Every key input is also a `ControlsKey` input (or the pointer's
/// own detached edge), so a stale reuse cannot happen: whenever the world
/// would differ, the key already forced the rebuild that gets to decide.
pub(crate) struct WorldCache {
    graph: ArcSnapshot<crate::service::GraphSnapshot>,
    catalog: ArcSnapshot<crate::service::DeviceCatalog>,
    positions: std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    detached: Option<moiren_core::graph::PortId>,
    locale: crate::i18n::UiLanguage,
    pub(crate) world: std::sync::Arc<crate::ui::pages::graph::graph_canvas::CanvasWorld>,
}

impl WorldCache {
    /// Projects the cache entry for a world built from `snapshot` and
    /// `positions`; the key fields stay private so a stale reuse cannot be
    /// smuggled past [`Self::matches`].
    pub(crate) fn build(
        snapshot: &crate::service::AppSnapshot,
        positions: std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
        detached: Option<moiren_core::graph::PortId>,
        world: std::sync::Arc<crate::ui::pages::graph::graph_canvas::CanvasWorld>,
    ) -> Self {
        Self {
            graph: ArcSnapshot::new(snapshot.graph.clone()),
            catalog: ArcSnapshot::new(snapshot.catalog.clone()),
            positions,
            detached,
            locale: crate::i18n::current(),
            world,
        }
    }

    pub(crate) fn matches(
        &self,
        snapshot: &crate::service::AppSnapshot,
        positions: &std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
        detached: Option<moiren_core::graph::PortId>,
    ) -> bool {
        // Pointer fast path on the graph and catalog; positions compare by
        // value (an allocation-free O(n) walk) because they mutate in place.
        self.graph == ArcSnapshot::new(snapshot.graph.clone())
            && self.catalog == ArcSnapshot::new(snapshot.catalog.clone())
            && self.positions == *positions
            && self.detached == detached
            && self.locale == crate::i18n::current()
    }
}

/// The graph page's projection and panel state (design 20 §4): the
/// camera and node position table, the canvas focus handle, the context
/// menu and binding menu (with its scroll), the library search input and
/// pending spawn, the projected frame shared with the meter layer, HUD
/// overlay rects, the canvas size, the hovered card, the spectrum paint
/// scratch, the cached world and selection identities, the shared text
/// shaping cache, and the cached GraphPanel host.
pub(crate) struct GraphViewState {
    pub(crate) camera: Camera,
    pub(crate) node_positions: std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    pub(crate) focus: FocusHandle,
    pub(crate) context_menu: Option<super::ContextMenu>,
    pub(crate) binding_menu: Option<super::BindingMenu>,
    pub(crate) binding_menu_scroll: ScrollHandle,
    pub(crate) pending_node_spawn: Option<super::PendingNodeSpawn>,
    pub(crate) node_search: Entity<component::input::InputState>,
    pub(crate) controls: Entity<GraphPanelHost>,
    pub(crate) meter_frame: Entity<GraphMeterLayer>,
    pub(crate) canvas_frame: crate::ui::pages::graph::graph_canvas_view::PreparedFrame,
    pub(crate) overlays: crate::ui::pages::graph::graph_canvas::OverlayRects,
    pub(crate) canvas_size: Option<(f32, f32)>,
    pub(crate) hovered_node: Option<moiren_core::graph::NodeId>,
    pub(crate) spectrum_scratch: Vec<f32>,
    /// The canvas world from the last rebuild, reused whenever nothing it
    /// contains moved (audit A-1).
    pub(crate) world_cache: Option<WorldCache>,
    /// How many times the world actually re-projected; the cache tests
    /// assert gesture frames leave it flat.
    #[cfg(feature = "gpui-test")]
    pub(crate) world_builds: usize,
    /// The selection's canvas-id strings, rebuilt only when the selection
    /// changes — the rebuild path formats one `Debug` id per selected node
    /// otherwise (audit A-1's clone tax).
    pub(crate) selected_ids: Option<(
        std::collections::BTreeSet<moiren_core::graph::NodeId>,
        Rc<Vec<String>>,
    )>,
    /// The selected route's wire id, rebuilt only when the route or the
    /// graph changes (the O(edges) lookup otherwise runs per rebuild).
    pub(crate) selected_wire: Option<SelectedWireCache>,
    /// The paint-time readers the canvas closures capture, built once (they
    /// only capture the shell entity) instead of per rebuild.
    pub(in crate::ui) live_readers: Option<LiveReaderHandles>,
    /// The canvas painters' text shaping cache, shared by the cached paint
    /// and the hot layer (audit A-1).
    pub(in crate::ui) shapes: crate::ui::pages::graph::graph_canvas_view::CanvasShapesHandle,
}

/// The selected route's cached wire identity: the route it was resolved
/// for and the graph it was resolved against (audit A-1's clone tax — the
/// O(edges) lookup otherwise runs on every rebuild).
pub(crate) struct SelectedWireCache {
    pub(crate) route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
    pub(crate) graph: std::sync::Arc<crate::service::GraphSnapshot>,
    pub(crate) wire: Option<Rc<str>>,
}

/// The paint-time reader closures handed to the canvas view; built once per
/// shell, cloned into every rebuild's paint closures by handle.
#[derive(Clone)]
pub(in crate::ui) struct LiveReaderHandles {
    pub(in crate::ui) levels: Rc<crate::ui::pages::graph::graph_canvas_view::NodeLevelsFn>,
    pub(in crate::ui) spectrum: Rc<crate::ui::pages::graph::graph_canvas_view::NodeSpectrumFn>,
}

pub(crate) fn mount(shell: &StudioShell, _window: &Window) -> AnyElement {
    let controls = shell
        .graph_view
        .controls
        .clone()
        .cached(StyleRefinement::default().size_full())
        .into_any_element();
    div()
        .relative()
        .flex()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .child(controls)
        // The meter frame follows the page content: painting-free, hitbox
        // free, and the only part of the page that renders on an
        // animation-only frame.
        .child(shell.graph_view.meter_frame.clone())
        .into_any_element()
}

/// The Graph page's hot surface: live bars over the cached canvas, painting
/// from the frame the canvas prepaint published and the observations of the
/// current frame.
pub(crate) struct GraphMeters {
    shell: WeakEntity<StudioShell>,
    frame: crate::ui::pages::graph::graph_canvas_view::PreparedFrame,
}

impl GraphMeters {
    pub(crate) fn new(
        shell: WeakEntity<StudioShell>,
        frame: crate::ui::pages::graph::graph_canvas_view::PreparedFrame,
    ) -> Self {
        Self { shell, frame }
    }
}

pub(crate) type GraphMeterLayer = HotLayerView<GraphMeters>;

impl HotSurface for GraphMeters {
    type Frame = crate::ui::pages::graph::graph_canvas::CanvasFrame;

    fn pump(&mut self, _window: &mut Window, cx: &mut App) -> bool {
        let Some(shell) = self.shell.upgrade() else {
            return false;
        };
        shell.update(cx, |shell, cx| {
            #[cfg(feature = "gpui-test")]
            shell.trace.borrow_mut().begin(true);
            #[cfg(feature = "gpui-test")]
            let began = std::time::Instant::now();
            shell.pump_meters(cx);
            #[cfg(feature = "gpui-test")]
            {
                let mut trace = shell.trace.borrow_mut();
                trace.add(began.elapsed());
                trace.graph_frames += 1;
                trace.finish(shell.metering.node_meter_points.borrow().len());
            }
            shell.frames_wanted(cx)
        })
    }

    fn frame(&self) -> &crate::ui::pages::graph::graph_canvas_view::PreparedFrame {
        &self.frame
    }

    fn paint(
        &self,
        frame: &CanvasFrame,
        _layer: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        shell.update(cx, |shell, cx| {
            paint_graph_meter_frame(frame, shell, window, cx);
            paint_dragged_entities(frame, shell, window, cx);
            paint_connection_ghost(frame, shell, window, cx);
            paint_box_select_band(frame, shell, window, cx);
            paint_fault_tooltip(frame, shell, window, cx);
        });
    }
}

/// The dragged selection's cards and their wires, painted by the hot layer
/// at the gesture's live offset (audit A-1). During a card drag the cached
/// canvas hides these entities — its frozen paint cannot move them — so this
/// is the only place their pixels reach the screen while the drag holds:
/// wires first (they sit under every card, as in the cached layer), then the
/// cards with their live meters and spectra. Paint order puts them above
/// the static content, which is the one layered difference a gesture hot
/// layer carries: the cached canvas would have interleaved them at their
/// topological depth.
fn paint_dragged_entities(
    frame: &CanvasFrame,
    shell: &mut StudioShell,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    if shell.graph_gestures.node_drag.is_none() {
        return;
    }
    let dragged: std::collections::BTreeSet<moiren_core::graph::NodeId> =
        shell.selection.nodes.clone();
    let offset = shell.graph_gestures.node_drag_offset;
    let zoom = frame.camera.zoom;
    let origin = (
        f32::from(frame.bounds.origin.x),
        f32::from(frame.bounds.origin.y),
    );
    window.with_content_mask(
        Some(ContentMask {
            bounds: frame.bounds,
        }),
        |window| {
            for wire in &frame.world.wires {
                let src_moved = dragged.contains(&wire.src);
                let dst_moved = dragged.contains(&wire.dst);
                if !src_moved && !dst_moved {
                    continue;
                }
                let move_end = |anchor: (f32, f32)| (anchor.0 + offset.0, anchor.1 + offset.1);
                let from = if src_moved {
                    move_end(wire.from)
                } else {
                    wire.from
                };
                let to = if dst_moved {
                    move_end(wire.to)
                } else {
                    wire.to
                };
                let geometry = crate::ui::pages::graph::graph_canvas::WireCurve::new(from, to)
                    .project(frame.camera);
                crate::ui::pages::graph::graph_canvas_view::paint_wire(
                    frame, wire, &geometry, origin, false, true, window,
                );
            }
            for (index, node) in frame.world.nodes.iter().enumerate() {
                let Some(id) = node.node_id else {
                    continue;
                };
                if !dragged.contains(&id) {
                    continue;
                }
                let layout = frame.nodes[index].translated(offset.0 * zoom, offset.1 * zoom);
                if !layout.card.intersects(&frame.viewport()) {
                    continue;
                }
                let reading = {
                    let state = shell.state.read(cx);
                    state.node_meters.levels(id).map(|levels| {
                        crate::ui::pages::graph::graph_canvas_view::NodeMeterReading {
                            levels,
                            clips: state.node_meters.clips(id),
                        }
                    })
                };
                // The curve borrows the subscription table, which the paint
                // below cannot hold across its `cx`: copy it into the
                // shell's scratch buffer, exactly as the meter frame does
                // for the analyzer cards it paints. The shaping cache borrows
                // the graph view too, so it is taken after the scratch, per
                // card.
                let axis = if node.spectrum {
                    let state = shell.state.read(cx);
                    let curve = state.node_spectra.curve(id);
                    shell.graph_view.spectrum_scratch.clear();
                    if let Some(curve) = curve {
                        shell
                            .graph_view
                            .spectrum_scratch
                            .extend_from_slice(curve.points);
                    }
                    curve.map(|curve| (curve.lo_hz, curve.hi_hz))
                } else {
                    None
                };
                let spectrum = axis.map(|(lo_hz, hi_hz)| crate::ui::monitoring::SpectrumCurve {
                    points: &shell.graph_view.spectrum_scratch,
                    lo_hz,
                    hi_hz,
                });
                let mut shapes = shell.graph_view.shapes.borrow_mut();
                crate::ui::pages::graph::graph_canvas_view::paint_node(
                    frame,
                    node,
                    &layout,
                    true,
                    crate::ui::pages::graph::graph_canvas_view::NodeLive {
                        meters: reading.as_ref(),
                        spectrum,
                    },
                    crate::ui::pages::graph::graph_canvas_view::Paint {
                        shapes: &mut shapes,
                        window,
                    },
                    cx,
                );
            }
        },
    );
    #[cfg(feature = "gpui-test")]
    {
        shell.trace.borrow_mut().graph_gesture_paints += 1;
    }
}

/// The connection drag's ghost line and hover verdict, painted by the hot
/// layer from the drag's live cursor (audit A-1): the cached canvas never
/// sees the cursor, so drag frames cost no rebuild and no cached repaint.
fn paint_connection_ghost(
    frame: &CanvasFrame,
    shell: &mut StudioShell,
    window: &mut Window,
    _cx: &mut Context<StudioShell>,
) {
    let Some(drag) = shell.graph_gestures.connection_drag.as_ref() else {
        return;
    };
    let (port, cursor) = (drag.port, drag.cursor);
    crate::ui::pages::graph::graph_canvas_view::paint_connection_drag(frame, port, cursor, window);
    #[cfg(feature = "gpui-test")]
    {
        shell.trace.borrow_mut().graph_gesture_paints += 1;
    }
}

/// The fault tooltip for the hovered card, when it wears one. Hover lives
/// on the shell outside every panel key, so it reaches the screen through
/// this layer alone: one notified frame paints it, and it stands until
/// the next hover change repaints.
fn paint_fault_tooltip(
    frame: &CanvasFrame,
    shell: &StudioShell,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    let Some(id) = shell.graph_view.hovered_node else {
        return;
    };
    let Some(index) = frame
        .world
        .nodes
        .iter()
        .position(|node| node.node_id == Some(id))
    else {
        return;
    };
    if !frame.world.nodes[index].faulted {
        return;
    }
    let mut shapes = shell.graph_view.shapes.borrow_mut();
    crate::ui::pages::graph::graph_canvas_view::paint_fault_tooltip(
        frame,
        index,
        crate::ui::pages::graph::graph_canvas_view::Paint {
            shapes: &mut shapes,
            window,
        },
        cx,
    );
}

/// The rubber band, painted from the live drag state by the hot layer once
/// per frame — dynamic pixels cannot live in the cached canvas, whose
/// closures only run on rebuilds. While the band has moved past its
/// threshold this also applies the live selection, exactly as the canvas
/// prepaint used to report it.
fn paint_box_select_band(
    frame: &CanvasFrame,
    shell: &mut StudioShell,
    window: &mut Window,
    _cx: &mut Context<StudioShell>,
) {
    let Some(drag) = shell.graph_gestures.selection_drag.as_ref() else {
        return;
    };
    let (anchor, current) = (drag.anchor, drag.current);
    let a = frame.local_position(anchor);
    let b = frame.local_position(current);
    let left = a.0.min(b.0);
    let top = a.1.min(b.1);
    let screen = frame.window_rect(Bounds {
        origin: point(left, top),
        size: size((a.0 - b.0).abs(), (a.1 - b.1).abs()),
    });
    window.paint_quad(fill(
        screen,
        theme::controls::FOCUS.opacity(theme::ALPHA_SELECTION),
    ));
    window.paint_quad(PaintQuad {
        bounds: screen,
        corner_radii: 2f32.into(),
        background: theme::controls::TRANSPARENT.into(),
        border_widths: 1f32.into(),
        border_color: theme::controls::FOCUS,
        border_style: BorderStyle::default(),
    });
}

fn paint_graph_meter_frame(
    frame: &CanvasFrame,
    shell: &mut StudioShell,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    // A box select does NOT rebuild per move — the hot layer owns the bars
    // for the gesture's whole duration, painting them under the band that
    // follows this call. A connection drag now behaves the same: its ghost
    // paints through the hot layer below, and the bars keep coming from
    // here because the cached canvas freezes between rebuilds. A card drag
    // hides the dragged entities from both the cached canvas and this
    // overlay — paint_dragged_entities owns their pixels.
    let dragging = shell.graph_gestures.node_drag.is_some();
    // The HUD overlays (editing toolbar, overview, library handle) live in
    // the cached panel beneath this layer: carve their rects out of every
    // card's mask so live bars never paint over them.
    let overlays = shell.graph_view.overlays.rects();
    window.with_content_mask(
        Some(ContentMask {
            bounds: frame.bounds,
        }),
        |window| {
            for (index, node) in frame.world.nodes.iter().enumerate() {
                let Some(id) = node.node_id else {
                    continue;
                };
                if dragging && shell.selection.nodes.contains(&id) {
                    continue;
                }
                let layout = &frame.nodes[index];
                if !layout.card.intersects(&frame.viewport()) {
                    continue;
                }
                // The canvas paints cards in frame order, later ones on
                // top. A later card overlapping this node's meter tracks
                // (or its spectrum plot) would be cut into by the overlay's
                // opaque paint, so leave those to the cached canvas until
                // the next rebuild.
                let covered = frame.world.nodes[index + 1..]
                    .iter()
                    .zip(&frame.nodes[index + 1..])
                    .any(|(_, later)| {
                        later.card.intersects(&layout.meter.bounds())
                            || layout
                                .spectrum
                                .is_some_and(|plot| later.card.intersects(&plot))
                    });
                if covered {
                    continue;
                }
                let reading = {
                    let state = shell.state.read(cx);
                    state.node_meters.levels(id).map(|levels| {
                        crate::ui::pages::graph::graph_canvas_view::NodeMeterReading {
                            levels,
                            clips: state.node_meters.clips(id),
                        }
                    })
                };
                // The curve borrows the subscription table, which the paint
                // below cannot hold across its `cx`: copy it into the
                // shell's scratch buffer (sized once, never reallocated at
                // steady state).
                let axis = if layout.spectrum.is_some() {
                    let state = shell.state.read(cx);
                    let curve = state.node_spectra.curve(id);
                    shell.graph_view.spectrum_scratch.clear();
                    if let Some(curve) = curve {
                        shell
                            .graph_view
                            .spectrum_scratch
                            .extend_from_slice(curve.points);
                    }
                    curve.map(|curve| (curve.lo_hz, curve.hi_hz))
                } else {
                    None
                };
                let pieces = crate::ui::pages::graph::graph_canvas::carve(
                    frame.window_rect(layout.card),
                    &overlays,
                );
                for piece in pieces {
                    window.with_content_mask(Some(ContentMask { bounds: piece }), |window| {
                        if layout.spectrum.is_some() {
                            let curve =
                                axis.map(|(lo_hz, hi_hz)| crate::ui::monitoring::SpectrumCurve {
                                    points: &shell.graph_view.spectrum_scratch,
                                    lo_hz,
                                    hi_hz,
                                });
                            let mut shapes = shell.graph_view.shapes.borrow_mut();
                            crate::ui::pages::graph::graph_canvas_view::paint_spectrum_body(
                                frame,
                                layout,
                                &node.detail,
                                node.tint(),
                                curve,
                                crate::ui::pages::graph::graph_canvas_view::Paint {
                                    shapes: &mut shapes,
                                    window,
                                },
                                cx,
                            );
                        }
                        crate::ui::pages::graph::graph_canvas_view::paint_meter_bars(
                            frame,
                            layout,
                            reading.as_ref(),
                            window,
                        );
                    });
                }
            }
        },
    );
    #[cfg(feature = "gpui-test")]
    {
        shell.trace.borrow_mut().graph_meter_paints += 1;
    }
}

/// The shell's live spectrum curve for the canvas paint closure, copied
/// out at rebuild time (an analyzer's 121 points) so the cached paint owns
/// what it draws. `None` paints the plot's furniture alone — or when the
/// shell is gone: the handle is weak, because the closures live on the
/// shell itself and a strong capture would retain the closed window.
pub(in crate::ui) fn node_spectrum(
    this: &WeakEntity<StudioShell>,
) -> Rc<crate::ui::pages::graph::graph_canvas_view::NodeSpectrumFn> {
    let this = this.clone();
    Rc::new(move |id, cx: &App| {
        let this = this.upgrade()?;
        let state = this.read(cx).state.read(cx);
        state.node_spectra.curve(id).map(|curve| {
            crate::ui::pages::graph::graph_canvas_view::NodeSpectrumReading {
                points: curve.points.into(),
                lo_hz: curve.lo_hz,
                hi_hz: curve.hi_hz,
            }
        })
    })
}

/// The shell's live meter readings for the canvas paint closure: display
/// ballistics plus latched clips for one node, read at paint time so meter
/// motion repaints without re-projecting the graph. `None` keeps tracks
/// empty — or when the shell is gone: the handle is weak, because the
/// closures live on the shell itself and a strong capture would retain the
/// closed window.
pub(in crate::ui) fn node_levels(
    this: &WeakEntity<StudioShell>,
) -> Rc<crate::ui::pages::graph::graph_canvas_view::NodeLevelsFn> {
    let this = this.clone();
    Rc::new(move |id, cx: &App| {
        let this = this.upgrade()?;
        let state = this.read(cx).state.read(cx);
        state.node_meters.levels(id).map(|levels| {
            crate::ui::pages::graph::graph_canvas_view::NodeMeterReading {
                levels,
                clips: state.node_meters.clips(id),
            }
        })
    })
}
