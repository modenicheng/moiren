use crate::{
    service::{
        AppSnapshot, NativeSessionObservations, ServiceContext, SessionGeneration, SessionPhase,
    },
    ui::presentation::native_timing_rows,
};
use moiren_windows_audio::{
    clock_bridge::{BridgeSnapshot, QueuedFrameAge, QueuedFrameStamp},
    engine_backend::{
        NativeBridgeObservation, NativeObservationSnapshot, NativeRenderPaddingObservation,
        NativeStreamKind, NativeStreamObservation,
    },
    latency::{InputDeliverySnapshot, render::RenderPaddingSnapshot},
};

fn live_snapshot() -> AppSnapshot {
    let mut snapshot = crate::service::snapshot(&ServiceContext::default());
    snapshot.phase = SessionPhase::Running;
    snapshot.generation = SessionGeneration(8);
    let nodes = snapshot.graph.graph.nodes();
    let input = nodes[0].id();
    let output = nodes[1].id();
    let stream = |node, kind| NativeStreamObservation {
        node,
        kind,
        timing: None,
        shared_period: None,
        render_queue: None,
        input_queue: None,
        virtual_budget: None,
    };
    snapshot.native_observations = Some(NativeSessionObservations {
        generation: SessionGeneration(8),
        observation: NativeObservationSnapshot {
            virtual_clock: None,
            virtual_callback: None,
            process_delivery: Vec::new(),
            owner_generation: 3,
            running: true,
            streams: vec![
                stream(input, NativeStreamKind::PhysicalInput),
                stream(output, NativeStreamKind::MasterOutput),
            ],
            boundary_probes: Vec::new(),
            bridges: vec![NativeBridgeObservation {
                queue_history: None,
                packet_history: None,
                node: input,
                counters: BridgeSnapshot {
                    fill_frames: 960,
                    last_qpc_100ns: 777,
                    native_delivery: Some(InputDeliverySnapshot {
                        packets: u64::MAX,
                        max_packet_frames: u32::MAX,
                        max_callback_burst_frames: (1 << 53) + 1,
                        gap_samples: (1 << 53) + 3,
                        max_gap_100ns: u64::MAX - 1,
                        packet_bound_violations: (1 << 53) + 5,
                        gap_bound_violations: (1 << 53) + 7,
                        unavailable_clocks: (1 << 53) + 9,
                        regressed_clocks: (1 << 53) + 11,
                    }),
                    oldest_frame_age: Some(QueuedFrameAge {
                        stamp: QueuedFrameStamp {
                            generation: (1 << 53) + 13,
                            packet_position_frames: u64::MAX,
                            packet_qpc_100ns: 1,
                            packet_accepted_frames: 480,
                            packet_offset_frames: 24,
                            rate_correction_ppm: Some(-17),
                        },
                        observed_qpc_100ns: u64::MAX,
                        input_sample_rate: 48_000,
                        estimated_frame_qpc_100ns: 5_001,
                        estimated_age_100ns: u64::MAX - 5_001,
                    }),
                    ..Default::default()
                },
            }],
            render_padding: vec![NativeRenderPaddingObservation {
                node: output,
                sample: Some(RenderPaddingSnapshot {
                    padding_frames: 0,
                    capacity_frames: 960,
                    sample_rate: Some(48_000),
                    period_frames: Some(240),
                    sequence: u64::MAX,
                }),
            }],
        },
    });
    snapshot
}

fn value<'a>(rows: &'a [(String, String)], key: &str) -> &'a str {
    &rows
        .iter()
        .find(|(label, _)| label == key)
        .unwrap_or_else(|| panic!("missing {key}: {rows:?}"))
        .1
}

