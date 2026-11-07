use rust_i18n::t;

use super::panel::{CachedPanel, PanelHost, PublishRef};
use super::*;
use crate::service::AudioLatencyProfile;
use crate::ui::widgets::overlay_scrollbar::overlay_scrollbar;
use controls::*;
use gpui_kit::base::FocusTrapElement;
use gpui_kit::base::ScrollbarAxis;
use moiren_engine::audio_config::{
    AudioBackend, AudioSettings, ProcessingPrecision, SAMPLE_RATES, StreamSettings,
};
use moiren_engine::node::DeviceMode;

mod fields;
mod menu;
mod model;
#[cfg(test)]
mod model_tests;
mod persistence;
mod transaction;
use fields::{choices, pref_choices, rate_title, row, row_with_note, rows, select_trigger};
pub(in crate::ui::window) use fields::{footer, stream_editor};
use menu::ValueField;
pub(in crate::ui::window) use menu::{ValueMenu, menu_overlay};
pub(in crate::ui::window) use model::{
    Direction, Draft, Editor, Section, SettingsEditor, StreamTarget, device_stream_mut,
};

/// The settings workspace: rebuilt only when the published snapshot
/// (diagnostics), the active section, the audio draft, or one of the
/// immediately-applied preferences changed. The open value menu
/// ([`ValueMenu`]) is deliberately not part of the key — it never changes
/// what the page itself renders.
pub(in crate::ui::window) struct SettingsPanel;

#[derive(PartialEq)]
pub(in crate::ui::window) struct SettingsKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
    section: Section,
    global: Draft,
    /// The page body scrolls inside the cached splice, whose paint bakes
    /// coordinates: the offset rides the key or a scroll freezes.
    scroll: Pixels,
    meters: crate::project::MeterDisplayPreferences,
    density: Density,
    restore_last_project: bool,
    pending_request: bool,
    project_message: Option<String>,
}

pub(in crate::ui::window) type SettingsPanelHost = PanelHost<SettingsPanel>;

impl CachedPanel for SettingsPanel {
    type Owner = StudioShell;
    type Key = SettingsKey;

    fn key(&self, shell: &Self::Owner, cx: &App) -> Self::Key {
        let settings = &shell.settings;
        let state = shell.state.read(cx);
        Self::Key {
            snapshot: PublishRef::new(std::sync::Arc::clone(&state.snapshot)),
            section: settings.section,
            global: settings.global.clone(),
            meters: shell.preferences.meters,
            density: shell.density,
            restore_last_project: shell.preferences.restore_last_project,
            pending_request: state.pending_request,
            project_message: shell.project_message.clone(),
            scroll: shell.settings_scroll.offset().y,
        }
    }

