use super::model::*;
use super::{AudioSettings, Density, SampleFormat, StreamSettings};

fn draft(audio: AudioSettings) -> Draft {
    Draft::new(
        audio,
        crate::service::AudioLatencyProfile::Compatible,
        Density::Comfortable,
        Default::default(),
    )
}

#[test]
fn stale_device_draft_preserves_new_defaults_and_other_device_overrides() {
    let original = AudioSettings::default();
    let mut device = draft(original.clone());
    device_stream_mut(&mut device.audio, Direction::Output, "device-a").buffer_frames = Some(128);
    let mut current = original;
    current.processing.sample_rate = 96_000;
    current.output.sample_rate = Some(88_200);
    current.input_overrides.insert(
        "microphone".into(),
        StreamSettings {
            buffer_frames: Some(512),
            ..Default::default()
        },
    );
    current.output_overrides.insert(
        "device-b".into(),
        StreamSettings {
            sample_rate: Some(44_100),
            ..Default::default()
        },
    );
    let merged = device
        .submission(&current, device.profile, device.density, Default::default())
        .unwrap()
        .audio;
    assert_eq!(merged.processing, current.processing);
    assert_eq!(merged.output, current.output);
    assert_eq!(merged.input_overrides, current.input_overrides);
    assert_eq!(
        merged.output_overrides["device-b"],
        current.output_overrides["device-b"]
    );
    assert_eq!(merged.output_overrides["device-a"].buffer_frames, Some(128));
    assert_eq!(merged.output_overrides["device-a"].sample_rate, None);
}

#[test]
fn stale_global_draft_preserves_new_and_removed_device_overrides() {
    let mut original = AudioSettings::default();
    original
        .output_overrides
        .insert("removed".into(), StreamSettings::default());
    let mut global = draft(original.clone());
    global.audio.input.sample_rate = Some(96_000);
    let mut current = original;
    current.output_overrides.remove("removed");
    current.input_overrides.insert(
        "new".into(),
        StreamSettings {
            format: SampleFormat::Pcm24,
            ..Default::default()
        },
    );
    let merged = global
        .submission(&current, global.profile, global.density, Default::default())
        .unwrap()
        .audio;
    assert_eq!(merged.input.sample_rate, Some(96_000));
    assert_eq!(merged.input_overrides, current.input_overrides);
    assert!(!merged.output_overrides.contains_key("removed"));
}

#[test]
fn reset_removes_only_the_edited_override_and_uses_latest_defaults() {
    let mut original = AudioSettings::default();
    original.input_overrides.insert(
        "a".into(),
        StreamSettings {
            sample_rate: Some(44_100),
            ..Default::default()
        },
    );
    let mut device = draft(original.clone());
    device.audio.input_overrides.remove("a");
    let mut current = original;
    current.input.sample_rate = Some(96_000);
    current
        .input_overrides
        .insert("b".into(), StreamSettings::default());
    let merged = device
        .submission(&current, device.profile, device.density, Default::default())
        .unwrap()
        .audio;
    assert!(!merged.input_overrides.contains_key("a"));
    assert_eq!(merged.effective_input("a"), current.input);
    assert!(merged.input_overrides.contains_key("b"));
}

#[test]
fn conflicting_full_device_override_is_rejected_without_losing_draft() {
    let original = AudioSettings::default();
    let mut device = draft(original.clone());
    device_stream_mut(&mut device.audio, Direction::Input, "a").sample_rate = Some(44_100);
    let mut current = original;
    current.input_overrides.insert(
        "a".into(),
        StreamSettings {
            buffer_frames: Some(2048),
            ..Default::default()
        },
    );
    let error = device
        .submission(&current, device.profile, device.density, Default::default())
        .err()
        .unwrap();
    assert!(error.starts_with("Input override for a changed elsewhere"));
    assert_eq!(device.audio.input_overrides["a"].sample_rate, Some(44_100));
    assert!(device.dirty());
    assert_eq!(current.input_overrides["a"].buffer_frames, Some(2048));
}