#[test]
fn native_live_rows_expose_cumulative_virtual_clock_loss_as_engine_frames() {
    use moiren_windows_audio::virtual_owner::{VirtualClockCounters, VirtualClockSnapshot};

    let mut snapshot = live_snapshot();
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .virtual_clock = Some(VirtualClockSnapshot {
        schema_version: 1,
        engine_sample_rate: 48_000,
        engine_block_frames: 64,
        recovery_cap_periods: 1,
        counters: VirtualClockCounters {
            rendered_periods: (1 << 53) + 1,
            rendered_frames: u64::MAX,
            late_wake_bursts: (1 << 53) + 3,
            clamped_periods: (1 << 53) + 5,
            clamped_frames: u64::MAX - 1,
            timeline_resets: (1 << 53) + 7,
            max_backlog_periods: (1 << 53) + 9,
            counters_saturated: true,
        },
    });
    let rows = native_timing_rows(&snapshot);
    assert_eq!(
        value(&rows, "Virtual clock observation"),
        "Schema 1 (owner-local cumulative counters, not latency)"
    );
    assert_eq!(
        value(&rows, "Virtual clock configuration"),
        "48000 Hz Engine; block 64 frames; recovery cap 1 periods"
    );
    assert_eq!(
        value(&rows, "Virtual clock rendered"),
        "9007199254740993 periods / 18446744073709551615 Engine frames"
    );
    assert_eq!(
        value(&rows, "Virtual clock recovery"),
        "Late wakes 9007199254740995; skipped 9007199254740997 periods / 18446744073709551614 Engine frames (not native PCM drops)"
    );
    assert_eq!(
        value(&rows, "Virtual clock timeline"),
        "Resets 9007199254740999; maximum backlog 9007199254741001 periods (before cap)"
    );
    assert_eq!(
        value(&rows, "Virtual clock counter saturation"),
        "Saturated (counts are lower bounds)"
    );
    assert_eq!(
        value(&rows, "Physical end-to-end latency"),
        "Unknown / unqualified"
    );
}

#[test]
fn native_live_rows_keep_absent_or_unsupported_virtual_clock_counters_unknown() {
    use moiren_windows_audio::virtual_owner::{VirtualClockCounters, VirtualClockSnapshot};

    let mut snapshot = live_snapshot();
    let unknown_keys = [
        "Virtual clock configuration",
        "Virtual clock rendered",
        "Virtual clock recovery",
        "Virtual clock timeline",
        "Virtual clock counter saturation",
    ];
    let rows = native_timing_rows(&snapshot);
    assert_eq!(value(&rows, "Virtual clock observation"), "Unknown");
    for key in unknown_keys {
        assert_eq!(value(&rows, key), "Unknown");
    }
    let clock = VirtualClockSnapshot {
        schema_version: 2,
        engine_sample_rate: 48_000,
        engine_block_frames: 64,
        recovery_cap_periods: 1,
        counters: VirtualClockCounters::default(),
    };
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .virtual_clock = Some(clock);
    let rows = native_timing_rows(&snapshot);
    assert_eq!(
        value(&rows, "Virtual clock observation"),
        "Unsupported schema 2"
    );
    for key in unknown_keys {
        assert_eq!(value(&rows, key), "Unknown");
    }
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .virtual_clock = Some(VirtualClockSnapshot {
        schema_version: 1,
        ..clock
    });
    let rows = native_timing_rows(&snapshot);
    assert_eq!(
        value(&rows, "Virtual clock rendered"),
        "0 periods / 0 Engine frames"
    );
    assert_eq!(
        value(&rows, "Virtual clock counter saturation"),
        "Not saturated"
    );
    for (phase, generation, running) in [
        (SessionPhase::Idle, SessionGeneration(8), true),
        (SessionPhase::Running, SessionGeneration(9), true),
        (SessionPhase::Running, SessionGeneration(8), false),
    ] {
        let mut stale = snapshot.clone();
        stale.phase = phase;
        stale.generation = generation;
        stale
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .running = running;
        assert!(
            !native_timing_rows(&stale)
                .iter()
                .any(|(key, _)| key.starts_with("Virtual clock"))
        );
    }
}

#[test]
fn native_live_rows_translate_virtual_clock_loss_and_saturation_limits() {
    let locale = crate::i18n::SIMPLIFIED_CHINESE;
    assert_eq!(
        rust_i18n::t!("Virtual clock recovery", locale = locale),
        "虚拟时钟恢复"
    );
    assert_eq!(
        rust_i18n::t!(
            "Late wakes %{late}; skipped %{periods} periods / %{frames} Engine frames (not native PCM drops)",
            locale = locale,
            late = 3,
            periods = 5,
            frames = 320,
        ),
        "迟到唤醒 3 次；跳过 5 周期 / 320 Engine 帧（不是原生 PCM 丢帧）"
    );
    assert_eq!(
        rust_i18n::t!("Unsupported schema %{schema}", locale = locale, schema = 2),
        "不支持的版本 2"
    );
    assert_eq!(
        rust_i18n::t!("Saturated (counts are lower bounds)", locale = locale),
        "已饱和（计数仅为下界）"
    );
}

