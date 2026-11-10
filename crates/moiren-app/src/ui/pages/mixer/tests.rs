use super::super::{Page, WorkspaceView};
use super::fader_readout_gain;
use super::model::reorder;
use crate::ui::shell::StudioShell;
use crate::ui::shell::test_harness::{
    finish_service, settle_panels, shell_with_state, test_shell, wait_until,
};
use crate::ui::widgets::fader::{FaderScale, send_gain};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Modifiers, MouseButton, Pixels, ScrollDelta, ScrollWheelEvent,
    SharedString, TestAppContext, VisualTestContext, point, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use std::time::Duration;

fn poll(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    wait_until(Duration::from_secs(2), || {
        let pending = view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).pending_request
        });
        cx.update(|window, cx| window.render_frame(cx));
        !pending
    });
}

#[gpui_kit::test]
fn mixer_controls_target_selected_nodes_and_restore_layout(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let (source, gain, sink) = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph.graph;
        let node = |kind| {
            graph
                .nodes()
                .iter()
                .find(|node| node.kind() == kind)
                .unwrap()
                .id()
        };
        (
            node(NodeKind::Source),
            node(NodeKind::Gain),
            node(NodeKind::Sink),
        )
    });
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("strip-mute-{source:?}")), cx);
    });
    poll(&view, cx);
    cx.update(|window, cx| window.click(SharedString::from(format!("strip-mode-{gain:?}")), cx));
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert!(graph.node(source).unwrap().mixer.muted);
        assert!(!graph.node(gain).unwrap().mixer.muted);
        assert_eq!(graph.node(gain).unwrap().mixer.channel_mode, 1);
        assert_eq!(graph.node(source).unwrap().mixer.channel_mode, 0);
        assert_eq!(graph.active_revision, None);
        reorder(&mut shell.mixer.presentation.order, gain, source);
        let document = shell.project_document(cx).unwrap();
        shell.mixer.presentation.order.clear();
        shell.apply_project_presentation(&document, cx).unwrap();
        assert_eq!(shell.mixer.presentation.order[0], gain);
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        let dock = window.find("mixer-dock").bounds();
        assert!(
            dock.contains(
                &window
                    .find(SharedString::from(format!("strip-header-{sink:?}")))
                    .bounds()
                    .center()
            )
        );
        let output = window
            .find(SharedString::from(format!("strip-{sink:?}")))
            .bounds();
        assert!((f32::from(output.right() - dock.right())).abs() <= 1.);
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn mixer_header_drag_reorders_channels(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let order = view.update(cx, |shell, _| shell.mixer.presentation.order.clone());
    let origin = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("strip-header-{:?}", order[1])))
            .bounds()
            .center()
    });
    let target = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("strip-header-{:?}", order[0])))
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(origin, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        origin + point(px(12.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    view.update(cx, |shell, _| {
        assert_eq!(shell.mixer.presentation.order[0], order[1])
    });
    finish_service(runtime);
}

/// The Bus & output dock's divider is a live resize handle: dragging it moves
/// the divider under the cursor (the pane hugs the strip row's right edge),
/// the width clamps to one strip at the narrow end and the pane's 55% cap at
/// the wide end with no dead zone, and the docked strips stay flush against
/// the pane's right edge.
#[gpui_kit::test]
fn mixer_dock_divider_drag_resizes_the_dock(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    // The inspector's closing spring keeps the strip row's right edge moving
    // for a while; the divider's geometry must be measured against the
    // settled row or the drag lands on stale coordinates.
    crate::ui::shell::test_harness::settle_panels(cx);
    let dock_width = |cx: &mut VisualTestContext| {
        cx.update(|window, _| window.find("mixer-dock").bounds().size.width)
    };
    assert!(
        (f32::from(dock_width(cx)) - 360.).abs() < 0.5,
        "the dock starts at the default width"
    );
    let drag_divider_to = |cx: &mut VisualTestContext, x: Pixels| {
        let divider = cx.update(|window, _| window.find("mixer-dock-resize").bounds().center());
        // The platform drag gesture arms only after a visible move, so step
        // off the press point first and render the frame the drag owns.
        cx.simulate_mouse_down(divider, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            divider + point(px(-12.), px(0.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.update(|window, cx| window.render_frame(cx));
        let target = point(x, divider.y);
        cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
        cx.update(|window, cx| window.render_frame(cx));
        cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
        cx.update(|window, cx| window.render_frame(cx));
    };
    // 100px left of the default divider: the pane grows by exactly the drag.
    // The target rides the border itself, not the grab handle's left edge
    // (which reaches 3px past it), so the arithmetic stays in whole strips.
    let divider_x = cx.update(|window, _| window.find("mixer-dock").bounds().left());
    drag_divider_to(cx, divider_x - px(100.));
    view.update(cx, |shell, _| {
        assert!(
            (shell.mixer.presentation.dock_width - 460.).abs() < 0.5,
            "got {}",
            shell.mixer.presentation.dock_width
        );
    });
    let sink = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph.graph;
        graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Sink)
            .unwrap()
            .id()
    });
    let (dock, output) = cx.update(|window, _| {
        (
            window.find("mixer-dock").bounds(),
            window
                .find(SharedString::from(format!("strip-{sink:?}")))
                .bounds(),
        )
    });
    assert!((f32::from(dock.size.width) - 460.).abs() < 0.5);
    assert!(
        (f32::from(output.right() - dock.right())).abs() <= 1.,
        "the docked strip must stay flush with the resized pane"
    );
    // Hard against the right edge asks for a near-zero dock; the floor keeps
    // one full strip.
    drag_divider_to(cx, dock.right() - px(10.));
    view.update(cx, |shell, _| {
        assert!(
            (shell.mixer.presentation.dock_width - 160.).abs() < 0.5,
            "the one-strip floor must hold, got {}",
            shell.mixer.presentation.dock_width
        );
    });
    assert!(
        (f32::from(dock_width(cx)) - 160.).abs() < 0.5,
        "the laid-out dock must honor the floor"
    );
    // Hard against the left edge asks for the whole row; the 55% cap holds
    // and the divider still tracks the pointer at the clamp.
    drag_divider_to(cx, px(2.));
    let dock = cx.update(|window, _| window.find("mixer-dock").bounds());
    view.update(cx, |shell, _| {
        let clamped = shell.mixer.presentation.dock_width;
        assert!(
            clamped > 460. && clamped < 1600.,
            "the cap must engage inside the requested span, got {clamped}"
        );
        assert!(
            (f32::from(dock.size.width) - clamped).abs() < 0.5,
            "the pane renders the clamped width {clamped}, not {}",
            f32::from(dock.size.width)
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn compact_mixer_keeps_send_controls_and_fixed_width_meters_unclipped(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1024.), px(720.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    cx.update(|window, cx| {
        // The I/O block is density-independent by design: compact or not,
        // every strip carries the same expanded height.
        let io = window
            .find(SharedString::from(format!("io-{id:?}")))
            .bounds();
        assert_eq!(io.size.height, px(super::routing::IO_EXPANDED_H));
        let strip = window
            .find(SharedString::from(format!("strip-{id:?}")))
            .bounds();
        let color_bar = window
            .find(SharedString::from(format!("strip-color-{id:?}")))
            .bounds();
        assert!((f32::from(color_bar.left() - strip.left())).abs() <= 1.);
        assert!((f32::from(color_bar.right() - strip.right())).abs() <= 1.);
        let levels = window
            .find(SharedString::from(format!("strip-levels-{id:?}")))
            .bounds();
        let unity = window
            .find(SharedString::from(format!("strip-unity-{id:?}")))
            .bounds();
        let fader = view.read(cx).mixer.presentation.faders[&id]
            .state
            .read(cx)
            .bounds();
        assert!(
            (f32::from(unity.top() - (fader.top() + fader.size.height * 0.24))).abs() <= 1.,
            "0 dB reference must align with the fader's unity gain position"
        );
        assert!(levels.size.width <= strip.size.width);
        assert!(levels.bottom() <= strip.bottom());
        let readout = window
            .find(SharedString::from(format!("strip-readout-{id:?}")))
            .bounds();
        assert!(readout.bottom() <= levels.top());
        assert!(io.bottom() <= levels.top());
        assert!(strip.bottom() <= px(720.));
        let footer = window
            .find(SharedString::from(format!("strip-header-{id:?}")))
            .bounds();
        assert!(footer.top() >= levels.bottom());
        let meter_readout = window
            .find(SharedString::from(format!("meter-readout-{id:?}")))
            .bounds();
        assert!(
            meter_readout.bottom() <= footer.top(),
            "peak readout must stay above the channel identity"
        );
    });
    let edges = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .edges()
            .iter()
            .filter(|edge| edge.src() == id)
            .map(|edge| edge.id())
            .collect::<Vec<_>>()
    });
    cx.update(|window, _| {
        let strip = window
            .find(SharedString::from(format!("strip-{id:?}")))
            .bounds();
        for edge in edges {
            let tap = window
                .find(SharedString::from(format!("send-pre-{edge:?}")))
                .bounds();
            assert!(
                tap.right() <= strip.right() - px(6.),
                "tap escaped the fixed strip width"
            );
        }
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::AddNode {
                    kind: NodeKind::Gain,
                    channels: 16,
                },
            ));
        });
    });
    poll(&view, cx);
    let surround = view.update(cx, |shell, cx| {
        let id = shell.state.read(cx).snapshot.graph.nodes.last().unwrap().id;
        let first = shell.mixer.presentation.order[0];
        reorder(&mut shell.mixer.presentation.order, id, first);
        cx.notify();
        id
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        let strip = window
            .find(SharedString::from(format!("strip-{surround:?}")))
            .bounds();
        let meter = window
            .find(SharedString::from(format!("meter-reset-{surround:?}")))
            .bounds();
        assert!(meter.left() >= strip.left());
        assert!(
            meter.right() <= strip.right(),
            "actual multichannel bars overflowed the narrow strip"
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn mixer_meter_animation_does_not_rebuild_strips(cx: &mut TestAppContext) {
    use crate::service::{
        AppRequest, InputSelection, ManagedDriver, RunLimit, ServiceRuntime, SessionPhase,
        SessionSpec,
    };
    use std::sync::atomic::Ordering;
    let backend = crate::service::test_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }))
        .unwrap();
    let snapshot = crate::ui::shell::test_harness::wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let node = snapshot.graph.graph.nodes()[0].id();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| crate::ui::state::UiState::new(handle, snapshot));
    let (view, cx) = shell_with_state(cx, state);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| {
        assert!(
            !window.is_a11y_active(),
            "this regression measures the accessibility-inactive cache path"
        );
        window.render_frame(cx);
    });
    // Live publications must arm the animation loop before the steady-state
    // strip measurements begin.
    wait_until(Duration::from_secs(3), || {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |shell, cx| {
            shell.state.read(cx).node_meters.raw_levels(node).is_some()
        })
    });
    // The inspector's closing spring is a real layout change, not a meter
    // invalidation. Let it finish before measuring steady-state frames.
    std::thread::sleep(Duration::from_millis(900));
    for _ in 0..3 {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
    let before = view.read_with(cx, |shell, _| {
        let trace = shell.trace.borrow();
        (trace.strip_renders, trace.meter_paints)
    });
    cx.update(|window, _| {
        let toolbar = window.find("app-toolbar").bounds();
        let status = window.find("engine-statusbar").bounds();
        assert_eq!(toolbar.size.width, px(1440.));
        assert_eq!(status.size.width, toolbar.size.width);
    });
    for _ in 0..10 {
        cx.update(|window, cx| {
            assert!(window.simulate_next_frame(cx) > 0);
            window.draw(cx).clear(cx);
        });
    }
    let after = view.read_with(cx, |shell, _| {
        let trace = shell.trace.borrow();
        assert!(
            shell.metering.node_meter_points.borrow().contains(&node),
            "cached meter demand must survive"
        );
        (trace.strip_renders, trace.meter_paints)
    });
    assert_eq!(
        after.0, before.0,
        "meter animation should only redraw meters"
    );
    assert!(
        after.1 >= before.1 + 10,
        "the meter canvas must actually paint"
    );
    // The toolbar and status bar rebuild every frame by necessity: their
    // window drag regions must re-register their hitboxes per frame (the
    // splice does not replay window-control registrations). The cached
    // invariants that matter are the strips and their meters.

    // Service elapsed-time publications notify the shell, not Mixer controls.
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            std::sync::Arc::make_mut(&mut state.snapshot).elapsed += Duration::from_secs(1);
        });
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders),
        after.0,
        "control polling must not invalidate unchanged strips"
    );

    // A real control edit invalidates the cache without a forced refresh.
    // (Selection styles the live column since the routing split, so the
    // cached body's edit flag steps in as the trigger instead.)
    view.update(cx, |shell, cx| {
        shell.mixer.presentation.editing_value = Some(node);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders) > after.0);

    let controls_before = view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders);
    let fader = view.read_with(cx, |shell, _| {
        shell.mixer.presentation.faders[&node].state.clone()
    });
    cx.update(|window, cx| fader.update(cx, |slider, cx| slider.set_value(0.25, window, cx)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders) > controls_before,
        "slider state notifications must invalidate cached thumb geometry"
    );

    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Settings {
            resume: shell.presentation.workspace.page(),
        };
        cx.notify();
    });
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| {
        assert_eq!(
            window.simulate_next_frame(cx),
            0,
            "settings must stop the meter frame loop"
        );
    });
    finish_service(runtime);
}

