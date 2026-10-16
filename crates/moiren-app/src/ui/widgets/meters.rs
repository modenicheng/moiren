//! Real source/sink observations; missing windows retain unknown readouts.
use crate::ui::theme;
use gpui_kit::prelude::*;
use gpui_kit::*;
use moiren_engine::meter::signal::ChannelLevel;
fn label(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(10.))
        .text_color(theme::TEXT_MUTED)
        .child(text.into())
}
fn db(value: f64) -> String {
    if value <= 0. {
        "−∞".into()
    } else {
        format!("{:.1}", 20. * value.log10())
    }
}
pub fn stereo_meter_with_clip(levels: Option<[ChannelLevel; 2]>, clips: [bool; 2]) -> Div {
    let mut pair = div()
        .flex()
        .flex_none()
        .h(px(260.))
        .gap(px(2.))
        .line_height(px(12.));
    let mut ticks = div()
        .relative()
        .mt(px(16.))
        .h(px(190.))
        .flex_none()
        .w(px(20.))
        .text_size(px(8.))
        .text_color(theme::TEXT_FAINT);
    for db in [0, -6, -12, -24, -48, -60] {
        ticks = ticks.child(
            div()
                .absolute()
                .top(px((-db as f32 / 60.) * 184.))
                .child(db.to_string()),
        );
    }
    for (top, name) in [(210., "PK"), (226., "RMS")] {
        ticks = ticks.child(div().absolute().top(px(top)).child(name));
    }
    pair = pair.child(ticks);
    for (index, name) in ["L", "R"].into_iter().enumerate() {
        let channel = levels.map(|l| l[index]);
        let amount = channel.map(|l| meter_height(l.rms)).unwrap_or(0.);
        pair = pair.child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .w(px(24.))
                .items_center()
                .gap(px(4.))
                .child(
                    div()
                        .h(px(12.))
                        .flex_none()
                        .px(px(3.))
                        .rounded(px(2.))
                        .text_size(px(8.))
                        .text_color(
                            if clips[index] || channel.is_some_and(|level| level.peak >= 1.) {
                                theme::DANGER
                            } else {
                                theme::TEXT_FAINT
                            },
                        )
                        .bg(
                            if clips[index] || channel.is_some_and(|level| level.peak >= 1.) {
                                theme::soft(theme::DANGER)
                            } else {
                                theme::SURFACE
                            },
                        )
                        .child(if channel.is_some() || clips[index] {
                            "CLIP"
                        } else {
                            "—"
                        }),
                )
                .child(
                    div()
                        .relative()
                        .w(px(7.))
                        .h(px(190.))
                        .flex_none()
                        .overflow_hidden()
                        .bg(theme::BORDER)
                        .when(channel.is_some(), |v| {
                            let mut view = v;
                            for (bottom, top, color) in [
                                (0., 0.8, theme::SUCCESS),
                                (0.8, 0.95, theme::WARNING),
                                (0.95, 1., theme::DANGER),
                            ] {
                                let height = (amount.min(top) - bottom).max(0.);
                                if height > 0. {
                                    view = view.child(
                                        div()
                                            .absolute()
                                            .bottom(relative(bottom))
                                            .w_full()
                                            .h(relative(height))
                                            .bg(color),
                                    );
                                }
                            }
                            view
                        })
                        .when_some(channel, |v, l| {
                            v.child(
                                div()
                                    .absolute()
                                    .bottom(relative(meter_height(l.peak)))
                                    .w_full()
                                    .h(px(2.))
                                    .bg(theme::TEXT),
                            )
                        }),
                )
                .child(label(name))
                .child(
                    div().font_family(theme::FONT_MONO).text_size(px(8.)).child(
                        channel
                            .map(|l| db(f64::from(l.peak)))
                            .unwrap_or_else(|| "—".into()),
                    ),
                )
                .child(
                    div().font_family(theme::FONT_MONO).text_size(px(8.)).child(
                        channel
                            .map(|level| db(f64::from(level.rms)))
                            .unwrap_or_else(|| "—".into()),
                    ),
                ),
        );
    }
    pair
}

pub(crate) fn meter_height(value: f32) -> f32 {
    if value <= 0. {
        0.
    } else {
        ((20. * value.log10() + 60.) / 60.).clamp(0., 1.)
    }
}

#[cfg(test)]
mod tests {
    use super::meter_height;

    #[test]
    fn meter_regions_follow_dbfs_and_clamp_clipping() {
        assert_eq!(meter_height(0.), 0.);
        assert_eq!(meter_height(1.), 1.);
        assert_eq!(meter_height(2.), 1.);
        assert!((meter_height(10f32.powf(-12. / 20.)) - 0.8).abs() < 0.00001);
        assert!((meter_height(10f32.powf(-3. / 20.)) - 0.95).abs() < 0.00001);
    }
}
