use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    latency::packets::PacketPublicationOrigin,
    stats::SILENT,
};
use std::cell::Cell;

mod common;

fn config() -> ClockBridgeConfig {
    ClockBridgeConfig {
        capacity_frames: 8,
        target_fill_frames: 2,
        trim_on_prime: false,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    }
}

fn packet(frames: usize) -> CapturePacket {
    CapturePacket {
        frames,
        flags: SILENT,
        device_position_frames: 0,
        qpc_100ns: 999,
    }
}

#[test]
fn packet_publication_brackets_actual_commit_and_preserves_partial_and_zero_acceptance() {
    let (mut ingress, mut source, observer) = capture_bridge(config()).unwrap();
    let tick = Cell::new(100_u64);
    let mut clock = || {
        let now = tick.get();
        tick.set(now + 1);
        Some(now)
    };
    for (offered, accepted) in [(6, 6), (6, 2), (3, 0)] {
        assert_eq!(
            ingress
                .push_packet_with_publication_clock(&[], packet(offered), &mut clock)
                .unwrap(),
            accepted
        );
    }
    let history = observer.packet_history().unwrap();
    assert_eq!(history.total_events, 3);
    assert_eq!(history.offered_frames, 15);
    assert_eq!(history.published_frames, 8);
    for (index, event) in history.events.iter().enumerate() {
        assert_eq!(event.sample.origin, PacketPublicationOrigin::NativeCapture);
        assert_eq!(event.sample.qpc_before_100ns, Some(100 + index as u64 * 2));
        assert_eq!(event.sample.qpc_after_100ns, Some(101 + index as u64 * 2));
        assert_ne!(
            event.sample.qpc_after_100ns,
            Some(999),
            "not a sample timestamp"
        );
    }
    assert_eq!(history.events[1].offered_before_frames, 6);
    assert_eq!(history.events[1].published_before_frames, 6);
    assert_eq!(history.events[1].sample.accepted_frames, 2);
    assert_eq!(history.events[2].published_before_frames, 8);
    source.read_interleaved(&mut [0.0; 2]).unwrap();
    assert_eq!(
        observer.queue_history().unwrap().events[0]
            .sample
            .before
            .published_frames(),
        Some(8)
    );
    assert!(!history.saturated);
}

#[test]
fn packet_publication_untimed_is_unknown_and_invalid_or_empty_calls_do_not_publish() {
    let (mut ingress, _source, observer) = capture_bridge(config()).unwrap();
    ingress.push_packet(&[], packet(2)).unwrap();
    let calls = Cell::new(0);
    let mut clock = || {
        calls.set(calls.get() + 1);
        Some(7)
    };
    ingress
        .push_packet_with_publication_clock(&[], packet(0), &mut clock)
        .unwrap();
    let invalid = CapturePacket {
        flags: 0,
        ..packet(2)
    };
    assert!(
        ingress
            .push_packet_with_publication_clock(&[], invalid, &mut clock)
            .is_err()
    );
    assert_eq!(calls.get(), 0);
    let history = observer.packet_history().unwrap();
    assert_eq!(history.total_events, 1);
    assert_eq!(history.unavailable_clocks, 1);
    assert_eq!(history.events[0].sample.qpc_before_100ns, None);
    assert_eq!(history.events[0].sample.qpc_after_100ns, None);
}

#[test]
fn injected_clock_observes_ring_before_and_after_publication_not_only_packet_processing() {
    let (mut ingress, source, observer) = capture_bridge(config()).unwrap();
    let calls = Cell::new(0);
    ingress
        .push_packet_with_publication_clock(&[], packet(4), || {
            let call = calls.get();
            assert_eq!(source.snapshot().fill_frames, if call == 0 { 0 } else { 4 });
            calls.set(call + 1);
            Some(100 + call)
        })
        .unwrap();
    assert_eq!(calls.get(), 2);
    assert_eq!(observer.packet_history().unwrap().published_frames, 4);
}

