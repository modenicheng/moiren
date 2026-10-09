use super::*;
use moiren_engine::boundary::{AudioWriter, audio_bridge};
use moiren_windows_audio::clock_bridge::{ClockBridgeConfig, capture_bridge};

fn fixture() -> (
    HostSession,
    DemandRenderer,
    Vec<(SourceId, AudioWriter<f32>)>,
) {
    let (host, renderer) = AudioHost::prepare(HostConfig::default()).unwrap();
    let mut session = HostSession {
        host: Some(host),
        render: None,
        observer: RenderObserver::default(),
        sources: BTreeMap::new(),
        status: SessionStatus::Running,
        output_endpoint_id: "synthetic".into(),
        failure: None,
        runtime: SessionRuntime::default(),
        final_graph: None,
        render_report: None,
        events: Vec::new(),
    };
    let mut inputs = Vec::new();
    for endpoint in ["a", "b"] {
        let (writer, reader) = audio_bridge(2, 256, 8192).unwrap();
        let id = session
            .host_mut()
            .unwrap()
            .add_source_with_settings(
                reader,
                SourceSettings {
                    gain: 0.05,
                    pan: 0.0,
                    available: true,
                },
            )
            .unwrap();
        let gate = session.host().unwrap().source_gate(id).unwrap();
        let (_, _, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
        // The fixture replaces device workers with owned software sources;
        // it still executes the real compiled graph and backend DemandRenderer.
        session.sources.insert(
            id,
            CaptureOwner {
                selection: CaptureSelection::Physical {
                    endpoint_id: endpoint.into(),
                },
                session: None,
                observer,
                gate,
                status: SourceStatus::Staged,
                enabled: true,
                report: None,
                failure: None,
                last_start_failure: None,
            },
        );
        inputs.push((id, writer));
    }
    session.publish().unwrap();
    let (engine, reader) = renderer.into_parts();
    let mut renderer = DemandRenderer::new(engine, reader).unwrap();
    renderer.set_observer(session.observer.clone()).unwrap();
    (session, renderer, inputs)
}
fn render(renderer: &mut DemandRenderer, inputs: &mut [(SourceId, AudioWriter<f32>)]) -> [f32; 2] {
    for (_, writer) in inputs {
        writer.write_interleaved(&[0.5, 0.5]).unwrap();
    }
    let mut out = [0.0; 2];
    renderer.render_interleaved(&mut out).unwrap();
    out
}
fn finish(mut session: HostSession, renderer: DemandRenderer) -> Vec<HostEvent> {
    let (engine, reader) = renderer.into_parts();
    session.host.take().unwrap().finish_parts(engine, reader)
}
#[test]
fn native_observation_drives_runtime_and_stopping_one_source_preserves_the_other() {
    let (mut session, mut renderer, mut inputs) = fixture();
    assert_eq!(render(&mut renderer, &mut inputs), [0.05, 0.05]);
    session.poll();
    let runtime = session.runtime_snapshot();
    assert_eq!(runtime.timeline, 1);
    assert_eq!(runtime.rendered_blocks, 1);
    assert_eq!(runtime.peak_amplitude, 0.05);
    assert!(
        !runtime.stream_started,
        "software demand must not pretend to start a device"
    );
    let first = inputs[0].0;
    let second = inputs[1].0;
    session.stop_source(first).unwrap();
    assert_eq!(session.sources[&first].status, SourceStatus::Stopped);
    assert!(!session.sources[&first].gate.is_available());
    assert!(session.sources[&second].gate.is_available());
    assert_eq!(render(&mut renderer, &mut inputs), [0.025, 0.025]);
    session.enable_source(second, false).unwrap();
    assert_eq!(render(&mut renderer, &mut inputs), [0.0, 0.0]);
    finish(session, renderer);
}
#[test]
fn failed_source_preparation_preserves_active_plan_and_existing_gates() {
    let (mut session, mut renderer, mut inputs) = fixture();
    render(&mut renderer, &mut inputs);
    session.poll();
    let revision = session.runtime_snapshot().active_revision;
    let invalid = CaptureSelection::Physical {
        endpoint_id: String::new(),
    };
    assert!(session.add_source(invalid.clone()).is_err());
    assert!(session.replace_source(inputs[0].0, invalid).is_err());
    assert!(session.sources[&inputs[0].0].last_start_failure.is_some());
    assert!(session.sources.values().all(|s| s.gate.is_available()));
    assert_eq!(session.runtime_snapshot().active_revision, revision);
    assert_eq!(render(&mut renderer, &mut inputs), [0.05, 0.05]);
    finish(session, renderer);
}
#[test]
fn exited_process_requires_new_selection_and_shutdown_returns_future_receipt() {
    let (mut session, mut renderer, mut inputs) = fixture();
    render(&mut renderer, &mut inputs);
    session.poll();
    let id = inputs[0].0;
    let source = session.sources.get_mut(&id).unwrap();
    source.selection = CaptureSelection::Process {
        identity: ProcessIdentity {
            pid: 42,
            creation_time_100ns: 100,
            executable_name: "test".into(),
        },
    };
    source.status = SourceStatus::TargetExited;
    source.gate.set_available(false);
    assert!(matches!(
        session.restart_source(id),
        Err(SessionError::ProcessRestartNeedsSelection)
    ));
    let gain = session.graph_snapshot().unwrap().sources[0].gain_node;
    let reply = session
        .submit_parameter(
            gain,
            moiren_engine::processor::Gain::LEVEL,
            ParamValue::Float(0.1),
            ApplyAt::Frame(1000),
            0,
        )
        .unwrap();
    assert_eq!(reply.code, ReplyCode::Accepted);
    let events = finish(session, renderer);
    assert!(events.iter().any(|event|matches!(event,HostEvent::Parameter(r) if r.request_id==reply.request_id && r.code!=ReplyCode::Accepted)));
}
