use super::*;

#[::core::prelude::v1::test]
fn mapper_presets_respect_actual_input_and_output_widths() {
    for channels in [1, 2, 4, 8] {
        let identity = mapper_preset(channels, channels, false).unwrap();
        assert!(identity.valid_layout(channels, channels));
        assert_eq!(identity.mapping, (0..channels).collect::<Vec<_>>());
    }
    assert!(mapper_preset(0, 0, false).is_none());
    assert!(mapper_preset(257, 257, false).is_none());
    assert!(mapper_preset(1, 1, true).is_none());
    assert!(mapper_preset(2, 1, false).is_none());
    assert!(mapper_preset(1, 2, true).is_none());
    let swap = mapper_preset(4, 4, true).unwrap();
    assert_eq!(swap.mapping, [1, 0, 2, 3]);
    assert!(swap.valid_layout(4, 4));
}

#[::core::prelude::v1::test]
fn matrix_projects_real_mapping_without_inventing_rectangular_identity() {
    assert_eq!(
        mapper_matrix(&ChannelMapperSettings::default(), 2, 2).unwrap(),
        vec![vec![1., 0.], vec![0., 1.]]
    );
    assert!(mapper_matrix(&ChannelMapperSettings::default(), 2, 1).is_none());
    let copied = ChannelMapperSettings {
        mapping: vec![1, 1, 0],
        matrix: Vec::new(),
    };
    assert_eq!(
        mapper_matrix(&copied, 2, 3).unwrap(),
        vec![vec![0., 1.], vec![0., 1.], vec![1., 0.]]
    );
    let weighted = ChannelMapperSettings {
        mapping: Vec::new(),
        matrix: vec![vec![0.5, -0.25], vec![0., 0.]],
    };
    assert_eq!(mapper_matrix(&weighted, 2, 2).unwrap(), weighted.matrix);
    assert!(mapper_matrix(&weighted, 1, 2).is_none());
}

#[::core::prelude::v1::test]
fn matrix_weight_edits_preserve_other_cells_and_signed_headroom() {
    let matrix = vec![vec![2., -0.5], vec![0., 1.]];
    let changed = adjusted_matrix(&matrix, 0, 0, 0.25).unwrap();
    assert!(changed.mapping.is_empty());
    assert_eq!(changed.matrix, vec![vec![2.25, -0.5], vec![0., 1.]]);
    assert_eq!(matrix[0][0], 2.);
    assert_eq!(
        adjusted_matrix(&matrix, 0, 1, -0.25).unwrap().matrix[0][1],
        -0.75
    );
    assert!(adjusted_matrix(&matrix, 2, 0, 0.25).is_none());
    assert!(adjusted_matrix(&matrix, 0, 0, f64::NAN).is_none());
    assert!(adjusted_matrix(&[vec![f64::MAX]], 0, 0, f64::MAX).is_none());
}
