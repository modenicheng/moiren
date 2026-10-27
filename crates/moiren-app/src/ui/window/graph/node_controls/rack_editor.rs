//! One selected Rack stage, shared selection with independent editing surfaces.
use super::*;
use crate::service::RackEdit;
use moiren_engine::rack::{RackProcessor, RackSettings};

mod matrix;
mod model;
mod parameters;
mod structure;
#[cfg(all(test, feature = "gpui-test"))]
mod tests;
use model::{Key, Surface};
pub(super) use model::{RackEditorState, sync};

pub(in crate::ui::window::graph) fn inspector(
    shell: &StudioShell,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    node.builtin
        .rack()
        .and_then(|settings| content(shell, node, settings, Surface::Inspector, cx))
        .unwrap_or_else(|| {
            note("Invalid rack layout; inspect its stream routes.").into_any_element()
        })
}

pub(super) fn card(
    shell: &StudioShell,
    node: &GraphNodeState,
    settings: &RackSettings,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    content(shell, node, settings, Surface::Card, cx)
}

fn content(
    shell: &StudioShell,
    node: &GraphNodeState,
    settings: &RackSettings,
    surface: Surface,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    let graph = &shell.state.read(cx).snapshot.graph;
    let topology = graph.graph.get_node(node.id).ok()?;
    let inputs: Vec<_> = topology.inputs().iter().map(|p| p.channels()).collect();
    let outputs: Vec<_> = topology.outputs().iter().map(|p| p.channels()).collect();
    let epoch = *graph.rack_epochs.get(&node.id)?;
    let layout = settings.resolve_layout(&inputs, &outputs)?;
    let scope = Scope {
        node: node.id,
        epoch,
        surface,
        settings,
        inputs: &inputs,
        outputs: &outputs,
    };
    let selected = shell.node_controls.rack.selected_slot(node.id, settings);
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .w_full()
        .child(structure::list(&scope, selected, &layout, cx));
    if let Some(slot) = selected.and_then(|id| settings.slots.iter().find(|s| s.id == id)) {
        let (kind, leaf) = parameters::stage_node(node, slot);
        let mut editor = div()
            .id(scope.id(&format!("editor-{}", slot.id)))
            .flex()
            .flex_col()
            .gap(px(6.))
            .w_full()
            .min_w_0()
            .pt(px(6.))
            .border_t_1()
            .border_color(theme::BORDER)
            .child(structure::actions(&scope, slot, cx));
        if matches!(slot.processor, RackProcessor::ChannelMapper(_)) {
            let input = layout.slots.get(&slot.id)?.input_channels;
            let output = slot.processor.output_channels(input).ok()?;
            editor = editor.child(matrix::content(shell, &scope, slot.id, input, output, cx));
        } else {
            let mut fields = div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .w_full()
                .when(surface == Surface::Card, |view| view.justify_center());
            for field in parameters::fields(kind, &leaf, surface) {
                let key = Key {
                    surface,
                    node: node.id,
                    epoch,
                    slot: slot.id,
                    field,
                };
                let state = &shell.node_controls.rack;
                if let (Some(input), Some(slider)) =
                    (state.inputs.get(&key), state.sliders.get(&key))
                {
                    fields = if surface == Surface::Card {
                        fields.child(super::fields::dial_field(
                            scope.id(&format!("field-{}-{field:?}", slot.id)),
                            field.label(),
                            field.unit(),
                            input,
                            super::rotary::keyed_knob(
                                scope.id(&format!("knob-{}-{field:?}", slot.id)),
                                slider,
                                cx,
                            ),
                            state.errors.get(&key),
                        ))
                    } else {
                        fields.child(precision_field(&scope, key, input, state.errors.get(&key)))
                    };
                }
            }
            editor = editor.child(fields);
            if let RackProcessor::Pan { mono, swap, .. } = slot.processor {
                editor = editor.child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(scope.button(
                            "mono",
                            "Mono",
                            mono,
                            RackEdit::ToggleMono { slot: slot.id },
                            cx,
                        ))
                        .child(scope.button(
                            "swap",
                            "Swap L/R",
                            swap,
                            RackEdit::ToggleSwap { slot: slot.id },
                            cx,
                        )),
                );
            }
            if surface == Surface::Card && kind == NodeKind::Compressor {
                editor = editor.child(note("Input, knee, hold and output gain in Details"));
            }
        }
        view = view.child(editor.gpui_test_observe());
    }
    view = view.child(structure::add_stage(&scope, &layout, cx));
    Some(view.into_any_element())
}

fn precision_field(
    scope: &Scope<'_>,
    key: Key,
    input: &Entity<component::input::InputState>,
    error: Option<&String>,
) -> AnyElement {
    div()
        .id(scope.id(&format!("field-{}-{:?}", key.slot, key.field)))
        .flex()
        .flex_col()
        .gap(px(2.))
        .w(px(114.))
        .flex_none()
        .child(note(key.field.label()))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(
                    div().flex_1().min_w_0().child(Styled::h(
                        component::input::Input::new(input)
                            .px(px(4.))
                            .py(px(0.))
                            .text_size(px(11.))
                            .text_right()
                            .bg(theme::SURFACE_3),
                        px(26.),
                    )),
                )
                .child(div().w(px(22.)).flex_none().child(note(key.field.unit()))),
        )
        .when_some(error, |view, error| {
            view.child(note(error.clone()).text_color(theme::DANGER))
        })
        .gpui_test_observe()
        .into_any_element()
}

/// Preview supplies enabled states only. Submit the intent, never a rendered
/// Rack clone: queued edits must merge with the service's newest candidate.
struct Scope<'a> {
    node: NodeId,
    epoch: u64,
    surface: Surface,
    settings: &'a RackSettings,
    inputs: &'a [usize],
    outputs: &'a [usize],
}

impl Scope<'_> {
    fn id(&self, suffix: &str) -> String {
        format!("rack-{:?}-{:?}-{suffix}", self.surface, self.node)
    }

    fn button(
        &self,
        key: &str,
        label: &str,
        selected: bool,
        edit: RackEdit,
        cx: &Context<StudioShell>,
    ) -> ActionButton {
        controls::button(self.id(key), t!(label).to_string())
            .density(Density::Compact)
            .variant(if selected {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .disabled(
                edit.preview(self.settings, self.inputs, self.outputs)
                    .is_err(),
            )
            .on_click(controls::edit(
                cx.entity(),
                GraphEdit::EditRack {
                    node: self.node,
                    epoch: self.epoch,
                    edit,
                },
            ))
    }
}
