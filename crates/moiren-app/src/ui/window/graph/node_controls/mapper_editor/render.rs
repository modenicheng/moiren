//! Four-by-four signed routing tiles; IDs are shared with native captures.
use super::*;
use model::{Key, PAGE_SIZE, Surface};

pub(super) fn content(
    shell: &StudioShell,
    node: NodeId,
    surface: Surface,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    let routing = model::routing(&shell.state.read(cx).snapshot.graph, node)?;
    let state = &shell.node_controls.mapper;
    let incarnation = state.incarnation(node)?;
    let target = state.target(node)?;
    let root = surface.id(node);
    let page = model::page(
        routing.inputs(),
        routing.outputs(),
        state
            .pages
            .get(&(surface, node))
            .copied()
            .unwrap_or_default(),
    );
    let columns = page.1 * PAGE_SIZE..((page.1 + 1) * PAGE_SIZE).min(routing.inputs());
    let rows = page.0 * PAGE_SIZE..((page.0 + 1) * PAGE_SIZE).min(routing.outputs());
    let mut presets = div().flex().gap(px(4.)).min_w_0();
    for (label, suffix, swap) in [("Identity", "identity", false), ("Swap 1/2", "swap", true)] {
        let this = cx.entity();
        let target = target.clone();
        presets = presets.child(
            controls::button(format!("{root}-{suffix}"), t!(label).to_string())
                .density(Density::Compact)
                .disabled(!routing.preset_allowed(swap) || shell.pending_project.is_some())
                .tooltip(t!(if !routing.preset_allowed(swap) {
                    "Identity and swap require equal input/output widths"
                } else if swap {
                    "Swap channels 1 and 2; preserve all remaining channels"
                } else {
                    "Route each input to the corresponding output"
                }))
                .on_click(move |_, window, cx| {
                    this.update(cx, |shell, cx| {
                        model::preset(shell, &target, swap, window, cx)
                    })
                }),
        );
    }
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
    let mut table = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .w_full()
        .min_w_0()
        .child(header);
    let mut errors = div().flex().flex_col().gap(px(3.)).min_w_0();
    for output in rows {
        let mut row = div().flex().gap(px(4.)).items_center().child(
            div()
                .w(px(42.))
                .flex_none()
                .child(note(t!("Out %{index}", index = output + 1).to_string())),
        );
        for input in columns.clone() {
            let key = Key {
                surface,
                node,
                incarnation,
                output,
                input,
            };
            let id = format!("{root}-weight-{output}-{input}");
            let mut cell = div().id(id.clone()).flex_1().min_w_0();
            if let Some(editor) = state.inputs.get(&key) {
                cell = cell.child(Styled::h(
                    component::input::Input::new(editor)
                        .id(format!("{id}-input"))
                        .appearance(false)
                        .px(px(2.))
                        .py(px(0.))
                        .font_family(theme::FONT_MONO)
                        .text_size(px(11.))
                        .text_center()
                        .bg(theme::SURFACE_3),
                    px(26.),
                ));
            }
            row = row.child(cell.gpui_test_observe());
            if let Some(error) = state.errors.get(&key) {
                // Keep error prose out of a narrow 4-column cell. Its stable ID
                // still identifies the exact field on either editing surface.
                errors = errors.child(
                    note(format!(
                        "{}: {}",
                        t!(
                            "Out %{output} / In %{input}",
                            output = output + 1,
                            input = input + 1
                        ),
                        t!(error.as_str())
                    ))
                    .id(format!("{id}-error"))
                    .text_color(theme::DANGER)
                    .gpui_test_observe(),
                );
            }
        }
        table = table.child(row);
    }
    let mut view = div()
        .id(root.clone())
        .flex()
        .flex_col()
        .gap(px(6.))
        .w_full()
        .min_w_0()
        .child(detail_line(
            "Routing weights",
            &t!(
                "%{inputs} in / %{outputs} out",
                inputs = routing.inputs(),
                outputs = routing.outputs()
            ),
        ))
        .child(presets)
        .child(table);
    for (horizontal, count, current) in [
        (true, routing.inputs(), page.1),
        (false, routing.outputs(), page.0),
    ] {
        if count <= PAGE_SIZE {
            continue;
        }
        let axis = if horizontal { "input" } else { "output" };
        let first = current * PAGE_SIZE + 1;
        let last = ((current + 1) * PAGE_SIZE).min(count);
        let title = t!(
            if horizontal {
                "Input page %{page} / %{total}"
            } else {
                "Output page %{page} / %{total}"
            },
            page = current + 1,
            total = count.div_ceil(PAGE_SIZE)
        );
        let mut pager = div().flex().items_center().gap(px(4.)).min_w_0().child(
            note(format!("{title} ({first}-{last})"))
                .id(format!("{root}-{axis}-range"))
                .flex_1()
                .min_w_0()
                .gpui_test_observe(),
        );
        for (suffix, label, delta, disabled) in [
            ("prev", "<", -1isize, current == 0),
            ("next", ">", 1, last == count),
        ] {
            let this = cx.entity();
            let target = target.clone();
            pager = pager.child(
                controls::button(format!("{root}-{axis}-{suffix}"), label)
                    .density(Density::Compact)
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        this.update(cx, |shell, cx| {
                            model::navigate(shell, &target, surface, horizontal, delta, window, cx)
                        })
                    }),
            );
        }
        view = view.child(pager);
    }
    if let Some(error) = state.action_errors.get(&node) {
        view = view.child(note(error.clone()).text_color(theme::DANGER));
    }
    Some(
        view.child(errors)
            .child(note("Signed weights; negative values invert polarity"))
            .gpui_test_observe()
            .into_any_element(),
    )
}
