use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct ClockPoint {
    pub arrival_ms: f64,
    pub position_units: u64,
    pub qpc_100ns: u64,
    pub hresult: i32,
}

#[derive(Serialize)]
pub struct ClockFit {
    pub samples: u64,
    pub span_seconds: f64,
    pub rate_ratio: f64,
    pub drift_ppm: f64,
    pub rms_residual_us: f64,
}

#[derive(Default, Serialize)]
pub struct ClockSummary {
    pub accepted_reads: u64,
    pub low_accuracy_reads: u64,
    pub failed_reads: u64,
    pub zero_qpc_reads: u64,
    pub regressions: u64,
    pub duplicate_reads: u64,
    pub inconsistent_qpc_reads: u64,
    pub stationary_reads: u64,
    pub fit: Option<ClockFit>,
}

#[derive(Serialize)]
pub struct DeviceClockFit {
    pub samples: u64,
    pub span_seconds: f64,
    pub frames_per_second: f64,
    pub rms_residual_frames: f64,
}

#[derive(Serialize)]
pub struct DeviceClockSummary {
    pub diagnostics: ClockSummary,
    pub fit: Option<DeviceClockFit>,
}

/// IAudioClock2 positions count device frames, whose nominal rate may be unknown.
/// Report frames/second directly, without assuming the client's mix sample rate.
pub fn analyze_device_clock(
    points: &[ClockPoint],
    window: Option<(u64, u64)>,
) -> DeviceClockSummary {
    let mut diagnostics = analyze_clock(points, 1, window);
    let fit = diagnostics.fit.take().map(|fit| DeviceClockFit {
        samples: fit.samples,
        span_seconds: fit.span_seconds,
        frames_per_second: fit.rate_ratio,
        rms_residual_frames: fit.rms_residual_us / 1_000_000.0,
    });
    DeviceClockSummary { diagnostics, fit }
}

