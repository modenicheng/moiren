//! Typed signal registration and bounded, fully initialized transport storage.
//!
//! Registration and name resolution are non-real-time operations. Publication,
//! subscription and host authority are deliberately not exposed by this layer.

mod budget;
mod error;
mod lifecycle;
mod registry;
mod storage;
mod types;

pub use error::{LookupError, RegisterError};
pub use registry::Bus;
pub use types::{
    BlockInit, BlockShape, BusConfig, LatestBlock, LatestValue, PublicationMeta, Shape,
    StreamBlock, StreamValue, Topic, TopicId, ValueShape,
};
