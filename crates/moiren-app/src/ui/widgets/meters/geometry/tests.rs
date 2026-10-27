use super::*;

fn contained(rect: Rect, width: f32, height: f32) {
    let epsilon = 0.001;
    assert!(rect.width.is_finite() && rect.height.is_finite());
    assert!(rect.width >= 0. && rect.height >= 0., "{rect:?}");
    assert!(rect.x >= -epsilon && rect.y >= -epsilon, "{rect:?}");
    assert!(rect.right() <= width + epsilon, "{rect:?} / {width}");
    assert!(rect.bottom() <= height + epsilon, "{rect:?} / {height}");
}

#[::core::prelude::v1::test]
fn stereo_node_has_100_pixel_width_and_grows_only_for_extra_channels() {
    assert_eq!(node_width(0), 100.);
    assert_eq!(node_width(1), 100.);
    assert_eq!(node_width(2), 100.);
    assert_eq!(node_width(4), 176.);
    assert_eq!(node_width(8), 328.);
    for channels in [1, 2, 4, 8, 32, 256] {
        assert_eq!(
            super::super::node_meter_width(channels),
            node_width(channels.min(8))
        );
        let layout = VerticalGeometry::node(node_width(channels), 64., channels);
        for index in 0..channels {
            assert!(layout.channel(index).unwrap().readout.unwrap().width >= 38.);
        }
    }
}

#[::core::prelude::v1::test]
fn node_clip_track_labels_and_readouts_stay_inside_short_and_tall_bounds() {
    for height in [64., 80., 128., 176., 196., 240., 300.] {
        for channels in [1, 2, 4, 8, 32, 256] {
            let width = node_width(channels);
            let layout = VerticalGeometry::node(width, height, channels);
            let heading = layout.clip_heading.unwrap();
            contained(heading, width, height);
            assert!(heading.bottom() <= layout.track.y - TICK_LINE_HEIGHT / 2.);
            let mut right = 0.;
            for index in 0..channels {
                let channel = layout.channel(index).unwrap();
                for rect in [
                    channel.clip,
                    channel.track,
                    channel.label.unwrap(),
                    channel.readout.unwrap(),
                ] {
                    contained(rect, width, height);
                }
                assert!(channel.track.height > 0.);
                assert!(channel.track.width > 0.);
                assert!(channel.clip.bottom() <= channel.track.y);
                assert!(channel.track.bottom() < channel.label.unwrap().y);
                assert!(channel.label.unwrap().bottom() <= channel.readout.unwrap().y);
                let label = channel.label.unwrap();
                assert!(
                    (channel.track.x + channel.track.width / 2. - label.x - label.width / 2.).abs()
                        < 0.001
                );
                assert!(channel.track.x >= right);
                right = channel.track.right();
            }
            assert!(layout.channel(channels).is_none());
        }
    }
}

#[::core::prelude::v1::test]
fn constrained_widths_never_produce_negative_bars_or_overflow() {
    for width in [0., 1., 8., 16., 32., 64., 88., 100.] {
        for height in [0., 1., 8., 24., 64., 300.] {
            for channels in [0, 1, 2, 8, 256] {
                let layout = VerticalGeometry::node(width, height, channels);
                for index in 0..layout.channels {
                    let channel = layout.channel(index).unwrap();
                    for rect in [
                        channel.clip,
                        channel.track,
                        channel.label.unwrap(),
                        channel.readout.unwrap(),
                    ] {
                        contained(rect, width, height);
                    }
                }
            }
        }
    }
}

#[::core::prelude::v1::test]
fn ruler_ticks_keep_clearance_and_use_the_actual_effective_track() {
    for height in [64., 80., 128., 176., 196., 240., 300.] {
        let layout = VerticalGeometry::node(node_width(2), height, 2);
        let ticks = scale_ticks(layout.track);
        assert!(ticks.iter().any(|tick| tick.db == Some(0.)));
        for pair in ticks.windows(2) {
            assert!(pair[1].y - pair[0].y >= TICK_LINE_HEIGHT + TICK_GAP - 0.001);
        }
        for tick in ticks {
            assert!(tick.y - TICK_LINE_HEIGHT / 2. >= 0.);
            assert!(tick.y + TICK_LINE_HEIGHT / 2. <= height);
            let gain = tick.db.map_or(0., |db| 10f64.powf(db / 20.));
            assert!((tick.y - level_y(layout.track, gain)).abs() < 0.0001);
        }
    }
    let short = VerticalGeometry::node(node_width(2), 64., 2);
    let tall = VerticalGeometry::node(node_width(2), 300., 2);
    assert!(scale_ticks(short.track).len() < scale_ticks(tall.track).len());
}

