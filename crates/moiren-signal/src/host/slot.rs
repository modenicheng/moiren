//! Stable producer allocation and the only production unsafe boundary.
use std::{any::Any, cell::UnsafeCell, marker::PhantomData, sync::Arc};

use crate::{DemandSnapshot, budget::Charge, sequence::Sequence, storage::PoolWriter};

pub(crate) struct Owner<Resource> {
    resource: Option<Arc<Resource>>,
    _charge: Charge,
}
impl<Resource> Owner<Resource> {
    pub(crate) fn new(resource: Resource, charge: Charge) -> Self {
        Self {
            resource: Some(Arc::new(resource)),
            _charge: charge,
        }
    }
}
impl<Resource> Clone for Owner<Resource> {
    fn clone(&self) -> Self {
        Self {
            resource: self.resource.clone(),
            _charge: self._charge.clone(),
        }
    }
}
impl<Resource> std::ops::Deref for Owner<Resource> {
    type Target = Resource;
    fn deref(&self) -> &Resource {
        self.resource.as_ref().expect("live owner")
    }
}
impl<Resource> Drop for Owner<Resource> {
    fn drop(&mut self) {
        if let Some(resource) = Arc::into_inner(self.resource.take().expect("live owner")) {
            // First free the Arc allocation, then all its resource fields, and
            // finally this owner's reservation. All final releases are non-RT.
            drop(resource);
        }
    }
}

pub(crate) enum Producer<S: Clone + Send> {
    Latest(triple_buffer::Input<S>),
    Stream(PoolWriter<S>),
}

pub(crate) struct Writer<S: Clone + Send> {
    pub(crate) producer: Producer<S>,
    pub(crate) sequence: Sequence,
    pub(crate) demand: DemandSnapshot,
    pub(crate) capacity: usize,
}

pub(crate) struct Cell<S: Clone + Send> {
    writer: UnsafeCell<Writer<S>>,
}

// SAFETY: Writer is Send. Only the unique live RtHost may access the UnsafeCell;
// control and directory roots only retain allocation ownership. The lifecycle
// gate reserves one host identity until that host is stopped and dropped off RT.
unsafe impl<S: Clone + Send> Sync for Cell<S> {}

trait ErasedCell: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
}
impl<S: Clone + Send + 'static> ErasedCell for Cell<S> {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(crate) struct RootInner {
    owner: Box<dyn ErasedCell>,
    charge: Option<Charge>,
}

#[derive(Clone)]
pub(crate) struct CellRoot(Option<Arc<RootInner>>);

impl CellRoot {
    pub(crate) fn new<S: Clone + Send + 'static>(producer: Producer<S>, capacity: usize) -> Self {
        Self(Some(Arc::new(RootInner {
            owner: Box::new(Cell {
                writer: UnsafeCell::new(Writer {
                    producer,
                    sequence: Sequence::default(),
                    demand: DemandSnapshot {
                        active: false,
                        generation: 0,
                    },
                    capacity,
                }),
            }),
            charge: None,
        })))
    }
    pub(crate) fn attach_charge(&mut self, charge: Charge) {
        Arc::get_mut(self.0.as_mut().expect("live cell"))
            .expect("unshared prepared cell")
            .charge = Some(charge);
    }
    pub(crate) fn pointer<S: Clone + Send + 'static>(&self) -> Pointer {
        let cell = self
            .0
            .as_ref()
            .expect("live cell")
            .owner
            .as_any()
            .downcast_ref::<Cell<S>>()
            .expect("sealed shape matches registered producer");
        Pointer(cell as *const Cell<S> as *const ())
    }
    #[cfg(test)]
    pub(crate) fn test_writer<S: Clone + Send + 'static>(&mut self) -> &mut Writer<S> {
        let pointer = self.pointer::<S>();
        // SAFETY: Private transport tests exclusively borrow the Bus storage,
        // before any public host claims it. No production writer bypass exists.
        unsafe { &mut *(*(pointer.0.cast::<Cell<S>>())).writer.get() }
    }
}
impl Drop for CellRoot {
    fn drop(&mut self) {
        if let Some(root) = Arc::into_inner(self.0.take().expect("live cell")) {
            // The Arc allocation has freed before the box and its charge drop.
            // RootInner field order frees the producer box before refunding.
            drop(root);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pointer(*const ());
#[cfg(test)]
impl Pointer {
    pub(crate) fn address(self) -> usize {
        self.0 as usize
    }
}
// SAFETY: This pointer is an identity, never a freestanding access capability.
// It is resolved and type checked off RT and used only by Access below while
// borrowing the unique host. Retained cell roots keep its box address stable.
unsafe impl Send for Pointer {}
// SAFETY: Sharing pointer identities cannot read or mutate the producer.
unsafe impl Sync for Pointer {}

pub(crate) struct Access<'a> {
    pub(crate) binding: &'a super::binding::ErasedBinding,
    pub(crate) host: &'a mut super::RtHost,
    pub(crate) marker: PhantomData<std::rc::Rc<()>>,
}
impl<'a> Access<'a> {
    pub(crate) fn demand<S: Clone + Send + 'static>(
        self,
    ) -> Result<DemandSnapshot, crate::PublishError> {
        let (writer, state) = self.writer::<S>();
        if state.demand.is_closed() {
            return Err(crate::PublishError::Closed);
        }
        Ok(writer.demand)
    }
    pub(crate) fn writer<S: Clone + Send + 'static>(
        self,
    ) -> (&'a mut Writer<S>, &'a crate::subscriber::TopicState) {
        let owner = &self.binding.owner;
        debug_assert_eq!(owner.host, self.host.root.id);
        // SAFETY: SignalScope validated current table, host, owner, topic, key
        // and the off-RT typed pointer. The sealed shape selects the original S.
        // Access consumes an actual exclusive host borrow for 'a; scope/guard
        // borrows prevent a second writer or activation until this borrow ends.
        // Table and host control roots retain this stable box and StateHandle
        // throughout 'a. RT never accesses the erased owner via Any/downcast.
        let writer = unsafe { &mut *(*(owner.pointer.0.cast::<Cell<S>>())).writer.get() };
        (writer, &owner.slot.state)
    }
}

pub(crate) fn refresh(host: &mut super::RtHost) {
    if let Some(table) = &host.active {
        for binding in &table.bindings {
            (binding.owner.refresh)(
                binding.owner.pointer,
                binding.owner.slot.state.demand.snapshot(),
            );
        }
    }
}

pub(crate) fn refresh_typed<S: Clone + Send + 'static>(pointer: Pointer, demand: DemandSnapshot) {
    // SAFETY: Only refresh(&mut RtHost) calls the function pointer bound to S
    // during preparation. Current table roots retain its cell; no scope can
    // coexist with begin_block's exclusive host borrow.
    unsafe {
        (*(*(pointer.0.cast::<Cell<S>>())).writer.get()).demand = demand;
    }
}