#[test]
fn native_live_rows_expose_virtual_whole_burst_budget_without_inventing_negotiation() {
    use moiren_windows_audio::{
        latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
        virtual_owner::VirtualBufferOptions,
    };
    let mut snapshot = live_snapshot();
    let options = VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 4800,
            target_frames: 945,
            jitter_frames: 96,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 480,
            max_gap_frames: 480,
            source: DeliveryTimingSource::Declared,
        }),
        recovery_cap_periods: Some(1),
    };
    let budget = options.prepare_recovery(48_000, 64).unwrap();
    for (index, kind) in [
        NativeStreamKind::VirtualPlayback,
        NativeStreamKind::VirtualMicrophone,
    ]
    .into_iter()
    .enumerate()
    {
        let stream = &mut snapshot
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .streams[index];
        stream.kind = kind;
        stream.virtual_budget = Some(budget);
        stream.input_queue = budget.queue;
    }
    for kind in ["1 Virtual playback", "2 Virtual microphone"] {
        let rows = native_timing_rows(&snapshot);
        assert_eq!(
            value(&rows, &format!("{kind}: virtual Engine demand")),
            "48000 Hz Engine; block 64 / whole burst 64 frames; recovery cap 1 periods"
        );
        assert_eq!(
            value(&rows, &format!("{kind}: virtual native reserve")),
            "Transfer 65 / FIR 32 / wake 96 frames at 48 kHz (modeled, not latency)"
        );
        assert_eq!(
            value(&rows, &format!("{kind}: virtual delivery evidence")),
            "Declared assumption; packet bound 480 / gap bound 480; negotiated packet Unknown frames at 48 kHz"
        );
    }
    let stream = &mut snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .streams[1];
    stream.virtual_budget = Some(budget.admit_native_packet(480).unwrap());
    assert!(
        value(
            &native_timing_rows(&snapshot),
            "2 Virtual microphone: virtual delivery evidence"
        )
        .contains("negotiated packet 480")
    );
    for (phase, generation, running) in [
        (SessionPhase::Idle, SessionGeneration(8), true),
        (SessionPhase::Running, SessionGeneration(9), true),
        (SessionPhase::Running, SessionGeneration(8), false),
    ] {
        let mut stale = snapshot.clone();
        stale.phase = phase;
        stale.generation = generation;
        stale
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .running = running;
        assert!(
            !native_timing_rows(&stale)
                .iter()
                .any(|(key, _)| key.contains("virtual Engine demand"))
        );
    }
}

#[test]
fn native_live_rows_virtual_compatible_keeps_absent_bounds_unknown() {
    let mut snapshot = live_snapshot();
    let stream = &mut snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .streams[0];
    stream.kind = NativeStreamKind::VirtualPlayback;
    stream.virtual_budget = Some(
        moiren_windows_audio::virtual_owner::VirtualBufferOptions::default()
            .prepare_recovery(48_000, 64)
            .unwrap(),
    );
    let rows = native_timing_rows(&snapshot);
    assert_eq!(
        value(&rows, "1 Virtual playback: virtual Engine demand"),
        "48000 Hz Engine; block 64 / whole burst 512 frames; recovery cap 8 periods"
    );
    assert_eq!(
        value(&rows, "1 Virtual playback: virtual native reserve"),
        "Transfer 514 / FIR 32 / wake Unknown frames at 48 kHz (modeled, not latency)"
    );
    assert_eq!(
        value(&rows, "1 Virtual playback: virtual delivery evidence"),
        "Compatible; packet bound Unknown / gap bound Unknown; negotiated packet Unknown frames at 48 kHz"
    );
    assert!(
        !rows
            .iter()
            .any(|(key, _)| key == "2 Master output: virtual Engine demand")
    );
}

