use rust_i18n::t;

use super::*;
use crate::ui::shell::panel::{CachedPanel, PanelHost, PublishRef};
use crate::ui::state::UiFailure;
use controls::*;
use std::sync::Arc;

pub(super) fn toolbar(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    #[cfg(feature = "gpui-test")]
    {
        shell.trace.borrow_mut().toolbar_renders += 1;
    }
    let this = cx.entity();
    // The toolbar's project label shares the status bar's truncation, at a
    // tighter limit when the window goes compact.
    let project_label = truncate_name(
        &shell.project_io.name,
        theme::compact(shell.chrome.compact, 16, STATUS_NAME_LIMIT),
    );
    let mut tabs = div()
        .id("workspace-tabs")
        .role(Role::TabList)
        .flex()
        .h_full()
        .items_center()
        // The strip participates in the toolbar's squeeze (design 21 §2),
        // but its basis must be definite: a flex container's auto basis is
        // measured from the children's *content* (~86 px), not their flex
        // bases (112 px), so an auto-width strip under-measures and its own
        // line overflows, shrinking every tab to content width at rest
        // ("Devi…"). Pin the rest width to the four bases and the floor to
        // the four shrink minimums; a squeezed toolbar then takes the strip
        // down to the tabs' `min_w` evenly.
        .flex_initial()
        .flex_basis(px(4. * crate::ui::theme::controls::TAB_WIDTH))
        .min_w(px(4. * crate::ui::theme::controls::TAB_MIN_WIDTH));
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
                // Settings shows no active tab: the page it resumes to is
                // underneath, not shown.
                .active(matches!(
                    shell.presentation.workspace,
                    WorkspaceView::Page(shown) if shown == page
                ))
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
                        .gap(px(theme::SPACE_2))
                        .window_control_area(WindowControlArea::Drag)
                        .child(gpui_kit::img("brand/mark.svg").size(px(24.)))
                        .when(!shell.chrome.compact, |v| v.child(heading("Moiren"))),
                )
                .child(
                    ActionButton::new("project-menu", project_label)
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Small)
                        .on_click({
                            let this = this.clone();
                            move |_, _, cx| {
                                this.update(cx, |shell, cx| {
                                    shell.chrome.project_menu = !shell.chrome.project_menu;
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
                .gap(px(theme::SPACE_1))
                .child(
                    // The gear is the page toggle (click again closes it) but
                    // carries no active state of its own: the page itself is
                    // the open indicator. The engine's start/stop control
                    // lives in the status bar's phase panel, next to the
                    // state it changes.
                    IconButton::new("open-settings", t!("Settings"), ActionIcon::Settings)
                        .on_click({
                            let this = this.clone();
                            move |_, window, cx| {
                                this.update(cx, |shell, cx| {
                                    shell.toggle_settings(window, cx);
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

/// The chrome panels are siblings of the animated meter views, never cached
/// parents of the page panels: rebuilding a cached parent forces its nested
/// caches fresh.
pub(super) type ToolbarChrome = PanelHost<ToolbarPanel>;
pub(super) type StatusbarChrome = PanelHost<StatusbarPanel>;

/// Everything the toolbar tree consumes. Shell notifications arrive far more
/// often than these inputs change (meter pumps are excluded by design — the
/// meter frames own the animation loops), so the key exists to keep those
/// notifications from rebuilding the chrome. The published snapshot is not
/// an input any more: the transport button that read the session phase moved
/// into the status bar's engine panel.
#[derive(PartialEq)]
pub(crate) struct ToolbarKey {
    /// One field for the page the tab strip highlights and whether Settings
    /// replaces the workspace: the former `page` + `settings_open` pair
    /// encoded exactly this, with no extra information.
    workspace: WorkspaceView,
    compact: bool,
    project_name: String,
}

pub(crate) struct ToolbarPanel;

impl CachedPanel for ToolbarPanel {
    type Owner = StudioShell;
    type Key = ToolbarKey;

    fn key(&self, shell: &StudioShell, _cx: &App) -> Self::Key {
        Self::Key {
            workspace: shell.presentation.workspace,
            compact: shell.chrome.compact,
            project_name: shell.project_io.name.clone(),
        }
    }

    fn render(
        &self,
        shell: &mut StudioShell,
        _window: &mut Window,
        cx: &mut Context<StudioShell>,
    ) -> AnyElement {
        // A cached view is laid out as a root. Give the original flex row its
        // allocated width explicitly, not its intrinsic content width.
        div()
            .size_full()
            .child(toolbar(shell, cx))
            .into_any_element()
    }
}

/// The status bar projects the published snapshot; the service dedups
/// publishes, so publish identity is most of the truth. The error summary
/// also renders `dispatch_error`, which `UiState::try_dispatch` writes
/// synchronously with no publish behind it — the old error strip's key
/// carried it for exactly this reason, and the summary inherited the job:
/// cloned only at capture time so a dispatch failure while the bar stands
/// still rebuilds on its own notification. The typed [`UiFailure`] keeps
/// that property — equality on the stored meaning, strictly stronger than
/// equality on a rendered string.
#[derive(PartialEq)]
pub(crate) struct StatusbarKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
    dispatch_error: Option<UiFailure>,
}

pub(crate) struct StatusbarPanel;

impl CachedPanel for StatusbarPanel {
    type Owner = StudioShell;
    type Key = StatusbarKey;

    fn key(&self, shell: &StudioShell, cx: &App) -> Self::Key {
        let state = shell.state.read(cx);
        Self::Key {
            snapshot: PublishRef::new(Arc::clone(&state.snapshot)),
            dispatch_error: state.dispatch_error.clone(),
        }
    }

    fn render(
        &self,
        shell: &mut StudioShell,
        _window: &mut Window,
        cx: &mut Context<StudioShell>,
    ) -> AnyElement {
        #[cfg(feature = "gpui-test")]
        {
            shell.trace.borrow_mut().status_renders += 1;
        }
        // A cached view is laid out as a root. Give the original flex row its
        // allocated width explicitly, not its intrinsic content width.
        div()
            .size_full()
            .child(statusbar(shell, cx).gpui_test_observe())
            .into_any_element()
    }
}

/// The window chrome's shell-owned state (design 20 §4): the toolbar and
/// status panel hosts, the project-menu toggle, the side-panel visibility
/// pair, the compact-width flag, the density preference, the fault
/// popover anchor, and the status bar's hover-panel state machine. None of
/// the status fields ride a panel key: the bar is rendered live every drawn
/// frame (`status_for_page`), and a key input would only add rebuild churn.
pub(crate) struct ShellChrome {
    pub(crate) project_menu: bool,
    pub(crate) sidebar_open: bool,
    pub(crate) inspector_open: bool,
    pub(crate) compact: bool,
    pub(crate) density: Density,
    pub(crate) toolbar: Entity<ToolbarChrome>,
    pub(crate) status: Entity<StatusbarChrome>,
    pub(crate) fault_popover: Option<fault_popover::FaultPopover>,
    /// The status bar item whose panel is open, if any.
    pub(crate) status_popup: Option<status::StatusItem>,
    /// Where the pointer rests, as the hover listeners last reported it:
    /// the item under it, and whether it is on the open panel. The panel
    /// state machine arbitrates on these facts, not on event order.
    pub(crate) status_hover_item: Option<status::StatusItem>,
    pub(crate) status_hover_panel: bool,
    /// The pending hover transition the status timer resolves.
    pub(crate) status_wait: Option<status::StatusWait>,
    /// The armed open-delay or close-grace timer; dropping the task cancels.
    pub(crate) status_timer: Option<Task<()>>,
    /// `Window::is_window_hovered`'s last observed value, so `status::sync`
    /// closes panels on the pointer-leaves-the-window transition.
    pub(crate) status_window_hovered: bool,
    /// The performance panel's CPU history and process table.
    pub(crate) perf: crate::ui::monitoring::perf::PerfModel,
    /// The process sampler, alive only while the performance panel is open.
    pub(crate) perf_task: Option<Task<()>>,
}

pub(super) fn toolbar_for_page(shell: &StudioShell, _window: &Window) -> AnyElement {
    // The chrome keeps its keyed `PanelHost` render dedup (`ToolbarChrome`
    // / `StatusbarChrome` above) but is deliberately NOT `.cached()`-spliced:
    // the toolbar and status bar carry the window drag regions
    // (`window_control_area(Drag)`), whose hitboxes are registered during
    // prepaint and cleared every frame. gpui-fast's splice replays
    // layout/paint/a11y but not those registrations, so a spliced chrome
    // would drop its drag regions from the platform's hit test after the
    // first cache hit — the title bar simply stopped dragging. Both bars
    // are cheap to rebuild; the pages stay cached.
    div()
        .w_full()
        .h(px(theme::TOOLBAR_HEIGHT))
        .flex_none()
        .child(shell.chrome.toolbar.clone())
        .into_any_element()
}

pub(super) fn status_for_page(shell: &StudioShell, _window: &Window) -> AnyElement {
    // See `toolbar_for_page`: the status bar's drag region needs a fresh
    // registration every frame.
    div()
        .w_full()
        .h(px(theme::STATUSBAR_HEIGHT))
        .flex_none()
        .child(shell.chrome.status.clone())
        .into_any_element()
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

/// Status-bar names must survive long Windows device names.
pub(super) const STATUS_NAME_LIMIT: usize = 24;

/// Truncates `name` to `limit` characters, reserving one slot for the
/// ellipsis so the result never exceeds the limit. Long Windows device names
/// and the toolbar's project label share it at their own limits.
pub(super) fn truncate_name(name: &str, limit: usize) -> String {
    if name.chars().count() > limit {
        format!("{}…", name.chars().take(limit - 1).collect::<String>())
    } else {
        name.to_string()
    }
}

pub(super) fn statusbar(shell: &StudioShell, cx: &Context<StudioShell>) -> Stateful<Div> {
    div()
        .id("engine-statusbar")
        .flex()
        .flex_none()
        .h(px(theme::STATUSBAR_HEIGHT))
        .px(px(theme::SPACE_2))
        .items_center()
        .justify_between()
        .gap(px(theme::SPACE_2))
        .bg(theme::SURFACE)
        .border_t_1()
        .border_color(theme::BORDER)
        // The left group yields: the clock label truncates before the bar
        // grows wide (design 21 §2). Both groups span the bar's height so an
        // item's anchor does too — the panels hang off the anchor's bottom
        // edge, which must be the bar's.
        .child(
            div()
                .flex()
                .h_full()
                .items_center()
                .gap(px(theme::SPACE_1))
                .min_w_0()
                .child(status::phase_item(shell, cx))
                // The unlocated errors' home after the error strip: a
                // VS Code-style red summary beside the phase item; clicking
                // opens the fault popover with the whole group.
                .when(
                    super::fault_popover::has_unlocated(shell.state.read(cx)),
                    |view| view.child(fault_summary(shell, cx)),
                )
                .child(status::clock_item(shell, cx)),
        )
        .child(
            div()
                .flex()
                .h_full()
                .min_w_0()
                .items_center()
                .gap(px(theme::SPACE_1))
                .child(status::xrun_item(shell, cx))
                .child(status::perf_item(shell, cx))
                .child(status::buffer_item(shell, cx))
                .child(status::format_item(shell, cx)),
        )
}

/// The status bar's error summary: a "!" marker plus the first unlocated
/// message in danger red, clickable into the fault popover with the whole
/// graph-wide group. Located faults never land here — they surface on
/// their own node card or mixer strip.
fn fault_summary(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    let this = cx.entity();
    let summary = super::fault_popover::unlocated_summary(shell.state.read(cx)).unwrap_or_default();
    div()
        .id("statusbar-faults")
        .flex()
        .flex_none()
        .min_w_0()
        .max_w(px(420.))
        .items_center()
        .gap(px(theme::SPACE_1_5))
        .overflow_x_hidden()
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            this.update(cx, |shell, cx| {
                let position = window.mouse_position();
                shell.chrome.fault_popover = Some(super::fault_popover::FaultPopover {
                    node: None,
                    anchor: Bounds {
                        origin: point(position.x, position.y - px(14.)),
                        size: size(px(2.), px(14.)),
                    },
                });
                cx.notify();
            })
        })
        .tooltip(|window, cx| {
            component::tooltip::Tooltip::new(t!("Show error details").into_owned())
                .build(window, cx)
        })
        .child(
            div()
                .flex_none()
                .size(px(14.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::DANGER)
                .text_color(theme::controls::ON_ACCENT)
                .text_size(px(theme::TEXT_TINY))
                .font_weight(FontWeight::SEMIBOLD)
                .child("!"),
        )
        .child(
            div()
                .min_w_0()
                .overflow_x_hidden()
                .text_size(px(theme::TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::DANGER)
                .child(summary),
        )
        .gpui_test_observe()
}
