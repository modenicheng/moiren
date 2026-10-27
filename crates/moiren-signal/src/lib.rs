//! Typed signal registration and bounded, fully initialized transport storage.
//!
//! Registration, subscription and name resolution are non-real-time operations.
//! Publication requires the unique host and a current operation scope.
//!
//! The Stream transport is spec-complete but carries no production traffic
//! yet: it is awaiting its first routed consumer (the planned UI waveform
//! view). Latest carries all current publication paths.

mod budget;
mod demand;
mod dispatcher;
mod error;
mod host;
mod lifecycle;
mod registry;
mod sequence;
mod storage;
mod subscriber;
#[cfg(test)]
mod test_alloc;
mod transport;
mod types;

pub use demand::DemandSnapshot;
pub use error::{
    ActivateError, HostError, LookupError, PublishError, ReadError, RegisterError, SubscribeError,
    UnregisterError, WriteError, WriteUnavailable,
};
pub use host::{
    ActivateFailure, ActivationTransaction, BindingKey, ErasedBinding, HostControl, HostId,
    OpSignalBindings, PreparedHostUpdate, PreparedPublisher, PublisherHandle, RetiredHostUpdate,
    RtHost, SignalBlockWriteGuard, SignalScope,
};
pub use registry::Bus;
pub use subscriber::{BlockMessage, BlockReadGuard, Subscriber, ValueMessage};
pub use transport::latest::BlockWriteGuard;
pub use transport::stream::StreamBlockWriteGuard;
pub use types::{
    BlockInit, BlockShape, BusConfig, LatestBlock, LatestValue, PublicationMeta, PublishOutcome,
    PublisherStatus, PumpBudget, PumpReport, Shape, StreamBlock, StreamValue, SubscriberStats,
    SubscriptionOptions, Topic, TopicId, TopicLifecycle, TopicStats, TopicStatus, ValueShape,
};
