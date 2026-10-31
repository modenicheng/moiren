use super::*;
use std::sync::mpsc;

fn session() -> SessionSpec {
    SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "render".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }
}

struct ObserveStartup {
    first_poll: Option<mpsc::Receiver<()>>,
    observed: mpsc::Sender<AudioSettings>,
}

impl ServiceDriver for ObserveStartup {
    fn request(&mut self, _: &mut ServiceContext, _: &mut JobPool, _: AppRequest) {}

    fn poll(&mut self, context: &mut ServiceContext, _: &mut JobPool, _: RoundBudget) {
        if let Some(first_poll) = self.first_poll.take() {
            // Keep the initial snapshot stable until the test has inspected it;
            // try_snapshot intentionally returns None while publication holds its lock.
            first_poll.recv_timeout(Duration::from_secs(5)).unwrap();
            self.observed.send(context.audio_settings.clone()).unwrap();
        }
    }

    fn is_drained(&self) -> bool {
        true
    }
}

#[test]
fn saved_audio_is_present_before_first_driver_poll_and_initial_snapshot() {
    let mut settings = AudioSettings::default();
    settings.processing.sample_rate = 96_000;
    settings.processing.precision = ProcessingPrecision::F64;
    settings.processing.block_frames = 1024;
    let (first_poll_tx, first_poll_rx) = mpsc::channel();
    let (observed_tx, observed_rx) = mpsc::channel();
    let runtime = ServiceRuntime::spawn_with_settings(
        ObserveStartup {
            first_poll: Some(first_poll_rx),
            observed: observed_tx,
        },
        settings.clone(),
        AudioLatencyProfile::Low,
    )
    .unwrap();
    let handle = runtime.handle();
    let initial = handle.try_snapshot().unwrap();
    assert_eq!(initial.audio_settings, settings);
    assert_eq!(initial.latency_profile, AudioLatencyProfile::Low);
    first_poll_tx.send(()).unwrap();
    let observed = observed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    crate::ui::finish_service(runtime);
    assert_eq!(observed, settings);
}

#[test]
fn active_audio_settings_require_running_current_owner() {
    let mut context = ServiceContext {
        active_audio_settings: Some(AudioSettings::default()),
        ..ServiceContext::default()
    };
    assert_eq!(snapshot(&context).active_audio_settings, None);
    context.core.managed_desired(session()).unwrap();
    let generation = context.core.generation();
    context.core.managed_execution(generation, false, None);
    assert_eq!(snapshot(&context).active_audio_settings, None);
    context.core.managed_execution(generation, true, None);
    assert_eq!(
        snapshot(&context).active_audio_settings,
        Some(AudioSettings::default())
    );
    context.core.stop().unwrap();
    assert_eq!(snapshot(&context).active_audio_settings, None);
}

#[test]
fn active_transport_settings_are_fenced_by_running_owner_generation() {
    let mut context = ServiceContext {
        active_transport_latency: Some(TransportLatencySettings::default()),
        ..Default::default()
    };
    assert_eq!(snapshot(&context).active_transport_latency, None);
    context.core.managed_desired(session()).unwrap();
    let old = context.core.generation();
    context.core.managed_execution(old, true, None);
    assert_eq!(
        snapshot(&context).active_transport_latency,
        Some(Default::default())
    );
    context.core.stop().unwrap();
    assert_eq!(snapshot(&context).active_transport_latency, None);
    context.core.managed_desired(session()).unwrap();
    assert_ne!(context.core.generation(), old);
    context.core.managed_execution(old, true, None);
    assert_eq!(snapshot(&context).active_transport_latency, None);
}
