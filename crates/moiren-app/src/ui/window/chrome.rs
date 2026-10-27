use rust_i18n::t;

use super::*;
use controls::*;

pub(super) fn toolbar(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    #[cfg(feature = "gpui-test")]
    {
        shell.mixer.trace.borrow_mut().toolbar_renders += 1;
    }
    let this = cx.entity();
    let state = shell.state.read(cx);
    let limit = if shell.compact { 16 } else { 24 };
    let project_label = if shell.project_name.chars().count() > limit {
        format!(
            "{}…",
            shell
                .project_name
                .chars()
                .take(limit - 1)
                .collect::<String>()
        )
    } else {
        shell.project_name.clone()
    };
    let running = matches!(
        state.snapshot.phase,
        SessionPhase::Running | SessionPhase::Starting
    );
    let mut tabs = div()
        .id("workspace-tabs")
        .role(Role::TabList)
        .flex()
        .h_full()
        .items_center();
    for (id, page) in [
        ("tab-graph", Page::Graph),
        ("tab-mixer", Page::Mixer),
        ("tab-matrix", Page::Matrix),
        ("tab-devices", Page::Devices),
    ] {
        let this = this.clone();
        tabs = tabs.child(
            WorkspaceTab::new(id, t!(page.title()))
                .icon(match page {
                    Page::Graph => ActionIcon::Graph,
                    Page::Mixer => ActionIcon::Mixer,
                    Page::Matrix => ActionIcon::Matrix,
                    Page::Devices => ActionIcon::Devices,
                })
                .active(shell.presentation.page == page)
                .on_click(move |_, _, cx| this.update(cx, |shell, cx| shell.set_page(page, cx))),
        );
    }
    div()
        .id("app-toolbar")
        .flex()
        .flex_none()
        .h(px(theme::TOOLBAR_HEIGHT))
        .items_center()
        .justify_between()
        .px(px(theme::SPACE_2))
        .gap(px(theme::SPACE_2))
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(
            div()
                .relative()
                .flex()
                .items_center()
                .gap(px(theme::SPACE_2))
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .window_control_area(WindowControlArea::Drag)
                        .child(gpui_kit::img("brand/mark.svg").size(px(24.)))
                        .when(!shell.compact, |v| v.child(heading("Moiren"))),
                )
                .child(
                    ActionButton::new("project-menu", project_label)
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Small)
                        .on_click({
                            let this = this.clone();
                            move |_, _, cx| {
                                this.update(cx, |shell, cx| {
                                    shell.project_menu = !shell.project_menu;
                                    cx.notify();
                                })
                            }
                        }),
                ),
        )
        .child(titlebar_drag_region("titlebar-drag-leading"))
        .child(tabs)
        .child(titlebar_drag_region("titlebar-drag-trailing"))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .child(
                    button("transport", if running { t!("Stop") } else { t!("Start") })
                        .variant(if running {
                            ButtonVariant::Danger
                        } else {
                            ButtonVariant::Primary
                        })
                        .disabled(
                            state.pending_request
                                || matches!(
                                    state.snapshot.phase,
                                    SessionPhase::Stopping
                                        | SessionPhase::Exiting
                                        | SessionPhase::Exited
                                )
                                || (!running && !state.can_start_transport()),
                        )
                        .on_click({
                            let this = this.clone();
                            move |_, _, cx| {
                                this.update(cx, |shell, cx| {
                                    shell.state.update(cx, |state, _| state.toggle_transport());
                                    cx.notify();
                                })
                            }
                        }),
                )
                .child(
                    IconButton::new("open-settings", t!("Settings"), ActionIcon::Settings)
                        .on_click({
                            let this = this.clone();
                            move |_, window, cx| {
                                this.update(cx, |shell, cx| {
                                    shell.open_settings(window, cx);
                                })
                            }
                        }),
                )
                .child(
                    WindowControlButton::new("win-min", WindowAction::Minimize)
                        .on_click(|_, window, _| window.minimize_window()),
                )
                .child(
                    WindowControlButton::new("win-max", WindowAction::Maximize).on_click(
                        |_, window, _| {
                            if let Err(error) = super::native::toggle_maximize(window) {
                                eprintln!("Moiren maximize: {error}");
                            }
                        },
                    ),
                )
                .child(
                    WindowControlButton::new("win-close", WindowAction::Close).on_click({
                        let this = cx.entity().downgrade();
                        move |_, window, cx| {
                            let _ = this.update(cx, |shell, cx| shell.close_window(window, cx));
                        }
                    }),
                ),
        )
        .gpui_test_observe()
}

/// Chrome is a sibling of the animated meter view, never a cached parent of
/// the controls. Rebuilding a cached parent forces its nested caches fresh.
pub(super) struct CachedChrome {
    shell: WeakEntity<StudioShell>,
    toolbar: bool,
    _subscription: Subscription,
}

impl CachedChrome {
    pub fn new(shell: Entity<StudioShell>, toolbar: bool, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&shell, |_, _, cx| cx.notify());
        Self {
            shell: shell.downgrade(),
            toolbar,
            _subscription: subscription,
        }
    }
}

