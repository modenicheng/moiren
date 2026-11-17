//! The status bar's five panel bodies. Each is a pure projection of the
//! shell state at render time: the bar re-renders every drawn frame, so
//! nothing here is cached and the live readouts (process rows, CPU history,
//! XRUN strip) simply read the models. Painting shares the two canvas
//! helpers with the pages' plots; colors stay on the theme's status tokens.

use std::time::{Duration, Instant};

use rust_i18n::t;

use super::*;
use crate::ui::monitoring::perf::ThreadRow;
use moiren_engine::threads::ThreadClass;

/// One second of one-minute history per point.
const HISTORY_LEN: usize = crate::ui::monitoring::perf::HISTORY_LEN;
const PAD: f32 = 10.;
const STRIP_HEIGHT: f32 = 16.;
const CPU_COLUMN: f32 = 52.;
const MARK_WIDTH: f32 = 2.;
/// The XRUN strip's window: now sits at its right edge.
const STRIP_WINDOW: Duration = Duration::from_secs(60);

fn panel_width(item: StatusItem) -> f32 {
    match item {
        StatusItem::Engine => 280.,
        StatusItem::Clock => 320.,
        StatusItem::Xrun => 300.,
        StatusItem::Perf => 360.,
        StatusItem::Format => 320.,
    }
}

/// The open panel for `item`, hung above the bar and fading in. Left-side
/// items align their panel's left edge with their own; right-side items
/// align right, so the panels never lean past the window edge.
pub(super) fn panel(
    shell: &StudioShell,
    cx: &Context<StudioShell>,
    item: StatusItem,
) -> AnyElement {
    let this = cx.entity();
    let right = matches!(
        item,
        StatusItem::Xrun | StatusItem::Perf | StatusItem::Format
    );
    let body = match item {
        StatusItem::Engine => engine(shell, cx),
        StatusItem::Clock => clock(shell, cx),
        StatusItem::Xrun => xrun(shell, cx),
        StatusItem::Perf => perf(shell, cx),
        StatusItem::Format => format(shell, cx),
    };
    let content = div()
        .id(item.panel_id())
        .absolute()
        .bottom(px(theme::STATUSBAR_HEIGHT + theme::POPOVER_ANCHOR_GAP))
        .map(|panel| {
            if right {
                panel.right_0()
            } else {
                panel.left_0()
            }
        })
        .w(px(panel_width(item)))
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_2))
        .p(px(PAD))
        .rounded(px(theme::RADIUS_MD))
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER_STRONG)
        .text_size(px(theme::TEXT_SM))
        .text_color(theme::TEXT_MUTED)
        // What the panel covers belongs to the panel: no hover or press
        // reaches the page underneath.
        .occlude()
        .on_hover(move |hovered, _, cx| {
            this.update(cx, |shell, cx| shell.status_panel_hover(*hovered, cx))
        })
        .children(body)
        .gpui_test_observe();
    deferred(
        content.with_animation(item.fade_id(), Animation::new(FADE_IN), |panel, delta| {
            panel.opacity(delta)
        }),
    )
    .with_priority(1)
    .into_any_element()
}

// --- Shared pieces ----------------------------------------------------------

/// A panel header: title, optional hint, spacer, optional trailing control.
fn header(
    title: impl Into<SharedString>,
    sub: Option<SharedString>,
    trailing: Option<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(theme::SPACE_2))
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::TEXT)
                .child(title.into()),
        )
        .when_some(sub, |row, sub| {
            row.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::TEXT_CAPTION))
                    .text_color(theme::TEXT_FAINT)
                    .child(sub),
            )
        })
        .child(div().flex_1())
        .when_some(trailing, |row, trailing| row.child(trailing))
        .into_any_element()
}

/// A key-value readout row.
fn kv(key: impl Into<SharedString>, value: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .justify_between()
        .gap(px(theme::SPACE_3))
        .child(
            div()
                .flex_none()
                .text_color(theme::TEXT_FAINT)
                .child(key.into()),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(theme::TEXT)
                .child(value.into()),
        )
        .into_any_element()
}

