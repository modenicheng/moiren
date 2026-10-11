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
