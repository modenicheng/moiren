use rust_i18n::t;

use super::*;

mod arrange;
mod binding_menu;
mod compressor;
mod context_menu;
pub mod graph_canvas;
pub mod graph_canvas_view;
mod headline;
mod interactions;
mod library;
pub mod node_catalog;
pub(crate) use binding_menu::{
    BindingMenu, overlay as binding_menu_overlay, sync as sync_binding_menu,
};
pub(crate) use compressor::CompressorControls;
mod node_inspector;
pub(crate) use context_menu::overlay as context_menu_overlay;
pub(crate) mod page;
pub(crate) use library::PendingNodeSpawn;
pub(crate) use library::{library, library_rail};
pub(crate) use node_inspector::inspector;

/// The graph canvas's in-flight pointer gestures (design 20 §4): node
/// drags (with snap offset and alignment guides), the rubber-band
/// selection, connection drags, knob drags, background pan/click state,
/// the right-press anchor, and the hand-tool toggle.
#[derive(Default)]
pub(crate) struct GraphInteractionState {
    pub(crate) node_drag: Option<(moiren_core::graph::NodeId, f32, f32)>,
    pub(crate) selection_drag: Option<SelectionDrag>,
    pub(crate) connection_drag: Option<ConnectionDrag>,
    pub(crate) knob_drag: Option<(
        moiren_core::graph::NodeId,
        crate::ui::pages::graph::graph_canvas::KnobControl,
        f32,
        f64,
    )>,
    pub(crate) pan_grab: Option<(f32, f32)>,
    pub(crate) background_click: Option<(f32, f32)>,
    pub(crate) right_press: Option<(f32, f32)>,
    pub(crate) node_drag_snap: (f32, f32),
    pub(crate) drag_guides: Option<crate::ui::pages::graph::graph_canvas::DragGuides>,
    pub(crate) hand_tool: bool,
}

/// The workspace selection (design 20 §4): the multi-selection set, its
/// primary (last-clicked) member that drives the inspector, and the route
/// pick the matrix/inspectors target.
#[derive(Default)]
pub(crate) struct Selection {
    pub(crate) node: Option<moiren_core::graph::NodeId>,
    pub(crate) nodes: std::collections::BTreeSet<moiren_core::graph::NodeId>,
    pub(crate) route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests;

/// An existing edge picked up off its input by a connection drag: the edge
/// stays in the service until release commits, while the canvas shows the
/// head as free and the drag pulls from the edge's source output.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetachedEdge {
    edge: moiren_core::graph::EdgeId,
    destination: moiren_core::graph::PortId,
}

/// One port's wire end in flight during a connection drag.
#[derive(Clone, Copy)]
pub(crate) struct ConnectionDrag {
    port: crate::ui::pages::graph::graph_canvas::PortTarget,
    cursor: (f32, f32),
    detached: Option<DetachedEdge>,
}

/// A rubber-band selection drag on the canvas background. Starts as a plain
/// background click; past the 3 px threshold it becomes a live band that
/// replaces the selection (or unions with `base` when Ctrl/Shift held).
pub(crate) struct SelectionDrag {
    anchor: (f32, f32),
    current: (f32, f32),
    additive: bool,
    base: std::collections::BTreeSet<moiren_core::graph::NodeId>,
    moved: bool,
    hits: std::collections::BTreeSet<moiren_core::graph::NodeId>,
}

/// The canvas context menu: over a node (identity actions plus the tint
/// palette) or over the background (arrange). `at` is the window-space
/// release point the menu anchors to.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ContextMenu {
    Node {
        node: moiren_core::graph::NodeId,
        at: (f32, f32),
    },
    Background {
        at: (f32, f32),
    },
}

impl StudioShell {
    /// The rubber band's live application, driven by the meter hot layer
    /// once per frame while a box select is in flight. Returns whether the
    /// selection changed (the caller notifies once per change, not per
    /// frame).
    pub(crate) fn apply_box_select(
        &mut self,
        rect: Bounds<f32>,
        nodes: &[crate::ui::pages::graph::graph_canvas::CanvasNode],
        additive: bool,
        _cx: &mut Context<Self>,
    ) -> bool {
        let hits: std::collections::BTreeSet<moiren_core::graph::NodeId> = nodes
            .iter()
            .filter(|node| {
                crate::ui::pages::graph::graph_canvas::NodeLayout::new(node)
                    .card
                    .intersects(&rect)
            })
            .filter_map(|node| node.node_id)
            .collect();
        let Some(drag) = self.graph_gestures.selection_drag.as_mut() else {
            return false;
        };
        if !drag.moved {
            return false;
        }
        drag.hits = hits;
        let selection = if additive {
            drag.base.union(&drag.hits).copied().collect()
        } else {
            drag.hits.clone()
        };
        if self.selection.nodes == selection {
            return false;
        }
        self.selection.nodes = selection;
        if self.selection.nodes.len() == 1 {
            self.selection.node = self.selection.nodes.iter().next().copied();
        } else if self
            .selection
            .node
            .is_none_or(|id| !self.selection.nodes.contains(&id))
        {
            self.selection.node = None;
        }
        if !self.selection.nodes.is_empty() {
            self.selection.route = None;
        }
        true
    }

