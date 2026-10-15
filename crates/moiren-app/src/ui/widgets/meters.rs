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
        "−∞ dB".into()
    } else {
        format!("{:.1}", 20. * value.log10())
    }
}
pub fn stereo_meter(levels: Option<[ChannelLevel; 2]>) -> Div {
    let mut pair = div().flex().h(px(220.)).gap(px(6.));
    for (index, name) in ["L", "R"].into_iter().enumerate() {
        let channel = levels.map(|l| l[index]);
        let amount = channel.map(|l| meter_height(l.rms)).unwrap_or(0.);
        pair = pair.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .child(
                    div()
                        .relative()
                        .w(px(7.))
                        .h(px(190.))
                        .bg(theme::BORDER)
                        .when(channel.is_some(), |v| {
                            v.child(div().absolute().bottom_0().w_full().h(relative(amount)).bg(
                                if channel.is_some_and(|l| l.peak >= 1.) {
                                    theme::DANGER
                                } else {
                                    theme::SUCCESS
                                },
                            ))
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
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(10.))
                        .child(
                            channel
                                .map(|l| db(f64::from(l.peak)))
                                .unwrap_or_else(|| "—".into()),
                        ),
                ),
        );
    }
    pair
}

fn meter_height(value: f32) -> f32 {
    if value <= 0. {
        0.
    } else {
        ((20. * value.log10() + 60.) / 60.).clamp(0., 1.)
    }
}
