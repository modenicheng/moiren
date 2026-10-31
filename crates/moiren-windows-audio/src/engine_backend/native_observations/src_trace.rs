//! Pinned, bounded control archive of actual virtual-boundary callbacks.
use super::{NativeObservationReadHandle, NativeStreamKind, NodeId, serialize_node};
use crate::virtual_owner::{BoundarySrcCheckpoint, BoundarySrcHistory, SRC_HISTORY_CAPACITY};
use serde::Serialize;

pub const SRC_TRACE_MAX_CHECKPOINTS: usize = 131_072;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SrcTraceRefusal {
    MissingCallbacks,
    ArchiveLimit,
    HistoryUnavailable,
    OpenWriter,
    InvalidHistory,
    NoCallbacks,
}

#[derive(Debug, Serialize)]
pub struct NativeSrcBoundaryTrace {
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    pub kind: NativeStreamKind,
    pub total_callbacks: u64,
    pub missing_callbacks: u64,
    pub unretained_callbacks: u64,
    pub unavailable_polls: u64,
    pub writer_finished: bool,
    pub drained_checkpoints: u64,
    pub checkpoints: Vec<BoundarySrcCheckpoint>,
}

#[derive(Debug, Serialize)]
pub struct NativeSrcChunkBoundary {
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    pub kind: NativeStreamKind,
    pub checkpoints: Vec<BoundarySrcCheckpoint>,
}

#[derive(Debug, Serialize)]
pub struct NativeSrcTraceChunk {
    pub schema_version: u32,
    pub owner_generation: u64,
    pub sequence: u64,
    pub boundaries: Vec<NativeSrcChunkBoundary>,
}

#[derive(Debug, Serialize)]
pub struct NativeSrcTrace {
    pub schema_version: u32,
    pub owner_generation: u64,
    pub checkpoint_limit: usize,
    pub retained_checkpoints: usize,
    /// Ownership handed to the control caller, not proof of persistence.
    pub drained_checkpoints: u64,
    pub drained_chunks: u64,
    pub finalized: bool,
    /// Sequence-one through the final boundary callback, not clean audio,
    /// native cleanup, a common source clock or physical qualification.
    pub history_complete: bool,
    pub qualified: bool,
    pub refusal: Option<SrcTraceRefusal>,
    pub boundaries: Vec<NativeSrcBoundaryTrace>,
}

pub struct NativeSrcTraceCollector {
    owner: NativeObservationReadHandle,
    artifact: NativeSrcTrace,
    // Keep bounded overlap after draining, so a changed suffix cannot replace
    // earlier facts. This is control storage, separate from pending chunks.
    overlap: Vec<Vec<BoundarySrcCheckpoint>>,
}

impl NativeObservationReadHandle {
    /// Pin this exact session; later registry replacement cannot redirect it.
    /// The limit is shared by all boundaries, not multiplied by node count.
    pub fn src_trace_collector(
        &self,
        limit: usize,
    ) -> Result<NativeSrcTraceCollector, &'static str> {
        if !(1..=SRC_TRACE_MAX_CHECKPOINTS).contains(&limit) {
            return Err("SRC trace limit must be 1..131072 checkpoints");
        }
        if self.0.probes.is_empty() {
            return Err("SRC trace requires virtual boundary probes");
        }
        Ok(NativeSrcTraceCollector {
            owner: self.clone(),
            artifact: NativeSrcTrace {
                schema_version: 2,
                owner_generation: self.0.generation,
                checkpoint_limit: limit,
                retained_checkpoints: 0,
                drained_checkpoints: 0,
                drained_chunks: 0,
                finalized: false,
                history_complete: false,
                qualified: false,
                refusal: None,
                boundaries: self
                    .0
                    .probes
                    .iter()
                    .map(|(node, kind, _)| NativeSrcBoundaryTrace {
                        node: *node,
                        kind: *kind,
                        total_callbacks: 0,
                        missing_callbacks: 0,
                        unretained_callbacks: 0,
                        unavailable_polls: 0,
                        writer_finished: false,
                        drained_checkpoints: 0,
                        checkpoints: Vec::new(),
                    })
                    .collect(),
            },
            overlap: vec![Vec::new(); self.0.probes.len()],
        })
    }
}

