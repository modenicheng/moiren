use super::*;
use crate::ui::shell::settings::{Direction, Editor};
use crate::ui::widgets::overlay_scrollbar::{notify_owner, scroll_region};
use gpui_kit::base::ScrollbarAxis;
use rust_i18n::t;

/// The inspected device's configuration and monitoring (design 23 §3.3). The
/// chrome is the shared inspector column's flush panel — the same `panel()`
/// language the empty state uses, not a floating card inside the column —
/// with the draft footer pinned under the scrolled content.
pub(super) fn inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let input = shell.devices.device_tab == DeviceTab::Inputs;
    let direction = if input {
        Direction::Input
    } else {
        Direction::Output
    };
    let id = shell
        .devices
        .selected_device
        .as_deref()
        .expect("inspected device");
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let draft = &shell.settings.editor.device;
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
        .gap(px(theme::SPACE_3))
        .child(heading(name))
        .child(label(if input {
            t!("Input configuration")
        } else {
            t!("Output configuration")
        }))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(theme::SPACE_1_5))
                .child(badge(
                    if custom {
                        t!("Device override")
                    } else {
                        t!("Using global defaults")
                    },
                    theme::controls::FOCUS,
                )),
        )
        .child(hairline())
        .child(detail_mono(
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
            .gap(px(theme::SPACE_1_5))
            .child(
                button("device-use-defaults", t!("Use defaults"))
                    .disabled(shell.settings.editor.busy() || !custom)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            let draft = &mut shell.settings.editor.device;
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
                    .disabled(shell.settings.editor.busy() || custom)
                    .on_click({
                        let this = cx.entity();
                        move |_, _, cx| {
                            this.update(cx, |shell, cx| {
                                settings::device_stream_mut(
                                    &mut shell.settings.editor.device.audio,
                                    direction,
                                    &override_id,
                                );
                                shell.settings.editor.device.changed();
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
    // The monitor section (design 23 §3.3): the candidate graph's users
    // merged with the engine's live streams — two independent facts about
    // this device, shown as such. Replaces the old desired-paired "active
    // session" block, which could only speak about the legacy primary pair.
    let device = DeviceKey::Physical {
        endpoint_id: id.to_owned(),
        input,
    };
    let users = monitor::device_users(snapshot, &device);
    content = content.child(hairline()).child(section_title(t!("Users")));
    if users.is_empty() {
        content = content.child(label(t!("No nodes are bound to this device.")));
    } else {
        for row in user_rows(
            snapshot,
            &device,
            |node| {
                (
                    SharedString::from(format!("device-user-{node:?}")),
                    SharedString::from(format!("reveal-{node:?}")),
                )
            },
            cx,
        ) {
            content = content.child(row);
        }
        // The request the running engine actually honored on this device —
        // only meaningful while a stream is live here.
        if users
            .iter()
            .any(|user| user.state != monitor::UserState::NotRunning)
            && let Some(active) = snapshot
                .active_audio_settings
                .as_ref()
                .filter(|_| snapshot.phase == SessionPhase::Running)
        {
            let active_stream = if input {
                active.effective_input(id)
            } else {
                active.effective_output(id)
            };
            content = content.child(detail_mono(
                &t!("Active request"),
                settings::stream_summary(active.backend, active_stream),
            ));
        }
    }
    div()
        .id("device-override-inspector")
        .flex_none()
        .w_full()
        .h_full()
        .flex()
        .flex_col()
        .min_h_0()
        .bg(theme::SURFACE)
        .border_l_1()
        .border_color(theme::BORDER)
        .child(
            scroll_region(
                "scrollbar-device-override-scroll",
                &shell.devices.device_override_scroll,
                ScrollbarAxis::Vertical,
                div()
                    .id("device-override-scroll")
                    .w_full()
                    .h_full()
                    .track_scroll(&shell.devices.device_override_scroll)
                    .restrict_scroll_to_axis()
                    .overflow_y_scroll()
                    .on_scroll_wheel(notify_owner(cx.entity()))
                    .p(px(theme::SPACE_4))
                    .child(content)
                    .gpui_test_observe(),
            )
            .flex_1()
            .min_h_0(),
        )
        .child(settings::footer(shell, Editor::Device, cx))
        .gpui_test_observe()
        .into_any_element()
}
