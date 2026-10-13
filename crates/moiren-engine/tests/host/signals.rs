use super::support::*;
use moiren_core::graph::{NodeKind, SendParams};
use moiren_engine::{host::*, processor::CompressorSettings};
use std::time::{Duration, Instant};

#[test]
fn signals_attach_automatically_and_retained_subscriber_does_not_hold_owner_alive() {
    let (mut project, input, gain, output) = project();
    project.graph.remove_node(gain).unwrap();
    project.nodes.remove(&gain);
    let compressor = project.graph.create_node(NodeKind::Compressor, 1).unwrap();
    project.nodes.insert(
        compressor,
        NodeConfig::Compressor(CompressorSettings::default()),
    );
    let source = project.graph.get_node(input).unwrap().outputs()[0].id();
    let destination = project.graph.get_node(output).unwrap().inputs()[0].id();
    let compressor_input = project.graph.get_node(compressor).unwrap().inputs()[0].id();
    let compressor_output = project.graph.get_node(compressor).unwrap().outputs()[0].id();
    project
        .graph
        .connect(source, compressor_input, SendParams::default())
        .unwrap();
    project
        .graph
        .connect(compressor_output, destination, SendParams::default())
        .unwrap();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    let snapshot = wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let mut subscription = observer.subscribe_compressor(compressor).unwrap();
    assert_eq!(subscription.owner_generation, snapshot.owner_generation);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if subscription.subscriber.read_latest().is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(host);
    assert_eq!(observer.snapshot().host, HostState::Closed);
    assert_eq!(
        subscription.subscriber.status().publisher,
        moiren_signal::PublisherStatus::NoPublisher
    );
}
