//! The status bar's interactive items and their hover-opened panels.
//!
//! Every item is a VS Code-style control: a quiet hover lift, a click, and —
//! for the informational ones — a panel that opens on hover and hangs above
//! the bar. The panels anchor in place inside their item (an absolute child
//! of the relative item, like the web prototype) instead of mounting at the
//! shell root behind an [`occluding_page`]: an occluding page would swallow
//! the bar's own hover, and the bar renders live every frame (it is not
//! `.cached()`-spliced, see `chrome::status_for_page`), so the open state
//! needs no key and no anchor-geometry plumbing.
//!
//! Open/close is a small state machine on [`chrome::ShellChrome`]: a cold
//! hover opens after an intent delay (so sweeping the bar toward the taskbar
//! does not flash panels), a second item switches at once, and leaving both
//! the anchor and the panel closes after a grace period long enough to cross
//! the gap between them. One panel is open at a time. The fork's Windows
//! platform never delivers a hover-clear when the pointer leaves the window
//! entirely, so [`sync`] watches `Window::is_window_hovered` and closes on
//! that transition — the bar sits on the window's bottom edge, the most
//! common exit path.
//!
//! [`occluding_page`]: crate::ui::widgets::occluding_page::occluding_page

use std::time::{Duration, Instant};

use rust_i18n::t;

use super::*;
use crate::service::AppSnapshot;
use crate::ui::presentation;
use controls::*;

mod panels;

/// The bar items that own a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusItem {
    Engine,
    Clock,
    Xrun,
    Perf,
    Format,
}

impl StatusItem {
    /// The bar button's element id.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Engine => "statusbar-phase",
            Self::Clock => "statusbar-clock",
            Self::Xrun => "statusbar-xrun",
            Self::Perf => "statusbar-perf",
            Self::Format => "statusbar-format",
        }
    }

    /// The panel's element id.
    pub(crate) fn panel_id(self) -> &'static str {
        match self {
            Self::Engine => "status-panel-engine",
            Self::Clock => "status-panel-clock",
            Self::Xrun => "status-panel-xrun",
            Self::Perf => "status-panel-perf",
            Self::Format => "status-panel-format",
        }
    }

    /// The fade animation's element id, one per item so a switch replays it.
    fn fade_id(self) -> &'static str {
        match self {
            Self::Engine => "status-fade-engine",
            Self::Clock => "status-fade-clock",
            Self::Xrun => "status-fade-xrun",
            Self::Perf => "status-fade-perf",
            Self::Format => "status-fade-format",
        }
    }
}

/// A pending hover transition the timer will resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusWait {
    Open(StatusItem),
    Close,
}

/// A cold hover must rest this long before its panel opens: crossing the
/// bar en route to the taskbar must not flash a panel at every item.
const OPEN_DELAY: Duration = Duration::from_millis(150);
/// Leaving both anchor and panel starts this countdown; crossing the
/// `POPOVER_ANCHOR_GAP` gap in between lands inside it.
const CLOSE_GRACE: Duration = Duration::from_millis(220);
/// The fade-in: quick enough to read as a response, slow enough to see.
const FADE_IN: Duration = Duration::from_millis(140);
/// The hover target's height inside the 28 px bar.
const ITEM_HEIGHT: f32 = 22.;
const DOT_SIZE: f32 = 8.;

/// Reconciles hover-open panels with the window's hovered flag: the first
/// render that observes the pointer gone closes the panel and drops the
/// pointer's resting facts with it. Re-entry always lands on a fresh
/// `on_hover`, which re-arms the open through the normal path — a re-entry
/// that never sends a mouse-move (title-bar activation) must not resurrect
/// the last item's panel. The latch keeps this transition-only, so headless
/// windows (never platform-hovered) leave their panels under the test
/// harness' synthetic mouse moves alone.
pub(crate) fn sync(shell: &mut StudioShell, window: &Window) {
    let hovered = window.is_window_hovered();
    if shell.chrome.status_window_hovered && !hovered {
        shell.status_disarm();
        shell.status_stop_process_sampling();
        shell.chrome.status_popup = None;
        shell.chrome.status_hover_item = None;
        shell.chrome.status_hover_panel = false;
    }
    shell.chrome.status_window_hovered = hovered;
}

impl StudioShell {
    /// The pointer entered or left a bar item.
    pub(crate) fn status_item_hover(
        &mut self,
        item: StatusItem,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        if hovered {
            self.chrome.status_hover_item = Some(item);
        } else if self.chrome.status_hover_item == Some(item) {
            self.chrome.status_hover_item = None;
        }
        self.status_hover_changed(cx);
    }

    /// The pointer entered or left the open panel itself.
    pub(crate) fn status_panel_hover(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.chrome.status_hover_panel = hovered;
        self.status_hover_changed(cx);
    }

