use super::*;

#[cfg(windows)]
#[test]
fn joined_tally_retains_real_backend_handles_after_live_registry_clear_and_replacement() {
    use moiren_engine::boundary::{AudioReader, audio_bridge};
    use moiren_windows_audio::{
        engine_backend::ProfessionalBackend,
        virtual_owner::{
            BoundaryProbe, PreparedVirtualMicrophone, PreparedVirtualPlayback,
            VirtualBoundaryProvider, VirtualBoundaryRequest, VirtualResourceSession,
        },
    };
    use std::sync::{Mutex, atomic::AtomicU64};

    #[derive(Default)]
    struct Provider(Mutex<Vec<AudioReader<f32>>>);
    struct Session;
    impl VirtualResourceSession for Session {
        fn activate(&self) -> Result<(), host::BackendError> {
            Ok(())
        }
        fn cancel(&self) {}
        fn status(&self) -> host::OwnerStatus {
            host::OwnerStatus::Running
        }
        fn stop_and_join(self: Box<Self>) -> Result<(), host::BackendError> {
            Ok(())
        }
    }
    impl VirtualBoundaryProvider for Provider {
        fn prepare_playback(
            &self,
            _: VirtualBoundaryRequest,
            _: &host::CancellationToken,
        ) -> Result<PreparedVirtualPlayback, host::BackendError> {
            unreachable!()
        }
        fn prepare_microphone(
            &self,
            _: VirtualBoundaryRequest,
            _: &host::CancellationToken,
        ) -> Result<PreparedVirtualMicrophone, host::BackendError> {
            let (writer, reader) = audio_bridge(2, 16384, 1 << 20).unwrap();
            self.0.lock().unwrap().push(reader);
            Ok(PreparedVirtualMicrophone {
                carrier: writer,
                reset_epoch: Arc::new(AtomicU64::new(0)),
                producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
                consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
                probe: Arc::new(BoundaryProbe::default()),
                session: Box::new(Session),
            })
        }
    }
    let mut rig = Rig::new();
    let native = Arc::new(Mutex::new(None));
    let professional = Arc::new(Mutex::new(None));
    let native_publisher = native.clone();
    let professional_publisher = professional.clone();
    rig.driver.factory = Arc::new(move |_, _, _, _| {
        let backend = ProfessionalBackend::<f32>::new()
            .unwrap()
            .with_virtual_provider(Arc::new(Provider::default()));
        *native_publisher.lock().unwrap() = Some(backend.native_observations());
        *professional_publisher.lock().unwrap() = Some(backend.observations());
        Ok(Box::new(backend))
    });
    rig.driver.native_observations = Some(native);
    rig.driver.professional_observations = Some(professional);
    rig.driver.exports_runtime.adapter = Some(Arc::new(
        super::super::super::exports::runtime::tests::FakeAdapter::default(),
    ));
    let sink = rig.node(NodeKind::Sink);
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(vec![
        ExportIntent {
            endpoint_id: moiren_engine::node::VirtualEndpointId::from_bytes([0x71; 16]),
            direction: ExportDirection::Microphone,
            name: "Joined tally test".into(),
            channels: 2,
            enabled: true,
            binding: Some(sink),
        },
    ])));
    rig.start();
    rig.wait(|r| {
        r.c.native_observations
            .as_ref()
            .is_some_and(|v| v.observation.virtual_clock.is_some())
    });
    let live = rig
        .c
        .native_observations
        .as_ref()
        .unwrap()
        .observation
        .clone();
    let native_handle = rig
        .driver
        .native_observations
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .retain_session()
        .unwrap();
    let backend_handle = rig
        .driver
        .professional_observations
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .retain_session()
        .unwrap();
    let first = rig.c.core.generation();
    rig.request(AppRequest::Stop);
    rig.wait(|r| r.driver.cleaning == 0);
    let cleanup = rig.c.last_cleanup.as_ref().unwrap();
    assert_eq!(cleanup.generation, first);
    assert_eq!(cleanup.state.host, host::HostState::Closed);
    let joined = cleanup
        .final_observations
        .as_ref()
        .expect("real post-join counters")
        .clone();
    let final_native = joined.native.as_ref().unwrap();
    assert!(!final_native.running);
    assert_eq!(
        final_native.owner_generation,
        cleanup.state.owner_generation
    );
    assert_eq!(*final_native, native_handle.snapshot());
    assert_eq!(joined.backend.as_ref().unwrap(), &backend_handle.snapshot());
    assert!(
        final_native.virtual_clock.unwrap().counters.rendered_frames
            >= live.virtual_clock.unwrap().counters.rendered_frames
    );
    assert!(!joined.backend.as_ref().unwrap().running);
    assert!(rig.c.native_observations.is_none());
    assert!(rig.c.professional_observations.is_none());
    rig.start();
    assert_ne!(rig.c.core.generation(), first);
    assert_eq!(
        rig.c
            .last_cleanup
            .as_ref()
            .unwrap()
            .final_observations
            .as_ref(),
        Some(&joined)
    );
    rig.request(AppRequest::Stop);
    rig.wait(|r| r.driver.cleaning == 0);
    assert_ne!(rig.c.last_cleanup.as_ref().unwrap().generation, first);
    assert!(!Arc::ptr_eq(
        rig.c
            .last_cleanup
            .as_ref()
            .unwrap()
            .final_observations
            .as_ref()
            .unwrap(),
        &joined
    ));
}