#[::core::prelude::v1::test]
fn fill_top_ticks_and_peak_markers_share_fader_scale_positions() {
    for height in [64., 128., 300.] {
        let layout = VerticalGeometry::node(node_width(2), height, 2);
        let track = layout.channel(0).unwrap().track;
        for (db, position) in FaderScale::ANCHORS {
            let gain = 10f64.powf(db / 20.);
            let y = level_y(track, gain);
            assert!((y - (track.y + track.height * position)).abs() < 0.0001);
            let fill = vertical_span(track, 0., 1. - FaderScale::position(gain));
            assert!((fill.y - y).abs() < 0.0001);
            let marker = vertical_marker(track, gain);
            contained(marker, node_width(2), height);
            assert!(marker.y >= track.y && marker.bottom() <= track.bottom() + 0.001);
            if position > 0.1 && position < 0.9 {
                assert!((marker.y + marker.height / 2. - y).abs() < 0.0001);
            }
        }
        assert_eq!(level_y(track, 0.), track.bottom());
    }
}

#[::core::prelude::v1::test]
fn compact_stereo_preserves_legacy_footprint_but_uses_actual_canvas_height() {
    assert_eq!(COMPACT_STEREO_HEIGHT, 182.);
    let legacy = VerticalGeometry::compact(16., COMPACT_STEREO_HEIGHT, 2);
    for index in 0..2 {
        let channel = legacy.channel(index).unwrap();
        assert_eq!(channel.track.width, 7.);
        assert_eq!(channel.track.height, 176.);
        assert_eq!(channel.clip.y, 0.);
        assert!(channel.clip.bottom() <= channel.track.y);
    }
    for height in [0., 1., 64., 128., 300.] {
        let layout = VerticalGeometry::compact(16., height, 2);
        for index in 0..2 {
            let channel = layout.channel(index).unwrap();
            contained(channel.track, 16., height);
            contained(channel.clip, 16., height);
            assert!(channel.label.is_none() && channel.readout.is_none());
            assert!(layout.clip_heading.is_none());
        }
    }
}

#[::core::prelude::v1::test]
fn horizontal_clips_are_separate_rightmost_cells_for_every_channel() {
    for channels in [1, 2, 4, 8, 256] {
        for width in [0., 1., 5., 16., 88., 300.] {
            let height = horizontal_height(channels);
            let layout = HorizontalGeometry::new(width, height, channels);
            let mut bottom = 0.;
            for index in 0..channels {
                let channel = layout.channel(index).unwrap();
                contained(channel.track, width, height);
                contained(channel.clip, width, height);
                assert!(channel.track.right() <= channel.clip.x);
                assert!((channel.clip.right() - width).abs() < 0.001);
                assert!(channel.track.y >= bottom);
                bottom = channel.track.bottom();
                for gain in [0., 0.1, 1., 4.] {
                    let marker = horizontal_marker(channel.track, gain);
                    contained(marker, width, height);
                    assert!(marker.right() <= channel.track.right() + 0.001);
                }
            }
        }
    }
}

#[::core::prelude::v1::test]
fn horizontal_rows_remain_disjoint_when_the_caller_constrains_height() {
    for height in [0., 1., 3., 10., 64., 300.] {
        for channels in [1, 2, 8, 256] {
            let layout = HorizontalGeometry::new(88., height, channels);
            let mut bottom = 0.;
            for index in 0..channels {
                let channel = layout.channel(index).unwrap();
                contained(channel.track, 88., height);
                contained(channel.clip, 88., height);
                assert!(channel.track.y + 0.001 >= bottom);
                bottom = channel.track.bottom();
            }
        }
    }
}
