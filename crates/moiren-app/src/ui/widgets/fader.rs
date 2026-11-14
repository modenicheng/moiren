//! One numerical scale for the main Mixer's fader, ruler and meters.
use crate::ui::observe::TestObserve;
use crate::ui::theme;
use crate::ui::widgets::scroll_step::{ScrollGesture, scroll_gesture};
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct FaderScale;

impl FaderScale {
    pub const ANCHORS: [(f64, f32); 10] = [
        (12., 0.),
        (6., 0.10),
        (0., 0.24),
        (-6., 0.38),
        (-12., 0.51),
        (-24., 0.67),
        (-36., 0.78),
        (-48., 0.86),
        (-60., 0.92),
        (-72., 0.97),
    ];

    /// Downwards from the track top. Silence has its own endpoint; finite
    /// gains below the last tick keep their value when snapshots synchronize.
    pub fn position(gain: f64) -> f32 {
        if gain <= 0. {
            return 1.;
        }
        let db = 20. * gain.log10();
        for pair in Self::ANCHORS.windows(2) {
            let [(high, top), (low, bottom)] = pair else {
                unreachable!()
            };
            if db >= *low {
                return top + ((high - db) / (high - low)).clamp(0., 1.) as f32 * (bottom - top);
            }
        }
        // Below the last tick the scale is the amplitude-linear tail of
        // [`Self::gain`] (gain = floor * (1 - position) / 0.03), and this
        // must be its exact inverse: a flat clamp here would collapse every
        // sub-floor gain onto the -72 dB tick, and each confirmed publish
        // would then yank a thumb the user had dragged below it back up —
        // fighting the drag until the pointer overshoots the rail and the
        // rail clamp lands the fader on digital silence.
        let floor = 10f64.powf(-72. / 20.);
        1. - (gain / floor * 0.03).min(1.) as f32
    }

    pub fn gain(position: f32) -> f64 {
        let position = position.clamp(0., 1.);
        if position >= 1. {
            return 0.;
        }
        for pair in Self::ANCHORS.windows(2) {
            let [(high, top), (low, bottom)] = pair else {
                unreachable!()
            };
            if position <= *bottom {
                let db = high + f64::from((position - top) / (bottom - top)) * (low - high);
                return 10f64.powf(db / 20.);
            }
        }
        // A short amplitude-linear segment reaches exact digital zero.
        10f64.powf(-72. / 20.) * f64::from((1. - position) / 0.03)
    }
}

/// These children all fill the same inset track. Relative positions therefore
/// share geometry even when the window or display scale changes.
pub fn ruler() -> Div {
    // A ruler is one painted surface, rather than eleven independently laid
    // out labels on every animation frame. It shares the track's exact bounds.
    div().w(px(26.)).h_full().flex_none().child(
        canvas(
            |_, _, _| {},
            |bounds, _, window, cx| {
                for (db, y) in FaderScale::ANCHORS {
                    // Keep the finite anchor, but avoid colliding with infinity
                    // at the compact track's bottom.
                    if db == -72. {
                        continue;
                    }
                    paint_text(
                        &format!("{db:+.0}"),
                        point(
                            bounds.left(),
                            bounds.top() + bounds.size.height * y - px(5.),
                        ),
                        9.,
                        TextAlign::Right,
                        Some(bounds.size.width - px(4.)),
                        window,
                        cx,
                    );
                }
                paint_text(
                    "−∞",
                    point(bounds.left(), bounds.bottom() - px(5.)),
                    9.,
                    TextAlign::Right,
                    Some(bounds.size.width - px(4.)),
                    window,
                    cx,
                );
            },
        )
        .size_full(),
    )
}

pub(super) fn paint_text(
    text: &str,
    origin: Point<Pixels>,
    font_size: f32,
    align: TextAlign,
    width: Option<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let line = window.text_system().shape_line(
        text.to_owned().into(),
        px(font_size),
        &[TextRun {
            len: text.len(),
            font: font(theme::FONT_MONO),
            color: theme::TEXT_FAINT,
            ..Default::default()
        }],
        None,
    );
    let _ = line.paint(origin, px(10.), align, width, window, cx);
}

