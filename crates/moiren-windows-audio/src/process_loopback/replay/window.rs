//! Explicit finite-window acceptance. Never future-safe or physical proof.
use super::{
    ObservedProcessTrace, ProcessBridgeOptions, ProcessIdentity, ProcessReplayError,
    ProcessReplayReport,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayWindowPolicy {
    /// Consumer/output-rate frames in complete zero-output requests before start.
    pub max_initial_unserved_frames: u64,
    /// Native input frames discarded while initially priming, before first output.
    pub max_initial_prime_discarded_frames: u64,
    pub min_output_frames: u64,
    pub min_output_reads: u64,
    /// Conservative span between first-read end and last-read beginning.
    pub min_output_span_100ns: u64,
}

impl ReplayWindowPolicy {
    pub fn validate(self) -> Result<(), ProcessReplayError> {
        if self.min_output_frames == 0
            || self.min_output_reads < 2
            || self.min_output_span_100ns == 0
        {
            return Err(ProcessReplayError::InvalidWindowPolicy);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayWindowStatus {
    ObservedViolation,
    Inconclusive,
    NoViolationInWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayWindowViolation {
    InitialUnserved,
    InitialPrimeDiscard,
    PostStartUnserved,
    Overflow,
    LaterPrimeDiscard,
    BacklogDiscard,
    AgeDiscard,
    ResetDiscard,
    Reset,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayWindowAssessment {
    pub policy: ReplayWindowPolicy,
    pub status: ReplayWindowStatus,
    pub violations: Vec<ReplayWindowViolation>,
    pub insufficient_output_frames: bool,
    pub insufficient_output_reads: bool,
    pub insufficient_output_span: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssessedProcessReplay {
    pub report: ProcessReplayReport,
    pub assessment: ReplayWindowAssessment,
}

impl ObservedProcessTrace {
    /// Run actual candidate queue mechanics once and assess that exact result.
    /// Public mutable report fields cannot be promoted through an assess API.
    /// A no-violation result says nothing about waveform, age, native wake
    /// grouping, unobserved time, physical latency or future sustained service.
    pub fn replay_window(
        &self,
        process: &ProcessIdentity,
        generation: u64,
        options: ProcessBridgeOptions,
        policy: ReplayWindowPolicy,
    ) -> Result<AssessedProcessReplay, ProcessReplayError> {
        policy.validate()?;
        let report = self.replay(process, generation, options)?;
        let s = &report.candidate;
        let i = &report.inventory;
        let mut violations = Vec::new();
        for (violated, cause) in [
            (
                i.initial_unserved_frames > policy.max_initial_unserved_frames,
                ReplayWindowViolation::InitialUnserved,
            ),
            (
                i.initial_prime_discarded_frames > policy.max_initial_prime_discarded_frames,
                ReplayWindowViolation::InitialPrimeDiscard,
            ),
            (
                i.post_start_unserved_frames != 0,
                ReplayWindowViolation::PostStartUnserved,
            ),
            (s.dropped_frames != 0, ReplayWindowViolation::Overflow),
            (
                i.later_prime_discarded_frames != 0,
                ReplayWindowViolation::LaterPrimeDiscard,
            ),
            (
                s.backlog_discarded_frames != 0,
                ReplayWindowViolation::BacklogDiscard,
            ),
            (
                s.age_discarded_frames != 0,
                ReplayWindowViolation::AgeDiscard,
            ),
            (
                s.reset_discarded_frames != 0,
                ReplayWindowViolation::ResetDiscard,
            ),
            (s.resets != 0, ReplayWindowViolation::Reset),
        ] {
            if violated {
                violations.push(cause);
            }
        }
        let insufficient_output_frames = s.output_frames < policy.min_output_frames;
        let insufficient_output_reads = i.output_reads < policy.min_output_reads;
        let insufficient_output_span = i
            .output_span_lower_100ns
            .is_none_or(|span| span < policy.min_output_span_100ns);
        let status = if !violations.is_empty() {
            ReplayWindowStatus::ObservedViolation
        } else if insufficient_output_frames
            || insufficient_output_reads
            || insufficient_output_span
        {
            ReplayWindowStatus::Inconclusive
        } else {
            ReplayWindowStatus::NoViolationInWindow
        };
        Ok(AssessedProcessReplay {
            report,
            assessment: ReplayWindowAssessment {
                policy,
                status,
                violations,
                insufficient_output_frames,
                insufficient_output_reads,
                insufficient_output_span,
            },
        })
    }
}
