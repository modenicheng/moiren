use super::*;

impl<S: ProcessingSample> Supervisor<S> {
    pub(super) fn publish_snapshot(&self) {
        let closing = self.closing();
        let active = self.active.as_ref().filter(|active| active.confirmed);
        let update = if let Some((basis, reason)) = &self.blocked {
            if *basis == self.basis() {
                UpdateState::Blocked {
                    graph_revision: basis.graph,
                    reason: reason.clone(),
                }
            } else {
                UpdateState::Idle
            }
        } else if let Some(candidate) = &self.pending {
            UpdateState::Pending(candidate.graph)
        } else if let Some((basis, _)) = &self.preparing {
            UpdateState::Preparing(basis.graph)
        } else {
            UpdateState::Idle
        };
        let waiting = self.unavailable()
            || matches!(
                &update,
                UpdateState::Blocked {
                    reason: Diagnostic::Backend(BackendError::ResourceUnavailable(_)),
                    ..
                }
            );
        let timeline = active.and_then(|active| {
            active
                .owner
                .timeline()
                .filter(|timeline| (timeline.revision, timeline.epoch) == active.control.identity())
        });
        let mut snapshot = self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        // Publish both projects and metadata while holding the same lock. Arc
        // reuse keeps frequent timeline reads from copying an unchanged graph.
        if snapshot.desired.as_ref() != &self.project {
            snapshot.desired = Arc::new(self.project.clone());
        }
        snapshot.active = active.map(|active| {
            snapshot
                .active
                .as_ref()
                .filter(|project| project.as_ref() == &active.project)
                .cloned()
                .unwrap_or_else(|| Arc::new(active.project.clone()))
        });
        snapshot.state = HostSnapshot {
            host: if closing {
                HostState::Closing
            } else {
                HostState::Open
            },
            execution: if active.is_some() {
                ExecutionState::Running
            } else if waiting {
                ExecutionState::WaitingForResources
            } else if self.preparing.is_some() || self.active.is_some() {
                ExecutionState::Preparing
            } else {
                ExecutionState::Dormant
            },
            desired_revision: self.desired,
            active_revision: active.map(|active| active.graph),
            plan_revision: active.map(|active| active.control.identity().0),
            owner_generation: self.owner_generation,
            resource_generation: self.resources,
            timeline,
            update,
            fault: self.fault.clone(),
            cleanup_error: self.cleanup_error.clone(),
        };
    }
}