    fn render(
        &self,
        shell: &mut Self::Owner,
        window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement {
        page(shell, window, cx)
    }
}

/// The settings page's cached splice; mounted in place of the workspace
/// while settings are open (the shell's own `.when`).
pub(in crate::ui::window) fn page_for(shell: &StudioShell, _window: &Window) -> AnyElement {
    // The cached wrapper is laid out as a root; it fills the workspace row
    // exactly the way the pages it replaces do.
    shell
        .settings_panel
        .clone()
        .cached(StyleRefinement::default().flex_1().min_w_0().min_h_0())
        .into_any_element()
}

pub(super) fn page(shell: &StudioShell, _window: &Window, cx: &Context<StudioShell>) -> AnyElement {
    let this = cx.entity();
    let escape = this.clone();
    let close = this.clone();
    let mut nav = div().flex().flex_col().gap(px(2.)).px(px(8.)).pt(px(4.));
    for (id, icon, title, section) in [
        (
            "settings-nav-audio",
            ActionIcon::Wave,
            t!("Audio"),
            Section::Audio,
        ),
        (
            "settings-nav-interface",
            ActionIcon::Sliders,
            t!("Interface"),
            Section::Interface,
        ),
        (
            "settings-nav-diagnostics",
            ActionIcon::ListDetails,
            t!("Diagnostics"),
            Section::Diagnostics,
        ),
    ] {
        let this = this.clone();
        let selected = shell.settings.section == section;
        nav = nav.child(
            div()
                .id(SharedString::from(format!("{id}-row")))
                .relative()
                .child(
                    button(id, title)
                        .icon(icon)
                        .full_width()
                        .justify_start()
                        .density(shell.density)
                        .variant(if selected {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        })
                        .on_click(move |_, _, cx| {
                            this.update(cx, |shell, cx| {
                                shell.settings.section = section;
                                shell.settings_menu = None;
                                cx.notify();
                            })
                        }),
                )
                .when(selected, |row| {
                    row.child(
                        div()
                            .absolute()
                            .left(px(0.))
                            .top(px(6.))
                            .bottom(px(6.))
                            .w(px(2.))
                            .rounded(px(1.))
                            .bg(theme::PRIMARY),
                    )
                }),
        );
    }
    let content = match shell.settings.section {
        Section::Audio => audio(shell, cx).into_any_element(),
        Section::Interface => interface(shell, cx).into_any_element(),
        Section::Diagnostics => diagnostics(shell, cx).into_any_element(),
    };
    div()
        .id("settings-page")
        .size_full()
        .flex()
        .min_h_0()
        .bg(theme::WINDOW_BG)
        .gpui_test_observe()
        // The focus trap owns the element it wraps (the id above stays the
        // observable page root), so keyboard dispatch keeps living here —
        // the workspace underneath is unmounted while settings are open.
        .child(
            div()
                .size_full()
                .flex()
                .min_h_0()
                .track_focus(&shell.settings.focus)
                .on_key_down(move |event, window, cx| {
                    if event.keystroke.key == "escape" {
                        escape.update(cx, |shell, cx| {
                            // Escape peels one layer: an open value menu
                            // first, the page itself second.
                            if shell.settings_menu.take().is_none() {
                                shell.close_settings(window, cx);
                            } else {
                                cx.notify();
                            }
                        });
                    }
                    cx.stop_propagation();
                })
                .focus_trap("settings-focus-trap", &shell.settings.focus)
                .child(
                    div()
                        .id("settings-rail")
                        .flex_none()
                        .w(px(196.))
                        .h_full()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .border_r_1()
                        .border_color(theme::BORDER)
                        .bg(theme::WINDOW_BG)
                        .p(px(8.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px(px(4.))
                                .child(heading(t!("Settings")))
                                .child(
                                    IconButton::new(
                                        "close-settings",
                                        t!("Close"),
                                        ActionIcon::Close,
                                    )
                                    .on_click(
                                        move |_, window, cx| {
                                            close.update(cx, |shell, cx| {
                                                shell.close_settings(window, cx)
                                            })
                                        },
                                    ),
                                ),
                        )
                        .child(nav),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(
                            // The bar overlay is a sibling of the scroll
                            // container, never a child: the container
                            // translates its children by the offset at
                            // prepaint, so an overlay inside it would drift
                            // away with the content.
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .child(
                                    div()
                                        .id("settings-scroll")
                                        .flex_1()
                                        .min_h_0()
                                        .track_scroll(&shell.settings_scroll)
                                        .overflow_y_scroll()
                                        .on_scroll_wheel({
                                            let this = this.clone();
                                            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
                                        })
                                        .px(px(16.))
                                        .py(px(24.))
                                        // A fixed reading measure, centered: on wide
                                        // windows the section content holds together
                                        // as one block instead of rows drifting
                                        // apart across the workspace.
                                        .child(
                                            div().flex().justify_center().child(
                                                div()
                                                    .w(px(COLUMN_WIDTH))
                                                    .max_w_full()
                                                    .flex()
                                                    .flex_col()
                                                    .gap(px(16.))
                                                    .child(content),
                                            ),
                                        )
                                        .gpui_test_observe(),
                                )
                                .child(overlay_scrollbar(
                                    &shell.settings_scroll,
                                    "scrollbar-settings-scroll",
                                    ScrollbarAxis::Vertical,
                                )),
                        )
                        .when(shell.settings.section == Section::Audio, |column| {
                            column.child(footer(shell, Editor::Global, cx))
                        }),
                )
                .into_any_element(),
        )
        .into_any_element()
}

/// The reading measure of the settings content column.
const COLUMN_WIDTH: f32 = 680.;

/// A group card's title: one step under the section heading so the card
/// reads as a unit, not a second page.
fn group_title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(theme::TEXT_MD))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

/// The card shell shared by titled and untitled groups: raised surface,
/// hairline border, the page's one structural voice against the canvas.
fn card_shell() -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(theme::RADIUS_MD))
        .border_1()
        .border_color(theme::HAIRLINE)
        .bg(theme::SURFACE)
}

/// A titled settings group: the reading unit of the page. Rows go inside
/// via [`rows`]; the card bounds their measure on wide windows.
fn group(title: impl Into<SharedString>, body: impl IntoElement) -> Div {
    card_shell()
        .px(px(16.))
        .pt(px(12.))
        .pb(px(4.))
        .gap(px(10.))
        .child(group_title(title))
        .child(body)
}

