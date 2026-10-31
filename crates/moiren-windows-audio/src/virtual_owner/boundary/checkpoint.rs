//! Coherent bounded callback history; not a source clock or waveform verdict.
use crate::resample::SrcCursor;
use moiren_engine::{boundary::BoundaryReport, processor::ProcessContext};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};

const FIELDS: usize = 31;
pub const SRC_HISTORY_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BoundarySrcHistory {
    pub schema_version: u32,
    pub total_callbacks: u64,
    pub overwritten_callbacks: u64,
    /// Boundary attachment retired, not proof that native workers joined cleanly.
    pub writer_finished: bool,
    pub checkpoints: Vec<BoundarySrcCheckpoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct BoundarySrcCheckpoint {
    pub schema_version: u32,
    /// Increases for every nonempty callback, including prime/fence waits.
    /// A skipped sequence is missing evidence, never interpolated history.
    pub sequence: u64,
    pub timeline_epoch: u64,
    pub timeline_start: u64,
    pub engine_frames: u64,
    pub engine_sample_rate: u32,
    pub boundary_epoch_before: u64,
    pub boundary_epoch_after: u64,
    pub requested_epoch_after: u64,
    pub before: Option<SrcCursor>,
    pub after: Option<SrcCursor>,
    /// Exact step captured after the servo update, before moving any PCM.
    /// None when no SRC processing occurred, including reset/fence waits.
    pub processing_step: Option<f64>,
    /// Successful carrier frames, in the carrier's native rate. For playback
    /// this excludes silence-fill; for microphone it excludes drop-new output.
    pub carrier_frames: u64,
    /// Explicit startup/recovery edge mute only, in Engine output frames.
    /// Not all silence: starvation/invalid-layout/fence output needs the report
    /// and frontiers too; a nonzero xrun is never excused by this counter.
    pub muted_frames: u64,
    /// The returned Engine-rate report, not proof of non-silent native output.
    pub transferred_frames: u64,
    pub discontinuity: bool,
    pub xruns: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn point(index: u64) -> BoundarySrcCheckpoint {
        let mut point = BoundarySrcCheckpoint::begin(
            &ProcessContext {
                timeline_epoch: index,
                timeline_start: index * 32,
                frames: 32,
                processing_sr: 48_000.0,
            },
            32,
            48_000,
            index,
            None,
        );
        point.carrier_frames = index * 2;
        point.processing_step = Some(0.998);
        point.before = Some(SrcCursor {
            reset_generation: index,
            input_frames: index * 3,
            output_frames: index * 4,
            next_input_frame: (1 << 54) + index,
            fraction: 0.125,
            step: 1.002,
            saturated: false,
        });
        point.after = point.before.map(|mut cursor| {
            cursor.output_frames += 32;
            cursor.fraction = 0.875;
            cursor
        });
        point
    }

    #[test]
    fn checkpoint_unpublished_contention_exhaustion_and_reuse_are_unknown() {
        let shared = PublishedCheckpoint::default();
        shared.attach();
        assert_eq!(shared.snapshot(), None);
        shared.publish(point(1));
        assert_eq!(shared.snapshot().unwrap().sequence, 1);
        shared.version.store(3, SeqCst);
        assert_eq!(shared.snapshot(), None);
        shared.version.store(u64::MAX - 1, SeqCst);
        shared.publish(point(2));
        assert_eq!(shared.snapshot(), None);
        assert_eq!(shared.history(), None);
        let shared = PublishedCheckpoint::default();
        shared.attach();
        shared.publish(point(1));
        shared.attach();
        assert_eq!(shared.snapshot(), None);
        shared.publish(point(2));
        assert_eq!(shared.snapshot(), None);
        assert_eq!(shared.history(), None);
    }

    #[test]
    fn checkpoint_concurrent_samples_are_coherent_or_unknown_without_retry() {
        let shared = Arc::new(PublishedCheckpoint::default());
        shared.attach();
        let writer = shared.clone();
        let thread = std::thread::spawn(move || {
            for index in 1..=10_000 {
                writer.publish(point(index));
            }
        });
        while !thread.is_finished() {
            if let Some(point) = shared.snapshot() {
                assert_eq!(point.timeline_epoch, point.sequence);
                assert_eq!(point.timeline_start, point.sequence * 32);
                assert_eq!(point.carrier_frames, point.sequence * 2);
                assert_eq!(point.processing_step, Some(0.998));
                assert_eq!(point.engine_sample_rate, 48_000);
                assert_eq!(point.before, self::point(point.sequence).before);
                assert_eq!(point.after, self::point(point.sequence).after);
            }
        }
        thread.join().unwrap();
        assert_eq!(shared.snapshot().unwrap().sequence, 10_000);
    }

    #[test]
    fn checkpoint_history_preserves_callbacks_between_polls_and_explicit_overwrite() {
        let shared = PublishedCheckpoint::default();
        assert!(shared.history().is_none());
        shared.attach();
        let empty = shared.history().unwrap();
        assert!(empty.checkpoints.is_empty());
        assert!(!empty.writer_finished);
        for index in 1..=300 {
            shared.publish(point(index));
        }
        let history = shared.history().unwrap();
        assert_eq!(history.total_callbacks, 300);
        assert_eq!(history.overwritten_callbacks, 44);
        assert_eq!(history.checkpoints.len(), 256);
        for (index, checkpoint) in history.checkpoints.iter().enumerate() {
            let sequence = 45 + index as u64;
            assert_eq!(checkpoint.sequence, sequence);
            assert_eq!(checkpoint.before, point(sequence).before);
            assert_eq!(checkpoint.after, point(sequence).after);
        }
        assert_eq!(shared.snapshot(), history.checkpoints.last().copied());
        shared.finish();
        assert!(shared.history().unwrap().writer_finished);
        shared.version.store(601, SeqCst);
        assert!(shared.history().is_none());
        shared.version.store(600, SeqCst);
        shared.attach();
        assert!(shared.history().is_none());
    }

    #[test]
    fn checkpoint_concurrent_history_is_a_coherent_ordered_suffix_or_unknown() {
        let shared = Arc::new(PublishedCheckpoint::default());
        shared.attach();
        let writer = shared.clone();
        let thread = std::thread::spawn(move || {
            for index in 1..=10_000 {
                writer.publish(point(index));
            }
            writer.finish();
        });
        while !thread.is_finished() {
            if let Some(history) = shared.history() {
                for (index, checkpoint) in history.checkpoints.iter().enumerate() {
                    let sequence = history.overwritten_callbacks + index as u64 + 1;
                    assert_eq!(checkpoint.sequence, sequence);
                    assert_eq!(checkpoint.before, point(sequence).before);
                    assert_eq!(checkpoint.after, point(sequence).after);
                }
                assert_eq!(
                    history.total_callbacks,
                    history.overwritten_callbacks + history.checkpoints.len() as u64
                );
            }
        }
        thread.join().unwrap();
        let history = shared.history().unwrap();
        assert!(history.writer_finished);
        assert_eq!(history.total_callbacks, 10_000);
    }
}

#[derive(Default)]
pub(super) struct TransferFacts {
    pub step: Option<f64>,
    pub carrier_frames: usize,
    pub muted_frames: usize,
}

impl BoundarySrcCheckpoint {
    pub(super) fn begin(
        context: &ProcessContext,
        frames: usize,
        rate: u32,
        epoch: u64,
        cursor: Option<SrcCursor>,
    ) -> Self {
        Self {
            schema_version: 1,
            sequence: 0,
            timeline_epoch: context.timeline_epoch,
            timeline_start: context.timeline_start,
            engine_frames: frames as u64,
            engine_sample_rate: rate,
            boundary_epoch_before: epoch,
            boundary_epoch_after: epoch,
            requested_epoch_after: epoch,
            before: cursor,
            after: cursor,
            processing_step: None,
            carrier_frames: 0,
            muted_frames: 0,
            transferred_frames: 0,
            discontinuity: false,
            xruns: 0,
        }
    }

    pub(super) fn finish(
        &mut self,
        epoch: u64,
        requested: u64,
        cursor: Option<SrcCursor>,
        facts: TransferFacts,
        report: BoundaryReport,
    ) {
        self.boundary_epoch_after = epoch;
        self.requested_epoch_after = requested;
        self.after = cursor;
        self.processing_step = facts.step;
        self.carrier_frames = facts.carrier_frames as u64;
        self.muted_frames = facts.muted_frames as u64;
        self.transferred_frames = report.transferred_frames as u64;
        self.discontinuity = report.discontinuity;
        self.xruns = report.xruns;
    }
}

fn cursor_fields(cursor: Option<SrcCursor>) -> [u64; 8] {
    cursor.map_or([0; 8], |value| {
        [
            1,
            value.reset_generation,
            value.input_frames,
            value.output_frames,
            value.next_input_frame,
            value.fraction.to_bits(),
            value.step.to_bits(),
            u64::from(value.saturated),
        ]
    })
}

fn cursor_from(fields: &[u64]) -> Option<SrcCursor> {
    (fields[0] != 0).then(|| SrcCursor {
        reset_generation: fields[1],
        input_frames: fields[2],
        output_frames: fields[3],
        next_input_frame: fields[4],
        fraction: f64::from_bits(fields[5]),
        step: f64::from_bits(fields[6]),
        saturated: fields[7] != 0,
    })
}

#[derive(Debug)]
pub(super) struct PublishedCheckpoint {
    version: AtomicU64,
    attached: AtomicBool,
    shared_writer: AtomicBool,
    finished: AtomicBool,
    fields: Box<[[AtomicU64; FIELDS]]>,
}

impl Default for PublishedCheckpoint {
    fn default() -> Self {
        Self {
            version: AtomicU64::new(0),
            attached: AtomicBool::new(false),
            shared_writer: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            // Prepare directly on the heap, not a large callback/owner stack.
            fields: (0..SRC_HISTORY_CAPACITY)
                .map(|_| std::array::from_fn(|_| AtomicU64::new(0)))
                .collect(),
        }
    }
}

impl PublishedCheckpoint {
    /// Preparation only. A probe reused across boundaries loses SRC provenance
    /// permanently; it must not masquerade as one stream's single-writer data.
    pub(super) fn attach(&self) {
        if self.attached.swap(true, SeqCst) || self.finished.load(SeqCst) {
            self.shared_writer.store(true, SeqCst);
        }
    }

    /// Unique boundary writer; fixed atomic scalar work, no retry or heap work.
    pub(super) fn publish(&self, mut point: BoundarySrcCheckpoint) {
        if self.shared_writer.load(SeqCst) || self.finished.load(SeqCst) {
            return;
        }
        let before = self.version.load(SeqCst);
        let Some(after) = before.checked_add(2).filter(|_| before.is_multiple_of(2)) else {
            self.version.store(u64::MAX, SeqCst);
            return;
        };
        point.sequence = after / 2;
        self.version.store(before + 1, SeqCst);
        let mut fields = [0; FIELDS];
        fields[..9].copy_from_slice(&[
            point.sequence,
            point.timeline_epoch,
            point.timeline_start,
            point.engine_frames,
            point.boundary_epoch_before,
            point.boundary_epoch_after,
            point.requested_epoch_after,
            u64::from(point.processing_step.is_some()),
            point.processing_step.unwrap_or(0.0).to_bits(),
        ]);
        fields[9..17].copy_from_slice(&cursor_fields(point.before));
        fields[17..25].copy_from_slice(&cursor_fields(point.after));
        fields[25..30].copy_from_slice(&[
            point.carrier_frames,
            point.muted_frames,
            point.transferred_frames,
            u64::from(point.discontinuity),
            point.xruns,
        ]);
        fields[30] = u64::from(point.engine_sample_rate);
        let index = ((point.sequence - 1) % SRC_HISTORY_CAPACITY as u64) as usize;
        for (slot, field) in self.fields[index].iter().zip(fields) {
            slot.store(field, SeqCst);
        }
        self.version.store(after, SeqCst);
    }

    /// One-attempt atomic-payload seqlock: unknown before first publication,
    /// on contention/exhaustion, or after a second boundary attached this probe.
    pub(super) fn snapshot(&self) -> Option<BoundarySrcCheckpoint> {
        let before = self.version.load(SeqCst);
        if before == 0 || !before.is_multiple_of(2) || self.shared_writer.load(SeqCst) {
            return None;
        }
        let index = ((before / 2 - 1) % SRC_HISTORY_CAPACITY as u64) as usize;
        let f = self.load_slot(index);
        (before == self.version.load(SeqCst) && !self.shared_writer.load(SeqCst)).then(|| decode(f))
    }

    pub(super) fn finish(&self) {
        self.finished.store(true, SeqCst);
    }

    /// Control only: allocate one bounded copy, or return unknown without retry.
    pub(super) fn history(&self) -> Option<BoundarySrcHistory> {
        let before = self.version.load(SeqCst);
        if !before.is_multiple_of(2)
            || !self.attached.load(SeqCst)
            || self.shared_writer.load(SeqCst)
        {
            return None;
        }
        let total = before / 2;
        let count = total.min(SRC_HISTORY_CAPACITY as u64);
        let mut checkpoints = Vec::with_capacity(count as usize);
        for sequence in total - count + 1..=total {
            let index = ((sequence - 1) % SRC_HISTORY_CAPACITY as u64) as usize;
            checkpoints.push(decode(self.load_slot(index)));
        }
        let writer_finished = self.finished.load(SeqCst);
        (before == self.version.load(SeqCst) && !self.shared_writer.load(SeqCst)).then_some(
            BoundarySrcHistory {
                schema_version: 1,
                total_callbacks: total,
                overwritten_callbacks: total - count,
                writer_finished,
                checkpoints,
            },
        )
    }

    fn load_slot(&self, index: usize) -> [u64; FIELDS] {
        std::array::from_fn(|i| self.fields[index][i].load(SeqCst))
    }
}

fn decode(f: [u64; FIELDS]) -> BoundarySrcCheckpoint {
    BoundarySrcCheckpoint {
        schema_version: 1,
        sequence: f[0],
        timeline_epoch: f[1],
        timeline_start: f[2],
        engine_frames: f[3],
        engine_sample_rate: f[30] as u32,
        boundary_epoch_before: f[4],
        boundary_epoch_after: f[5],
        requested_epoch_after: f[6],
        processing_step: (f[7] != 0).then(|| f64::from_bits(f[8])),
        before: cursor_from(&f[9..17]),
        after: cursor_from(&f[17..25]),
        carrier_frames: f[25],
        muted_frames: f[26],
        transferred_frames: f[27],
        discontinuity: f[28] != 0,
        xruns: f[29],
    }
}
