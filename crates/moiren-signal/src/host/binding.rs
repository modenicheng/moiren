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
pub struct PublisherHandle<P: Shape> {
    pub(crate) host: HostId,
    pub(crate) key: BindingKey,
    pub(crate) identity: u64,
    pub(crate) topic: TopicId,
    pub(crate) pointer: Pointer,
    pub(crate) marker: PhantomData<fn() -> P>,
}
impl<P: Shape> Copy for PublisherHandle<P> {}
impl<P: Shape> Clone for PublisherHandle<P> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Preparation-side lease. Clone/erase/drop are non-RT operations.
pub struct PreparedPublisher<P: Shape> {
    pub(crate) binding: ErasedBinding,
    pub(crate) marker: PhantomData<fn() -> P>,
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
