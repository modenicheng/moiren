//! Unit tests for the UI state projection, split out of `state.rs`
//! (the `transport_tests` module is the earlier precedent).

use super::*;
use crate::service::{ProcessRow, ServiceMailbox};
use crate::ui::{finish_service, initial_projection, spawn_service, wait_until};
use moiren_core::graph::NodeKind;

fn catalog_state(backend: AudioBackend) -> (UiState, ServiceMailbox) {
    let row = |id: &str, name: &str| DeviceRow {
        endpoint_id: id.into(),
        name: name.into(),
    };
    let mut context = crate::service::ServiceContext::default();
    context.audio_settings.backend = backend;
    context.catalog = Arc::new(DeviceCatalog {
        inputs: vec![
            row("wasapi-input", "WASAPI microphone"),
            row("shared-id", "WASAPI shared input"),
        ],
        outputs: vec![
            row("wasapi-output", "WASAPI speakers"),
            row("shared-id", "WASAPI shared output"),
        ],
        asio_drivers: vec![
            row("asio:{driver}", "ASIO interface"),
            row("shared-id", "ASIO shared driver"),
        ],
        processes: vec![ProcessRow {
            pid: 42,
            creation_time_100ns: 7,
            name: "Player".into(),
        }],
        default_input_endpoint_id: Some("wasapi-input".into()),
        default_output_endpoint_id: Some("wasapi-output".into()),
        ..Default::default()
    });
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let initial = handle.try_snapshot().unwrap();
    (UiState::new(handle, initial), mailbox)
}

#[test]
fn asio_candidates_and_selected_rows_use_the_same_driver_in_both_directions() {
    let (mut state, _) = catalog_state(AudioBackend::Asio);
    assert_eq!(state.selection.input_id, None);
    assert_eq!(state.selection.output_id, None);
    state.source_mode = SourceMode::Input;
    state.selection.input_id = Some("asio:{driver}".into());
    state.selection.output_id = Some("asio:{driver}".into());
    let spec = state
        .candidate_spec()
        .expect("installed ASIO duplex driver");
    assert_eq!(spec.output_endpoint_id, "asio:{driver}");
    assert_eq!(
        spec.source,
        InputSelection::Physical {
            endpoint_id: "asio:{driver}".into()
        }
    );
    assert_eq!(
        state.selection.input_index(&state.snapshot.catalog),
        Some(0)
    );
    assert_eq!(
        state.selection.output_index(&state.snapshot.catalog),
        Some(0)
    );
    assert_eq!(
        state.selection.input_selection(&state.snapshot.catalog),
        Some(spec.source.clone())
    );
    assert_eq!(source_name(&state.snapshot, &spec.source), "ASIO interface");
    assert_eq!(
        output_name(&state.snapshot, &spec.output_endpoint_id),
        "ASIO interface"
    );
}

#[test]
fn vanished_asio_driver_keeps_saved_identity_without_wasapi_fallback() {
    let (mut state, _) = catalog_state(AudioBackend::Asio);
    state.source_mode = SourceMode::Input;
    state.selection.input_id = Some("shared-id".into());
    state.selection.output_id = Some("shared-id".into());
    Arc::make_mut(&mut Arc::make_mut(&mut state.snapshot).catalog)
        .asio_drivers
        .clear();
    assert!(state.candidate_config().is_some());
    assert!(state.candidate_spec().is_none());
    assert_eq!(state.selection.input_index(&state.snapshot.catalog), None);
    assert_eq!(state.selection.output_index(&state.snapshot.catalog), None);
    assert_eq!(
        state.selection.input_selection(&state.snapshot.catalog),
        None
    );
    assert_eq!(
        output_name(&state.snapshot, "shared-id"),
        "Unavailable output"
    );
    assert_eq!(
        source_name(
            &state.snapshot,
            &InputSelection::Physical {
                endpoint_id: "shared-id".into()
            }
        ),
        "Unavailable input"
    );
    assert_eq!(state.selection.input_id.as_deref(), Some("shared-id"));
    assert_eq!(state.selection.output_id.as_deref(), Some("shared-id"));
}