/// A hairline separating the readouts from the chart.
fn divider() -> AnyElement {
    div()
        .h(px(theme::HAIRLINE_W))
        .bg(theme::HAIRLINE)
        .into_any_element()
}

fn note(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_color(theme::TEXT_FAINT)
        .child(text.into())
        .into_any_element()
}

// --- Audio engine -----------------------------------------------------------

fn engine(shell: &StudioShell, cx: &Context<StudioShell>) -> Vec<AnyElement> {
    let this = cx.entity();
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let phase = snapshot.phase;
    let running = matches!(phase, SessionPhase::Running | SessionPhase::Starting);
    let (color, description, description_color) = match phase {
        SessionPhase::Running => (
            theme::SIGNAL_DEVICE,
            t!("Realtime thread is running").into_owned(),
            theme::TEXT_MUTED,
        ),
        SessionPhase::Starting => (
            theme::TEXT_FAINT,
            t!("Opening audio devices…").into_owned(),
            theme::TEXT_MUTED,
        ),
        SessionPhase::Stopping => (
            theme::TEXT_FAINT,
            t!("Releasing audio devices…").into_owned(),
            theme::TEXT_MUTED,
        ),
        SessionPhase::Failed if snapshot.waiting => (
            theme::WARNING,
            t!("Waiting for audio devices to return").into_owned(),
            theme::TEXT_MUTED,
        ),
        SessionPhase::Failed => (
            theme::DANGER,
            snapshot
                .error
                .as_ref()
                .map(|error| t!(error.as_str()).into_owned())
                .unwrap_or_else(|| t!("The audio engine stopped with an error").into_owned()),
            theme::DANGER,
        ),
        // Start is start-by-graph (design 23 §4.2): the panel makes no
        // UI-side claim about the graph's readiness — the service is the one
        // place that validates, and its verdict lands on the nodes the
        // faults name.
        SessionPhase::Idle => (
            theme::WARNING,
            t!("Engine is idle").into_owned(),
            theme::TEXT_MUTED,
        ),
        SessionPhase::Exiting | SessionPhase::Exited => (
            theme::TEXT_FAINT,
            t!("Shutting down").into_owned(),
            theme::TEXT_MUTED,
        ),
    };
    let label = if running {
        t!("Stop engine")
    } else if phase == SessionPhase::Failed {
        t!("Restart engine")
    } else {
        t!("Start engine")
    };
    // Design 23 §4.2: the button stays clickable on an unstartable graph —
    // only an in-flight request or a transitional phase disables it.
    let disabled = state.pending_request
        || matches!(
            phase,
            SessionPhase::Stopping | SessionPhase::Exiting | SessionPhase::Exited
        );
    vec![
        header(
            t!("Audio engine"),
            None,
            Some(badge(presentation::phase_text(snapshot), color).into_any_element()),
        ),
        div()
            .text_color(description_color)
            .child(description)
            .into_any_element(),
        div()
            .flex()
            .child(
                ActionButton::new("status-engine-toggle", label)
                    .variant(if running {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Primary
                    })
                    .size(ControlSize::Small)
                    .icon(if running {
                        ActionIcon::Stop
                    } else {
                        ActionIcon::Play
                    })
                    .disabled(disabled)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.state.update(cx, |state, _| state.toggle_transport());
                            cx.notify();
                        })
                    }),
            )
            .into_any_element(),
    ]
}

// --- Master clock -----------------------------------------------------------

