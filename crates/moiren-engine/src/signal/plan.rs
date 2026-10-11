use moiren_signal::{ErasedBinding, HostControl, HostError, OpSignalBindings, PreparedHostUpdate};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SignalPlanError {
    #[error("one operation's signal publishers use more than one binding key")]
    MultipleBindingKeys,
    #[error(transparent)]
    Host(#[from] HostError),
}

/// Non-RT prepared signal ownership attached to one execution plan.
pub(crate) struct PreparedSignalPlan {
    pub(crate) update: PreparedHostUpdate,
    pub(crate) operations: Box<[Option<OpSignalBindings>]>,
}

impl PreparedSignalPlan {
    pub(crate) fn compatible_bindings(left: &[ErasedBinding], right: &[ErasedBinding]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .all(|binding| right.iter().any(|other| binding.schema() == other.schema()))
    }
    pub(crate) fn into_parts(self) -> (PreparedHostUpdate, Box<[Option<OpSignalBindings>]>) {
        (self.update, self.operations)
    }

    pub(crate) fn prepare(
        host: &HostControl,
        operations: &[Box<[ErasedBinding]>],
    ) -> Result<Self, SignalPlanError> {
        let all: Vec<ErasedBinding> = operations
            .iter()
            .flat_map(|bindings| bindings.iter().cloned())
            .collect();
        let update = host.prepare_update(&all)?;
        let operation_tables = operations
            .iter()
            .map(|bindings| {
                if bindings.is_empty() {
                    Ok(None)
                } else {
                    let key = bindings[0].key();
                    if bindings.iter().any(|binding| binding.key() != key) {
                        return Err(SignalPlanError::MultipleBindingKeys);
                    }
                    update
                        .op_bindings(key)?
                        .for_operation(bindings)
                        .map(Some)
                        .map_err(SignalPlanError::Host)
                }
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        Ok(Self {
            update,
            operations: operation_tables,
        })
    }
}
