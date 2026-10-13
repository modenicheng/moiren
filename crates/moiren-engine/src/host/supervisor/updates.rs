use super::*;

impl<S: ProcessingSample> Supervisor<S> {
    pub(super) fn retired(&mut self) {
        if self.cleanup_inflight >= self.config.cleanup_capacity - 2 {
            return;
        }
        let Some(active) = &mut self.active else {
            return;
        };
        let Some(retired) = active.plans.poll_retired() else {
            return;
        };
        let Some(candidate) = self.pending.take() else {
            self.fault = Some(Diagnostic::Fault(
                "retirement has no matching candidate".into(),
            ));
            return;
        };
        let control = match retired.outcome() {
            RetireOutcome::Replaced {
                active_revision, ..
            } => {
                if active_revision != candidate.control.identity().0 {
                    self.fault = Some(Diagnostic::Fault("retirement revision mismatch".into()));
                }
                active.bindings = candidate.bindings;
                active.snapshot = candidate.snapshot;
                active.project = candidate.project;
                active.graph = candidate.graph;
                let control = std::mem::replace(&mut active.control, candidate.control);
                let graph = active.graph;
                let plan = active.control.identity().0;
                self.activate_edits(graph, Some(plan));
                self.reconcile();
                control
            }
            RetireOutcome::Rejected(error) => {
                if candidate.graph == self.desired && !self.closing() {
                    self.blocked = Some((self.basis(), Diagnostic::Compile(error.to_string())));
                }
                candidate.control
            }
        };
        self.cleanup(Cleanup::Retired(retired, control));
    }

    pub(super) fn completions(&mut self) {
        if self.cleanup_inflight >= self.config.cleanup_capacity {
            return;
        }
        let Ok(completion) = self.prepared.try_recv() else {
            return;
        };
        self.preparing = None;
        let eligible = !self.closing()
            && completion.basis == self.basis()
            && !self.unavailable()
            && self.project.has_output();
        if !eligible {
            if let Ok(ready) = completion.result {
                self.cleanup(Cleanup::Discard(ready));
            }
            return;
        }
        match completion.result {
            Err(reason) => {
                if matches!(reason, Diagnostic::Fault(_)) {
                    self.fault = Some(reason.clone());
                }
                self.blocked = Some((completion.basis, reason));
            }
            Ok(Ready::Initial(initial)) => {
                let shared = self.shared.clone();
                let gate = shared
                    .wake_lock
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                if shared.closing.load(Ordering::Acquire) {
                    drop(gate);
                    self.cleanup(Cleanup::Discard(Ready::Initial(initial)));
                    return;
                }
                if let Err(error) = initial.owner.activate() {
                    drop(gate);
                    self.blocked = Some((completion.basis, Diagnostic::Backend(error)));
                    self.cleanup(Cleanup::Discard(Ready::Initial(initial)));
                    return;
                }
                drop(gate);
                self.active = Some(Active {
                    owner: initial.owner,
                    plans: initial.plans,
                    control: initial.control,
                    bindings: initial.bindings,
                    snapshot: initial.snapshot,
                    project: completion.project,
                    graph: completion.basis.graph,
                    signals: initial.signals,
                    confirmed: false,
                });
                self.blocked = None;
            }
            Ok(Ready::Update(update)) => {
                let snapshot = update.plan.snapshot();
                let active = self
                    .active
                    .as_mut()
                    .expect("eligible update retains basis owner");
                match active.plans.publish(update.plan) {
                    Ok(()) => {
                        self.pending = Some(Candidate {
                            control: update.control,
                            bindings: update.bindings,
                            snapshot,
                            project: completion.project,
                            graph: completion.basis.graph,
                        });
                        self.blocked = None;
                    }
                    Err(failure) => {
                        self.blocked = Some((
                            completion.basis,
                            Diagnostic::Compile(failure.reason.to_string()),
                        ));
                        self.cleanup(Cleanup::Discard(Ready::Update(
                            crate::compiler::CompiledUpdate {
                                plan: failure.plan,
                                control: update.control,
                                bindings: update.bindings,
                                stats: update.stats,
                            },
                        )));
                    }
                }
            }
        }
    }

