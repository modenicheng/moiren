use serde::Serialize;
use thiserror::Error;

pub const DISCONTINUITY: u32 = 1;
pub const SILENT: u32 = 2;
pub const TIMESTAMP_ERROR: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PacketError {
    #[error("zero channel count")]
    ZeroChannels,
    #[error("packet size overflow")]
    SizeOverflow,
    #[error("packet length does not match frames and channels")]
    LengthMismatch,
}

#[derive(Default)]
pub struct PacketMetrics {
    pub samples: u64,
    pub non_finite_samples: u64,
    pub peak: f64,
    sum_squares: f64,
}

impl PacketMetrics {
    /// RMS includes silent samples and excludes non-finite samples.
    pub fn rms(&self) -> f64 {
        let finite = self.samples - self.non_finite_samples;
        if finite == 0 {
            0.0
        } else {
            (self.sum_squares / finite as f64).sqrt()
        }
    }
}

/// Inspect an interleaved f32 packet without allocation or alignment assumptions.
/// SILENT packets do not require readable data, per the WASAPI buffer contract.
pub fn analyze_f32(
    data: &[u8],
    frames: u32,
    channels: u16,
    silent: bool,
) -> Result<PacketMetrics, PacketError> {
    if channels == 0 {
        return Err(PacketError::ZeroChannels);
    }
    let samples = u64::from(frames) * u64::from(channels);
    let mut metrics = PacketMetrics {
        samples,
        ..Default::default()
    };
    if silent {
        return Ok(metrics);
    }
    let expected = usize::try_from(samples)
        .ok()
        .and_then(|n| n.checked_mul(4))
        .ok_or(PacketError::SizeOverflow)?;
    if data.len() != expected {
        return Err(PacketError::LengthMismatch);
    }
    for sample in data.as_chunks::<4>().0 {
        let value = f64::from(f32::from_le_bytes(*sample));
        if !value.is_finite() {
            metrics.non_finite_samples += 1;
            continue;
        }
        metrics.sum_squares += value * value;
        metrics.peak = metrics.peak.max(value.abs());
    }
    Ok(metrics)
}

#[derive(Default)]
pub struct CaptureStats {
    summary: CaptureSummary,
    metrics: PacketMetrics,
    last_timestamp: Option<(u64, u64)>,
}

#[derive(Clone, Default, Serialize)]
pub struct CaptureSummary {
    pub packets: u64,
    pub frames: u64,
    pub samples: u64,
    pub silent_packets: u64,
    pub signal_packets: u64,
    pub startup_discontinuities: u64,
    pub later_discontinuities: u64,
    pub timestamp_error_packets: u64,
    pub valid_timestamp_packets: u64,
    pub timestamp_regressions: u64,
    pub non_finite_samples: u64,
    pub min_packet_frames: Option<u32>,
    pub max_packet_frames: u32,
    pub first_device_position_frames: Option<u64>,
    pub last_device_position_frames: Option<u64>,
    pub first_qpc_100ns: Option<u64>,
    pub last_qpc_100ns: Option<u64>,
    pub rms: f64,
    pub peak: f64,
}

impl CaptureStats {
    pub fn observe(
        &mut self,
        frames: u32,
        flags: u32,
        position: u64,
        qpc: u64,
        metrics: PacketMetrics,
    ) {
        let summary = &mut self.summary;
        if flags & DISCONTINUITY != 0 {
            if summary.packets == 0 {
                summary.startup_discontinuities += 1;
            } else {
                summary.later_discontinuities += 1;
            }
        }
        summary.packets += 1;
        summary.frames += u64::from(frames);
        summary.silent_packets += u64::from(flags & SILENT != 0);
        summary.signal_packets += u64::from(metrics.peak > 0.0);
        summary.min_packet_frames = Some(
            summary
                .min_packet_frames
                .map_or(frames, |min| min.min(frames)),
        );
        summary.max_packet_frames = summary.max_packet_frames.max(frames);
        if flags & TIMESTAMP_ERROR != 0 {
            summary.timestamp_error_packets += 1;
        } else {
            if self
                .last_timestamp
                .is_some_and(|(p, t)| position < p || qpc < t)
            {
                summary.timestamp_regressions += 1;
            }
            summary.valid_timestamp_packets += 1;
            summary.first_device_position_frames.get_or_insert(position);
            summary.first_qpc_100ns.get_or_insert(qpc);
            summary.last_device_position_frames = Some(position);
            summary.last_qpc_100ns = Some(qpc);
            self.last_timestamp = Some((position, qpc));
        }
        self.metrics.samples += metrics.samples;
        self.metrics.non_finite_samples += metrics.non_finite_samples;
        self.metrics.sum_squares += metrics.sum_squares;
        self.metrics.peak = self.metrics.peak.max(metrics.peak);
    }

