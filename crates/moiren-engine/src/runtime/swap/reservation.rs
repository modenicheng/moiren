use super::*;
use crate::{buffer::ProcessIo, control::ProcessParameters, processor::ProcessContext};

struct ReservedIo {
    schema: ProcessorSchema,
}

impl<S: ProcessingSample> RtProcessor<S> for ReservedIo {
    fn state_type_id(&self) -> TypeId {
        self.schema.state_type
    }
    fn role(&self) -> ProcessorRole {
        self.schema.role
    }
    fn latency_frames(&self) -> u32 {
        self.schema.latency
    }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if self.schema.io.as_ref() == Some(&IoSchema::from_io(io)) {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }
    fn signal_bindings(&self) -> &[moiren_signal::ErasedBinding] {
        &self.schema.signals
    }
    fn process(
        &mut self,
        _: &ProcessContext,
        _: ProcessIo<'_, S>,
        _: ProcessParameters<'_>,
        _: ProcessSignals<'_>,
    ) {
        unreachable!("unbound reservations cannot render")
    }
}

impl PlanSnapshot {
    /// Prepared sink identities, without inspecting or borrowing live IO.
    pub fn sink_processors(&self) -> impl Iterator<Item = ProcessorId> + '_ {
        self.0
            .processors
            .iter()
            .filter(|schema| schema.role == ProcessorRole::Sink)
            .map(|schema| schema.id)
    }

    pub(crate) fn processor_role(&self, processor: ProcessorId) -> Option<ProcessorRole> {
        self.0
            .processors
            .iter()
            .find(|schema| schema.id == processor)
            .map(|schema| schema.role)
    }

    /// The reservation owns only immutable metadata, never a live bridge or
    /// DSP borrow. Exact basis identity is checked again before activation.
    pub(crate) fn reserve_io<S: ProcessingSample>(
        &self,
        previous: ProcessorId,
        next: ProcessorId,
    ) -> Result<(ProcessorInstance<S>, Vec<ParamSpec>), PlanSwapError> {
        let schema = self
            .0
            .processors
            .iter()
            .find(|schema| schema.id == previous)
            .ok_or(PlanSwapError::MissingProcessor)?;
        if !matches!(schema.role, ProcessorRole::Source | ProcessorRole::Sink) {
            return Err(PlanSwapError::IncompatibleProcessor);
        }
        let mut parameters: Vec<_> = self
            .0
            .parameters
            .iter()
            .copied()
            .filter(|parameter| parameter.key.processor == previous)
            .collect();
        for parameter in &mut parameters {
            parameter.key.processor = next;
        }
        Ok((
            ProcessorInstance {
                id: next,
                state_type: schema.state_type,
                processor: Box::new(ReservedIo {
                    schema: schema.clone(),
                }),
                reservation: Some((self.clone(), previous)),
            },
            parameters,
        ))
    }
}
