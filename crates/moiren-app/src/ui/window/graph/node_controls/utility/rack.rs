//! Ordered builtin stages retain stable identities and stream membership.
use super::*;
use moiren_engine::rack::{RackProcessor, RackSettings, RackSlot};

pub(super) fn content(
    shell: &StudioShell,
    node: &GraphNodeState,
    settings: &RackSettings,
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
    let inputs: Vec<_> = topology
        .inputs()
        .iter()
        .map(|port| port.channels())
        .collect();
    let outputs: Vec<_> = topology
        .outputs()
        .iter()
        .map(|port| port.channels())
        .collect();
    let layout = settings.resolve_layout(&inputs, &outputs);
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .w_full()
        .child(detail_line(
            "Stages",
            &format!("{} / {}", settings.slots.len(), RackSettings::MAX_SLOTS),
        ));
    let mut rows = div()
        .id(format!("card-rack-slots-{:?}", node.id))
        .flex()
        .flex_col()
        .gap(px(4.))
        .max_h(px(360.))
        .overflow_y_scroll();
    if settings.slots.is_empty() {
        rows = rows.child(note("No stages. The configured routes pass through."));
    }
    for (index, slot) in settings.slots.iter().enumerate() {
        let route = layout
            .as_ref()
            .and_then(|layout| layout.slots.get(&slot.id));
        let name = t!(rack_processor_name(&slot.processor));
        let title = if layout
            .as_ref()
            .is_some_and(|layout| layout.routes.len() > 1)
        {
            t!(
                "%{index}. %{name} / R%{route}",
                index = index + 1,
                name = name,
                route = route.map_or(0, |slot| slot.route + 1)
            )
            .to_string()
        } else {
            t!("%{index}. %{name}", index = index + 1, name = name).to_string()
        };
        let mut row = div().flex().items_center().gap(px(3.)).child(
            note(title)
                .text_color(if slot.bypass {
                    theme::TEXT_FAINT
                } else {
                    theme::TEXT
                })
                .flex_1()
                .min_w_0()
                .truncate(),
        );
        for (key, label, change, tooltip) in [
            (
                "bypass",
                if slot.bypass { "Off" } else { "On" },
                RackChange::Bypass(slot.id),
                "Toggle stage bypass",
            ),
            (
                "up",
                "^",
                RackChange::Move(slot.id, -1),
                "Move earlier within this route",
            ),
            (
                "down",
                "v",
                RackChange::Move(slot.id, 1),
                "Move later within this route",
            ),
            (
                "remove",
                "x",
                RackChange::Remove(slot.id),
                "Remove this stage",
            ),
        ] {
            row = row.child(
                choice(
                    format!("card-rack-{:?}-{}-{key}", node.id, slot.id),
                    label,
                    false,
                    edited_rack(settings, change, &inputs, &outputs)
                        .map(|settings| builtin_edit(node.id, BuiltinState::Rack(settings))),
                    cx,
                )
                .variant(if key == "bypass" && slot.bypass {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .tooltip(t!(tooltip)),
            );
        }
        rows = rows.child(row);
    }
    view = view.child(rows);
    if let Some(layout) = layout {
        let mut menu = div()
            .id(format!("card-rack-presets-{:?}", node.id))
            .flex()
            .flex_col()
            .gap(px(4.))
            .w(px(248.))
            .max_h(px(288.))
            .overflow_y_scroll();
        for (route_index, route) in layout.routes.iter().enumerate() {
            menu = menu.child(note(
                t!(
                    "R%{route}: Input %{input} -> Output %{output}",
                    route = route_index + 1,
                    input = usize::from(route.input_port) + 1,
                    output = usize::from(route.output_port) + 1
                )
                .to_string(),
            ));
            for (index, processor) in rack_presets().into_iter().enumerate() {
                let name = rack_processor_name(&processor);
                let next = edited_rack(
                    settings,
                    RackChange::Add(route_index, processor),
                    &inputs,
                    &outputs,
                );
                menu = menu.child(
                    choice(
                        format!("card-rack-add-{:?}-{route_index}-{index}", node.id),
                        name,
                        false,
                        next.map(|settings| builtin_edit(node.id, BuiltinState::Rack(settings))),
                        cx,
                    )
                    .full_width()
                    .justify_start()
                    .tooltip(t!(
                        "Add builtin defaults to this route; incompatible widths are disabled",
                    )),
                );
            }
        }
        view = view.child(
            component::popover::Popover::new(format!("card-rack-add-popup-{:?}", node.id))
                .trigger(UtilityTrigger(
                    controls::button(format!("card-rack-add-{:?}", node.id), t!("+ Add stage"))
                        .full_width()
                        .disabled(settings.slots.len() >= RackSettings::MAX_SLOTS),
                ))
                .child(menu),
        );
    } else {
        view = view.child(
            note("Invalid rack layout; inspect its stream routes.").text_color(theme::WARNING),
        );
    }
    Some(view.into_any_element())
}

enum RackChange {
    Bypass(u32),
    Remove(u32),
    Move(u32, i8),
    Add(usize, RackProcessor),
}

fn edited_rack(
    settings: &RackSettings,
    change: RackChange,
    inputs: &[usize],
    outputs: &[usize],
) -> Option<RackSettings> {
    let layout = settings.resolve_layout(inputs, outputs)?;
    let mut next = settings.clone();
    match change {
        RackChange::Bypass(id) => {
            let slot = next.slots.iter_mut().find(|slot| slot.id == id)?;
            slot.bypass = !slot.bypass;
        }
        RackChange::Remove(id) => {
            let index = next.slots.iter().position(|slot| slot.id == id)?;
            next.slots.remove(index);
            for route in &mut next.routes {
                route.slots.retain(|slot| *slot != id);
            }
        }
        RackChange::Move(id, direction) => {
            let route = layout.slots.get(&id)?.route;
            let indices: Vec<_> = next
                .slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| {
                    layout
                        .slots
                        .get(&slot.id)
                        .is_some_and(|slot| slot.route == route)
                })
                .map(|(index, _)| index)
                .collect();
            let position = indices
                .iter()
                .position(|&index| next.slots[index].id == id)?;
            let target = match direction {
                -1 => position.checked_sub(1)?,
                1 => position.checked_add(1)?,
                _ => return None,
            };
            // Route slot lists are membership sets. Only the slots vector
            // determines execution order, including interleaved stream routes.
            next.slots.swap(indices[position], *indices.get(target)?);
        }
        RackChange::Add(route, processor) => {
            layout.routes.get(route)?;
            if next.slots.len() >= RackSettings::MAX_SLOTS {
                return None;
            }
            let id = match next.slots.iter().map(|slot| slot.id).max() {
                None => 0,
                Some(max) => max.checked_add(1).or_else(|| {
                    (0..=RackSettings::MAX_SLOTS as u32)
                        .find(|id| !next.slots.iter().any(|slot| slot.id == *id))
                })?,
            };
            next.slots.push(RackSlot {
                id,
                bypass: false,
                processor,
            });
            if !next.routes.is_empty() {
                next.routes.get_mut(route)?.slots.push(id);
            }
        }
    }
    // Bypassing, removing or moving a width-changing mapper can invalidate
    // downstream stages. Offer only edits valid for the actual Rack ports.
    next.valid_layout(inputs, outputs).then_some(next)
}

fn rack_presets() -> [RackProcessor; 8] {
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

fn rack_processor_name(processor: &RackProcessor) -> &'static str {
    match processor {
        RackProcessor::Gain { .. } => "Gain",
        RackProcessor::Pan { .. } => "Balance",
        RackProcessor::Compressor(_) => "Compressor",
        RackProcessor::Eq(_) => "EQ",
        RackProcessor::Gate(_) => "Gate",
        RackProcessor::Limiter(_) => "Limiter",
        RackProcessor::Delay(_) => "Delay",
        RackProcessor::ChannelMapper(_) => "Mapper",
    }
}

#[cfg(test)]
mod tests;
