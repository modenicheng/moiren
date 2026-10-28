//! Diagnostic replay assumptions, deliberately not measured production admission.
use super::ProcessTraceSessionEvidence;
use moiren_windows_audio::{
    latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    process_loopback::{AssessedProcessReplay, ProcessBridgeOptions, ReplayWindowPolicy},
};

impl ProcessTraceSessionEvidence {
    /// Control/background worker only. These declared bounds describe the closed
    /// packet prefix, not whole native drains or future delivery. A successful
    /// replay must never authorize an owner replacement or a sample-age claim.
    pub fn evaluate_candidate(
        &self,
        capacity_frames: u32,
        target_frames: u32,
        jitter_frames: u32,
        policy: ReplayWindowPolicy,
    ) -> Result<AssessedProcessReplay, String> {
        let trace = self.ordered_trace()?;
        super::super::MeasuredProcessRequest {
            node: self.origin.node,
            session_generation: self.origin.generation,
            native_owner_generation: self.origin.native_owner_generation,
            graph_revision: self.origin.graph_revision,
            capacity_frames,
            target_frames,
            jitter_frames,
        }
        .validate()?;
        let archive = &self.artifact;
        if archive.input_sample_rate != 48_000
            || !archive.process.same_process(&self.origin.process)
            || archive.owner_generation != self.origin.native_owner_generation
        {
            return Err("Process trace metadata does not match its retained origin".into());
        }
        let packets = &archive.packet_history.events;
        let max_packet_frames = packets
            .iter()
            .map(|e| e.sample.offered_frames)
            .max()
            .ok_or("Process replay needs packet observations")?;
        // Take the outer envelope, never the narrower interval between end and
        // beginning. Existing immutable trace validation has checked the clocks.
        let mut max_gap_100ns = None;
        for pair in packets.windows(2) {
            let gap = pair[1]
                .sample
                .qpc_after_100ns
                .zip(pair[0].sample.qpc_before_100ns)
                .and_then(|(after, before)| after.checked_sub(before))
                .ok_or("Process replay packet clock envelope is unavailable or regressed")?;
            max_gap_100ns = Some(max_gap_100ns.unwrap_or(0).max(gap));
        }
        let max_gap_100ns = max_gap_100ns
            .ok_or("Process replay needs two packet observations for its declared gap")?;
        let frames = (u128::from(max_gap_100ns) * 48_000).div_ceil(10_000_000);
        let options = ProcessBridgeOptions {
            input: TransportInputPolicy::Target {
                capacity_frames,
                target_frames,
                jitter_frames,
            },
            delivery: Some(InputDeliveryTiming {
                sample_rate: 48_000,
                max_packet_frames: u32::try_from(max_packet_frames)
                    .map_err(|_| "Process packet bound overflow")?,
                max_gap_frames: u32::try_from(frames).map_err(|_| "Process gap bound overflow")?,
                source: DeliveryTimingSource::Declared,
            }),
            engine_sample_rate: archive.consumer_sample_rate,
            max_demand_frames: archive.prepared_consumer_demand_frames,
            age_limit_100ns: None,
        };
        trace
            .replay_window(
                &self.origin.process,
                self.origin.native_owner_generation,
                options,
                policy,
            )
            .map_err(|error| error.to_string())
    }
}
