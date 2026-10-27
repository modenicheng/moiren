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
            row = row
                .child(
                    div()
                        .h(px(28.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(note(label)),
                )
                .child(rotary::knob(shell, node, p, cx))
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .w(px(88.))
                        .h(px(24.))
                        .child(
                            div().absolute().left(px(8.)).w(px(64.)).child(
                                component::input::Input::new(input)
                                    .appearance(false)
                                    .px(px(0.))
                                    .py(px(0.))
                                    .h(px(22.))
                                    .text_size(px(11.))
                                    .text_center(),
                            ),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(72.))
                                .top(px(3.))
                                .w(px(16.))
                                .text_size(px(9.))
                                .line_height(px(22.))
                                .text_color(theme::TEXT_FAINT)
                                .child(p.unit()),
                        ),
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
                                    div().w(px(70.)).child(
                                        component::input::Input::new(input)
                                            .appearance(false)
                                            .px(px(3.))
                                            .py(px(0.))
                                            .h(px(22.))
                                            .text_size(px(11.))
                                            .text_right(),
                                    ),
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