/// The hot layer's static label shaping cache, shared across strips and
/// keyed by (text, font size): the strip meters' "L"/"R"/channel-number
/// labels repaint every display frame, but their texts come from a tiny
/// fixed population, so keying turns per-frame shaping into lookups forever
/// — the same text-keyed move the strip readouts made with the mixer page's
/// `ReadoutShapes` cache (audit A-2, the HC-8 residual). The key carries the
/// font size because label call sites shape at their own sizes; the font
/// family, weight and color are compile-time constants of the shared
/// painter above, and any future parameterization must join the key.
#[derive(Default)]
pub(crate) struct LabelShapes {
    /// Shaped lines grouped by font size (raw `f32` bits): the inner maps
    /// look up by `&str`, so a hit never allocates.
    lines: HashMap<u32, HashMap<String, gpui_kit::ShapedLine>>,
    /// Test census: real shaper runs and the texts painted through the
    /// cache.
    #[cfg(feature = "gpui-test")]
    census: LabelCensus,
}

#[cfg(feature = "gpui-test")]
#[derive(Default)]
struct LabelCensus {
    shapes: usize,
    painted: std::collections::BTreeSet<String>,
}

/// Far above the honest label population (the fixed L/R pair, one number per
/// channel, one "{channels} ch" per channel count); a pathological feed that
/// exceeds it clears the cache and pays reshapes again instead of growing
/// without bound.
const LABEL_SHAPE_CACHE_CAP: usize = 512;

impl LabelShapes {
    /// Paint `text` through the cache: shape only on a (text, size) miss,
    /// then paint exactly like the free [`paint_text`]. The signature mirrors
    /// that free painter one receiver wider.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_text(
        &mut self,
        text: &str,
        origin: Point<Pixels>,
        font_size: f32,
        align: TextAlign,
        width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(feature = "gpui-test")]
        self.census.painted.insert(text.to_owned());
        let Some(line) = self.line(text, font_size, window) else {
            return;
        };
        let _ = line.paint(origin, px(10.), align, width, window, cx);
    }

    /// The shaped line for `(text, font_size)`, running the shaper only on a
    /// cache miss.
    fn line(
        &mut self,
        text: &str,
        font_size: f32,
        window: &mut Window,
    ) -> Option<&gpui_kit::ShapedLine> {
        let size = font_size.to_bits();
        if !self
            .lines
            .get(&size)
            .is_some_and(|lines| lines.contains_key(text))
        {
            let entries: usize = self.lines.values().map(HashMap::len).sum();
            if entries >= LABEL_SHAPE_CACHE_CAP {
                self.lines.clear();
            }
            let line = window.text_system().shape_line(
                text.to_owned().into(),
                px(font_size),
                &[TextRun {
                    len: text.len(),
                    font: font(theme::FONT_MONO),
                    color: theme::TEXT_FAINT,
                    ..Default::default()
                }],
                None,
            );
            self.lines
                .entry(size)
                .or_default()
                .insert(text.to_owned(), line);
            #[cfg(feature = "gpui-test")]
            {
                self.census.shapes += 1;
            }
        }
        self.lines.get(&size).and_then(|lines| lines.get(text))
    }
}

#[cfg(all(test, feature = "gpui-test"))]
impl LabelShapes {
    /// How many shaper runs the cache has needed, and how many distinct
    /// (text, size) entries it holds. Every run inserts exactly one entry,
    /// so the pair pins "one shape per distinct label, everything else is a
    /// lookup".
    pub(crate) fn stats(&self) -> (usize, usize) {
        let entries: usize = self.lines.values().map(HashMap::len).sum();
        (self.census.shapes, entries)
    }

    /// The distinct texts painted through the cache so far.
    pub(crate) fn painted(&self) -> std::collections::BTreeSet<String> {
        self.census.painted.clone()
    }
}

/// Shared wheel/double-click commit for one computed value: writes the
/// slider value and dispatches the graph edit.
pub type Commit = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// Interaction beyond dragging, supplied by the value-space owner: wheel
/// stepping and the double-click reset. `commit` writes the slider value and
/// dispatches the graph edit as one step; `set_value` emits no
/// [`base::slider::SliderEvent::Change`], so committing never feeds back into
/// the drag path.
#[derive(Clone)]
pub struct SliderBehavior {
    /// Wheel travel: (current value, signed notches) -> new value. +1 is one
    /// notch up/right (increase), -1 down/left; fine-grained wheels pass a
    /// fraction of a notch.
    pub step: Rc<dyn Fn(f32, f32) -> f32>,
    /// Value a double click restores (unity / default / center).
    pub reset: f32,
    /// Commit an externally computed value: set slider value + dispatch the
    /// graph edit.
    pub commit: Commit,
}