impl NativeSrcTraceCollector {
    /// Control only. A transient unknown does not invent a gap: a later suffix
    /// can still cover it. If it cannot, the missing sequences stay explicit.
    pub fn poll(&mut self) {
        self.collect(false);
    }

    fn collect(&mut self, final_read: bool) {
        if self.artifact.finalized {
            return;
        }
        for ((boundary, (_, _, probe)), overlap) in self
            .artifact
            .boundaries
            .iter_mut()
            .zip(&self.owner.0.probes)
            .zip(&self.overlap)
        {
            let Some(history) = probe.src_history() else {
                boundary.unavailable_polls = boundary.unavailable_polls.saturating_add(1);
                if final_read {
                    self.artifact
                        .refusal
                        .get_or_insert(SrcTraceRefusal::HistoryUnavailable);
                }
                continue;
            };
            let remaining = self.artifact.checkpoint_limit - self.artifact.retained_checkpoints;
            let before = boundary.checkpoints.len();
            if let Some(refusal) = accept(boundary, history, remaining, overlap) {
                self.artifact.refusal.get_or_insert(refusal);
            }
            self.artifact.retained_checkpoints += boundary.checkpoints.len() - before;
        }
    }

    /// Call after independently joining owners. Closure is read from each
    /// actual boundary, never inferred from registry Running=false or this call.
    pub fn finish(&mut self) {
        if self.artifact.finalized {
            return;
        }
        self.collect(true);
        for boundary in &self.artifact.boundaries {
            let failure = if !boundary.writer_finished {
                Some(SrcTraceRefusal::OpenWriter)
            } else if boundary.total_callbacks == 0 {
                Some(SrcTraceRefusal::NoCallbacks)
            } else {
                None
            };
            if let Some(failure) = failure {
                self.artifact.refusal.get_or_insert(failure);
            }
        }
        self.artifact.finalized = true;
        self.artifact.history_complete = self.artifact.refusal.is_none()
            && self.artifact.boundaries.iter().all(|boundary| {
                boundary.writer_finished
                    && boundary.total_callbacks > 0
                    && boundary.missing_callbacks == 0
                    && boundary.unretained_callbacks == 0
                    && boundary.drained_checkpoints + boundary.checkpoints.len() as u64
                        == boundary.total_callbacks
            });
    }

    pub fn artifact(&self) -> &NativeSrcTrace {
        &self.artifact
    }

    /// Move pending checkpoints to a bounded control-side chunk. The caller
    /// owns persistence and must retain a failed/full-queue chunk; draining is
    /// not an IO acknowledgment. Loss/cap refusals never reset at a chunk edge.
    pub fn drain_chunk(&mut self) -> Option<NativeSrcTraceChunk> {
        if self.artifact.retained_checkpoints == 0 {
            return None;
        }
        let Some(sequence) = self.artifact.drained_chunks.checked_add(1) else {
            self.artifact
                .refusal
                .get_or_insert(SrcTraceRefusal::InvalidHistory);
            self.artifact.history_complete = false;
            return None;
        };
        let mut boundaries = Vec::new();
        for (boundary, overlap) in self.artifact.boundaries.iter_mut().zip(&mut self.overlap) {
            if boundary.checkpoints.is_empty() {
                continue;
            }
            let start = boundary
                .checkpoints
                .len()
                .saturating_sub(SRC_HISTORY_CAPACITY);
            // A new suffix need only retain the most recent callback window.
            // Older drained sequences cannot reappear in a valid history.
            if start == 0 {
                let excess = (overlap.len() + boundary.checkpoints.len())
                    .saturating_sub(SRC_HISTORY_CAPACITY);
                overlap.drain(..excess);
                overlap.extend_from_slice(&boundary.checkpoints);
            } else {
                *overlap = boundary.checkpoints[start..].to_vec();
            }
            boundary.drained_checkpoints += boundary.checkpoints.len() as u64;
            boundaries.push(NativeSrcChunkBoundary {
                node: boundary.node,
                kind: boundary.kind,
                checkpoints: std::mem::take(&mut boundary.checkpoints),
            });
        }
        self.artifact.drained_checkpoints += self.artifact.retained_checkpoints as u64;
        self.artifact.retained_checkpoints = 0;
        self.artifact.drained_chunks = sequence;
        Some(NativeSrcTraceChunk {
            schema_version: 1,
            owner_generation: self.artifact.owner_generation,
            sequence,
            boundaries,
        })
    }
}

