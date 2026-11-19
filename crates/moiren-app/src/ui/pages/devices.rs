use super::*;
use crate::ui::shell::panel::{CachedPanel, PanelHost, PublishRef};
use crate::ui::widgets::overlay_scrollbar::{notify_owner, scroll_region};
use controls::*;
use gpui_kit::base::ScrollbarAxis;
use rust_i18n::t;
use std::borrow::Cow;

mod exports;
pub(crate) use exports::ExportControls;
pub(crate) mod monitor;
use monitor::DeviceKey;
mod overrides;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceTab {
    Inputs,
    #[default]
    Outputs,
    Exports,
    Clock,
}

/// The devices page behind a [`PanelHost`]: rebuilt only when the published
/// snapshot, the active tab, the selected device, or an export draft input
/// changed.
pub(crate) struct DevicesPanel;

#[derive(PartialEq)]
pub(crate) struct DevicesKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
    device_tab: DeviceTab,
    selected_device: Option<String>,
    /// The rows read the settings editor's device draft (the Override badge
    /// and the saved-override rows). The draft does not compare cheaply —
    /// `AudioSettings` carries the override maps — so the key rides the
    /// draft's edit revision (`Draft::revision`) instead of cloning the
    /// value; see the field's doc in `settings::model` for the rationale.
    device_revision: u64,
    exports_scroll: Point<Pixels>,
    /// The device list scrolls inside the cached page (same
    /// scroll-under-cache rationale as `exports_scroll`).
    device_list_scroll: Pixels,
}

pub(crate) type DevicesPanelHost = PanelHost<DevicesPanel>;

impl CachedPanel for DevicesPanel {
    type Owner = StudioShell;
    type Key = DevicesKey;

    fn key(&self, shell: &Self::Owner, cx: &App) -> Self::Key {
        Self::Key {
            snapshot: PublishRef::new(std::sync::Arc::clone(&shell.state.read(cx).snapshot)),
            device_tab: shell.devices.device_tab,
            selected_device: shell.devices.selected_device.clone(),
            device_revision: shell.settings.editor.device.revision,
            // The export list scrolls inside the cached page; the offset
            // rides in the key so wheel steps rebuild it (the mixer's
            // precedent for scrollable containers under a cached panel).
            exports_scroll: shell.devices.exports_scroll.offset(),
            device_list_scroll: shell.devices.device_list_scroll.offset().y,
        }
    }