impl Render for CachedChrome {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(shell) = self.shell.upgrade() else {
            return div().into_any_element();
        };
        shell.update(cx, |shell, cx| {
            let content = if self.toolbar {
                toolbar(shell, cx).into_any_element()
            } else {
                #[cfg(feature = "gpui-test")]
                {
                    shell.mixer.trace.borrow_mut().status_renders += 1;
                }
                statusbar(shell.state.read(cx))
                    .gpui_test_observe()
                    .into_any_element()
            };
            // A cached view is laid out as a root. Give the original flex row
            // its allocated width explicitly, not its intrinsic content width.
            div().size_full().child(content).into_any_element()
        })
    }
}

pub(super) fn toolbar_for_page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    if shell.presentation.page == Page::Mixer {
        shell
            .mixer_toolbar
            .clone()
            .cached(
                StyleRefinement::default()
                    .w_full()
                    .h(px(theme::TOOLBAR_HEIGHT))
                    .flex_none(),
            )
            .into_any_element()
    } else {
        toolbar(shell, cx).into_any_element()
    }
}

pub(super) fn status_for_page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    if shell.presentation.page == Page::Mixer {
        shell
            .mixer_status
            .clone()
            .cached(StyleRefinement::default().w_full().h(px(28.)).flex_none())
            .into_any_element()
    } else {
        statusbar(shell.state.read(cx))
            .gpui_test_observe()
            .into_any_element()
    }
}

fn titlebar_drag_region(id: &'static str) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .min_w(px(20.0))
        .h_full()
        .window_control_area(WindowControlArea::Drag)
        .gpui_test_observe()
}

/// Resolves an endpoint id against the catalog, falling back to the raw id
/// when the device has vanished since the session started.
fn endpoint_name<'a>(rows: &'a [crate::service::DeviceRow], endpoint_id: &'a str) -> &'a str {
    rows.iter()
        .find(|row| row.endpoint_id == endpoint_id)
        .map(|row| row.name.as_str())
        .unwrap_or(endpoint_id)
}

/// Status-bar names must survive long Windows device names.
fn truncate_name(name: &str) -> String {
    const LIMIT: usize = 24;
    if name.chars().count() > LIMIT {
        format!("{}…", name.chars().take(LIMIT - 1).collect::<String>())
    } else {
        name.to_string()
    }
}

/// "In name → Out name" for the desired session chain; software sources name
/// themselves instead of borrowing a device row.
fn route_label(
    source: &crate::service::InputSelection,
    output_endpoint_id: &str,
    catalog: &crate::service::DeviceCatalog,
) -> String {
    let source = match source {
        crate::service::InputSelection::Physical { endpoint_id } => {
            truncate_name(endpoint_name(&catalog.inputs, endpoint_id))
        }
        crate::service::InputSelection::Process {
            pid,
            creation_time_100ns,
        } => catalog
            .processes
            .iter()
            .find(|row| row.pid == *pid && row.creation_time_100ns == *creation_time_100ns)
            .map(|row| truncate_name(&row.name))
            .unwrap_or_else(|| format!("#{pid}")),
        crate::service::InputSelection::Tone { frequency_hz } => {
            t!("Tone generator · %{hz} Hz", hz = frequency_hz).to_string()
        }
    };
    let sink = truncate_name(endpoint_name(&catalog.outputs, output_endpoint_id));
    t!("%{source} → %{sink}", source = source, sink = sink).to_string()
}

/// One compact audio-format line: rate, period, and the period's duration.
/// Running monitors report measured device truth; everything else shows the
/// configured processing values instead of an empty bar.
fn format_text(
    rate: u32,
    period: Option<u32>,
    precision: moiren_engine::audio_config::ProcessingPrecision,
) -> String {
    use moiren_engine::audio_config::ProcessingPrecision;
    if rate == 0 {
        return "—".into();
    }
    let khz = format!("{:.1}", f64::from(rate) / 1000.);
    let short = match precision {
        ProcessingPrecision::F32 => "f32",
        ProcessingPrecision::F64 => "f64",
    };
    match period.filter(|frames| *frames != 0) {
        Some(frames) => {
            let ms = f64::from(frames) * 1000. / f64::from(rate);
            t!(
                "%{khz} kHz · %{frames} smp · %{ms} ms · %{precision}",
                khz = khz,
                frames = frames,
                ms = format!("{ms:.1}"),
                precision = short
            )
            .to_string()
        }
        None => t!(
            "%{khz} kHz · period unknown · %{precision}",
            khz = khz,
            precision = short
        )
        .to_string(),
    }
}

