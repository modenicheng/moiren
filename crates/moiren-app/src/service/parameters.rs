//! Bounded accepted ledger; only unaccepted slider intent may be replaced.
use super::{ActiveSession, ControlSummary};
use moiren_core::graph::NodeId;
use moiren_core::protocol::{
    ApplyAt, ControlReply, ParamValue, ParameterId, ParameterKey, ParameterRequest, ReplyCode,
};
use moiren_engine::processor::{Gain, Pan};
const LIMIT: usize = 64;
pub(super) struct Parameters {
    unsent: [Option<f64>; 2],
    node_unsent: Vec<(NodeId, ParameterId, ParamValue)>,
    ledger: Vec<ControlReply>,
    next_id: u64,
}
impl Default for Parameters {
    fn default() -> Self {
        Self {
            unsent: [None, None],
            node_unsent: Vec::with_capacity(LIMIT),
            ledger: Vec::with_capacity(LIMIT),
            next_id: 1,
        }
    }
}
impl Parameters {
    pub fn clear_node_intent(
        &mut self,
        summary: &mut ControlSummary,
        node: NodeId,
        parameter: ParameterId,
    ) {
        self.node_unsent
            .retain(|entry| entry.0 != node || entry.1 != parameter);
        summary.pending_node_parameters = self.node_unsent.len();
    }
    pub fn clear_primary_intent(&mut self, summary: &mut ControlSummary, slot: usize) {
        self.unsent[slot] = None;
        if self.unsent == [None, None] {
            summary.pending_gain_pan = None;
        }
    }
    #[cfg(test)]
    fn set_node(
        &mut self,
        summary: &mut ControlSummary,
        node: NodeId,
        parameter: ParameterId,
        value: f64,
    ) -> bool {
        self.set_node_value(summary, node, parameter, ParamValue::Float(value))
    }
    pub fn set_node_value(
        &mut self,
        summary: &mut ControlSummary,
        node: NodeId,
        parameter: ParameterId,
        value: ParamValue,
    ) -> bool {
        if let Some(intent) = self
            .node_unsent
            .iter_mut()
            .find(|entry| entry.0 == node && entry.1 == parameter)
        {
            intent.2 = value;
        } else {
            if self.node_unsent.len() == LIMIT {
                return false;
            }
            self.node_unsent.push((node, parameter, value));
        }
        summary.pending_node_parameters = self.node_unsent.len();
        true
    }
    pub fn set(&mut self, summary: &mut ControlSummary, gain: f64, pan: f64) {
        self.unsent = [Some(gain), Some(pan)];
        summary.pending_gain_pan = Some((gain, pan));
    }
    pub fn clear_intent(&mut self, summary: &mut ControlSummary) {
        self.unsent = [None, None];
        self.node_unsent.clear();
        summary.pending_node_parameters = 0;
        summary.pending_gain_pan = None;
    }
    pub fn is_drained(&self) -> bool {
        self.ledger.is_empty()
    }
    fn terminal(&mut self, summary: &mut ControlSummary, reply: ControlReply) {
        if reply.code == ReplyCode::Accepted {
            return;
        }
        if let Some(index) = self.ledger.iter().position(|entry| {
            entry.request_id == reply.request_id
                && entry.plan_revision == reply.plan_revision
                && entry.timeline_epoch == reply.timeline_epoch
        }) {
            self.ledger.swap_remove(index);
            summary.last_result = Some(reply);
        }
        summary.accepted_pending = self.ledger.len();
    }
    pub fn poll(
        &mut self,
        session: &mut dyn ActiveSession,
        summary: &mut ControlSummary,
        budget: usize,
    ) {
        let timeline = session.timeline();
        let nodes = session.gain_pan_nodes();
        let Some((control, bindings)) = session.control() else {
            return;
        };
        for _ in 0..budget.min(32) {
            let Some(reply) = control.poll_applied() else {
                break;
            };
            self.terminal(summary, reply);
        }
        let Some(timeline) = timeline else {
            return;
        };
        let (revision, epoch) = control.identity();
        if timeline.revision != revision || timeline.epoch != epoch {
            return;
        }
        for (slot, node, parameter) in nodes
            .into_iter()
            .flat_map(|(gain, pan)| [(0, gain, Gain::LEVEL), (1, pan, Pan::POSITION)])
        {
            if self.ledger.len() == LIMIT {
                break;
            }
            let Some(value) = self.unsent[slot] else {
                continue;
            };
            let Some(processor) = bindings.node(node) else {
                continue;
            };
            let Some(next) = self.next_id.checked_add(1) else {
                return;
            };
            let reply = control.submit(
                ParameterRequest {
                    request_id: self.next_id,
                    plan_revision: revision,
                    timeline_epoch: epoch,
                    target: ParameterKey {
                        processor,
                        parameter,
                    },
                    at: ApplyAt::NextBlock,
                    value: ParamValue::Float(value),
                    ramp_frames: 0,
                },
                timeline.frame,
            );
            if reply.code == ReplyCode::QueueFull {
                break;
            }
            self.next_id = next;
            self.unsent[slot] = None;
            if reply.code == ReplyCode::Accepted {
                self.ledger.push(reply);
            }
            summary.last_result = Some(reply);
        }
        for _ in 0..32 {
            if self.ledger.len() == LIMIT {
                break;
            }
            let Some(&(node, parameter, value)) = self.node_unsent.first() else {
                break;
            };
            let Some(processor) = bindings.node(node) else {
                break;
            };
            let Some(next) = self.next_id.checked_add(1) else {
                break;
            };
            let reply = control.submit(
                ParameterRequest {
                    request_id: self.next_id,
                    plan_revision: revision,
                    timeline_epoch: epoch,
                    target: ParameterKey {
                        processor,
                        parameter,
                    },
                    at: ApplyAt::NextBlock,
                    value,
                    ramp_frames: 0,
                },
                timeline.frame,
            );
            if reply.code == ReplyCode::QueueFull {
                break;
            }
            self.next_id = next;
            self.node_unsent.swap_remove(0);
            if reply.code == ReplyCode::Accepted {
                self.ledger.push(reply);
            }
            summary.last_result = Some(reply);
        }
        summary.pending_node_parameters = self.node_unsent.len();
        summary.accepted_pending = self.ledger.len();
        if self.unsent == [None, None] {
            summary.pending_gain_pan = None;
        }
    }
    /// The owner has conclusively joined before this is called. Normally every
    /// entry is covered by retired RT replies; a panicked output owner destroys
    /// its engine during unwind and cannot supply frame/apply evidence. Those
    /// remaining accepted requests end as StaleRevision, never invented Applied.
    pub fn reaped(
        &mut self,
        summary: &mut ControlSummary,
        replies: &mut std::vec::IntoIter<ControlReply>,
        budget: &mut usize,
    ) -> bool {
        while *budget > 0 {
            if let Some(reply) = replies.next() {
                self.terminal(summary, reply);
            } else if let Some(accepted) = self.ledger.pop() {
                summary.last_result = Some(ControlReply {
                    code: ReplyCode::StaleRevision,
                    ..accepted
                });
            } else {
                break;
            }
            *budget -= 1;
        }
        summary.accepted_pending = self.ledger.len();
        replies.len() == 0 && self.ledger.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{
        BackendError, GraphEdit, ServiceGraph, SessionReport, StartedState, TimelineSnapshot,
    };
    use moiren_core::graph::NodeKind;
    use moiren_engine::{
        boundary::ConstantSource, compiler::CompiledBindings, control::ControlPort,
    };

    struct Session(crate::session_graph::PreparedSessionGraph, bool);
    impl ActiveSession for Session {
        fn request_stop(&self) -> Result<(), BackendError> {
            Ok(())
        }
        fn is_finished(&self) -> bool {
            false
        }
        fn poll_started(&self) -> StartedState {
            StartedState::Running
        }
        fn control(&mut self) -> Option<(&mut ControlPort, &CompiledBindings)> {
            Some((&mut self.0.compiled.control, &self.0.compiled.bindings))
        }
        fn timeline(&self) -> Option<TimelineSnapshot> {
            let engine = &self.0.compiled.engine;
            Some(TimelineSnapshot {
                frame: engine.timeline(),
                epoch: engine.epoch(),
                revision: engine.revision(),
            })
        }
        fn gain_pan_nodes(&self) -> Option<(NodeId, NodeId)> {
            self.1.then_some(self.0.controls).flatten()
        }
        fn join(self: Box<Self>) -> Result<SessionReport, BackendError> {
            Ok(SessionReport::stopped())
        }
    }

    #[test]
    fn individual_node_edit_is_accepted_then_applied_by_the_audio_thread_without_a_primary_pair() {
        let mut graph = ServiceGraph::default();
        let snapshot = graph.snapshot();
        let gain = snapshot
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == NodeKind::Gain)
            .unwrap()
            .id();
        let mut session = Session(
            crate::session_graph::prepare_routing(
                &graph.preparation(),
                ConstantSource {
                    channels: 2,
                    value: 1.0,
                },
                32,
                None,
            )
            .unwrap(),
            false,
        );
        let mut parameters = Parameters::default();
        let mut summary = ControlSummary::default();
        graph
            .edit(GraphEdit::SetGain {
                node: gain,
                gain: 0.25,
            })
            .unwrap();
        assert!(parameters.set_node(&mut summary, gain, Gain::LEVEL, 0.25));
        assert_eq!(summary.pending_node_parameters, 1);
        parameters.poll(&mut session, &mut summary, 32);
        assert_eq!(summary.last_result.unwrap().code, ReplyCode::Accepted);
        assert_eq!(summary.accepted_pending, 1);
        assert_eq!(summary.pending_node_parameters, 0);
        session.0.compiled.engine.render(8).unwrap();
        let mut samples = [0.0; 16];
        session.0.output.read_interleaved(&mut samples).unwrap();
        assert_eq!(samples, [0.25; 16]);
        parameters.poll(&mut session, &mut summary, 32);
        assert_eq!(summary.last_result.unwrap().code, ReplyCode::Applied);
        assert_eq!(summary.accepted_pending, 0);
    }

