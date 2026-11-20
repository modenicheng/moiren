//! The three-state revision machine: candidate → pending → active, with the
//! confirmed double gate that only a matching owner generation may close.
use super::{GraphFault, ServiceGraph, SessionGeneration};

impl ServiceGraph {
    pub fn preparing(&mut self, generation: SessionGeneration, revision: u64) {
        let changed = self.pending_generation != Some(generation)
            || self.snapshot.pending_revision != Some(revision);
        self.pending_generation = Some(generation);
        self.snapshot.pending_revision = Some(revision);
        if changed {
            self.bump();
        }
    }
    pub fn running(&mut self, generation: SessionGeneration, revision: u64) {
        if self.pending_generation == Some(generation)
            && self.snapshot.pending_revision == Some(revision)
        {
            // The matching pending state always clears here: a real change.
            self.snapshot.active_revision = Some(revision);
            self.snapshot.pending_revision = None;
            self.pending_generation = None;
            self.snapshot.preparing_nodes.clear();
            self.bump();
        }
    }
    pub fn stopped(&mut self) {
        // Projected every round while an owner waits for resources; only an
        // actual confirmation/pending change may advance the token.
        let changed =
            self.snapshot.active_revision.is_some() || !self.snapshot.preparing_nodes.is_empty();
        self.snapshot.active_revision = None;
        self.snapshot.preparing_nodes.clear();
        if changed {
            self.bump();
        }
    }
    /// Only a managed owner may call this after host/RT confirmation. An older
    /// applied topology may become active while a newer candidate is pending.
    pub(crate) fn confirmed(&mut self, generation: SessionGeneration, revision: u64) {
        // Projected every round while the session runs; the settled steady
        // state re-reports the same revision and must not advance the token.
        let mut changed = false;
        if self.snapshot.candidate_revision == revision
            && (self.snapshot.active_revision != Some(revision)
                || self.snapshot.pending_revision == Some(revision))
        {
            self.snapshot.last_error = None;
            self.snapshot.faults.clear();
            changed = true;
        }
        if self.snapshot.active_revision != Some(revision) {
            self.snapshot.active_revision = Some(revision);
            changed = true;
        }
        if self.pending_generation == Some(generation)
            && self.snapshot.pending_revision == Some(revision)
        {
            self.clear_pending();
            changed = true;
        }
        if changed {
            self.bump();
        }
    }
    pub fn clear_pending(&mut self) {
        let changed = self.pending_generation.is_some()
            || self.snapshot.pending_revision.is_some()
            || !self.snapshot.preparing_nodes.is_empty();
        self.pending_generation = None;
        self.snapshot.pending_revision = None;
        self.snapshot.preparing_nodes.clear();
        if changed {
            self.bump();
        }
    }
    pub fn preparation_failed(&mut self, generation: SessionGeneration, error: impl ToString) {
        if self.pending_generation == Some(generation) {
            self.clear_pending();
            self.record_error(error);
        }
    }
    /// A rebuild rejected by the engine: keep the old topology running and
    /// report the located fault.
    pub fn preparation_rejected(
        &mut self,
        generation: SessionGeneration,
        fault: GraphFault,
        error: String,
    ) {
        if self.pending_generation == Some(generation) {
            self.clear_pending();
            self.record_fault(fault);
            if self.snapshot.last_error.as_deref() != Some(error.as_str()) {
                self.snapshot.last_error = Some(error);
                self.bump();
            }
        }
    }
}
