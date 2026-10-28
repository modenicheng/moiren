use moiren_windows_audio::{
    clock_bridge::BridgeSnapshot,
    latency::TelemetryMode,
    latency::render::{RENDER_EVENT_CAPACITY, RenderCallback, RenderDiagnostics},
};

mod common;

#[test]
fn diagnostic_recorder_does_not_embed_large_bridge_history_in_owner_handoffs() {
    assert!(
        size_of::<RenderDiagnostics>() < 256,
        "bounded callback history must be prepared on the heap, not copied into every native report"
    );
}

fn callback(at: u64) -> RenderCallback {
    RenderCallback {
        wake_qpc_100ns: Some(at),
        padding_before_frames: 120,
        demand_frames: 240,
        bridge_before: Some(BridgeSnapshot {
            fill_frames: 1600,
            correction_ppm: 42.0,
            ..Default::default()
        }),
        bridge_after: Some(BridgeSnapshot {
            fill_frames: 1360,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn recovery_retains_preceding_callbacks_and_distinguishes_native_wake_budget() {
    let mut diagnostics = RenderDiagnostics::new(TelemetryMode::Trace, 75_000);
    for index in 0..12 {
        diagnostics.record(callback(100_000 + index * 50_000));
    }
    let mut recovered = callback(800_000);
    recovered.bridge_after.as_mut().unwrap().backlog_recoveries = 1;
    recovered
        .bridge_after
        .as_mut()
        .unwrap()
        .backlog_discarded_frames = 780;
    diagnostics.record(recovered);
    let snapshot = diagnostics.finish();
    assert_eq!(snapshot.events.len(), 1);
    let event = &snapshot.events[0];
    assert!(event.wake_over_budget);
    assert!(event.bridge_recovered);
    assert!(!event.empty_padding);
    assert!(!event.bridge_underrun);
    assert_eq!(event.callback.wake_interval_100ns, Some(150_000));
    assert_eq!(event.previous_callbacks.len(), 8);
    assert_eq!(event.previous_callbacks[0].wake_qpc_100ns, Some(300_000));
    assert_eq!(event.callback.bridge_before.unwrap().correction_ppm, 42.0);
}

#[test]
fn late_events_replace_old_events_with_visible_loss_and_no_rt_heap_work() {
    let mut diagnostics = RenderDiagnostics::new(TelemetryMode::Trace, 75_000);
    let snapshot = common::assert_allocates_nothing(|| {
        for index in 0..RENDER_EVENT_CAPACITY + 17 {
            let mut observation = callback(index as u64 * 50_000);
            observation.padding_before_frames = 0;
            observation.failed = index.is_multiple_of(2);
            diagnostics.record(observation);
        }
        diagnostics.finish()
    });
    assert_eq!(snapshot.events.len(), RENDER_EVENT_CAPACITY);
    assert_eq!(snapshot.overwritten_events, 17);
    assert_eq!(snapshot.events[0].callback.wake_qpc_100ns, Some(850_000));
    assert!(snapshot.events.iter().all(|event| event.empty_padding));
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap()["events"][0]["previous_callbacks"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
}

#[test]
fn summary_mode_and_missing_or_regressed_clock_do_not_invent_wake_delays() {
    let mut empty = callback(100_000);
    empty.padding_before_frames = 0;
    empty.failed = true;
    for mode in [TelemetryMode::Disabled, TelemetryMode::Summary] {
        let mut disabled = RenderDiagnostics::new(mode, 75_000);
        disabled.record(empty);
        let snapshot = disabled.finish();
        assert!(!snapshot.enabled);
        assert_eq!(snapshot.events.capacity(), 0);
        assert!(snapshot.events.is_empty());
    }
    let mut diagnostics = RenderDiagnostics::new(TelemetryMode::Trace, 75_000);
    diagnostics.record(callback(100_000));
    let mut invalid = callback(90_000);
    invalid.wake_qpc_100ns = None;
    diagnostics.record(invalid);
    diagnostics.record(callback(300_000));
    diagnostics.record(callback(200_000));
    assert!(diagnostics.finish().events.is_empty());
}
