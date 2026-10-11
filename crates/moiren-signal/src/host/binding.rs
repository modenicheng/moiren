use super::slot::{CellRoot, Owner, Pointer};
use crate::{
    Shape, TopicId,
    budget::Charge,
    lifecycle::{StateHandle, TopicLease},
};
use std::marker::PhantomData;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostId(pub(crate) u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingKey {
    pub(crate) host: HostId,
    pub(crate) value: u64,
}

pub(crate) struct SlotRoot {
    pub(crate) state: StateHandle,
    pub(crate) cell: CellRoot,
}

pub(crate) struct BindingOwner {
    pub(crate) host: HostId,
    pub(crate) key: BindingKey,
    pub(crate) identity: u64,
    pub(crate) topic: TopicId,
    pub(crate) pointer: Pointer,
    pub(crate) refresh: fn(Pointer, crate::DemandSnapshot),
    pub(crate) slot: Owner<SlotRoot>,
    pub(crate) _lease: TopicLease,
}

/// Typed identity only. Publication requires an active operation scope.
///
/// Latest value function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestValue, PublisherHandle};
/// fn coerce_latest_value(original: PublisherHandle<LatestValue<fn(&u32)>>)
///     -> PublisherHandle<LatestValue<fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Stream value function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamValue, PublisherHandle};
/// fn coerce_stream_value(original: PublisherHandle<StreamValue<fn(&u32)>>)
///     -> PublisherHandle<StreamValue<fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Latest block header function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestBlock, PublisherHandle};
/// fn coerce_latest_block_header(original: PublisherHandle<LatestBlock<fn(&u32), u32>>)
///     -> PublisherHandle<LatestBlock<fn(&'static u32), u32>>
/// {
///     original
/// }
/// ```
///
/// Latest block element function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestBlock, PublisherHandle};
/// fn coerce_latest_block_element(original: PublisherHandle<LatestBlock<u32, fn(&u32)>>)
///     -> PublisherHandle<LatestBlock<u32, fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Stream block header function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamBlock, PublisherHandle};
/// fn coerce_stream_block_header(original: PublisherHandle<StreamBlock<fn(&u32), u32>>)
///     -> PublisherHandle<StreamBlock<fn(&'static u32), u32>>
/// {
///     original
/// }
/// ```
///
/// Stream block element function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamBlock, PublisherHandle};
/// fn coerce_stream_block_element(original: PublisherHandle<StreamBlock<u32, fn(&u32)>>)
///     -> PublisherHandle<StreamBlock<u32, fn(&'static u32)>>
/// {
///     original
/// }
/// ```
pub struct PublisherHandle<P: Shape> {
    pub(crate) host: HostId,
    pub(crate) key: BindingKey,
    pub(crate) identity: u64,
    pub(crate) topic: TopicId,
    pub(crate) pointer: Pointer,
    // The host casts this token back to its prepared typed cell; keep P
    // invariant so a safe lifetime coercion cannot change that payload type.
    pub(crate) marker: PhantomData<fn(P) -> P>,
}
impl<P: Shape> Copy for PublisherHandle<P> {}
impl<P: Shape> Clone for PublisherHandle<P> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Preparation-side lease. Clone/erase/drop are non-RT operations.
///
/// Latest value function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestValue, PreparedPublisher};
/// fn coerce_latest_value(original: PreparedPublisher<LatestValue<fn(&u32)>>)
///     -> PreparedPublisher<LatestValue<fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Stream value function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamValue, PreparedPublisher};
/// fn coerce_stream_value(original: PreparedPublisher<StreamValue<fn(&u32)>>)
///     -> PreparedPublisher<StreamValue<fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Latest block header function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestBlock, PreparedPublisher};
/// fn coerce_latest_block_header(original: PreparedPublisher<LatestBlock<fn(&u32), u32>>)
///     -> PreparedPublisher<LatestBlock<fn(&'static u32), u32>>
/// {
///     original
/// }
/// ```
///
/// Latest block element function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestBlock, PreparedPublisher};
/// fn coerce_latest_block_element(original: PreparedPublisher<LatestBlock<u32, fn(&u32)>>)
///     -> PreparedPublisher<LatestBlock<u32, fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Stream block header function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamBlock, PreparedPublisher};
/// fn coerce_stream_block_header(original: PreparedPublisher<StreamBlock<fn(&u32), u32>>)
///     -> PreparedPublisher<StreamBlock<fn(&'static u32), u32>>
/// {
///     original
/// }
/// ```
///
/// Stream block element function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamBlock, PreparedPublisher};
/// fn coerce_stream_block_element(original: PreparedPublisher<StreamBlock<u32, fn(&u32)>>)
///     -> PreparedPublisher<StreamBlock<u32, fn(&'static u32)>>
/// {
///     original
/// }
/// ```
pub struct PreparedPublisher<P: Shape> {
    pub(crate) binding: ErasedBinding,
    // Prepared authority and its derived handle must have identical P types.
    pub(crate) marker: PhantomData<fn(P) -> P>,
}
impl<P: Shape> PreparedPublisher<P> {
    pub fn handle(&self) -> PublisherHandle<P> {
        let owner = &self.binding.owner;
        PublisherHandle {
            host: owner.host,
            key: owner.key,
            identity: owner.identity,
            topic: owner.topic,
            pointer: owner.pointer,
            marker: PhantomData,
        }
    }
    pub fn erase(&self) -> ErasedBinding {
        self.binding.clone()
    }
}

#[derive(Clone)]
pub struct ErasedBinding {
    pub(crate) owner: Owner<BindingOwner>,
}

/// Prevalidated per-operation table. Creating or dropping it is non-RT.
pub struct OpSignalBindings {
    pub(crate) table: u64,
    pub(crate) host: HostId,
    pub(crate) key: BindingKey,
    pub(crate) bindings: Box<[ErasedBinding]>,
    pub(crate) _root: Owner<super::HostRoot>,
    pub(crate) _charge: Charge,
}