#[test]
fn native_live_rows_virtual_measured_bounds_do_not_supply_a_negotiated_packet() {
    use moiren_windows_audio::{
        latency::transport::{DeliveryTimingSource, InputDeliveryTiming},
        virtual_owner::VirtualBufferOptions,
    };
    for (source, evidence) in [
        (DeliveryTimingSource::Empirical, "Empirical observation"),
        (
            DeliveryTimingSource::NativePacket,
            "Native packet observation",
        ),
    ] {
        let mut snapshot = live_snapshot();
        let stream = &mut snapshot
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .streams[0];
        stream.kind = NativeStreamKind::VirtualPlayback;
        stream.virtual_budget = Some(
            VirtualBufferOptions {
                delivery: Some(InputDeliveryTiming {
                    sample_rate: 48_000,
                    max_packet_frames: 480,
                    max_gap_frames: 480,
                    source,
                }),
                ..Default::default()
            }
            .prepare_recovery(48_000, 64)
            .unwrap(),
        );
        assert_eq!(
            value(
                &native_timing_rows(&snapshot),
                "1 Virtual playback: virtual delivery evidence"
            ),
            format!(
                "{evidence}; packet bound 480 / gap bound 480; negotiated packet Unknown frames at 48 kHz"
            )
        );
    }
}

#[test]
fn native_live_rows_translate_virtual_budget_units_and_unknowns() {
    let locale = crate::i18n::SIMPLIFIED_CHINESE;
    for (key, expected) in [
        ("%{prefix}: virtual Engine demand", "输入：虚拟 Engine 需求"),
        ("%{prefix}: virtual native reserve", "输入：虚拟原生储备"),
        ("%{prefix}: virtual delivery evidence", "输入：虚拟交付依据"),
    ] {
        assert_eq!(
            rust_i18n::t!(key, locale = locale, prefix = "输入"),
            expected
        );
    }
    assert_eq!(
        rust_i18n::t!(
            "%{rate} Hz Engine; block %{block} / whole burst %{burst} frames; recovery cap %{cap} periods",
            locale = locale,
            rate = 44100,
            block = 64,
            burst = 512,
            cap = 8,
        ),
        "44100 Hz Engine；块 64 / 整次突发 512 帧；追赶上限 8 周期"
    );
    assert_eq!(
        rust_i18n::t!(
            "Transfer %{transfer} / FIR %{fir} / wake %{wake} frames at 48 kHz (modeled, not latency)",
            locale = locale,
            transfer = 514,
            fir = 32,
            wake = "未知",
        ),
        "传输 514 / FIR 32 / 唤醒 未知 帧（48 kHz 模型值，不是延迟）"
    );
    assert_eq!(
        rust_i18n::t!(
            "%{evidence}; packet bound %{packet} / gap bound %{gap}; negotiated packet %{negotiated} frames at 48 kHz",
            locale = locale,
            evidence = "声明假设",
            packet = 480,
            gap = 480,
            negotiated = "未知",
        ),
        "声明假设；数据包边界 480 / 间隔边界 480；已协商数据包 未知 帧（48 kHz）"
    );
    for (key, expected) in [
        ("Declared assumption", "声明假设"),
        ("Empirical observation", "实测观测"),
        ("Native packet observation", "原生数据包观测"),
    ] {
        assert_eq!(rust_i18n::t!(key, locale = locale), expected);
    }
}

#[test]
fn native_live_rows_expose_admitted_input_reserve_without_inventing_latency() {
    use moiren_windows_audio::latency::PreparedInputQueue;

    let mut snapshot = live_snapshot();
    let queue = &mut snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .streams[0]
        .input_queue;
    *queue = Some(PreparedInputQueue {
        capacity_frames: 16_384,
        prime_frames: 1344,
        servo_reference_frames: 1344,
        low_water_frames: 706,
        high_water_frames: 15_904,
        modeled_minimum_frames: Some(1333),
    });
    let rows = native_timing_rows(&snapshot);
    assert_eq!(
        value(&rows, "1 Physical input: effective prepared bridge queue"),
        "Prime 1344 / capacity 16384 frames"
    );
    assert_eq!(
        value(&rows, "1 Physical input: admitted input reserve"),
        "Minimum 1333; low 706 / high 15904 input frames (modeled reserve, not latency)"
    );
    for (phase, generation, running) in [
        (SessionPhase::Running, SessionGeneration(9), true),
        (SessionPhase::Idle, SessionGeneration(8), true),
        (SessionPhase::Running, SessionGeneration(8), false),
    ] {
        let mut hidden = snapshot.clone();
        hidden.phase = phase;
        hidden.generation = generation;
        hidden
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .running = running;
        assert!(
            !native_timing_rows(&hidden)
                .iter()
                .any(|(label, _)| label.contains("admitted input reserve"))
        );
    }
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .streams[0]
        .input_queue = None;
    assert!(
        !native_timing_rows(&snapshot)
            .iter()
            .any(|(label, _)| label.contains("admitted input reserve"))
    );
}

