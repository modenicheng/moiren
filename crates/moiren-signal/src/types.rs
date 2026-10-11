use std::{any::TypeId, marker::PhantomData};

use crate::lifecycle::{DirectoryId, StateHandle};

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
/// Sequence zero denotes initialized storage, not a message. Published blocks
/// may have length zero; their nonzero sequence still identifies a publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicationMeta {
    pub sequence: u64,
    pub activation_generation: u64,
    pub len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicLifecycle {
    Registered,
    Closed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublisherStatus {
    Bound,
    NoPublisher,
}
/// Lifecycle and binding availability are independent of an empty read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopicStatus {
    pub lifecycle: TopicLifecycle,
    pub publisher: PublisherStatus,
}

impl PublicationMeta {
    pub(crate) const INITIAL: Self = Self {
        sequence: 0,
        activation_generation: 0,
        len: 0,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Published(PublicationMeta),
    NoDemand,
    DroppedFull { sequence: u64 },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubscriptionOptions {
    pub stream_depth: Option<usize>,
}

/// Directory identity, slot and registration generation; never a write capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TopicId {
    pub(crate) directory: DirectoryId,
    pub(crate) slot: usize,
    pub(crate) generation: u64,
}

/// A typed reference to a directory entry, without publication authority.
/// Clone and final release are non-RT operations; status/statistics survive closure.
#[derive(Debug)]
pub struct Topic<P: Shape> {
    pub(crate) id: TopicId,
    pub(crate) state: StateHandle,
    pub(crate) marker: PhantomData<fn() -> P>,
}

impl<P: Shape> Topic<P> {
    pub fn id(&self) -> TopicId {
        self.id
    }
    pub fn status(&self) -> TopicStatus {
        self.state.status()
    }
    pub fn stats(&self) -> TopicStats {
        TopicStats {
            attempts: self
                .state
                .attempt_sequence
                .load(std::sync::atomic::Ordering::Acquire),
            ingress_drops: self
                .state
                .ingress_drops
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

impl<P: Shape> PartialEq for Topic<P> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<P: Shape> Eq for Topic<P> {}
impl<P: Shape> Clone for Topic<P> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            state: self.state.clone(),
            marker: PhantomData,
        }
    }
}

/// Maximum paid work for one non-RT pump call. A delivery also covers a
/// source copy, one discarded ingress owner, or one recipient inspection.
#[derive(Debug, Clone, Copy)]
pub struct PumpBudget {
    pub topic_checks: usize,
    pub deliveries: usize,
    pub copy_bytes: usize,
}

/// `has_pending` conservatively includes unexamined directory slots when
/// checks run out. `required_copy_bytes` is the largest blocked atomic copy.
#[derive(Debug, Clone, Copy, Default)]
pub struct PumpReport {
    pub topic_checks: usize,
    pub deliveries: usize,
    pub copy_bytes: usize,
    pub has_pending: bool,
    pub required_copy_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TopicStats {
    pub attempts: u64,
    pub ingress_drops: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubscriberStats {
    pub delivered: u64,
    pub overflow_drops: u64,
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
pub trait ValueShape: Shape + sealed::ValueDispatch<Self::Value> {
    type Value: Copy + Send + Sync + 'static;
}
pub trait BlockShape: Shape + sealed::BlockDispatch<Self::Header, Self::Element> {
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

#[allow(private_interfaces)]
pub(crate) mod sealed {
    use std::any::TypeId;
    pub trait Sealed {
        fn demand_access(
            access: crate::host::slot::Access<'_>,
        ) -> Result<crate::DemandSnapshot, crate::PublishError>;
        fn bind_cell(
            cell: &crate::host::slot::CellRoot,
        ) -> (
            crate::host::slot::Pointer,
            fn(crate::host::slot::Pointer, crate::DemandSnapshot),
        );
        const STREAM: bool;
        fn payload_type() -> (TypeId, Option<TypeId>);
        fn subscribe(
            bus: &mut crate::Bus,
            topic: &crate::Topic<Self>,
            options: crate::SubscriptionOptions,
        ) -> Result<crate::Subscriber<Self>, crate::SubscribeError>
        where
            Self: crate::Shape + Sized;
    }
    pub trait ValueDispatch<Value> {
        fn publish_access(
            access: crate::host::slot::Access<'_>,
            value: Value,
        ) -> Result<crate::PublishOutcome, crate::PublishError>;
    }
    pub trait BlockDispatch<Header: Copy + Send + 'static, Element: Copy + Send + 'static> {
        type Guard<'a>;
        fn typed_write_access(
            access: crate::host::slot::Access<'_>,
            max_len: usize,
        ) -> Result<Self::Guard<'_>, crate::WriteError>;
        fn write_access(
            access: crate::host::slot::Access<'_>,
            max_len: usize,
        ) -> Result<crate::SignalBlockWriteGuard<'_, Header, Element>, crate::WriteError>;
    }
}

macro_rules! value_shape {
    ($shape:ident, $mode:ident) => {
        #[allow(private_interfaces)]
        impl<T: Copy + Send + Sync + 'static> sealed::Sealed for $shape<T> {
            fn demand_access(
                access: crate::host::slot::Access<'_>,
            ) -> Result<crate::DemandSnapshot, crate::PublishError> {
                access.demand::<crate::storage::ValueSlot<T>>()
            }
            fn bind_cell(
                cell: &crate::host::slot::CellRoot,
            ) -> (
                crate::host::slot::Pointer,
                fn(crate::host::slot::Pointer, crate::DemandSnapshot),
            ) {
                (
                    cell.pointer::<crate::storage::ValueSlot<T>>(),
                    crate::host::slot::refresh_typed::<crate::storage::ValueSlot<T>>,
                )
            }
            const STREAM: bool = matches!(Mode::$mode, Mode::Stream);
            fn payload_type() -> (TypeId, Option<TypeId>) {
                (TypeId::of::<T>(), None)
            }
            fn subscribe(
                bus: &mut crate::Bus,
                topic: &crate::Topic<Self>,
                options: crate::SubscriptionOptions,
            ) -> Result<crate::Subscriber<Self>, crate::SubscribeError> {
                if Self::STREAM {
                    return bus
                        .subscribe_stream::<Self, crate::storage::ValueSlot<T>>(topic, options);
                }
                bus.subscribe_latest::<Self, crate::storage::ValueSlot<T>>(topic, options)
            }
        }
        impl<T: Copy + Send + Sync + 'static> Shape for $shape<T> {}
        #[allow(private_interfaces)]
        impl<T: Copy + Send + Sync + 'static> sealed::ValueDispatch<T> for $shape<T> {
            fn publish_access(
                access: crate::host::slot::Access<'_>,
                value: T,
            ) -> Result<PublishOutcome, crate::PublishError> {
                let (writer, state) = access.writer::<crate::storage::ValueSlot<T>>();
                if matches!(Mode::$mode, Mode::Stream) {
                    crate::transport::stream::StreamEndpoint::new(writer, state).publish(value)
                } else {
                    crate::transport::latest::LatestEndpoint::new(writer, state).publish(value)
                }
            }
        }
        impl<T: Copy + Send + Sync + 'static> ValueShape for $shape<T> {
            type Value = T;
        }
    };
}
macro_rules! block_shape {
    ($shape:ident, $mode:ident, $module:ident, $endpoint:ident, $guard:ident) => {
        #[allow(private_interfaces)]
        impl<H: Copy + Send + Sync + 'static, E: Copy + Send + Sync + 'static> sealed::Sealed
            for $shape<H, E>
        {
            fn demand_access(
                access: crate::host::slot::Access<'_>,
            ) -> Result<crate::DemandSnapshot, crate::PublishError> {
                access.demand::<crate::storage::BlockSlot<H, E>>()
            }
            fn bind_cell(
                cell: &crate::host::slot::CellRoot,
            ) -> (
                crate::host::slot::Pointer,
                fn(crate::host::slot::Pointer, crate::DemandSnapshot),
            ) {
                (
                    cell.pointer::<crate::storage::BlockSlot<H, E>>(),
                    crate::host::slot::refresh_typed::<crate::storage::BlockSlot<H, E>>,
                )
            }
            const STREAM: bool = matches!(Mode::$mode, Mode::Stream);
            fn payload_type() -> (TypeId, Option<TypeId>) {
                (TypeId::of::<H>(), Some(TypeId::of::<E>()))
            }
            fn subscribe(
                bus: &mut crate::Bus,
                topic: &crate::Topic<Self>,
                options: crate::SubscriptionOptions,
            ) -> Result<crate::Subscriber<Self>, crate::SubscribeError> {
                if Self::STREAM {
                    return bus
                        .subscribe_stream::<Self, crate::storage::BlockSlot<H, E>>(topic, options);
                }
                bus.subscribe_latest::<Self, crate::storage::BlockSlot<H, E>>(topic, options)
            }
        }
        impl<H: Copy + Send + Sync + 'static, E: Copy + Send + Sync + 'static> Shape
            for $shape<H, E>
        {
        }
        #[allow(private_interfaces)]
        impl<H: Copy + Send + Sync + 'static, E: Copy + Send + Sync + 'static>
            sealed::BlockDispatch<H, E> for $shape<H, E>
        {
            type Guard<'a> = crate::$guard<'a, H, E>;
            fn typed_write_access(
                access: crate::host::slot::Access<'_>,
                max_len: usize,
            ) -> Result<Self::Guard<'_>, crate::WriteError> {
                let (writer, state) = access.writer::<crate::storage::BlockSlot<H, E>>();
                crate::transport::$module::$endpoint::new(writer, state).try_write_owned(max_len)
            }
            fn write_access(
                access: crate::host::slot::Access<'_>,
                max_len: usize,
            ) -> Result<crate::SignalBlockWriteGuard<'_, H, E>, crate::WriteError> {
                Self::typed_write_access(access, max_len).map(crate::SignalBlockWriteGuard::$mode)
            }
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
block_shape!(LatestBlock, Latest, latest, LatestEndpoint, BlockWriteGuard);
block_shape!(
    StreamBlock,
    Stream,
    stream,
    StreamEndpoint,
    StreamBlockWriteGuard
);