#[test]
fn requested_backend_controls_names_and_row_validation_across_switches() {
    let (mut state, mailbox) = catalog_state(AudioBackend::Wasapi);
    state.source_mode = SourceMode::Input;
    assert!(state.candidate_spec().is_some());
    assert_eq!(
        output_name(&state.snapshot, "shared-id"),
        "WASAPI shared output"
    );
    let source = InputSelection::Physical {
        endpoint_id: "shared-id".into(),
    };
    assert_eq!(source_name(&state.snapshot, &source), "WASAPI shared input");

    let mut snapshot = (*state.snapshot).clone();
    snapshot.active_audio_settings = Some(snapshot.audio_settings.clone());
    snapshot.audio_settings.backend = AudioBackend::Asio;
    mailbox.publish(snapshot);
    state.poll();
    assert!(
        state.candidate_spec().is_none(),
        "WASAPI selections remain unavailable in ASIO"
    );
    assert_eq!(state.selection.input_index(&state.snapshot.catalog), None);
    assert_eq!(state.selection.output_index(&state.snapshot.catalog), None);
    assert_eq!(
        state.selection.input_selection(&state.snapshot.catalog),
        None
    );
    assert_eq!(
        output_name(&state.snapshot, "shared-id"),
        "ASIO shared driver"
    );
    assert_eq!(source_name(&state.snapshot, &source), "ASIO shared driver");
    assert_eq!(
        output_name(&state.snapshot, "wasapi-output"),
        "Unavailable output"
    );

    state.selection.input_id = Some("asio:{driver}".into());
    state.selection.output_id = Some("asio:{driver}".into());
    assert!(state.candidate_spec().is_some());
    let mut snapshot = (*state.snapshot).clone();
    snapshot.audio_settings.backend = AudioBackend::Wasapi;
    mailbox.publish(snapshot);
    state.poll();
    assert!(
        state.candidate_spec().is_none(),
        "ASIO selections remain unavailable in WASAPI"
    );
    assert_eq!(state.selection.input_index(&state.snapshot.catalog), None);
    assert_eq!(state.selection.output_index(&state.snapshot.catalog), None);
    assert_eq!(
        output_name(&state.snapshot, "asio:{driver}"),
        "Unavailable output"
    );
}

#[test]
fn asio_output_accepts_tone_and_live_process_without_substituting_physical_input() {
    let (mut state, _) = catalog_state(AudioBackend::Asio);
    state.selection.output_id = Some("asio:{driver}".into());
    assert!(
        state.candidate_spec().is_some(),
        "software tone uses an ASIO output"
    );
    state.source_mode = SourceMode::Application;
    state.selection.process_key = Some((42, 7));
    let spec = state
        .candidate_spec()
        .expect("live process uses an ASIO output");
    assert_eq!(
        source_name(&state.snapshot, &spec.source),
        "Player (PID 42)"
    );
    assert_eq!(
        state.selection.input_selection(&state.snapshot.catalog),
        Some(spec.source)
    );
    state.selection.process_key = Some((42, 8));
    state.selection.input_id = Some("asio:{driver}".into());
    assert_eq!(
        state.selection.input_selection(&state.snapshot.catalog),
        None
    );
    assert!(state.candidate_spec().is_none());
}

#[test]
fn control_feedback_reports_results_without_internal_protocol_identifiers() {
    use moiren_core::protocol::{ControlReply, ReplyCode};
    let runtime = spawn_service();
    let handle = runtime.handle();
    let snapshot = initial_projection(&handle);
    let mut state = UiState::new(handle.clone(), snapshot);
    for (code, expected) in [
        (ReplyCode::Applied, "Changes applied"),
        (ReplyCode::AppliedLate, "Changes applied"),
        (ReplyCode::Accepted, "Applying changes..."),
        (
            ReplyCode::StaleRevision,
            "Audio routing changed. Try again.",
        ),
    ] {
        Arc::make_mut(&mut state.snapshot).control.last_result = Some(ControlReply {
            code,
            request_id: 1234,
            plan_revision: 5678,
            timeline_epoch: 91011,
            effective_frame: 121314,
        });
        assert_eq!(
            crate::ui::presentation::control_feedback(&state.snapshot, state.pending_request),
            expected
        );
    }
    finish_service(runtime);
}

