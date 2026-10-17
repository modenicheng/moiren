use moiren_core::protocol::{
    ApplyAt, ControlReply, ParamValue, ParameterKey, ParameterRequest, ReplyCode,
};

use super::{
    EngineProject, ParameterFailure, ParameterOutcome, ParameterReceipt, ParameterTarget,
    ParameterTiming, SendParameter,
};
use crate::{compiler::CompiledBindings, control::ControlPort};

struct Pending {
    id: u64,
    target: ParameterTarget,
    value: ParamValue,
    timing: ParameterTiming,
    ramp_frames: u32,
    receipt: Option<ParameterReceipt>,
    submitted: Option<(u64, u64)>,
    followup: Option<(u64, ParamValue, u32)>,
}
#[derive(Default)]
pub(crate) struct Router {
    pending: Vec<Pending>,
}
impl Router {
    pub(crate) fn accept(
        &mut self,
        id: u64,
        target: ParameterTarget,
        value: ParamValue,
        timing: ParameterTiming,
        ramp_frames: u32,
        receipt: Option<ParameterReceipt>,
    ) {
        self.pending.retain(|previous| {
            if previous.target == target
                && previous.submitted.is_none()
                && matches!(previous.timing, ParameterTiming::NextBlock)
            {
                if let Some(receipt) = &previous.receipt {
                    receipt.cell.set(ParameterOutcome::Superseded);
                }
                false
            } else {
                true
            }
        });
        if receipt.is_none()
            && let Some(pending) = self
                .pending
                .iter_mut()
                .find(|pending| pending.target == target && pending.receipt.is_none())
        {
            pending.followup = Some((id, value, ramp_frames));
            return;
        }
        if let Some(receipt) = &receipt {
            receipt.cell.set(ParameterOutcome::Accepted);
        }
        self.pending.push(Pending {
            id,
            target,
            value,
            timing,
            ramp_frames,
            receipt,
            submitted: None,
            followup: None,
        });
    }
    pub(crate) fn has_target(&self, target: ParameterTarget, value: ParamValue) -> bool {
        self.pending.iter().any(|pending| {
            pending.target == target
                && (pending.value == value
                    || pending
                        .followup
                        .is_some_and(|(_, latest, _)| latest == value))
        })
    }
    pub(crate) fn prune(&mut self, project: &EngineProject) {
        self.pending.retain(|pending| {
            if pending.submitted.is_none() && !project.target_exists(pending.target) {
                finish(
                    pending,
                    ParameterOutcome::Rejected(ParameterFailure::TargetRemoved),
                );
                false
            } else {
                true
            }
        });
    }
    pub(crate) fn route(
        &mut self,
        desired: &EngineProject,
        active: &EngineProject,
        bindings: &CompiledBindings,
        control: &mut ControlPort,
        frame: u64,
    ) {
        let identity = control.identity();
        self.pending.retain_mut(|pending| {
            if pending.submitted.is_some() {
                return true;
            }
            if !desired.target_exists(pending.target) {
                finish(
                    pending,
                    ParameterOutcome::Rejected(ParameterFailure::TargetRemoved),
                );
                return false;
            }
            let (at, epoch) = match pending.timing {
                ParameterTiming::NextBlock => (ApplyAt::NextBlock, identity.1),
                ParameterTiming::Frame { epoch, frame } => (ApplyAt::Frame(frame), epoch),
            };
            if epoch != identity.1 {
                finish(
                    pending,
                    ParameterOutcome::Rejected(ParameterFailure::StaleEpoch),
                );
                return false;
            }
            if !active.target_exists(pending.target) {
                return true;
            }
            let Some(target) = resolve(bindings, pending.target) else {
                finish(
                    pending,
                    ParameterOutcome::Rejected(ParameterFailure::InvalidTarget),
                );
                return false;
            };
            let reply = control.submit(
                ParameterRequest {
                    request_id: pending.id,
                    plan_revision: identity.0,
                    timeline_epoch: epoch,
                    target,
                    at,
                    value: pending.value,
                    ramp_frames: pending.ramp_frames,
                },
                frame,
            );
            match reply.code {
                ReplyCode::Accepted => {
                    pending.submitted = Some(identity);
                    true
                }
                ReplyCode::QueueFull => true,
                ReplyCode::StaleRevision
                    if matches!(pending.timing, ParameterTiming::NextBlock) =>
                {
                    true
                }
                code => {
                    finish(
                        pending,
                        ParameterOutcome::Rejected(ParameterFailure::Runtime(code)),
                    );
                    false
                }
            }
        });
    }
    pub(crate) fn reply(
        &mut self,
        reply: ControlReply,
        project: &mut EngineProject,
        closing: bool,
    ) {
        let Some(index) = self.pending.iter().position(|pending| {
            pending.id == reply.request_id
                && pending.submitted == Some((reply.plan_revision, reply.timeline_epoch))
        }) else {
            return;
        };
        let pending = &mut self.pending[index];
        let applied = matches!(reply.code, ReplyCode::Applied | ReplyCode::AppliedLate);
        if applied {
            if matches!(pending.timing, ParameterTiming::Frame { .. })
                && project.target_exists(pending.target)
            {
                project.set_parameter(pending.target, pending.value);
            }
            finish(pending, ParameterOutcome::Applied(reply));
        } else if reply.code == ReplyCode::StaleRevision
            && !closing
            && matches!(pending.timing, ParameterTiming::NextBlock)
            && project.target_exists(pending.target)
        {
            if project
                .parameter_values()
                .iter()
                .any(|value| value.target == pending.target && value.value == pending.value)
            {
                pending.submitted = None;
                return;
            }
            finish(pending, ParameterOutcome::Superseded);
        } else {
            finish(
                pending,
                if closing {
                    ParameterOutcome::Closed
                } else {
                    ParameterOutcome::Rejected(ParameterFailure::Runtime(reply.code))
                },
            );
        }
        if !closing && let Some((id, value, ramp_frames)) = pending.followup.take() {
            pending.id = id;
            pending.value = value;
            pending.ramp_frames = ramp_frames;
            pending.submitted = None;
            return;
        }
        self.pending.remove(index);
    }
    pub(crate) fn poll(
        &mut self,
        control: &mut ControlPort,
        project: &mut EngineProject,
        closing: bool,
    ) {
        while let Some(reply) = control.poll_applied() {
            self.reply(reply, project, closing);
        }
    }
    pub(crate) fn close(&mut self) {
        for pending in self.pending.drain(..) {
            finish(&pending, ParameterOutcome::Closed);
        }
    }
}
fn finish(pending: &Pending, outcome: ParameterOutcome) {
    if let Some(receipt) = &pending.receipt {
        receipt.cell.set(outcome);
    }
}
pub(crate) fn resolve(
    bindings: &CompiledBindings,
    target: ParameterTarget,
) -> Option<ParameterKey> {
    match target {
        ParameterTarget::Node { node, parameter } => {
            bindings.node(node).map(|processor| ParameterKey {
                processor,
                parameter,
            })
        }
        ParameterTarget::RackSlot {
            node,
            slot,
            parameter,
        } => {
            // Slot IDs survive configuration edits; ProcessorIds do not survive
            // recompilation. Always use this active plan's returned slot mapping.
            bindings
                .rack_slot(node, slot)
                .map(|processor| ParameterKey {
                    processor,
                    parameter,
                })
        }
        ParameterTarget::Edge { edge, parameter } => {
            bindings.edge(edge).map(|keys| match parameter {
                SendParameter::Gain => keys.gain,
                SendParameter::Pan => keys.pan,
                SendParameter::Mute => keys.mute,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compiler::{self, NodeBindings},
        host::{NodeConfig, model::ReceiptCell},
        processor::Gain,
    };
    use moiren_core::graph::NodeKind;
    use std::sync::{Arc, atomic::AtomicUsize};

    fn receipt() -> ParameterReceipt {
        ParameterReceipt {
            cell: ReceiptCell::reserve(ParameterOutcome::Queued, &Arc::new(AtomicUsize::new(0)), 4)
                .unwrap(),
        }
    }

    #[test]
    fn next_block_parameter_waits_when_rt_has_retired_the_observed_plan() {
        let mut project = EngineProject::default();
        let node = project.graph.create_node(NodeKind::Gain, 1).unwrap();
        project
            .nodes
            .insert(node, NodeConfig::Gain { initial: 1.0 });
        let bind = || {
            let mut bindings = NodeBindings::<f32>::new();
            bindings.bind_gain(node, 1.0).unwrap();
            bindings
        };
        let mut compiled = compiler::compile(
            &project.graph,
            bind(),
            project.processing.compile_config(1, 1),
        )
        .unwrap();
        let basis = compiled.engine.plan_snapshot();
        let mut plans = compiled.engine.enable_plan_switching(2).unwrap();
        let mut update = compiler::compile_update(
            &project.graph,
            bind(),
            project.processing.compile_config(2, 1),
            &basis,
            &[],
            None,
        )
        .unwrap();
        plans.publish(update.plan).unwrap();
        compiled.engine.render(8).unwrap();
        let receipt = receipt();
        let mut router = Router::default();
        router.accept(
            1,
            ParameterTarget::Node {
                node,
                parameter: Gain::LEVEL,
            },
            ParamValue::Float(0.5),
            ParameterTiming::NextBlock,
            0,
            Some(receipt.clone()),
        );
        router.route(
            &project,
            &project,
            &compiled.bindings,
            &mut compiled.control,
            8,
        );
        assert_eq!(receipt.outcome(), ParameterOutcome::Accepted);
        assert_eq!(router.pending.len(), 1);
        router.route(&project, &project, &update.bindings, &mut update.control, 8);
        compiled.engine.render(8).unwrap();
        router.poll(&mut update.control, &mut project, false);
        assert!(matches!(receipt.outcome(), ParameterOutcome::Applied(_)));
        assert!(router.pending.is_empty());
    }

    #[test]
    fn stale_inflight_parameter_superseded_by_new_intent_is_not_a_runtime_failure() {
        let mut project = EngineProject::default();
        let node = project.graph.create_node(NodeKind::Gain, 1).unwrap();
        project
            .nodes
            .insert(node, NodeConfig::Gain { initial: 1.0 });
        let target = ParameterTarget::Node {
            node,
            parameter: Gain::LEVEL,
        };
        project.set_parameter(target, ParamValue::Float(0.9));
        let receipt = receipt();
        let mut router = Router::default();
        router.accept(
            1,
            target,
            ParamValue::Float(0.5),
            ParameterTiming::NextBlock,
            0,
            Some(receipt.clone()),
        );
        router.pending[0].submitted = Some((1, 1));
        router.reply(
            ControlReply {
                request_id: 1,
                plan_revision: 1,
                timeline_epoch: 1,
                code: ReplyCode::StaleRevision,
                effective_frame: 0,
            },
            &mut project,
            false,
        );
        assert_eq!(receipt.outcome(), ParameterOutcome::Superseded);
        assert!(router.pending.is_empty());
    }
}