fn audio(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let draft = &shell.settings.global;
    let config = &draft.audio;
    let overrides_this = cx.entity();
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(heading(t!("Audio")))
        .child(group(
            t!("Processing"),
            rows([
            row(
                &t!("Backend"),
                choices(
                    shell,
                    Editor::Global,
                    "audio-backend",
                    config.backend,
                    AudioBackend::ALL.map(|value| (value, t!(value.title()).into())),
                    |draft, value| draft.audio.backend = value,
                    cx,
                ),
            ),
            row(
                &t!("Processing sample rate"),
                choices(
                    shell,
                    Editor::Global,
                    "processing-rate",
                    config.processing.sample_rate,
                    SAMPLE_RATES.map(|rate| (rate, rate_title(rate))),
                    |draft, value| draft.audio.processing.sample_rate = value,
                    cx,
                ),
            ),
            row(
                &t!("Processing precision"),
                choices(
                    shell,
                    Editor::Global,
                    "processing-precision",
                    config.processing.precision,
                    ProcessingPrecision::ALL.map(|value| (value, t!(value.title()).into())),
                    |draft, value| draft.audio.processing.precision = value,
                    cx,
                ),
            ),
            row_with_note(
                &t!("Processing block"),
                t!("Buffers are requests; the active session reports the actual periods in Diagnostics.").into_owned(),
                select_trigger(
                    shell,
                    Editor::Global,
                    "processing-buffer",
                    StreamTarget {
                        direction: Direction::Input,
                        device: None,
                    },
                    ValueField::ProcessingBlock,
                    fields::buffer_title(
                        config.processing.block_frames,
                        config.processing.sample_rate,
                    ),
                    cx,
                ),
            ),
            ]),
        ))
        .child(
            card_shell().p(px(16.)).child(
                div()
                    .flex()
                    .gap(px(24.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(group_title(t!("Input defaults")))
                            .child(stream_editor(
                                shell,
                                Editor::Global,
                                Direction::Input,
                                None,
                                cx,
                            )),
                    )
                    .child(div().w(px(1.)).bg(theme::HAIRLINE))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(group_title(t!("Output defaults")))
                            .child(stream_editor(
                                shell,
                                Editor::Global,
                                Direction::Output,
                                None,
                                cx,
                            )),
                    ),
            ),
        )
        .child(group(
            t!("Device buffer policy"),
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(choices(
                    shell,
                    Editor::Global,
                    "latency",
                    shell.settings.global.profile,
                    [
                        AudioLatencyProfile::Compatible,
                        AudioLatencyProfile::Balanced,
                        AudioLatencyProfile::Low,
                    ]
                    .map(|profile| (profile, t!(profile.title()).into())),
                    |draft, profile| draft.profile = profile,
                    cx,
                ))
                .child(label(t!(shell.settings.global.profile.description())))
                .child(div().h(px(1.)).bg(theme::HAIRLINE))
                .child(row(
                    &t!("Device overrides"),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(label(t!(
                            "%{input_count} input / %{output_count} output",
                            input_count = config.input_overrides.len(),
                            output_count = config.output_overrides.len()
                        )))
                        .child(
                            button("settings-overrides-devices", t!("Configure in Devices"))
                                .on_click(move |_, _, cx| {
                                    overrides_this.update(cx, |shell, cx| {
                                        shell.set_page(Page::Devices, cx)
                                    })
                                }),
                        ),
                )),
        ))
}

fn interface(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let this = cx.entity();
    let meters = shell.preferences.meters;
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(heading(t!("Interface")))
        .child(group(
            t!("General"),
            rows([
                row(
                    &t!("Language"),
                    pref_choices(
                        shell,
                        "language",
                        shell.preferences.language,
                        crate::i18n::UiLanguage::ALL
                            .map(|language| (language, language.title().to_owned())),
                        |shell, language, window, cx| shell.set_language(language, window, cx),
                        cx,
                    ),
                ),
                row(
                    &t!("Density"),
                    pref_choices(
                        shell,
                        "density",
                        shell.density,
                        [
                            (Density::Comfortable, t!("Comfortable").into()),
                            (Density::Compact, t!("Compact").into()),
                        ],
                        |shell, density, _, cx| shell.set_density(density, cx),
                        cx,
                    ),
                ),
                row(
                    &t!("Restore last project at launch"),
                    component::switch::Switch::new("settings-restore-project")
                        .checked(shell.preferences.restore_last_project)
                        .on_click({
                            let this = this.clone();
                            move |checked, _, cx| {
                                this.update(cx, |shell, cx| {
                                    shell.set_restore_last_project(*checked, cx);
                                })
                            }
                        }),
                ),
            ]),
        ))
        .child(group(
            t!("Meters"),
            rows([
                row(
                    &t!("Peak fall-off"),
                    pref_choices(
                        shell,
                        "meter-falloff",
                        meters.falloff,
                        crate::project::MeterFalloff::ALL
                            .map(|falloff| (falloff, format!("{} dB/s", falloff.db_per_second()))),
                        |shell, falloff, _, cx| shell.set_meter_falloff(falloff, cx),
                        cx,
                    ),
                ),
                row(
                    &t!("Peak hold"),
                    pref_choices(
                        shell,
                        "peak-hold",
                        meters.peak_hold,
                        crate::project::PeakHold::ALL.map(|hold| (hold, peak_hold_title(hold))),
                        |shell, hold, _, cx| shell.set_peak_hold(hold, cx),
                        cx,
                    ),
                ),
            ]),
        ))
}

