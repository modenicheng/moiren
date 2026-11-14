use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    latency::{
        CallbackObservation, DeviceClockObservation, LatencyRecorder, MeasurementMethod,
        TRACE_CAPACITY, TelemetryMode, qpc_ticks_to_100ns,
    },
    stats::SILENT,
};

mod common;

#[test]
fn event_publication_is_allocation_free_after_retention_wraps() {
    let (mut writer, observer) = moiren_windows_audio::latency::events::event_history();
    common::alloc_guard::assert_allocates_nothing(|| {
        for sequence in 1..=1024 {
            writer.record(64, Some(sequence));
        }
        writer.record(64, None);
        writer.record(64, Some(1));
    });
    let snapshot = observer.snapshot().unwrap();
    assert_eq!(snapshot.events.len(), 256);
    assert_eq!(snapshot.total_events, 1026);
    assert_eq!(snapshot.unavailable_clocks, 1);
}

fn callback(wake: u64, duration: u64) -> CallbackObservation {
    CallbackObservation {
        wake_qpc_100ns: Some(wake),
        completed_qpc_100ns: wake.checked_add(duration),
        demand_frames: 480,
        padding_before_frames: Some(576),
        submitted_frames: 480,
    }
}

#[test]
fn report_snapshots_do_not_embed_full_trace_storage_in_every_owner_handoff() {
    assert!(
        size_of::<moiren_windows_audio::latency::LatencySnapshot>() < 2048,
        "large inline traces multiply across capture/render cleanup stack frames"
    );
}

#[test]
fn qpc_conversion_uses_frequency_and_rejects_overflow() {
    assert_eq!(qpc_ticks_to_100ns(1, 3), Some(3_333_333));
    assert_eq!(qpc_ticks_to_100ns(48_000, 48_000), Some(10_000_000));
    assert_eq!(qpc_ticks_to_100ns(u64::MAX, 10_000_000), Some(u64::MAX));
    assert_eq!(qpc_ticks_to_100ns(u64::MAX, 1), None);
    assert_eq!(qpc_ticks_to_100ns(1, 0), None);
}

#[test]
fn histogram_quantiles_bound_observed_ranks_and_keep_exact_extrema() {
    let mut recorder = LatencyRecorder::new(TelemetryMode::Summary);
    for duration in 0..=100 {
        recorder.record(callback(0, duration));
    }
    let snapshot = recorder.snapshot();
    let duration = snapshot.callback_duration_100ns;
    assert_eq!(duration.samples, 101);
    assert_eq!(duration.min, Some(0));
    assert_eq!(duration.max, Some(100));
    assert_eq!(duration.p50_upper_bound, Some(55));
    assert_eq!(duration.p95_upper_bound, Some(95));
    assert_eq!(duration.p99_upper_bound, Some(100));
    assert!(snapshot.trace.records().is_empty());
    assert_eq!(snapshot.trace_dropped, 0);
    assert_eq!(snapshot.measurement_method, MeasurementMethod::Unknown);
}

#[test]
fn every_histogram_boundary_including_u64_max_is_representable() {
    for exponent in 0..64 {
        let base = 1u64 << exponent;
        for value in [
            base - 1,
            base,
            base.saturating_add(base / 4),
            base.saturating_mul(2).saturating_sub(1),
        ] {
            let mut recorder = LatencyRecorder::new(TelemetryMode::Summary);
            recorder.record(callback(0, value));
            let histogram = recorder.snapshot().callback_duration_100ns;
            assert_eq!(histogram.min, Some(value));
            assert_eq!(histogram.max, Some(value));
            assert!(histogram.p99_upper_bound.unwrap() >= value);
        }
    }
    let mut recorder = LatencyRecorder::new(TelemetryMode::Summary);
    recorder.record(callback(0, u64::MAX));
    assert_eq!(
        recorder.snapshot().callback_duration_100ns.p99_upper_bound,
        Some(u64::MAX)
    );
}

#[test]
fn disabled_profiling_does_not_mutate_any_observations() {
    let mut recorder = LatencyRecorder::new(TelemetryMode::Disabled);
    let before = recorder.snapshot();
    recorder.record(callback(100, 20));
    recorder.record(CallbackObservation::default());
    assert_eq!(recorder.snapshot(), before);
}

#[test]
fn invalid_and_regressed_timestamps_never_wrap_into_latency() {
    let mut recorder = LatencyRecorder::new(TelemetryMode::Trace);
    recorder.record(callback(100, 10));
    recorder.record(callback(90, 10));
    recorder.record(CallbackObservation {
        wake_qpc_100ns: Some(200),
        completed_qpc_100ns: Some(199),
        ..Default::default()
    });
    recorder.record(CallbackObservation::default());
    recorder.record(callback(300, 10));
    recorder.record(callback(400, 10));
    let snapshot = recorder.snapshot();
    assert_eq!(snapshot.invalid_timestamps, 2);
    assert_eq!(snapshot.regressed_wakes, 1);
    assert_eq!(snapshot.wake_interval_100ns.samples, 1);
    assert_eq!(snapshot.wake_interval_100ns.max, Some(100));
    assert_eq!(snapshot.callback_duration_100ns.samples, 4);
    assert_eq!(snapshot.demand_frames.samples, 6);
    assert_eq!(snapshot.trace.records().len(), 4);
}