fn expert() -> TransportLatencySettings {
    TransportLatencySettings {
        process: Some(ExpertInputBuffer::default()),
        vad: Some(ExpertInputBuffer::default()),
        vad_recovery_cap_periods: Some(1),
        ..Default::default()
    }
}

#[test]
fn reaped_owner_preserves_final_closed_state_and_cleanup_failure() {
    let mut rig = Rig::new();
    rig.start();
    let generation = rig.c.core.generation();
    rig.state.fail_stop.store(true, Ordering::SeqCst);
    rig.request(AppRequest::Stop);
    rig.wait(|rig| rig.driver.is_drained());
    let cleanup = rig.c.last_cleanup.as_ref().expect("joined-owner receipt");
    assert_eq!(cleanup.generation, generation);
    assert_eq!(cleanup.state.host, host::HostState::Closed);
    assert!(
        cleanup
            .state
            .cleanup_error
            .as_ref()
            .is_some_and(|error| error.to_string().contains("injected stop failure"))
    );
    let published = crate::service::snapshot(&rig.c);
    assert_eq!(published.last_cleanup.unwrap().generation, generation);
    assert!(published.active_audio_settings.is_none());
}

#[test]
fn transport_draft_requires_apply_and_replacement_reaps_previous_owner() {
    let mut r = Rig::new();
    r.start();
    let defaults = TransportLatencySettings::default();
    let first = r.c.core.generation();
    assert_eq!(r.c.active_transport_latency, Some(defaults));
    let settings = expert();
    r.request(AppRequest::SetTransportLatency(settings));
    r.tick();
    assert_eq!(r.c.transport_latency, settings);
    assert_eq!(r.c.active_transport_latency, Some(defaults));
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
    r.request(AppRequest::ApplyGraph);
    r.tick();
    assert_eq!(r.c.active_transport_latency, None);
    r.wait(|r| r.c.active_transport_latency == Some(settings));
    assert_ne!(r.c.core.generation(), first);
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
    r.request(AppRequest::Stop);
    assert_eq!(r.c.active_transport_latency, None);
    r.wait(|r| r.driver.is_drained());
}

#[test]
fn transport_draft_after_apply_does_not_change_pending_replacement() {
    let mut r = Rig::new();
    r.start();
    let applied = expert();
    let mut draft = applied;
    draft.process.as_mut().unwrap().target_frames = 1024;
    draft.vad_recovery_cap_periods = Some(2);
    r.request(AppRequest::SetTransportLatency(applied));
    r.request(AppRequest::ApplyGraph);
    r.request(AppRequest::SetTransportLatency(draft));
    r.wait(|r| r.c.active_transport_latency == Some(applied));
    assert_eq!(r.c.transport_latency, draft);
    for _ in 0..5 {
        r.tick();
    }
    assert_eq!(r.c.active_transport_latency, Some(applied));
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
}

#[test]
fn invalid_transport_settings_preserve_requested_and_running_configuration() {
    let mut r = Rig::new();
    r.start();
    let mut invalid = expert();
    invalid.vad.as_mut().unwrap().target_frames = u32::MAX;
    r.request(AppRequest::SetTransportLatency(invalid));
    assert_eq!(r.c.transport_latency, TransportLatencySettings::default());
    assert_eq!(
        r.c.active_transport_latency,
        Some(TransportLatencySettings::default())
    );
    assert!(r.c.graph.snapshot().last_error.is_some());
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
    assert_eq!(r.c.core.phase(), SessionPhase::Running);
}

#[test]
fn invalid_vad_recovery_caps_preserve_requested_and_running_configuration() {
    let mut r = Rig::new();
    r.start();
    let defaults = TransportLatencySettings::default();
    for cap in [0, 9, u64::MAX] {
        r.request(AppRequest::SetTransportLatency(TransportLatencySettings {
            vad_recovery_cap_periods: Some(cap),
            ..defaults
        }));
        assert_eq!(r.c.transport_latency, defaults);
        assert_eq!(r.c.active_transport_latency, Some(defaults));
        assert!(r.c.graph.snapshot().last_error.is_some());
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
        assert_eq!(r.c.core.phase(), SessionPhase::Running);
    }
}