fn clock(shell: &StudioShell, cx: &Context<StudioShell>) -> Vec<AnyElement> {
    let snapshot = &shell.state.read(cx).snapshot;
    let mut rows = vec![header(
        t!("Master clock"),
        Some(
            t!("Sample clock source of the whole graph")
                .into_owned()
                .into(),
        ),
        None,
    )];
    let streams = presentation::output_streams(snapshot);
    if !streams.is_empty() {
        for stream in &streams {
            let detail = format!(
                "{} / {}",
                presentation::format_short_text(
                    stream.sample_rate,
                    Some(t!(stream.format.title()).into_owned().into()),
                ),
                presentation::buffer_text(stream.buffer_frames, stream.sample_rate),
            );
            rows.push(clock_row(
                if stream.master {
                    t!("Master output")
                } else {
                    t!("Follower output")
                },
                if stream.master {
                    theme::SIGNAL_DEVICE
                } else {
                    theme::TEXT_FAINT
                },
                super::sink_device_name(snapshot, stream.node).unwrap_or_else(|| "—".into()),
                Some(detail.into()),
            ));
        }
        if !streams.iter().any(|stream| stream.master) {
            rows.push(note(t!("Engine realtime clock; no hardware master")));
        }
        return rows;
    }
    // No stream is observed: the graph's own policy names the master. While
    // a session is active the first observation is merely still on its way
    // (they refresh once a second), so only an idle graph speaks of the
    // next start.
    let active = matches!(
        snapshot.phase,
        SessionPhase::Starting | SessionPhase::Running | SessionPhase::Stopping
    );
    let graph = &snapshot.graph;
    match crate::service::clock_master(&graph.graph, &graph.exports) {
        Some(node) => rows.push(clock_row(
            t!("Master output"),
            theme::SIGNAL_DEVICE,
            super::sink_device_name(snapshot, node).unwrap_or_else(|| "—".into()),
            (!active).then(|| t!("Takes effect at the next start").into_owned().into()),
        )),
        // No physical master — virtual outputs, capture-only, or an empty
        // graph: the engine's own realtime clock paces the timeline.
        None => rows.push(note(t!("Engine realtime clock; no hardware master"))),
    }
    rows
}

fn clock_row(
    role: impl Into<SharedString>,
    color: Hsla,
    name: String,
    detail: Option<SharedString>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_0_5))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(theme::SPACE_2))
                .child(badge(role, color))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(theme::TEXT)
                        .child(name),
                ),
        )
        .when_some(detail, |row, detail| {
            row.child(
                div()
                    .text_size(px(theme::TEXT_CAPTION))
                    .text_color(theme::TEXT_FAINT)
                    .child(detail),
            )
        })
        .into_any_element()
}

// --- XRUN statistics --------------------------------------------------------

fn xrun(shell: &StudioShell, cx: &Context<StudioShell>) -> Vec<AnyElement> {
    let this = cx.entity();
    let stats = &shell.state.read(cx).runtime.xruns;
    let now = Instant::now();
    let ages: Vec<f32> = stats
        .events()
        .iter()
        .filter_map(|(when, _)| {
            let age = now.duration_since(*when);
            (age <= STRIP_WINDOW).then_some(age.as_secs_f32())
        })
        .collect();
    let total = stats.total();
    let total_text = match stats.percent().filter(|_| total > 0) {
        Some(percent) => t!(
            "%{count} events (%{percent})",
            count = total,
            percent = presentation::percent_text(f64::from(percent))
        )
        .into_owned(),
        None => format!("{total}"),
    };
    let last = stats
        .last_event()
        .map(|when| presentation::ago_text(now.duration_since(when)))
        .unwrap_or_else(|| "—".into());
    let longest = stats
        .started_at()
        .map(|_| presentation::span_text(stats.longest_clean(now)))
        .unwrap_or_else(|| "—".into());
    vec![
        header(
            t!("XRUN statistics"),
            Some(t!("Events in the last 60 s").into_owned().into()),
            Some(
                ActionButton::new("status-xrun-reset", t!("Reset"))
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Small)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell
                                .state
                                .update(cx, |state, _| state.runtime.xruns.reset(Instant::now()));
                            cx.notify();
                        })
                    })
                    .into_any_element(),
            ),
        ),
        xrun_strip(ages),
        kv(t!("Total XRUNs"), total_text),
        kv(t!("Last event"), last),
        kv(t!("Longest clean run"), longest),
    ]
}

