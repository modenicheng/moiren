use super::{Duration, NodeId, NodeKind, Page, StudioShell, reorder};
use crate::ui::widgets::fader::FaderScale;
use crate::ui::window::test_harness::{finish_service, shell_with_state, test_shell, wait_until};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, SharedString,
    TestAppContext, VisualTestContext, point, px, size,
};

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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
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
        reorder(&mut shell.mixer.order, gain, source);
        let document = shell.project_document(cx).unwrap();
        shell.mixer.order.clear();
        shell.apply_project_presentation(&document, cx).unwrap();
        assert_eq!(shell.mixer.order[0], gain);
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let order = view.update(cx, |shell, _| shell.mixer.order.clone());
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
    view.update(cx, |shell, _| assert_eq!(shell.mixer.order[0], order[1]));
    finish_service(runtime);
}

#[gpui_kit::test]
fn compact_mixer_keeps_send_controls_and_fixed_width_meters_unclipped(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1024.), px(720.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    cx.update(|window, cx| {
        let sends = window
            .find(SharedString::from(format!("sends-{id:?}")))
            .bounds();
        assert_eq!(sends.size.height, px(80.));
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
        let fader = view.read(cx).mixer.faders[&id].state.read(cx).bounds();
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
        let inputs = window
            .find(SharedString::from(format!("inputs-{id:?}")))
            .bounds();
        assert!(inputs.bottom() <= sends.top());
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
                .find(SharedString::from(format!("send-tap-{edge:?}")))
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
        let first = shell.mixer.order[0];
        reorder(&mut shell.mixer.order, id, first);
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
    let snapshot = crate::ui::window::test_harness::wait_for(Duration::from_secs(3), || {
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
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
        let trace = shell.mixer.trace.borrow();
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
        let trace = shell.mixer.trace.borrow();
        assert!(
            shell.node_meter_points.borrow().contains(&node),
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
        view.read_with(cx, |shell, _| shell.mixer.trace.borrow().strip_renders),
        after.0,
        "control polling must not invalidate unchanged strips"
    );

    // A real control edit invalidates the cache without a forced refresh.
    view.update(cx, |shell, cx| {
        shell.selected_node = Some(node);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(view.read_with(cx, |shell, _| shell.mixer.trace.borrow().strip_renders) > after.0);

    let controls_before = view.read_with(cx, |shell, _| shell.mixer.trace.borrow().strip_renders);
    let fader = view.read_with(cx, |shell, _| shell.mixer.faders[&node].state.clone());
    cx.update(|window, cx| fader.update(cx, |slider, cx| slider.set_value(0.25, window, cx)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        view.read_with(cx, |shell, _| shell.mixer.trace.borrow().strip_renders) > controls_before,
        "slider state notifications must invalidate cached thumb geometry"
    );

    view.update(cx, |shell, cx| {
        shell.settings_open = true;
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
    let snapshot = crate::ui::window::test_harness::wait_for(Duration::from_secs(3), || {
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
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
        view.read_with(cx, |shell, _| shell.meter_points.get()),
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
        view.read_with(cx, |shell, _| shell.meter_points.get()),
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    // The inspector's closing spring notifies its way through the first
    // second; let it finish or every frame rebuilds every panel.
    crate::ui::window::test_harness::settle_panels(cx);
    let target = view.update(cx, |shell, _| shell.mixer.order[0]);
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
    let before = view.read_with(cx, |shell, _| shell.mixer.trace.borrow().strip_renders);
    cx.update(|window, cx| {
        assert!(!window.is_a11y_active(), "measuring the cached path");
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let mid = view.read_with(cx, |shell, _| shell.mixer.trace.borrow().strip_renders);
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let after = view.read_with(cx, |shell, _| shell.mixer.trace.borrow().strip_renders);
    eprintln!("DELTA first={} second={}", mid - before, after - mid);
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::window::test_harness::settle_panels(cx);
    let node = view.update(cx, |shell, _| shell.mixer.order[0]);
    // Start from unity (0 dB): thumb at FaderScale position 0.24.
    let start_value = 1. - FaderScale::position(1.);
    let slider = view.read_with(cx, |shell, _| shell.mixer.faders[&node].state.clone());
    cx.update(|window, cx| {
        slider.update(cx, |slider, cx| slider.set_value(start_value, window, cx))
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (thumb_x, thumb_y) = view.update(cx, |shell, cx| {
        let bounds = shell.mixer.faders[&node].state.read(cx).bounds();
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
        shell.mixer.faders[&node].state.read(cx).value().end()
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
    // The dispatch rides the mailbox: until a poll consumes the published
    // confirmation, the UI snapshot still holds the PRE-DRAG gain, and every
    // rendered frame runs the snapshot write-back. The thumb must hold the
    // user's value through that window instead of snapping back to the rail.
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let held = view.read_with(cx, |shell, cx| {
            shell.mixer.faders[&node].state.read(cx).value().end()
        });
        assert!(
            held > 0.5,
            "the thumb snapped back to the pre-drag snapshot value {held} before confirmation"
        );
    }
    // A stale publication (the driver's strip sync races the edit path on a
    // real machine): a revision the UI has not seen, carrying the PRE-DRAG
    // gain. The fresh user commit must shield the thumb from it.
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
        shell.mixer.faders[&node].state.read(cx).value().end()
    });
    assert!(
        shielded > 0.5,
        "a stale publication yanked the thumb back to {shielded}"
    );
    poll(&view, cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (final_value, final_db) = view.update(cx, |shell, cx| {
        let value = shell.mixer.faders[&node].state.read(cx).value().end();
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::window::test_harness::settle_panels(cx);
    let node = view.update(cx, |shell, _| shell.mixer.order[0]);
    // Select the strip the way the header click does.
    view.update(cx, |shell, cx| {
        shell.selected_node = Some(node);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let start_value = 1. - FaderScale::position(1.);
    let slider = view.read_with(cx, |shell, _| shell.mixer.faders[&node].state.clone());
    cx.update(|window, cx| {
        slider.update(cx, |slider, cx| slider.set_value(start_value, window, cx))
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (thumb_x, thumb_y) = view.update(cx, |shell, cx| {
        let bounds = shell.mixer.faders[&node].state.read(cx).bounds();
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
            shell.mixer.faders[&node].state.read(cx).value().end()
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
    crate::ui::window::test_harness::settle_panels(cx);
    cx.update(|window, cx| {
        slider.update(cx, |slider, cx| slider.set_value(start_value, window, cx))
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (thumb_x, thumb_y) = view.update(cx, |shell, cx| {
        let bounds = shell.mixer.faders[&node].state.read(cx).bounds();
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
            shell.mixer.faders[&node].state.read(cx).value().end()
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::window::test_harness::settle_panels(cx);
    let node = view.update(cx, |shell, _| shell.mixer.order[0]);
    let start_value = 1. - FaderScale::position(1.);
    let slider = view.read_with(cx, |shell, _| shell.mixer.faders[&node].state.clone());
    cx.update(|window, cx| {
        slider.update(cx, |slider, cx| slider.set_value(start_value, window, cx))
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    // A real layout change the strip key does not mention: the window is
    // shortened, so every strip's height (and its fader track) changes.
    let strips_before_resize =
        view.read_with(cx, |shell, _| shell.mixer.trace.borrow().strip_renders);
    cx.simulate_resize(size(px(1440.), px(640.)));
    view.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let strips_after_resize =
        view.read_with(cx, |shell, _| shell.mixer.trace.borrow().strip_renders);
    eprintln!(
        "RESIZE-REBUILDS {}",
        strips_after_resize - strips_before_resize
    );
    let (thumb_x, thumb_y, _track_height) = view.update(cx, |shell, cx| {
        let bounds = shell.mixer.faders[&node].state.read(cx).bounds();
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
        let value = shell.mixer.faders[&node].state.read(cx).value().end();
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::window::test_harness::settle_panels(cx);
    let node = view.update(cx, |shell, _| shell.mixer.order[0]);
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
        let bounds = shell.mixer.faders[&node].state.read(cx).bounds();
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
            shell.mixer.faders[&node].state.read(cx).value().end(),
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
            shell.mixer.faders[&node].state.read(cx).value().end(),
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
    let snapshot = crate::ui::window::test_harness::wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let node = snapshot.graph.graph.nodes()[0].id();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| crate::ui::state::UiState::new(handle, snapshot));
    let (view, cx) = crate::ui::window::test_harness::shell_with_state(cx, state);
    cx.simulate_resize(size(px(1024.), px(720.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    crate::ui::window::test_harness::settle_panels(cx);
    view.update(cx, |shell, cx| {
        assert!(
            shell.meter_points.get().0,
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
    crate::ui::window::test_harness::wait_until(Duration::from_secs(8), || {
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
            shell.meter_points.get().0,
            "the probe must stay mounted while the page is at rest"
        );
        assert!(
            !shell.meters.input_subscribed(),
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
    let snapshot = crate::ui::window::test_harness::wait_for(Duration::from_secs(3), || {
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
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
    crate::ui::window::test_harness::settle_panels(cx);
    backend_state.pause_audio.store(true, Ordering::SeqCst);
    // Drain any already-published windows before resetting the clip latch.
    std::thread::sleep(Duration::from_millis(20));
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let (bar_before, strips_before, paints_before) = view.read_with(cx, |shell, cx| {
        let state = shell.state.read(cx);
        let trace = shell.mixer.trace.borrow();
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
        let trace = shell.mixer.trace.borrow();
        assert!(
            state.node_meters.channel_levels(node).unwrap()[0].bar < bar_before,
            "cached controls must not freeze display ballistics between publications"
        );
        assert_eq!(trace.strip_renders, strips_before);
        assert!(trace.meter_paints > paints_before);
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
        shell.mixer_geometry.track_bounds(node).unwrap()
    });
    view.update(cx, |shell, cx| {
        shell.mixer_scroll.set_offset(point(px(-160.), px(0.)));
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let after = view.read_with(cx, |shell, _| {
        shell.mixer_geometry.track_bounds(node).unwrap()
    });
    assert_eq!(after.left(), before.left() - px(160.));
    assert!(
        !view.read_with(cx, |shell, _| shell
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

#[gpui_kit::test]
fn routing_actions_preserve_send_gain_select_the_edge_and_keep_apply_explicit(
    cx: &mut TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
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
        assert_eq!(shell.selected_route, Some(ports));
        assert!(shell.inspector_open);
        assert_eq!(shell.selected_node, None);
    });
    // Closing the inspector restores room without changing route identity.
    view.update(cx, |shell, cx| {
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let (slider, bounds) = view.update(cx, |shell, cx| {
        let slider = shell.mixer.faders[&dst].state.clone();
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
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
            assert_eq!(shell.mixer.editing_send, Some(edge));
            shell.mixer.send_inputs[&edge].update(cx, |input, cx| {
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
        assert!(shell.mixer.editing_send.is_none());
        shell.mixer.order.clone()
    });
    let toggle = cx.update(|window, _| {
        let bounds = window
            .find(SharedString::from(format!("inputs-{:?}", order[0])))
            .bounds();
        bounds.origin + point(px(40.), px(14.))
    });
    cx.simulate_click(toggle, Modifiers::default());
    cx.update(|window, cx| {
        window.render_frame(cx);
        for id in &order {
            assert_eq!(
                window
                    .find(SharedString::from(format!("inputs-{id:?}")))
                    .bounds()
                    .size
                    .height,
                px(28.)
            );
        }
    });
    view.update(cx, |shell, cx| {
        shell.mixer.editing_send = Some(edge);
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::Disconnect(edge),
            ));
        });
    });
    poll(&view, cx);
    view.update(cx, |shell, _| {
        assert!(shell.mixer.editing_send.is_none());
        assert!(!shell.mixer.send_inputs.contains_key(&edge));
        assert!(!shell.mixer.sends.contains_key(&edge));
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn send_numeric_edit_commits_on_blur_and_rejects_invalid(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    activate_window(cx);
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
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
            (gain, shell.mixer.editing_send)
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
            assert_eq!(shell.mixer.editing_send, Some(edge), "editor must open");
        });
    };
    let editor = view.update(cx, |shell, _| shell.mixer.send_inputs[&edge].clone());

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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let edge = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.graph.edges()[0].id()
    });
    let editor = view.update(cx, |shell, _| shell.mixer.send_inputs[&edge].clone());
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
            assert_eq!(shell.mixer.editing_send, Some(edge), "editor must open");
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    let input = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| {
        view.update(cx, |shell, _| shell.mixer.value_inputs[&id].clone())
    };
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> (f64, Option<NodeId>) {
        view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            (
                graph.node(id).expect("strip node").mixer.gain,
                shell.mixer.editing_value,
            )
        })
    };
    let editor = input(&view, cx);

    let editing = open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
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
        open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
        type_and_commit(&view, cx, &editor, rejected);
        let (value, editing) = gain(&view, cx);
        assert_eq!(value, before, "\"{rejected}\" must change nothing");
        assert_eq!(editing, None, "an invalid entry still closes the editor");
    }

    open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
    type_and_commit(&view, cx, &editor, "-inf");
    let (value, _) = gain(&view, cx);
    assert_eq!(value, 0., "\"-inf\" is exact digital silence");
    open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    let editor = view.update(cx, |shell, _| shell.mixer.value_inputs[&id].clone());
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> (f64, Option<NodeId>) {
        view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            (
                graph.node(id).expect("strip node").mixer.gain,
                shell.mixer.editing_value,
            )
        })
    };

    // Focus loss runs the same commit path as Enter.
    let editing = open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
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
        open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    let editor = view.update(cx, |shell, _| shell.mixer.value_inputs[&id].clone());
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> (f64, Option<NodeId>) {
        view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            (
                graph.node(id).expect("strip node").mixer.gain,
                shell.mixer.editing_value,
            )
        })
    };

    open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    let editor = view.update(cx, |shell, _| shell.mixer.value_inputs[&id].clone());
    let gain = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| -> (f64, Option<NodeId>) {
        view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            (
                graph.node(id).expect("strip node").mixer.gain,
                shell.mixer.editing_value,
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

    open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
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
    open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    let label = cx.update(|window, _| window.find(readout_of(id)).bounds());

    open_editor(&view, cx, readout_of(id), |shell| shell.mixer.editing_value);
    let input = view.update(cx, |shell, _| shell.mixer.value_inputs[&id].clone());
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    let label = SharedString::from(format!("balance-label-{id:?}"));
    let editing_flag = |shell: &StudioShell| shell.mixer.editing_balance;
    let editor = view.update(cx, |shell, _| shell.mixer.balance_inputs[&id].clone());
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
                    shell.mixer.editing_balance,
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
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
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
        shell.mixer.node_gain_inputs[&gain_id].clone()
    });
    view.update(cx, |shell, _| shell.mixer.editing_node_gain = Some(gain_id));
    type_and_commit(&view, cx, &input, "+6");
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert!(
            (graph.node(gain_id).unwrap().gain - 10f64.powf(6. / 20.)).abs() < 1e-9,
            "\"+6\" set the node gain, got {}",
            graph.node(gain_id).unwrap().gain
        );
        assert_eq!(graph.node(gain_id).unwrap().mixer.gain, 1.);
        assert!(shell.mixer.editing_node_gain.is_none());
    });

    view.update(cx, |shell, _| shell.mixer.editing_node_gain = Some(gain_id));
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
            .node_pan_inputs
            .keys()
            .copied()
            .last()
            .expect("the pan node has a position editor");
        (id, shell.mixer.node_pan_inputs[&id].clone())
    });
    assert!(
        !view.update(cx, |shell, _| shell.mixer.editing_value.is_some()),
        "the pan node's strip fader family stays independent"
    );
    view.update(cx, |shell, _| shell.mixer.editing_node_pan = Some(pan_id));
    type_and_commit(&view, cx, &input, "R25");
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert!(
            (graph.node(pan_id).unwrap().pan - 0.25).abs() < 1e-9,
            "\"R25\" set the pan position, got {}",
            graph.node(pan_id).unwrap().pan
        );
        assert_eq!(graph.node(pan_id).unwrap().mixer.balance, 0.);
        assert!(shell.mixer.editing_node_pan.is_none());
    });
    finish_service(runtime);
}
