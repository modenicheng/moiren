//! Self-contained meter surfaces; the Mixer retains its separate shared track.
use super::geometry::{
    self, ChannelGeometry, HorizontalGeometry, Rect, TICK_LINE_HEIGHT, VerticalGeometry,
};
use super::*;
use crate::ui::window::TestObserve;

/// A node card chooses the height. Clip latches, scale and per-channel readouts
/// consume space inside that allocation, never above or below its bounds.
pub(super) fn node(levels: Option<&[DisplayLevel]>, channels: usize, clips: &[bool]) -> Div {
    let levels = levels.map(<[DisplayLevel]>::to_vec);
    let clips = clips.to_vec();
    div()
        .w(px(super::node_meter_width(channels)))
        .h_full()
        .min_h_0()
        .flex_none()
        // Let the inner viewport scroll first, then contain wheel events even
        // at its edges so they cannot pan or zoom the graph underneath.
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            // The owning card's element id scopes this stable scroll state.
            div()
                .id("node-meter-viewport")
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .overflow_x_scroll()
                .child(
                    div()
                        .id("node-meter-content")
                        .w(px(geometry::node_width(channels)))
                        .h_full()
                        .flex_none()
                        .child(
                            canvas(
                                |_, _, _| {},
                                move |bounds, _, window, cx| {
                                    let layout = VerticalGeometry::node(
                                        f32::from(bounds.size.width),
                                        f32::from(bounds.size.height),
                                        channels,
                                    );
                                    paint_vertical(
                                        bounds,
                                        &layout,
                                        levels.as_deref(),
                                        &clips,
                                        window,
                                        cx,
                                    );
                                },
                            )
                            .size_full(),
                        )
                        .gpui_test_observe(),
                )
                .gpui_test_observe(),
        )
}

pub(super) fn compact_stereo(levels: Option<[DisplayLevel; 2]>, clips: [bool; 2]) -> Div {
    // The flyout historically allocates 176px tracks plus a 3px gap and 3px
    // clip row. Keep that 182px intrinsic box; explicit caller heights still
    // override it and every painted inset follows the resulting actual bounds.
    div()
        .w(px(16.))
        .h(px(geometry::COMPACT_STEREO_HEIGHT))
        .min_h_0()
        .flex_none()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let layout = VerticalGeometry::compact(
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                        2,
                    );
                    paint_vertical(
                        bounds,
                        &layout,
                        levels.as_ref().map(|levels| levels.as_slice()),
                        &clips,
                        window,
                        cx,
                    );
                },
            )
            .size_full(),
        )
}

pub(super) fn compact_horizontal(
    levels: Option<&[DisplayLevel]>,
    channels: usize,
    clips: &[bool],
) -> Div {
    let levels = levels.map(<[DisplayLevel]>::to_vec);
    let clips = clips.to_vec();
    div()
        .w_full()
        .h(px(geometry::horizontal_height(channels)))
        .flex_none()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let layout = HorizontalGeometry::new(
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                        channels,
                    );
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for index in 0..layout.channels {
                            let channel = layout.channel(index).expect("channel within layout");
                            quad(bounds, channel.track, theme::BORDER, window);
                            if let Some(level) = reading(levels.as_deref(), index) {
                                for span in meter_spans(level) {
                                    let (left, width) = horizontal_span(&span);
                                    quad(
                                        bounds,
                                        Rect {
                                            x: channel.track.x + channel.track.width * left,
                                            width: channel.track.width * width,
                                            ..channel.track
                                        },
                                        span.color,
                                        window,
                                    );
                                }
                                quad(
                                    bounds,
                                    geometry::horizontal_marker(
                                        channel.track,
                                        f64::from(level.marker),
                                    ),
                                    theme::TEXT,
                                    window,
                                );
                            }
                            paint_clip(
                                bounds,
                                channel,
                                clips.get(index).copied().unwrap_or(false),
                                window,
                            );
                        }
                    });
                },
            )
            .size_full(),
        )
}

