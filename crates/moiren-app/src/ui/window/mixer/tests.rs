use super::{Duration, NodeId, NodeKind, Page, StudioShell, reorder};
use crate::ui::widgets::fader::FaderScale;
use crate::ui::window::test_harness::{
    finish_service, initial_projection, shell_with_state, spawn_service, test_shell, wait_until,
};
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
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut snapshot = (*initial_projection(&handle)).clone();
    snapshot.phase = crate::service::SessionPhase::Running;
    let node = snapshot.graph.graph.nodes()[0].id();
    snapshot
        .signal_topics
        .node_levels
        .insert(node, snapshot.signal_topics.tone_output.clone());
    cx.update(gpui_kit::init);
    let state = cx.new(|_| crate::ui::state::UiState::new(handle, std::sync::Arc::new(snapshot)));
    let (view, cx) = shell_with_state(cx, state);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        shell.state.update(cx, |state, _| {
            std::sync::Arc::make_mut(&mut state.snapshot).phase =
                crate::service::SessionPhase::Running;
        });
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
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
    let chrome_before = view.read_with(cx, |shell, _| {
        let trace = shell.mixer.trace.borrow();
        (trace.toolbar_renders, trace.status_renders)
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
    assert_eq!(
        view.read_with(cx, |shell, _| {
            let trace = shell.mixer.trace.borrow();
            (trace.toolbar_renders, trace.status_renders)
        }),
        chrome_before,
        "meter animation must not rebuild toolbar or status bar"
    );

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
        view.read_with(cx, |shell, _| shell.node_meters.raw_levels(node).is_some())
    });
    let levels = view.read_with(cx, |shell, _| {
        shell.node_meters.raw_levels(node).unwrap().to_vec()
    });
    assert!(
        levels[0].peak >= 1.,
        "the fake physical source is full scale"
    );
    assert!(view.read_with(cx, |shell, _| shell.node_meters.channel_clips(node)[0]));
    crate::ui::window::test_harness::settle_panels(cx);
    backend_state.pause_audio.store(true, Ordering::SeqCst);
    // Drain any already-published windows before resetting the clip latch.
    std::thread::sleep(Duration::from_millis(20));
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let (bar_before, strips_before, paints_before) = view.read_with(cx, |shell, _| {
        let trace = shell.mixer.trace.borrow();
        (
            shell.node_meters.channel_levels(node).unwrap()[0].bar,
            trace.strip_renders,
            trace.meter_paints,
        )
    });
    std::thread::sleep(Duration::from_millis(30));
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    view.read_with(cx, |shell, _| {
        let trace = shell.mixer.trace.borrow();
        assert!(
            shell.node_meters.channel_levels(node).unwrap()[0].bar < bar_before,
            "cached controls must not freeze display ballistics between publications"
        );
        assert_eq!(trace.strip_renders, strips_before);
        assert!(trace.meter_paints > paints_before);
    });
    cx.update(|window, cx| window.click(SharedString::from(format!("meter-reset-{node:?}")), cx));
    assert!(view.read_with(cx, |shell, _| {
        shell.node_meters.channel_clips(node).is_empty()
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
    assert!(view.read_with(cx, |shell, _| shell.node_meters.raw_levels(node).is_none()));
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
