use super::*;
use controls::*;

pub(super) fn toolbar(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
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
            WorkspaceTab::new(id, page.title())
                .icon(match page {
                    Page::Graph => ActionIcon::Graph,
                    Page::Mixer => ActionIcon::Mixer,
                    Page::Matrix => ActionIcon::Matrix,
                    Page::Devices => ActionIcon::Devices,
                })
                .active(!shell.settings_open && shell.presentation.page == page)
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
                    button("transport", if running { "Stop" } else { "Start" })
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
                                || (!running && state.candidate_spec().is_none()),
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
                    IconButton::new("open-settings", "Settings", ActionIcon::Settings).on_click({
                        let this = this.clone();
                        move |_, _, cx| {
                            this.update(cx, |shell, cx| {
                                shell.settings_open = !shell.settings_open;
                                shell.project_menu = false;
                                cx.notify();
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
    let format = snapshot
        .monitor_diagnostics
        .filter(|d| d.generation == snapshot.generation && running)
        .map(|d| {
            format!(
                "{} Hz  ·  period {} frames",
                d.observation.prepared.output.sample_rate,
                d.observation
                    .prepared
                    .output
                    .actual_period_frames
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "—".into())
            )
        })
        .unwrap_or_default();
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
                .child(badge(state.phase_text(), color))
                .when(running || !snapshot.elapsed.is_zero(), |view| {
                    view.child(label(state.elapsed_text()))
                })
                .when(!format.is_empty(), |view| view.child(label(format))),
        )
}