/// Silence parks every observed layer at the scale bottom, so the vsync loop
/// must stand down instead of redrawing idle meters; fresh data re-arms it
/// through the control-side pump exactly as the desktop poll would.
#[gpui_kit::test]
fn mixer_meter_silence_stands_down_the_frame_loop_until_data_returns(cx: &mut TestAppContext) {
    use crate::service::{
        AppRequest, InputSelection, ManagedDriver, RunLimit, ServiceRuntime, SessionPhase,
        SessionSpec,
    };
    use std::sync::atomic::Ordering;
    let backend = crate::service::test_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let backend_state = backend.0.clone();
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }))
        .unwrap();
    let snapshot = crate::ui::shell::test_harness::wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let node = snapshot.graph.graph.nodes()[0].id();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| crate::ui::state::UiState::new(handle, snapshot));
    let (view, cx) = shell_with_state(cx, state);
    cx.simulate_resize(size(px(1024.), px(720.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    // Live full-scale audio keeps the loop armed.
    wait_until(Duration::from_secs(3), || {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |shell, cx| {
            let state = shell.state.read(cx);
            state.node_meters.raw_levels(node).is_some()
                && state.node_meters.channel_levels(node).is_some()
        })
    });
    // Digital silence (no publications) parks every layer at the scale
    // bottom; the loop must stop requesting frames entirely.
    backend_state.pause_audio.store(true, Ordering::SeqCst);
    wait_until(Duration::from_secs(3), || {
        cx.update(|window, cx| {
            let frames = window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
            frames == 0
        })
    });
    // Fresh data re-arms the loop: the control-side pump notices the raw
    // change, notifies, and the canvas requests frames again.
    backend_state.pause_audio.store(false, Ordering::SeqCst);
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            if shell.pump_meters(cx) {
                cx.notify();
            }
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.simulate_next_frame(cx) > 0
        })
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn mixer_phase_only_start_restores_legacy_meter_probes(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        shell.state.update(cx, |state, _| {
            let snapshot = std::sync::Arc::make_mut(&mut state.snapshot);
            snapshot.desired = Some(crate::service::SessionSpec {
                source: crate::service::InputSelection::Tone { frequency_hz: 440. },
                output_endpoint_id: "output".into(),
                limit: crate::service::RunLimit::UntilStopped,
                gain: 1.,
                pan: 0.,
                max_block_frames: 256,
            });
            let graph = std::sync::Arc::make_mut(&mut snapshot.graph);
            for entry in &mut graph.nodes {
                if graph.graph.get_node(entry.id).unwrap().kind() == NodeKind::Source {
                    entry.source = snapshot.desired.as_ref().map(|spec| spec.source.clone());
                }
                if graph.graph.get_node(entry.id).unwrap().kind() == NodeKind::Sink {
                    entry.output_endpoint_id = Some("output".into());
                }
            }
        });
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    std::thread::sleep(Duration::from_millis(900));
    for _ in 0..3 {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
    assert_eq!(
        view.read_with(cx, |shell, _| shell.metering.meter_points.get()),
        (false, false)
    );
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            std::sync::Arc::make_mut(&mut state.snapshot).phase =
                crate::service::SessionPhase::Running
        });
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        view.read_with(cx, |shell, _| shell.metering.meter_points.get()),
        (true, true)
    );
    finish_service(runtime);
}

/// The per-strip split (design 17): a publish that changes one strip's
/// content rebuilds exactly that strip's panel — its siblings compare their
/// keys equal and replay their cached paint. Under the whole-page panel this
/// same edit paid one full strip-tree render per publish.
#[gpui_kit::test]
fn mixer_publish_rebuilds_only_the_changed_strip(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    // The inspector's closing spring notifies its way through the first
    // second; let it finish or every frame rebuilds every panel.
    crate::ui::shell::test_harness::settle_panels(cx);
    let target = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let muted = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(target)
                .is_some_and(|node| node.mixer.muted)
        })
    };
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.edit_mixer(target, |state| state.muted = !state.muted)
        });
    });
    // Land the publish without rendering: `render_frame` refreshes the whole
    // window and would rebuild every panel regardless of the caches.
    wait_until(Duration::from_secs(2), || muted(&view, cx));
    let before = view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders);
    cx.update(|window, cx| {
        assert!(!window.is_a11y_active(), "measuring the cached path");
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let after = view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders);
    assert_eq!(
        after - before,
        1,
        "only the edited strip's panel may rebuild; siblings must replay"
    );
    finish_service(runtime);
}

