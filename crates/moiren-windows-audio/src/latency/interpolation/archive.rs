//! Bounded control-side cursor archive. Drained ownership is not persisted IO.
use super::{CAPACITY, InterpolationEvent, InterpolationHistorySnapshot};
use serde::Serialize;

pub const MAX_TRACE_EVENTS: usize = 131_072;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterpolationTraceRefusal {
    MissingEvents,
    InvalidHistory,
    Saturated,
    ArchiveLimit,
    OpenWriter,
    IncompleteRead,
    NoReads,
}

#[derive(Debug, Serialize)]
pub struct InterpolationTraceChunk {
    pub schema_version: u32,
    pub sequence: u64,
    pub events: Vec<InterpolationEvent>,
}

/// Structural history only, never authenticated native provenance, successful
/// persistence, uninterrupted audio, OBS correspondence or physical latency.
#[derive(Debug, Serialize)]
pub struct InterpolationTrace {
    pub schema_version: u32,
    pub event_limit: usize,
    pub retained_events: usize,
    pub drained_events: u64,
    pub drained_chunks: u64,
    pub total_events: u64,
    pub missing_events: u64,
    pub unretained_events: u64,
    pub requested_frames: u64,
    pub started_reads: u64,
    pub completed_reads: u64,
    pub writer_finished: bool,
    pub finalized: bool,
    pub history_complete: bool,
    pub qualified: bool,
    pub refusal: Option<InterpolationTraceRefusal>,
    pub events: Vec<InterpolationEvent>,
}

pub struct InterpolationTraceCollector {
    artifact: InterpolationTrace,
    overlap: Vec<InterpolationEvent>,
    previous: Option<InterpolationHistorySnapshot>,
}

impl InterpolationTraceCollector {
    pub fn new(limit: usize) -> Result<Self, &'static str> {
        if !(1..=MAX_TRACE_EVENTS).contains(&limit) {
            return Err("interpolation trace limit must be 1..131072 events");
        }
        Ok(Self {
            artifact: InterpolationTrace {
                schema_version: 1,
                event_limit: limit,
                retained_events: 0,
                drained_events: 0,
                drained_chunks: 0,
                total_events: 0,
                missing_events: 0,
                unretained_events: 0,
                requested_frames: 0,
                started_reads: 0,
                completed_reads: 0,
                writer_finished: false,
                finalized: false,
                history_complete: false,
                qualified: false,
                refusal: None,
                events: Vec::new(),
            },
            overlap: Vec::new(),
            previous: None,
        })
    }

    /// Editable diagnostic input is validated, not promoted to native proof.
    /// A transient unavailable sample should be retried by the control caller;
    /// a malformed sample or actual lost sequence permanently refuses closure.
    pub fn observe(&mut self, history: InterpolationHistorySnapshot) {
        if self.artifact.finalized {
            return;
        }
        if !valid(&history, self.previous.as_ref()) {
            self.refuse(InterpolationTraceRefusal::InvalidHistory);
            return;
        }
        if history.saturated {
            self.refuse(InterpolationTraceRefusal::Saturated);
        }
        // Preflight overlap before mutating the retained prefix. Keep at most
        // one native window even after callers take earlier chunks away.
        for point in &history.events {
            if point.sequence <= self.artifact.total_events {
                let old = [&self.artifact.events[..], &self.overlap[..]]
                    .into_iter()
                    .find_map(|events| {
                        events
                            .binary_search_by_key(&point.sequence, |event| event.sequence)
                            .ok()
                            .map(|index| &events[index])
                    });
                if old.is_some_and(|old| *old != *point) {
                    self.refuse(InterpolationTraceRefusal::InvalidHistory);
                    return;
                }
            }
        }
        let mut last = self.artifact.total_events;
        for point in &history.events {
            if point.sequence <= last {
                continue;
            }
            if point.sequence - last > 1 {
                self.artifact.missing_events += point.sequence - last - 1;
                self.refuse(InterpolationTraceRefusal::MissingEvents);
            }
            last = point.sequence;
            if self.artifact.events.len() < self.artifact.event_limit {
                self.artifact.events.push(*point);
            } else {
                self.artifact.unretained_events += 1;
                self.refuse(InterpolationTraceRefusal::ArchiveLimit);
            }
        }
        self.artifact.total_events = history.total_events;
        self.artifact.retained_events = self.artifact.events.len();
        self.artifact.requested_frames = history.requested_frames;
        self.artifact.started_reads = history.started_reads;
        self.artifact.completed_reads = history.completed_reads;
        self.artifact.writer_finished = history.writer_finished;
        self.previous = Some(history);
    }

    fn refuse(&mut self, refusal: InterpolationTraceRefusal) {
        self.artifact.refusal.get_or_insert(refusal);
        self.artifact.history_complete = false;
    }

    pub fn artifact(&self) -> &InterpolationTrace {
        &self.artifact
    }

    /// Call only after the actual source is joined/dropped. The snapshot's
    /// writer marker remains authoritative; this method cannot manufacture it.
    pub fn finish(&mut self) {
        if self.artifact.finalized {
            return;
        }
        if !self.artifact.writer_finished {
            self.refuse(InterpolationTraceRefusal::OpenWriter);
        } else if self.artifact.started_reads != self.artifact.completed_reads {
            self.refuse(InterpolationTraceRefusal::IncompleteRead);
        } else if self.artifact.started_reads == 0 {
            self.refuse(InterpolationTraceRefusal::NoReads);
        }
        self.artifact.finalized = true;
        self.artifact.history_complete = self.artifact.refusal.is_none()
            && self.artifact.missing_events == 0
            && self.artifact.unretained_events == 0
            && self.artifact.drained_events + self.artifact.events.len() as u64
                == self.artifact.total_events;
    }

    /// Move one pending chunk to control. Callers must retain it until durable
    /// persistence succeeds; the collector has no IO acknowledgment authority.
    pub fn drain_chunk(&mut self) -> Option<InterpolationTraceChunk> {
        if self.artifact.events.is_empty() {
            return None;
        }
        let Some(sequence) = self.artifact.drained_chunks.checked_add(1) else {
            self.refuse(InterpolationTraceRefusal::InvalidHistory);
            return None;
        };
        let events = std::mem::take(&mut self.artifact.events);
        let start = events.len().saturating_sub(CAPACITY);
        if start == 0 {
            let excess = (self.overlap.len() + events.len()).saturating_sub(CAPACITY);
            self.overlap.drain(..excess);
            self.overlap.extend_from_slice(&events);
        } else {
            self.overlap = events[start..].to_vec();
        }
        self.artifact.drained_events += events.len() as u64;
        self.artifact.drained_chunks = sequence;
        self.artifact.retained_events = 0;
        Some(InterpolationTraceChunk {
            schema_version: 1,
            sequence,
            events,
        })
    }
}

