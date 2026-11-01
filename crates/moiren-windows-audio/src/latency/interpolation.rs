//! Actual linear-SRC cursor spans, not an estimated affine clock or sample age.
//! Accepted-input and requested-output indices are local to one bridge owner.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering::SeqCst},
};

const CAPACITY: usize = 256;
const FIELDS: usize = 9;
pub(crate) const QUANTUM_FRAMES: usize = 32;

/// One actually emitted constant-step span. Replay normalizes phase before
/// each sample, interpolates finite f32 input in f64, applies the 32-frame
/// recovery fade, then casts to f32. No source/device timestamp is implied.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpolationSpan {
    /// Sequence of the nonempty source read, including reads that emit nothing.
    pub read_sequence: u64,
    /// Requested-output timeline, including priming and unfilled read tails.
    pub output_start_frame: u64,
    /// Accepted-input timeline; drops before ring publication are excluded.
    /// Join packet accepted frontiers separately to recover offered positions.
    pub input_start_frame: u64,
    pub frames: u32,
    pub generation: u64,
    #[serde(rename = "phase_bits", with = "f64_bits")]
    pub phase: f64,
    #[serde(rename = "step_bits", with = "f64_bits")]
    pub step: f64,
    pub fade_remaining: u32,
}

// Some JSON consumers round decimal f64 input by one ULP. Carry exact IEEE
// bits as decimal strings (also safe past JavaScript's 53-bit integer range).
// Consumers may derive display decimals, but must replay these exact bits.
mod f64_bits {
    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&value.to_bits())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        let value = String::deserialize(deserializer)?;
        let bits: u64 = value.parse().map_err(de::Error::custom)?;
        let value = f64::from_bits(bits);
        if !value.is_finite() {
            return Err(de::Error::custom(
                "interpolation bits must represent a finite f64",
            ));
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpolationEvent {
    pub sequence: u64,
    pub span: InterpolationSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpolationHistorySnapshot {
    #[serde(deserialize_with = "super::bounded_history::deserialize_events")]
    pub events: Vec<InterpolationEvent>,
    pub total_events: u64,
    pub overwritten_events: u64,
    /// Full requested timeline, including entirely silent/failed reads.
    pub requested_frames: u64,
    pub started_reads: u64,
    /// Less than started_reads while one source read is in flight.
    pub completed_reads: u64,
    pub saturated: bool,
    /// Unique source writer dropped; not full native transport cleanup.
    pub writer_finished: bool,
}

struct PublishedHistory {
    version: AtomicU64,
    total: AtomicU64,
    saturated: AtomicU64,
    finished: AtomicU64,
    requested: AtomicU64,
    started: AtomicU64,
    completed: AtomicU64,
    events: [[AtomicU64; FIELDS]; CAPACITY],
}

pub(crate) fn storage_bytes() -> usize {
    size_of::<PublishedHistory>()
}

pub(crate) fn history() -> (InterpolationHistoryWriter, InterpolationHistoryObserver) {
    let shared = Arc::new(PublishedHistory {
        version: AtomicU64::new(0),
        total: AtomicU64::new(0),
        saturated: AtomicU64::new(0),
        finished: AtomicU64::new(0),
        requested: AtomicU64::new(0),
        started: AtomicU64::new(0),
        completed: AtomicU64::new(0),
        events: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
    });
    (
        InterpolationHistoryWriter {
            shared: shared.clone(),
            total: 0,
            last: None,
            requested: 0,
            started: 0,
        },
        InterpolationHistoryObserver(shared),
    )
}

pub(crate) struct InterpolationHistoryWriter {
    shared: Arc<PublishedHistory>,
    total: u64,
    last: Option<InterpolationSpan>,
    requested: u64,
    started: u64,
}

impl InterpolationHistoryWriter {
    pub(crate) fn begin_read(&mut self, frames: usize) -> Option<(u64, u64)> {
        if self.shared.saturated.load(SeqCst) != 0 {
            return None;
        }
        let version = self.shared.version.load(SeqCst);
        let (Some(after), Some(requested), Some(started)) = (
            version.checked_add(2),
            self.requested.checked_add(frames as u64),
            self.started.checked_add(1),
        ) else {
            self.mark_saturated();
            return None;
        };
        self.shared.version.store(version + 1, SeqCst);
        self.shared.requested.store(requested, SeqCst);
        self.shared.started.store(started, SeqCst);
        self.shared.version.store(after, SeqCst);
        let start = self.requested;
        self.requested = requested;
        self.started = started;
        Some((started, start))
    }

    pub(crate) fn finish_read(&mut self) {
        let version = self.shared.version.load(SeqCst);
        let Some(after) = version.checked_add(2) else {
            self.mark_saturated();
            return;
        };
        self.shared.version.store(version + 1, SeqCst);
        self.shared.completed.store(self.started, SeqCst);
        self.shared.version.store(after, SeqCst);
    }

    pub(crate) fn mark_saturated(&self) {
        self.shared.saturated.store(1, SeqCst);
    }

    /// Single-writer, fixed-size scalar publication once per emitted span.
    /// Invalid/overflowing diagnostics stop recording, never stop audio.
    pub(crate) fn record(&mut self, span: InterpolationSpan) {
        if self.shared.saturated.load(SeqCst) != 0 {
            return;
        }
        let end = span.output_start_frame.checked_add(u64::from(span.frames));
        let ordered = self.last.is_none_or(|last| {
            let last_end = last.output_start_frame + u64::from(last.frames);
            span.read_sequence >= last.read_sequence
                && span.output_start_frame >= last_end
                && (span.read_sequence != last.read_sequence || span.output_start_frame == last_end)
        });
        let version = self.shared.version.load(SeqCst);
        let (Some(after), Some(total)) = (version.checked_add(2), self.total.checked_add(1)) else {
            self.mark_saturated();
            return;
        };
        if span.read_sequence == 0
            || !(1..=QUANTUM_FRAMES as u32).contains(&span.frames)
            || !span.phase.is_finite()
            || !(0.0..1.0).contains(&span.phase)
            || !span.step.is_finite()
            || span.step <= 0.0
            || span.fade_remaining > 32
            || end.is_none()
            || !ordered
        {
            self.mark_saturated();
            return;
        }
        self.shared.version.store(version + 1, SeqCst);
        // The sequence is implicit in the stable total/index. Every payload
        // word is atomic, avoiding seqlock races on f64 or non-atomic structs.
        let fields = [
            span.read_sequence,
            span.output_start_frame,
            span.input_start_frame,
            u64::from(span.frames),
            span.generation,
            span.phase.to_bits(),
            span.step.to_bits(),
            u64::from(span.fade_remaining),
            total,
        ];
        for (slot, value) in self.shared.events[((total - 1) % CAPACITY as u64) as usize]
            .iter()
            .zip(fields)
        {
            slot.store(value, SeqCst);
        }
        self.shared.total.store(total, SeqCst);
        self.shared.version.store(after, SeqCst);
        self.total = total;
        self.last = Some(span);
    }
}

impl Drop for InterpolationHistoryWriter {
    fn drop(&mut self) {
        self.shared.finished.store(1, SeqCst);
    }
}

#[derive(Clone)]
pub(crate) struct InterpolationHistoryObserver(Arc<PublishedHistory>);

impl InterpolationHistoryObserver {
    /// Control-side owned suffix. One attempt; contention never yields a stale
    /// success or retries on the audio owner. Allocation stays on control.
    pub(crate) fn snapshot(&self) -> Option<InterpolationHistorySnapshot> {
        let shared = &self.0;
        let before = shared.version.load(SeqCst);
        if !before.is_multiple_of(2) {
            return None;
        }
        let writer_finished = shared.finished.load(SeqCst) != 0;
        let saturated = shared.saturated.load(SeqCst) != 0;
        let total = shared.total.load(SeqCst);
        let requested_frames = shared.requested.load(SeqCst);
        let started_reads = shared.started.load(SeqCst);
        let completed_reads = shared.completed.load(SeqCst);
        let count = total.min(CAPACITY as u64);
        let mut events = Vec::with_capacity(count as usize);
        for sequence in total - count + 1..=total {
            let f: [u64; FIELDS] = std::array::from_fn(|index| {
                shared.events[((sequence - 1) % CAPACITY as u64) as usize][index].load(SeqCst)
            });
            events.push(InterpolationEvent {
                sequence: f[8],
                span: InterpolationSpan {
                    read_sequence: f[0],
                    output_start_frame: f[1],
                    input_start_frame: f[2],
                    frames: f[3] as u32,
                    generation: f[4],
                    phase: f64::from_bits(f[5]),
                    step: f64::from_bits(f[6]),
                    fade_remaining: f[7] as u32,
                },
            });
        }
        (before == shared.version.load(SeqCst)).then_some(InterpolationHistorySnapshot {
            events,
            total_events: total,
            overwritten_events: total - count,
            requested_frames,
            started_reads,
            completed_reads,
            saturated,
            writer_finished,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(read_sequence: u64, output_start_frame: u64) -> InterpolationSpan {
        InterpolationSpan {
            read_sequence,
            output_start_frame,
            input_start_frame: output_start_frame,
            frames: 1,
            generation: 1,
            phase: 0.25,
            step: 1.002,
            fade_remaining: 0,
        }
    }

    #[test]
    fn counter_or_version_exhaustion_invalidates_evidence_without_wrapping() {
        for field in 0..4 {
            let (mut writer, observer) = history();
            match field {
                0 => writer.requested = u64::MAX,
                1 => writer.started = u64::MAX,
                2 => writer.total = u64::MAX,
                _ => writer.shared.version.store(u64::MAX - 1, SeqCst),
            }
            if field == 2 {
                writer.record(span(1, 0));
            } else {
                assert!(writer.begin_read(1).is_none());
            }
            let snapshot = observer.snapshot().unwrap();
            assert!(snapshot.saturated);
            assert!(snapshot.events.is_empty());
            assert_eq!(snapshot.total_events, 0);
        }
    }

    #[test]
    fn invalid_phase_or_disordered_spans_preserve_the_valid_prefix() {
        for field in 0..7 {
            let (mut writer, observer) = history();
            writer.begin_read(8).unwrap();
            writer.record(span(1, 0));
            let mut bad = span(1, 1);
            match field {
                0 => bad.phase = f64::NAN,
                1 => bad.phase = 1.0,
                2 => bad.step = f64::INFINITY,
                3 => bad.frames = 33,
                4 => bad.fade_remaining = 33,
                5 => bad.output_start_frame = 0,
                _ => bad.output_start_frame = u64::MAX,
            }
            writer.record(bad);
            writer.finish_read();
            let snapshot = observer.snapshot().unwrap();
            assert!(snapshot.saturated);
            assert_eq!(snapshot.events.len(), 1);
            assert_eq!(snapshot.events[0].span, span(1, 0));
        }
    }

    #[test]
    fn concurrent_wrap_and_writer_drop_only_publish_coherent_owned_suffixes() {
        let (mut writer, observer) = history();
        std::thread::scope(|scope| {
            let task = scope.spawn(move || {
                for index in 0..4000 {
                    writer.begin_read(1).unwrap();
                    writer.record(span(index + 1, index));
                    writer.finish_read();
                }
            });
            while !task.is_finished() {
                if let Some(snapshot) = observer.snapshot() {
                    assert!(snapshot.completed_reads <= snapshot.started_reads);
                    assert!(snapshot.started_reads - snapshot.completed_reads <= 1);
                    assert_eq!(snapshot.requested_frames, snapshot.started_reads);
                    for event in &snapshot.events {
                        assert_eq!(event.sequence, event.span.read_sequence);
                        assert_eq!(event.span.output_start_frame + 1, event.sequence);
                        assert_eq!(event.span.phase, 0.25);
                        assert_eq!(event.span.step, 1.002);
                    }
                }
            }
            task.join().unwrap();
        });
        let snapshot = observer.snapshot().unwrap();
        assert!(snapshot.writer_finished);
        assert!(!snapshot.saturated);
        assert_eq!(snapshot.total_events, 4000);
        assert_eq!(snapshot.overwritten_events, 3744);
        assert_eq!(snapshot.completed_reads, 4000);
    }

    #[test]
    fn serialized_suffix_roundtrips_without_allowing_unbounded_event_import() {
        let (mut writer, observer) = history();
        writer.begin_read(1).unwrap();
        writer.record(span(1, 0));
        writer.finish_read();
        let original = observer.snapshot().unwrap();
        let mut value = serde_json::to_value(&original).unwrap();
        let restored: InterpolationHistorySnapshot = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(original, restored);
        value["events"] = serde_json::Value::Array(vec![value["events"][0].clone(); 257]);
        assert!(serde_json::from_value::<InterpolationHistorySnapshot>(value).is_err());
    }
}