/// A small press-drag on the fader must track the pointer: the thumb moves a
/// couple of pixels, the confirmed publishes land mid-drag, the release
/// commits — and the value never snaps to the rail (−inf). Regression for a
/// probabilistic mapping fault where a tiny drag ended at digital silence.
#[gpui_kit::test]
fn mixer_fader_small_drag_tracks_the_pointer_and_never_snaps_to_silence(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::shell::test_harness::settle_panels(cx);
    let node = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    // Start from unity (0 dB): thumb at FaderScale position 0.24.
    let start_value = 1. - FaderScale::position(1.);
    let slider = view.read_with(cx, |shell, _| {
        shell.mixer.presentation.faders[&node].state.clone()
    });
    cx.update(|window, cx| {
        slider.update(cx, |slider, cx| slider.set_value(start_value, window, cx))
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (thumb_x, thumb_y) = view.update(cx, |shell, cx| {
        let bounds = shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .bounds();
        (
            bounds.center().x,
            bounds.top() + bounds.size.height * FaderScale::position(1.),
        )
    });
    let press = point(thumb_x, thumb_y);
    cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
    // Four small downward moves with frames in between: publishes land
    // mid-drag exactly as they do on a real machine.
    for step in 0..4 {
        cx.simulate_mouse_move(
            press + point(px(0.), px(2. * (step as f32) + 2.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
    }
    // Mid-drag sanity: the value tracks downward, never the rail.
    let mid_value = view.update(cx, |shell, cx| {
        shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .value()
            .end()
    });
    assert!(
        mid_value > 0.5 && mid_value < start_value,
        "mid-drag value {mid_value} must track between unity and the rail"
    );
    cx.simulate_mouse_up(
        press + point(px(0.), px(10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    let released = view.read_with(cx, |shell, cx| {
        shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .value()
            .end()
    });
    assert!(
        released > 0.5 && released < start_value,
        "the release must land between unity and the rail, got {released}"
    );
    // The dispatch rides the mailbox: until a poll consumes the published
    // confirmation, the UI snapshot still holds an older gain, and every
    // rendered frame runs the snapshot write-back. The thumb must hold the
    // user's value through that window instead of snapping back.
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let held = view.read_with(cx, |shell, cx| {
            shell.mixer.presentation.faders[&node]
                .state
                .read(cx)
                .value()
                .end()
        });
        assert_eq!(
            held, released,
            "the thumb left the released value before confirmation"
        );
    }
    // A stale publication — the release echo race: a revision the UI has
    // not seen, built before the service consumed the release, so it still
    // carries an older gain. The revision fence must keep it off the thumb
    // entirely; without it the thumb snapped back by the last dispatch
    // tick's travel and forward again on the next publish (design 22 R1).
    view.update(cx, |shell, cx| {
        let stale_gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(node)
            .unwrap()
            .mixer
            .gain;
        shell.state.update(cx, |state, _| {
            let snapshot = std::sync::Arc::make_mut(&mut state.snapshot);
            let graph = std::sync::Arc::make_mut(&mut snapshot.graph);
            graph.candidate_revision += 1;
            if let Some(entry) = graph.nodes.iter_mut().find(|entry| entry.id == node) {
                entry.mixer.gain = stale_gain;
            }
        });
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let shielded = view.read_with(cx, |shell, cx| {
        shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .value()
            .end()
    });
    assert_eq!(
        shielded, released,
        "a stale publication pulled the just-released thumb back"
    );
    poll(&view, cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (final_value, final_db) = view.update(cx, |shell, cx| {
        let value = shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .value()
            .end();
        let gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(node)
            .unwrap()
            .mixer
            .gain;
        (value, 20. * gain.log10())
    });
    assert!(
        final_value > 0.5,
        "a small drag must not land on the rail (value {final_value}, {final_db} dB)"
    );
    assert!(
        (-6. ..=0.01).contains(&final_db),
        "a ~10px drag from unity landed at {final_db} dB"
    );
    finish_service(runtime);
}

/// The fader readout renders the slider's own value while the slider leads
/// the snapshot (design 22 D1): mid-drag and after the release until the
/// service confirms it, the number tracks the thumb instead of trailing it
/// by a round trip; the confirmation then swaps in the published gain
/// without moving the number, because the edit dispatched exactly that gain.
#[gpui_kit::test]
fn fader_readout_tracks_the_thumb_until_the_service_confirms_it(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);
    let node = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    // (readout, published, the thumb's own) gains.
    let gains = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| {
        view.read_with(cx, |shell, cx| {
            let slot = &shell.mixer.presentation.faders[&node];
            let published = shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(node)
                .expect("strip node")
                .mixer
                .gain;
            let own = FaderScale::gain(1. - slot.state.read(cx).value().end());
            (
                fader_readout_gain(Some(slot), published, cx),
                published,
                own,
            )
        })
    };
    let (readout, published, _) = gains(&view, cx);
    assert_eq!(
        readout, published,
        "at rest the readout is the published gain"
    );
    let press = view.update(cx, |shell, cx| {
        let slider = shell.mixer.presentation.faders[&node].state.read(cx);
        let bounds = slider.bounds();
        point(
            bounds.center().x,
            bounds.top() + bounds.size.height * (1. - slider.value().end()),
        )
    });
    cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
    // The first move past the threshold starts the drag; the second one
    // moves the value.
    for travel in [12., 24.] {
        cx.simulate_mouse_move(
            press + point(px(0.), px(travel)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
    }
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (readout, published, own) = gains(&view, cx);
    assert_eq!(readout, own, "mid-drag the readout shows the thumb's value");
    assert_ne!(readout, published, "not the published gain it left behind");
    cx.simulate_mouse_up(
        press + point(px(0.), px(24.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    let (released, published, own) = gains(&view, cx);
    assert_eq!(
        released, own,
        "until the service confirms the release, the readout keeps the thumb's value"
    );
    assert_ne!(released, published);
    poll(&view, cx);
    let (confirmed, published, _) = gains(&view, cx);
    assert_eq!(
        confirmed, published,
        "confirmed: the readout is the published gain"
    );
    assert_eq!(
        confirmed, released,
        "the confirmation must not move the number"
    );
    finish_service(runtime);
}

/// The user's repro: select the strip by clicking its bottom header, then
/// drag the fader a tiny bit — the thumb must not lock onto -inf. The
/// selection rebuilds the strip and (when the inspector is open) rewires
/// the node's meter visibility and the inspector panel; none of that may
/// corrupt the shared slider state.
#[gpui_kit::test]
fn mixer_fader_small_drag_survives_strip_selection(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::shell::test_harness::settle_panels(cx);
    let node = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    // Select the strip the way the header click does.
    view.update(cx, |shell, cx| {
        shell.selection.node = Some(node);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let start_value = 1. - FaderScale::position(1.);
    let slider = view.read_with(cx, |shell, _| {
        shell.mixer.presentation.faders[&node].state.clone()
    });
    cx.update(|window, cx| {
        slider.update(cx, |slider, cx| slider.set_value(start_value, window, cx))
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (thumb_x, thumb_y) = view.update(cx, |shell, cx| {
        let bounds = shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .bounds();
        (
            bounds.center().x,
            bounds.top() + bounds.size.height * FaderScale::position(1.),
        )
    });
    let press = point(thumb_x, thumb_y);
    cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
    for step in 0..4 {
        cx.simulate_mouse_move(
            press + point(px(0.), px(2. * (step as f32) + 2.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
    }
    cx.simulate_mouse_up(
        press + point(px(0.), px(10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        let held = view.read_with(cx, |shell, cx| {
            shell.mixer.presentation.faders[&node]
                .state
                .read(cx)
                .value()
                .end()
        });
        assert!(
            held > 0.5,
            "with the strip selected the thumb snapped to {held}"
        );
    }
    poll(&view, cx);
    let final_db = view.update(cx, |shell, cx| {
        let gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(node)
            .unwrap()
            .mixer
            .gain;
        20. * gain.log10()
    });
    assert!(
        (-6. ..=0.01).contains(&final_db),
        "a ~10px drag from unity landed at {final_db} dB with the strip selected"
    );
    // And the same drag with the inspector open: the inspector shares the
    // slider state and rebuilds per publish.
    view.update(cx, |shell, cx| {
        shell.open_inspector();
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::shell::test_harness::settle_panels(cx);
    cx.update(|window, cx| {
        slider.update(cx, |slider, cx| slider.set_value(start_value, window, cx))
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (thumb_x, thumb_y) = view.update(cx, |shell, cx| {
        let bounds = shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .bounds();
        (
            bounds.center().x,
            bounds.top() + bounds.size.height * FaderScale::position(1.),
        )
    });
    let press = point(thumb_x, thumb_y);
    cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
    for step in 0..4 {
        cx.simulate_mouse_move(
            press + point(px(0.), px(2. * (step as f32) + 2.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
    }
    cx.simulate_mouse_up(
        press + point(px(0.), px(10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let held = view.read_with(cx, |shell, cx| {
            shell.mixer.presentation.faders[&node]
                .state
                .read(cx)
                .value()
                .end()
        });
        assert!(
            held > 0.5,
            "with the inspector open the thumb snapped to {held}"
        );
    }
    poll(&view, cx);
    let final_db = view.update(cx, |shell, cx| {
        let gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(node)
            .unwrap()
            .mixer
            .gain;
        20. * gain.log10()
    });
    assert!(
        (-6. ..=0.01).contains(&final_db),
        "a ~10px drag from unity landed at {final_db} dB with the inspector open"
    );
    finish_service(runtime);
}

/// Same small drag, but the window was resized after the strips laid out:
/// the strips must re-derive their pointer mapping from the new geometry
/// instead of mapping against the pre-resize track and clamping to the rail.
#[gpui_kit::test]
fn mixer_fader_small_drag_after_resize_tracks_the_new_geometry(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::shell::test_harness::settle_panels(cx);
    let node = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let start_value = 1. - FaderScale::position(1.);
    let slider = view.read_with(cx, |shell, _| {
        shell.mixer.presentation.faders[&node].state.clone()
    });
    cx.update(|window, cx| {
        slider.update(cx, |slider, cx| slider.set_value(start_value, window, cx))
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    // A real layout change the strip key does not mention: the window is
    // shortened, so every strip's height (and its fader track) changes.
    cx.simulate_resize(size(px(1440.), px(640.)));
    view.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let (thumb_x, thumb_y, _track_height) = view.update(cx, |shell, cx| {
        let bounds = shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .bounds();
        (
            bounds.center().x,
            bounds.top() + bounds.size.height * FaderScale::position(1.),
            f32::from(bounds.size.height),
        )
    });
    let press = point(thumb_x, thumb_y);
    cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
    for step in 0..4 {
        cx.simulate_mouse_move(
            press + point(px(0.), px(2. * (step as f32) + 2.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
    }
    cx.simulate_mouse_up(
        press + point(px(0.), px(10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    poll(&view, cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (final_value, final_db) = view.update(cx, |shell, cx| {
        let value = shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .value()
            .end();
        let gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(node)
            .unwrap()
            .mixer
            .gain;
        (value, 20. * gain.log10())
    });
    assert!(
        final_value > 0.5,
        "a small drag after a resize must not land on the rail (value {final_value}, {final_db} dB)"
    );
    assert!(
        (-6. ..=0.01).contains(&final_db),
        "a ~10px drag after a resize landed at {final_db} dB"
    );
    finish_service(runtime);
}

/// Undo must outrank an in-flight fader drag (audit SD-7): the drag holds the
/// snapshot write-back down, so an undo landing mid-drag used to stay
/// invisible (thumb pinned to the pointer) and the release then dispatched
/// the stale pointer value over the undone gain — undo seemed to work only
/// sometimes. The journal step now cancels the active drag: the write-back
/// reopens so the thumb follows the undone value, and the release drops the
/// gesture's stale tail instead of landing it as a fresh edit.
#[gpui_kit::test]
fn undo_during_a_fader_drag_lands_and_the_release_cannot_overwrite_it(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::shell::test_harness::settle_panels(cx);
    let node = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let gain_of = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> f64 {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(node)
                .expect("strip node")
                .mixer
                .gain
        })
    };
    // A journaled baseline edit gives the undo something to restore: 0.5
    // (-6 dB). The drag below will push the journal past it.
    view.update(cx, |shell, cx| {
        let mixer = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(node)
            .expect("strip node")
            .mixer;
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::SetMixer {
                    node,
                    state: crate::service::MixerState { gain: 0.5, ..mixer },
                },
            ));
        });
    });
    wait_until(Duration::from_secs(2), || {
        (gain_of(&view, cx) - 0.5).abs() < 1e-9
    });
    // Same-node mixer edits merge into one undo step inside the journal's
    // 500 ms coalesce window; the baseline must fall outside it so the undo
    // steps back to 0.5 rather than past it to unity.
    std::thread::sleep(Duration::from_millis(600));
    let undo_position = 1. - FaderScale::position(0.5);
    // Press below the -6 dB thumb and hold: the press revalues the slider and
    // its Change dispatches immediately, so the journal head is the press's
    // own value and the pointer owns the thumb (write-back suppressed).
    let press = view.update(cx, |shell, cx| {
        let bounds = shell.mixer.presentation.faders[&node]
            .state
            .read(cx)
            .bounds();
        point(
            bounds.center().x,
            bounds.top() + bounds.size.height * FaderScale::position(0.5) + px(8.),
        )
    });
    cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
    wait_until(Duration::from_secs(2), || gain_of(&view, cx) < 0.5);
    view.update(cx, |shell, cx| {
        assert!(
            shell.state.read(cx).snapshot.graph.can_undo,
            "the drag's dispatch must be journaled before the undo"
        );
        assert_ne!(
            shell.mixer.presentation.faders[&node]
                .state
                .read(cx)
                .value()
                .end(),
            undo_position,
            "the dragging thumb must sit at the pointer, not at the snapshot"
        );
    });
    // Undo mid-drag. The undo publishes the restored gain while the pointer
    // is still down; deliberately no frame renders in between, so the slider
    // state still carries the stale pointer value when the release lands.
    // The journal step is invoked directly: the mixer page keeps no element
    // under the shell's focus handle for ctrl-z to bubble through (the
    // keystroke routing itself is covered by the window-level undo tests).
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| shell.undo_redo_graph(false, window, cx));
    });
    wait_until(Duration::from_secs(2), || {
        (gain_of(&view, cx) - 0.5).abs() < 1e-9
    });
    cx.simulate_mouse_up(press, MouseButton::Left, Modifiers::default());
    // The cancelled gesture's release must not re-land its stale value: drain
    // the mailbox and give any (wrongly) dispatched edit time to publish.
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
    }
    let (gain, thumb) = view.update(cx, |shell, cx| {
        (
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(node)
                .expect("strip node")
                .mixer
                .gain,
            shell.mixer.presentation.faders[&node]
                .state
                .read(cx)
                .value()
                .end(),
        )
    });
    assert!(
        (gain - 0.5).abs() < 1e-9,
        "the release must not overwrite the undone gain, got {gain}"
    );
    assert!(
        (thumb - undo_position).abs() < 1e-6,
        "the thumb must follow the undone value after the drag was cancelled, got {thumb}"
    );
    finish_service(runtime);
}

/// The capture-tap monitor meters must not keep the frame loop armed for a
/// display the node level topics already cover. The Graph page mounts no
/// legacy probes and stands down at silence; the Mixer page's pre-fader tap
/// keeps producing signal even when the fader is muted, so a live tap
/// subscription pinned the whole page to display-rate frames and burned a
/// core.
#[gpui_kit::test]
fn mixer_stands_down_while_the_capture_tap_still_has_signal(cx: &mut TestAppContext) {
    use crate::service::{
        AppRequest, InputSelection, ManagedDriver, RunLimit, ServiceRuntime, SessionPhase,
        SessionSpec,
    };
    use std::sync::atomic::Ordering;
    let backend = crate::service::test_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let backend_state = backend.0.clone();
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }))
        .unwrap();
    let snapshot = crate::ui::shell::test_harness::wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let node = snapshot.graph.graph.nodes()[0].id();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| crate::ui::state::UiState::new(handle, snapshot));
    let (view, cx) = crate::ui::shell::test_harness::shell_with_state(cx, state);
    cx.simulate_resize(size(px(1024.), px(720.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::shell::test_harness::settle_panels(cx);
    view.update(cx, |shell, cx| {
        assert!(
            shell.metering.meter_points.get().0,
            "the source strip's capture probe must be mounted on the page"
        );
        assert!(
            shell
                .state
                .read(cx)
                .snapshot
                .signal_topics
                .node_levels
                .contains_key(&node),
            "the source node's level topic must cover its meter"
        );
    });
    // Park the session at digital silence (the fake engine ignores mixer
    // mutes, so the silence fixture pauses its render loop instead). The
    // probe stays mounted and the node stays managed: the legacy capture
    // tap must not be subscribed, and its motion must not hold the frame
    // arbiter open for a display the node topic already covers.
    backend_state.pause_audio.store(true, Ordering::SeqCst);
    // The ballistics need a few seconds to fall from full scale to rest.
    crate::ui::shell::test_harness::wait_until(Duration::from_secs(8), || {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |shell, cx| {
            shell.state.read(cx).node_meters.at_rest() && !shell.frames_wanted(cx)
        })
    });
    view.read_with(cx, |shell, cx| {
        assert!(
            shell.metering.meter_points.get().0,
            "the probe must stay mounted while the page is at rest"
        );
        assert!(
            !shell.metering.meters.input_subscribed(),
            "the capture tap stayed subscribed for a display the node topic covers"
        );
        assert!(
            !shell.frames_wanted(cx),
            "the frame loop must stand down once every displayed meter is at rest"
        );
    });
    finish_service(runtime);
}

/// Exercise a real managed publisher: painting cannot pass just by reusing
/// frozen levels or silently dropping visibility/subscription demand.
#[gpui_kit::test]
fn mixer_cached_controls_keep_live_levels_clip_reset_and_scroll_geometry(cx: &mut TestAppContext) {
    use crate::service::{
        AppRequest, GraphEdit, InputSelection, ManagedDriver, RunLimit, ServiceRuntime,
        SessionPhase, SessionSpec,
    };
    use std::sync::{Arc, atomic::Ordering};
    let backend = crate::service::test_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let backend_state = backend.0.clone();
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }))
        .unwrap();
    let snapshot = crate::ui::shell::test_harness::wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let node = snapshot.graph.graph.nodes()[0].id();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| crate::ui::state::UiState::new(handle, snapshot));
    let (view, cx) = shell_with_state(cx, state);
    cx.simulate_resize(size(px(1024.), px(720.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    wait_until(Duration::from_secs(3), || {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |shell, cx| {
            shell.state.read(cx).node_meters.raw_levels(node).is_some()
        })
    });
    let levels = view.read_with(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .node_meters
            .raw_levels(node)
            .unwrap()
            .to_vec()
    });
    assert!(
        levels[0].peak >= 1.,
        "the fake physical source is full scale"
    );
    assert!(view.read_with(cx, |shell, cx| {
        shell.state.read(cx).node_meters.channel_clips(node)[0]
    }));
    crate::ui::shell::test_harness::settle_panels(cx);
    backend_state.pause_audio.store(true, Ordering::SeqCst);
    // Drain any already-published windows before resetting the clip latch.
    std::thread::sleep(Duration::from_millis(20));
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let (bar_before, strips_before, paints_before) = view.read_with(cx, |shell, cx| {
        let state = shell.state.read(cx);
        let trace = shell.trace.borrow();
        (
            state.node_meters.channel_levels(node).unwrap()[0].bar,
            trace.strip_renders,
            trace.meter_paints,
        )
    });
    std::thread::sleep(Duration::from_millis(30));
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    view.read_with(cx, |shell, cx| {
        let state = shell.state.read(cx);
        let trace = shell.trace.borrow();
        assert_eq!(
            state.node_meters.channel_levels(node).unwrap()[0].bar,
            bar_before,
            "a publisher stall must hold the bar instead of reading as a dip"
        );
        assert_eq!(trace.strip_renders, strips_before);
        assert!(
            trace.meter_paints > paints_before,
            "cached controls must not freeze the meter pump"
        );
    });
    cx.update(|window, cx| window.click(SharedString::from(format!("meter-reset-{node:?}")), cx));
    assert!(view.read_with(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .node_meters
            .channel_clips(node)
            .is_empty()
    }));

    // Add enough strips to overflow. Both the cache and overlay geometry must
    // follow an ordinary notified edit; no test-only forced refresh here.
    for _ in 0..5 {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                    kind: NodeKind::Gain,
                    channels: 2,
                }))
            })
        });
        poll(&view, cx);
    }
    let before = view.read_with(cx, |shell, _| {
        shell.mixer.geometry.track_bounds(node).unwrap()
    });
    view.update(cx, |shell, cx| {
        shell.mixer.scroll.set_offset(point(px(-160.), px(0.)));
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let after = view.read_with(cx, |shell, _| {
        shell.mixer.geometry.track_bounds(node).unwrap()
    });
    assert_eq!(after.left(), before.left() - px(160.));
    assert!(
        !view.read_with(cx, |shell, _| shell
            .metering
            .node_meter_points
            .borrow()
            .contains(&node)),
        "offscreen strips must not remain demanded"
    );
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            Arc::make_mut(&mut state.snapshot).phase = SessionPhase::Idle
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    assert!(view.read_with(cx, |shell, cx| {
        shell.state.read(cx).node_meters.raw_levels(node).is_none()
    }));
    finish_service(runtime);
}

/// The strip readout must speak the inspector's marker semantics (audit
/// SD-6): a full-scale transient followed by silence walks the number back
/// down with the decaying marker instead of staying pinned at the retained
/// raw peak's all-time maximum, and the text is spelled by the same
/// `dbfs_text` the inspector's "Peak" row uses. The readout's shaping is
/// shared across strips and keyed by text: held frames re-shape nothing,
/// every distinct dB string shapes exactly once, and no per-node cache
/// entry exists to outlive a deleted node (audit HC-7).
#[gpui_kit::test]
fn mixer_strip_readouts_follow_the_marker_and_share_one_shape_per_text(cx: &mut TestAppContext) {
    use crate::service::{
        AppRequest, InputSelection, ManagedDriver, RunLimit, ServiceRuntime, SessionPhase,
        SessionSpec,
    };
    use crate::ui::presentation::dbfs_text;
    use std::sync::atomic::Ordering;
    let full_scale = dbfs_text(1.);
    let backend = crate::service::test_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }))
        .unwrap();
    let snapshot = crate::ui::shell::test_harness::wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let node = snapshot.graph.graph.nodes()[0].id();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| crate::ui::state::UiState::new(handle, snapshot));
    let (view, cx) = shell_with_state(cx, state);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    // The fake physical source runs full scale, so every visible strip's
    // readout holds the same "+0.0 dBFS" text. Warm up until every demanded
    // node is observed and painted at that held value.
    wait_until(Duration::from_secs(3), || {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |shell, cx| {
            let surface = shell.mixer.frame.read(cx).surface();
            let surface = surface.borrow();
            shell.metering.node_meter_points.borrow().iter().all(|&id| {
                shell
                    .state
                    .read(cx)
                    .node_meters
                    .channel_levels(id)
                    .is_some()
                    && surface
                        .last_readout(id)
                        .is_some_and(|text| text == full_scale)
            }) && surface
                .last_readout(node)
                .is_some_and(|text| text == full_scale)
        })
    });
    let (shapes_held, entries_held, paints_held) = view.read_with(cx, |shell, cx| {
        let surface = shell.mixer.frame.read(cx).surface();
        let (shapes, entries) = surface.borrow().readout_shape_stats();
        (shapes, entries, shell.trace.borrow().meter_paints)
    });
    assert_eq!(entries_held, shapes_held, "one entry per distinct text");
    for _ in 0..2 {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
    view.read_with(cx, |shell, cx| {
        let surface = shell.mixer.frame.read(cx).surface();
        let (shapes, entries) = surface.borrow().readout_shape_stats();
        assert_eq!(
            shapes, shapes_held,
            "every strip holds the same text: held frames must replay the shaped line"
        );
        assert_eq!(
            entries, shapes,
            "each shaper run inserted exactly one distinct text"
        );
        assert!(
            shell.trace.borrow().meter_paints > paints_held,
            "held frames actually painted readouts"
        );
    });

    // Silence the fader: publications continue (silent windows), the bar
    // falls immediately, the marker holds its half second and then falls at
    // the default Fast rate (40 dB/s). The readout must follow it down
    // instead of staying latched at the transient's maximum.
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.edit_mixer(node, |state| state.muted = true)
        });
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(node)
                .is_some_and(|node| node.mixer.muted)
        })
    });
    // Keep pumping across the hold and into the fall: a single frame after a
    // long sleep would drain a backlog of stale pre-mute windows from the
    // bounded queue and re-anchor the marker, instead of observing the decay
    // the display sees at real frame rates.
    let decay = std::time::Instant::now() + Duration::from_millis(1000);
    while std::time::Instant::now() < decay {
        std::thread::sleep(Duration::from_millis(30));
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
    let (shapes_fallen, readout, inspector) = view.read_with(cx, |shell, cx| {
        let surface = shell.mixer.frame.read(cx).surface();
        let surface = surface.borrow();
        let (shapes, entries) = surface.readout_shape_stats();
        assert_eq!(
            entries, shapes,
            "the shared cache still holds one entry per text"
        );
        let marker = shell
            .state
            .read(cx)
            .node_meters
            .channel_levels(node)
            .unwrap()[0]
            .marker;
        (
            shapes,
            surface.last_readout(node).unwrap(),
            dbfs_text(marker),
        )
    });
    assert!(
        shapes_fallen > shapes_held,
        "the falling marker must re-shape: the old raw-peak readout never changed again"
    );
    assert_eq!(
        readout, inspector,
        "the strip readout equals the inspector's Peak spelling at the same instant"
    );
    assert_ne!(
        readout, full_scale,
        "the transient's maximum is no longer pinned"
    );
    finish_service(runtime);
}

#[gpui_kit::test]
fn routing_actions_preserve_send_gain_select_the_edge_and_keep_apply_explicit(
    cx: &mut TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let (edge, gain, dst, ports) = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph.graph;
        let edge = &graph.edges()[0];
        (
            edge.id(),
            edge.params().gain,
            edge.dst(),
            (edge.src_port(), edge.dst_port()),
        )
    });
    cx.update(|window, cx| window.click(SharedString::from(format!("send-mute-{edge:?}")), cx));
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        let edge = graph.graph.get_edge(edge).unwrap();
        assert!(edge.params().mute);
        assert_eq!(edge.params().gain, gain);
        assert_eq!(graph.active_revision, None);
    });
    cx.update(|window, cx| window.click(SharedString::from(format!("input-{edge:?}")), cx));
    view.update(cx, |shell, _| {
        assert_eq!(shell.selection.route, Some(ports));
        assert!(shell.chrome.inspector_open);
        assert_eq!(shell.selection.node, None);
    });
    // Closing the inspector restores room without changing route identity.
    view.update(cx, |shell, cx| {
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let (slider, bounds) = view.update(cx, |shell, cx| {
        let slider = shell.mixer.presentation.faders[&dst].state.clone();
        let bounds = slider.read(cx).bounds();
        (slider, bounds)
    });
    let minus_six = FaderScale::position(10f64.powf(-6. / 20.));
    let target = point(
        bounds.center().x,
        bounds.top() + bounds.size.height * minus_six,
    );
    cx.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        let db = 20. * graph.node(dst).unwrap().mixer.gain.log10();
        assert!(
            (db + 6.).abs() < 0.02,
            "shared inverse pointer mapping produced {db} dB"
        );
        assert!(slider.read(cx).value().end().is_finite());
        assert_eq!(graph.active_revision, None);
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn balance_drag_targets_its_own_strip_and_double_click_centers_it(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let bounds = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("balance-track-{id:?}")))
            .bounds()
    });
    let target = point(bounds.left() + bounds.size.width * 0.75, bounds.center().y);
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        bounds.center() + point(px(12.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert!(
            (graph.node(id).unwrap().mixer.balance - 0.5).abs() < 0.02,
            "dragging the first strip must use that strip's balance bounds"
        );
        assert!(
            graph
                .nodes
                .iter()
                .filter(|node| node.id != id)
                .all(|node| node.mixer.balance == 0.),
            "other strips must keep their balance"
        );
        assert_eq!(graph.active_revision, None);
    });
    cx.simulate_event(gpui_kit::MouseDownEvent {
        position: target,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui_kit::MouseUpEvent {
        position: target,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .unwrap()
                .mixer
                .balance,
            0.
        );
    });
    finish_service(runtime);
}

/// A wheel notch is `Lines { y: 1. }` — gpui's positive wheel delta means the
/// wheel moved up, which must step the balance right (+0.1), matching the
/// inspector's Left/Right buttons' sign convention. Wheel steps ride the same
/// dispatch as the drag edits and clamp to the ±1 track limits.
#[gpui_kit::test]
fn balance_wheel_steps_a_tenth_and_clamps_at_the_limits(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let set_balance = |view: &Entity<StudioShell>, cx: &mut VisualTestContext, balance: f64| {
        view.update(cx, |shell, cx| {
            let mixer = shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .expect("strip node")
                .mixer;
            shell.state.update(cx, |state, _| {
                state.dispatch(crate::service::AppRequest::EditGraph(
                    crate::service::GraphEdit::SetMixer {
                        node: id,
                        state: crate::service::MixerState { balance, ..mixer },
                    },
                ));
            });
        });
        poll(view, cx);
    };
    let balance = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> f64 {
        view.update(cx, |shell, cx| {
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .expect("strip node")
                .mixer
                .balance
        })
    };
    let notch = |view: &Entity<StudioShell>, cx: &mut VisualTestContext, y: f32| {
        let center = cx.update(|window, _| {
            window
                .find(SharedString::from(format!("balance-track-{id:?}")))
                .bounds()
                .center()
        });
        cx.simulate_event(ScrollWheelEvent {
            position: center,
            delta: ScrollDelta::Lines(point(0., y)),
            ..Default::default()
        });
        poll(view, cx);
    };

    assert_eq!(balance(&view, cx), 0.);
    // Wheel up moves the balance right by one tenth.
    notch(&view, cx, 1.);
    assert!((balance(&view, cx) - 0.1).abs() < 1e-9);
    // Wheel down moves it back left, past center, by the same step.
    notch(&view, cx, -1.);
    notch(&view, cx, -1.);
    assert!((balance(&view, cx) + 0.1).abs() < 1e-9);

    // Stepping clamps at the right limit instead of overshooting it.
    set_balance(&view, cx, 0.95);
    notch(&view, cx, 1.);
    assert_eq!(balance(&view, cx), 1.);
    notch(&view, cx, 1.);
    assert_eq!(balance(&view, cx), 1.);
    set_balance(&view, cx, -0.95);
    notch(&view, cx, -1.);
    assert_eq!(balance(&view, cx), -1.);
    finish_service(runtime);
}

#[gpui_kit::test]
fn send_numeric_edit_and_deleted_edge_cleanup_preserve_shared_layout(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let edge = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.graph.edges()[0].id()
    });
    let position = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("send-value-{edge:?}")))
            .bounds()
            .center()
    });
    cx.simulate_event(gpui_kit::MouseDownEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui_kit::MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            assert_eq!(shell.mixer.presentation.editing_send, Some(edge));
            shell.mixer.presentation.send_inputs[&edge].update(cx, |input, cx| {
                input.set_value("-18.5", window, cx);
                cx.emit(gpui_kit::component::input::InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
    });
    poll(&view, cx);
    let order = view.update(cx, |shell, cx| {
        let gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .get_edge(edge)
            .unwrap()
            .params()
            .gain;
        assert!((20. * gain.log10() + 18.5).abs() < 0.00001);
        assert!(shell.mixer.presentation.editing_send.is_none());
        shell.mixer.presentation.order.clone()
    });
    // One Sends head folds the list on every strip (the collapse is global,
    // which is what keeps the meters aligned).
    let toggle = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("sends-head-{:?}", order[0])))
            .bounds()
            .center()
    });
    cx.simulate_click(toggle, Modifiers::default());
    cx.update(|window, cx| {
        window.render_frame(cx);
        for id in &order {
            assert_eq!(
                window
                    .find(SharedString::from(format!("io-{id:?}")))
                    .bounds()
                    .size
                    .height,
                px(super::routing::IO_COLLAPSED_H)
            );
        }
    });
    view.update(cx, |shell, cx| {
        shell.mixer.presentation.editing_send = Some(edge);
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::Disconnect(edge),
            ));
        });
    });
    poll(&view, cx);
    view.update(cx, |shell, _| {
        assert!(shell.mixer.presentation.editing_send.is_none());
        assert!(!shell.mixer.presentation.send_inputs.contains_key(&edge));
        assert!(!shell.mixer.presentation.sends.contains_key(&edge));
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn send_numeric_edit_commits_on_blur_and_rejects_invalid(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    activate_window(cx);
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let edge = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.graph.edges()[0].id()
    });
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| {
        view.update(cx, |shell, cx| {
            let gain = shell
                .state
                .read(cx)
                .snapshot
                .graph
                .graph
                .get_edge(edge)
                .unwrap()
                .params()
                .gain;
            (gain, shell.mixer.presentation.editing_send)
        })
    };
    let open = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| {
        let position = cx.update(|window, _| {
            window
                .find(SharedString::from(format!("send-value-{edge:?}")))
                .bounds()
                .center()
        });
        cx.simulate_event(gpui_kit::MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 2,
            first_mouse: false,
        });
        cx.simulate_event(gpui_kit::MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 2,
        });
        view.update(cx, |shell, _| {
            assert_eq!(
                shell.mixer.presentation.editing_send,
                Some(edge),
                "editor must open"
            );
        });
    };
    let editor = view.update(cx, |shell, _| {
        shell.mixer.presentation.send_inputs[&edge].clone()
    });

    // Focus loss runs the same commit path as Enter.
    open(&view, cx);
    type_and_blur(&view, cx, &editor, "-18.5");
    let (value, editing) = gain(&view, cx);
    assert!(
        (20. * value.log10() + 18.5).abs() < 1e-9,
        "blur must commit -18.5 dB"
    );
    assert_eq!(editing, None, "blur closes the send editor");

    // An invalid entry changes nothing on blur, and still closes.
    open(&view, cx);
    type_and_blur(&view, cx, &editor, "abc");
    let (value, editing) = gain(&view, cx);
    assert!(
        (20. * value.log10() + 18.5).abs() < 1e-9,
        "an invalid blur changes nothing"
    );
    assert_eq!(editing, None);
    finish_service(runtime);
}

