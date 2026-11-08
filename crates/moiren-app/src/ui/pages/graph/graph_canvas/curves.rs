//! Static curve math for audio-node card thumbnails (design 18): the
//! peaking-EQ magnitude response, the dynamics static transfer, and the
//! samplers that turn either into plottable points. Pure functions over
//! engine settings — no UI types, no gpui — so painted thumbnails,
//! hit-testing, and tests share one source of truth.
//!
//! The engine kernels keep their coefficient helpers crate-private, so the
//! formulas below are faithful reimplementations of `eq.rs` and
//! `compressor.rs`; they must move if those kernels change.

use crate::service::CompressorState;
use moiren_engine::processor::{EqSettings, GateSettings, LimiterSettings};

/// Magnitude response of the peaking band, in dB, at `freq_hz`.
///
/// The biquad is the RBJ peaking-EQ cookbook form of `Eq::coefficients`
/// (private to the engine crate), reimplemented here coefficient for
/// coefficient: the design frequency is clamped to 0.49 × the sample rate,
/// Q is floored at 0.1, and a non-finite or non-positive rate falls back to
/// 48 kHz, exactly as `eq.rs` does. The probe frequency is clamped the same
/// way. |H(e^{jω})| is evaluated from the normalized coefficients of
/// H(z) = (b0 + b1·z⁻¹ + b2·z⁻²) / (1 + a1·z⁻¹ + a2·z⁻²).
pub fn eq_magnitude_db(settings: &EqSettings, sample_rate: f64, freq_hz: f64) -> f64 {
    let [b0, b1, b2, a1, a2] = peaking_coefficients(
        settings.frequency_hz,
        settings.gain_db,
        settings.q,
        sample_rate,
    );
    let sample_rate = sanitize_sample_rate(sample_rate);
    let freq_hz = freq_hz.clamp(0.0, (sample_rate * 0.49).max(1.0));
    let omega = 2.0 * std::f64::consts::PI * freq_hz / sample_rate;
    let (sin1, cos1) = omega.sin_cos();
    let (sin2, cos2) = (2.0 * omega).sin_cos();
    // Squared magnitudes of the numerator and denominator polynomials on
    // the unit circle; the dB value follows from their ratio.
    let numerator_re = b0 + b1 * cos1 + b2 * cos2;
    let numerator_im = -(b1 * sin1 + b2 * sin2);
    let denominator_re = 1.0 + a1 * cos1 + a2 * cos2;
    let denominator_im = -(a1 * sin1 + a2 * sin2);
    let numerator = numerator_re * numerator_re + numerator_im * numerator_im;
    let denominator = denominator_re * denominator_re + denominator_im * denominator_im;
    10.0 * (numerator / denominator).log10()
}

/// `[b0, b1, b2, a1, a2]` of the peaking band, normalized by a0. Mirrors
/// `Eq::coefficients` (private to the engine crate); keep the two in sync.
fn peaking_coefficients(frequency_hz: f64, gain_db: f64, q: f64, sample_rate: f64) -> [f64; 5] {
    let sample_rate = sanitize_sample_rate(sample_rate);
    let frequency_hz = frequency_hz.clamp(0.0, (sample_rate * 0.49).max(1.0));
    let q = q.max(0.1);
    let omega = 2.0 * std::f64::consts::PI * frequency_hz / sample_rate;
    let sine = omega.sin();
    let alpha = sine / (2.0 * q);
    let cosine = omega.cos();
    let amplitude = 10.0_f64.powf(gain_db / 40.0);
    let a0 = 1.0 + alpha / amplitude;
    [
        (1.0 + alpha * amplitude) / a0,
        (-2.0 * cosine) / a0,
        (1.0 - alpha * amplitude) / a0,
        (-2.0 * cosine) / a0,
        (1.0 - alpha / amplitude) / a0,
    ]
}

/// The same guard `dynamics.rs` applies before filter math: non-finite or
/// non-positive rates fall back to 48 kHz so the response stays defined.
fn sanitize_sample_rate(sample_rate: f64) -> f64 {
    if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate
    } else {
        48_000.0
    }
}