fn valid(
    history: &InterpolationHistorySnapshot,
    previous: Option<&InterpolationHistorySnapshot>,
) -> bool {
    let total = history.total_events;
    let count = total.min(CAPACITY as u64);
    if history.events.len() as u64 != count
        || history.overwritten_events != total - count
        || history.completed_reads > history.started_reads
        || history.started_reads - history.completed_reads > 1
        || history.requested_frames < history.started_reads
    {
        return false;
    }
    if let Some(previous) = previous
        && (total < previous.total_events
            || history.requested_frames < previous.requested_frames
            || history.started_reads < previous.started_reads
            || history.completed_reads < previous.completed_reads
            || (previous.saturated && !history.saturated)
            || (previous.writer_finished && history != previous))
    {
        return false;
    }
    if let Some(previous) = previous {
        let reads = history.started_reads - previous.started_reads;
        let frames = history.requested_frames - previous.requested_frames;
        if frames < reads || (reads == 0 && frames != 0) {
            return false;
        }
    }
    for (index, event) in history.events.iter().enumerate() {
        let span = event.span;
        let end = span.output_start_frame.checked_add(u64::from(span.frames));
        if previous.is_some_and(|previous| {
            event.sequence > previous.total_events
                && (span.read_sequence <= previous.completed_reads
                    || (span.read_sequence <= previous.started_reads
                        && end.is_none_or(|end| end > previous.requested_frames))
                    || (span.read_sequence > previous.started_reads
                        && span.output_start_frame < previous.requested_frames))
        }) {
            return false;
        }
        if event.sequence != total - count + index as u64 + 1
            || span.read_sequence == 0
            || span.read_sequence > history.started_reads
            || !(1..=32).contains(&span.frames)
            || !span.phase.is_finite()
            || !(0.0..1.0).contains(&span.phase)
            || !span.step.is_finite()
            || span.step <= 0.0
            || span.fade_remaining > 32
            || end.is_none_or(|end| end > history.requested_frames)
        {
            return false;
        }
        let old = if index > 0 {
            Some(history.events[index - 1])
        } else {
            previous
                .and_then(|previous| previous.events.last())
                .copied()
                .filter(|old| old.sequence.checked_add(1) == Some(event.sequence))
        };
        if let Some(old) = old {
            let old = old.span;
            let last_end = old.output_start_frame + u64::from(old.frames);
            if span.read_sequence < old.read_sequence
                || span.output_start_frame < last_end
                || (span.read_sequence == old.read_sequence && span.output_start_frame != last_end)
            {
                return false;
            }
        }
    }
    true
}
