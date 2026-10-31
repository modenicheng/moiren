use super::*;
use crate::{
    latency::{InputDeliveryRecorder, events::event_history, transport::TransportInputPolicy},
    process_loopback::{MeasuredProcessDelivery, ProcessIdentity},
};

#[test]
fn real_preparation_accounts_measured_storage_and_refuses_negotiated_whole_demand() {
    for capacity in [2048, 65_536] {
        let mut project = EngineProject::default();
        project.processing.engine.processing_sr = 48_000.0;
        project.processing.audio_byte_budget = 3 * 1024 * 1024;
        let source = project.graph.create_node(NodeKind::Source, 2).unwrap();
        project.nodes.insert(
            source,
            NodeConfig::Input(InputConfig {
                source: InputSource::Application {
                    executable: "moiren:process:42:123".into(),
                    intent: CaptureIntent::PassiveTap,
                },
                channels: 2,
                device: Default::default(),
            }),
        );
        add_physical_output(&mut project, 2);
        let identity = ProcessIdentity {
            pid: 42,
            creation_time_100ns: 123,
            executable_name: "source.exe".into(),
        };
        let (mut producer, producer_observer) = event_history();
        let (mut consumer, consumer_observer) = event_history();
        let mut delivery = InputDeliveryRecorder::new(None).unwrap();
        for stamp in [1_000_000, 1_020_000, 1_040_000] {
            producer.record(96, Some(stamp));
            consumer.record(96, Some(stamp + 1000));
            delivery.packet(96, Some(stamp - 1));
            delivery.callback_burst(96);
        }
        let evidence = MeasuredProcessDelivery::from_observations(
            identity.clone(),
            6,
            48_000,
            48_000,
            96,
            delivery.snapshot(),
            producer_observer.snapshot().unwrap(),
            consumer_observer.snapshot().unwrap(),
        )
        .unwrap();
        let backend = ProfessionalBackend::<f32>::new()
            .unwrap()
            .with_measured_process_bridge(
                source,
                &identity,
                6,
                evidence,
                TransportInputPolicy::Target {
                    capacity_frames: capacity,
                    target_frames: 256,
                    jitter_frames: 0,
                },
            )
            .unwrap();
        let native = native_devices(Default::default());
        let mut signals = moiren_signal::Bus::new(Default::default()).unwrap();
        let (host, _rt) = signals.create_host(8).unwrap();
        let error = backend
            .prepare_with_native(
                request(&project, &host),
                &CancellationToken::default(),
                &native,
            )
            .err()
            .expect("measured preparation must refuse before opening Process capture");
        let message = error.to_string();
        if capacity == 2048 {
            assert!(message.contains("input target cannot cover"), "{message}");
            assert_eq!(native.probe.opened.load(Ordering::Acquire), 1);
        } else {
            assert!(message.contains("audio preparation budget"), "{message}");
            assert_eq!(native.probe.opened.load(Ordering::Acquire), 0);
        }
        assert_eq!(native.probe.started.load(Ordering::Acquire), 0);
        assert_eq!(
            native.probe.dropped.load(Ordering::Acquire),
            native.probe.opened.load(Ordering::Acquire),
        );
    }
}
