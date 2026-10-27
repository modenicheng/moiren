//! Process-loopback capture hands its bridge back to the control side.
use super::Startup;
use crate::{
    capture::{CaptureError, CaptureReport},
    clock_bridge::{CaptureIngress, ClockBridgeConfig, capture_bridge_with_clock},
    latency::NativeStreamTiming,
};
use std::sync::mpsc;

pub(super) enum CapturePreparation {
    Immediate(mpsc::SyncSender<Result<Startup, CaptureError>>),
}
impl CapturePreparation {
    pub(super) fn prepare(
        &self,
        timing: NativeStreamTiming,
        channels: usize,
        report: &mut CaptureReport,
        age_clock: Option<crate::latency::clock::QpcClock>,
    ) -> Result<CaptureIngress, CaptureError> {
        let Self::Immediate(sender) = self;
        let _ = report;
        // Process loopback positions come from the loopback transport, not a
        // position-clocked physical client; gap inference stays off.
        let (ingress, source, observer) = capture_bridge_with_clock(
            ClockBridgeConfig {
                input_sample_rate: timing.sample_rate,
                input_channels: channels,
                detect_position_gaps: false,
                ..ClockBridgeConfig::default()
            },
            age_clock,
        )?;
        sender
            .send(Ok(Startup {
                source,
                observer,
                sample_rate: timing.sample_rate,
                channels,
            }))
            .map_err(|_| CaptureError::StartupLost)?;
        Ok(ingress)
    }
}
