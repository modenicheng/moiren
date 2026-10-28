//! Offline accumulation of actual candidate boundaries, not nominal SRC math.
use super::ProcessReplayError as Error;
use crate::latency::queue::QueueHistorySnapshot;
use serde::Serialize;

/// Event-boundary samples include initial zero, post-publication peaks and
/// post-read troughs. They are not continuous/inter-read extrema or sample age.
/// Output envelopes use the original trace's host brackets, not replay CPU time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReplayInventoryReport {
    pub retired_input_frames: u64,
    pub consumed_input_frames: u64,
    pub reset_discarded_frames: u64,
    pub initial_prime_discarded_frames: u64,
    pub later_prime_discarded_frames: u64,
    /// Entire zero-output requests before the first output-bearing read.
    pub initial_unserved_frames: u64,
    /// Includes any unserved suffix of the first output-bearing read.
    pub post_start_unserved_frames: u64,
    pub event_boundary_min_pending_frames: u64,
    pub event_boundary_max_pending_frames: u64,
    pub output_reads: u64,
    pub first_output_before_100ns: Option<u64>,
    pub first_output_after_100ns: Option<u64>,
    pub last_output_before_100ns: Option<u64>,
    pub last_output_after_100ns: Option<u64>,
    pub output_span_lower_100ns: Option<u64>,
    pub output_span_upper_100ns: Option<u64>,
    pub first_output_pending_frames: Option<u64>,
    pub last_output_pending_frames: Option<u64>,
    pub post_start_event_min_pending_frames: Option<u64>,
    pub post_start_event_max_pending_frames: Option<u64>,
    /// Offered supply after the final output-bearing read, never a slope.
    pub trailing_offered_frames: u64,
}

#[derive(Default)]
pub(super) struct Inventory {
    report: ReplayInventoryReport,
    processed_reads: u64,
    started_read_accounting: bool,
    discarded: u64,
}

impl Inventory {
    pub(super) fn packet(&mut self, offered: u64) {
        // Full trace work is bounded to 2^24, so this sum cannot exhaust.
        self.report.trailing_offered_frames += offered;
    }

    pub(super) fn boundary(&mut self, pending: u64) {
        let s = &mut self.report;
        s.event_boundary_max_pending_frames = s.event_boundary_max_pending_frames.max(pending);
        if s.output_reads != 0 {
            s.post_start_event_min_pending_frames = Some(
                s.post_start_event_min_pending_frames
                    .map_or(pending, |n| n.min(pending)),
            );
            s.post_start_event_max_pending_frames = Some(
                s.post_start_event_max_pending_frames
                    .map_or(pending, |n| n.max(pending)),
            );
        }
    }

    pub(super) fn output(&mut self, frames: u64, before: u64, after: u64, pending: u64) {
        if frames == 0 {
            return;
        }
        let s = &mut self.report;
        if s.output_reads == 0 {
            s.first_output_before_100ns = Some(before);
            s.first_output_after_100ns = Some(after);
            s.first_output_pending_frames = Some(pending);
        }
        s.output_reads += 1;
        s.last_output_before_100ns = Some(before);
        s.last_output_after_100ns = Some(after);
        s.last_output_pending_frames = Some(pending);
        s.trailing_offered_frames = 0;
        if s.output_reads >= 2 {
            s.output_span_lower_100ns = before.checked_sub(s.first_output_after_100ns.unwrap());
            s.output_span_upper_100ns = after.checked_sub(s.first_output_before_100ns.unwrap());
        }
    }

    pub(super) fn reads(&mut self, history: &QueueHistorySnapshot) -> Result<(), Error> {
        for event in &history.events {
            if event.sequence <= self.processed_reads {
                continue;
            }
            if event.sequence != self.processed_reads + 1 {
                return Err(Error::InvalidHistory);
            }
            let sample = event.sample;
            let before_retired = sample
                .before
                .retired_frames()
                .ok_or(Error::InvalidHistory)?;
            let retired = sample.after.retired_frames().ok_or(Error::InvalidHistory)?;
            if before_retired != self.report.retired_input_frames || retired < before_retired {
                return Err(Error::InvalidHistory);
            }
            self.report.retired_input_frames = retired;
            add(
                &mut self.report.reset_discarded_frames,
                sample.reset_discarded_frames,
            )?;
            let unserved = sample
                .requested_frames
                .checked_sub(sample.output_frames)
                .ok_or(Error::InvalidHistory)?;
            if self.started_read_accounting || sample.output_frames != 0 {
                add(&mut self.report.post_start_unserved_frames, unserved)?;
            } else {
                add(&mut self.report.initial_unserved_frames, unserved)?;
            }
            if self.started_read_accounting {
                add(
                    &mut self.report.later_prime_discarded_frames,
                    sample.prime_discarded_frames,
                )?;
            } else {
                add(
                    &mut self.report.initial_prime_discarded_frames,
                    sample.prime_discarded_frames,
                )?;
            }
            self.started_read_accounting |= sample.output_frames != 0;
            for discarded in [
                sample.prime_discarded_frames,
                sample.backlog_discarded_frames,
                sample.age_discarded_frames,
                sample.reset_discarded_frames,
            ] {
                add(&mut self.discarded, discarded)?;
            }
            self.processed_reads = event.sequence;
        }
        if history.saturated || self.processed_reads != history.total_events {
            return Err(Error::InvalidHistory);
        }
        Ok(())
    }

    pub(super) fn finish(
        mut self,
        written: u64,
        pending: u64,
    ) -> Result<ReplayInventoryReport, Error> {
        if self.report.retired_input_frames.checked_add(pending) != Some(written) {
            return Err(Error::InvalidHistory);
        }
        self.report.consumed_input_frames = self
            .report
            .retired_input_frames
            .checked_sub(self.discarded)
            .ok_or(Error::InvalidHistory)?;
        if self.report.output_reads == 0 {
            // Without a first output there is no producer-only *tail* boundary.
            self.report.trailing_offered_frames = 0;
        }
        Ok(self.report)
    }
}

fn add(total: &mut u64, value: u64) -> Result<(), Error> {
    *total = total.checked_add(value).ok_or(Error::InvalidHistory)?;
    Ok(())
}