/// Static input-to-output level transfer, in dB, for the compressor family.
///
/// This is the wet path only (`mix` = 1): attack, release, and hold shape
/// time, not the steady state. The detector level is the input plus the
/// input gain; [`reduction_db`] is subtracted from it; makeup and output
/// gains are applied last.
#[derive(Debug, Clone, Copy)]
pub struct DynamicsCurve {
    /// Detector threshold in dB.
    pub threshold_db: f64,
    /// Compression ratio; `f64::INFINITY` renders the limiter's ceiling.
    pub ratio: f64,
    /// Total knee width in dB, centered on the threshold.
    pub knee_db: f64,
    /// Pre-detector input gain in dB.
    pub input_gain_db: f64,
    /// Post-reduction makeup gain in dB.
    pub makeup_db: f64,
    /// Final output gain in dB.
    pub output_gain_db: f64,
    /// Set only by [`DynamicsCurve::gate`]: the closed-gate output floor in
    /// dB. `None` selects the compressor slope shape.
    gate_floor_db: Option<f64>,
}

impl DynamicsCurve {
    /// Static transfer of the app's compressor node. Mix is a crossfade, not
    /// a curve property, so the wet shape is what a thumbnail paints.
    pub fn compressor(state: &CompressorState) -> Self {
        Self {
            threshold_db: state.threshold_db,
            ratio: state.ratio,
            knee_db: state.knee_db,
            input_gain_db: state.input_gain_db,
            makeup_db: state.makeup_gain_db,
            output_gain_db: state.output_gain_db,
            gate_floor_db: None,
        }
    }

    /// Zero-attack peak ceiling: an infinite ratio holds every level above
    /// the threshold at the threshold, per `limiter.rs`. The input gain
    /// feeds the detector and the unity region below it, exactly as the
    /// kernel applies the gain before limiting.
    pub fn limiter(settings: &LimiterSettings) -> Self {
        Self {
            threshold_db: settings.threshold_db,
            ratio: f64::INFINITY,
            knee_db: 0.0,
            input_gain_db: settings.input_gain_db,
            makeup_db: 0.0,
            output_gain_db: 0.0,
            gate_floor_db: None,
        }
    }

    /// Gate as a transfer curve: unity at and above the threshold — `gate.rs`
    /// opens at envelope >= threshold, so the unity branch runs exactly
    /// through (threshold, threshold) — and the range floor below it.
    ///
    /// Semantic choice: `gate.rs` applies the range as a multiplicative gain
    /// when closed (output = input + range dB), which on a thumbnail would
    /// fall off the painted level range entirely; the conventional absolute
    /// floor at `range_db` reads as the same gate and stays on-canvas. The
    /// closed-to-open transition is a hard jump — a gate is discontinuous by
    /// design — and `gate.rs` has no knee, so none is drawn.
    pub fn gate(settings: &GateSettings) -> Self {
        Self {
            threshold_db: settings.threshold_db,
            ratio: 1.0,
            knee_db: 0.0,
            input_gain_db: 0.0,
            makeup_db: 0.0,
            output_gain_db: 0.0,
            gate_floor_db: Some(settings.range_db),
        }
    }

    /// Output level in dB for an input level in dB.
    pub fn transfer_db(&self, input_db: f64) -> f64 {
        if let Some(floor_db) = self.gate_floor_db {
            // The gate detector of gate.rs has no input gain; the curve
            // stores none. Makeup and output gains still apply last.
            let open = input_db >= self.threshold_db;
            let level = if open { input_db } else { floor_db };
            return level + self.makeup_db + self.output_gain_db;
        }
        let level_db = input_db + self.input_gain_db;
        let reduction = reduction_db(level_db, self.threshold_db, self.ratio, self.knee_db);
        input_db + self.input_gain_db - reduction + self.makeup_db + self.output_gain_db
    }
}

