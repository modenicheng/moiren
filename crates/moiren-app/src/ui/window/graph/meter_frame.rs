//! Cached page content behind a [`PanelHost`] and the display-rate meter
//! layer behind a [`HotLayerView`]: the meter canvas is a sibling, never a
//! descendant of the cached subtree — gpui-fast replays a cached panel's
//! paint without re-running its closures, so live bars must be painted by
//! the hot layer at the frame the canvas prepaint published.
use super::*;
use crate::ui::window::hot::{HotLayerView, HotSurface};
use crate::ui::window::panel::{ArcSnapshot, CachedPanel, PanelHost, cached_or_live};
use pages::graph_canvas::CanvasFrame;
use std::rc::Rc;

/// The Graph page panel behind [`PanelHost`]: the canvas world, library, and
/// editing toolbar, rebuilt only when their semantic key changes.
pub(in crate::ui::window) struct GraphPanel;

/// Only inputs the page content consumes belong in this key. Elapsed time
/// and audio observations must not invalidate it: the shell root still
/// rebuilds on every drawn frame, and those rebuilds must splice this cache
/// instead of re-projecting the graph.
#[derive(PartialEq)]
pub(in crate::ui::window) struct ControlsKey {
    graph: ArcSnapshot<crate::service::GraphSnapshot>,
    generation: crate::service::SessionGeneration,
    phase: SessionPhase,
    desired: Option<crate::service::SessionSpec>,
    positions: std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    camera: Camera,
    canvas: Option<(f32, f32)>,
    selected_nodes: Vec<moiren_core::graph::NodeId>,
    selected_route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
    connection: Option<(pages::graph_canvas::PortTarget, (f32, f32))>,
    selection_box: Option<((f32, f32), (f32, f32))>,
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
            generation: snapshot.generation,
            phase: snapshot.phase,
            desired: snapshot.desired.clone(),
            positions: shell.node_positions.clone(),
            camera: shell.camera,
            canvas: shell.canvas_size,
            selected_nodes: shell.selected_nodes.iter().copied().collect(),
            selected_route: shell.selected_route,
            connection: shell.connection_drag.map(|drag| (drag.port, drag.cursor)),
            selection_box: shell
                .selection_drag
                .as_ref()
                .map(|drag| (drag.anchor, drag.current)),
            hand_tool: shell.hand_tool,
            sidebar_open: shell.sidebar_open,
            compact: shell.compact,
            preferences: shell.preferences.clone(),
        }
    }
}

pub(in crate::ui::window) type GraphPanelHost = PanelHost<GraphPanel>;

impl CachedPanel for GraphPanel {
    type Owner = StudioShell;
    type Key = ControlsKey;

    fn key(shell: &StudioShell, cx: &App) -> Self::Key {
        ControlsKey::new(shell, cx)
    }

    fn render(
        shell: &mut StudioShell,
        _window: &mut Window,
        cx: &mut Context<StudioShell>,
    ) -> AnyElement {
        #[cfg(feature = "gpui-test")]
        {
            shell.mixer.trace.borrow_mut().graph_renders += 1;
        }
        super::page(shell, cx)
    }

    fn on_rebuild(shell: &mut StudioShell) {
        // Fresh visibility demand: the canvas prepaint re-registers the
        // nodes its fitted camera shows during this same layout pass.
        shell.meter_points.set((false, false));
        shell.node_meter_points.borrow_mut().clear();
    }

    fn observe_nested(
        shell: &Entity<StudioShell>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // The library's search input is a model entity nested in this cached
        // tree; its notifications cannot invalidate the cached ancestor by
        // themselves, so typing must re-render through an explicit observer.
        let search = shell.read_with(cx, |shell, _| shell.node_search.clone());
        vec![cx.observe(&search, |_, _, cx| cx.notify())]
    }
}

pub(in crate::ui::window) fn page(shell: &StudioShell, window: &Window) -> AnyElement {
    let controls = cached_or_live(
        shell.graph_controls.clone(),
        window,
        |entity| {
            entity
                .cached(StyleRefinement::default().size_full())
                .into_any_element()
        },
        |entity| div().size_full().child(entity).into_any_element(),
    );
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
        .child(shell.graph_meter_frame.clone())
        .into_any_element()
}