    fn render(
        &self,
        shell: &mut Self::Owner,
        _window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement {
        page(shell, cx)
    }

    fn observe_nested(
        &self,
        owner: &Entity<Self::Owner>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // The export drafts' inputs are model entities nested in this cached
        // tree; typing must re-render through explicit observers.
        let inputs = owner.read_with(cx, |shell, _| {
            shell.devices.export_controls.input_entities()
        });
        inputs
            .into_iter()
            .map(|input| cx.observe(&input, |host, _, cx| host.notify_nested(cx)))
            .collect()
    }
}

/// The devices page's cached splice. The Exports tab stays live: its list
/// scrolls under the cached subtree, and a rebuild's scroll clamp resets the
/// tracked offset between frames (the scroll-under-cache hazard in design
/// 17) — until that has a framework-level answer, the tab keeps the
/// rebuild-every-notify behavior it always had.
/// The Devices page's shell-owned state (design 20 §4): the active tab and
/// its selection, the export editor drafts, the page's scroll containers,
/// and the cached DevicesPanel host.
pub(crate) struct DevicesPageState {
    pub(crate) device_tab: DeviceTab,
    pub(crate) selected_device: Option<String>,
    pub(crate) export_controls: ExportControls,
    pub(crate) device_list_scroll: ScrollHandle,
    pub(crate) device_override_scroll: ScrollHandle,
    pub(crate) exports_scroll: ScrollHandle,
    pub(crate) panel: Entity<DevicesPanelHost>,
}

pub(crate) fn mount(
    shell: &StudioShell,
    _window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    if shell.devices.device_tab == DeviceTab::Exports {
        return page(shell, cx);
    }
    shell
        .devices
        .panel
        .clone()
        .cached(StyleRefinement::default().flex_1().min_w_0().min_h_0())
        .into_any_element()
}

pub(crate) fn page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let catalog = &snapshot.catalog;
    let mut tabs = div()
        .id("devices-tabs")
        .role(Role::TabList)
        .flex()
        .h_full()
        .items_center()
        .gap(px(theme::SPACE_1));
    for (id, title, tab) in [
        ("devices-inputs", t!("Inputs"), DeviceTab::Inputs),
        ("devices-outputs", t!("Outputs"), DeviceTab::Outputs),
        // Design 23 §3.1: exports are the user-visible "virtual devices";
        // the code identifiers keep the historical export names.
        ("devices-exports", t!("Virtual devices"), DeviceTab::Exports),
        ("devices-clock", t!("Clock"), DeviceTab::Clock),
    ] {
        tabs = tabs.child(devices_tab(shell, cx, id, title, tab));
    }
    let mut list = div()
        .id("device-list")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(theme::SPACE_2))
        .track_scroll(&shell.devices.device_list_scroll)
        .restrict_scroll_to_axis()
        .overflow_y_scroll()
        .on_scroll_wheel(notify_owner(cx.entity()));
    match shell.devices.device_tab {
        DeviceTab::Exports => {
            list = exports::list(shell, cx);
        }
        DeviceTab::Inputs | DeviceTab::Outputs => {
            list = physical_device_list(shell, cx, list);
        }
        DeviceTab::Clock => {
            list = list.child(heading(t!("Active audio clocks")));
            let views = crate::ui::presentation::native_timing_views(snapshot);
            if views.processing.is_none() && views.streams.is_empty() {
                list = list.child(empty_state(
                    "clock-empty",
                    "icons/clock.svg",
                    None,
                    t!("No active native timing. Start audio to inspect the actual stream clocks."),
                ));
            } else {
                if let Some((key, value)) = views.processing {
                    list = list.child(
                        card("clock-processing")
                            .gap(px(theme::SPACE_2))
                            .child(section_title(key))
                            .child(mono_caption(value))
                            .gpui_test_observe(),
                    );
                }
                for view in views.streams {
                    list = list.child(
                        card(SharedString::from(format!("clock-stream-{:?}", view.node)))
                            .gap(px(theme::SPACE_2))
                            .child(heading(view.device_title.clone()))
                            .child(caption(format!("{} / {}", view.node_label, view.role())))
                            .children(
                                view.rows
                                    .into_iter()
                                    .map(|(key, value)| detail_mono(&key, value)),
                            )
                            .gpui_test_observe(),
                    );
                }
            }
        }
    }
    div()
        .id("devices-page")
        // Definite extents first (`size_full`), grow second (`flex_1`): a
        // cached panel lays its content out as a layout root, where grow has
        // no parent to grow against and a `flex_1`-only root hugs its
        // content — the whole page collapsing to its widest row (the mixer
        // page root and the chrome status bar share this rule).
        .size_full()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(theme::PAGE_TOOLBAR_HEIGHT))
                .items_center()
                .justify_between()
                .px(px(theme::SPACE_4))
                .gap(px(theme::SPACE_2))
                .child(tabs)
                .child(
                    button("refresh-devices", t!("Refresh devices"))
                        .disabled(state.pending_request)
                        .on_click({
                            let this = cx.entity();
                            move |_, _, cx| {
                                this.update(cx, |s, cx| {
                                    s.state.update(cx, |state, _| {
                                        state.dispatch(AppRequest::RefreshCatalog)
                                    });
                                    cx.notify();
                                })
                            }
                        }),
                ),
        )
        .when_some(catalog.asio_error.clone(), |view, error| {
            view.child(
                div()
                    .id("devices-asio-error")
                    .flex_none()
                    .px(px(theme::SPACE_4))
                    .py(px(theme::SPACE_2))
                    .bg(theme::soft(theme::DANGER))
                    .text_color(theme::DANGER)
                    .child(t!(
                        "ASIO discovery failed: %{error}. Refresh devices to retry.",
                        error = error
                    ))
                    .gpui_test_observe(),
            )
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .p(px(theme::SPACE_4))
                .child(
                    // The reading column: one capped measure, left-anchored
                    // like the mixer's strips. Deliberately not centered: the
                    // inspector column opens and closes beside this row, and
                    // a centered column would slide the whole list sideways
                    // on every toggle (the settings page can center because
                    // nothing resizes its host). The wrapper mirrors the
                    // list's own flex sizing: in a real viewport the pair
                    // stretches to the padded row, in a degenerate replay
                    // layout the rows' content height keeps them visible
                    // (percentage heights would collapse to zero there). The
                    // tab decides which list and bar are mounted — stacking
                    // both bars' viewport hitboxes would deafen the list
                    // beneath them to the wheel.
                    scroll_region(
                        match shell.devices.device_tab {
                            DeviceTab::Exports => "scrollbar-virtual-export-list",
                            _ => "scrollbar-device-list",
                        },
                        match shell.devices.device_tab {
                            DeviceTab::Exports => &shell.devices.exports_scroll,
                            _ => &shell.devices.device_list_scroll,
                        },
                        ScrollbarAxis::Vertical,
                        list.gpui_test_observe(),
                    )
                    .flex_none()
                    .w(px(theme::READING_COLUMN))
                    .max_w_full()
                    .h_full()
                    .flex()
                    .flex_col()
                    .min_h_0(),
                ),
        )
        .gpui_test_observe()
        .into_any_element()
}

