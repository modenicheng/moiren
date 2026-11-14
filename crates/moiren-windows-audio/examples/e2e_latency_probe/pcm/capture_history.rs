//! Strict control-side framing for bounded native offers. No clock is inferred.
use super::PacketMetadata;
use anyhow::{Context, Result, ensure};
use moiren_windows_audio::stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR};
use serde::{Deserialize, Deserializer, Serialize};

const FORMAT: &str = "moiren_native_capture_packet_history";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Header {
    schema_version: u32,
    format: String,
    capture: Aggregate,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Aggregate {
    schema_version: u32,
    sample_rate: u32,
    channels: usize,
    capacity_frames: u64,
    packets: u64,
    observed_frames: u64,
    stored_frames: u64,
    dropped_frames: u64,
    silent_frames: u64,
    overflow_events: u64,
    detect_position_gaps: bool,
    gap_events: u64,
    gap_frames: u64,
    discontinuity_events: u64,
    timestamp_error_packets: u64,
    packet_history: History,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct History {
    capacity_packets: u64,
    stored_packets: u64,
    omitted_packets: u64,
    omitted_frames: u64,
    counters_saturated: bool,
    complete: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    packet_sequence: u64,
    observed_frame_start: u64,
    stored_frame_start: u64,
    frames: u32,
    stored_frames: u32,
    flags: u32,
    device_position_frames: u64,
    // A missing field is corruption; an explicit null is retained unknown time.
    #[serde(deserialize_with = "nullable_stamp")]
    qpc_100ns: Option<u64>,
}

fn nullable_stamp<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(deserializer)
}

impl Record {
    pub(super) fn stored_frames(self) -> u32 {
        self.stored_frames
    }

    pub(super) fn packet(self) -> PacketMetadata {
        // This projection is private diagnostic arithmetic, never serialized as
        // a capture record. The raw nullable stamp is retained separately.
        PacketMetadata {
            packet_sequence: self.packet_sequence,
            flags: self.flags,
            device_position_frames: self.device_position_frames,
            qpc_100ns: self.qpc_100ns.unwrap_or(0),
            frames: self.frames,
            stored_start_frame: self.stored_frame_start,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Evidence {
    scope: &'static str,
    header: Header,
    issues: Vec<&'static str>,
}

pub(super) struct State {
    header: Header,
    packets: u64,
    observed: u64,
    stored: u64,
    silent: u64,
    overflows: u64,
    discontinuities: u64,
    timestamp_errors: u64,
    next_position: Option<u64>,
    gap_events: u64,
    gap_frames: u64,
}

impl State {
    pub(super) fn new(header: Header, frames: u64, sample_rate: u32) -> Result<Self> {
        ensure!(
            header.schema_version == 1 && header.format == FORMAT,
            "unsupported capture packet history format/version"
        );
        let capture = &header.capture;
        let history = &capture.packet_history;
        ensure!(
            capture.schema_version == 3
                && capture.sample_rate == sample_rate
                && capture.channels == 2,
            "capture history format differs from stereo PCM analysis"
        );
        ensure!(
            capture.capacity_frames > 0
                && capture.stored_frames == frames
                && frames <= capture.capacity_frames,
            "capture history stored PCM length/capacity mismatch"
        );
        ensure!(
            (1..=65_536).contains(&history.capacity_packets)
                && history.stored_packets <= history.capacity_packets,
            "invalid capture packet history capacity/count"
        );
        ensure!(
            history.stored_packets.checked_add(history.omitted_packets) == Some(capture.packets),
            "capture history packet totals mismatch"
        );
        ensure!(
            capture.stored_frames.checked_add(capture.dropped_frames)
                == Some(capture.observed_frames),
            "capture history offered/stored/dropped totals mismatch"
        );
        ensure!(
            history.complete == (history.omitted_packets == 0 && !history.counters_saturated),
            "capture history completeness contradicts loss/saturation"
        );
        ensure!(
            (history.omitted_packets == 0) == (history.omitted_frames == 0),
            "capture history omitted packet/frame totals mismatch"
        );
        ensure!(
            history.omitted_frames >= history.omitted_packets
                && u128::from(history.omitted_frames)
                    <= u128::from(history.omitted_packets) * u128::from(u32::MAX)
                && (history.omitted_packets == 0
                    || history.stored_packets == history.capacity_packets),
            "capture history omission is not a full retained prefix"
        );
        ensure!(
            capture.silent_frames <= capture.observed_frames
                && capture.overflow_events <= capture.packets
                && capture.discontinuity_events <= capture.packets
                && capture.timestamp_error_packets <= capture.packets,
            "capture history aggregate counters exceed offers"
        );
        ensure!(
            capture.detect_position_gaps || (capture.gap_events == 0 && capture.gap_frames == 0),
            "capture history positionless source declares position gaps"
        );
        Ok(Self {
            header,
            packets: 0,
            observed: 0,
            stored: 0,
            silent: 0,
            overflows: 0,
            discontinuities: 0,
            timestamp_errors: 0,
            next_position: None,
            gap_events: 0,
            gap_frames: 0,
        })
    }

    pub(super) fn accept(&mut self, record: Record) -> Result<()> {
        let capture = &self.header.capture;
        ensure!(
            self.packets < capture.packet_history.stored_packets,
            "extra capture history packet"
        );
        ensure!(
            record.observed_frame_start == self.observed
                && record.stored_frame_start == self.stored,
            "capture history observed/stored frontier mismatch"
        );
        ensure!(
            record.frames > 0 && record.packet_sequence > 0,
            "capture history packet must be nonempty with nonzero sequence"
        );
        let expected =
            u64::from(record.frames).min(capture.capacity_frames.saturating_sub(self.stored));
        ensure!(
            u64::from(record.stored_frames) == expected,
            "capture history stored offer differs from bounded PCM storage"
        );
        // At most 65536 u32-sized records are admitted, so prefix arithmetic
        // cannot saturate. The header still preserves whole-offer saturation.
        self.packets += 1;
        self.observed += u64::from(record.frames);
        self.stored += u64::from(record.stored_frames);
        self.silent += u64::from(record.frames) * u64::from(record.flags & SILENT != 0);
        self.overflows += u64::from(record.stored_frames < record.frames);
        self.discontinuities += u64::from(record.flags & DISCONTINUITY != 0);
        self.timestamp_errors += u64::from(record.flags & TIMESTAMP_ERROR != 0);
        if record.flags & TIMESTAMP_ERROR != 0 {
            self.next_position = None;
        } else if capture.detect_position_gaps {
            if let Some(expected) = self.next_position
                && expected != record.device_position_frames
            {
                self.gap_events += 1;
                self.gap_frames = self
                    .gap_frames
                    .saturating_add(expected.abs_diff(record.device_position_frames));
            }
            self.next_position = record
                .device_position_frames
                .checked_add(u64::from(record.frames));
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<(Evidence, bool)> {
        let capture = &self.header.capture;
        let history = &capture.packet_history;
        ensure!(
            self.packets == history.stored_packets,
            "truncated capture packet history"
        );
        ensure!(
            self.observed.checked_add(history.omitted_frames) == Some(capture.observed_frames),
            "capture history observed frame total mismatch"
        );
        // PCM remains a contiguous bounded prefix even after packet history
        // fills. The omitted suffix cannot secretly stop or restart storage.
        let suffix_stored = history
            .omitted_frames
            .min(capture.capacity_frames.saturating_sub(self.stored));
        ensure!(
            self.stored.checked_add(suffix_stored) == Some(capture.stored_frames),
            "capture history omitted suffix contradicts stored PCM frontier"
        );
        if history.omitted_packets == 0 {
            ensure!(
                self.stored == capture.stored_frames
                    && self.silent == capture.silent_frames
                    && self.overflows == capture.overflow_events
                    && self.discontinuities == capture.discontinuity_events
                    && self.timestamp_errors == capture.timestamp_error_packets
                    && self.gap_events == capture.gap_events
                    && self.gap_frames == capture.gap_frames,
                "capture history aggregate counters differ from retained offers"
            );
        } else {
            let suffix_overflows = capture
                .overflow_events
                .checked_sub(self.overflows)
                .context("capture history overflow count below retained prefix")?;
            ensure!(
                suffix_overflows_possible(
                    history.omitted_packets,
                    history.omitted_frames,
                    capture.capacity_frames.saturating_sub(self.stored),
                    suffix_overflows,
                ),
                "capture history omitted overflow count contradicts bounded storage"
            );
            ensure!(
                capture
                    .silent_frames
                    .checked_sub(self.silent)
                    .is_some_and(|count| count <= history.omitted_frames)
                    && capture
                        .overflow_events
                        .checked_sub(self.overflows)
                        .is_some_and(|count| count <= history.omitted_packets)
                    && capture
                        .discontinuity_events
                        .checked_sub(self.discontinuities)
                        .is_some_and(|count| count <= history.omitted_packets)
                    && capture
                        .timestamp_error_packets
                        .checked_sub(self.timestamp_errors)
                        .is_some_and(|count| count <= history.omitted_packets)
                    && capture
                        .gap_events
                        .checked_sub(self.gap_events)
                        .is_some_and(|count| count <= history.omitted_packets)
                    && capture.gap_frames >= self.gap_frames,
                "capture history omitted suffix cannot explain aggregate counters"
            );
        }
        let mut issues = Vec::new();
        if !history.complete {
            issues.push("native_capture_packet_history_incomplete");
        }
        if history.counters_saturated {
            issues.push("native_capture_counters_saturated");
        }
        if capture.dropped_frames != 0 || capture.overflow_events != 0 {
            issues.push("native_capture_pcm_storage_incomplete");
        }
        if capture.gap_events != 0 || capture.gap_frames != 0 {
            issues.push("native_capture_position_gaps");
        }
        let complete = issues.is_empty() && self.stored == capture.stored_frames;
        Ok((
            Evidence {
                scope: "native_getbuffer_offers_before_release_not_capture_completion_or_physical_time",
                header: self.header,
                issues,
            },
            complete,
        ))
    }
}

fn suffix_overflows_possible(packets: u64, frames: u64, remaining: u64, overflows: u64) -> bool {
    if frames <= remaining {
        return overflows == 0;
    }
    if overflows == 0 || overflows > packets {
        return false;
    }
    // Once an offer crosses the capacity frontier, all following offers are
    // overflowing. Find whether the first such offer can store a valid prefix
    // while every omitted packet stays nonempty and u32-sized. u128 keeps
    // adversarial aggregate counts from overflowing the feasibility arithmetic.
    let (remaining, lost, count, clean, limit) = (
        u128::from(remaining),
        u128::from(frames - remaining),
        u128::from(overflows),
        u128::from(packets - overflows),
        u128::from(u32::MAX),
    );
    if clean > remaining || lost < count || lost > count * limit {
        return false;
    }
    let first_stored_min = remaining.saturating_sub(clean * limit);
    let first_stored_max = (remaining - clean).min(limit - 1).min(count * limit - lost);
    first_stored_min <= first_stored_max
}
