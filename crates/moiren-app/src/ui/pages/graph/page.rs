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
    connection: Option<(
        crate::ui::pages::graph::graph_canvas::PortTarget,
        (f32, f32),
        Option<super::DetachedEdge>,
    )>,
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
            connection: shell
                .graph_gestures
                .connection_drag
                .map(|drag| (drag.port, drag.cursor, drag.detached)),
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

/// The graph page's projection and panel state (design 20 §4): the
/// camera and node position table, the canvas focus handle, the context
/// menu and binding menu (with its scroll), the library search input and
/// pending spawn, the projected frame shared with the meter layer, HUD
/// overlay rects, the canvas size, the hovered card, the spectrum paint
/// scratch, and the cached GraphPanel host.
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
            paint_box_select_band(frame, shell, window, cx);
            paint_fault_tooltip(frame, shell, window, cx);
        });
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
    crate::ui::pages::graph::graph_canvas_view::paint_fault_tooltip(frame, index, window, cx);
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
    // A connection drag rebuilds the page every move (the connection rides
    // the panel key), so the cached canvas shows live bars and the overlay
    // line must stay above them. A box select does NOT rebuild per move —
    // the hot layer owns the bars for the gesture's whole duration,
    // painting them under the band that follows this call.
    if shell.graph_gestures.connection_drag.is_some() {
        return;
    }
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
                            crate::ui::pages::graph::graph_canvas_view::paint_spectrum_body(
                                frame,
                                layout,
                                &node.detail,
                                node.tint(),
                                curve,
                                window,
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
/// what it draws. `None` paints the plot's furniture alone.
pub(crate) fn node_spectrum(
    this: &Entity<StudioShell>,
) -> Rc<crate::ui::pages::graph::graph_canvas_view::NodeSpectrumFn> {
    let this = this.clone();
    Rc::new(move |id, cx: &App| {
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
/// motion repaints without re-projecting the graph. `None` keeps tracks empty.
pub(crate) fn node_levels(
    this: &Entity<StudioShell>,
) -> Rc<crate::ui::pages::graph::graph_canvas_view::NodeLevelsFn> {
    let this = this.clone();
    Rc::new(move |id, cx: &App| {
        let state = this.read(cx).state.read(cx);
        state.node_meters.levels(id).map(|levels| {
            crate::ui::pages::graph::graph_canvas_view::NodeMeterReading {
                levels,
                clips: state.node_meters.clips(id),
            }
        })
    })
}