#[test]
fn abandoned_reader_still_records_offered_supply_without_inventing_publication() {
    let (mut ingress, source, observer) = capture_bridge(config()).unwrap();
    drop(source);
    assert_eq!(
        ingress
            .push_packet_with_publication_clock(&[], packet(4), || Some(100))
            .unwrap(),
        0
    );
    let history = observer.packet_history().unwrap();
    assert_eq!(history.offered_frames, 4);
    assert_eq!(history.published_frames, 0);
    assert_eq!(history.events[0].sample.accepted_frames, 0);
    assert_eq!(observer.snapshot().dropped_frames, 4);
}

#[test]
fn packet_publication_never_allocates_or_deallocates_and_retains_frontier_suffix() {
    let (mut ingress, mut source, observer) = capture_bridge(config()).unwrap();
    common::assert_allocates_nothing(|| {
        for stamp in 1..=300 {
            ingress
                .push_packet_with_publication_clock(&[], packet(3), || Some(stamp))
                .unwrap();
            source.read_interleaved(&mut [0.0; 6]).unwrap();
        }
    });
    drop(ingress);
    let history = observer.packet_history().unwrap();
    assert_eq!(history.total_events, 300);
    assert_eq!(history.overwritten_events, 44);
    assert_eq!(history.events[0].offered_before_frames, 132);
    assert_eq!(history.events[0].published_before_frames, 132);
    assert_eq!(history.offered_frames, 900);
    assert_eq!(history.published_frames, 900);
    assert!(!history.saturated);
}

#[test]
fn packet_publication_storage_is_part_of_actual_preparation_budget() {
    let bytes = moiren_windows_audio::latency::packets::packet_history_storage_bytes();
    assert!(bytes >= 256 * 8 * 8);
    assert!(
        capture_bridge(ClockBridgeConfig {
            byte_budget: bytes,
            ..config()
        })
        .is_err()
    );
}

#[test]
fn packet_publication_discontinuity_and_overflow_keep_the_written_generation() {
    let (mut ingress, _source, observer) = capture_bridge(config()).unwrap();
    for flags in [0, 0, moiren_windows_audio::stats::DISCONTINUITY, 0] {
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    flags: flags | SILENT,
                    ..packet(6)
                },
            )
            .unwrap();
    }
    let history = observer.packet_history().unwrap();
    let generations = history
        .events
        .iter()
        .map(|event| event.sample.buffer_generation)
        .collect::<Vec<_>>();
    assert_eq!(generations, [1, 1, 3, 4]);
    assert_eq!(history.events[1].sample.accepted_frames, 2);
    assert_eq!(history.events[2].sample.accepted_frames, 0);
    assert_ne!(
        history.events[2].sample.flags & moiren_windows_audio::stats::DISCONTINUITY,
        0
    );
}

#[test]
fn graph_output_publication_is_counted_but_never_native_packet_timing() {
    use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
    use moiren_engine::{
        boundary::ConstantSource,
        compiler::{CompileConfig, NodeBindings, compile},
        runtime::EngineConfig,
    };
    let (ingress, _source, observer) = capture_bridge(config()).unwrap();
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    graph
        .connect(
            graph.get_node(source).unwrap().outputs()[0].id(),
            graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let mut bindings = NodeBindings::<f32>::new();
    bindings
        .bind_source(
            source,
            ConstantSource {
                channels: 2,
                value: 0.25,
            },
        )
        .unwrap();
    bindings.bind_sink(sink, ingress).unwrap();
    let mut compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 6,
                max_events_per_block: 4,
            },
            audio_byte_budget: 1 << 20,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 4,
            control_horizon_frames: 48_000,
        },
    )
    .unwrap();
    common::assert_allocates_nothing(|| {
        compiled.engine.render(6).unwrap();
    });
    common::assert_allocates_nothing(|| {
        compiled.engine.render(6).unwrap();
    });
    let history = observer.packet_history().unwrap();
    assert_eq!(history.offered_frames, 12);
    assert_eq!(history.published_frames, 8);
    assert_eq!(history.unavailable_clocks, 2);
    for event in history.events {
        assert_eq!(event.sample.origin, PacketPublicationOrigin::GraphOutput);
        assert_eq!(event.sample.qpc_before_100ns, None);
        assert_eq!(event.sample.qpc_after_100ns, None);
    }
    assert!(observer.delivery_events().unwrap().events.is_empty());
}