fn peak_hold_title(hold: crate::project::PeakHold) -> String {
    if hold == crate::project::PeakHold::Off {
        t!("Off").into_owned()
    } else {
        format!("{} s", hold.duration().as_secs_f32())
    }
}

fn diagnostics(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let requested = &snapshot.audio_settings;
    let running = snapshot.phase == SessionPhase::Running;
    let requested_column = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(label(t!("Requested")))
        .child(detail(&t!("Backend"), t!(requested.backend.title())))
        .child(detail(&t!("Processing"), processing_summary(requested)))
        .child(detail(
            &t!("Input"),
            stream_summary(requested.backend, requested.input),
        ))
        .child(detail(
            &t!("Output"),
            stream_summary(requested.backend, requested.output),
        ))
        .child(detail(
            &t!("Device buffer policy"),
            t!(snapshot.latency_profile.title()),
        ));
    let columns = match (running, snapshot.active_audio_settings.as_ref()) {
        (true, Some(active)) => div()
            .flex()
            .gap(px(28.))
            .child(requested_column)
            .child(div().w(px(1.)).bg(theme::HAIRLINE))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(label(t!("Active")))
                    .child(detail(&t!("Backend"), t!(active.backend.title())))
                    .child(detail(&t!("Processing"), processing_summary(active)))
                    .child(detail(
                        &t!("Input"),
                        stream_summary(active.backend, active.input),
                    ))
                    .child(detail(
                        &t!("Output"),
                        stream_summary(active.backend, active.output),
                    ))
                    .when_some(snapshot.active_latency_profile, |column, profile| {
                        column.child(detail(&t!("Device buffer policy"), t!(profile.title())))
                    }),
            ),
        _ => requested_column,
    };
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(heading(t!("Diagnostics")))
                .child(badge(
                    crate::ui::presentation::phase_text(snapshot),
                    if running {
                        theme::SUCCESS
                    } else {
                        theme::TEXT_MUTED
                    },
                )),
        )
        .child(card_shell().p(px(16.)).child(columns))
        // The timing rows carry their own names, so the card stays untitled
        // rather than repeating one.
        .child(card_shell().px(px(16.)).pt(px(8.)).pb(px(8.)).child(rows(
            crate::ui::presentation::native_timing_rows(snapshot)
                .into_iter()
                .map(|(key, value)| detail(&key, value)),
        )))
        .child(group(
            t!("System"),
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(rows([detail(
                    &t!("Catalog"),
                    t!(
                        "%{inputs} inputs / %{outputs} outputs / %{asio} ASIO drivers / %{apps} applications",
                        inputs = snapshot.catalog.inputs.len(),
                        outputs = snapshot.catalog.outputs.len(),
                        asio = snapshot.catalog.asio_drivers.len(),
                        apps = snapshot.catalog.processes.len()
                    ),
                )]))
                .child(label(crate::ui::presentation::control_feedback(
                    snapshot,
                    state.pending_request,
                )))
                .when_some(snapshot.catalog.asio_error.clone(), |view, error| {
                    view.child(
                        div()
                            .id("settings-asio-error")
                            .text_color(theme::DANGER)
                            .child(t!("ASIO discovery failed: %{error}", error = error).to_string())
                            .gpui_test_observe(),
                    )
                })
                .when_some(shell.project_message.clone(), |view, text| {
                    view.child(label(text))
                }),
        ))
        .child(
            div().self_start().child(
                button("quit-application", t!("Quit Moiren"))
                    .variant(ButtonVariant::Danger)
                    .on_click(|_, _, cx| cx.quit()),
            ),
        )
}

pub(super) fn processing_summary(audio: &AudioSettings) -> String {
    format!(
        "{} / {} / {} frames",
        rate_title(audio.processing.sample_rate),
        t!(audio.processing.precision.title()),
        audio.processing.block_frames
    )
}

pub(super) fn stream_summary(backend: AudioBackend, stream: StreamSettings) -> String {
    let encoding = if backend == AudioBackend::Asio {
        t!("Driver encoding").into()
    } else {
        format!(
            "{} / {}",
            if stream.mode == DeviceMode::Shared {
                t!("Shared")
            } else {
                t!("Exclusive")
            },
            t!(stream.format.title())
        )
    };
    format!(
        "{} / {} / {}",
        stream
            .sample_rate
            .map(rate_title)
            .unwrap_or_else(|| t!("Follow processing").into()),
        encoding,
        stream
            .buffer_frames
            .map(|frames| format!("{frames} frames"))
            .unwrap_or_else(|| t!("Driver default").into())
    )
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests;