#[gpui_kit::test]
fn send_editor_accepts_the_shared_silence_spellings(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    activate_window(cx);
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let edge = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.graph.edges()[0].id()
    });
    let editor = view.update(cx, |shell, _| {
        shell.mixer.presentation.send_inputs[&edge].clone()
    });
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| {
        view.update(cx, |shell, cx| {
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .graph
                .get_edge(edge)
                .unwrap()
                .params()
                .gain
        })
    };
    let open = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| {
        let position = cx.update(|window, _| {
            window
                .find(SharedString::from(format!("send-value-{edge:?}")))
                .bounds()
                .center()
        });
        cx.simulate_event(gpui_kit::MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 2,
            first_mouse: false,
        });
        cx.simulate_event(gpui_kit::MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 2,
        });
        view.update(cx, |shell, _| {
            assert_eq!(
                shell.mixer.presentation.editing_send,
                Some(edge),
                "editor must open"
            );
        });
    };

    // The readout's U+2212 −∞ spelling must round-trip: the editor shows
    // "-inf", but the styled strip label renders the Unicode minus.
    open(&view, cx);
    type_and_blur(&view, cx, &editor, "−inf");
    assert_eq!(gain(&view, cx), 0., "−inf (U+2212) must parse as silence");
    // The case-insensitive long form the fader editor already accepts.
    open(&view, cx);
    type_and_blur(&view, cx, &editor, "-INFINITY");
    assert_eq!(gain(&view, cx), 0., "-INFINITY must parse as silence");
    finish_service(runtime);
}

