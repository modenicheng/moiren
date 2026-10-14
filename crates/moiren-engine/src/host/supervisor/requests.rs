use super::*;

impl<S: ProcessingSample> Supervisor<S> {
    pub(super) fn commands(&mut self) {
        for _ in 0..self.config.command_capacity {
            let Ok(command) = self.commands.try_recv() else {
                break;
            };
            if self.closing() {
                close_command(command);
                continue;
            }
            match command {
                Command::Edit(edit, receipt) => {
                    let previous = self.project.clone();
                    let result = catch_unwind(AssertUnwindSafe(|| self.project.edit(edit)));
                    match result {
                        Ok(Ok(())) => {
                            if let Err(error) = self.config.validate_project(&self.project) {
                                self.project = previous;
                                receipt.cell.set(EditOutcome::Rejected(error));
                                continue;
                            }
                            self.resource_nodes.retain(|node, _| {
                                matches!(
                                    self.project.nodes.get(node),
                                    Some(NodeConfig::Input(_) | NodeConfig::Output(_))
                                ) && previous.nodes.get(node) == self.project.nodes.get(node)
                            });
                            if same_structure(&previous, &self.project) {
                                self.send_values(&previous);
                                receipt.cell.set(EditOutcome::Activated {
                                    graph_revision: self.desired,
                                    plan_revision: self
                                        .active
                                        .as_ref()
                                        .filter(|active| active.confirmed)
                                        .map(|active| active.control.identity().0),
                                });
                            } else if let Some(revision) = self.desired.checked_add(1) {
                                self.desired = revision;
                                for (_, previous) in self.edits.drain(..) {
                                    previous.cell.set(EditOutcome::Superseded {
                                        by_graph_revision: revision,
                                    });
                                }
                                receipt.cell.set(EditOutcome::Accepted {
                                    graph_revision: revision,
                                });
                                self.edits.push((revision, receipt));
                                if let Some((_, cancel)) = &self.preparing {
                                    cancel.cancel();
                                }
                                if self.pending.is_some()
                                    && let Some(active) = &mut self.active
                                {
                                    active.plans.cancel_pending();
                                }
                            } else {
                                self.project = previous;
                                self.fault =
                                    Some(Diagnostic::Fault("graph revision exhausted".into()));
                                receipt
                                    .cell
                                    .set(EditOutcome::Rejected(ProjectError::Capacity));
                            }
                        }
                        Ok(Err(error)) => receipt.cell.set(EditOutcome::Rejected(error)),
                        Err(_) => {
                            self.fault = Some(Diagnostic::Fault("project edit panicked".into()));
                            receipt
                                .cell
                                .set(EditOutcome::Rejected(ProjectError::InvalidProcessing));
                        }
                    }
                }
                Command::Parameter {
                    id,
                    target,
                    value,
                    timing,
                    ramp_frames,
                    receipt,
                } => {
                    let failure = if self.project.validate_parameter(target, value).is_err()
                        || (!matches!(value, moiren_core::protocol::ParamValue::Float(_))
                            && ramp_frames != 0)
                    {
                        Some(ParameterFailure::InvalidValue)
                    } else if let ParameterTiming::Frame { epoch, .. } = timing {
                        if !self.active.as_ref().is_some_and(|active| {
                            active.confirmed
                                && active.control.identity().1 == epoch
                                && active.project.target_exists(target)
                        }) {
                            Some(ParameterFailure::StaleEpoch)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(failure) = failure {
                        receipt.cell.set(ParameterOutcome::Rejected(failure));
                        continue;
                    }
                    if !self
                        .project
                        .parameters
                        .iter()
                        .any(|parameter| parameter.target == target)
                        && self.project.parameters.len() >= self.config.max_parameters
                    {
                        receipt
                            .cell
                            .set(ParameterOutcome::Rejected(ParameterFailure::Capacity));
                        continue;
                    }
                    if matches!(timing, ParameterTiming::NextBlock) {
                        self.project.set_parameter(target, value);
                    }
                    self.router
                        .accept(id, target, value, timing, ramp_frames, Some(receipt));
                }
            }
        }
        self.router.prune(&self.project);
    }

    pub(super) fn send_values(&mut self, previous: &EngineProject) {
        for edge in self.project.graph.edges().to_vec() {
            if let Ok(old) = previous.graph.get_edge(edge.id())
                && old.params() != edge.params()
            {
                for (parameter, value) in [
                    (
                        SendParameter::Gain,
                        moiren_core::protocol::ParamValue::Float(edge.params().gain),
                    ),
                    (
                        SendParameter::Pan,
                        moiren_core::protocol::ParamValue::Float(edge.params().pan),
                    ),
                    (
                        SendParameter::Mute,
                        moiren_core::protocol::ParamValue::Bool(edge.params().mute),
                    ),
                ] {
                    let target = ParameterTarget::Edge {
                        edge: edge.id(),
                        parameter,
                    };
                    self.project.set_parameter(target, value);
                }
            }
        }
        let previous_values = previous.parameter_values();
        let values = self.project.parameter_values();
        for parameter in values {
            if !previous_values.contains(&parameter)
                && !self.router.has_target(parameter.target, parameter.value)
            {
                match self.shared.next_parameter_id() {
                    Ok(id) => self.router.accept(
                        id,
                        parameter.target,
                        parameter.value,
                        ParameterTiming::NextBlock,
                        0,
                        None,
                    ),
                    Err(error) => self.fault = Some(Diagnostic::Fault(error.to_string())),
                }
            }
        }
    }

    pub(super) fn resources(&mut self) {
        if self.closing() {
            return;
        }
        for _ in 0..self.config.max_nodes {
            let event = match catch_unwind(AssertUnwindSafe(|| self.backend.poll_resource())) {
                Ok(event) => event,
                Err(_) => {
                    self.fault = Some(Diagnostic::Fault("resource monitor panicked".into()));
                    break;
                }
            };
            let Some(event) = event else {
                break;
            };
            if !matches!(
                self.project.nodes.get(&event.node),
                Some(NodeConfig::Input(_) | NodeConfig::Output(_))
            ) {
                continue;
            }
            if self.project.nodes.get(&event.node) != Some(&event.configuration) {
                continue;
            }
            if self
                .resource_nodes
                .get(&event.node)
                .is_some_and(|&(generation, _)| generation >= event.generation)
            {
                continue;
            }
            self.resource_nodes
                .insert(event.node, (event.generation, event.available));
            if let Some(generation) = self.resources.checked_add(1) {
                self.resources = generation;
            } else {
                self.fault = Some(Diagnostic::Fault("resource generation exhausted".into()));
            }
            if let Some((_, cancel)) = &self.preparing {
                cancel.cancel();
            }
            if let Some(active) = &mut self.active {
                active.plans.cancel_pending();
                active.owner.cancel();
            }
            self.stop_active();
        }
    }
}
