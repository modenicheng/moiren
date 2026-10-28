use super::*;

impl ManagedDriver {
    pub(super) fn clear_parameters(&mut self, c: &mut ServiceContext) {
        self.unsent.clear();
        self.parameters.clear();
        self.confirm_parameters = None;
        self.confirmation_values.clear();
        // The next owner compiles strip state from the candidate, so the
        // live-value cache must re-seed from it instead of stale submissions.
        self.strip_live.clear();
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
            c.graph
                .record_error("The live parameter queue is busy; the candidate value is kept");
        }
        c.control.pending_node_parameters = self.unsent.len();
    }

    /// Submit the effective strip values (dim-folded gain, solo-audible mute)
    /// changed by this transaction. `edited` always submits; solo changes fan
    /// out by comparing against the pre-edit candidate. The diagnostic cache
    /// may be empty after startup or stale after Apply, so it is not a baseline.
    pub(super) fn sync_effective_strip_parameters(
        &mut self,
        c: &mut ServiceContext,
        edited: NodeId,
        before: &BTreeMap<NodeId, (f64, bool)>,
    ) {
        if self
            .owner
            .as_ref()
            .is_none_or(|owner| owner.document != self.document)
        {
            return;
        }
        let configs: BTreeMap<NodeId, moiren_engine::processor::StripConfig> =
            c.graph.mixer_configs().into_iter().collect();
        for (node, config) in &configs {
            let desired = (config.gain, config.muted);
            let changed = *node == edited || before.get(node) != Some(&desired);
            if changed {
                for (parameter, value) in [
                    (StripParameter::Gain, ParamValue::Float(config.gain)),
                    (
                        StripParameter::EffectiveMute,
                        ParamValue::Bool(config.muted),
                    ),
                ] {
                    self.submit_parameter(
                        c,
                        ParameterTarget::Strip {
                            node: *node,
                            parameter,
                        },
                        value,
                    );
                }
            }
        }
        self.strip_live = configs
            .into_iter()
            .map(|(node, config)| (node, (config.gain, config.muted)))
            .collect();
    }
}
