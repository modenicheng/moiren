//! Real source/sink observations; missing windows retain unknown readouts.
//! Bars are dual-layer: the bright fill is the moving RMS body, the dark
//! zone above it extends to the peak bar, and the thin marker holds the
//! recent peak maximum. The digital scale uses green below −6 dBFS, yellow
//! from −6 to 0 dBFS, and red above digital zero.
use super::fader::LabelShapes;
use crate::ui::monitoring::DisplayLevel;
use crate::ui::theme;
use gpui_kit::*;

/// Main Mixer bars fill exactly the shared fader track; there is no second
/// ruler or independent dB interpolation here. Channel count is never inferred
/// from a stereo projection of mono/multichannel data.
pub(crate) fn channel_meter_width(channels: usize) -> f32 {
    let count = channels.max(1) as f32;
    let (width, gap) = channel_track_layout(channels);
    width * count + gap * (count - 1.)
}

/// Per-track width and gap for `channels` bars inside the shared 40px channel
/// span. The total-width probe and the painter must agree, so both consume
/// this one layout.
fn channel_track_layout(channels: usize) -> (f32, f32) {
    let count = channels.max(1) as f32;
    let gap = (8. / count).min(3.);
    let width = (28. / count)
        .clamp(2., 10.)
        .min((40. - gap * (count - 1.)) / count);
    (width, gap)
}

/// The clip lamp at the loud end of one meter track: above a vertical
/// track, right of a horizontal one. Lit red while a clip latches, dim
/// otherwise; the surrounding element owns click-to-reset. The single clip
/// indicator for every meter surface — the fill never changes background
/// for clipping.
pub(crate) fn paint_clip_lamp(
    track: Bounds<Pixels>,
    axis: Axis,
    clipped: bool,
    window: &mut Window,
) {
    let lamp = match axis {
        Axis::Vertical => Bounds::new(
            point(track.left(), track.top() - px(10.)),
            size(track.size.width, px(3.)),
        ),
        Axis::Horizontal => Bounds::new(
            point(track.right() + px(3.), track.top()),
            size(px(3.), track.size.height),
        ),
    };
    window.paint_quad(fill(
        lamp,
        if clipped {
            theme::DANGER
        } else {
            theme::BORDER
        },
    ));
}