#[test]
fn snapshot_is_independent_and_serializes_only_valid_trace_entries() {
    let mut recorder = LatencyRecorder::new(TelemetryMode::Trace);
    recorder.record(callback(100, 20));
    let before = recorder.snapshot();
    recorder.record(callback(200, 30));
    assert_eq!(before.trace.records().len(), 1);
    assert_eq!(before.callback_duration_100ns.max, Some(20));
    assert_eq!(recorder.snapshot().trace.records().len(), 2);
    let json = serde_json::to_value(before).unwrap();
    assert_eq!(json["trace"].as_array().unwrap().len(), 1);
    assert_eq!(json["measurement_method"], "unknown");
}

#[test]
fn trace_retains_paired_native_device_clock_position_and_qpc() {
    let mut recorder = LatencyRecorder::new(TelemetryMode::Trace);
    recorder.record_with_device_clock(
        callback(100, 20),
        Some(DeviceClockObservation {
            position_units: 48_000,
            qpc_100ns: 200,
            hresult: 0,
        }),
    );

    let snapshot = recorder.snapshot();
    let record = snapshot.trace.records().first().unwrap();
    assert_eq!(
        record.device_clock,
        Some(DeviceClockObservation {
            position_units: 48_000,
            qpc_100ns: 200,
            hresult: 0,
        })
    );

    let json = serde_json::to_value(snapshot).unwrap();
    assert_eq!(json["trace"][0]["device_clock"]["position_units"], 48_000);
    assert_eq!(json["trace"][0]["device_clock"]["qpc_100ns"], 200);
    assert_eq!(json["trace"][0]["device_clock"]["hresult"], 0);
}

#[test]
fn capture_has_no_invented_render_padding_samples() {
    let mut recorder = LatencyRecorder::new(TelemetryMode::Summary);
    recorder.record(CallbackObservation {
        padding_before_frames: None,
        ..callback(100, 30)
    });
    assert_eq!(recorder.snapshot().padding_before_frames.samples, 0);
}

#[test]
fn trace_overflow_keeps_audio_complete_without_allocation_or_deallocation() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::default()
    })
    .unwrap();
    ingress
        .push_packet(
            &[],
            CapturePacket {
                frames: 8,
                flags: SILENT,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    let mut recorder = LatencyRecorder::new(TelemetryMode::Trace);
    let mut output = [0.0f32; 8];
    common::assert_allocates_nothing(|| {
        for index in 0..TRACE_CAPACITY + 17 {
            assert_eq!(
                ingress
                    .push_packet(
                        &[],
                        CapturePacket {
                            frames: 4,
                            flags: SILENT,
                            device_position_frames: 8 + index as u64 * 4,
                            qpc_100ns: 1 + index as u64,
                        },
                    )
                    .unwrap(),
                4
            );
            assert_eq!(
                source
                    .read_interleaved(&mut output)
                    .unwrap()
                    .transferred_frames,
                4
            );
            recorder.record_with_device_clock(
                callback(100 + index as u64 * 100, 20),
                Some(DeviceClockObservation {
                    position_units: index as u64 * 4,
                    qpc_100ns: 100 + index as u64 * 100,
                    hresult: if index % 2 == 0 { 0 } else { -1 },
                }),
            );
        }
    });
    // Snapshot ownership is finalized off the audio path, after recording ends.
    let snapshot = recorder.snapshot();
    assert_eq!(snapshot.trace.records().len(), TRACE_CAPACITY);
    assert_eq!(snapshot.trace_dropped, 17);
    for (index, record) in snapshot.trace.records().iter().enumerate() {
        assert_eq!(
            record.device_clock,
            Some(DeviceClockObservation {
                position_units: index as u64 * 4,
                qpc_100ns: 100 + index as u64 * 100,
                hresult: if index % 2 == 0 { 0 } else { -1 },
            }),
            "overflow must retain the original clock sample at {index}"
        );
    }
    assert_eq!(
        snapshot.callback_duration_100ns.samples,
        (TRACE_CAPACITY + 17) as u64
    );
    assert_eq!(observer.snapshot().dropped_frames, 0);
    assert_eq!(observer.snapshot().underrun_frames, 0);
}

#[test]
fn latest_packet_timestamp_includes_dropped_packets_and_is_not_fifo_age() {
    let (mut ingress, _source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 8,
        target_fill_frames: 4,
        ..ClockBridgeConfig::default()
    })
    .unwrap();
    assert_eq!(
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    frames: 8,
                    flags: SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 10
                }
            )
            .unwrap(),
        8
    );
    assert_eq!(
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    frames: 4,
                    flags: SILENT,
                    device_position_frames: 8,
                    qpc_100ns: 20
                }
            )
            .unwrap(),
        0
    );
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.last_qpc_100ns, 20);
    assert_eq!(snapshot.dropped_frames, 4);
    assert_eq!(snapshot.written_frames, 8);
}
