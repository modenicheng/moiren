use super::*;

impl ManagedDriver {
    pub(super) fn clear_parameters(&mut self, c: &mut ServiceContext) {
        self.unsent.clear();
        self.parameters.clear();
        self.confirm_parameters = None;
        self.confirmation_values.clear();
        c.control = ControlSummary::default();
    }
    pub(super) fn submit_parameter(
        &mut self,
        c: &mut ServiceContext,
        target: ParameterTarget,
        value: ParamValue,
    ) {
        if self
            .owner
            .as_ref()
            .is_none_or(|owner| owner.document != self.document)
        {
            return;
        }
        if let Some(confirmation) = self.confirm_parameters.take() {
            self.confirmation_values.clear();
            if c.graph.snapshot().pending_revision == Some(confirmation.revision) {
                c.graph.clear_pending();
            }
        }
        if let Some(intent) = self.unsent.iter_mut().find(|intent| intent.0 == target) {
            intent.1 = value;
        } else if self.unsent.len() < 64 {
            self.unsent.push((target, value));
        } else {
            c.graph.record_error("实时参数队列繁忙；候选值已保留");
        }
        c.control.pending_node_parameters = self.unsent.len();
    }
    pub(super) fn parameter_edit(edit: &GraphEdit) -> Vec<(ParameterTarget, ParamValue)> {
        use moiren_engine::{
            host::SendParameter,
            processor::{Gain, Pan},
        };
        let node =
            |node, parameter, value| vec![(ParameterTarget::Node { node, parameter }, value)];
        match edit {
            GraphEdit::SetCompressor {
                node: id,
                parameter,
                value,
            } => node(*id, parameter.id(), ParamValue::Float(*value)),
            GraphEdit::SetGain { node: id, gain } => {
                node(*id, Gain::LEVEL, ParamValue::Float(*gain))
            }
            GraphEdit::SetPan { node: id, pan } => {
                node(*id, Pan::POSITION, ParamValue::Float(*pan))
            }
            GraphEdit::SetMono { node: id, mono } => node(*id, Pan::MONO, ParamValue::Bool(*mono)),
            GraphEdit::SetSwap { node: id, swap } => node(*id, Pan::SWAP, ParamValue::Bool(*swap)),
            GraphEdit::SetSend { edge, params } => vec![
                (
                    ParameterTarget::Edge {
                        edge: *edge,
                        parameter: SendParameter::Gain,
                    },
                    ParamValue::Float(params.gain),
                ),
                (
                    ParameterTarget::Edge {
                        edge: *edge,
                        parameter: SendParameter::Pan,
                    },
                    ParamValue::Float(params.pan),
                ),
                (
                    ParameterTarget::Edge {
                        edge: *edge,
                        parameter: SendParameter::Mute,
                    },
                    ParamValue::Bool(params.mute),
                ),
            ],
            _ => Vec::new(),
        }
    }
}