fn paint_vertical(
    bounds: Bounds<Pixels>,
    layout: &VerticalGeometry,
    levels: Option<&[DisplayLevel]>,
    clips: &[bool],
    window: &mut Window,
    cx: &mut App,
) {
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        if let Some(rect) = layout.clip_heading {
            text(bounds, rect, "CLIP", TextStyle::Header, window, cx);
        }
        if let Some(ruler) = layout.ruler.filter(|ruler| ruler.width >= 20.) {
            for tick in geometry::scale_ticks(layout.track) {
                let label = tick
                    .db
                    .map_or_else(|| "\u{2212}\u{221e}".to_owned(), |db| format!("{db:+.0}"));
                text(
                    bounds,
                    Rect {
                        y: tick.y - TICK_LINE_HEIGHT / 2.,
                        height: TICK_LINE_HEIGHT,
                        width: (ruler.width - 4.).max(0.),
                        ..ruler
                    },
                    &label,
                    TextStyle::Scale,
                    window,
                    cx,
                );
                quad(
                    bounds,
                    Rect {
                        x: ruler.right() - 3.,
                        y: tick.y - 0.5,
                        width: 3.,
                        height: 1.,
                    },
                    theme::BORDER_STRONG,
                    window,
                );
            }
        }
        for index in 0..layout.channels {
            let channel = layout.channel(index).expect("channel within layout");
            quad(bounds, channel.track, theme::BORDER, window);
            let level = reading(levels, index);
            if let Some(level) = level {
                for span in meter_spans(level) {
                    quad(
                        bounds,
                        geometry::vertical_span(channel.track, span.bottom, span.height),
                        span.color,
                        window,
                    );
                }
                quad(
                    bounds,
                    geometry::vertical_marker(channel.track, f64::from(level.marker)),
                    theme::TEXT,
                    window,
                );
            }
            // Clip reset remains an outer-shell action. Painting never infers
            // new overloads from a held marker or combines different channels.
            paint_clip(
                bounds,
                channel,
                clips.get(index).copied().unwrap_or(false),
                window,
            );
            if let Some(rect) = channel.label {
                let label = if layout.channels == 2 {
                    ["L", "R"][index].to_owned()
                } else {
                    (index + 1).to_string()
                };
                text(bounds, rect, &label, TextStyle::Reading, window, cx);
            }
            if let Some(rect) = channel.readout {
                text(
                    bounds,
                    rect,
                    &held_peak(level.map(|level| level.marker)),
                    TextStyle::Reading,
                    window,
                    cx,
                );
            }
        }
    });
}

fn reading(levels: Option<&[DisplayLevel]>, index: usize) -> Option<DisplayLevel> {
    levels
        .and_then(|levels| levels.get(index))
        .copied()
        .filter(|level| {
            [level.bar, level.marker, level.rms]
                .into_iter()
                .all(|value| value.is_finite() && value >= 0.)
        })
}

fn held_peak(value: Option<f32>) -> String {
    match value.filter(|value| value.is_finite() && *value >= 0.) {
        None => "\u{2014}".to_owned(),
        Some(0.) => "\u{2212}\u{221e}".to_owned(),
        Some(value) => format!("{:.1}", 20. * f64::from(value).log10()),
    }
}

fn paint_clip(
    bounds: Bounds<Pixels>,
    channel: ChannelGeometry,
    clipped: bool,
    window: &mut Window,
) {
    quad(
        bounds,
        channel.clip,
        if clipped {
            theme::DANGER
        } else {
            theme::BORDER
        },
        window,
    );
}

fn pixels(bounds: Bounds<Pixels>, rect: Rect) -> Bounds<Pixels> {
    Bounds::new(
        bounds.origin + point(px(rect.x), px(rect.y)),
        size(px(rect.width), px(rect.height)),
    )
}

fn quad(bounds: Bounds<Pixels>, rect: Rect, color: Hsla, window: &mut Window) {
    if rect.width > 0. && rect.height > 0. {
        window.paint_quad(fill(pixels(bounds, rect).intersect(&bounds), color));
    }
}

enum TextStyle {
    Header,
    Scale,
    Reading,
}

fn text(
    bounds: Bounds<Pixels>,
    rect: Rect,
    value: &str,
    style: TextStyle,
    window: &mut Window,
    cx: &mut App,
) {
    if rect.width < 8. || rect.height < TICK_LINE_HEIGHT {
        return;
    }
    let cell = pixels(bounds, rect).intersect(&bounds);
    window.with_content_mask(Some(ContentMask { bounds: cell }), |window| {
        let (font_size, align, color) = match style {
            TextStyle::Header => (9., TextAlign::Left, theme::TEXT_MUTED),
            TextStyle::Scale => (9., TextAlign::Right, theme::TEXT_FAINT),
            TextStyle::Reading => (10., TextAlign::Center, theme::TEXT_MUTED),
        };
        let line = window.text_system().shape_line(
            value.to_owned().into(),
            px(font_size),
            &[TextRun {
                len: value.len(),
                font: font(theme::FONT_MONO),
                color,
                ..Default::default()
            }],
            None,
        );
        let _ = line.paint(
            cell.origin,
            px(TICK_LINE_HEIGHT),
            align,
            Some(cell.size.width),
            window,
            cx,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn held_peak_is_short_numeric_and_distinguishes_silence_from_unknown() {
        assert_eq!(held_peak(None), "\u{2014}");
        assert_eq!(held_peak(Some(f32::NAN)), "\u{2014}");
        assert_eq!(held_peak(Some(0.)), "\u{2212}\u{221e}");
        assert_eq!(held_peak(Some(1.)), "0.0");
        assert_eq!(held_peak(Some(10f32.powf(-18. / 20.))), "-18.0");
        assert_eq!(held_peak(Some(10f32.powf(6. / 20.))), "6.0");
    }

    #[::core::prelude::v1::test]
    fn missing_or_invalid_channel_readings_do_not_borrow_another_channel() {
        let levels = [DisplayLevel {
            bar: 1.,
            marker: 1.,
            rms: 0.5,
        }];
        assert!(reading(Some(&levels), 0).is_some());
        assert!(reading(Some(&levels), 1).is_none());
        assert!(reading(None, 0).is_none());
        assert!(
            reading(
                Some(&[DisplayLevel {
                    marker: f32::INFINITY,
                    ..levels[0]
                }]),
                0
            )
            .is_none()
        );
    }
}
