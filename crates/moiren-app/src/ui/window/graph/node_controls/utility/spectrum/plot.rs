//! Peak-preserving display reduction; FFT magnitudes come only from the engine.
use super::*;
use crate::ui::spectrum::SpectrumObservation;

const CEILING_DB: f64 = 6.;
const PLOT_HEIGHT: f32 = 156.;

pub(super) fn render(
    id: NodeId,
    surface: &str,
    observation: Option<&SpectrumObservation>,
    channel: usize,
    floor: f64,
    status: &str,
) -> AnyElement {
    let nyquist = observation.map(|value| value.window.sample_rate / 2.);
    let mut plot = div()
        .id(format!("spectrum-plot-{surface}-{id:?}"))
        .relative()
        .w_full()
        .h(px(PLOT_HEIGHT + 22.))
        .flex_none()
        .bg(theme::WINDOW_BG)
        .border_1()
        .border_color(theme::BORDER)
        .rounded(px(theme::RADIUS_SM));
    for db in [0., -30., -60., -90., -120.]
        .into_iter()
        .filter(|db| *db >= floor)
    {
        plot = plot.child(
            div()
                .absolute()
                .left(px(3.))
                .top(px(6. + db_position(db, floor) * PLOT_HEIGHT - 6.))
                .text_size(px(9.))
                .text_color(theme::TEXT_FAINT)
                .child(format!("{db:.0}")),
        );
    }
    let data = observation.cloned();
    let status = t!(status).to_string();
    let inner = div()
        .absolute()
        .left(px(30.))
        .right(px(8.))
        .top(px(6.))
        .h(px(PLOT_HEIGHT))
        .overflow_hidden()
        .child(
            canvas(
                move |bounds, _, _| {
                    let width = f32::from(bounds.size.width).ceil().clamp(1., 4096.) as usize;
                    data.as_ref()
                        .map(|value| trace(value, channel, floor, width))
                },
                move |bounds, trace, window, _| {
                    for db in [6., 0., -30., -60., -90., -120.]
                        .into_iter()
                        .filter(|db| *db >= floor)
                    {
                        let mut line = bounds;
                        line.origin.y += bounds.size.height * db_position(db, floor);
                        line.size.height = px(1.);
                        window.paint_quad(fill(line, theme::BORDER));
                    }
                    if let Some(nyquist) = nyquist {
                        for hz in frequency_ticks(nyquist) {
                            let mut line = bounds;
                            line.origin.x += bounds.size.width * frequency_position(hz, nyquist);
                            line.size.width = px(1.);
                            window.paint_quad(fill(line, theme::BORDER.opacity(0.6)));
                        }
                    }
                    if let Some(trace) = trace {
                        let mut path = PathBuilder::stroke(px(1.5));
                        for (index, (x, y)) in trace.into_iter().enumerate() {
                            let point = bounds.origin
                                + point(bounds.size.width * x, bounds.size.height * y);
                            if index == 0 {
                                path.move_to(point);
                            } else {
                                path.line_to(point);
                            }
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, theme::SIGNAL_DEVICE);
                        }
                    }
                },
            )
            .size_full(),
        );
    plot = plot.child(inner);
    if let Some(nyquist) = nyquist {
        let mut ticks = div()
            .absolute()
            .left(px(30.))
            .right(px(8.))
            .bottom(px(1.))
            .h(px(13.));
        for hz in frequency_ticks(nyquist) {
            let x = frequency_position(hz, nyquist);
            let label = div()
                .absolute()
                .text_size(px(9.))
                .text_color(theme::TEXT_FAINT)
                .child(if x <= 0.01 {
                    format!("{} Hz", frequency_tick_label(hz))
                } else {
                    frequency_tick_label(hz)
                });
            ticks = ticks.child(if x >= 0.99 {
                label.right_0()
            } else if x <= 0.01 {
                label.left_0()
            } else {
                label.left(relative(x)).ml(px(-10.))
            });
        }
        plot = plot.child(ticks);
    } else {
        plot = plot.child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .px(px(36.))
                .child(note(status).text_center()),
        );
    }
    plot.tooltip(|window, cx| {
        component::tooltip::Tooltip::new(t!("Log frequency / dBFS per bin; DC excluded from curve. Peak-preserving display, no averaging.").to_string())
            .build(window, cx)
    }).gpui_test_observe().into_any_element()
}

pub(super) fn frequency_label(hz: f64) -> String {
    if hz >= 1000. {
        format!("{:.2} kHz", hz / 1000.)
    } else {
        format!("{hz:.1} Hz")
    }
}

fn frequency_tick_label(hz: f64) -> String {
    if hz >= 1000. {
        if hz % 1000. < 1. {
            format!("{:.0}k", hz / 1000.)
        } else {
            format!("{:.1}k", hz / 1000.)
        }
    } else {
        format!("{hz:.0}")
    }
}

fn low_frequency(nyquist: f64) -> f64 {
    20f64.min(nyquist / 2.)
}