#[test]
fn failed_submission_rebase_does_not_turn_unedited_defaults_into_owned_edits() {
    let original = AudioSettings::default();
    let mut device = draft(original.clone());
    device_stream_mut(&mut device.audio, Direction::Output, "a").buffer_frames = Some(64);
    let mut current = original;
    current.output.sample_rate = Some(88_200);
    let first = device
        .submission(&current, device.profile, device.density, Default::default())
        .unwrap();
    assert_eq!(first.audio.output.sample_rate, Some(88_200));
    current.output.sample_rate = Some(96_000);
    let retry = device
        .submission(&current, device.profile, device.density, Default::default())
        .unwrap();
    assert_eq!(retry.audio.output.sample_rate, Some(96_000));
    assert_eq!(retry.audio.output_overrides["a"].buffer_frames, Some(64));
}

#[test]
fn restart_confirmation_expects_the_saved_profile_for_every_configuration() {
    use crate::service::AudioLatencyProfile;
    assert_eq!(
        expected_active_profile(AudioLatencyProfile::Low),
        Some(AudioLatencyProfile::Low)
    );
    assert_eq!(
        expected_active_profile(AudioLatencyProfile::Balanced),
        Some(AudioLatencyProfile::Balanced)
    );
    assert_eq!(
        expected_active_profile(AudioLatencyProfile::Compatible),
        Some(AudioLatencyProfile::Compatible)
    );
}

#[test]
fn stream_summaries_show_wasapi_mode_and_asio_driver_owned_encoding() {
    use moiren_engine::{audio_config::AudioBackend, node::DeviceMode};
    let stream = StreamSettings {
        mode: DeviceMode::Exclusive,
        format: SampleFormat::Pcm24,
        ..Default::default()
    };
    let wasapi = super::stream_summary(AudioBackend::Wasapi, stream);
    assert!(wasapi.contains("Exclusive"));
    assert!(wasapi.contains("24-bit PCM"));
    let asio = super::stream_summary(AudioBackend::Asio, stream);
    assert!(asio.contains("Driver encoding"));
    assert!(!asio.contains("PCM"));
    assert!(!asio.contains("Exclusive"));
}

#[test]
fn transport_draft_rebases_independent_inputs_and_rejects_same_input_conflicts() {
    use crate::service::{ExpertInputBuffer, TransportLatencySettings};
    let mut edited = draft(AudioSettings::default());
    edited.transport.process = Some(ExpertInputBuffer::default());
    let current = TransportLatencySettings {
        vad: Some(ExpertInputBuffer {
            target_frames: 1024,
            ..Default::default()
        }),
        ..Default::default()
    };
    let merged = edited
        .submission(&edited.audio, edited.profile, edited.density, current)
        .unwrap();
    assert_eq!(merged.transport.process, edited.transport.process);
    assert_eq!(merged.transport.vad, current.vad);
    let conflict = TransportLatencySettings {
        process: Some(ExpertInputBuffer {
            target_frames: 4096,
            ..Default::default()
        }),
        ..current
    };
    assert!(
        edited
            .submission(&edited.audio, edited.profile, edited.density, conflict)
            .is_err()
    );
    assert_eq!(edited.transport.process.unwrap().target_frames, 2048);
    assert!(edited.dirty());
}

#[test]
fn device_draft_does_not_overwrite_new_transport_intent_and_commit_tracks_it() {
    use crate::service::{ExpertInputBuffer, TransportLatencySettings};
    let mut device = draft(AudioSettings::default());
    device_stream_mut(&mut device.audio, Direction::Input, "microphone").buffer_frames = Some(128);
    let transport = TransportLatencySettings {
        process: Some(ExpertInputBuffer::default()),
        ..Default::default()
    };
    let merged = device
        .submission(
            &AudioSettings::default(),
            device.profile,
            device.density,
            transport,
        )
        .unwrap();
    assert_eq!(merged.transport, transport);
    device.transport = merged.transport;
    device.committed();
    assert!(!device.dirty());
    device.transport.process = None;
    assert!(device.dirty());
}