/// Detector-side gain reduction of the compressor family: Giannoulis /
/// Massberg / Reiss, JAES 2012, Eq. (4), mirrored from
/// `Compressor::target_reduction` (private to the engine crate); keep the
/// two in sync. Below the knee the reduction is zero; inside the knee a
/// quadratic in `level − threshold` makes value and slope continuous at
/// both edges; above it the reduction line has slope `1 − 1/ratio`.
fn reduction_db(level_db: f64, threshold_db: f64, ratio: f64, knee_db: f64) -> f64 {
    let above = level_db - threshold_db;
    let slope = 1.0 - 1.0 / ratio;
    if above <= -knee_db * 0.5 {
        0.0
    } else if knee_db > 0.0 && above < knee_db * 0.5 {
        slope * (above + knee_db * 0.5).powi(2) / (2.0 * knee_db)
    } else {
        slope * above
    }
}

/// Samples `f` at `samples` geometrically spaced points from `x_min` to
/// `x_max` (both ends included, exactly, so callers can anchor axes).
/// Requires `0 < x_min <= x_max`; the EQ frequency response is the intended
/// use. Fewer than two samples yields zero points or the single start point.
/// What a card's curve thumbnail draws (design 18 B3): the EQ's magnitude
/// response over log frequency, or a dynamics transfer curve over a
/// ±60 dB input window.
#[derive(Debug, Clone, Copy)]
pub enum CurveSpec {
    Eq(moiren_engine::processor::EqSettings),
    Dynamics(DynamicsCurve),
}

pub fn sample_curve_log_x(
    f: impl Fn(f64) -> f64,
    x_min: f64,
    x_max: f64,
    samples: usize,
) -> Vec<(f64, f64)> {
    if samples == 0 {
        return Vec::new();
    }
    if samples == 1 {
        return vec![(x_min, f(x_min))];
    }
    let ratio = x_max / x_min;
    let span = (samples - 1) as f64;
    (0..samples)
        .map(|index| {
            let x = match index {
                0 => x_min,
                index if index + 1 == samples => x_max,
                index => x_min * ratio.powf(index as f64 / span),
            };
            (x, f(x))
        })
        .collect()
}

