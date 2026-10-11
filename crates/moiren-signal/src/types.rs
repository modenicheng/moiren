use std::{any::TypeId, marker::PhantomData};

use crate::lifecycle::DirectoryId;

/// Resource limits, fixed for a bus's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusConfig {
    pub byte_budget: usize,
    pub max_topics: usize,
    pub max_subscribers_per_topic: usize,
    pub default_stream_depth: usize,
    pub max_payload_bytes: usize,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            byte_budget: 32 * 1024 * 1024,
            max_topics: 128,
            max_subscribers_per_topic: 64,
            default_stream_depth: 8,
            max_payload_bytes: 1024 * 1024,
        }
    }
}

/// Valid initialization values for every element in a fixed-capacity block.
#[derive(Debug, Clone, Copy)]
pub struct BlockInit<H, E> {
    pub header: H,
    pub element: E,
    pub capacity: usize,
}

/// Transport metadata; business headers remain in the typed payload.
///
/// Sequence zero and length zero describe initialized storage, not a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicationMeta {
    pub sequence: u64,
    pub activation_generation: u64,
    pub len: usize,
}

impl PublicationMeta {
    pub(crate) const INITIAL: Self = Self {
        sequence: 0,
        activation_generation: 0,
        len: 0,
    };
}

/// Directory identity, slot and registration generation; never a write capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TopicId {
    pub(crate) directory: DirectoryId,
    pub(crate) slot: usize,
    pub(crate) generation: u64,
}

/// A typed reference to a directory entry, without publication authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Topic<P: Shape> {
    pub(crate) id: TopicId,
    pub(crate) marker: PhantomData<fn() -> P>,
}

impl<P: Shape> Topic<P> {
    pub fn id(&self) -> TopicId {
        self.id
    }
}

/// The four supported signal shapes. External implementations are sealed out.
///
/// ```compile_fail
/// struct ExternalShape;
/// impl moiren_signal::Shape for ExternalShape {}
/// ```
pub trait Shape: sealed::Sealed + Send + Sync + 'static {}
/// A single Copy payload; owned strings and vectors are not accepted.
///
/// ```compile_fail
/// let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
/// bus.register_latest_value("owned", String::from("heap payload"));
/// ```
pub trait ValueShape: Shape {
    type Value: Copy + Send + Sync + 'static;
}
pub trait BlockShape: Shape {
    type Header: Copy + Send + Sync + 'static;
    type Element: Copy + Send + Sync + 'static;
}

#[derive(Debug, Clone, Copy)]
pub struct LatestValue<T>(PhantomData<fn() -> T>);
#[derive(Debug, Clone, Copy)]
pub struct StreamValue<T>(PhantomData<fn() -> T>);
#[derive(Debug, Clone, Copy)]
pub struct LatestBlock<H, E>(PhantomData<fn() -> (H, E)>);
#[derive(Debug, Clone, Copy)]
pub struct StreamBlock<H, E>(PhantomData<fn() -> (H, E)>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Latest,
    Stream,
}

pub(crate) type PayloadType = (TypeId, Option<TypeId>);

impl Mode {
    pub(crate) fn of<P: Shape>() -> Self {
        if P::STREAM {
            Self::Stream
        } else {
            Self::Latest
        }
    }
}

pub(crate) mod sealed {
    use std::any::TypeId;
    pub trait Sealed {
        const STREAM: bool;
        fn payload_type() -> (TypeId, Option<TypeId>);
    }
}

macro_rules! value_shape {
    ($shape:ident, $mode:ident) => {
        impl<T: Copy + Send + Sync + 'static> sealed::Sealed for $shape<T> {
            const STREAM: bool = matches!(Mode::$mode, Mode::Stream);
            fn payload_type() -> (TypeId, Option<TypeId>) {
                (TypeId::of::<T>(), None)
            }
        }
        impl<T: Copy + Send + Sync + 'static> Shape for $shape<T> {}
        impl<T: Copy + Send + Sync + 'static> ValueShape for $shape<T> {
            type Value = T;
        }
    };
}
macro_rules! block_shape {
    ($shape:ident, $mode:ident) => {
        impl<H: Copy + Send + Sync + 'static, E: Copy + Send + Sync + 'static> sealed::Sealed
            for $shape<H, E>
        {
            const STREAM: bool = matches!(Mode::$mode, Mode::Stream);
            fn payload_type() -> (TypeId, Option<TypeId>) {
                (TypeId::of::<H>(), Some(TypeId::of::<E>()))
            }
        }
        impl<H: Copy + Send + Sync + 'static, E: Copy + Send + Sync + 'static> Shape
            for $shape<H, E>
        {
        }
        impl<H: Copy + Send + Sync + 'static, E: Copy + Send + Sync + 'static> BlockShape
            for $shape<H, E>
        {
            type Header = H;
            type Element = E;
        }
    };
}
value_shape!(LatestValue, Latest);
value_shape!(StreamValue, Stream);
block_shape!(LatestBlock, Latest);
block_shape!(StreamBlock, Stream);