pub(crate) fn paint_channel_meter(
    bounds: Bounds<Pixels>,
    levels: Option<&[DisplayLevel]>,
    channels: usize,
    clips: &[bool],
    labels: &mut LabelShapes,
    window: &mut Window,
    cx: &mut App,
) {
    let (width, gap) = channel_track_layout(channels);
    for index in 0..channels {
        let left = bounds.left() + px(index as f32 * (width + gap));
        let track = Bounds::new(
            point(left, bounds.top()),
            size(px(width), bounds.size.height),
        );
        let level = levels.and_then(|levels| levels.get(index));
        if let Some(level) = level {
            paint_meter_fill(track, *level, theme::BORDER, window);
        }
        // A held display marker is not a new overload. Trust the
        // raw-window latch so clicking reset clears it immediately.
        let clip = clips.get(index).copied().unwrap_or(false);
        paint_clip_lamp(track, Axis::Vertical, clip, window);
        if width + gap >= 5. {
            // The labels are static short texts from a fixed population, so
            // they are spelled on the stack and painted through the shared
            // (text, size) shaping cache: a held display frame reshapes
            // nothing (audit A-2).
            let mut digits = [0u8; 24];
            let label: &str = if channels == 2 {
                ["L", "R"][index]
            } else {
                let len = decimal_digits(index + 1, &mut digits);
                std::str::from_utf8(&digits[..len]).expect("decimal digits are ascii")
            };
            labels.paint_text(
                label,
                point(left, bounds.bottom() + px(6.)),
                8.,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
    if width + gap < 5. {
        let mut digits = [0u8; 24];
        let len = decimal_digits(channels, &mut digits);
        digits[len..len + " ch".len()].copy_from_slice(b" ch");
        let label =
            std::str::from_utf8(&digits[..len + " ch".len()]).expect("decimal digits are ascii");
        labels.paint_text(
            label,
            point(bounds.left(), bounds.bottom() + px(6.)),
            8.,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

/// Writes `number`'s decimal digits into `buffer` and returns how many it
/// wrote. The hot layer spells the per-channel labels every display frame,
/// so they are formatted on the stack instead of through `to_string` (audit
/// A-2: the hot layer must not allocate per frame). `buffer` must hold at
/// least 20 bytes — the most digits a `usize` can grow.
fn decimal_digits(mut number: usize, buffer: &mut [u8]) -> usize {
    debug_assert!(buffer.len() >= 20, "a usize has at most 20 digits");
    let mut len = 0;
    loop {
        buffer[len] = b'0' + (number % 10) as u8;
        len += 1;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    buffer[..len].reverse();
    len
}

fn meter_height(value: f32) -> f32 {
    if value <= 0. {
        return 0.;
    }
    let (floor_db, floor_position) = scale_floor();
    let floor = 10f64.powf(floor_db / 20.);
    let value = f64::from(value);
    if value >= floor {
        use super::fader::FaderScale;
        1. - FaderScale::position(value)
    } else {
        // `FaderScale::position` holds finite gains at the last tick so the
        // fader itself stays visible; a meter bar must keep falling, so
        // invert the amplitude-linear tail of `FaderScale::gain` that owns
        // the `floor_position`..1 segment reaching exact digital zero.
        (value / floor) as f32 * (1. - floor_position)
    }
}

/// The last scale tick. The fader holds finite gains at it on purpose, so
/// meter readings must never reuse `FaderScale::position` below it.
fn scale_floor() -> (f64, f32) {
    use super::fader::FaderScale;
    let (&floor, _) = FaderScale::ANCHORS.split_last().expect("non-empty");
    floor
}

/// Track position of a held peak marker, downwards from the top in 0..1,
/// sharing [`meter_height`]'s inverted tail: below the last tick the fader
/// holds position, but a falling marker keeps sliding through the −∞ tail
/// and rests at the bottom edge.
fn meter_marker_position(marker: f32) -> f32 {
    1. - meter_height(marker)
}

/// Fill one vertical meter track from the shared `meter_spans` math: the
/// zone colors rise from the quiet bottom, the dark RMS tail covers the
/// transient zone, and the held marker stands at its reading. Every meter
/// surface paints these spans; only the track geometry and the surrounding
/// decorations differ.
pub(crate) fn paint_meter_fill(
    track: Bounds<Pixels>,
    level: DisplayLevel,
    background: Hsla,
    window: &mut Window,
) {
    window.paint_quad(fill(track, background));
    for span in meter_spans(level) {
        window.paint_quad(fill(
            Bounds::new(
                point(
                    track.left(),
                    track.bottom() - track.size.height * (span.bottom + span.height),
                ),
                size(track.size.width, track.size.height * span.height),
            ),
            span.color,
        ));
    }
    let y = track.top() + track.size.height * meter_marker_position(level.marker);
    window.paint_quad(fill(
        Bounds::new(
            point(track.left(), y - px(1.)),
            size(track.size.width, px(2.)),
        ),
        theme::TEXT,
    ));
}

/// [`paint_meter_fill`] rotated for horizontal tracks: `bottom` becomes the
/// quiet left edge and `height` the extent from there.
pub(crate) fn paint_meter_fill_horizontal(
    track: Bounds<Pixels>,
    level: DisplayLevel,
    background: Hsla,
    window: &mut Window,
) {
    window.paint_quad(fill(track, background));
    for span in meter_spans(level) {
        window.paint_quad(fill(
            Bounds::new(
                point(track.left() + track.size.width * span.bottom, track.top()),
                size(track.size.width * span.height, track.size.height),
            ),
            span.color,
        ));
    }
    let x = track.left() + track.size.width * meter_height(level.marker);
    window.paint_quad(fill(
        Bounds::new(point(x, track.top()), size(px(2.), track.size.height)),
        theme::TEXT,
    ));
}

/// One colored layer of a meter bar, spanning `bottom`..`bottom + height` in
/// the 0..1 track space.
#[derive(Clone, Copy)]
struct MeterSpan {
    bottom: f32,
    height: f32,
    color: Hsla,
}

/// The most layers one channel can wear: the three zone fills plus the dark
/// RMS tail.
const METER_SPAN_SLOTS: usize = 4;

/// One channel's layers, built on the stack. Every meter surface recomputes
/// this list every display frame for every channel — the hot layers most of
/// all — so it must never allocate: the fixed slot array replaces the
/// per-call `Vec` this used to return (audit A-2).
struct MeterSpans {
    slots: [Option<MeterSpan>; METER_SPAN_SLOTS],
}

impl MeterSpans {
    fn new() -> Self {
        Self {
            slots: [None; METER_SPAN_SLOTS],
        }
    }

    fn push(&mut self, span: MeterSpan) {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("a channel wears at most METER_SPAN_SLOTS spans");
        *slot = Some(span);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }
}

impl std::ops::Index<usize> for MeterSpans {
    type Output = MeterSpan;

    fn index(&self, index: usize) -> &MeterSpan {
        self.slots[index]
            .as_ref()
            .expect("a span behind every index below len")
    }
}

impl IntoIterator for MeterSpans {
    type Item = MeterSpan;
    type IntoIter = std::iter::Flatten<std::array::IntoIter<Option<MeterSpan>, METER_SPAN_SLOTS>>;

    fn into_iter(self) -> Self::IntoIter {
        self.slots.into_iter().flatten()
    }
}

/// Layers for one channel: the bright segment fill covers the RMS body, and
/// a translucent dark extension above it reaches the peak bar so the
/// sustained reading stays light and transients show as the dark tail.
/// Zone boundaries use the compact digital reference: −6 dBFS
/// (green→yellow) and 0 dBFS (yellow→red). This is the single source for the
/// segmentation strategy: every meter surface paints these spans through the
/// [`paint_meter_fill`] painters in its own orientation.
fn meter_spans(level: DisplayLevel) -> MeterSpans {
    let bar_amount = meter_height(level.bar);
    let mut spans = MeterSpans::new();
    let green_top = meter_height(10f32.powf(-6. / 20.));
    let red_top = meter_height(1.);
    for (bottom, top, color) in [
        (0., green_top, theme::SIGNAL_DEVICE),
        (green_top, red_top, theme::WARNING),
        (red_top, 1., theme::DANGER),
    ] {
        let height = (bar_amount.min(top) - bottom).max(0.);
        if height > 0. {
            spans.push(MeterSpan {
                bottom,
                height,
                color,
            });
        }
    }
    // The display ballistics can leave the smoothed RMS above a falling bar;
    // the dark tail must stay above the lit fill.
    let rms_amount = meter_height(level.rms).min(bar_amount);
    let tail = bar_amount - rms_amount;
    if tail > 0. {
        spans.push(MeterSpan {
            bottom: rms_amount,
            height: tail,
            color: theme::METER_PEAK_ZONE,
        });
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::{
        METER_SPAN_SLOTS, MeterSpan, decimal_digits, meter_height, meter_marker_position,
        meter_spans,
    };
    use crate::ui::monitoring::DisplayLevel;
    use crate::ui::theme;

    fn level(peak_db: f32, rms_db: f32) -> DisplayLevel {
        let peak = 10f32.powf(peak_db / 20.);
        DisplayLevel {
            bar: peak,
            marker: peak,
            rms: 10f32.powf(rms_db / 20.),
        }
    }

    fn same_color(span: &MeterSpan, expected: gpui_kit::Hsla) -> bool {
        span.color.h == expected.h
            && span.color.s == expected.s
            && span.color.l == expected.l
            && span.color.a == expected.a
    }

    #[test]
    fn meter_regions_follow_dbfs_and_clamp_clipping() {
        assert_eq!(meter_height(0.), 0.);
        assert!((meter_height(1.) - 0.76).abs() < 0.00001);
        assert!(meter_height(2.) > meter_height(1.));
        assert_eq!(meter_height(10f32.powf(12. / 20.)), 1.);
        assert_eq!(meter_height(2. * 10f32.powf(12. / 20.)), 1.);
        assert!((meter_height(10f32.powf(-6. / 20.)) - 0.62).abs() < 0.00001);
    }

    #[test]
    fn silence_below_the_scale_floor_keeps_falling_to_zero_height() {
        let floor = 10f32.powf(-72. / 20.);
        // The tail meets the interpolated scale exactly at the last tick.
        assert!((meter_height(floor) - 0.03).abs() < 0.00001);
        // A decaying bar keeps shrinking through the tail instead of
        // saturating at the fader's −72 dB hold.
        let fading = meter_height(floor * 0.01);
        assert!(fading > 0. && fading < 0.0004, "fading = {fading}");
        assert!(meter_height(floor * 0.5) < meter_height(floor));
        // And exact digital zero stays fully invisible.
        assert_eq!(meter_height(0.), 0.);
    }

    #[test]
    fn peak_markers_slide_through_the_silence_tail_to_the_bottom_edge() {
        let floor = 10f32.powf(-72. / 20.);
        // Mid-scale markers keep their interpolated position.
        assert!((meter_marker_position(1.) - 0.24).abs() < 0.001);
        // Below the fader's −72 dB hold the marker keeps sliding instead of
        // pinning at the tick, converging on the bottom edge as it fades.
        let at_floor = meter_marker_position(floor);
        assert!((at_floor - 0.97).abs() < 0.00001);
        assert!(meter_marker_position(floor * 0.5) > at_floor);
        let faded = meter_marker_position(floor * 0.01);
        assert!(faded > 0.999 && faded < 1., "faded = {faded}");
    }

    #[test]
    fn meter_spans_light_the_rms_body_and_darken_the_peak_tail() {
        let spans = meter_spans(level(3., -30.));
        // The bar at +3 dB crosses all three zones: green through −6 dB,
        // yellow through 0 dB, red above digital zero.
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].bottom, 0.);
        let green_top = meter_height(10f32.powf(-6. / 20.));
        let red_top = meter_height(1.);
        let peak = meter_height(10f32.powf(3. / 20.));
        let rms = meter_height(10f32.powf(-30. / 20.));
        assert!((spans[0].height - green_top).abs() < 0.00001);
        assert!((spans[1].bottom - green_top).abs() < 0.00001);
        assert!((spans[1].height - (red_top - green_top)).abs() < 0.00001);
        assert!((spans[2].bottom - red_top).abs() < 0.00001);
        assert!((spans[2].height - (peak - red_top)).abs() < 0.00001);
        // The dark tail covers the transient zone from the −30 dB RMS body
        // up to the peak reading.
        assert!((spans[3].bottom - rms).abs() < 0.00001);
        assert!((spans[3].height - (peak - rms)).abs() < 0.00001);
        assert!(same_color(&spans[3], theme::METER_PEAK_ZONE));
    }

    #[test]
    fn meter_spans_skip_the_dark_tail_when_rms_reaches_the_bar() {
        // The display ballistics can leave the smoothed RMS above a falling
        // bar; the whole fill then reads as the bright RMS body.
        let spans = meter_spans(DisplayLevel {
            bar: 0.1,
            marker: 0.1,
            rms: 0.5,
        });
        assert_eq!(spans.len(), 1);
        assert!((spans[0].height - meter_height(0.1)).abs() < 0.00001);
        assert!(!same_color(&spans[0], theme::METER_PEAK_ZONE));
    }

    #[test]
    fn meter_spans_stay_within_the_fixed_stack_slots() {
        // The hot layers rebuild this list every display frame for every
        // channel, so it is built on the stack with a fixed slot count (the
        // three zone fills plus the dark tail). The bound must hold for
        // every reading — clipping bars with a falling RMS tail included —
        // or `MeterSpans::push` would panic.
        for step in 0..=60i32 {
            let db = -72. + step as f32 * 2.;
            let bar = 10f32.powf(db / 20.);
            let rms = bar * 0.7;
            let spans = meter_spans(DisplayLevel {
                bar,
                marker: bar,
                rms,
            });
            assert!(
                spans.len() <= METER_SPAN_SLOTS,
                "a {db} dB bar wore {} spans",
                spans.len()
            );
        }
    }

    #[test]
    fn decimal_labels_are_spelled_on_the_stack_without_allocation() {
        // The per-frame channel labels are formatted into a stack buffer
        // instead of a `String` (audit A-2); the digits must come out in
        // reading order.
        let mut buffer = [0u8; 24];
        for (number, expected) in [
            (1usize, "1"),
            (9, "9"),
            (10, "10"),
            (16, "16"),
            (128, "128"),
        ] {
            let len = decimal_digits(number, &mut buffer);
            assert_eq!(std::str::from_utf8(&buffer[..len]).unwrap(), expected);
        }
    }
}
