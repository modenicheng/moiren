use super::{NodeKind, ServiceRuntime, TrayMixer, UiState};
use crate::ui::widgets::fader::FaderScale;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, SharedString,
    TestAppContext, VisualTestContext, point, px, size,
};
use std::time::Duration;

/// Poll until the snapshot satisfies `done` and no apply is in flight.
/// Headless sessions never confirm revisions, so waiting for
/// `active_revision` would never settle.
fn poll_until(
    view: &Entity<TrayMixer>,
    cx: &mut VisualTestContext,
    mut done: impl FnMut(&crate::service::AppSnapshot) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let ready = view.update(cx, |flyout, cx| {
            flyout.state.update(cx, |state, _| state.poll());
            let snapshot = &flyout.state.read(cx).snapshot;
            snapshot.graph.pending_revision.is_none() && done(snapshot)
        });
        cx.update(|window, cx| window.render_frame(cx));
        if ready {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The two halves of the flyout's meter pipeline must stay wired together:
/// the strips' canvases register into the frame the hot surface reads, and
/// the hot layer paints them. The surfaces drifting apart left the flyout
/// with no meter bars at all.
#[gpui_kit::test]
fn tray_mixer_hot_layer_paints_the_strips_registered_meter_tracks(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = cx.add_window_view(|window, cx| TrayMixer::new(state, None, window, cx));
    cx.update(|window, _| window.resize(size(px(360.), px(372.))));
    cx.update(|window, cx| window.render_frame(cx));
    let (registered, painted) = view.update(cx, |flyout, cx| {
        let meters = flyout.meter_layer.read(cx).surface();
        let meters = meters.borrow();
        (meters.registered_tracks(), meters.painted_tracks())
    });
    assert!(
        registered > 0,
        "the hot surface must see the strips' registered meter tracks"
    );
    assert!(
        painted >= registered,
        "the hot layer must paint every registered track"
    );
    handle.request_exit();
    runtime.join().unwrap();
}

#[gpui_kit::test]
fn tray_mixer_lists_audio_strips_and_applies_mute_immediately(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = cx.add_window_view(|window, cx| TrayMixer::new(state, None, window, cx));
    cx.update(|window, _| window.resize(size(px(360.), px(372.))));
    cx.update(|window, cx| window.render_frame(cx));
    let (source, dsp, sink) = view.update(cx, |flyout, cx| {
        let graph = &flyout.state.read(cx).snapshot.graph.graph;
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
    // Audio-relevant strips render; DSP shaping stays in the main mixer.
    view.update(cx, |flyout, _| {
        assert!(flyout.faders.contains_key(&source));
        assert!(flyout.faders.contains_key(&sink));
        assert!(!flyout.faders.contains_key(&dsp));
    });
    cx.update(|window, cx| {
        window.click(
            SharedString::from(format!("tray-strip-mute-{source:?}")),
            cx,
        );
    });
    // The mute edit lands in the candidate; every edit applies itself.
    poll_until(&view, cx, |snapshot| {
        snapshot
            .graph
            .node(source)
            .is_some_and(|node| node.mixer.muted)
    });
    handle.request_exit();
    runtime.join().unwrap();
}

#[gpui_kit::test]
fn tray_mixer_fader_drag_edits_the_gain(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = cx.add_window_view(|window, cx| TrayMixer::new(state, None, window, cx));
    cx.update(|window, _| window.resize(size(px(360.), px(372.))));
    cx.update(|window, cx| window.render_frame(cx));
    let source = view.update(cx, |flyout, cx| {
        let graph = &flyout.state.read(cx).snapshot.graph.graph;
        graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Source)
            .unwrap()
            .id()
    });
    // Press the track's midpoint and release: the press jumps the value
    // and the edit applies itself. The tray now shares the main mixer's
    // perceptual value space, so the midpoint lands near the scale's middle
    // (about -11.5 dB); derive the window from the scale instead of a
    // hardcoded amplitude range.
    let target = cx.update(|window, _| {
        let levels = window
            .find(SharedString::from(format!("tray-strip-levels-{source:?}")))
            .bounds();
        point(levels.left() + px(40.), levels.top() + px(104.))
    });
    cx.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    let middle = FaderScale::gain(0.5);
    poll_until(&view, cx, |snapshot| {
        snapshot
            .graph
            .node(source)
            .is_some_and(|node| (middle * 0.9..middle * 1.1).contains(&node.mixer.gain))
    });
    handle.request_exit();
    runtime.join().unwrap();
}

/// Wheel and double-click go through the behavior's commit path: one wheel
/// notch is a multiplicative +1 dB, and a double click restores unity (0 dB).
/// A wheel notch is `Lines { y: 1. }` — gpui's positive wheel delta means the
/// wheel moved up, which must increase the value.
#[gpui_kit::test]
fn tray_mixer_fader_wheel_steps_and_double_click_resets(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = cx.add_window_view(|window, cx| TrayMixer::new(state, None, window, cx));
    cx.update(|window, _| window.resize(size(px(360.), px(372.))));
    cx.update(|window, cx| window.render_frame(cx));
    let source = view.update(cx, |flyout, cx| {
        let graph = &flyout.state.read(cx).snapshot.graph.graph;
        graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Source)
            .unwrap()
            .id()
    });
    let before = view.update(cx, |flyout, cx| {
        flyout
            .state
            .read(cx)
            .snapshot
            .graph
            .node(source)
            .unwrap()
            .mixer
            .gain
    });
    let expected = before * 10f64.powf(1. / 20.);
    // Hover the fader and wheel up one notch.
    let center = view.update(cx, |flyout, cx| {
        flyout.faders[&source].state.read(cx).bounds().center()
    });
    cx.simulate_mouse_move(center, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position: center,
        delta: ScrollDelta::Lines(point(0., 1.)),
        ..Default::default()
    });
    poll_until(&view, cx, |snapshot| {
        snapshot
            .graph
            .node(source)
            .is_some_and(|node| (node.mixer.gain - expected).abs() < expected * 0.01)
    });
    // The commit path also wrote the stepped value into the slider state.
    view.update(cx, |flyout, cx| {
        let value = flyout.faders[&source].state.read(cx).value().end();
        let stepped = 1. - FaderScale::position(expected);
        assert!(
            (value - stepped).abs() < 0.001,
            "slider value {value} != stepped {stepped}"
        );
    });
    // Double click the track below the thumb (a plain click jumps there; the
    // second click restores unity).
    cx.simulate_event(gpui_kit::MouseDownEvent {
        position: center,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(gpui_kit::MouseUpEvent {
        position: center,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    cx.simulate_event(gpui_kit::MouseDownEvent {
        position: center,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui_kit::MouseUpEvent {
        position: center,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    poll_until(&view, cx, |snapshot| {
        // f32 round trips through the perceptual scale leave ~1e-7 slack.
        snapshot
            .graph
            .node(source)
            .is_some_and(|node| (node.mixer.gain - 1.).abs() < 1e-6)
    });
    handle.request_exit();
    runtime.join().unwrap();
}
