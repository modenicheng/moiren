use super::*;
use crate::latency::{InputDeliveryRecorder, events::event_history};

#[derive(Clone)]
struct Observations {
    delivery: InputDeliverySnapshot,
    producer: EventHistorySnapshot,
    consumer: EventHistorySnapshot,
}

fn process() -> ProcessIdentity {
    ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    }
}

fn history(events: &[(u64, u64)]) -> EventHistorySnapshot {
    let (mut writer, observer) = event_history();
    for &(frames, stamp) in events {
        writer.record(frames, Some(stamp));
    }
    observer.snapshot().unwrap()
}

fn observations(
    producer: &[(u64, u64)],
    consumer: &[(u64, u64)],
    packet_frames: u32,
) -> Observations {
    let mut recorder = InputDeliveryRecorder::new(None).unwrap();
    for &(frames, stamp) in producer {
        let mut remaining = frames;
        while remaining > 0 {
            let packet = remaining.min(u64::from(packet_frames)) as u32;
            recorder.packet(packet, Some(stamp - 1));
            remaining -= u64::from(packet);
        }
        recorder.callback_burst(frames);
    }
    Observations {
        delivery: recorder.snapshot(),
        producer: history(producer),
        consumer: history(consumer),
    }
}

fn regular() -> Observations {
    observations(
        &[
            (96, 1_000_000),
            (96, 1_020_000),
            (96, 1_040_000),
            (96, 1_060_000),
            (96, 1_080_000),
        ],
        &[
            (96, 1_001_000),
            (96, 1_021_000),
            (96, 1_041_000),
            (96, 1_061_000),
            (96, 1_081_000),
        ],
        48,
    )
}

fn measure_with(
    observation: Observations,
    consumer_rate: u32,
    prepared_demand: u32,
) -> Result<MeasuredProcessDelivery, MeasuredAdmissionError> {
    MeasuredProcessDelivery::from_observations(
        process(),
        7,
        48_000,
        consumer_rate,
        prepared_demand,
        observation.delivery,
        observation.producer,
        observation.consumer,
    )
}

fn measure(observation: Observations) -> Result<MeasuredProcessDelivery, MeasuredAdmissionError> {
    measure_with(observation, 48_000, 96)
}

fn target(capacity: u32, frames: u32, jitter: u32) -> TransportInputPolicy {
    TransportInputPolicy::Target {
        capacity_frames: capacity,
        target_frames: frames,
        jitter_frames: jitter,
    }
}

fn admit(
    evidence: &MeasuredProcessDelivery,
    input: TransportInputPolicy,
    rate: u32,
    demand: u32,
) -> Result<ProcessBridgeOptions, MeasuredAdmissionError> {
    evidence.admit_for_consumer(&process(), 7, input, rate, demand)
}

#[test]
fn measured_history_admits_1024_512_and_256_without_inventing_a_native_period() {
    let evidence = measure(regular()).unwrap();
    assert!(evidence.process().same_process(&process()));
    assert_eq!(evidence.owner_generation(), 7);
    for frames in [1024, 512, 256] {
        let options = admit(&evidence, target(2048, frames, 16), 48_000, 96).unwrap();
        let delivery = options.delivery.unwrap();
        assert_eq!(delivery.source, DeliveryTimingSource::Empirical);
        assert_eq!(delivery.max_packet_frames, 96);
        assert_eq!(delivery.max_gap_frames, 96);
        assert_eq!(options.prepare().unwrap().modeled_minimum_frames, Some(210));
        assert_eq!(options.prepare().unwrap().prime_frames, frames);
        let json = serde_json::to_value(options).unwrap();
        assert!(json["delivery"].get("period_frames").is_none());
        assert_eq!(json["age_limit_100ns"], serde_json::Value::Null);
    }
}