fn measured(selection_id: u64) -> crate::service::MeasuredProcessSummary {
    use crate::service::{MeasuredProcessRequest, MeasuredProcessSummary, SessionGeneration};
    let snapshot = crate::service::snapshot(&Default::default());
    MeasuredProcessSummary {
        selection_id,
        request: MeasuredProcessRequest {
            node: snapshot.graph.graph.nodes()[0].id(),
            session_generation: SessionGeneration(7),
            native_owner_generation: 11,
            graph_revision: 3,
            capacity_frames: 4096,
            target_frames: 512,
            jitter_frames: 96,
        },
        process_pid: 42,
        process_creation_time_100ns: 123,
        delivery: crate::service::measured_process::MeasuredDeliveryBound {
            sample_rate: 48_000,
            max_packet_frames: 480,
            max_gap_frames: 480,
        },
    }
}

fn running_settings() -> crate::service::AppSnapshot {
    let mut snapshot = crate::service::snapshot(&Default::default());
    snapshot.phase = crate::service::SessionPhase::Running;
    snapshot.active_audio_settings = Some(snapshot.audio_settings.clone());
    snapshot.active_transport_latency = Some(snapshot.transport_latency);
    snapshot.active_latency_profile = Some(snapshot.latency_profile);
    snapshot
}

#[test]
fn measured_process_summary_identifies_override_instead_of_compatible_default() {
    let selection = measured(17);
    let text = super::process_input_summary(None, Some(&selection));
    for fact in [
        "Measured",
        "source node",
        "PID 42",
        "created 123",
        "target 512",
        "capacity 4096",
        "jitter 96",
        "session 7",
        "owner 11",
        "revision 3",
        "selection 17",
    ] {
        assert!(text.contains(fact), "missing {fact:?} in {text:?}");
    }
    assert!(!text.contains("Compatible"));
    assert_eq!(super::process_input_summary(None, None), "Compatible");
}

#[test]
fn measured_capture_rebases_current_selection_not_the_old_desktop_draft() {
    let editor = draft(AudioSettings::default());
    let mut snapshot = running_settings();
    snapshot.measured_process = Some(measured(17));
    snapshot.active_measured_process = Some(measured(16));
    let captured = editor.capture(&snapshot, editor.density).unwrap();
    assert_eq!(captured.measured_process, snapshot.measured_process);
    assert!(captured.matches_requested(&snapshot));
    assert!(!captured.matches_active(&snapshot));
    snapshot.active_measured_process = snapshot.measured_process.clone();
    assert!(captured.matches_active(&snapshot));
}

#[test]
fn measured_capture_expects_transport_save_to_clear_desired_override() {
    let mut editor = draft(AudioSettings::default());
    editor.transport.vad = Some(crate::service::ExpertInputBuffer::default());
    let mut snapshot = running_settings();
    snapshot.measured_process = Some(measured(17));
    snapshot.active_measured_process = snapshot.measured_process.clone();
    let captured = editor.capture(&snapshot, editor.density).unwrap();
    assert!(captured.measured_process.is_none());
    assert!(!captured.matches_active(&snapshot));
    snapshot.transport_latency = captured.transport;
    snapshot.active_transport_latency = Some(captured.transport);
    snapshot.measured_process = None;
    assert!(captured.matches_requested(&snapshot));
    assert!(
        !captured.matches_active(&snapshot),
        "old measured owner is not the saved request"
    );
    snapshot.active_measured_process = None;
    assert!(captured.matches_active(&snapshot));
}

#[test]
fn measured_restart_cannot_confirm_a_new_selection_over_the_captured_request() {
    let editor = draft(AudioSettings::default());
    let mut snapshot = running_settings();
    snapshot.measured_process = Some(measured(17));
    snapshot.active_measured_process = snapshot.measured_process.clone();
    let captured = editor.capture(&snapshot, editor.density).unwrap();
    snapshot.measured_process = Some(measured(18));
    snapshot.active_measured_process = snapshot.measured_process.clone();
    assert!(!captured.matches_requested(&snapshot));
    assert!(!captured.matches_active(&snapshot));
    snapshot.measured_process = captured.measured_process.clone();
    snapshot.active_measured_process = captured.measured_process.clone();
    assert!(captured.matches_requested(&snapshot));
    assert!(captured.matches_active(&snapshot));
}
