//! Typed signal registration and bounded, fully initialized transport storage.
//!
//! Registration, subscription and name resolution are non-real-time operations.
//! Publication endpoints remain crate-private until the scoped host owns access.

mod budget;
mod demand;
mod error;
mod lifecycle;
mod registry;
mod sequence;
mod storage;
mod subscriber;
mod transport;
mod types;

pub use demand::DemandSnapshot;
pub use error::{
    LookupError, PublishError, RegisterError, SubscribeError, WriteError, WriteUnavailable,
};
pub use registry::Bus;
pub use subscriber::{BlockReadGuard, Subscriber, ValueMessage};
pub use transport::latest::BlockWriteGuard;
pub use types::{
    BlockInit, BlockShape, BusConfig, LatestBlock, LatestValue, PublicationMeta, PublishOutcome,
    Shape, StreamBlock, StreamValue, SubscriptionOptions, Topic, TopicId, ValueShape,
};
