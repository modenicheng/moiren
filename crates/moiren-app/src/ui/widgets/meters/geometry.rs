//! Local-pixel layouts keep decorations inside independently sized meter canvases.
use super::super::fader::FaderScale;

pub(super) const COMPACT_STEREO_HEIGHT: f32 = 182.;
pub(super) const TICK_LINE_HEIGHT: f32 = 12.;
const TICK_GAP: f32 = 3.;

#[derive(Debug, Clone, Copy)]
pub(super) struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ChannelGeometry {
    pub track: Rect,
    pub clip: Rect,
    pub label: Option<Rect>,
    pub readout: Option<Rect>,
}

pub(super) struct VerticalGeometry {
    pub channels: usize,
    pub track: Rect,
    pub ruler: Option<Rect>,
    pub clip_heading: Option<Rect>,
    clip_y: f32,
    clip_height: f32,
    label_y: Option<f32>,
    readout_y: Option<f32>,
    footer_line_height: f32,
    gap: f32,
    max_bar_width: f32,
}

impl VerticalGeometry {
    pub fn node(width: f32, height: f32, channels: usize) -> Self {
        let width = extent(width);
        let height = extent(height);
        let header = 18f32.min(height / 3.);
        let footer = 28f32.min(height - header);
        let footer_line_height = (footer / 2.).min(TICK_LINE_HEIGHT);
        let ruler_width = width.min(22.);
        let left = (ruler_width + 2.).min(width);
        let right = width;
        let track = Rect {
            x: left,
            y: header,
            width: right - left,
            height: (height - header - footer).max(0.),
        };
        let clip_height = header.min(3.);
        Self {
            channels: channels.max(1),
            track,
            ruler: Some(Rect {
                x: 0.,
                width: ruler_width,
                ..track
            }),
            clip_heading: (header >= TICK_LINE_HEIGHT * 1.5).then_some(Rect {
                x: 0.,
                y: 0.,
                width: ruler_width,
                height: TICK_LINE_HEIGHT,
            }),
            clip_y: (header - clip_height).min((TICK_LINE_HEIGHT - clip_height) / 2.),
            clip_height,
            label_y: Some(height - footer_line_height * 2.),
            readout_y: Some(height - footer_line_height),
            footer_line_height,
            gap: 4.,
            max_bar_width: 14.,
        }
    }

    pub fn compact(width: f32, height: f32, channels: usize) -> Self {
        let height = extent(height);
        let clip_height = height.min(3.);
        let track_top = (clip_height + 3.).min(height);
        Self {
            channels: channels.max(1),
            track: Rect {
                x: 0.,
                y: track_top,
                width: extent(width),
                height: height - track_top,
            },
            ruler: None,
            clip_heading: None,
            clip_y: 0.,
            clip_height,
            label_y: None,
            readout_y: None,
            footer_line_height: 0.,
            gap: 2.,
            max_bar_width: 7.,
        }
    }

    pub fn channel(&self, index: usize) -> Option<ChannelGeometry> {
        if index >= self.channels {
            return None;
        }
        let count = self.channels as f32;
        // Collapse spacing before widths. Even a constrained 256-channel
        // canvas retains nonnegative, disjoint columns without painting outside.
        let gap = self.gap.min(self.track.width / count / 2.);
        let width = ((self.track.width - gap * (count - 1.)) / count)
            .max(0.)
            .min(self.max_bar_width);
        let group_width = width * count + gap * (count - 1.);
        let left = self.track.x + (self.track.width - group_width).max(0.) / 2.;
        let cell_width = self.track.width / count;
        let cell_x = (self.track.x + index as f32 * cell_width).min(self.track.right());
        let x = if self.label_y.is_some() {
            cell_x + (cell_width - width).max(0.) / 2.
        } else {
            left + index as f32 * (width + gap)
        }
        .min(self.track.right());
        let track = Rect {
            x,
            width: width.min((self.track.right() - x).max(0.)),
            ..self.track
        };
        let footer = |y| Rect {
            x: cell_x,
            y,
            width: cell_width.min((self.track.right() - cell_x).max(0.)),
            height: self.footer_line_height,
        };
        Some(ChannelGeometry {
            track,
            clip: Rect {
                y: self.clip_y,
                height: self.clip_height,
                ..track
            },
            label: self.label_y.map(footer),
            readout: self.readout_y.map(footer),
        })
    }
}

/// Natural canvas width, including every channel even when it is off screen.
pub(super) fn node_width(channels: usize) -> f32 {
    // Each 38px cell leaves a visible gap around five 10px monospaced
    // glyphs (e.g. -72.0), rather than joining adjacent channel readings.
    100. + channels.saturating_sub(2) as f32 * 38.
}

