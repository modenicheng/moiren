//! Prepared recorder capacity, analysis settings and rack structure.
use super::*;

pub(super) fn controls(
    id: moiren_core::graph::NodeId,
    state: &crate::service::BuiltinState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let mut view = div().flex().flex_col().gap(px(theme::SPACE_2));
    match state {
        crate::service::BuiltinState::Recorder(settings) => {
            view = view
                .child(heading(t!("Recorder")))
                .child(detail(
                    &t!("Buffer"),
                    format!("{} frames", settings.capacity_frames),
                ))
                .child(
                    button("recorder-expand", t!("Add 48,000 frames"))
                        .disabled(settings.capacity_frames >= 1_000_000)
                        .on_click(edit(
                            cx.entity(),
                            GraphEdit::SetBuiltin {
                                node: id,
                                state: crate::service::BuiltinState::Recorder(RecorderSettings {
                                    capacity_frames: settings
                                        .capacity_frames
                                        .saturating_add(48_000)
                                        .min(1_000_000),
                                }),
                            },
                        )),
                )
                .child(
                    button("recorder-shrink", t!("Remove 48,000 frames"))
                        .disabled(settings.capacity_frames <= 1)
                        .on_click(edit(
                            cx.entity(),
                            GraphEdit::SetBuiltin {
                                node: id,
                                state: crate::service::BuiltinState::Recorder(RecorderSettings {
                                    capacity_frames: settings
                                        .capacity_frames
                                        .saturating_sub(48_000)
                                        .max(1),
                                }),
                            },
                        )),
                );
        }
        crate::service::BuiltinState::Meter(settings) => {
            view = view
                .child(heading(t!("Level meter")))
                .child(detail(
                    &t!("Window"),
                    t!("%{frames} frames", frames = settings.window_frames).to_string(),
                ))
                .child(
                    button("meter-window", t!("Use 2048 frame window")).on_click(edit(
                        cx.entity(),
                        GraphEdit::SetBuiltin {
                            node: id,
                            state: crate::service::BuiltinState::Meter(MeterSettings {
                                window_frames: 2048,
                            }),
                        },
                    )),
                );
        }
        crate::service::BuiltinState::SpectrumAnalyzer(settings) => {
            view = view
                .child(heading(t!("Spectrum analyzer")))
                .child(detail(
                    &t!("FFT"),
                    t!("%{points} points", points = settings.fft_size),
                ))
                .child(detail(
                    &t!("Hop"),
                    t!("%{frames} frames", frames = settings.hop_frames),
                ))
                .child(
                    button("spectrum-size", t!("Use 2048 point FFT")).on_click(edit(
                        cx.entity(),
                        GraphEdit::SetBuiltin {
                            node: id,
                            state: crate::service::BuiltinState::SpectrumAnalyzer(
                                SpectrumSettings {
                                    fft_size: 2048,
                                    hop_frames: 1024,
                                },
                            ),
                        },
                    )),
                );
        }
        crate::service::BuiltinState::Rack(settings) => {
            let next_slot_id = settings
                .slots
                .iter()
                .map(|slot| slot.id)
                .max()
                .map_or(0, |id| id.saturating_add(1));
            let mut next_slots = settings.slots.clone();
            next_slots.push(RackSlot {
                id: next_slot_id,
                bypass: false,
                processor: RackProcessor::Gain { initial: 1.0 },
            });
            let mut next_routes = settings.routes.clone();
            if let Some(route) = next_routes.first_mut() {
                route.slots.push(next_slot_id);
            }
            view = view
                .child(heading(t!("Processor rack")))
                .child(detail(&t!("Stages"), settings.slots.len().to_string()))
                .child(button("rack-add-gain", t!("Add gain stage")).on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::Rack(RackSettings {
                            slots: next_slots,
                            routes: next_routes,
                        }),
                    },
                )))
                .child(
                    button("rack-clear", t!("Clear stages"))
                        .disabled(settings.slots.is_empty())
                        .on_click(edit(
                            cx.entity(),
                            GraphEdit::SetBuiltin {
                                node: id,
                                state: crate::service::BuiltinState::Rack(clear_rack(settings)),
                            },
                        )),
                );
        }
        _ => {}
    }
    view.into_any_element()
}

fn clear_rack(settings: &RackSettings) -> RackSettings {
    let mut next = settings.clone();
    next.slots.clear();
    // Keep the independent stream routes, but not references to removed DSP.
    for route in &mut next.routes {
        route.slots.clear();
    }
    next
}

#[cfg(test)]
mod tests {
    use super::clear_rack;
    use moiren_engine::rack::{RackProcessor, RackRoute, RackSettings, RackSlot};

    #[::core::prelude::v1::test]
    fn clearing_stages_preserves_routes_without_dangling_slot_references() {
        let settings = RackSettings {
            slots: vec![RackSlot {
                id: 8,
                bypass: false,
                processor: RackProcessor::Gain { initial: 1. },
            }],
            routes: vec![RackRoute {
                input_port: 0,
                output_port: 1,
                slots: vec![8],
            }],
        };
        let cleared = clear_rack(&settings);
        assert!(cleared.slots.is_empty());
        assert_eq!(
            cleared.routes,
            vec![RackRoute {
                input_port: 0,
                output_port: 1,
                slots: Vec::new()
            }]
        );
        assert_eq!(settings.routes[0].slots, [8]);
    }
}