pub(super) fn statusbar(state: &UiState) -> Stateful<Div> {
    let snapshot = &state.snapshot;
    let running = snapshot.phase == SessionPhase::Running;
    let color = if running {
        theme::SUCCESS
    } else if snapshot.phase == SessionPhase::Failed {
        theme::DANGER
    } else {
        theme::TEXT_FAINT
    };
    let settings = snapshot
        .active_audio_settings
        .as_ref()
        .unwrap_or(&snapshot.audio_settings);
    let (rate, period) = match crate::ui::presentation::measured_master_output(snapshot) {
        Some((rate, frames)) => (rate, Some(frames)),
        None => (
            settings.processing.sample_rate,
            Some(settings.processing.block_frames),
        ),
    };
    div()
        .id("engine-statusbar")
        .flex()
        .flex_none()
        .h(px(28.))
        .px(px(theme::SPACE_3))
        .items_center()
        .justify_between()
        .gap(px(theme::SPACE_3))
        .bg(theme::SURFACE)
        .border_t_1()
        .border_color(theme::BORDER)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(theme::SPACE_3))
                .min_w_0()
                .child(badge(crate::ui::presentation::phase_text(snapshot), color))
                .when(running || !snapshot.elapsed.is_zero(), |view| {
                    view.child(label(crate::ui::presentation::elapsed_text(snapshot)))
                })
                .when_some(
                    snapshot.desired.as_ref().map(|desired| {
                        route_label(
                            &desired.source,
                            &desired.output_endpoint_id,
                            &snapshot.catalog,
                        )
                    }),
                    |view, route| {
                        view.child(
                            div()
                                .id("statusbar-route")
                                .min_w_0()
                                .overflow_x_hidden()
                                .text_size(px(theme::TEXT_SM))
                                .text_color(theme::TEXT_MUTED)
                                .child(route)
                                .gpui_test_observe(),
                        )
                    },
                ),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(theme::SPACE_3))
                .child(
                    div()
                        .id("statusbar-format")
                        .text_size(px(theme::TEXT_SM))
                        .text_color(theme::TEXT_MUTED)
                        .child(format_text(rate, period, settings.processing.precision))
                        .gpui_test_observe(),
                ),
        )
}

#[cfg(test)]
mod tests {
    // Explicit imports: a glob here would shadow the built-in #[test] with
    // the gpui test macro re-exported through the window module chain.
    use super::{format_text, route_label};
    use crate::service::{DeviceCatalog, DeviceRow, InputSelection};
    use moiren_engine::audio_config::ProcessingPrecision;

    fn catalog_with(named: &[(&str, &str)]) -> DeviceCatalog {
        let mut catalog = DeviceCatalog::default();
        for (endpoint_id, name) in named {
            catalog.outputs.push(DeviceRow {
                endpoint_id: (*endpoint_id).into(),
                name: (*name).into(),
            });
        }
        catalog
    }

    // Expected strings assert on the interpolated facts (names, numbers,
    // units) rather than whole translations so they hold under any locale.

    #[test]
    fn route_names_tone_sources_and_resolves_catalog_rows() {
        let catalog = catalog_with(&[("out-1", "Studio monitors")]);
        let label = route_label(
            &InputSelection::Tone { frequency_hz: 440. },
            "out-1",
            &catalog,
        );
        assert!(label.contains("440"), "names the frequency: {label}");
        assert!(label.contains("Studio monitors"), "names the sink: {label}");
        assert!(label.contains('\u{2192}'), "joins source and sink: {label}");
    }

    #[test]
    fn route_falls_back_to_the_endpoint_id_for_vanished_devices() {
        let label = route_label(
            &InputSelection::Physical {
                endpoint_id: "gone-input".into(),
            },
            "gone-output",
            &DeviceCatalog::default(),
        );
        assert!(label.contains("gone-input"), "{label}");
        assert!(label.contains("gone-output"), "{label}");
    }

    #[test]
    fn route_truncates_long_windows_device_names() {
        let catalog = catalog_with(&[(
            "out-1",
            "Very Long Manufacturer Name With Many Words (High Definition Audio)",
        )]);
        let label = route_label(
            &InputSelection::Tone { frequency_hz: 440. },
            "out-1",
            &catalog,
        );
        assert!(label.chars().count() < 60, "label stays compact: {label}");
        assert!(
            label.ends_with('\u{2026}'),
            "truncated name ends with ellipsis"
        );
    }

    #[test]
    fn format_shows_rate_period_duration_and_precision() {
        let text = format_text(48_000, Some(256), ProcessingPrecision::F32);
        assert!(text.contains("48.0"), "rate in kHz: {text}");
        assert!(text.contains("256"), "period in samples: {text}");
        assert!(text.contains("5.3"), "period duration: {text}");
        assert!(text.contains("f32"), "precision: {text}");
    }

    #[test]
    fn format_admits_an_unknown_period_instead_of_inventing_one() {
        let text = format_text(96_000, None, ProcessingPrecision::F64);
        assert!(text.contains("96.0"), "{text}");
        assert!(text.contains("f64"), "{text}");
        assert!(!text.contains(" smp"), "no invented period: {text}");
        assert_eq!(
            format_text(0, Some(256), ProcessingPrecision::F32),
            "\u{2014}"
        );
    }
}
