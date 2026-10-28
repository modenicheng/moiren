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
        .relative()
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
                                    let mut layout = VerticalGeometry::node(
                                        f32::from(bounds.size.width),
                                        f32::from(bounds.size.height),
                                        channels,
                                    );
                                    // Only channels scroll; the fixed sibling
                                    // paints the ruler and heading once.
                                    layout.ruler = None;
                                    layout.clip_heading = None;
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
        .child(
            // A bare canvas has no hitbox: scrolling and the parent's clip
            // reset still reach the existing viewport beneath this overlay.
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let layout = VerticalGeometry::node(
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                        channels,
                    );
                    let gutter = node_gutter_bounds(bounds, &layout);
                    window.with_content_mask(Some(ContentMask { bounds: gutter }), |window| {
                        // Match graph_canvas_view::nodes::paint_node. Cover the
                        // footer too, so a partial previous readout cannot leak.
                        window.paint_quad(fill(gutter, theme::SURFACE_3));
                        paint_vertical_scale(bounds, &layout, window, cx);
                    });
                },
            )
            .absolute()
            .left_0()
            .top_0()
            .size_full(),
        )
}

fn node_gutter_bounds(bounds: Bounds<Pixels>, layout: &VerticalGeometry) -> Bounds<Pixels> {
    Bounds::new(
        bounds.origin,
        size(
            px(layout.track.x).min(bounds.size.width),
            bounds.size.height,
        ),
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
        paint_vertical_scale(bounds, layout, window, cx);
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

fn paint_vertical_scale(
    bounds: Bounds<Pixels>,
    layout: &VerticalGeometry,
    window: &mut Window,
    cx: &mut App,
) {
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
    fn node_gutter_stays_inside_viewport_and_aligns_with_natural_track() {
        for height in [0., 1., 24., 64., 128., 300.] {
            for width in [0., 12., 24., 100., 176., 328.] {
                let bounds = Bounds::new(point(px(41.), px(17.)), size(px(width), px(height)));
                for channels in [1, 2, 32, 256] {
                    let fixed = VerticalGeometry::node(width, height, channels);
                    let natural =
                        VerticalGeometry::node(geometry::node_width(channels), height, channels);
                    let gutter = node_gutter_bounds(bounds, &fixed);
                    assert_eq!(gutter.origin, bounds.origin);
                    assert_eq!(gutter.size.width, px(width.min(24.)));
                    assert_eq!(gutter.size.height, bounds.size.height);
                    assert!(gutter.right() <= bounds.right());
                    assert_eq!(fixed.track.y, natural.track.y);
                    assert_eq!(fixed.track.height, natural.track.height);
                    assert_eq!(
                        fixed.clip_heading.map(|rect| rect.y),
                        natural.clip_heading.map(|rect| rect.y)
                    );
                    let fixed_ticks: Vec<_> = geometry::scale_ticks(fixed.track)
                        .into_iter()
                        .map(|tick| (tick.db, tick.y))
                        .collect();
                    let natural_ticks: Vec<_> = geometry::scale_ticks(natural.track)
                        .into_iter()
                        .map(|tick| (tick.db, tick.y))
                        .collect();
                    assert_eq!(fixed_ticks, natural_ticks);
                }
            }
        }
    }

    #[::core::prelude::v1::test]
    fn node_gutter_masks_partial_previous_channel_without_changing_scroll_end() {
        for channels in [8, 32, 256] {
            for visible in [2, 4, 8].into_iter().filter(|visible| *visible <= channels) {
                let natural_width = geometry::node_width(channels);
                let viewport_width = geometry::node_width(visible);
                let scroll_end = natural_width - viewport_width;
                assert_eq!(scroll_end, (channels - visible) as f32 * 38.);
                for height in [64., 128., 300.] {
                    let bounds =
                        Bounds::new(point(px(0.), px(0.)), size(px(viewport_width), px(height)));
                    let layout = VerticalGeometry::node(natural_width, height, channels);
                    let fixed = VerticalGeometry::node(viewport_width, height, channels);
                    let gutter = node_gutter_bounds(bounds, &fixed);
                    let first = channels - visible;
                    if first > 0 {
                        let previous = layout.channel(first - 1).unwrap();
                        for rect in [
                            previous.track,
                            previous.clip,
                            previous.label.unwrap(),
                            previous.readout.unwrap(),
                        ] {
                            assert!(rect.right() - scroll_end <= f32::from(gutter.right()));
                        }
                    }
                    for index in first..channels {
                        let channel = layout.channel(index).unwrap();
                        for rect in [
                            channel.track,
                            channel.clip,
                            channel.label.unwrap(),
                            channel.readout.unwrap(),
                        ] {
                            assert!(rect.x - scroll_end >= f32::from(gutter.right()));
                            assert!(rect.right() - scroll_end <= viewport_width);
                        }
                    }
                    let last = layout.channel(channels - 1).unwrap().readout.unwrap();
                    assert_eq!(last.right() - scroll_end, viewport_width);
                }
            }
        }
    }

    #[::core::prelude::v1::test]
    fn node_gutter_does_not_cover_stereo_or_first_page_channel_cells() {
        for channels in [1, 2, 4, 8, 32] {
            let viewport_width = super::super::node_meter_width(channels.min(4));
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(viewport_width), px(176.)));
            let natural = VerticalGeometry::node(geometry::node_width(channels), 176., channels);
            let fixed = VerticalGeometry::node(viewport_width, 176., channels);
            let gutter = node_gutter_bounds(bounds, &fixed);
            for index in 0..channels.min(4) {
                let channel = natural.channel(index).unwrap();
                for rect in [
                    channel.track,
                    channel.clip,
                    channel.label.unwrap(),
                    channel.readout.unwrap(),
                ] {
                    assert!(rect.x >= f32::from(gutter.right()));
                    assert!(rect.right() <= viewport_width);
                }
            }
        }
    }

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