    /// One arbitration for every hover transition, keyed on where the
    /// pointer rests — never on the transition itself. The deferred panel's
    /// listeners fire before the item's (later registration runs first in
    /// the bubble phase), so the enter and the leave of one pointer step can
    /// arrive in either order; only the resulting facts are stable.
    fn status_hover_changed(&mut self, cx: &mut Context<Self>) {
        let popup = self.chrome.status_popup;
        let wait = self.chrome.status_wait;
        match (
            self.chrome.status_hover_item,
            self.chrome.status_hover_panel,
        ) {
            (Some(item), _) if popup == Some(item) => self.status_disarm(),
            // Switching items while a panel is open: no intent delay, the
            // user is browsing the bar.
            (Some(item), _) if popup.is_some() => {
                self.status_disarm();
                self.status_open(item, cx);
            }
            (Some(item), _) => {
                if wait != Some(StatusWait::Open(item)) {
                    self.status_arm(StatusWait::Open(item), OPEN_DELAY, cx);
                }
            }
            (None, true) => self.status_disarm(),
            (None, false) if popup.is_some() => {
                if wait != Some(StatusWait::Close) {
                    self.status_arm(StatusWait::Close, CLOSE_GRACE, cx);
                }
            }
            // Neither rests under the pointer: a pending cold-open dies.
            (None, false) => self.status_disarm(),
        }
    }

    /// Clicking an item toggles its panel (touch has no hover to open it).
    pub(crate) fn status_toggle(&mut self, item: StatusItem, cx: &mut Context<Self>) {
        self.status_disarm();
        if self.chrome.status_popup == Some(item) {
            self.status_close(cx);
        } else {
            self.status_open(item, cx);
        }
    }

    /// A press landed outside every item and panel: close without grace.
    pub(crate) fn status_dismiss_outside(&mut self, cx: &mut Context<Self>) {
        if self.chrome.status_popup.is_some() {
            self.status_disarm();
            self.status_close(cx);
        }
    }

    pub(crate) fn status_close(&mut self, cx: &mut Context<Self>) {
        self.chrome.status_popup = None;
        self.chrome.status_hover_panel = false;
        self.status_stop_process_sampling();
        cx.notify();
    }

    fn status_open(&mut self, item: StatusItem, cx: &mut Context<Self>) {
        self.chrome.status_popup = Some(item);
        self.chrome.status_hover_panel = false;
        if item == StatusItem::Perf {
            self.status_start_process_sampling(cx);
        } else {
            self.status_stop_process_sampling();
        }
        cx.notify();
    }

    fn status_arm(&mut self, wait: StatusWait, delay: Duration, cx: &mut Context<Self>) {
        self.chrome.status_wait = Some(wait);
        self.chrome.status_timer = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |shell, cx| shell.status_wait_fired(wait, cx));
        }));
    }

    fn status_wait_fired(&mut self, wait: StatusWait, cx: &mut Context<Self>) {
        self.chrome.status_wait = None;
        self.chrome.status_timer = None;
        match wait {
            StatusWait::Open(item) => self.status_open(item, cx),
            StatusWait::Close => self.status_close(cx),
        }
    }

    fn status_disarm(&mut self) {
        self.chrome.status_wait = None;
        self.chrome.status_timer = None;
    }

    /// Samples the process table off the UI thread once a second while the
    /// performance panel is open. The first sample lands immediately so the
    /// rows appear at once, with zero CPU shares until a second table
    /// provides the delta.
    fn status_start_process_sampling(&mut self, cx: &mut Context<Self>) {
        if self.chrome.perf_task.is_some() {
            return;
        }
        let cores = std::thread::available_parallelism()
            .map(|cores| cores.get() as f32)
            .unwrap_or(1.);
        let own_pid = std::process::id();
        self.chrome.perf_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                let table = cx
                    .background_executor()
                    .spawn(async move { crate::system::sample_processes() })
                    .await;
                let open = this
                    .update(cx, |shell, cx| {
                        if shell.chrome.status_popup != Some(StatusItem::Perf) {
                            return false;
                        }
                        shell
                            .chrome
                            .perf
                            .ingest_processes(table, Instant::now(), cores, own_pid);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        }));
    }

    fn status_stop_process_sampling(&mut self) {
        self.chrome.perf_task = None;
        self.chrome.perf.forget_processes();
    }
}

// --- Bar items -------------------------------------------------------------