    #[test]
    fn latest_primary_or_node_intent_wins_before_realtime_submission() {
        for node_is_latest in [false, true] {
            let graph = ServiceGraph::default();
            let (gain, _) = graph.preparation().gain_pan_nodes().unwrap();
            let mut session = Session(
                crate::session_graph::prepare_routing(
                    &graph.preparation(),
                    ConstantSource {
                        channels: 2,
                        value: 1.0,
                    },
                    32,
                    None,
                )
                .unwrap(),
                true,
            );
            let mut parameters = Parameters::default();
            let mut summary = ControlSummary::default();
            if node_is_latest {
                parameters.set(&mut summary, 0.5, 0.0);
                parameters.clear_primary_intent(&mut summary, 0);
                parameters.set_node(&mut summary, gain, Gain::LEVEL, 0.25);
            } else {
                parameters.set_node(&mut summary, gain, Gain::LEVEL, 0.25);
                parameters.clear_node_intent(&mut summary, gain, Gain::LEVEL);
                parameters.set(&mut summary, 0.5, 0.0);
            }
            parameters.poll(&mut session, &mut summary, 32);
            session.0.compiled.engine.render(8).unwrap();
            let mut samples = [0.0; 16];
            session.0.output.read_interleaved(&mut samples).unwrap();
            assert_eq!(samples, [if node_is_latest { 0.25 } else { 0.5 }; 16]);
            parameters.poll(&mut session, &mut summary, 32);
            assert_eq!(summary.last_result.unwrap().code, ReplyCode::Applied);
            assert_eq!(summary.accepted_pending, 0);
        }
    }