/// The page's category tab: the appbar navigation's language at page scale —
/// quiet text that carries the 2px accent underline while active, instead of
/// a filled button. `Role::Tab` keeps the strip one accessible tab list.
fn devices_tab(
    shell: &StudioShell,
    cx: &Context<StudioShell>,
    id: &'static str,
    title: impl Into<SharedString>,
    tab: DeviceTab,
) -> impl IntoElement {
    let active = shell.devices.device_tab == tab;
    let this = cx.entity();
    div()
        .id(id)
        .role(Role::Tab)
        .aria_selected(active)
        .relative()
        .flex()
        .h_full()
        .items_center()
        .px(px(theme::SPACE_3))
        .cursor_pointer()
        .text_size(px(theme::TEXT_SM))
        .line_height(px(16.))
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .text_color(if active {
            theme::TEXT
        } else {
            theme::TEXT_MUTED
        })
        .hover(|style| style.bg(theme::controls::RAISED))
        .child(title.into())
        .when(active, |tab| {
            tab.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(theme::ACCENT_BAR_W))
                    .bg(theme::controls::FOCUS),
            )
        })
        .on_click(move |_, _, cx| {
            this.update(cx, |s, cx| {
                s.devices.device_tab = tab;
                s.devices.selected_device = None;
                cx.notify();
            })
        })
        .gpui_test_observe()
}

/// The page's card shell: raised surface, hairline border, one radius — the
/// settings group card's visual language carried over to the devices page's
/// browsing cards. Callers add their own vertical rhythm (`.gap`).
pub(super) fn card(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .p(px(theme::SPACE_4))
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::HAIRLINE)
        .rounded(px(theme::RADIUS_MD))
}

/// A section's title inside a card or the inspector: one step under the page
/// heading so the section reads as a unit (the settings group card's title).
pub(super) fn section_title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(theme::TEXT_MD))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

/// A technical metric line: mono caption in the muted color. Stream
/// parameter summaries and clock values live here — numbers that update
/// live, so the mono face keeps them from drifting sideways.
pub(super) fn mono_caption(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(theme::FONT_MONO)
        .text_size(px(theme::TEXT_CAPTION))
        .text_color(theme::TEXT_MUTED)
        .child(text.into())
}

/// A plain caption line for non-numeric metadata (a card's direction /
/// channel shape, a clock group's node / role): the caption size in the UI
/// face — mono stays reserved for numeric readouts.
pub(super) fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(theme::TEXT_CAPTION))
        .text_color(theme::TEXT_MUTED)
        .child(text.into())
}