/// One bar item: the hover/click anchor plus, while open, its panel.
fn anchor(
    shell: &StudioShell,
    cx: &Context<StudioShell>,
    item: StatusItem,
    text_color: Hsla,
    fill: Option<Hsla>,
    content: impl IntoIterator<Item = AnyElement>,
) -> AnyElement {
    let this = cx.entity();
    let open = shell.chrome.status_popup == Some(item);
    div()
        .relative()
        .flex()
        .flex_none()
        .h_full()
        .items_center()
        // Presses on the item or its panel stop here: the shell root's
        // outside-press listener only sees presses that landed elsewhere.
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id(item.id())
                .flex()
                .items_center()
                .gap(px(theme::SPACE_1_5))
                .h(px(ITEM_HEIGHT))
                .px(px(theme::SPACE_2))
                .rounded(px(theme::RADIUS_SM))
                .text_size(px(theme::TEXT_SM))
                .text_color(text_color)
                .map(|el| match fill {
                    // The failed phase wears its fill at rest; hover only
                    // lifts it within the danger family.
                    Some(fill) => el
                        .bg(fill)
                        .hover(|style| style.bg(theme::controls::DANGER_HOVER)),
                    None => el
                        .hover(|style| style.bg(theme::SURFACE_4))
                        .when(open, |el| el.bg(theme::SURFACE_4)),
                })
                .cursor_pointer()
                .on_hover({
                    let this = this.clone();
                    move |hovered, _, cx| {
                        this.update(cx, |shell, cx| shell.status_item_hover(item, *hovered, cx))
                    }
                })
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.status_toggle(item, cx))
                })
                .children(content)
                .gpui_test_observe(),
        )
        .when(open, |bar| bar.child(panels::panel(shell, cx, item)))
        .into_any_element()
}

/// A small glyph for the bar items. A bare svg paints nothing without an
/// explicit color, so the caller always supplies one.
fn glyph(path: &'static str, color: Hsla) -> AnyElement {
    svg()
        .path(path)
        .flex_none()
        .size(px(theme::ICON_SM))
        .text_color(color)
        .into_any_element()
}

/// The phase item: run state text with the state marker, the engine panel
/// hanging off it. Running reads signal cyan as pure text, idle warning,
/// failed wears the danger fill, transitions stay faint behind a spinner.
pub(super) fn phase_item(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let snapshot = &shell.state.read(cx).snapshot;
    let phase = snapshot.phase;
    let (text_color, fill) = match phase {
        SessionPhase::Running => (theme::SIGNAL_DEVICE, None),
        SessionPhase::Idle => (theme::WARNING, None),
        SessionPhase::Failed => (theme::controls::ON_ACCENT, Some(theme::DANGER)),
        _ => (theme::TEXT_FAINT, None),
    };
    let marker = match phase {
        SessionPhase::Starting | SessionPhase::Stopping => spinner(text_color),
        SessionPhase::Idle => div()
            .flex_none()
            .size(px(DOT_SIZE))
            .rounded_full()
            .border_1()
            .border_color(text_color)
            .into_any_element(),
        _ => div()
            .flex_none()
            .size(px(DOT_SIZE))
            .rounded_full()
            .bg(text_color)
            .into_any_element(),
    };
    let label = div()
        .whitespace_nowrap()
        .when(fill.is_some(), |el| el.font_weight(FontWeight::SEMIBOLD))
        .child(presentation::phase_text(snapshot))
        .into_any_element();
    anchor(
        shell,
        cx,
        StatusItem::Engine,
        text_color,
        fill,
        [marker, label],
    )
}