/// Estimate the rate of the API-exposed clock against correlated QPC timestamps.
/// GetPosition units must be normalized by GetFrequency, not by sample rate.
/// A shared QPC window lets callers compare streams with different start times.
pub fn analyze_clock(
    points: &[ClockPoint],
    frequency: u64,
    window: Option<(u64, u64)>,
) -> ClockSummary {
    let mut result = ClockSummary::default();
    if frequency == 0 {
        return result;
    }
    let mut origin = None;
    let mut last = None;
    let (mut mx, mut my, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
    for point in points {
        if window.is_some_and(|(start, end)| point.qpc_100ns < start || point.qpc_100ns > end) {
            continue;
        }
        if point.hresult == 1 {
            result.low_accuracy_reads += 1;
            continue;
        }
        if point.hresult != 0 {
            result.failed_reads += 1;
            continue;
        }
        if point.qpc_100ns == 0 {
            result.zero_qpc_reads += 1;
            continue;
        }
        if let Some((p, t)) = last {
            if point.position_units < p || point.qpc_100ns < t {
                result.regressions += 1;
            }
            if point.qpc_100ns == t {
                if point.position_units == p {
                    result.duplicate_reads += 1;
                } else {
                    result.inconsistent_qpc_reads += 1;
                }
                continue;
            }
            result.stationary_reads += u64::from(point.position_units == p);
        }
        let (p0, t0) = *origin.get_or_insert((point.position_units, point.qpc_100ns));
        last = Some((point.position_units, point.qpc_100ns));
        let (Some(p), Some(t)) = (
            point.position_units.checked_sub(p0),
            point.qpc_100ns.checked_sub(t0),
        ) else {
            continue;
        };
        let x = t as f64 / 10_000_000.0;
        let y = p as f64 / frequency as f64;
        result.accepted_reads += 1;
        let n = result.accepted_reads as f64;
        let dx = x - mx;
        let dy = y - my;
        mx += dx / n;
        my += dy / n;
        sxx += dx * (x - mx);
        sxy += dx * (y - my);
    }
    if let (Some((p0, t0)), Some((p1, t1))) = (origin, last) {
        let span = t1.saturating_sub(t0) as f64 / 10_000_000.0;
        if result.regressions == 0
            && result.inconsistent_qpc_reads == 0
            && result.accepted_reads >= 3
            && span >= 1.0
            && p1 > p0
            && sxx > 0.0
        {
            let rate_ratio = sxy / sxx;
            // A second pass avoids subtracting two large sums to obtain a tiny
            // timing residual, which loses microsecond accuracy on long traces.
            let mut residual = 0.0;
            let mut previous_qpc = None;
            for point in points {
                if point.hresult != 0
                    || point.qpc_100ns == 0
                    || window.is_some_and(|(start, end)| {
                        point.qpc_100ns < start || point.qpc_100ns > end
                    })
                    || previous_qpc == Some(point.qpc_100ns)
                {
                    continue;
                }
                let (Some(p), Some(t)) = (
                    point.position_units.checked_sub(p0),
                    point.qpc_100ns.checked_sub(t0),
                ) else {
                    continue;
                };
                previous_qpc = Some(point.qpc_100ns);
                let x = t as f64 / 10_000_000.0;
                let y = p as f64 / frequency as f64;
                residual += ((y - my) - rate_ratio * (x - mx)).powi(2);
            }
            result.fit = Some(ClockFit {
                samples: result.accepted_reads,
                span_seconds: span,
                rate_ratio,
                drift_ppm: (rate_ratio - 1.0) * 1_000_000.0,
                rms_residual_us: (residual / result.accepted_reads as f64).sqrt() * 1_000_000.0,
            });
        }
    }
    result
}

pub fn relative_ppm(left_rate: f64, right_rate: f64) -> Option<f64> {
    if !left_rate.is_finite() || !right_rate.is_finite() || left_rate <= 0.0 || right_rate <= 0.0 {
        return None;
    }
    Some((left_rate / right_rate - 1.0) * 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(step: u64) -> Vec<ClockPoint> {
        (0..=20)
            .map(|i| ClockPoint {
                arrival_ms: i as f64 * 1000.0,
                position_units: 500_000 + i * step,
                qpc_100ns: 8_000_000_000_000 + i * 10_000_000,
                hresult: 0,
            })
            .collect()
    }

    #[test]
    fn byte_positions_use_frequency_not_sample_rate() {
        let fit = analyze_clock(&points(384_000), 384_000, None).fit.unwrap();
        assert!(fit.drift_ppm.abs() < 1e-8);
        assert_eq!(fit.span_seconds, 20.0);
    }

    #[test]
    fn positive_and_negative_drift_are_measured() {
        let fast = analyze_clock(&points(1_000_100), 1_000_000, None)
            .fit
            .unwrap();
        let slow = analyze_clock(&points(999_925), 1_000_000, None)
            .fit
            .unwrap();
        assert!((fast.drift_ppm - 100.0).abs() < 1e-6);
        assert!((slow.drift_ppm + 75.0).abs() < 1e-6);
    }

    #[test]
    fn stationary_positions_do_not_become_a_false_clock_rate() {
        assert!(analyze_clock(&points(0), 384_000, None).fit.is_none());
        assert!(analyze_clock(&points(1), 0, None).fit.is_none());
    }

    #[test]
    fn low_accuracy_reads_are_excluded() {
        let mut readings = points(48_000);
        readings[10].hresult = 1; // S_FALSE: successful but reduced accuracy.
        readings[10].position_units = 1;
        let summary = analyze_clock(&readings, 48_000, None);
        assert_eq!(summary.low_accuracy_reads, 1);
        assert_eq!(summary.regressions, 0);
        assert!(summary.fit.unwrap().drift_ppm.abs() < 1e-8);
    }

    #[test]
    fn reused_qpc_with_advancing_position_is_not_a_position_reset() {
        let mut readings = points(48_000);
        readings[10].qpc_100ns = readings[9].qpc_100ns;
        let summary = analyze_clock(&readings, 48_000, None);
        assert_eq!(summary.regressions, 0);
        assert_eq!(summary.inconsistent_qpc_reads, 1);
        assert!(summary.fit.is_none());
    }

    #[test]
    fn reset_and_bad_qpc_invalidate_fit() {
        let mut readings = points(48_000);
        readings[10].position_units = 1;
        let summary = analyze_clock(&readings, 48_000, None);
        assert_eq!(summary.regressions, 1);
        assert!(summary.fit.is_none());
    }

    #[test]
    fn same_qpc_window_is_used_for_comparison() {
        let readings = points(48_000);
        let summary = analyze_clock(
            &readings,
            48_000,
            Some((readings[5].qpc_100ns, readings[15].qpc_100ns)),
        );
        let fit = summary.fit.unwrap();
        assert_eq!(fit.samples, 11);
        assert_eq!(fit.span_seconds, 10.0);
    }

    #[test]
    fn long_trace_preserves_microsecond_residual() {
        let readings: Vec<_> = (0..30_000u64)
            .map(|i| ClockPoint {
                arrival_ms: i as f64 * 10.0,
                position_units: 100 + i * 100_000 + if i % 2 == 0 { 10 } else { 0 },
                qpc_100ns: 8_000_000_000_000 + i * 100_000,
                hresult: 0,
            })
            .collect();
        let fit = analyze_clock(&readings, 10_000_000, None).fit.unwrap();
        assert!(
            (fit.rms_residual_us - 0.5).abs() < 0.005,
            "{}",
            fit.rms_residual_us
        );
    }

    #[test]
    fn relative_rate_is_ratio_not_difference_of_sample_counts() {
        assert!((relative_ppm(1.0001, 0.999925).unwrap() - 175.0131259844).abs() < 1e-6);
        assert!(relative_ppm(1.0, 0.0).is_none());
    }

    #[test]
    fn device_clock_reports_frames_per_second_without_assumed_nominal_rate() {
        let summary = analyze_device_clock(&points(48_005), None);
        assert!(summary.diagnostics.fit.is_none());
        let fit = summary.fit.unwrap();
        assert!((fit.frames_per_second - 48_005.0).abs() < 1e-6);
        assert!(fit.rms_residual_frames < 1e-6);
    }
}
