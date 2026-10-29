//! Real bridge replay with a private software control fixture. Native origin
//! is simulated only inside this cfg(test) seam; it is not native IO evidence.
use super::*;
use crate::service::{
    MeasuredProcessRequest, ProcessWindowPolicy,
    managed::process_traces::{Collection, SessionTrace},
    measured_process::live::{LiveProcessEvaluation, WindowEvaluation},
};
use moiren_windows_audio::{
    clock_bridge::{CaptureIngress, CapturePacket, ClockBridgeConfig, ClockSource, capture_bridge},
    latency::transport::TransportInputPolicy,
    process_loopback::{ObservedProcessTrace, ProcessTraceArchive, ReplayWindowPolicy},
    stats::SILENT,
};

fn policy() -> ProcessWindowPolicy {
    ProcessWindowPolicy {
        max_initial_unserved_frames: 1000,
        max_initial_prime_discarded_frames: 1000,
        min_output_frames: 96,
        min_output_reads: 2,
        min_output_span_100ns: 1,
    }
}

struct SoftwareWindow {
    archive: ProcessTraceArchive,
    _ingress: CaptureIngress,
    _source: ClockSource,
    retained: bool,
    measurement: moiren_windows_audio::process_loopback::MeasuredProcessDelivery,
    gate: Arc<AtomicBool>,
}

impl Collection for SoftwareWindow {
    fn poll(&mut self) {}
    fn artifact(&self) -> ProcessTraceArchive {
        self.archive.clone()
    }
    fn ordered_trace(&self) -> Result<ObservedProcessTrace, String> {
        self.archive
            .trace(&self.archive.process, self.archive.owner_generation)
            .map_err(|error| error.to_string())
    }
    fn prepare_live_window(
        &mut self,
        input: TransportInputPolicy,
        policy: ReplayWindowPolicy,
    ) -> Result<WindowEvaluation, String> {
        let archive = self.archive.clone();
        let measurement = self.measurement.clone();
        let options = measurement
            .admit_for_consumer(
                &archive.process,
                archive.owner_generation,
                input,
                48_000,
                96,
            )
            .map_err(|error| error.to_string())?;
        let already_issued = self.retained;
        let gate = self.gate.clone();
        Ok(Box::new(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !gate.load(Ordering::Acquire) {
                if Instant::now() >= deadline {
                    return Err("software window test gate timed out".into());
                }
                std::thread::park_timeout(Duration::from_millis(1));
            }
            let report = archive
                .replay_live_window(&archive.process, archive.owner_generation, options, policy)
                .map_err(|error| error.to_string())?;
            Ok(LiveProcessEvaluation::software(
                measurement,
                report,
                already_issued,
            ))
        }))
    }
}

fn install(
    r: &mut Rig,
    request: MeasuredProcessRequest,
    starve: bool,
    retained: bool,
) -> Arc<AtomicBool> {
    let owner = r.driver.owner.as_ref().unwrap();
    let host = owner.observer.read();
    let origin = r.driver.process_trace_origin(&r.c, &host).unwrap().unwrap();
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        target_fill_frames: 256,
        trim_on_prime: false,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    for index in 0..12 {
        let stamp = 1_000_000 + index * 20_000;
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames: 96,
                    flags: SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(stamp),
            )
            .unwrap();
        source
            .read_interleaved_with_publication_clock(&mut [0.; 192], || Some(stamp + 100))
            .unwrap();
        if starve && index > 3 {
            for offset in 1..12 {
                source
                    .read_interleaved_with_publication_clock(&mut [0.; 192], || {
                        Some(stamp + 100 + offset * 100)
                    })
                    .unwrap();
            }
        }
    }
    let archive = ProcessTraceArchive {
        schema_version: 2,
        process: origin.process.clone(),
        owner_generation: request.native_owner_generation,
        input_sample_rate: 48_000,
        consumer_sample_rate: 48_000,
        prepared_consumer_demand_frames: 96,
        packet_history: observer.packet_history().unwrap(),
        queue_history: observer.queue_history().unwrap(),
        collection_refusal: None,
    };
    let measurement =
        r.c.native_observations
            .as_ref()
            .unwrap()
            .observation
            .process_delivery[0]
            .measured(&origin.process, request.native_owner_generation)
            .unwrap();
    let gate = Arc::new(AtomicBool::new(true));
    r.driver.process_trace = Some(SessionTrace::new(
        origin,
        Box::new(SoftwareWindow {
            archive,
            _ingress: ingress,
            _source: source,
            retained,
            measurement,
            gate: gate.clone(),
        }),
    ));
    gate
}