/// Wraps a slider in the shared gesture surface: a wheel notch steps by
/// `behavior.step`, a touchpad pan tracks the fingers proportionally, and a
/// double click restores `behavior.reset` — all through `commit`. Scroll
/// events stop propagation here so adjusting a slider never scrolls the
/// surrounding strip or panel container.
fn gestures(
    state: &Entity<base::slider::SliderState>,
    behavior: &SliderBehavior,
    axis: Axis,
    slider: impl IntoElement,
) -> Stateful<Div> {
    let hint = t!("Scroll to adjust · double-click to reset").to_string();
    // The gesture surface's own bounds: a pan maps its travel onto this
    // extent so the thumb tracks the fingers one-to-one.
    let surface = Rc::new(Cell::new(Bounds::default()));
    div()
        .id(("slider-gestures", state.entity_id()))
        .relative()
        .tooltip(move |window, cx| component::tooltip::Tooltip::new(hint.clone()).build(window, cx))
        .on_scroll_wheel({
            let state = state.clone();
            let behavior = behavior.clone();
            let surface = surface.clone();
            move |event: &ScrollWheelEvent, window, cx| {
                cx.stop_propagation();
                let length = axis_length(axis, surface.get());
                let Some(gesture) = scroll_gesture(&event.delta, axis, length) else {
                    return;
                };
                let slider = state.read(cx);
                let current = slider.value().end();
                let (min, max) = (slider.min_value(), slider.max_value());
                let value = match gesture {
                    ScrollGesture::Notch(notches) => (behavior.step)(current, notches),
                    ScrollGesture::Pan(travel) => (current + travel * (max - min)).clamp(min, max),
                };
                // An immediate, user-paced gesture: bypass drag coalescing
                // and leave the dragging flag alone.
                if value != current {
                    (behavior.commit)(value, window, cx);
                }
            }
        })
        // The double click resets on its second press, in the capture phase,
        // ahead of everything under the pointer. The first press jumps the
        // thumb to the pointer, so the second lands on the thumb — which
        // claims its presses for itself, so a bubble-phase click handler
        // here never counted to two. Stopping the press also keeps the track
        // from re-jumping and gpui from starting a drag on it.
        .capture_any_mouse_down({
            let behavior = behavior.clone();
            move |event: &MouseDownEvent, window, cx| {
                if event.button == MouseButton::Left && event.click_count >= 2 {
                    cx.stop_propagation();
                    (behavior.commit)(behavior.reset, window, cx);
                }
            }
        })
        .child(
            canvas(move |bounds, _, _| surface.set(bounds), |_, _, _, _| {})
                .absolute()
                .size_full(),
        )
        .child(slider)
}

/// Extent of `bounds` along `axis`, the length pans travel across.
fn axis_length(axis: Axis, bounds: Bounds<Pixels>) -> f32 {
    match axis {
        Axis::Vertical => f32::from(bounds.size.height),
        Axis::Horizontal => f32::from(bounds.size.width),
    }
}

/// Linear position of `value` inside the slider's own value range, 0..1.
/// Pure so the geometry tests run without a rendered slider.
fn normalized_value(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span <= 0. {
        return 0.;
    }
    ((value - min) / span).clamp(0., 1.)
}

/// Vertical thumb position from the track top. The fader's slider value space
/// is inverted (1 = top), so the thumb rides at `1 - normalized`.
fn thumb_top(value: f32, min: f32, max: f32) -> f32 {
    1. - normalized_value(value, min, max)
}