#[test]
fn measured_admission_requires_a_target_and_pinned_identity_and_generation() {
    let evidence = measure(regular()).unwrap();
    let policy = target(2048, 256, 16);
    let mut recycled = process();
    recycled.creation_time_100ns += 1;
    assert!(matches!(
        evidence.admit_for_consumer(&recycled, 7, policy, 48_000, 96),
        Err(MeasuredAdmissionError::IdentityMismatch)
    ));
    assert!(matches!(
        evidence.admit_for_consumer(&process(), 8, policy, 48_000, 96),
        Err(MeasuredAdmissionError::GenerationMismatch)
    ));
    assert!(matches!(
        admit(&evidence, TransportInputPolicy::Compatible, 48_000, 96),
        Err(MeasuredAdmissionError::TargetRequired)
    ));
}

#[test]
fn zero_identity_generation_and_invalid_rates_or_prepared_demand_are_refused() {
    for (pid, creation, generation, input_rate, consumer_rate, demand) in [
        (0, 123, 7, 48_000, 48_000, 96),
        (42, 0, 7, 48_000, 48_000, 96),
        (42, 123, 0, 48_000, 48_000, 96),
        (42, 123, 7, 44_100, 48_000, 96),
        (42, 123, 7, 48_000, 0, 96),
        (42, 123, 7, 48_000, 12_345, 96),
        (42, 123, 7, 48_000, 48_000, 0),
    ] {
        let observation = regular();
        let identity = ProcessIdentity {
            pid,
            creation_time_100ns: creation,
            ..process()
        };
        assert!(
            MeasuredProcessDelivery::from_observations(
                identity,
                generation,
                input_rate,
                consumer_rate,
                demand,
                observation.delivery,
                observation.producer,
                observation.consumer,
            )
            .is_err()
        );
    }
}

#[test]
fn actual_larger_consumer_demand_is_revalidated_including_rate_conversion() {
    let evidence = measure(regular()).unwrap();
    assert!(admit(&evidence, target(2048, 256, 16), 48_000, 96).is_ok());
    assert!(matches!(
        admit(&evidence, target(2048, 256, 16), 48_000, 256),
        Err(MeasuredAdmissionError::Budget(
            LatencyBudgetError::InsufficientInputBudget
        ))
    ));
    let options = admit(&evidence, target(2048, 512, 16), 44_100, 128).unwrap();
    assert_eq!(options.max_demand_frames, 128);
    assert_eq!(options.prepare().unwrap().low_water_frames, 142);
    for (rate, demand) in [(0, 96), (12_345, 96), (48_000, 0)] {
        assert!(admit(&evidence, target(2048, 512, 16), rate, demand).is_err());
    }
}

#[test]
fn observed_and_prepared_consumer_bounds_survive_a_smaller_new_estimate() {
    let mut observation = regular();
    observation.consumer.events[1].frames = 200;
    let evidence = measure_with(observation, 48_000, 96).unwrap();
    let options = admit(&evidence, target(2048, 512, 16), 48_000, 32).unwrap();
    assert_eq!(options.max_demand_frames, 200);
    assert!(admit(&evidence, target(2048, 256, 16), 48_000, 32).is_err());
    let evidence = measure_with(regular(), 48_000, 220).unwrap();
    assert_eq!(
        admit(&evidence, target(2048, 512, 16), 48_000, 32)
            .unwrap()
            .max_demand_frames,
        220
    );
}

#[test]
fn historical_consumer_demand_is_rebound_upward_between_engine_rates() {
    let evidence = measure_with(regular(), 44_100, 96).unwrap();
    let options = admit(&evidence, target(2048, 512, 16), 48_000, 32).unwrap();
    assert_eq!(options.max_demand_frames, 105);
    assert_eq!(options.prepare().unwrap().low_water_frames, 107);
}