/// Clicks one editable readout and asserts its editor opened.
fn open_editor(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    element: SharedString,
    editing: impl Fn(&StudioShell) -> Option<NodeId>,
) -> NodeId {
    cx.update(|window, cx| window.click(element, cx));
    view.update(cx, |shell, _| editing(shell).expect("editor must open"))
}

/// Types into an open editor and presses Enter; Enter runs the family's
/// subscription synchronously, then the shell re-renders past the dispatch.
fn type_and_commit(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    input: &Entity<gpui_kit::component::input::InputState>,
    text: &str,
) {
    cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            cx.emit(gpui_kit::component::input::InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
        });
    });
    poll(view, cx);
}

/// Makes the window active so real focus changes dispatch `Focus`/`Blur`
/// events: headless windows start inactive, and gpui blanks the focus paths
/// of the focus event for inactive windows, so `window.blur()` alone would
/// reach no listener.
fn activate_window(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
}

/// Types into an open editor and blurs it for real: the frame render turns
/// the focus loss into the input's `Blur` event, which runs the family's
/// commit subscription exactly like Enter does. One frame is drawn while the
/// editor holds focus first — gpui dispatches focus events on focus-path
/// changes between rendered frames, so the editor must appear focused in a
/// rendered frame before the loss can be observed.
fn type_and_blur(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    input: &Entity<gpui_kit::component::input::InputState>,
    text: &str,
) {
    cx.update(|window, cx| {
        input.update(cx, |input, cx| input.set_value(text, window, cx));
        window.render_frame(cx);
        window.blur(cx);
        window.render_frame(cx);
    });
    poll(view, cx);
}

fn readout_of(id: NodeId) -> SharedString {
    SharedString::from(format!("strip-readout-{id:?}"))
}

#[gpui_kit::test]
fn fader_readout_click_to_type_commits_valid_and_rejects_invalid(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let input = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| {
        view.update(cx, |shell, _| {
            shell.mixer.presentation.value_inputs[&id].clone()
        })
    };
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> (f64, Option<NodeId>) {
        view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            (
                graph.node(id).expect("strip node").mixer.gain,
                shell.mixer.presentation.editing_value,
            )
        })
    };
    let editor = input(&view, cx);

    let editing = open_editor(&view, cx, readout_of(id), |shell| {
        shell.mixer.presentation.editing_value
    });
    assert_eq!(editing, id);
    type_and_commit(&view, cx, &editor, "-6");
    let (value, editing) = gain(&view, cx);
    assert!(
        (value - 10f64.powf(-6. / 20.)).abs() < 1e-9,
        "typed -6 dB must land, got {value}"
    );
    assert_eq!(editing, None, "a valid entry closes the editor");

    let (before, _) = gain(&view, cx);
    for rejected in ["abc", "99"] {
        open_editor(&view, cx, readout_of(id), |shell| {
            shell.mixer.presentation.editing_value
        });
        type_and_commit(&view, cx, &editor, rejected);
        let (value, editing) = gain(&view, cx);
        assert_eq!(value, before, "\"{rejected}\" must change nothing");
        assert_eq!(editing, None, "an invalid entry still closes the editor");
    }

    open_editor(&view, cx, readout_of(id), |shell| {
        shell.mixer.presentation.editing_value
    });
    type_and_commit(&view, cx, &editor, "-inf");
    let (value, _) = gain(&view, cx);
    assert_eq!(value, 0., "\"-inf\" is exact digital silence");
    open_editor(&view, cx, readout_of(id), |shell| {
        shell.mixer.presentation.editing_value
    });
    type_and_commit(&view, cx, &editor, "−∞");
    let (value, editing) = gain(&view, cx);
    assert_eq!(value, 0., "the Unicode ∞ spelling is silence too");
    assert_eq!(editing, None);
    finish_service(runtime);
}

