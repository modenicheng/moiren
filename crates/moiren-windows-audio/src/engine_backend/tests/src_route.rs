//! Preparation provenance only; these controls do not acquire native audio.
use super::*;
use moiren_engine::processor::{ChannelMode, StripConfig};

fn bus_project() -> EngineProject {
    let mut project = virtual_project(2, 48_000);
    let direct = project.graph.edges()[0].clone();
    project.graph.disconnect(direct.id()).unwrap();
    let bus = project.graph.create_node(NodeKind::Bus, 2).unwrap();
    let input = project.graph.add_input_port(bus, 2).unwrap();
    let output = project.graph.get_node(bus).unwrap().outputs()[0].id();
    project
        .graph
        .connect(direct.src_port(), input, Default::default())
        .unwrap();
    project
        .graph
        .connect(output, direct.dst_port(), Default::default())
        .unwrap();
    project.nodes.insert(bus, NodeConfig::Bus);
    project
}

#[test]
fn src_route_matches_exact_prepared_bus_topology_not_matching_labels() {
    let project = bus_project();
    let route = native_observations::src_route::prepared_route(&project, 41, 17).unwrap();
    assert_eq!(route.prepared_graph_revision, 41);
    assert_eq!(route.prepared_plan_revision, 17);
    assert_eq!(route.schema_version, 2);
    assert_eq!(route.engine_sample_rate, 48_000);
    assert_eq!(route.engine_block_frames, 256);
    assert_eq!(route.channels, 2);
    assert_ne!(route.source, route.bus);
    assert_ne!(route.source, route.sink);
    assert_eq!(
        route.source_endpoint.to_string(),
        "01010101010101010101010101010101"
    );
    assert_eq!(
        route.sink_endpoint.to_string(),
        "02020202020202020202020202020202"
    );
    assert!(
        native_observations::src_route::prepared_route(&virtual_project(2, 48_000), 41, 17)
            .is_none()
    );
    let mut wrong = project.clone();
    let edge = wrong.graph.edges()[0].id();
    wrong.graph.disconnect(edge).unwrap();
    assert!(native_observations::src_route::prepared_route(&wrong, 41, 17).is_none());
    let mut extra = project.clone();
    let gain = extra.graph.create_node(NodeKind::Gain, 2).unwrap();
    extra.nodes.insert(gain, NodeConfig::Gain { initial: 1.0 });
    assert!(native_observations::src_route::prepared_route(&extra, 41, 17).is_none());
}

#[test]
fn src_route_refuses_nonidentity_controls_or_extra_bus_inputs() {
    use moiren_core::graph::SendParams;
    for params in [
        SendParams {
            gain: 0.5,
            ..Default::default()
        },
        SendParams {
            pan: 0.5,
            ..Default::default()
        },
        SendParams {
            mute: true,
            ..Default::default()
        },
        SendParams {
            tap: moiren_core::graph::SendTap::PreFader,
            ..Default::default()
        },
    ] {
        let mut project = bus_project();
        let edge = project.graph.edges()[0].id();
        project.graph.set_send_params(edge, params).unwrap();
        assert!(native_observations::src_route::prepared_route(&project, 1, 17).is_none());
    }
    for strip in [
        StripConfig {
            gain: 0.5,
            ..Default::default()
        },
        StripConfig {
            balance: 0.5,
            ..Default::default()
        },
        StripConfig {
            muted: true,
            ..Default::default()
        },
        StripConfig {
            mode: ChannelMode::Mono,
            ..Default::default()
        },
    ] {
        let mut project = bus_project();
        let node = project.graph.nodes()[0].id();
        project.strips.insert(node, strip);
        assert!(native_observations::src_route::prepared_route(&project, 1, 17).is_none());
    }
    let mut project = bus_project();
    let bus = project
        .graph
        .nodes()
        .iter()
        .find(|n| n.kind() == NodeKind::Bus)
        .unwrap()
        .id();
    project.graph.add_input_port(bus, 2).unwrap();
    assert!(native_observations::src_route::prepared_route(&project, 1, 17).is_none());
}

#[test]
fn src_route_preparation_rejects_desired_controls_and_preserves_explicit_identity_strips() {
    use moiren_core::protocol::ParamValue;
    use moiren_engine::host::{DesiredParameter, ParameterTarget, StripParameter};
    let mut project = bus_project();
    let node = project.graph.nodes()[0].id();
    project.strips.insert(node, StripConfig::default());
    assert!(native_observations::src_route::prepared_route(&project, 1, 17).is_some());
    project.parameters.push(DesiredParameter {
        target: ParameterTarget::Strip {
            node,
            parameter: StripParameter::Gain,
        },
        value: ParamValue::Float(0.5),
    });
    assert!(native_observations::src_route::prepared_route(&project, 1, 17).is_none());
}

