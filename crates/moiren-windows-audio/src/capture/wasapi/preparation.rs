//! Process-loopback capture hands its configured bridge back to control.
use super::Startup;
use crate::{
    capture::{CaptureError, CaptureReport},
    clock_bridge::{CaptureIngress, ClockBridgeConfig, capture_bridge_with_clock},
    latency::clock::QpcClock,
};
use std::sync::mpsc;

pub(super) enum CapturePreparation {
    Configured {
        sender: mpsc::SyncSender<Result<Startup, CaptureError>>,
        config: ClockBridgeConfig,
    },
}
impl CapturePreparation {
    pub(super) fn prepare(
        &self,
        sample_rate: u32,
        channels: usize,
        report: &mut CaptureReport,
        age_clock: Option<QpcClock>,
    ) -> Result<CaptureIngress, CaptureError> {
        let Self::Configured { sender, config } = self;
        if config.max_ingress_elapsed_100ns.is_some() && age_clock.is_none() {
            return Err(crate::clock_bridge::ClockBridgeError::InvalidConfig.into());
        }
        // Process loopback positions do not establish a physical clock. Keep
        // an explicitly configured physical clock only when packet lengths
        // and device positions use the same rate (also used by loop tests).
        let detect_position_gaps = report.source == crate::capture::CaptureSource::Physical
            && config.detect_position_gaps
            && report
                .shared_period
                .mix_format
                .is_none_or(|mix| mix.sample_rate == sample_rate);
        let config = ClockBridgeConfig {
            input_sample_rate: sample_rate,
            input_channels: channels,
            detect_position_gaps,
            ..*config
        };
        let (ingress, source, observer) = capture_bridge_with_clock(config, age_clock)?;
        report.input_queue = config.prepared_input;
        sender
            .send(Ok(Startup {
                source,
                observer,
                sample_rate,
                channels,
            }))
            .map_err(|_| CaptureError::StartupLost)?;
        Ok(ingress)
    }
}

#[cfg(test)]
mod tests;