#[test]
#[cfg(windows)]
fn professional_timing_uses_actual_format_and_hides_stale_generation() {
    use crate::service::{
        AudioBackend, AudioSettings, ProfessionalSessionObservations, SampleFormat,
        SessionGeneration,
    };
    use moiren_engine::node::DeviceOptions;
    use moiren_windows_audio::professional::{
        ProfessionalObservationSnapshot, ProfessionalStreamObservation, native::NativeStreamInfo,
    };
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut state = UiState::new(handle.clone(), initial_projection(&handle));
    let node = state.snapshot.graph.graph.nodes()[0].id();
    let snapshot = Arc::make_mut(&mut state.snapshot);
    snapshot.generation = SessionGeneration(7);
    snapshot.phase = SessionPhase::Running;
    let mut settings = AudioSettings::default();
    settings.processing.sample_rate = 96_000;
    snapshot.active_audio_settings = Some(settings);
    snapshot.professional_observations = Some(ProfessionalSessionObservations {
        generation: SessionGeneration(7),
        observation: ProfessionalObservationSnapshot {
            owner_generation: 3,
            running: true,
            streams: vec![ProfessionalStreamObservation {
                node,
                input: false,
                master: true,
                requested: DeviceOptions {
                    backend: AudioBackend::Asio,
                    hardware_sample_rate: Some(96_000),
                    sample_format: SampleFormat::Native,
                    period_frames: Some(64),
                    ..Default::default()
                },
                actual: NativeStreamInfo {
                    sample_rate: 44_100,
                    format: SampleFormat::Pcm24,
                    buffer_frames: 1024,
                    channels: 2,
                },
                dropped_frames: 9,
                silent_frames: 11,
                native_callbacks: 100,
                native_dropped_frames: 3,
                native_silent_frames: 5,
                driver_latency: Some(moiren_windows_audio::professional::native::NativeLatency {
                    input_frames: 32,
                    output_frames: 64,
                }),
            }],
        },
    });
    let rows = crate::ui::presentation::native_timing_rows(&state.snapshot);
    let actual = &rows
        .iter()
        .find(|(key, _)| key.contains("actual stream"))
        .unwrap()
        .1;
    assert!(actual.contains("44100 Hz"));
    assert!(actual.contains("24-bit PCM"));
    assert!(actual.contains("1024 frames"));
    assert!(!actual.contains("96000"));
    let mode = &rows
        .iter()
        .find(|(key, _)| key.contains("backend / mode"))
        .unwrap()
        .1;
    assert!(mode.contains("driver owns mode"));
    assert!(!mode.contains("Shared"));
    let native = &rows
        .iter()
        .find(|(key, _)| key.contains("ASIO callback counters"))
        .unwrap()
        .1;
    assert!(native.contains("100 callbacks"));
    assert!(native.contains("dropped 3 / render silence 5"));
    assert!(native.contains("startup"));
    let latency = &rows
        .iter()
        .find(|(key, _)| key.contains("driver-reported latency"))
        .unwrap()
        .1;
    assert!(latency.contains("Input 32 / output 64 frames"));
    assert!(latency.contains("excludes application queues"));
    Arc::make_mut(&mut state.snapshot).generation = SessionGeneration(8);
    assert!(
        !crate::ui::presentation::native_timing_rows(&state.snapshot)
            .iter()
            .any(|(key, _)| key.contains("actual stream"))
    );
    finish_service(runtime);
}

#[test]
fn restored_audio_settings_sync_block_size_without_starting_audio() {
    use crate::service::{AudioSettings, ProcessingPrecision};
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut state = UiState::new(handle.clone(), initial_projection(&handle));
    let mut audio = AudioSettings::default();
    audio.processing.sample_rate = 96_000;
    audio.processing.precision = ProcessingPrecision::F64;
    audio.processing.block_frames = 1024;
    state.restore_preferences(&crate::project::AppPreferences {
        audio: audio.clone(),
        ..Default::default()
    });
    assert_eq!(state.max_block_frames, 1024);
    wait_until(std::time::Duration::from_secs(2), || {
        state.poll();
        !state.pending_request
    });
    assert_eq!(state.snapshot.audio_settings, audio);
    assert_eq!(state.snapshot.active_audio_settings, None);
    assert_eq!(state.snapshot.phase, SessionPhase::Idle);
    assert_eq!(
        crate::ui::presentation::latency_apply_text(&state.snapshot),
        "Explicit stream buffers control professional audio latency"
    );
    finish_service(runtime);
}