#[test]
fn packet_gap_and_full_drain_gap_both_constrain_admission_with_ceiling() {
    let mut observation = regular();
    observation.delivery.max_gap_100ns = 40_001;
    let evidence = measure(observation).unwrap();
    let options = admit(&evidence, target(2048, 512, 16), 48_000, 96).unwrap();
    assert_eq!(options.delivery.unwrap().max_gap_frames, 193);
    assert!(admit(&evidence, target(2048, 256, 16), 48_000, 96).is_err());

    let mut observation = regular();
    observation.delivery.max_gap_100ns = 19_000;
    let evidence = measure(observation).unwrap();
    assert_eq!(
        admit(&evidence, target(2048, 256, 16), 48_000, 96)
            .unwrap()
            .delivery
            .unwrap()
            .max_gap_frames,
        96
    );
}

#[test]
fn burst_headroom_uses_complete_callback_drain_instead_of_single_packet() {
    let evidence = measure(regular()).unwrap();
    assert!(matches!(
        admit(&evidence, target(352, 256, 16), 48_000, 96),
        Err(MeasuredAdmissionError::Budget(
            LatencyBudgetError::InvalidInputTarget
        ))
    ));
    assert_eq!(
        admit(&evidence, target(353, 256, 16), 48_000, 96)
            .unwrap()
            .prepare()
            .unwrap()
            .high_water_frames,
        257
    );
}

#[test]
fn favourable_phase_and_timestamp_ties_never_reduce_the_all_phase_minimum() {
    let evidence = measure(regular()).unwrap();
    assert_eq!(evidence.phase_range_100ns(), Some((1000, 1000)));
    assert_eq!(evidence.timestamp_ties(), 0);
    assert!(admit(&evidence, target(2048, 209, 16), 48_000, 96).is_err());
    let mut observation = regular();
    for event in &mut observation.consumer.events {
        event.qpc_100ns = event.qpc_100ns.map(|stamp| stamp - 1000);
    }
    let evidence = measure(observation).unwrap();
    assert_eq!(evidence.phase_range_100ns(), None);
    assert_eq!(evidence.timestamp_ties(), 5);
    assert_eq!(
        admit(&evidence, target(2048, 256, 16), 48_000, 96)
            .unwrap()
            .prepare()
            .unwrap()
            .modeled_minimum_frames,
        Some(210)
    );
}

#[test]
fn phase_report_retains_a_range_and_uses_only_overlapping_consumers() {
    let evidence = measure(observations(
        &[
            (96, 1_000_000),
            (96, 1_020_000),
            (96, 1_040_000),
            (96, 1_060_000),
        ],
        &[
            (96, 1_001_000),
            (96, 1_025_000),
            (96, 1_060_000),
            (96, 2_000_000),
        ],
        48,
    ))
    .unwrap();
    assert_eq!(evidence.phase_range_100ns(), Some((1000, 5000)));
    assert_eq!(evidence.timestamp_ties(), 1);
    let json = serde_json::to_value(evidence).unwrap();
    assert_eq!(json["phase"]["consumer_events"], 3);
    assert_eq!(json["phase"]["producer_events"], 3);
}

#[test]
fn per_wake_admission_retains_but_does_not_qualify_pathological_consumer_density() {
    let producer: Vec<_> = (0..5)
        .map(|index| (96, 1_000_000 + index * 20_000))
        .collect();
    let consumer: Vec<_> = (0..100)
        .map(|index| (96, 1_001_000 + index * 1000))
        .collect();
    let evidence = measure(observations(&producer, &consumer, 48)).unwrap();
    // Every individual wake fits the conservative floor, but their density
    // cannot sustain queue balance. With unknown entry fill and intra-drain
    // publication order this API does not claim a queue-feasibility replay.
    // Keep the actual event history and phase counts so callers see that limit.
    let options = admit(&evidence, target(2048, 256, 16), 48_000, 96).unwrap();
    assert_eq!(options.prepare().unwrap().modeled_minimum_frames, Some(210));
    let json = serde_json::to_value(evidence).unwrap();
    assert_eq!(json["consumer"]["total_events"], 100);
    assert_eq!(json["producer"]["total_events"], 5);
    assert_eq!(json["phase"]["consumer_events"], 80);
    assert_eq!(json["phase"]["producer_events"], 4);
}

