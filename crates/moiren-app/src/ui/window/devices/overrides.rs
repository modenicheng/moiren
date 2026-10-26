use super::*;
use crate::ui::window::settings::{Direction, Editor};
use rust_i18n::t;

pub(super) fn inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let input = shell.device_tab == DeviceTab::Inputs;
    let direction = if input {
        Direction::Input
    } else {
        Direction::Output
    };
    let id = shell.selected_device.as_deref().expect("inspected device");
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let draft = &shell.settings.device;
    let overrides = if input {
        &draft.audio.input_overrides
    } else {
        &draft.audio.output_overrides
    };
    let custom = overrides.contains_key(id);
    let catalog = &snapshot.catalog;
    let rows = if draft.audio.backend == moiren_engine::audio_config::AudioBackend::Asio {
        &catalog.asio_drivers
    } else if input {
        &catalog.inputs
    } else {
        &catalog.outputs
    };
    let name = rows
        .iter()
        .find(|row| row.endpoint_id == id)
        .map(|row| row.name.clone())
        .unwrap_or_else(|| t!("Unavailable device").to_string());
    let saved = if input {
        snapshot.audio_settings.effective_input(id)
    } else {
        snapshot.audio_settings.effective_output(id)
    };
    let mut content = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(heading(name))
        .child(label(if input {
            t!("Input configuration")
        } else {
            t!("Output configuration")
        }))
        .child(badge(
            if custom {
                t!("Device override")
            } else {
                t!("Using global defaults")
            },
            theme::PRIMARY,
        ))
        .child(detail(
            &t!("Saved request"),
            settings::stream_summary(snapshot.audio_settings.backend, saved),
        ));
    let this = cx.entity();
    let defaults_id = id.to_owned();
    let override_id = id.to_owned();
    content = content.child(
        div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .child(
                button("device-use-defaults", t!("Use defaults"))
                    .disabled(shell.settings.busy() || !custom)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            let draft = &mut shell.settings.device;
                            if input {
                                draft.audio.input_overrides.remove(&defaults_id);
                            } else {
                                draft.audio.output_overrides.remove(&defaults_id);
                            }
                            draft.changed();
                            cx.notify();
                        })
                    }),
            )
            .child(
                button("device-override", t!("Override"))
                    .variant(if custom {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .disabled(shell.settings.busy() || custom)
                    .on_click({
                        let this = cx.entity();
                        move |_, _, cx| {
                            this.update(cx, |shell, cx| {
                                settings::device_stream_mut(
                                    &mut shell.settings.device.audio,
                                    direction,
                                    &override_id,
                                );
                                shell.settings.device.changed();
                                cx.notify();
                            })
                        }
                    }),
            ),
    );
    if draft.audio.backend == moiren_engine::audio_config::AudioBackend::Asio {
        content = content.child(label(t!("ASIO input and output share one driver session. Their sample rate and buffer requests must agree.")));
    }
    if custom {
        content = content.child(settings::stream_editor(shell, Editor::Device, direction, Some(id), cx))
            .child(label(t!("This complete override stays independent of global defaults. Use defaults removes it when saved.")));
    } else {
        content = content.child(label(t!("This device follows the current global defaults. Choose Override to customize all its stream settings.")));
    }
    if let Some(active) = snapshot
        .active_audio_settings
        .as_ref()
        .filter(|_| snapshot.phase == SessionPhase::Running)
    {
        let bound = snapshot.desired.as_ref().is_some_and(|session| {
            if input {
                matches!(&session.source, crate::service::InputSelection::Physical { endpoint_id } if endpoint_id == id)
            } else {
                session.output_endpoint_id == id
            }
        });
        if bound {
            let active_stream = if input {
                active.effective_input(id)
            } else {
                active.effective_output(id)
            };
            content = content
                .child(heading(t!("Active session")))
                .child(detail(
                    &t!("Active request"),
                    settings::stream_summary(active.backend, active_stream),
                ))
                .child(label(t!("Observed session timing")));
            for (key, value) in crate::ui::presentation::native_timing_rows(snapshot) {
                content = content.child(detail(&key, value));
            }
        } else {
            content = content.child(label(t!(
                "This device is not bound to the active source/output pair."
            )));
        }
    }
    content = content.child(label(t!("Configure never changes routing. Use next start on a device row updates the candidate graph.")));
    div()
        .id("device-override-inspector")
        .flex_none()
        .w(px(410.))
        .max_w(relative(0.55))
        .flex()
        .flex_col()
        .min_h_0()
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        .rounded(px(theme::RADIUS_MD))
        .child(
            div()
                .id("device-override-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(16.))
                .child(content)
                .gpui_test_observe(),
        )
        .child(settings::footer(shell, Editor::Device, cx))
        .gpui_test_observe()
        .into_any_element()
}
