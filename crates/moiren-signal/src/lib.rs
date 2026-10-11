//! Typed signal registration and bounded, fully initialized transport storage.
//!
//! Registration, subscription and name resolution are non-real-time operations.
//! Publication endpoints remain crate-private until the scoped host owns access.

mod budget;
mod demand;
mod dispatcher;
mod error;
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
    LookupError, PublishError, ReadError, RegisterError, SubscribeError, UnregisterError,
    WriteError, WriteUnavailable,
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