    pub(super) fn owner_status(&mut self) {
        let Some(active) = &mut self.active else {
            return;
        };
        let status = catch_unwind(AssertUnwindSafe(|| active.owner.status()))
            .unwrap_or(OwnerStatus::Failed(BackendError::WorkerPanicked));
        match status {
            OwnerStatus::Running if !active.confirmed => {
                active.confirmed = true;
                let graph = active.graph;
                let plan = active.control.identity().0;
                self.activate_edits(graph, Some(plan));
                self.reconcile();
            }
            OwnerStatus::Failed(error) => {
                if let Some((_, cancel)) = &self.preparing {
                    cancel.cancel();
                }
                if matches!(error, BackendError::WorkerPanicked) {
                    self.fault = Some(Diagnostic::Backend(error.clone()));
                }
                self.stop_active();
                self.blocked = Some((self.basis(), Diagnostic::Backend(error)));
            }
            _ => {}
        }
    }

    pub(super) fn reconcile(&mut self) {
        let Some(active) = &self.active else {
            return;
        };
        let prepared = active.project.clone();
        self.send_values(&prepared);
    }

    pub(super) fn activate_edits(&mut self, graph: u64, plan_revision: Option<u64>) {
        self.publish_snapshot();
        self.edits.retain(|(revision, receipt)| {
            if *revision == graph {
                receipt.cell.set(EditOutcome::Activated {
                    graph_revision: graph,
                    plan_revision,
                });
                false
            } else {
                true
            }
        });
    }

    pub(super) fn schedule(&mut self) {
        if self.preparing.is_some()
            || self.pending.is_some()
            || self.cleanup_inflight > self.config.cleanup_capacity - 3
            || self.fault.is_some()
            || self
                .blocked
                .as_ref()
                .is_some_and(|(basis, _)| *basis == self.basis())
        {
            return;
        }
        if let Some(active) = &self.active {
            if !active.confirmed || active.graph == self.desired {
                return;
            }
        } else if self.cleanup_inflight != 0 {
            return;
        }
        let Some(plan_revision) = self.plan_revision.checked_add(1) else {
            self.fault = Some(Diagnostic::Fault("plan revision exhausted".into()));
            return;
        };
        self.plan_revision = plan_revision;
        if self.active.is_none() {
            let (Some(owner), Some(epoch)) = (
                self.owner_generation.checked_add(1),
                self.epoch.checked_add(1),
            ) else {
                self.fault = Some(Diagnostic::Fault("owner identity exhausted".into()));
                return;
            };
            self.owner_generation = owner;
            self.epoch = epoch;
        }
        let mut reuse = Vec::new();
        let mut edge_reuse = Vec::new();
        if let Some(active) = &self.active {
            for node in self.project.graph.nodes() {
                if active
                    .project
                    .graph
                    .get_node(node.id())
                    .is_ok_and(|old| old == node)
                    && active.project.nodes.get(&node.id()) == self.project.nodes.get(&node.id())
                    && let Some(processor) = active.bindings.node(node.id())
                {
                    reuse.push((node.id(), processor));
                }
            }
            for edge in self.project.graph.edges() {
                if active
                    .project
                    .graph
                    .get_edge(edge.id())
                    .is_ok_and(|old| old == edge)
                    && let Some(keys) = active.bindings.edge(edge.id())
                {
                    edge_reuse.push((edge.id(), keys.gain.processor));
                }
            }
        }
        let basis = self.basis();
        let cancel = CancellationToken::default();
        let job = PrepareJob {
            basis,
            plan_revision,
            epoch: self.epoch,
            project: self.project.clone(),
            reuse,
            edge_reuse,
            active: self.active.as_ref().map(|active| active.snapshot.clone()),
            signals: self.active.as_ref().map(|active| active.signals.clone()),
            cancel: cancel.clone(),
        };
        self.preparations
            .try_send(job)
            .unwrap_or_else(|_| unreachable!("one reserved preparation slot"));
        self.preparing = Some((basis, cancel));
    }
}
