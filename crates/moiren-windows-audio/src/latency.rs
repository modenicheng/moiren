//! Bounded owner-local diagnostics. Queue and callback observations are not
//! an end-to-end latency measurement. Quantiles are histogram upper bounds.
mod telemetry;

#[cfg(windows)]
pub(crate) mod clock;

use serde::Serialize;
pub use telemetry::{
    CallbackObservation, HistogramSnapshot, LatencyRecorder, LatencySnapshot, TRACE_CAPACITY,
    TraceRecord, TraceSnapshot,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryMode {
    #[default]
    Disabled,
    Summary,
    Trace,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementMethod {
    #[default]
    Unknown,
    Estimate,
    SoftwareLoopback,
    PhysicalCable,
    ExternalDualChannel,
}

/// Convert raw QPC ticks to the same 100 ns unit used by WASAPI timestamps.
/// Frequency is prepared once; intermediate multiplication cannot overflow.
pub fn qpc_ticks_to_100ns(ticks: u64, frequency: u64) -> Option<u64> {
    if frequency == 0 {
        return None;
    }
    u64::try_from(u128::from(ticks) * 10_000_000 / u128::from(frequency)).ok()
}