/// The starting/stopping ring: the only animation on the bar, and only
/// while a phase transition is in flight. A perpetual running pulse would
/// own the frame pipeline for as long as audio runs — the idle-frame
/// discipline (#37/#40) forbids exactly that — so running wears a still dot.
fn spinner(color: Hsla) -> AnyElement {
    svg()
        .path("icons/loader-2.svg")
        .flex_none()
        .size(px(theme::ICON_SM))
        .text_color(color)
        .with_animation(
            "status-spinner",
            Animation::new(Duration::from_millis(800))
                .repeat()
                .with_max_fps(30.),
            |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
}

/// The master clock item: the observed master output's device while
/// running, the prospective master otherwise. The panel carries the full
/// names; the bar truncates.
pub(super) fn clock_item(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let snapshot = &shell.state.read(cx).snapshot;
    let name = match presentation::master_stream(snapshot) {
        Some(stream) => sink_device_name(snapshot, stream.node),
        None => prospective_master_name(snapshot),
    };
    let label = match name {
        Some(name) => chrome::truncate_name(&name, chrome::STATUS_NAME_LIMIT),
        None => "—".to_string(),
    };
    let content = [
        glyph("icons/clock.svg", theme::TEXT_FAINT),
        div().whitespace_nowrap().child(label).into_any_element(),
    ];
    anchor(
        shell,
        cx,
        StatusItem::Clock,
        theme::TEXT_MUTED,
        None,
        content,
    )
}

/// The display name of the device a sink node renders to. The raw catalog
/// is the honest source here — the clock identifies a device that is or will
/// be running, which can sit outside the settings page's backend-filtered
/// rows — and an unresolvable binding falls back to the node's own label.
pub(super) fn sink_device_name(
    snapshot: &AppSnapshot,
    node: moiren_core::graph::NodeId,
) -> Option<String> {
    let state = snapshot.graph.node(node)?;
    Some(
        state
            .output_endpoint_id
            .as_deref()
            .and_then(|endpoint| {
                snapshot
                    .catalog
                    .outputs
                    .iter()
                    .find(|row| row.endpoint_id == endpoint)
                    .map(|row| row.name.clone())
            })
            .unwrap_or_else(|| state.label.clone()),
    )
}

/// The clock source a start would use: the prospective master's device, or
/// the engine clock — an output-free or virtual-only graph runs on it too.
fn prospective_master_name(snapshot: &AppSnapshot) -> Option<String> {
    let graph = &snapshot.graph;
    match crate::service::clock_master(&graph.graph, &graph.exports) {
        Some(node) => sink_device_name(snapshot, node),
        None => Some(t!("Engine clock").into_owned()),
    }
}

/// The XRUN item: count plus the share of rendered blocks, warning colored
/// while any shortfall happened in this statistics window.
pub(super) fn xrun_item(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let stats = &shell.state.read(cx).runtime.xruns;
    let total = stats.total();
    let value = match stats.percent() {
        _ if total == 0 => "0".to_string(),
        Some(percent) => format!(
            "{total} / {}",
            presentation::percent_text(f64::from(percent))
        ),
        None => total.to_string(),
    };
    let value_color = if total > 0 {
        theme::WARNING
    } else {
        theme::TEXT_FAINT
    };
    let content = [
        glyph("icons/bolt-off.svg", theme::TEXT_FAINT),
        div().child(t!("XRUN")).into_any_element(),
        div()
            .font_family(theme::FONT_MONO)
            .text_color(value_color)
            .child(value)
            .into_any_element(),
    ];
    anchor(
        shell,
        cx,
        StatusItem::Xrun,
        theme::TEXT_MUTED,
        None,
        content,
    )
}

/// The performance item: the DSP load as the engine's render accounting
/// reports it, blank while no session renders.
pub(super) fn perf_item(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let value = match shell.state.read(cx).runtime.dsp() {
        Some(load) => format!("{:.1}%", load.average),
        None => "—".into(),
    };
    let content = [
        glyph("icons/cpu.svg", theme::TEXT_FAINT),
        div()
            .font_family(theme::FONT_MONO)
            .child(value)
            .into_any_element(),
    ];
    anchor(
        shell,
        cx,
        StatusItem::Perf,
        theme::TEXT_MUTED,
        None,
        content,
    )
}

/// Rate and buffer for the bar and the info panel: the measured master
/// output while running, the configured processing values otherwise.
pub(super) fn rate_and_buffer(snapshot: &AppSnapshot) -> (u32, u32) {
    match presentation::master_stream(snapshot) {
        Some(stream) => (stream.sample_rate, stream.buffer_frames),
        None => {
            let processing = snapshot
                .active_audio_settings
                .as_ref()
                .unwrap_or(&snapshot.audio_settings)
                .processing;
            (processing.sample_rate, processing.block_frames)
        }
    }
}

/// The buffer item: static text, no panel — the format item's panel owns
/// the surrounding facts.
pub(super) fn buffer_item(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let (rate, frames) = rate_and_buffer(&shell.state.read(cx).snapshot);
    div()
        .id("statusbar-buffer")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(theme::SPACE_1_5))
        .h(px(ITEM_HEIGHT))
        .px(px(theme::SPACE_2))
        .text_size(px(theme::TEXT_SM))
        .text_color(theme::TEXT_MUTED)
        .child(glyph("icons/gauge.svg", theme::TEXT_FAINT))
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .child(presentation::buffer_text(frames, rate)),
        )
        .gpui_test_observe()
        .into_any_element()
}

/// The format item: rate and bit depth — the measured device format while
/// running, the configured engine precision otherwise — with the engine
/// information panel hanging off it.
pub(super) fn format_item(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let snapshot = &shell.state.read(cx).snapshot;
    let (rate, depth) = match presentation::master_stream(snapshot) {
        Some(stream) => (stream.sample_rate, t!(stream.format.title()).into_owned()),
        None => {
            let processing = snapshot
                .active_audio_settings
                .as_ref()
                .unwrap_or(&snapshot.audio_settings)
                .processing;
            (
                processing.sample_rate,
                t!(processing.precision.title()).into_owned(),
            )
        }
    };
    let content = [
        glyph("icons/wave.svg", theme::TEXT_FAINT),
        div()
            .font_family(theme::FONT_MONO)
            .child(presentation::format_short_text(rate, Some(depth.into())))
            .into_any_element(),
    ];
    anchor(
        shell,
        cx,
        StatusItem::Format,
        theme::TEXT_MUTED,
        None,
        content,
    )
}
