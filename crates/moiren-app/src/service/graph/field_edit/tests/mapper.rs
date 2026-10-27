use super::*;

fn settings(service: &ServiceGraph, node: NodeId) -> ChannelMapperSettings {
    builtin(service, node).channel_mapper().unwrap().clone()
}

fn weight(node: NodeId, input: usize, output: usize, weight: f64) -> GraphEdit {
    GraphEdit::SetMapperWeight {
        node,
        input,
        output,
        weight,
    }
}

#[test]
fn mapper_queued_cells_preserve_rectangular_routing_and_accept_signed_unbounded_weights() {
    let (mut service, node) = fixture(
        NodeKind::ChannelMapper,
        3,
        2,
        BuiltinState::ChannelMapper(ChannelMapperSettings {
            mapping: vec![2, 0],
            matrix: Vec::new(),
        }),
    );
    let topology = service.snapshot().graph;
    assert!(service.edit(weight(node, 1, 0, -2.5)).unwrap());
    assert!(service.edit(weight(node, 2, 1, 3.25)).unwrap());
    let mapper = settings(&service, node);
    assert!(mapper.mapping.is_empty());
    assert_eq!(mapper.matrix, [vec![0., -2.5, 1.], vec![1., 0., 3.25]]);
    assert!(mapper.valid_layout(3, 2));
    assert_eq!(service.snapshot().graph, topology);
    assert_eq!(
        service.snapshot().node(node).unwrap().label,
        "Preserved node label"
    );
    service.edit(weight(node, 0, 0, f64::MAX)).unwrap();
    assert_eq!(settings(&service, node).matrix[0][0], f64::MAX);
}

#[test]
fn mapper_empty_settings_expand_identity_without_overwriting_other_cells() {
    let (mut service, node) = fixture(NodeKind::ChannelMapper, 3, 3, BuiltinState::None);
    service.edit(weight(node, 2, 0, -0.5)).unwrap();
    assert_eq!(
        settings(&service, node).matrix,
        [vec![1., 0., -0.5], vec![0., 1., 0.], vec![0., 0., 1.]]
    );
}

#[test]
fn mapper_cell_undo_coalesces_only_the_same_cell_and_redo_preserves_other_edits() {
    let (mut service, node) = fixture(NodeKind::ChannelMapper, 2, 2, BuiltinState::None);
    for value in [-0.25, -0.75] {
        service.edit(weight(node, 1, 0, value)).unwrap();
    }
    service.edit(weight(node, 0, 1, 2.)).unwrap();
    assert_eq!(service.journal.undo.len(), 2);
    assert!(service.undo());
    assert_eq!(
        settings(&service, node).matrix,
        [vec![1., -0.75], vec![0., 1.]]
    );
    assert!(service.undo());
    assert_eq!(builtin(&service, node), BuiltinState::None);
    assert!(service.redo());
    assert!(service.redo());
    assert_eq!(
        settings(&service, node).matrix,
        [vec![1., -0.75], vec![2., 1.]]
    );
}

#[test]
fn mapper_presets_use_actual_width_swap_only_first_two_and_keep_discrete_undo() {
    let (mut service, node) = fixture(NodeKind::ChannelMapper, 4, 4, BuiltinState::None);
    service.edit(weight(node, 3, 0, -3.)).unwrap();
    assert!(
        service
            .edit(GraphEdit::MapperPreset { node, swap: true })
            .unwrap()
    );
    assert_eq!(
        settings(&service, node),
        ChannelMapperSettings {
            mapping: vec![1, 0, 2, 3],
            matrix: Vec::new(),
        }
    );
    assert!(
        service
            .edit(GraphEdit::MapperPreset { node, swap: false })
            .unwrap()
    );
    assert_eq!(settings(&service, node), ChannelMapperSettings::identity(4));
    assert_eq!(service.journal.undo.len(), 3);
    assert!(service.undo());
    assert_eq!(settings(&service, node).mapping, [1, 0, 2, 3]);
    assert!(service.undo());
    assert_eq!(settings(&service, node).matrix[0][3], -3.);
}

#[test]
fn mapper_invalid_coordinates_nonfinite_values_and_wrong_types_reject_atomically() {
    let (mut service, node) = fixture(
        NodeKind::ChannelMapper,
        3,
        2,
        BuiltinState::ChannelMapper(ChannelMapperSettings {
            mapping: vec![2, 0],
            matrix: Vec::new(),
        }),
    );
    for edit in [
        weight(node, 3, 0, 1.),
        weight(node, 0, 2, 1.),
        weight(node, usize::MAX, usize::MAX, 1.),
        weight(node, 0, 0, f64::NAN),
        weight(node, 0, 0, f64::INFINITY),
        weight(node, 0, 0, f64::NEG_INFINITY),
        GraphEdit::MapperPreset { node, swap: false },
        GraphEdit::MapperPreset { node, swap: true },
    ] {
        assert_rejected(&mut service, edit);
    }
    let (mut service, node) = fixture(NodeKind::Gain, 2, 2, BuiltinState::None);
    assert_rejected(&mut service, weight(node, 0, 0, 1.));
    assert_rejected(&mut service, GraphEdit::MapperPreset { node, swap: false });
    let (mut service, node) = fixture(NodeKind::ChannelMapper, 1, 1, BuiltinState::None);
    assert_rejected(&mut service, GraphEdit::MapperPreset { node, swap: true });
    service
        .edit(GraphEdit::MapperPreset { node, swap: false })
        .unwrap();
    assert_eq!(settings(&service, node), ChannelMapperSettings::identity(1));
}

#[test]
fn mapper_256_channel_last_cell_uses_real_port_bounds() {
    let (mut service, node) = fixture(NodeKind::ChannelMapper, 256, 256, BuiltinState::None);
    service.edit(weight(node, 255, 255, -1.5)).unwrap();
    let mapper = settings(&service, node);
    assert_eq!(mapper.matrix[255][255], -1.5);
    assert_eq!(mapper.matrix[0][0], 1.);
    assert_rejected(&mut service, weight(node, 256, 255, 0.));
}
