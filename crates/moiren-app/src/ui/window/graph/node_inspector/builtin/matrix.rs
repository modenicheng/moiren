//! Weighted channel routing edits preserve all other matrix cells.
use super::*;

pub(super) fn controls(
    id: moiren_core::graph::NodeId,
    node: &moiren_core::graph::Node,
    settings: &ChannelMapperSettings,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let mut view = div().flex().flex_col().gap(px(theme::SPACE_2));
    let mapping = if settings.mapping.is_empty() && settings.matrix.is_empty() {
        t!("Identity routing")
    } else if !settings.matrix.is_empty() {
        t!("Weighted matrix")
    } else {
        t!("Explicit channel map")
    };
    view = view
        .child(heading(t!("Channel mapper")))
        .child(detail(&t!("Mode"), mapping))
        .child(label(t!("Static routing is prepared before audio starts.")));
    let input_channels = node.inputs().first().map_or(0, |port| port.channels());
    let output_channels = node.channels();
    if input_channels == 2 && output_channels == 2 {
        view = view.child(button("channel-map-swap", t!("Swap L/R")).on_click(edit(
            cx.entity(),
            GraphEdit::SetBuiltin {
                node: id,
                state: crate::service::BuiltinState::ChannelMapper(ChannelMapperSettings {
                    mapping: vec![1, 0],
                    matrix: Vec::new(),
                }),
            },
        )));
    }
    view =
        view.child(
            button("channel-map-identity", t!("Identity")).on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::ChannelMapper(
                        ChannelMapperSettings::default(),
                    ),
                },
            )),
        );
    if input_channels <= MAX_MATRIX_EDITOR_CHANNELS && output_channels <= MAX_MATRIX_EDITOR_CHANNELS
    {
        let matrix = channel_mapper_matrix(settings, input_channels, output_channels);
        if settings.matrix.is_empty() {
            view = view.child(
                button("channel-map-edit-matrix", t!("Edit weighted matrix")).on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::ChannelMapper(ChannelMapperSettings {
                            mapping: Vec::new(),
                            matrix: matrix.clone(),
                        }),
                    },
                )),
            );
        } else {
            view = view
                .child(heading(t!("Weight matrix")))
                .child(detail(
                    &t!("Layout"),
                    t!(
                        "%{outputs} outputs x %{inputs} inputs",
                        outputs = output_channels,
                        inputs = input_channels
                    ),
                ))
                .child(label(t!(
                    "Adjust one output row at a time. Values are clamped to -1..1."
                )));
            for output in 0..output_channels {
                let mut row = div()
                    .id(format!("channel-map-matrix-row-{output}"))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(4.));
                row = row.child(label(t!("Out %{index}", index = output + 1)));
                for input in 0..input_channels {
                    let value = matrix[output][input];
                    let down = matrix_cell_edit(id, &matrix, output, input, -0.25);
                    let up = matrix_cell_edit(id, &matrix, output, input, 0.25);
                    row = row.child(
                        div()
                            .id(format!("channel-map-matrix-cell-{output}-{input}"))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(2.))
                            .child(label(t!("In %{index}", index = input + 1)))
                            .child(
                                button(
                                    format!("channel-map-matrix-out-{output}-in-{input}-up"),
                                    "+",
                                )
                                .disabled(value >= 1.)
                                .on_click(edit(cx.entity(), up)),
                            )
                            .child(label(format!("{value:+.2}")).text_color(theme::TEXT))
                            .child(
                                button(
                                    format!("channel-map-matrix-out-{output}-in-{input}-down"),
                                    "−",
                                )
                                .disabled(value <= -1.)
                                .on_click(edit(cx.entity(), down)),
                            ),
                    );
                }
                view = view.child(row);
            }
        }
    } else {
        view = view.child(label(
                    t!("Matrix editor is limited to 8 input and output channels; use a project file for larger layouts."),
                ));
    }

    view.into_any_element()
}

pub(super) fn channel_mapper_matrix(
    settings: &ChannelMapperSettings,
    input_channels: usize,
    output_channels: usize,
) -> Vec<Vec<f64>> {
    if !settings.matrix.is_empty() {
        return settings.matrix.clone();
    }
    (0..output_channels)
        .map(|output| {
            let source = settings.mapping.get(output).copied().unwrap_or(output);
            (0..input_channels)
                .map(|input| f64::from((input == source) as u8))
                .collect()
        })
        .collect()
}

pub(super) fn matrix_cell_edit(
    node: moiren_core::graph::NodeId,
    matrix: &[Vec<f64>],
    output: usize,
    input: usize,
    delta: f64,
) -> GraphEdit {
    let mut matrix = matrix.to_vec();
    matrix[output][input] = (matrix[output][input] + delta).clamp(-1., 1.);
    GraphEdit::SetBuiltin {
        node,
        state: crate::service::BuiltinState::ChannelMapper(ChannelMapperSettings {
            mapping: Vec::new(),
            matrix,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{channel_mapper_matrix, matrix_cell_edit};
    use crate::service::GraphEdit;
    use moiren_core::graph::{LogicalGraph, NodeKind};
    use moiren_engine::processor::ChannelMapperSettings;

    #[::core::prelude::v1::test]
    fn matrix_editor_starts_from_identity_or_explicit_channel_mapping() {
        assert_eq!(
            channel_mapper_matrix(&ChannelMapperSettings::default(), 2, 2),
            vec![vec![1., 0.], vec![0., 1.]]
        );
        let settings = ChannelMapperSettings {
            mapping: vec![1, 0],
            matrix: Vec::new(),
        };
        assert_eq!(
            channel_mapper_matrix(&settings, 2, 2),
            vec![vec![0., 1.], vec![1., 0.]]
        );
        let settings = ChannelMapperSettings {
            mapping: Vec::new(),
            matrix: vec![vec![0.5, -0.5]],
        };
        assert_eq!(channel_mapper_matrix(&settings, 2, 1), settings.matrix);
    }

    #[::core::prelude::v1::test]
    fn matrix_edits_clamp_one_cell_without_mutating_the_snapshot() {
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(NodeKind::ChannelMapper, 2).unwrap();
        let matrix = vec![vec![1., 0.5], vec![-1., 1.]];
        for (row, column, delta, expected) in
            [(0, 0, 0.25, 1.), (1, 0, -0.25, -1.), (0, 1, 0.25, 0.75)]
        {
            let GraphEdit::SetBuiltin {
                node: edited,
                state: crate::service::BuiltinState::ChannelMapper(settings),
            } = matrix_cell_edit(node, &matrix, row, column, delta)
            else {
                panic!("expected matrix edit")
            };
            assert_eq!(edited, node);
            assert!(settings.mapping.is_empty());
            let mut wanted = matrix.clone();
            wanted[row][column] = expected;
            assert_eq!(settings.matrix, wanted);
        }
        assert_eq!(matrix, vec![vec![1., 0.5], vec![-1., 1.]]);
    }
}
