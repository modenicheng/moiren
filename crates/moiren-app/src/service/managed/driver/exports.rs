//! Export resource reconciliation per poll round.
use super::super::*;

impl ManagedDriver {
    pub(super) fn poll_exports(&mut self, c: &mut ServiceContext, pool: &mut JobPool) {
        // Only the export list and revisions are consulted; borrowing them
        // avoids the full graph snapshot deep copy on every poll. The export
        // list itself is cloned unless the exiting path clears it anyway.
        let (mut exports, candidate_revision, active_revision) = {
            let graph = c.graph.snapshot_ref();
            let pending_is_none = graph.pending_revision.is_none();
            let active_revision = graph.active_revision;
            let exports = if !self.exiting {
                graph.exports.clone()
            } else {
                Vec::new()
            };
            (
                exports,
                graph.candidate_revision,
                (active_revision, pending_is_none),
            )
        };
        let active = self
            .owner
            .as_ref()
            .map_or_else(Vec::new, |o| o.exports.clone());
        let basis = self.owner.as_ref().and_then(|owner| {
            let host = owner.observer.read();
            (host.state.execution == ExecutionState::Running
                && host.state.active_revision == Some(owner.active_revision.0)
                && active_revision.0 == Some(owner.active_revision.1)
                && active_revision.1
                && !self.apply
                && owner.document == self.document
                && !self.exiting)
                .then_some([
                    owner.generation.0,
                    host.state.owner_generation,
                    owner.active_revision.0,
                    owner.active_revision.1,
                    owner.active_apply_id,
                ])
        });
        self.exports_runtime
            .project(&mut c.exports, active_revision.0, &active, basis);
        if self.exiting {
            exports.clear();
        } else {
            // Unapplied candidate changes cannot replace a resource still used
            // by the current Engine owner. Retirement also waits for its reap.
            for intent in &active {
                exports.retain(|i| i.endpoint_id != intent.endpoint_id);
                exports.push(intent.clone());
            }
        }
        // Reconciliation retains every export leased by the current owner.
        // Once old owners are reaped, unrelated endpoints may retire while a
        // replacement owner runs. The provider also checks each PCM drain.
        let drained = self.cleaning == 0;
        let work = if self.exiting {
            self.exports_runtime
                .shutdown_work(drained && self.owner.is_none())
        } else {
            self.exports_runtime.work(
                &c.exports,
                &exports,
                candidate_revision,
                active_revision.0,
                drained,
            )
        };
        if let Some(work) = work
            && let Ok(completion) = self.queue().port().try_reserve()
        {
            self.exports_runtime
                .submit(pool, completion, work, Completion::Exports);
        }
    }
}