/// Vertical channel fader for the main mixer and the tray flyout: a flat
/// capsule track, a fill from the thumb down to the quiet end, a 0 dB detent
/// tick (the flyout has no ruler or unity line of its own), and a classic
/// fader cap — taller than wide, light-bodied with a dark horizontal slot —
/// centered on the current value. The widget reads its value from the slider
/// state, so callers never duplicate the gain mapping.
pub fn fader(
    state: &Entity<base::slider::SliderState>,
    behavior: &SliderBehavior,
    cx: &App,
) -> Stateful<Div> {
    let slider = state.read(cx);
    let thumb = thumb_top(slider.value().end(), slider.min_value(), slider.max_value());
    gestures(
        state,
        behavior,
        Axis::Vertical,
        base::slider::Slider::new(state)
            .vertical()
            .relative()
            .size_full()
            // Keep the indicator: gpui uses it to record pointer mapping bounds.
            // Without it the state keeps zero bounds and drags map to NaN.
            .child(
                base::slider::SliderIndicator::new(state)
                    .absolute()
                    .left(px(13.))
                    .top_0()
                    .w(px(6.))
                    .h_full(),
            )
            .child(
                base::slider::SliderTrack::new(state)
                    .axis(Axis::Vertical)
                    .absolute()
                    .left(px(13.))
                    .top_0()
                    .w(px(6.))
                    .h_full()
                    .rounded(px(theme::RADIUS_SM))
                    .bg(theme::WINDOW_BG)
                    .border_1()
                    .border_color(theme::BORDER),
            )
            .child(
                // The value space is inverted: everything louder than the thumb
                // lights up between it and the bottom (quiet) end.
                div()
                    .absolute()
                    .left(px(13.))
                    .top(relative(thumb))
                    .bottom_0()
                    .w(px(6.))
                    .rounded(px(theme::RADIUS_SM))
                    .bg(theme::TRACK_FILL),
            )
            .child(
                div()
                    .absolute()
                    .left(px(11.))
                    .top(relative(FaderScale::position(1.)))
                    .w(px(10.))
                    .h(px(1.))
                    .bg(theme::TEXT_FAINT),
            )
            .child(
                base::slider::SliderThumb::new(state)
                    .axis(Axis::Vertical)
                    .absolute()
                    // Track spans x 13..19, so a 14px cap centers at left 9
                    // and overhangs the 6px track by 4px per side.
                    .left(px(9.))
                    .top(relative(thumb))
                    // Half the 22px cap height keeps it centered on the value.
                    .mt(px(-11.))
                    .w(px(14.))
                    .h(px(22.))
                    .rounded(px(3.))
                    // A light body so the cap reads against the dark page; the
                    // grip is a dark slot punched through it.
                    .bg(theme::TEXT)
                    .border_1()
                    .border_color(theme::BORDER_STRONG)
                    .child(
                        div()
                            .absolute()
                            .left(px(2.))
                            .right(px(2.))
                            .top(px(10.))
                            .h(px(2.))
                            .bg(theme::controls::RAISED),
                    ),
            ),
    )
    .flex_none()
    .w(px(32.))
    .h_full()
}

/// Minimal horizontal slider for inspector parameters, the channel strip and
/// sends: a thin capsule track, a fill from the quiet (left) end and a dot
/// thumb. No detents, ticks or labels; the surrounding row keeps the value.
pub fn hslider(
    state: &Entity<base::slider::SliderState>,
    behavior: &SliderBehavior,
    cx: &App,
) -> Stateful<Div> {
    let slider = state.read(cx);
    let fill = normalized_value(slider.value().end(), slider.min_value(), slider.max_value());
    gestures(
        state,
        behavior,
        Axis::Horizontal,
        base::slider::Slider::new(state)
            .horizontal()
            .relative()
            .size_full()
            .child(
                base::slider::SliderIndicator::new(state)
                    .absolute()
                    .left_0()
                    .top(px(5.))
                    .w_full()
                    .h(px(6.)),
            )
            .child(
                base::slider::SliderTrack::new(state)
                    .axis(Axis::Horizontal)
                    .absolute()
                    .left_0()
                    .top(px(5.))
                    .w_full()
                    .h(px(6.))
                    .rounded_full()
                    .bg(theme::WINDOW_BG)
                    .border_1()
                    .border_color(theme::BORDER),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(5.))
                    .w(relative(fill))
                    .h(px(6.))
                    .rounded_full()
                    .bg(theme::TRACK_FILL),
            )
            .child(
                base::slider::SliderThumb::new(state)
                    .axis(Axis::Horizontal)
                    .absolute()
                    .left(relative(fill))
                    .ml(px(-6.))
                    .top(px(2.))
                    .w(px(12.))
                    .h(px(12.))
                    .rounded_full()
                    .bg(theme::TEXT_MUTED)
                    .hover(|style| style.bg(theme::TEXT))
                    .border_1()
                    .border_color(theme::BORDER_STRONG),
            ),
    )
    .flex_none()
    .w_full()
    .h(px(16.))
}

