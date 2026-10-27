//! A real channel spectrum and prepared FFT controls, shared by both surfaces.
use super::*;
use crate::ui::spectrum::SpectrumStatus;
use moiren_engine::spectrum::SpectrumSettings;

pub(in crate::ui::window::graph::node_controls) mod editor;
mod plot;

#[derive(Clone, Copy, Default)]
pub(in crate::ui::window::graph::node_controls) struct SpectrumView {
    pub(in crate::ui::window::graph::node_controls) channel: usize,
    range: DisplayRange,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum DisplayRange {
    Db60,
    #[default]
    Db90,
    Db120,
}

impl DisplayRange {
    fn floor(self) -> f64 {
        match self {
            Self::Db60 => -60.,
            Self::Db90 => -90.,
            Self::Db120 => -120.,
        }
    }
}

pub(in crate::ui::window::graph) fn inspector(
    shell: &StudioShell,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    content(
        shell,
        node.id,
        *node
            .builtin
            .resolved(NodeKind::SpectrumAnalyzer)
            .spectrum()
            .unwrap(),
        "inspector",
        cx,
    )
}

pub(super) fn content(
    shell: &StudioShell,
    id: NodeId,
    settings: SpectrumSettings,
    surface: &'static str,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let snapshot = &shell.state.read(cx).snapshot;
    // The service snapshot can change before the next display poll. Check its
    // identity-aware status before using a retained presentation observation.
    let status = shell.node_spectra.status(id, snapshot);
    let observation = (status == SpectrumStatus::Live)
        .then(|| shell.node_spectra.observation(id))
        .flatten();
    let active = snapshot.signal_topics.node_spectra.get(&id);
    let channels = observation.map_or_else(
        || {
            snapshot
                .graph
                .graph
                .get_node(id)
                .map_or(1, |node| node.channels())
        },
        |value| value.window.channels,
    );
    let view = shell
        .node_controls
        .spectrum
        .get(&id)
        .copied()
        .unwrap_or_default();
    let channel = view.channel.min(channels.saturating_sub(1));
    let status_short = match status {
        SpectrumStatus::Waiting => "Waiting",
        SpectrumStatus::Stale => "Stale",
        SpectrumStatus::Error(_) => "Error",
        _ => status.label(),
    };
    let mut selector = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(4.))
        .child(
            note(status_short.to_owned()).text_color(if observation.is_some() {
                theme::SIGNAL_DEVICE
            } else {
                theme::TEXT_MUTED
            }),
        );
    let mut navigation = div().flex().items_center().gap(px(4.));
    for (key, label, forwards) in [("prev", "<", false), ("next", ">", true)] {
        if forwards {
            navigation = navigation.child(
                note(format!("Ch {} / {}", channel + 1, channels))
                    .id(format!("spectrum-channel-readout-{surface}-{id:?}"))
                    .aria_label(format!("Ch {} / {}", channel + 1, channels))
                    .gpui_test_observe(),
            );
        }
        let this = cx.entity();
        navigation = navigation.child(
            controls::button(format!("spectrum-channel-{key}-{surface}-{id:?}"), label)
                .density(Density::Compact)
                .disabled(channels <= 1)
                .tooltip(t!(if forwards {
                    "Next channel"
                } else {
                    "Previous channel"
                }))
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        let view = shell.node_controls.spectrum.entry(id).or_default();
                        let current = view.channel.min(channels.saturating_sub(1));
                        view.channel = if forwards {
                            (current + 1) % channels.max(1)
                        } else {
                            (current + channels.saturating_sub(1)) % channels.max(1)
                        };
                        cx.notify();
                    });
                }),
        );
    }
    selector = selector.child(navigation);
    let mut ranges = div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(note("Floor").w(px(36.)));
    for range in [DisplayRange::Db60, DisplayRange::Db90, DisplayRange::Db120] {
        let this = cx.entity();
        ranges = ranges.child(
            controls::button(
                format!("spectrum-range-{surface}-{id:?}-{}", -range.floor()),
                format!("{:.0}", range.floor()),
            )
            .density(Density::Compact)
            .variant(if range == view.range {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.node_controls.spectrum.entry(id).or_default().range = range;
                    cx.notify();
                });
            }),
        );
    }
    ranges = ranges.child(note("dBFS"));
    let peak = observation.and_then(|value| plot::peak_bin(value, channel));
    let peak_text = peak.map_or_else(
        || t!("No live signal").to_string(),
        |(hz, db)| {
            format!(
                "{} / {} dBFS",
                plot::frequency_label(hz),
                if db.is_finite() {
                    format!("{db:.1}")
                } else {
                    "-inf".into()
                }
            )
        },
    );
    let mut content = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .w_full()
        .min_w_0()
        .child(selector)
        .child(plot::render(
            id,
            surface,
            observation,
            channel,
            view.range.floor(),
            status.label(),
        ))
        .child(detail_line("Peak bin", &peak_text))
        .child(ranges);
    if let Some(observation) = observation {
        let window = observation.window;
        content = content.child(detail_line(
            "Resolution / window",
            &format!(
                "{:.1} Hz / {:.1} ms",
                window.sample_rate / window.fft_size as f64,
                1000. * window.fft_size as f64 / window.sample_rate
            ),
        ));
    }
    content = content.child(settings_controls(id, settings, surface, cx));
    if surface == "inspector" {
        content = content.child(editor::render(shell, id));
    }
    if let Some(active) = active.filter(|active| active.settings != settings) {
        content = content.child(
            note(format!(
                "{}: {} / {}",
                t!("Running FFT / hop"),
                active.settings.fft_size,
                active.settings.hop_frames
            ))
            .text_color(theme::WARNING),
        );
    }
    content
        .child(note("Hann window / peak amplitude per bin"))
        .into_any_element()
}

fn settings_controls(
    id: NodeId,
    settings: SpectrumSettings,
    surface: &'static str,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let mut fft = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(3.))
        .child(note("FFT").w(px(28.)).flex_none());
    for size in [512, 1024, 2048, 4096, 8192] {
        fft = fft.child(setting_choice(
            format!("{surface}-spectrum-fft-{id:?}-{size}"),
            size.to_string(),
            size == settings.fft_size,
            GraphEdit::SetSpectrumFft {
                node: id,
                fft_size: size,
            },
            cx,
        ));
    }
    let mut hop = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(3.))
        .child(note("Hop").w(px(28.)).flex_none());
    for divisor in [4, 2, 1] {
        let frames = settings.fft_size / divisor;
        hop = hop.child(setting_choice(
            format!("{surface}-spectrum-hop-{id:?}-{frames}"),
            frames.to_string(),
            frames == settings.hop_frames,
            GraphEdit::SetSpectrumHop {
                node: id,
                hop_frames: frames,
            },
            cx,
        ));
    }
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(fft)
        .child(hop)
        .child(note(format!(
            "{} / {} {}",
            settings.fft_size,
            settings.hop_frames,
            t!("frames")
        )))
        .into_any_element()
}

fn setting_choice(
    id: String,
    label: String,
    selected: bool,
    edit: GraphEdit,
    cx: &Context<StudioShell>,
) -> ActionButton {
    // A highlighted value is only a rendered snapshot. Keep it clickable so
    // a fast click back still wins over an earlier queued edit.
    let this = cx.entity();
    controls::button(id, label)
        .density(Density::Compact)
        .variant(if selected {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        })
        .on_click(move |_, _, cx| {
            this.update(cx, |shell, cx| {
                editor::dispatch(shell, edit.clone(), cx);
                cx.notify();
            });
        })
}
