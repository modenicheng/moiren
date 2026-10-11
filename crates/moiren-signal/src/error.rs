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