#[test]
fn native_live_rows_keep_unmodeled_input_minimum_unknown() {
    let mut snapshot = live_snapshot();
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .streams[0]
        .input_queue = Some(moiren_windows_audio::latency::PreparedInputQueue::compatible());
    assert_eq!(
        value(
            &native_timing_rows(&snapshot),
            "1 Physical input: admitted input reserve"
        ),
        "Minimum Unknown; low 2 / high 8191 input frames (modeled reserve, not latency)"
    );
}

#[test]
fn native_live_rows_translate_admitted_input_reserve_limits() {
    let locale = crate::i18n::SIMPLIFIED_CHINESE;
    assert_eq!(
        rust_i18n::t!(
            "%{prefix}: admitted input reserve",
            locale = locale,
            prefix = "输入"
        ),
        "输入：已准入输入储备"
    );
    assert_eq!(
        rust_i18n::t!(
            "Minimum %{minimum}; low %{low} / high %{high} input frames (modeled reserve, not latency)",
            locale = locale,
            minimum = 1333,
            low = 706,
            high = 15904
        ),
        "最低 1333；低水位 706 / 高水位 15904 输入帧（模型储备，不是延迟）"
    );
}

#[test]
fn professional_and_native_diagnostics_coexist_and_filter_stale_publishers_independently() {
    use crate::service::{ProfessionalSessionObservations, SampleFormat};
    use moiren_engine::node::DeviceOptions;
    use moiren_windows_audio::{
        engine_backend::{ProfessionalObservationSnapshot, ProfessionalStreamObservation},
        native::NativeStreamInfo,
    };

    let mut fresh = live_snapshot();
    fresh.professional_observations = Some(ProfessionalSessionObservations {
        generation: fresh.generation,
        observation: ProfessionalObservationSnapshot {
            owner_generation: 3,
            running: true,
            streams: vec![ProfessionalStreamObservation {
                node: fresh.graph.graph.nodes()[1].id(),
                input: false,
                master: true,
                requested: DeviceOptions::default(),
                actual: NativeStreamInfo {
                    sample_rate: 48_000,
                    format: SampleFormat::Float32,
                    buffer_frames: 240,
                    channels: 2,
                },
                dropped_frames: 0,
                silent_frames: 0,
                native_callbacks: 0,
                native_wait_wakes: 0,
                native_wait_timeouts: 0,
                native_written_frames: 0,
                native_dropped_frames: 0,
                native_silent_frames: 0,
                native_capture_quality: None,
                driver_latency: None,
            }],
        },
    });
    let assert_publishers = |snapshot: &AppSnapshot, professional: bool, native: bool| {
        let rows = native_timing_rows(snapshot);
        assert_eq!(
            rows.iter().any(|(key, _)| key.contains(": actual stream")),
            professional,
        );
        assert_eq!(
            rows.iter()
                .any(|(key, _)| key.contains(": native render padding")),
            native,
        );
        assert_eq!(
            crate::ui::presentation::measured_master_output(snapshot),
            professional.then_some((48_000, 240)),
        );
    };
    assert_publishers(&fresh, true, true);
    for stopped in [false, true] {
        let mut snapshot = fresh.clone();
        let status = snapshot.native_observations.as_mut().unwrap();
        if stopped {
            status.observation.running = false;
        } else {
            status.generation = SessionGeneration(9);
        }
        assert_publishers(&snapshot, true, false);

        let mut snapshot = fresh.clone();
        let status = snapshot.professional_observations.as_mut().unwrap();
        if stopped {
            status.observation.running = false;
        } else {
            status.generation = SessionGeneration(9);
        }
        assert_publishers(&snapshot, false, true);
    }
    fresh.phase = SessionPhase::Stopping;
    assert_publishers(&fresh, false, false);
    assert_eq!(
        native_timing_rows(&fresh),
        vec![("Actual native timing".into(), "\u{2014}".into())],
    );
}