/// Samples `f` at `samples` evenly spaced points from `x_min` to `x_max`
/// (both ends included, exactly, so callers can anchor axes). Dynamics
/// transfer curves are the intended use. Fewer than two samples yields zero
/// points or the single start point.
pub fn sample_curve_linear_x(
    f: impl Fn(f64) -> f64,
    x_min: f64,
    x_max: f64,
    samples: usize,
) -> Vec<(f64, f64)> {
    if samples == 0 {
        return Vec::new();
    }
    if samples == 1 {
        return vec![(x_min, f(x_min))];
    }
    let span = (samples - 1) as f64;
    (0..samples)
        .map(|index| {
            let x = match index {
                0 => x_min,
                index if index + 1 == samples => x_max,
                index => x_min + (x_max - x_min) * (index as f64 / span),
            };
            (x, f(x))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f64 = 48_000.0;

    #[test]
    fn eq_peaking_hits_its_center_gain_and_settles_far_away() {
        let settings = EqSettings {
            frequency_hz: 1_000.0,
            gain_db: 6.0,
            q: 1.0,
        };
        assert!((eq_magnitude_db(&settings, SAMPLE_RATE, 1_000.0) - 6.0).abs() <= 0.1);
        for frequency in [20.0, 20_000.0] {
            assert!(
                eq_magnitude_db(&settings, SAMPLE_RATE, frequency).abs() <= 0.5,
                "{frequency} Hz"
            );
        }
    }

    #[test]
    fn eq_peaking_is_symmetric_on_a_log_axis_around_its_center() {
        let settings = EqSettings {
            frequency_hz: 1_000.0,
            gain_db: 6.0,
            q: 1.0,
        };
        for factor in [2.0, 4.0] {
            let below = eq_magnitude_db(&settings, SAMPLE_RATE, 1_000.0 / factor);
            let above = eq_magnitude_db(&settings, SAMPLE_RATE, 1_000.0 * factor);
            assert!((below - above).abs() < 0.1, "{factor}x: {below} vs {above}");
        }
    }

    #[test]
    fn eq_zero_gain_is_flat_zero_db_everywhere() {
        let settings = EqSettings {
            frequency_hz: 1_000.0,
            gain_db: 0.0,
            q: 1.0,
        };
        for frequency in [20.0, 100.0, 1_000.0, 5_000.0, 20_000.0] {
            assert!(
                eq_magnitude_db(&settings, SAMPLE_RATE, frequency).abs() < 1e-12,
                "{frequency} Hz"
            );
        }
    }

    #[test]
    fn hard_knee_compressor_matches_unity_and_ratio_lines() {
        let curve = DynamicsCurve::compressor(&CompressorState {
            threshold_db: -18.0,
            ratio: 4.0,
            knee_db: 0.0,
            input_gain_db: 0.0,
            makeup_gain_db: 0.0,
            output_gain_db: 0.0,
            ..Default::default()
        });
        // Below the threshold the transfer is the unity line.
        for input in [-60.0, -40.0, -20.0] {
            assert!((curve.transfer_db(input) - input).abs() < 1e-12, "{input}");
        }
        // Above it, threshold + (input - threshold) / ratio.
        for input in [0.0, 6.0, 22.0] {
            let expected = -18.0 + (input + 18.0) / 4.0;
            assert!(
                (curve.transfer_db(input) - expected).abs() < 1e-9,
                "{input}"
            );
        }
        // Continuous where the two lines meet.
        assert!((curve.transfer_db(-18.0) - (-18.0)).abs() < 1e-12);
        assert!((curve.transfer_db(-18.0 + 1e-6) - curve.transfer_db(-18.0 - 1e-6)).abs() < 1e-5);
    }

    #[test]
    fn soft_knee_keeps_value_and_slope_continuous_at_both_edges() {
        let curve = DynamicsCurve::compressor(&CompressorState {
            threshold_db: -18.0,
            ratio: 4.0,
            knee_db: 6.0,
            input_gain_db: 0.0,
            makeup_gain_db: 0.0,
            output_gain_db: 0.0,
            ..Default::default()
        });
        // Knee edges at threshold ± knee/2. The quadratic branch meets the
        // unity and compression lines within 1e-5 dB of each edge.
        for edge in [-21.0, -15.0] {
            let below = curve.transfer_db(edge - 1e-6);
            let at = curve.transfer_db(edge);
            let above = curve.transfer_db(edge + 1e-6);
            assert!((below - at).abs() < 1e-5, "value below {edge}");
            assert!((above - at).abs() < 1e-5, "value above {edge}");
        }
        // Slopes by central finite differences just inside and just outside
        // each edge: unity below, 1/ratio above, blended in between.
        let h = 1e-3;
        let slope = |x: f64| (curve.transfer_db(x + h) - curve.transfer_db(x - h)) / (2.0 * h);
        for (edge, inward) in [(-21.0, 0.01), (-15.0, -0.01)] {
            let inner = slope(edge + inward);
            let outer = slope(edge - inward);
            assert!((inner - outer).abs() < 0.05, "slope jump at {edge}");
        }
        // The quadratic branch at the threshold: reduction 0.75 * 3^2 / 12.
        assert!((curve.transfer_db(-18.0) - (-18.5625)).abs() < 1e-9);
    }

    #[test]
    fn input_gain_feeds_the_detector_before_the_threshold() {
        let curve = DynamicsCurve::compressor(&CompressorState {
            input_gain_db: 6.0,
            threshold_db: -18.0,
            ratio: 4.0,
            knee_db: 0.0,
            ..Default::default()
        });
        // -20 dB plus 6 dB of input gain lands 4 dB above the threshold.
        let expected = -14.0 - 0.75 * 4.0;
        assert!((curve.transfer_db(-20.0) - expected).abs() < 1e-9);
    }

    #[test]
    fn gains_apply_last_and_ratio_one_is_transparent() {
        let curve = DynamicsCurve::compressor(&CompressorState {
            ratio: 1.0,
            input_gain_db: 2.0,
            makeup_gain_db: 3.0,
            output_gain_db: -1.0,
            ..Default::default()
        });
        for input in [-60.0, -18.0, 0.0] {
            assert!(
                (curve.transfer_db(input) - (input + 4.0)).abs() < 1e-9,
                "{input}"
            );
        }
    }

    #[test]
    fn gate_parks_at_the_range_floor_below_threshold_and_is_unity_above() {
        let curve = DynamicsCurve::gate(&GateSettings {
            threshold_db: -40.0,
            range_db: -80.0,
            ..Default::default()
        });
        // Closed: the output sits at the (negative) range floor.
        for input in [-120.0, -80.0, -60.0, -40.5] {
            assert!((curve.transfer_db(input) - (-80.0)).abs() < 1e-9, "{input}");
        }
        // Open at envelope >= threshold: unity through the threshold point
        // itself; below it the curve steps to the floor in one hard jump.
        for input in [-40.0, -20.0, 0.0] {
            assert!((curve.transfer_db(input) - input).abs() < 1e-9, "{input}");
        }
    }

    #[test]
    fn limiter_holds_the_ceiling_and_passes_below() {
        let curve = DynamicsCurve::limiter(&LimiterSettings {
            threshold_db: -6.0,
            ..Default::default()
        });
        for input in [-60.0, -12.0, -6.0] {
            assert!((curve.transfer_db(input) - input).abs() < 1e-9, "{input}");
        }
        for input in [-3.0, 0.0, 10.0] {
            assert!((curve.transfer_db(input) - (-6.0)).abs() < 0.1, "{input}");
        }
        // Input gain feeds the detector and the unity region, as limiter.rs
        // applies the gain before limiting.
        let boosted = DynamicsCurve::limiter(&LimiterSettings {
            threshold_db: -1.0,
            input_gain_db: 6.0,
            ..Default::default()
        });
        assert!((boosted.transfer_db(-10.0) - (-4.0)).abs() < 1e-9);
        assert!((boosted.transfer_db(0.0) - (-1.0)).abs() < 0.1);
    }

    #[test]
    fn log_sampler_is_geometric_with_exact_endpoints() {
        let points = sample_curve_log_x(|x| 2.0 * x, 20.0, 20_000.0, 5);
        assert_eq!(points.len(), 5);
        assert_eq!(points[0].0, 20.0);
        assert_eq!(points[4].0, 20_000.0);
        let step = points[1].0 / points[0].0;
        for window in points.windows(2) {
            let ratio = window[1].0 / window[0].0;
            assert!((ratio - step).abs() <= 1e-9 * step, "{ratio} vs {step}");
        }
        for (x, y) in &points {
            assert!((y - 2.0 * x).abs() < 1e-9);
        }
    }

    #[test]
    fn linear_sampler_is_even_with_exact_endpoints() {
        let points = sample_curve_linear_x(|x| x * x, -60.0, 0.0, 7);
        assert_eq!(points.len(), 7);
        assert_eq!(points[0].0, -60.0);
        assert_eq!(points[6].0, 0.0);
        let step = points[1].0 - points[0].0;
        for window in points.windows(2) {
            assert!((window[1].0 - window[0].0 - step).abs() < 1e-9);
        }
        for (x, y) in &points {
            assert!((y - x * x).abs() < 1e-9);
        }
    }

    #[test]
    fn samplers_handle_degenerate_sample_counts() {
        assert!(sample_curve_log_x(|x| x, 20.0, 20_000.0, 0).is_empty());
        assert!(sample_curve_linear_x(|x| x, -60.0, 0.0, 0).is_empty());
        let single = sample_curve_log_x(|x| x + 1.0, 20.0, 20_000.0, 1);
        assert_eq!(single, vec![(20.0, 21.0)]);
        let single = sample_curve_linear_x(|x| x + 1.0, -60.0, 0.0, 1);
        assert_eq!(single, vec![(-60.0, -59.0)]);
    }
}