#[test]
fn insufficient_histories_and_nonoverlap_do_not_create_phase_evidence() {
    for (producer, consumer) in [
        (vec![(96, 100)], vec![(96, 110), (96, 210)]),
        (vec![(96, 100), (96, 200)], vec![(96, 110)]),
        (vec![(96, 100), (96, 200)], vec![(96, 300), (96, 400)]),
        (
            vec![(96, 100), (96, 200), (96, 300)],
            vec![(96, 250), (96, 350), (96, 450)],
        ),
        (vec![(96, 100), (96, 100)], vec![(96, 100), (96, 100)]),
    ] {
        assert!(measure(observations(&producer, &consumer, 48)).is_err());
    }
}

#[test]
fn unavailable_regressed_or_saturated_evidence_is_refused_even_with_valid_tail() {
    for producer in [true, false] {
        for fault in 0..6 {
            let mut observation = regular();
            let history = if producer {
                &mut observation.producer
            } else {
                &mut observation.consumer
            };
            match fault {
                0 => history.unavailable_clocks = 1,
                1 => history.regressed_clocks = 1,
                2 => history.saturated = true,
                3 => history.events[1].qpc_100ns = None,
                4 => history.events[1].qpc_100ns = Some(0),
                5 => history.events[1].qpc_100ns = Some(1),
                _ => unreachable!(),
            }
            assert!(measure(observation).is_err(), "{producer} {fault}");
        }
    }
    for fault in 0..2 {
        let mut observation = regular();
        if fault == 0 {
            observation.delivery.unavailable_clocks = 1;
        } else {
            observation.delivery.regressed_clocks = 1;
        }
        assert!(measure(observation).is_err());
    }
}

#[test]
fn retained_sequences_counts_and_frame_shapes_must_be_consistent() {
    for producer in [true, false] {
        for fault in 0..8 {
            let mut observation = regular();
            let history = if producer {
                &mut observation.producer
            } else {
                &mut observation.consumer
            };
            match fault {
                0 => history.events[0].sequence = 0,
                1 => history.events[2].sequence += 1,
                2 => history.total_events += 1,
                3 => history.overwritten_events += 1,
                4 => history.events[1].frames = 0,
                5 => history.events[1].frames = u64::MAX,
                6 => history.total_events = u64::MAX,
                7 => history.events.resize(257, history.events[0]),
                _ => unreachable!(),
            }
            assert!(measure(observation).is_err(), "{producer} {fault}");
        }
    }
}

#[test]
fn self_consistent_shortened_suffix_cannot_erase_historical_demand() {
    let mut observation = regular();
    observation.consumer.events[0].frames = 5000;
    observation.consumer.events.drain(..2);
    observation.consumer.overwritten_events = 2;
    assert_eq!(observation.consumer.total_events, 5);
    assert!(matches!(
        measure(observation),
        Err(MeasuredAdmissionError::InvalidHistory {
            observation: "consumer"
        })
    ));
}

#[test]
fn genuine_overwritten_prefixes_are_explicit_and_keep_cumulative_maxima() {
    let producer: Vec<_> = (0..300)
        .map(|index| {
            let frames = if index == 0 { 192 } else { 96 };
            (frames, 1_000_000 + index * 20_000)
        })
        .collect();
    let consumer: Vec<_> = (0..300)
        .map(|index| (96, 1_001_000 + index * 20_000))
        .collect();
    let observation = observations(&producer, &consumer, 48);
    assert_eq!(observation.producer.overwritten_events, 44);
    let evidence = measure(observation).unwrap();
    let options = admit(&evidence, target(2048, 256, 16), 48_000, 96).unwrap();
    assert_eq!(options.delivery.unwrap().max_packet_frames, 192);
    let json = serde_json::to_value(evidence).unwrap();
    assert_eq!(json["producer"]["overwritten_events"], 44);
    assert_eq!(json["consumer"]["overwritten_events"], 44);
    assert_eq!(json["delivery"]["max_callback_burst_frames"], 192);
}