    /// Shared pointer-session preamble for canvas hit targets: cancel any
    /// in-flight drag or click first; under the hand tool the pointer instead
    /// becomes a pan grab. Returns true when the hand tool consumed the event.
    pub(crate) fn begin_pointer_session(&mut self, window: &mut Window) -> bool {
        self.graph_gestures.node_drag = None;
        self.graph_gestures.node_drag_snap = (0., 0.);
        self.graph_gestures.pan_grab = None;
        self.graph_gestures.background_click = None;
        self.graph_gestures.selection_drag = None;
        self.graph_gestures.knob_drag = None;
        self.sync_drag_demand();
        if self.graph_gestures.hand_tool {
            let position = window.mouse_position();
            self.graph_gestures.pan_grab = Some((f32::from(position.x), f32::from(position.y)));
            self.sync_drag_demand();
            true
        } else {
            false
        }
    }

    /// Cancels every in-flight canvas gesture without a pointer event to
    /// consume it. Lifecycle paths that unmount the canvas or steal the
    /// pointer (page switch, project load, window deactivation) drop the
    /// closing mouse-up on the floor; a grab left armed there would paint a
    /// stale rubber band and pin the DRAG frame demand after the user has
    /// moved on. Same effect as the canvas's Escape handler.
    pub(crate) fn cancel_active_gesture(&mut self, cx: &mut Context<Self>) {
        let cancelled = self.drag_active() || self.graph_gestures.background_click.is_some();
        self.graph_gestures.connection_drag = None;
        self.graph_gestures.node_drag = None;
        self.graph_gestures.node_drag_snap = (0., 0.);
        self.graph_gestures.drag_guides = None;
        self.graph_gestures.pan_grab = None;
        self.graph_gestures.background_click = None;
        self.graph_gestures.selection_drag = None;
        self.graph_gestures.knob_drag = None;
        // Keyboard focus stays on the canvas while the page's popovers are
        // open, so Escape lands here: it dismisses them too. The session
        // popover and the context menu ride along — they can be open on
        // any page, and on the graph page this is the Escape path that
        // reaches them.
        let dismissed = self.graph_view.binding_menu.take().is_some()
            | std::mem::take(&mut self.graph_view.context_menu).is_some()
            | std::mem::take(&mut self.graph_gestures.right_press).is_some()
            | std::mem::take(&mut self.chrome.project_menu);
        if cancelled {
            self.sync_drag_demand();
        }
        if cancelled || dismissed {
            cx.notify();
        }
    }

    pub(crate) fn clear_canvas_selection(&mut self) {
        self.selection.nodes.clear();
        self.selection.node = None;
        self.selection.route = None;
    }

    /// The "arrange" action: re-lays the whole graph into DAG layers, writes
    /// the presentation position table (like a card drag, not an undo step),
    /// and fits the camera to the result. Fit mode re-fits on its own; an
    /// adjusted camera is fitted once here so the arranged graph lands in
    /// view.
    pub(crate) fn arrange_graph(&mut self, cx: &mut Context<Self>) {
        let world = {
            let state = self.state.read(cx);
            crate::ui::pages::graph::graph_canvas::CanvasWorld::positioned(
                &state.snapshot,
                &self.graph_view.node_positions,
            )
        };
        let sizes: arrange::CardSizes = world
            .nodes
            .iter()
            .filter_map(|node| {
                let id = node.node_id?;
                let card = crate::ui::pages::graph::graph_canvas::NodeLayout::new(node).card;
                Some((id, (card.size.width, card.size.height)))
            })
            .collect();
        let snapshot = &self.state.read(cx).snapshot;
        let arranged = arrange::arrange(&snapshot.graph.graph, &sizes);
        let bbox = arrange::bounds(&arranged, &sizes);
        for node in arranged {
            self.graph_view
                .node_positions
                .insert(node.node_id, node.position);
        }
        if let Some((width, height)) = self.graph_view.canvas_size {
            self.graph_view
                .camera
                .fit(size(px(width), px(height)), bbox);
        }
        cx.notify();
    }

    /// Single selection over a node, clearing routes and the rest of the
    /// band — the context menu's identity actions act on this node alone.
    pub(crate) fn select_single_node(&mut self, node: moiren_core::graph::NodeId) {
        self.selection.nodes.clear();
        self.selection.nodes.insert(node);
        self.selection.node = Some(node);
        self.selection.route = None;
    }

