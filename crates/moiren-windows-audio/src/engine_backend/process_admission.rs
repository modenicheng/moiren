use super::*;
use crate::{
    latency::transport::TransportInputPolicy,
    process_loopback::{MeasuredProcessDelivery, ProcessBridgeOptions, ProcessIdentity},
};

pub(super) struct MeasuredBridgeRequest {
    node: NodeId,
    expected_process: ProcessIdentity,
    source_generation: u64,
    evidence: MeasuredProcessDelivery,
    input: TransportInputPolicy,
    evaluated_envelope: Option<ProcessBridgeOptions>,
}

impl<S: ProcessingSample> ProfessionalBackend<S> {
    /// Reuse explicitly selected evidence when preparing a replacement owner.
    /// Revalidate against that owner's complete negotiated consumption burst;
    /// an insufficient measured target fails instead of using the global policy.
    pub fn with_measured_process_bridge(
        mut self,
        node: NodeId,
        expected_process: &ProcessIdentity,
        source_generation: u64,
        evidence: MeasuredProcessDelivery,
        input: TransportInputPolicy,
    ) -> Result<Self, BackendError> {
        if self
            .measured_process_bridges
            .iter()
            .any(|request| request.node == node)
        {
            return Err(unsupported("duplicate measured Process bridge request"));
        }
        evidence
            .admit_for_consumer(
                expected_process,
                source_generation,
                input,
                crate::NATIVE_SAMPLE_RATE,
                1,
            )
            .map_err(unsupported)?;
        self.measured_process_bridges.push(MeasuredBridgeRequest {
            node,
            expected_process: expected_process.clone(),
            source_generation,
            evidence,
            input,
            evaluated_envelope: None,
        });
        Ok(self)
    }

    /// Preserve an independently replayed candidate's exact rate/demand envelope.
    /// Native negotiation may use less demand, but cannot silently grow beyond
    /// what the historical window assessed, even if the target could fit it.
    pub fn with_evaluated_process_bridge(
        self,
        node: NodeId,
        expected_process: &ProcessIdentity,
        source_generation: u64,
        evidence: MeasuredProcessDelivery,
        options: ProcessBridgeOptions,
    ) -> Result<Self, BackendError> {
        let admitted = evidence
            .admit_for_consumer(
                expected_process,
                source_generation,
                options.input,
                options.engine_sample_rate,
                options.max_demand_frames,
            )
            .map_err(unsupported)?;
        if admitted != options {
            return Err(unsupported(
                "evaluated Process options do not match measured admission",
            ));
        }
        let mut backend = self.with_measured_process_bridge(
            node,
            expected_process,
            source_generation,
            evidence,
            options.input,
        )?;
        backend
            .measured_process_bridges
            .last_mut()
            .unwrap()
            .evaluated_envelope = Some(options);
        Ok(backend)
    }

    pub(super) fn validate_measured_sources(
        &self,
        project: &moiren_engine::host::EngineProject,
    ) -> Result<(), BackendError> {
        for request in &self.measured_process_bridges {
            let Some(NodeConfig::Input(input)) = project.nodes.get(&request.node) else {
                return Err(unsupported(
                    "measured Process input was removed or replaced",
                ));
            };
            let InputSource::Application { executable, .. } = &input.source else {
                return Err(unsupported("measured Process input was replaced"));
            };
            let identity = process_selector(executable)?;
            if identity
                != (
                    request.expected_process.pid,
                    request.expected_process.creation_time_100ns,
                )
            {
                return Err(unsupported(
                    "measured Process identity no longer matches the input node",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn process_bridge_for(
        &self,
        node: NodeId,
        identity: (u32, u64),
        rate: u32,
        complete_demand: u32,
    ) -> Result<ProcessBridgeOptions, BackendError> {
        let Some(request) = self
            .measured_process_bridges
            .iter()
            .find(|request| request.node == node)
        else {
            return self
                .process_bridge
                .for_consumer(rate, complete_demand)
                .map_err(unsupported);
        };
        if identity
            != (
                request.expected_process.pid,
                request.expected_process.creation_time_100ns,
            )
        {
            return Err(unsupported(
                "measured Process identity no longer matches the input node",
            ));
        }
        if let Some(envelope) = request.evaluated_envelope {
            if rate != envelope.engine_sample_rate || complete_demand > envelope.max_demand_frames {
                return Err(unsupported(
                    "native Process demand exceeds the evaluated rate/whole-wake envelope",
                ));
            }
            return Ok(envelope);
        }
        request
            .evidence
            .admit_for_consumer(
                &request.expected_process,
                request.source_generation,
                request.input,
                rate,
                complete_demand,
            )
            .map_err(unsupported)
    }

    pub(super) fn measured_source_generation(&self, node: NodeId) -> Option<u64> {
        self.measured_process_bridges
            .iter()
            .find(|request| request.node == node)
            .map(|request| request.source_generation)
    }
}

#[cfg(test)]
mod tests;