#[test]
fn overwritten_prefix_cannot_explain_a_burst_larger_than_its_packet_supply() {
    let producer: Vec<_> = (0..260)
        .map(|index| (48, 1_000_000 + index * 20_000))
        .collect();
    let consumer: Vec<_> = (0..260)
        .map(|index| (48, 1_001_000 + index * 20_000))
        .collect();
    let mut observation = observations(&producer, &consumer, 48);
    assert_eq!(observation.producer.overwritten_events, 4);
    // All 260 packets are already required by the 260 observed callbacks;
    // the overwritten prefix cannot also contain a two-packet callback.
    observation.delivery.max_callback_burst_frames = 96;
    assert!(matches!(
        measure(observation),
        Err(MeasuredAdmissionError::InconsistentDelivery)
    ));
}

#[test]
fn overwritten_prefix_cannot_explain_more_packets_than_its_maximum_frames() {
    let producer: Vec<_> = (0..260)
        .map(|index| (48, 1_000_000 + index * 20_000))
        .collect();
    let consumer: Vec<_> = (0..260)
        .map(|index| (48, 1_001_000 + index * 20_000))
        .collect();
    let mut observation = observations(&producer, &consumer, 48);
    observation.delivery.packets = 260 * 48 + 1;
    observation.delivery.gap_samples = observation.delivery.packets - 1;
    assert!(matches!(
        measure(observation),
        Err(MeasuredAdmissionError::InconsistentDelivery)
    ));
}

#[test]
fn aggregate_packet_and_burst_evidence_cannot_contradict_retained_history() {
    for fault in 0..10 {
        let mut observation = regular();
        match fault {
            0 => observation.delivery.max_packet_frames = 0,
            1 => observation.delivery.max_packet_frames = 97,
            2 => observation.delivery.max_callback_burst_frames = 95,
            3 => observation.delivery.max_callback_burst_frames = 97,
            4 => observation.delivery.packets = 4,
            5 => observation.delivery.gap_samples = 0,
            6 => observation.delivery.packet_bound_violations = 11,
            7 => observation.delivery.gap_bound_violations = 10,
            8 => observation.delivery.packets = u64::MAX,
            9 => {
                observation.delivery.max_packet_frames = 47;
            }
            _ => unreachable!(),
        }
        assert!(measure(observation).is_err(), "{fault}");
    }
}

#[test]
fn previous_declared_bound_violations_remain_raw_facts_not_invented_bounds() {
    let mut observation = regular();
    observation.delivery.packet_bound_violations = 2;
    observation.delivery.gap_bound_violations = 1;
    let evidence = measure(observation).unwrap();
    assert!(admit(&evidence, target(2048, 256, 16), 48_000, 96).is_ok());
    let json = serde_json::to_value(evidence).unwrap();
    assert_eq!(json["delivery"]["packet_bound_violations"], 2);
    assert_eq!(json["delivery"]["gap_bound_violations"], 1);
    assert_eq!(json["producer"]["total_events"], 5);
    assert_eq!(json["consumer"]["total_events"], 5);
}

#[test]
fn overflowing_gaps_bursts_demand_and_budget_are_refused() {
    let mut observation = regular();
    observation.delivery.max_gap_100ns = u64::MAX;
    assert!(matches!(
        measure(observation),
        Err(MeasuredAdmissionError::ArithmeticOverflow)
    ));
    let mut observation = regular();
    observation.delivery.max_callback_burst_frames = u64::MAX;
    assert!(measure(observation).is_err());
    let evidence = measure_with(regular(), 44_100, u32::MAX).unwrap();
    assert!(matches!(
        admit(&evidence, target(u32::MAX, 1024, 16), 96_000, 1),
        Err(MeasuredAdmissionError::ArithmeticOverflow)
    ));
    let evidence = measure(regular()).unwrap();
    assert!(matches!(
        admit(&evidence, target(u32::MAX, 1024, u32::MAX), 48_000, 96),
        Err(MeasuredAdmissionError::Budget(
            LatencyBudgetError::ArithmeticOverflow
        ))
    ));
}