/// The Graph page's hot surface: live bars over the cached canvas, painting
/// from the frame the canvas prepaint published and the observations of the
/// current frame.
pub(in crate::ui::window) struct GraphMeters {
    shell: WeakEntity<StudioShell>,
    frame: pages::graph_canvas_view::PreparedFrame,
}

impl GraphMeters {
    pub(in crate::ui::window) fn new(
        shell: WeakEntity<StudioShell>,
        frame: pages::graph_canvas_view::PreparedFrame,
    ) -> Self {
        Self { shell, frame }
    }
}

pub(in crate::ui::window) type GraphMeterLayer = HotLayerView<GraphMeters>;

impl HotSurface for GraphMeters {
    type Frame = pages::graph_canvas::CanvasFrame;

    fn pump(&mut self, _window: &mut Window, cx: &mut App) -> bool {
        let Some(shell) = self.shell.upgrade() else {
            return false;
        };
        shell.update(cx, |shell, cx| {
            #[cfg(feature = "gpui-test")]
            shell.mixer.trace.borrow_mut().begin(true);
            #[cfg(feature = "gpui-test")]
            let began = std::time::Instant::now();
            shell.pump_meters(cx);
            #[cfg(feature = "gpui-test")]
            {
                let mut trace = shell.mixer.trace.borrow_mut();
                trace.add(began.elapsed());
                trace.graph_frames += 1;
                trace.finish(shell.node_meter_points.borrow().len());
            }
            shell.frames_wanted(cx)
        })
    }

    fn frame(&self) -> &pages::graph_canvas_view::PreparedFrame {
        &self.frame
    }

    fn paint(&self, frame: &CanvasFrame, window: &mut Window, cx: &mut App) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        shell.update(cx, |shell, cx| {
            paint_graph_meter_frame(frame, shell, window, cx)
        });
    }
}

fn paint_graph_meter_frame(
    frame: &CanvasFrame,
    shell: &mut StudioShell,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    // Band and connection drags paint overlays that must stay above the
    // bars, and they rebuild the page every frame anyway, so the cached
    // canvas already shows live bars.
    if shell.selection_drag.is_some() || shell.connection_drag.is_some() {
        return;
    }
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
                // would be cut into by the overlay's opaque tracks, so
                // leave those bars to the cached canvas until the next
                // rebuild.
                let covered = frame.world.nodes[index + 1..]
                    .iter()
                    .zip(&frame.nodes[index + 1..])
                    .any(|(_, later)| later.card.intersects(&layout.meter));
                if covered {
                    continue;
                }
                let reading = {
                    let state = shell.state.read(cx);
                    state.node_meters.levels(id).map(|levels| {
                        pages::graph_canvas_view::NodeMeterReading {
                            levels,
                            clips: state.node_meters.clips(id),
                        }
                    })
                };
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: frame.window_rect(layout.card),
                    }),
                    |window| {
                        pages::graph_canvas_view::paint_meter_bars(
                            frame,
                            layout,
                            reading.as_ref(),
                            window,
                        );
                    },
                );
            }
        },
    );
    #[cfg(feature = "gpui-test")]
    {
        shell.mixer.trace.borrow_mut().graph_meter_paints += 1;
    }
}

/// The shell's live meter readings for the canvas paint closure: display
/// ballistics plus latched clips for one node, read at paint time so meter
/// motion repaints without re-projecting the graph. `None` keeps tracks empty.
pub(in crate::ui::window) fn node_levels(
    this: &Entity<StudioShell>,
) -> Rc<pages::graph_canvas_view::NodeLevelsFn> {
    let this = this.clone();
    Rc::new(move |id, cx: &App| {
        let state = this.read(cx).state.read(cx);
        state
            .node_meters
            .levels(id)
            .map(|levels| pages::graph_canvas_view::NodeMeterReading {
                levels,
                clips: state.node_meters.clips(id),
            })
    })
}