    /// Enters the inspector's inline rename for `node`, seeded with the
    /// label it shows today and focused so typing replaces it immediately.
    pub(crate) fn begin_node_rename(
        &mut self,
        node: moiren_core::graph::NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial = {
            let state = self.state.read(cx);
            state
                .snapshot
                .graph
                .node(node)
                .map(crate::ui::presentation::display_label)
        };
        self.inspector.node_name_editing = Some(node);
        if let Some(initial) = initial {
            self.inspector
                .node_name
                .update(cx, |input, cx| input.set_value(initial, window, cx));
        }
        let input = self.inspector.node_name.clone();
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    pub(crate) fn delete_graph_selection(&mut self, cx: &mut Context<Self>) {
        // The primary selection may have been set from outside the canvas
        // (mixer strip, library locator); it always deletes with the band.
        if let Some(id) = self.selection.node {
            self.selection.nodes.insert(id);
        }
        let mut removed = false;
        for id in self.selection.nodes.iter().copied().collect::<Vec<_>>() {
            let removed_here = self.state.update(cx, |state, _| {
                state
                    .try_dispatch(AppRequest::EditGraph(
                        crate::service::GraphEdit::RemoveNode(id),
                    ))
                    .is_ok()
            });
            removed |= removed_here;
        }
        if !removed {
            let edge_id = {
                let graph = &self.state.read(cx).snapshot.graph.graph;
                self.selection.route.and_then(|ports| {
                    graph
                        .edges()
                        .iter()
                        .find(|edge| (edge.src_port(), edge.dst_port()) == ports)
                        .map(|edge| edge.id())
                })
            };
            if let Some(edge_id) = edge_id
                && self.state.update(cx, |state, _| {
                    state
                        .try_dispatch(AppRequest::EditGraph(
                            crate::service::GraphEdit::Disconnect(edge_id),
                        ))
                        .is_ok()
                })
            {
                self.selection.route = None;
            }
        } else {
            self.selection.nodes.clear();
            self.selection.node = None;
            self.selection.route = None;
        }
        self.graph_gestures.connection_drag = None;
        self.graph_gestures.node_drag = None;
        self.graph_gestures.node_drag_snap = (0., 0.);
        cx.notify();
    }
}

/// The graph page's cached content: canvas, library, editing toolbar. The
/// shell renders this through [`page::page`], which caches it behind
/// [`page::ControlsKey`] and adds the animation-owner meter frame as a
/// sibling.
pub(crate) fn page(shell: &StudioShell, window: &Window, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    // Meter bar readings are not baked in: the canvas paint consults the
    // live ballistics per draw, so meter motion repaints without the page
    // cache being invalidated or the world being re-projected.
    let mut world = crate::ui::pages::graph::graph_canvas::CanvasWorld::positioned(
        &state.snapshot,
        &shell.graph_view.node_positions,
    );
    // The drag's snap guides ride the world so the canvas paint layer, a
    // shell-free closure, can draw them above the grid.
    world.guides = shell.graph_gestures.drag_guides;
    // A wire end picked up off an input is already in flight: drop it from
    // the projection so the canvas paints the drag as a free head (no stale
    // line, an empty origin port, and no phantom Displacing/Cycles verdicts
    // against the edge being moved).
    if let Some(destination) = shell
        .graph_gestures
        .connection_drag
        .as_ref()
        .and_then(|drag| drag.detached)
        .map(|picked| picked.destination)
    {
        world.wires.retain(|wire| wire.dst_port != destination);
    }
    let world = std::sync::Arc::new(world);
    let camera = shell.graph_view.camera;
    // The canvas view identifies nodes by their Debug id string; this is the
    // single NodeId -> canvas-id conversion on the render path.
    let selected_ids: Vec<String> = shell
        .selection
        .nodes
        .iter()
        .map(|id| format!("{id:?}"))
        .collect();
    let selected_wire = shell.selection.route.and_then(|ports| {
        state
            .snapshot
            .graph
            .graph
            .edges()
            .iter()
            .find(|edge| (edge.src_port(), edge.dst_port()) == ports)
            .map(|edge| format!("{:?}", edge.id()))
    });
    let this = cx.entity();
    let node_levels = page::node_levels(&this);
    let node_spectrum = page::node_spectrum(&this);

    // Canvas view with interaction plumbed to the shell entity; every
    // callback body lives in `interactions.rs`, one builder per field.
    let canvas_view = crate::ui::pages::graph::graph_canvas_view::graph_canvas_view(
        world.clone(),
        camera,
        &selected_ids,
        selected_wire.as_deref(),
        crate::ui::pages::graph::graph_canvas_view::CanvasCallbacks {
            focus: shell.graph_view.focus.clone(),
            connection: shell.graph_gestures.connection_drag.map(|drag| {
                (
                    drag.port,
                    drag.cursor,
                    drag.detached.map(|picked| picked.destination),
                )
            }),
            node_levels,
            node_spectrum,
            on_port_down: interactions::port_down(&this, &world),
            on_connection_move: interactions::connection_move(&this),
            on_port_up: interactions::port_up(&this),
            on_knob_down: interactions::knob_down(&this, &world),
            on_chip: interactions::chip(&this, &world),
            on_badge: interactions::badge(&this, &world),
            on_knob_wheel: interactions::knob_wheel(&this, &world),
            on_expand: interactions::expand(&this, &world),
            on_delete: interactions::delete(&this),
            on_cancel: interactions::cancel(&this),
            on_drop_node: interactions::drop_node(&this),
            on_background_down: interactions::background_down(&this),
            on_canvas_mouse_move: interactions::background_move(&this),
            on_zoom: interactions::zoom(&this),
            on_pan: interactions::scroll_pan(&this),
            on_select_node: interactions::select_node(&this, &world),
            on_select_wire: interactions::select_wire(&this),
            on_camera_fit: interactions::camera_fit(&this, &world),
        },
        // Shared with the meter frame: its overlay repaints bars at this
        // frame's geometry between cache rebuilds.
        shell.graph_view.canvas_frame.clone(),
        &shell.graph_view.overlays,
    );

    div()
        .id("graph-page")
        .size_full()
        .flex()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .child(
            // Row-reversed so the canvas leads the DOM: gpui has no z-index,
            // and the library's protruding handle must paint above the canvas.
            div()
                .relative()
                .flex()
                .flex_row_reverse()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(
                    div()
                        .id("graph-canvas-host")
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(canvas_view)
                        .child(editing_toolbar(shell, &this)),
                )
                .child(sidebar::library_host(shell, window, cx)),
        )
        .gpui_test_observe()
        .into_any_element()
}

fn editing_toolbar(shell: &StudioShell, this: &Entity<StudioShell>) -> impl IntoElement {
    use controls::*;
    let mut view = div()
        .absolute()
        .top(px(12.))
        .left(px(12.))
        .flex()
        .items_center()
        .gap(px(4.))
        .p(px(4.))
        .rounded(px(theme::RADIUS_MD))
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        // The meter frame paints around this rect (it is a later sibling of
        // the cached panel, so live bars would otherwise cover the HUD).
        .child(shell.graph_view.overlays.probe("toolbar"));
    for (id, name, icon, hand) in [
        ("tool-select", t!("Select"), ActionIcon::Cursor, false),
        ("tool-hand", t!("Hand"), ActionIcon::HandMove, true),
    ] {
        let this = this.clone();
        view = view.child(
            button(id, name.clone())
                .icon(icon)
                .icon_only()
                .tooltip(name)
                .variant(if shell.graph_gestures.hand_tool == hand {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .on_click(move |_, _, cx| {
                    this.update(cx, |s, cx| {
                        s.graph_gestures.hand_tool = hand;
                        cx.notify();
                    })
                }),
        );
    }
    view = view.child(
        div()
            .flex_none()
            .w(px(1.))
            .h(px(16.))
            .mx(px(2.))
            .rounded_full()
            .bg(theme::BORDER),
    );
    // Zoom-out · readout · zoom-in reads as a stepper around the percentage;
    // the readout's width is pinned so the stepper never reflows.
    let zoom_step = |id: &'static str, icon: ActionIcon, name: SharedString, factor: f32| {
        button(id, name.clone())
            .icon(icon)
            .icon_only()
            .tooltip(name)
            .on_click({
                let this = this.clone();
                move |_, _, cx: &mut App| {
                    this.update(cx, |s, cx| {
                        s.graph_view
                            .camera
                            .zoom_around_center(s.graph_view.canvas_size, factor);
                        cx.notify();
                    })
                }
            })
    };
    view = view.child(zoom_step(
        "zoom-out",
        ActionIcon::ZoomOut,
        t!("Zoom out").into(),
        1. / 1.1,
    ));
    view = view.child(
        div()
            .min_w(px(44.))
            .flex()
            .justify_center()
            .child(label(format!(
                "{}%",
                (shell.graph_view.camera.zoom * 100.).round() as i32
            ))),
    );
    view = view.child(zoom_step(
        "zoom-in",
        ActionIcon::ZoomIn,
        t!("Zoom in").into(),
        1.1,
    ));
    view.child(button("zoom-fit", t!("Fit")).on_click({
        let this = this.clone();
        move |_, _, cx| {
            this.update(cx, |s, cx| {
                s.graph_view.camera.adjusted = false;
                cx.notify();
            })
        }
    }))
}
