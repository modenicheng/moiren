//! Diagnostic validation of the sink's stored timeline, not a native epoch or
//! hardware clock. Do not infer either from uncalibrated WASAPI packet fields.
use super::{MarkerCorrelation, PcmAnalysis, PcmSchedule, analyze, marker_distribution};
use anyhow::{Context, Result, ensure};
use moiren_windows_audio::stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Seek};

const MAX_METADATA_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PACKETS: usize = 1_000_000;
const MAX_RECORD_BYTES: usize = 1024;

// Exactly measurement_sink::PacketMetadata; notably, there is no epoch field.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PacketMetadata {
    packet_sequence: u64,
    flags: u32,
    device_position_frames: u64,
    qpc_100ns: u64,
    frames: u32,
    stored_start_frame: u64,
}

#[derive(Debug, Serialize)]
struct PacketObservation {
    record_index: usize,
    #[serde(flatten)]
    packet: PacketMetadata,
    issues: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct StartupEvidence {
    scope: &'static str,
    marker_phase: &'static str,
    packet: Option<PacketMetadata>,
    issues: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct PacketEvidence {
    schema_version: u32,
    packet_count: usize,
    coverage_complete: bool,
    native_epoch: &'static str,
    timestamp_provenance: &'static str,
    qpc_interval_policy: &'static str,
    unknown_timestamp_packets: usize,
    unknown_position_packets: usize,
    startup: StartupEvidence,
    observations: Vec<PacketObservation>,
    coverage_error: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct PacketAnalysis {
    #[serde(flatten)]
    pub analysis: PcmAnalysis,
    packet_evidence: PacketEvidence,
    distribution_scope: &'static str,
    delay_basis: &'static str,
}

impl PacketAnalysis {
    pub fn failed(&self) -> bool {
        self.analysis.failed_markers != 0
            || !self.packet_evidence.coverage_complete
            || !self.packet_evidence.startup.issues.is_empty()
            || !self.packet_evidence.observations.is_empty()
    }
}

struct PacketSpan {
    start: u64,
    end: u64,
    invalid: bool,
}

pub struct PacketTimeline {
    evidence: PacketEvidence,
    // Prefix/omitted metadata must not be disguised as continuous PCM. A
    // coverage mismatch makes the entire stored mapping untrustworthy.
    spans: Vec<PacketSpan>,
}

impl PacketTimeline {
    pub fn read(
        reader: impl Read,
        max_bytes: u64,
        max_packets: usize,
        frames: u64,
        sample_rate: u32,
    ) -> Result<Self> {
        ensure!(
            (1..=MAX_METADATA_BYTES).contains(&max_bytes),
            "invalid packet metadata byte cap"
        );
        ensure!(
            (1..=MAX_PACKETS).contains(&max_packets),
            "invalid packet count cap"
        );
        ensure!(sample_rate > 0, "packet sample rate must be nonzero");
        let mut reader = BufReader::new(reader.take(max_bytes + 1));
        let mut record = Vec::with_capacity(MAX_RECORD_BYTES + 1);
        let mut spans = Vec::new();
        let mut observations = Vec::new();
        let mut startup = StartupEvidence {
            scope: "first_retained_capture_packet_only_not_steady_state",
            marker_phase: "not_recorded",
            packet: None,
            issues: Vec::new(),
        };
        let mut previous: Option<PacketMetadata> = None;
        let mut consumed = 0_u64;
        let mut next_stored = 0;
        let mut coverage_complete = true;
        let mut unknown_timestamp_packets = 0;
        let mut unknown_position_packets = 0;
        loop {
            record.clear();
            // Limit before read_until: neither an unterminated line nor a
            // growing input can force an unbounded allocation/read.
            let count = (&mut reader)
                .take((MAX_RECORD_BYTES + 1) as u64)
                .read_until(b'\n', &mut record)?;
            if count == 0 {
                break;
            }
            consumed += count as u64;
            ensure!(consumed <= max_bytes, "packet metadata byte cap exceeded");
            ensure!(
                count <= MAX_RECORD_BYTES,
                "packet metadata record exceeds 1024 bytes"
            );
            ensure!(
                record.last() == Some(&b'\n'),
                "unterminated packet metadata record"
            );
            ensure!(spans.len() < max_packets, "packet count cap exceeded");
            let packet: PacketMetadata = serde_json::from_slice(&record)
                .with_context(|| format!("parse packet metadata record {}", spans.len()))?;
            ensure!(packet.frames > 0, "packet frames must be nonzero");
            ensure!(
                packet.packet_sequence > 0,
                "packet sequence must be nonzero"
            );
            let end = packet
                .stored_start_frame
                .checked_add(u64::from(packet.frames))
                .context("stored packet range overflow")?;
            let position_end = packet
                .device_position_frames
                .checked_add(u64::from(packet.frames));
            let mut issues = Vec::new();
            if packet.stored_start_frame != next_stored || end > frames {
                coverage_complete = false;
                issues.push("stored_pcm_coverage_mismatch");
            }
            next_stored = end;
            if packet.flags & DISCONTINUITY != 0 {
                issues.push("native_data_discontinuity_epoch_unknown");
            }
            if packet.flags & SILENT != 0 {
                issues.push("native_silent_packet");
            }
            if packet.flags & !(DISCONTINUITY | SILENT | TIMESTAMP_ERROR) != 0 {
                issues.push("unknown_native_flags");
            }
            let timestamp_known = packet.flags & TIMESTAMP_ERROR == 0 && packet.qpc_100ns != 0;
            if !timestamp_known {
                unknown_timestamp_packets += 1;
                issues.push("native_timestamp_unknown_or_error");
            }
            // All-zero Process Loopback positions are unavailable, not a
            // hardware clock. A zero reached from a positive position is
            // still a reset observation below, never a fabricated epoch.
            if packet.device_position_frames == 0 {
                unknown_position_packets += 1;
            }
            if position_end.is_none() {
                issues.push("native_position_overflow");
            }
            if let Some(prior) = &previous {
                if prior.packet_sequence.checked_add(1) != Some(packet.packet_sequence) {
                    issues.push("packet_sequence_loss_reorder_or_reset");
                }
                if timestamp_known && prior.flags & TIMESTAMP_ERROR == 0 && prior.qpc_100ns != 0 {
                    if packet.qpc_100ns <= prior.qpc_100ns {
                        issues.push("native_qpc_repeat_or_rewind_epoch_unknown");
                    } else {
                        // Admission policy, not calibrated clock quality:
                        // tolerate one sample of quantization plus 1000 ppm
                        // drift. Larger jumps may be loss/reset, never delay.
                        let observed = u128::from(packet.qpc_100ns - prior.qpc_100ns)
                            * u128::from(sample_rate);
                        let expected = u128::from(prior.frames) * 10_000_000;
                        if observed.abs_diff(expected) > 10_000_000 + expected / 1000 {
                            issues.push("native_qpc_interval_outside_diagnostic_policy");
                        }
                    }
                    if (prior.device_position_frames != 0 || packet.device_position_frames != 0)
                        && prior
                            .device_position_frames
                            .checked_add(u64::from(prior.frames))
                            != Some(packet.device_position_frames)
                    {
                        issues.push("native_position_gap_repeat_or_rewind_epoch_unknown");
                    }
                }
            } else if packet.packet_sequence != 1 {
                issues.push("capture_prefix_packets_missing");
            }
            let invalid = !issues.is_empty();
            if previous.is_none() {
                startup.issues = issues;
                startup.packet = Some(packet);
            } else if invalid {
                observations.push(PacketObservation {
                    record_index: spans.len(),
                    packet,
                    issues,
                });
            }
            spans.push(PacketSpan {
                start: packet.stored_start_frame,
                end,
                invalid,
            });
            previous = Some(packet);
        }
        ensure!(!spans.is_empty(), "empty packet metadata");
        coverage_complete &= next_stored == frames;
        Ok(Self {
            evidence: PacketEvidence {
                schema_version: 1,
                packet_count: spans.len(),
                coverage_complete,
                native_epoch: "not_recorded",
                timestamp_provenance: "unverified_native_packet_fields_not_hardware_or_acoustics",
                qpc_interval_policy: "nominal_schedule_rate; tolerance_one_frame_plus_1000ppm; diagnostic_invalidation_not_clock_calibration",
                unknown_timestamp_packets,
                unknown_position_packets,
                startup,
                observations,
                coverage_error: (!coverage_complete).then_some("metadata does not cover PCM exactly once in stored order; all windows invalidated"),
            },
            spans,
        })
    }

    pub fn analyze(
        self,
        reader: &mut (impl Read + Seek),
        bytes: u64,
        schedule: PcmSchedule,
    ) -> Result<PacketAnalysis> {
        // Preserve the legacy correlation path and its bounds/failure details;
        // validation can only remove candidates, never manufacture a delay.
        let window = schedule.validate(bytes)? as u64;
        let mut analysis = analyze(reader, bytes, schedule)?;
        let mut span_index = 0;
        for result in &mut analysis.results {
            let end = result.start_frame + window;
            while span_index < self.spans.len() && self.spans[span_index].end <= result.start_frame
            {
                span_index += 1;
            }
            let mut index = span_index;
            let mut invalid = !self.evidence.coverage_complete;
            // With exact coverage and nonoverlapping scheduled windows, this
            // visits O(packets + markers), not their Cartesian product.
            while !invalid && index < self.spans.len() && self.spans[index].start < end {
                invalid |= self.spans[index].invalid;
                index += 1;
            }
            if invalid {
                let reason = "packet evidence invalidates this correlation window (loss, reset, timestamp, silence or coverage); alignments are diagnostic only";
                result.error = Some(match result.error.take() {
                    Some(error) => format!("{error}; {reason}"),
                    None => reason.to_string(),
                });
                result.delay_frames = None;
            }
        }
        let delays: Vec<_> = analysis
            .results
            .iter()
            .filter_map(|result| {
                let delay_frames = result.delay_frames?;
                let reference = result.reference.as_ref()?;
                let processed = result.processed.as_ref()?;
                Some(MarkerCorrelation {
                    delay_frames,
                    confidence: reference.confidence.min(processed.confidence),
                    peak_margin: reference.peak_margin.min(processed.peak_margin),
                    inverted: processed.inverted,
                })
            })
            .collect();
        analysis.failed_markers = analysis.results.len() - delays.len();
        analysis.distribution = if delays.is_empty() {
            None
        } else {
            Some(marker_distribution(
                &delays,
                analysis.schedule.sample_rate,
                analysis.schedule.max_markers,
            )?)
        };
        analysis.schema_version = 2;
        analysis.mode = "offline_stereo_f32le_packet_validated_marker_windows";
        Ok(PacketAnalysis {
            analysis,
            packet_evidence: self.evidence,
            distribution_scope: "successful_windows_only",
            delay_basis: "per_window_common_pcm_sample_offsets_at_schedule_rate; drift is not resampled; no physical boundary, calibration or startup phase inferred",
        })
    }
}