    #[test]
    fn mono_and_swap_are_real_boolean_controls_with_applied_audio_evidence() {
        let mut graph = ServiceGraph::default();
        let (gain, pan) = graph.preparation().gain_pan_nodes().unwrap();
        graph
            .edit(GraphEdit::SetGain {
                node: gain,
                gain: 1.0,
            })
            .unwrap();
        graph
            .edit(GraphEdit::SetPan {
                node: pan,
                pan: 0.5,
            })
            .unwrap();
        let mut session = Session(
            crate::session_graph::prepare_routing(
                &graph.preparation(),
                ConstantSource {
                    channels: 2,
                    value: 1.0,
                },
                32,
                None,
            )
            .unwrap(),
            true,
        );
        let mut parameters = Parameters::default();
        let mut summary = ControlSummary::default();
        for (parameter, value, expected) in [
            (Pan::SWAP, true, [1.0, 0.5]),
            (Pan::MONO, true, [0.75, 0.75]),
            (Pan::SWAP, false, [0.75, 0.75]),
            (Pan::MONO, false, [0.5, 1.0]),
        ] {
            assert!(parameters.set_node_value(
                &mut summary,
                pan,
                parameter,
                ParamValue::Bool(value)
            ));
            parameters.poll(&mut session, &mut summary, 32);
            assert_eq!(summary.last_result.unwrap().code, ReplyCode::Accepted);
            assert_eq!(summary.accepted_pending, 1);
            session.0.compiled.engine.render(8).unwrap();
            let mut samples = [0.0; 16];
            session.0.output.read_interleaved(&mut samples).unwrap();
            assert_eq!(samples.as_slice(), expected.repeat(8));
            parameters.poll(&mut session, &mut summary, 32);
            assert_eq!(summary.last_result.unwrap().code, ReplyCode::Applied);
            assert_eq!(summary.accepted_pending, 0);
        }
    }
}
