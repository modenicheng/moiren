use super::{OpSignalBindings, PublisherHandle, RtHost, slot::Access};
use crate::{BlockShape, PublishError, PublishOutcome, Shape, ValueShape, WriteError};
use std::{marker::PhantomData, rc::Rc};

/// Exclusive host permission for one current operation table.
///
/// ```compile_fail
/// use moiren_signal::SignalScope;
/// fn duplicate(scope: SignalScope<'_>) { let _ = scope.clone(); }
/// ```
/// ```compile_fail
/// use moiren_signal::SignalScope;
/// fn send(scope: SignalScope<'_>) {
///     std::thread::scope(|threads| { threads.spawn(move || drop(scope)); });
/// }
/// ```
/// ```compile_fail
/// fn require_sync<Value: Sync>() {}
/// require_sync::<moiren_signal::SignalScope<'static>>();
/// ```
pub struct SignalScope<'a> {
    pub(crate) host: &'a mut RtHost,
    pub(crate) bindings: &'a OpSignalBindings,
    pub(crate) marker: PhantomData<Rc<()>>,
}
impl SignalScope<'_> {
    pub fn demand<P: Shape>(
        &mut self,
        handle: &PublisherHandle<P>,
    ) -> Result<crate::DemandSnapshot, PublishError> {
        P::demand_access(self.access(handle)?)
    }
    pub(crate) fn access<P: Shape>(
        &mut self,
        handle: &PublisherHandle<P>,
    ) -> Result<Access<'_>, PublishError> {
        if handle.host != self.host.root.id || self.bindings.host != self.host.root.id {
            return Err(PublishError::ForeignHost);
        }
        if self
            .host
            .active
            .as_ref()
            .is_none_or(|active| active.identity != self.bindings.table)
            || handle.key != self.bindings.key
        {
            return Err(PublishError::InactiveBinding);
        }
        let binding = self
            .bindings
            .bindings
            .iter()
            .find(|binding| {
                let owner = &binding.owner;
                owner.identity == handle.identity
                    && owner.topic == handle.topic
                    && owner.pointer == handle.pointer
                    && owner.key == handle.key
            })
            .ok_or(PublishError::InactiveBinding)?;
        Ok(Access {
            binding,
            host: self.host,
            marker: PhantomData,
        })
    }
    pub fn publish<P: ValueShape>(
        &mut self,
        handle: &PublisherHandle<P>,
        value: P::Value,
    ) -> Result<PublishOutcome, PublishError> {
        P::publish_access(self.access(handle)?, value)
    }
    pub fn try_write<'borrow, P: BlockShape>(
        &'borrow mut self,
        handle: &PublisherHandle<P>,
        max_len: usize,
    ) -> Result<
        <P as crate::types::sealed::BlockDispatch<P::Header, P::Element>>::Guard<'borrow>,
        WriteError,
    > {
        P::typed_write_access(self.access(handle)?, max_len)
    }
    pub fn try_write_uniform<'borrow, P: BlockShape>(
        &'borrow mut self,
        handle: &PublisherHandle<P>,
        max_len: usize,
    ) -> Result<SignalBlockWriteGuard<'borrow, P::Header, P::Element>, WriteError> {
        P::write_access(self.access(handle)?, max_len)
    }
    pub fn publish_slice<P: BlockShape>(
        &mut self,
        handle: &PublisherHandle<P>,
        header: P::Header,
        elements: &[P::Element],
    ) -> Result<PublishOutcome, PublishError> {
        let mut write = match P::write_access(self.access(handle)?, elements.len()) {
            Ok(write) => write,
            Err(error) => return crate::error::flatten_write_error(error),
        };
        *write.header_mut() = header;
        write.elements_mut().copy_from_slice(elements);
        write.commit(elements.len())
    }
}

/// The guard borrows the scope's actual exclusive host borrow.
///
/// ```compile_fail
/// use moiren_signal::*;
/// let mut bus = Bus::new(Default::default()).unwrap();
/// let topic = bus.register_latest_block("block", BlockInit { header: 0_u32, element: 0_u32, capacity: 2 }).unwrap();
/// let _reader = bus.subscribe(&topic, Default::default()).unwrap();
/// let (control, mut host) = bus.create_host(1).unwrap();
/// let key = control.new_binding().unwrap();
/// let publisher = control.prepare(&topic, key).unwrap();
/// let update = control.prepare_update(&[publisher.erase()]).unwrap();
/// let op = update.op_bindings(key).unwrap();
/// drop(host.activate(update).unwrap());
/// host.begin_block();
/// let mut write = { let mut scope = host.scope(&op); scope.try_write(&publisher.handle(), 2).unwrap() };
/// write.commit(2).unwrap();
/// ```
/// ```compile_fail
/// use moiren_signal::*;
/// let mut bus = Bus::new(Default::default()).unwrap();
/// let topic = bus.register_stream_block("block", BlockInit { header: 0_u32, element: 0_u32, capacity: 2 }, 2).unwrap();
/// let _reader = bus.subscribe(&topic, Default::default()).unwrap();
/// let (control, mut host) = bus.create_host(1).unwrap();
/// let key = control.new_binding().unwrap();
/// let publisher = control.prepare(&topic, key).unwrap();
/// let update = control.prepare_update(&[publisher.erase()]).unwrap();
/// let op = update.op_bindings(key).unwrap();
/// drop(host.activate(update).unwrap());
/// host.begin_block();
/// let write = { let mut scope = host.scope(&op); scope.try_write(&publisher.handle(), 2).unwrap() };
/// write.commit(2).unwrap();
/// ```
/// ```compile_fail
/// fn require_send<Value: Send>() {}
/// require_send::<moiren_signal::SignalBlockWriteGuard<'static, u32, u32>>();
/// ```
#[must_use = "a block write is published only by explicit commit"]
pub enum SignalBlockWriteGuard<'a, H: Copy + Send + 'static, E: Copy + Send + 'static> {
    Latest(crate::BlockWriteGuard<'a, H, E>),
    Stream(crate::StreamBlockWriteGuard<'a, H, E>),
}
impl<H: Copy + Send + 'static, E: Copy + Send + 'static> SignalBlockWriteGuard<'_, H, E> {
    pub fn header_mut(&mut self) -> &mut H {
        match self {
            Self::Latest(write) => write.header_mut(),
            Self::Stream(write) => write.header_mut(),
        }
    }
    pub fn elements_mut(&mut self) -> &mut [E] {
        match self {
            Self::Latest(write) => write.elements_mut(),
            Self::Stream(write) => write.elements_mut(),
        }
    }
    pub fn commit(self, len: usize) -> Result<PublishOutcome, PublishError> {
        match self {
            Self::Latest(write) => write.commit(len),
            Self::Stream(write) => write.commit(len),
        }
    }
}