#[test]
fn native_live_rows_keep_full_width_delivery_age_provenance_and_padding_distinct() {
    let rows = native_timing_rows(&live_snapshot());
    for (key, expected) in [
        (
            "Stream 1: native delivery packets",
            "18446744073709551615 packets",
        ),
        ("Stream 1: maximum native packet", "4294967295 frames"),
        (
            "Stream 1: maximum callback burst",
            "9007199254740993 frames",
        ),
        (
            "Stream 1: native arrival gaps",
            "9007199254740995 observations; max 18446744073709551614 x 100 ns",
        ),
        (
            "Stream 1: delivery bound violations",
            "Packet 9007199254740997 / gap 9007199254740999 observations",
        ),
        (
            "Stream 1: delivery clock observations",
            "Unavailable 9007199254741001 / regressed 9007199254741003 observations",
        ),
        (
            "Stream 1: oldest pending frame age estimate",
            "18446744073709546614 x 100 ns (owner-local estimate, not physical E2E)",
        ),
        (
            "Stream 1: oldest frame correlation",
            "Generation 9007199254741005; packet position 18446744073709551615 frames; offset 24 / accepted 480 frames",
        ),
        (
            "Stream 1: age QPC correlation",
            "Packet 1 / frame 5001 / observed 18446744073709551615 x 100 ns",
        ),
        (
            "Stream 1: age rate basis",
            "48000 Hz; qualified Capture/QPC correction -17 ppm",
        ),
        (
            "Stream 2: native render padding",
            "0 / capacity 960 frames (native queue only, not E2E)",
        ),
        ("Stream 2: observed render rate", "48000 Hz"),
        ("Stream 2: observed render period", "240 frames"),
        ("Stream 2: render sample sequence", "18446744073709551615"),
        ("Physical end-to-end latency", "Unknown / unqualified"),
    ] {
        assert_eq!(value(&rows, key), expected, "{key}");
    }
}

#[test]
fn native_live_rows_missing_and_zero_delivery_evidence_is_unknown_not_zero_timing() {
    let mut snapshot = live_snapshot();
    for delivery in [
        None,
        Some(InputDeliverySnapshot::default()),
        Some(InputDeliverySnapshot {
            max_packet_frames: 128,
            max_callback_burst_frames: 256,
            gap_samples: 3,
            max_gap_100ns: 123,
            ..Default::default()
        }),
    ] {
        let observation = &mut snapshot.native_observations.as_mut().unwrap().observation;
        observation.bridges[0].counters.native_delivery = delivery;
        observation.bridges[0].counters.oldest_frame_age = None;
        observation.render_padding[0].sample = None;
        let rows = native_timing_rows(&snapshot);
        for key in [
            "Stream 1: maximum native packet",
            "Stream 1: maximum callback burst",
            "Stream 1: oldest pending frame age estimate",
            "Stream 2: native render padding",
            "Stream 2: observed render rate",
            "Stream 2: observed render period",
            "Stream 2: render sample sequence",
        ] {
            assert_eq!(value(&rows, key), "Unknown", "{key}");
        }
        if let Some(delivery) = delivery {
            assert_eq!(
                value(&rows, "Stream 1: native arrival gaps"),
                if delivery.gap_samples == 0 {
                    "0 observations; timing unknown"
                } else {
                    "3 observations; timing unknown"
                }
            );
        } else {
            assert_eq!(value(&rows, "Stream 1: native arrival gaps"), "Unknown");
        }
    }
    let observation = &mut snapshot.native_observations.as_mut().unwrap().observation;
    observation.bridges[0].counters.native_delivery = Some(InputDeliverySnapshot {
        packets: 1,
        max_gap_100ns: 999,
        ..Default::default()
    });
    let rows = native_timing_rows(&snapshot);
    assert_eq!(
        value(&rows, "Stream 1: native arrival gaps"),
        "0 observations; timing unknown"
    );
}

#[test]
fn native_live_rows_map_padding_by_node_and_preserve_observed_nonzero_and_zero_gap() {
    let mut snapshot = live_snapshot();
    let observation = &mut snapshot.native_observations.as_mut().unwrap().observation;
    observation.streams.swap(0, 1);
    let padding = observation.render_padding[0].sample.as_mut().unwrap();
    padding.padding_frames = 336;
    padding.capacity_frames = u32::MAX;
    let delivery = observation.bridges[0]
        .counters
        .native_delivery
        .as_mut()
        .unwrap();
    delivery.gap_samples = 1;
    delivery.max_gap_100ns = 0;
    let rows = native_timing_rows(&snapshot);
    assert_eq!(
        value(&rows, "Stream 1: native render padding"),
        "336 / capacity 4294967295 frames (native queue only, not E2E)"
    );
    assert_eq!(
        value(&rows, "Stream 2: native arrival gaps"),
        "1 observations; max 0 x 100 ns"
    );
    assert_eq!(
        value(&rows, "Physical end-to-end latency"),
        "Unknown / unqualified"
    );
}