#[gpui_kit::test]
fn fader_readout_blur_commits_valid_and_rejects_invalid(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    activate_window(cx);
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let editor = view.update(cx, |shell, _| {
        shell.mixer.presentation.value_inputs[&id].clone()
    });
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> (f64, Option<NodeId>) {
        view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            (
                graph.node(id).expect("strip node").mixer.gain,
                shell.mixer.presentation.editing_value,
            )
        })
    };

    // Focus loss runs the same commit path as Enter.
    let editing = open_editor(&view, cx, readout_of(id), |shell| {
        shell.mixer.presentation.editing_value
    });
    assert_eq!(editing, id);
    type_and_blur(&view, cx, &editor, "-6");
    let (value, editing) = gain(&view, cx);
    assert!(
        (value - 10f64.powf(-6. / 20.)).abs() < 1e-9,
        "blur must commit -6 dB, got {value}"
    );
    assert_eq!(editing, None, "blur closes the editor");

    // An invalid entry changes nothing on blur, and still closes.
    let (before, _) = gain(&view, cx);
    for rejected in ["abc", "99"] {
        open_editor(&view, cx, readout_of(id), |shell| {
            shell.mixer.presentation.editing_value
        });
        type_and_blur(&view, cx, &editor, rejected);
        let (value, editing) = gain(&view, cx);
        assert_eq!(
            value, before,
            "blur with \"{rejected}\" must change nothing"
        );
        assert_eq!(editing, None, "an invalid blur still closes the editor");
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn fader_readout_enter_then_stale_blur_commits_once(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    activate_window(cx);
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let editor = view.update(cx, |shell, _| {
        shell.mixer.presentation.value_inputs[&id].clone()
    });
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> (f64, Option<NodeId>) {
        view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            (
                graph.node(id).expect("strip node").mixer.gain,
                shell.mixer.presentation.editing_value,
            )
        })
    };

    open_editor(&view, cx, readout_of(id), |shell| {
        shell.mixer.presentation.editing_value
    });
    type_and_commit(&view, cx, &editor, "-6");
    let (value, editing) = gain(&view, cx);
    assert!((value - 10f64.powf(-6. / 20.)).abs() < 1e-9);
    assert_eq!(editing, None);

    // Enter closed the editor; a late blur carrying new text must find no
    // active editor and dispatch nothing.
    cx.update(|window, cx| {
        editor.update(cx, |input, cx| {
            input.set_value("+6", window, cx);
            cx.emit(gpui_kit::component::input::InputEvent::Blur);
        });
    });
    poll(&view, cx);
    let (value, editing) = gain(&view, cx);
    assert!(
        (value - 10f64.powf(-6. / 20.)).abs() < 1e-9,
        "a stale blur after Enter must not re-dispatch, got {value}"
    );
    assert_eq!(editing, None);
    finish_service(runtime);
}

/// Clicking anywhere outside the open editor commits it like Enter. The
/// engine statusbar carries no focusable surface, so the input never loses
/// focus and this exercises the window root's outside-click path, not the
/// blur path.
#[gpui_kit::test]
fn fader_readout_commits_when_clicking_outside_the_editor(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    activate_window(cx);
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let editor = view.update(cx, |shell, _| {
        shell.mixer.presentation.value_inputs[&id].clone()
    });
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> (f64, Option<NodeId>) {
        view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            (
                graph.node(id).expect("strip node").mixer.gain,
                shell.mixer.presentation.editing_value,
            )
        })
    };
    let outside = cx.update(|window, _| window.find("engine-statusbar").bounds().center());
    let click = |view: &Entity<StudioShell>,
                 cx: &mut VisualTestContext,
                 position: gpui_kit::Point<gpui_kit::Pixels>| {
        cx.simulate_event(gpui_kit::MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(gpui_kit::MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        poll(view, cx);
    };

    open_editor(&view, cx, readout_of(id), |shell| {
        shell.mixer.presentation.editing_value
    });
    cx.update(|window, cx| {
        editor.update(cx, |input, cx| input.set_value("-6", window, cx));
        window.render_frame(cx);
    });
    click(&view, cx, outside);
    let (value, editing) = gain(&view, cx);
    assert!(
        (value - 10f64.powf(-6. / 20.)).abs() < 1e-9,
        "an outside click must commit -6 dB, got {value}"
    );
    assert_eq!(editing, None, "an outside click closes the editor");

    // Invalid text: the outside click still closes, and changes nothing.
    let (before, _) = gain(&view, cx);
    open_editor(&view, cx, readout_of(id), |shell| {
        shell.mixer.presentation.editing_value
    });
    cx.update(|window, cx| {
        editor.update(cx, |input, cx| input.set_value("abc", window, cx));
        window.render_frame(cx);
    });
    click(&view, cx, outside);
    let (value, editing) = gain(&view, cx);
    assert_eq!(
        value, before,
        "an outside click with invalid text changes nothing"
    );
    assert_eq!(
        editing, None,
        "an invalid outside click still closes the editor"
    );
    finish_service(runtime);
}

#[gpui_kit::test]
fn fader_readout_editor_occupies_the_label_box(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let label = cx.update(|window, _| window.find(readout_of(id)).bounds());

    open_editor(&view, cx, readout_of(id), |shell| {
        shell.mixer.presentation.editing_value
    });
    let input = view.update(cx, |shell, _| {
        shell.mixer.presentation.value_inputs[&id].clone()
    });
    let editor =
        cx.update(|window, _| window.find(("editable-editor", input.entity_id())).bounds());
    assert!(
        (f32::from(editor.top()) - f32::from(label.top())).abs() <= 1.,
        "the editor text must start where the readout started"
    );
    // One text line in the readout's 12px font is round(phi * 12) = 19px;
    // the old editor forced a 32px input box.
    assert_eq!(
        editor.size.height,
        px(19.),
        "the editor is one text line tall"
    );
    assert!(
        (f32::from(editor.left()) - f32::from(label.left())).abs() <= 1.
            && (f32::from(editor.right()) - f32::from(label.right())).abs() <= 1.,
        "the underline field spans the readout's width"
    );
    finish_service(runtime);
}

#[gpui_kit::test]
fn balance_label_click_to_type_commits_valid_and_rejects_invalid(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    activate_window(cx);
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let label = SharedString::from(format!("balance-label-{id:?}"));
    let editing_flag = |shell: &StudioShell| shell.mixer.presentation.editing_balance;
    let editor = view.update(cx, |shell, _| {
        shell.mixer.presentation.balance_inputs[&id].clone()
    });
    let balance =
        |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> (f64, Option<NodeId>) {
            view.update(cx, |shell, cx| {
                (
                    shell
                        .state
                        .read(cx)
                        .snapshot
                        .graph
                        .node(id)
                        .expect("strip node")
                        .mixer
                        .balance,
                    shell.mixer.presentation.editing_balance,
                )
            })
        };

    // Clicking the label opens the editor; it must not also map the click
    // position onto the balance (those handlers live on the track now).
    let editing = open_editor(&view, cx, label.clone(), editing_flag);
    assert_eq!(editing, id);
    let (value, _) = balance(&view, cx);
    assert_eq!(value, 0., "opening the editor must not move the balance");

    type_and_commit(&view, cx, &editor, "R25");
    let (value, editing) = balance(&view, cx);
    assert!((value - 0.25).abs() < 1e-9, "\"R25\" landed {value}");
    assert_eq!(editing, None);

    open_editor(&view, cx, label.clone(), editing_flag);
    type_and_commit(&view, cx, &editor, "C");
    let (value, _) = balance(&view, cx);
    assert_eq!(value, 0., "\"C\" recenters");

    // Focus loss runs the same commit path as Enter.
    open_editor(&view, cx, label.clone(), editing_flag);
    type_and_blur(&view, cx, &editor, "L 50");
    let (value, editing) = balance(&view, cx);
    assert!(
        (value + 0.5).abs() < 1e-9,
        "blur must commit \"L 50\", got {value}"
    );
    assert_eq!(editing, None, "blur closes the editor");
    open_editor(&view, cx, label.clone(), editing_flag);
    type_and_blur(&view, cx, &editor, "C");
    let (value, _) = balance(&view, cx);
    assert_eq!(value, 0., "a blurred \"C\" recenters");
    open_editor(&view, cx, label.clone(), editing_flag);
    type_and_blur(&view, cx, &editor, "150");
    let (value, editing) = balance(&view, cx);
    assert_eq!(value, 0., "an invalid blur changes nothing");
    assert_eq!(editing, None, "an invalid blur still closes the editor");

    open_editor(&view, cx, label, editing_flag);
    type_and_commit(&view, cx, &editor, "150");
    let (value, editing) = balance(&view, cx);
    assert_eq!(value, 0., "\"150\" is out of range and changes nothing");
    assert_eq!(editing, None);
    finish_service(runtime);
}

#[gpui_kit::test]
fn node_gain_and_pan_readouts_commit_their_own_parameters(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let gain_id = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        graph
            .nodes
            .iter()
            .find(|node| {
                graph
                    .graph
                    .get_node(node.id)
                    .is_ok_and(|logical| logical.kind() == NodeKind::Gain)
            })
            .unwrap()
            .id
    });

    // The Gain node's strip fader edits mixer.gain; its "Requested" field is
    // a separate family editing the node's own DSP gain.
    let input = view.update(cx, |shell, _| {
        shell.mixer.presentation.node_gain_inputs[&gain_id].clone()
    });
    view.update(cx, |shell, _| {
        shell.mixer.presentation.editing_node_gain = Some(gain_id)
    });
    type_and_commit(&view, cx, &input, "+6");
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert!(
            (graph.node(gain_id).unwrap().gain - 10f64.powf(6. / 20.)).abs() < 1e-9,
            "\"+6\" set the node gain, got {}",
            graph.node(gain_id).unwrap().gain
        );
        assert_eq!(graph.node(gain_id).unwrap().mixer.gain, 1.);
        assert!(shell.mixer.presentation.editing_node_gain.is_none());
    });

    view.update(cx, |shell, _| {
        shell.mixer.presentation.editing_node_gain = Some(gain_id)
    });
    type_and_commit(&view, cx, &input, "99");
    view.update(cx, |shell, cx| {
        let gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(gain_id)
            .unwrap()
            .gain;
        assert!(
            (gain - 10f64.powf(6. / 20.)).abs() < 1e-9,
            "\"99\" is out of range and changes nothing"
        );
    });

    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::AddNode {
                    kind: NodeKind::Pan,
                    channels: 2,
                },
            ));
        });
    });
    poll(&view, cx);
    let (pan_id, input) = view.update(cx, |shell, _| {
        let id = shell
            .mixer
            .presentation
            .node_pan_inputs
            .keys()
            .copied()
            .last()
            .expect("the pan node has a position editor");
        (id, shell.mixer.presentation.node_pan_inputs[&id].clone())
    });
    assert!(
        !view.update(cx, |shell, _| shell
            .mixer
            .presentation
            .editing_value
            .is_some()),
        "the pan node's strip fader family stays independent"
    );
    view.update(cx, |shell, _| {
        shell.mixer.presentation.editing_node_pan = Some(pan_id)
    });
    type_and_commit(&view, cx, &input, "R25");
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert!(
            (graph.node(pan_id).unwrap().pan - 0.25).abs() < 1e-9,
            "\"R25\" set the pan position, got {}",
            graph.node(pan_id).unwrap().pan
        );
        assert_eq!(graph.node(pan_id).unwrap().mixer.balance, 0.);
        assert!(shell.mixer.presentation.editing_node_pan.is_none());
    });
    finish_service(runtime);
}

