use moiren_signal::{
    BlockShape, DemandSnapshot, PublishError, PublishOutcome, PublisherHandle,
    SignalBlockWriteGuard, SignalScope, ValueShape, WriteError,
};
use std::{marker::PhantomData, rc::Rc};

/// Per-operation, borrowed permission to publish engine observations.
///
/// The empty constructor intentionally borrows a call-local token. It is not a
/// default capability and therefore cannot be manufactured without a render
/// call. The `Rc` marker keeps the scope local to the render thread.
///
/// ```compile_fail
/// use moiren_engine::signal::ProcessSignals;
/// fn duplicate(signals: ProcessSignals<'_>) { let _ = signals.clone(); }
/// ```
/// ```compile_fail
/// use moiren_engine::signal::ProcessSignals;
/// fn send(signals: ProcessSignals<'_>) {
///     std::thread::scope(|threads| { threads.spawn(move || drop(signals)); });
/// }
/// ```
/// ```compile_fail
/// fn require_sync<Value: Sync>() {}
/// require_sync::<moiren_engine::signal::ProcessSignals<'static>>();
/// ```
/// ```compile_fail
/// let _ = moiren_engine::signal::ProcessSignals::default();
/// ```
/// ```compile_fail
/// use moiren_engine::signal::ProcessSignals;
/// fn escape(signals: ProcessSignals<'_>) -> ProcessSignals<'static> { signals }
/// ```
/// ```compile_fail
/// use moiren_engine::signal::ProcessSignals;
/// use moiren_signal::{LatestBlock, PublisherHandle, SignalBlockWriteGuard};
/// fn escape_guard<'call>(mut signals: ProcessSignals<'call>,
///     handle: &PublisherHandle<LatestBlock<u64, f32>>)
///     -> SignalBlockWriteGuard<'call, u64, f32> {
///     signals.try_write(handle, 1).unwrap()
/// }
/// ```
pub struct ProcessSignals<'a> {
    scope: Option<SignalScope<'a>>,
    marker: PhantomData<Rc<()>>,
}

impl<'a> ProcessSignals<'a> {
    pub(crate) fn from_scope(scope: SignalScope<'a>) -> Self {
        Self {
            scope: Some(scope),
            marker: PhantomData,
        }
    }

    pub(crate) fn empty(_call: &'a mut ()) -> Self {
        Self {
            scope: None,
            marker: PhantomData,
        }
    }

    pub fn demand<P: moiren_signal::Shape>(
        &mut self,
        handle: &PublisherHandle<P>,
    ) -> Result<DemandSnapshot, PublishError> {
        match &mut self.scope {
            Some(scope) => scope.demand(handle),
            None => Ok(DemandSnapshot {
                active: false,
                generation: 0,
            }),
        }
    }

    pub fn publish<P: ValueShape>(
        &mut self,
        handle: &PublisherHandle<P>,
        value: P::Value,
    ) -> Result<PublishOutcome, PublishError> {
        match &mut self.scope {
            Some(scope) => scope.publish(handle, value),
            None => Err(PublishError::InactiveBinding),
        }
    }

    pub fn try_write<'borrow, P: BlockShape>(
        &'borrow mut self,
        handle: &PublisherHandle<P>,
        max_len: usize,
    ) -> Result<SignalBlockWriteGuard<'borrow, P::Header, P::Element>, WriteError> {
        match &mut self.scope {
            Some(scope) => scope.try_write_uniform(handle, max_len),
            None => Err(WriteError::Invalid(PublishError::InactiveBinding)),
        }
    }

    pub fn publish_slice<P: BlockShape>(
        &mut self,
        handle: &PublisherHandle<P>,
        header: P::Header,
        elements: &[P::Element],
    ) -> Result<PublishOutcome, PublishError> {
        match &mut self.scope {
            Some(scope) => scope.publish_slice(handle, header, elements),
            None => Err(PublishError::InactiveBinding),
        }
    }
}

impl<'a> From<SignalScope<'a>> for ProcessSignals<'a> {
    fn from(scope: SignalScope<'a>) -> Self {
        Self::from_scope(scope)
    }
}
