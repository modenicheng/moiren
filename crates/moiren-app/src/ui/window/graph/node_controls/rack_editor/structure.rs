//! Ordered stage navigation and route-aware structural intents.
use super::*;
use moiren_engine::rack::{RackLayout, RackSlot};

pub(super) fn list(
    scope: &Scope<'_>,
    selected: Option<u32>,
    layout: &RackLayout,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .w_full()
        .child(detail_line(
            "Stages",
            &format!(
                "{} / {}",
                scope.settings.slots.len(),
                RackSettings::MAX_SLOTS
            ),
        ));
    if scope.settings.slots.is_empty() {
        return view
            .child(note("No stages. The configured routes pass through."))
            .into_any_element();
    }
    let mut rows = div()
        .id(scope.id("stages"))
        .flex()
        .flex_col()
        .gap(px(4.))
        .w_full()
        .max_h(px(96.))
        .overflow_y_scroll();
    for chunk in scope.settings.slots.chunks(2) {
        let mut row = div().flex().gap(px(4.)).w_full().flex_none();
        for slot in chunk {
            let position = scope
                .settings
                .slots
                .iter()
                .position(|candidate| candidate.id == slot.id)
                .unwrap()
                + 1;
            let route = layout.slots.get(&slot.id).map_or(0, |s| s.route);
            let label = if layout.routes.len() > 1 {
                format!("{position}. {} / R{}", t!(name(&slot.processor)), route + 1)
            } else {
                format!("{position}. {}", t!(name(&slot.processor)))
            };
            let node = scope.node;
            let slot_id = slot.id;
            let this = cx.entity();
            row = row.child(
                div().flex_1().min_w_0().child(
                    controls::button(scope.id(&format!("select-{slot_id}")), label)
                        .density(Density::Compact)
                        .full_width()
                        .justify_start()
                        .variant(if selected == Some(slot_id) {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .when(slot.bypass, |button| button.tooltip(t!("Bypassed")))
                        .on_click(move |_, window, cx| {
                            this.update(cx, |shell, cx| {
                                shell.node_controls.rack.selected.insert(node, slot_id);
                                // Build the new input entities before rendering this editor.
                                sync(shell, window, cx);
                                cx.notify();
                            })
                        }),
                ),
            );
        }
        if chunk.len() == 1 {
            row = row.child(div().flex_1());
        }
        rows = rows.child(row);
    }
    // Selection lives in the shared state, never in the service graph.
    view = view.child(rows.gpui_test_observe());
    view.into_any_element()
}

pub(super) fn actions(scope: &Scope<'_>, slot: &RackSlot, cx: &Context<StudioShell>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(detail_line(
            name(&slot.processor),
            &t!(if slot.bypass { "Bypassed" } else { "Enabled" }),
        ))
        .child(
            div()
                .flex()
                .gap(px(4.))
                .child(scope.button(
                    "bypass",
                    "Bypass",
                    slot.bypass,
                    RackEdit::ToggleBypass { slot: slot.id },
                    cx,
                ))
                .child(
                    scope
                        .button(
                            "up",
                            "Up",
                            false,
                            RackEdit::Move {
                                slot: slot.id,
                                direction: -1,
                            },
                            cx,
                        )
                        .tooltip(t!("Move earlier within this route")),
                )
                .child(
                    scope
                        .button(
                            "down",
                            "Down",
                            false,
                            RackEdit::Move {
                                slot: slot.id,
                                direction: 1,
                            },
                            cx,
                        )
                        .tooltip(t!("Move later within this route")),
                )
                .child(
                    scope
                        .button(
                            "remove",
                            "Remove",
                            false,
                            RackEdit::Remove { slot: slot.id },
                            cx,
                        )
                        .variant(ButtonVariant::Ghost),
                ),
        )
        .into_any_element()
}

pub(super) fn add_stage(
    scope: &Scope<'_>,
    layout: &RackLayout,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let mut menu = div()
        .id(scope.id("presets"))
        .flex()
        .flex_col()
        .gap(px(4.))
        .w(px(248.))
        .max_h(px(288.))
        .overflow_y_scroll();
    for (route, stream) in layout.routes.iter().enumerate() {
        menu = menu.child(note(
            t!(
                "R%{route}: Input %{input} -> Output %{output}",
                route = route + 1,
                input = usize::from(stream.input_port) + 1,
                output = usize::from(stream.output_port) + 1
            )
            .to_string(),
        ));
        for (index, processor) in presets().into_iter().enumerate() {
            let label = name(&processor);
            menu = menu.child(
                scope
                    .button(
                        &format!("add-{route}-{index}"),
                        label,
                        false,
                        RackEdit::Add { route, processor },
                        cx,
                    )
                    .full_width()
                    .justify_start()
                    .tooltip(t!(
                        "Add builtin defaults to this route; incompatible widths are disabled"
                    )),
            );
        }
    }
    let mut view = div().flex().gap(px(4.)).w_full().child(
        div().flex_1().child(
            component::popover::Popover::new(scope.id("add-popup"))
                .trigger(Trigger(
                    controls::button(scope.id("add"), t!("+ Add stage"))
                        .full_width()
                        .disabled(scope.settings.slots.len() >= RackSettings::MAX_SLOTS),
                ))
                .child(menu),
        ),
    );
    if scope.surface == Surface::Inspector {
        view = view.child(
            scope
                .button("clear", "Clear stages", false, RackEdit::Clear, cx)
                .variant(ButtonVariant::Ghost)
                .disabled(
                    scope.settings.slots.is_empty()
                        || RackEdit::Clear
                            .preview(scope.settings, scope.inputs, scope.outputs)
                            .is_err(),
                ),
        );
    }
    view.into_any_element()
}

fn presets() -> [RackProcessor; 8] {
    [
        RackProcessor::Gain { initial: 1. },
        RackProcessor::Pan {
            initial: 0.,
            mono: false,
            swap: false,
        },
        RackProcessor::Eq(Default::default()),
        RackProcessor::Compressor(Default::default()),
        RackProcessor::Gate(Default::default()),
        RackProcessor::Limiter(Default::default()),
        RackProcessor::Delay(Default::default()),
        RackProcessor::ChannelMapper(Default::default()),
    ]
}

fn name(processor: &RackProcessor) -> &'static str {
    match processor {
        RackProcessor::Gain { .. } => "Gain",
        RackProcessor::Pan { .. } => "Balance",
        RackProcessor::Eq(_) => "EQ",
        RackProcessor::Compressor(_) => "Compressor",
        RackProcessor::Gate(_) => "Gate",
        RackProcessor::Limiter(_) => "Limiter",
        RackProcessor::Delay(_) => "Delay",
        RackProcessor::ChannelMapper(_) => "Mapper",
    }
}

#[derive(IntoElement)]
struct Trigger(ActionButton);
impl component::Selectable for Trigger {
    fn is_selected(&self) -> bool {
        false
    }
    fn selected(self, _: bool) -> Self {
        self
    }
}
impl RenderOnce for Trigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.0
    }
}
