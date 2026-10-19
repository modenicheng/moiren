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
}