#[test]
fn both_precision_factories_receive_the_applied_transport_snapshot() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let mut r = Rig::new();
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let f32_observed = observed.clone();
        let f64_observed = observed.clone();
        let f32_backend = FakeBackend(r.state.clone());
        let f64_backend = f32_backend.clone();
        r.driver.factory = Arc::new(move |_, _, transport, _| {
            #[cfg(windows)]
            assert_eq!(
                transport
                    .virtual_options()
                    .prepare_recovery(48_000, 64)
                    .unwrap()
                    .recovery_cap_periods,
                1
            );
            f32_observed
                .lock()
                .unwrap()
                .push((ProcessingPrecision::F32, *transport));
            Ok(Box::new(f32_backend.clone()))
        });
        r.driver.f64_factory = Some(Arc::new(move |_, _, transport, _| {
            #[cfg(windows)]
            assert_eq!(
                transport
                    .virtual_options()
                    .prepare_recovery(48_000, 64)
                    .unwrap()
                    .recovery_cap_periods,
                1
            );
            f64_observed
                .lock()
                .unwrap()
                .push((ProcessingPrecision::F64, *transport));
            Ok(Box::new(f64_backend.clone()))
        }));
        let settings = expert();
        let mut audio = AudioSettings::default();
        audio.processing.precision = precision;
        r.request(AppRequest::SetAudioSettings(audio));
        r.request(AppRequest::SetTransportLatency(settings));
        r.request(AppRequest::EditGraph(GraphEdit::BindOutput {
            node: r.node(NodeKind::Sink),
            endpoint_id: "output".into(),
        }));
        r.request(AppRequest::ApplyGraph);
        r.wait(|r| r.c.active_transport_latency == Some(settings));
        assert_eq!(*observed.lock().unwrap(), vec![(precision, settings)]);
        assert_eq!(
            r.c.active_audio_settings
                .as_ref()
                .unwrap()
                .processing
                .precision,
            precision
        );
    }
}

#[cfg(windows)]
#[test]
fn native_buffer_preparation_refusal_keeps_requested_values_without_active_claim() {
    let mut r = Rig::new();
    r.start();
    r.driver.factory = Arc::new(move |_, _, settings, _| {
        settings
            .virtual_options()
            .validate()
            .map_err(|error| host::BackendError::Unsupported(error.to_string()))?;
        unreachable!("the native FIR preflight must refuse this target")
    });
    let requested = TransportLatencySettings {
        vad: Some(ExpertInputBuffer {
            capacity_frames: 512,
            target_frames: 40,
            jitter_frames: 0,
            max_packet_frames: 32,
            max_gap_frames: 32,
            ..Default::default()
        }),
        ..Default::default()
    };
    requested.validate().unwrap();
    r.request(AppRequest::SetTransportLatency(requested));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.core.phase() == SessionPhase::Failed);
    assert_eq!(r.c.transport_latency, requested);
    assert_eq!(r.c.active_transport_latency, None);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
    assert!(
        r.c.graph
            .snapshot()
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("input") || error.contains("budget"))
    );
}

#[cfg(windows)]
#[test]
fn process_age_refusal_preserves_desired_intent_without_fallback_for_both_precisions() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let mut r = Rig::new();
        r.start();
        r.driver.factory = Arc::new(|_, _, settings, _| {
            settings
                .process_options()
                .prepare()
                .map_err(|error| host::BackendError::Unsupported(error.to_string()))?;
            unreachable!("unsupported Process sample age must fail before opening a backend")
        });
        r.driver.f64_factory = Some(Arc::new(|_, _, settings, _| {
            settings
                .process_options()
                .prepare()
                .map_err(|error| host::BackendError::Unsupported(error.to_string()))?;
            unreachable!("unsupported Process sample age must fail before opening a backend")
        }));
        let mut requested = serde_json::to_value(expert()).unwrap();
        requested["process"]["age_limit_100ns"] = serde_json::json!(100_000);
        let requested: TransportLatencySettings = serde_json::from_value(requested).unwrap();
        let mut audio = AudioSettings::default();
        audio.processing.precision = precision;
        r.request(AppRequest::SetAudioSettings(audio));
        r.request(AppRequest::SetTransportLatency(requested));
        assert_eq!(r.c.transport_latency, requested);
        assert_eq!(
            r.c.active_transport_latency,
            Some(TransportLatencySettings::default())
        );
        r.request(AppRequest::ApplyGraph);
        r.wait(|r| r.c.core.phase() == SessionPhase::Failed);
        assert_eq!(r.c.transport_latency, requested);
        assert_eq!(r.c.active_transport_latency, None);
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
        assert!(
            r.c.graph
                .snapshot()
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("sample-age enforcement unsupported"))
        );
    }
}