fn frequency_position(hz: f64, nyquist: f64) -> f32 {
    let low = low_frequency(nyquist);
    ((hz.max(low) / low).ln() / (nyquist / low).ln()).clamp(0., 1.) as f32
}

fn frequency_ticks(nyquist: f64) -> Vec<f64> {
    let mut ticks = vec![low_frequency(nyquist)];
    for hz in [100., 1000., 10000.] {
        let x = frequency_position(hz, nyquist);
        if hz < nyquist && x > 0.15 && x < 0.85 {
            ticks.push(hz);
        }
    }
    ticks.push(nyquist);
    ticks
}

fn db_position(db: f64, floor: f64) -> f32 {
    ((CEILING_DB - db.max(floor)) / (CEILING_DB - floor)).clamp(0., 1.) as f32
}

fn amplitude_db(amplitude: f32) -> f64 {
    if amplitude > 0. {
        20. * f64::from(amplitude).log10()
    } else {
        f64::NEG_INFINITY
    }
}

fn channel_bins(observation: &SpectrumObservation, channel: usize) -> Option<&[f32]> {
    let bins = observation.window.fft_size / 2 + 1;
    if channel >= observation.window.channels {
        return None;
    }
    observation
        .magnitudes
        .get(channel * bins..(channel + 1) * bins)
}

pub(super) fn peak_bin(observation: &SpectrumObservation, channel: usize) -> Option<(f64, f64)> {
    let (index, &amplitude) = channel_bins(observation, channel)?
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))?;
    Some((
        if amplitude == 0. {
            0.
        } else {
            index as f64 * observation.window.sample_rate / observation.window.fft_size as f64
        },
        amplitude_db(amplitude),
    ))
}

fn trace(
    observation: &SpectrumObservation,
    channel: usize,
    floor: f64,
    width: usize,
) -> Vec<(f32, f32)> {
    let Some(bins) = channel_bins(observation, channel) else {
        return Vec::new();
    };
    let nyquist = observation.window.sample_rate / 2.;
    let bin_hz = observation.window.sample_rate / observation.window.fft_size as f64;
    let mut columns: Vec<Option<(f32, f32)>> = vec![None; width.clamp(1, 4096)];
    for (index, &amplitude) in bins.iter().enumerate().skip(1) {
        let hz = index as f64 * bin_hz;
        if hz < low_frequency(nyquist) {
            continue;
        }
        let x = frequency_position(hz, nyquist);
        let column = (x * (columns.len() - 1) as f32).round() as usize;
        // Log bins become denser than pixels at high frequencies. Keep the
        // strongest actual bin rather than decimating away a narrow peak.
        if columns[column].is_none_or(|(_, old)| amplitude > old) {
            columns[column] = Some((x, amplitude));
        }
    }
    columns
        .into_iter()
        .flatten()
        .map(|(x, amplitude)| (x, db_position(amplitude_db(amplitude), floor)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_engine::spectrum::SpectrumWindow;

    fn observation() -> SpectrumObservation {
        let mut magnitudes = vec![0.; 1026];
        magnitudes[20] = 0.5;
        magnitudes[513 + 40] = 0.25;
        SpectrumObservation {
            window: SpectrumWindow {
                timeline_epoch: 1,
                frame_start: 0,
                frame_end: 1024,
                sample_rate: 48000.,
                fft_size: 1024,
                channels: 2,
            },
            magnitudes: magnitudes.into(),
        }
    }

    #[::core::prelude::v1::test]
    fn spectrum_plot_reports_channel_major_peak_amplitude_not_power_or_rms() {
        let observation = observation();
        let (hz, db) = peak_bin(&observation, 0).unwrap();
        assert_eq!(hz, 937.5);
        assert!((db + 6.0206).abs() < 0.0001);
        assert_eq!(peak_bin(&observation, 1).unwrap().0, 1875.);
        assert!(peak_bin(&observation, 2).is_none());
    }

    #[::core::prelude::v1::test]
    fn spectrum_plot_reduction_preserves_peaks_and_is_bounded() {
        let observation = observation();
        for width in [1, 10, 284, 10000] {
            let trace = trace(&observation, 0, -90., width);
            assert!(trace.len() <= width.min(4096));
            assert!(
                trace
                    .iter()
                    .all(|(x, y)| (0. ..=1.).contains(x) && (0. ..=1.).contains(y))
            );
            let minimum = trace.iter().map(|(_, y)| *y).fold(1f32, f32::min);
            assert!((minimum - db_position(-6.0206, -90.)).abs() < 0.0001);
        }
    }

    #[::core::prelude::v1::test]
    fn spectrum_plot_ticks_follow_actual_nyquist_without_crowding() {
        for nyquist in [4000., 22050., 24000., 96000.] {
            let ticks = frequency_ticks(nyquist);
            assert_eq!(ticks.last(), Some(&nyquist));
            assert!(ticks.windows(2).all(|pair| pair[1] > pair[0]));
            assert!(ticks.iter().all(|hz| *hz <= nyquist));
        }
        assert_eq!(db_position(12., -90.), 0.);
        assert_eq!(db_position(f64::NEG_INFINITY, -90.), 1.);
    }
}