/// == tests appended from feat/overlay-scrollbars ==
/// An edge edit must hit every strip's cache equally: the routing rows
/// moved out of the cached body, so edges render nowhere inside the cache
/// and the pruned key no longer compares the touching edges (the old key
/// rebuilt the two endpoint strips while their siblings replayed). The live
/// sections pick the new rows up on the same notification.
#[gpui_kit::test]
fn edge_edits_hit_every_strip_cache_equally_after_the_routing_split(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let (source, destination) = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph.graph;
        let source = graph
            .nodes()
            .iter()
            .find(|node| !node.outputs().is_empty())
            .unwrap()
            .outputs()[0]
            .id();
        let sink = graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Sink)
            .unwrap()
            .id();
        let destination = graph
            .nodes()
            .iter()
            .find(|node| !node.inputs().is_empty() && node.id() != sink)
            .unwrap()
            .inputs()[0]
            .id();
        (source, destination)
    });
    poll(&view, cx);
    // The inspector's closing spring and the sliders' first value write-backs
    // churn rebuilds for about a second after opening; let them drain so the
    // baseline counts a steady state.
    settle_panels(cx);
    poll(&view, cx);
    let before = view.read_with(cx, |shell, cx| {
        let deltas: Vec<usize> = shell
            .mixer
            .presentation
            .strips
            .values()
            .map(|host| host.read(cx).rebuild_count())
            .collect();
        (deltas, shell.trace.borrow().strip_renders)
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::Connect {
                    source,
                    destination,
                },
            ));
        });
        cx.notify();
    });
    poll(&view, cx);
    // The new send slider's first value write-back rides the same churn;
    // let it drain before counting.
    settle_panels(cx);
    poll(&view, cx);
    cx.update(|window, cx| window.render_frame(cx));
    let after = view.read_with(cx, |shell, cx| {
        let deltas: Vec<usize> = shell
            .mixer
            .presentation
            .strips
            .values()
            .map(|host| host.read(cx).rebuild_count())
            .collect();
        (deltas, shell.trace.borrow().strip_renders)
    });
    let per_strip: Vec<usize> = after
        .0
        .iter()
        .zip(before.0.iter())
        .map(|(a, b)| a - b)
        .collect();
    assert!(
        per_strip.iter().all(|d| *d == per_strip[0]),
        "an edge edit must hit every strip's cache equally, got {per_strip:?}"
    );
    assert!(
        after.1 >= before.1,
        "the connect publish must have gone through"
    );
    // The live routing section shows the new row on the same frame.
    assert!(
        view.read_with(cx, |shell, cx| shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .edges()
            .iter()
            .any(|edge| {
                edge.src_port() == source && edge.dst_port() == destination
            })),
        "the connect must have published"
    );
    finish_service(runtime);
}

/// Docked strips scroll with the dock pane: their cached paint keys on the
/// dock's own offset, so a dock scroll moves them instead of freezing the
/// old coordinates (the dock container never tracked a handle before).
#[gpui_kit::test]
fn docked_strips_follow_the_dock_scroll(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        // Narrow the pane below one strip so the dock overflows.
        shell.mixer.presentation.dock_width = 100.;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    // The inspector's closing spring slides the whole row while it settles;
    // let it finish or the strip's absolute x drifts between measurements.
    settle_panels(cx);
    let sink = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Sink)
            .unwrap()
            .id()
    });
    let before = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("strip-{sink:?}")))
            .bounds()
    });
    view.update(cx, |shell, cx| {
        shell.mixer.dock_scroll.set_offset(point(px(-60.), px(0.)));
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let after = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("strip-{sink:?}")))
            .bounds()
    });
    assert_eq!(
        after.origin.x,
        before.origin.x - px(60.),
        "the docked strip must follow the dock pane's scroll"
    );
    finish_service(runtime);
}

/// Lands a node-subject fault on a strip through a fresh publish: the strip
/// key projects its fault group, so exactly that strip rebuilds while its
/// siblings replay; the badge appears and clicking it opens the fault
/// popover, which a click outside dismisses; clearing the fault rebuilds
/// the strip back to rest.
#[gpui_kit::test]
fn mixer_fault_rebuilds_only_its_strip_and_the_badge_opens_the_popover(cx: &mut TestAppContext) {
    use crate::service::{GraphFault, GraphSubject};
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::shell::test_harness::settle_panels(cx);
    let target = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let badge_id = SharedString::from(format!("strip-fault-{target:?}"));

    // A publish that lands the fault, delivered the way the service does.
    let before = view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            let mut snapshot = (*state.snapshot).clone();
            let graph = std::sync::Arc::make_mut(&mut snapshot.graph);
            graph.faults.push(GraphFault {
                subject: GraphSubject::Node(target),
                cause: "The input source configuration is invalid".into(),
                hint: Some("Choose an input source at the top of the channel".into()),
            });
            state.snapshot = std::sync::Arc::new(snapshot);
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let after = view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders);
    assert_eq!(
        after - before,
        1,
        "only the faulted strip may rebuild; siblings must replay"
    );

    // The badge opens the popover on this strip's fault group.
    cx.update(|window, cx| {
        window.click(badge_id.clone(), cx);
        window.draw(cx).clear(cx);
    });
    assert!(
        view.read_with(cx, |shell, _| shell
            .chrome
            .fault_popover
            .is_some_and(|popover| popover.node == Some(target))),
        "the badge click must open the fault popover on its strip"
    );
    cx.update(|window, _| {
        assert!(
            window.try_find("fault-popover").is_some(),
            "popover renders"
        );
    });

    // A press outside the popover dismisses it.
    cx.update(|window, cx| {
        window.click("statusbar-format", cx);
        window.draw(cx).clear(cx);
    });
    assert!(view.read_with(cx, |shell, _| shell.chrome.fault_popover.is_none()));
    cx.update(|window, _| {
        assert!(window.try_find("fault-popover").is_none());
    });

    // Clearing the fault rebuilds the strip back to rest; the badge goes.
    // Re-baseline first: the clicks above refresh the whole window, which
    // rebuilds every panel regardless of the caches and would pollute the
    // delta.
    let before_clear = view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            let mut snapshot = (*state.snapshot).clone();
            std::sync::Arc::make_mut(&mut snapshot.graph).faults.clear();
            state.snapshot = std::sync::Arc::new(snapshot);
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let cleared = view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders);
    assert_eq!(
        cleared - before_clear,
        1,
        "clearing the fault rebuilds exactly the faulted strip"
    );
    cx.update(|window, _| {
        assert!(window.try_find(badge_id).is_none(), "the badge is gone");
    });
    finish_service(runtime);
}

/// Hover feeds the meter frame's fault banner through the shell's
/// `hovered_strip`, never a panel key: hovering a strip tracks it, leaving
/// The strip's cached body must fill the strip instead of hugging its
/// content: the levels row takes every pixel the routing sections leave
/// over and the title stays pinned to the strip's bottom edge. Regression
/// guard for the cached body's root sizing — a cached view lays its content
/// out as a layout root where `flex_1` has no parent to grow against, so
/// the body's root must claim definite extents (see `strip_body`).
#[gpui_kit::test]
fn mixer_strip_body_fills_the_strip_and_pins_the_title_to_its_bottom(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::shell::test_harness::settle_panels(cx);
    let id = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    cx.update(|window, _| {
        let strip = window
            .find(SharedString::from(format!("strip-{id:?}")))
            .bounds();
        let levels = window
            .find(SharedString::from(format!("strip-levels-{id:?}")))
            .bounds();
        let header = window
            .find(SharedString::from(format!("strip-header-{id:?}")))
            .bounds();
        let track = window
            .find(SharedString::from(format!("balance-track-{id:?}")))
            .bounds();
        let inputs = window
            .find(SharedString::from(format!("io-{id:?}")))
            .bounds();
        // The title rides the strip's bottom border; the levels row owns
        // everything between it and the fixed control stack.
        assert!(
            (f32::from(header.bottom() - strip.bottom())).abs() <= 1.,
            "title must stay pinned to the strip bottom (header {header:?}, strip {strip:?})",
        );
        assert!(
            f32::from(levels.size.height) > f32::from(strip.size.height) * 0.4,
            "levels row must adapt to the leftover height (levels {levels:?}, strip {strip:?})",
        );
        assert!((f32::from(levels.bottom() + px(32.) - header.top())).abs() <= 1.);
        // The body fills the width too: the centered balance track spans the
        // strip's inner width instead of hugging its content.
        assert!(
            (f32::from(track.size.width - (strip.size.width - px(18.)))).abs() <= 2.,
            "balance track must span the strip's inner width (track {track:?}, strip {strip:?})",
        );
        assert!(
            inputs.bottom() <= track.top(),
            "the I/O block ends where the balance track begins"
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn mixer_strip_hover_tracks_the_strip_for_the_fault_banner(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::shell::test_harness::settle_panels(cx);
    let target = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    cx.update(|window, cx| {
        window.hover(SharedString::from(format!("strip-{target:?}")), cx);
    });
    assert_eq!(
        view.read_with(cx, |shell, _| shell.mixer.hovered_strip),
        Some(target),
        "hovering the strip must track it for the banner"
    );
    // The hover-driven notify rides the cached path: on the next cached
    // frames no strip panel may rebuild — hover lives outside every key.
    // (window.hover itself refreshes the whole window, so the baseline
    // sits after it, measuring the cached frames alone.)
    cx.update(|window, cx| {
        window.hover("engine-statusbar", cx);
    });
    assert_eq!(
        view.read_with(cx, |shell, _| shell.mixer.hovered_strip),
        None
    );
    let before = view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders);
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let after = view.read_with(cx, |shell, _| shell.trace.borrow().strip_renders);
    assert_eq!(
        after - before,
        0,
        "a hover change must never rebuild a strip panel"
    );
    finish_service(runtime);
}

// --- The reworked I/O block --------------------------------------------------

fn show_mixer(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    view.update(cx, |shell, cx| {
        shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
        shell.chrome.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
}

fn add_node(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    kind: NodeKind,
    channels: usize,
) -> NodeId {
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::AddNode { kind, channels },
            ));
        });
    });
    poll(view, cx);
    view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .nodes
            .last()
            .expect("node appended")
            .id
    })
}

