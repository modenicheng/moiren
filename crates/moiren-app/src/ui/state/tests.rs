//! Unit tests for the UI state projection, split out of `state.rs`
//! (the `transport_tests` module is the earlier precedent).

use super::*;
use crate::service::InputSelection;
use crate::ui::{finish_service, initial_projection, spawn_service, wait_until};
use moiren_core::graph::NodeKind;

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
    use moiren_windows_audio::engine_backend::{
        ProfessionalObservationSnapshot, ProfessionalStreamObservation, StreamEndpoint,
    };
    use moiren_windows_audio::native::NativeStreamInfo;
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
                endpoint: StreamEndpoint::Physical("asio:{probe-driver}".into()),
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
                xruns: 0,
                native_callbacks: 100,
                native_dropped_frames: 3,
                native_silent_frames: 5,
                native_xruns: 0,
                engine_blocks: 0,
                engine_busy_100ns: 0,
                engine_peak_block_100ns: 0,
                driver_latency: Some(moiren_windows_audio::native::NativeLatency {
                    input_frames: 32,
                    output_frames: 64,
                }),
            }],
        },
    });
    let views = crate::ui::presentation::native_timing_views(&state.snapshot);
    assert_eq!(views.processing.as_ref().unwrap().0, "Processing");
    let stream = &views.streams[0];
    assert_eq!(stream.node, node);
    // The endpoint identity carries the service-side row id; with no catalog
    // row for it the title falls back to the raw identity.
    assert_eq!(stream.device_title, "asio:{probe-driver}");
    let actual = &stream
        .rows
        .iter()
        .find(|(key, _)| key.contains("Actual stream"))
        .unwrap()
        .1;
    assert!(actual.contains("44100 Hz"));
    assert!(actual.contains("24-bit PCM"));
    assert!(actual.contains("1024 frames"));
    assert!(!actual.contains("96000"));
    let mode = &stream
        .rows
        .iter()
        .find(|(key, _)| key.contains("Backend / mode"))
        .unwrap()
        .1;
    assert!(mode.contains("driver owns mode"));
    assert!(!mode.contains("Shared"));
    let native = &stream
        .rows
        .iter()
        .find(|(key, _)| key.contains("ASIO callback counters"))
        .unwrap()
        .1;
    assert!(native.contains("100 callbacks"));
    assert!(native.contains("dropped 3 / render silence 5"));
    assert!(native.contains("startup"));
    let latency = &stream
        .rows
        .iter()
        .find(|(key, _)| key.contains("Driver-reported latency"))
        .unwrap()
        .1;
    assert!(latency.contains("Input 32 / output 64 frames"));
    assert!(latency.contains("excludes application queues"));
    Arc::make_mut(&mut state.snapshot).generation = SessionGeneration(8);
    let stale = crate::ui::presentation::native_timing_views(&state.snapshot);
    assert!(stale.streams.is_empty() && stale.processing.is_none());
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
    wait_until(std::time::Duration::from_secs(2), || {
        state.poll();
        !state.pending_request
    });
    assert_eq!(state.snapshot.audio_settings, audio);
    assert_eq!(state.snapshot.active_audio_settings, None);
    assert_eq!(state.snapshot.phase, SessionPhase::Idle);
    assert_eq!(
        crate::ui::presentation::latency_apply_text(&state.snapshot),
        "Selected profile activates on the next audio start"
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
    // The typed failure carries the closed AudioConfigError set whole; the
    // old frozen string asserted only the rendered prose ("sample rate").
    assert!(matches!(
        state.dispatch_error,
        Some(UiFailure::InvalidAudioSettings(
            crate::service::AudioConfigError::InvalidSampleRate
        ))
    ));
    assert!(!state.pending_request);
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

#[test]
fn display_label_shows_the_node_label_even_when_a_device_is_bound() {
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
                endpoint_id: "wasapi-input".into(),
            },
        },
    ));
    wait_until(std::time::Duration::from_secs(2), || {
        state.poll();
        !state.pending_request
    });
    // The old presentation followed the bound device name while the label
    // was the default; the card's binding chip carries that information
    // now, so the label shows what the node is, bare.
    let entry = state.snapshot.graph.node(source).unwrap();
    assert_eq!(
        crate::ui::presentation::display_label(entry),
        crate::service::default_label(NodeKind::Source)
    );
    finish_service(runtime);
}

#[test]
fn dispatch_failure_text_translates_at_paint_per_locale() {
    // failure_text reads the process locale at paint; under the default
    // locale it renders the English key, and the explicit zh-CN lookup of
    // the same key differs (the i18n tests' explicit-locale pattern keeps
    // parallel tests independent of global state).
    let failure = UiFailure::Dispatch(crate::service::DispatchError::Busy);
    let english = crate::ui::presentation::failure_text(&failure);
    let chinese = rust_i18n::t!(
        "The audio service is busy; try again in a moment.",
        locale = crate::i18n::SIMPLIFIED_CHINESE
    )
    .into_owned();
    assert_eq!(english, "The audio service is busy; try again in a moment.");
    assert_ne!(
        english, chinese,
        "the zh-CN catalog must translate the dispatch failure"
    );
}