#[test]
fn windowed_superseding_requests_invalidate_inflight_evaluation_without_retiring_owner() {
    for change in 0..3 {
        let mut r = Rig::new();
        let request = measured_tests::start_process(&mut r);
        let gate = install(&mut r, request, false, true);
        gate.store(false, Ordering::Release);
        r.request(AppRequest::ApplyWindowedProcessBridge {
            request,
            policy: policy(),
        });
        assert!(r.driver.window_evaluation.is_some());
        match change {
            0 => r.request(AppRequest::SetTransportLatency(Default::default())),
            1 => r.request(AppRequest::SetAudioSettings(AudioSettings::default())),
            2 => r.request(AppRequest::Stop),
            _ => unreachable!(),
        }
        gate.store(true, Ordering::Release);
        r.wait(|r| r.pool.is_idle());
        r.tick();
        assert!(r.c.measured_process.is_none());
        assert_eq!(r.driver.measured_selection_id, 0);
        assert_eq!(
            r.state.stops.load(Ordering::SeqCst),
            usize::from(change == 2)
        );
        if change != 2 {
            r.request(AppRequest::Stop);
        }
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn windowed_worker_backpressure_preserves_the_original_owner_and_selection() {
    let mut r = Rig::new();
    let request = measured_tests::start_process(&mut r);
    install(&mut r, request, false, true);
    let gate = Arc::new(AtomicBool::new(false));
    let held = gate.clone();
    r.pool
        .try_submit(
            JobPriority::Prepare,
            Box::new(move || {
                while !held.load(Ordering::Acquire) {
                    std::thread::park_timeout(Duration::from_millis(1));
                }
            }),
        )
        .unwrap();
    r.request(AppRequest::ApplyWindowedProcessBridge {
        request,
        policy: policy(),
    });
    gate.store(true, Ordering::Release);
    assert!(r.driver.window_evaluation.is_none());
    assert!(r.c.measured_process.is_none());
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
    assert!(
        r.c.graph
            .snapshot()
            .last_error
            .unwrap()
            .contains("worker unavailable")
    );
    r.request(AppRequest::Stop);
    r.wait(|r| r.driver.is_drained());
}

#[test]
fn windowed_collection_nonce_and_origin_changes_discard_inflight_result() {
    for fault in 0..3 {
        let mut r = Rig::new();
        let request = measured_tests::start_process(&mut r);
        let gate = install(&mut r, request, false, true);
        gate.store(false, Ordering::Release);
        r.request(AppRequest::ApplyWindowedProcessBridge {
            request,
            policy: policy(),
        });
        match fault {
            0 => r.driver.process_trace.as_mut().unwrap().instance = Arc::new(()),
            1 => r.driver.document += 1,
            2 => r.driver.owner.as_mut().unwrap().active_revision.0 += 1,
            _ => unreachable!(),
        }
        gate.store(true, Ordering::Release);
        r.wait(|r| r.driver.window_evaluation.is_none());
        assert!(r.c.measured_process.is_none());
        assert_eq!(r.driver.measured_selection_id, 0);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
        assert!(r.c.graph.snapshot().last_error.is_some());
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn windowed_invalid_policy_and_ingress_intent_refuse_without_scheduling() {
    for fault in 0..4 {
        let mut r = Rig::new();
        let request = measured_tests::start_process(&mut r);
        install(&mut r, request, false, true);
        let mut policy = policy();
        match fault {
            0 => policy.min_output_frames = 0,
            1 => policy.min_output_reads = 1,
            2 => policy.min_output_span_100ns = 0,
            3 => r.c.transport_latency.process_max_ingress_elapsed_100ns = Some(1),
            _ => unreachable!(),
        }
        r.request(AppRequest::ApplyWindowedProcessBridge { request, policy });
        assert!(r.driver.window_evaluation.is_none());
        assert!(r.c.measured_process.is_none());
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
        assert!(r.c.graph.snapshot().last_error.is_some());
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

fn complete_selection(r: &mut Rig) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !r.pool.is_idle() {
        assert!(Instant::now() < deadline, "window worker did not finish");
        std::thread::park_timeout(Duration::from_millis(1));
    }
    r.driver.poll_completions(&mut r.c, RoundBudget::default());
    assert!(r.driver.apply);
    assert!(
        r.c.measured_process
            .as_ref()
            .unwrap()
            .summary()
            .observed_window
            .is_some()
    );
}

#[test]
fn windowed_origin_graph_settings_and_source_are_fenced_before_retirement() {
    for fault in 0..5 {
        let mut r = Rig::new();
        let request = measured_tests::start_process(&mut r);
        install(&mut r, request, false, true);
        r.request(AppRequest::ApplyWindowedProcessBridge {
            request,
            policy: policy(),
        });
        complete_selection(&mut r);
        match fault {
            0 => r.driver.document += 1,
            1 => r.c.audio_settings.output.buffer_frames = Some(480),
            2 => r.c.transport_latency.process_max_ingress_elapsed_100ns = Some(1000),
            3 => {
                r.request(AppRequest::EditGraph(GraphEdit::AddNode {
                    kind: NodeKind::Bus,
                    channels: 2,
                }));
            }
            4 => r.driver.owner.as_mut().unwrap().active_revision.0 += 1,
            _ => unreachable!(),
        }
        r.tick();
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 0, "fault {fault}");
        assert!(r.c.graph.snapshot().last_error.is_some(), "fault {fault}");
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn windowed_captured_settings_and_joined_predecessor_are_fenced_before_open() {
    for fault in 0..4 {
        let mut r = Rig::new();
        let request = measured_tests::start_process(&mut r);
        install(&mut r, request, false, true);
        r.request(AppRequest::ApplyWindowedProcessBridge {
            request,
            policy: policy(),
        });
        complete_selection(&mut r);
        r.driver.apply(&mut r.c);
        assert!(r.driver.owner.is_none());
        let deadline = Instant::now() + Duration::from_secs(5);
        while r.driver.cleaning != 0 {
            assert!(Instant::now() < deadline);
            r.c.dispatch(&mut r.pool);
            r.driver.poll_completions(&mut r.c, RoundBudget::default());
            std::thread::park_timeout(Duration::from_millis(1));
        }
        match fault {
            0 => r.c.last_cleanup.as_mut().unwrap().generation.0 += 1,
            1 => r.c.last_cleanup.as_mut().unwrap().state.owner_generation += 1,
            2 => r.c.last_cleanup.as_mut().unwrap().state.desired_revision += 1,
            3 => {
                // A superseding Apply captured different settings. Restoring
                // the current draft must not repair that captured request.
                r.driver
                    .apply_settings
                    .as_mut()
                    .unwrap()
                    .audio
                    .output
                    .buffer_frames = Some(480);
            }
            _ => unreachable!(),
        }
        r.driver.apply(&mut r.c);
        assert!(r.driver.owner.is_none(), "fault {fault}");
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 1, "fault {fault}");
        assert!(r.c.graph.snapshot().last_error.is_some(), "fault {fault}");
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn windowed_missing_or_caller_collection_preserves_live_owner() {
    for caller in [false, true] {
        let mut r = Rig::new();
        let request = measured_tests::start_process(&mut r);
        if caller {
            install(&mut r, request, false, false);
        }
        r.request(AppRequest::ApplyWindowedProcessBridge {
            request,
            policy: policy(),
        });
        r.wait(|r| r.driver.window_evaluation.is_none());
        assert!(r.c.measured_process.is_none());
        assert_eq!(r.driver.measured_selection_id, 0);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
        assert!(r.c.graph.snapshot().last_error.is_some());
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn windowed_density_loss_and_insufficient_exposure_refuse_before_replacement() {
    for starve in [false, true] {
        let mut r = Rig::new();
        let request = measured_tests::start_process(&mut r);
        install(&mut r, request, starve, true);
        let mut policy = policy();
        policy.min_output_frames = u64::MAX;
        r.request(AppRequest::ApplyWindowedProcessBridge { request, policy });
        r.wait(|r| r.driver.window_evaluation.is_none());
        assert!(r.c.measured_process.is_none());
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
        let error = r.c.graph.snapshot().last_error.unwrap();
        assert!(
            error.contains(if starve {
                "ObservedViolation"
            } else {
                "Inconclusive"
            }),
            "{error}"
        );
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn windowed_opt_in_replaces_owner_in_both_precisions_with_bound_policy() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let mut r = Rig::new();
        let mut audio = AudioSettings::default();
        audio.processing.precision = precision;
        r.request(AppRequest::SetAudioSettings(audio));
        r.driver
            .set_measured_test_factory(FakeBackend(r.state.clone()));
        let request = measured_tests::start_process(&mut r);
        install(&mut r, request, false, true);
        r.request(AppRequest::ApplyWindowedProcessBridge {
            request,
            policy: policy(),
        });
        r.wait(|r| r.driver.window_evaluation.is_none());
        let desired = r.c.measured_process.as_ref().unwrap().summary().clone();
        let window = desired.observed_window.as_ref().unwrap();
        assert_eq!(window.policy, policy());
        assert!(window.replayed_packet_events > 0 && window.replayed_read_events > 0);
        assert!(r.c.active_measured_process.is_none());
        r.wait(|r| r.c.active_measured_process.as_ref() == Some(&desired));
        assert_ne!(r.c.core.generation(), request.session_generation);
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}