/// The 60-second event strip: one tick per shortfall, newest at the right
/// edge. XRUNs are discrete events, so the strip shows their clustering and
/// the time since the last one — a line chart of a counter would show
/// neither.
fn xrun_strip(ages: Vec<f32>) -> AnyElement {
    let empty = ages.is_empty();
    div()
        .relative()
        .w_full()
        .h(px(STRIP_HEIGHT))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    // The axis sits along the bottom edge.
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.left(), bounds.bottom() - px(theme::HAIRLINE_W)),
                            size(bounds.size.width, px(theme::HAIRLINE_W)),
                        ),
                        theme::BORDER,
                    ));
                    let width = f32::from(bounds.size.width);
                    for age in &ages {
                        let x = ((1. - age / STRIP_WINDOW.as_secs_f32()) * width)
                            .clamp(0., width - MARK_WIDTH);
                        window.paint_quad(fill(
                            Bounds::new(
                                point(bounds.left() + px(x), bounds.top() + px(1.)),
                                size(px(MARK_WIDTH), bounds.size.height - px(3.)),
                            ),
                            theme::WARNING,
                        ));
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .when(empty, |strip| {
            strip.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(theme::TEXT_CAPTION))
                    .text_color(theme::TEXT_FAINT)
                    .child(t!("No events in the last 60 s")),
            )
        })
        .into_any_element()
}

// --- Performance ------------------------------------------------------------

/// The per-thread row's fixed columns: the rate figure and the CPU share.
const RATE_COLUMN: f32 = 62.;
/// The over-budget count column: a "!" marker plus the window's count.
const WARN_COLUMN: f32 = 30.;
/// The two sparklines' height: a quarter of the old system chart, enough to
/// read a shape without owning the panel.
const MINI_CHART_HEIGHT: f32 = 28.;

fn perf(shell: &StudioShell, cx: &Context<StudioShell>) -> Vec<AnyElement> {
    let this = cx.entity();
    let dsp = shell.state.read(cx).runtime.dsp();
    let threads = shell.chrome.perf.threads();
    let mut body = vec![header(
        t!("Performance"),
        Some(
            t!("Moiren's own threads, refreshed every second")
                .into_owned()
                .into(),
        ),
        Some(
            ActionButton::new("status-perf-reset", t!("Reset"))
                .variant(ButtonVariant::Ghost)
                .size(ControlSize::Small)
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.chrome.perf.reset_threads();
                        cx.notify();
                    })
                })
                .into_any_element(),
        ),
    )];
    if !threads.sampled() {
        body.push(note(t!("Sampling…")));
    } else if !threads.os_data() {
        body.push(note(t!("Thread CPU sampling is Windows-only")));
    }
    let thread_rows = threads.rows();
    let rt: Vec<_> = thread_rows
        .iter()
        .filter(|row| row.class == ThreadClass::Rt)
        .collect();
    if !rt.is_empty() {
        // The RT group collapses on its own when the session stops: the
        // threads exit, their registry rows go, and no zombie rows remain.
        body.push(group_caption(t!("Realtime threads")));
        body.extend(rt.iter().map(|row| thread_row(row)));
        body.push(mini_chart(
            threads.master_history().iter().copied().collect(),
        ));
    }
    let control: Vec<_> = thread_rows
        .iter()
        .filter(|row| row.class == ThreadClass::Control)
        .collect();
    if !control.is_empty() {
        body.push(group_caption(t!("Control threads")));
        body.extend(control.iter().map(|row| thread_row(row)));
    }
    let ui: Vec<_> = thread_rows
        .iter()
        .filter(|row| row.class == ThreadClass::Ui)
        .collect();
    if !ui.is_empty() {
        body.push(group_caption(t!("User interface")));
        body.extend(ui.iter().map(|row| thread_row(row)));
    }
    body.push(group_caption(t!("Plugin host")));
    let plugins: Vec<_> = thread_rows
        .iter()
        .filter(|row| row.class == ThreadClass::Plugin)
        .collect();
    if plugins.is_empty() {
        // The Phase 3 slot: a placeholder, not a dead row — plugin host
        // threads will appear here as they register.
        body.push(note(t!("No plugin host thread")));
    } else {
        body.extend(plugins.iter().map(|row| thread_row(row)));
    }
    let (other_percent, other_threads) = threads.other();
    if other_threads > 0 {
        body.push(group_caption(t!("Other threads")));
        body.push(
            div()
                .flex()
                .items_center()
                .gap(px(theme::SPACE_2))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(theme::TEXT_MUTED)
                        .child(t!("%{count} unnamed", count = other_threads)),
                )
                .child(
                    div()
                        .flex_none()
                        .font_family(theme::FONT_MONO)
                        .text_color(theme::TEXT_MUTED)
                        .child(
                            other_percent
                                .map(|percent| format!("{percent:.1}%"))
                                .unwrap_or_else(|| "—".into()),
                        ),
                )
                .into_any_element(),
        );
        if threads.native_callbacks_advanced() {
            body.push(note(t!("Includes the driver-owned ASIO callback thread")));
        }
    }
    body.push(divider());
    // The bottom totals: Moiren's own threads first, then the machine (the
    // retained system accounting, downgraded to one line — DWM and audiodg
    // competing for cores are invisible from inside the process).
    let own_total = thread_rows
        .iter()
        .filter_map(|row| row.cpu_percent)
        .sum::<f32>();
    body.push(kv(t!("Moiren threads"), format!("{own_total:.1}%")));
    body.push(kv(
        t!("Total CPU / last 60 s"),
        shell
            .chrome
            .perf
            .current()
            .map(|percent| format!("{percent:.1}%"))
            .unwrap_or_else(|| "—".into()),
    ));
    body.push(mini_chart(
        shell.chrome.perf.history().iter().copied().collect(),
    ));
    if let Some(load) = dsp {
        body.push(kv(
            t!("DSP load"),
            t!(
                "%{average} avg / %{peak} peak",
                average = format!("{:.1}%", load.average),
                peak = format!("{:.1}%", load.peak)
            ),
        ));
    }
    body
}