/// The I/O block's whole point: every strip carries the same height, and the
/// Sends head folds the list on every strip at once, so the meters below
/// stay aligned however the routing differs.
#[gpui_kit::test]
fn io_block_heights_are_uniform_and_fold_globally(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    show_mixer(&view, cx);
    let order = view.update(cx, |shell, _| shell.mixer.presentation.order.clone());
    assert!(order.len() > 1, "the default session starts with strips");
    let io_heights = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        window.render_frame(cx);
        order
            .iter()
            .map(|id| {
                window
                    .find(SharedString::from(format!("io-{id:?}")))
                    .bounds()
                    .size
                    .height
            })
            .collect::<Vec<_>>()
    };
    cx.update(|window, cx| {
        let heights = io_heights(window, cx);
        assert!(
            heights
                .iter()
                .all(|h| *h == px(super::routing::IO_EXPANDED_H)),
            "every strip expanded: {heights:?}"
        );
    });
    let head = view.update(cx, |shell, _| {
        SharedString::from(format!(
            "sends-head-{:?}",
            shell.mixer.presentation.order[0]
        ))
    });
    cx.update(|window, cx| window.click(head.clone(), cx));
    cx.update(|window, cx| {
        let heights = io_heights(window, cx);
        assert!(
            heights
                .iter()
                .all(|h| *h == px(super::routing::IO_COLLAPSED_H)),
            "one head folds every strip: {heights:?}"
        );
    });
    cx.update(|window, cx| window.click(head.clone(), cx));
    cx.update(|window, cx| {
        let heights = io_heights(window, cx);
        assert!(
            heights
                .iter()
                .all(|h| *h == px(super::routing::IO_EXPANDED_H)),
            "one head unfolds every strip: {heights:?}"
        );
    });
    finish_service(runtime);
}

/// The dropdown is a live multi-select: checking a candidate connects it to
/// the node's lowest free input port at once and the list stays open for the
/// next pick; unchecking disconnects. The single-input row swaps its ghost
/// button to the − remove control while occupied.
#[gpui_kit::test]
fn input_dropdown_connects_at_the_free_port_and_the_row_swaps_to_remove(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    show_mixer(&view, cx);
    let gain = add_node(&view, cx, NodeKind::Gain, 2);
    let source = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Source)
            .map(|node| node.id())
            .expect("default session has a source")
    });
    // Empty and able: the row carries the persistent +, never the −.
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(SharedString::from(format!("input-add-{gain:?}")))
            .bounds();
        assert!(
            window
                .try_find(SharedString::from(format!("input-remove-{gain:?}")))
                .is_none(),
            "an empty node must not offer a remove control"
        );
    });
    // Open the dropdown and check the source: the edge lands on the Gain's
    // only input port, and the list stays open.
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("input-add-{gain:?}")), cx);
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(SharedString::from(format!("input-popover-{gain:?}")))
            .bounds();
    });
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("input-pop-row-{source:?}")), cx);
    });
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = shell.state.read(cx).snapshot.graph.clone();
        let edge = graph
            .graph
            .edges()
            .iter()
            .find(|edge| edge.dst() == gain)
            .expect("checking the source connects it");
        assert_eq!(edge.src(), source);
        let free = graph.graph.get_node(gain).unwrap().inputs()[0].id();
        assert_eq!(edge.dst_port(), free, "the edge takes the free input port");
    });
    view.update(cx, |shell, _| {
        assert!(
            shell.mixer.presentation.input_popover.is_some(),
            "the list stays open for the next pick"
        );
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(SharedString::from(format!("input-popover-{gain:?}")))
            .bounds();
        window
            .find(SharedString::from(format!("input-remove-{gain:?}")))
            .bounds();
        assert!(
            window
                .try_find(SharedString::from(format!("input-add-{gain:?}")))
                .is_none(),
            "an occupied single-input node swaps + for −"
        );
    });
    // Unchecking disconnects through the same row.
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("input-pop-row-{source:?}")), cx);
    });
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = shell.state.read(cx).snapshot.graph.clone();
        assert!(
            graph.graph.edges().iter().all(|edge| edge.dst() != gain),
            "unchecking disconnects the candidate"
        );
    });
    finish_service(runtime);
}

/// A press outside the open dropdown dismisses it, like every shell popover.
#[gpui_kit::test]
fn input_dropdown_closes_on_an_outside_press(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    show_mixer(&view, cx);
    let gain = add_node(&view, cx, NodeKind::Gain, 2);
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("input-add-{gain:?}")), cx);
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(SharedString::from(format!("input-popover-{gain:?}")))
            .bounds();
    });
    // A press over the first strip's fader area — anywhere off the dropdown
    // box — dismisses it.
    let first = view.update(cx, |shell, _| shell.mixer.presentation.order[0]);
    let outside = cx.update(|window, _| {
        // Press on the first strip's title row: anywhere off the dropdown
        // box dismisses it.
        let strip = window
            .find(SharedString::from(format!("strip-{first:?}")))
            .bounds();
        point(strip.left() + px(80.), strip.bottom() - px(6.))
    });
    cx.simulate_click(outside, Modifiers::default());
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find(SharedString::from(format!("input-popover-{gain:?}")))
                .is_none(),
            "an outside press closes the dropdown"
        );
    });
    view.update(cx, |shell, _| {
        assert!(shell.mixer.presentation.input_popover.is_none());
    });
    finish_service(runtime);
}

/// At the node's input cap the unchecked candidates are dead rows: clicking
/// them connects nothing, while the checked row still releases its input.
#[gpui_kit::test]
fn input_dropdown_rows_go_dead_at_the_port_cap(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    show_mixer(&view, cx);
    // A bus starts without input ports; one AddBusInput gives it exactly
    // one, so a single connection saturates the cap.
    let bus = add_node(&view, cx, NodeKind::Bus, 2);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::AddBusInput(bus),
            ));
        });
    });
    poll(&view, cx);
    let second_source = add_node(&view, cx, NodeKind::Source, 2);
    let (sources, port) = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        (
            graph
                .graph
                .nodes()
                .iter()
                .filter(|node| node.kind() == NodeKind::Source)
                .map(|node| node.id())
                .collect::<Vec<_>>(),
            graph.graph.get_node(bus).unwrap().inputs()[0].id(),
        )
    });
    assert!(sources.len() >= 2, "two candidates with outputs are needed");
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("input-add-{bus:?}")), cx);
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(SharedString::from(format!("input-popover-{bus:?}")))
            .bounds();
    });
    let first = sources[0];
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("input-pop-row-{first:?}")), cx);
    });
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = shell.state.read(cx).snapshot.graph.clone();
        let edge = graph
            .graph
            .edges()
            .iter()
            .find(|edge| edge.dst() == bus)
            .expect("the first check connects");
        assert_eq!(edge.dst_port(), port);
    });
    // The cap is reached: the second source's unchecked row is dead.
    cx.update(|window, cx| {
        window.click(
            SharedString::from(format!("input-pop-row-{second_source:?}")),
            cx,
        );
    });
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = shell.state.read(cx).snapshot.graph.clone();
        assert_eq!(
            graph
                .graph
                .edges()
                .iter()
                .filter(|edge| edge.dst() == bus)
                .count(),
            1,
            "a dead row must not connect"
        );
    });
    // The checked row keeps working: unchecking releases the port.
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("input-pop-row-{first:?}")), cx);
    });
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = shell.state.read(cx).snapshot.graph.clone();
        assert!(
            graph.graph.edges().iter().all(|edge| edge.dst() != bus),
            "the checked row still releases its input"
        );
    });
    finish_service(runtime);
}

/// A routing row's click selects the edge; its double click jumps to the
/// Graph page with the same wire selected.
#[gpui_kit::test]
fn routing_rows_select_and_double_click_jumps_to_the_graph(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    show_mixer(&view, cx);
    let (edge, ports) = view.update(cx, |shell, cx| {
        let edge = &shell.state.read(cx).snapshot.graph.graph.edges()[0];
        (edge.id(), (edge.src_port(), edge.dst_port()))
    });
    // Single click selects on the input row.
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("input-{edge:?}")), cx);
    });
    view.update(cx, |shell, _| {
        assert_eq!(shell.selection.route, Some(ports));
        assert!(shell.chrome.inspector_open);
        assert!(matches!(
            shell.presentation.workspace,
            WorkspaceView::Page(Page::Mixer)
        ));
    });
    // Double click jumps on both rows.
    cx.update(|window, cx| {
        window.double_click(SharedString::from(format!("input-{edge:?}")), cx);
    });
    view.update(cx, |shell, _| {
        assert!(matches!(
            shell.presentation.workspace,
            WorkspaceView::Page(Page::Graph)
        ));
        assert_eq!(shell.selection.route, Some(ports));
    });
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            shell.presentation.workspace = WorkspaceView::Page(Page::Mixer);
            cx.notify();
        });
        window.render_frame(cx);
    });
    cx.update(|window, cx| {
        window.double_click(SharedString::from(format!("send-row-{edge:?}")), cx);
    });
    view.update(cx, |shell, _| {
        assert!(matches!(
            shell.presentation.workspace,
            WorkspaceView::Page(Page::Graph)
        ));
    });
    finish_service(runtime);
}

/// The send bar's whole value chain is one number: the pointer fraction the
/// kit reports becomes the slider value, the fill paints that same fraction,
/// and the committed gain is that fraction's inverse through the shared
/// scale — so the bar lands exactly under the mouse in both directions.
/// (Regression: the fill once re-derived its width through a dB detour that
/// was not in the drag's value space, so the painted bar barely moved.)
#[gpui_kit::test]
fn send_bar_drag_paints_and_commits_the_pointer_fraction(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    show_mixer(&view, cx);
    let edge = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.graph.edges()[0].id()
    });
    let (origin, loud, quiet) = cx.update(|window, _| {
        let bounds = window
            .find(SharedString::from(format!("send-track-{edge:?}")))
            .bounds();
        let at = |fraction: f32| {
            point(
                bounds.left() + bounds.size.width * fraction,
                bounds.top() + px(2.),
            )
        };
        (at(0.), at(0.75), at(0.10))
    });
    cx.simulate_mouse_down(origin, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(loud, Some(MouseButton::Left), Modifiers::default());
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_move(quiet, Some(MouseButton::Left), Modifiers::default());
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_up(quiet, MouseButton::Left, Modifiers::default());
    poll(&view, cx);
    // The slider value — the number the fill paints — sits at the pointer's
    // final fraction, and the graph carries the gain of that same fraction.
    view.update(cx, |shell, cx| {
        let value = shell.mixer.presentation.sends[&edge]
            .state
            .read(cx)
            .value()
            .end();
        assert!(
            (value - 0.10).abs() < 0.01,
            "the fill fraction follows the pointer: {value}"
        );
        let gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .get_edge(edge)
            .unwrap()
            .params()
            .gain;
        let expected = send_gain(0.10);
        assert!(
            (gain - expected).abs() < 0.01,
            "the committed gain is the pointer fraction's gain: {gain} vs {expected}"
        );
    });
    // The painted bar itself: the fill's width is the same fraction of the
    // track, measured in pixels — the regression this pins is the paint, not
    // just the value underneath it.
    let fill = view.update(cx, |shell, _| {
        SharedString::from(format!(
            "send-fill-{}",
            shell.mixer.presentation.sends[&edge].state.entity_id()
        ))
    });
    cx.update(|window, _| {
        let track = window
            .find(SharedString::from(format!("send-track-{edge:?}")))
            .bounds();
        let fill = window.find(fill).bounds();
        let fraction = f32::from(fill.size.width / track.size.width);
        assert!(
            (fraction - 0.10).abs() < 0.01,
            "the painted fill sits at the pointer fraction: {fraction}"
        );
    });
    finish_service(runtime);
}