/// The send bars' value space: the channel fader's [`FaderScale`] laid along
/// the horizontal axis. A slider value IS its fill fraction — the pointer maps
/// onto this space directly, so the painted fill tracks the mouse and the
/// touchpad one-to-one with no second mapping to drift out of sync. 0 is
/// exact silence at the quiet (left) rail, unity sits at the scale's 0 dB
/// anchor three quarters along, and 1 is the +12 dB scale top filling the bar
/// flush against the loud (right) rail (boosts past it pin at full).
pub fn send_value(gain: f64) -> f32 {
    1. - FaderScale::position(gain)
}

/// [`send_value`]'s inverse: the gain a send slider value stands for. The
/// write-back (`sync_sliders`) and the drag dispatch both go through this one
/// pair, so a value painted at a fraction also commits the gain whose fill is
/// that fraction.
pub fn send_gain(value: f32) -> f64 {
    FaderScale::gain(1. - value)
}

/// Visible height of the send level bar. Off the spacing grid on purpose:
/// twice the accent bar width, the same thickness the approved prototype
/// shipped (`ACCENT_BAR_W × 2`).
pub const SEND_BAR_H: f32 = 4.;

/// The send level bar: a plain rectangular controller — no thumb, no rounded
/// corners, flush at both ends. The whole bar is the control: the fill's
/// extent is the level, growing from the quiet (left) end toward the loud
/// (right) end — silence empties it, unity reaches three quarters, and the
/// +12 dB scale top fills it flush against the right edge. The track is
/// [`theme::controls::RAISED`] for every
/// send regardless of selection or mute; muting only grays the fill. The 4 px
/// under the track are breathing room and part of the drag hit area, so the
/// element is twice [`SEND_BAR_H`] tall while the visible strip stays 4 px.
pub fn send_bar(
    state: &Entity<base::slider::SliderState>,
    behavior: &SliderBehavior,
    muted: bool,
    cx: &App,
) -> Stateful<Div> {
    let slider = state.read(cx);
    // The value space IS the fill fraction (see [`send_value`]): paint the
    // position the slider reports, never a second dB detour.
    let fill = normalized_value(slider.value().end(), slider.min_value(), slider.max_value());
    gestures(
        state,
        behavior,
        Axis::Horizontal,
        base::slider::Slider::new(state)
            .horizontal()
            .relative()
            .size_full()
            // The pointer mapping and the drag hitbox span the whole element,
            // the 4 px below the visible strip included.
            .child(
                base::slider::SliderIndicator::new(state)
                    .absolute()
                    .left_0()
                    .top_0()
                    .w_full()
                    .h_full(),
            )
            .child(
                base::slider::SliderTrack::new(state)
                    .axis(Axis::Horizontal)
                    .absolute()
                    .left_0()
                    .top_0()
                    .w_full()
                    .h_full(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .w_full()
                    .h(px(SEND_BAR_H))
                    .bg(theme::controls::RAISED),
            )
            .child(
                div()
                    .id(SharedString::from(format!(
                        "send-fill-{}",
                        state.entity_id()
                    )))
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(relative(fill))
                    .h(px(SEND_BAR_H))
                    .bg(if muted {
                        theme::TRACK_FILL
                    } else {
                        theme::SIGNAL_DEVICE
                    })
                    .gpui_test_observe(),
            ),
    )
    .flex_none()
    .w_full()
    // Track + the hit-area strip below it.
    .h(px(SEND_BAR_H * 2.))
}

#[cfg(test)]
mod tests {
    use super::FaderScale;

    #[test]
    fn scale_aligns_working_levels_and_round_trips() {
        for (db, y) in [
            (12., 0.),
            (6., 0.10),
            (0., 0.24),
            (-6., 0.38),
            (-12., 0.51),
            (-24., 0.67),
            (-36., 0.78),
            (-48., 0.86),
            (-60., 0.92),
            (-72., 0.97),
        ] {
            let gain = 10f64.powf(db / 20.);
            assert!((FaderScale::position(gain) - y).abs() < 0.00001);
            assert!((20. * FaderScale::gain(y).log10() - db).abs() < 0.0001);
        }
        for db in -71..13 {
            let gain = 10f64.powf(f64::from(db) / 20.);
            assert!(
                (20. * FaderScale::gain(FaderScale::position(gain)).log10() - f64::from(db)).abs()
                    < 0.001
            );
        }
    }

    #[test]
    fn silence_is_distinct_from_finite_values_below_the_last_tick() {
        assert_eq!(FaderScale::position(0.), 1.);
        assert_eq!(FaderScale::gain(1.), 0.);
        // The tail inverts gain()'s amplitude-linear segment exactly, so a
        // sub-floor gain maps onto its own position below the -72 dB tick.
        let floor = 10f64.powf(-72. / 20.);
        assert!(
            (f64::from(FaderScale::position(1e-5)) - (1. - (1e-5 / floor * 0.03))).abs() < 1e-6
        );
        assert!(FaderScale::gain(0.99) > 0.);
    }

    #[test]
    fn tail_positions_round_trip_through_the_amplitude_linear_segment() {
        // Every sub-floor gain must survive the write-back round trip: the
        // thumb a user dragged below -72 dB is where the next snapshot puts
        // it again, instead of collapsing onto the tick and fighting the
        // drag until the rail clamp lands on digital silence.
        for step in 1..24 {
            let position = 0.97 + 0.03 * (step as f32) / 24.;
            let gain = FaderScale::gain(position);
            assert!(gain > 0., "tail position {position} must stay finite");
            let round = FaderScale::position(gain);
            assert!(
                (round - position).abs() < 1e-6,
                "tail round trip moved {position} to {round}"
            );
        }
    }

    #[test]
    fn normalized_maps_values_inside_the_slider_range() {
        use super::{normalized_value, thumb_top};
        assert_eq!(normalized_value(0., 0., 1.), 0.);
        assert_eq!(normalized_value(0.5, 0., 1.), 0.5);
        assert_eq!(normalized_value(12., -60., 24.), 72. / 84.);
        assert_eq!(normalized_value(-70., -60., 24.), 0.);
        assert_eq!(normalized_value(30., -60., 24.), 1.);
        // A degenerate span keeps the thumb at the quiet end instead of NaN.
        assert_eq!(normalized_value(5., 5., 5.), 0.);
        assert_eq!(thumb_top(0., 0., 1.), 1.);
    }

    #[test]
    fn send_values_lay_the_fader_scale_on_the_bar_axis() {
        use super::send_value;
        // The bar's value space is the fader's scale mirrored onto the width:
        // silence empties the bar at the left rail, unity reaches its 0 dB
        // anchor three quarters along, and the +12 dB scale top fills it.
        assert_eq!(send_value(0.), 0.);
        assert!((send_value(1.) - 0.76).abs() < 0.0001);
        assert_eq!(send_value(10f64.powf(12. / 20.)), 1.);
        // Boosts past the scale top pin at full, exactly like the fader's top
        // anchor.
        assert_eq!(send_value(10f64.powf(24. / 20.)), 1.);
    }

    #[test]
    fn the_send_value_space_round_trips_through_the_scale() {
        use super::{send_gain, send_value};
        // The dispatch (value -> gain) and the write-back (gain -> value)
        // share one inverse pair, so a value painted at a fraction commits
        // the gain whose fill is that fraction — the bar can never disagree
        // with the committed level.
        for step in 0..=100 {
            let value = step as f32 / 100.;
            assert!((send_value(send_gain(value)) - value).abs() < 0.0001);
        }
        for db in -72..=12 {
            let gain = 10f64.powf(f64::from(db) / 20.);
            assert!((20. * send_gain(send_value(gain)).log10() - f64::from(db)).abs() < 0.001);
        }
    }

    #[test]
    fn vertical_thumb_positions_mirror_the_fader_scale() {
        use super::thumb_top;
        // The slider's inverted value space round-trips through the scale: a
        // value derived from a gain puts the thumb at the gain's position.
        for (db, position) in FaderScale::ANCHORS {
            let gain = 10f64.powf(db / 20.);
            let value = 1. - FaderScale::position(gain);
            assert!((thumb_top(value, 0., 1.) - position).abs() < 0.00001);
        }
    }
}
