//! Opt-in native acceptance. Capture is never recorded and gain stays zero.
#![cfg(all(windows, target_pointer_width = "64"))]

use moiren_core::graph::{NodeKind, SendParams};
use moiren_engine::{
    audio_config::{AudioBackend, SampleFormat},
    host::{
        EngineHost, EngineProject, ExecutionState, HostConfig, HostState, NodeConfig, UpdateState,
    },
    node::EndpointSelector,
    sample::ProcessingSample,
};
use moiren_windows_audio::{
    engine_backend::{physical_input, physical_output},
    professional::{
        ProfessionalBackend,
        asio::{AsioCapabilities, query_capabilities},
    },
};
use std::time::{Duration, Instant};

const FL_ASIO: &str = "{188135E1-7171-3434-854F-01A3C71F3DF9}";

fn silent_host<S: ProcessingSample>(capabilities: &AsioCapabilities) {
    assert!(!capabilities.inputs.is_empty() && !capabilities.outputs.is_empty());
    let rate = [48000, 44100, 96000, 88200]
        .into_iter()
        .find(|rate| {
            capabilities
                .sample_rates
                .iter()
                .any(|candidate| candidate.sample_rate == *rate && candidate.supported)
        })
        .expect("driver must support an engine rate");
    let mut project = EngineProject::default();
    project.processing.engine.processing_sr = f64::from(rate);
    let input = project.graph.create_node(NodeKind::Source, 2).unwrap();
    let gain = project.graph.create_node(NodeKind::Gain, 2).unwrap();
    let output = project.graph.create_node(NodeKind::Sink, 2).unwrap();
    let endpoint = EndpointSelector::PinnedEndpoint {
        endpoint_id: format!("asio:{FL_ASIO}"),
        stable_id: None,
    };
    let mut source = physical_input(endpoint.clone());
    let mut sink = physical_output(endpoint);
    for device in [&mut source.device, &mut sink.device] {
        device.backend = AudioBackend::Asio;
        device.hardware_sample_rate = Some(rate);
        device.sample_format = SampleFormat::Native;
        // None preserves the exact preferred-size convention, not a 256 guess.
        device.period_frames = None;
    }
    project.nodes.insert(input, NodeConfig::Input(source));
    project
        .nodes
        .insert(gain, NodeConfig::Gain { initial: 0.0 });
    project.nodes.insert(output, NodeConfig::Output(sink));
    for (from, to) in [(input, gain), (gain, output)] {
        project
            .graph
            .connect(
                project.graph.get_node(from).unwrap().outputs()[0].id(),
                project.graph.get_node(to).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
    }
    let backend = ProfessionalBackend::<S>::new().unwrap();
    let observations = backend.observations();
    let host = EngineHost::<S>::open(HostConfig::default(), project, backend).unwrap();
    let observer = host.observer();
    // Windows audio RPC during third-party activation can exceed 30 seconds on
    // this machine. Keep a finite startup deadline plus an external watchdog.
    let deadline = Instant::now() + Duration::from_secs(150);
    loop {
        let state = observer.snapshot();
        if state.execution == ExecutionState::Running
            && state
                .timeline
                .is_some_and(|timeline| timeline.frame >= u64::from(rate) * 2)
        {
            let actual = observations
                .snapshot()
                .expect("running native observations");
            assert!(actual.running);
            assert_eq!(actual.owner_generation, state.owner_generation);
            assert_eq!(actual.streams.len(), 2);
            for stream in &actual.streams {
                assert_eq!(stream.actual.sample_rate, rate);
                assert_eq!(
                    stream.actual.buffer_frames,
                    capabilities.buffer.preferred_frames
                );
                assert!(stream.native_callbacks > 0);
            }
            println!(
                "precision={} host={state:?} native={actual:?}",
                std::any::type_name::<S>()
            );
            break;
        }
        assert!(state.fault.is_none(), "host fault: {state:?}");
        assert!(
            !matches!(state.update, UpdateState::Blocked { .. }),
            "native host blocked: {state:?}"
        );
        assert!(
            Instant::now() < deadline,
            "finite ASIO host timeout: {state:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(host);
    let stopped = observer.snapshot();
    assert_eq!(stopped.host, HostState::Closed);
    assert!(stopped.cleanup_error.is_none(), "{stopped:?}");
    assert!(observations.snapshot().is_none());
}

#[test]
#[ignore = "opens FL Studio ASIO only; requires explicit MOIREN_ASIO_HARDWARE_TEST_DRIVER opt-in"]
fn fl_asio_preferred_duplex_silent_engine_host_both_precisions() {
    assert_eq!(
        std::env::var("MOIREN_ASIO_HARDWARE_TEST_DRIVER").as_deref(),
        Ok(FL_ASIO),
        "this acceptance test opts into FL Studio ASIO only"
    );
    let capabilities = query_capabilities(FL_ASIO).unwrap();
    println!("capabilities={capabilities:?}");
    silent_host::<f32>(&capabilities);
    silent_host::<f64>(&capabilities);
}