#[test]
fn invalid_restored_audio_does_not_hide_its_error_with_latency_dispatch() {
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut state = UiState::new(handle.clone(), initial_projection(&handle));
    let mut preferences = crate::project::AppPreferences::default();
    preferences.audio.processing.sample_rate = 192_000;
    preferences.latency_profile = crate::service::AudioLatencyProfile::Low;
    state.restore_preferences(&preferences);
    assert!(
        state
            .dispatch_error
            .as_ref()
            .unwrap()
            .contains("sample rate")
    );
    assert!(!state.pending_request);
    assert_eq!(state.max_block_frames, 256);
    assert_eq!(state.snapshot.consumed_request_id, RequestId::default());
    finish_service(runtime);
}

#[test]
fn restored_latency_preference_selects_next_start_without_starting_audio() {
    use crate::service::AudioLatencyProfile;
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut state = UiState::new(handle.clone(), initial_projection(&handle));
    state.restore_preferences(&crate::project::AppPreferences {
        latency_profile: AudioLatencyProfile::Low,
        ..Default::default()
    });
    wait_until(std::time::Duration::from_secs(2), || {
        state.poll();
        !state.pending_request
    });
    assert_eq!(state.snapshot.latency_profile, AudioLatencyProfile::Low);
    assert_eq!(state.snapshot.phase, SessionPhase::Idle);
    assert_eq!(state.snapshot.active_latency_profile, None);
    assert_eq!(
        crate::ui::presentation::latency_apply_text(&state.snapshot),
        "Selected profile activates on the next audio start"
    );
    let snapshot = Arc::make_mut(&mut state.snapshot);
    snapshot.active_latency_profile = Some(AudioLatencyProfile::Compatible);
    assert_eq!(
        crate::ui::presentation::latency_apply_text(&state.snapshot),
        "Selected profile activates on the next audio start"
    );
    Arc::make_mut(&mut state.snapshot).active_latency_profile = Some(AudioLatencyProfile::Low);
    assert_eq!(
        crate::ui::presentation::latency_apply_text(&state.snapshot),
        "Active profile matches selection"
    );
    finish_service(runtime);
}

#[cfg(windows)]
#[test]
fn native_timing_projection_hides_stopped_and_stale_owner_observations() {
    use crate::service::{NativeSessionObservations, SessionGeneration};
    use moiren_windows_audio::{
        engine_backend::{
            NativeBridgeObservation, NativeObservationSnapshot, NativeStreamKind,
            NativeStreamObservation,
        },
        latency::NativeStreamTiming,
    };
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut snapshot = (*initial_projection(&handle)).clone();
    snapshot.phase = SessionPhase::Running;
    snapshot.generation = SessionGeneration(8);
    let node = snapshot.graph.graph.nodes()[0].id();
    snapshot.native_observations = Some(NativeSessionObservations {
        generation: SessionGeneration(8),
        observation: NativeObservationSnapshot {
            owner_generation: 3,
            running: true,
            streams: vec![NativeStreamObservation {
                node,
                kind: NativeStreamKind::MasterOutput,
                timing: Some(NativeStreamTiming {
                    sample_rate: 48_000,
                    period_frames: Some(336),
                    capacity_frames: 960,
                }),
                shared_period: Some(moiren_windows_audio::shared_stream::SharedPeriodReport {
                    desired_period_ms: Some(5.0),
                    actual_period_frames: Some(336),
                    ..Default::default()
                }),
                render_queue: None,
                input_queue: None,
            }],
            boundary_probes: vec![],
            bridges: vec![NativeBridgeObservation {
                node,
                counters: moiren_windows_audio::clock_bridge::BridgeSnapshot {
                    underrun_frames: 4,
                    dropped_frames: 8,
                    resets: 1,
                    backlog_recoveries: 1,
                    ..Default::default()
                },
            }],
        },
    });
    let mut state = UiState::new(handle.clone(), Arc::new(snapshot));
    assert!(
        crate::ui::presentation::native_timing_rows(&state.snapshot)[0]
            .1
            .contains("336 frames (7.00 ms)")
    );
    assert!(
        crate::ui::presentation::native_timing_rows(&state.snapshot)
            .iter()
            .any(|(_, value)| value == "5.00 ms requested")
    );
    assert!(
        crate::ui::presentation::native_timing_rows(&state.snapshot)
            .iter()
            .any(|(label, value)| {
                label == "Stream 1: faults / recovery"
                    && value == "Underrun 4 / dropped 8 frames; resets 1 / backlog recoveries 1"
            })
    );
    Arc::make_mut(&mut state.snapshot).generation = SessionGeneration(9);
    assert_eq!(
        crate::ui::presentation::native_timing_rows(&state.snapshot),
        vec![("Actual native timing".into(), "—".into())]
    );
    Arc::make_mut(&mut state.snapshot).generation = SessionGeneration(8);
    Arc::make_mut(&mut state.snapshot).phase = SessionPhase::Idle;
    assert_eq!(
        crate::ui::presentation::native_timing_rows(&state.snapshot)[0].1,
        "—"
    );
    finish_service(runtime);
}

