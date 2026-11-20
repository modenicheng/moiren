use gpui_kit::{Axis, Window};

use crate::ui::pages::graph::graph_canvas::{CanvasFrame, MeterLayout, NodeLayout};
use crate::ui::theme;

use super::super::NodeMeterReading;
use super::shared::rounded;

/// One node card's meter tracks and live bars, in window space. The zone
/// segmentation, dark RMS tail, held marker, and clip latch are the same
/// `meter_spans` math every other meter surface paints (design 17 D1); only
/// the surface differs. Shared by the cached canvas paint and the hot
/// layer's overlay paint so both draw identical geometry; callers own the
/// card content mask. Curve nodes run their meters vertically along the
/// body's right edge, everything else as a horizontal strip under the
/// body (design 18 B2).
pub(in crate::ui) fn paint_meter_bars(
    frame: &CanvasFrame,
    layout: &NodeLayout,
    reading: Option<&NodeMeterReading>,
    window: &mut Window,
) {
    let stride = (theme::card::METER_H + theme::card::METER_CHANNEL_GAP) * layout.scale;
    for channel in 0..theme::card::METER_CHANNELS {
        // The clip lamp reserves the last 3 world units of the horizontal
        // row's loud end — the same loud-end indicator every meter surface
        // uses; the fill never changes background for clipping. Vertical
        // tracks run the body's full height instead, so their bottom edge
        // aligns with the plot below it.
        let (track, axis) = match layout.meter {
            MeterLayout::Horizontal { bounds } => {
                let mut meter = bounds;
                meter.origin.y += channel as f32 * stride;
                meter.size.width -= 3. * layout.scale;
                (meter, Axis::Horizontal)
            }
            MeterLayout::Vertical { bounds } => {
                let mut meter = bounds;
                meter.origin.x -= channel as f32 * stride;
                (meter, Axis::Vertical)
            }
        };
        let track = frame.window_rect(track);
        let clip = reading.is_some_and(|reading| reading.clips[channel]);
        rounded(track, theme::WINDOW_BG, layout.scale, window);
        if let Some(level) = reading.and_then(|reading| reading.levels.get(channel)) {
            match axis {
                Axis::Horizontal => {
                    crate::ui::widgets::meters::paint_meter_fill_horizontal(
                        track,
                        *level,
                        theme::WINDOW_BG,
                        window,
                    );
                }
                _ => {
                    crate::ui::widgets::meters::paint_meter_fill(
                        track,
                        *level,
                        theme::WINDOW_BG,
                        window,
                    );
                }
            }
        }
        crate::ui::widgets::meters::paint_clip_lamp(track, axis, clip, window);
    }
}