/// Keep card layout bounded; extra channels retain their width inside a scroll viewport.
pub(super) fn node_viewport_width(channels: usize) -> f32 {
    node_width(channels.min(8))
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ScaleTick {
    pub db: Option<f64>,
    pub y: f32,
}

pub(super) fn scale_ticks(track: Rect) -> Vec<ScaleTick> {
    if track.height <= 0. {
        return Vec::new();
    }
    let mut ticks: Vec<ScaleTick> = Vec::new();
    // Keep digital zero first and silence second; finite anchors fill only
    // the remaining room. The ruler never crowds its labels on short cards.
    for db in [Some(0.), None].into_iter().chain(
        FaderScale::ANCHORS
            .into_iter()
            .filter(|(db, _)| *db != 0.)
            .map(|(db, _)| Some(db)),
    ) {
        let y = level_y(track, db.map_or(0., |db| 10f64.powf(db / 20.)));
        if ticks
            .iter()
            .all(|tick| (tick.y - y).abs() >= TICK_LINE_HEIGHT + TICK_GAP)
        {
            ticks.push(ScaleTick { db, y });
        }
    }
    ticks.sort_by(|left, right| left.y.total_cmp(&right.y));
    ticks
}

pub(super) fn level_y(track: Rect, gain: f64) -> f32 {
    track.y + track.height * FaderScale::position(gain)
}

pub(super) fn vertical_span(track: Rect, bottom: f32, height: f32) -> Rect {
    let bottom = bottom.clamp(0., 1.);
    let top = (bottom + height).clamp(bottom, 1.);
    Rect {
        y: track.bottom() - track.height * top,
        height: track.height * (top - bottom),
        ..track
    }
}

pub(super) fn vertical_marker(track: Rect, gain: f64) -> Rect {
    let y = level_y(track, gain);
    let top = (y - 1.).max(track.y);
    let bottom = (y + 1.).min(track.bottom());
    Rect {
        y: top,
        height: (bottom - top).max(0.),
        ..track
    }
}

pub(super) struct HorizontalGeometry {
    width: f32,
    height: f32,
    pub channels: usize,
}

impl HorizontalGeometry {
    pub fn new(width: f32, height: f32, channels: usize) -> Self {
        Self {
            width: extent(width),
            height: extent(height),
            channels: channels.max(1),
        }
    }

    pub fn channel(&self, index: usize) -> Option<ChannelGeometry> {
        if index >= self.channels {
            return None;
        }
        let count = self.channels as f32;
        let height = (self.height / count).min(4.);
        let gap = if self.channels > 1 {
            ((self.height - height * count) / (count - 1.)).clamp(0., 2.)
        } else {
            0.
        };
        let y = (index as f32 * (height + gap)).min(self.height);
        let height = height.min((self.height - y).max(0.));
        let clip_width = self.width.min(6.);
        let clip_x = self.width - clip_width;
        let track_width = (clip_x - 3.).max(0.);
        let clip_height = height.min(3.);
        Some(ChannelGeometry {
            track: Rect {
                x: 0.,
                y,
                width: track_width,
                height,
            },
            clip: Rect {
                x: clip_x,
                y: y + (height - clip_height) / 2.,
                width: clip_width,
                height: clip_height,
            },
            label: None,
            readout: None,
        })
    }
}

pub(super) fn horizontal_height(channels: usize) -> f32 {
    let count = channels.max(1) as f32;
    count * 4. + (count - 1.) * 2.
}

pub(super) fn horizontal_marker(track: Rect, gain: f64) -> Rect {
    let x = track.x + track.width * (1. - FaderScale::position(gain));
    let left = (x - 1.).max(track.x);
    let right = (x + 1.).min(track.right());
    Rect {
        x: left,
        width: (right - left).max(0.),
        ..track
    }
}

fn extent(value: f32) -> f32 {
    if value.is_finite() { value.max(0.) } else { 0. }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod width_tests {
    use super::{VerticalGeometry, node_viewport_width, node_width};

    #[::core::prelude::v1::test]
    fn exposed_node_width_caps_at_eight_channels_without_changing_stereo() {
        for channels in 0..=8 {
            assert_eq!(node_viewport_width(channels), node_width(channels));
        }
        for channels in [9, 16, 32, 64, 256] {
            assert_eq!(node_viewport_width(channels), 328.);
            assert_eq!(super::super::node_meter_width(channels), 328.);
        }
        assert_eq!(node_viewport_width(2), 100.);
    }

    #[::core::prelude::v1::test]
    fn natural_canvas_keeps_every_channel_readout_and_clip_reachable() {
        for channels in [2, 8, 9, 32, 256] {
            let natural = node_width(channels);
            let viewport = node_viewport_width(channels);
            assert_eq!(natural, 100. + channels.saturating_sub(2) as f32 * 38.);
            let layout = VerticalGeometry::node(natural, 128., channels);
            for index in 0..channels {
                let channel = layout.channel(index).unwrap();
                assert!(channel.readout.unwrap().width >= 38.);
                assert!(channel.clip.width > 0.);
                assert_eq!(channel.clip.x, channel.track.x);
            }
            let last = layout.channel(channels - 1).unwrap().readout.unwrap();
            let scroll_end = natural - viewport;
            assert!(last.x - scroll_end >= 0.);
            assert!((last.right() - scroll_end - viewport).abs() < 0.001);
        }
    }
}