#[test]
fn native_live_rows_age_comes_only_from_the_supplied_oldest_pending_frame() {
    let mut snapshot = live_snapshot();
    let original = native_timing_rows(&snapshot);
    let counters = &mut snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .bridges[0]
        .counters;
    counters.fill_frames = usize::MAX;
    counters.last_qpc_100ns = u64::MAX;
    let changed = native_timing_rows(&snapshot);
    for key in [
        "Stream 1: oldest pending frame age estimate",
        "Stream 1: oldest frame correlation",
        "Stream 1: age QPC correlation",
        "Stream 1: age rate basis",
    ] {
        assert_eq!(value(&changed, key), value(&original, key));
    }
    let counters = &mut snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .bridges[0]
        .counters;
    counters
        .oldest_frame_age
        .as_mut()
        .unwrap()
        .stamp
        .rate_correction_ppm = None;
    assert_eq!(
        value(&native_timing_rows(&snapshot), "Stream 1: age rate basis"),
        "48000 Hz nominal rate (Capture/QPC correction unqualified)"
    );
    // Reset/empty/invalid upstream states revoke the optional DTO rather than
    // authorizing presentation to derive a replacement from fill or last QPC.
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .bridges[0]
        .counters
        .oldest_frame_age = None;
    assert_eq!(
        value(
            &native_timing_rows(&snapshot),
            "Stream 1: oldest pending frame age estimate"
        ),
        "Unknown"
    );
}

#[test]
fn native_live_rows_preserve_valid_zero_age_and_unknown_render_metadata() {
    let mut snapshot = live_snapshot();
    let observation = &mut snapshot.native_observations.as_mut().unwrap().observation;
    let age = observation.bridges[0]
        .counters
        .oldest_frame_age
        .as_mut()
        .unwrap();
    age.estimated_age_100ns = 0;
    age.observed_qpc_100ns = age.estimated_frame_qpc_100ns;
    let padding = observation.render_padding[0].sample.as_mut().unwrap();
    padding.sample_rate = None;
    padding.period_frames = None;
    let rows = native_timing_rows(&snapshot);
    assert_eq!(
        value(&rows, "Stream 1: oldest pending frame age estimate"),
        "0 x 100 ns (owner-local estimate, not physical E2E)"
    );
    assert_eq!(value(&rows, "Stream 2: observed render rate"), "Unknown");
    assert_eq!(value(&rows, "Stream 2: observed render period"), "Unknown");
    assert_eq!(
        value(&rows, "Stream 2: native render padding"),
        "0 / capacity 960 frames (native queue only, not E2E)"
    );
}

#[test]
fn native_live_rows_remain_inside_phase_generation_and_running_filter() {
    let fresh = live_snapshot();
    let unavailable = vec![("Actual native timing".into(), "\u{2014}".into())];
    for phase in [
        SessionPhase::Idle,
        SessionPhase::Starting,
        SessionPhase::Stopping,
        SessionPhase::Failed,
        SessionPhase::Exiting,
        SessionPhase::Exited,
    ] {
        let mut snapshot = fresh.clone();
        snapshot.phase = phase;
        assert_eq!(native_timing_rows(&snapshot), unavailable, "{phase:?}");
    }
    let mut snapshot = fresh.clone();
    snapshot.generation = SessionGeneration(9);
    assert_eq!(native_timing_rows(&snapshot), unavailable);
    let mut snapshot = fresh;
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .running = false;
    assert_eq!(native_timing_rows(&snapshot), unavailable);
}

#[test]
fn native_live_rows_label_unmapped_native_samples_without_using_another_node() {
    let mut snapshot = live_snapshot();
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .streams
        .clear();
    let rows = native_timing_rows(&snapshot);
    assert_eq!(
        value(&rows, "Unmapped stream: native render padding"),
        "0 / capacity 960 frames (native queue only, not E2E)"
    );
    assert_eq!(
        value(&rows, "Unmapped stream: maximum native packet"),
        "4294967295 frames"
    );
}

