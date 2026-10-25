use super::{NodeKind, ServiceRuntime, TrayMixer, UiState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Modifiers, MouseButton, SharedString, TestAppContext, VisualTestContext,
    point, px, size,
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
    // and the edit applies itself. The track midpoint maps to roughly
    // -18 dB.
    let target = cx.update(|window, _| {
        let levels = window
            .find(SharedString::from(format!("tray-strip-levels-{source:?}")))
            .bounds();
        point(levels.left() + px(40.), levels.top() + px(104.))
    });
    cx.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    poll_until(&view, cx, |snapshot| {
        snapshot
            .graph
            .node(source)
            .is_some_and(|node| (0.10..0.16).contains(&node.mixer.gain))
    });
    handle.request_exit();
    runtime.join().unwrap();
}
