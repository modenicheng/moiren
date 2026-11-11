//! The session popover: the project identity, the save/open actions, and
//! the recent-projects quick switch.
use super::*;
use crate::ui::widgets::occluding_page::{claim_presses, occluding_page};
use controls::*;
use rust_i18n::t;

/// The session popover: an occluding shell-root overlay (same dismissal
/// chrome as the settings menu and the fault popover) with the project
/// identity, the save/open actions, and the recent-projects quick switch.
/// The popover claims presses inside itself by stopping propagation, so
/// the overlay's outside handler closes it without any bounds math.
pub(in crate::ui::shell) fn menu(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let this = cx.entity();
    let close = this.clone();
    let save_pending = shell.project_io.busy || shell.state.read(cx).pending_request;
    let current = shell.project_io.path.clone();
    let recents = shell.preferences.values.recent_projects.clone();
    let mut popover = claim_presses(
        div()
            .id("project-popover")
            .absolute()
            .top(px(theme::TOOLBAR_HEIGHT))
            .left(px(32.))
            .w(px(280.))
            .p(px(theme::SPACE_3))
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_2))
            .bg(theme::SURFACE)
            .border_1()
            .border_color(theme::BORDER)
            .rounded(px(theme::RADIUS_MD))
            .occlude()
            .gpui_test_observe(),
    )
    .child(heading(shell.project_io.name.clone()))
    .child(label(
        shell
            .project_io
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| t!("Not saved to disk").into()),
    ))
    .child(
        button("open-project", t!("Open project…"))
            .disabled(shell.project_io.busy)
            .on_click({
                let this = this.clone();
                move |_, _, cx| this.update(cx, |s, cx| s.open_project(cx))
            }),
    )
    .child(
        button("save-project", t!("Save project"))
            .disabled(save_pending)
            .on_click({
                let this = this.clone();
                move |_, _, cx| this.update(cx, |s, cx| s.save_project(false, cx))
            }),
    )
    .child(
        button("save-project-as", t!("Save project as…"))
            .disabled(save_pending)
            .on_click({
                let this = this.clone();
                move |_, _, cx| this.update(cx, |s, cx| s.save_project(true, cx))
            }),
    );
    if !recents.is_empty() {
        popover = popover.child(label(t!("Recent projects")));
        for (index, path) in recents.iter().enumerate() {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let is_current = current.as_ref() == Some(path);
            popover = popover.child(
                button(SharedString::from(format!("recent-project-{index}")), name)
                    .full_width()
                    .justify_start()
                    .variant(ButtonVariant::Ghost)
                    .disabled(save_pending || is_current)
                    .on_click({
                        let this = this.clone();
                        let path = path.clone();
                        move |_, _, cx| {
                            this.update(cx, |s, cx| s.open_recent_project(path.clone(), cx))
                        }
                    }),
            );
        }
    }
    let popover = popover
        .when_some(shell.project_io.message.clone(), |v, message| {
            v.child(label(message_text(&message)))
        })
        .child(label(t!("Opening a project keeps audio stopped.")));
    occluding_page(
        "project-menu-page",
        true,
        move |cx| {
            close.update(cx, |shell, cx| {
                shell.chrome.project_menu = false;
                cx.notify();
            })
        },
        popover,
    )
}
