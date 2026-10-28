//! Aligned parameter captions, dials, numeric fields and units.
use super::*;

pub(super) fn parameter_row(
    shell: &StudioShell,
    node: &GraphNodeState,
    p: Parameter,
    cx: &Context<StudioShell>,
) -> AnyElement {
    parameter_control(shell, node, p, p != Parameter::Level, cx)
}

pub(super) fn parameter_control(
    shell: &StudioShell,
    node: &GraphNodeState,
    p: Parameter,
    rotary: bool,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let key = (node.id, p);
    let mut row = div()
        .id(format!("card-param-{:?}-{p:?}", node.id))
        .flex()
        .flex_col()
        .gap(px(2.));
    row = if rotary {
        row.w(px(88.)).items_center().flex_none()
    } else {
        row.w_full()
    };
    if let (Some(input), Some(slot)) = (
        shell.node_controls.inputs.get(&key),
        shell.node_controls.sliders.get(&key),
    ) {
        let label = if p == Parameter::Level
            && matches!(
                node.source,
                Some(InputSelection::Physical { .. }) | Some(InputSelection::Process { .. })
            ) {
            "Digital trim"
        } else {
            p.label()
        };
        if rotary {
            return dial_field(
                format!("card-param-{:?}-{p:?}", node.id),
                label,
                p.unit(),
                input,
                rotary::knob(shell, node, p, cx),
                shell.node_controls.errors.get(&key),
            );
        } else {
            row = row
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(4.))
                        .child(note(label))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(3.))
                                .child(
                                    div().w(px(70.)).child(Styled::h(
                                        component::input::Input::new(input)
                                            .appearance(false)
                                            .px(px(3.))
                                            .py(px(0.))
                                            .text_size(px(11.))
                                            .text_right(),
                                        px(22.),
                                    )),
                                )
                                .child(
                                    div()
                                        .w(px(22.))
                                        .text_size(px(10.))
                                        .text_color(theme::TEXT_FAINT)
                                        .child(p.unit()),
                                ),
                        ),
                )
                .child(hslider(&slot.state, &slot.behavior, cx));
        }
    }
    if let Some(error) = shell.node_controls.errors.get(&key) {
        row = row.child(
            div()
                .text_color(theme::DANGER)
                .text_size(px(10.))
                .child(error.clone()),
        );
    }
    row.gpui_test_observe().into_any_element()
}

/// The renderer does not own the target or input state. Rack stage controls
/// reuse this alignment while keeping each surface's edits and focus distinct.
pub(super) fn dial_field(
    id: String,
    label: &str,
    unit: &'static str,
    input: &Entity<component::input::InputState>,
    knob: AnyElement,
    error: Option<&String>,
) -> AnyElement {
    let label_id = format!("{id}-label");
    let unit_id = format!("{id}-unit");
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(2.))
        .w(px(88.))
        .items_center()
        .flex_none()
        .child(
            div()
                .id(label_id)
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(note(label.to_owned()))
                .gpui_test_observe(),
        )
        .child(knob)
        .child(
            div()
                .relative()
                .flex_none()
                .w(px(88.))
                .h(px(24.))
                .child(
                    div().absolute().left(px(12.)).w(px(64.)).child(Styled::h(
                        component::input::Input::new(input)
                            .appearance(false)
                            .px(px(0.))
                            .py(px(0.))
                            .text_size(px(11.))
                            .text_center(),
                        px(22.),
                    )),
                )
                .child(
                    div()
                        .id(unit_id)
                        .absolute()
                        .left(px(76.))
                        .top_0()
                        .w(px(12.))
                        .h(px(22.))
                        .font_family(theme::FONT_MONO)
                        .text_size(px(9.))
                        .line_height(px(22.))
                        .whitespace_nowrap()
                        .text_color(theme::TEXT_FAINT)
                        .child(unit)
                        .gpui_test_observe(),
                ),
        )
        .when_some(error, |view, error| {
            view.child(
                div()
                    .text_color(theme::DANGER)
                    .text_size(px(10.))
                    .child(error.clone()),
            )
        })
        .gpui_test_observe()
        .into_any_element()
}