#[test]
fn native_live_rows_translate_units_provenance_and_measurement_limits() {
    let locale = crate::i18n::SIMPLIFIED_CHINESE;
    for (key, expected) in [
        ("%{prefix}: native delivery packets", "输入：原生交付数据包"),
        (
            "%{prefix}: maximum native packet",
            "输入：原生数据包最大帧数",
        ),
        (
            "%{prefix}: maximum callback burst",
            "输入：回调突发最大帧数",
        ),
        ("%{prefix}: native arrival gaps", "输入：原生到达间隔"),
        ("%{prefix}: delivery bound violations", "输入：交付边界超限"),
        (
            "%{prefix}: delivery clock observations",
            "输入：交付时钟观测",
        ),
        (
            "%{prefix}: oldest pending frame age estimate",
            "输入：最早待处理帧的年龄估计",
        ),
        (
            "%{prefix}: oldest frame correlation",
            "输入：最早帧的关联信息",
        ),
        ("%{prefix}: age QPC correlation", "输入：帧年龄的 QPC 关联"),
        ("%{prefix}: age rate basis", "输入：帧年龄的采样率依据"),
        ("%{prefix}: native render padding", "输入：原生输出填充量"),
        (
            "%{prefix}: observed render rate",
            "输入：观测到的输出采样率",
        ),
        (
            "%{prefix}: observed render period",
            "输入：观测到的输出周期",
        ),
        ("%{prefix}: render sample sequence", "输入：输出观测序号"),
    ] {
        assert_eq!(
            rust_i18n::t!(key, locale = locale, prefix = "输入"),
            expected
        );
    }
    assert_eq!(
        rust_i18n::t!(
            "%{ticks} x 100 ns (owner-local estimate, not physical E2E)",
            locale = locale,
            ticks = u64::MAX
        ),
        "18446744073709551615 x 100 ns（所有者局部估计，非物理端到端延迟）"
    );
    assert_eq!(
        rust_i18n::t!(
            "%{padding} / capacity %{capacity} frames (native queue only, not E2E)",
            locale = locale,
            padding = 0,
            capacity = 960
        ),
        "0 / 容量 960 帧（仅为原生队列，非端到端延迟）"
    );
    assert_eq!(
        rust_i18n::t!(
            "Packet %{packet} / gap %{gap} observations",
            locale = locale,
            packet = u64::MAX,
            gap = 7
        ),
        "数据包 18446744073709551615 / 间隔 7 次观测"
    );
    assert_eq!(
        rust_i18n::t!(
            "Unavailable %{unavailable} / regressed %{regressed} observations",
            locale = locale,
            unavailable = 3,
            regressed = u64::MAX
        ),
        "不可用 3 / 回退 18446744073709551615 次观测"
    );
    assert_eq!(
        rust_i18n::t!(
            "Generation %{generation}; packet position %{position} frames; offset %{offset} / accepted %{accepted} frames",
            locale = locale,
            generation = 7,
            position = u64::MAX,
            offset = 24,
            accepted = 480
        ),
        "代次 7；数据包位置 18446744073709551615 帧；偏移 24 / 已接收 480 帧"
    );
    assert_eq!(
        rust_i18n::t!(
            "Packet %{packet} / frame %{frame} / observed %{observed} x 100 ns",
            locale = locale,
            packet = 1,
            frame = 5001,
            observed = u64::MAX
        ),
        "数据包 1 / 帧 5001 / 观测时刻 18446744073709551615 x 100 ns"
    );
    assert_eq!(
        rust_i18n::t!(
            "%{rate} Hz; qualified Capture/QPC correction %{ppm} ppm",
            locale = locale,
            rate = 48000,
            ppm = -17
        ),
        "48000 Hz；已验证的 Capture/QPC 校正 -17 ppm"
    );
    assert_eq!(
        rust_i18n::t!(
            "%{rate} Hz nominal rate (Capture/QPC correction unqualified)",
            locale = locale,
            rate = 48000
        ),
        "48000 Hz 标称采样率（Capture/QPC 校正未经验证）"
    );
    assert_eq!(
        rust_i18n::t!(
            "%{count} observations; timing unknown",
            locale = locale,
            count = 3
        ),
        "3 次观测；时序未知"
    );
    assert_eq!(
        rust_i18n::t!("%{rate} Hz", locale = locale, rate = 48000),
        "48000 Hz"
    );
}