/// A group's quiet caption row.
fn group_caption(caption: impl Into<SharedString>) -> AnyElement {
    div()
        .text_size(px(theme::TEXT_CAPTION))
        .text_color(theme::TEXT_FAINT)
        .child(caption.into())
        .into_any_element()
}

/// The sparkline height for both kept charts (the RT master row and the
/// bottom system total): a quarter of the old chart, enough to read a shape.
fn mini_chart(history: Vec<f32>) -> AnyElement {
    cpu_chart(history, MINI_CHART_HEIGHT)
}

/// One per-thread row: color dot, name, class badge, then the rate, CPU and
/// over-budget columns.
fn thread_row(row: &ThreadRow) -> AnyElement {
    let (dot, badge_text) = match row.class {
        ThreadClass::Rt => (theme::SIGNAL_DEVICE, "RT"),
        ThreadClass::Control => (theme::TEXT_FAINT, "CTL"),
        ThreadClass::Ui => (theme::TEXT_FAINT, "UI"),
        ThreadClass::Plugin => (theme::TEXT_FAINT, "VST"),
    };
    let rate = row
        .rate
        .map(|rate| format!("{rate:.0}/s"))
        .unwrap_or_else(|| "—".into());
    let cpu = row
        .cpu_percent
        .map(|percent| format!("{percent:.1}%"))
        .unwrap_or_else(|| "—".into());
    div()
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_0_5))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(theme::SPACE_2))
                .child(div().flex_none().size(px(DOT_SIZE)).rounded_full().bg(dot))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme::TEXT)
                        .child(row.name.clone()),
                )
                .child(badge(badge_text, dot))
                .child(
                    div()
                        .flex_none()
                        .w(px(RATE_COLUMN))
                        .flex()
                        .justify_end()
                        .font_family(theme::FONT_MONO)
                        .text_color(theme::TEXT_MUTED)
                        .child(rate),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(CPU_COLUMN))
                        .flex()
                        .justify_end()
                        .font_family(theme::FONT_MONO)
                        .text_color(theme::TEXT_MUTED)
                        .child(cpu),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(WARN_COLUMN))
                        .flex()
                        .justify_end()
                        .font_family(theme::FONT_MONO)
                        .when(row.over_budget > 0, |cell| {
                            cell.text_color(theme::WARNING)
                                .child(format!("!{}", row.over_budget))
                        }),
                ),
        )
        .when_some(row.detail.clone(), |row_el, detail| {
            row_el.child(
                div()
                    .text_size(px(theme::TEXT_CAPTION))
                    .text_color(theme::TEXT_FAINT)
                    .child(detail),
            )
        })
        .into_any_element()
}