    pub fn summary(&self) -> CaptureSummary {
        CaptureSummary {
            samples: self.metrics.samples,
            non_finite_samples: self.metrics.non_finite_samples,
            rms: self.metrics.rms(),
            peak: self.metrics.peak,
            ..self.summary.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(samples: &[f32]) -> Vec<u8> {
        samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect()
    }

    #[test]
    fn stereo_frames_are_not_sample_count() {
        let metrics = analyze_f32(&bytes(&[0.5, -0.5, 1.0, -1.0]), 2, 2, false).unwrap();
        assert_eq!(metrics.samples, 4);
        assert!((metrics.rms() - 0.625_f64.sqrt()).abs() < 1e-12);
        assert_eq!(metrics.peak, 1.0);
    }

    #[test]
    fn silent_packets_require_no_readable_payload() {
        let metrics = analyze_f32(&[], 480, 2, true).unwrap();
        assert_eq!(metrics.samples, 960);
        assert_eq!(metrics.rms(), 0.0);
        assert_eq!(metrics.peak, 0.0);
    }

    #[test]
    fn malformed_payload_is_rejected() {
        assert!(analyze_f32(&[0; 7], 1, 2, false).is_err());
        assert!(analyze_f32(&[], 1, 0, true).is_err());
    }

    #[test]
    fn non_finite_values_are_counted_without_poisoning_metrics() {
        let metrics = analyze_f32(&bytes(&[f32::NAN, f32::INFINITY, 0.5]), 3, 1, false).unwrap();
        assert_eq!(metrics.non_finite_samples, 2);
        assert_eq!(metrics.rms(), 0.5);
    }

    #[test]
    fn silence_contributes_to_aggregate_rms() {
        let mut stats = CaptureStats::default();
        stats.observe(
            2,
            0,
            0,
            100,
            analyze_f32(&bytes(&[1.0, 1.0]), 2, 1, false).unwrap(),
        );
        stats.observe(2, SILENT, 2, 200, analyze_f32(&[], 2, 1, true).unwrap());
        let summary = stats.summary();
        assert_eq!(summary.frames, 4);
        assert_eq!(summary.signal_packets, 1);
        assert!((summary.rms - 0.5_f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn startup_discontinuity_is_distinguished_from_later_glitches() {
        let mut stats = CaptureStats::default();
        for (position, time) in [(0, 100), (1, 200)] {
            stats.observe(
                1,
                DISCONTINUITY | SILENT,
                position,
                time,
                analyze_f32(&[], 1, 1, true).unwrap(),
            );
        }
        let summary = stats.summary();
        assert_eq!(summary.startup_discontinuities, 1);
        assert_eq!(summary.later_discontinuities, 1);
    }

    #[test]
    fn erroneous_timestamps_do_not_create_false_regressions() {
        let mut stats = CaptureStats::default();
        for (flags, position, time) in [
            (SILENT, 10, 100),
            (SILENT | TIMESTAMP_ERROR, 0, 0),
            (SILENT, 20, 200),
        ] {
            stats.observe(
                1,
                flags,
                position,
                time,
                analyze_f32(&[], 1, 1, true).unwrap(),
            );
        }
        let summary = stats.summary();
        assert_eq!(summary.timestamp_error_packets, 1);
        assert_eq!(summary.timestamp_regressions, 0);
        assert_eq!(summary.valid_timestamp_packets, 2);
    }

    #[test]
    fn timestamp_regressions_are_reported() {
        let mut stats = CaptureStats::default();
        for (position, time) in [(20, 200), (10, 100)] {
            stats.observe(
                1,
                SILENT,
                position,
                time,
                analyze_f32(&[], 1, 1, true).unwrap(),
            );
        }
        assert_eq!(stats.summary().timestamp_regressions, 1);
    }
}
