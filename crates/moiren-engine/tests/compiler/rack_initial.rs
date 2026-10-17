use super::*;
use moiren_engine::rack::{RackProcessor, RackSettings, RackSlot};

#[test]
fn rack_parameter_initial_is_rendered_before_control_events() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let rack = graph.create_node(NodeKind::Rack, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    connect(&mut graph, source, rack, Default::default());
    connect(&mut graph, rack, sink, Default::default());

    let (writer, mut reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 1);
    bindings.bind_sink(sink, writer).unwrap();
    bindings
        .bind_rack(
            rack,
            RackSettings {
                slots: vec![RackSlot {
                    id: 42,
                    bypass: false,
                    processor: RackProcessor::Gain { initial: 2.0 },
                }],
                ..Default::default()
            },
        )
        .unwrap();
    bindings
        .bind_rack_parameter(rack, 42, Gain::LEVEL, ParamValue::Float(0.25))
        .unwrap();

    let mut compiled = compile(&graph, bindings, config()).unwrap();
    compiled.engine.render(2).unwrap();
    let mut samples = [0.0; 2];
    reader.read_interleaved(&mut samples).unwrap();
    assert_samples(&samples, &[0.09375, 0.09375]);
}
