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
        _ => {}
    }
    view.into_any_element()
}