#[test]
fn service_resource_binding_changes_update_the_shared_session_projection() {
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut state = UiState::new(handle.clone(), initial_projection(&handle));
    let source = state
        .snapshot
        .graph
        .graph
        .nodes()
        .iter()
        .find(|n| n.kind() == NodeKind::Source)
        .unwrap()
        .id();
    state.dispatch(AppRequest::EditGraph(
        crate::service::GraphEdit::BindSource {
            node: source,
            source: InputSelection::Physical {
                endpoint_id: "service-selected-input".into(),
            },
        },
    ));
    wait_until(std::time::Duration::from_secs(2), || {
        state.poll();
        !state.pending_request
    });
    assert_eq!(state.source_mode, SourceMode::Input);
    assert_eq!(
        state.selection.input_id.as_deref(),
        Some("service-selected-input")
    );
    finish_service(runtime);
}

#[test]
fn unavailable_resources_remain_in_saved_candidate_configuration() {
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut state = UiState::new(handle.clone(), initial_projection(&handle));
    let spec = SessionSpec {
        source: InputSelection::Physical {
            endpoint_id: "unplugged-input".into(),
        },
        output_endpoint_id: "unplugged-output".into(),
        limit: RunLimit::Seconds(35),
        gain: 0.25,
        pan: -0.4,
        max_block_frames: 512,
    };
    state.restore_candidate(&spec);
    assert_eq!(state.candidate_config(), Some(spec));
    assert!(
        state.candidate_spec().is_none(),
        "unavailable devices cannot start"
    );
    finish_service(runtime);
}

#[test]
fn graph_only_template_clears_previous_project_resource_selection() {
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut state = UiState::new(handle.clone(), initial_projection(&handle));
    state.selection.output_id = Some("old-output".into());
    state.selection.input_id = Some("old-input".into());
    state.source_mode = SourceMode::Input;
    state.max_block_frames = 1024;
    state.run_limit = RunLimit::Seconds(60);
    state.muted = true;
    state.restore_graph_candidate();
    assert_eq!(state.source_mode, SourceMode::Tone);
    assert_eq!(state.tone_frequency, 440.);
    assert!(state.selection.output_id.is_none());
    assert!(state.selection.input_id.is_none());
    assert_eq!(state.max_block_frames, 256);
    assert_eq!(state.run_limit, RunLimit::UntilStopped);
    assert!(!state.muted);
    assert!(state.candidate_config().is_none());
    finish_service(runtime);
}

#[test]
fn running_projection_edits_candidate_nodes_instead_of_legacy_session_pair() {
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut snapshot = (*initial_projection(&handle)).clone();
    snapshot.phase = SessionPhase::Running;
    let mut state = UiState::new(handle.clone(), Arc::new(snapshot));
    state.gain = 0.25;
    state.pan = 0.4;
    state.request_parameters();
    wait_until(std::time::Duration::from_secs(2), || {
        state.poll();
        !state.pending_request
    });
    for node in state.snapshot.graph.graph.nodes() {
        let entry = state.snapshot.graph.node(node.id()).unwrap();
        match node.kind() {
            NodeKind::Gain => assert_eq!(entry.gain, 0.25),
            NodeKind::Pan => assert_eq!(entry.pan, 0.4),
            _ => {}
        }
    }
    assert!(state.snapshot.control.pending_gain_pan.is_none());
    finish_service(runtime);
}