fn accept(
    boundary: &mut NativeSrcBoundaryTrace,
    history: BoundarySrcHistory,
    limit: usize,
    overlap: &[BoundarySrcCheckpoint],
) -> Option<SrcTraceRefusal> {
    let total = history.total_callbacks;
    let count = total.min(SRC_HISTORY_CAPACITY as u64);
    if history.schema_version != 1
        || history.checkpoints.len() as u64 != count
        || history.overwritten_callbacks != total - count
        || total < boundary.total_callbacks
        || (boundary.writer_finished
            && (!history.writer_finished || total != boundary.total_callbacks))
        || history
            .checkpoints
            .iter()
            .enumerate()
            .any(|(index, point)| {
                point.schema_version != 1 || point.sequence != total - count + index as u64 + 1
            })
    {
        return Some(SrcTraceRefusal::InvalidHistory);
    }
    let mut refusal = None;
    let previous = boundary.total_callbacks;
    let mut last = previous;
    let mut retained = 0;
    for point in history.checkpoints {
        if point.sequence <= previous {
            let retained = [&boundary.checkpoints[..], overlap]
                .into_iter()
                .find_map(|points| {
                    points
                        .binary_search_by_key(&point.sequence, |p| p.sequence)
                        .ok()
                        .map(|index| &points[index])
                });
            if retained.is_some_and(|retained| *retained != point) {
                return Some(SrcTraceRefusal::InvalidHistory);
            }
            continue;
        }
        if point.sequence > last + 1 {
            boundary.missing_callbacks += point.sequence - last - 1;
            refusal.get_or_insert(SrcTraceRefusal::MissingCallbacks);
        }
        last = point.sequence;
        if retained < limit {
            boundary.checkpoints.push(point);
            retained += 1;
        } else {
            boundary.unretained_callbacks += 1;
            refusal.get_or_insert(SrcTraceRefusal::ArchiveLimit);
        }
    }
    boundary.total_callbacks = total;
    boundary.writer_finished = history.writer_finished;
    refusal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn src_trace_rejects_changed_overlap_and_malformed_suffix_without_replacing_prefix() {
        let mut fixture = super::super::src_trace_tests::Fixture::new();
        fixture.callbacks(0, 3);
        let mut collector = fixture
            .observations
            .retain_session()
            .unwrap()
            .src_trace_collector(8)
            .unwrap();
        collector.poll();
        let before = serde_json::to_value(collector.artifact()).unwrap();
        let mut changed = fixture.probe.src_history().unwrap();
        changed.checkpoints[0].carrier_frames += 1;
        assert_eq!(
            accept(&mut collector.artifact.boundaries[0], changed, 5, &[]),
            Some(SrcTraceRefusal::InvalidHistory)
        );
        let mut malformed = fixture.probe.src_history().unwrap();
        malformed.checkpoints[1].sequence = 99;
        assert_eq!(
            accept(&mut collector.artifact.boundaries[0], malformed, 5, &[]),
            Some(SrcTraceRefusal::InvalidHistory)
        );
        assert_eq!(serde_json::to_value(collector.artifact()).unwrap(), before);
    }

    #[test]
    fn src_trace_drained_overlap_still_rejects_changed_earlier_facts() {
        let mut fixture = super::super::src_trace_tests::Fixture::new();
        let mut collector = fixture
            .observations
            .retain_session()
            .unwrap()
            .src_trace_collector(256)
            .unwrap();
        fixture.callbacks(0, 200);
        collector.poll();
        collector.drain_chunk().unwrap();
        fixture.callbacks(200, 3);
        collector.poll();
        collector.drain_chunk().unwrap();
        assert_eq!(collector.overlap[0].len(), 203);
        let mut changed = fixture.probe.src_history().unwrap();
        changed.checkpoints[0].carrier_frames += 1;
        assert_eq!(
            accept(
                &mut collector.artifact.boundaries[0],
                changed,
                256,
                &collector.overlap[0]
            ),
            Some(SrcTraceRefusal::InvalidHistory)
        );
        assert_eq!(collector.artifact.drained_checkpoints, 203);
        assert_eq!(collector.artifact.retained_checkpoints, 0);
    }
}