/// Total CPU over the last minute at the given height: grid at the quarter
/// marks, the halo fill under the curve, and the curve itself in signal
/// cyan. The line hugs the right edge until a minute of history accumulates.
fn cpu_chart(history: Vec<f32>, height: f32) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let y_of =
                |percent: f32| bounds.top() + px(height - percent.clamp(0., 100.) / 100. * height);
            for mark in [25., 50., 75.] {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left(), y_of(mark)),
                        size(bounds.size.width, px(theme::HAIRLINE_W)),
                    ),
                    theme::HAIRLINE,
                ));
            }
            if history.len() < 2 {
                return;
            }
            let step = width / (HISTORY_LEN - 1) as f32;
            let x_of =
                |index: usize| bounds.right() - px((history.len() - 1 - index) as f32 * step);
            let mut area = PathBuilder::fill();
            area.move_to(point(x_of(0), bounds.bottom()));
            for (index, percent) in history.iter().enumerate() {
                area.line_to(point(x_of(index), y_of(*percent)));
            }
            area.line_to(point(bounds.right(), bounds.bottom()));
            area.close();
            if let Ok(path) = area.build() {
                window.paint_path(path, theme::SIGNAL_DEVICE.alpha(theme::ALPHA_SELECTION));
            }
            let mut line = PathBuilder::stroke(px(1.5));
            for (index, percent) in history.iter().enumerate() {
                let p = point(x_of(index), y_of(*percent));
                if index == 0 {
                    line.move_to(p);
                } else {
                    line.line_to(p);
                }
            }
            if let Ok(path) = line.build() {
                window.paint_path(path, theme::SIGNAL_DEVICE);
            }
        },
    )
    .w_full()
    .h(px(height))
    .into_any_element()
}

// --- Engine information -----------------------------------------------------

fn format(shell: &StudioShell, cx: &Context<StudioShell>) -> Vec<AnyElement> {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let settings = snapshot
        .active_audio_settings
        .as_ref()
        .unwrap_or(&snapshot.audio_settings);
    let master = presentation::master_stream(snapshot);
    let backend = match master.as_ref() {
        Some(stream) => backend_text(stream.backend, stream.mode),
        None => backend_text(settings.backend, settings.output.mode),
    };
    let mut rows = vec![header(t!("Engine information"), None, None)];
    rows.push(kv(t!("Backend"), backend));
    rows.push(kv(
        t!("Processing format"),
        presentation::format_short_text(
            settings.processing.sample_rate,
            Some(
                t!(settings.processing.precision.title())
                    .into_owned()
                    .into(),
            ),
        ),
    ));
    if let Some(stream) = master.as_ref() {
        rows.push(kv(t!("Device format"), t!(stream.format.title())));
        if let Some((input, output)) = stream.driver_latency {
            rows.push(kv(
                t!("Driver latency"),
                t!(
                    "In %{input} / out %{output} smp",
                    input = input,
                    output = output
                ),
            ));
        }
    }
    let (rate, frames) = super::rate_and_buffer(snapshot);
    rows.push(kv(t!("Buffer"), presentation::buffer_text(frames, rate)));
    let blocks = state.runtime.xruns.blocks();
    if blocks > 0 {
        rows.push(kv(t!("Rendered blocks"), format!("{blocks}")));
    }
    rows
}

fn backend_text(
    backend: moiren_engine::audio_config::AudioBackend,
    mode: moiren_engine::node::DeviceMode,
) -> String {
    match backend {
        moiren_engine::audio_config::AudioBackend::Asio => {
            t!("ASIO driver (driver owns mode)").into_owned()
        }
        moiren_engine::audio_config::AudioBackend::Wasapi => {
            format!("WASAPI / {}", t!(mode.title()))
        }
    }
}
