/// A registration failure leaves the directory, identities and budget unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    #[error("topic names must contain 1..=256 UTF-8 bytes and no NUL")]
    InvalidName,
    #[error("invalid capacity or bus configuration")]
    InvalidCapacity,
    #[error("storage layout arithmetic overflow")]
    SizeOverflow,
    #[error("bus byte budget exceeded")]
    BudgetExceeded,
    #[error("topic limit reached")]
    TopicLimit,
    #[error("topic name is already registered")]
    DuplicateName,
    #[error("identity generation exhausted")]
    GenerationExhausted,
    #[error("single publication copy layout exceeds the payload limit")]
    PayloadTooLarge,
}

/// Off-thread topic lookup distinguishes payload identity from transport mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LookupError {
    #[error("topic not found")]
    NotFound,
    #[error("topic payload type or value/block shape differs")]
    TypeMismatch,
    #[error("topic transport mode differs")]
    ModeMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubscribeError {
    #[error("topic is closed")]
    Closed,
    #[error("topic does not belong to this bus")]
    ForeignTopic,
    #[error("subscription options do not match the topic mode")]
    InvalidOptions,
    #[error("invalid subscription capacity")]
    InvalidCapacity,
    #[error("bus byte budget exceeded")]
    BudgetExceeded,
    #[error("business subscriber limit reached")]
    SubscriberLimit,
    #[error("activation generation exhausted")]
    DemandExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PublishError {
    #[error("topic is closed")]
    Closed,
    #[error("invalid publication length")]
    InvalidLength,
    #[error("publication sequence exhausted")]
    SequenceExhausted,
    #[error("publisher belongs to another host")]
    ForeignHost,
    #[error("publisher binding is inactive")]
    InactiveBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteUnavailable {
    NoDemand,
    Full { sequence: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("write unavailable: {0:?}")]
    Unavailable(WriteUnavailable),
    #[error(transparent)]
    Invalid(#[from] PublishError),
}

/// Publisher-visible result of a failed guard acquisition: an unavailable
/// write is a successful no-delivery outcome, while an invalid one surfaces.
pub(crate) fn flatten_write_error(
    error: WriteError,
) -> Result<crate::PublishOutcome, PublishError> {
    match error {
        WriteError::Unavailable(WriteUnavailable::NoDemand) => Ok(crate::PublishOutcome::NoDemand),
        WriteError::Unavailable(WriteUnavailable::Full { sequence }) => {
            Ok(crate::PublishOutcome::DroppedFull { sequence })
        }
        WriteError::Invalid(error) => Err(error),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("destination requires at least {required} elements")]
    DestinationTooSmall { required: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnregisterError {
    #[error("topic has a publisher binding or outstanding lifecycle leases")]
    Busy,
    #[error("topic is closed")]
    Closed,
    #[error("topic does not belong to this bus")]
    ForeignTopic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error("invalid host capacity")]
    InvalidCapacity,
    #[error("bus byte budget exceeded")]
    BudgetExceeded,
    #[error("topic belongs to another live publisher host or is duplicated")]
    PublisherConflict,
    #[error("host or topic is closed")]
    Closed,
    #[error("topic belongs to another bus")]
    ForeignTopic,
    #[error("fixed host capacity exceeded")]
    HostCapacity,
    #[error("binding identity exhausted")]
    BindingExhausted,
    #[error("unknown operation binding")]
    UnknownBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ActivateError {
    #[error("update belongs to another host")]
    ForeignHost,
    #[error("host or topic is closed")]
    Closed,
    #[error("update was prepared against a superseded basis")]
    StaleBasis,
    #[error("fixed host capacity exceeded")]
    HostCapacity,
    #[error("publisher ownership changed")]
    PublisherConflict,
}
