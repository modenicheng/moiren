//! Prepared channel routing and signed matrix weight controls.
use super::*;
use moiren_engine::processor::ChannelMapperSettings;

const MAX_MATRIX_CHANNELS: usize = 8;

pub(super) fn content(
    shell: &StudioShell,
    node: &GraphNodeState,
    settings: &ChannelMapperSettings,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    let topology = shell
        .state
        .read(cx)
        .snapshot
        .graph
        .graph
        .get_node(node.id)
        .ok()?;
    let inputs = topology.inputs().first()?.channels();
    let outputs = topology.outputs().first()?.channels();
    let mut presets = div().flex().items_center().gap(px(4.));
    for (label, swap) in [("Identity", false), ("Swap 1/2", true)] {
        let preset = mapper_preset(inputs, outputs, swap);
        let selected = preset.as_ref().is_some_and(|preset| preset == settings);
        presets = presets.child(
            choice(
                format!("card-map-{:?}-{swap}", node.id),
                label,
                selected,
                preset.map(|state| builtin_edit(node.id, BuiltinState::ChannelMapper(state))),
                cx,
            )
            .tooltip(t!(if inputs != outputs {
                "Identity and swap require equal input/output widths"
            } else if swap {
                "Swap channels 1 and 2; preserve all remaining channels"
            } else {
                "Route each input to the corresponding output"
            })),
        );
    }
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .w_full()
        .child(detail_line(
            "Routing weights",
            &t!(
                "%{inputs} in / %{outputs} out",
                inputs = inputs,
                outputs = outputs
            ),
        ))
        .child(presets);
    if !settings.valid_layout(inputs, outputs) {
        return Some(
            view.child(
                note("Invalid channel mapping; inspect the node settings.")
                    .text_color(theme::WARNING),
            )
            .into_any_element(),
        );
    }
    if inputs > MAX_MATRIX_CHANNELS || outputs > MAX_MATRIX_CHANNELS {
        let mut routes = div()
            .id(format!("card-map-routes-{:?}", node.id))
            .flex()
            .flex_col()
            .max_h(px(96.))
            .overflow_y_scroll()
            .overflow_x_scroll();
        for output in 0..outputs {
            let sources = if let Some(row) = settings.matrix.get(output) {
                let terms: Vec<_> = row
                    .iter()
                    .enumerate()
                    .filter(|(_, weight)| **weight != 0.)
                    .map(|(input, weight)| {
                        t!(
                            "%{weight} * In %{input}",
                            weight = format!("{weight:+.2}"),
                            input = input + 1
                        )
                        .to_string()
                    })
                    .collect();
                if terms.is_empty() {
                    t!("Silent").to_string()
                } else {
                    terms.join(" ")
                }
            } else {
                t!(
                    "In %{index}",
                    index = settings.mapping.get(output).copied().unwrap_or(output) + 1
                )
                .to_string()
            };
            routes = routes.child(note(
                t!(
                    "Out %{output} <- %{sources}",
                    output = output + 1,
                    sources = sources
                )
                .to_string(),
            ));
        }
        return Some(
            view.child(routes)
                .child(note("Inline matrix editing supports up to 8 x 8 channels."))
                .into_any_element(),
        );
    }
    let matrix = mapper_matrix(settings, inputs, outputs)?;
    let mut header = div().flex().gap(px(4.)).child(div().w(px(38.)).flex_none());
    for input in 0..inputs {
        header = header.child(
            note(t!("In %{index}", index = input + 1).to_string())
                .w(px(54.))
                .flex_none()
                .text_center(),
        );
    }
    let mut grid = div()
        .id(format!("card-map-matrix-{:?}", node.id))
        .max_h(px(320.))
        .overflow_y_scroll()
        .overflow_x_scroll()
        .child(header);
    for (output, weights) in matrix.iter().enumerate() {
        let mut row = div().flex().items_center().gap(px(4.)).mt(px(3.)).child(
            note(t!("Out %{index}", index = output + 1).to_string())
                .w(px(38.))
                .flex_none(),
        );
        for (input, value) in weights.iter().enumerate() {
            let key = format!("card-map-{:?}-{output}-{input}", node.id);
            let mut editor = div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .w(px(194.))
                .child(detail_line(
                    &t!(
                        "Out %{output} / In %{input}",
                        output = output + 1,
                        input = input + 1
                    ),
                    &format!("{value:+.2}"),
                ));
            let mut actions = div().flex().gap(px(6.));
            for (label, delta) in [("-0.25", -0.25), ("+0.25", 0.25)] {
                actions = actions.child(choice(
                    format!("{key}-{label}"),
                    label,
                    false,
                    adjusted_matrix(&matrix, output, input, delta).map(|settings| {
                        builtin_edit(node.id, BuiltinState::ChannelMapper(settings))
                    }),
                    cx,
                ));
            }
            editor = editor
                .child(actions)
                .child(note("Signed linear weight; 0 disconnects this cell."));
            row = row.child(
                div().w(px(54.)).flex_none().child(
                    component::popover::Popover::new(format!("{key}-popup"))
                        .trigger(UtilityTrigger(
                            controls::button(key, format!("{value:+.2}"))
                                .density(Density::Compact)
                                .full_width()
                                .variant(if *value == 0. {
                                    ButtonVariant::Ghost
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .tooltip(t!(
                                    "Edit Out %{output} from In %{input} weight",
                                    output = output + 1,
                                    input = input + 1
                                )),
                        ))
                        .child(editor),
                ),
            );
        }
        grid = grid.child(row);
    }
    view = view.child(grid);
    Some(view.into_any_element())
}

fn mapper_preset(inputs: usize, outputs: usize, swap: bool) -> Option<ChannelMapperSettings> {
    if inputs != outputs || !(1..=256).contains(&inputs) || (swap && inputs < 2) {
        return None;
    }
    let mut settings = ChannelMapperSettings::identity(inputs);
    if swap {
        settings.mapping.swap(0, 1);
    }
    Some(settings)
}

fn mapper_matrix(
    settings: &ChannelMapperSettings,
    inputs: usize,
    outputs: usize,
) -> Option<Vec<Vec<f64>>> {
    if !settings.valid_layout(inputs, outputs) {
        return None;
    }
    if !settings.matrix.is_empty() {
        return Some(settings.matrix.clone());
    }
    Some(
        (0..outputs)
            .map(|output| {
                let source = settings.mapping.get(output).copied().unwrap_or(output);
                (0..inputs)
                    .map(|input| f64::from(u8::from(input == source)))
                    .collect()
            })
            .collect(),
    )
}

fn adjusted_matrix(
    matrix: &[Vec<f64>],
    output: usize,
    input: usize,
    delta: f64,
) -> Option<ChannelMapperSettings> {
    let mut matrix = matrix.to_vec();
    let cell = matrix.get_mut(output)?.get_mut(input)?;
    let next = *cell + delta;
    // The engine supports signed weights and headroom above unity. Do not
    // silently normalize a saved matrix or clamp unrelated cells to +/-1.
    if !next.is_finite() || next == *cell {
        return None;
    }
    *cell = next;
    Some(ChannelMapperSettings {
        mapping: Vec::new(),
        matrix,
    })
}

#[cfg(test)]
mod tests;
