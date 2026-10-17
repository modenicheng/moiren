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
                self.shared
                    .signals
                    .install_readers(self.owner_generation, candidate.readers);
                active.bindings = candidate.bindings;
                active.snapshot = candidate.snapshot;
                active.project = candidate.project;
                active.graph = candidate.graph;
                self.structural_values
                    .retain(|(revision, _)| *revision > candidate.graph);
                self.shared
                    .signals
                    .activate(active.owner_generation, &active.project);
                let control = std::mem::replace(&mut active.control, candidate.control);
                let graph = active.graph;
                let plan = active.control.identity().0;
                self.activate_edits(graph, Some(plan));
                self.reconcile();
                control
            }
            RetireOutcome::Rejected(error) => {
                if candidate.graph == self.desired
                    && candidate.parameter_intent == self.parameter_intent
                    && !self.closing()
                {
                    self.blocked = Some((self.basis(), Diagnostic::Compile(error.to_string())));
                }
                drop(candidate.readers);
                self.cleanup(Cleanup::Retired(retired, candidate.control));
                return;
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
                self.cleanup(Cleanup::Discard {
                    ready,
                    signal_hub: Arc::clone(&self.shared.signals),
                });
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
                    self.cleanup(Cleanup::Discard {
                        ready: Ready::Initial(initial),
                        signal_hub: Arc::clone(&self.shared.signals),
                    });
                    return;
                }
                if let Err(error) = initial.owner.activate() {
                    drop(gate);
                    self.blocked = Some((completion.basis, Diagnostic::Backend(error)));
                    self.cleanup(Cleanup::Discard {
                        ready: Ready::Initial(initial),
                        signal_hub: Arc::clone(&self.shared.signals),
                    });
                    return;
                }
                drop(gate);
                self.shared
                    .signals
                    .install_readers(initial.owner_generation, initial.readers);
                self.shared
                    .signals
                    .activate(initial.owner_generation, &completion.project);
                self.active = Some(Active {
                    owner: initial.owner,
                    plans: initial.plans,
                    control: initial.control,
                    bindings: initial.bindings,
                    snapshot: initial.snapshot,
                    project: completion.project,
                    graph: completion.basis.graph,
                    signals: initial.signals,
                    owner_generation: completion.basis.owner,
                    confirmed: false,
                });
                self.structural_values
                    .retain(|(revision, _)| *revision > completion.basis.graph);
                self.blocked = None;
            }
            Ok(Ready::Update { update, readers }) => {
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
                            parameter_intent: completion.basis.parameter_intent,
                            readers,
                        });
                        self.blocked = None;
                    }
                    Err(failure) => {
                        self.blocked = Some((
                            completion.basis,
                            Diagnostic::Compile(failure.reason.to_string()),
                        ));
                        self.cleanup(Cleanup::Discard {
                            ready: Ready::Update {
                                update: crate::compiler::CompiledUpdate {
                                    plan: failure.plan,
                                    control: update.control,
                                    bindings: update.bindings,
                                    stats: update.stats,
                                },
                                readers,
                            },
                            signal_hub: Arc::clone(&self.shared.signals),
                        });
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
                receipt.set(EditOutcome::Activated {
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
            // Reclaim retired/cancelled topic shapes before reserving another
            // candidate. Bounded signal capacity must not depend on worker speed.
            || self.cleanup_inflight != 0
            || self.fault.is_some()
            || self
                .blocked
                .as_ref()
                .is_some_and(|(basis, _)| *basis == self.basis())
        {
            return;
        }
        if let Some(active) = &self.active
            && (!active.confirmed || active.graph == self.desired)
        {
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
        let mut observer_reuse = Vec::new();
        let mut rack_reuse = Vec::new();
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
                    if let Some(observer) = active.bindings.observer(node.id()) {
                        observer_reuse.push((node.id(), observer));
                    }
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
            rack_reuse = reusable_rack_slots(&active.project, &self.project, &active.bindings);
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
            observer_reuse,
            rack_reuse,
            initial_parameters: self
                .structural_values
                .iter()
                .map(|(_, target)| *target)
                .collect(),
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

/// A stable slot keeps its state even if unrelated edits renumber processors.
/// Widths come from the resolved chain: an upstream mapper can change a slot's
/// IO without changing that slot's settings or the enclosing Rack's main width.
fn reusable_rack_slots(
    active: &EngineProject,
    desired: &EngineProject,
    bindings: &CompiledBindings,
) -> Vec<(
    (moiren_core::graph::NodeId, u32),
    moiren_core::protocol::ProcessorId,
)> {
    let mut reuse = Vec::new();
    for (&node, config) in &desired.nodes {
        let (NodeConfig::Rack(new), Some(NodeConfig::Rack(old))) =
            (config, active.nodes.get(&node))
        else {
            continue;
        };
        let resolve = |project: &EngineProject, settings: &crate::rack::RackSettings| {
            let graph_node = project.graph.get_node(node).ok()?;
            settings.resolve_layout(
                &graph_node
                    .inputs()
                    .iter()
                    .map(|port| port.channels())
                    .collect::<Vec<_>>(),
                &graph_node
                    .outputs()
                    .iter()
                    .map(|port| port.channels())
                    .collect::<Vec<_>>(),
            )
        };
        let (Some(previous), Some(next)) = (resolve(active, old), resolve(desired, new)) else {
            continue;
        };
        for slot in new.slots.iter().filter(|slot| !slot.bypass) {
            let Some(old_slot) = old
                .slots
                .iter()
                .find(|old| old.id == slot.id && !old.bypass)
            else {
                continue;
            };
            let (Some(old_io), Some(new_io)) =
                (previous.slots.get(&slot.id), next.slots.get(&slot.id))
            else {
                continue;
            };
            if old_slot.processor == slot.processor
                && old_io.input_channels == new_io.input_channels
                && old_io.output_channels == new_io.output_channels
                && let Some(processor) = bindings.rack_slot(node, slot.id)
            {
                reuse.push(((node, slot.id), processor));
            }
        }
    }
    reuse
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compiler::{self, NodeBindings},
        processor::DelaySettings,
        rack::{RackProcessor, RackSettings, RackSlot},
    };

    fn rack_project(channels: usize) -> (EngineProject, moiren_core::graph::NodeId) {
        let mut project = EngineProject::default();
        let node = project.graph.create_rack(&[channels], &[channels]).unwrap();
        project.nodes.insert(
            node,
            NodeConfig::Rack(RackSettings {
                slots: vec![RackSlot {
                    id: 42,
                    bypass: false,
                    processor: RackProcessor::Delay(DelaySettings::default()),
                }],
                ..Default::default()
            }),
        );
        (project, node)
    }

    #[test]
    fn rack_reuse_requires_unchanged_config_active_slot_and_actual_io() {
        let (active, node) = rack_project(1);
        let NodeConfig::Rack(settings) = &active.nodes[&node] else {
            unreachable!()
        };
        let mut bindings = NodeBindings::<f64>::new();
        bindings.bind_rack(node, settings.clone()).unwrap();
        let compiled = compiler::compile(
            &active.graph,
            bindings,
            active.processing.compile_config(1, 1),
        )
        .unwrap();
        let retained = reusable_rack_slots(&active, &active, &compiled.bindings);
        assert_eq!(
            retained,
            vec![((node, 42), compiled.bindings.rack_slot(node, 42).unwrap())]
        );

        let (different_width, _) = rack_project(2);
        assert!(reusable_rack_slots(&active, &different_width, &compiled.bindings).is_empty());
        for bypass in [false, true] {
            let mut changed = active.clone();
            let NodeConfig::Rack(settings) = changed.nodes.get_mut(&node).unwrap() else {
                unreachable!()
            };
            if bypass {
                settings.slots[0].bypass = true;
            } else {
                settings.slots[0].processor = RackProcessor::Delay(DelaySettings {
                    max_delay_ms: 42.0,
                    ..Default::default()
                });
            }
            assert!(reusable_rack_slots(&active, &changed, &compiled.bindings).is_empty());
            assert!(reusable_rack_slots(&changed, &active, &compiled.bindings).is_empty());
        }
    }
}