/// The read-only user rows shared by the device inspector and the virtual
/// device card (design 23 §3.3/§3.4): who the candidate graph routes through
/// `device`, with the engine's live stream facts under each running user and
/// a jump to the node. The empty case stays with the caller — the two
/// surfaces word it differently and the tests anchor their ids. `ids` maps a
/// user node to its `(row id, reveal button id)`.
pub(super) fn user_rows(
    snapshot: &crate::service::AppSnapshot,
    device: &DeviceKey,
    ids: impl Fn(moiren_core::graph::NodeId) -> (SharedString, SharedString),
    cx: &Context<StudioShell>,
) -> Vec<AnyElement> {
    let users = monitor::device_users(snapshot, device);
    let timing = crate::ui::presentation::native_timing_views(snapshot);
    let mut rows = Vec::new();
    for user in &users {
        let node = user.node;
        let (row_id, reveal_id) = ids(node);
        let state_text = match user.state {
            monitor::UserState::Running => t!("Running").to_string(),
            monitor::UserState::NotRunning => {
                if snapshot.phase == SessionPhase::Running {
                    t!("Waiting to take effect").to_string()
                } else {
                    t!("Not running").to_string()
                }
            }
            monitor::UserState::Detached => {
                t!("Binding changed; the old stream is still running").to_string()
            }
        };
        let this = cx.entity();
        rows.push(
            div()
                .id(row_id)
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(theme::SPACE_2))
                .child(label(user.label.clone()).min_w_0().truncate())
                .child(if user.state == monitor::UserState::Running {
                    badge(state_text, theme::SIGNAL_DEVICE).into_any_element()
                } else {
                    div()
                        .text_size(px(theme::TEXT_SM))
                        .text_color(if user.state == monitor::UserState::Detached {
                            theme::WARNING
                        } else {
                            theme::TEXT_MUTED
                        })
                        .child(state_text)
                        .into_any_element()
                })
                .child(
                    button(reveal_id, t!("Locate in graph")).on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| shell.reveal_node(node, cx));
                    }),
                )
                .gpui_test_observe()
                .into_any_element(),
        );
        // The engine's live facts under a running user, mono so the frame
        // counters hold their width while they tick.
        for view in timing
            .streams
            .iter()
            .filter(|view| view.node == node && user.state == monitor::UserState::Running)
        {
            for (key, value) in &view.rows {
                rows.push(detail_mono(key, value.clone()).into_any_element());
            }
        }
    }
    rows
}

