//! Cached canvas controls and the animation-owner meter frame for the graph
//! page, mirroring the Mixer's render isolation: the page content (canvas,
//! library, editing toolbar) is cached behind a semantic key, while a
//! hitbox-free sibling view pumps observations and owns the frame loop. A
//! drawn frame rebuilds the shell's root element, so without this cache the
//! world projection and every interaction closure would rebuild per frame.
use super::*;
use std::rc::Rc;

pub(in crate::ui::window) struct GraphControls {
    shell: WeakEntity<StudioShell>,
    key: Option<ControlsKey>,
    _search_subscription: Subscription,
    _subscription: Subscription,
}

/// Only inputs the page content consumes belong in this key. Elapsed time
/// and audio observations must not invalidate it: the shell root still
/// rebuilds on every drawn frame, and those rebuilds must splice this cache
/// instead of re-projecting the graph.
#[derive(PartialEq)]
struct ControlsKey {
    graph: std::sync::Arc<crate::service::GraphSnapshot>,
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
            graph: snapshot.graph.clone(),
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

impl GraphControls {
    pub fn new(
        shell: Entity<StudioShell>,
        search: Entity<component::input::InputState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&shell, |controls, shell, cx| {
            let key = ControlsKey::new(shell.read(cx), cx);
            if controls.key.as_ref() != Some(&key) {
                controls.key = Some(key);
                cx.notify();
            }
        });
        // The library's search input is a model entity nested in this cached
        // tree; its notifications cannot invalidate the cached ancestor by
        // themselves, so typing must re-render through an explicit observer.
        let search_subscription = cx.observe(&search, |_, _, cx| cx.notify());
        Self {
            shell: shell.downgrade(),
            key: None,
            _search_subscription: search_subscription,
            _subscription: subscription,
        }
    }
}

impl Render for GraphControls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(shell) = self.shell.upgrade() {
            self.key = Some(ControlsKey::new(shell.read(cx), cx));
        }
        #[cfg(feature = "gpui-test")]
        if let Some(shell) = self.shell.upgrade() {
            shell.read(cx).mixer.trace.borrow_mut().graph_renders += 1;
        }
        let Some(shell) = self.shell.upgrade() else {
            return div().into_any_element();
        };
        shell.update(cx, |shell, cx| {
            // Fresh visibility demand: the canvas prepaint re-registers the
            // nodes its fitted camera shows during this same layout pass.
            shell.meter_points.set((false, false));
            shell.node_meter_points.borrow_mut().clear();
            super::page(shell, cx)
        })
    }
}

pub(in crate::ui::window) fn page(shell: &StudioShell, window: &Window) -> AnyElement {
    // Active accessibility needs the controls rebuilt into each frame's tree.
    let controls = if window.is_a11y_active() {
        div()
            .size_full()
            .child(shell.graph_controls.clone())
            .into_any_element()
    } else {
        shell
            .graph_controls
            .clone()
            .cached(StyleRefinement::default().size_full())
            .into_any_element()
    };
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

pub(in crate::ui::window) struct GraphMeterFrame {
    shell: WeakEntity<StudioShell>,
}

impl GraphMeterFrame {
    pub fn new(shell: WeakEntity<StudioShell>) -> Self {
        Self { shell }
    }
}

impl Render for GraphMeterFrame {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pump_shell = self.shell.clone();
        // No hitbox: the canvas, its input overlay, and the editing toolbar
        // underneath keep their pointer routing untouched.
        canvas(
            move |_, window, cx| {
                let _ = pump_shell.update(cx, |shell, cx| {
                    #[cfg(feature = "gpui-test")]
                    shell.mixer.trace.borrow_mut().begin(true);
                    #[cfg(feature = "gpui-test")]
                    let began = std::time::Instant::now();
                    shell.pump_meters(cx);
                    // Motion, not visibility, sustains the loop: at rest the
                    // control-side poll keeps the observations current and
                    // re-arms the frames once fresh data moves a layer.
                    if shell.meters_frame_active(cx) && shell.meters_in_motion() {
                        window.request_animation_frame();
                    }
                    #[cfg(feature = "gpui-test")]
                    {
                        let mut trace = shell.mixer.trace.borrow_mut();
                        trace.add(began.elapsed());
                        trace.graph_frames += 1;
                        trace.finish(shell.node_meter_points.borrow().len());
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

/// The shell's live meter readings for the canvas paint closure: display
/// ballistics for one node, as bar heights. `None` keeps the track empty.
pub(in crate::ui::window) fn node_levels(
    this: &Entity<StudioShell>,
) -> Rc<pages::graph_canvas_view::NodeLevelsFn> {
    let this = this.clone();
    Rc::new(move |id, cx: &App| {
        this.read(cx)
            .node_meters
            .levels(id)
            .map(|levels| levels.map(|level| level.bar))
    })
}
