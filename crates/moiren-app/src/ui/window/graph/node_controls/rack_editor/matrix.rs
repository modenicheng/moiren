//! Paged signed routing weights, bounded to sixteen precision fields per pane.
use super::*;
use model::Field;
use parameters::PAGE_SIZE;

pub(super) fn content(
    shell: &StudioShell,
    scope: &Scope<'_>,
    slot: u32,
    inputs: usize,
    outputs: usize,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let state = &shell.node_controls.rack;
    let page = state
        .mapper_pages
        .get(&(scope.surface, scope.node))
        .copied()
        .unwrap_or_default();
    let (page, _) = parameters::weight_fields(inputs, outputs, page);
    let columns = page.1 * PAGE_SIZE..((page.1 + 1) * PAGE_SIZE).min(inputs);
    let rows = page.0 * PAGE_SIZE..((page.0 + 1) * PAGE_SIZE).min(outputs);
    let mut table = div().flex().flex_col().gap(px(4.)).w_full();
    let mut header = div().flex().gap(px(4.)).child(div().w(px(42.)).flex_none());
    for input in columns.clone() {
        header = header.child(
            div()
                .flex_1()
                .min_w_0()
                .text_center()
                .child(note(t!("In %{index}", index = input + 1).to_string())),
        );
    }
    table = table.child(header);
    for output in rows {
        let mut row = div().flex().gap(px(4.)).items_center().child(
            div()
                .w(px(42.))
                .flex_none()
                .child(note(t!("Out %{index}", index = output + 1).to_string())),
        );
        for input in columns.clone() {
            let field = Field::Weight { input, output };
            let key = Key {
                surface: scope.surface,
                node: scope.node,
                epoch: scope.epoch,
                slot,
                field,
            };
            let mut cell = div().flex_1().min_w_0().flex().flex_col();
            if let Some(editor) = state.inputs.get(&key) {
                cell = cell.child(Styled::h(
                    component::input::Input::new(editor)
                        .id(scope.id(&format!("weight-{slot}-{output}-{input}")))
                        .appearance(false)
                        .px(px(2.))
                        .py(px(0.))
                        .text_size(px(11.))
                        .text_center()
                        .bg(theme::SURFACE_3),
                    px(26.),
                ));
            }
            if let Some(error) = state.errors.get(&key) {
                cell = cell.child(note(error.clone()).text_color(theme::DANGER));
            }
            row = row.child(cell);
        }
        table = table.child(row);
    }
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .w_full()
        .child(detail_line(
            "Routing weights",
            &t!(
                "%{inputs} in / %{outputs} out",
                inputs = inputs,
                outputs = outputs
            ),
        ))
        .child(
            div()
                .flex()
                .gap(px(4.))
                .child(scope.button(
                    "identity",
                    "Identity",
                    false,
                    RackEdit::MapperPreset { slot, swap: false },
                    cx,
                ))
                .child(scope.button(
                    "swap-preset",
                    "Swap 1/2",
                    false,
                    RackEdit::MapperPreset { slot, swap: true },
                    cx,
                )),
        )
        .child(table)
        .when(inputs > PAGE_SIZE, |view| {
            view.child(pager(scope, page, true, inputs, cx))
        })
        .when(outputs > PAGE_SIZE, |view| {
            view.child(pager(scope, page, false, outputs, cx))
        })
        .child(note("Signed weights; negative values invert polarity"))
        .into_any_element()
}

fn pager(
    scope: &Scope<'_>,
    page: (usize, usize),
    horizontal: bool,
    count: usize,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let current = if horizontal { page.1 } else { page.0 };
    let total = count.div_ceil(PAGE_SIZE);
    let mut view = div().flex().items_center().gap(px(4.)).child(
        div().flex_1().child(note(
            t!(
                if horizontal {
                    "Input page %{page} / %{total}"
                } else {
                    "Output page %{page} / %{total}"
                },
                page = current + 1,
                total = total
            )
            .to_string(),
        )),
    );
    for (label, target) in [
        ("<", current.checked_sub(1)),
        (">", (current + 1 < total).then_some(current + 1)),
    ] {
        let node = scope.node;
        let surface = scope.surface;
        let this = cx.entity();
        view = view.child(
            controls::button(scope.id(&format!("page-{horizontal}-{label}")), label)
                .density(Density::Compact)
                .disabled(target.is_none())
                .on_click(move |_, window, cx| {
                    if let Some(target) = target {
                        this.update(cx, |shell, cx| {
                            let page = shell
                                .node_controls
                                .rack
                                .mapper_pages
                                .entry((surface, node))
                                .or_default();
                            if horizontal {
                                page.1 = target
                            } else {
                                page.0 = target
                            }
                            sync(shell, window, cx);
                            cx.notify();
                        });
                    }
                }),
        );
    }
    view.into_any_element()
}