/// The input/output tabs: physical device rows, each answering what it is,
/// how it is configured, and who is using it (design 23 §2/§3.2). The row
/// itself is the only interaction — it opens the device's detail view.
fn physical_device_list(
    shell: &StudioShell,
    cx: &Context<StudioShell>,
    list: Stateful<Div>,
) -> Stateful<Div> {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let catalog = &snapshot.catalog;
    let input = shell.devices.device_tab == DeviceTab::Inputs;
    let asio = snapshot.audio_settings.backend == moiren_engine::audio_config::AudioBackend::Asio;
    let available = if input {
        crate::ui::state::input_rows(catalog, snapshot.audio_settings.backend)
    } else {
        crate::ui::state::output_rows(catalog, snapshot.audio_settings.backend)
    };
    let mut rows = available.clone();
    let saved = if input {
        &shell.settings.editor.device.audio.input_overrides
    } else {
        &shell.settings.editor.device.audio.output_overrides
    };
    for id in saved.keys() {
        if !catalog.vad_endpoints.values().any(|owned| owned == id)
            && !rows.iter().any(|row| &row.endpoint_id == id)
        {
            rows.push(crate::service::DeviceRow {
                endpoint_id: id.clone(),
                name: t!("Unavailable device (saved override)").into(),
            });
        }
    }
    let mut list = list;
    if rows.is_empty() {
        return list.child(empty_state(
            "device-list-empty",
            "icons/devices.svg",
            Some(t!("No devices available").into()),
            if asio {
                t!("Refresh to discover installed ASIO drivers.")
            } else {
                t!("Refresh to discover connected Windows audio endpoints.")
            },
        ));
    }
    let timing = crate::ui::presentation::native_timing_views(snapshot);
    for row in &rows {
        let present = available
            .iter()
            .any(|entry| entry.endpoint_id == row.endpoint_id);
        let inspected = shell.devices.selected_device.as_ref() == Some(&row.endpoint_id);
        let default = if input {
            catalog.default_input_endpoint_id.as_ref() == Some(&row.endpoint_id)
        } else {
            catalog.default_output_endpoint_id.as_ref() == Some(&row.endpoint_id)
        };
        let device = DeviceKey::Physical {
            endpoint_id: row.endpoint_id.clone(),
            input,
        };
        let users = monitor::device_users(snapshot, &device);
        let live = users
            .iter()
            .filter(|user| user.state != monitor::UserState::NotRunning)
            .count();
        let bound_idle = users.len() - live;
        let saved_request = if input {
            snapshot.audio_settings.effective_input(&row.endpoint_id)
        } else {
            snapshot.audio_settings.effective_output(&row.endpoint_id)
        };
        let summary = timing
            .streams
            .iter()
            .find(|view| {
                users.iter().any(|user| {
                    user.node == view.node && user.state != monitor::UserState::NotRunning
                })
            })
            .map(|view| view.summary.clone())
            .unwrap_or_else(|| {
                settings::stream_summary(snapshot.audio_settings.backend, saved_request)
            });
        let this = cx.entity();
        let id = row.endpoint_id.clone();
        let click_id = id.clone();
        list = list.child(
            div()
                .id(SharedString::from(format!("device-row-{id}")))
                .flex()
                .flex_col()
                .gap(px(theme::SPACE_1_5))
                .px(px(theme::SPACE_4))
                .py(px(theme::SPACE_3))
                .bg(theme::SURFACE)
                .border_1()
                .border_color(if inspected {
                    theme::controls::FOCUS
                } else {
                    theme::HAIRLINE
                })
                .rounded(px(theme::RADIUS_MD))
                .cursor_pointer()
                .hover(|style| style.bg(theme::controls::RAISED))
                // Design 23 §3.2/R2: opening the detail view is the row's
                // only interaction. Routing and run state live on the
                // graph, never here.
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.inspect_device(click_id.clone(), cx))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::SPACE_2))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                // Design 21 §2: Windows device names are
                                // unbounded user text — one line, ellipsized.
                                .child(heading(row.name.clone()).min_w_0().truncate()),
                        )
                        // Status facts first (what the device is doing),
                        // config identity after (how it is set up); the
                        // group hugs the row's right edge.
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .flex_none()
                                .items_center()
                                .justify_end()
                                .gap(px(theme::SPACE_1_5))
                                .when(!present, |v| {
                                    v.child(badge(t!("Unavailable"), theme::WARNING))
                                })
                                .when(live > 0, |v| {
                                    v.child(badge(
                                        t!("In use ×%{count}", count = live),
                                        theme::SIGNAL_DEVICE,
                                    ))
                                })
                                .when(bound_idle > 0, |v| {
                                    v.child(badge(
                                        t!("Bound ×%{count}", count = bound_idle),
                                        theme::TEXT_MUTED,
                                    ))
                                })
                                .when(default && !asio, |v| {
                                    v.child(badge(t!("Default"), theme::TEXT_MUTED))
                                })
                                .when(saved.contains_key(&id), |v| {
                                    v.child(badge(t!("Override"), theme::controls::FOCUS))
                                }),
                        ),
                )
                .child(mono_caption(summary))
                .gpui_test_observe(),
        );
    }
    list
}

/// The devices page's inspector: the selected device's configuration and
/// monitoring (design 23 §3.3), or one orienting sentence per tab when no
/// device is selected.
pub(crate) fn inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    if matches!(
        shell.devices.device_tab,
        DeviceTab::Inputs | DeviceTab::Outputs
    ) {
        if shell.devices.selected_device.is_some() {
            return div()
                .id("device-override-host")
                .flex()
                .flex_none()
                .w_full()
                .h_full()
                .child(overrides::inspector(shell, cx))
                .into_any_element();
        }
        return inspector_hint(
            shell,
            "icons/devices.svg",
            t!("Select a device to view its configuration and monitoring."),
        );
    }
    let (icon, sentence) = match shell.devices.device_tab {
        DeviceTab::Exports => (
            "icons/devices.svg",
            t!("Create and configure virtual devices here. Bind them on Graph input/output nodes."),
        ),
        _ => (
            "icons/clock.svg",
            t!("The clock overview groups every live stream by device. Start audio to see it."),
        ),
    };
    inspector_hint(shell, icon, sentence)
}

fn inspector_hint(
    shell: &StudioShell,
    icon: &'static str,
    sentence: Cow<'static, str>,
) -> AnyElement {
    panel_host(
        "device-inspector",
        &shell.inspector.scroll,
        panel("device-inspector", &shell.inspector.scroll)
            .child(empty_state("device-inspector-empty", icon, None, sentence))
            .gpui_test_observe(),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests;
