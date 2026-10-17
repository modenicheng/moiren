use super::{MeasurementMethod, TelemetryMode};
use serde::{Serialize, Serializer};

pub const TRACE_CAPACITY: usize = 256;
const HISTOGRAM_BUCKETS: usize = 257;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HistogramSnapshot {
    pub samples: u64,
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub p50_upper_bound: Option<u64>,
    pub p95_upper_bound: Option<u64>,
    pub p99_upper_bound: Option<u64>,
    pub saturated: bool,
}

#[derive(Clone)]
struct Histogram {
    buckets: [u64; HISTOGRAM_BUCKETS],
    samples: u64,
    min: Option<u64>,
    max: Option<u64>,
    saturated: bool,
}
impl Histogram {
    fn new() -> Self {
        Self {
            buckets: [0; HISTOGRAM_BUCKETS],
            samples: 0,
            min: None,
            max: None,
            saturated: false,
        }
    }
    fn record(&mut self, value: u64) {
        let bucket = if value == 0 {
            0
        } else {
            let exponent = 63 - value.leading_zeros();
            let base = 1u64 << exponent;
            let quarter = ((u128::from(value - base) * 4) / u128::from(base)) as usize;
            1 + exponent as usize * 4 + quarter
        };
        self.saturated |= self.samples == u64::MAX || self.buckets[bucket] == u64::MAX;
        self.samples = self.samples.saturating_add(1);
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.min = Some(self.min.map_or(value, |previous| previous.min(value)));
        self.max = Some(self.max.map_or(value, |previous| previous.max(value)));
    }
    fn quantile(&self, percentile: u64) -> Option<u64> {
        if self.samples == 0 || self.saturated {
            return None;
        }
        let rank =
            self.samples / 100 * percentile + (self.samples % 100 * percentile).div_ceil(100);
        let mut accumulated = 0u64;
        for (index, count) in self.buckets.iter().enumerate() {
            accumulated = accumulated.saturating_add(*count);
            if accumulated >= rank {
                if index == 0 {
                    return Some(0);
                }
                let exponent = (index - 1) / 4;
                let quarter = (index - 1) % 4;
                let boundary = ((1u128 << exponent) * (quarter as u128 + 5)).div_ceil(4);
                return Some(((boundary - 1).min(u128::from(u64::MAX)) as u64).min(self.max?));
            }
        }
        None
    }
    fn snapshot(&self) -> HistogramSnapshot {
        HistogramSnapshot {
            samples: self.samples,
            min: self.min,
            max: self.max,
            p50_upper_bound: self.quantile(50),
            p95_upper_bound: self.quantile(95),
            p99_upper_bound: self.quantile(99),
            saturated: self.saturated,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CallbackObservation {
    pub wake_qpc_100ns: Option<u64>,
    pub completed_qpc_100ns: Option<u64>,
    pub demand_frames: u64,
    pub padding_before_frames: Option<u32>,
    pub submitted_frames: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TraceRecord {
    pub wake_qpc_100ns: u64,
    pub demand_frames: u64,
    pub padding_before_frames: Option<u32>,
    pub submitted_frames: u64,
    pub callback_duration_100ns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceSnapshot {
    records: Box<[TraceRecord]>,
}
impl TraceSnapshot {
    pub fn records(&self) -> &[TraceRecord] {
        &self.records
    }
}
impl Serialize for TraceSnapshot {
    fn serialize<Output: Serializer>(
        &self,
        serializer: Output,
    ) -> Result<Output::Ok, Output::Error> {
        self.records().serialize(serializer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LatencySnapshot {
    pub schema_version: u32,
    pub mode: TelemetryMode,
    /// Owner diagnostics alone cannot establish an end-to-end measurement.
    pub measurement_method: MeasurementMethod,
    pub wake_interval_100ns: HistogramSnapshot,
    pub callback_duration_100ns: HistogramSnapshot,
    pub demand_frames: HistogramSnapshot,
    pub padding_before_frames: HistogramSnapshot,
    pub submitted_frames: HistogramSnapshot,
    pub invalid_timestamps: u64,
    pub regressed_wakes: u64,
    pub trace_dropped: u64,
    pub counters_saturated: bool,
    pub trace: TraceSnapshot,
}

/// One writer owns fixed storage. A full diagnostic trace never blocks, grows,
/// or changes the audio transfer. Only non-RT snapshots copy valid records
/// into owned storage, keeping nested owner/report handoffs small on Windows.
pub struct LatencyRecorder {
    mode: TelemetryMode,
    previous_wake: Option<u64>,
    wake_interval: Histogram,
    callback_duration: Histogram,
    demand: Histogram,
    padding: Histogram,
    submitted: Histogram,
    invalid_timestamps: u64,
    regressed_wakes: u64,
    trace_dropped: u64,
    counters_saturated: bool,
    trace: [TraceRecord; TRACE_CAPACITY],
    trace_len: usize,
}
impl LatencyRecorder {
    pub fn new(mode: TelemetryMode) -> Self {
        Self {
            mode,
            previous_wake: None,
            wake_interval: Histogram::new(),
            callback_duration: Histogram::new(),
            demand: Histogram::new(),
            padding: Histogram::new(),
            submitted: Histogram::new(),
            invalid_timestamps: 0,
            regressed_wakes: 0,
            trace_dropped: 0,
            counters_saturated: false,
            trace: [TraceRecord::default(); TRACE_CAPACITY],
            trace_len: 0,
        }
    }
    pub fn record(&mut self, observation: CallbackObservation) {
        if self.mode == TelemetryMode::Disabled {
            return;
        }
        self.demand.record(observation.demand_frames);
        self.submitted.record(observation.submitted_frames);
        if let Some(padding) = observation.padding_before_frames {
            self.padding.record(u64::from(padding));
        }
        let timing = observation
            .wake_qpc_100ns
            .zip(observation.completed_qpc_100ns)
            .and_then(|(wake, completed)| {
                completed.checked_sub(wake).map(|duration| (wake, duration))
            });
        let Some((wake, duration)) = timing else {
            increment(&mut self.invalid_timestamps, &mut self.counters_saturated);
            self.previous_wake = None;
            return;
        };
        self.callback_duration.record(duration);
        if let Some(previous) = self.previous_wake {
            if let Some(interval) = wake.checked_sub(previous) {
                self.wake_interval.record(interval);
            } else {
                increment(&mut self.regressed_wakes, &mut self.counters_saturated);
            }
        }
        self.previous_wake = Some(wake);
        if self.mode == TelemetryMode::Trace {
            if self.trace_len == TRACE_CAPACITY {
                increment(&mut self.trace_dropped, &mut self.counters_saturated);
            } else {
                self.trace[self.trace_len] = TraceRecord {
                    wake_qpc_100ns: wake,
                    demand_frames: observation.demand_frames,
                    padding_before_frames: observation.padding_before_frames,
                    submitted_frames: observation.submitted_frames,
                    callback_duration_100ns: duration,
                };
                self.trace_len += 1;
            }
        }
    }
    /// Non-RT only, after streaming stops. Quantile scans and allocation of
    /// valid trace records have no place in a callback; an empty trace is free.
    pub fn snapshot(&self) -> LatencySnapshot {
        LatencySnapshot {
            schema_version: 1,
            mode: self.mode,
            measurement_method: MeasurementMethod::Unknown,
            wake_interval_100ns: self.wake_interval.snapshot(),
            callback_duration_100ns: self.callback_duration.snapshot(),
            demand_frames: self.demand.snapshot(),
            padding_before_frames: self.padding.snapshot(),
            submitted_frames: self.submitted.snapshot(),
            invalid_timestamps: self.invalid_timestamps,
            regressed_wakes: self.regressed_wakes,
            trace_dropped: self.trace_dropped,
            counters_saturated: self.counters_saturated
                || self.wake_interval.saturated
                || self.callback_duration.saturated
                || self.demand.saturated
                || self.padding.saturated
                || self.submitted.saturated,
            trace: TraceSnapshot {
                records: self.trace[..self.trace_len].into(),
            },
        }
    }
}
fn increment(counter: &mut u64, saturated: &mut bool) {
    *saturated |= *counter == u64::MAX;
    *counter = counter.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_is_visible_and_never_wraps_or_claims_precise_quantiles() {
        let mut histogram = Histogram::new();
        histogram.samples = u64::MAX;
        histogram.buckets[0] = u64::MAX;
        histogram.record(0);
        let snapshot = histogram.snapshot();
        assert!(snapshot.saturated);
        assert_eq!(snapshot.samples, u64::MAX);
        assert_eq!(snapshot.p99_upper_bound, None);
        let mut recorder = LatencyRecorder::new(TelemetryMode::Trace);
        recorder.trace_len = TRACE_CAPACITY;
        recorder.trace_dropped = u64::MAX;
        recorder.record(CallbackObservation {
            wake_qpc_100ns: Some(1),
            completed_qpc_100ns: Some(2),
            ..Default::default()
        });
        assert_eq!(recorder.trace_dropped, u64::MAX);
        assert!(recorder.snapshot().counters_saturated);
    }
    #[test]
    fn quantile_rank_at_u64_max_does_not_overflow() {
        let mut histogram = Histogram::new();
        histogram.samples = u64::MAX;
        histogram.buckets[0] = u64::MAX;
        assert_eq!(histogram.quantile(99), Some(0));
    }
}