#[test]
fn src_route_preparation_keeps_backend_rate_and_block_authority() {
    for channels in [1, 2] {
        for rate in SAMPLE_RATES {
            let mut project = bus_project();
            project.processing.engine.processing_sr = f64::from(rate);
            if channels == 1 {
                // Rebuild all port widths, not just the IO labels.
                project = virtual_project(1, rate);
                let edge = project.graph.edges()[0].clone();
                project.graph.disconnect(edge.id()).unwrap();
                let bus = project.graph.create_node(NodeKind::Bus, 1).unwrap();
                let input = project.graph.add_input_port(bus, 1).unwrap();
                let output = project.graph.get_node(bus).unwrap().outputs()[0].id();
                project
                    .graph
                    .connect(edge.src_port(), input, Default::default())
                    .unwrap();
                project
                    .graph
                    .connect(output, edge.dst_port(), Default::default())
                    .unwrap();
                project.nodes.insert(bus, NodeConfig::Bus);
            }
            let route = native_observations::src_route::prepared_route(&project, 9, 17).unwrap();
            assert_eq!(route.channels, channels);
            assert_eq!(route.engine_sample_rate, rate);
        }
    }
    for rate in [8000.0, 48000.5, 384000.0] {
        let mut project = bus_project();
        project.processing.engine.processing_sr = rate;
        assert!(native_observations::src_route::prepared_route(&project, 1, 17).is_none());
    }
    let mut project = bus_project();
    project.processing.engine.max_block_frames = 4097;
    assert!(native_observations::src_route::prepared_route(&project, 1, 17).is_none());
}

#[test]
fn src_route_preparation_never_relabels_other_input_or_output_paths() {
    let mut project = bus_project();
    let NodeConfig::Input(input) = project
        .nodes
        .values_mut()
        .find(|c| matches!(c, NodeConfig::Input(_)))
        .unwrap()
    else {
        unreachable!()
    };
    input.source = InputSource::Software {
        name: "not-virtual".into(),
    };
    assert!(native_observations::src_route::prepared_route(&project, 1, 17).is_none());
    let mut project = bus_project();
    let NodeConfig::Output(output) = project
        .nodes
        .values_mut()
        .find(|c| matches!(c, NodeConfig::Output(_)))
        .unwrap()
    else {
        unreachable!()
    };
    output.target = OutputTarget::Physical(EndpointSelector::PinnedEndpoint {
        endpoint_id: "not-opened".into(),
        stable_id: None,
    });
    assert!(native_observations::src_route::prepared_route(&project, 1, 17).is_none());
}

#[test]
fn src_route_backend_lifecycle_retains_preparation_and_sticky_update_invalidation() {
    let project = bus_project();
    let probe = Arc::new(Probe::default());
    let backend = backend::<f32>(&probe);
    let mut signals = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = signals.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(
            request(&project, &host),
            &cancel,
            &native_devices(Default::default()),
        )
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    let retained = backend.native_observations().retain_session().unwrap();
    let first = retained.src_route();
    assert!(!first.invalidated);
    assert_eq!(first.prepared.as_ref().unwrap().prepared_graph_revision, 1);
    let mut collector = retained.src_trace_collector(64).unwrap();
    backend.native_observations().invalidate_src_route(8);
    assert_eq!(retained.src_route(), first);
    let mut update = request(&project, &host);
    update.mode = PrepareMode::Update;
    update.graph_revision = 2;
    // No reuse: this update fails. Even an unsuccessful preparation must not
    // leave a prepared witness looking like active-plan confirmation.
    assert!(
        backend
            .prepare_with_native(
                update,
                &CancellationToken::default(),
                &native_devices(Default::default())
            )
            .is_err()
    );
    assert!(retained.src_route().invalidated);
    collector.poll();
    assert!(collector.artifact().route.invalidated);
    assert_eq!(collector.artifact().route.prepared, first.prepared);
    let _ = owner.stop_and_join();
    collector.finish();
    assert_eq!(collector.artifact().schema_version, 4);
    assert!(collector.artifact().route.invalidated);
    assert_eq!(collector.artifact().route.prepared, first.prepared);
}

#[test]
fn src_route_retained_owner_cannot_follow_registry_replacement() {
    let project = bus_project();
    let probe = Arc::new(Probe::default());
    let backend = backend::<f32>(&probe);
    let mut signals = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = signals.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(
            request(&project, &host),
            &cancel,
            &native_devices(Default::default()),
        )
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    let old = backend.native_observations().retain_session().unwrap();
    let prepared = old.src_route().prepared.unwrap();
    backend
        .native_observations()
        .publish(8, Vec::new(), Vec::new(), Vec::new());
    let current = backend.native_observations().retain_session().unwrap();
    assert!(current.src_route().prepared.is_none());
    backend.native_observations().invalidate_src_route(8);
    assert!(current.src_route().invalidated);
    assert!(!old.src_route().invalidated);
    assert_eq!(old.src_route().prepared.unwrap(), prepared);
    let _ = owner.stop_and_join();
    assert_eq!(
        backend
            .native_observations()
            .retain_session()
            .unwrap()
            .snapshot()
            .owner_generation,
        8
    );
}
