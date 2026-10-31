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
    pub checkpoints: Vec<BoundarySrcCheckpoint>,
}

#[derive(Debug, Serialize)]
pub struct NativeSrcTrace {
    pub schema_version: u32,
    pub owner_generation: u64,
    pub checkpoint_limit: usize,
    pub retained_checkpoints: usize,
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
                schema_version: 1,
                owner_generation: self.0.generation,
                checkpoint_limit: limit,
                retained_checkpoints: 0,
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
                        checkpoints: Vec::new(),
                    })
                    .collect(),
            },
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
        for (boundary, (_, _, probe)) in self
            .artifact
            .boundaries
            .iter_mut()
            .zip(&self.owner.0.probes)
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
            if let Some(refusal) = accept(boundary, history, remaining) {
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
                    && boundary.checkpoints.len() as u64 == boundary.total_callbacks
            });
    }

    pub fn artifact(&self) -> &NativeSrcTrace {
        &self.artifact
    }
}

fn accept(
    boundary: &mut NativeSrcBoundaryTrace,
    history: BoundarySrcHistory,
    limit: usize,
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
            if let Ok(index) = boundary
                .checkpoints
                .binary_search_by_key(&point.sequence, |p| p.sequence)
                && boundary.checkpoints[index] != point
            {
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
            accept(&mut collector.artifact.boundaries[0], changed, 5),
            Some(SrcTraceRefusal::InvalidHistory)
        );
        let mut malformed = fixture.probe.src_history().unwrap();
        malformed.checkpoints[1].sequence = 99;
        assert_eq!(
            accept(&mut collector.artifact.boundaries[0], malformed, 5),
            Some(SrcTraceRefusal::InvalidHistory)
        );
        assert_eq!(serde_json::to_value(collector.artifact()).unwrap(), before);
    }
}
